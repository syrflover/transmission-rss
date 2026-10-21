//! `/api/save-folder`: what the save folder of a subscription or a rule is
//! chosen from (`docs/specs/collection.md`, 방영작 구독). The screens take the
//! folder as a work and a season, `<work>/Season NN`.
//!
//! | call                                   | success                                    |
//! | -------------------------------------- | ------------------------------------------ |
//! | `GET /save-folder/works`               | `200 { works: [FolderWorkView] }`          |
//! | `GET /save-folder/suggestion?...`      | `200 { suggestion: SuggestionView \| null }` |
//!
//! `works` lists the works of the collect folder, then of the archive folder,
//! each folder's by folder name (a name in both listed once, from the collect
//! folder): the library's records, not the disk. Without the collect folder
//! set there are none.
//!
//! `suggestion` takes `release` (a release's work, as the channel's titles
//! give it) or `work` (a work folder picked from `works` or typed), not both,
//! and optionally `anime_no` and `start`, the Anissia anime the folder is for
//! and its start date as Anissia gives it (`YYYY-MM-DD` or `YYYY-MM`). Without
//! a start date the airing is placed today, as for a rule that is no
//! subscription. The rule is [`trss_collect::save_folder`]; `null` when no
//! folder name is left of what was asked.

use axum::{
    extract::{rejection::QueryRejection, Query, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use trss_collect::save_folder::{self, Airing, Asked};
use trss_core::calendar::day_of;
use trss_library::store::library::FolderWork;

use crate::web::{error::ApiError, state::AppState};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/save-folder/works", get(works))
        .route("/save-folder/suggestion", get(suggestion))
}

const BAD_QUERY: &str =
    "무엇으로 저장 폴더를 제안할지 알 수 없어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";

/// A work a save folder can be in.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "FolderWork"))]
pub struct FolderWorkView {
    /// The work's folder name: what the work field takes.
    name: String,
    /// The work title (작품명).
    title: String,
    /// The work is in the archive folder: a rule saving into it brings it over.
    archived: bool,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "FolderWorksAnswer"))]
pub(super) struct WorksAnswer {
    works: Vec<FolderWorkView>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "FolderSuggestion"))]
struct SuggestionView {
    /// The work field: the library work's folder name when it is one.
    work: String,
    season: u32,
    /// `<work>/Season NN`.
    directory: String,
    /// The library work the folder is, when it is one.
    library: Option<FolderWorkView>,
    /// Why the season is what it is, for the person.
    reason: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "FolderSuggestionAnswer"))]
pub(super) struct SuggestionAnswer {
    suggestion: Option<SuggestionView>,
}

#[derive(Deserialize)]
struct SuggestionQuery {
    release: Option<String>,
    work: Option<String>,
    anime_no: Option<i64>,
    start: Option<String>,
}

/// The collect folder and the archive folder, as registered (no trailing
/// `/`); none while the collect folder is not set.
async fn folders(state: &AppState) -> Result<(Vec<String>, Option<String>), ApiError> {
    let Some(settings) = state
        .settings
        .collection()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    else {
        return Ok((Vec::new(), None));
    };
    let trimmed = |path: &str| path.trim_end_matches('/').to_owned();
    let archive = settings.archive_folder.as_deref().map(trimmed);
    let folders = std::iter::once(trimmed(&settings.folder))
        .chain(archive.clone())
        .collect();
    Ok((folders, archive))
}

fn view(work: FolderWork, archive: Option<&str>) -> FolderWorkView {
    FolderWorkView {
        archived: archive == Some(work.folder.as_str()),
        name: work.dir_name,
        title: work.title,
    }
}

async fn works(State(state): State<AppState>) -> Result<Json<WorksAnswer>, ApiError> {
    let (folders, archive) = folders(&state).await?;
    let works = save_folder::folder_works(&state.library, &folders)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(WorksAnswer {
        works: works
            .into_iter()
            .map(|w| view(w, archive.as_deref()))
            .collect(),
    }))
}

/// A query value that is there and not blank.
fn given(text: &Option<String>) -> Option<&str> {
    text.as_deref().filter(|t| !t.trim().is_empty())
}

async fn suggestion(
    State(state): State<AppState>,
    parsed: Result<Query<SuggestionQuery>, QueryRejection>,
) -> Result<Json<SuggestionAnswer>, ApiError> {
    let Query(q) = parsed.map_err(|_| ApiError::invalid(BAD_QUERY))?;
    let asked = match (given(&q.release), given(&q.work)) {
        (Some(release), None) => Asked::Release(release),
        (None, Some(work)) => Asked::Work(work),
        _ => return Err(ApiError::invalid(BAD_QUERY)),
    };
    let airing = Airing::of(q.start.as_deref(), day_of(state.anissia.now()));
    let (folders, archive) = folders(&state).await?;
    let found = save_folder::suggest(
        &state.library,
        &state.seasons.store,
        &folders,
        asked,
        q.anime_no,
        airing,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(SuggestionAnswer {
        suggestion: found.map(|s| SuggestionView {
            directory: s.directory(),
            reason: s.message(),
            work: s.work,
            season: s.season,
            library: s.library.map(|w| view(w, archive.as_deref())),
        }),
    }))
}

#[cfg(test)]
mod tests;
