//! The launcher over HTTP, with a fake Chromium (`trss-fake-chromium`) that
//! serves `/json/version` and an echo WebSocket.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use futures::{SinkExt, StreamExt};
use reqwest::{Client, Method, StatusCode};
use tempfile::TempDir;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use trss_browser::{
    launcher::{api, Config, Launcher},
    protocol::{RunList, Started},
};

const TOKEN: &str = "test-token";

struct Harness {
    _dir: TempDir,
    base: String,
    runs_dir: PathBuf,
    downloads_dir: PathBuf,
    client: Client,
    launcher: Launcher,
}

async fn harness_with(tweak: impl FnOnce(&mut Config)) -> Harness {
    let dir = TempDir::new().unwrap();
    let runs_dir = dir.path().join("runs");
    let downloads_dir = dir.path().join("downloads");
    std::fs::create_dir_all(&downloads_dir).unwrap();
    let mut config = Config::new(TOKEN);
    config.chromium = env!("CARGO_BIN_EXE_trss-fake-chromium").into();
    config.runs_dir = runs_dir.clone();
    config.downloads_dir = downloads_dir.clone();
    config.display = ":77".to_owned();
    config.kill_grace = Duration::from_secs(2);
    config.ready_timeout = Duration::from_secs(10);
    tweak(&mut config);
    let launcher = Launcher::open(config).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn({
        let app = api::router(launcher.clone());
        async move { axum::serve(listener, app).await }
    });
    Harness {
        _dir: dir,
        base,
        runs_dir,
        downloads_dir,
        client: Client::new(),
        launcher,
    }
}

async fn harness() -> Harness {
    harness_with(|_| {}).await
}

impl Harness {
    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, String) {
        let mut request = self
            .client
            .request(method, format!("http://{}{path}", self.base))
            .bearer_auth(TOKEN);
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        (status, response.text().await.unwrap())
    }

    async fn start(&self, id: &str) -> (StatusCode, String) {
        self.call(
            Method::POST,
            "/runs",
            Some(serde_json::json!({ "run": id })),
        )
        .await
    }

    async fn started(&self, id: &str) -> Started {
        let (status, body) = self.start(id).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        serde_json::from_str(&body).unwrap()
    }

    async fn runs(&self) -> RunList {
        let (status, body) = self.call(Method::GET, "/runs", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        serde_json::from_str(&body).unwrap()
    }

    async fn run_ids(&self) -> Vec<String> {
        self.runs().await.runs.into_iter().map(|r| r.run).collect()
    }

    async fn end(&self, id: &str) {
        let (status, body) = self
            .call(Method::DELETE, &format!("/runs/{id}"), None)
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }

    async fn connect(
        &self,
        id: &str,
    ) -> Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::Error,
    > {
        let mut request = format!("ws://{}/runs/{id}/cdp", self.base)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
        connect_async(request).await.map(|(socket, _)| socket)
    }
}

async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !check().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn every_request_needs_the_token() {
    let h = harness().await;
    let client = Client::new();
    let url = |path: &str| format!("http://{}{path}", h.base);

    let requests = [
        (Method::POST, "/runs"),
        (Method::GET, "/runs"),
        (Method::DELETE, "/runs/a"),
        (Method::POST, "/reset"),
        (Method::GET, "/runs/a/cdp"),
    ];
    for (method, path) in requests {
        for auth in [
            None,
            Some("Bearer wrong"),
            Some("Bearer"),
            Some("Basic test-token"),
            Some(TOKEN),
        ] {
            let mut request = client
                .request(method.clone(), url(path))
                .header("content-type", "application/json")
                .body(r#"{"run":"a"}"#);
            if let Some(auth) = auth {
                request = request.header("authorization", auth);
            }
            let status = request.send().await.unwrap().status();
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{method} {path} with {auth:?}"
            );
        }
    }
    // Nothing was started by those.
    assert!(h.run_ids().await.is_empty());

    // The upgrade is refused before it is an upgrade.
    let refused = connect_async(format!("ws://{}/runs/a/cdp", h.base)).await;
    assert!(
        matches!(
            refused,
            Err(tokio_tungstenite::tungstenite::Error::Http(ref r)) if r.status() == 401
        ),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_start_gives_the_run_a_profile_and_a_downloads_folder() {
    let h = harness().await;
    let started = h.started("job-1").await;
    assert_eq!(started.run, "job-1");
    assert_eq!(
        PathBuf::from(&started.downloads),
        h.downloads_dir.join("job-1")
    );
    assert!(h.downloads_dir.join("job-1").is_dir());

    let profile = h.runs_dir.join("job-1");
    let args = std::fs::read_to_string(profile.join("args.txt")).unwrap();
    let args: Vec<&str> = args.lines().collect();
    for wanted in [
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-dev-shm-usage",
        "--window-size=1280,800",
        "--force-device-scale-factor=1",
        "about:blank",
    ] {
        assert!(args.contains(&wanted), "{wanted} missing from {args:?}");
    }
    assert!(args.contains(&format!("--user-data-dir={}", profile.display()).as_str()));
    assert!(args
        .iter()
        .any(|a| a.starts_with("--remote-debugging-port=")));
    // Nothing that would mark the browser as automated or hide that.
    for flag in args.iter() {
        for forbidden in [
            "headless",
            "automation",
            "stealth",
            "disable-blink-features",
            "user-agent",
        ] {
            assert!(!flag.contains(forbidden), "{flag}");
        }
    }
    // Windowed on the display, not headless.
    assert_eq!(
        std::fs::read_to_string(profile.join("display.txt")).unwrap(),
        ":77"
    );
    // Out through the launcher's proxy alone.
    let proxy = h.launcher.proxy_addr();
    for wanted in [
        format!("--proxy-server=http://{proxy}"),
        "--proxy-bypass-list=<-loopback>".to_owned(),
        "--disable-quic".to_owned(),
        "--webrtc-ip-handling-policy=disable_non_proxied_udp".to_owned(),
    ] {
        assert!(
            args.contains(&wanted.as_str()),
            "{wanted} missing from {args:?}"
        );
    }
}

/// The browser's proxy does not lead to the launcher, or anything else on
/// loopback: what a page asks of it for those is refused before any
/// connection is made.
#[tokio::test]
async fn the_browsers_proxy_does_not_reach_the_launcher() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let h = harness().await;
    let proxy = h.launcher.proxy_addr();
    assert!(proxy.ip().is_loopback());
    for head in [
        format!("CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n", h.base, h.base),
        format!(
            "GET http://{}/runs HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {TOKEN}\r\n\r\n",
            h.base, h.base
        ),
        format!("CONNECT {proxy} HTTP/1.1\r\nHost: {proxy}\r\n\r\n"),
        "GET http://localhost:9230/runs HTTP/1.1\r\nHost: localhost:9230\r\n\r\n".to_owned(),
    ] {
        let mut socket = tokio::net::TcpStream::connect(proxy).await.unwrap();
        socket.write_all(head.as_bytes()).await.unwrap();
        let mut answer = [0u8; 12];
        socket.read_exact(&mut answer).await.unwrap();
        assert_eq!(&answer, b"HTTP/1.1 403", "{head}");
    }
}

#[tokio::test]
async fn starting_the_same_run_again_answers_the_run_that_exists() {
    let h = harness().await;
    let first = h.started("a").await;
    let before = h.runs().await;
    let second = h.started("a").await;
    assert_eq!(first, second);
    let after = h.runs().await;
    assert_eq!(after, before);
    assert_eq!(after.runs.len(), 1);
    assert!(after.runs[0].pid_alive);
}

#[tokio::test]
async fn runs_beyond_the_cap_are_refused() {
    let h = harness_with(|c| c.max_runs = 2).await;
    h.started("a").await;
    h.started("b").await;
    let (status, body) = h.start("c").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert!(!h.runs_dir.join("c").exists());
    assert!(!h.downloads_dir.join("c").exists());
    // A run that exists is still answered at the cap.
    assert_eq!(h.start("a").await.0, StatusCode::OK);
    // Each has a folder of its own.
    assert!(h.runs_dir.join("a").is_dir() && h.runs_dir.join("b").is_dir());
    assert_ne!(
        h.started("a").await.downloads,
        h.started("b").await.downloads
    );

    h.end("a").await;
    assert_eq!(h.start("c").await.0, StatusCode::OK);
}

#[tokio::test]
async fn invalid_run_ids_are_refused() {
    let h = harness().await;
    for bad in [
        "",
        "..",
        "../x",
        "a b",
        "a/b",
        "a.b",
        "x\n",
        &"y".repeat(81),
    ] {
        let (status, body) = h.start(bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad:?}: {body}");
    }
    let (status, _) = h.call(Method::DELETE, "/runs/a.b", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Nothing was made, in the runs folder or beside it.
    assert_eq!(std::fs::read_dir(&h.runs_dir).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(&h.downloads_dir).unwrap().count(), 0);
    assert!(!h.runs_dir.parent().unwrap().join("x").exists());
}

#[tokio::test]
async fn the_proxy_relays_frames_both_ways() {
    let h = harness().await;
    h.started("a").await;
    let mut socket = h.connect("a").await.unwrap();

    socket
        .send(Message::text(r#"{"id":1,"method":"Browser.getVersion"}"#))
        .await
        .unwrap();
    let reply = socket.next().await.unwrap().unwrap();
    assert_eq!(
        reply,
        Message::text(r#"{"id":1,"method":"Browser.getVersion"}"#)
    );

    socket
        .send(Message::Binary(vec![1, 2, 3].into()))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::Binary(vec![1, 2, 3].into())
    );

    // A message of a screencast's size.
    let big = "x".repeat(2 * 1024 * 1024);
    socket.send(Message::text(big.clone())).await.unwrap();
    assert_eq!(socket.next().await.unwrap().unwrap(), Message::text(big));
}

/// The worker drives a run and the web shows its page: two clients of one
/// run at once, each with its own connection to the browser, both behind the
/// token.
#[tokio::test]
async fn two_clients_of_one_run_each_get_their_own_answers() {
    let h = harness().await;
    h.started("a").await;
    let mut worker = h.connect("a").await.unwrap();
    let mut web = h.connect("a").await.unwrap();

    worker.send(Message::text("from the worker")).await.unwrap();
    web.send(Message::text("from the web")).await.unwrap();
    assert_eq!(
        web.next().await.unwrap().unwrap(),
        Message::text("from the web")
    );
    assert_eq!(
        worker.next().await.unwrap().unwrap(),
        Message::text("from the worker")
    );

    // The second client closing leaves the first one's connection alone.
    drop(web);
    worker.send(Message::text("still here")).await.unwrap();
    assert_eq!(
        worker.next().await.unwrap().unwrap(),
        Message::text("still here")
    );
    // And a second client still needs the token.
    let refused = connect_async(format!("ws://{}/runs/a/cdp", h.base)).await;
    assert!(
        matches!(refused, Err(tokio_tungstenite::tungstenite::Error::Http(ref r)) if r.status() == 401),
        "{refused:?}"
    );
}

#[tokio::test]
async fn an_unknown_or_ended_run_has_no_proxy() {
    let h = harness().await;
    let refused = h.connect("nobody").await;
    assert!(
        matches!(refused, Err(tokio_tungstenite::tungstenite::Error::Http(ref r)) if r.status() == 404),
        "{refused:?}"
    );
    h.started("a").await;
    h.end("a").await;
    let refused = h.connect("a").await;
    assert!(
        matches!(refused, Err(tokio_tungstenite::tungstenite::Error::Http(ref r)) if r.status() == 404),
        "{refused:?}"
    );
}

#[tokio::test]
async fn ending_a_run_closes_its_sockets_and_removes_its_profile() {
    let h = harness().await;
    h.started("a").await;
    h.started("b").await;
    let mut first = h.connect("a").await.unwrap();
    let mut second = h.connect("a").await.unwrap();
    let mut other = h.connect("b").await.unwrap();
    assert!(h.runs_dir.join("a/Default/Cookies").is_file());

    h.end("a").await;

    for socket in [&mut first, &mut second] {
        let closed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match socket.next().await {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                    Some(Ok(_)) => {}
                }
            }
        })
        .await;
        assert!(closed.is_ok(), "the socket stayed open");
    }
    assert!(!h.runs_dir.join("a").exists(), "the profile stays");
    // What the worker did not move out goes with the run.
    assert!(!h.downloads_dir.join("a").exists());
    assert!(h.downloads_dir.join("b").is_dir());
    assert_eq!(h.run_ids().await, ["b"]);

    // The other run is untouched.
    other.send(Message::text("still here")).await.unwrap();
    assert_eq!(
        other.next().await.unwrap().unwrap(),
        Message::text("still here")
    );

    // Ending twice, or a run that never was, is fine.
    h.end("a").await;
    h.end("nobody").await;
}

#[tokio::test]
async fn a_new_run_gets_a_fresh_profile() {
    let h = harness().await;
    h.started("a").await;
    std::fs::write(h.runs_dir.join("a/Default/Local Storage"), "site state").unwrap();
    h.end("a").await;
    h.started("a2").await;
    assert!(!h.runs_dir.join("a2/Default/Local Storage").exists());
    // The same id again starts clean too.
    h.started("a").await;
    assert!(!h.runs_dir.join("a/Default/Local Storage").exists());
}

#[tokio::test]
async fn a_reset_ends_every_run_and_removes_every_profile() {
    let h = harness().await;
    h.started("a").await;
    h.started("b").await;
    let mut socket = h.connect("a").await.unwrap();
    // Something no run owns.
    std::fs::create_dir_all(h.runs_dir.join("stray/Default")).unwrap();
    std::fs::write(h.runs_dir.join("stray-file"), "x").unwrap();

    let (status, _) = h.call(Method::POST, "/reset", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert!(h.run_ids().await.is_empty());
    assert_eq!(std::fs::read_dir(&h.runs_dir).unwrap().count(), 0);
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(message)) = socket.next().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
        }
    })
    .await;
    assert!(closed.is_ok());
    // And a run can start again.
    h.started("c").await;
}

#[tokio::test]
async fn a_launcher_start_removes_what_an_earlier_one_left() {
    let h = harness().await;
    std::fs::create_dir_all(h.runs_dir.join("old/Default")).unwrap();
    std::fs::write(h.runs_dir.join("old/Default/Cookies"), "x").unwrap();
    let config = h.launcher.config().clone();
    let again = Launcher::open(config).await.unwrap();
    assert_eq!(std::fs::read_dir(&h.runs_dir).unwrap().count(), 0);
    assert!(again.list().is_empty());
}

#[tokio::test]
async fn a_chromium_that_exits_by_itself_ends_its_run() {
    let h = harness_with(|c| c.chromium_args = vec!["--fake-exit-after-ms=600".into()]).await;
    h.started("a").await;
    let mut socket = h.connect("a").await.unwrap();

    eventually("the run to be gone", async || h.run_ids().await.is_empty()).await;
    assert!(!h.runs_dir.join("a").exists());
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(message)) = socket.next().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
        }
    })
    .await;
    assert!(closed.is_ok());
    // The slot is free again.
    assert_eq!(h.start("b").await.0, StatusCode::OK);
}

#[tokio::test]
async fn a_chromium_that_ignores_sigterm_is_killed_after_the_grace() {
    let h = harness_with(|c| {
        c.chromium_args = vec!["--fake-ignore-term".into()];
        c.kill_grace = Duration::from_millis(400);
    })
    .await;
    h.started("a").await;
    let pid = h.runs().await.runs[0].run.clone();
    assert_eq!(pid, "a");

    let began = Instant::now();
    h.end("a").await;
    let took = began.elapsed();
    assert!(
        took >= Duration::from_millis(350),
        "{took:?}: it was not given the grace"
    );
    assert!(took < Duration::from_secs(5), "{took:?}");
    assert!(h.run_ids().await.is_empty());
    assert!(!h.runs_dir.join("a").exists());
}

#[tokio::test]
async fn a_chromium_that_does_not_start_leaves_nothing_behind() {
    let exits = harness_with(|c| c.chromium_args = vec!["--fake-exit-at-start".into()]).await;
    let (status, _) = exits.start("a").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(exits.run_ids().await.is_empty());
    assert!(!exits.runs_dir.join("a").exists());
    assert!(!exits.downloads_dir.join("a").exists());

    let never = harness_with(|c| {
        c.chromium_args = vec!["--fake-never-ready".into()];
        c.ready_timeout = Duration::from_millis(500);
        c.kill_grace = Duration::from_millis(300);
    })
    .await;
    let (status, _) = never.start("a").await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert!(never.run_ids().await.is_empty());
    assert!(!never.runs_dir.join("a").exists());
}

/// The processes whose command line names the profile folder `profile`.
fn chromium_of(profile: &std::path::Path) -> Vec<u32> {
    let needle = format!("--user-data-dir={}", profile.display());
    let mut found = Vec::new();
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        // A zombie has no command line, so only a live process matches.
        if cmdline
            .split(|b| *b == 0)
            .any(|arg| arg == needle.as_bytes())
        {
            found.push(pid);
        }
    }
    found
}

/// A start request that is given up after `wait`, as the worker's is when its
/// own time is out.
async fn start_given_up_after(h: &Harness, id: &str, wait: Duration) {
    let client = Client::builder().timeout(wait).build().unwrap();
    let given_up = client
        .post(format!("http://{}/runs", h.base))
        .bearer_auth(TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "run": id }).to_string())
        .send()
        .await;
    assert!(given_up.is_err(), "the start was not slow enough");
}

#[tokio::test]
async fn an_end_of_a_run_whose_start_the_client_gave_up_cancels_that_start() {
    let h = harness_with(|c| c.chromium_args = vec!["--fake-ready-after-ms=2500".into()]).await;
    start_given_up_after(&h, "a", Duration::from_millis(300)).await;
    // The start goes on without the client.
    assert!(h.run_ids().await.is_empty());
    assert!(h.runs_dir.join("a").is_dir());
    assert_eq!(chromium_of(&h.runs_dir.join("a")).len(), 1);

    let began = Instant::now();
    h.end("a").await;
    assert!(
        began.elapsed() < Duration::from_secs(2),
        "the end waited for the start to finish"
    );
    assert!(h.run_ids().await.is_empty());
    assert!(!h.runs_dir.join("a").exists(), "the profile stays");
    assert!(
        !h.downloads_dir.join("a").exists(),
        "the downloads folder stays"
    );
    assert!(
        chromium_of(&h.runs_dir.join("a")).is_empty(),
        "a Chromium stays"
    );

    // It does not come back when the browser would have been ready.
    tokio::time::sleep(Duration::from_millis(2800)).await;
    assert!(h.run_ids().await.is_empty());
    assert!(!h.runs_dir.join("a").exists());
}

#[tokio::test]
async fn a_start_the_client_gave_up_on_still_makes_a_run_that_an_end_removes_whole() {
    let h = harness_with(|c| c.chromium_args = vec!["--fake-ready-after-ms=700".into()]).await;
    start_given_up_after(&h, "a", Duration::from_millis(200)).await;
    eventually("the run to be there", async || h.run_ids().await == ["a"]).await;
    std::fs::write(h.downloads_dir.join("a/leftover"), "x").unwrap();

    h.end("a").await;
    assert!(h.run_ids().await.is_empty());
    assert!(!h.runs_dir.join("a").exists());
    assert!(!h.downloads_dir.join("a").exists());
    assert!(chromium_of(&h.runs_dir.join("a")).is_empty());
}

#[tokio::test]
async fn an_end_is_not_answered_before_a_start_under_way_is_over() {
    // The start is slower than the end's request: the end must not answer
    // "done" while the run is yet to appear.
    let h = harness_with(|c| c.chromium_args = vec!["--fake-ready-after-ms=1200".into()]).await;
    let starting = tokio::spawn({
        let (client, url) = (h.client.clone(), format!("http://{}/runs", h.base));
        async move {
            client
                .post(url)
                .bearer_auth(TOKEN)
                .header("content-type", "application/json")
                .body(r#"{"run":"a"}"#)
                .send()
                .await
                .unwrap()
                .status()
        }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    h.end("a").await;
    assert!(h.run_ids().await.is_empty(), "a run appeared after the end");
    assert_eq!(starting.await.unwrap(), StatusCode::CONFLICT);
    assert!(!h.runs_dir.join("a").exists());
}

#[tokio::test]
async fn a_start_queued_behind_another_gives_up_when_it_is_ended() {
    let h = harness_with(|c| c.chromium_args = vec!["--fake-ready-after-ms=1500".into()]).await;
    let first = tokio::spawn({
        let (client, url) = (h.client.clone(), format!("http://{}/runs", h.base));
        async move {
            client
                .post(url)
                .bearer_auth(TOKEN)
                .header("content-type", "application/json")
                .body(r#"{"run":"first"}"#)
                .send()
                .await
                .unwrap()
                .status()
        }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let second = tokio::spawn({
        let (client, url) = (h.client.clone(), format!("http://{}/runs", h.base));
        async move {
            client
                .post(url)
                .bearer_auth(TOKEN)
                .header("content-type", "application/json")
                .body(r#"{"run":"second"}"#)
                .send()
                .await
                .unwrap()
                .status()
        }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let began = Instant::now();
    h.end("second").await;
    assert!(began.elapsed() < Duration::from_millis(900));
    assert_eq!(second.await.unwrap(), StatusCode::CONFLICT);
    assert_eq!(first.await.unwrap(), StatusCode::OK);
    assert_eq!(h.run_ids().await, ["first"]);
    assert!(!h.runs_dir.join("second").exists());
    assert!(!h.downloads_dir.join("second").exists());
}

#[tokio::test]
async fn two_requests_to_start_one_run_make_one_run() {
    let h = harness_with(|c| c.chromium_args = vec!["--fake-ready-after-ms=500".into()]).await;
    let (a, b) = tokio::join!(h.start("a"), h.start("a"));
    assert_eq!(a.0, StatusCode::OK, "{}", a.1);
    assert_eq!(b.0, StatusCode::OK, "{}", b.1);
    assert_eq!(a.1, b.1);
    assert_eq!(h.run_ids().await, ["a"]);
    assert_eq!(chromium_of(&h.runs_dir.join("a")).len(), 1);
}

#[tokio::test]
async fn a_reset_makes_the_starts_under_way_give_up() {
    let h = harness_with(|c| c.chromium_args = vec!["--fake-ready-after-ms=2500".into()]).await;
    start_given_up_after(&h, "a", Duration::from_millis(300)).await;
    let began = Instant::now();
    let (status, _) = h.call(Method::POST, "/reset", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(began.elapsed() < Duration::from_secs(2));
    assert!(h.run_ids().await.is_empty());
    assert!(!h.runs_dir.join("a").exists());
    assert!(!h.downloads_dir.join("a").exists());
    assert!(chromium_of(&h.runs_dir.join("a")).is_empty());
}

#[tokio::test]
async fn an_end_of_an_unknown_run_removes_the_folders_it_left() {
    let h = harness().await;
    std::fs::create_dir_all(h.downloads_dir.join("gone")).unwrap();
    std::fs::write(h.downloads_dir.join("gone/f"), "x").unwrap();
    std::fs::create_dir_all(h.runs_dir.join("gone/Default")).unwrap();
    h.end("gone").await;
    assert!(!h.downloads_dir.join("gone").exists());
    assert!(!h.runs_dir.join("gone").exists());
}

#[tokio::test]
async fn the_launcher_binary_does_not_start_without_a_token() {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_trss-browserd"))
        .env_remove("TRSS_BROWSER_TOKEN")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("TRSS_BROWSER_TOKEN"), "{stderr}");
}
