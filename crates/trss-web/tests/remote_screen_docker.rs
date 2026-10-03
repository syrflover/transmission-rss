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
//! It starts a throwaway container named `trss-remote-screen-<pid>` (removed
//! at the end). Where the check box is, the test reads from the page through
//! a DevTools connection of its own, standing in for the person who sees it
//! in the frame.

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

async fn start(downloads: &Path) -> (Container, Url) {
    let name = format!("trss-remote-screen-{}", std::process::id());
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
    let (_container, base) = start(&downloads).await;

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
