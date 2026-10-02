//! `/api/settings/policy`: the common policy (`docs/specs/settings.md`, 공통
//! 정책): the order of the subtitle formats and the server browser's idle time
//! and concurrent jobs, one version for all three
//! ([`trss_core::settings::policy`]).
//!
//! | call                     | success                         |
//! | ------------------------ | ------------------------------- |
//! | `GET /settings/policy`   | `200 PolicyView`                |
//! | `PUT /settings/policy`   | `200 PolicyView` (as saved)     |
//!
//! ```json
//! { "format_order": ["ass", "srt", "smi"],
//!   "idle_timeout_seconds": 300, "max_concurrent_jobs": 1,
//!   "version": 2, "saved_at": 1790780400000,
//!   "limits": { "idle_timeout_seconds": { "min": 60, "max": 3600 },
//!               "max_concurrent_jobs": { "min": 1, "max": 3 } },
//!   "overrides": [{ "work_id": "…", "name": "Lycoris Recoil",
//!                   "format_order": ["srt", "ass", "smi"] }] }
//! ```
//!
//! - Before the first save the policy is the defaults (`ass`, `srt`, `smi`;
//!   300 seconds; 1 job) with `version` 0 and `saved_at` `null`.
//! - The `PUT` body is `{ "version", "format_order", "idle_timeout_seconds",
//!   "max_concurrent_jobs" }`, all of them. `format_order` names `ass`, `srt`
//!   and `smi` once each; the numbers are within `limits`. Anything else
//!   answers `400` with a sentence, and nothing is saved.
//! - A `version` that is not the stored one answers `409` with the stored
//!   policy as `current`, and nothing is saved. Nothing retries it.
//! - `overrides` are the works that order the formats their own way, most
//!   recently changed first; the work's subtitles change that, not this call.

use axum::{
    extract::{rejection::JsonRejection, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::{commands_api::now_millis, ApiError, AppState};
use trss_core::settings::{
    policy::{FormatOrder, Policy, CONCURRENT_JOBS, IDLE_TIMEOUT_SECONDS},
    SettingsError,
};

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new().route("/settings/policy", get(read).put(write))
}

#[derive(Debug, Clone, Serialize)]
struct Range {
    min: u32,
    max: u32,
}

#[derive(Debug, Clone, Serialize)]
struct Limits {
    idle_timeout_seconds: Range,
    max_concurrent_jobs: Range,
}

#[derive(Debug, Clone, Serialize)]
struct OverrideView {
    work_id: String,
    name: String,
    format_order: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
struct PolicyView {
    format_order: Vec<&'static str>,
    idle_timeout_seconds: u32,
    max_concurrent_jobs: u32,
    version: i64,
    saved_at: Option<i64>,
    limits: Limits,
    overrides: Vec<OverrideView>,
}

fn codes(order: &FormatOrder) -> Vec<&'static str> {
    order.formats().iter().map(|f| f.code()).collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteBody {
    /// The version the client saw.
    version: i64,
    format_order: Vec<String>,
    /// Any integer, so a negative one is refused with the range like any other
    /// out of it, not as a body that does not read.
    idle_timeout_seconds: i64,
    max_concurrent_jobs: i64,
}

const BAD_BODY: &str = "요청 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";

fn internal(e: SettingsError) -> ApiError {
    ApiError::Internal(e.to_string())
}

/// The policy as stored, with the works that order the formats their own way.
async fn current(state: &AppState) -> Result<PolicyView, ApiError> {
    let policy = state.settings.policy().await.map_err(internal)?;
    view(state, policy).await
}

/// `policy` as the screen reads it. A save answers with the row it wrote, so
/// another save right after it never shows as this one's.
async fn view(state: &AppState, policy: Policy) -> Result<PolicyView, ApiError> {
    let Policy {
        format_order,
        idle_timeout_seconds,
        max_concurrent_jobs,
        version,
        saved_at,
    } = policy;
    let overrides = state
        .settings
        .work_format_orders()
        .await
        .map_err(internal)?
        .into_iter()
        .map(|w| OverrideView {
            work_id: w.work_id,
            name: w.name,
            format_order: codes(&w.format_order),
        })
        .collect();
    Ok(PolicyView {
        format_order: codes(&format_order),
        idle_timeout_seconds,
        max_concurrent_jobs,
        version,
        saved_at,
        limits: Limits {
            idle_timeout_seconds: Range {
                min: *IDLE_TIMEOUT_SECONDS.start(),
                max: *IDLE_TIMEOUT_SECONDS.end(),
            },
            max_concurrent_jobs: Range {
                min: *CONCURRENT_JOBS.start(),
                max: *CONCURRENT_JOBS.end(),
            },
        },
        overrides,
    })
}

async fn read(State(state): State<AppState>) -> Result<Json<PolicyView>, ApiError> {
    Ok(Json(current(&state).await?))
}

async fn write(
    State(state): State<AppState>,
    parsed: Result<Json<WriteBody>, JsonRejection>,
) -> Result<Json<PolicyView>, ApiError> {
    let Json(body) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;
    let Some(order) = FormatOrder::from_codes(&body.format_order) else {
        return Err(ApiError::invalid(
            "자막 형식 순서에는 ASS·SRT·SMI가 한 번씩 있어야 해요.",
        ));
    };
    let within = |value: i64, range: &std::ops::RangeInclusive<u32>| {
        u32::try_from(value).ok().filter(|v| range.contains(v))
    };
    let Some(idle) = within(body.idle_timeout_seconds, &IDLE_TIMEOUT_SECONDS) else {
        return Err(ApiError::invalid(format!(
            "브라우저 유휴 시간은 {}분부터 {}분까지 정할 수 있어요.",
            IDLE_TIMEOUT_SECONDS.start() / 60,
            IDLE_TIMEOUT_SECONDS.end() / 60
        )));
    };
    let Some(jobs) = within(body.max_concurrent_jobs, &CONCURRENT_JOBS) else {
        return Err(ApiError::invalid(format!(
            "동시 작업 수는 {}개부터 {}개까지 정할 수 있어요.",
            CONCURRENT_JOBS.start(),
            CONCURRENT_JOBS.end()
        )));
    };
    match state
        .settings
        .put_policy(body.version, order, idle, jobs, now_millis())
        .await
    {
        Ok(saved) => Ok(Json(view(&state, saved).await?)),
        Err(SettingsError::Conflict { .. }) => Err(ApiError::conflict_with(current(&state).await?)),
        Err(SettingsError::Invalid(_)) => {
            Err(ApiError::invalid("입력한 값으로는 저장할 수 없어요."))
        }
        Err(e) => Err(internal(e)),
    }
}
