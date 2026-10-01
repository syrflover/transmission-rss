//! A work's cover (`docs/specs/library.md`, 작품 표지): its state, its image,
//! and the user's choices from the cover view of the work detail screen.
//!
//! | method and path                                   | body                                    |
//! | ------------------------------------------------- | --------------------------------------- |
//! | `GET  /api/library/works/{id}/artwork`            |                                         |
//! | `GET  /api/library/works/{id}/artwork/image`      | (the image bytes)                       |
//! | `POST /api/library/works/{id}/artwork/search`     | `{ "q": "…", "page": 1 }`               |
//! | `POST /api/library/works/{id}/artwork/pick`       | `{ "version": 3, "anilist_media_id": 1 }` |
//! | `POST /api/library/works/{id}/artwork/upload?version=3` | the file's bytes, any content type |
//! | `POST /api/library/works/{id}/artwork/clear`      | `{ "version": 3 }`                      |
//! | `POST /api/library/works/{id}/artwork/auto`       | `{ "version": 3 }`                      |
//! | `POST /api/library/works/{id}/artwork/repair`     | `{ "version": 3 }`                      |
//!
//! The state (also the answer of every change):
//!
//! ```json
//! { "mode": "manual", "source": "anilist", "anilist_media_id": 143270, "version": 4,
//!   "image": { "id": "…", "origin": "anilist", "format": "jpeg", "byte_size": 81234,
//!              "status": "available", "url": "/api/library/works/…/artwork/image?v=…" },
//!   "pending": null, "note": null }
//! ```
//!
//! - `mode` is `auto`, `manual` or `disabled`; `source` and `anilist_media_id`
//!   what is selected. `version` goes with every change: a change made from
//!   an older version answers `409` with the current state in `current`.
//! - `image` is the current image's reference with `status`, checked now
//!   against the file: `available`, `missing`, `mismatch` (other bytes) or
//!   `unverified` (could not be checked). A reference stays when its file is
//!   unusable; only `available` is shown as the cover.
//! - `pending` is the automatic work still to do (`search`, `fetch`), `note`
//!   why the last automatic attempt selected or received nothing, as `code`
//!   and `message`.
//! - `image` answers the bytes of the current image, after checking the file
//!   again (inside the app data folder, no link, size, SHA-256, format), with
//!   the format's content type, `ETag` (the SHA-256) and `no-cache`, so the
//!   browser asks each time and the server checks each time. `404` when there
//!   is no image or the file is not the recorded one.
//! - `search` asks AniList for anime matching `q` (the work's folder name when
//!   left out), one page of up to 50: `{ "items": [{ "id", "title", "titles",
//!   "format", "season_year", "thumb_url" }], "has_next": bool, "page": 1 }`.
//!   `thumb_url` is AniList's own image address, shown by the browser.
//! - `pick` makes an entry of the search the cover: the server asks AniList
//!   for the entry, fetches its cover from AniList's image host and verifies
//!   it before anything changes.
//! - `upload` takes the file's bytes as the body; the format is judged from
//!   the bytes, never the name or the content type. At most
//!   [`crate::artwork::UPLOAD_SLOTS`] uploads are taken in at once; another
//!   waits for its turn before its body is read.
//! - `clear` makes the work `disabled` (no cover, no automatic search),
//!   `auto` goes back to automatic with a new search, `repair` asks for the
//!   selected AniList entry's image again.
//! - A refused change answers `400` with the reason; the cover stays as it was.

use axum::{
    body::Body,
    extract::{rejection::JsonRejection, DefaultBodyLimit, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::{ApiError, AppState};
use crate::{
    artwork::{
        title::Candidate, ActionError, AnilistError, Artwork, ImageFetchError, MAX_IMAGE_BYTES,
        USER_MAX_WAIT,
    },
    store::artwork::{ArtworkError, Selection, UserChange},
};

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/library/works/{id}/artwork", get(show))
        .route("/library/works/{id}/artwork/image", get(image))
        .route("/library/works/{id}/artwork/search", post(search))
        .route("/library/works/{id}/artwork/pick", post(pick))
        .route(
            "/library/works/{id}/artwork/upload",
            post(upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/library/works/{id}/artwork/clear", post(clear))
        .route("/library/works/{id}/artwork/auto", post(auto))
        .route("/library/works/{id}/artwork/repair", post(repair))
}

/// Where a work's image is served; `v` changes with the image so a new one is
/// never taken for the old.
pub fn image_url(work_id: &str, image_id: &str) -> String {
    format!(
        "/api/library/works/{}/artwork/image?v={}",
        url::form_urlencoded::byte_serialize(work_id.as_bytes()).collect::<String>(),
        url::form_urlencoded::byte_serialize(image_id.as_bytes()).collect::<String>()
    )
}

#[derive(Serialize)]
struct ImageView {
    id: String,
    origin: &'static str,
    format: &'static str,
    byte_size: u64,
    status: &'static str,
    url: String,
}

#[derive(Serialize)]
struct NoteView {
    code: &'static str,
    message: &'static str,
}

#[derive(Serialize)]
struct ArtworkView {
    mode: &'static str,
    source: Option<&'static str>,
    anilist_media_id: Option<i64>,
    version: i64,
    image: Option<ImageView>,
    pending: Option<&'static str>,
    note: Option<NoteView>,
}

/// The state with the image's file checked now.
async fn view(artwork: &Artwork, selection: Selection) -> ArtworkView {
    let image = match selection.image {
        None => None,
        Some(image) => {
            let status = match artwork.image_state(image.clone()).await {
                Ok(_) => "available",
                Err(unavailable) => unavailable.code(),
            };
            Some(ImageView {
                url: image_url(&selection.work_id, &image.id),
                id: image.id,
                origin: image.origin.code(),
                format: image.format.code(),
                byte_size: image.byte_size,
                status,
            })
        }
    };
    ArtworkView {
        mode: selection.mode.code(),
        source: selection.source.map(|s| s.code()),
        anilist_media_id: selection.anilist_media_id,
        version: selection.version,
        image,
        pending: selection.job.map(|j| j.kind.code()),
        note: selection.note.map(|n| NoteView {
            code: n.code(),
            message: n.message(),
        }),
    }
}

const WORK_NOT_FOUND: &str = "이 작품을 찾지 못했어요.";

fn busy(retry_after: std::time::Duration) -> ApiError {
    ApiError::invalid(format!(
        "AniList 요청이 몰려 있어요. {}초 뒤에 다시 해 주세요.",
        retry_after.as_secs().max(1)
    ))
}

/// The answer for a refused action; a conflict carries the current state.
async fn refused(artwork: &Artwork, error: ActionError) -> ApiError {
    match error {
        ActionError::Store(ArtworkError::NotFound) => ApiError::not_found(WORK_NOT_FOUND),
        ActionError::Store(ArtworkError::Conflict(current)) => ApiError::Conflict {
            message: "다른 곳에서 먼저 표지를 바꿨어요. 지금 상태를 확인하고 다시 골라 주세요."
                .into(),
            current: serde_json::to_value(view(artwork, *current).await).ok(),
        },
        ActionError::Store(ArtworkError::Invalid(message)) => ApiError::invalid(message),
        ActionError::Store(ArtworkError::Interrupted) => {
            ApiError::invalid("표지를 저장하는 동안 작업이 끊겼어요. 다시 해 주세요.")
        }
        ActionError::Store(ArtworkError::Db(e)) => ApiError::Internal(e.to_string()),
        ActionError::Anilist(AnilistError::Busy { retry_after }) => busy(retry_after),
        ActionError::Anilist(AnilistError::Store(e)) => ApiError::Internal(e.to_string()),
        ActionError::Anilist(e) => {
            eprintln!("trss-web: AniList: {e}");
            ApiError::invalid("AniList에 연결하지 못했어요. 잠시 뒤 다시 해 주세요.")
        }
        ActionError::Fetch(ImageFetchError::TooLarge) => {
            ApiError::invalid(crate::artwork::Rejected::TooLarge.message())
        }
        ActionError::Fetch(e) => {
            eprintln!("trss-web: AniList image: {e}");
            ApiError::invalid("AniList의 표지 이미지를 받지 못했어요. 잠시 뒤 다시 해 주세요.")
        }
        ActionError::Rejected(r) => ApiError::invalid(r.message()),
        ActionError::NoEntry => ApiError::invalid("AniList에서 이 작품을 찾지 못했어요."),
        ActionError::NoCover => ApiError::invalid("이 AniList 작품에는 표지 이미지가 없어요."),
        ActionError::NoAppData => ApiError::Internal("no app data folder configured".into()),
        ActionError::Publish(e) => ApiError::Internal(format!("artwork publish: {e}")),
    }
}

async fn answer(
    state: &AppState,
    result: Result<Selection, ActionError>,
) -> Result<Json<ArtworkView>, ApiError> {
    match result {
        Ok(selection) => Ok(Json(view(&state.artwork, selection).await)),
        Err(e) => Err(refused(&state.artwork, e).await),
    }
}

async fn show(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ArtworkView>, ApiError> {
    let selection = state.artwork.store.selection(&id).await;
    answer(&state, selection.map_err(ActionError::from)).await
}

async fn image(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let not_found = || ApiError::not_found("표지 이미지가 없어요.").into_response();
    let selection = match state.artwork.store.selection(&id).await {
        Ok(selection) => selection,
        Err(ArtworkError::NotFound) => return not_found(),
        Err(e) => return ApiError::Internal(e.to_string()).into_response(),
    };
    let Some(image) = selection.image else {
        return not_found();
    };
    let format = image.format;
    let etag = format!("\"{}\"", image.sha256);
    let cached = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag));
    // A browser holding the image needs only to know it is still there.
    let mut response = if cached {
        match state.artwork.image_state(image).await {
            Ok(()) => StatusCode::NOT_MODIFIED.into_response(),
            Err(_) => return not_found(),
        }
    } else {
        match state.artwork.image(image).await {
            Ok(bytes) => (StatusCode::OK, Body::from(bytes)).into_response(),
            Err(_) => return not_found(),
        }
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(format.mime()),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if let Ok(value) = HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, value);
    }
    response
}

#[derive(Deserialize)]
struct SearchBody {
    q: Option<String>,
    page: Option<u32>,
}

#[derive(Serialize)]
struct CandidateView {
    id: i64,
    title: String,
    titles: Vec<String>,
    format: Option<String>,
    season_year: Option<i32>,
    thumb_url: Option<String>,
}

impl From<Candidate> for CandidateView {
    fn from(c: Candidate) -> Self {
        let title = c.display_title().to_owned();
        let mut titles: Vec<String> = Vec::new();
        for t in [&c.english, &c.romaji, &c.native].into_iter().flatten() {
            if !titles.contains(t) {
                titles.push(t.clone());
            }
        }
        CandidateView {
            id: c.id,
            title,
            titles,
            format: c.format,
            season_year: c.season_year,
            thumb_url: c.thumb_url,
        }
    }
}

#[derive(Serialize)]
struct SearchView {
    items: Vec<CandidateView>,
    has_next: bool,
    page: u32,
}

fn bad_body(_: JsonRejection) -> ApiError {
    ApiError::invalid("요청 내용을 읽지 못했어요.")
}

async fn search(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<SearchBody>, JsonRejection>,
) -> Result<Json<SearchView>, ApiError> {
    let Json(body) = body.map_err(bad_body)?;
    let page = body.page.unwrap_or(1);
    if !(1..=100).contains(&page) {
        return Err(ApiError::invalid("검색 결과 쪽 번호가 범위를 벗어났어요."));
    }
    let q = match body
        .q
        .map(|q| q.trim().to_owned())
        .filter(|q| !q.is_empty())
    {
        Some(q) => q,
        None => {
            let work = state
                .library
                .work_detail(&id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .ok_or_else(|| ApiError::not_found(WORK_NOT_FOUND))?;
            work.dir_name
        }
    };
    if q.chars().count() > 200 {
        return Err(ApiError::invalid("검색어는 200자까지 쓸 수 있어요."));
    }
    let answer = state
        .artwork
        .anilist
        .search_page(&q, page, Some(USER_MAX_WAIT))
        .await;
    match answer {
        Ok(page_answer) => Ok(Json(SearchView {
            items: page_answer
                .candidates
                .into_iter()
                .map(CandidateView::from)
                .collect(),
            has_next: page_answer.has_next,
            page,
        })),
        Err(e) => Err(refused(&state.artwork, ActionError::Anilist(e)).await),
    }
}

#[derive(Deserialize)]
struct PickBody {
    version: i64,
    anilist_media_id: i64,
}

async fn pick(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<PickBody>, JsonRejection>,
) -> Result<Json<ArtworkView>, ApiError> {
    let Json(body) = body.map_err(bad_body)?;
    if body.anilist_media_id <= 0 {
        return Err(ApiError::invalid("AniList 작품 번호가 올바르지 않아요."));
    }
    let result = state
        .artwork
        .pick(&id, body.version, body.anilist_media_id)
        .await;
    answer(&state, result).await
}

#[derive(Deserialize)]
struct UploadParams {
    version: Option<i64>,
}

async fn upload(
    State(state): State<AppState>,
    Path(id): Path<String>,
    params: Result<Query<UploadParams>, axum::extract::rejection::QueryRejection>,
    body: Body,
) -> Result<Json<ArtworkView>, ApiError> {
    let version = params
        .ok()
        .and_then(|Query(p)| p.version)
        .ok_or_else(|| ApiError::invalid("표지의 버전이 빠졌어요. 화면을 새로 고쳐 주세요."))?;
    // The turn comes first, so only so many bodies are held at once.
    let slot = state.artwork.upload_slot().await;
    let bytes = axum::body::to_bytes(body, MAX_IMAGE_BYTES)
        .await
        .map_err(|_| ApiError::invalid(crate::artwork::Rejected::TooLarge.message()))?;
    let result = state.artwork.upload(&id, version, bytes, Some(slot)).await;
    answer(&state, result).await
}

#[derive(Deserialize)]
struct VersionBody {
    version: i64,
}

async fn change(
    state: AppState,
    id: String,
    body: Result<Json<VersionBody>, JsonRejection>,
    change: UserChange,
) -> Result<Json<ArtworkView>, ApiError> {
    let Json(body) = body.map_err(bad_body)?;
    let result = state.artwork.change(&id, body.version, change).await;
    answer(&state, result).await
}

async fn clear(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<VersionBody>, JsonRejection>,
) -> Result<Json<ArtworkView>, ApiError> {
    change(state, id, body, UserChange::Clear).await
}

async fn auto(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<VersionBody>, JsonRejection>,
) -> Result<Json<ArtworkView>, ApiError> {
    change(state, id, body, UserChange::Auto).await
}

async fn repair(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<VersionBody>, JsonRejection>,
) -> Result<Json<ArtworkView>, ApiError> {
    change(state, id, body, UserChange::Repair).await
}
