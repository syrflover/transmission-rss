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
//!   "subscription": null,
//!   "reference_titles": [{ "kind": "native", "title": "…" }, …] }
//! ```
//!
//! - `version` goes with every change of the link: a change made from an
//!   older version answers `409` with the current link in `current`, and
//!   changes nothing. A season never linked has version 0 and no `anime`.
//! - `subscription` is set (`{ rule_id, anime_no, subject }`) while a
//!   subscription is connected to the season: the season's anime is the
//!   subscription's, and `link` refuses (`400`) to change it, whatever the
//!   request names. The season's anime follows the subscription; to link it to
//!   another, the subscription is deleted first (the link stays after that).
//! - `reference_titles` are the titles the user reads to write a search: the
//!   native, English and romaji titles and the Korean synonyms (the ones with
//!   Hangul) of the season's linked AniList entries, then the work's folder
//!   name, without empty or repeated ones. They are text to read, nothing is
//!   searched with them.
//! - `search` looks `q` up in Anissia's full anime list, finished anime
//!   included, one page of up to 30 at a time. `page` counts from 1. `q` is
//!   required: an empty one answers `400`. The list matches Korean titles
//!   (the native title of an AniList entry and the folder names of a library
//!   rarely hit), so nothing is searched until the user writes a query,
//!   reading the link's `reference_titles`. When Anissia does not answer, or
//!   its answer is not the list's, it answers `502` (`unavailable`) with a
//!   sentence; the schedule (`GET /anissia/schedule/{week}`) and the links
//!   already stored are not affected.
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

/// A title the user can read to choose what to search for. `kind` is `native`,
/// `english`, `romaji` or `korean` (an AniList entry's titles) or `folder` (the
/// work's folder name).
#[derive(Debug, Serialize)]
pub struct ReferenceTitle {
    kind: &'static str,
    title: String,
}

#[derive(Debug, Serialize)]
pub struct AnissiaLinkView {
    season: u32,
    version: i64,
    anime: Option<LinkedAnime>,
    subscription: Option<HoldingSubscription>,
    /// The titles of the season's linked AniList entries and the work's folder
    /// name, for the user to read when writing a search; no duplicates.
    reference_titles: Vec<ReferenceTitle>,
}

/// The titles to show for a season: each linked entry's native, English and
/// romaji titles and Korean synonyms, then the folder name, leaving out empty
/// ones and ones already listed (compared without regard to case).
fn reference_titles(entries: &[trss_anilist::Entry], folder: &str) -> Vec<ReferenceTitle> {
    let mut out: Vec<ReferenceTitle> = Vec::new();
    let mut add = |kind: &'static str, title: Option<&str>| {
        let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) else {
            return;
        };
        if out.iter().all(|t| !t.title.eq_ignore_ascii_case(title)) {
            out.push(ReferenceTitle {
                kind,
                title: title.to_owned(),
            });
        }
    };
    for entry in entries {
        add("native", entry.native.as_deref());
        add("english", entry.english.as_deref());
        add("romaji", entry.romaji.as_deref());
        for korean in &entry.korean_titles {
            add("korean", Some(korean));
        }
    }
    add("folder", Some(folder));
    out
}

/// The links of `seasons` of work `work_id`, by season number. `connected` are
/// the work's subscriptions that are connected to a season of it
/// ([`trss_collect::store::channels::ChannelStore::subscriptions_of_work`]);
/// a link read here for a season the work has no row of is untouched (version 0).
pub async fn link_views(
    state: &AppState,
    work_id: &str,
    work_name: &str,
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
    // The holder is the first by rule ID, as the store's checks name it, whatever
    // order the work's subscriptions are listed in.
    let mut held: HashMap<u32, (&Rule, i64)> = HashMap::new();
    for (season, rule) in connected {
        let Some(subscription) = rule.subscription.as_ref() else {
            continue;
        };
        let entry = (rule, subscription.anissia_anime_no);
        held.entry(*season)
            .and_modify(|first| {
                if rule.id < first.0.id {
                    *first = entry;
                }
            })
            .or_insert(entry);
    }
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
    let infos = state
        .seasons
        .store
        .links_of_seasons(seasons.iter().map(|s| (work_id.to_owned(), *s)).collect())
        .await
        .map_err(|e| internal(&e))?;
    Ok(seasons
        .iter()
        .zip(infos)
        .map(|(season, info)| {
            let link = links.get(season);
            let view = AnissiaLinkView {
                reference_titles: reference_titles(
                    info.as_ref().map_or(&[], |i| i.entries.as_slice()),
                    work_name,
                ),
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
    let work_name = state
        .library
        .work_detail(work_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found(SEASON_NOT_FOUND))?
        .dir_name;
    let connected = state
        .channels
        .subscriptions_of_work(work_id)
        .await
        .map_err(ApiError::from)?;
    let mut views = link_views(state, work_id, &work_name, &[season], &connected).await?;
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

/// The sentence for a search Anissia did not answer, whatever the failure. The
/// full list is an endpoint Anissia does not document, so an answer that cannot
/// be read says so; every kind of failure says that the schedule is another
/// place to pick from (during a `429` wait only the weeks read already).
fn search_unavailable(error: AnissiaError) -> ApiError {
    const HINT: &str = " 편성표에서는 고를 수 있어요.";
    const HINT_WAITING: &str = " 이미 불러온 편성표에서는 고를 수 있어요.";
    match error {
        AnissiaError::Invalid(detail) => {
            eprintln!("trss-web: Anissia's full list: {detail}");
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
/// reliably is, so the user reads the reference titles and writes the query.
fn query_of(q: Option<String>) -> Result<String, ApiError> {
    let q = q.map(|q| q.trim().to_owned()).unwrap_or_default();
    if q.is_empty() {
        return Err(ApiError::invalid("검색어를 입력해 주세요."));
    }
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
    let q = query_of(body.q)?;
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
async fn named_anime(state: &AppState, body: &LinkBody, anime_no: i64) -> Result<Anime, ApiError> {
    match (body.week, body.q.as_deref()) {
        (Some(week), None) => scheduled_anime(state, week, anime_no).await,
        (None, Some(_)) => {
            let page = page_of(body.page)?;
            let q = query_of(body.q.clone())?;
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
        "이 시즌은 구독{name}을 따라가고 있어서 연결을 바꿀 수 없어요. 다른 작품에 잇고 싶으면 그 구독을 먼저 삭제해 주세요. 삭제해도 시즌의 연결은 남고, 그 뒤 여기서 바꾸거나 끊을 수 있어요."
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
        Some(no) => Some(named_anime(&state, &body, no).await?),
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
