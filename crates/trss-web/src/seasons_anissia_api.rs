//! A season's Anissia link (`docs/specs/library.md`, 작품 연결과 제외): which
//! Anissia anime the season is linked to, finding it, and the user's choice.
//!
//! | method and path                                          | body                                                  |
//! | -------------------------------------------------------- | ----------------------------------------------------- |
//! | `GET  /api/library/works/{id}/seasons/{n}/anissia`        |                                                       |
//! | `POST /api/library/works/{id}/seasons/{n}/anissia/search` | `{ "q": "…", "page": 1 }`                             |
//! | `POST /api/library/works/{id}/seasons/{n}/anissia/link`   | `{ "version": 3, "anime_no": 3441, "week": 2 }`       |
//! | `GET  /api/library/works/{id}/seasons/{n}/anissia/candidates` |                                                   |
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
//!   Korean synonyms (the ones with Hangul, first since Anissia matches them)
//!   and the native, English and romaji titles of the season's linked AniList
//!   entries, then the work's folder
//!   name, without empty or repeated ones. They are text to read, nothing is
//!   searched with them.
//! - `search` looks `q` up in Anissia's full anime list, finished anime
//!   included, one page of up to 30 at a time. `page` counts from 1 up to the
//!   answer's `max_page`. The answer is `{ "q", "items", "has_next", "page",
//!   "max_page" }`. `q` is required: an empty one answers `400`. The list matches Korean titles
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
//!   link. Once the link is stored the worker is asked to read the anime's
//!   subtitle lines at once (`anissia_captions`, [`super::commands_api`]), so
//!   the answer does not wait for Anissia.
//!
//! `candidates` is the season's 자막 후보 (`docs/specs/subtitles.md`): the
//! observations the app has of the season's linked anime, whatever was observed
//! before the season was linked ([`trss_collect::anissia::captions`]).
//!
//! ```json
//! { "season": 2, "anime_no": 3441, "read_at": 1790780400000,
//!   "refresh": { "id": "captions-3441-1790780400000", "state": "done", … },
//!   "candidates": [
//!     { "id": 12, "source_id": "6f0c…", "creator": "에루샤",
//!       "post_url": "https://erulabo.com/837", "episode": "12", "episode_shown": "12",
//!       "updated": "2026-09-10T12:10:00", "updated_at": 1789009800000,
//!       "updated_parse_failed": false, "first_seen_at": 1790780400000,
//!       "sort_at": 1789009800000,
//!       "revision": { "of": 7, "same_post": true },
//!       "job": { "id": "1f0c…", "state": "done", "wait": null } } ],
//!   "creator_episodes": [
//!     { "source_id": "6f0c…", "episode_segments": [
//!       { "text": "1–4", "count": 4, "whole": true },
//!       { "text": "SP", "count": 1, "whole": false } ] } ],
//!   "mappings": [
//!     { "source_id": "6f0c…", "kind": "auto", "offset": -12,
//!       "evidence": "13화가 1화 방영 뒤에 올라왔고 앞 시즌 회차 수(12)만큼 이어 셌어요",
//!       "decided_at": 1790780400000, "version": 4, "exceptions": [] } ],
//!   "previous_episodes": 12, "season_episodes": 12, "max_job_candidates": 200 }
//! ```
//!
//! - `candidates` are newest first by `sort_at`: the update time, or the time
//!   the state was first seen when `updated` is not a date and time
//!   (`updated_at` is `null` and `updated_parse_failed` is `true`; `updated`
//!   is Anissia's text as it was). A season with no link has `anime_no`
//!   `null` and none.
//! - `episode` is Anissia's text as written (`0`, `13.5`): it is not a number
//!   and no episode of the season; `episode_shown` is it without the leading
//!   zeros of a whole number (`1` for `01`). `source_id` is the app's ID of the creator's
//!   lines of the anime; `creator` is the display name Anissia gives and is not
//!   an ID.
//! - `revision` marks a revision candidate: a subtitle job received the
//!   creator's subtitle for the same `episode` from an earlier observation
//!   (`of` is that observation, the latest such; `same_post` says whether it
//!   had the same `post_url`: the post was fixed, or the episode was posted
//!   again). It is `null` otherwise, also when the creator was only observed
//!   with the episode before ([`trss_collect::store::anissia::revision_of`]).
//!   It also marks a candidate whose creator the user named for a subtitle
//!   file of the season's episode ([`super::subtitle_creator_api`]): the same
//!   episode, once the source's mapping is applied (compared as numbers when the
//!   source has none, and not marked while its mapping is undecided). Such a
//!   mark has `of` and `same_post` `null`, because the file's post is not known
//!   ([`trss_collect::store::anissia::revision_by_mapping`]).
//! - `job` is how the latest subtitle job that took the candidate stands, as
//!   its item for the candidate: `state` `pending`, `running`, `waiting`
//!   (`wait` `auth` or `subtitle`), `held`, `failed` or `done`, with the job's
//!   `id` for its page; `null` when no job took it.
//! - `read_at` is when the 30-minute reading last read the whole recent list
//!   (`null` before the first); `refresh` is the latest `anissia_captions`
//!   command for the anime (`pending`, `running`, `done` or `failed` with its
//!   outcome), or `null`: the read made when the season was linked and the
//!   user's `새로고침` both show there.
//! - `creator_episodes` are the episodes each creator's candidates are about
//!   (`source_id`, in that order), as runs for the creator's group to name
//!   ([`trss_core::episode::segments`]): the whole numbers as runs of
//!   consecutive ones, `0` among them, then every other text (`13.5`, `SP`)
//!   alone, each as `{ "text", "count", "whole" }`.
//! - `mappings` are the sources' episode mappings to the season, one per
//!   source the app decided one for (the subscribed creator's,
//!   [`trss_jobs::mapping`]): `kind` `auto` (`offset` is added to Anissia's
//!   whole episode), `undecided` (`offset` `null`) or `user` (the user's, see
//!   [`super::mapping_api`], which has the `exceptions` too), with `evidence`,
//!   the grounds that agree or why none do, for `자동 · <근거>`, and `version`,
//!   what a save of the user's mapping carries (a source without an entry is
//!   version 0). `previous_episodes` is the episodes of all the earlier seasons
//!   together when each is known (`0` for the first season), `null` otherwise:
//!   the sum `앞 시즌에 이어 셈` subtracts. `season_episodes` is the season's own
//!   episode count `N` as the app measures a mapping against (the AniList count, else the highest
//!   scheduled episode), `null` when it is not known. `max_job_candidates` is
//!   the most candidates one job takes (`POST /api/subtitle-jobs`), for the
//!   screen to ask for no more.

use std::collections::{BTreeMap, HashMap};

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::{
    commands_api::{ask_anissia_captions, CommandView},
    mapping_api::MappingView,
    subscriptions_api::{scheduled_anime, unavailable, USER_MAX_WAIT},
    ApiError, AppState,
};
use trss_anissia::{Anime, AnissiaError, ScheduleEntry};
use trss_collect::store::{
    anissia::{mark_attributed, Attributed},
    channels::{Rule, SeasonAnimeError},
};
use trss_core::episode::{segments, shown, EpisodeSegment};
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
        .route(
            "/library/works/{id}/seasons/{season}/anissia/candidates",
            get(candidates),
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

/// The titles to show for a season: each linked entry's Korean synonyms and
/// native, English and romaji titles, then the folder name, leaving out empty
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
        // Korean first: Anissia finds an anime by its Korean title.
        for korean in &entry.korean_titles {
            add("korean", Some(korean));
        }
        add("native", entry.native.as_deref());
        add("english", entry.english.as_deref());
        add("romaji", entry.romaji.as_deref());
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

pub(super) fn refused_store(error: SeasonError) -> ApiError {
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
    /// The last page a search answers; a page past it is refused.
    max_page: u32,
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
        max_page: MAX_PAGE,
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
        Ok(_) => {
            // The lines Anissia holds for the anime are read by the worker, not
            // waited for here.
            if let Some(anime_no) = body.anime_no {
                ask_anissia_captions(&state, anime_no).await;
            }
            crate::jobs_api::follow_now(&state).await;
            Ok(Json(view_of(&state, &id, season).await?))
        }
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

#[derive(Serialize)]
struct CandidateObservation {
    id: i64,
    source_id: String,
    creator: String,
    post_url: String,
    episode: String,
    /// `episode` without the leading zeros of a whole number.
    episode_shown: String,
    updated: String,
    updated_at: Option<i64>,
    updated_parse_failed: bool,
    first_seen_at: i64,
    sort_at: i64,
    revision: Option<RevisionView>,
    job: Option<PickView>,
}

#[derive(Serialize)]
struct PickView {
    id: String,
    state: &'static str,
    wait: Option<&'static str>,
}

#[derive(Serialize)]
struct RevisionView {
    /// The earlier observation whose subtitle was received; `null` when the
    /// candidate revises a subtitle file whose creator the user named.
    of: Option<i64>,
    /// Whether that observation had the same post address; `null` with `of`.
    same_post: Option<bool>,
}

#[derive(Serialize)]
struct CreatorEpisodes {
    source_id: String,
    episode_segments: Vec<EpisodeSegment>,
}

#[derive(Serialize)]
struct CandidatesView {
    season: u32,
    anime_no: Option<i64>,
    read_at: Option<i64>,
    refresh: Option<CommandView>,
    candidates: Vec<CandidateObservation>,
    /// The episodes each creator's candidates are about, as runs.
    creator_episodes: Vec<CreatorEpisodes>,
    mappings: Vec<MappingView>,
    /// The episodes of the earlier seasons together, when each is known.
    previous_episodes: Option<u32>,
    /// The season's episode count `N` (AniList, else the highest scheduled episode), when it is known.
    season_episodes: Option<u32>,
    /// The most candidates one job takes (`POST /api/subtitle-jobs`).
    max_job_candidates: usize,
}

impl From<&trss_collect::store::anissia::Candidate> for CandidateObservation {
    fn from(c: &trss_collect::store::anissia::Candidate) -> Self {
        CandidateObservation {
            id: c.id,
            source_id: c.source_id.clone(),
            creator: c.creator.clone(),
            post_url: c.post_url.clone(),
            episode: c.episode.clone(),
            episode_shown: shown(&c.episode).to_owned(),
            updated: c.updated.clone(),
            updated_at: c.updated_at,
            updated_parse_failed: c.updated_at.is_none(),
            first_seen_at: c.first_seen_at,
            sort_at: c.sort_at(),
            revision: c.revision.as_ref().map(|r| RevisionView {
                of: r.of,
                same_post: r.same_post,
            }),
            job: None,
        }
    }
}

/// The episodes each creator's candidates are about, as runs, in the order of
/// the creators' source IDs.
fn creator_episodes(observed: &[trss_collect::store::anissia::Candidate]) -> Vec<CreatorEpisodes> {
    let mut by_source: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for c in observed {
        by_source
            .entry(c.source_id.as_str())
            .or_default()
            .push(c.episode.as_str());
    }
    by_source
        .into_iter()
        .map(|(source_id, episodes)| CreatorEpisodes {
            source_id: source_id.to_owned(),
            episode_segments: segments(episodes),
        })
        .collect()
}

async fn candidates(
    State(state): State<AppState>,
    Path((id, season)): Path<(String, u32)>,
) -> Result<Json<CandidatesView>, ApiError> {
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let link = state
        .seasons
        .store
        .anissia_link(&id, season)
        .await
        .map_err(refused_store)?;
    let read_at = state
        .anissia_store
        .caption_poll()
        .await
        .map_err(|e| internal(&e))?
        .and_then(|(_, read_at)| read_at);
    let Some(anime_no) = link.anime_no else {
        return Ok(Json(CandidatesView {
            season,
            anime_no: None,
            read_at,
            refresh: None,
            candidates: Vec::new(),
            creator_episodes: Vec::new(),
            mappings: Vec::new(),
            previous_episodes: None,
            season_episodes: None,
            max_job_candidates: super::jobs_api::MAX_CANDIDATES,
        }));
    };
    let picks = state
        .jobs
        .picks_of_anime(anime_no)
        .await
        .map_err(|e| internal(&e))?;
    let received = picks
        .iter()
        .filter(|p| p.item_state == trss_jobs::ItemState::Done)
        .filter_map(|p| {
            Some(trss_collect::store::anissia::Received {
                source_id: p.source_id.clone()?,
                episode: p.episode.clone(),
                observation_id: p.observation_id,
                post_url: p.post_url.clone(),
            })
        })
        .collect();
    // The latest job of each candidate (the picks come in the order taken).
    let latest: HashMap<i64, &trss_jobs::store::Pick> =
        picks.iter().map(|p| (p.observation_id, p)).collect();
    let mut observed = state
        .anissia_store
        .candidates(anime_no, received)
        .await
        .map_err(|e| internal(&e))?;
    let mappings = state
        .follow
        .mappings(&id, season)
        .await
        .map_err(|e| internal(&e))?;
    // A subtitle file whose creator the user named makes the creator's later
    // candidate of the same episode a revision candidate too.
    let attributed: Vec<Attributed> = state
        .library
        .attributed_subtitles(&id, season)
        .await
        .map_err(|e| internal(&e))?
        .into_iter()
        .map(|a| Attributed {
            source_id: a.source_id,
            episode: a.episode,
        })
        .collect();
    mark_attributed(&mut observed, &mappings, &attributed);
    let refresh = state
        .commands
        .latest_for_subjects(
            trss_collect::commands::anissia_captions::KIND,
            vec![anime_no.to_string()],
        )
        .await
        .map_err(|e| internal(&e))?
        .remove(&anime_no.to_string());
    let facts = state
        .follow
        .season_facts(&id, season)
        .await
        .map_err(|e| internal(&e))?;
    let creator_episodes = creator_episodes(&observed);
    let mut mappings: Vec<MappingView> = mappings
        .into_iter()
        .map(|(source_id, m)| MappingView::of(source_id, m))
        .collect();
    mappings.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    Ok(Json(CandidatesView {
        season,
        anime_no: Some(anime_no),
        read_at,
        refresh: refresh.as_ref().map(CommandView::from),
        candidates: observed
            .iter()
            .map(|c| CandidateObservation {
                job: latest.get(&c.id).map(|p| PickView {
                    id: p.job_id.clone(),
                    state: p.item_state.code(),
                    wait: p.item_wait.map(trss_jobs::Wait::code),
                }),
                ..CandidateObservation::from(c)
            })
            .collect(),
        creator_episodes,
        mappings,
        previous_episodes: facts.previous,
        season_episodes: facts.total,
        max_job_candidates: super::jobs_api::MAX_CANDIDATES,
    }))
}
