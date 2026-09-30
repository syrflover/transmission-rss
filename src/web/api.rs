use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::json;

/// JSON API mounted under `/api`.
///
/// Unknown paths answer a JSON 404 here instead of falling through to the
/// single-page app, so a mistyped API call is not mistaken for a screen.
pub fn router() -> Router {
    Router::new()
        .route("/health", get(health))
        .fallback(not_found)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response()
}
