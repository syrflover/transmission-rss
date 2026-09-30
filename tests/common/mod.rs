//! Test support shared by the worker tests: a fake Transmission RPC server, a
//! fake RSS server, and a harness that wires a worker to them and to a
//! temporary app database.
//!
//! The fake speaks the subset of the Transmission JSON-RPC protocol that the
//! `transmission-rpc` client uses here (session-id handshake, `session-set`,
//! `torrent-add`, `torrent-get`, `torrent-rename-path`, `torrent-remove`,
//! `torrent-stop`, `torrent-set-location`), so the real client code paths run
//! against it. No real Transmission daemon was used.
//!
//! `torrent-set-location` with `move` moves the torrent's data on the real
//! disk (`downloadDir`/`name` to `location`/`name`, merging into folders that
//! exist, creating the missing ones), as Transmission does. The new folder is
//! reported at once, or after [`FakeTransmission::lag_locations`] looks. With
//! [`FakeTransmission::async_locations`] the data moves only when the new
//! folder is reported, as Transmission 4 moves in the background after it has
//! answered; [`FakeTransmission::fail_location_of`] makes that background move
//! fail the way Transmission shows it: the old folder stays and the torrent
//! reports a local error (`error` 3 and `errorString`).

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
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tokio::{net::TcpListener, sync::Semaphore, task::JoinHandle};
use tower::ServiceExt;
use transmission_rss::{
    store::{
        channels::{ChannelInput, ChannelStore, ChannelWithRules, RuleInput},
        history::{HistoryItem, HistoryQuery, HistoryStore, MAX_PAGE_SIZE},
        settings::SettingsStore,
        Db,
    },
    transmission::RenamePolicy,
    web::AppState,
    worker::{lock_path_for, Worker, WorkerEnv},
};

/// The collect folder every harness starts with. Test channels are given by
/// the folder they used to have as a base folder (`/media/anime`), which
/// [`Harness::add_channel`] turns into a rule-directory prefix under it, so the
/// save paths the tests expect stay the same text.
pub const COLLECT_FOLDER: &str = "/media";

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

    /// Lets one held (or the next) request through.
    pub fn release_one(&self) {
        self.release.add_permits(1);
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
    /// Bytes still to download (`leftUntilDone`); 0 for a finished torrent,
    /// which is the default.
    pub left_until_done: i64,
    /// A `torrent-set-location` whose folder is not reported yet: the folder
    /// and how many more `torrent-get` answers keep the old one.
    pub pending_location: Option<(String, u32)>,
    /// Whether the data of the pending move is still to be moved.
    pub pending_data: bool,
    /// The torrent's files as `torrent-get` lists them, relative to
    /// `download_dir`; empty means one file named like the torrent.
    pub files: Vec<String>,
    /// Transmission's `error` (0 none, 3 a local error) and `errorString`.
    pub error: u8,
    pub error_string: String,
    /// `metadataPercentComplete`: below 1 for a magnet still fetching its
    /// metadata (which lists no files and nothing left to download).
    pub metadata: f64,
    /// Each file's `length`; by default the size of the file on disk under
    /// `download_dir`, or 100 when it is not there.
    pub file_length: Option<i64>,
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
            left_until_done: 0,
            pending_location: None,
            pending_data: false,
            files: Vec::new(),
            error: 0,
            error_string: String::new(),
            metadata: 1.0,
            file_length: None,
        }
    }

    /// Saved in `dir`.
    pub fn in_dir(mut self, dir: impl AsRef<std::path::Path>) -> Self {
        self.download_dir = dir.as_ref().to_str().unwrap().to_owned();
        self
    }

    pub fn bot(mut self) -> Self {
        self.labels = vec![BOT_LABEL.to_owned()];
        self
    }

    pub fn status(mut self, status: u8) -> Self {
        self.status = status;
        self
    }

    /// Still downloading: some bytes are left.
    pub fn unfinished(mut self) -> Self {
        self.left_until_done = 1 << 20;
        self
    }

    /// The files `torrent-get` lists, relative to the torrent's folder.
    pub fn files(mut self, files: &[&str]) -> Self {
        self.files = files.iter().map(|f| f.to_string()).collect();
        self
    }

    /// A magnet still fetching its metadata: no files, nothing left.
    pub fn without_metadata(mut self) -> Self {
        self.metadata = 0.0;
        self.files = Vec::new();
        self
    }

    /// Each file `length` bytes long, as the torrent says.
    pub fn file_length(mut self, length: i64) -> Self {
        self.file_length = Some(length);
        self
    }

    /// Reporting a local error (`error` 3) with this text.
    pub fn local_error(mut self, text: &str) -> Self {
        self.error = 3;
        self.error_string = text.to_owned();
        self
    }
}

#[derive(Default)]
struct TrState {
    torrents: Vec<FakeTorrent>,
    calls: Vec<Call>,
    holds: HashMap<String, Arc<Gate>>,
    /// Like `holds`, but the request is carried out first and only the answer
    /// is held.
    holds_answer: HashMap<String, Arc<Gate>>,
    reject_adds: Option<String>,
    /// Leave `file-count` out of `torrent-get` answers, which makes the client
    /// code that reads it panic.
    omit_file_count: bool,
    /// `file-count` for a torrent `torrent-add` takes, by hash (default 1).
    file_counts: HashMap<String, usize>,
    /// How many `torrent-get` answers still report the old folder after a
    /// `torrent-set-location`.
    location_lag: u32,
    /// Makes every `torrent-set-location` answer with this refusal text.
    reject_locations: Option<String>,
    /// Refusal texts of `torrent-set-location` for single torrents, by hash.
    rejected_locations: HashMap<String, String>,
    /// Moves the data only when the new folder is reported.
    async_locations: bool,
    /// Background moves that fail with this `errorString`, by hash.
    failing_locations: HashMap<String, String>,
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

    /// Forgets the torrent `hash`, as a person removing it in Transmission.
    pub fn remove(&self, hash: &str) {
        self.state
            .lock()
            .unwrap()
            .torrents
            .retain(|t| t.hash != hash);
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

    /// Finishes the download of `hash`: nothing is left to download.
    pub fn finish(&self, hash: &str) {
        for t in self.state.lock().unwrap().torrents.iter_mut() {
            if t.hash == hash {
                t.left_until_done = 0;
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

    /// Makes `torrent-get` answers leave out `file-count`. The worker's renaming
    /// step then panics (after the torrent was added), which is how tests
    /// reach a panic in an item's task without a hook in the product code.
    pub fn omit_file_count(&self, omit: bool) {
        self.state.lock().unwrap().omit_file_count = omit;
    }

    /// Makes the torrent `hash`, once `torrent-add` takes it, have `count` files.
    pub fn files_on_add(&self, hash: &str, count: usize) {
        self.state
            .lock()
            .unwrap()
            .file_counts
            .insert(hash.to_owned(), count);
    }

    /// After a `torrent-set-location`, the next `lag` `torrent-get` answers
    /// still report the old folder, as a Transmission still moving the files.
    pub fn lag_locations(&self, lag: u32) {
        self.state.lock().unwrap().location_lag = lag;
    }

    /// Reports every folder a `torrent-set-location` asked for, now.
    pub fn settle_locations(&self) {
        let mut st = self.state.lock().unwrap();
        let failing = st.failing_locations.clone();
        for t in st.torrents.iter_mut() {
            if let Some((location, _)) = t.pending_location.take() {
                settle_location(t, location, &failing);
            }
        }
    }

    /// Makes every `torrent-set-location` answer with this refusal text.
    pub fn reject_locations(&self, result: Option<&str>) {
        self.state.lock().unwrap().reject_locations = result.map(str::to_owned);
    }

    /// Makes a `torrent-set-location` of the torrent `hash` answer with this
    /// refusal text (`None` takes it back).
    pub fn reject_location_of(&self, hash: &str, result: Option<&str>) {
        let mut st = self.state.lock().unwrap();
        match result {
            Some(result) => st
                .rejected_locations
                .insert(hash.to_owned(), result.to_owned()),
            None => st.rejected_locations.remove(hash),
        };
    }

    /// Moves a torrent's data only when its new folder is reported (at the
    /// next `torrent-get`, or after the lag), not when the move is asked for.
    pub fn async_locations(&self, on: bool) {
        self.state.lock().unwrap().async_locations = on;
    }

    /// Makes the background move of the torrent `hash` fail: when it would be
    /// reported, the old folder stays and the torrent reports a local error
    /// with `error_string` (`None` takes it back).
    pub fn fail_location_of(&self, hash: &str, error_string: Option<&str>) {
        let mut st = self.state.lock().unwrap();
        match error_string {
            Some(text) => st
                .failing_locations
                .insert(hash.to_owned(), text.to_owned()),
            None => st.failing_locations.remove(hash),
        };
    }

    /// The torrent `hash` as the fake holds it.
    pub fn torrent(&self, hash: &str) -> FakeTorrent {
        self.torrents()
            .into_iter()
            .find(|t| t.hash == hash)
            .unwrap_or_else(|| panic!("no torrent {hash}"))
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

    /// Carries out every request of `method` but holds its answer until the
    /// returned gate is released, as a Transmission that is slow to reply to a
    /// change it has already made.
    pub fn hold_answer(&self, method: &str) -> Arc<Gate> {
        let gate = Gate::new();
        self.state
            .lock()
            .unwrap()
            .holds_answer
            .insert(method.to_owned(), gate.clone());
        gate
    }

    /// What the server was asked to change, in a form that does not depend on
    /// request order: one sorted line per `session-set`, `torrent-add`,
    /// `torrent-rename-path`, `torrent-remove`, `torrent-stop` and
    /// `torrent-set-location`.
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
                    "torrent-set-location" => format!(
                        "torrent-set-location ids={} location={} move={}",
                        a["ids"], a["location"], a["move"]
                    ),
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
    let method = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|r| r["method"].as_str().map(str::to_owned));
    let response = tr_rpc_answer(State(state.clone()), headers, body).await;
    if response.status() != StatusCode::CONFLICT {
        let gate = method.and_then(|m| state.lock().unwrap().holds_answer.get(&m).cloned());
        if let Some(gate) = gate {
            gate.pass().await;
        }
    }
    response
}

async fn tr_rpc_answer(
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
            let file_count = st.file_counts.get(&hash).copied().unwrap_or(1);
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
                file_count,
                left_until_done: 0,
                pending_location: None,
                pending_data: false,
                files: Vec::new(),
                error: 0,
                error_string: String::new(),
                metadata: 1.0,
                file_length: None,
            });
            ok(json!({ "torrent-added": { "id": id, "hashString": hash, "name": name } }))
                .into_response()
        }

        "torrent-get" => {
            let failing = st.failing_locations.clone();
            for t in st.torrents.iter_mut() {
                match t.pending_location.take() {
                    Some((location, 0)) => settle_location(t, location, &failing),
                    Some((location, n)) => t.pending_location = Some((location, n - 1)),
                    None => {}
                }
            }
            let omit_file_count = st.omit_file_count;
            let wanted = ids(&args);
            let torrents: Vec<Value> = st
                .torrents
                .iter()
                .filter(|t| wanted.is_empty() && args["ids"].is_null() || wanted.contains(&t.hash))
                .map(|t| {
                    let mut torrent = json!({
                        "id": t.id, "name": t.name, "hashString": t.hash, "status": t.status,
                        "labels": t.labels, "file-count": t.file_count, "downloadDir": t.download_dir,
                        "leftUntilDone": t.left_until_done, "sizeWhenDone": 1_i64 << 30,
                        "error": t.error, "errorString": t.error_string,
                        "metadataPercentComplete": t.metadata,
                        "files": fake_files(t),
                    });
                    if omit_file_count {
                        torrent.as_object_mut().unwrap().remove("file-count");
                    }
                    torrent
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

        "torrent-set" => {
            let wanted = ids(&args);
            if let Some(labels) = args["labels"].as_array() {
                let labels: Vec<String> = labels
                    .iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect();
                for t in st.torrents.iter_mut().filter(|t| wanted.contains(&t.hash)) {
                    t.labels = labels.clone();
                }
            }
            ok(json!({})).into_response()
        }

        "torrent-set-location" => {
            if let Some(reason) = st.reject_locations.clone() {
                return err(&reason).into_response();
            }
            let wanted = ids(&args);
            if let Some(reason) = wanted.iter().find_map(|h| st.rejected_locations.get(h)) {
                return err(&reason.clone()).into_response();
            }
            let location = args["location"].as_str().unwrap_or_default().to_owned();
            let moves = args["move"].as_bool().unwrap_or(false);
            let lag = st.location_lag;
            let later = st.async_locations;
            let failing = st.failing_locations.clone();
            for t in st.torrents.iter_mut().filter(|t| wanted.contains(&t.hash)) {
                if later || failing.contains_key(&t.hash) {
                    // Answered now, moved in the background.
                    t.pending_location = Some((location.clone(), lag));
                    t.pending_data = moves;
                    continue;
                }
                if moves {
                    let from = std::path::Path::new(&t.download_dir).join(&t.name);
                    let to = std::path::Path::new(&location).join(&t.name);
                    if let Err(e) = move_tree(&from, &to) {
                        return err(&format!("move failed: {e}")).into_response();
                    }
                }
                if lag == 0 {
                    t.download_dir = location.clone();
                } else {
                    t.pending_location = Some((location.clone(), lag));
                }
            }
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

/// Ends the background move of `t` to `location`: fails it when `failing`
/// names the torrent, else moves its data if still to do and reports the folder.
fn settle_location(t: &mut FakeTorrent, location: String, failing: &HashMap<String, String>) {
    let data = std::mem::take(&mut t.pending_data);
    if let Some(text) = failing.get(&t.hash) {
        t.error = 3;
        t.error_string = text.clone();
        return;
    }
    if data {
        let from = std::path::Path::new(&t.download_dir).join(&t.name);
        let to = std::path::Path::new(&location).join(&t.name);
        if let Err(e) = move_tree(&from, &to) {
            t.error = 3;
            t.error_string = format!("move failed: {e}");
            return;
        }
    }
    t.download_dir = location;
}

/// A torrent's `files` as `torrent-get` lists them: all done, or none of
/// their bytes for a torrent still downloading.
fn fake_files(t: &FakeTorrent) -> Value {
    if t.metadata < 1.0 {
        return json!([]);
    }
    let names = if t.files.is_empty() {
        vec![t.name.clone()]
    } else {
        t.files.clone()
    };
    names
        .into_iter()
        .map(|name| {
            let length = t.file_length.unwrap_or_else(|| {
                std::fs::metadata(std::path::Path::new(&t.download_dir).join(&name))
                    .map(|m| m.len() as i64)
                    .unwrap_or(100)
            });
            let done = if t.left_until_done > 0 { 0 } else { length };
            json!({ "name": name, "length": length, "bytesCompleted": done })
        })
        .collect()
}

/// Moves `from` to `to` as Transmission moves a torrent's data: a missing
/// `from` is nothing to move, folders are created as needed, and a folder that
/// exists at `to` is merged into.
fn move_tree(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    let Ok(meta) = std::fs::symlink_metadata(from) else {
        return Ok(());
    };
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::symlink_metadata(to).is_err() {
        return std::fs::rename(from, to);
    }
    if !meta.is_dir() {
        return Err(std::io::ErrorKind::AlreadyExists.into());
    }
    for entry in std::fs::read_dir(from)? {
        let name = entry?.file_name();
        move_tree(&from.join(&name), &to.join(&name))?;
    }
    std::fs::remove_dir(from)
}

/// Hash (lowercased) and release name (`dn`, else the hash) of a magnet link.
/// An `http(s)` link stands for a `.torrent` download and is read the same way
/// from its `xt` and `dn` query values (the fake downloads nothing).
fn parse_magnet(link: &str) -> Option<(String, String)> {
    let url = url::Url::parse(link).ok()?;
    if !matches!(url.scheme(), "magnet" | "http" | "https") {
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
    /// A harness whose collect folder is [`COLLECT_FOLDER`].
    pub async fn new() -> Harness {
        let harness = Harness::without_collect_folder().await;
        SettingsStore::new(harness.db.clone())
            .put_collection(0, COLLECT_FOLDER.to_owned(), None)
            .await
            .unwrap();
        harness
    }

    /// A harness on a database with no collect folder set, as a fresh one is.
    pub async fn without_collect_folder() -> Harness {
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
    ///
    /// `base_dir` is the folder the channel's rules' directories are under, as
    /// channels used to have: it must be [`COLLECT_FOLDER`] or inside it, and
    /// what lies below the collect folder is put in front of each rule's
    /// directory, so the rule saves to `base_dir` + directory as before.
    pub async fn add_channel(
        &self,
        feed_path: &str,
        base_dir: &str,
        excludes: &[&str],
        mut rules: Vec<RuleInput>,
    ) -> ChannelWithRules {
        let url = format!("{}?filter=1080p&token={SECRET}", self.feeds.url(feed_path));
        let mut input = ChannelInput::new(url);
        input.excludes = excludes.iter().map(|s| s.to_string()).collect();
        let below = std::path::Path::new(base_dir)
            .strip_prefix(COLLECT_FOLDER)
            .unwrap_or_else(|_| panic!("{base_dir} is not inside {COLLECT_FOLDER}"))
            .to_string_lossy()
            .into_owned();
        for rule in &mut rules {
            rule.directory = transmission_rss::folders::prefixed(&below, &rule.directory);
        }
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

// --- the real web API, in process ---------------------------------------------------

/// The `/api` router on the harness's own database handle, called without a
/// socket. Requests go through the same handlers and store as `trss-web`.
pub struct WebApi {
    router: Router,
}

impl WebApi {
    pub fn new(db: Db) -> WebApi {
        WebApi {
            router: Router::new().nest(
                "/api",
                transmission_rss::web::api::router().with_state(AppState::new(db)),
            ),
        }
    }

    /// Sends one request; returns the status, the raw body text and its JSON
    /// (`null` when the body is not JSON).
    pub async fn call(
        &self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, String, Value) {
        let mut request = axum::http::Request::builder().method(method).uri(uri);
        let body = match body {
            Some(json) => {
                request = request.header("content-type", "application/json");
                axum::body::Body::from(json.to_string())
            }
            None => axum::body::Body::empty(),
        };
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let json = serde_json::from_str(&text).unwrap_or(Value::Null);
        (status, text, json)
    }
}

impl Harness {
    /// The channels API on this harness's database.
    pub fn web_api(&self) -> WebApi {
        WebApi::new(self.db.clone())
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
