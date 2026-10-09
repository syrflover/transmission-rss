//! `GET /api/library/works`: the library list, one page at a time
//! (`docs/specs/library.md`, 라이브러리 화면). The server sorts, filters and
//! searches; the screen asks for the next page when it reaches the end.
//!
//! ```text
//! GET /api/library/works?sort=subtitle&filter=all&q=lycoris&after=<cursor>&limit=60
//! ```
//!
//! | parameter | meaning                                                              |
//! | --------- | -------------------------------------------------------------------- |
//! | `sort`    | `title`, `year`, `added`, `video`, `subtitle` (default `subtitle`)  |
//! | `filter`  | `all`, `airing`, `complete`, `partial`, `none`, `check` (default `all`) |
//! | `q`       | text the work's title (its folder name) or a linked AniList entry's native, English or romaji title contains, without regard to case; NFC-normalized |
//! | `after`   | the `next` of the previous page; leave out for the first page        |
//! | `limit`   | 1 to 200 works, default 60                                           |
//!
//! ```json
//! { "items": [{
//!     "id": "…", "name": "Lycoris Recoil", "missing": false,
//!     "watch_folder": { "id": "…", "path": "/media/anime" },
//!     "latest_season": 2,
//!     "video":    [{ "first": "01", "last": "03" }, { "first": "05", "last": "12" }],
//!     "subtitle": [{ "first": "01", "last": "03" }],
//!     "subtitle_coverage": "some",
//!     "subtitle_check_needed": false,
//!     "added_at": 1760000000000,
//!     "video_added_at": null,
//!     "subtitle_added_at": 1760000100000,
//!     "cover_url": "/api/library/works/…/artwork/image?v=…",
//!     "todos": ["auth", "replacement"]
//!   }],
//!   "next": "7b2273…", "total": 520, "library_count": 520 }
//! ```
//!
//! - `next` is an opaque cursor, `null` after the last page. It continues
//!   *after the last item of this page*, so a work added while the client pages
//!   never makes it see an item twice. A cursor goes with the same `sort` as
//!   the page it came from (another sort is a `400`); the filter and the search
//!   are sent again each time. `total` is how many works the filter and the
//!   search match, `library_count` how many the library has.
//! - Order: the sort's time latest first (`added`: the work's, `video` /
//!   `subtitle`: the latest known video / subtitle over every season), an
//!   unknown time after every known one, then the title (NFC, case ignored),
//!   then the ID. `title` is the title order; `year` is the start year of the
//!   latest local season's first AniList entry, latest first, unknown last.
//! - Filters: `complete`, `partial` and `none` are the subtitle coverage of the
//!   latest season; `check` is a subtitle file that could not be placed or a
//!   work whose folder is gone; `airing` matches the works whose latest local season links an
//!   entry that is releasing.
//! - `name` is the work's folder name. `latest_season` is the highest season
//!   number recorded for the work (`null` without a season folder); `video` and
//!   `subtitle` are the episodes of that season that have a file, as ranges of
//!   consecutive episodes written as in the file names. `01` and `1` are one
//!   episode, and an episode that is not a whole number (`17.5`) is a range of
//!   its own. See [`trss_library::store::library`]'s overview for the rules.
//! - `subtitle_coverage` is `all` (there is a video and every episode with a
//!   video has a subtitle), `some`, or `none`; `null` for a work whose folder is
//!   gone (`missing`), which has no holdings counted.
//! - `subtitle_check_needed` is true while the work has a subtitle file that
//!   could not be attached to an episode.
//! - Times are Unix milliseconds, `null` when unknown: `added_at` is when the
//!   work first appeared, `video_added_at` / `subtitle_added_at` the latest
//!   known time a video / subtitle was added over every season.
//! - `cover_url` is where the work's cover image is served while it has one
//!   (see [`super::artwork_api`]), `null` otherwise. The image is checked when
//!   it is asked for, so the URL may still answer `404`; the screen then
//!   keeps the placeholder.
//! - `todos` are the kinds of the work's to-dos that need the person, the
//!   grid's badges on the cover: `auth` (`인증 필요`), `receive_failed`
//!   (`받기 실패`), `replacement` (`교체 승인`) and `episode_check`
//!   (`회차 확인 필요`, a mapping's or a job's 배치 확인), once each in the
//!   to-do list's order ([`super::todo_api`]). The page reads the to-dos
//!   once for all its works; when they cannot be read, the page still
//!   answers, with no badges.

use axum::{
    extract::{rejection::QueryRejection, Query, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use std::collections::HashMap;

use super::{artwork_api::image_url, todo_api::todo_list, ApiError, AppState};
use trss_jobs::todo::badges_by_work;
use trss_library::store::library::{
    Cursor, EpisodeRange, Filter, LibraryError, ListQuery, Sort, WorkOverview,
};

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
    cover_url: Option<String>,
    todos: Vec<&'static str>,
}

impl WorkView {
    fn new(
        work: WorkOverview,
        image_ids: &HashMap<String, String>,
        badges: &mut HashMap<String, Vec<&'static str>>,
    ) -> Self {
        WorkView {
            cover_url: image_ids
                .get(&work.id)
                .map(|image| image_url(&work.id, image)),
            todos: badges.remove(&work.id).unwrap_or_default(),
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
struct WorkPage {
    items: Vec<WorkView>,
    next: Option<String>,
    total: usize,
    library_count: usize,
}

const DEFAULT_LIMIT: usize = 60;
const MAX_LIMIT: usize = 200;

/// The parameters as sent; each is checked in [`ListQuery`]'s constructor so a
/// bad one answers with a sentence instead of the extractor's plain text.
#[derive(Deserialize, Default)]
struct Params {
    sort: Option<String>,
    filter: Option<String>,
    q: Option<String>,
    after: Option<String>,
    limit: Option<String>,
}

fn parse(params: Params) -> Result<ListQuery, ApiError> {
    let sort = match params.sort.as_deref() {
        None | Some("") => Sort::Subtitle,
        Some(code) => Sort::from_code(code).ok_or_else(|| {
            ApiError::invalid("알 수 없는 정렬이에요. 제목순·방영연도순·최근 작품 추가순·최근 영상 추가순·최근 자막 추가순 중에서 골라 주세요.")
        })?,
    };
    let filter = match params.filter.as_deref() {
        None | Some("") => Filter::All,
        Some(code) => Filter::from_code(code).ok_or_else(|| {
            ApiError::invalid("알 수 없는 필터예요. 전체·방영 중·자막 다 갖춤·자막 일부·자막 없음·확인 필요 중에서 골라 주세요.")
        })?,
    };
    let after = match params.after.as_deref() {
        None | Some("") => None,
        Some(text) => {
            let cursor = Cursor::decode(text).ok_or_else(|| {
                ApiError::invalid(
                    "이어서 받을 위치를 알 수 없어요. 목록을 처음부터 다시 불러와 주세요.",
                )
            })?;
            if cursor.sort() != sort {
                return Err(ApiError::invalid(
                    "이어서 받을 위치가 다른 정렬의 것이에요. 목록을 처음부터 다시 불러와 주세요.",
                ));
            }
            Some(cursor)
        }
    };
    let limit = match params.limit.as_deref() {
        None | Some("") => DEFAULT_LIMIT,
        Some(text) => match text.parse::<usize>() {
            Ok(n) if (1..=MAX_LIMIT).contains(&n) => n,
            _ => {
                return Err(ApiError::invalid(format!(
                    "한 번에 받을 작품 수는 1에서 {MAX_LIMIT} 사이로 정해 주세요."
                )))
            }
        },
    };
    Ok(ListQuery {
        sort,
        filter,
        search: params.q.unwrap_or_default(),
        after,
        limit,
    })
}

async fn list(
    State(state): State<AppState>,
    params: Result<Query<Params>, QueryRejection>,
) -> Result<Json<WorkPage>, ApiError> {
    let Query(params) = params.map_err(|_| {
        ApiError::invalid("목록을 받을 조건을 읽지 못했어요. 주소를 확인해 주세요.")
    })?;
    let query = parse(params)?;
    let page = state.library.list(query).await.map_err(|e| match e {
        LibraryError::Db(e) => ApiError::Internal(e.to_string()),
        other => ApiError::Internal(other.to_string()),
    })?;
    let image_ids = state
        .artwork
        .store
        .image_ids()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    // The badges are extra: a list whose to-dos cannot be read still answers.
    let mut badges = match todo_list(&state).await {
        Ok(todos) => badges_by_work(&todos.needs),
        Err(e) => {
            eprintln!("Cannot read the to-dos for the library's badges: {e:?}");
            HashMap::new()
        }
    };
    Ok(Json(WorkPage {
        items: page
            .items
            .into_iter()
            .map(|work| WorkView::new(work, &image_ids, &mut badges))
            .collect(),
        next: page.next.map(|cursor| cursor.encode()),
        total: page.total,
        library_count: page.library_count,
    }))
}

#[cfg(test)]
mod tests;
