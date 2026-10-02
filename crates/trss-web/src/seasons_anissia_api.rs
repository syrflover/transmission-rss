//! A season's Anissia link (`docs/specs/library.md`, 작품 연결과 제외): which
//! Anissia anime the season is linked to, finding it, and the user's choice.
//!
//! | method and path                                          | body                                                  |
//! | -------------------------------------------------------- | ----------------------------------------------------- |
//! | `GET  /api/library/works/{id}/seasons/{n}/anissia`        |                                                       |
//! | `POST /api/library/works/{id}/seasons/{n}/anissia/search` | `{ "q": "…", "page": 1 }`                             |
//! | `POST /api/library/works/{id}/seasons/{n}/anissia/link`   | `{ "version": 3, "anime_no": 3441, "week": 2 }`       |
//!
//! The link (also the answer of `link`, and each season of the work detail's
//! `seasons[].anissia`):
//!
//! ```json
//! { "season": 2, "version": 3,
//!   "anime": { "anime_no": 3441, "subject": "…", "original_subject": "…",
//!              "status": "END", "url": "https://anissia.net/anime?animeNo=3441" },
//!   "subscription": null }
//! ```
//!
//! - `version` goes with every change of the link: a change made from an
//!   older version answers `409` with the current link in `current`, and
//!   changes nothing. A season never linked has version 0 and no `anime`.
//! - `subscription` is set (`{ rule_id, anime_no, subject }`) while a
//!   subscription is connected to the season: the season's anime is the
//!   subscription's, and `link` refuses (`400`) to change it, whatever the
//!   request names. It is changed from the subscription.
//! - `search` looks `q` up in Anissia's full anime list, finished anime
//!   included (`q` is the work's folder name when left out; the season folder
//!   of a library is always `Season NN`, which names nothing to search for),
//!   one page of up to 30 at a time. `page` counts from 1. When Anissia does
//!   not answer, or its answer is not the list's, it answers `502`
//!   (`unavailable`) with a sentence; the schedule (`GET /anissia/schedule/
//!   {week}`) and the links already stored are not affected.
//! - `link` makes `anime_no` the season's anime (`null` cuts the link). The
//!   anime is not taken from the request: it must be in the schedule's `week`
//!   (the `편성표` tab) or in the page `page` of the search for `q` (the `전체
//!   목록` tab) that Anissia just answered, and its snapshot is stored with the
//!   link.

use std::collections::{BTreeMap, HashMap};

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::{
    subscriptions_api::{scheduled_anime, unavailable, USER_MAX_WAIT},
    ApiError, AppState,
};
use trss_anissia::{Anime, AnissiaError, ScheduleEntry};
use trss_collect::store::channels::{Rule, SeasonAnimeError};
use trss_library::store::seasons::SeasonError;

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/library/works/{id}/seasons/{season}/anissia", get(show))
        .route(
            "/library/works/{id}/seasons/{season}/anissia/search",
            post(search),
        )
        .route(
            "/library/works/{id}/seasons/{season}/anissia/link",
            post(link),
        )
}

/// An anime's page on Anissia.
pub fn anime_url(anime_no: i64) -> String {
    format!("https://anissia.net/anime?animeNo={anime_no}")
}

const SEASON_NOT_FOUND: &str = "이 작품의 이 시즌을 찾지 못했어요.";
/// The most pages of one search the screen can ask for.
const MAX_PAGE: u32 = 100;

#[derive(Debug, Serialize)]
pub struct LinkedAnime {
    anime_no: i64,
    subject: String,
    original_subject: Option<String>,
    /// Anissia's `ON` or `OFF` for an anime the schedule lists, `END` for a
    /// finished one.
    status: String,
    url: String,
}

impl From<&Anime> for LinkedAnime {
    fn from(a: &Anime) -> Self {
        LinkedAnime {
            anime_no: a.anime_no,
            subject: a.subject.clone(),
            original_subject: a.original_subject.clone(),
            status: a.status.clone(),
            url: anime_url(a.anime_no),
        }
    }
}

/// The subscription that holds a season.
#[derive(Debug, Serialize)]
pub struct HoldingSubscription {
    rule_id: String,
    anime_no: i64,
    subject: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AnissiaLinkView {
    season: u32,
    version: i64,
    anime: Option<LinkedAnime>,
    subscription: Option<HoldingSubscription>,
}

/// The links of `seasons` of work `work_id`, by season number. `connected` are
/// the work's subscriptions that are connected to a season of it
/// ([`trss_collect::store::channels::ChannelStore::subscriptions_of_work`]);
/// a link read here for a season the work has no row of is untouched (version 0).
pub async fn link_views(
    state: &AppState,
    work_id: &str,
    seasons: &[u32],
    connected: &[(u32, Rule)],
) -> Result<BTreeMap<u32, AnissiaLinkView>, ApiError> {
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let links = state
        .seasons
        .store
        .anissia_links_of(work_id)
        .await
        .map_err(|e| internal(&e))?;
    let held: HashMap<u32, (&Rule, i64)> = connected
        .iter()
        .filter_map(|(season, rule)| {
            Some((
                *season,
                (rule, rule.subscription.as_ref()?.anissia_anime_no),
            ))
        })
        .collect();
    let wanted: Vec<i64> = links
        .values()
        .filter_map(|l| l.anime_no)
        .chain(held.values().map(|(_, no)| *no))
        .collect();
    let animes = state
        .anissia_store
        .animes(wanted)
        .await
        .map_err(|e| internal(&e))?;
    Ok(seasons
        .iter()
        .map(|season| {
            let link = links.get(season);
            let view = AnissiaLinkView {
                season: *season,
                version: link.map_or(0, |l| l.version),
                anime: link
                    .and_then(|l| l.anime_no)
                    .and_then(|no| animes.get(&no))
                    .map(LinkedAnime::from),
                subscription: held
                    .get(season)
                    .map(|(rule, anime_no)| HoldingSubscription {
                        rule_id: rule.id.clone(),
                        anime_no: *anime_no,
                        subject: animes.get(anime_no).map(|a| a.subject.clone()),
                    }),
            };
            (*season, view)
        })
        .collect())
}

/// The link of one season, read now.
async fn view_of(
    state: &AppState,
    work_id: &str,
    season: u32,
) -> Result<AnissiaLinkView, ApiError> {
    state
        .seasons
        .store
        .anissia_link(work_id, season)
        .await
        .map_err(refused_store)?;
    let connected = state
        .channels
        .subscriptions_of_work(work_id)
        .await
        .map_err(ApiError::from)?;
    let mut views = link_views(state, work_id, &[season], &connected).await?;
    views
        .remove(&season)
        .ok_or_else(|| ApiError::Internal("the season's link view is missing".into()))
}

fn refused_store(error: SeasonError) -> ApiError {
    match error {
        SeasonError::NotFound => ApiError::not_found(SEASON_NOT_FOUND),
        SeasonError::Invalid(message) => ApiError::invalid(message),
        other => ApiError::Internal(other.to_string()),
    }
}

fn bad_body(_: JsonRejection) -> ApiError {
    ApiError::invalid("요청 내용을 읽지 못했어요.")
}

async fn show(
    State(state): State<AppState>,
    Path((id, season)): Path<(String, u32)>,
) -> Result<Json<AnissiaLinkView>, ApiError> {
    Ok(Json(view_of(&state, &id, season).await?))
}

#[derive(Deserialize)]
struct SearchBody {
    q: Option<String>,
    page: Option<u32>,
}

#[derive(Serialize)]
struct CandidateView {
    anime_no: i64,
    subject: String,
    original_subject: Option<String>,
    /// `ON`, `OFF` or `END`, as Anissia writes it.
    status: String,
    /// 0 (Sunday) to 6 (Saturday), 7 (`기타`) or 8 (`신작`).
    week: u8,
    start_date: Option<String>,
    end_date: Option<String>,
    genres: Vec<String>,
    url: String,
}

impl From<&ScheduleEntry> for CandidateView {
    fn from(e: &ScheduleEntry) -> Self {
        CandidateView {
            anime_no: e.anime_no,
            subject: e.subject.clone(),
            original_subject: e.original_subject.clone(),
            status: e.status.clone(),
            week: e.week,
            start_date: e.start_date.clone(),
            end_date: e.end_date.clone(),
            genres: e.genres.clone(),
            url: anime_url(e.anime_no),
        }
    }
}

#[derive(Serialize)]
struct SearchView {
    q: String,
    items: Vec<CandidateView>,
    has_next: bool,
    page: u32,
}

/// The sentence for a search Anissia did not answer. The full list is an
/// endpoint Anissia does not document, so an answer that cannot be read says
/// so, and says that the schedule still works.
fn search_unavailable(error: AnissiaError) -> ApiError {
    match error {
        AnissiaError::Invalid(detail) => {
            eprintln!("trss-web: Anissia's full list: {detail}");
            ApiError::Unavailable(
                "Anissia 전체 목록의 응답을 읽지 못해 검색하지 못했어요. 편성표에서는 고를 수 있어요."
                    .into(),
            )
        }
        other => unavailable(other),
    }
}

/// The text to search for: `q` as the user wrote it, or the work's folder name.
async fn query_of(state: &AppState, work_id: &str, q: Option<String>) -> Result<String, ApiError> {
    let q = match q.map(|q| q.trim().to_owned()).filter(|q| !q.is_empty()) {
        Some(q) => q,
        None => {
            state
                .library
                .work_detail(work_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .ok_or_else(|| ApiError::not_found(SEASON_NOT_FOUND))?
                .dir_name
        }
    };
    if q.chars().count() > 200 {
        return Err(ApiError::invalid("검색어는 200자까지 쓸 수 있어요."));
    }
    Ok(q)
}

fn page_of(page: Option<u32>) -> Result<u32, ApiError> {
    let page = page.unwrap_or(1);
    if (1..=MAX_PAGE).contains(&page) {
        Ok(page)
    } else {
        Err(ApiError::invalid("검색 결과 쪽 번호가 범위를 벗어났어요."))
    }
}

async fn search(
    State(state): State<AppState>,
    Path((id, season)): Path<(String, u32)>,
    body: Result<Json<SearchBody>, JsonRejection>,
) -> Result<Json<SearchView>, ApiError> {
    let Json(body) = body.map_err(bad_body)?;
    let page = page_of(body.page)?;
    // Only a season of this work is searched for.
    state
        .seasons
        .store
        .anissia_link(&id, season)
        .await
        .map_err(refused_store)?;
    let q = query_of(&state, &id, body.q).await?;
    let found = state
        .anissia
        .search_anime(&q, page - 1, Some(USER_MAX_WAIT))
        .await
        .map_err(search_unavailable)?;
    Ok(Json(SearchView {
        q,
        items: found.page.entries.iter().map(CandidateView::from).collect(),
        has_next: !found.page.last,
        page,
    }))
}

#[derive(Deserialize)]
struct LinkBody {
    version: i64,
    /// The anime to link; `null` cuts the link.
    anime_no: Option<i64>,
    /// The schedule's week the anime was picked from (the `편성표` tab)...
    week: Option<u8>,
    /// ...or the search it was picked from (the `전체 목록` tab).
    q: Option<String>,
    page: Option<u32>,
}

/// The anime `body` names, as the snapshot to keep: one Anissia just listed in
/// the place the request says.
async fn named_anime(
    state: &AppState,
    work_id: &str,
    body: &LinkBody,
    anime_no: i64,
) -> Result<Anime, ApiError> {
    match (body.week, body.q.as_deref()) {
        (Some(week), None) => scheduled_anime(state, week, anime_no).await,
        (None, Some(_)) => {
            let page = page_of(body.page)?;
            let q = query_of(state, work_id, body.q.clone()).await?;
            let found = state
                .anissia
                .search_anime(&q, page - 1, Some(USER_MAX_WAIT))
                .await
                .map_err(search_unavailable)?;
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
        _ => Err(ApiError::invalid(
            "작품을 편성표나 검색 결과에서 골라 주세요.",
        )),
    }
}

/// What a subscription holding the season says to a change from the work detail.
fn subscribed(holder: &HoldingSubscription) -> ApiError {
    let name = holder
        .subject
        .as_deref()
        .map_or_else(String::new, |s| format!(" ‘{s}’"));
    ApiError::invalid(format!(
        "이 시즌은 구독{name}에 이어져 있어서 작품 상세에서는 연결을 바꿀 수 없어요. 수집 화면의 구독에서 바꿔 주세요."
    ))
}

async fn conflict(state: &AppState, work_id: &str, season: u32) -> ApiError {
    ApiError::Conflict {
        message: "다른 곳에서 먼저 Anissia 연결을 바꿨어요. 지금 상태를 확인하고 다시 골라 주세요."
            .into(),
        current: match view_of(state, work_id, season).await {
            Ok(view) => serde_json::to_value(view).ok(),
            Err(_) => None,
        },
    }
}

async fn link(
    State(state): State<AppState>,
    Path((id, season)): Path<(String, u32)>,
    body: Result<Json<LinkBody>, JsonRejection>,
) -> Result<Json<AnissiaLinkView>, ApiError> {
    let Json(body) = body.map_err(bad_body)?;
    if body.anime_no.is_some_and(|no| no <= 0) {
        return Err(ApiError::invalid("Anissia 작품 번호가 올바르지 않아요."));
    }
    // Refused before Anissia is asked anything: a season a subscription holds,
    // and a link that has moved on.
    let current = view_of(&state, &id, season).await?;
    if let Some(holder) = &current.subscription {
        return Err(subscribed(holder));
    }
    if current.version != body.version {
        return Err(conflict(&state, &id, season).await);
    }
    let anime = match body.anime_no {
        Some(no) => Some(named_anime(&state, &id, &body, no).await?),
        None => None,
    };
    match state
        .channels
        .set_season_anime(&id, season, body.version, anime)
        .await
    {
        Ok(_) => Ok(Json(view_of(&state, &id, season).await?)),
        Err(SeasonAnimeError::NoWork) => Err(ApiError::not_found(SEASON_NOT_FOUND)),
        // A rule connected to the season after the check above.
        Err(SeasonAnimeError::Subscribed { rule_id, anime_no }) => {
            Err(subscribed(&HoldingSubscription {
                rule_id,
                anime_no,
                subject: state
                    .anissia_store
                    .anime(anime_no)
                    .await
                    .ok()
                    .flatten()
                    .map(|a| a.subject),
            }))
        }
        Err(SeasonAnimeError::Conflict(_)) => Err(conflict(&state, &id, season).await),
        Err(SeasonAnimeError::Db(e)) => Err(ApiError::Internal(e.to_string())),
    }
}
