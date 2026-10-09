//! A fake launcher for the pool's tests: the launcher's HTTP API on a local
//! port, and a scripted DevTools behind `/runs/{id}/cdp` that answers the
//! commands the pool sends the way Chromium does and records them.

#![allow(dead_code)]

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use trss_browser::{
    protocol::{RunInfo, RunList, StartRequest, Started},
    BrowserPolicy, BrowserPool, PolicySource, PoolConfig,
};

pub const TOKEN: &str = "fake-token";

/// A command the pool sent.
#[derive(Debug, Clone)]
pub struct Command {
    pub session: Option<String>,
    pub method: String,
    pub params: Value,
}

pub struct FakeRun {
    pub started_at: i64,
    /// Everything the pool sent over the run's DevTools socket.
    pub commands: Arc<Mutex<Vec<Command>>>,
    /// Raw messages for the browser to send to the pool.
    pub inject: broadcast::Sender<String>,
    /// Cancelled to cut the DevTools socket from the launcher's side.
    pub cut: CancellationToken,
    targets: Arc<Mutex<u32>>,
}

pub struct FakeState {
    pub downloads_root: PathBuf,
    pub max_runs: usize,
    pub runs: Mutex<HashMap<String, FakeRun>>,
    /// "POST /reset", "POST /runs a", "DELETE /runs a", "GET /runs" in order.
    pub calls: Mutex<Vec<String>>,
    pub fail_delete: AtomicBool,
    pub fail_reset: AtomicBool,
    /// How long `GET /runs` waits after it has read the runs before it
    /// answers, in milliseconds.
    pub list_delay_ms: AtomicU64,
    pub clock: Arc<AtomicI64>,
}

#[derive(Clone)]
pub struct FakeLauncher {
    pub state: Arc<FakeState>,
    pub url: url::Url,
}

pub async fn fake_launcher(
    downloads_root: PathBuf,
    max_runs: usize,
    clock: Arc<AtomicI64>,
) -> FakeLauncher {
    let state = Arc::new(FakeState {
        downloads_root,
        max_runs,
        runs: Mutex::default(),
        calls: Mutex::default(),
        fail_delete: AtomicBool::new(false),
        fail_reset: AtomicBool::new(false),
        list_delay_ms: AtomicU64::new(0),
        clock,
    });
    let app = Router::new()
        .route("/runs", post(start).get(list))
        .route("/runs/{id}", delete(end))
        .route("/runs/{id}/cdp", get(cdp))
        .route("/reset", post(reset))
        .with_state(state.clone());
    let served = trss_core::loopback::serve(|listener| async move {
        axum::serve(listener, app).await.ok();
    })
    .await;
    let url = format!("http://127.0.0.1:{}", served.addr.port())
        .parse()
        .unwrap();
    FakeLauncher { state, url }
}

fn authorized(headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        == Some(&format!("Bearer {TOKEN}"))
}

fn unauthorized() -> Response {
    StatusCode::UNAUTHORIZED.into_response()
}

async fn start(
    State(s): State<Arc<FakeState>>,
    headers: HeaderMap,
    Json(req): Json<StartRequest>,
) -> Response {
    if !authorized(&headers) {
        return unauthorized();
    }
    s.calls
        .lock()
        .unwrap()
        .push(format!("POST /runs {}", req.run));
    let downloads = s.downloads_root.join(&req.run);
    let mut runs = s.runs.lock().unwrap();
    if !runs.contains_key(&req.run) {
        if runs.len() >= s.max_runs {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error": "full"})),
            )
                .into_response();
        }
        std::fs::create_dir_all(&downloads).unwrap();
        runs.insert(
            req.run.clone(),
            FakeRun {
                started_at: s.clock.load(Ordering::SeqCst),
                commands: Arc::default(),
                inject: broadcast::channel(256).0,
                cut: CancellationToken::new(),
                targets: Arc::new(Mutex::new(0)),
            },
        );
    }
    Json(Started {
        run: req.run,
        downloads: downloads.to_string_lossy().into_owned(),
    })
    .into_response()
}

async fn list(State(s): State<Arc<FakeState>>, headers: HeaderMap) -> Response {
    if !authorized(&headers) {
        return unauthorized();
    }
    s.calls.lock().unwrap().push("GET /runs".to_owned());
    let list = {
        let runs = s.runs.lock().unwrap();
        RunList {
            runs: runs
                .iter()
                .map(|(id, r)| RunInfo {
                    run: id.clone(),
                    started_at: r.started_at,
                    pid_alive: true,
                    downloads: s.downloads_root.join(id).to_string_lossy().into_owned(),
                })
                .collect(),
        }
    };
    // The answer is as of now, and reaches the pool later.
    let delay = s.list_delay_ms.load(Ordering::SeqCst);
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    Json(list).into_response()
}

async fn end(
    State(s): State<Arc<FakeState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !authorized(&headers) {
        return unauthorized();
    }
    s.calls.lock().unwrap().push(format!("DELETE /runs {id}"));
    if s.fail_delete.load(Ordering::SeqCst) {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if let Some(run) = s.runs.lock().unwrap().remove(&id) {
        run.cut.cancel();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn reset(State(s): State<Arc<FakeState>>, headers: HeaderMap) -> Response {
    if !authorized(&headers) {
        return unauthorized();
    }
    s.calls.lock().unwrap().push("POST /reset".to_owned());
    if s.fail_reset.load(Ordering::SeqCst) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    for (_, run) in s.runs.lock().unwrap().drain() {
        run.cut.cancel();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn cdp(
    State(s): State<Arc<FakeState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !authorized(&headers) {
        return unauthorized();
    }
    let parts = {
        let runs = s.runs.lock().unwrap();
        runs.get(&id).map(|r| {
            (
                r.commands.clone(),
                r.inject.subscribe(),
                r.cut.clone(),
                r.targets.clone(),
            )
        })
    };
    let Some((commands, injected, cut, targets)) = parts else {
        return StatusCode::NOT_FOUND.into_response();
    };
    upgrade.on_upgrade(move |socket| script(socket, commands, injected, cut, targets))
}

fn attached(target: &str, session: &str, kind: &str) -> String {
    json!({
        "method": "Target.attachedToTarget",
        "params": {
            "sessionId": session,
            "targetInfo": { "targetId": target, "type": kind, "url": "about:blank" },
            "waitingForDebugger": true,
        }
    })
    .to_string()
}

/// Answers the commands as Chromium would, and sends the events they cause.
async fn script(
    mut socket: WebSocket,
    commands: Arc<Mutex<Vec<Command>>>,
    mut injected: broadcast::Receiver<String>,
    cut: CancellationToken,
    targets: Arc<Mutex<u32>>,
) {
    loop {
        tokio::select! {
            _ = cut.cancelled() => break,
            event = injected.recv() => match event {
                Ok(text) => {
                    if socket.send(Message::text(text)).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            },
            message = socket.recv() => {
                let Some(Ok(Message::Text(text))) = message else { break };
                let request: Value = serde_json::from_str(text.as_str()).unwrap();
                let session = request["sessionId"].as_str().map(str::to_owned);
                let method = request["method"].as_str().unwrap().to_owned();
                commands.lock().unwrap().push(Command {
                    session: session.clone(),
                    method: method.clone(),
                    params: request["params"].clone(),
                });
                let mut extra = Vec::new();
                let result = match (method.as_str(), session.is_some()) {
                    // The browser's own first page appears when auto-attach is on.
                    ("Target.setAutoAttach", false) => {
                        let mut count = targets.lock().unwrap();
                        if *count == 0 {
                            *count = 1;
                            extra.push(attached("T0", "S0", "page"));
                        }
                        json!({})
                    }
                    ("Target.createTarget", _) => {
                        let mut count = targets.lock().unwrap();
                        let n = *count;
                        *count += 1;
                        extra.push(attached(&format!("T{n}"), &format!("S{n}"), "page"));
                        json!({ "targetId": format!("T{n}") })
                    }
                    ("Page.navigate", _) => json!({ "frameId": "F" }),
                    ("Runtime.evaluate", _) => {
                        json!({ "result": { "value": request["params"]["expression"] } })
                    }
                    _ => json!({}),
                };
                let mut answer = json!({ "id": request["id"], "result": result });
                if let Some(session) = &session {
                    answer["sessionId"] = json!(session);
                }
                if socket.send(Message::text(answer.to_string())).await.is_err() {
                    break;
                }
                for event in extra {
                    if socket.send(Message::text(event)).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
}

impl FakeLauncher {
    pub fn calls(&self) -> Vec<String> {
        self.state.calls.lock().unwrap().clone()
    }

    pub fn calls_matching(&self, prefix: &str) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter(|c| c.starts_with(prefix))
            .collect()
    }

    pub fn run_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.state.runs.lock().unwrap().keys().cloned().collect();
        ids.sort();
        ids
    }

    pub fn commands(&self, run: &str) -> Vec<Command> {
        let runs = self.state.runs.lock().unwrap();
        runs.get(run)
            .map(|r| r.commands.lock().unwrap().clone())
            .unwrap_or_default()
    }

    /// Sends a raw DevTools message from the browser of `run` to the pool.
    pub fn emit(&self, run: &str, message: Value) {
        let runs = self.state.runs.lock().unwrap();
        let _ = runs[run].inject.send(message.to_string());
    }

    pub fn emit_attached(&self, run: &str, target: &str, session: &str, kind: &str) {
        let runs = self.state.runs.lock().unwrap();
        let _ = runs[run].inject.send(attached(target, session, kind));
    }

    /// The launcher's side drops the DevTools socket; the run stays.
    pub fn cut_connection(&self, run: &str) {
        let runs = self.state.runs.lock().unwrap();
        runs[run].cut.cancel();
    }

    /// A run in the launcher that no pool asked for.
    pub fn add_orphan(&self, run: &str) {
        std::fs::create_dir_all(self.state.downloads_root.join(run)).unwrap();
        self.state.runs.lock().unwrap().insert(
            run.to_owned(),
            FakeRun {
                started_at: 0,
                commands: Arc::default(),
                inject: broadcast::channel(16).0,
                cut: CancellationToken::new(),
                targets: Arc::new(Mutex::new(0)),
            },
        );
    }

    /// The launcher loses a run without the pool asking (its Chromium exited).
    pub fn lose(&self, run: &str) {
        // Not cut: the pool must learn of it from the list.
        self.state.runs.lock().unwrap().remove(run);
    }
}

/// A pool over a fake launcher, with a clock and a policy the test moves.
pub struct Setup {
    pub dir: tempfile::TempDir,
    pub downloads_root: PathBuf,
    pub fake: FakeLauncher,
    pub pool: BrowserPool,
    pub clock: Arc<AtomicI64>,
    pub policy: Arc<Mutex<BrowserPolicy>>,
}

impl Setup {
    pub async fn new(policy: BrowserPolicy) -> Setup {
        Setup::with(policy, TOKEN, |_| {}).await
    }

    pub async fn with(
        policy: BrowserPolicy,
        token: &str,
        before: impl FnOnce(&std::path::Path),
    ) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let downloads_root = dir.path().join("browser-downloads");
        std::fs::create_dir_all(&downloads_root).unwrap();
        before(&downloads_root);
        let clock = Arc::new(AtomicI64::new(1_000_000));
        let fake = fake_launcher(downloads_root.clone(), 8, clock.clone()).await;
        let policy = Arc::new(Mutex::new(policy));
        let pool = make_pool(&fake, &downloads_root, token, &clock, &policy).await;
        Setup {
            dir,
            downloads_root,
            fake,
            pool,
            clock,
            policy,
        }
    }

    pub fn advance(&self, by: Duration) {
        self.clock
            .fetch_add(by.as_millis() as i64, Ordering::SeqCst);
    }

    pub fn set_policy(&self, policy: BrowserPolicy) {
        *self.policy.lock().unwrap() = policy;
    }
}

pub async fn make_pool(
    fake: &FakeLauncher,
    downloads_root: &std::path::Path,
    token: &str,
    clock: &Arc<AtomicI64>,
    policy: &Arc<Mutex<BrowserPolicy>>,
) -> BrowserPool {
    let mut config = PoolConfig::new(fake.url.clone(), token, downloads_root);
    config.slot_recheck = Duration::from_millis(100);
    let clock = clock.clone();
    let policy = policy.clone();
    BrowserPool::new(
        config,
        Arc::new(move || clock.load(Ordering::SeqCst)),
        PolicySource::new(move || {
            let policy = *policy.lock().unwrap();
            Box::pin(async move { policy })
        }),
    )
    .await
    .unwrap()
}

pub async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !check() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
