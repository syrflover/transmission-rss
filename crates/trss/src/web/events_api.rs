//! `GET /api/events`: tells a screen what changed, as server-sent events.
//!
//! The screens used to ask the server again and again whether a command had
//! ended or a search had got further. They now keep this one stream open and
//! read again, through the routes they always read from, when it says that
//! the thing they show changed (`docs/requirements/web-app.md`, 웹 명령과 상태 갱신).
//! The stream carries no data of the things: only their kind and ID, so the
//! data has one source, the route that serves it.
//!
//! | event     | `data`                              | meaning                                              |
//! | --------- | ----------------------------------- | ---------------------------------------------------- |
//! | `ready`   | `{}`                                | sent first, once, after the stream listens to the changes; the screen reads again what it shows, so what changed before the stream listened is not lost |
//! | `change`  | `{"kind":"command","id":"…"}`       | the thing `id` of `kind` changed ([`trss_core::changes::Change`]); `id` is `null` when the changed row names no such thing, which says that anything of `kind` may have changed |
//! | `lagged`  | `{}`                                | the stream fell behind and missed changes; the screen reads again everything it shows |
//! | `alive`   | `{}`                                | sent every [`KEEP_ALIVE`] while nothing else is; the screen reads nothing again, and opens the stream anew if it hears nothing at all for three intervals (45 s) on its side (`web/src/lib/events.ts`) |
//!
//! Every event has JSON `data`: a browser dispatches no event whose data is
//! empty. `alive` is an event and not an SSE comment because a comment never
//! reaches a page: a connection that died without a close (a laptop that slept
//! through a restart, a NAT that forgot it) would look open forever. The stream stays open after `lagged`. It ends when the browser
//! closes it and when the process shuts down ([`AppState::shutdown`]): a
//! stream left open would keep the server from stopping until its grace period
//! ran out. A browser that was cut off opens the stream again by itself and
//! gets `ready` again.
//!
//! The stream is not compressed or buffered by this server (nothing in the
//! router compresses a response), and `X-Accel-Buffering: no` asks a reverse
//! proxy that buffers by default (nginx) to pass it on at once. It is behind
//! the `Host` check of [`super::origin_guard`] like every route.

use std::{convert::Infallible, time::Duration};

use axum::{
    extract::State,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::get,
    Router,
};
use futures::stream;
use serde::Serialize;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::CancellationToken;
use trss_core::changes::Change;

use super::AppState;

/// How often an `alive` event keeps an idle stream from being closed by a
/// proxy or the browser's network stack, and lets the page tell a stream that
/// died silently from one that is idle.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

pub fn routes() -> Router<AppState> {
    Router::new().route("/events", get(events))
}

async fn events(State(state): State<AppState>) -> impl IntoResponse {
    // Listening starts here, before the response is sent: a change announced
    // after this is kept for the stream however late it is first read, and
    // the `ready` it sends first comes after listening began.
    let feed = Feed {
        changes: state.changes.subscribe(),
        shutdown: state.shutdown.clone(),
        ready: false,
    };
    (
        [("x-accel-buffering", "no")],
        Sse::new(stream::unfold(feed, Feed::next)).keep_alive(
            KeepAlive::new()
                .interval(KEEP_ALIVE)
                .event(StreamEvent::Alive(NoData {}).sse()),
        ),
    )
}

/// What one open stream has: where its changes come from, the end of the
/// process, and whether `ready` was sent.
struct Feed {
    changes: broadcast::Receiver<Change>,
    shutdown: CancellationToken,
    ready: bool,
}

impl Feed {
    /// The next event, or `None` when the stream ends.
    async fn next(mut self) -> Option<(Result<Event, Infallible>, Feed)> {
        if !self.ready {
            self.ready = true;
            return Some((Ok(StreamEvent::Ready(NoData {}).sse()), self));
        }
        let event = tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => return None,
            heard = self.changes.recv() => match heard {
                Ok(change) => StreamEvent::Change(change).sse(),
                Err(RecvError::Lagged(_)) => StreamEvent::Lagged(NoData {}).sse(),
                Err(RecvError::Closed) => return None,
            },
        };
        Some((Ok(event), self))
    }
}

/// The data of an event with no content of its own, which still has data to
/// dispatch.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "Record<string, never>"))]
pub struct NoData {}

/// An event of the stream: its name (`event`) and its JSON `data`, as the web
/// reads them. The web's `StreamEvent` is generated from it.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "event", content = "data", rename_all = "lowercase")]
pub enum StreamEvent {
    Ready(NoData),
    Change(Change),
    Lagged(NoData),
    Alive(NoData),
}

impl StreamEvent {
    /// The event's name and its JSON data as the server sends them. The name
    /// is the serialized `event`, so the wire and the generated type share
    /// one source.
    pub(super) fn wire(&self) -> (String, String) {
        let tagged = serde_json::to_value(self).expect("an event serializes");
        let name = tagged["event"]
            .as_str()
            .expect("an event is tagged with its name")
            .to_owned();
        let data = match self {
            Self::Ready(none) | Self::Lagged(none) | Self::Alive(none) => {
                serde_json::to_string(none)
            }
            Self::Change(change) => serde_json::to_string(change),
        };
        (name, data.expect("an event's data serializes"))
    }

    fn sse(&self) -> Event {
        let (name, data) = self.wire();
        Event::default().event(name).data(data)
    }
}

#[cfg(test)]
mod tests;
