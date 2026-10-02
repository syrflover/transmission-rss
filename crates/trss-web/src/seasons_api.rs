//! A season's info (`docs/specs/library.md`, 시즌 정보): the AniList entries it
//! is linked to, what the screen shows of them, and the user's choices about
//! the link.
//!
//! | method and path                                          | body                                        |
//! | -------------------------------------------------------- | ------------------------------------------- |
//! | `GET  /api/library/works/{id}/seasons/{n}/info`          |                                             |
//! | `POST /api/library/works/{id}/seasons/{n}/search`        | `{ "q": "…", "page": 1 }`                   |
//! | `POST /api/library/works/{id}/seasons/{n}/links`         | `{ "version": 3, "anilist_ids": [1, 2] }`   |
//! | `POST /api/library/works/{id}/seasons/{n}/auto`          | `{ "version": 3 }`                          |
//! | `POST /api/library/works/{id}/seasons/{n}/refresh`       |                                             |
//!
//! The info (also the answer of every change, and each season of the work
//! detail's `seasons[].info`):
//!
//! ```json
//! { "season": 2, "version": 3, "origin": "user", "pending": null, "note": null,
//!   "can_auto": false,
//!   "entries": [{ "id": 143270, "title": "Lycoris Recoil", "romaji": "…", "english": "…",
//!                 "native": "…", "format": "TV", "status": "FINISHED", "episodes": 13,
//!                 "start": { "year": 2022, "month": 7, "day": 2 },
//!                 "end":   { "year": 2022, "month": 9, "day": 24 },
//!                 "url": "https://anilist.co/anime/143270" }],
//!   "airing": { "start": { … }, "end": { … }, "state": "finished" },
//!   "episodes": 13, "studios": ["A-1 Pictures"], "genres": ["Action"],
//!   "anilist_url": "https://anilist.co/anime/143270",
//!   "synopsis": ["First paragraph.", "Second paragraph."],
//!   "suggestions": [] }
//! ```
//!
//! - `version` goes with every change of the links: a change made from an older
//!   version answers `409` with the current info in `current`, and changes
//!   nothing. A season never touched has version 0.
//! - `origin` is `auto` (the app linked the entry itself, or nothing is linked
//!   yet) or `user`. `pending` is `search` while the app has the season's
//!   automatic search to do; `note` says why the last one linked nothing.
//!   `can_auto` is true for the work's first season, the only one the app
//!   searches for.
//! - `airing`, `episodes`, `studios`, `genres` and `synopsis` are the linked
//!   entries taken together ([`trss_legacy::seasons::combine`]) and are `null`/empty
//!   without an entry. `airing.state` is `releasing` while any entry is,
//!   otherwise the last entry's status (`finished`, `not_yet_released`,
//!   `cancelled`, `hiatus`). `episodes` is `null` when any entry's count is
//!   unknown. `synopsis` is the first entry's description as plain text in
//!   paragraphs; it carries no markup, and the screen puts it on the page as
//!   text.
//! - `anilist_url` is the first entry's AniList page, `null` without one.
//! - `suggestions` are the `SEQUEL` entries of the previous season's last
//!   entry, offered while the season has no entry; they link nothing until the
//!   user confirms one with `links`.
//! - `search` asks AniList for anime matching `q` (the work's folder name when
//!   left out), one page of up to 50, as the cover's search does.
//! - `links` makes `anilist_ids` the season's entries in this order (empty
//!   unlinks), the user's choice; entries not stored yet are asked of AniList
//!   first. `auto` unlinks the first season and asks for a new automatic
//!   search. `refresh` asks AniList again for the season's entries, finished
//!   ones too.

use std::collections::BTreeMap;

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::{ApiError, AppState};
use trss_anilist::{title::Candidate, AnilistError, Entry, FuzzyDate, Sequel};
use trss_legacy::{
    artwork::USER_MAX_WAIT,
    seasons::{combine, describe, ActionError},
    store::seasons::{SeasonError, SeasonLink},
};

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/library/works/{id}/seasons/{season}/info", get(show))
        .route("/library/works/{id}/seasons/{season}/search", post(search))
        .route("/library/works/{id}/seasons/{season}/links", post(links))
        .route("/library/works/{id}/seasons/{season}/auto", post(auto))
        .route(
            "/library/works/{id}/seasons/{season}/refresh",
            post(refresh),
        )
}

/// An entry's AniList page.
pub fn anilist_url(id: i64) -> String {
    format!("https://anilist.co/anime/{id}")
}

#[derive(Serialize)]
pub struct EntryView {
    id: i64,
    title: String,
    romaji: Option<String>,
    english: Option<String>,
    native: Option<String>,
    format: Option<String>,
    status: Option<String>,
    episodes: Option<u32>,
    start: FuzzyDate,
    end: FuzzyDate,
    url: String,
}

impl From<&Entry> for EntryView {
    fn from(entry: &Entry) -> Self {
        EntryView {
            id: entry.id,
            title: entry.display_title().to_owned(),
            romaji: entry.romaji.clone(),
            english: entry.english.clone(),
            native: entry.native.clone(),
            format: entry.format.clone(),
            status: entry.status.clone(),
            episodes: entry.episodes,
            start: entry.start,
            end: entry.end,
            url: anilist_url(entry.id),
        }
    }
}

#[derive(Serialize)]
pub struct SuggestionView {
    id: i64,
    title: String,
    romaji: Option<String>,
    english: Option<String>,
    native: Option<String>,
    format: Option<String>,
    status: Option<String>,
    start: FuzzyDate,
    url: String,
}

impl From<Sequel> for SuggestionView {
    fn from(s: Sequel) -> Self {
        let title = s
            .english
            .as_deref()
            .or(s.romaji.as_deref())
            .or(s.native.as_deref())
            .unwrap_or("")
            .to_owned();
        SuggestionView {
            id: s.id,
            title,
            romaji: s.romaji,
            english: s.english,
            native: s.native,
            format: s.format,
            status: s.status,
            start: s.start,
            url: anilist_url(s.id),
        }
    }
}

#[derive(Serialize)]
struct AiringView {
    start: FuzzyDate,
    end: FuzzyDate,
    state: Option<String>,
}

#[derive(Serialize)]
struct NoteView {
    code: &'static str,
    message: &'static str,
}

/// A season's info as the API answers it.
#[derive(Serialize)]
pub struct SeasonInfoView {
    season: u32,
    version: i64,
    origin: &'static str,
    pending: Option<&'static str>,
    note: Option<NoteView>,
    can_auto: bool,
    entries: Vec<EntryView>,
    airing: Option<AiringView>,
    episodes: Option<u32>,
    studios: Vec<String>,
    genres: Vec<String>,
    anilist_url: Option<String>,
    synopsis: Option<Vec<String>>,
    suggestions: Vec<SuggestionView>,
}

/// The info of `link`. `previous` is the link of the season before it (its
/// last entry's sequels are offered while this one has none) and `first` the
/// work's first season.
pub fn season_view(
    link: &SeasonLink,
    previous: Option<&SeasonLink>,
    first: Option<u32>,
) -> SeasonInfoView {
    let combined = combine::combine(&link.entries);
    let synopsis = link
        .entries
        .first()
        .and_then(|e| e.description.as_deref())
        .map(|d| describe::paragraphs(&describe::plain_text(d)))
        .filter(|p| !p.is_empty());
    let (airing, episodes, studios, genres) = match combined {
        Some(c) => (
            Some(AiringView {
                start: c.start,
                end: c.end,
                state: c.state,
            }),
            c.episodes,
            c.studios,
            c.genres,
        ),
        None => (None, None, Vec::new(), Vec::new()),
    };
    SeasonInfoView {
        season: link.season,
        version: link.version,
        origin: link.origin.code(),
        pending: link.job.as_ref().map(|_| "search"),
        note: link.note.map(|n| NoteView {
            code: n.code(),
            message: n.message(),
        }),
        can_auto: first == Some(link.season),
        entries: link.entries.iter().map(EntryView::from).collect(),
        airing,
        episodes,
        studios,
        genres,
        anilist_url: link.entries.first().map(|e| anilist_url(e.id)),
        synopsis,
        suggestions: combine::suggestions(previous.map(|p| p.entries.as_slice()), &link.entries)
            .into_iter()
            .map(SuggestionView::from)
            .collect(),
    }
}

/// An untouched season's link: version 0, nothing linked.
pub fn untouched(work_id: &str, season: u32) -> SeasonLink {
    SeasonLink {
        work_id: work_id.to_owned(),
        season,
        version: 0,
        origin: trss_legacy::store::seasons::Origin::Auto,
        entries: Vec::new(),
        job: None,
        note: None,
    }
}

/// The info of every recorded season of a work, by season number, and the
/// work's first season. `seasons` are the numbers of the seasons the work has.
pub async fn work_infos(
    state: &AppState,
    work_id: &str,
    seasons: &[u32],
) -> Result<(BTreeMap<u32, SeasonLink>, Option<u32>), ApiError> {
    let mut links = state
        .seasons
        .store
        .links_of(work_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let first = seasons.iter().copied().filter(|s| *s >= 1).min();
    for season in seasons {
        links
            .entry(*season)
            .or_insert_with(|| untouched(work_id, *season));
    }
    Ok((links, first))
}

/// The info of one season, read now.
async fn info_of(state: &AppState, work_id: &str, season: u32) -> Result<SeasonInfoView, ApiError> {
    let link = state
        .seasons
        .store
        .link(work_id, season)
        .await
        .map_err(refused_store)?;
    let previous = match season.checked_sub(1).filter(|p| *p >= 1) {
        Some(p) => state.seasons.store.link(work_id, p).await.ok(),
        None => None,
    };
    let first = state
        .seasons
        .store
        .first_season(work_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(season_view(&link, previous.as_ref(), first))
}

const SEASON_NOT_FOUND: &str = "이 작품의 이 시즌을 찾지 못했어요.";

fn refused_store(error: SeasonError) -> ApiError {
    match error {
        SeasonError::NotFound => ApiError::not_found(SEASON_NOT_FOUND),
        SeasonError::Invalid(message) => ApiError::invalid(message),
        other => ApiError::Internal(other.to_string()),
    }
}

fn busy(retry_after: std::time::Duration) -> ApiError {
    ApiError::invalid(format!(
        "AniList 요청이 몰려 있어요. {}초 뒤에 다시 해 주세요.",
        retry_after.as_secs().max(1)
    ))
}

/// The answer for a refused action; a conflict carries the current info.
async fn refused(state: &AppState, work_id: &str, season: u32, error: ActionError) -> ApiError {
    match error {
        ActionError::Store(SeasonError::Conflict(_)) => ApiError::Conflict {
            message:
                "다른 곳에서 먼저 시즌 정보를 바꿨어요. 지금 상태를 확인하고 다시 골라 주세요."
                    .into(),
            current: match info_of(state, work_id, season).await {
                Ok(view) => serde_json::to_value(view).ok(),
                Err(_) => None,
            },
        },
        ActionError::Store(e) => refused_store(e),
        ActionError::Anilist(AnilistError::Busy { retry_after }) => busy(retry_after),
        ActionError::Anilist(AnilistError::Store(e)) => ApiError::Internal(e.to_string()),
        ActionError::Anilist(e) => {
            eprintln!("trss-web: AniList: {e}");
            ApiError::invalid("AniList에 연결하지 못했어요. 잠시 뒤 다시 해 주세요.")
        }
        ActionError::NoEntry(_) => ApiError::invalid("AniList에서 이 항목을 찾지 못했어요."),
    }
}

async fn answer(
    state: &AppState,
    work_id: &str,
    season: u32,
    result: Result<SeasonLink, ActionError>,
) -> Result<Json<SeasonInfoView>, ApiError> {
    match result {
        Ok(_) => Ok(Json(info_of(state, work_id, season).await?)),
        Err(e) => Err(refused(state, work_id, season, e).await),
    }
}

fn bad_body(_: JsonRejection) -> ApiError {
    ApiError::invalid("요청 내용을 읽지 못했어요.")
}

async fn show(
    State(state): State<AppState>,
    Path((id, season)): Path<(String, u32)>,
) -> Result<Json<SeasonInfoView>, ApiError> {
    Ok(Json(info_of(&state, &id, season).await?))
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
    url: String,
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
            url: anilist_url(c.id),
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

async fn search(
    State(state): State<AppState>,
    Path((id, season)): Path<(String, u32)>,
    body: Result<Json<SearchBody>, JsonRejection>,
) -> Result<Json<SearchView>, ApiError> {
    let Json(body) = body.map_err(bad_body)?;
    let page = body.page.unwrap_or(1);
    if !(1..=100).contains(&page) {
        return Err(ApiError::invalid("검색 결과 쪽 번호가 범위를 벗어났어요."));
    }
    // Only a season of this work is searched for.
    state
        .seasons
        .store
        .link(&id, season)
        .await
        .map_err(refused_store)?;
    let q = match body
        .q
        .map(|q| q.trim().to_owned())
        .filter(|q| !q.is_empty())
    {
        Some(q) => q,
        None => {
            state
                .library
                .work_detail(&id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .ok_or_else(|| ApiError::not_found(SEASON_NOT_FOUND))?
                .dir_name
        }
    };
    if q.chars().count() > 200 {
        return Err(ApiError::invalid("검색어는 200자까지 쓸 수 있어요."));
    }
    match state
        .seasons
        .anilist
        .search_page(&q, page, Some(USER_MAX_WAIT))
        .await
    {
        Ok(answer) => Ok(Json(SearchView {
            items: answer
                .candidates
                .into_iter()
                .map(CandidateView::from)
                .collect(),
            has_next: answer.has_next,
            page,
        })),
        Err(e) => Err(refused(&state, &id, season, ActionError::Anilist(e)).await),
    }
}

#[derive(Deserialize)]
struct LinksBody {
    version: i64,
    anilist_ids: Vec<i64>,
}

async fn links(
    State(state): State<AppState>,
    Path((id, season)): Path<(String, u32)>,
    body: Result<Json<LinksBody>, JsonRejection>,
) -> Result<Json<SeasonInfoView>, ApiError> {
    let Json(body) = body.map_err(bad_body)?;
    if body.anilist_ids.iter().any(|id| *id <= 0) {
        return Err(ApiError::invalid("AniList 작품 번호가 올바르지 않아요."));
    }
    let result = state
        .seasons
        .set_links(&id, season, body.version, body.anilist_ids)
        .await;
    answer(&state, &id, season, result).await
}

#[derive(Deserialize)]
struct VersionBody {
    version: i64,
}

async fn auto(
    State(state): State<AppState>,
    Path((id, season)): Path<(String, u32)>,
    body: Result<Json<VersionBody>, JsonRejection>,
) -> Result<Json<SeasonInfoView>, ApiError> {
    let Json(body) = body.map_err(bad_body)?;
    let result = state.seasons.restart_auto(&id, season, body.version).await;
    answer(&state, &id, season, result).await
}

async fn refresh(
    State(state): State<AppState>,
    Path((id, season)): Path<(String, u32)>,
) -> Result<Json<SeasonInfoView>, ApiError> {
    let result = state.seasons.refresh_season(&id, season).await;
    answer(&state, &id, season, result).await
}
