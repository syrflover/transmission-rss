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
//!         "state": "received_name" } ] },
//!   { "kind": "add_failed", "at": 1759990000000, "history_item_id": 9,
//!     "title": "…", "reason": "Transmission에 연결하지 못했어요: …" } ] }
//! ```
//!
//! - `revision` items are replacements of video revisions that failed, or
//!   whose rename after the old video was removed has not gone through yet
//!   ([`crate::store::revisions::Revision::is_failure`]). `files` are the two
//!   videos with what became of each: the old one `kept` (still there) or
//!   `removed`; the new one under the name it was received with
//!   (`received_name`), or `not_received` (its download did not complete or
//!   is not in the rule's folder, so `path` is `null`). Paths are relative to
//!   the work folder; `work` is `null` when the library has no work at that
//!   folder, and the paths are then absolute. An item goes away once the
//!   worker sees one of the two files gone, or the rename go through; a
//!   `not_received` one also once its torrent is right again or a cycle
//!   receives its item again.
//! - `add_failed` items are the history items a rule picked and Transmission
//!   did not add (`추가 실패`), the newest 200; `다시 받기` is on the history
//!   item.
//!
//! Items come newest first, each kind by its own time, revisions first.

use std::path::Path as FsPath;

use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;

use super::{ApiError, AppState};
use crate::{
    revision::season_episode,
    store::{
        history::{HistoryQuery, HistoryResult},
        revisions::{Revision, RevisionState},
    },
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
    pub reason: String,
    pub files: Vec<FailureFile>,
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
        state: if row.state == RevisionState::Removed {
            "removed"
        } else {
            "kept"
        },
    };
    let new = match &row.received_name {
        Some(name) => FailureFile {
            role: "new",
            path: Some(path(name)),
            state: "received_name",
        },
        None => FailureFile {
            role: "new",
            path: None,
            state: "not_received",
        },
    };
    RevisionFailure {
        at: row.updated_at,
        reason: row.reason.clone().unwrap_or_default(),
        files: vec![old, new],
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
    for row in state.revisions.failures().await.map_err(|e| internal(&e))? {
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
