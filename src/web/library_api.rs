//! `GET /api/library/works`: the library list (`docs/specs/library.md`, 라이브러리
//! 화면). One answer carries every work, so the screen sorts, filters and
//! searches without asking again.
//!
//! ```json
//! { "works": [{
//!     "id": "…", "name": "Lycoris Recoil", "missing": false,
//!     "watch_folder": { "id": "…", "path": "/media/anime" },
//!     "latest_season": 2,
//!     "video":    [{ "first": "01", "last": "03" }, { "first": "05", "last": "12" }],
//!     "subtitle": [{ "first": "01", "last": "03" }],
//!     "subtitle_coverage": "some",
//!     "subtitle_check_needed": false,
//!     "added_at": 1760000000000,
//!     "video_added_at": null,
//!     "subtitle_added_at": 1760000100000
//! }] }
//! ```
//!
//! - `name` is the work's folder name. `latest_season` is the highest season
//!   number recorded for the work (`null` without a season folder); `video` and
//!   `subtitle` are the episodes of that season that have a file, as ranges of
//!   consecutive episodes written as in the file names. `01` and `1` are one
//!   episode, and an episode that is not a whole number (`17.5`) is a range of
//!   its own. See [`crate::store::library`]'s overview for the rules.
//! - `subtitle_coverage` is `all` (there is a video and every episode with a
//!   video has a subtitle), `some`, or `none`; `null` for a work whose folder is
//!   gone (`missing`), which has no holdings counted.
//! - `subtitle_check_needed` is true while the work has a subtitle file that
//!   could not be attached to an episode.
//! - Times are Unix milliseconds, `null` when unknown: `added_at` is when the
//!   work first appeared, `video_added_at` / `subtitle_added_at` the latest
//!   known time a video / subtitle was added over every season.
//! - Works come by folder name; the order the screen shows is the screen's.

use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;

use super::{ApiError, AppState};
use crate::store::library::{EpisodeRange, LibraryError, WorkOverview};

pub fn routes() -> Router<AppState> {
    Router::new().route("/library/works", get(list))
}

#[derive(Serialize)]
struct WatchFolderRef {
    id: String,
    path: String,
}

#[derive(Serialize)]
struct RangeView {
    first: String,
    last: String,
}

impl From<EpisodeRange> for RangeView {
    fn from(range: EpisodeRange) -> Self {
        RangeView {
            first: range.first,
            last: range.last,
        }
    }
}

#[derive(Serialize)]
struct WorkView {
    id: String,
    name: String,
    missing: bool,
    watch_folder: WatchFolderRef,
    latest_season: Option<u32>,
    video: Vec<RangeView>,
    subtitle: Vec<RangeView>,
    subtitle_coverage: Option<&'static str>,
    subtitle_check_needed: bool,
    added_at: Option<i64>,
    video_added_at: Option<i64>,
    subtitle_added_at: Option<i64>,
}

impl From<WorkOverview> for WorkView {
    fn from(work: WorkOverview) -> Self {
        WorkView {
            id: work.id,
            name: work.dir_name,
            missing: work.missing,
            watch_folder: WatchFolderRef {
                id: work.watch_folder_id,
                path: work.watch_folder_path,
            },
            latest_season: work.latest_season,
            video: work.video.into_iter().map(RangeView::from).collect(),
            subtitle: work.subtitle.into_iter().map(RangeView::from).collect(),
            subtitle_coverage: work.subtitle_coverage.map(|c| c.code()),
            subtitle_check_needed: work.subtitle_check_needed,
            added_at: work.first_seen_at,
            video_added_at: work.video_added_at,
            subtitle_added_at: work.subtitle_added_at,
        }
    }
}

#[derive(Serialize)]
struct WorkList {
    works: Vec<WorkView>,
}

async fn list(State(state): State<AppState>) -> Result<Json<WorkList>, ApiError> {
    let works = state.library.overview().await.map_err(|e| match e {
        LibraryError::Db(e) => ApiError::Internal(e.to_string()),
        LibraryError::Duplicate => ApiError::Internal("unexpected duplicate".into()),
    })?;
    Ok(Json(WorkList {
        works: works.into_iter().map(WorkView::from).collect(),
    }))
}
