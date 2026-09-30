//! `trss-web`: the web server.
//!
//! It serves the JSON API under `/api` and the React single-page app's static
//! build (a plain directory, see [`env::STATIC_DIR_VAR`]) from the same
//! process, so production needs no Node server. Client-side routes such as
//! `/collect` fall back to `index.html` so a reload on them works.

use std::path::Path;

use axum::{
    http::{header, HeaderValue},
    Router,
};
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

pub mod api;
pub mod channels_api;
pub mod commands_api;
pub mod env;
pub mod error;
pub mod history_api;
pub mod import_api;
pub mod rules_api;
pub mod state;
pub mod status_api;

pub use error::ApiError;
pub use state::AppState;

/// Builds the app router for the frontend build in `static_dir`.
pub fn router(static_dir: &Path, state: AppState) -> Router {
    // Files under `assets/` carry a content hash in their name, so they can be
    // cached for good; a missing one is a real 404, never the app shell.
    let assets = Router::new()
        .route_service("/assets/{*path}", ServeDir::new(static_dir))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        ));

    // Everything else is a top-level file (favicon, ...) or a client route.
    // The shell must be revalidated so a new release is picked up.
    let pages = Router::new()
        .fallback_service(
            ServeDir::new(static_dir).fallback(ServeFile::new(static_dir.join("index.html"))),
        )
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ));

    Router::new()
        .nest("/api", api::router().with_state(state))
        .merge(assets)
        .fallback_service(pages)
}

/// Resolves when the process is asked to stop (Ctrl-C, or SIGTERM from `docker stop`).
pub async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{header, Method, Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;

    const INDEX: &str = "<!doctype html><title>TRSS</title>";

    fn test_state() -> AppState {
        AppState::new(crate::store::Db::open_blocking(":memory:").unwrap())
    }

    fn build_dir() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), INDEX).unwrap();
        std::fs::write(dir.path().join("favicon.svg"), "<svg/>").unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("assets/app-abc123.js"), "console.log(1)").unwrap();
        dir
    }

    async fn get(dir: &TempDir, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
        let request = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let response = router(dir.path(), test_state())
            .oneshot(request)
            .await
            .unwrap();
        let (parts, body) = response.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        (
            parts.status,
            parts.headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    #[tokio::test]
    async fn health_reports_ok_as_json() {
        let dir = build_dir();
        let (status, headers, body) = get(&dir, "/api/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "application/json");
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["status"], "ok");
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn unknown_api_path_is_a_json_404_not_the_app() {
        let dir = build_dir();
        let (status, headers, body) = get(&dir, "/api/nope").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(headers[header::CONTENT_TYPE], "application/json");
        assert!(!body.contains("<!doctype"));
    }

    #[tokio::test]
    async fn root_and_client_routes_serve_the_app_shell() {
        let dir = build_dir();
        for uri in ["/", "/index.html", "/collect", "/library/works/12"] {
            let (status, headers, body) = get(&dir, uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(body, INDEX, "{uri}");
            assert_eq!(headers[header::CACHE_CONTROL], "no-cache", "{uri}");
        }
    }

    #[tokio::test]
    async fn static_files_are_served_and_hashed_assets_are_immutable() {
        let dir = build_dir();

        let (status, _, body) = get(&dir, "/favicon.svg").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "<svg/>");

        let (status, headers, body) = get(&dir, "/assets/app-abc123.js").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "console.log(1)");
        assert!(headers[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("immutable"));
    }

    #[tokio::test]
    async fn missing_asset_is_404_not_the_app_shell() {
        let dir = build_dir();
        let (status, _, body) = get(&dir, "/assets/missing.js").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(!body.contains("<!doctype"));
    }
}
