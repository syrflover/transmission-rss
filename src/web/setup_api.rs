//! `/api/first-run`: the `처음 설정` checklist that takes the place of the
//! weekly schedule on an install that began empty (`docs/specs/settings.md`,
//! 처음 실행).
//!
//! | call                       | success                                   |
//! | -------------------------- | ----------------------------------------- |
//! | `PUT /first-run/{step}`    | `200 FirstRunView` (`{ "skipped": bool }`) |
//!
//! ```json
//! { "active": true,
//!   "steps": [{ "step": "folder", "done": false, "skipped": false },
//!             { "step": "import", "done": false, "skipped": true }] }
//! ```
//!
//! The checklist itself is read with the weekly schedule
//! (`GET /schedule/week`, [`super::schedule_api`]), which sends `first_run` while
//! the checklist is up and the schedule once it is not.
//!
//! The steps are `folder` (`감시 폴더 등록`) and `import` (`기존 설정 가져오기`).
//! A step is `done` when its result exists: a registered watch folder, a
//! channel. That is read from the data every time, so nothing has to mark it
//! (the folder may be added in the settings and the import applied from any
//! device). What the data cannot say is a skip, which the server keeps so every
//! device shows the same checklist ([`crate::store::setup`]). The checklist is
//! `active` while a step is neither done nor skipped.
//!
//! Only an install that began empty has a first run. For any other, `PUT` is
//! `404`: there is no checklist to skip a step of.

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    routing::put,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::{commands_api::now_millis, ApiError, AppState};
use crate::store::setup::{FirstRun, Step};

pub fn routes() -> Router<AppState> {
    Router::new().route("/first-run/{step}", put(skip))
}

#[derive(Debug, Serialize)]
pub struct StepView {
    /// `folder` or `import`.
    pub step: &'static str,
    pub done: bool,
    pub skipped: bool,
}

#[derive(Debug, Serialize)]
pub struct FirstRunView {
    /// A step is neither done nor skipped, so the checklist is shown.
    pub active: bool,
    pub steps: Vec<StepView>,
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

async fn view_of(state: &AppState, run: FirstRun) -> Result<FirstRunView, ApiError> {
    let folders = state.library.folders().await.map_err(internal)?;
    let channels = state.channels.list_channels().await.map_err(internal)?;
    let steps: Vec<StepView> = Step::ALL
        .into_iter()
        .map(|step| StepView {
            step: step.code(),
            done: match step {
                Step::Folder => !folders.is_empty(),
                Step::Import => !channels.is_empty(),
            },
            skipped: run.skipped(step),
        })
        .collect();
    Ok(FirstRunView {
        active: steps.iter().any(|s| !s.done && !s.skipped),
        steps,
    })
}

/// The checklist while it is up: the install began empty and a step is neither
/// done nor skipped. `None` otherwise (the weekly schedule shows).
pub async fn checklist(state: &AppState) -> Result<Option<FirstRunView>, ApiError> {
    let Some(run) = state.setup.first_run().await.map_err(internal)? else {
        return Ok(None);
    };
    let view = view_of(state, run).await?;
    Ok(view.active.then_some(view))
}

#[derive(Deserialize)]
struct SkipBody {
    skipped: bool,
}

async fn skip(
    State(state): State<AppState>,
    Path(step): Path<String>,
    body: Result<Json<SkipBody>, JsonRejection>,
) -> Result<Json<FirstRunView>, ApiError> {
    let Json(SkipBody { skipped }) =
        body.map_err(|_| ApiError::invalid("건너뛸지 여부를 읽지 못했어요."))?;
    let step = Step::parse(&step).ok_or_else(|| ApiError::not_found("없는 단계예요."))?;
    let began = state
        .setup
        .set_skipped(step, skipped, now_millis())
        .await
        .map_err(internal)?;
    if !began {
        return Err(ApiError::not_found("처음 설정 중이 아니에요."));
    }
    let run = state
        .setup
        .first_run()
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError::not_found("처음 설정 중이 아니에요."))?;
    Ok(Json(view_of(&state, run).await?))
}

#[cfg(test)]
mod tests;
