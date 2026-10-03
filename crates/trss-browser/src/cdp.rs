//! A small Chrome DevTools Protocol client over the launcher's proxy.
//!
//! One WebSocket carries commands (an `id`, a `method`, `params`, and a
//! `sessionId` for a target the connection attached to in flatten mode) and
//! their answers, and the events the browser sends. [`Connection::command`]
//! sends one and waits for its answer; [`Connection::events`] is a feed of
//! every event of the browser and of its sessions.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, Message};
use tokio_util::sync::{CancellationToken, DropGuard};

/// How long a command may wait for its answer.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
/// How many events a slow reader may fall behind by.
const EVENT_BACKLOG: usize = 4096;

#[derive(Debug, Clone, thiserror::Error)]
pub enum CdpError {
    #[error("the DevTools connection is closed")]
    Closed,
    #[error("DevTools refused {method} ({code}): {message}")]
    Command {
        method: String,
        code: i64,
        message: String,
    },
    #[error("DevTools did not answer {0} in time")]
    Timeout(String),
    #[error("cannot connect to the browser's DevTools: {0}")]
    Connect(String),
    #[error("the browser run is not there")]
    NotFound,
}

/// An event of the browser or of one of its sessions.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub method: String,
    pub params: Value,
    /// The session the event came from; `None` for the browser itself.
    pub session_id: Option<String>,
}

type Pending = oneshot::Sender<Result<Value, CdpError>>;

struct Shared {
    next_id: AtomicU64,
    /// The commands waiting for an answer, with the method for the error.
    pending: Mutex<HashMap<u64, (String, Pending)>>,
    events: broadcast::Sender<Event>,
    out: mpsc::UnboundedSender<String>,
    /// Cancelled when the connection is over, from either side.
    closed: CancellationToken,
}

/// A connection to a run's browser-level DevTools socket. Cheap to clone; the
/// connection closes when the last clone is dropped or [`Connection::close`]
/// is called.
#[derive(Clone)]
pub struct Connection {
    shared: Arc<Shared>,
    _close_on_drop: Arc<DropGuard>,
}

impl Connection {
    /// Connects to `url` (`ws://…/runs/{id}/cdp` of the launcher) with the
    /// launcher's bearer `token`.
    pub async fn connect(url: &str, token: &str) -> Result<Connection, CdpError> {
        let mut request = url
            .into_client_request()
            .map_err(|e| CdpError::Connect(e.to_string()))?;
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {token}")
                .parse()
                .map_err(|_| CdpError::Connect("the token is not a header value".into()))?,
        );
        let config = tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(256 << 20))
            .max_frame_size(Some(64 << 20));
        let (socket, _) =
            tokio_tungstenite::connect_async_with_config(request, Some(config), false)
                .await
                .map_err(|e| match e {
                    tungstenite::Error::Http(response) if response.status() == 404 => {
                        CdpError::NotFound
                    }
                    tungstenite::Error::Http(response) => {
                        CdpError::Connect(format!("the launcher answered {}", response.status()))
                    }
                    other => CdpError::Connect(other.to_string()),
                })?;

        let closed = CancellationToken::new();
        let (out, out_rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(EVENT_BACKLOG);
        let shared = Arc::new(Shared {
            next_id: AtomicU64::new(1),
            pending: Mutex::default(),
            events,
            out,
            closed: closed.clone(),
        });
        tokio::spawn(io(socket, shared.clone(), out_rx));
        Ok(Connection {
            shared,
            _close_on_drop: Arc::new(closed.drop_guard()),
        })
    }

    /// Sends `method` (in `session`, when given) and waits for its answer.
    pub async fn command(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, CdpError> {
        self.command_within(session, method, params, COMMAND_TIMEOUT)
            .await
    }

    pub async fn command_within(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, CdpError> {
        if self.shared.closed.is_cancelled() {
            return Err(CdpError::Closed);
        }
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = Value::String(session.to_owned());
        }
        let (tx, rx) = oneshot::channel();
        self.shared
            .pending
            .lock()
            .expect("pending lock")
            .insert(id, (method.to_owned(), tx));
        if self.shared.out.send(message.to_string()).is_err() {
            self.forget(id);
            return Err(CdpError::Closed);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err(CdpError::Closed),
            Err(_) => {
                self.forget(id);
                Err(CdpError::Timeout(method.to_owned()))
            }
        }
    }

    fn forget(&self, id: u64) {
        self.shared
            .pending
            .lock()
            .expect("pending lock")
            .remove(&id);
    }

    /// A feed of the events from now on.
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.shared.events.subscribe()
    }

    pub fn is_closed(&self) -> bool {
        self.shared.closed.is_cancelled()
    }

    /// Resolves when the connection is over.
    pub async fn closed(&self) {
        self.shared.closed.cancelled().await;
    }

    pub fn close(&self) {
        self.shared.closed.cancel();
    }
}

impl Shared {
    fn dispatch(&self, text: &str) {
        let Ok(mut message) = serde_json::from_str::<Value>(text) else {
            return;
        };
        if let Some(id) = message.get("id").and_then(Value::as_u64) {
            let Some((method, tx)) = self.pending.lock().expect("pending lock").remove(&id) else {
                return;
            };
            let answer = match message.get("error") {
                Some(error) => Err(CdpError::Command {
                    method,
                    code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                }),
                None => Ok(message["result"].take()),
            };
            let _ = tx.send(answer);
        } else if let Some(method) = message.get("method").and_then(Value::as_str) {
            let _ = self.events.send(Event {
                method: method.to_owned(),
                params: message["params"].take(),
                session_id: message
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
    }

    fn fail_pending(&self) {
        let pending = std::mem::take(&mut *self.pending.lock().expect("pending lock"));
        for (_, (_, tx)) in pending {
            let _ = tx.send(Err(CdpError::Closed));
        }
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Reads the socket into the pending commands and the event feed, and writes
/// what the commands send, until either side closes.
async fn io(socket: Socket, shared: Arc<Shared>, mut out: mpsc::UnboundedReceiver<String>) {
    let (mut sink, mut stream) = socket.split();
    loop {
        tokio::select! {
            biased;
            _ = shared.closed.cancelled() => break,
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => shared.dispatch(text.as_str()),
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            outgoing = out.recv() => match outgoing {
                Some(text) => {
                    if sink.send(Message::text(text)).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
        }
    }
    shared.closed.cancel();
    shared.fail_pending();
    let _ = sink.close().await;
}
