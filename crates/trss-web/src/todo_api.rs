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
//! Each work's kinds of to-do are also the badges of the library list
//! ([`badges_by_work`], [`super::library_api`]).
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
use trss_jobs::{
    model::PlanState,
    place::replace::records::{Compared, Comparison, Format, Item, PlanView},
    ItemState, Wait,
};

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
    EpisodeCheck {
        key: String,
        at: i64,
        work: Option<WorkRefView>,
        title: String,
        season: u32,
        creator: String,
        source_id: String,
        episodes: Vec<String>,
        reason: Option<String>,
        sources: usize,
    },
    PlacementCheck {
        key: String,
        at: i64,
        work: Option<WorkRefView>,
        title: String,
        season: Option<i64>,
        creator: Option<String>,
        origin: String,
        source: Option<String>,
        files: Vec<String>,
        reason: Option<String>,
        job_id: String,
    },
    VideoCheck {
        key: String,
        at: i64,
        work: Option<WorkRefView>,
        title: String,
        season: u32,
        path: String,
        reason: String,
        seen: String,
    },
    Replacement {
        key: String,
        at: i64,
        work: Option<WorkRefView>,
        title: String,
        season: Option<i64>,
        episodes: Vec<i64>,
        creator: Option<String>,
        job_id: String,
        jobs: usize,
        changes: Changes,
        current_received_at: Option<i64>,
        current_changed_at: Option<i64>,
        new_received_at: Option<i64>,
    },
}

/// What the open plans of a `교체 승인` to-do change, summed: the card's
/// reason line.
#[derive(Debug, Default, Serialize, PartialEq)]
pub struct Changes {
    /// Dialogue lines.
    added: u64,
    changed: u64,
    removed: u64,
    /// Lines whose timing moved.
    timing: u64,
    /// Styles added, removed and changed.
    styles: u64,
    /// Fonts added and removed.
    fonts: u64,
    /// Plans whose contents were not compared (made before the app compared
    /// them, or not readable): what they change is not in the numbers above.
    uncompared: u64,
    /// Compared plans that left a part out: a language of the dialogue with
    /// no counterpart, or the styles and fonts of an ASS set against another
    /// format. Two files with no styles at all leave nothing out.
    partial: u64,
    /// The open plans summed, `uncompared` and `partial` of them among them.
    plans: u64,
}

impl Changes {
    fn add(&mut self, comparison: Option<&Comparison>) {
        self.plans += 1;
        let Some(Comparison {
            result: Compared::Diff(diff),
            ..
        }) = comparison
        else {
            self.uncompared += 1;
            return;
        };
        self.added += diff.dialogue.added;
        self.changed += diff.dialogue.changed;
        self.removed += diff.dialogue.removed;
        self.timing += diff.timing.count;
        if let Some(styles) = &diff.styles {
            self.styles +=
                (styles.added.len() + styles.removed.len() + styles.changed.len()) as u64;
        }
        if let Some(fonts) = &diff.fonts {
            self.fonts += (fonts.added.len() + fonts.removed.len()) as u64;
        }
        let ass = diff.old.format == Format::Ass || diff.new.format == Format::Ass;
        let left_out = diff.not_compared.iter().any(|n| n.item == Item::Dialogue)
            || (ass && (diff.styles.is_none() || diff.fonts.is_none()));
        if left_out {
            self.partial += 1;
        }
    }
}

/// When the subtitles of a `교체 승인` to-do's open plans were received: the
/// `현재`·`새 자막` line of its card in the work detail. Of several plans, the
/// newest of each.
#[derive(Debug, Default)]
struct Received {
    /// The current subtitle the app manages: when it was received.
    current: Option<i64>,
    /// A current file the app did not manage: its change time.
    current_changed: Option<i64>,
    new: Option<i64>,
}

impl Received {
    fn add(&mut self, view: &PlanView) {
        if let Some((path, file)) = view.plan.current() {
            match view.applied.iter().find(|(p, _)| *p == path.path) {
                Some((_, facts)) => self.current = self.current.max(Some(facts.received_at)),
                None => {
                    self.current_changed = self.current_changed.max(Some(file.mtime / 1_000_000))
                }
            }
        }
        if let Some(new) = &view.new {
            self.new = self.new.max(Some(new.received_at));
        }
    }
}

impl Todo {
    fn at(&self) -> i64 {
        match self {
            Todo::Auth { at, .. }
            | Todo::ReceiveFailed { at, .. }
            | Todo::EpisodeCheck { at, .. }
            | Todo::PlacementCheck { at, .. }
            | Todo::VideoCheck { at, .. }
            | Todo::Replacement { at, .. } => *at,
        }
    }

    /// The library's work it is about, when it has one.
    fn work_id(&self) -> Option<&str> {
        match self {
            Todo::Auth { work, .. }
            | Todo::ReceiveFailed { work, .. }
            | Todo::EpisodeCheck { work, .. }
            | Todo::PlacementCheck { work, .. }
            | Todo::VideoCheck { work, .. }
            | Todo::Replacement { work, .. } => work.as_ref().map(|w| w.id.as_str()),
        }
    }

    /// Its kind as a badge names it: a job's 배치 확인 and a video's episode
    /// are a `회차 확인 필요` like a mapping's.
    fn badge(&self) -> &'static str {
        match self {
            Todo::Auth { .. } => "auth",
            Todo::ReceiveFailed { .. } => "receive_failed",
            Todo::Replacement { .. } => "replacement",
            Todo::EpisodeCheck { .. } | Todo::PlacementCheck { .. } | Todo::VideoCheck { .. } => {
                "episode_check"
            }
        }
    }
}

/// Each work's to-do badges (the library grid's): the kinds of its to-dos,
/// once each, in the order of `todos` ([`todo_list`]'s: the red kinds first).
/// A to-do of no work has none.
pub fn badges_by_work(todos: &[Todo]) -> HashMap<String, Vec<&'static str>> {
    let mut badges: HashMap<String, Vec<&'static str>> = HashMap::new();
    for todo in todos {
        let Some(work) = todo.work_id() else {
            continue;
        };
        let kinds = badges.entry(work.to_owned()).or_default();
        if !kinds.contains(&todo.badge()) {
            kinds.push(todo.badge());
        }
    }
    badges
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
    let mut replacements = replacement_todos(state).await?;
    // `회차 확인 필요` is a question on top of the others: when it cannot be
    // read, the rest of the list (and the badge) still answers.
    let mut checks = match episode_check_todos(state).await {
        Ok(checks) => checks,
        Err(e) => {
            eprintln!("Cannot read the 회차 확인 필요 to-dos: {e:?}");
            Vec::new()
        }
    };
    checks.extend(placement_check_todos(state).await?);
    // So is a video's: the library's read failing leaves the rest answering.
    match video_check_todos(state).await {
        Ok(videos) => checks.extend(videos),
        Err(e) => eprintln!("Cannot read the videos' 회차 확인 필요 to-dos: {e:?}"),
    }
    auth.sort_by_key(|t| std::cmp::Reverse(t.at()));
    failed.sort_by_key(|t| std::cmp::Reverse(t.at()));
    replacements.sort_by_key(|t| std::cmp::Reverse(t.at()));
    checks.sort_by_key(|t| std::cmp::Reverse(t.at()));
    auth.extend(failed);
    auth.extend(replacements);
    auth.extend(checks);
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

/// The `교체 승인` to-dos: one per work whose jobs wait for a person to
/// approve or refuse replacing an episode's subtitle (per job when it has no
/// work), which opens its oldest job's detail.
async fn replacement_todos(state: &AppState) -> Result<Vec<Todo>, ApiError> {
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let waits = state
        .jobs
        .approval_waits()
        .await
        .map_err(|e| internal(&e))?;
    let covers = covers_of(state, &waits).await?;
    // Oldest first, so each work's first job is its oldest.
    let mut groups: Vec<(String, Vec<&trss_jobs::store::JobRow>)> = Vec::new();
    for row in &waits {
        let key = format!("replacement:{}", row.work_id.as_deref().unwrap_or(&row.id));
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, rows)) => rows.push(row),
            None => groups.push((key, vec![row])),
        }
    }
    let mut todos = Vec::new();
    for (key, rows) in groups {
        let oldest = rows[0];
        let mut episodes = Vec::new();
        let mut changes = Changes::default();
        let mut received = Received::default();
        for row in &rows {
            let plans = state
                .jobs
                .replacements(&row.id)
                .await
                .map_err(|e| internal(&e))?;
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
            work: work_ref(oldest, &covers),
            title: title_of(oldest),
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
async fn placement_check_todos(state: &AppState) -> Result<Vec<Todo>, ApiError> {
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let waits = state
        .jobs
        .placement_waits()
        .await
        .map_err(|e| internal(&e))?;
    let covers = covers_of(state, &waits).await?;
    let mut todos = Vec::new();
    for row in &waits {
        // An upload's or a find job's table is confirmed as a whole: its
        // note says what it waits for, not one file's question.
        let Some((asked, whole)) = state
            .jobs
            .placeable(&row.id)
            .await
            .map_err(|e| internal(&e))?
        else {
            continue;
        };
        todos.push(Todo::PlacementCheck {
            key: format!("placement:{}", row.id),
            at: row.state_at,
            work: work_ref(row, &covers),
            title: title_of(row),
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
/// ([`trss_jobs::Follow::episode_checks`]). Derived at each read, so it is gone
/// as soon as the user's mapping or exceptions cover what it asked about.
async fn episode_check_todos(state: &AppState) -> Result<Vec<Todo>, ApiError> {
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let checks = state
        .follow
        .episode_checks()
        .await
        .map_err(|e| internal(&e))?;
    // One to-do per work: the lowest season's check names the creator and the
    // episodes; it has been waiting since the oldest of the work's checks.
    let mut groups: Vec<(trss_jobs::follow::EpisodeCheck, usize, i64)> = Vec::new();
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
    let ids: Vec<String> = groups.iter().map(|(g, ..)| g.work_id.clone()).collect();
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
        .map(|(check, sources, since)| Todo::EpisodeCheck {
            key: format!("episode:{}", check.work_id),
            at: since,
            title: check.anime_title.unwrap_or_else(|| check.work_name.clone()),
            work: Some(WorkRefView {
                cover_url: covers
                    .get(&check.work_id)
                    .map(|image| image_url(&check.work_id, image)),
                id: check.work_id,
                name: check.work_name,
            }),
            season: check.season,
            creator: check.creator,
            source_id: check.source_id,
            episodes: check.episodes,
            reason: check.undecided,
            sources,
        })
        .collect())
}

/// The `회차 확인 필요` to-dos of videos: one per video of a season folder
/// whose name gives no episode of it, until a person checks it or it is
/// renamed or moved ([`trss_library::store::library::LibraryStore::video_checks`]).
async fn video_check_todos(state: &AppState) -> Result<Vec<Todo>, ApiError> {
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let checks = state
        .library
        .video_checks()
        .await
        .map_err(|e| internal(&e))?;
    let mut ids: Vec<String> = checks.iter().map(|c| c.work_id.clone()).collect();
    ids.dedup();
    let covers = match ids.is_empty() {
        true => HashMap::new(),
        false => state
            .artwork
            .store
            .image_ids_of(ids)
            .await
            .map_err(|e| internal(&e))?,
    };
    Ok(checks
        .into_iter()
        .map(|check| Todo::VideoCheck {
            key: format!("video:{}:{}", check.work_id, check.path),
            at: check.identity.mtime_ns.div_euclid(1_000_000),
            title: check.work_name.clone(),
            work: Some(WorkRefView {
                cover_url: covers
                    .get(&check.work_id)
                    .map(|image| image_url(&check.work_id, image)),
                id: check.work_id,
                name: check.work_name,
            }),
            season: check.season,
            reason: check.reason.message().to_owned(),
            seen: super::video_check_api::seen(check.identity),
            path: check.path,
        })
        .collect())
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

#[cfg(test)]
mod tests;
