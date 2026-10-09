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
//!     "reason": "CAPTCHA", "job_id": "…", "jobs": 1, "badge": "auth" },
//!   { "kind": "receive_failed", "key": "revision:<work id>", "at": 1759990000000,
//!     "context": "revision", "work": { … }, "title": "Show", "season": 1,
//!     "episodes": ["14"], "count": 1, "reason": "…", "channel_id": null,
//!     "badge": "receive_failed" } ],
//!   "count": 2,
//!   "badges": { "<work id>": ["auth", "receive_failed"] } }
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
//! - `episode_check` (`회차 확인 필요`): a subscribed creator's source whose
//!   episode mapping the app could not decide (`reason` says why; the
//!   mapping is `undecided`), or whose newly seen `episodes` fit the mapping
//!   no way and have no exception of the user's ([`trss_jobs::mapping`]). One
//!   to-do per work, derived at each read from the stored mappings and
//!   conflicts, so it is gone once the user's mapping is saved or the
//!   exceptions cover the conflicting episodes. It names the lowest season
//!   that needs it (`season`), its creator and `source_id`, which the work
//!   detail's candidates open (`/library/<work>?season=<n>&section=candidates&source=<id>`);
//!   `sources` counts the sources of the work that need the user, and `at` is
//!   since when the oldest waits.
//!
//! - `placement_check` (`회차 확인 필요` too): a subtitle job waiting for a
//!   person to say which episode its files are (the job's 배치 확인): one
//!   to-do per job, which opens the job's detail (`job_id`). `files` are the
//!   names of the files it asks about (an upload's or a find job's subtitles,
//!   which its table places as a whole, else the held ones), `reason` the
//!   first one's question, or, for a whole table, what the job waits for (its
//!   note). `origin` is how the job came to be (`pick`, `auto`, `upload`,
//!   `find`, `relocate`) and `source` its posts' host, its source as the work
//!   detail's card says it.
//! - `video_check` (`회차 확인 필요` too): a video directly in a season folder
//!   other than `Season 00` whose name gives no episode (`no_episode`) or
//!   another season's (`season_mismatch`), which the app neither puts on an
//!   episode nor counts missing ([`trss_library::store::library::VideoCheck`]).
//!   One to-do per video, until the person says `확인함`
//!   ([`super::video_check_api`]) or the video is renamed or moved; another
//!   video put at the path is asked about again. `path` is the season folder
//!   and the file name (the card's title in the work detail), `season` the
//!   folder's, `reason` why as a sentence, `at` the video's modification
//!   time, and `seen` the video as the scan saw it, which `확인함` sends back.
//!   It opens the work detail's `파일` card (`/library/<work>#files`).
//! - `replacement` (`교체 승인`): the subtitle jobs waiting for a person to
//!   approve or refuse replacing an episode's subtitle
//!   ([`trss_jobs::place::replace`]), one to-do per work (per job when it
//!   has no work). `at` is since when its oldest job waits, `job_id` that job
//!   (its detail is where the person compares, `비교`), `episodes` the
//!   episodes whose plan waits, `jobs` how many jobs wait, and `changes`,
//!   what the open plans change, summed: `added`, `changed` and `removed`
//!   dialogue lines, `timing` lines, `styles` (added, removed and changed),
//!   `fonts` (added and removed) and `uncompared`, how many open plans have
//!   no comparison of their contents (made before the app compared them, or
//!   not readable), which the other numbers leave out, `partial`, how many
//!   compared plans left a part out (a language of the dialogue with no
//!   counterpart, or the styles and fonts of an ASS set against another
//!   format), and `plans`, how many open plans were summed (`uncompared` and
//!   `partial` of them among them). `current_received_at` and
//!   `new_received_at` are when the open plans' current and new subtitles
//!   were received, the newest of each (the work detail's card line);
//!   `current_changed_at` is, when no current subtitle is one the app
//!   manages, the newest change time of those files, else `null`.
//!
//! The to-dos are gathered, summed and ordered by [`trss_jobs::todo`]; this
//! module only answers with them.
//!
//! Each work's kinds of to-do are also the badges of the library list
//! ([`trss_jobs::todo::badges_by_work`], [`super::library_api`]). The answer
//! says them so the screen does not work them out: each to-do has a `badge`
//! (`auth`, `receive_failed`, `replacement` or `episode_check`, which
//! `episode_check`, `placement_check` and `video_check` all are), and `badges`
//! maps each work's ID to its to-dos' badges, once each, in the list's order
//! (`{ "badges": { "<work id>": ["receive_failed", "episode_check"] } }`);
//! a to-do of no work has none.
//!
//! `auth` comes before `receive_failed`, that before `replacement`, and that
//! before `episode_check`, `placement_check` and `video_check`, each newest
//! first. Failed subtitle
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
//!
//! `GET /api/todo/subtitle-follow`: the source of the suggestion kind
//! `자막 구독`, one per work whose subscription gets subtitles with no creator
//! chosen yet (`제작자 미정`) while its anime has candidates
//! ([`trss_jobs::follow`]). It names no creator: the user picks one in the work
//! detail's 자막 후보. Like every suggestion, it is not counted in `count`.
//!
//! ```json
//! { "suggestions": [
//!   { "work": { "id": "…", "name": "Show", "cover_url": "…" }, "title": "작품",
//!     "season": 1, "rule_id": "…", "anime_no": 3441,
//!     "episodes": ["1", "2", "3", "4"], "creators": 2,
//!     "since": 1760000000000 } ] }
//! ```
//!
//! `title` is the anime's Anissia title, else the work's name; `episodes` are
//! the candidates' episodes as Anissia writes them, once each in the order
//! first seen; `since` is when the first candidate was seen.

use std::{collections::HashMap, path::Path as FsPath};

use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;

use super::{
    artwork_api::image_url,
    commands_api::CommandView,
    in_place::{Blocked, Evidence},
    jobs_api::WorkRefView,
    ApiError, AppState,
};
use trss_collect::{
    commands::receive_once,
    revisions::received_again_on_retry,
    store::{
        history::{HistoryQuery, HistoryResult},
        revisions::{Revision, RevisionState},
    },
};
use trss_core::trname_names::season_episode;
use trss_jobs::todo::{Sources, TodoError, TodoList, ADD_FAILURES};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/todo", get(todos))
        .route("/todo/count", get(todo_count))
        .route("/todo/receive-failures", get(list))
        .route("/todo/subtitle-follow", get(follow_suggestions))
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
        let blocked = evidence.retry_check(&item).await?;
        offers.insert(
            item.id,
            RetryOffer {
                can_retry: blocked.is_ok(),
                retry_blocked: blocked.err().and_then(|why| match why {
                    // Told whatever else is the matter.
                    Blocked::InPlace(_) => Some(why.message()),
                    Blocked::Refused(refused) => {
                        refused.explains_missing_button().then(|| why.message())
                    }
                }),
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
// `자막 구독`

#[derive(Debug, Serialize, PartialEq)]
struct FollowSuggestionView {
    work: WorkRefView,
    title: String,
    season: u32,
    rule_id: String,
    anime_no: i64,
    episodes: Vec<String>,
    creators: usize,
    since: i64,
}

#[derive(Debug, Serialize)]
struct FollowSuggestions {
    suggestions: Vec<FollowSuggestionView>,
}

async fn follow_suggestions(
    State(state): State<AppState>,
) -> Result<Json<FollowSuggestions>, ApiError> {
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let found = state.follow.suggestions().await.map_err(|e| internal(&e))?;
    let ids: Vec<String> = found.iter().map(|s| s.work_id.clone()).collect();
    let covers = match ids.is_empty() {
        true => HashMap::new(),
        false => state
            .artwork
            .store
            .image_ids_of(ids)
            .await
            .map_err(|e| internal(&e))?,
    };
    Ok(Json(FollowSuggestions {
        suggestions: found
            .into_iter()
            .map(|s| FollowSuggestionView {
                title: s.anime_title.unwrap_or_else(|| s.work_name.clone()),
                work: WorkRefView {
                    cover_url: covers
                        .get(&s.work_id)
                        .map(|image| image_url(&s.work_id, image)),
                    id: s.work_id,
                    name: s.work_name,
                },
                season: s.season,
                rule_id: s.rule_id,
                anime_no: s.anime_no,
                episodes: s.episodes,
                creators: s.creators,
                since: s.since,
            })
            .collect(),
    }))
}

// ---------------------------------------------------------------------------
// `처리 필요`

/// The to-dos that need the person (see the module docs), gathered and put in
/// order by [`trss_jobs::todo`].
pub async fn todo_list(state: &AppState) -> Result<TodoList, ApiError> {
    Ok(trss_jobs::todo::list(&Sources {
        jobs: &state.jobs,
        place: &state.place,
        follow: &state.follow,
        library: &state.library,
        artwork: &state.artwork.store,
        revisions: &state.revisions,
        history: &state.history,
        channels: &state.channels,
        settings: &state.settings,
        cover_url: image_url,
    })
    .await?)
}

impl From<TodoError> for ApiError {
    fn from(e: TodoError) -> Self {
        match e {
            TodoError::Channel(e) => e.into(),
            TodoError::Read(message) => ApiError::Internal(message),
        }
    }
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

#[cfg(test)]
mod tests;
