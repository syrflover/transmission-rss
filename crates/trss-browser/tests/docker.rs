//! The real image: Xvfb, the launcher and Debian's Chromium in a container.
//!
//! Needs Docker and the image (`TRSS_BROWSER_IMAGE`, default
//! `ghcr.io/syrflover/trss-browser:local`, built from `Dockerfile.browser`):
//!
//! ```sh
//! docker build -f Dockerfile.browser -t ghcr.io/syrflover/trss-browser:local .
//! cargo test -p trss-browser --test docker -- --ignored --nocapture
//! ```
//!
//! It starts a throwaway container named `trss-browser-test-<pid>` (removed at
//! the end), publishes no port (the launcher is reached at the container's
//! address on the bridge network), and prints the memory it measured.

use std::{
    net::SocketAddr,
    path::Path,
    process::{Command, Output},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use serde_json::json;
use trss_browser::{BrowserPolicy, BrowserPool, DownloadState, PolicySource, PoolConfig};

const TOKEN: &str = "test-token";
const MEMORY_LIMIT: u64 = 768 * 1024 * 1024;

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
struct Container {
    name: String,
}

impl Drop for Container {
    fn drop(&mut self) {
        let _ = docker(&["rm", "-f", &self.name]);
    }
}

impl Container {
    fn exec(&self, script: &str) -> String {
        docker_ok(&["exec", &self.name, "sh", "-c", script])
    }
}

#[derive(Debug)]
struct Memory {
    /// The container's cgroup, as the memory limit counts it.
    current_mib: f64,
    anon_mib: f64,
    file_mib: f64,
    /// (name, VmRSS in KiB) of each process.
    processes: Vec<(String, u64)>,
}

impl Memory {
    fn rss_of(&self, name: &str) -> u64 {
        self.processes
            .iter()
            .filter(|(n, _)| n.starts_with(name))
            .map(|(_, r)| r)
            .sum()
    }
}

fn measure(container: &Container) -> Memory {
    let out = container.exec(
        "echo CUR $(cat /sys/fs/cgroup/memory.current); \
         grep -E '^(anon|file) ' /sys/fs/cgroup/memory.stat | sed 's/^/STAT /'; \
         for d in /proc/[0-9]*; do awk '/^Name:/{n=$2} /^VmRSS:/{r=$2} END{if (n != \"\") print \"PROC\", n, r+0}' $d/status 2>/dev/null; done",
    );
    let mib = |bytes: &str| bytes.parse::<f64>().unwrap() / 1024.0 / 1024.0;
    let (mut current, mut anon, mut file, mut processes) = (0.0, 0.0, 0.0, Vec::new());
    for line in out.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["CUR", bytes] => current = mib(bytes),
            ["STAT", "anon", bytes] => anon = mib(bytes),
            ["STAT", "file", bytes] => file = mib(bytes),
            ["PROC", name, rss] => processes.push(((*name).to_owned(), rss.parse().unwrap())),
            _ => {}
        }
    }
    Memory {
        current_mib: current,
        anon_mib: anon,
        file_mib: file,
        processes,
    }
}

fn report(label: &str, memory: &Memory) {
    let total_kib: u64 = memory.processes.iter().map(|(_, r)| r).sum();
    println!(
        "MEASURED {label}: cgroup memory.current {:.1} MiB (anon {:.1} MiB, file cache {:.1} MiB); {} processes, RSS sum {:.1} MiB (shared pages counted per process); Xvfb {:.1} MiB, launcher {:.1} MiB, chromium {:.1} MiB",
        memory.current_mib,
        memory.anon_mib,
        memory.file_mib,
        memory.processes.len(),
        total_kib as f64 / 1024.0,
        memory.rss_of("Xvfb") as f64 / 1024.0,
        memory.rss_of("trss-browserd") as f64 / 1024.0,
        memory.rss_of("chrom") as f64 / 1024.0,
    );
}

/// A site the container can reach: it sets a cookie, offers a file to
/// download, and remembers the cookies each request came with.
#[derive(Clone, Default)]
struct Site {
    cookies_seen: Arc<Mutex<Vec<(String, String)>>>,
}

async fn spawn_site() -> (SocketAddr, Site) {
    let site = Site::default();
    let app = Router::new()
        .route("/", get(index))
        .route("/blank", get(blank))
        .route("/download-page", get(download_page))
        .route("/file.zip", get(file))
        .with_state(site.clone());
    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, site)
}

fn note(site: &Site, path: &str, headers: &HeaderMap) {
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    site.cookies_seen
        .lock()
        .unwrap()
        .push((path.to_owned(), cookie));
}

async fn index(State(site): State<Site>, headers: HeaderMap) -> impl IntoResponse {
    note(&site, "/", &headers);
    (
        [(
            header::SET_COOKIE,
            "trss_test=remembered; Path=/; Max-Age=3600",
        )],
        Html("<title>cookie</title><p>cookie set</p>"),
    )
}

async fn blank(State(site): State<Site>, headers: HeaderMap) -> Html<&'static str> {
    note(&site, "/blank", &headers);
    Html("<title>blank</title><p>nothing</p>")
}

async fn download_page(State(site): State<Site>, headers: HeaderMap) -> Html<&'static str> {
    note(&site, "/download-page", &headers);
    Html(
        r#"<title>download</title><a id="dl" href="/file.zip" style="display:block;width:300px;height:80px;background:#ccc">download the file</a>"#,
    )
}

async fn file() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/zip"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"sample.zip\"; filename*=UTF-8''%EC%9E%90%EB%A7%89.zip",
            ),
        ],
        FILE_BODY.to_vec(),
    )
}

const FILE_BODY: &[u8] = b"PK\x03\x04 not really a zip, but bytes the browser saved";

async fn wait_for_launcher(base: &str) {
    let client = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let up = client
            .get(format!("{base}/runs"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if up {
            return;
        }
        assert!(Instant::now() < deadline, "the launcher did not come up");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn cookie_of(page: &trss_browser::Page) -> String {
    page.evaluate("document.cookie")
        .await
        .unwrap()
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
#[ignore = "needs docker and the trss-browser image"]
async fn the_real_image_runs_a_browser_per_job_and_forgets_it() {
    let downloads = tempfile::tempdir().unwrap();
    let receive = tempfile::tempdir().unwrap();
    let (site_addr, site) = spawn_site().await;
    let site_url = format!("http://host.docker.internal:{}", site_addr.port());

    // --- the container, as the compose file runs it (no port published) ---
    let name = format!("trss-browser-test-{}", std::process::id());
    let _ = docker(&["rm", "-f", &name]);
    let container = Container { name: name.clone() };
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
        "--add-host=host.docker.internal:host-gateway",
        "-e",
        &format!("TRSS_BROWSER_TOKEN={TOKEN}"),
        "-e",
        "TRSS_BROWSER_MAX_RUNS=3",
        "-v",
        &format!("{}:/downloads", downloads.path().display()),
        &image(),
    ]);
    let ip = docker_ok(&[
        "inspect",
        "-f",
        "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
        &name,
    ]);
    assert!(!ip.is_empty(), "the container has no bridge address");
    let base = format!("http://{ip}:9230");
    wait_for_launcher(&base).await;

    // --- nothing is published, and nothing but the launcher listens outside ---
    assert_eq!(docker_ok(&["port", &name]), "", "a port is published");
    assert_eq!(
        docker_ok(&["inspect", "-f", "{{json .HostConfig.PortBindings}}", &name]),
        "{}"
    );
    for port in [9230u16, 9222] {
        let reached = tokio::time::timeout(
            Duration::from_secs(2),
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await;
        assert!(
            !matches!(reached, Ok(Ok(_))),
            "127.0.0.1:{port} on the host reaches something"
        );
    }
    let unauthenticated = reqwest::get(format!("{base}/runs")).await.unwrap();
    assert_eq!(unauthenticated.status(), 401);

    // --- the memory limit applies ---
    assert_eq!(
        docker_ok(&["inspect", "-f", "{{.HostConfig.Memory}}", &name]),
        MEMORY_LIMIT.to_string()
    );
    assert_eq!(
        container.exec("cat /sys/fs/cgroup/memory.max"),
        MEMORY_LIMIT.to_string()
    );

    // --- idle: Xvfb and the launcher ---
    tokio::time::sleep(Duration::from_secs(5)).await;
    let idle = measure(&container);
    report("idle (Xvfb + launcher)", &idle);
    assert!(idle.rss_of("Xvfb") > 0 && idle.rss_of("trss-browserd") > 0);
    assert_eq!(idle.rss_of("chrom"), 0, "a Chromium is running with no run");
    assert!(
        idle.current_mib < 120.0,
        "idle container uses {:.1} MiB",
        idle.current_mib
    );

    // --- the worker's side ---
    let pool = BrowserPool::new(
        PoolConfig::new(base.parse().unwrap(), TOKEN, downloads.path()),
        trss_core::system_clock(),
        PolicySource::fixed(BrowserPolicy::default()),
    )
    .await
    .unwrap();

    // One run, on about:blank.
    let run = pool.start("job-1").await.unwrap();
    tokio::time::sleep(Duration::from_secs(5)).await;
    let blank = measure(&container);
    report("one run on about:blank", &blank);
    assert!(blank.rss_of("chrom") > 0, "no Chromium with a run");
    assert!(blank.current_mib < MEMORY_LIMIT as f64 / 1024.0 / 1024.0);
    // The profile is where the launcher says, and it is the run's alone.
    let runs_dir = container.exec("ls /tmp/trss-runs");
    assert_eq!(runs_dir.trim(), run.run_id());
    // The browser's DevTools port listens on loopback only; the launcher on all.
    let listening =
        container.exec("cat /proc/net/tcp /proc/net/tcp6 | awk '$4 == \"0A\" {print $2}'");
    for local in listening.lines() {
        let (address, port) = local.split_once(':').unwrap();
        let port = u16::from_str_radix(port, 16).unwrap();
        let loopback = address == "0100007F" || address == "00000000000000000000000001000000";
        assert!(
            loopback || port == 9230,
            "something listens beyond loopback on port {port} ({address})"
        );
    }

    // A page, a cookie in the profile, the ad blocklist.
    let page = run.new_page(&format!("{site_url}/")).await.unwrap();
    for _ in 0..50 {
        if cookie_of(&page).await.contains("trss_test=remembered") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(cookie_of(&page).await.contains("trss_test=remembered"));
    page.navigate(&format!("{site_url}/blank")).await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        cookie_of(&page).await.contains("trss_test=remembered"),
        "the cookie did not last within the run"
    );

    let mut events = run.events().unwrap();
    page.evaluate(
        "fetch('https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js').catch(() => 0); 1",
    )
    .await
    .unwrap();
    let blocked = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.method == "Network.loadingFailed"
                && event.params["blockedReason"] == "inspector"
            {
                return event;
            }
        }
    })
    .await;
    assert!(blocked.is_ok(), "an ad address was not blocked");

    // A click on a download link: the file arrives and is moved.
    let _busy = run.busy_guard().unwrap();
    let page = run
        .new_page(&format!("{site_url}/download-page"))
        .await
        .unwrap();
    let center = page
        .evaluate(
            "(() => { const r = document.getElementById('dl').getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2]; })()",
        )
        .await
        .unwrap();
    let (x, y) = (center[0].as_f64().unwrap(), center[1].as_f64().unwrap());
    for (kind, extra) in [
        ("mouseMoved", json!({})),
        ("mousePressed", json!({"clickCount": 1})),
        ("mouseReleased", json!({"clickCount": 1})),
    ] {
        let mut params = json!({"type": kind, "x": x, "y": y, "button": if kind == "mouseMoved" { "none" } else { "left" }});
        params
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        page.send("Input.dispatchMouseEvent", params).await.unwrap();
    }
    let download = tokio::time::timeout(Duration::from_secs(30), run.download_finished())
        .await
        .expect("no download finished")
        .unwrap();
    assert_eq!(download.state, DownloadState::Completed);
    assert_eq!(download.host.as_deref(), Some("host.docker.internal"));
    let moved = run.move_download(&download, receive.path()).await.unwrap();
    println!(
        "MEASURED download: {} ({} bytes) arrived and was moved",
        moved.name, moved.size
    );
    assert_eq!(moved.name, "자막.zip");
    assert_eq!(moved.size, FILE_BODY.len() as u64);
    assert_eq!(std::fs::read(&moved.path).unwrap(), FILE_BODY);
    assert!(
        std::fs::read_dir(run.downloads_dir())
            .unwrap()
            .next()
            .is_none(),
        "the downloads folder still holds the file"
    );
    drop(_busy);

    let with_page = measure(&container);
    report("one run after a page, a block and a download", &with_page);

    // --- end the run: Chromium and its profile go ---
    let first_run_id = run.run_id().to_owned();
    run.end().await;
    assert!(run.is_ended());
    assert!(matches!(
        run.command("Browser.getVersion", json!({})).await,
        Err(trss_browser::BrowserError::RunEnded { .. })
    ));
    assert_eq!(
        container.exec("ls /tmp/trss-runs").trim(),
        "",
        "the profile stays"
    );
    assert!(!downloads.path().join(&first_run_id).exists());
    tokio::time::sleep(Duration::from_secs(2)).await;
    let after = measure(&container);
    report("after the run ended", &after);
    assert_eq!(after.rss_of("chrom"), 0, "Chromium outlived its run");

    // --- a new run does not remember the old one ---
    let before = site.cookies_seen.lock().unwrap().len();
    let second = pool.start("job-1").await.unwrap();
    assert_ne!(second.run_id(), first_run_id);
    let page = second.new_page(&format!("{site_url}/blank")).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        cookie_of(&page).await,
        "",
        "a cookie of the earlier run came back"
    );
    let seen = site.cookies_seen.lock().unwrap()[before..].to_vec();
    assert!(
        seen.iter().any(|(path, _)| path == "/blank")
            && seen.iter().all(|(_, cookie)| cookie.is_empty()),
        "the site saw a cookie from the new run: {seen:?}"
    );
    let second_profile = container.exec("ls /tmp/trss-runs");
    assert_eq!(second_profile.trim(), second.run_id());
    second.end().await;

    // --- a worker that restarts finds no run of the one before ---
    let orphan = pool.start("job-2").await.unwrap();
    assert_eq!(container.exec("ls /tmp/trss-runs").trim(), orphan.run_id());
    let restarted = BrowserPool::new(
        PoolConfig::new(base.parse().unwrap(), TOKEN, downloads.path()),
        trss_core::system_clock(),
        PolicySource::fixed(BrowserPolicy::default()),
    )
    .await
    .unwrap();
    assert_eq!(
        container.exec("ls /tmp/trss-runs").trim(),
        "",
        "a run outlived the reset"
    );
    assert_eq!(measure(&container).rss_of("chrom"), 0);
    for _ in 0..50 {
        if orphan.is_ended() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        orphan.is_ended(),
        "the old worker's handle still works after the reset"
    );
    assert!(restarted.status().is_empty());

    // --- the launcher is still the same healthy process; nothing was killed ---
    assert_eq!(
        docker_ok(&["inspect", "-f", "{{.State.OOMKilled}}", &name]),
        "false"
    );
    assert_eq!(
        docker_ok(&["inspect", "-f", "{{.State.Running}}", &name]),
        "true"
    );
    let logs = String::from_utf8_lossy(&docker(&["logs", &name]).stdout).into_owned()
        + &String::from_utf8_lossy(&docker(&["logs", &name]).stderr);
    assert!(!logs.contains("panicked"), "{logs}");
    assert!(!logs.contains("SECRET"));
    pool.shutdown().await;
    assert_eq!(Path::new(&downloads.path()).read_dir().unwrap().count(), 0);
}
