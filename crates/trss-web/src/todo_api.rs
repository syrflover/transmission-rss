//! The to-dos (`docs/specs/jobs.md`, 할 일).
//!
//! `GET /api/todo` is what needs the person (`처리 필요`), the cards of the
//! to-do screen, and `count`, the menu badge:
//!
//! ```json
//! { "needs": [
//!   { "kind": "auth", "key": "auth:<work id>", "at": 1760000000000,
//!     "work": { "id": "…", "name": "Show", "cover_url": "…" }, "title": "작품",
//!     "season": 1, "episodes": ["11", "12"], "creator": "제작자",
//!     "reason": "CAPTCHA", "job_id": "…", "jobs": 1 },
//!   { "kind": "receive_failed", "key": "revision:<work id>", "at": 1759990000000,
//!     "context": "revision", "work": { … }, "title": "Show", "season": 1,
//!     "episodes": ["14"], "count": 1, "reason": "…", "channel_id": null } ],
//!   "count": 2 }
//! ```
//!
//! - `auth` (`인증 필요`): the subtitle jobs waiting for a person to pass a
//!   site's check, one to-do per work (per job when it has no work). `at` is
//!   since when its oldest job waits, `job_id` that job, `episodes` the items
//!   that wait.
//! - `receive_failed` (`받기 실패`): the failures listed below, one to-do per
//!   work for revisions (`context: "revision"`, the work's folder when the
//!   library has no work there) and one per rule for add failures
//!   (`context: "add_failed"`, with the rule's work when the collect folder
//!   holds it, and the channel for the history's filter).
//!
//! `auth` comes before `receive_failed`, each newest first. Failed subtitle
//! jobs are not to-dos: the screen's job list shows them. The suggestions
//! (`제안`) come from their own APIs. `GET /api/todo/count` is `{ "count" }`
//! alone.
//!
//! `GET /api/todo/receive-failures`: the source of the to-do kind `받기 실패`:
//! collection failures the person has to deal with, which the work detail
//! also shows on their episode rows ([`super::library_work_api`]).
//!
//! ```json
//! { "items": [
//!   { "kind": "revision", "at": 1760000000000, "history_item_id": 12,
//!     "title": "[SubsPlease] Show - 14v2 (1080p) [1A2B3C4D].mkv",
//!     "work": { "id": "…", "name": "Show" }, "season": 1, "episode": "14",
//!     "reason": "받은 영상의 CRC32 …",
//!     "files": [
//!       { "role": "old", "path": "Season 01/Show S01E14.mkv", "state": "kept" },
//!       { "role": "new", "path": "Season 01/[SubsPlease] Show - 14v2 (1080p) [1A2B3C4D].mkv",
//!         "state": "received_name" } ],
//!     "can_retry": false, "retry_blocked": null, "command": null },
//!   { "kind": "add_failed", "at": 1759990000000, "history_item_id": 9,
//!     "title": "…", "reason": "Transmission에 연결하지 못했어요: …" } ] }
//! ```
//!
//! - `revision` items are replacements of video revisions that failed, or
//!   whose rename after the old video was removed has not gone through yet
//!   ([`trss_collect::store::revisions::Revision::is_failure`]). `files` are the two
//!   videos with what became of each: the old one `kept` (still there) or
//!   `removed`; the new one under the name it was received with
//!   (`received_name`), `missing` from there (a replacement that ended after
//!   the old video was removed, so the episode has no video), or
//!   `not_received` (its download did not complete or is not in the rule's
//!   folder, so `path` is `null`). Paths are relative to
//!   the work folder; `work` is `null` when the library has no work at that
//!   folder, and the paths are then absolute. An item goes away once the
//!   worker sees one of the two files gone, or the rename go through; a
//!   `not_received` one also once its torrent is right again or a cycle
//!   receives its item again. `can_retry` says whether `다시 받기` (the
//!   `receive_once` command of `history_item_id`) is offered: on a revision
//!   whose download stopped before it was received (its torrent left
//!   Transmission or reported an error), or whose replacement ended with no
//!   video under the episode name, even when it left the feed or came
//!   from a past episode search, while the rule recorded on it is active and
//!   no higher revision of the episode is in the folder or on its way
//!   ([`receive_once::RevisionRetry`]) and the rule's folder is the one the
//!   replacement was decided for ([`receive_once::same_destination`]), and
//!   the episode's place is not known to hold the same or a higher revision
//!   already ([`super::in_place`]). `retry_blocked` says why it is
//!   missing on such a revision, and `command` is its command that has not
//!   ended yet. The link it receives comes from the history record, never
//!   from the screen.
//! - `add_failed` items are the history items a rule picked and Transmission
//!   did not add (`추가 실패`), the newest 200; `다시 받기` is on the history
//!   item.
//!
//! Items come newest first, each kind by its own time, revisions first.

use std::{collections::HashMap, path::Path as FsPath};

use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;

use super::{
    artwork_api::image_url,
    commands_api::CommandView,
    in_place::Evidence,
    jobs_api::{covers_of, title_of, work_ref, WorkRefView},
    ApiError, AppState,
};
use trss_collect::{
    commands::receive_once,
    revision::season_episode,
    revisions::received_again_on_retry,
    store::{
        history::{HistoryQuery, HistoryResult},
        revisions::{Revision, RevisionState},
    },
};
use trss_jobs::{ItemState, Wait};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/todo", get(todos))
        .route("/todo/count", get(todo_count))
        .route("/todo/receive-failures", get(list))
}

#[derive(Debug, Serialize, PartialEq)]
pub struct WorkLink {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct FailureFile {
    /// `old` (the video being replaced) or `new` (the revision).
    pub role: &'static str,
    pub path: Option<String>,
    /// `kept`, `removed`, `received_name` or `not_received`.
    pub state: &'static str,
}

/// A failed replacement as an episode row and the to-do source show it.
#[derive(Debug, Serialize, PartialEq)]
pub struct RevisionFailure {
    pub at: i64,
    /// The revision's history item, which `다시 받기` receives.
    pub history_item_id: i64,
    pub reason: String,
    pub files: Vec<FailureFile>,
    #[serde(flatten)]
    pub retry: RetryOffer,
}

/// `다시 받기` on a failed replacement (see the module docs).
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct RetryOffer {
    pub can_retry: bool,
    pub retry_blocked: Option<String>,
    pub command: Option<CommandView>,
}

/// [`RetryOffer`]s of the failed replacements `rows`, by history item. A
/// row `다시 받기` does not receive again ([`received_again_on_retry`]) has
/// none.
pub async fn retry_offers(
    state: &AppState,
    rows: &[Revision],
) -> Result<HashMap<i64, RetryOffer>, ApiError> {
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let stopped: Vec<&Revision> = rows
        .iter()
        .filter(|row| received_again_on_retry(row))
        .collect();
    let mut offers = HashMap::new();
    if stopped.is_empty() {
        return Ok(offers);
    }
    let subjects = stopped.iter().map(|row| row.item_id.to_string()).collect();
    let open = state
        .commands
        .open_for_subjects(receive_once::KIND, subjects)
        .await
        .map_err(|e| internal(&e))?;
    let collect_folder = state
        .settings
        .collection()
        .await
        .map_err(|e| internal(&e))?
        .map(|collect| collect.folder);
    let mut evidence = Evidence::load(state).await?;
    for row in stopped {
        let Some(item) = state
            .history
            .get(row.item_id)
            .await
            .map_err(|e| internal(&e))?
        else {
            continue;
        };
        let command = open.get(&item.id.to_string()).map(CommandView::from);
        // The episode's place holds this revision or a higher one already:
        // told first, as it is what makes the button pointless whatever else
        // is the matter.
        if let Some(place) = evidence.held(&item, row).await? {
            offers.insert(
                item.id,
                RetryOffer {
                    can_retry: false,
                    retry_blocked: Some(place.message()),
                    command,
                },
            );
            continue;
        }
        let revision = receive_once::revision_retry(&state.revisions, item.id)
            .await
            .map_err(|e| internal(&e))?;
        let channel = state.channels.get_channel(&item.channel_id).await?;
        let rule = match &item.rule_id {
            Some(id) => state.channels.get_rule(id).await?,
            None => None,
        };
        let plan = receive_once::retry_plan_for(&item, channel.as_ref(), rule.as_ref(), &revision)
            .and_then(|plan| match &collect_folder {
                Some(folder) => {
                    receive_once::same_destination(&revision, FsPath::new(folder), plan.rule)?;
                    Ok(plan)
                }
                None => Ok(plan),
            });
        offers.insert(
            item.id,
            RetryOffer {
                can_retry: plan.is_ok(),
                retry_blocked: plan
                    .err()
                    .filter(|why| why.explains_missing_button())
                    .map(|why| why.message().to_owned()),
                command,
            },
        );
    }
    Ok(offers)
}

/// The files of a failed replacement, with paths relative to `base` when
/// they are inside it.
pub fn failure_of(row: &Revision, base: Option<&FsPath>) -> RevisionFailure {
    let path = |name: &str| {
        let full = FsPath::new(&row.folder).join(name);
        let shown = base
            .and_then(|base| full.strip_prefix(base).ok())
            .unwrap_or(&full);
        shown.to_string_lossy().into_owned()
    };
    let old = FailureFile {
        role: "old",
        path: Some(path(&row.episode_name)),
        // A replacement received again that has not got its video yet
        // removed the old one before.
        state: if matches!(row.state, RevisionState::Removed | RevisionState::Abandoned)
            || (row.not_received() && row.claimed_at.is_some())
        {
            "removed"
        } else {
            "kept"
        },
    };
    let new = match &row.received_name {
        Some(name) => FailureFile {
            role: "new",
            path: Some(path(name)),
            state: if row.state == RevisionState::Abandoned {
                "missing"
            } else {
                "received_name"
            },
        },
        None => FailureFile {
            role: "new",
            path: None,
            state: "not_received",
        },
    };
    RevisionFailure {
        at: row.updated_at,
        history_item_id: row.item_id,
        reason: row.reason.clone().unwrap_or_default(),
        files: vec![old, new],
        retry: RetryOffer::default(),
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum FailureItem {
    Revision {
        at: i64,
        history_item_id: i64,
        title: Option<String>,
        work: Option<WorkLink>,
        season: Option<u32>,
        episode: Option<String>,
        reason: String,
        files: Vec<FailureFile>,
        #[serde(flatten)]
        retry: Box<RetryOffer>,
    },
    AddFailed {
        at: i64,
        history_item_id: i64,
        title: String,
        reason: Option<String>,
    },
}

#[derive(Serialize)]
struct FailureList {
    items: Vec<FailureItem>,
}

/// The newest add failures listed.
const ADD_FAILURES: usize = 200;

async fn list(State(state): State<AppState>) -> Result<Json<FailureList>, ApiError> {
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let mut items = Vec::new();
    let rows = state.revisions.failures().await.map_err(|e| internal(&e))?;
    let mut offers = retry_offers(&state, &rows).await?;
    for row in rows {
        let work = state
            .revisions
            .work_at(row.folder.clone())
            .await
            .map_err(|e| internal(&e))?;
        let title = state
            .history
            .get(row.item_id)
            .await
            .map_err(|e| internal(&e))?
            .map(|item| item.title);
        let base = FsPath::new(&row.folder).parent();
        let failure = failure_of(&row, work.as_ref().and(base));
        let (season, episode) = season_episode(&row.episode_name).unzip();
        items.push(FailureItem::Revision {
            at: failure.at,
            history_item_id: row.item_id,
            title,
            work: work.map(|w| WorkLink {
                id: w.id,
                name: w.name,
            }),
            season,
            episode,
            reason: failure.reason,
            files: failure.files,
            retry: Box::new(offers.remove(&row.item_id).unwrap_or_default()),
        });
    }
    let failed = state
        .history
        .list(HistoryQuery {
            result: Some(HistoryResult::AddFailed),
            limit: ADD_FAILURES,
            ..Default::default()
        })
        .await
        .map_err(|e| internal(&e))?;
    items.extend(
        failed
            .items
            .into_iter()
            .filter(|item| item.rule_id.is_some())
            .map(|item| FailureItem::AddFailed {
                at: item.result_at,
                history_item_id: item.id,
                title: item.title,
                reason: item.reason,
            }),
    );
    Ok(Json(FailureList { items }))
}

// ---------------------------------------------------------------------------
// `처리 필요`

#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Todo {
    Auth {
        key: String,
        at: i64,
        work: Option<WorkRefView>,
        title: String,
        season: Option<i64>,
        episodes: Vec<String>,
        creator: Option<String>,
        reason: String,
        job_id: String,
        jobs: usize,
    },
    ReceiveFailed {
        key: String,
        at: i64,
        context: &'static str,
        work: Option<WorkRefView>,
        title: String,
        season: Option<u32>,
        episodes: Vec<String>,
        count: usize,
        reason: Option<String>,
        channel_id: Option<String>,
    },
}

impl Todo {
    fn at(&self) -> i64 {
        match self {
            Todo::Auth { at, .. } | Todo::ReceiveFailed { at, .. } => *at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TodoList {
    pub needs: Vec<Todo>,
    pub count: usize,
}

async fn todos(State(state): State<AppState>) -> Result<Json<TodoList>, ApiError> {
    Ok(Json(todo_list(&state).await?))
}

#[derive(Serialize)]
struct TodoCount {
    count: usize,
}

async fn todo_count(State(state): State<AppState>) -> Result<Json<TodoCount>, ApiError> {
    Ok(Json(TodoCount {
        count: todo_list(&state).await?.count,
    }))
}

/// The to-dos that need the person (see the module docs).
pub async fn todo_list(state: &AppState) -> Result<TodoList, ApiError> {
    let mut auth = auth_todos(state).await?;
    let mut failed = receive_failed_todos(state).await?;
    auth.sort_by_key(|t| std::cmp::Reverse(t.at()));
    failed.sort_by_key(|t| std::cmp::Reverse(t.at()));
    auth.extend(failed);
    Ok(TodoList {
        count: auth.len(),
        needs: auth,
    })
}

async fn auth_todos(state: &AppState) -> Result<Vec<Todo>, ApiError> {
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let waits = state.jobs.auth_waits().await.map_err(|e| internal(&e))?;
    let covers = covers_of(state, &waits).await?;
    // Oldest first, so each work's first job is its oldest.
    let mut groups: Vec<(String, Vec<&trss_jobs::store::JobRow>)> = Vec::new();
    for row in &waits {
        let key = format!("auth:{}", row.work_id.as_deref().unwrap_or(&row.id));
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, rows)) => rows.push(row),
            None => groups.push((key, vec![row])),
        }
    }
    let mut todos = Vec::new();
    for (key, rows) in groups {
        let oldest = rows[0];
        let (mut episodes, mut reason) = (Vec::new(), None);
        for row in &rows {
            for item in state.jobs.items(&row.id).await.map_err(|e| internal(&e))? {
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
            work: work_ref(oldest, &covers),
            title: title_of(oldest),
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

/// One `받기 실패` to-do as it is gathered.
struct FailedGroup {
    key: String,
    context: &'static str,
    at: i64,
    work: Option<WorkLink>,
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

async fn receive_failed_todos(state: &AppState) -> Result<Vec<Todo>, ApiError> {
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
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

    for row in state.revisions.failures().await.map_err(|e| internal(&e))? {
        let work = state
            .revisions
            .work_at(row.folder.clone())
            .await
            .map_err(|e| internal(&e))?;
        let folder_name = FsPath::new(&row.folder)
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
            work: work.map(|w| WorkLink {
                id: w.id,
                name: w.name,
            }),
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

    let failed = state
        .history
        .list(HistoryQuery {
            result: Some(HistoryResult::AddFailed),
            limit: ADD_FAILURES,
            ..Default::default()
        })
        .await
        .map_err(|e| internal(&e))?;
    let collect_folder = state
        .settings
        .collection()
        .await
        .map_err(|e| internal(&e))?
        .map(|collect| collect.folder);
    let mut rules = HashMap::new();
    for item in failed.items {
        let Some(rule_id) = item.rule_id.clone() else {
            continue;
        };
        if !rules.contains_key(&rule_id) {
            let rule = state.channels.get_rule(&rule_id).await?;
            let work = match (&rule, &collect_folder) {
                (Some(rule), Some(folder)) => state
                    .revisions
                    .work_at(
                        FsPath::new(folder)
                            .join(&rule.directory)
                            .to_string_lossy()
                            .into_owned(),
                    )
                    .await
                    .map_err(|e| internal(&e))?,
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
            work: work.as_ref().map(|w| WorkLink {
                id: w.id.clone(),
                name: w.name.clone(),
            }),
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

    let ids: Vec<String> = groups
        .iter()
        .filter_map(|g| g.work.as_ref().map(|w| w.id.clone()))
        .collect();
    let covers = match ids.is_empty() {
        true => HashMap::new(),
        false => state
            .artwork
            .store
            .image_ids_of(ids)
            .await
            .map_err(|e| internal(&e))?,
    };
    Ok(groups
        .into_iter()
        .map(|g| Todo::ReceiveFailed {
            key: g.key,
            at: g.at,
            context: g.context,
            work: g.work.map(|w| WorkRefView {
                cover_url: covers.get(&w.id).map(|image| image_url(&w.id, image)),
                id: w.id,
                name: w.name,
            }),
            title: g.title,
            season: g.season,
            episodes: g.episodes,
            count: g.count,
            reason: g.reason,
            channel_id: g.channel_id,
        })
        .collect())
}
