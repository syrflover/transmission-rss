//! A job's 배치 확인 (`docs/specs/subtitles.md`, 배치 확인): the table its
//! detail shows while the job waits for a person to place its files, and the
//! person's placing of it (see the module docs of [`super`]).

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use trss_jobs::{
    model::{JobState, Wait},
    place::records::{Confirmed, PlanRow, RowPlacing},
    store::JobRow,
};

use super::{internal, now_millis, ApiError, AppState};

/// One episode a row can be placed on.
#[derive(Debug, Serialize)]
pub(super) struct EpisodeChoice {
    episode: i64,
    /// The name of the episode's video, when it has one.
    video: Option<String>,
    /// How many videos the library has for it: a subtitle goes beside one
    /// only when there is one.
    videos: usize,
    /// Whether it has a subtitle already: a file applied there waits for a
    /// replacement's approval.
    subtitle: bool,
}

/// The table of a job waiting for its 배치 확인.
#[derive(Debug, Serialize)]
pub(super) struct ConfirmView {
    /// `whole`: an upload's or a find job's plan before anything of it is
    /// kept; `held`: the rows a job asks about (보류한 줄).
    scope: &'static str,
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
        .jobs
        .placeable(&row.id)
        .await
        .map_err(|e| internal(&e))?
    else {
        return Ok(None);
    };
    let waiting = match whole {
        true => row.state == JobState::Waiting && row.wait == Some(Wait::Placement),
        false => {
            !matches!(row.state, JobState::Running | JobState::Held)
                && row.wait != Some(Wait::Approval)
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
        .jobs
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
            true => "whole",
            false => "held",
        },
        positions: asked.iter().map(|r| r.position).collect(),
        total,
        episodes,
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct PlacingRequest {
    position: i64,
    /// The episode it goes on; `null`: on no episode.
    episode: Option<i64>,
    /// Whether it is applied (`적용`), else stored only (`적용하지 않음`).
    apply: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ConfirmRequest {
    /// Every row the table places.
    rows: Vec<PlacingRequest>,
}

/// `POST /api/subtitle-jobs/{id}/placement`: the person's 배치 확인 of the
/// job's table. The job goes back in line to keep and apply what it places.
pub(super) async fn confirm(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ConfirmRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
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
        .jobs
        .confirm_placement(&id, placings, total, now_millis())
        .await
        .map_err(|e| internal(&e))?;
    match confirmed {
        Confirmed::Queued { applied, stored } => {
            if let Some(path) = &state.worker_wake {
                trss_core::wake::wake_worker(path);
            }
            Ok(Json(json!({ "applied": applied, "stored": stored })))
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
