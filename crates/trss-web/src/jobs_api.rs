//! `/api/subtitle-jobs`: the subtitle jobs (`docs/specs/jobs.md`, 할 일 화면
//! and 작업 상세). The web makes them; the worker carries them out
//! ([`trss_jobs`]).
//!
//! - `POST /api/subtitle-jobs` `{ "id", "work_id", "season", "candidates": [<observation id>] }`
//!   makes a job of the candidates a person picked for a season. `id` is made
//!   by the browser for the action: a repeat with the same content answers
//!   `200` with the job it made, other content under the same ID `409`. An ID
//!   that starts with `auto:` or `recheck:` is the app's own (the subscribed
//!   creator's receipts, [`trss_jobs::follow`], and the revisions the
//!   recheck makes, [`trss_jobs::recheck`]) and is refused with `400`. A new
//!   job answers `202` `{ "id" }` and wakes the worker. The candidates must be
//!   the season's Anissia anime's ([`trss_collect::store::anissia::AnissiaStore::candidates`])
//!   and of one creator; the job copies their posts and episodes as they are.
//! - `GET /api/subtitle-jobs` the jobs by group: `failed` (`failed`,
//!   `partial`, newest first), `waiting` (`waiting` for a person's check,
//!   then for a source, then `held`, then `pending`, each oldest first),
//!   `running`, and the first five of `done` as a page.
//! - `GET /api/subtitle-jobs/done?after=&limit=` the next page of `done`
//!   (`limit` up to 50, 20 by default).
//! - `GET /api/subtitle-jobs/{id}` one job with its steps, items with their
//!   files, its folder in the receive area, its log, newest first, and its
//!   remote screen (`screen`, see [`super::screen_api`]; `null` for none). It
//!   reads only: it never asks for a browser run.
//! - `POST /api/subtitle-jobs/{id}/screen` and the screen's socket: see
//!   [`super::screen_api`].
//! - `POST /api/subtitle-jobs/find` `{ "id", "work_id", "season", "creator" }`
//!   makes a find job (직접 찾기): the server browser opens the most recently
//!   observed post of `creator` (a source ID of the season's Anissia anime's
//!   candidates) for a person to browse on the job's remote screen, and keeps
//!   what they download there. `id` works as above (`202`, `200`, `409`,
//!   `400` for the app's own). A season with no Anissia anime is refused with
//!   `400`, a creator with no candidate of the season too.
//! - `POST /api/subtitle-jobs/{id}/finish` a person finishes a find job
//!   (`받기 끝내기`): the request is written and the worker woken, which ends
//!   the job once no download of its run is on its way, and at once when no
//!   run is bound, after taking what a run left in its folder. `200`
//!   `{ "state": "finishing" }` (`finishing` on the job until it ended), or
//!   `{ "state": "done" }` for a job that ended before. `400` for a job that
//!   is no find job, `404` for no job.
//!
//! A failed job, item and file carry their failure's class as `failure`
//! (`missing`, `expired`, `not_a_file`, `changed`, `network`; an item also
//! `no_subtitle` and `needs_input`; the screens name them), a job the class of
//! its first failed item. A file carries the format
//! its bytes were checked to be (`zip`, `ass`, `srt`, `smi`, `other`) and the
//! answer's status, media type and, for a failure, size.
//!
//! An item of a revision job whose files are the same bytes as the earlier
//! receipt's has `unchanged_from`, that receipt's job: there is nothing to
//! replace.
//!
//! A job's `origin` is `pick` (a person picked its candidates), `auto` (the
//! subscribed creator's episode, made by the app, [`trss_jobs::follow`]) or
//! `upload` (the subtitles and fonts a person uploaded, already `done`,
//! [`super::subtitle_upload_api`]). An upload job has `upload` (what it kept
//! by kind, and how many files it dropped), no episodes, only the steps it
//! went through (`receive`), its package's files with their `kind`
//! (`subtitle`, `font`, `archive`) and `dropped` (the names and reasons of the
//! files it did not keep) in its detail. A find job (`find`, 직접 찾기) has the
//! same as it receives, its steps `open` and `receive` as it reached them,
//! and ends `done` with the note `받은 파일 없음` when it kept nothing; a job
//! that receives a revision of a subtitle received before has `revision_of`
//! (that observation) and `revises_job` (the latest job that received it, or
//! `null`), both `null` otherwise. `revises_attributed` is `true` on the job
//! of a line of the subscribed creator that revises a subtitle file whose
//! creator the user named: nothing of it was received before, so the other two
//! stay `null`.
//!
//! A job's `title` is its anime's Anissia title, else its work's name. No
//! answer carries a cookie, a token or a signed address: posts are public
//! pages, and the files are named by their place in the receive area.

use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use trss_jobs::{
    store::{DonePage, FileRow, JobDetail, JobRow, StepRow},
    AskedFinish, Created, FileState, ItemState, JobState, NewFind, NewItem, NewJob, StepKind, Wait,
    FIND, UPLOAD,
};

use super::{artwork_api::image_url, commands_api::now_millis, ApiError, AppState};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/subtitle-jobs", get(groups).post(create))
        .route("/subtitle-jobs/done", get(done))
        .route("/subtitle-jobs/{id}", get(detail))
        .route("/subtitle-jobs/find", post(create_find))
        .route("/subtitle-jobs/{id}/finish", post(finish))
}

/// How many done jobs the groups carry.
const DONE_FIRST: usize = 5;
const DONE_PAGE: usize = 20;
const DONE_PAGE_MAX: usize = 50;
/// The most candidates one job takes.
const MAX_CANDIDATES: usize = 200;

fn internal(e: &dyn std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkRefView {
    pub id: String,
    pub name: String,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct ProgressView {
    pub done: usize,
    pub failed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct JobRowView {
    pub id: String,
    /// `pick` (a person picked the candidates) or `auto` (the subscribed
    /// creator's, received without a pick).
    pub origin: String,
    /// For a revision of a received subtitle: the observation received before
    /// and the latest job that received it.
    pub revision_of: Option<i64>,
    pub revises_job: Option<String>,
    /// A revision of a subtitle file whose creator the user named (no earlier
    /// receipt exists, so `revision_of` and `revises_job` are `null`).
    pub revises_attributed: bool,
    pub state: &'static str,
    pub wait: Option<&'static str>,
    pub stage: Option<&'static str>,
    pub note: Option<String>,
    pub state_at: i64,
    pub created_at: i64,
    pub work: Option<WorkRefView>,
    pub title: String,
    pub season: Option<i64>,
    pub episodes: Vec<String>,
    pub creator: Option<String>,
    pub source: Option<String>,
    pub progress: ProgressView,
    pub failure: Option<&'static str>,
    /// For an upload or a find job: what it kept and dropped.
    pub upload: Option<UploadView>,
    /// A find job a person finished, which the worker ends once no download
    /// of its run is on its way.
    pub finishing: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct UploadView {
    pub subtitles: usize,
    pub fonts: usize,
    pub archives: usize,
    pub dropped: usize,
}

/// The work a job is about, while the library has it, with its cover.
pub fn work_ref(row: &JobRow, covers: &HashMap<String, String>) -> Option<WorkRefView> {
    let (id, name) = (row.work_id.as_ref()?, row.work_name.as_ref()?);
    Some(WorkRefView {
        id: id.clone(),
        name: name.clone(),
        cover_url: covers.get(id).map(|image| image_url(id, image)),
    })
}

/// What to call a job: its anime's title, else its work's name.
pub fn title_of(row: &JobRow) -> String {
    row.anime_title
        .clone()
        .or_else(|| row.work_name.clone())
        .unwrap_or_else(|| "작품을 찾지 못한 작업".to_owned())
}

fn view(row: &JobRow, covers: &HashMap<String, String>) -> JobRowView {
    JobRowView {
        id: row.id.clone(),
        origin: row.origin.clone(),
        revision_of: row.revision_of,
        revises_job: row.revises_job.clone(),
        revises_attributed: row.revises_attributed,
        state: row.state.code(),
        wait: row.wait.map(Wait::code),
        stage: row.stage.map(StepKind::code),
        note: row.note.clone(),
        state_at: row.state_at,
        created_at: row.created_at,
        work: work_ref(row, covers),
        title: title_of(row),
        season: row.season,
        episodes: row.episodes.clone(),
        creator: row.creator.clone(),
        source: row.source.clone(),
        progress: ProgressView {
            done: row.progress.done,
            failed: row.progress.failed,
            total: row.progress.total,
        },
        failure: row.failure.map(|f| f.code()),
        upload: row.upload.map(|u| UploadView {
            subtitles: u.subtitles,
            fonts: u.fonts,
            archives: u.archives,
            dropped: u.dropped,
        }),
        finishing: row.finishing,
    }
}

/// The cover image IDs of the works `rows` name.
pub async fn covers_of<'a>(
    state: &AppState,
    rows: impl IntoIterator<Item = &'a JobRow>,
) -> Result<HashMap<String, String>, ApiError> {
    let mut ids: Vec<String> = rows.into_iter().filter_map(|r| r.work_id.clone()).collect();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    state
        .artwork
        .store
        .image_ids_of(ids)
        .await
        .map_err(|e| internal(&e))
}

#[derive(Debug, Serialize)]
struct DonePageView {
    items: Vec<JobRowView>,
    next: Option<String>,
    total: usize,
}

async fn page_view(state: &AppState, page: DonePage) -> Result<DonePageView, ApiError> {
    let covers = covers_of(state, &page.items).await?;
    Ok(DonePageView {
        items: page.items.iter().map(|r| view(r, &covers)).collect(),
        next: page.next,
        total: page.total,
    })
}

#[derive(Debug, Serialize)]
struct Groups {
    failed: Vec<JobRowView>,
    waiting: Vec<JobRowView>,
    running: Vec<JobRowView>,
    done: DonePageView,
}

/// Where a not-done job sits in `waiting`: a person's check first, then the
/// other waits (a find job, which waits for a person's browsing, among them),
/// held, then pending.
fn waiting_rank(row: &JobRow) -> Option<u8> {
    match (row.state, row.wait) {
        (JobState::Waiting, Some(Wait::Auth)) if row.origin != FIND => Some(0),
        (JobState::Waiting, _) => Some(1),
        (JobState::Held, _) => Some(2),
        (JobState::Pending, _) => Some(3),
        _ => None,
    }
}

async fn groups(State(state): State<AppState>) -> Result<Json<Groups>, ApiError> {
    let open = state.jobs.open_jobs().await.map_err(|e| internal(&e))?;
    let covers = covers_of(&state, &open).await?;
    let mut failed: Vec<&JobRow> = open
        .iter()
        .filter(|r| matches!(r.state, JobState::Failed | JobState::Partial))
        .collect();
    failed.sort_by_key(|r| std::cmp::Reverse((r.state_at, r.seq)));
    let mut waiting: Vec<(u8, &JobRow)> = open
        .iter()
        .filter_map(|r| waiting_rank(r).map(|rank| (rank, r)))
        .collect();
    waiting.sort_by_key(|(rank, r)| (*rank, r.seq));
    let running = open.iter().filter(|r| r.state == JobState::Running);

    let done = state
        .jobs
        .done_page(None, DONE_FIRST)
        .await
        .map_err(|e| internal(&e))?;
    Ok(Json(Groups {
        failed: failed.into_iter().map(|r| view(r, &covers)).collect(),
        waiting: waiting.into_iter().map(|(_, r)| view(r, &covers)).collect(),
        running: running.map(|r| view(r, &covers)).collect(),
        done: page_view(&state, done).await?,
    }))
}

#[derive(Debug, Deserialize)]
struct DoneQuery {
    after: Option<String>,
    limit: Option<usize>,
}

async fn done(
    State(state): State<AppState>,
    Query(query): Query<DoneQuery>,
) -> Result<Json<DonePageView>, ApiError> {
    let limit = query.limit.unwrap_or(DONE_PAGE).clamp(1, DONE_PAGE_MAX);
    let page = state
        .jobs
        .done_page(query.after, limit)
        .await
        .map_err(|e| internal(&e))?;
    Ok(Json(page_view(&state, page).await?))
}

#[derive(Debug, Serialize)]
struct StepView {
    step: &'static str,
    state: &'static str,
    at: Option<i64>,
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct FileView {
    name: String,
    /// The folders the post shows the file in (`회차/2화`), when it does.
    folder: Option<String>,
    state: &'static str,
    size: Option<u64>,
    sha256: Option<String>,
    path: Option<String>,
    shared_with: Option<String>,
    reason: Option<String>,
    format: Option<&'static str>,
    failure: Option<&'static str>,
    http_status: Option<u16>,
    content_type: Option<String>,
    response_size: Option<u64>,
    /// For an uploaded file: what its content check judged it to be.
    kind: Option<&'static str>,
    /// For an uploaded archive: its format (`zip`, `rar`, `7z`, `gz`, `bz2`, `xz`, `tar`).
    archive: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct DroppedView {
    name: String,
    reason: String,
}

#[derive(Debug, Serialize)]
struct ItemView {
    id: i64,
    episode: String,
    post_url: String,
    state: &'static str,
    wait: Option<&'static str>,
    reason: Option<String>,
    failure: Option<&'static str>,
    /// For an item of a revision job whose files are the earlier receipt's
    /// bytes: that receipt's job. There is nothing to replace.
    unchanged_from: Option<String>,
    files: Vec<FileView>,
}

#[derive(Debug, Serialize)]
struct LogView {
    at: i64,
    message: String,
    detail: Option<String>,
}

#[derive(Debug, Serialize)]
struct DetailView {
    #[serde(flatten)]
    row: JobRowView,
    steps: Vec<StepView>,
    items: Vec<ItemView>,
    dropped: Vec<DroppedView>,
    receive_dir: String,
    log: Vec<LogView>,
    /// The remote screen of a job that waits for a site's check in the
    /// server browser ([`super::screen_api`]); `null` when it has none.
    screen: Option<super::screen_api::ScreenView>,
}

/// The steps in order, those not reached `upcoming`; `auth` only when the job
/// reached it. An upload has no fetching to do: it shows the steps it went
/// through.
fn steps_view(steps: &[StepRow], origin: &str) -> Vec<StepView> {
    [
        StepKind::Found,
        StepKind::Open,
        StepKind::Auth,
        StepKind::Receive,
    ]
    .into_iter()
    .filter_map(|kind| {
        let row = steps.iter().find(|s| s.step == kind);
        if row.is_none() && (kind == StepKind::Auth || origin == UPLOAD || origin == FIND) {
            return None;
        }
        Some(match row {
            Some(s) => StepView {
                step: kind.code(),
                state: s.state.code(),
                at: Some(s.at),
                note: s.note.clone(),
            },
            None => StepView {
                step: kind.code(),
                state: "upcoming",
                at: None,
                note: None,
            },
        })
    })
    .collect()
}

fn file_view(
    file: &FileRow,
    item: ItemState,
    state: &AppState,
    owners: &HashMap<&str, &str>,
) -> Option<FileView> {
    let shown = match file.state {
        // Under way only while its episode runs; a held or stopped episode
        // left it unconfirmed.
        FileState::Intended | FileState::Fetched if item == ItemState::Running => "receiving",
        FileState::Intended | FileState::Fetched => "held",
        FileState::Done => "done",
        FileState::Held => "held",
        FileState::Failed => "failed",
        // An attempt that left no bytes, replaced by the next one.
        FileState::Abandoned => return None,
    };
    // Only a received file is at its path; a held one's path may hold
    // another file or nothing.
    let received = file.state == FileState::Done;
    Some(FileView {
        name: file.name.clone(),
        folder: file.folder.clone(),
        state: shown,
        size: file.size,
        sha256: file.sha256.clone(),
        path: file
            .path
            .as_ref()
            .filter(|_| received)
            .map(|p| state.receive_root.join(p).to_string_lossy().into_owned()),
        shared_with: file
            .same_as
            .as_deref()
            .and_then(|id| owners.get(id))
            .map(|ep| (*ep).to_owned()),
        reason: file.reason.clone(),
        format: file.format.map(|f| f.code()),
        failure: file.failure.map(|f| f.code()),
        http_status: file.http_status,
        content_type: file.content_type.clone(),
        response_size: file.response_size,
        kind: file.kind.map(|k| k.code()),
        archive: file.archive.map(|a| a.code()),
    })
}

async fn detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<DetailView>, ApiError> {
    let Some(JobDetail {
        row,
        steps,
        items,
        dropped,
        events,
    }) = state.jobs.detail(&id).await.map_err(|e| internal(&e))?
    else {
        return Err(ApiError::not_found("작업을 찾지 못했어요."));
    };
    let covers = covers_of(&state, [&row]).await?;
    // Read only: showing the job never asks for a browser run.
    let screen = super::screen_api::screen_of(&state, &row.id).await?;
    // Which item's episode received each file, for the items that share it.
    let owners: HashMap<&str, &str> = items
        .iter()
        .flat_map(|item| {
            item.files
                .iter()
                .map(move |f| (f.id.as_str(), item.episode.as_str()))
        })
        .collect();
    let items_view = items
        .iter()
        .map(|item| ItemView {
            id: item.id,
            episode: item.episode.clone(),
            post_url: item.post_url.clone(),
            state: item.state.code(),
            wait: item.wait.map(Wait::code),
            reason: item.reason.clone(),
            failure: item.failure.map(|f| f.code()),
            unchanged_from: item.unchanged_from.clone(),
            files: item
                .files
                .iter()
                .filter_map(|f| file_view(f, item.state, &state, &owners))
                .collect(),
        })
        .collect();
    Ok(Json(DetailView {
        steps: steps_view(&steps, &row.origin),
        items: items_view,
        dropped: dropped
            .into_iter()
            .map(|d| DroppedView {
                name: d.name,
                reason: d.reason,
            })
            .collect(),
        receive_dir: state
            .receive_root
            .join(&row.id)
            .to_string_lossy()
            .into_owned(),
        log: events
            .into_iter()
            .map(|e| LogView {
                at: e.at,
                message: e.message,
                detail: e.detail,
            })
            .collect(),
        row: view(&row, &covers),
        screen,
    }))
}

#[derive(Debug, Deserialize, Serialize)]
struct CreateRequest {
    id: String,
    work_id: String,
    season: u32,
    candidates: Vec<i64>,
}

async fn create(
    State(state): State<AppState>,
    Json(request): Json<CreateRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    if request.id.trim().is_empty() {
        return Err(ApiError::invalid("요청 ID가 비어 있어요."));
    }
    // The app's own receipts of the subscribed creator (`auto:`) and the
    // recheck's revisions (`recheck:`) take these IDs.
    if trss_jobs::is_app_command(&request.id) {
        return Err(ApiError::invalid("이 요청 ID는 쓸 수 없어요."));
    }
    if request.candidates.is_empty() {
        return Err(ApiError::invalid("받을 후보를 하나 이상 골라 주세요."));
    }
    if request.candidates.len() > MAX_CANDIDATES {
        return Err(ApiError::invalid(format!(
            "한 작업에는 후보를 {MAX_CANDIDATES}개까지 담을 수 있어요."
        )));
    }
    let mut unique = request.candidates.clone();
    unique.sort();
    unique.dedup();
    if unique.len() != request.candidates.len() {
        return Err(ApiError::invalid("같은 후보를 두 번 골랐어요."));
    }

    let link = state
        .seasons
        .store
        .anissia_link(&request.work_id, request.season)
        .await
        .map_err(|e| match e {
            trss_library::store::seasons::SeasonError::NotFound => {
                ApiError::not_found("시즌을 찾지 못했어요. 화면을 새로고침해 주세요.")
            }
            e => internal(&e),
        })?;
    let Some(anime_no) = link.anime_no else {
        return Err(ApiError::invalid(
            "이 시즌은 Anissia 작품에 연결돼 있지 않아요.",
        ));
    };
    let candidates = state
        .anissia_store
        .candidates(anime_no, Vec::new())
        .await
        .map_err(|e| internal(&e))?;
    let by_id: HashMap<i64, _> = candidates.iter().map(|c| (c.id, c)).collect();
    let mut picked = Vec::with_capacity(request.candidates.len());
    for id in &request.candidates {
        let Some(candidate) = by_id.get(id) else {
            return Err(ApiError::invalid(
                "고른 후보를 찾지 못했어요. 화면을 새로고침해 주세요.",
            ));
        };
        picked.push(*candidate);
    }
    if picked.iter().any(|c| c.source_id != picked[0].source_id) {
        return Err(ApiError::invalid(
            "한 작업에는 한 제작자의 후보만 담을 수 있어요.",
        ));
    }

    // The request's content as the browser sent it, in canonical JSON.
    let canonical = json!({
        "work_id": request.work_id,
        "season": request.season,
        "candidates": request.candidates,
    })
    .to_string();
    let job = NewJob {
        command_id: request.id.clone(),
        request: canonical,
        origin: "pick".to_owned(),
        work_id: Some(request.work_id.clone()),
        season: Some(i64::from(request.season)),
        anime_no: Some(anime_no),
        source_id: Some(picked[0].source_id.clone()),
        creator: Some(picked[0].creator.clone()),
        revision_of: None,
        revises_attributed: false,
        items: picked
            .iter()
            .map(|c| NewItem {
                observation_id: Some(c.id),
                episode: c.episode.clone(),
                post_url: c.post_url.clone(),
                found_at: c.first_seen_at,
            })
            .collect(),
    };
    match state
        .jobs
        .create(job, now_millis())
        .await
        .map_err(|e| internal(&e))?
    {
        Created::Created(id) => {
            if let Some(path) = &state.worker_wake {
                trss_core::wake::wake_worker(path);
            }
            Ok((StatusCode::ACCEPTED, Json(json!({ "id": id }))))
        }
        Created::Existing(id) => Ok((StatusCode::OK, Json(json!({ "id": id })))),
        Created::Mismatch(id) => Err(ApiError::Conflict {
            message: "같은 요청 ID로 다른 작업을 만든 적이 있어요. 화면을 새로고침해 주세요."
                .to_owned(),
            current: Some(json!({ "id": id })),
        }),
    }
}

#[derive(Debug, Deserialize)]
struct FindRequest {
    id: String,
    work_id: String,
    season: u32,
    /// The source ID of the creator.
    creator: String,
}

async fn create_find(
    State(state): State<AppState>,
    Json(request): Json<FindRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let id = request.id.trim();
    if id.is_empty() {
        return Err(ApiError::invalid("요청 ID가 비어 있어요."));
    }
    if id.chars().count() > super::subtitle_upload_api::ID_MAX {
        return Err(ApiError::invalid("요청 ID가 너무 길어요."));
    }
    if trss_jobs::is_app_command(id) {
        return Err(ApiError::invalid("이 요청 ID는 쓸 수 없어요."));
    }
    if request.work_id.is_empty() {
        return Err(ApiError::invalid("작품이 빠졌어요."));
    }
    let link = state
        .seasons
        .store
        .anissia_link(&request.work_id, request.season)
        .await
        .map_err(|e| match e {
            trss_library::store::seasons::SeasonError::NotFound => {
                ApiError::not_found("시즌을 찾지 못했어요. 화면을 새로고침해 주세요.")
            }
            e => internal(&e),
        })?;
    let Some(anime_no) = link.anime_no else {
        return Err(ApiError::invalid(
            "이 시즌은 Anissia 작품에 연결돼 있지 않아서 직접 찾을 제작자가 없어요.",
        ));
    };
    let candidates = state
        .anissia_store
        .candidates(anime_no, Vec::new())
        .await
        .map_err(|e| internal(&e))?;
    // The creator's most recently observed post: the browser opens it.
    let Some(newest) = candidates
        .iter()
        .filter(|c| c.source_id == request.creator)
        .max_by_key(|c| c.id)
    else {
        return Err(ApiError::invalid(
            "고른 제작자가 이 시즌의 제작자가 아니에요. 화면을 새로고침해 주세요.",
        ));
    };
    if !url::Url::parse(&newest.post_url).is_ok_and(|u| matches!(u.scheme(), "http" | "https")) {
        return Err(ApiError::invalid("이 제작자의 게시물 주소를 열 수 없어요."));
    }

    let canonical = json!({
        "find": {
            "work_id": request.work_id,
            "season": request.season,
            "creator": request.creator,
        }
    })
    .to_string();
    let find = NewFind {
        command_id: id.to_owned(),
        request: canonical,
        work_id: request.work_id.clone(),
        season: i64::from(request.season),
        anime_no,
        source_id: newest.source_id.clone(),
        creator: newest.creator.clone(),
        post_url: newest.post_url.clone(),
    };
    match state
        .jobs
        .create_find(find, now_millis())
        .await
        .map_err(|e| internal(&e))?
    {
        Created::Created(id) => {
            if let Some(path) = &state.worker_wake {
                trss_core::wake::wake_worker(path);
            }
            Ok((StatusCode::ACCEPTED, Json(json!({ "id": id }))))
        }
        Created::Existing(id) => Ok((StatusCode::OK, Json(json!({ "id": id })))),
        Created::Mismatch(id) => Err(ApiError::Conflict {
            message: "같은 요청 ID로 다른 작업을 만든 적이 있어요. 화면을 새로고침해 주세요."
                .to_owned(),
            current: Some(json!({ "id": id })),
        }),
    }
}

async fn finish(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let asked = state
        .jobs
        .ask_finish(&id, now_millis())
        .await
        .map_err(|e| internal(&e))?;
    let shown = match asked {
        AskedFinish::Missing => return Err(ApiError::not_found("작업을 찾지 못했어요.")),
        AskedFinish::NotFind => {
            return Err(ApiError::invalid("직접 찾기 작업만 받기를 끝낼 수 있어요."))
        }
        AskedFinish::Ended => "done",
        AskedFinish::Asked => {
            if let Some(path) = &state.worker_wake {
                trss_core::wake::wake_worker(path);
            }
            "finishing"
        }
    };
    Ok(Json(json!({ "state": shown })))
}

/// Makes the subscribed creators' jobs now ([`trss_jobs::Follow::evaluate`])
/// and wakes the worker when it made one: after the creator was set or
/// changed, the rule's receiving turned on, or a season's AniList or Anissia
/// link saved. A failure is logged and changes nothing of the answer, since the
/// worker looks again at its next start and after its next reading.
pub async fn follow_now(state: &AppState) {
    match state.follow.evaluate(now_millis()).await {
        Ok(made) if !made.is_empty() => {
            if let Some(path) = &state.worker_wake {
                trss_core::wake::wake_worker(path);
            }
        }
        Ok(_) => {}
        Err(err) => eprintln!("Cannot look at the subscribed creators' subtitles: {err}"),
    }
}

#[cfg(test)]
mod tests;
