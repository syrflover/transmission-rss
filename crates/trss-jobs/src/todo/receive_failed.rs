//! The `받기 실패` to-dos: the failures of the video replacements and of the
//! rules' adds, one to-do per work (per rule for an add).

use std::{collections::HashMap, path::Path};

use trss_collect::store::{
    history::{HistoryItem, HistoryQuery, HistoryResult, HistoryStore},
    revisions::{Revision, RevisionStore, WorkRef as FolderWork},
};
use trss_core::{episode::segments, trname_names::season_episode};

use super::{gather::Sources, Todo, TodoError, WorkRef};

/// The newest add failures listed.
pub const ADD_FAILURES: usize = 200;

/// A failed video replacement with the library's work at its folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedRevision {
    pub row: Revision,
    /// `None` when the library has no work at the folder.
    pub work: Option<FolderWork>,
}

/// A history item a rule picked that Transmission did not add.
#[derive(Debug, Clone, PartialEq)]
pub struct AddFailed {
    pub rule_id: String,
    pub item: HistoryItem,
}

/// What the `받기 실패` are made of: the failed replacements, and the newest
/// [`ADD_FAILURES`] adds of the rules that failed, newest first
/// (`GET /api/todo/receive-failures` lists them item by item, the to-do per
/// work and rule).
#[derive(Debug, Clone, PartialEq)]
pub struct ReceiveFailures {
    pub revisions: Vec<FailedRevision>,
    pub adds: Vec<AddFailed>,
}

/// Reads the failures the `받기 실패` are made of.
pub async fn receive_failures(
    revisions: &RevisionStore,
    history: &HistoryStore,
) -> Result<ReceiveFailures, TodoError> {
    let mut failed = Vec::new();
    for row in revisions.failures().await.map_err(TodoError::read)? {
        let work = revisions
            .work_at(row.folder.clone())
            .await
            .map_err(TodoError::read)?;
        failed.push(FailedRevision { row, work });
    }
    let adds = history
        .list(HistoryQuery {
            result: Some(HistoryResult::AddFailed),
            limit: ADD_FAILURES,
            ..Default::default()
        })
        .await
        .map_err(TodoError::read)?
        .items
        .into_iter()
        .filter_map(|item| {
            Some(AddFailed {
                rule_id: item.rule_id.clone()?,
                item,
            })
        })
        .collect();
    Ok(ReceiveFailures {
        revisions: failed,
        adds,
    })
}

/// One `받기 실패` to-do as it is gathered.
struct FailedGroup {
    key: String,
    context: &'static str,
    at: i64,
    /// The library's work it is about: its ID and name.
    work: Option<(String, String)>,
    title: String,
    season: Option<u32>,
    episodes: Vec<String>,
    count: usize,
    reason: Option<String>,
    channel_id: Option<String>,
}

impl FailedGroup {
    /// Adds one failure at `at`; the newest gives the reason and season.
    fn add(&mut self, at: i64, reason: Option<String>, episode: Option<(u32, String)>) {
        self.count += 1;
        if at >= self.at {
            self.at = at;
            self.reason = reason;
            if let Some((season, _)) = &episode {
                self.season = Some(*season);
            }
        }
        if let Some((_, episode)) = episode {
            if !self.episodes.contains(&episode) {
                self.episodes.push(episode);
            }
        }
    }
}

pub(super) async fn todos(sources: &Sources<'_>) -> Result<Vec<Todo>, TodoError> {
    let mut groups: Vec<FailedGroup> = Vec::new();
    let add = |groups: &mut Vec<FailedGroup>, fresh: FailedGroup, at, reason, episode| {
        let index = match groups.iter().position(|g| g.key == fresh.key) {
            Some(index) => index,
            None => {
                groups.push(fresh);
                groups.len() - 1
            }
        };
        groups[index].add(at, reason, episode);
    };

    let gathered = receive_failures(sources.revisions, sources.history).await?;
    for FailedRevision { row, work } in gathered.revisions {
        let folder_name = Path::new(&row.folder)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| row.folder.clone());
        let fresh = FailedGroup {
            key: format!(
                "revision:{}",
                work.as_ref().map(|w| w.id.as_str()).unwrap_or(&row.folder)
            ),
            context: "revision",
            at: i64::MIN,
            title: work.as_ref().map(|w| w.name.clone()).unwrap_or(folder_name),
            work: work.map(|w| (w.id, w.name)),
            season: None,
            episodes: Vec::new(),
            count: 0,
            reason: None,
            channel_id: None,
        };
        add(
            &mut groups,
            fresh,
            row.updated_at,
            row.reason.clone(),
            season_episode(&row.episode_name),
        );
    }

    let collect_folder = sources
        .settings
        .collection()
        .await
        .map_err(TodoError::read)?
        .map(|collect| collect.folder);
    let mut rules = HashMap::new();
    for AddFailed { rule_id, item } in gathered.adds {
        if !rules.contains_key(&rule_id) {
            let rule = sources.channels.get_rule(&rule_id).await?;
            let work = match (&rule, &collect_folder) {
                (Some(rule), Some(folder)) => sources
                    .revisions
                    .work_at(
                        Path::new(folder)
                            .join(&rule.directory)
                            .to_string_lossy()
                            .into_owned(),
                    )
                    .await
                    .map_err(TodoError::read)?,
                _ => None,
            };
            rules.insert(rule_id.clone(), (rule.map(|r| r.directory), work));
        }
        let (directory, work) = &rules[&rule_id];
        let fresh = FailedGroup {
            key: format!("add_failed:{rule_id}"),
            context: "add_failed",
            at: i64::MIN,
            title: work
                .as_ref()
                .map(|w| w.name.clone())
                .or_else(|| directory.clone())
                .unwrap_or_else(|| item.title.clone()),
            work: work.as_ref().map(|w| (w.id.clone(), w.name.clone())),
            season: None,
            episodes: Vec::new(),
            count: 0,
            reason: None,
            channel_id: Some(item.channel_id.clone()),
        };
        add(
            &mut groups,
            fresh,
            item.result_at,
            item.reason.clone(),
            None,
        );
    }

    let covers = sources
        .covers(
            groups
                .iter()
                .filter_map(|g| g.work.as_ref().map(|(id, _)| id.clone()))
                .collect(),
        )
        .await?;
    Ok(groups
        .into_iter()
        .map(|g| Todo::ReceiveFailed {
            key: g.key,
            at: g.at,
            context: g.context,
            work: g
                .work
                .map(|(id, name)| -> WorkRef { sources.work(id, name, &covers) }),
            title: g.title,
            season: g.season,
            episode_segments: segments(g.episodes.iter().map(String::as_str)),
            episodes: g.episodes,
            count: g.count,
            reason: g.reason,
            channel_id: g.channel_id,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group() -> FailedGroup {
        FailedGroup {
            key: "revision:w1".into(),
            context: "revision",
            at: i64::MIN,
            work: None,
            title: "Show".into(),
            season: None,
            episodes: Vec::new(),
            count: 0,
            reason: None,
            channel_id: None,
        }
    }

    #[test]
    fn the_newest_failure_gives_the_reason_and_the_season() {
        let mut g = group();

        g.add(20, Some("newer".into()), Some((2, "03".into())));
        g.add(10, Some("older".into()), Some((1, "02".into())));

        assert_eq!(g.at, 20);
        assert_eq!(g.reason.as_deref(), Some("newer"));
        assert_eq!(g.season, Some(2));
        assert_eq!(g.count, 2);
    }

    #[test]
    fn a_failure_at_the_same_time_as_the_newest_takes_over() {
        let mut g = group();

        g.add(10, Some("first".into()), None);
        g.add(10, Some("second".into()), None);

        assert_eq!(g.reason.as_deref(), Some("second"));
    }

    #[test]
    fn an_episode_is_listed_once_in_the_order_it_was_first_seen() {
        let mut g = group();

        g.add(1, None, Some((1, "14".into())));
        g.add(2, None, Some((1, "13".into())));
        g.add(3, None, Some((1, "14".into())));

        assert_eq!(g.episodes, ["14", "13"]);
        assert_eq!(g.count, 3);
    }

    #[test]
    fn a_failure_of_no_episode_keeps_the_season_it_had() {
        let mut g = group();

        g.add(1, None, Some((3, "01".into())));
        g.add(2, None, None);

        assert_eq!(g.season, Some(3));
        assert_eq!(g.episodes, ["01"]);
    }
}
