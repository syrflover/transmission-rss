use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Serialize;

use super::{
    error::{ErrorBody, ErrorCode},
    state::AppState,
};

/// JSON API mounted under `/api`.
///
/// Unknown paths answer a JSON 404 here instead of falling through to the
/// single-page app, so a mistyped API call is not mistaken for a screen.
/// Failed calls answer the shape described in [`super::error`].
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .merge(super::archive_api::routes())
        .merge(super::archive_batch_api::routes())
        .merge(super::artwork_api::routes())
        .merge(super::channels_api::routes())
        .merge(super::commands_api::routes())
        .merge(super::events_api::routes())
        .merge(super::history_api::routes())
        .merge(super::import_api::routes())
        .merge(super::jobs_api::routes())
        .merge(super::library_api::routes())
        .merge(super::library_work_api::routes())
        .merge(super::mapping_api::routes())
        .merge(super::past_search_api::routes())
        .merge(super::policy_api::routes())
        .merge(super::rules_api::routes())
        .merge(super::save_folder_api::routes())
        .merge(super::schedule_api::routes())
        .merge(super::screen_api::routes())
        .merge(super::seasons_anissia_api::routes())
        .merge(super::seasons_api::routes())
        .merge(super::settings_api::routes())
        .merge(super::setup_api::routes())
        .merge(super::status_api::routes())
        .merge(super::subscriptions_api::routes())
        .merge(super::subtitle_upload_api::routes())
        .merge(super::subtitle_creator_api::routes())
        .merge(super::video_check_api::routes())
        .merge(super::todo_api::routes())
        .merge(super::watch_folders_api::routes())
        .fallback(not_found)
}

/// `GET /api/health`: the process answers.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(super) struct Health {
    /// Always `ok`.
    #[cfg_attr(test, ts(type = r#""ok""#))]
    status: &'static str,
    version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorBody::new(ErrorCode::NotFound, "없는 API 경로예요.")),
    )
        .into_response()
}
