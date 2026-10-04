//! A job of the fake check post from start to end with the real server
//! browser (ignored): the worker's runner brings the post to the check and
//! waits (`인증`), a tap relayed through the web's remote screen passes it,
//! the browser downloads the file, and the job receives it (`받기`, then
//! done) and lets the run go (`docs/specs/jobs.md`, 작업 화면 안의 인증과
//! 브라우저 수명).
//!
//! Needs Docker and the browser image (`TRSS_BROWSER_IMAGE`, default
//! `ghcr.io/syrflover/trss-browser:local`, built from `Dockerfile.browser`):
//!
//! ```sh
//! cargo test -p trss-web --test remote_screen_docker -- --ignored --nocapture
//! ```
//!
//! It starts a throwaway container named `trss-remote-screen-<pid>-<test>`
//! (removed at the end). Where the check box is, the test reads from the page
//! through a DevTools connection of its own, standing in for the person who
//! sees it in the frame.
//!
//! The second test is a find job's screen with the browser controls: back
//! and forward with the blank page the server passed through as the floor,
//! the host as the only address that is sent, a popup that becomes a tab, and
//! switching to and closing tabs (ticket 0055).
//!
//! The third is a find job's page that its own script holds: the screen
//! says the page does not answer and stays, comes back when the page moves
//! again, ends `stuck` when opened on a page held for good, and a person's
//! ask for a new run opens the post again in a new run (ticket 0056).
//!
//! The fourth is the page's dialogs on a find job's screen: a confirm and a
//! prompt a tap opens are answered from the screen, a `beforeunload` of the
//! page itself is asked and one after the person's reload is left at once,
//! and an alert left open when the screen closes is dismissed (ticket 0057).

use std::{
    path::Path,
    process::{Command, Output},
    sync::Arc,
    time::Duration,
};

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::{net::TcpListener, sync::Notify};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
use tokio_util::sync::CancellationToken;
use trss_browser::{
    cdp::Connection, client::LauncherClient, BrowserPolicy, BrowserPool, PolicySource, PoolConfig,
};
use trss_core::Db;
use trss_jobs::{
    Created, FileState, Format, JobState, JobStore, NewItem, NewJob, ReceiveArea, Runner,
    ScreenState, StepKind, StepState, Wait,
};
use trss_subtitles::{
    auth::BrowserAuth,
    fake::{self, FakeSource},
    Sources,
};
use trss_web::{env::BrowserAccess, screen_api::RemoteScreens, AppState};
use url::Url;

const TOKEN: &str = "remote-screen-token";

fn image() -> String {
    std::env::var("TRSS_BROWSER_IMAGE")
        .unwrap_or_else(|_| "ghcr.io/syrflover/trss-browser:local".to_owned())
}

fn docker(args: &[&str]) -> Output {
    Command::new("docker").args(args).output().expect("docker")
}

fn docker_ok(args: &[&str]) -> String {
    let out = docker(args);
    assert!(
        out.status.success(),
        "docker {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Removes the container at the end, whatever happened.
struct Container(String);

impl Drop for Container {
    fn drop(&mut self) {
        let _ = docker(&["rm", "-f", &self.0]);
    }
}

async fn start(downloads: &Path, test: &str) -> (Container, Url) {
    let name = format!("trss-remote-screen-{}-{test}", std::process::id());
    let _ = docker(&["rm", "-f", &name]);
    let container = Container(name.clone());
    std::fs::set_permissions(
        downloads,
        std::os::unix::fs::PermissionsExt::from_mode(0o1777),
    )
    .unwrap();
    docker_ok(&[
        "run",
        "-d",
        "--name",
        &name,
        "--memory",
        "768m",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges:true",
        "-e",
        &format!("TRSS_BROWSER_TOKEN={TOKEN}"),
        "-v",
        &format!("{}:/downloads", downloads.display()),
        &image(),
    ]);
    let ip = docker_ok(&[
        "inspect",
        "-f",
        "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
        &name,
    ]);
    let base = Url::parse(&format!("http://{ip}:9230")).unwrap();
    let client = LauncherClient::new(base.clone(), TOKEN).unwrap();
    for _ in 0..80 {
        if client.list().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    (container, base)
}

/// Polls `check` every 100 ms for up to `wait`.
async fn until<F, Fut>(wait: Duration, what: &str, check: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + wait;
    while !check().await {
        assert!(
            tokio::time::Instant::now() < deadline,
            "not in time: {what}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next_of(socket: &mut Socket, kind: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
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

#[tokio::test]
#[ignore = "needs docker and the trss-browser image"]
async fn a_tap_relayed_through_the_remote_screen_passes_the_check_and_the_file_is_received() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = dir.path().join("browser-downloads");
    std::fs::create_dir(&downloads).unwrap();
    let (_container, base) = start(&downloads, "check").await;

    // The worker's side.
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let store = JobStore::new(db.clone());
    let config = PoolConfig::new(base.clone(), TOKEN, &downloads).with_activity(
        trss_browser::ActivitySource::new({
            let screens = trss_jobs::ScreenStore::new(db.clone());
            move || {
                let screens = screens.clone();
                Box::pin(async move { screens.live_inputs().await.unwrap_or_default() })
            }
        }),
    );
    let pool = BrowserPool::new(
        config,
        trss_core::system_clock(),
        PolicySource::fixed(BrowserPolicy::default()),
    )
    .await
    .unwrap();
    let area = ReceiveArea::in_app_data(dir.path());
    let runner = Runner::new(
        store.clone(),
        Sources::none().with_fake(FakeSource),
        area.clone(),
        trss_core::system_clock(),
    )
    .with_auth(BrowserAuth::shared(pool.clone()));
    let job = match store
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
    let cancel = CancellationToken::new();
    runner.run_ready(&cancel).await.unwrap();
    let detail = store.detail(&job).await.unwrap().unwrap();
    assert_eq!(
        (detail.row.state, detail.row.wait),
        (JobState::Waiting, Some(Wait::Auth)),
        "{:?}",
        detail.events
    );
    let screen = runner.screens().screen(&job).await.unwrap().unwrap();
    assert_eq!(screen.state, ScreenState::Ready);
    let (run, target) = (screen.run_id.unwrap(), screen.target_id.unwrap());
    println!("the check is on screen: run {run}");

    // The worker's loops: the screens, and the jobs when woken.
    let wake = Arc::new(Notify::new());
    let worker = tokio::spawn({
        let (runner, wake, cancel) = (runner.clone(), wake.clone(), cancel.clone());
        async move {
            while !cancel.is_cancelled() {
                runner.tend_screens(&wake, &cancel).await.unwrap();
                tokio::select! {
                    _ = wake.notified() => {
                        runner.run_ready(&cancel).await.unwrap();
                    }
                    _ = tokio::time::sleep(Duration::from_millis(200)) => {}
                }
            }
        }
    });

    // The web's side.
    let state = AppState::new(db.clone()).with_remote_screens(
        RemoteScreens::new(&BrowserAccess {
            url: base.clone(),
            token: TOKEN.to_owned(),
        })
        .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let web = listener.local_addr().unwrap();
    // The app's own router, with its Host and Origin checks.
    let router = trss_web::router(dir.path(), state);
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut request = format!("ws://{web}/api/subtitle-jobs/{job}/screen/socket?run={run}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", format!("http://{web}").parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();

    // A phone opens the screen.
    let (width, height) = (402, 666);
    socket
        .send(Message::Text(
            json!({ "type": "viewport", "width": width, "height": height, "dpr": 3, "touch": true })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let viewport = next_of(&mut socket, "viewport").await;
    let gen = viewport["gen"].as_u64().unwrap();
    let frame = next_of(&mut socket, "frame").await;
    assert_eq!(frame["gen"].as_u64(), Some(gen));
    assert_eq!(
        (frame["width"].as_f64(), frame["height"].as_f64()),
        (Some(width as f64), Some(height as f64))
    );
    let jpeg = frame["data"].as_str().unwrap();
    // `/9j/` is the start of a JPEG (FF D8 FF) in base64.
    assert!(jpeg.starts_with("/9j/"), "not a JPEG frame");
    println!("a frame of {width}x{height}: {} base64 bytes", jpeg.len());

    // Where the person sees the box (the page keeps it in the middle).
    let eyes = Connection::connect(
        &LauncherClient::new(base.clone(), TOKEN)
            .unwrap()
            .cdp_url(&run),
        TOKEN,
    )
    .await
    .unwrap();
    let session = eyes
        .command(
            None,
            "Target.attachToTarget",
            json!({ "targetId": target, "flatten": true }),
        )
        .await
        .unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let rect = eyes
        .command(
            Some(&session),
            "Runtime.evaluate",
            json!({
                "expression": "JSON.stringify(document.getElementById('check').getBoundingClientRect())",
                "returnByValue": true,
            }),
        )
        .await
        .unwrap();
    let rect: Value = serde_json::from_str(rect["result"]["value"].as_str().unwrap()).unwrap();
    let (x, y) = (
        rect["x"].as_f64().unwrap() + rect["width"].as_f64().unwrap() / 2.0,
        rect["y"].as_f64().unwrap() + rect["height"].as_f64().unwrap() / 2.0,
    );
    println!("the box is at {x:.0},{y:.0} of the phone's screen");
    assert!(x > 0.0 && x < width as f64 && y > 0.0 && y < height as f64);
    drop(eyes);

    // A tap made on an older size goes nowhere.
    let tap = |gen: u64, event: &str| {
        Message::Text(
            json!({ "type": "touch", "gen": gen, "event": event,
                    "points": if event == "touchEnd" { json!([]) } else { json!([{ "x": x, "y": y }]) } })
            .to_string()
            .into(),
        )
    };
    socket.send(tap(gen - 1, "touchStart")).await.unwrap();
    assert_eq!(
        next_of(&mut socket, "dropped").await["gen"].as_u64(),
        Some(gen)
    );

    // The person taps the box.
    socket.send(tap(gen, "touchStart")).await.unwrap();
    socket.send(tap(gen, "touchEnd")).await.unwrap();

    // The file came: the screen ends, as the binding is cleared (`run`) or,
    // when the job is received first, as the run goes (`browser`).
    let ended = next_of(&mut socket, "ended").await;
    println!("the screen ended: {}", ended["reason"]);
    assert!(ended["reason"] == "run" || ended["reason"] == "browser");
    until(
        Duration::from_secs(30),
        "the job received the file",
        || async { store.detail(&job).await.unwrap().unwrap().row.state == JobState::Done },
    )
    .await;
    let detail = store.detail(&job).await.unwrap().unwrap();
    let step = |kind| {
        detail
            .steps
            .iter()
            .find(|s| s.step == kind)
            .map(|s| s.state)
    };
    assert_eq!(step(StepKind::Auth), Some(StepState::Done));
    assert_eq!(step(StepKind::Receive), Some(StepState::Done));
    let file = &detail.items[0].files[0];
    assert_eq!(
        (file.state, file.name.as_str(), file.format),
        (FileState::Done, "ep1.srt", Some(Format::Srt))
    );
    assert_eq!(
        std::fs::read(area.at(file.path.as_deref().unwrap())).unwrap(),
        fake::srt("ep1")
    );
    for event in detail.events.iter().rev() {
        println!(
            "log: {} {}",
            event.message,
            event.detail.as_deref().unwrap_or_default()
        );
    }
    // The job ended, so its run did.
    until(Duration::from_secs(10), "the run ended", || async {
        pool.run_of_job(&job).is_none()
    })
    .await;
    assert!(runner.screens().screen(&job).await.unwrap().is_none());

    cancel.cancel();
    let _ = worker.await;
    pool.shutdown().await;
}

// ---------------------------------------------------------------------------
// The browser controls of a find job's screen (ticket 0055)

/// A socket of the screen, with every message it carried but frames.
struct Watched {
    socket: Socket,
    /// Every message but frames, in arrival order.
    seen: Vec<String>,
    /// Whether `next` took the message of `seen` at the same index.
    taken: Vec<bool>,
}

impl Watched {
    /// The first message of `kind` not taken yet (messages that came before
    /// it stay for their own `next`), within 30 s.
    async fn next(&mut self, kind: &str) -> Value {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                for (i, text) in self.seen.iter().enumerate() {
                    let value: Value = serde_json::from_str(text).unwrap();
                    if !self.taken[i] && value["type"] == kind {
                        self.taken[i] = true;
                        return value;
                    }
                }
                match self.socket.next().await {
                    Some(Ok(Message::Text(text))) => {
                        let value: Value = serde_json::from_str(text.as_str()).unwrap();
                        if value["type"] != "frame" {
                            self.seen.push(text.to_string());
                            self.taken.push(false);
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

    async fn send(&mut self, message: Value) {
        self.socket
            .send(Message::Text(message.to_string().into()))
            .await
            .unwrap();
    }

    /// The generation of the last size the screen was told.
    fn gen(&self) -> u64 {
        self.seen
            .iter()
            .rev()
            .map(|text| serde_json::from_str::<Value>(text).unwrap())
            .find(|value| value["type"] == "viewport")
            .and_then(|value| value["gen"].as_u64())
            .expect("a size was told")
    }

    /// The next frame, within 30 s (the messages before it are kept).
    async fn frame(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match self.socket.next().await {
                    Some(Ok(Message::Text(text))) => {
                        let value: Value = serde_json::from_str(text.as_str()).unwrap();
                        if value["type"] == "frame" {
                            return value;
                        }
                        self.seen.push(text.to_string());
                        self.taken.push(false);
                    }
                    Some(Ok(_)) => {}
                    other => panic!("the socket ended before a frame: {other:?}"),
                }
            }
        })
        .await
        .expect("a frame in time")
    }

    /// Every message but frames that comes within `wait`, kept in `seen`.
    async fn drain(&mut self, wait: Duration) {
        let _ = tokio::time::timeout(wait, async {
            while let Some(Ok(message)) = self.socket.next().await {
                if let Message::Text(text) = message {
                    let value: Value = serde_json::from_str(text.as_str()).unwrap();
                    if value["type"] != "frame" {
                        self.seen.push(text.to_string());
                        self.taken.push(false);
                    }
                }
            }
        })
        .await;
    }

    /// Whether a message of `kind` came (taken or not).
    fn saw(&self, kind: &str) -> bool {
        self.seen
            .iter()
            .any(|text| serde_json::from_str::<Value>(text).unwrap()["type"] == kind)
    }

    /// A tap of one finger at (`x`, `y`) on the generation `gen`.
    async fn tap(&mut self, gen: u64, x: f64, y: f64) {
        self.send(json!({ "type": "touch", "gen": gen, "event": "touchStart",
                          "points": [{ "x": x, "y": y }] }))
            .await;
        self.send(json!({ "type": "touch", "gen": gen, "event": "touchEnd", "points": [] }))
            .await;
    }
}

/// Opens the job's screen socket for the binding (`run`, `bound`) as a phone
/// does: its size first.
async fn open_screen(web: std::net::SocketAddr, job: &str, run: &str, bound: i64) -> Watched {
    let mut screen = connect_screen(web, job, run, bound).await;
    screen.next("viewport").await;
    screen
}

/// [`open_screen`] without waiting for the size to be applied.
async fn connect_screen(web: std::net::SocketAddr, job: &str, run: &str, bound: i64) -> Watched {
    let mut request =
        format!("ws://{web}/api/subtitle-jobs/{job}/screen/socket?run={run}&bound={bound}")
            .into_client_request()
            .unwrap();
    request
        .headers_mut()
        .insert("origin", format!("http://{web}").parse().unwrap());
    let (socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let mut screen = Watched {
        socket,
        seen: Vec::new(),
        taken: Vec::new(),
    };
    screen
        .send(json!({ "type": "viewport", "width": 402, "height": 666, "dpr": 3, "touch": true }))
        .await;
    screen
}

/// A POST of `body` as JSON to the web's `path`: the answer's status.
async fn post(web: std::net::SocketAddr, path: &str, body: &Value) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body = body.to_string();
    let mut stream = tokio::net::TcpStream::connect(web).await.unwrap();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {web}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.unwrap();
    String::from_utf8_lossy(&answer)[9..12].parse().unwrap()
}

/// A DevTools connection of the test's own to the first page: the person's
/// eyes and finger (it clicks where the page says the link is).
struct Eyes {
    conn: Connection,
    session: String,
}

impl Eyes {
    async fn on(base: &Url, run: &str, target: &str) -> Eyes {
        let conn = Connection::connect(
            &LauncherClient::new(base.clone(), TOKEN)
                .unwrap()
                .cdp_url(run),
            TOKEN,
        )
        .await
        .unwrap();
        let session = conn
            .command(
                None,
                "Target.attachToTarget",
                json!({ "targetId": target, "flatten": true }),
            )
            .await
            .unwrap()["sessionId"]
            .as_str()
            .unwrap()
            .to_owned();
        Eyes { conn, session }
    }

    async fn send(&self, method: &str, params: Value) -> Value {
        self.conn
            .command(Some(&self.session), method, params)
            .await
            .unwrap()
    }

    async fn eval(&self, expression: &str) -> Value {
        self.send(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true }),
        )
        .await["result"]["value"]
            .clone()
    }

    async fn title(&self) -> String {
        self.eval("document.title")
            .await
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    async fn title_ends_with(&self, ending: &str) {
        until(Duration::from_secs(20), ending, || async {
            self.title().await.ends_with(ending)
        })
        .await;
    }

    /// A trusted click in the middle of the element `id`.
    async fn click(&self, id: &str) {
        let rect = self
            .eval(&format!(
                "JSON.stringify(document.getElementById('{id}').getBoundingClientRect())"
            ))
            .await;
        let rect: Value = serde_json::from_str(rect.as_str().unwrap()).unwrap();
        let x = rect["x"].as_f64().unwrap() + rect["width"].as_f64().unwrap() / 2.0;
        let y = rect["y"].as_f64().unwrap() + rect["height"].as_f64().unwrap() / 2.0;
        for (kind, button, count) in [
            ("mouseMoved", "none", 0),
            ("mousePressed", "left", 1),
            ("mouseReleased", "left", 1),
        ] {
            self.send(
                "Input.dispatchMouseEvent",
                json!({ "type": kind, "x": x, "y": y, "button": button, "clickCount": count }),
            )
            .await;
        }
    }

    /// The page's history: the index of the entry shown, and the addresses.
    async fn history(&self) -> (usize, Vec<String>) {
        let history = self.send("Page.getNavigationHistory", json!({})).await;
        (
            history["currentIndex"].as_u64().unwrap() as usize,
            history["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["url"].as_str().unwrap().to_owned())
                .collect(),
        )
    }
}

/// A find job at the fake blog's newest post, brought to its screen by the
/// worker's runner, which goes on tending it, with a web in front: what the
/// find screen tests share.
struct FindWorld {
    _dir: tempfile::TempDir,
    _container: Container,
    base: Url,
    store: JobStore,
    pool: BrowserPool,
    screens: trss_jobs::ScreenStore,
    job: String,
    web: std::net::SocketAddr,
    cancel: CancellationToken,
    worker: tokio::task::JoinHandle<()>,
}

async fn find_world(test: &str) -> FindWorld {
    let dir = tempfile::tempdir().unwrap();
    let downloads = dir.path().join("browser-downloads");
    std::fs::create_dir(&downloads).unwrap();
    let (container, base) = start(&downloads, test).await;

    // The worker's side: a find job at the fake blog's newest post.
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    db.run::<_, trss_core::DbError, _>(|c| {
        c.execute(
            "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
             VALUES ('src-maker', 3441, '메이커', 1)",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let store = JobStore::new(db.clone());
    let pool = BrowserPool::new(
        PoolConfig::new(base.clone(), TOKEN, &downloads),
        trss_core::system_clock(),
        PolicySource::fixed(BrowserPolicy::default()),
    )
    .await
    .unwrap();
    let area = ReceiveArea::in_app_data(dir.path());
    let runner = Runner::new(
        store.clone(),
        Sources::none().with_fake(FakeSource),
        area,
        trss_core::system_clock(),
    )
    .with_auth(BrowserAuth::shared(pool.clone()));
    let job = match store
        .create_find(
            trss_jobs::NewFind {
                command_id: "find-1".to_owned(),
                request: r#"{"find":{}}"#.to_owned(),
                work_id: "w1".to_owned(),
                season: 1,
                anime_no: 3441,
                source_id: "src-maker".to_owned(),
                creator: "메이커".to_owned(),
                post_url: format!("https://{}/blog/maker", fake::HOST),
            },
            900,
        )
        .await
        .unwrap()
    {
        Created::Created(id) => id,
        other => panic!("{other:?}"),
    };
    let cancel = CancellationToken::new();
    runner.run_ready(&cancel).await.unwrap();
    let wake = Arc::new(Notify::new());
    let worker = tokio::spawn({
        let (runner, wake, cancel) = (runner.clone(), wake.clone(), cancel.clone());
        async move {
            while !cancel.is_cancelled() {
                runner.tend_screens(&wake, &cancel).await.unwrap();
                tokio::select! {
                    _ = wake.notified() => {
                        runner.run_ready(&cancel).await.unwrap();
                    }
                    _ = tokio::time::sleep(Duration::from_millis(200)) => {}
                }
            }
        }
    });

    // The web's side.
    let state = AppState::new(db.clone()).with_remote_screens(
        RemoteScreens::new(&BrowserAccess {
            url: base.clone(),
            token: TOKEN.to_owned(),
        })
        .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let web = listener.local_addr().unwrap();
    let router = trss_web::router(dir.path(), state);
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    FindWorld {
        _dir: dir,
        _container: container,
        base,
        store,
        pool,
        screens: runner.screens().clone(),
        job,
        web,
        cancel,
        worker,
    }
}

impl FindWorld {
    async fn end(self) {
        self.cancel.cancel();
        let _ = self.worker.await;
        self.pool.shutdown().await;
    }
}

#[tokio::test]
#[ignore = "needs docker and the trss-browser image"]
async fn a_find_screen_steps_back_and_forward_never_before_its_first_page_and_switches_and_closes_tabs(
) {
    let world = find_world("find").await;
    let FindWorld {
        ref base,
        ref screens,
        ref job,
        web,
        ..
    } = world;
    let first = screens.screen(job).await.unwrap().unwrap();
    assert_eq!(first.state, ScreenState::Ready);
    let (run, first_target) = (
        first.run_id.clone().unwrap(),
        first.target_id.clone().unwrap(),
    );
    let (switch, close) = (
        format!("/api/subtitle-jobs/{job}/screen/switch"),
        format!("/api/subtitle-jobs/{job}/screen/close"),
    );
    let mut every_message: Vec<String> = Vec::new();

    // The page the server prepared has a blank page behind it, and the screen
    // says nothing can be stepped back to, with the host and no more.
    let eyes = Eyes::on(base, &run, &first_target).await;
    let newest = fake::BLOG_POSTS;
    eyes.title_ends_with(&format!("{newest}화")).await;
    let (current, urls) = eyes.history().await;
    println!("the page's history: entry {current} of {}", urls.len());
    assert_eq!(
        urls[0], "about:blank",
        "the server passes through a blank page"
    );
    assert!(current >= 1);
    let mut screen = open_screen(web, job, &run, first.bound_at.unwrap()).await;
    let nav = screen.next("nav").await;
    assert_eq!(
        nav,
        json!({ "type": "nav", "back": false, "forward": false, "host": fake::HOST })
    );
    let tabs = screen.next("tabs").await;
    assert_eq!(
        tabs["tabs"],
        json!([{ "id": first_target, "title": format!("가짜 블로그 maker {newest}화"),
                 "host": fake::HOST, "shown": true, "closable": false }])
    );

    // A step back asked anyway does not go to the blank page.
    screen.send(json!({ "type": "back" })).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(eyes.history().await.0, current);
    assert!(eyes.title().await.ends_with(&format!("{newest}화")));

    // The person goes to the post before: back is on, then forward.
    eyes.click("before").await;
    eyes.title_ends_with(&format!("{}화", newest - 1)).await;
    let nav = screen.next("nav").await;
    assert_eq!(
        (nav["back"].clone(), nav["forward"].clone()),
        (json!(true), json!(false))
    );
    screen.send(json!({ "type": "back" })).await;
    eyes.title_ends_with(&format!("{newest}화")).await;
    let nav = screen.next("nav").await;
    assert_eq!(
        (nav["back"].clone(), nav["forward"].clone()),
        (json!(false), json!(true))
    );
    assert_eq!(eyes.history().await.0, current, "back stops at the post");
    screen.send(json!({ "type": "forward" })).await;
    eyes.title_ends_with(&format!("{}화", newest - 1)).await;
    let nav = screen.next("nav").await;
    assert_eq!(
        (nav["back"].clone(), nav["forward"].clone()),
        (json!(true), json!(false))
    );
    assert_eq!(nav["host"], fake::HOST);
    // The steps are the person's input for the run's idle end.
    assert!(!screens.live_inputs().await.unwrap().is_empty());
    println!("back and forward went between the posts and never before the first page");

    // A window the post opens that stays is shown, and is a tab.
    eyes.click("popup").await;
    until(Duration::from_secs(20), "the popup is shown", || async {
        screens
            .screen(job)
            .await
            .unwrap()
            .unwrap()
            .target_id
            .as_deref()
            != Some(first_target.as_str())
    })
    .await;
    let ended = screen.next("ended").await;
    assert_eq!(ended["reason"], "run");
    every_message.append(&mut screen.seen.clone());
    let popup_screen = screens.screen(job).await.unwrap().unwrap();
    let popup = popup_screen.target_id.clone().unwrap();
    assert_eq!(
        popup_screen.pages,
        vec![first_target.clone(), popup.clone()]
    );
    let mut screen = open_screen(web, job, &run, popup_screen.bound_at.unwrap()).await;
    screen.next("nav").await;
    let tabs = screen.next("tabs").await;
    let tabs = tabs["tabs"].as_array().unwrap();
    assert_eq!(tabs.len(), 2, "{tabs:?}");
    assert_eq!(
        (tabs[0]["id"].as_str(), tabs[0]["closable"].clone()),
        (Some(first_target.as_str()), json!(false))
    );
    assert_eq!(
        (
            tabs[1]["id"].as_str(),
            tabs[1]["shown"].clone(),
            tabs[1]["closable"].clone()
        ),
        (Some(popup.as_str()), json!(true), json!(true))
    );
    assert_eq!(tabs[1]["host"], fake::HOST);
    println!(
        "the popup is a tab: {} and {}",
        tabs[0]["title"], tabs[1]["title"]
    );

    // Requests: the old binding, a page that is not listed and the first
    // page's close are refused.
    let (bound, stale) = (popup_screen.bound_at.unwrap(), first.bound_at.unwrap());
    assert_eq!(
        post(
            web,
            &switch,
            &json!({ "run": run, "bound": stale, "target": first_target })
        )
        .await,
        409
    );
    assert_eq!(
        post(
            web,
            &switch,
            &json!({ "run": run, "bound": bound, "target": "nope" })
        )
        .await,
        409
    );
    assert_eq!(
        post(
            web,
            &close,
            &json!({ "run": run, "bound": bound, "target": first_target })
        )
        .await,
        409
    );

    // The person goes to the first tab: the screen moves there and stays.
    assert_eq!(
        post(
            web,
            &switch,
            &json!({ "run": run, "bound": bound, "target": first_target })
        )
        .await,
        202
    );
    let ended = screen.next("ended").await;
    assert_eq!(ended["reason"], "run");
    every_message.append(&mut screen.seen.clone());
    tokio::time::sleep(Duration::from_secs(3)).await;
    let back_on_first = screens.screen(job).await.unwrap().unwrap();
    assert_eq!(
        back_on_first.target_id.as_deref(),
        Some(first_target.as_str())
    );
    let mut screen = open_screen(web, job, &run, back_on_first.bound_at.unwrap()).await;
    screen.next("nav").await;
    let tabs = screen.next("tabs").await;
    let shown: Vec<_> = tabs["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["shown"] == true)
        .map(|t| t["id"].clone())
        .collect();
    assert_eq!(shown, vec![json!(first_target)], "the choice is not undone");
    // The popup was in front of the first page in the browser, which draws
    // only the tab in front.
    screen.frame().await;
    println!("the switch to the first tab was kept across the follower's looks, and it is drawn");

    // The popup is a hidden tab now: closing it closes that page only, and
    // the screen stays as it is.
    let bound = back_on_first.bound_at.unwrap();
    assert_eq!(
        post(
            web,
            &close,
            &json!({ "run": run, "bound": bound, "target": popup })
        )
        .await,
        202
    );
    let tabs = screen.next("tabs").await;
    assert_eq!(tabs["tabs"].as_array().unwrap().len(), 1, "{tabs}");
    assert_eq!(
        screens.screen(job).await.unwrap().unwrap().bound_at,
        Some(bound),
        "closing a hidden tab leaves the screen as it is"
    );
    println!("a hidden tab was closed and the screen stayed");

    // A new popup is shown; closing the shown one goes back to the first page.
    eyes.click("popup").await;
    until(
        Duration::from_secs(20),
        "the popup is shown again",
        || async {
            screens
                .screen(job)
                .await
                .unwrap()
                .unwrap()
                .target_id
                .as_deref()
                != Some(first_target.as_str())
        },
    )
    .await;
    let ended = screen.next("ended").await;
    assert_eq!(ended["reason"], "run");
    every_message.append(&mut screen.seen.clone());
    let second = screens.screen(job).await.unwrap().unwrap();
    assert_eq!(
        post(
            web,
            &close,
            &json!({ "run": run, "bound": second.bound_at.unwrap() })
        )
        .await,
        202
    );
    until(
        Duration::from_secs(20),
        "the screen is back on the first page",
        || async {
            let now = screens.screen(job).await.unwrap().unwrap();
            now.target_id.as_deref() == Some(first_target.as_str())
                && now.pages == vec![first_target.clone()]
        },
    )
    .await;
    let back = screens.screen(job).await.unwrap().unwrap();
    open_screen(web, job, &run, back.bound_at.unwrap())
        .await
        .frame()
        .await;
    println!("the shown tab was closed and the screen went back to the first page, drawn");

    // Whatever was sent over the socket named only hosts.
    for text in &every_message {
        for secret in ["/blog", "maker/", "?", "#"] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
    }
    println!(
        "{} messages checked: no path, query or fragment",
        every_message.len()
    );

    world.end().await;
}

// ---------------------------------------------------------------------------
// A page that does not answer (ticket 0056)

#[tokio::test]
#[ignore = "needs docker and the trss-browser image"]
async fn a_page_held_by_its_script_stalls_the_screen_and_a_new_run_opens_the_post_again() {
    let world = find_world("stuck").await;
    let FindWorld {
        ref base,
        ref store,
        ref pool,
        ref screens,
        ref job,
        web,
        ..
    } = world;
    let first = screens.screen(job).await.unwrap().unwrap();
    let (run, target, bound) = (
        first.run_id.clone().unwrap(),
        first.target_id.clone().unwrap(),
        first.bound_at.unwrap(),
    );
    let eyes = Eyes::on(base, &run, &target).await;
    eyes.title_ends_with(&format!("{}화", fake::BLOG_POSTS))
        .await;
    let mut screen = open_screen(web, job, &run, bound).await;
    screen.next("nav").await;
    let gen = screen.gen();

    // The page's script holds it for 9 s; a tap meanwhile is not answered,
    // and the screen says so and stays.
    eyes.eval(
        "setTimeout(() => { const end = Date.now() + 9000; while (Date.now() < end) {} }, 100); 0",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let tapped = std::time::Instant::now();
    screen.tap(gen, 200.0, 600.0).await;
    let page = screen.next("page").await;
    assert_eq!(page["responding"], false, "{page}");
    println!(
        "the screen said the page does not answer {:?} after the tap",
        tapped.elapsed()
    );

    // The page moves again: the screen says so, with a new generation.
    let page = screen.next("page").await;
    assert_eq!(page["responding"], true, "{page}");
    let viewport = screen.next("viewport").await;
    assert!(viewport["gen"].as_u64().unwrap() > gen, "{viewport}");
    println!(
        "the page answered again {:?} after the tap, as generation {}",
        tapped.elapsed(),
        viewport["gen"]
    );

    // Inputs go to the page again.
    eyes.eval("window.taps = 0; addEventListener('touchstart', () => window.taps++); 0")
        .await;
    let gen = screen.gen();
    screen.tap(gen, 200.0, 600.0).await;
    until(
        Duration::from_secs(10),
        "the tap reached the page",
        || async { eyes.eval("window.taps").await == json!(1) },
    )
    .await;
    println!("a tap reached the page that moved again");

    // The script holds the page for good: a screen opened anew on it ends
    // `stuck`.
    eyes.eval("setTimeout(() => { while (true) {} }, 100); 0")
        .await;
    drop(eyes);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let gen = screen.gen();
    screen.tap(gen, 200.0, 600.0).await;
    let page = screen.next("page").await;
    assert_eq!(page["responding"], false, "{page}");
    drop(screen);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let opened = std::time::Instant::now();
    let mut again = connect_screen(web, job, &run, bound).await;
    let ended = again.next("ended").await;
    assert_eq!(ended["reason"], "stuck", "{ended}");
    println!(
        "a screen opened on the held page ended stuck in {:?}",
        opened.elapsed()
    );

    // A new run for another binding is refused; for this one the worker
    // lets the run go and opens the post again in a new run.
    let restart = format!("/api/subtitle-jobs/{job}/screen/restart");
    assert_eq!(
        post(web, &restart, &json!({ "run": run, "bound": bound - 1 })).await,
        409
    );
    assert_eq!(
        post(web, &restart, &json!({ "run": run, "bound": bound })).await,
        202
    );
    let asked = std::time::Instant::now();
    until(
        Duration::from_secs(60),
        "a new run shows the post",
        || async {
            let now = screens.screen(job).await.unwrap().unwrap();
            now.state == ScreenState::Ready
                && now.run_id.as_deref().is_some_and(|r| r != run)
                && now.bound_at.is_some()
        },
    )
    .await;
    let second = screens.screen(job).await.unwrap().unwrap();
    let new_run = second.run_id.clone().unwrap();
    println!(
        "a new run showed the post {:?} after the ask",
        asked.elapsed()
    );
    assert_eq!(
        pool.run_of_job(job).map(|r| r.run_id().to_owned()),
        Some(new_run.clone())
    );
    let mut screen = open_screen(web, job, &new_run, second.bound_at.unwrap()).await;
    let nav = screen.next("nav").await;
    assert_eq!(nav["host"], fake::HOST);
    let frame = screen.frame().await;
    assert_eq!(frame["gen"].as_u64(), Some(screen.gen()));
    let events = store.detail(job).await.unwrap().unwrap().events;
    assert!(
        events
            .iter()
            .any(|e| e.message == trss_jobs::screen::RESTARTED_FIND),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| e.message == trss_jobs::screen::RUN_ENDED),
        "{events:?}"
    );
    println!("the screen of the new run shows the post");

    world.end().await;
}

// ---------------------------------------------------------------------------
// The page's dialogs (ticket 0057)

#[tokio::test]
#[ignore = "needs docker and the trss-browser image"]
async fn the_pages_dialogs_are_answered_from_the_screen_and_none_is_left_open() {
    let world = find_world("dialogs").await;
    let FindWorld {
        ref base,
        ref screens,
        ref job,
        web,
        ..
    } = world;
    let first = screens.screen(job).await.unwrap().unwrap();
    let (run, target, bound) = (
        first.run_id.clone().unwrap(),
        first.target_id.clone().unwrap(),
        first.bound_at.unwrap(),
    );
    let eyes = Eyes::on(base, &run, &target).await;
    eyes.title_ends_with(&format!("{}화", fake::BLOG_POSTS))
        .await;
    let mut screen = open_screen(web, job, &run, bound).await;
    screen.next("nav").await;
    let mut every_message: Vec<String> = Vec::new();

    // A button whose click asks to confirm.
    let button = |script: &str| {
        format!(
            "document.getElementById('ask')?.remove();
             document.body.insertAdjacentHTML('beforeend', '<button id=ask style=\"position:fixed;left:0;top:0;width:300px;height:200px;z-index:2147483647\">ask</button>');
             document.getElementById('ask').onclick = () => {{ {script} }}; 0"
        )
    };
    eyes.eval(&button("window.answer = confirm('정말 받을까요?')"))
        .await;
    let gen = screen.gen();
    screen.tap(gen, 100.0, 100.0).await;
    let dialog = screen.next("dialog").await["dialog"].clone();
    assert_eq!(
        (
            dialog["kind"].clone(),
            dialog["message"].clone(),
            dialog["host"].clone()
        ),
        (json!("confirm"), json!("정말 받을까요?"), json!(fake::HOST))
    );
    // Waiting for the person is not a page that does not answer.
    screen.drain(Duration::from_secs(7)).await;
    assert!(!screen.saw("page"), "{:?}", screen.seen);
    assert!(!screen.saw("ended"), "{:?}", screen.seen);
    screen
        .send(json!({ "type": "dialog", "id": dialog["id"], "accept": true }))
        .await;
    assert_eq!(screen.next("dialog").await["dialog"], Value::Null);
    assert_eq!(eyes.eval("window.answer").await, json!(true));
    println!("a confirm a tap opened was shown and its 확인 reached the page");

    // A prompt: the person's text reaches the page.
    eyes.eval(&button("window.answer = prompt('이름은요?', '기본')"))
        .await;
    let gen = screen.gen();
    screen.tap(gen, 100.0, 100.0).await;
    let dialog = screen.next("dialog").await["dialog"].clone();
    assert_eq!(
        (dialog["kind"].clone(), dialog["prompt"].clone()),
        (json!("prompt"), json!("기본"))
    );
    screen
        .send(json!({ "type": "dialog", "id": dialog["id"], "accept": true, "text": "답" }))
        .await;
    assert_eq!(screen.next("dialog").await["dialog"], Value::Null);
    assert_eq!(eyes.eval("window.answer").await, json!("답"));
    println!("a prompt got the person's text");

    // The page goes of itself: the person is asked and stays.
    eyes.eval(
        "window.marker = 1;
         addEventListener('beforeunload', (e) => { e.preventDefault(); e.returnValue = ''; });
         setTimeout(() => location.reload(), 100); 0",
    )
    .await;
    let dialog = screen.next("dialog").await["dialog"].clone();
    assert_eq!(dialog["kind"], "beforeunload");
    screen
        .send(json!({ "type": "dialog", "id": dialog["id"], "accept": false }))
        .await;
    assert_eq!(screen.next("dialog").await["dialog"], Value::Null);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        eyes.eval("window.marker").await,
        json!(1),
        "the page stayed"
    );
    println!("the page's own leaving was asked, and it stayed");

    // The person's reload: the page asks, and it is left without asking
    // the person. The test's own connection sees the dialog the screen is
    // never sent.
    every_message.append(&mut screen.seen.clone());
    let asked = screen.seen.len();
    let mut events = eyes.conn.events();
    eyes.send("Page.enable", json!({})).await;
    screen.send(json!({ "type": "reload" })).await;
    until(
        Duration::from_secs(15),
        "the page was read anew",
        || async { eyes.eval("typeof window.marker").await == json!("undefined") },
    )
    .await;
    eyes.send("Page.disable", json!({})).await;
    screen.drain(Duration::from_secs(1)).await;
    assert!(
        !screen.seen[asked..]
            .iter()
            .any(|text| text.contains("\"dialog\"")),
        "{:?}",
        &screen.seen[asked..]
    );
    let mut dialogs = Vec::new();
    while let Ok(event) = events.try_recv() {
        if event.session_id.as_deref() == Some(eyes.session.as_str())
            && event.method.starts_with("Page.javascriptDialog")
        {
            dialogs.push((
                event.method,
                event.params["type"].clone(),
                event.params["result"].clone(),
            ));
        }
    }
    assert_eq!(
        dialogs,
        [
            (
                "Page.javascriptDialogOpening".to_owned(),
                json!("beforeunload"),
                Value::Null
            ),
            (
                "Page.javascriptDialogClosed".to_owned(),
                Value::Null,
                json!(true)
            ),
        ]
    );
    println!("the person's reload brought the page's ask, and it was left without asking");

    // An alert left open when the screen closes is dismissed: a screen
    // opened anew is not stuck, and the page answers.
    eyes.eval("setTimeout(() => alert('알림'), 200); 0").await;
    let dialog = screen.next("dialog").await["dialog"].clone();
    assert_eq!(
        (dialog["kind"].clone(), dialog["message"].clone()),
        (json!("alert"), json!("알림"))
    );
    every_message.append(&mut screen.seen.clone());
    drop(screen);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let mut screen = open_screen(web, job, &run, bound).await;
    screen.frame().await;
    assert_eq!(eyes.eval("1 + 1").await, json!(2));
    every_message.append(&mut screen.seen.clone());
    println!("the alert left open was dismissed and the screen opened again");

    // Whatever was sent over the socket named only hosts. A dialog's text is
    // the page's own words, as the frames are, and is left out.
    for text in &every_message {
        let mut value: Value = serde_json::from_str(text).unwrap();
        if let Some(dialog) = value["dialog"].as_object_mut() {
            dialog.remove("message");
            dialog.remove("prompt");
        }
        let text = value.to_string();
        for secret in ["/blog", "maker/", "?", "#"] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
    }
    println!(
        "{} messages checked: no path, query or fragment",
        every_message.len()
    );
    drop(eyes);
    world.end().await;
}
