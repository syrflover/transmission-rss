//! The to-dos read from the jobs, the subscribed creators and the library.

use std::collections::HashMap;

use trss_collect::store::{
    channels::ChannelStore, history::HistoryStore, revisions::RevisionStore,
};
use trss_core::settings::SettingsStore;
use trss_library::store::{artwork::ArtworkStore, library::LibraryStore};

use super::{Changes, Received, Todo, TodoError, WorkRef};
use crate::{
    follow::{EpisodeCheck, Follow},
    model::PlanState,
    store::{JobRow, JobStore},
    ItemState, Wait,
};

/// The stores the to-dos are read from.
pub struct Sources<'a> {
    pub jobs: &'a JobStore,
    pub follow: &'a Follow,
    pub library: &'a LibraryStore,
    pub artwork: &'a ArtworkStore,
    pub revisions: &'a RevisionStore,
    pub history: &'a HistoryStore,
    pub channels: &'a ChannelStore,
    pub settings: &'a SettingsStore,
    /// The address of a work's cover image, from the work's ID and the
    /// image's: how the web serves it.
    pub cover_url: fn(work_id: &str, image_id: &str) -> String,
}

impl Sources<'_> {
    /// The cover image ID of each of the works `ids` names that has one.
    pub(super) async fn covers(
        &self,
        mut ids: Vec<String>,
    ) -> Result<HashMap<String, String>, TodoError> {
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.artwork
            .image_ids_of(ids)
            .await
            .map_err(TodoError::read)
    }

    /// The work `id` named `name`, with its cover when `covers` has one.
    pub(super) fn work(
        &self,
        id: String,
        name: String,
        covers: &HashMap<String, String>,
    ) -> WorkRef {
        WorkRef {
            cover_url: covers.get(&id).map(|image| (self.cover_url)(&id, image)),
            id,
            name,
        }
    }

    /// The work a job is about, while the library has it, with its cover.
    fn work_of(&self, row: &JobRow, covers: &HashMap<String, String>) -> Option<WorkRef> {
        let (id, name) = (row.work_id.as_ref()?, row.work_name.as_ref()?);
        Some(self.work(id.clone(), name.clone(), covers))
    }

    async fn covers_of_jobs(&self, rows: &[JobRow]) -> Result<HashMap<String, String>, TodoError> {
        self.covers(rows.iter().filter_map(|r| r.work_id.clone()).collect())
            .await
    }
}

/// `rows` (oldest first) grouped by their work, or by the job itself when it
/// has none: `<prefix>:<work or job id>` and the group's jobs, the groups in
/// the order their first job comes in, so each group's first job is its
/// oldest.
fn group_by_work<'a>(rows: &'a [JobRow], prefix: &str) -> Vec<(String, Vec<&'a JobRow>)> {
    let mut groups: Vec<(String, Vec<&JobRow>)> = Vec::new();
    for row in rows {
        let key = format!("{prefix}:{}", row.work_id.as_deref().unwrap_or(&row.id));
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, rows)) => rows.push(row),
            None => groups.push((key, vec![row])),
        }
    }
    groups
}

/// The `인증 필요` to-dos: one per work whose jobs wait for a person to pass
/// a site's check (per job when it has no work).
pub(super) async fn auth(sources: &Sources<'_>) -> Result<Vec<Todo>, TodoError> {
    let waits = sources.jobs.auth_waits().await.map_err(TodoError::read)?;
    let covers = sources.covers_of_jobs(&waits).await?;
    let mut todos = Vec::new();
    for (key, rows) in group_by_work(&waits, "auth") {
        let oldest = rows[0];
        let (mut episodes, mut reason) = (Vec::new(), None);
        for row in &rows {
            for item in sources.jobs.items(&row.id).await.map_err(TodoError::read)? {
                if item.state == ItemState::Waiting && item.wait == Some(Wait::Auth) {
                    reason = reason.or(item.reason);
                    if !episodes.contains(&item.episode) {
                        episodes.push(item.episode);
                    }
                }
            }
        }
        todos.push(Todo::Auth {
            key,
            at: oldest.state_at,
            work: sources.work_of(oldest, &covers),
            title: oldest.title(),
            season: oldest.season,
            episodes,
            creator: oldest.creator.clone(),
            reason: reason.unwrap_or_default(),
            job_id: oldest.id.clone(),
            jobs: rows.len(),
        });
    }
    Ok(todos)
}

/// The `교체 승인` to-dos: one per work whose jobs wait for a person to
/// approve or refuse replacing an episode's subtitle (per job when it has no
/// work), which opens its oldest job's detail.
pub(super) async fn replacements(sources: &Sources<'_>) -> Result<Vec<Todo>, TodoError> {
    let waits = sources
        .jobs
        .approval_waits()
        .await
        .map_err(TodoError::read)?;
    let covers = sources.covers_of_jobs(&waits).await?;
    let mut todos = Vec::new();
    for (key, rows) in group_by_work(&waits, "replacement") {
        let oldest = rows[0];
        let mut episodes = Vec::new();
        let mut changes = Changes::default();
        let mut received = Received::default();
        for row in &rows {
            let plans = sources
                .jobs
                .replacements(&row.id)
                .await
                .map_err(TodoError::read)?;
            for view in plans {
                if view.plan.state != PlanState::Open {
                    continue;
                }
                if !episodes.contains(&view.plan.episode) {
                    episodes.push(view.plan.episode);
                }
                changes.add(view.comparison.as_ref());
                received.add(&view);
            }
        }
        episodes.sort();
        todos.push(Todo::Replacement {
            key,
            at: oldest.state_at,
            work: sources.work_of(oldest, &covers),
            title: oldest.title(),
            season: oldest.season,
            episodes,
            creator: oldest.creator.clone(),
            job_id: oldest.id.clone(),
            jobs: rows.len(),
            changes,
            current_changed_at: received
                .current_changed
                .filter(|_| received.current.is_none()),
            current_received_at: received.current,
            new_received_at: received.new,
        });
    }
    Ok(todos)
}

/// The `회차 확인 필요` to-dos of jobs: one per job whose files wait for a
/// person to say their episode (`docs/specs/jobs.md`, 할 일), which opens the
/// job's detail. Gone once the job no longer waits for it.
pub(super) async fn placement_checks(sources: &Sources<'_>) -> Result<Vec<Todo>, TodoError> {
    let waits = sources
        .jobs
        .placement_waits()
        .await
        .map_err(TodoError::read)?;
    let covers = sources.covers_of_jobs(&waits).await?;
    let mut todos = Vec::new();
    for row in &waits {
        // An upload's or a find job's table is confirmed as a whole: its
        // note says what it waits for, not one file's question.
        let Some((asked, whole)) = sources
            .jobs
            .placeable(&row.id)
            .await
            .map_err(TodoError::read)?
        else {
            continue;
        };
        todos.push(Todo::PlacementCheck {
            key: format!("placement:{}", row.id),
            at: row.state_at,
            work: sources.work_of(row, &covers),
            title: row.title(),
            season: row.season,
            creator: row.creator.clone(),
            origin: row.origin.clone(),
            source: row.source.clone(),
            files: asked.iter().map(|r| r.name.clone()).collect(),
            reason: match whole {
                true => row.note.clone(),
                false => asked.first().and_then(|r| r.question.clone()),
            },
            job_id: row.id.clone(),
        });
    }
    Ok(todos)
}

/// The `회차 확인 필요` to-dos: one per work whose subscribed creator's
/// mapping is undecided or has an episode that fits no mapping and no exception
/// ([`Follow::episode_checks`]). Derived at each read, so it is gone as soon
/// as the user's mapping or exceptions cover what it asked about.
pub(super) async fn episode_checks(sources: &Sources<'_>) -> Result<Vec<Todo>, TodoError> {
    let checks = sources
        .follow
        .episode_checks()
        .await
        .map_err(TodoError::read)?;
    // One to-do per work: the lowest season's check names the creator and the
    // episodes; it has been waiting since the oldest of the work's checks.
    let mut groups: Vec<(EpisodeCheck, usize, i64)> = Vec::new();
    for check in checks {
        match groups.iter_mut().find(|(g, ..)| g.work_id == check.work_id) {
            Some((_, count, since)) => {
                *count += 1;
                *since = (*since).min(check.since);
            }
            None => {
                let since = check.since;
                groups.push((check, 1, since));
            }
        }
    }
    let covers = sources
        .covers(groups.iter().map(|(g, ..)| g.work_id.clone()).collect())
        .await?;
    Ok(groups
        .into_iter()
        .map(|(check, sources_count, since)| Todo::EpisodeCheck {
            key: format!("episode:{}", check.work_id),
            at: since,
            title: check.anime_title.unwrap_or_else(|| check.work_name.clone()),
            work: Some(sources.work(check.work_id, check.work_name, &covers)),
            season: check.season,
            creator: check.creator,
            source_id: check.source_id,
            episodes: check.episodes,
            reason: check.undecided,
            sources: sources_count,
        })
        .collect())
}

/// The `회차 확인 필요` to-dos of videos: one per video of a season folder
/// whose name gives no episode of it, until a person checks it or it is
/// renamed or moved ([`LibraryStore::video_checks`]).
pub(super) async fn video_checks(sources: &Sources<'_>) -> Result<Vec<Todo>, TodoError> {
    let checks = sources
        .library
        .video_checks()
        .await
        .map_err(TodoError::read)?;
    let covers = sources
        .covers(checks.iter().map(|c| c.work_id.clone()).collect())
        .await?;
    Ok(checks
        .into_iter()
        .map(|check| Todo::VideoCheck {
            key: format!("video:{}:{}", check.work_id, check.path),
            at: check.identity.mtime_ns.div_euclid(1_000_000),
            title: check.work_name.clone(),
            work: Some(sources.work(check.work_id, check.work_name, &covers)),
            season: check.season,
            reason: check.reason.message().to_owned(),
            seen: super::seen(check.identity),
            path: check.path,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::JobState,
        store::{JobRow, Progress},
    };

    /// A job of `work` (`None`: the library has none) that waits.
    fn row(id: &str, work: Option<&str>) -> JobRow {
        JobRow {
            seq: 0,
            id: id.to_owned(),
            origin: "pick".into(),
            revision_of: None,
            revises_job: None,
            revises_attributed: false,
            state: JobState::Waiting,
            wait: None,
            stage: None,
            note: None,
            state_at: 0,
            created_at: 0,
            finished_at: None,
            work_id: work.map(str::to_owned),
            work_name: work.map(str::to_owned),
            season: None,
            anime_no: None,
            anime_title: None,
            creator: None,
            episodes: vec![],
            source: None,
            progress: Progress::default(),
            failure: None,
            upload: None,
            finishing: false,
            receiving: false,
        }
    }

    fn ids(groups: &[(String, Vec<&JobRow>)]) -> Vec<(String, Vec<String>)> {
        groups
            .iter()
            .map(|(key, rows)| (key.clone(), rows.iter().map(|r| r.id.clone()).collect()))
            .collect()
    }

    #[test]
    fn jobs_of_a_work_are_one_group_with_the_oldest_job_first() {
        let rows = [
            row("j1", Some("w1")),
            row("j2", Some("w2")),
            row("j3", Some("w1")),
        ];

        assert_eq!(
            ids(&group_by_work(&rows, "auth")),
            [
                ("auth:w1".to_owned(), vec!["j1".to_owned(), "j3".to_owned()]),
                ("auth:w2".to_owned(), vec!["j2".to_owned()]),
            ]
        );
    }

    #[test]
    fn a_job_of_no_work_is_a_group_of_its_own() {
        let rows = [row("j1", None), row("j2", None)];

        assert_eq!(
            ids(&group_by_work(&rows, "replacement")),
            [
                ("replacement:j1".to_owned(), vec!["j1".to_owned()]),
                ("replacement:j2".to_owned(), vec!["j2".to_owned()]),
            ]
        );
    }
}
