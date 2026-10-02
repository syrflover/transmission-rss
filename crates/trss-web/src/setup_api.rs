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
//! A step is `done` once it happened: a watch folder was registered, an import
//! was applied (a channel that exists for another reason does not count). The
//! store latches both when they happen ([`trss_library::store::setup`]), so removing
//! the folder or the channels later does not undo them, and a skip, which the
//! data cannot say, is kept there as well so every device shows the same
//! checklist. The checklist is `active` while a step is neither done nor
//! skipped.
//!
//! When both steps are done or skipped the checklist ends, and the end is kept:
//! it does not come back when folders or channels are removed afterwards. Only
//! taking back the skip of a step that is not done brings it back; the web
//! offers that in the notice shown right after the checklist disappears.
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
use trss_library::store::setup::{FirstRun, Step};

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
    /// The checklist is shown: it has not ended and a step is neither done nor
    /// skipped.
    pub active: bool,
    pub steps: Vec<StepView>,
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

fn view_of(run: FirstRun) -> FirstRunView {
    FirstRunView {
        active: run.active(),
        steps: Step::ALL
            .into_iter()
            .map(|step| StepView {
                step: step.code(),
                done: run.done(step),
                skipped: run.skipped(step),
            })
            .collect(),
    }
}

/// The checklist while it is up: the install began empty, it has not ended, and
/// a step is neither done nor skipped. `None` otherwise (the weekly schedule
/// shows). This is where a checklist whose steps are all settled ends for good.
pub async fn checklist(state: &AppState) -> Result<Option<FirstRunView>, ApiError> {
    let Some(run) = state.setup.settle(now_millis()).await.map_err(internal)? else {
        return Ok(None);
    };
    let view = view_of(run);
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
    Ok(Json(view_of(run)))
}

#[cfg(test)]
mod tests;
