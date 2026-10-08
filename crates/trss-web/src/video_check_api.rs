//! A person's `확인함` on a video the app asks about (`docs/specs/library.md`,
//! 할 일과 회차 목록): a video directly in a season folder other than
//! `Season 00` whose name gives no episode of it, which is a `회차 확인 필요`
//! to-do ([`super::todo_api`], `video_check`) until the person checks it
//! ([`trss_library::store::library::VideoCheck`]).
//!
//! `POST /api/library/works/{id}/videos/check` with
//! `{ "path": "Season 01/… .mkv", "seen": "1234:1760000000123456789" }`
//! answers `204`: the video is no longer asked about while it is the one at
//! the path. Renaming or moving it ends the question too; another video put at
//! the path is asked about again.
//!
//! - `seen` is the to-do's: the video's size and modification time as the
//!   scan saw them, which the screen sends back as it got it. A video that is
//!   not that one any more answers `409` and is not checked: the screen reads
//!   the to-dos again and shows the new one.
//! - A path the work has no such video at (renamed, moved, never asked about)
//!   answers `404`.
//! - No file is moved, renamed or received, and no job is made.

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::Deserialize;

use super::{ApiError, AppState};
use trss_library::{discovery::SeenFile, store::library::CheckError};

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new().route("/library/works/{id}/videos/check", post(check))
}

/// A video as the scan saw it, as the to-do gives it to the screen: its size
/// and modification time in nanoseconds, which a JSON number in a browser could
/// not hold exactly.
pub fn seen(identity: SeenFile) -> String {
    format!("{}:{}", identity.size, identity.mtime_ns)
}

fn parse_seen(seen: &str) -> Option<SeenFile> {
    let (size, mtime_ns) = seen.split_once(':')?;
    Some(SeenFile {
        size: size.parse().ok()?,
        mtime_ns: mtime_ns.parse().ok()?,
    })
}

#[derive(Deserialize)]
struct CheckBody {
    path: String,
    seen: String,
}

const BAD_BODY: &str = "확인할 영상의 경로와 본 파일을 보내 주세요.";

async fn check(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<CheckBody>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let Json(body) = body.map_err(|_| ApiError::invalid(BAD_BODY))?;
    let seen = parse_seen(&body.seen).ok_or_else(|| ApiError::invalid(BAD_BODY))?;
    match state
        .library
        .check_video(&id, &body.path, seen, state.anissia.now())
        .await
    {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(CheckError::NoFile) => Err(ApiError::not_found(
            "이 영상을 찾지 못했어요. 이름이 바뀌었거나 다른 곳으로 옮겨졌을 수 있어요.",
        )),
        Err(CheckError::Changed) => Err(ApiError::Conflict {
            message: "그 사이 이 자리에 다른 영상이 왔어요. 새 영상을 확인해 주세요.".into(),
            current: None,
        }),
        Err(CheckError::Db(e)) => Err(ApiError::Internal(e.to_string())),
    }
}
