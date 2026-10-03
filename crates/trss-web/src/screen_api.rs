//! The remote screen of a job that waits for a person's check on the site
//! (`docs/specs/jobs.md`, 작업 화면 안의 인증과 브라우저 수명).
//!
//! The worker's browser pool brought the post's page to the check in a run of
//! the server browser and bound the run to the job ([`trss_jobs::screen`]).
//! The web shows that page and relays a person's input to it through its own
//! DevTools connection to the run, made through the launcher's
//! token-protected proxy (`TRSS_BROWSER_URL`, `TRSS_BROWSER_TOKEN`); it never
//! starts, resets or ends a run, and never reaches a raw DevTools port. A web
//! without those variables has no remote screens (`unavailable`).
//!
//! # Preparing
//!
//! - `POST /api/subtitle-jobs/{id}/screen`: a person opened the job's page.
//!   For a job that waits for its check (`인증 필요`) with a screen, the
//!   request is written for the worker, which is woken: a live bound run
//!   counts it as use, and a run that is gone has the job go back in line to
//!   bring the page to the check anew. Answers the screen (below) or `null`
//!   (the job has no remote screen); `404` for no job. Nothing else asks for a
//!   run: reading the job (`GET /api/subtitle-jobs/{id}`, whose `screen` is
//!   the same object), the to-do lists and the socket below never do.
//! - The screen: `{ "state", "run", "bound", "note" }`. `state` is `ready` (a
//!   run shows the check: connect to `run`), `preparing` (the worker is
//!   bringing the page to the check), `closed` (no run shows it, `note` says
//!   why: opening the page prepares it again) or `unavailable` (this web has
//!   no server browser). `bound` (with `run`) is when the run was bound to
//!   the job, in milliseconds: another check of the job in the same run is a
//!   new binding with a later `bound`, to connect to anew.
//!
//! # The socket
//!
//! `GET /api/subtitle-jobs/{id}/screen/socket?run=<run>&bound=<bound>` is a
//! WebSocket, one per open screen, bound to the job and to the binding it was
//! opened for (`bound` may be left out: the run's binding now). It is refused
//! before the upgrade, in this order:
//!
//! | Status | When |
//! | --- | --- |
//! | `421` | the `Host` is not one the web answers to ([`crate::origin_guard`], before this module) |
//! | `403` | `Origin` is missing or is not the request's own (another site's page; [`crate::origin_guard`]) |
//! | `503` | this web has no server browser |
//! | `404` | no such job |
//! | `409` | the job does not wait for its check (or has no remote screen) |
//! | `410` | no run is bound to the job, or not the run or binding asked for (it ended) |
//!
//! A socket that is refused or ends never asks for a run: the client asks the
//! job again (and a person reopening the page prepares it).
//!
//! Messages are JSON text. The device sends first its size, then input:
//!
//! - `{"type":"viewport","width","height","dpr","touch"}`: the device's CSS
//!   size (200–4096), its pixel ratio (0.5–4) and whether it is a touch
//!   screen. The page is laid out for it (`Emulation.setDeviceMetricsOverride`
//!   with that ratio and `mobile` = touch, and
//!   `Emulation.setTouchEmulationEnabled`); the browser draws at its own
//!   scale (1). The last device to send its size wins, and every change makes
//!   a new generation `gen`.
//! - `{"type":"mouse","gen","event","x","y","button","buttons","clickCount","modifiers","deltaX","deltaY"}`
//!   (`Input.dispatchMouseEvent`; `event` `mousePressed`, `mouseReleased`,
//!   `mouseMoved`, `mouseWheel`), `{"type":"touch","gen","event","points":[{"x","y","id"}]}`
//!   (`Input.dispatchTouchEvent`), `{"type":"key","gen","event","key","code","text","keyCode","modifiers"}`
//!   (`Input.dispatchKeyEvent`), `{"type":"text","gen","text"}`
//!   (`Input.insertText`). Coordinates are CSS pixels of the frame. An input
//!   whose `gen` is not the current one, or that comes while the size
//!   changes, is dropped and answered `{"type":"dropped","gen":<current>}`;
//!   one out of bounds is ignored.
//! - `{"type":"reload"}`: reloads the page.
//!
//! The web sends:
//!
//! - `{"type":"viewport","gen","width","height","dpr","touch"}` when a change
//!   of size is done (to every socket of the run).
//! - `{"type":"frame","gen","width","height","data"}`: a JPEG frame
//!   (`Page.startScreencast`, base64 in `data`) of the current size. Frames
//!   are acknowledged as they come; one of another size, or one that comes
//!   while the size changes, is not sent. A slow socket skips frames.
//! - `{"type":"ended","reason"}`, then the socket closes: the run's page or
//!   connection is gone (`browser`), the job's binding changed (`run`: the
//!   file came, the run ended, or another run, page or binding took its
//!   place), the run could not be reached (`unreachable`), or a newer
//!   screen of the binding took this one's seat (`replaced`): at most
//!   [`MAX_SOCKETS`] are open on a binding, and one more takes the place of
//!   the oldest (the newest is the device the person is using).
//!
//! The sockets of one binding share one DevTools connection (a hub), which
//! goes when the last of them closes; the hub reads whether its binding is
//! still the job's every [`CHECK_EVERY`]. A socket that does not take what
//! is sent to it within [`SEND_TIMEOUT`] is closed. The web pings every
//! socket every [`PING_EVERY`] and drops one that sends nothing (a pong or
//! any message) within [`PONG_WITHIN`]: a device that slept or changed its
//! network leaves a connection that no send would find dead while the page
//! draws nothing. A person's input counts
//! as use of the run: it is recorded for the worker's idle end at most every
//! [`INPUT_EVERY`]; the frames alone are not use.

mod hub;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use axum::{
    extract::{
        ws::{
            close_code, rejection::WebSocketUpgradeRejection, CloseFrame, Message, WebSocket,
            WebSocketUpgrade,
        },
        Path, Query, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use trss_browser::client::LauncherClient;
use trss_jobs::{Screen, ScreenState};

pub use hub::{admits, Binding, View, Viewport, MAX_SOCKETS};
use hub::{ended_message, Handled, Hub};

use super::{commands_api::now_millis, env::BrowserAccess, ApiError, AppState};

/// How often a person's input is recorded for the worker's idle end.
pub const INPUT_EVERY: Duration = Duration::from_secs(10);
/// How often an open screen checks that its binding is still the job's.
pub const CHECK_EVERY: Duration = Duration::from_secs(2);
/// How long a message may wait for a socket that takes nothing.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the web pings an open screen's socket.
pub const PING_EVERY: Duration = Duration::from_secs(15);
/// How long a ping waits for the socket to answer before it is dropped.
pub const PONG_WITHIN: Duration = Duration::from_secs(10);
/// The largest message a socket takes.
const MAX_MESSAGE: usize = 64 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/subtitle-jobs/{id}/screen", post(prepare))
        .route("/subtitle-jobs/{id}/screen/socket", get(socket))
}

/// The web's way to the server browser's runs, and the hubs of the screens
/// open now. Cheap to clone.
#[derive(Clone)]
pub struct RemoteScreens {
    launcher: Arc<LauncherClient>,
    hubs: Arc<Mutex<HashMap<Binding, Weak<Hub>>>>,
    input_every: Duration,
    check_every: Duration,
    ping_every: Duration,
    pong_within: Duration,
}

impl RemoteScreens {
    pub fn new(access: &BrowserAccess) -> Result<RemoteScreens, String> {
        let launcher = LauncherClient::new(access.url.clone(), access.token.clone())
            .map_err(|e| format!("cannot make the server browser's client: {e}"))?;
        Ok(RemoteScreens {
            launcher: Arc::new(launcher),
            hubs: Arc::default(),
            input_every: INPUT_EVERY,
            check_every: CHECK_EVERY,
            ping_every: PING_EVERY,
            pong_within: PONG_WITHIN,
        })
    }

    /// Other times than [`PING_EVERY`] and [`PONG_WITHIN`] (tests).
    pub fn with_pings(mut self, every: Duration, within: Duration) -> RemoteScreens {
        self.ping_every = every;
        self.pong_within = within;
        self
    }

    /// Other times than [`INPUT_EVERY`] and [`CHECK_EVERY`] (tests).
    pub fn with_times(mut self, input_every: Duration, check_every: Duration) -> RemoteScreens {
        self.input_every = input_every;
        self.check_every = check_every;
        self
    }

    /// The hub of `binding`: the one open, or a new connection to its page.
    async fn hub(
        &self,
        state: &AppState,
        job: &str,
        binding: &Binding,
    ) -> Result<Arc<Hub>, String> {
        if let Some(hub) = self.open_hub(binding) {
            return Ok(hub);
        }
        let hub = Hub::open(
            &self.launcher.cdp_url(&binding.run),
            self.launcher.token(),
            job,
            binding.clone(),
            state.screens.clone(),
            self.input_every,
            self.check_every,
        )
        .await?;
        let mut hubs = self.hubs.lock().expect("hubs lock");
        // Another socket of the binding may have made one meanwhile: one hub
        // per binding, the first one made.
        if let Some(open) = hubs
            .get(binding)
            .and_then(Weak::upgrade)
            .filter(|h| !h.is_ended())
        {
            return Ok(open);
        }
        hubs.retain(|_, hub| hub.strong_count() > 0);
        hubs.insert(binding.clone(), Arc::downgrade(&hub));
        Ok(hub)
    }

    fn open_hub(&self, binding: &Binding) -> Option<Arc<Hub>> {
        self.hubs
            .lock()
            .expect("hubs lock")
            .get(binding)
            .and_then(Weak::upgrade)
            .filter(|hub| !hub.is_ended())
    }
}

/// A job's remote screen as the screens show it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ScreenView {
    pub state: &'static str,
    pub run: Option<String>,
    /// When `run` was bound to the job (ms): tells another check of the job
    /// in the same run.
    pub bound: Option<i64>,
    pub note: Option<String>,
}

const NO_BROWSER: &str =
    "이 웹 서버는 서버 브라우저에 연결돼 있지 않아 원격 화면을 보여줄 수 없어요";

fn view_of(state: &AppState, screen: Screen) -> ScreenView {
    if state.remote.is_none() {
        return ScreenView {
            state: "unavailable",
            run: None,
            bound: None,
            note: Some(NO_BROWSER.to_owned()),
        };
    }
    ScreenView {
        state: screen.state.code(),
        run: screen.run_id,
        bound: screen.bound_at,
        note: screen.note,
    }
}

/// The job's remote screen, or `None` when it has none. Reads only.
pub async fn screen_of(state: &AppState, job: &str) -> Result<Option<ScreenView>, ApiError> {
    Ok(state
        .screens
        .screen(job)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map(|screen| view_of(state, screen)))
}

async fn job_exists(state: &AppState, job: &str) -> Result<bool, ApiError> {
    Ok(state
        .jobs
        .detail(job)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .is_some())
}

async fn prepare(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Option<ScreenView>>, ApiError> {
    let screen = match state.remote {
        // Nothing to prepare a screen for.
        None => state.screens.screen(&id).await,
        Some(_) => state.screens.request_prepare(&id, now_millis()).await,
    }
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    let Some(screen) = screen else {
        if !job_exists(&state, &id).await? {
            return Err(ApiError::not_found("작업을 찾지 못했어요."));
        }
        return Ok(Json(None));
    };
    if state.remote.is_some() && screen.waiting {
        if let Some(path) = &state.worker_wake {
            trss_core::wake::wake_worker(path);
        }
    }
    Ok(Json(Some(view_of(&state, screen))))
}

fn refuse(status: StatusCode, error: &str, message: &str) -> Response {
    (status, Json(json!({ "error": error, "message": message }))).into_response()
}

#[derive(Debug, Deserialize)]
struct SocketQuery {
    #[serde(default)]
    run: Option<String>,
    #[serde(default)]
    bound: Option<i64>,
}

async fn socket(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SocketQuery>,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    // A page of another site cannot open this socket with a person's
    // browser: `crate::origin_guard`, around the whole router, refuses a
    // WebSocket whose `Origin` is missing or not the request's own.
    let Some(remote) = state.remote.clone() else {
        return refuse(StatusCode::SERVICE_UNAVAILABLE, "unavailable", NO_BROWSER);
    };
    let screen = match state.screens.screen(&id).await {
        Ok(screen) => screen,
        Err(e) => return ApiError::Internal(e.to_string()).into_response(),
    };
    let screen = match screen {
        Some(screen) if screen.waiting => screen,
        found => {
            if found.is_none() {
                match job_exists(&state, &id).await {
                    Ok(true) => {}
                    Ok(false) => {
                        return refuse(StatusCode::NOT_FOUND, "not_found", "작업을 찾지 못했어요.")
                    }
                    Err(e) => return e.into_response(),
                }
            }
            return refuse(
                StatusCode::CONFLICT,
                "conflict",
                "이 작업은 사이트 확인을 기다리고 있지 않아요.",
            );
        }
    };
    let (Some(run), Some(target), Some(bound_at), ScreenState::Ready) = (
        screen.run_id.clone(),
        screen.target_id.clone(),
        screen.bound_at,
        screen.state,
    ) else {
        return refuse(
            StatusCode::GONE,
            "gone",
            "이 작업의 서버 브라우저가 닫혔어요. 작업 화면을 다시 열면 다시 준비해요.",
        );
    };
    if query.run.as_deref() != Some(run.as_str())
        || query.bound.is_some_and(|bound| bound != bound_at)
    {
        return refuse(
            StatusCode::GONE,
            "gone",
            "연결하려던 서버 브라우저 실행이 끝났어요. 작업을 다시 읽어 주세요.",
        );
    }
    let upgrade = match upgrade {
        Ok(upgrade) => upgrade,
        Err(rejection) => return rejection.into_response(),
    };
    let binding = Binding {
        run,
        target,
        bound_at,
    };
    upgrade
        .max_message_size(MAX_MESSAGE)
        .on_upgrade(move |socket| serve(socket, state, remote, id, binding))
}

/// Sends `message`, and says whether the socket took it within
/// [`SEND_TIMEOUT`]: a socket that does not is given up.
async fn deliver(socket: &mut WebSocket, message: Message) -> bool {
    matches!(
        tokio::time::timeout(SEND_TIMEOUT, socket.send(message)).await,
        Ok(Ok(()))
    )
}

/// One open screen until it closes, the hub ends or its binding is no longer
/// the job's.
async fn serve(
    mut socket: WebSocket,
    state: AppState,
    remote: RemoteScreens,
    job: String,
    binding: Binding,
) {
    let hub = match remote.hub(&state, &job, &binding).await {
        Ok(hub) => hub,
        Err(err) => {
            eprintln!("trss-web: cannot reach the page of job {job}'s run: {err}");
            let _ = deliver(
                &mut socket,
                Message::Text(ended_message("unreachable").into()),
            )
            .await;
            let _ = deliver(&mut socket, Message::Close(None)).await;
            return;
        }
    };
    let seat = hub.seat();
    let mut out = hub.subscribe();
    let view = hub.view();
    if let Some(v) = view.viewport {
        let hello = json!({
            "type": "viewport", "gen": view.gen, "width": v.width,
            "height": v.height, "dpr": v.dpr, "touch": v.touch,
        });
        if !deliver(&mut socket, Message::Text(hello.to_string().into())).await {
            return;
        }
    }
    let mut ping = tokio::time::interval_at(
        tokio::time::Instant::now() + remote.ping_every,
        remote.ping_every,
    );
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let answer_by = tokio::time::sleep(Duration::ZERO);
    tokio::pin!(answer_by);
    let mut pinged = false;
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(message)) if !matches!(message, Message::Text(_) | Message::Close(_)) => {
                    // A pong, or anything else: the device is there.
                    pinged = false;
                }
                Some(Ok(Message::Text(text))) => {
                    pinged = false;
                    if let Handled::Dropped { gen } = hub.handle(text.as_str()).await {
                        let note = json!({ "type": "dropped", "gen": gen }).to_string();
                        if !deliver(&mut socket, Message::Text(note.into())).await {
                            return;
                        }
                    }
                }
                Some(Ok(_)) | None | Some(Err(_)) => break,
            },
            _ = ping.tick(), if !pinged => {
                if !deliver(&mut socket, Message::Ping(Default::default())).await {
                    return;
                }
                pinged = true;
                answer_by
                    .as_mut()
                    .reset(tokio::time::Instant::now() + remote.pong_within);
            }
            // No answer: a dead connection, dropped without a close.
            _ = &mut answer_by, if pinged => return,
            _ = seat.replaced() => {
                let ended = ended_message("replaced");
                if deliver(&mut socket, Message::Text(ended.into())).await {
                    let close = CloseFrame {
                        code: close_code::NORMAL,
                        reason: "a newer screen of this run took this one's place".into(),
                    };
                    let _ = deliver(&mut socket, Message::Close(Some(close))).await;
                }
                return;
            }
            sent = out.recv() => match sent {
                Ok(message) => {
                    if !deliver(&mut socket, Message::Text(message.as_ref().into())).await {
                        return;
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            },
            _ = hub.ended() => {
                let ended = ended_message(hub.end_reason());
                if !deliver(&mut socket, Message::Text(ended.into())).await {
                    return;
                }
                break;
            }
        }
    }
    let _ = deliver(&mut socket, Message::Close(None)).await;
}

#[cfg(test)]
mod tests;
