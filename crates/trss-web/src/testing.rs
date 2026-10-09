//! What the API tests of the modules share: the router a test sends to and
//! the request it builds, sends and reads.
//!
//! A module's `App` keeps its own `call`, `get` and so on as thin wrappers,
//! so the tests read the same in every module. A test that needs a raw or
//! multipart body, a streamed answer or its own headers (`artwork_api`,
//! `subtitle_upload_api`, [`crate::origin_guard`], the router tests of
//! `lib.rs`) builds its request itself.

use axum::{
    body::{Body, Bytes},
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use crate::AppState;

/// The API under `/api`, as the app serves it.
pub(crate) fn api(state: &AppState) -> Router {
    Router::new().nest("/api", bare_api(state))
}

/// The API's routes with no `/api` prefix, for the tests that call a route by
/// its own path.
pub(crate) fn bare_api(state: &AppState) -> Router {
    crate::api::router().with_state(state.clone())
}

/// A request that sends `body` (text) as JSON, or nothing.
pub(crate) fn request(method: Method, uri: &str, body: Option<String>) -> Request<Body> {
    let request = Request::builder().method(method).uri(uri);
    match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body)),
        None => request.body(Body::empty()),
    }
    .unwrap()
}

/// Sends `request` to `router`; the status and the whole body.
async fn read(router: &Router, request: Request<Body>) -> (StatusCode, Bytes) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    (
        status,
        response.into_body().collect().await.unwrap().to_bytes(),
    )
}

/// Sends `request` to `router`; the status and the body read as JSON (`Null`
/// when it is not).
pub(crate) async fn send(router: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let (status, bytes) = read(router, request).await;
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// [`send`] that also returns the body text.
pub(crate) async fn send_text(
    router: &Router,
    request: Request<Body>,
) -> (StatusCode, String, Value) {
    let (status, bytes) = read(router, request).await;
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    (status, text, json)
}

/// Sends a request with `body` as JSON, or none.
pub(crate) async fn call(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    send(router, request(method, uri, body.map(|b| b.to_string()))).await
}

/// [`call`] that also returns the body text.
pub(crate) async fn call_text(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String, Value) {
    send_text(router, request(method, uri, body.map(|b| b.to_string()))).await
}
