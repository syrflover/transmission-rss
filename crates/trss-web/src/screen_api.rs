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
//!   new binding with a later `bound`, to connect to anew. `popup` (with
//!   `run`) says the page shown is not the one the run was bound with: a
//!   screen follows a page the run opened (a popup), for a find job and a
//!   site's check alike. `limits` (with `run`) is what the socket takes, for
//!   the device to keep within: `{ "min_side", "max_side", "min_dpr",
//!   "max_dpr", "max_message_bytes", "max_prompt_text" }`. The device needs
//!   the viewport before the socket exists, so this answer is the earliest
//!   one that has them.
//!
//! # Tabs: switching and closing
//!
//! The worker lists the pages of a run that stayed, in the order they came
//! (the tabs); the web reads their titles and hosts from the browser. A
//! person switches to a page and closes one with requests the worker answers,
//! since it owns the runs; the web itself sends the browser nothing for them.
//! Both name the binding the person sees (`run`, `bound`) and count as the
//! person's input for the run's idle end.
//!
//! - `POST /api/subtitle-jobs/{id}/screen/switch` `{ "run", "bound", "target" }`:
//!   show the page `target`. Written when the binding is still the job's and
//!   `target` is one of the listed pages. The worker moves the screen there
//!   when the page is still open, as a new binding (`ended` `run`).
//! - `POST /api/subtitle-jobs/{id}/screen/close` `{ "run", "bound", "target"? }`:
//!   close the page `target`, shown or not (the page shown when it is left
//!   out). Written when the binding is still the job's, `target` is one of
//!   the listed pages and it is not the run's first page, which is never
//!   closed (the worker refuses it too, `Target.closeTarget` is sent for no
//!   other). Closing a page that is not shown leaves the screen as it is;
//!   closing the one shown moves it to the newest page left, a new binding.
//!
//! Both answer `202` (with `{}`) when asked, `409` when the job's screen is no longer that
//! binding or has no such page to switch to or close, `404` for no job.
//!
//! # Starting the run anew
//!
//! `POST /api/subtitle-jobs/{id}/screen/restart` `{ "run", "bound" }`: a
//! person asks for a new server browser run when the page shown does not
//! answer (`page`, `stuck` below). Written when the binding is still the
//! job's, and the worker is woken: it ends the run (once no download of it
//! is on its way) and brings the job back to the post in a new run, a new
//! binding. The open screens end as the run goes (`ended` `browser`), or
//! as the binding changes (`run`) when that comes first; the web itself
//! ends no run. It counts as the
//! person's input. Answers `202` (with `{}`), `409` when the job's screen is
//! no longer that binding, `404` for no job.
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
//! - `{"type":"back"}`, `{"type":"forward"}`: a step back or forward in the
//!   history of the page shown (`Page.navigateToHistoryEntry`). A step back
//!   never goes to the blank page (`about:blank`) the server passed through
//!   while it prepared the page: the first history entry whose address is not
//!   blank is the floor, judged here on the page's own history at every
//!   request, whatever `nav` last said. A step that is not allowed is
//!   ignored. They count as input.
//! - `{"type":"dialog","id","accept","text"}`: the answer to the dialog
//!   `id` the page shows (`Page.handleJavaScriptDialog`): `accept` is `확인`
//!   (`떠나기` for a `beforeunload`), and `text` (up to 2,000 characters) is
//!   what a prompt that is accepted answers. An answer to another dialog than
//!   the one shown is ignored. It is taken at once, not behind the inputs,
//!   and counts as input.
//!
//! The web sends:
//!
//! - `{"type":"viewport","gen","width","height","dpr","touch"}` when a change
//!   of size is done (to every socket of the run).
//! - `{"type":"frame","gen","width","height","data"}`: a JPEG frame
//!   (`Page.startScreencast`, base64 in `data`) of the current size. Frames
//!   are acknowledged as they come; one of another size, or one that comes
//!   while the size changes, is not sent. A slow socket skips frames.
//! - `{"type":"nav","back","forward","host"}`: whether a step back and a step
//!   forward is allowed on the page shown, and its host (`null` for a page
//!   with none: `about:blank`, `data:`, `blob:`). Sent when a socket connects
//!   and whenever it changes (the page navigated).
//! - `{"type":"tabs","tabs":[{"id","title","host","shown","closable"}]}`: the
//!   run's tabs in the order the worker listed them, the page shown marked
//!   (`shown`), `closable` false for the run's first page. `title` is cut to
//!   40 characters and empty when the page has none or it may name an
//!   address (below); `host` is `null` as above.
//!   A listed page the browser does not report is left out, a page the worker
//!   did not list never appears, and `id` is the name a switch or close
//!   request uses. Sent like `nav`.
//! - `{"type":"page","responding"}`: `false` once the page did not answer an
//!   input, a reload or a change of size within [`ANSWER_WITHIN`], `true`
//!   once it answers again. Meanwhile inputs are not sent to it, and a size
//!   sent is applied when it answers (a new generation). Sent like `nav`
//!   (not at all while the page never stalled).
//! - `{"type":"dialog","dialog":{"id","kind","message","host","prompt"}}`:
//!   the page shows a dialog (`Page.javascriptDialogOpening`), which no frame
//!   draws. `kind` is `alert`, `confirm`, `prompt` or `beforeunload`;
//!   `message` is cut to 1,000 characters and `prompt` (a prompt's default
//!   text) to 2,000; `host` is the host of the frame that opened it (`null`
//!   as above). While it shows, inputs, reloads and steps are not sent to
//!   the page and a size is applied once it closed. `{"type":"dialog",
//!   "dialog":null}` once it closed. Sent like `nav`. A `beforeunload` that
//!   comes within 3 s of a person's reload or step is answered `leave` by the
//!   web and not sent. The message is never logged or stored.
//! - `{"type":"ended","reason"}`, then the socket closes: the run's page or
//!   connection is gone (`browser`), the job's binding changed (`run`: the
//!   file came, the run ended, or another run, page or binding took its
//!   place), the run could not be reached (`unreachable`), the page did not
//!   answer within [`OPEN_WITHIN`] when the screen opened (`stuck`), or a
//!   newer screen of the binding took this one's seat (`replaced`): at most
//!   [`MAX_SOCKETS`] are open on a binding, and one more takes the place of
//!   the oldest (the newest is the device the person is using).
//!
//! Only a page's host is ever sent, never a path, query or fragment: an
//! address can carry a signed link or a token. None is logged or stored
//! either. A title that may name an address (the page's own, as the browser
//! writes it for a page with no title, or any other) is sent empty (the
//! `nav` module).
//!
//! The sockets of one binding share one DevTools connection (a hub), which
//! goes when the last of them closes; the hub reads whether its binding is
//! still the job's every [`CHECK_EVERY`]. A page that does not answer in time
//! does not end the hub: it is asked every [`PROBE_EVERY`] whether it answers
//! again (`page`, above). A socket's messages are taken in order apart from
//! the socket itself, so one the page does not answer holds back neither the
//! pings nor what is sent to it; moves of the pointer that pile up
//! meanwhile are let go first, and a press, a release, a key, a size, a
//! reload or a step only once all [`QUEUE`] places are taken. A socket
//! that is over leaves the rest of its messages untaken. A socket that does not take what
//! is sent to it within [`SEND_TIMEOUT`] is closed. The web pings every
//! socket every [`PING_EVERY`] and drops one that sends nothing (a pong or
//! any message) within [`PONG_WITHIN`]: a device that slept or changed its
//! network leaves a connection that no send would find dead while the page
//! draws nothing. A person's input counts
//! as use of the run: it is recorded for the worker's idle end at most every
//! [`INPUT_EVERY`]; the frames alone are not use.

mod hub;
mod nav;

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
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;
use trss_browser::client::LauncherClient;
use trss_jobs::Screen;

pub use hub::{admits, Binding, Times, View, Viewport, MAX_SOCKETS};
use hub::{
    ended_message, Handled, Hub, Incoming, OpenError, DIALOG_TEXT, MAX_DPR, MAX_SIDE, MIN_DPR,
    MIN_SIDE,
};

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
/// How long a command to the page may wait for its answer before the page is
/// taken as stalled.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(5);
/// How long the page may take to answer when a screen opens on it.
pub const OPEN_WITHIN: Duration = Duration::from_secs(10);
/// How often a stalled page is asked whether it answers again.
pub const PROBE_EVERY: Duration = Duration::from_secs(1);
/// The largest message a socket takes.
const MAX_MESSAGE: usize = 64 * 1024;
/// How many messages of a socket wait to be taken.
const QUEUE: usize = 64;
/// The places of the queue that moves of the pointer do not take
/// ([`Incoming::is_motion`]).
const KEPT_FROM_MOTION: usize = 16;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/subtitle-jobs/{id}/screen", post(prepare))
        .route("/subtitle-jobs/{id}/screen/socket", get(socket))
        .route("/subtitle-jobs/{id}/screen/close", post(close))
        .route("/subtitle-jobs/{id}/screen/switch", post(switch))
        .route("/subtitle-jobs/{id}/screen/restart", post(restart))
}

/// The web's way to the server browser's runs, and the hubs of the screens
/// open now. Cheap to clone.
#[derive(Clone)]
pub struct RemoteScreens {
    launcher: Arc<LauncherClient>,
    hubs: Arc<Mutex<HashMap<Binding, Weak<Hub>>>>,
    times: Times,
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
            times: Times {
                input_every: INPUT_EVERY,
                check_every: CHECK_EVERY,
                answer_within: ANSWER_WITHIN,
                open_within: OPEN_WITHIN,
                probe_every: PROBE_EVERY,
            },
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
        self.times.input_every = input_every;
        self.times.check_every = check_every;
        self
    }

    /// Other times than [`ANSWER_WITHIN`], [`OPEN_WITHIN`] and
    /// [`PROBE_EVERY`] (tests).
    pub fn with_answers(
        mut self,
        answer_within: Duration,
        open_within: Duration,
        probe_every: Duration,
    ) -> RemoteScreens {
        self.times.answer_within = answer_within;
        self.times.open_within = open_within;
        self.times.probe_every = probe_every;
        self
    }

    /// The hub of `binding`: the one open, or a new connection to its page.
    async fn hub(
        &self,
        state: &AppState,
        job: &str,
        binding: &Binding,
    ) -> Result<Arc<Hub>, OpenError> {
        if let Some(hub) = self.open_hub(binding) {
            return Ok(hub);
        }
        let hub = Hub::open(
            &self.launcher.cdp_url(&binding.run),
            self.launcher.token(),
            job,
            binding.clone(),
            state.screens.clone(),
            self.times,
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
    /// With `run`: the page shown is a page the post opened, which a person
    /// may close (`POST .../screen/close`).
    pub popup: bool,
    /// With `run`: what the socket takes, for the device to keep within.
    pub limits: Option<ScreenLimits>,
}

/// What the socket of a screen takes, which a device keeps within so that the
/// server need not refuse it: the screen draws no copy of these numbers.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct ScreenLimits {
    /// The least width or height (CSS pixels) of the viewport it takes.
    pub min_side: u32,
    /// The most width or height of the viewport.
    pub max_side: u32,
    /// The least pixel ratio of the viewport.
    pub min_dpr: f64,
    /// The most pixel ratio of the viewport.
    pub max_dpr: f64,
    /// The most bytes of one message of the socket.
    pub max_message_bytes: usize,
    /// The most characters of a prompt's answer.
    pub max_prompt_text: usize,
}

const LIMITS: ScreenLimits = ScreenLimits {
    min_side: MIN_SIDE,
    max_side: MAX_SIDE,
    min_dpr: MIN_DPR,
    max_dpr: MAX_DPR,
    max_message_bytes: MAX_MESSAGE,
    max_prompt_text: DIALOG_TEXT,
};

const NO_BROWSER: &str =
    "이 웹 서버는 서버 브라우저에 연결돼 있지 않아 원격 화면을 보여줄 수 없어요";

fn view_of(state: &AppState, screen: Screen) -> ScreenView {
    if state.remote.is_none() {
        return ScreenView {
            state: "unavailable",
            run: None,
            bound: None,
            note: Some(NO_BROWSER.to_owned()),
            popup: false,
            limits: None,
        };
    }
    ScreenView {
        state: screen.state.code(),
        limits: screen.run_id.is_some().then_some(LIMITS),
        run: screen.run_id,
        bound: screen.bound_at,
        note: screen.note,
        popup: screen.popup,
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
        state.wake_worker();
    }
    Ok(Json(Some(view_of(&state, screen))))
}

/// The binding of the screen a person asks to close a page of, and the page
/// (the one shown, when left out).
#[derive(Debug, Deserialize)]
struct CloseRequest {
    run: String,
    bound: i64,
    #[serde(default)]
    target: Option<String>,
}

async fn close(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<CloseRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let asked = state
        .screens
        .request_close(
            &id,
            &request.run,
            request.bound,
            request.target.as_deref(),
            now_millis(),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    answered(&state, &id, asked, NO_PAGE_TO_CLOSE).await
}

/// The binding of the screen a person asks to show another page of, and the
/// page.
#[derive(Debug, Deserialize)]
struct SwitchRequest {
    run: String,
    bound: i64,
    target: String,
}

async fn switch(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<SwitchRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let asked = state
        .screens
        .request_switch(
            &id,
            &request.run,
            request.bound,
            &request.target,
            now_millis(),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    answered(&state, &id, asked, NO_PAGE_TO_SHOW).await
}

/// The binding of the screen whose run a person asks to start anew.
#[derive(Debug, Deserialize)]
struct RestartRequest {
    run: String,
    bound: i64,
}

async fn restart(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<RestartRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let asked = state
        .screens
        .request_restart(&id, &request.run, request.bound, now_millis())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if asked {
        state.wake_worker();
    }
    answered(&state, &id, asked, NO_RUN_TO_RESTART).await
}

const NO_RUN_TO_RESTART: &str =
    "새로 띄울 서버 브라우저가 없어요. 화면이 바뀌었으면 새로 연 화면에서 다시 시도해 주세요.";
const NO_PAGE_TO_CLOSE: &str =
    "닫을 창이 없어요. 화면이 바뀌었으면 새로 연 화면에서 다시 시도해 주세요.";
const NO_PAGE_TO_SHOW: &str =
    "보여줄 창이 없어요. 화면이 바뀌었으면 새로 연 화면에서 다시 시도해 주세요.";

/// `202` for a request that was written, else `404` for no job and `409`
/// (`message`) when the screen is not what the person saw.
async fn answered(
    state: &AppState,
    job: &str,
    asked: bool,
    message: &str,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if asked {
        // A body, as the screens read every answer of the API as JSON.
        return Ok((StatusCode::ACCEPTED, Json(json!({}))));
    }
    if !job_exists(state, job).await? {
        return Err(ApiError::not_found("작업을 찾지 못했어요."));
    }
    Err(ApiError::Conflict {
        message: message.to_owned(),
        current: None,
    })
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
    let Some(binding) = screen.seat() else {
        return refuse(
            StatusCode::GONE,
            "gone",
            "이 작업의 서버 브라우저가 닫혔어요. 작업 화면을 다시 열면 다시 준비해요.",
        );
    };
    if query.run.as_deref() != Some(binding.run.as_str())
        || query.bound.is_some_and(|bound| bound != binding.bound_at)
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
    upgrade
        .max_message_size(MAX_MESSAGE)
        .on_upgrade(move |socket| serve(socket, state, remote, id, binding))
}

/// Whether a socket's `message` goes in its queue, which has `free` places
/// left: moves that pile up behind an input the page does not answer are
/// let go (they would reach the page all at once), and the last places are
/// kept for what must not be lost ([`Incoming::is_motion`]). A message is
/// still lost when every place is taken.
fn takes(message: &Incoming, free: usize) -> bool {
    !message.is_motion() || free > KEPT_FROM_MOTION
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
            let reason = match err {
                OpenError::Stuck => {
                    eprintln!("trss-web: the page of job {job}'s run does not answer");
                    "stuck"
                }
                OpenError::Unreachable(why) => {
                    eprintln!("trss-web: cannot reach the page of job {job}'s run: {why}");
                    "unreachable"
                }
            };
            let _ = deliver(&mut socket, Message::Text(ended_message(reason).into())).await;
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
    for state in hub.states() {
        if !deliver(&mut socket, Message::Text(state.as_ref().into())).await {
            return;
        }
    }
    // The socket's messages are taken in order by a task of their own: one
    // the page does not answer in time holds that task, not the pings,
    // frames and notes below. Once the socket is over the task takes no
    // more of them, but the one it is taking is not cut short: a change of
    // size must not stop halfway.
    let (queue, mut queued) = tokio::sync::mpsc::channel::<Incoming>(QUEUE);
    let (dropped, mut notes) = tokio::sync::mpsc::unbounded_channel::<u64>();
    let over = CancellationToken::new();
    let _over = over.clone().drop_guard();
    tokio::spawn({
        let hub = hub.clone();
        async move {
            while let Some(message) = queued.recv().await {
                if over.is_cancelled() {
                    return;
                }
                if let Handled::Dropped { gen } = hub.handle(message).await {
                    let _ = dropped.send(gen);
                }
            }
        }
    });
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
                    // Not a message of the protocol: ignored.
                    let Ok(message) = serde_json::from_str::<Incoming>(text.as_str()) else {
                        continue;
                    };
                    // An answer to a dialog does not wait behind the inputs.
                    if let Incoming::Dialog { id, accept, text } = message {
                        let hub = hub.clone();
                        tokio::spawn(async move { hub.answer_dialog(id, accept, text).await });
                        continue;
                    }
                    if takes(&message, queue.capacity()) {
                        let _ = queue.try_send(message);
                    }
                }
                Some(Ok(_)) | None | Some(Err(_)) => break,
            },
            Some(gen) = notes.recv() => {
                let note = json!({ "type": "dropped", "gen": gen }).to_string();
                if !deliver(&mut socket, Message::Text(note.into())).await {
                    return;
                }
            }
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
                // Frames may be skipped; the page's state may not.
                Err(RecvError::Lagged(_)) => {
                    for state in hub.states() {
                        if !deliver(&mut socket, Message::Text(state.as_ref().into())).await {
                            return;
                        }
                    }
                }
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
