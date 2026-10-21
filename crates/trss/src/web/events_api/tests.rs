//! The event stream as a browser meets it: through the whole router, so the
//! `Host` check is in front of it, read frame by frame.

use std::time::Duration;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::Response,
    Router,
};
use http_body_util::BodyExt;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use trss_core::changes::{ChangeKind, Changes};

use super::KEEP_ALIVE;
use crate::web::{self, AppState};

/// How long a test waits for something that should happen at once.
const PATIENCE: Duration = Duration::from_secs(5);

struct App {
    router: Router,
    changes: Changes,
    shutdown: CancellationToken,
    _static_dir: TempDir,
}

fn app_with(changes: Changes) -> App {
    let static_dir = tempfile::tempdir().unwrap();
    std::fs::write(static_dir.path().join("index.html"), "<title>TRSS</title>").unwrap();
    let shutdown = CancellationToken::new();
    let state = AppState::new(trss_core::Db::open_blocking(":memory:").unwrap())
        .with_changes(changes.clone())
        .with_shutdown(shutdown.clone());
    App {
        router: web::router(static_dir.path(), state),
        changes,
        shutdown,
        _static_dir: static_dir,
    }
}

fn app() -> App {
    app_with(Changes::default())
}

impl App {
    /// Opens the stream. The handler has run when this returns, so it listens
    /// to the changes, but nothing of the body was read.
    async fn open(&self) -> Response {
        self.open_as(None).await
    }

    async fn open_as(&self, host: Option<&str>) -> Response {
        let mut request = Request::builder().uri("/api/events");
        if let Some(host) = host {
            request = request.header(header::HOST, host);
        }
        self.router
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }
}

/// One event of a stream: its `event` name and its `data`.
#[derive(Debug, PartialEq, Eq)]
struct Sent {
    event: String,
    data: String,
}

fn sent(event: &str, data: &str) -> Sent {
    Sent {
        event: event.to_owned(),
        data: data.to_owned(),
    }
}

/// Reads a response body as events.
struct Events {
    body: Body,
    pending: String,
    /// How long it waits for the next frame.
    patience: Duration,
}

impl Events {
    fn of(response: Response) -> Self {
        Events {
            body: response.into_body(),
            pending: String::new(),
            patience: PATIENCE,
        }
    }

    /// The next event, skipping comments; `None` when the stream ended.
    async fn next(&mut self) -> Option<Sent> {
        loop {
            if let Some(end) = self.pending.find("\n\n") {
                let block: String = self.pending.drain(..end + 2).collect();
                let mut event = String::new();
                let mut data = Vec::new();
                for line in block.lines() {
                    if let Some(name) = line.strip_prefix("event:") {
                        event = name.trim().to_owned();
                    } else if let Some(text) = line.strip_prefix("data:") {
                        data.push(text.trim().to_owned());
                    }
                }
                if !event.is_empty() || !data.is_empty() {
                    return Some(Sent {
                        event,
                        data: data.join("\n"),
                    });
                }
                continue;
            }
            let frame = tokio::time::timeout(self.patience, self.body.frame())
                .await
                .expect("the stream sent nothing in time")?
                .unwrap();
            if let Ok(data) = frame.into_data() {
                self.pending.push_str(std::str::from_utf8(&data).unwrap());
            }
        }
    }
}

#[tokio::test]
async fn the_route_answers_an_event_stream_that_is_not_cached_or_buffered() {
    let response = app().open().await;

    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(headers[header::CONTENT_TYPE], "text/event-stream");
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    assert_eq!(headers["x-accel-buffering"], "no");
    assert!(headers.get(header::CONTENT_ENCODING).is_none());
    assert!(headers.get(header::CONTENT_LENGTH).is_none());
}

#[tokio::test]
async fn ready_comes_first_and_once_then_the_changes_in_the_order_they_were_announced() {
    let app = app();
    let mut events = Events::of(app.open().await);

    assert_eq!(events.next().await, Some(sent("ready", "{}")));
    app.changes.announce(ChangeKind::Command, "c-1");
    app.changes.announce(ChangeKind::PastSearch, "s-1");
    app.changes.announce(ChangeKind::Command, "c-1");

    assert_eq!(
        events.next().await,
        Some(sent("change", r#"{"kind":"command","id":"c-1"}"#))
    );
    assert_eq!(
        events.next().await,
        Some(sent("change", r#"{"kind":"past_search","id":"s-1"}"#))
    );
    assert_eq!(
        events.next().await,
        Some(sent("change", r#"{"kind":"command","id":"c-1"}"#))
    );
}

#[tokio::test]
async fn a_change_announced_before_the_stream_is_read_is_not_lost() {
    let app = app();
    let response = app.open().await;
    app.changes.announce(ChangeKind::Command, "early");

    let mut events = Events::of(response);

    assert_eq!(events.next().await, Some(sent("ready", "{}")));
    assert_eq!(
        events.next().await,
        Some(sent("change", r#"{"kind":"command","id":"early"}"#))
    );
}

#[tokio::test]
async fn a_change_announced_before_the_stream_opened_is_not_sent() {
    let app = app();
    app.changes.announce(ChangeKind::Command, "before");
    let mut events = Events::of(app.open().await);
    app.changes.announce(ChangeKind::Command, "after");

    assert_eq!(events.next().await, Some(sent("ready", "{}")));
    assert_eq!(
        events.next().await,
        Some(sent("change", r#"{"kind":"command","id":"after"}"#))
    );
}

#[tokio::test]
async fn a_stream_that_fell_behind_says_lagged_and_goes_on() {
    let app = app_with(Changes::with_capacity(2));
    let response = app.open().await;
    // Five changes into a channel that keeps two, before anything is read.
    for n in 0..5 {
        app.changes.announce(ChangeKind::Command, format!("c-{n}"));
    }

    let mut events = Events::of(response);

    assert_eq!(events.next().await, Some(sent("ready", "{}")));
    assert_eq!(events.next().await, Some(sent("lagged", "{}")));
    // What the channel still had follows, and later changes too.
    assert_eq!(
        events.next().await,
        Some(sent("change", r#"{"kind":"command","id":"c-3"}"#))
    );
    assert_eq!(
        events.next().await,
        Some(sent("change", r#"{"kind":"command","id":"c-4"}"#))
    );
    app.changes.announce(ChangeKind::Command, "later");
    assert_eq!(
        events.next().await,
        Some(sent("change", r#"{"kind":"command","id":"later"}"#))
    );
}

#[tokio::test]
async fn every_open_stream_hears_every_change() {
    let app = app();
    let mut first = Events::of(app.open().await);
    let mut second = Events::of(app.open().await);

    app.changes.announce(ChangeKind::PastSearch, "s-1");

    for events in [&mut first, &mut second] {
        assert_eq!(events.next().await, Some(sent("ready", "{}")));
        assert_eq!(
            events.next().await,
            Some(sent("change", r#"{"kind":"past_search","id":"s-1"}"#))
        );
    }
}

#[tokio::test]
async fn a_host_the_web_does_not_answer_to_is_refused_like_on_every_other_route() {
    let app = app();

    let refused = app.open_as(Some("attacker.example")).await;
    assert_eq!(refused.status(), StatusCode::MISDIRECTED_REQUEST);
    assert_ne!(
        refused.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/event-stream"
    );

    let served = app.open_as(Some("192.168.0.10:8080")).await;
    assert_eq!(served.status(), StatusCode::OK);
    assert_eq!(served.headers()[header::CONTENT_TYPE], "text/event-stream");
}

#[tokio::test]
async fn the_stream_ends_when_the_process_shuts_down() {
    let app = app();
    let mut events = Events::of(app.open().await);
    assert_eq!(events.next().await, Some(sent("ready", "{}")));

    app.shutdown.cancel();

    assert_eq!(events.next().await, None);
}

#[tokio::test]
async fn a_stream_opened_after_the_shutdown_began_ends_after_ready() {
    let app = app();
    app.shutdown.cancel();
    let mut events = Events::of(app.open().await);

    assert_eq!(events.next().await, Some(sent("ready", "{}")));
    assert_eq!(events.next().await, None);
}

/// The clock is paused: it moves only when everything waits, so the test
/// takes no real time and the interval is measured on it exactly.
#[tokio::test(start_paused = true)]
async fn an_idle_stream_sends_alive_every_interval_and_a_change_goes_between() {
    let app = app();
    let mut events = Events::of(app.open().await);
    events.patience = Duration::from_secs(60);
    assert_eq!(events.next().await, Some(sent("ready", "{}")));

    let waited = tokio::time::Instant::now();
    assert_eq!(events.next().await, Some(sent("alive", "{}")));
    assert!(waited.elapsed() >= KEEP_ALIVE, "{:?}", waited.elapsed());
    assert!(waited.elapsed() < KEEP_ALIVE * 2, "{:?}", waited.elapsed());

    app.changes.announce(ChangeKind::Command, "c1");
    assert_eq!(
        events.next().await,
        Some(sent("change", r#"{"kind":"command","id":"c1"}"#))
    );
    assert_eq!(events.next().await, Some(sent("alive", "{}")));
}
