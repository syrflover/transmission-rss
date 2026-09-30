use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::json;

use super::state::AppState;

/// JSON API mounted under `/api`.
///
/// Unknown paths answer a JSON 404 here instead of falling through to the
/// single-page app, so a mistyped API call is not mistaken for a screen.
/// Failed calls answer the shape described in [`super::error`].
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .merge(super::channels_api::routes())
        .merge(super::commands_api::routes())
        .merge(super::history_api::routes())
        .merge(super::import_api::routes())
        .fallback(not_found)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "not_found", "message": "없는 API 경로예요." })),
    )
        .into_response()
}
