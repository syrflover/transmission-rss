//! Test support shared by the worker tests: a fake Transmission RPC server, a
//! fake RSS server, and a harness that wires a worker to them and to a
//! temporary app database.
//!
//! The fake speaks the subset of the Transmission JSON-RPC protocol that the
//! `transmission-rpc` client uses here (session-id handshake, `session-set`,
//! `torrent-add`, `torrent-get`, `torrent-rename-path`, `torrent-remove`,
//! `torrent-stop`), so the real client code paths run against it. No real
//! Transmission daemon was used.

#![allow(dead_code)]

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use axum::{
    body::Bytes,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use tokio::{net::TcpListener, sync::Semaphore, task::JoinHandle};
use transmission_rss::{
    store::{
        channels::{ChannelInput, ChannelStore, ChannelWithRules, RuleInput},
        history::{HistoryItem, HistoryQuery, HistoryStore, MAX_PAGE_SIZE},
        Db,
    },
    transmission::RenamePolicy,
    worker::{lock_path_for, Worker, WorkerEnv},
};

pub const BOT_LABEL: &str = "managed:transmission-rss";
pub const SESSION_ID: &str = "fake-session-id";

pub const FEED_A: &str = include_str!("../fixtures/worker_feed_a.xml");
pub const FEED_B: &str = include_str!("../fixtures/worker_feed_b.xml");
pub const CHANNELS_YAML: &str = include_str!("../fixtures/worker_channels.yml");

// --- a gate that holds requests until the test lets them through ----------------

/// Holds every request that reaches it until [`Gate::release_all`].
pub struct Gate {
    arrived: Semaphore,
    release: Semaphore,
}

impl Gate {
    fn new() -> Arc<Gate> {
        Arc::new(Gate {
            arrived: Semaphore::new(0),
            release: Semaphore::new(0),
        })
    }

    async fn pass(&self) {
        self.arrived.add_permits(1);
        self.release.acquire().await.unwrap().forget();
    }

    /// Waits until a request has reached the gate.
    pub async fn wait_arrived(&self) {
        tokio::time::timeout(Duration::from_secs(20), self.arrived.acquire())
            .await
            .expect("no request reached the gate")
            .unwrap()
            .forget();
    }

    /// Lets the held requests and all later ones through.
    pub fn release_all(&self) {
        self.release.add_permits(100_000);
    }
}

// --- fake Transmission ---------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Call {
    pub method: String,
    pub args: Value,
}

#[derive(Clone, Debug)]
pub struct FakeTorrent {
    pub id: i64,
    pub hash: String,
    pub name: String,
    pub labels: Vec<String>,
    pub download_dir: String,
    /// Transmission status number: 0 stopped, 4 downloading, 6 seeding.
    pub status: u8,
    pub file_count: usize,
}

impl FakeTorrent {
    pub fn new(hash: &str, name: &str) -> Self {
        FakeTorrent {
            id: 0,
            hash: hash.to_owned(),
            name: name.to_owned(),
            labels: vec![],
            download_dir: "/downloads".to_owned(),
            status: 4,
            file_count: 1,
        }
    }

    pub fn bot(mut self) -> Self {
        self.labels = vec![BOT_LABEL.to_owned()];
        self
    }

    pub fn status(mut self, status: u8) -> Self {
        self.status = status;
        self
    }
}

#[derive(Default)]
struct TrState {
    torrents: Vec<FakeTorrent>,
    calls: Vec<Call>,
    holds: HashMap<String, Arc<Gate>>,
    reject_adds: Option<String>,
    next_id: i64,
}

pub struct FakeTransmission {
    pub addr: SocketAddr,
    state: Arc<Mutex<TrState>>,
    task: Option<JoinHandle<()>>,
}

impl FakeTransmission {
    pub async fn start() -> Self {
        Self::start_at("127.0.0.1:0".parse().unwrap()).await
    }

    /// Starts on a given address, e.g. the one a previous fake used before it
    /// was stopped.
    pub async fn start_at(addr: SocketAddr) -> Self {
        let state = Arc::new(Mutex::new(TrState {
            next_id: 1,
            ..Default::default()
        }));
        Self::serve(addr, state).await
    }

    async fn serve(addr: SocketAddr, state: Arc<Mutex<TrState>>) -> Self {
        let listener = TcpListener::bind(addr)
            .await
            .expect("bind fake transmission");
        let addr = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/transmission/rpc", post(tr_rpc))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        FakeTransmission {
            addr,
            state,
            task: Some(task),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}/transmission/rpc", self.addr)
    }

    /// Stops the server (connections are refused afterwards) and keeps the
    /// state, which a later [`FakeTransmission::restart`] serves again.
    pub async fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }

    pub async fn restart(&mut self) {
        assert!(self.task.is_none(), "stop it first");
        let mut again = Self::serve(self.addr, self.state.clone()).await;
        self.task = again.task.take();
    }

    pub fn preload(&self, mut torrent: FakeTorrent) {
        let mut st = self.state.lock().unwrap();
        torrent.id = st.next_id;
        st.next_id += 1;
        st.torrents.push(torrent);
    }

    pub fn torrents(&self) -> Vec<FakeTorrent> {
        self.state.lock().unwrap().torrents.clone()
    }

    pub fn set_status(&self, hash: &str, status: u8) {
        for t in self.state.lock().unwrap().torrents.iter_mut() {
            if t.hash == hash {
                t.status = status;
            }
        }
    }

    pub fn calls(&self) -> Vec<Call> {
        self.state.lock().unwrap().calls.clone()
    }

    pub fn calls_of(&self, method: &str) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|c| c.method == method)
            .collect()
    }

    pub fn clear_calls(&self) {
        self.state.lock().unwrap().calls.clear();
    }

    /// Makes every `torrent-add` answer with this refusal text.
    pub fn reject_adds(&self, result: Option<&str>) {
        self.state.lock().unwrap().reject_adds = result.map(str::to_owned);
    }

    /// Holds every request of `method` until the returned gate is released.
    pub fn hold(&self, method: &str) -> Arc<Gate> {
        let gate = Gate::new();
        self.state
            .lock()
            .unwrap()
            .holds
            .insert(method.to_owned(), gate.clone());
        gate
    }

    /// What the server was asked to change, in a form that does not depend on
    /// request order: one sorted line per `session-set`, `torrent-add`,
    /// `torrent-rename-path`, `torrent-remove` and `torrent-stop`.
    pub fn mutations(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .calls()
            .into_iter()
            .filter_map(|c| {
                let a = &c.args;
                let line = match c.method.as_str() {
                    "session-set" => format!("session-set {a}"),
                    "torrent-add" => format!(
                        "torrent-add filename={} download-dir={} labels={}",
                        a["filename"], a["download-dir"], a["labels"]
                    ),
                    "torrent-rename-path" => format!(
                        "torrent-rename-path ids={} path={} name={}",
                        a["ids"], a["path"], a["name"]
                    ),
                    "torrent-remove" => format!(
                        "torrent-remove ids={} delete-local-data={}",
                        sorted_ids(&a["ids"]),
                        a["delete-local-data"]
                    ),
                    "torrent-stop" => format!("torrent-stop ids={}", a["ids"]),
                    _ => return None,
                };
                Some(line)
            })
            .collect();
        out.sort();
        out
    }
}

impl Drop for FakeTransmission {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn sorted_ids(ids: &Value) -> String {
    let mut v: Vec<String> = ids
        .as_array()
        .map(|a| a.iter().map(|x| x.to_string()).collect())
        .unwrap_or_default();
    v.sort();
    format!("[{}]", v.join(","))
}

async fn tr_rpc(
    State(state): State<Arc<Mutex<TrState>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // The session-id handshake: unknown ids get 409 and the id to use.
    if headers
        .get("x-transmission-session-id")
        .and_then(|v| v.to_str().ok())
        != Some(SESSION_ID)
    {
        let mut res = StatusCode::CONFLICT.into_response();
        res.headers_mut().insert(
            "x-transmission-session-id",
            HeaderValue::from_static(SESSION_ID),
        );
        return res;
    }

    let request: Value = serde_json::from_slice(&body).expect("json rpc request");
    let method = request["method"].as_str().unwrap_or_default().to_owned();
    let args = request.get("arguments").cloned().unwrap_or(Value::Null);

    let gate = state.lock().unwrap().holds.get(&method).cloned();
    if let Some(gate) = gate {
        gate.pass().await;
    }

    let mut st = state.lock().unwrap();
    st.calls.push(Call {
        method: method.clone(),
        args: args.clone(),
    });

    let ok = |arguments: Value| Json(json!({ "arguments": arguments, "result": "success" }));
    let err = |text: &str| Json(json!({ "arguments": {}, "result": text }));

    let ids = |args: &Value| -> Vec<String> {
        args["ids"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };

    match method.as_str() {
        "session-set" => ok(json!({})).into_response(),

        "torrent-add" => {
            if let Some(reason) = st.reject_adds.clone() {
                return err(&reason).into_response();
            }
            let filename = args["filename"].as_str().unwrap_or_default();
            let Some((hash, name)) = parse_magnet(filename) else {
                return err("invalid or corrupt torrent file").into_response();
            };
            if let Some(existing) = st.torrents.iter().find(|t| t.hash == hash) {
                return ok(json!({
                    "torrent-duplicate": {
                        "id": existing.id, "hashString": existing.hash, "name": existing.name
                    }
                }))
                .into_response();
            }
            let id = st.next_id;
            st.next_id += 1;
            st.torrents.push(FakeTorrent {
                id,
                hash: hash.clone(),
                name: name.clone(),
                labels: args["labels"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                download_dir: args["download-dir"]
                    .as_str()
                    .unwrap_or("/downloads")
                    .to_owned(),
                status: 4,
                file_count: 1,
            });
            ok(json!({ "torrent-added": { "id": id, "hashString": hash, "name": name } }))
                .into_response()
        }

        "torrent-get" => {
            let wanted = ids(&args);
            let torrents: Vec<Value> = st
                .torrents
                .iter()
                .filter(|t| wanted.is_empty() && args["ids"].is_null() || wanted.contains(&t.hash))
                .map(|t| {
                    json!({
                        "id": t.id, "name": t.name, "hashString": t.hash, "status": t.status,
                        "labels": t.labels, "file-count": t.file_count, "downloadDir": t.download_dir,
                    })
                })
                .collect();
            ok(json!({ "torrents": torrents })).into_response()
        }

        "torrent-rename-path" => {
            let hash = ids(&args).into_iter().next().unwrap_or_default();
            let path = args["path"].as_str().unwrap_or_default();
            let name = args["name"].as_str().unwrap_or_default();
            match st.torrents.iter_mut().find(|t| t.hash == hash) {
                Some(t) if t.name == path && name != path => {
                    t.name = name.to_owned();
                    ok(json!({ "path": path, "name": name, "id": t.id })).into_response()
                }
                Some(_) => err("file not found").into_response(),
                None => err("no torrent").into_response(),
            }
        }

        "torrent-remove" => {
            let wanted = ids(&args);
            st.torrents.retain(|t| !wanted.contains(&t.hash));
            ok(json!({})).into_response()
        }

        "torrent-stop" => {
            let wanted = ids(&args);
            for t in st.torrents.iter_mut().filter(|t| wanted.contains(&t.hash)) {
                t.status = 0;
            }
            ok(json!({})).into_response()
        }

        other => err(&format!("method {other} not supported by the fake")).into_response(),
    }
}

/// Hash (lowercased) and release name (`dn`, else the hash) of a magnet link.
fn parse_magnet(link: &str) -> Option<(String, String)> {
    let url = url::Url::parse(link).ok()?;
    if url.scheme() != "magnet" {
        return None;
    }
    let mut hash = None;
    let mut name = None;
    for (k, v) in url.query_pairs() {
        match &*k {
            "xt" => hash = v.strip_prefix("urn:btih:").map(|h| h.to_lowercase()),
            "dn" => name = Some(v.into_owned()),
            _ => {}
        }
    }
    let hash = hash?;
    let name = name.unwrap_or_else(|| hash.clone());
    Some((hash, name))
}

// --- fake RSS server -------------------------------------------------------------

enum Route {
    Xml(String),
    Status(u16),
}

#[derive(Default)]
struct FeedState {
    routes: HashMap<String, Route>,
    requests: Vec<String>,
    holds: HashMap<String, Arc<Gate>>,
}

pub struct FeedServer {
    pub addr: SocketAddr,
    state: Arc<Mutex<FeedState>>,
    task: JoinHandle<()>,
}

impl FeedServer {
    pub async fn start() -> Self {
        let state = Arc::new(Mutex::new(FeedState::default()));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake feeds");
        let addr = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/{*path}", get(serve_feed))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        FeedServer { addr, state, task }
    }

    /// Base URL of a feed path, without a query.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}/{}", self.addr, path)
    }

    pub fn set_xml(&self, path: &str, xml: &str) {
        self.state
            .lock()
            .unwrap()
            .routes
            .insert(path.to_owned(), Route::Xml(xml.to_owned()));
    }

    pub fn set_status(&self, path: &str, status: u16) {
        self.state
            .lock()
            .unwrap()
            .routes
            .insert(path.to_owned(), Route::Status(status));
    }

    /// Every request received so far as `path?query`.
    pub fn requests(&self) -> Vec<String> {
        self.state.lock().unwrap().requests.clone()
    }

    pub fn hits(&self, path: &str) -> usize {
        let prefix = format!("{path}?");
        self.requests()
            .iter()
            .filter(|r| r.starts_with(&prefix) || r.as_str() == path)
            .count()
    }

    pub fn hold(&self, path: &str) -> Arc<Gate> {
        let gate = Gate::new();
        self.state
            .lock()
            .unwrap()
            .holds
            .insert(path.to_owned(), gate.clone());
        gate
    }
}

impl Drop for FeedServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_feed(
    State(state): State<Arc<Mutex<FeedState>>>,
    Path(path): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    let gate = {
        let mut st = state.lock().unwrap();
        st.requests
            .push(format!("{path}?{}", query.unwrap_or_default()));
        st.holds.get(&path).cloned()
    };
    if let Some(gate) = gate {
        gate.pass().await;
    }

    let st = state.lock().unwrap();
    match st.routes.get(&path) {
        Some(Route::Xml(xml)) => {
            ([("content-type", "application/rss+xml")], xml.clone()).into_response()
        }
        Some(Route::Status(code)) => StatusCode::from_u16(*code).unwrap().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// --- harness -----------------------------------------------------------------------

/// A channel URL query value that must never show up in logs or history.
pub const SECRET: &str = "SECRETTOKEN0123456789";

pub struct Harness {
    pub dir: tempfile::TempDir,
    pub db: Db,
    pub channels: ChannelStore,
    pub history: HistoryStore,
    pub tr: FakeTransmission,
    pub feeds: FeedServer,
    pub clock: Arc<AtomicI64>,
}

impl Harness {
    pub async fn new() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        let feeds = FeedServer::start().await;
        feeds.set_xml("feed-a", FEED_A);
        feeds.set_xml("feed-b", FEED_B);
        Harness {
            channels: ChannelStore::new(db.clone()),
            history: HistoryStore::new(db.clone()),
            tr: FakeTransmission::start().await,
            feeds,
            db,
            dir,
            clock: Arc::new(AtomicI64::new(1_000_000)),
        }
    }

    pub fn db_path(&self) -> std::path::PathBuf {
        self.dir.path().join("app.db")
    }

    pub fn worker_env(&self) -> WorkerEnv {
        self.worker_env_with(&[])
    }

    /// The worker settings as environment variables would give them, plus `extra`.
    pub fn worker_env_with(&self, extra: &[(&str, &str)]) -> WorkerEnv {
        let tr_url = self.tr.url();
        let extra: HashMap<String, String> = extra
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        WorkerEnv::from_lookup(move |key| match key {
            "TRANSMISSION_URL" => Some(tr_url.clone()),
            other => extra.get(other).cloned(),
        })
        .unwrap()
    }

    /// A worker on this harness's database and fakes, with a fast rename
    /// policy, a manual clock and no minimum gap between cycles.
    pub fn worker(&self) -> Worker {
        self.worker_with_db(self.db.clone())
    }

    pub fn worker_with_db(&self, db: Db) -> Worker {
        self.worker_with(db, &self.worker_env())
    }

    pub fn worker_with(&self, db: Db, env: &WorkerEnv) -> Worker {
        let clock = self.clock.clone();
        Worker::new(db, env, lock_path_for(&self.db_path()))
            .unwrap()
            .with_clock(Arc::new(move || clock.load(Ordering::SeqCst)))
            .with_rename_policy(RenamePolicy {
                delay: Duration::from_millis(5),
                attempts: 3,
            })
            .with_min_gap(Duration::ZERO)
    }

    /// Moves the manual clock forward.
    pub fn advance(&self, millis: i64) {
        self.clock.fetch_add(millis, Ordering::SeqCst);
    }

    pub fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    /// Adds a channel for one of the fake feeds. Every query value of the URL
    /// is secret, as for a channel added in the app.
    pub async fn add_channel(
        &self,
        feed_path: &str,
        base_dir: &str,
        excludes: &[&str],
        rules: Vec<RuleInput>,
    ) -> ChannelWithRules {
        let url = format!("{}?filter=1080p&token={SECRET}", self.feeds.url(feed_path));
        let mut input = ChannelInput::new(url, base_dir);
        input.excludes = excludes.iter().map(|s| s.to_string()).collect();
        self.channels
            .create_channel_with_rules(input, rules)
            .await
            .unwrap()
    }

    pub async fn history_items(&self) -> Vec<HistoryItem> {
        self.history
            .list(HistoryQuery {
                limit: MAX_PAGE_SIZE,
                ..Default::default()
            })
            .await
            .unwrap()
            .items
    }

    pub async fn item(&self, title_part: &str) -> HistoryItem {
        let items = self.history_items().await;
        let mut found = items.into_iter().filter(|i| i.title.contains(title_part));
        let item = found
            .next()
            .unwrap_or_else(|| panic!("no history item with {title_part:?}"));
        assert!(
            found.next().is_none(),
            "several history items with {title_part:?}"
        );
        item
    }
}

pub fn rule(match_text: &str, directory: &str) -> RuleInput {
    RuleInput {
        r#match: Some(match_text.to_owned()),
        directory: directory.to_owned(),
        ..Default::default()
    }
}

/// The rules of the first channel of `worker_channels.yml`, as stored rules.
pub fn feed_a_rules() -> Vec<RuleInput> {
    vec![
        rule("[SubsPlease] Sayonara Lara - ", "Sayonara Lara/Season 01"),
        RuleInput {
            case_insensitive: true,
            episode: -12,
            ..rule("sono bisque doll", "Sono Bisque Doll/Season 02")
        },
        rule("Sono Bisque Doll", "Sono Bisque Doll (overlap)"),
        RuleInput {
            episode: -24,
            ..rule(
                "[SubsPlease] Tensei Shitara Slime Datta Ken",
                "Slime/Season 04",
            )
        },
    ]
}
