//! A movie's Anissia link: the same four routes as a season's, for an episode
//! of season 0 (`docs/adr/0018-movies-in-season-zero-with-their-own-link.md`).
//!
//! | method and path                                              | body                                            |
//! | ------------------------------------------------------------ | ----------------------------------------------- |
//! | `GET  /api/library/works/{id}/movies/{episode}/anissia`        |                                                 |
//! | `POST /api/library/works/{id}/movies/{episode}/anissia/search` | `{ "q": "…", "page": 1 }`                       |
//! | `POST /api/library/works/{id}/movies/{episode}/anissia/link`   | `{ "version": 3, "anime_no": 3441, "week": 2 }` |
//! | `GET  /api/library/works/{id}/movies/{episode}/anissia/candidates` |                                             |
//!
//! `{episode}` is the number of an episode of the work's season 0 (a movie-only
//! work's movie is episode `1`); a work with no such episode answers `404`.
//!
//! The link (also the answer of `link`):
//!
//! ```json
//! { "episode": 1, "version": 3,
//!   "anime": { "anime_no": 3441, "subject": "…", "original_subject": "…",
//!              "status": "END", "url": "https://anissia.net/anime?animeNo=3441" },
//!   "reference_titles": [{ "kind": "native", "title": "…" }, …] }
//! ```
//!
//! - `version` goes with every change of the movie's link, apart from the
//!   seasons' links: a change made from an older version answers `409` with the
//!   current link in `current`, and changes nothing. A movie never linked has
//!   version 0 and no `anime`. No subscription holds a movie, so `link` never
//!   refuses for one.
//! - `reference_titles` are the titles of the AniList entries linked to the
//!   work's season 0, then the folder name, as a season's are.
//! - `search` and `link` work as a season's do (see [`super`]): `link` stores
//!   the anime picked from the schedule's `week` or the search for `q`, and
//!   asks the worker to read the anime's subtitle lines.
//!
//! `candidates` is the movie's 자막 후보: the observations the app has of the
//! movie's linked anime. It is a season's `candidates` without `mappings`,
//! `previous_episodes` and `season_episodes` (a movie has no episode mapping),
//! with `episode` for the movie's number in place of `season`, and every
//! candidate has `movie_episode`, the movie it belongs to: all of an anime's
//! lines belong to the movie whatever their own `episode` text says (`극장판`,
//! `0`, `1`, `SP`), which stays as Anissia wrote it.
//!
//! ```json
//! { "episode": 1, "anime_no": 3441, "read_at": 1790780400000, "refresh": null,
//!   "candidates": [ { "id": 12, "source_id": "6f0c…", "creator": "에루샤",
//!       "post_url": "https://erulabo.com/837", "episode": "극장판", "movie_episode": 1,
//!       "…": "as a season's candidate" } ],
//!   "creator_episodes": [ … ], "max_job_candidates": 200 }
//! ```

use std::collections::HashMap;

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;

use super::{
    bad_body, creator_episodes, named_anime, page_of, query_of, reference_titles,
    search_unavailable, CandidateObservation, CandidateView, CreatorEpisodes, LinkBody,
    LinkedAnime, PickView, ReferenceTitle, SearchBody, SearchView, MAX_PAGE,
};
use crate::{
    commands_api::{ask_anissia_captions, CommandView},
    subscriptions_api::USER_MAX_WAIT,
    ApiError, AppState,
};
use trss_collect::store::channels::MovieAnimeError;
use trss_core::episode::ranks;
use trss_library::store::seasons::{movie::MovieLink, SeasonError};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/library/works/{id}/movies/{episode}/anissia", get(show))
        .route(
            "/library/works/{id}/movies/{episode}/anissia/search",
            post(search),
        )
        .route(
            "/library/works/{id}/movies/{episode}/anissia/link",
            post(link),
        )
        .route(
            "/library/works/{id}/movies/{episode}/anissia/candidates",
            get(candidates),
        )
}

const MOVIE_NOT_FOUND: &str = "이 작품의 이 극장판을 찾지 못했어요.";

#[derive(Debug, Serialize)]
pub struct MovieLinkView {
    episode: u32,
    version: i64,
    anime: Option<LinkedAnime>,
    /// The titles of season 0's linked AniList entries and the work's folder
    /// name, for the user to read when writing a search; no duplicates.
    reference_titles: Vec<ReferenceTitle>,
}

fn refused_store(error: SeasonError) -> ApiError {
    match error {
        SeasonError::NotFound => ApiError::not_found(MOVIE_NOT_FOUND),
        other => ApiError::Internal(other.to_string()),
    }
}

/// The link of a movie as stored; `404` for an episode the work's season 0
/// does not have.
async fn stored(state: &AppState, work_id: &str, episode: u32) -> Result<MovieLink, ApiError> {
    state
        .seasons
        .store
        .movie_link(work_id, episode)
        .await
        .map_err(refused_store)
}

/// The link of one movie, read now.
async fn view_of(state: &AppState, work_id: &str, episode: u32) -> Result<MovieLinkView, ApiError> {
    let link = stored(state, work_id, episode).await?;
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let work_name = state
        .library
        .work_detail(work_id)
        .await
        .map_err(|e| internal(&e))?
        .ok_or_else(|| ApiError::not_found(MOVIE_NOT_FOUND))?
        .dir_name;
    let info = state
        .seasons
        .store
        .links_of_seasons(vec![(work_id.to_owned(), 0)])
        .await
        .map_err(|e| internal(&e))?
        .remove(0);
    let anime = match link.anime_no {
        Some(no) => state
            .anissia_store
            .animes(vec![no])
            .await
            .map_err(|e| internal(&e))?
            .get(&no)
            .map(LinkedAnime::from),
        None => None,
    };
    Ok(MovieLinkView {
        episode,
        version: link.version,
        anime,
        reference_titles: reference_titles(
            info.as_ref().map_or(&[], |i| i.entries.as_slice()),
            &work_name,
        ),
    })
}

async fn show(
    State(state): State<AppState>,
    Path((id, episode)): Path<(String, u32)>,
) -> Result<Json<MovieLinkView>, ApiError> {
    Ok(Json(view_of(&state, &id, episode).await?))
}

async fn search(
    State(state): State<AppState>,
    Path((id, episode)): Path<(String, u32)>,
    body: Result<Json<SearchBody>, JsonRejection>,
) -> Result<Json<SearchView>, ApiError> {
    let Json(body) = body.map_err(bad_body)?;
    let page = page_of(body.page)?;
    // Only a movie of this work is searched for.
    stored(&state, &id, episode).await?;
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

async fn conflict(state: &AppState, work_id: &str, episode: u32) -> ApiError {
    ApiError::Conflict {
        message: "다른 곳에서 먼저 Anissia 연결을 바꿨어요. 지금 상태를 확인하고 다시 골라 주세요."
            .into(),
        current: match view_of(state, work_id, episode).await {
            Ok(view) => serde_json::to_value(view).ok(),
            Err(_) => None,
        },
    }
}

async fn link(
    State(state): State<AppState>,
    Path((id, episode)): Path<(String, u32)>,
    body: Result<Json<LinkBody>, JsonRejection>,
) -> Result<Json<MovieLinkView>, ApiError> {
    let Json(body) = body.map_err(bad_body)?;
    if body.anime_no.is_some_and(|no| no <= 0) {
        return Err(ApiError::invalid("Anissia 작품 번호가 올바르지 않아요."));
    }
    // Refused before Anissia is asked anything: a link that has moved on.
    let current = view_of(&state, &id, episode).await?;
    if current.version != body.version {
        return Err(conflict(&state, &id, episode).await);
    }
    let anime = match body.anime_no {
        Some(no) => Some(named_anime(&state, &body, no).await?),
        None => None,
    };
    match state
        .channels
        .set_movie_anime(&id, episode, body.version, anime)
        .await
    {
        Ok(_) => {
            // The lines Anissia holds for the anime are read by the worker, not
            // waited for here.
            if let Some(anime_no) = body.anime_no {
                ask_anissia_captions(&state, anime_no).await;
            }
            Ok(Json(view_of(&state, &id, episode).await?))
        }
        Err(MovieAnimeError::NoWork) => Err(ApiError::not_found(MOVIE_NOT_FOUND)),
        Err(MovieAnimeError::Conflict(_)) => Err(conflict(&state, &id, episode).await),
        Err(MovieAnimeError::Db(e)) => Err(ApiError::Internal(e.to_string())),
    }
}

#[derive(Serialize)]
struct MovieCandidatesView {
    episode: u32,
    anime_no: Option<i64>,
    read_at: Option<i64>,
    refresh: Option<CommandView>,
    candidates: Vec<CandidateObservation>,
    /// The episodes each creator's candidates are about, as runs.
    creator_episodes: Vec<CreatorEpisodes>,
    /// The most candidates one job takes (`POST /api/subtitle-jobs`).
    max_job_candidates: usize,
}

async fn candidates(
    State(state): State<AppState>,
    Path((id, episode)): Path<(String, u32)>,
) -> Result<Json<MovieCandidatesView>, ApiError> {
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let link = stored(&state, &id, episode).await?;
    let read_at = state
        .anissia_store
        .caption_poll()
        .await
        .map_err(|e| internal(&e))?
        .and_then(|(_, read_at)| read_at);
    let Some(anime_no) = link.anime_no else {
        return Ok(Json(MovieCandidatesView {
            episode,
            anime_no: None,
            read_at,
            refresh: None,
            candidates: Vec::new(),
            creator_episodes: Vec::new(),
            max_job_candidates: crate::jobs_api::MAX_CANDIDATES,
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
    let observed = state
        .anissia_store
        .candidates(anime_no, received)
        .await
        .map_err(|e| internal(&e))?;
    let refresh = state
        .commands
        .latest_for_subjects(
            trss_collect::commands::anissia_captions::KIND,
            vec![anime_no.to_string()],
        )
        .await
        .map_err(|e| internal(&e))?
        .remove(&anime_no.to_string());
    let creator_episodes = creator_episodes(&observed);
    Ok(Json(MovieCandidatesView {
        episode,
        anime_no: Some(anime_no),
        read_at,
        refresh: refresh.as_ref().map(CommandView::from),
        candidates: observed
            .iter()
            .zip(ranks(observed.iter().map(|c| c.episode.as_str())))
            .map(|(c, episode_rank)| CandidateObservation {
                job: latest.get(&c.id).map(|p| PickView {
                    id: p.job_id.clone(),
                    state: p.item_state.code(),
                    wait: p.item_wait.map(trss_jobs::Wait::code),
                }),
                episode_rank,
                movie_episode: Some(episode),
                ..CandidateObservation::from(c)
            })
            .collect(),
        creator_episodes,
        max_job_candidates: crate::jobs_api::MAX_CANDIDATES,
    }))
}
