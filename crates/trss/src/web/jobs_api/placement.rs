//! A job's 배치 확인 (`docs/requirements/subtitles.md`, 배치 확인): the table its
//! detail shows while the job waits for a person to place its files, and the
//! person's placing of it (see the module docs of [`super`]).

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};

use trss_jobs::{
    model::{JobState, RemovalState, Wait},
    place::records::{Confirmed, PlanRow, RowPlacing},
    store::{JobRow, RELOCATE},
};

use super::{internal, now_millis, ApiError, AppState};

/// One episode a row can be placed on.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ConfirmEpisode"))]
pub(in crate::web) struct EpisodeChoice {
    episode: i64,
    /// The name of the episode's video when it has exactly one
    /// (`Show S01E01.mkv`), else `null`.
    video: Option<String>,
    /// How many videos the library has for it: a subtitle goes beside one
    /// only when there is one.
    videos: usize,
    /// Whether it has a subtitle already: a file applied there waits for a
    /// replacement's approval.
    subtitle: bool,
}

/// What a 배치 확인 asks about.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ConfirmScope"))]
#[serde(rename_all = "snake_case")]
pub(in crate::web) enum Scope {
    /// An upload's or a find job's plan before anything of it is kept.
    Whole,
    /// A relocation's plan before any copy moves, its rows and its removals
    /// ([`RelocationView`]) confirmed as they are.
    Relocate,
    /// The rows a job asks about (보류한 줄).
    Held,
}

/// The table of a job waiting for its 배치 확인.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ConfirmView"))]
pub(in crate::web) struct ConfirmView {
    scope: Scope,
    /// The plan rows the person places, by position.
    positions: Vec<i64>,
    /// The season's episode count, when known.
    total: Option<u32>,
    /// The episodes a row can go on: `1..=total`, else up to the last one
    /// the library or the plan names.
    episodes: Vec<EpisodeChoice>,
}

/// The table of the job, while it waits for its 배치 확인; `None` otherwise.
pub(super) async fn view(
    state: &AppState,
    row: &JobRow,
    plan: &[PlanRow],
) -> Result<Option<ConfirmView>, ApiError> {
    let (Some(work_id), Some(season)) = (
        row.work_id.clone(),
        row.season.and_then(|s| u32::try_from(s).ok()),
    ) else {
        return Ok(None);
    };
    let Some((asked, whole)) = state
        .place
        .placeable(&row.id)
        .await
        .map_err(|e| internal(&e))?
    else {
        return Ok(None);
    };
    let waiting = match whole {
        true => row.waits_for_placement(),
        false => {
            !matches!(
                row.state,
                JobState::Running | JobState::Held | JobState::Interrupted
            ) && row.wait != Some(Wait::Approval)
        }
    };
    // An upload's fonts and attachments alone are confirmed with no row.
    if !waiting || (asked.is_empty() && !whole) {
        return Ok(None);
    }
    let total = state
        .follow
        .season_facts(&work_id, season)
        .await
        .map_err(|e| internal(&e))?
        .total;
    let files = state
        .place
        .season_files(&work_id, season)
        .await
        .map_err(|e| internal(&e))?;
    let last = match total {
        Some(n) => i64::from(n),
        None => files
            .keys()
            .copied()
            .chain(
                plan.iter()
                    .filter_map(|r| r.placed.as_ref().map(|p| p.episode)),
            )
            .max()
            .unwrap_or(1),
    };
    let episodes = (1..=last)
        .map(|episode| {
            let of = files.get(&episode);
            let videos = of.map_or(&[][..], |f| f.videos.as_slice());
            EpisodeChoice {
                episode,
                video: match videos {
                    [one] => Some(one.rsplit('/').next().unwrap_or(one).to_owned()),
                    _ => None,
                },
                videos: videos.len(),
                subtitle: of.is_some_and(|f| !f.subtitles.is_empty()),
            }
        })
        .collect();
    Ok(Some(ConfirmView {
        scope: match whole {
            true if row.origin == RELOCATE => Scope::Relocate,
            true => Scope::Whole,
            false => Scope::Held,
        },
        positions: asked.iter().map(|r| r.position).collect(),
        total,
        episodes,
    }))
}

/// One removal of a relocation job (재배치): an applied copy it takes off
/// its old episode once the person confirms.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "Relocation"))]
pub(in crate::web) struct RelocationView {
    id: String,
    /// The `position` of the row that applies the same stored subtitle on its
    /// new episode; `null`: it is applied there already.
    position: Option<i64>,
    /// The episode the copy is on, and its path in the work folder.
    episode: i64,
    path: String,
    /// `planned` until the person confirms the relocation and until the
    /// copy is taken off; then `done` (taken off), `kept` (left where it is)
    /// or `held` (what became of it is not known).
    state: RemovalState,
    /// Why a `kept` or `held` copy is where it is.
    reason: Option<String>,
}

/// The removals of the job: none unless it is a relocation.
pub(super) async fn relocations(
    state: &AppState,
    row: &JobRow,
) -> Result<Vec<RelocationView>, ApiError> {
    if row.origin != RELOCATE {
        return Ok(Vec::new());
    }
    let removals = state
        .place
        .removals(&row.id)
        .await
        .map_err(|e| internal(&e))?;
    Ok(removals
        .into_iter()
        .map(|r| RelocationView {
            id: r.id,
            position: r.position,
            episode: r.episode,
            path: r.path,
            state: r.state,
            reason: r.reason,
        })
        .collect())
}

/// One row of the answer to a 배치 확인.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "PlacementChoice"))]
pub(in crate::web) struct PlacingRequest {
    position: i64,
    /// The episode it goes on; `null`: on no episode.
    episode: Option<i64>,
    /// Whether it is applied (`적용`), else stored only (`적용하지 않음`).
    apply: bool,
}

/// A person's answer to a job's 배치 확인: one entry for each of
/// `confirm.positions`, and for a relocation the `id` of each planned removal
/// it showed.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ConfirmPlacementBody"))]
pub(in crate::web) struct ConfirmRequest {
    /// Every row the table places.
    rows: Vec<PlacingRequest>,
    /// A relocation's removals the table showed, by `id`: another set is a
    /// plan the person did not see.
    #[serde(default)]
    #[cfg_attr(test, ts(optional = nullable))]
    removals: Vec<String>,
}

/// What a confirmed 배치 확인 will do: how many files are applied and stored.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "PlacementConfirmed"))]
pub(in crate::web) struct ConfirmedView {
    applied: usize,
    stored: usize,
}

/// `POST /api/subtitle-jobs/{id}/placement`: the person's 배치 확인 of the
/// job's table. The job goes back in line to keep and apply what it places.
pub(super) async fn confirm(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ConfirmRequest>,
) -> Result<Json<ConfirmedView>, ApiError> {
    let Some(row) = state
        .jobs
        .detail(&id)
        .await
        .map_err(|e| internal(&e))?
        .map(|d| d.row)
    else {
        return Err(ApiError::not_found("작업을 찾지 못했어요."));
    };
    let total = match (&row.work_id, row.season.and_then(|s| u32::try_from(s).ok())) {
        (Some(work), Some(season)) => {
            state
                .follow
                .season_facts(work, season)
                .await
                .map_err(|e| internal(&e))?
                .total
        }
        _ => None,
    };
    let placings = request
        .rows
        .into_iter()
        .map(|r| RowPlacing {
            position: r.position,
            episode: r.episode,
            apply: r.apply,
        })
        .collect();
    let confirmed = state
        .place
        .confirm_placement(&id, placings, request.removals, total, now_millis())
        .await
        .map_err(|e| internal(&e))?;
    match confirmed {
        Confirmed::Queued { applied, stored } => {
            state.wakes.jobs();
            Ok(Json(ConfirmedView { applied, stored }))
        }
        Confirmed::NotFound => Err(ApiError::not_found("작업을 찾지 못했어요.")),
        Confirmed::NotWaiting | Confirmed::Stale => Err(ApiError::Conflict {
            message: "배치 계획이 바뀌었어요. 화면을 새로고침해 주세요.".to_owned(),
            current: None,
        }),
        Confirmed::Refused(why) => Err(ApiError::invalid(why)),
    }
}

#[cfg(test)]
mod tests;
