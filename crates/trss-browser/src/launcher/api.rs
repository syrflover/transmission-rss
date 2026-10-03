//! The launcher's HTTP side (see [`crate::protocol`] for the requests).
//!
//! Every route, the WebSocket upgrade included, is behind the bearer token.
//! The token is compared in constant time and never logged.

use axum::{
    extract::{
        ws::{CloseFrame, Message as ClientMessage, WebSocket, WebSocketUpgrade},
        Path, Request, State,
    },
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message as UpstreamMessage;

use super::{Launcher, Run, StartError};
use crate::protocol::{is_valid_run_id, ErrorBody, RunList, StartRequest, Started};

/// The launcher's routes over `launcher`.
pub fn router(launcher: Launcher) -> Router {
    Router::new()
        .route("/runs", post(start_run).get(list_runs))
        .route("/runs/{id}", delete(end_run))
        .route("/runs/{id}/cdp", get(proxy_cdp))
        .route("/reset", post(reset))
        .layer(middleware::from_fn_with_state(
            launcher.clone(),
            require_token,
        ))
        .with_state(launcher)
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorBody {
            error: message.into(),
        }),
    )
        .into_response()
}

/// Whether `given` is `expected`, in a time that does not depend on how much
/// of it matches: both are hashed first, so even the length stays private.
fn same_token(given: &str, expected: &str) -> bool {
    let a = Sha256::digest(given.as_bytes());
    let b = Sha256::digest(expected.as_bytes());
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

async fn require_token(State(launcher): State<Launcher>, request: Request, next: Next) -> Response {
    match bearer(request.headers()) {
        Some(given) if same_token(given, &launcher.config().token) => next.run(request).await,
        _ => {
            let mut response = error(StatusCode::UNAUTHORIZED, "unauthorized");
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                "Bearer".parse().expect("a header value"),
            );
            response
        }
    }
}

async fn start_run(
    State(launcher): State<Launcher>,
    Json(request): Json<StartRequest>,
) -> Response {
    match launcher.start(&request.run).await {
        Ok(run) => Json(Started {
            run: run.id.clone(),
            downloads: run.downloads.to_string_lossy().into_owned(),
        })
        .into_response(),
        Err(err) => {
            let status = match err {
                StartError::InvalidId => StatusCode::BAD_REQUEST,
                StartError::TooManyRuns(_) => StatusCode::TOO_MANY_REQUESTS,
                StartError::Ending | StartError::Aborted => StatusCode::CONFLICT,
                StartError::DisplayDown => StatusCode::SERVICE_UNAVAILABLE,
                StartError::NotReady => StatusCode::GATEWAY_TIMEOUT,
                StartError::Spawn(_) | StartError::Exited(_) | StartError::Io(_) => {
                    StatusCode::BAD_GATEWAY
                }
            };
            if status.is_server_error() {
                eprintln!("Browser launcher: cannot start a run: {err}");
            }
            error(status, err.to_string())
        }
    }
}

async fn list_runs(State(launcher): State<Launcher>) -> Json<RunList> {
    Json(RunList {
        runs: launcher.list(),
    })
}

async fn end_run(State(launcher): State<Launcher>, Path(id): Path<String>) -> Response {
    if !is_valid_run_id(&id) {
        return error(StatusCode::BAD_REQUEST, StartError::InvalidId.to_string());
    }
    launcher.end(&id).await;
    StatusCode::NO_CONTENT.into_response()
}

async fn reset(State(launcher): State<Launcher>) -> StatusCode {
    launcher.reset().await;
    StatusCode::NO_CONTENT
}

async fn proxy_cdp(
    State(launcher): State<Launcher>,
    Path(id): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let Some(run) = launcher.get(&id) else {
        return error(StatusCode::NOT_FOUND, "no such run");
    };
    upgrade.on_upgrade(move |socket| relay(socket, run))
}

/// Relays frames between the worker's socket and the run's DevTools socket
/// until either closes or the run ends. The run ending closes both.
async fn relay(client: WebSocket, run: std::sync::Arc<Run>) {
    let upstream = match tokio_tungstenite::connect_async(run.ws_url.as_str()).await {
        Ok((socket, _)) => socket,
        Err(_) => {
            let mut client = client;
            let _ = client
                .send(ClientMessage::Close(Some(CloseFrame {
                    code: 1011,
                    reason: "the browser's DevTools socket cannot be reached".into(),
                })))
                .await;
            return;
        }
    };
    let (mut to_client, mut from_client) = client.split();
    let (mut to_browser, mut from_browser) = upstream.split();

    loop {
        tokio::select! {
            biased;
            _ = run.ended.cancelled() => break,
            message = from_client.next() => match message.map(|m| m.map(to_upstream)) {
                Some(Ok(Forward::Send(message))) => {
                    if to_browser.send(message).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Forward::Skip)) => {}
                Some(Ok(Forward::Stop)) | Some(Err(_)) | None => break,
            },
            message = from_browser.next() => match message.map(|m| m.map(to_client_message)) {
                Some(Ok(Forward::Send(message))) => {
                    if to_client.send(message).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Forward::Skip)) => {}
                Some(Ok(Forward::Stop)) | Some(Err(_)) | None => break,
            },
        }
    }
    let _ = to_client
        .send(ClientMessage::Close(Some(CloseFrame {
            code: 1000,
            reason: "".into(),
        })))
        .await;
    let _ = to_browser.close().await;
}

/// What to do with a frame of one side.
enum Forward<M> {
    Send(M),
    /// A ping or pong: each side answers for itself.
    Skip,
    Stop,
}

fn to_upstream(message: ClientMessage) -> Forward<UpstreamMessage> {
    match message {
        ClientMessage::Text(text) => Forward::Send(UpstreamMessage::text(text.as_str().to_owned())),
        ClientMessage::Binary(bytes) => Forward::Send(UpstreamMessage::Binary(bytes)),
        ClientMessage::Ping(_) | ClientMessage::Pong(_) => Forward::Skip,
        ClientMessage::Close(_) => Forward::Stop,
    }
}

fn to_client_message(message: UpstreamMessage) -> Forward<ClientMessage> {
    match message {
        UpstreamMessage::Text(text) => Forward::Send(ClientMessage::Text(text.as_str().into())),
        UpstreamMessage::Binary(bytes) => Forward::Send(ClientMessage::Binary(bytes)),
        UpstreamMessage::Ping(_) | UpstreamMessage::Pong(_) | UpstreamMessage::Frame(_) => {
            Forward::Skip
        }
        UpstreamMessage::Close(_) => Forward::Stop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_compare_by_value() {
        assert!(same_token("abc", "abc"));
        assert!(!same_token("abc", "abd"));
        assert!(!same_token("abc", "abcd"));
        assert!(!same_token("", "abc"));
    }

    #[test]
    fn the_bearer_scheme_is_required() {
        let mut headers = HeaderMap::new();
        assert_eq!(bearer(&headers), None);
        headers.insert(header::AUTHORIZATION, "Basic abc".parse().unwrap());
        assert_eq!(bearer(&headers), None);
        headers.insert(header::AUTHORIZATION, "Bearer abc".parse().unwrap());
        assert_eq!(bearer(&headers), Some("abc"));
    }
}
