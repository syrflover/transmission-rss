//! `GET /api/todo/receive-failures`: the source of the to-do kind `받기 실패`
//! (`docs/specs/jobs.md`, 할 일): collection failures the person has to deal
//! with. The to-do screen itself does not exist yet; this is what it will
//! read, and the work detail shows the replacement failures on their episode
//! rows ([`super::library_work_api`]).
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
//!   ([`crate::store::revisions::Revision::is_failure`]). `files` are the two
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
//!   replacement was decided for ([`receive_once::same_destination`]). `retry_blocked` says why it is
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

use super::{commands_api::CommandView, ApiError, AppState};
use crate::{
    revision::season_episode,
    store::{
        history::{HistoryQuery, HistoryResult},
        revisions::{Revision, RevisionState},
    },
    worker::{commands::receive_once, revisions::received_again_on_retry},
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/todo/receive-failures", get(list))
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
    pub retry_blocked: Option<&'static str>,
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
    for row in stopped {
        let Some(item) = state
            .history
            .get(row.item_id)
            .await
            .map_err(|e| internal(&e))?
        else {
            continue;
        };
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
                    .map(|why| why.message()),
                command: open.get(&item.id.to_string()).map(CommandView::from),
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
