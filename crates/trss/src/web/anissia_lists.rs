//! The two lists of Anissia a screen picks an anime from: a week of the
//! schedule, and a page of the full list's search (Anissia matches its Korean
//! titles). A pick names the list it was made from, and [`listed_anime`] reads
//! that list again: the anime must be in it, and the snapshot kept is the one
//! Anissia gave there. The subscribe flow, a rule's `Anissia 연결` and a
//! season's or a movie's Anissia link take their anime this way.

use super::{
    subscriptions_api::{unavailable, USER_MAX_WAIT},
    ApiError, AppState,
};
use trss_anissia::{Anime, AnissiaError, FetchedPage};

/// The most pages of one search the screen can ask for.
pub(super) const MAX_PAGE: u32 = 100;

/// The list a picked anime was listed in.
pub(super) enum Listed {
    /// A week of the schedule: 0 (Sunday) to 6, 7 (`기타`) or 8 (`신작`).
    Week(u8),
    /// A page of a search, counted from 1.
    Search { q: String, page: u32 },
}

impl Listed {
    /// The list a request names: its `week`, or its `q` with its `page` (1
    /// when left out). A request must name exactly one.
    pub(super) fn of(
        week: Option<u8>,
        q: Option<String>,
        page: Option<u32>,
    ) -> Result<Listed, ApiError> {
        match (week, q) {
            (Some(week), None) => Ok(Listed::Week(week)),
            (None, Some(q)) => Ok(Listed::Search {
                q: query_of(Some(q))?,
                page: page_of(page)?,
            }),
            _ => Err(ApiError::invalid(
                "작품을 편성표나 검색 결과에서 골라 주세요.",
            )),
        }
    }
}

/// Anime `anime_no` as the list `listed` holds it now, as the snapshot to
/// keep: the request names an anime Anissia really lists there.
pub(super) async fn listed_anime(
    state: &AppState,
    listed: &Listed,
    anime_no: i64,
) -> Result<Anime, ApiError> {
    match listed {
        Listed::Week(week) => {
            let fetched = state
                .anissia
                .schedule(*week, Some(USER_MAX_WAIT))
                .await
                .map_err(unavailable)?;
            let entry = fetched
                .value
                .iter()
                .find(|e| e.anime_no == anime_no)
                .ok_or_else(|| {
                    ApiError::invalid(
                        "편성표에서 이 작품을 찾지 못했어요. 편성표를 다시 불러와 골라 주세요.",
                    )
                })?;
            Ok(entry.snapshot(fetched.fetched_at))
        }
        Listed::Search { q, page } => {
            let found = search(state, q, *page).await?;
            let entry = found
                .page
                .entries
                .iter()
                .find(|e| e.anime_no == anime_no)
                .ok_or_else(|| {
                    ApiError::invalid(
                        "검색 결과에서 이 작품을 찾지 못했어요. 다시 검색해서 골라 주세요.",
                    )
                })?;
            Ok(entry.snapshot(found.fetched_at))
        }
    }
}

/// Page `page` (from 1) of the full list's search for `q`.
pub(super) async fn search(state: &AppState, q: &str, page: u32) -> Result<FetchedPage, ApiError> {
    state
        .anissia
        .search_anime(q, page - 1, Some(USER_MAX_WAIT))
        .await
        .map_err(search_unavailable)
}

/// The sentence for a search Anissia did not answer, whatever the failure. The
/// full list is an endpoint Anissia does not document, so an answer that cannot
/// be read says so; every kind of failure says that the schedule is another
/// place to pick from (during a `429` wait only the weeks read already).
pub(super) fn search_unavailable(error: AnissiaError) -> ApiError {
    const HINT: &str = " 편성표에서는 고를 수 있어요.";
    const HINT_WAITING: &str = " 이미 불러온 편성표에서는 고를 수 있어요.";
    match error {
        AnissiaError::Invalid(detail) => {
            eprintln!("trss: Anissia's full list: {detail}");
            ApiError::Unavailable(format!(
                "Anissia 전체 목록의 응답을 읽지 못해 검색하지 못했어요.{HINT}"
            ))
        }
        other => match unavailable(other) {
            ApiError::Unavailable(message) => {
                let waiting = message.contains("요청이 몰려");
                ApiError::Unavailable(format!(
                    "{message}{}",
                    if waiting { HINT_WAITING } else { HINT }
                ))
            }
            other => other,
        },
    }
}

/// The text to search for: what the user wrote. The server picks none itself:
/// Anissia's list matches Korean titles, which no title the app holds
/// reliably is, so the user writes the query.
pub(super) fn query_of(q: Option<String>) -> Result<String, ApiError> {
    let q = q.map(|q| q.trim().to_owned()).unwrap_or_default();
    if q.is_empty() {
        return Err(ApiError::invalid("검색어를 입력해 주세요."));
    }
    if q.chars().count() > 200 {
        return Err(ApiError::invalid("검색어는 200자까지 쓸 수 있어요."));
    }
    Ok(q)
}

/// The page asked for, 1 when left out.
pub(super) fn page_of(page: Option<u32>) -> Result<u32, ApiError> {
    let page = page.unwrap_or(1);
    if (1..=MAX_PAGE).contains(&page) {
        Ok(page)
    } else {
        Err(ApiError::invalid("검색 결과 쪽 번호가 범위를 벗어났어요."))
    }
}
