//! The remote screen against a fake launcher whose DevTools proxy answers
//! like a browser's page (the real one: `tests/remote_screen_docker.rs`,
//! ignored).

use std::{
    collections::HashMap,
    os::unix::net::UnixDatagram,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    extract::{ws::WebSocketUpgrade, Path as UrlPath},
    http::HeaderMap,
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, Message as WsMessage};
use tokio_util::sync::CancellationToken;
use trss_core::{Db, DbError};
use trss_jobs::{Created, JobStore, NewItem, NewJob, ScreenStore};
use url::Url;

use super::*;
use crate::env::BrowserAccess;

const TOKEN: &str = "launcher-token";

/// What the fake launcher saw.
#[derive(Default)]
struct Seen {
    /// Each DevTools command of every connection: (run, method, params).
    commands: Vec<(String, String, Value)>,
    /// How many DevTools connections were made to each run.
    connections: HashMap<String, usize>,
    /// Any other request (to start, end or list runs): the web makes none.
    others: Vec<String>,
}

/// The index of the entry shown, and the entries (ID, address).
type FakeHistory = (usize, Vec<(i64, String)>);

#[derive(Clone, Default)]
struct Launcher {
    seen: Arc<Mutex<Seen>>,
    /// Cancelled: the run's DevTools connections are closed (it ended).
    ended: Arc<Mutex<HashMap<String, CancellationToken>>>,
    /// Set: a start of the screencast sends only the frame before its
    /// answer (the page draws nothing new after it).
    quiet: Arc<std::sync::atomic::AtomicBool>,
    /// Set: a mouse press is answered only after 300 ms.
    slow_press: Arc<std::sync::atomic::AtomicBool>,
    /// Set: the page's own thread is held (a script that never yields), so
    /// what the page answers itself (inputs, `Runtime.evaluate`,
    /// `Page.enable`) gets no answer. The browser still answers the rest.
    stuck: Arc<std::sync::atomic::AtomicBool>,
    /// The history of every page: the index of the entry shown, and the
    /// entries (ID, address). A step to an entry changes the index.
    history: Arc<Mutex<FakeHistory>>,
    /// The targets the browser reports (`Target.getTargets`).
    targets: Arc<Mutex<Value>>,
}

/// The address a fake page is at: a path, a query and a fragment that must
/// never leave the web.
const SECRET_PAGE: &str = "https://blog.example.org/post/1?sig=SECRET#frag";
const SECRET_NEXT: &str = "https://files.example.org/get/a.srt?sig=SECRET2";

impl Launcher {
    fn new() -> Launcher {
        let launcher = Launcher::default();
        // What the server's own preparation leaves: a blank page, then the post.
        launcher.set_history(1, &["about:blank", SECRET_PAGE]);
        launcher
    }

    /// The history of the pages is `urls`, at the entry `current`.
    fn set_history(&self, current: usize, urls: &[&str]) {
        *self.history.lock().unwrap() = (
            current,
            urls.iter()
                .enumerate()
                .map(|(n, u)| (n as i64 + 10, (*u).to_owned()))
                .collect(),
        );
    }

    fn set_targets(&self, targets: Value) {
        *self.targets.lock().unwrap() = targets;
    }

    fn count(&self, method: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .commands
            .iter()
            .filter(|(_, m, _)| m == method)
            .count()
    }
}

impl Launcher {
    fn methods(&self, run: &str) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .commands
            .iter()
            .filter(|(r, _, _)| r == run)
            .map(|(_, m, _)| m.clone())
            .collect()
    }

    fn last(&self, method: &str) -> Option<Value> {
        self.seen
            .lock()
            .unwrap()
            .commands
            .iter()
            .rev()
            .find(|(_, m, _)| m == method)
            .map(|(_, _, p)| p.clone())
    }

    fn end(&self, run: &str) {
        self.token(run).cancel();
    }

    fn token(&self, run: &str) -> CancellationToken {
        self.ended
            .lock()
            .unwrap()
            .entry(run.to_owned())
            .or_default()
            .clone()
    }
}

async fn cdp(
    UrlPath(run): UrlPath<String>,
    headers: HeaderMap,
    axum::extract::State(launcher): axum::extract::State<Launcher>,
    upgrade: WebSocketUpgrade,
) -> axum::response::Response {
    if headers.get("authorization").and_then(|v| v.to_str().ok())
        != Some(&format!("Bearer {TOKEN}"))
    {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }
    *launcher
        .seen
        .lock()
        .unwrap()
        .connections
        .entry(run.clone())
        .or_default() += 1;
    upgrade.on_upgrade(move |socket| page(socket, run, launcher))
}

/// A page's DevTools: every command answered, and a frame of the emulated
/// size each time the screencast starts. One more (`stale…`) comes just
/// before the start is answered, as a frame of the layout before can: no
/// generation after a change of size may show it.
async fn page(mut socket: axum::extract::ws::WebSocket, run: String, launcher: Launcher) {
    use axum::extract::ws::Message;
    let ended = launcher.token(&run);
    let session = format!("S-{run}");
    let (mut width, mut height) = (800.0, 600.0);
    let mut frame = 0;
    loop {
        let message = tokio::select! {
            _ = ended.cancelled() => break,
            message = socket.recv() => message,
        };
        let Some(Ok(Message::Text(text))) = message else {
            break;
        };
        let command: Value = serde_json::from_str(text.as_str()).unwrap();
        let method = command["method"].as_str().unwrap().to_owned();
        let params = command["params"].clone();
        launcher
            .seen
            .lock()
            .unwrap()
            .commands
            .push((run.clone(), method.clone(), params.clone()));
        let own =
            method.starts_with("Input.") || method == "Runtime.evaluate" || method == "Page.enable";
        if own && launcher.stuck.load(std::sync::atomic::Ordering::SeqCst) {
            // No answer. A frame comes meanwhile, which the socket must
            // still be sent while its input waits.
            if method.starts_with("Input.") {
                let event = json!({
                    "method": "Page.screencastFrame",
                    "sessionId": session,
                    "params": {
                        "data": "held",
                        "sessionId": 5000,
                        "metadata": { "deviceWidth": width, "deviceHeight": height },
                    },
                });
                let _ = socket.send(Message::Text(event.to_string().into())).await;
            }
            continue;
        }
        let result = match method.as_str() {
            "Target.attachToTarget" => json!({ "sessionId": session }),
            "Target.getTargets" => {
                json!({ "targetInfos": launcher.targets.lock().unwrap().clone() })
            }
            "Page.getNavigationHistory" => {
                let (current, entries) = launcher.history.lock().unwrap().clone();
                json!({
                    "currentIndex": current,
                    "entries": entries.iter()
                        .map(|(id, url)| json!({ "id": id, "url": url, "title": "" }))
                        .collect::<Vec<_>>(),
                })
            }
            _ => json!({}),
        };
        // A step to a history entry is a navigation of the main frame.
        let stepped = (method == "Page.navigateToHistoryEntry")
            .then(|| {
                let mut history = launcher.history.lock().unwrap();
                let at = history
                    .1
                    .iter()
                    .position(|(id, _)| *id == params["entryId"])?;
                history.0 = at;
                Some(history.1[at].1.clone())
            })
            .flatten();
        let answer =
            json!({ "id": command["id"], "result": result, "sessionId": command["sessionId"] });
        if method == "Input.dispatchMouseEvent"
            && params["type"] == "mousePressed"
            && launcher
                .slow_press
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        if method == "Page.startScreencast" {
            let event = json!({
                "method": "Page.screencastFrame",
                "sessionId": session,
                "params": {
                    "data": format!("stale{frame}"),
                    "sessionId": 1000 + frame,
                    "metadata": { "deviceWidth": width, "deviceHeight": height },
                },
            });
            let _ = socket.send(Message::Text(event.to_string().into())).await;
        }
        if socket
            .send(Message::Text(answer.to_string().into()))
            .await
            .is_err()
        {
            break;
        }
        if method == "Emulation.setDeviceMetricsOverride" {
            width = params["width"].as_f64().unwrap();
            height = params["height"].as_f64().unwrap();
        }
        let quiet = launcher.quiet.load(std::sync::atomic::Ordering::SeqCst);
        if let Some(url) = stepped {
            let event = json!({
                "method": "Page.frameNavigated",
                "sessionId": session,
                "params": { "frame": { "id": "main", "url": url } },
            });
            let _ = socket.send(Message::Text(event.to_string().into())).await;
        }
        if method == "Page.startScreencast" && !quiet {
            frame += 1;
            let event = json!({
                "method": "Page.screencastFrame",
                "sessionId": session,
                "params": {
                    "data": format!("frame{frame}"),
                    "sessionId": frame,
                    "metadata": { "deviceWidth": width, "deviceHeight": height },
                },
            });
            let _ = socket.send(Message::Text(event.to_string().into())).await;
        }
    }
}

async fn other(
    launcher: axum::extract::State<Launcher>,
    uri: axum::http::Uri,
) -> axum::http::StatusCode {
    launcher.seen.lock().unwrap().others.push(uri.to_string());
    axum::http::StatusCode::NOT_FOUND
}

async fn serve(router: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("127.0.0.1:{}", addr.port())
}

struct Setup {
    _dir: tempfile::TempDir,
    state: AppState,
    launcher: Launcher,
    /// The web's address (host:port).
    web: String,
    wake: UnixDatagram,
    job: String,
    item: i64,
}

async fn setup(with_browser: bool) -> Setup {
    setup_pinging(with_browser, PING_EVERY, PONG_WITHIN).await
}

/// [`setup`], with the web pinging every `ping_every` and waiting
/// `pong_within` for an answer.
async fn setup_pinging(with_browser: bool, ping_every: Duration, pong_within: Duration) -> Setup {
    setup_with(with_browser, |remote| {
        remote.with_pings(ping_every, pong_within)
    })
    .await
}

/// [`setup`], with the web taking a page as stalled after `answer_within`
/// (`Page.enable`: twice that) and asking it again every 50 ms.
async fn setup_answering(answer_within: Duration) -> Setup {
    setup_with(true, |remote| {
        remote.with_answers(answer_within, answer_within * 2, Duration::from_millis(50))
    })
    .await
}

/// [`setup`], with the web's remote screens as `configure` makes them.
async fn setup_with(
    with_browser: bool,
    configure: impl FnOnce(RemoteScreens) -> RemoteScreens,
) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let launcher = Launcher::new();
    let launcher_addr = serve(
        Router::new()
            .route("/runs/{run}/cdp", get(cdp))
            .fallback(other)
            .with_state(launcher.clone()),
    )
    .await;
    let wake_path = dir.path().join("app.db.wake");
    let wake = UnixDatagram::bind(&wake_path).unwrap();
    wake.set_nonblocking(true).unwrap();
    let mut state = AppState::new(db.clone()).with_worker_wake(wake_path);
    if with_browser {
        let remote = RemoteScreens::new(&BrowserAccess {
            url: Url::parse(&format!("http://{launcher_addr}")).unwrap(),
            token: TOKEN.to_owned(),
        })
        .unwrap()
        .with_times(Duration::from_millis(200), Duration::from_millis(50));
        state = state.with_remote_screens(configure(remote));
    }
    // The app's own router: its Host and Origin checks are in front of the
    // screen's routes.
    let web = serve(crate::router(dir.path(), state.clone())).await;

    let job = match JobStore::new(db.clone())
        .create(
            NewJob {
                command_id: "c1".to_owned(),
                request: "{}".to_owned(),
                origin: "pick".to_owned(),
                work_id: None,
                season: Some(1),
                anime_no: None,
                source_id: None,
                creator: Some("제작자".to_owned()),
                revision_of: None,
                revises_attributed: false,
                items: vec![NewItem {
                    observation_id: None,
                    episode: "1".to_owned(),
                    post_url: "https://fake.trss.invalid/check/ep1".to_owned(),
                    found_at: 500,
                }],
            },
            900,
        )
        .await
        .unwrap()
    {
        Created::Created(id) => id,
        other => panic!("{other:?}"),
    };
    let item = state.jobs.items(&job).await.unwrap()[0].id;
    Setup {
        _dir: dir,
        state,
        launcher,
        web,
        wake,
        job,
        item,
    }
}

impl Setup {
    /// The job waits for its check with `run` bound, as the worker leaves it.
    async fn waiting_on(&self, run: &str) {
        self.bound_to(run, "T1", 1_000).await;
    }

    /// The job waits for its check with `run`'s page `target` bound at `at`.
    async fn bound_to(&self, run: &str, target: &str, at: i64) {
        let job = self.job.clone();
        self.state
            .jobs
            .db()
            .run::<_, DbError, _>(move |c| {
                c.execute(
                    "UPDATE subtitle_jobs SET state = 'waiting', wait = 'auth' WHERE id = ?1",
                    [&job],
                )?;
                c.execute(
                    "UPDATE subtitle_job_items SET state = 'waiting', wait = 'auth' WHERE job_id = ?1",
                    [&job],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        self.screens()
            .bind(&self.job, self.item, run, target, at)
            .await
            .unwrap();
    }

    fn screens(&self) -> &ScreenStore {
        &self.state.screens
    }

    fn woken(&self) -> usize {
        let mut buf = [0u8; 8];
        let mut n = 0;
        while self.wake.recv(&mut buf).is_ok() {
            n += 1;
        }
        n
    }

    async fn http(&self, method: &str, path: &str) -> (u16, Value) {
        let mut stream = tokio::net::TcpStream::connect(&self.web).await.unwrap();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            self.web
        );
        tokio::io::AsyncWriteExt::write_all(&mut stream, request.as_bytes())
            .await
            .unwrap();
        let mut answer = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut answer)
            .await
            .unwrap();
        let answer = String::from_utf8_lossy(&answer).into_owned();
        let status = answer[9..12].parse().unwrap();
        let body = answer.split("\r\n\r\n").nth(1).unwrap_or_default();
        // Chunked or not, the JSON is the one line that starts with `{`/`n`.
        let json = body
            .lines()
            .find(|l| l.starts_with('{') || l.starts_with('[') || *l == "null")
            .map(|l| serde_json::from_str(l).unwrap())
            .unwrap_or(Value::Null);
        (status, json)
    }

    /// Sends `body` as JSON with `method` to `path`; the answer's status.
    async fn http_json(&self, method: &str, path: &str, body: &Value) -> u16 {
        let body = body.to_string();
        let mut stream = tokio::net::TcpStream::connect(&self.web).await.unwrap();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.web,
            body.len()
        );
        tokio::io::AsyncWriteExt::write_all(&mut stream, request.as_bytes())
            .await
            .unwrap();
        let mut answer = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut answer)
            .await
            .unwrap();
        String::from_utf8_lossy(&answer)[9..12].parse().unwrap()
    }

    /// Opens the job's socket for `run` from a page whose origin is
    /// `origin` (`None`: no `Origin`).
    async fn open(&self, run: &str, origin: Option<&str>) -> Result<Socket, u16> {
        self.open_at(&format!("run={run}"), origin).await
    }

    /// Opens the job's socket with the query `query`.
    async fn open_at(&self, query: &str, origin: Option<&str>) -> Result<Socket, u16> {
        let url = format!(
            "ws://{}/api/subtitle-jobs/{}/screen/socket?{query}",
            self.web, self.job
        );
        let mut request = url.into_client_request().unwrap();
        if let Some(origin) = origin {
            request
                .headers_mut()
                .insert("origin", origin.parse().unwrap());
        }
        match tokio_tungstenite::connect_async(request).await {
            Ok((socket, _)) => Ok(socket),
            Err(tungstenite::Error::Http(response)) => Err(response.status().as_u16()),
            Err(other) => panic!("{other}"),
        }
    }

    fn origin(&self) -> String {
        format!("http://{}", self.web)
    }

    async fn input_at(&self) -> Option<i64> {
        let job = self.job.clone();
        self.state
            .jobs
            .db()
            .run::<_, DbError, _>(move |c| {
                Ok(c.query_row(
                    "SELECT input_at FROM subtitle_job_screens WHERE job_id = ?1",
                    [&job],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap()
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The next message of `socket` of `kind`, skipping others, within 2 s.
async fn next_of(socket: &mut Socket, kind: &str) -> Value {
    next_within(socket, kind, Duration::from_secs(2)).await
}

/// The next message of `socket` of `kind`, skipping others, within `wait`.
async fn next_within(socket: &mut Socket, kind: &str, wait: Duration) -> Value {
    tokio::time::timeout(wait, async {
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Text(text))) => {
                    let value: Value = serde_json::from_str(text.as_str()).unwrap();
                    if value["type"] == kind {
                        return value;
                    }
                }
                Some(Ok(_)) => {}
                other => panic!("the socket ended before {kind}: {other:?}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no {kind} in time"))
}

async fn send(socket: &mut Socket, message: Value) {
    socket
        .send(WsMessage::Text(message.to_string().into()))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_socket_from_another_site_for_a_job_not_waiting_or_for_an_ended_run_is_refused() {
    let s = setup(true).await;
    let origin = s.origin();
    // The job does not wait for its check yet.
    assert_eq!(s.open("run-1", Some(&origin)).await.err(), Some(409));

    s.waiting_on("run-1").await;
    // Another site's page, or none.
    assert_eq!(
        s.open("run-1", Some("http://evil.example")).await.err(),
        Some(403)
    );
    assert_eq!(
        s.open(
            "run-1",
            Some(&format!(
                "https://{}",
                s.web.replace("127.0.0.1", "localhost")
            ))
        )
        .await
        .err(),
        Some(403)
    );
    assert_eq!(s.open("run-1", None).await.err(), Some(403));
    // A name the web does not answer to (DNS rebinding), even with that
    // name's own origin.
    let mut request = format!(
        "ws://{}/api/subtitle-jobs/{}/screen/socket?run=run-1",
        s.web, s.job
    )
    .into_client_request()
    .unwrap();
    let port = s.web.rsplit(':').next().unwrap();
    let rebound = format!("evil.example:{port}");
    request
        .headers_mut()
        .insert("host", rebound.parse().unwrap());
    request
        .headers_mut()
        .insert("origin", format!("http://{rebound}").parse().unwrap());
    match tokio_tungstenite::connect_async(request).await {
        Err(tungstenite::Error::Http(response)) => assert_eq!(response.status().as_u16(), 421),
        other => panic!("{:?}", other.map(|_| ())),
    }
    // A run that is not the bound one.
    assert_eq!(s.open("run-0", Some(&origin)).await.err(), Some(410));
    // The bound run ended.
    s.screens()
        .unbind(&s.job, "run-1", trss_jobs::screen::RUN_ENDED, 2_000)
        .await
        .unwrap();
    assert_eq!(s.open("run-1", Some(&origin)).await.err(), Some(410));
    // No such job.
    let url = format!(
        "ws://{}/api/subtitle-jobs/nope/screen/socket?run=run-1",
        s.web
    );
    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("origin", origin.parse().unwrap());
    match tokio_tungstenite::connect_async(request).await {
        Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 404),
        other => panic!("{:?}", other.map(|_| ())),
    }
    // Nothing reached the browser.
    let seen = s.launcher.seen.lock().unwrap();
    assert!(seen.connections.is_empty());
    assert!(seen.others.is_empty());
}

#[tokio::test]
async fn a_web_without_the_server_browser_shows_no_screen() {
    let s = setup(false).await;
    s.waiting_on("run-1").await;
    assert_eq!(s.open("run-1", Some(&s.origin())).await.err(), Some(503));
    let (status, screen) = s
        .http("POST", &format!("/api/subtitle-jobs/{}/screen", s.job))
        .await;
    assert_eq!(status, 200);
    assert_eq!(screen["state"], "unavailable");
    assert!(s.screens().prepare_requests().await.unwrap().is_empty());
    assert_eq!(s.woken(), 0);
}

#[tokio::test]
async fn reading_the_job_and_the_lists_asks_for_no_run_and_opening_its_page_does() {
    let s = setup(true).await;
    s.waiting_on("run-1").await;
    for path in [
        format!("/api/subtitle-jobs/{}", s.job),
        "/api/subtitle-jobs".to_owned(),
        "/api/todo".to_owned(),
        "/api/todo/count".to_owned(),
    ] {
        let (status, _) = s.http("GET", &path).await;
        assert_eq!(status, 200, "{path}");
    }
    let (_, detail) = s
        .http("GET", &format!("/api/subtitle-jobs/{}", s.job))
        .await;
    assert_eq!(
        detail["screen"],
        json!({ "state": "ready", "run": "run-1", "bound": 1000, "note": null, "popup": false })
    );
    assert!(s.screens().prepare_requests().await.unwrap().is_empty());
    assert_eq!(s.woken(), 0);
    assert_eq!(s.input_at().await, None);

    let (status, screen) = s
        .http("POST", &format!("/api/subtitle-jobs/{}/screen", s.job))
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        screen,
        json!({ "state": "ready", "run": "run-1", "bound": 1000, "note": null, "popup": false })
    );
    assert_eq!(s.screens().prepare_requests().await.unwrap().len(), 1);
    assert_eq!(s.woken(), 1);
    // The web itself starts nothing.
    assert!(s.launcher.seen.lock().unwrap().others.is_empty());

    let (status, _) = s.http("POST", "/api/subtitle-jobs/nope/screen").await;
    assert_eq!(status, 404);
}

/// The job's screen with a popup `P1` listed after the first page `T1`.
async fn with_two_pages(s: &Setup) {
    s.waiting_on("run-1").await;
    s.screens()
        .set_pages(&s.job, "run-1", &["T1".to_owned(), "P1".to_owned()], 1_500)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_person_asks_the_worker_to_show_a_tab_and_the_web_sends_the_browser_nothing() {
    let s = setup(true).await;
    with_two_pages(&s).await;
    let switch = format!("/api/subtitle-jobs/{}/screen/switch", s.job);
    let asked =
        |bound: i64, target: &str| json!({ "run": "run-1", "bound": bound, "target": target });

    // Refused: a binding the person no longer sees, a page the worker did
    // not list, another run.
    assert_eq!(s.http_json("POST", &switch, &asked(999, "P1")).await, 409);
    assert_eq!(s.http_json("POST", &switch, &asked(1_000, "Z9")).await, 409);
    assert_eq!(
        s.http_json(
            "POST",
            &switch,
            &json!({ "run": "run-0", "bound": 1_000, "target": "P1" })
        )
        .await,
        409
    );
    assert_eq!(
        s.screens().take_switch(&s.job, "run-1").await.unwrap(),
        None
    );
    assert_eq!(s.input_at().await, None);

    // Asked for a listed page, for a check as for a find job's screen.
    assert_eq!(s.http_json("POST", &switch, &asked(1_000, "P1")).await, 202);
    assert!(s.input_at().await.is_some(), "it is the person's input");
    assert_eq!(
        s.screens()
            .take_switch(&s.job, "run-1")
            .await
            .unwrap()
            .as_deref(),
        Some("P1")
    );
    // Taken once.
    assert_eq!(
        s.screens().take_switch(&s.job, "run-1").await.unwrap(),
        None
    );
    assert_eq!(
        s.http_json(
            "POST",
            "/api/subtitle-jobs/nope/screen/switch",
            &asked(1_000, "P1")
        )
        .await,
        404
    );
    let seen = s.launcher.seen.lock().unwrap();
    assert!(seen.commands.is_empty() && seen.others.is_empty());
}

#[tokio::test]
async fn a_person_asks_the_worker_to_close_a_tab_but_never_the_first_page() {
    let s = setup(true).await;
    with_two_pages(&s).await;
    let close = format!("/api/subtitle-jobs/{}/screen/close", s.job);
    let asked = |bound: i64, target: Option<&str>| match target {
        Some(target) => json!({ "run": "run-1", "bound": bound, "target": target }),
        None => json!({ "run": "run-1", "bound": bound }),
    };

    // The screen shows the first page: closing "the page shown" and naming it
    // are refused, and so is a page that is not listed or a stale binding.
    assert_eq!(s.http_json("POST", &close, &asked(1_000, None)).await, 409);
    assert_eq!(
        s.http_json("POST", &close, &asked(1_000, Some("T1"))).await,
        409
    );
    assert_eq!(
        s.http_json("POST", &close, &asked(1_000, Some("Z9"))).await,
        409
    );
    assert_eq!(
        s.http_json("POST", &close, &asked(999, Some("P1"))).await,
        409
    );
    assert!(s
        .screens()
        .take_close(&s.job, "run-1")
        .await
        .unwrap()
        .is_empty());
    assert_eq!(s.input_at().await, None);

    // A tab that is not shown closes, for a check as for a find job's screen.
    assert_eq!(
        s.http_json("POST", &close, &asked(1_000, Some("P1"))).await,
        202
    );
    assert!(s.input_at().await.is_some(), "it is the person's input");
    assert_eq!(
        s.screens().take_close(&s.job, "run-1").await.unwrap(),
        vec!["P1".to_owned()]
    );
    // Taken once.
    assert!(s
        .screens()
        .take_close(&s.job, "run-1")
        .await
        .unwrap()
        .is_empty());

    // The page shown, when it is not the first, is the default.
    s.screens()
        .retarget(&s.job, "run-1", "P1", 2_000)
        .await
        .unwrap();
    assert_eq!(s.http_json("POST", &close, &asked(1_000, None)).await, 409);
    assert_eq!(s.http_json("POST", &close, &asked(2_000, None)).await, 202);
    assert_eq!(
        s.screens().take_close(&s.job, "run-1").await.unwrap(),
        vec!["P1".to_owned()]
    );
    // The first page, named while another is shown: still refused.
    assert_eq!(
        s.http_json("POST", &close, &asked(2_000, Some("T1"))).await,
        409
    );
    assert_eq!(
        s.http_json(
            "POST",
            "/api/subtitle-jobs/nope/screen/close",
            &asked(2_000, None)
        )
        .await,
        404
    );
    let seen = s.launcher.seen.lock().unwrap();
    assert!(seen.commands.is_empty() && seen.others.is_empty());
}

#[tokio::test]
async fn a_screen_tells_the_host_only_and_a_step_back_never_goes_before_the_first_page() {
    let s = setup(true).await;
    s.waiting_on("run-1").await;
    let origin = s.origin();
    let mut socket = s.open("run-1", Some(&origin)).await.unwrap();

    // The page the server prepared: a blank page, then the post. Nothing is
    // behind it, and only its host is said.
    let nav = next_of(&mut socket, "nav").await;
    assert_eq!(
        nav,
        json!({ "type": "nav", "back": false, "forward": false, "host": "blog.example.org" })
    );

    // A step back asked anyway goes nowhere, however the device believes.
    send(&mut socket, json!({ "type": "back" })).await;
    send(&mut socket, json!({ "type": "forward" })).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(s.launcher.count("Page.navigateToHistoryEntry"), 0);
    assert!(s.input_at().await.is_some(), "a person's step is input");

    // The person went on to another page: the page can step back to the post
    // (entry 11), and not before it.
    s.launcher
        .set_history(2, &["about:blank", SECRET_PAGE, SECRET_NEXT]);
    send(&mut socket, json!({ "type": "back" })).await;
    let nav = next_of(&mut socket, "nav").await;
    assert_eq!(
        s.launcher.last("Page.navigateToHistoryEntry").unwrap()["entryId"],
        11
    );
    assert_eq!(
        nav,
        json!({ "type": "nav", "back": false, "forward": true, "host": "blog.example.org" })
    );
    send(&mut socket, json!({ "type": "back" })).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(s.launcher.count("Page.navigateToHistoryEntry"), 1);

    send(&mut socket, json!({ "type": "forward" })).await;
    let nav = next_of(&mut socket, "nav").await;
    assert_eq!(
        s.launcher.last("Page.navigateToHistoryEntry").unwrap()["entryId"],
        12
    );
    assert_eq!(
        nav,
        json!({ "type": "nav", "back": true, "forward": false, "host": "files.example.org" })
    );

    // A socket that connects later is told the state at once.
    let mut other = s.open("run-1", Some(&origin)).await.unwrap();
    assert_eq!(next_of(&mut other, "nav").await, nav);
}

#[tokio::test]
async fn no_message_of_a_screen_carries_a_path_a_query_or_a_fragment() {
    let s = setup(true).await;
    with_two_pages(&s).await;
    s.launcher.set_history(1, &["about:blank", SECRET_PAGE]);
    s.launcher.set_targets(json!([
        { "targetId": "T1", "type": "page", "title": SECRET_PAGE, "url": SECRET_PAGE },
        { "targetId": "P1", "type": "page", "title": "받는 곳", "url": SECRET_NEXT },
        { "targetId": "Z9", "type": "page", "title": "not listed", "url": "https://z.example/" },
    ]));
    let origin = s.origin();
    let mut socket = s.open("run-1", Some(&origin)).await.unwrap();

    let mut texts = Vec::new();
    let mut tabs = Value::Null;
    tokio::time::timeout(Duration::from_secs(2), async {
        while tabs.is_null() {
            match socket.next().await {
                Some(Ok(WsMessage::Text(text))) => {
                    let value: Value = serde_json::from_str(text.as_str()).unwrap();
                    if value["type"] == "tabs" {
                        tabs = value.clone();
                    }
                    if value["type"] != "frame" {
                        texts.push(text.to_string());
                    }
                }
                Some(Ok(_)) => {}
                other => panic!("the socket ended: {other:?}"),
            }
        }
    })
    .await
    .expect("no tabs in time");
    // Both listed pages, in the worker's order; the unlisted one is no tab.
    assert_eq!(
        tabs,
        json!({ "type": "tabs", "tabs": [
            { "id": "T1", "title": "", "host": "blog.example.org", "shown": true, "closable": false },
            { "id": "P1", "title": "받는 곳", "host": "files.example.org", "shown": false, "closable": true },
        ] })
    );
    // The worker's list changes (a tab closed): the tabs follow.
    s.screens()
        .set_pages(&s.job, "run-1", &["T1".to_owned()], 2_000)
        .await
        .unwrap();
    let tabs = next_of(&mut socket, "tabs").await;
    assert_eq!(tabs["tabs"].as_array().unwrap().len(), 1);
    texts.push(tabs.to_string());
    for text in texts {
        for secret in ["/post", "/get", "sig", "SECRET", "frag", "?", "#"] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
    }
}

#[tokio::test]
async fn a_closed_screen_is_asked_for_again_only_by_opening_the_page() {
    let s = setup(true).await;
    s.waiting_on("run-1").await;
    s.screens()
        .unbind(&s.job, "run-1", trss_jobs::screen::RUN_ENDED, 2_000)
        .await
        .unwrap();
    let (_, detail) = s
        .http("GET", &format!("/api/subtitle-jobs/{}", s.job))
        .await;
    assert_eq!(detail["screen"]["state"], "closed");
    assert_eq!(detail["screen"]["note"], trss_jobs::screen::RUN_ENDED);
    // A socket that retries does not ask.
    for _ in 0..3 {
        assert_eq!(s.open("run-1", Some(&s.origin())).await.err(), Some(410));
    }
    assert!(s.screens().prepare_requests().await.unwrap().is_empty());
    assert_eq!(s.woken(), 0);

    let (_, screen) = s
        .http("POST", &format!("/api/subtitle-jobs/{}/screen", s.job))
        .await;
    assert_eq!(screen["state"], "preparing");
    assert_eq!(s.woken(), 1);
}

#[tokio::test]
async fn a_reconnect_reaches_the_same_run_and_asks_for_none() {
    let s = setup(true).await;
    s.waiting_on("run-1").await;
    let origin = s.origin();
    let mut first = s.open("run-1", Some(&origin)).await.unwrap();
    next_of(&mut first, "frame").await;
    // A second screen of the same run shares the hub.
    let mut second = s.open("run-1", Some(&origin)).await.unwrap();
    send(&mut second, json!({ "type": "reload" })).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(s.launcher.seen.lock().unwrap().connections["run-1"], 1);
    first.close(None).await.unwrap();
    second.close(None).await.unwrap();
    drop((first, second));
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Reconnected after both went: the same run, through the proxy again.
    let mut again = s.open("run-1", Some(&origin)).await.unwrap();
    next_of(&mut again, "frame").await;
    assert!(s.launcher.seen.lock().unwrap().others.is_empty());
    assert!(s.screens().prepare_requests().await.unwrap().is_empty());
    assert_eq!(s.woken(), 0);
    assert!(s
        .launcher
        .methods("run-1")
        .contains(&"Page.reload".to_owned()));
}

#[tokio::test]
async fn inputs_of_an_older_size_are_dropped_and_only_input_counts_as_use() {
    let s = setup(true).await;
    s.waiting_on("run-1").await;
    let origin = s.origin();
    let mut phone = s.open("run-1", Some(&origin)).await.unwrap();
    // Frames alone are not use.
    next_of(&mut phone, "frame").await;
    send(
        &mut phone,
        json!({ "type": "viewport", "width": 402, "height": 666, "dpr": 3, "touch": true }),
    )
    .await;
    let viewport = next_of(&mut phone, "viewport").await;
    assert_eq!(viewport["gen"], 1);
    // The frame sent before the new screencast's start was answered is of
    // the layout before: never the new generation's.
    let frame = next_of(&mut phone, "frame").await;
    assert_eq!(
        (
            frame["gen"].clone(),
            frame["width"].clone(),
            frame["data"].clone()
        ),
        (json!(1), json!(402.0), json!("frame2"))
    );
    let metrics = s
        .launcher
        .last("Emulation.setDeviceMetricsOverride")
        .unwrap();
    assert_eq!(
        metrics,
        json!({ "width": 402, "height": 666, "deviceScaleFactor": 3.0, "mobile": true })
    );
    assert_eq!(
        s.launcher
            .last("Emulation.setTouchEmulationEnabled")
            .unwrap()["enabled"],
        true
    );
    assert_eq!(s.input_at().await, None);
    assert!(s
        .launcher
        .methods("run-1")
        .contains(&"Page.screencastFrameAck".to_owned()));

    // Made on the frames before the size changed: dropped.
    send(
        &mut phone,
        json!({ "type": "touch", "gen": 0, "event": "touchStart", "points": [{ "x": 10, "y": 10 }] }),
    )
    .await;
    assert_eq!(next_of(&mut phone, "dropped").await["gen"], 1);
    assert!(!s
        .launcher
        .methods("run-1")
        .contains(&"Input.dispatchTouchEvent".to_owned()));
    assert_eq!(s.input_at().await, None);

    send(
        &mut phone,
        json!({ "type": "touch", "gen": 1, "event": "touchStart", "points": [{ "x": 10, "y": 10 }] }),
    )
    .await;
    send(
        &mut phone,
        json!({ "type": "text", "gen": 1, "text": "안녕" }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let methods = s.launcher.methods("run-1");
    assert!(methods.contains(&"Input.dispatchTouchEvent".to_owned()));
    assert!(methods.contains(&"Input.insertText".to_owned()));
    let first_input = s.input_at().await.expect("input is use");
    assert_eq!(
        s.screens().live_inputs().await.unwrap(),
        vec![("run-1".to_owned(), first_input)]
    );

    // A PC opens the same screen last: its size wins, and the phone's input
    // of the size before is dropped. The phone's finger, still down, is let
    // go on the page first.
    let mut pc = s.open("run-1", Some(&origin)).await.unwrap();
    send(
        &mut pc,
        json!({ "type": "viewport", "width": 1272, "height": 753, "dpr": 1, "touch": false }),
    )
    .await;
    assert_eq!(next_of(&mut phone, "viewport").await["gen"], 2);
    let cancel = s.launcher.last("Input.dispatchTouchEvent").unwrap();
    assert_eq!(cancel["type"], "touchCancel");
    let frame = next_of(&mut pc, "frame").await;
    assert_eq!(
        (frame["gen"].clone(), frame["data"].clone()),
        (json!(2), json!("frame3"))
    );
    let clicks = |s: &Setup| {
        s.launcher
            .methods("run-1")
            .iter()
            .filter(|m| *m == "Input.dispatchMouseEvent")
            .count()
    };
    send(
        &mut phone,
        json!({ "type": "mouse", "gen": 1, "event": "mousePressed", "x": 5, "y": 5, "button": "left", "clickCount": 1 }),
    )
    .await;
    assert_eq!(next_of(&mut phone, "dropped").await["gen"], 2);
    assert_eq!(clicks(&s), 0);
    send(
        &mut pc,
        json!({ "type": "mouse", "gen": 2, "event": "mousePressed", "x": 1000, "y": 700, "button": "left", "clickCount": 1 }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(clicks(&s), 1);
    // Recorded at most every 200 ms here (10 s in the app).
    tokio::time::sleep(Duration::from_millis(250)).await;
    send(
        &mut pc,
        json!({ "type": "mouse", "gen": 2, "event": "mouseReleased", "x": 1000, "y": 700, "button": "left", "clickCount": 1 }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(s.input_at().await.unwrap() > first_input);

    // A button held when the size changes is let go without a click.
    send(
        &mut pc,
        json!({ "type": "mouse", "gen": 2, "event": "mousePressed", "x": 30, "y": 40, "button": "left", "clickCount": 1 }),
    )
    .await;
    send(
        &mut pc,
        json!({ "type": "viewport", "width": 1000, "height": 700, "dpr": 1, "touch": false }),
    )
    .await;
    assert_eq!(next_of(&mut pc, "viewport").await["gen"], 3);
    let release = s.launcher.last("Input.dispatchMouseEvent").unwrap();
    assert_eq!(
        (
            release["type"].clone(),
            release["clickCount"].clone(),
            release["x"].clone()
        ),
        (json!("mouseReleased"), json!(0), json!(30.0))
    );
}

#[tokio::test]
async fn a_frame_sent_before_the_new_screencast_started_is_never_the_new_sizes() {
    let s = setup(true).await;
    s.waiting_on("run-1").await;
    let mut phone = s.open("run-1", Some(&s.origin())).await.unwrap();
    next_of(&mut phone, "frame").await;
    s.launcher
        .quiet
        .store(true, std::sync::atomic::Ordering::SeqCst);
    send(
        &mut phone,
        json!({ "type": "viewport", "width": 402, "height": 666, "dpr": 3, "touch": true }),
    )
    .await;
    assert_eq!(next_of(&mut phone, "viewport").await["gen"], 1);
    // Only the frame from before the start's answer came: of the size
    // before, though it reports the new size. Nothing of generation 1 is
    // sent.
    let shown = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            match phone.next().await {
                Some(Ok(WsMessage::Text(text))) => {
                    let value: Value = serde_json::from_str(text.as_str()).unwrap();
                    if value["type"] == "frame" && value["gen"] == 1 {
                        return value;
                    }
                }
                Some(Ok(_)) => {}
                other => panic!("{other:?}"),
            }
        }
    })
    .await;
    assert!(shown.is_err(), "{shown:?}");
}

#[tokio::test]
async fn another_check_in_the_same_run_ends_the_socket_and_is_connected_to_anew() {
    let s = setup(true).await;
    s.bound_to("run-1", "T1", 1_000).await;
    let origin = s.origin();
    let mut first = s
        .open_at("run=run-1&bound=1000", Some(&origin))
        .await
        .unwrap();
    next_of(&mut first, "frame").await;
    // The first check passed and the second is on another page of the run.
    s.bound_to("run-1", "T2", 2_000).await;
    assert_eq!(next_of(&mut first, "ended").await["reason"], "run");
    let (_, detail) = s
        .http("GET", &format!("/api/subtitle-jobs/{}", s.job))
        .await;
    assert_eq!(detail["screen"]["run"], "run-1");
    assert_eq!(detail["screen"]["bound"], 2000);
    // The binding before is gone; the new one is its own connection to its
    // own page.
    assert_eq!(
        s.open_at("run=run-1&bound=1000", Some(&origin)).await.err(),
        Some(410)
    );
    let mut second = s
        .open_at("run=run-1&bound=2000", Some(&origin))
        .await
        .unwrap();
    next_of(&mut second, "frame").await;
    assert_eq!(s.launcher.seen.lock().unwrap().connections["run-1"], 2);
    assert_eq!(
        s.launcher.last("Target.attachToTarget").unwrap()["targetId"],
        "T2"
    );
    // A page of the same binding that comes again (the same page bound
    // anew) is a new binding too.
    s.bound_to("run-1", "T2", 3_000).await;
    assert_eq!(next_of(&mut second, "ended").await["reason"], "run");
}

/// Sends an input of a generation that never was, and waits for its
/// `dropped`: the socket is served (and seated).
async fn served(socket: &mut Socket) {
    send(socket, json!({ "type": "text", "gen": 999, "text": "x" })).await;
    next_of(socket, "dropped").await;
}

#[tokio::test]
async fn a_screen_past_four_takes_the_place_of_the_oldest() {
    let s = setup(true).await;
    s.waiting_on("run-1").await;
    let origin = s.origin();
    let mut open = Vec::new();
    for _ in 0..MAX_SOCKETS {
        let mut socket = s.open("run-1", Some(&origin)).await.unwrap();
        served(&mut socket).await;
        open.push(socket);
    }
    let mut newest = s.open("run-1", Some(&origin)).await.unwrap();
    served(&mut newest).await;
    // The oldest is told, and closed.
    let mut oldest = open.remove(0);
    assert_eq!(next_of(&mut oldest, "ended").await["reason"], "replaced");
    let closed = tokio::time::timeout(Duration::from_secs(2), oldest.next())
        .await
        .unwrap();
    match closed {
        Some(Ok(WsMessage::Close(Some(frame)))) => {
            assert_eq!(u16::from(frame.code), 1000);
        }
        other => panic!("{other:?}"),
    }
    // The others stay.
    for socket in &mut open {
        served(socket).await;
    }
    // A screen that closes gives its place back: the next one replaces no one.
    let mut gone = open.pop().unwrap();
    gone.close(None).await.unwrap();
    drop(gone);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut again = s.open("run-1", Some(&origin)).await.unwrap();
    served(&mut again).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    for socket in &mut open {
        served(socket).await;
    }
    served(&mut newest).await;
    assert_eq!(s.launcher.seen.lock().unwrap().connections["run-1"], 1);
}

#[tokio::test]
async fn a_socket_that_answers_no_ping_is_dropped() {
    let s = setup_pinging(true, Duration::from_millis(100), Duration::from_millis(100)).await;
    s.waiting_on("run-1").await;
    let origin = s.origin();
    // A device that is there: it reads, and so answers the pings.
    let mut alive = s.open("run-1", Some(&origin)).await.unwrap();
    served(&mut alive).await;
    let pings = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let listening = tokio::spawn({
        let pings = pings.clone();
        async move {
            while let Some(Ok(message)) = alive.next().await {
                if matches!(message, WsMessage::Ping(_)) {
                    pings.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }
    });
    // A device that went away: nothing is read, so no ping is answered.
    let mut dead = s.open("run-1", Some(&origin)).await.unwrap();
    served(&mut dead).await;

    tokio::time::sleep(Duration::from_millis(800)).await;
    // The dead one was dropped without an `ended` (reading it now answers
    // the pings it got, too late).
    let rest = tokio::time::timeout(Duration::from_secs(2), async {
        let mut seen = Vec::new();
        while let Some(Ok(message)) = dead.next().await {
            if let WsMessage::Text(text) = &message {
                let value: Value = serde_json::from_str(text.as_str()).unwrap();
                seen.push(value["type"].as_str().unwrap_or_default().to_owned());
            }
            if matches!(message, WsMessage::Close(_)) {
                seen.push("close".to_owned());
            }
        }
        seen
    })
    .await
    .expect("the dead socket is dropped");
    assert!(!rest.contains(&"ended".to_owned()), "{rest:?}");
    assert!(!rest.contains(&"close".to_owned()), "{rest:?}");
    // The other one is still open, and was pinged.
    assert!(!listening.is_finished());
    assert!(pings.load(std::sync::atomic::Ordering::SeqCst) >= 3);
    listening.abort();
}

#[tokio::test]
async fn a_socket_ends_when_the_run_is_no_longer_the_jobs_or_the_browser_goes() {
    let s = setup(true).await;
    s.waiting_on("run-1").await;
    let origin = s.origin();
    let mut socket = s.open("run-1", Some(&origin)).await.unwrap();
    next_of(&mut socket, "frame").await;
    // The worker cleared the binding (the run was reaped).
    s.screens()
        .unbind(&s.job, "run-1", trss_jobs::screen::RUN_ENDED, 2_000)
        .await
        .unwrap();
    assert_eq!(next_of(&mut socket, "ended").await["reason"], "run");

    // A new binding; then its browser goes.
    s.waiting_on("run-2").await;
    let mut socket = s.open("run-2", Some(&origin)).await.unwrap();
    next_of(&mut socket, "frame").await;
    s.launcher.end("run-2");
    assert_eq!(next_of(&mut socket, "ended").await["reason"], "browser");
    assert!(s.launcher.seen.lock().unwrap().others.is_empty());
}

#[tokio::test]
async fn a_press_admitted_just_before_a_change_of_size_is_let_go_by_it() {
    let s = setup(true).await;
    s.waiting_on("run-1").await;
    let origin = s.origin();
    let mut pc = s.open("run-1", Some(&origin)).await.unwrap();
    send(
        &mut pc,
        json!({ "type": "viewport", "width": 1272, "height": 753, "dpr": 1, "touch": false }),
    )
    .await;
    assert_eq!(next_of(&mut pc, "viewport").await["gen"], 1);
    let mut phone = s.open("run-1", Some(&origin)).await.unwrap();
    served(&mut phone).await;

    // The press is admitted on generation 1 and still on its way to the page
    // when the phone's size comes.
    s.launcher
        .slow_press
        .store(true, std::sync::atomic::Ordering::SeqCst);
    send(
        &mut pc,
        json!({ "type": "mouse", "gen": 1, "event": "mousePressed", "x": 10, "y": 20, "button": "left", "clickCount": 1 }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    send(
        &mut phone,
        json!({ "type": "viewport", "width": 402, "height": 666, "dpr": 3, "touch": true }),
    )
    .await;
    assert_eq!(next_of(&mut phone, "viewport").await["gen"], 2);

    // The change of size waited for the press and let it go, before it laid
    // the page out anew.
    let commands: Vec<(String, Value)> = s
        .launcher
        .seen
        .lock()
        .unwrap()
        .commands
        .iter()
        .map(|(_, m, p)| (m.clone(), p.clone()))
        .collect();
    let at = |what: &dyn Fn(&(String, Value)) -> bool| commands.iter().position(what);
    let pressed = at(&|(m, p)| m == "Input.dispatchMouseEvent" && p["type"] == "mousePressed")
        .expect("the press went to the page");
    let released = at(&|(m, p)| m == "Input.dispatchMouseEvent" && p["type"] == "mouseReleased")
        .expect("the press is let go");
    let laid_out = commands
        .iter()
        .rposition(|(m, _)| m == "Page.stopScreencast")
        .unwrap();
    assert!(pressed < released && released < laid_out, "{commands:?}");
    assert_eq!(commands[released].1["clickCount"], 0);
}

/// The next frame of `socket` whose data is `data`, skipping others.
async fn frame_of(socket: &mut Socket, data: &str) -> Value {
    loop {
        let frame = next_of(socket, "frame").await;
        if frame["data"] == data {
            return frame;
        }
    }
}

#[tokio::test]
async fn a_page_that_does_not_answer_stalls_the_screen_and_comes_back_without_ending_it() {
    use std::sync::atomic::Ordering::SeqCst;
    let s = setup_answering(Duration::from_millis(800)).await;
    s.waiting_on("run-1").await;
    let origin = s.origin();
    let mut phone = s.open("run-1", Some(&origin)).await.unwrap();
    send(
        &mut phone,
        json!({ "type": "viewport", "width": 402, "height": 666, "dpr": 3, "touch": true }),
    )
    .await;
    assert_eq!(next_of(&mut phone, "viewport").await["gen"], 1);
    next_of(&mut phone, "frame").await;

    // The page's thread is held: a tap gets no answer. What comes meanwhile
    // is still sent to the socket.
    s.launcher.stuck.store(true, SeqCst);
    let sent = tokio::time::Instant::now();
    send(
        &mut phone,
        json!({ "type": "touch", "gen": 1, "event": "touchStart", "points": [{ "x": 10, "y": 10 }] }),
    )
    .await;
    frame_of(&mut phone, "held").await;
    assert!(
        sent.elapsed() < Duration::from_millis(600),
        "{:?}",
        sent.elapsed()
    );
    // Then the page is stalled, and the screen goes on.
    let page = next_within(&mut phone, "page", Duration::from_secs(3)).await;
    assert_eq!(page["responding"], false);

    // Inputs are not sent to it meanwhile, and a size waits for it.
    let touches = s.launcher.count("Input.dispatchTouchEvent");
    send(
        &mut phone,
        json!({ "type": "touch", "gen": 1, "event": "touchMove", "points": [{ "x": 12, "y": 12 }] }),
    )
    .await;
    send(
        &mut phone,
        json!({ "type": "viewport", "width": 390, "height": 844, "dpr": 3, "touch": true }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(s.launcher.count("Input.dispatchTouchEvent"), touches);
    assert_eq!(
        s.launcher
            .last("Emulation.setDeviceMetricsOverride")
            .unwrap()["width"],
        402
    );
    // A screen that connects now is told.
    let mut pc = s.open("run-1", Some(&origin)).await.unwrap();
    assert_eq!(next_of(&mut pc, "page").await["responding"], false);

    // The page answers again: the screens are told, and the page is laid out
    // anew at the last size as a new generation.
    s.launcher.stuck.store(false, SeqCst);
    let page = next_within(&mut phone, "page", Duration::from_secs(3)).await;
    assert_eq!(page["responding"], true);
    let viewport = next_of(&mut phone, "viewport").await;
    assert_eq!(
        (viewport["gen"].clone(), viewport["width"].clone()),
        (json!(2), json!(390))
    );
    assert_eq!(next_of(&mut pc, "page").await["responding"], true);
    // The finger that was down when it stalled is let go, and inputs reach
    // the page again.
    assert_eq!(
        s.launcher.last("Input.dispatchTouchEvent").unwrap()["type"],
        "touchCancel"
    );
    send(
        &mut phone,
        json!({ "type": "touch", "gen": 2, "event": "touchStart", "points": [{ "x": 20, "y": 20 }] }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        s.launcher.last("Input.dispatchTouchEvent").unwrap()["type"],
        "touchStart"
    );
    assert!(s.launcher.seen.lock().unwrap().others.is_empty());
}

#[tokio::test]
async fn inputs_that_pile_up_behind_one_the_page_does_not_answer_are_let_go_but_never_a_size() {
    use std::sync::atomic::Ordering::SeqCst;
    let s = setup_answering(Duration::from_millis(1500)).await;
    s.waiting_on("run-1").await;
    let origin = s.origin();
    let mut phone = s.open("run-1", Some(&origin)).await.unwrap();
    send(
        &mut phone,
        json!({ "type": "viewport", "width": 402, "height": 666, "dpr": 3, "touch": true }),
    )
    .await;
    assert_eq!(next_of(&mut phone, "viewport").await["gen"], 1);
    next_of(&mut phone, "frame").await;

    s.launcher.stuck.store(true, SeqCst);
    send(
        &mut phone,
        json!({ "type": "touch", "gen": 1, "event": "touchStart", "points": [{ "x": 10, "y": 10 }] }),
    )
    .await;
    // More than the queue takes, then a size.
    for n in 0..100 {
        send(
            &mut phone,
            json!({ "type": "mouse", "gen": 1, "event": "mouseMoved", "x": n, "y": 5 }),
        )
        .await;
    }
    send(
        &mut phone,
        json!({ "type": "viewport", "width": 390, "height": 844, "dpr": 3, "touch": true }),
    )
    .await;
    assert_eq!(
        next_within(&mut phone, "page", Duration::from_secs(3)).await["responding"],
        false
    );
    s.launcher.stuck.store(false, SeqCst);
    assert_eq!(
        next_within(&mut phone, "page", Duration::from_secs(4)).await["responding"],
        true
    );
    let viewport = next_of(&mut phone, "viewport").await;
    assert_eq!(
        (viewport["gen"].clone(), viewport["width"].clone()),
        (json!(2), json!(390))
    );
    // None of the moves reached the page.
    assert_eq!(s.launcher.count("Input.dispatchMouseEvent"), 0);
}

#[test]
fn a_full_queue_lets_moves_go_but_never_a_press_a_release_a_key_or_a_size() {
    let message = |value: Value| serde_json::from_value::<Incoming>(value).unwrap();
    let moves = [
        json!({ "type": "mouse", "gen": 1, "event": "mouseMoved", "x": 1, "y": 1 }),
        json!({ "type": "mouse", "gen": 1, "event": "mouseWheel", "x": 1, "y": 1, "deltaY": 40 }),
        json!({ "type": "touch", "gen": 1, "event": "touchMove", "points": [{ "x": 1, "y": 1 }] }),
    ];
    let kept = [
        json!({ "type": "mouse", "gen": 1, "event": "mousePressed", "x": 1, "y": 1, "button": "left" }),
        json!({ "type": "mouse", "gen": 1, "event": "mouseReleased", "x": 1, "y": 1, "button": "left" }),
        json!({ "type": "touch", "gen": 1, "event": "touchStart", "points": [{ "x": 1, "y": 1 }] }),
        json!({ "type": "touch", "gen": 1, "event": "touchEnd", "points": [] }),
        json!({ "type": "touch", "gen": 1, "event": "touchCancel", "points": [] }),
        json!({ "type": "key", "gen": 1, "event": "keyUp", "key": "a" }),
        json!({ "type": "text", "gen": 1, "text": "가" }),
        json!({ "type": "viewport", "width": 402, "height": 666, "dpr": 3, "touch": true }),
        json!({ "type": "reload" }),
        json!({ "type": "back" }),
    ];
    for value in moves {
        assert!(
            takes(&message(value.clone()), KEPT_FROM_MOTION + 1),
            "{value}"
        );
        assert!(!takes(&message(value.clone()), KEPT_FROM_MOTION), "{value}");
    }
    for value in kept {
        assert!(takes(&message(value.clone()), 1), "{value}");
    }
}

#[tokio::test]
async fn a_screen_that_opens_on_a_page_that_does_not_answer_ends_stuck() {
    use std::sync::atomic::Ordering::SeqCst;
    let s = setup_answering(Duration::from_millis(200)).await;
    s.waiting_on("run-1").await;
    let origin = s.origin();
    s.launcher.stuck.store(true, SeqCst);
    let mut socket = s.open("run-1", Some(&origin)).await.unwrap();
    let ended = next_of(&mut socket, "ended").await;
    assert_eq!(ended["reason"], "stuck");

    // Once the page answers, a screen of the same binding connects.
    s.launcher.stuck.store(false, SeqCst);
    let mut socket = s.open("run-1", Some(&origin)).await.unwrap();
    next_of(&mut socket, "frame").await;
    assert!(s.launcher.seen.lock().unwrap().others.is_empty());
}

#[tokio::test]
async fn a_person_asks_the_worker_to_start_the_run_anew_and_the_web_ends_no_run() {
    let s = setup(true).await;
    s.waiting_on("run-1").await;
    let restart = format!("/api/subtitle-jobs/{}/screen/restart", s.job);

    // Refused: a binding the person no longer sees, another run.
    assert_eq!(
        s.http_json("POST", &restart, &json!({ "run": "run-1", "bound": 999 }))
            .await,
        409
    );
    assert_eq!(
        s.http_json("POST", &restart, &json!({ "run": "run-0", "bound": 1_000 }))
            .await,
        409
    );
    assert!(s.screens().prepare_requests().await.unwrap().is_empty());
    assert_eq!(s.woken(), 0);
    assert_eq!(s.input_at().await, None);

    // Asked for the binding the person sees: a request to prepare that
    // starts the run anew, and the worker is woken for it.
    assert_eq!(
        s.http_json("POST", &restart, &json!({ "run": "run-1", "bound": 1_000 }))
            .await,
        202
    );
    let requests = s.screens().prepare_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].restart);
    assert_eq!(s.woken(), 1);
    assert!(s.input_at().await.is_some(), "it is the person's input");
    // The web itself ends nothing.
    assert!(s.launcher.seen.lock().unwrap().others.is_empty());

    assert_eq!(
        s.http_json(
            "POST",
            "/api/subtitle-jobs/nope/screen/restart",
            &json!({ "run": "run-1", "bound": 1_000 })
        )
        .await,
        404
    );
}
