//! The real image: Xvfb, the launcher and Debian's Chromium in a container.
//!
//! Needs Docker and the image (`TRSS_BROWSER_IMAGE`, default
//! `ghcr.io/syrflover/trss-browser:local`, built from `Dockerfile.browser`):
//!
//! ```sh
//! docker build -f Dockerfile.browser -t ghcr.io/syrflover/trss-browser:local .
//! cargo test -p trss-browser --test it docker:: -- --ignored --nocapture
//! ```
//!
//! It starts a throwaway container named `trss-browser-test-<pid>` (removed at
//! the end), publishes no port (the launcher is reached at the container's
//! address on the bridge network), and prints the memory it measured.
//!
//! The browser reaches only public addresses (the launcher's egress proxy).
//! The first test serves its site on the test host and lets the browser reach
//! exactly that address with `TRSS_BROWSER_EGRESS_ALLOW`; the second
//! (`the_browser_reaches_public_addresses_only`, which needs the internet)
//! shows that nothing else on the host, the LAN or the container is reached.
//! Run it against an image without the proxy with
//! `TRSS_BROWSER_EGRESS_CONTROL=1` to see the same probes get through there:
//! it then prints what was reached and asserts nothing.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    process::{Command, Output},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

/// The default bridge's gateway: the host as a container sees it, which
/// `host-gateway` stands for.
fn bridge_gateway() -> String {
    docker_ok(&[
        "network",
        "inspect",
        "bridge",
        "-f",
        "{{(index .IPAM.Config 0).Gateway}}",
    ])
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
    let served = trss_core::loopback::serve_on(Ipv4Addr::UNSPECIFIED, |listener| async move {
        axum::serve(listener, app).await.ok();
    })
    .await;
    (served.addr, site)
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
    // The host is not public: the egress proxy lets the browser reach this
    // one address of it, the site, and nothing else.
    let site_from_container = format!("{}:{}", bridge_gateway(), site_addr.port());

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
        &format!("TRSS_BROWSER_EGRESS_ALLOW={site_from_container}"),
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

/// The subnet of the egress test's own network, outside the private ranges
/// (a public block the test host takes for itself while the test runs).
const OWN_SUBNET: &str = "11.255.53.0/24";
const OWN_GATEWAY: &str = "11.255.53.1";

/// Removes a Docker network at the end, whatever happened. Declared before
/// the container that uses it, so it is dropped after it.
struct Network(String);

impl Drop for Network {
    fn drop(&mut self) {
        let _ = docker(&["network", "rm", &self.0]);
    }
}

/// The test host's LAN address: the source address of its route to the
/// internet (no packet is sent).
fn lan_address() -> IpAddr {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    socket.connect("1.1.1.1:80").unwrap();
    socket.local_addr().unwrap().ip()
}

/// A listener on every address of the test host that counts the TCP
/// connections it is given and answers each with `site reached` (readable
/// from any origin, so that a page could see it).
async fn counting_site() -> (u16, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let served = trss_core::loopback::serve_on(Ipv4Addr::UNSPECIFIED, |listener| async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                continue;
            };
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut request = [0u8; 4096];
                let _ =
                    tokio::time::timeout(Duration::from_secs(2), socket.read(&mut request)).await;
                let body = "site reached";
                let answer = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(answer.as_bytes()).await;
            });
        }
    })
    .await;
    (served.addr.port(), hits)
}

/// A UDP socket on every address of the test host that counts the datagrams
/// it is sent (WebRTC's and WebTransport's would land here).
async fn counting_udp() -> (u16, Arc<AtomicUsize>) {
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let datagrams = Arc::new(AtomicUsize::new(0));
    let counter = datagrams.clone();
    tokio::spawn(async move {
        let mut buffer = [0u8; 2048];
        while socket.recv_from(&mut buffer).await.is_ok() {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });
    (port, datagrams)
}

/// What the page's network said about `url` since `events` was taken: the
/// statuses it received and the errors it failed with.
fn network_outcome(
    events: &mut tokio::sync::broadcast::Receiver<trss_browser::cdp::Event>,
    url: &str,
) -> Vec<String> {
    let mut seen = Vec::new();
    let mut ids = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event.method.as_str() {
            "Network.requestWillBeSent" if event.params["request"]["url"] == url => {
                ids.push(event.params["requestId"].clone());
            }
            "Network.responseReceived" if event.params["response"]["url"] == url => {
                seen.push(format!(
                    "status {} from {}:{}",
                    event.params["response"]["status"],
                    event.params["response"]["remoteIPAddress"]
                        .as_str()
                        .unwrap_or("?"),
                    event.params["response"]["remotePort"]
                ));
            }
            "Network.loadingFailed" if ids.contains(&event.params["requestId"]) => {
                seen.push(format!(
                    "failed {} {} {}",
                    event.params["errorText"].as_str().unwrap_or(""),
                    event.params["blockedReason"].as_str().unwrap_or(""),
                    event.params["corsErrorStatus"]["corsError"]
                        .as_str()
                        .unwrap_or("")
                ));
            }
            _ => {}
        }
    }
    seen
}

/// Every way a page of the real image could reach the host, the LAN, the
/// Docker gateway, the launcher or a name that resolves to a private address
/// is refused; public sites load.
#[tokio::test]
#[ignore = "needs docker, the trss-browser image and the internet"]
async fn the_browser_reaches_public_addresses_only() {
    let control = std::env::var("TRSS_BROWSER_EGRESS_CONTROL").is_ok_and(|v| v == "1");
    let lan = lan_address();
    let gateway = bridge_gateway();
    let (port, hits) = counting_site().await;
    let (udp_port, datagrams) = counting_udp().await;
    let downloads = tempfile::tempdir().unwrap();
    // A real name in public DNS that resolves to the LAN address.
    let nip = format!("{}.nip.io", lan.to_string().replace('.', "-"));

    // More `docker run` arguments, to take one layer away and see the other
    // hold (`--tmpfs /etc/chromium/policies` hides the managed policy).
    let extra_args: Vec<String> = std::env::var("TRSS_BROWSER_EGRESS_DOCKER_ARGS")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();

    // The container is on a network of its own whose subnet is outside the
    // private ranges, as a Docker address pool can be: its gateway (this
    // host) has an address the classes call public, and only the proxy's
    // reading of the container's own networks refuses it.
    let network = format!("trss-browser-egress-net-{}", std::process::id());
    let _ = docker(&["network", "rm", &network]);
    docker_ok(&[
        "network",
        "create",
        "--subnet",
        OWN_SUBNET,
        "--gateway",
        OWN_GATEWAY,
        &network,
    ]);
    let _network = Network(network.clone());

    let name = format!("trss-browser-egress-{}", std::process::id());
    let _ = docker(&["rm", "-f", &name]);
    let container = Container { name: name.clone() };
    let add_lan = format!("--add-host=lan.trss.test:{lan}");
    let token = format!("TRSS_BROWSER_TOKEN={TOKEN}");
    let volume = format!("{}:/downloads", downloads.path().display());
    let image = image();
    let mut args = vec![
        "run",
        "-d",
        "--name",
        &name,
        "--network",
        &network,
        "--memory",
        "768m",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges:true",
        "--add-host=host.docker.internal:host-gateway",
        &add_lan,
        "-e",
        &token,
        // What the Debian wrapper would put before the launcher's flags.
        "-e",
        "CHROMIUM_FLAGS=--no-proxy-server",
        "-e",
        "CHROMIUM_USER_FLAGS=--no-proxy-server",
        "-v",
        &volume,
    ];
    args.extend(extra_args.iter().map(String::as_str));
    args.push(&image);
    docker_ok(&args);
    let ip = docker_ok(&[
        "inspect",
        "-f",
        "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
        &name,
    ]);
    let base = format!("http://{ip}:9230");
    wait_for_launcher(&base).await;
    let pool = BrowserPool::new(
        PoolConfig::new(base.parse().unwrap(), TOKEN, downloads.path()),
        trss_core::system_clock(),
        PolicySource::fixed(BrowserPolicy::default()),
    )
    .await
    .unwrap();
    let run = pool.start("egress").await.unwrap();
    let page = run.new_page("about:blank").await.unwrap();
    let text = |page: &trss_browser::Page| {
        let page = page.clone();
        async move {
            page.evaluate("document.body ? document.body.innerText : ''")
                .await
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()
        }
    };

    // What must not be reached: (label, address as a page writes it).
    let mut private: Vec<(String, String)> = vec![
        ("the host's LAN address".into(), format!("{lan}:{port}")),
        (
            "the default bridge's gateway".into(),
            format!("{gateway}:{port}"),
        ),
        (
            "the gateway of the container's own network (outside the private ranges)".into(),
            format!("{OWN_GATEWAY}:{port}"),
        ),
        (
            "the cloud metadata address".into(),
            "169.254.169.254:80".into(),
        ),
        (
            "a CGNAT (Tailscale) address".into(),
            "100.100.100.100:80".into(),
        ),
        (
            "host.docker.internal".into(),
            format!("host.docker.internal:{port}"),
        ),
        (
            "a name of /etc/hosts for the LAN address".into(),
            format!("lan.trss.test:{port}"),
        ),
        (
            "a public DNS name for the LAN address".into(),
            format!("{nip}:{port}"),
        ),
    ];
    let launcher: Vec<(String, String)> = vec![
        ("the launcher on loopback".into(), "127.0.0.1:9230".into()),
        ("the launcher as localhost".into(), "localhost:9230".into()),
        ("the launcher on [::1]".into(), "[::1]:9230".into()),
        (
            "the launcher on the container's address".into(),
            format!("{ip}:9230"),
        ),
    ];
    private.extend(launcher.iter().cloned());
    let mut reached: Vec<String> = Vec::new();
    let mut note_reach = |what: String, got_through: bool| {
        println!("{} {what}", if got_through { "REACHED" } else { "refused" });
        if got_through {
            reached.push(what);
        }
    };

    // --- top-level navigations ---
    for (label, address) in &private {
        for scheme in ["http", "https"] {
            let before = hits.load(Ordering::SeqCst);
            let navigated = page.navigate(&format!("{scheme}://{address}/runs")).await;
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let shown = text(&page).await;
            let outcome = match &navigated {
                Err(err) => format!("navigation error {err}"),
                Ok(()) => format!("page {:?}", shown.chars().take(60).collect::<String>()),
            };
            let through = hits.load(Ordering::SeqCst) > before
                || shown.contains("site reached")
                || shown.contains("unauthorized");
            note_reach(format!("navigate {scheme} {label}: {outcome}"), through);
        }
    }

    // --- fetch and WebSocket from a public page (plain HTTP, so that mixed
    //     content rules do not stop them first; neverssl.com has no HTTPS
    //     for Chromium to upgrade to) ---
    page.navigate("http://neverssl.com/").await.unwrap();
    tokio::time::sleep(Duration::from_secs(4)).await;
    let public_page = page
        .evaluate("[location.protocol, location.hostname, document.title]")
        .await
        .unwrap();
    println!("public page over HTTP: {public_page}");
    if !control {
        assert_eq!(public_page[0], "http:");
        assert!(public_page[1].as_str().unwrap().ends_with("neverssl.com"));
    }
    let mut events = run.events().unwrap();
    for (label, address) in &private {
        for scheme in ["http", "https"] {
            let url = format!("{scheme}://{address}/runs");
            let before = hits.load(Ordering::SeqCst);
            let answer = page
                .evaluate(&format!(
                    "fetch({url:?}).then(r => 'status ' + r.status, e => 'error ' + e.message)"
                ))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
            let network = network_outcome(&mut events, &url);
            let through = hits.load(Ordering::SeqCst) > before
                || answer.as_str().is_some_and(|a| a.starts_with("status 200"))
                || network
                    .iter()
                    .any(|n| n.starts_with("status 200") || n.starts_with("status 401"));
            note_reach(
                format!("fetch {scheme} {label}: {answer} {network:?}"),
                through,
            );
        }
        for scheme in ["ws", "wss"] {
            let before = hits.load(Ordering::SeqCst);
            let answer = page
                .evaluate(&format!(
                    "new Promise(r => {{ try {{ const w = new WebSocket({:?}); w.onopen = () => r('open'); w.onerror = () => r('error'); setTimeout(() => r('timeout'), 5000); }} catch (e) {{ r('threw ' + e.message); }} }})",
                    format!("{scheme}://{address}/runs/x/cdp")
                ))
                .await
                .unwrap();
            let through = hits.load(Ordering::SeqCst) > before || answer == "open";
            note_reach(format!("WebSocket {scheme} {label}: {answer}"), through);
        }
    }

    // --- UDP: WebRTC and WebTransport (both need a secure page) ---
    page.navigate("https://example.com/").await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let candidates = page
        .evaluate(&format!(
            "new Promise(async r => {{ const pc = new RTCPeerConnection({{ iceServers: [{{ urls: 'stun:{lan}:{udp_port}' }}, {{ urls: 'turn:{lan}:{udp_port}?transport=udp', username: 'u', credential: 'p' }}] }}); const found = []; pc.onicecandidate = e => {{ if (e.candidate) found.push(e.candidate.candidate) }}; pc.createDataChannel('x'); await pc.setLocalDescription(await pc.createOffer()); setTimeout(() => r(found), 5000); }})"
        ))
        .await
        .unwrap();
    let udp_candidates = candidates
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| {
            c.as_str()
                .is_some_and(|c| c.to_lowercase().contains(" udp "))
        })
        .count();
    note_reach(
        format!(
            "WebRTC: {udp_candidates} UDP candidates of {}",
            candidates.as_array().unwrap().len()
        ),
        udp_candidates > 0,
    );
    let transport = page
        .evaluate(&format!(
            "(async () => {{ try {{ const t = new WebTransport('https://{lan}:{udp_port}/'); const done = await Promise.race([t.ready.then(() => 'ready'), new Promise(r => setTimeout(() => r('timeout'), 5000))]); return done; }} catch (e) {{ return 'error ' + e.message; }} }})()"
        ))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let udp = datagrams.load(Ordering::SeqCst);
    note_reach(
        format!("WebTransport: {transport}; datagrams that arrived: {udp}"),
        udp > 0,
    );

    // --- public sites load ---
    let public_fetch = page
        .evaluate("fetch('https://example.com/').then(r => 'status ' + r.status, e => 'error ' + e.message)")
        .await
        .unwrap();
    println!("public fetch over HTTPS: {public_fetch}");
    let public_socket = page
        .evaluate("new Promise(r => { const w = new WebSocket('wss://ws.postman-echo.com/raw'); w.onopen = () => r('open'); w.onerror = () => r('error'); setTimeout(() => r('timeout'), 8000); })")
        .await
        .unwrap();
    println!("public WebSocket (wss://ws.postman-echo.com/raw): {public_socket}");
    page.navigate("https://harne1.tistory.com/763")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(6)).await;
    let tistory = page
        .evaluate("[location.hostname, document.title, document.images.length]")
        .await
        .unwrap();
    println!("Tistory post: {tistory}");
    // The image's managed policy is in force (its page is made of shadow
    // roots, so its text is gathered through them).
    page.navigate("chrome://policy").await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let policies = page
        .evaluate(
            "(function walk(n) { let t = ''; if (n.shadowRoot) t += walk(n.shadowRoot); for (const c of n.childNodes) { t += c.nodeType === 3 ? c.textContent + ' ' : walk(c); } return t; })(document.body).replace(/\\s+/g, ' ')",
        )
        .await
        .unwrap()
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let around = |name: &str| {
        policies
            .find(name)
            .map(|at| policies[at..].chars().take(70).collect::<String>())
            .unwrap_or_default()
    };
    println!(
        "chrome://policy: {:?} {:?} {:?}",
        around("EnableMediaRouter"),
        around("QuicAllowed"),
        around("WebRtcIPHandling")
    );
    let policy_in_force = policies.contains("disable_non_proxied_udp");

    run.end().await;
    let logs = String::from_utf8_lossy(&docker(&["logs", &name]).stdout).into_owned()
        + &String::from_utf8_lossy(&docker(&["logs", &name]).stderr);
    println!("launcher log:\n{logs}");
    pool.shutdown().await;
    drop(container);

    println!(
        "site connections: {}, datagrams: {}",
        hits.load(Ordering::SeqCst),
        datagrams.load(Ordering::SeqCst)
    );
    if control {
        println!("control run: {} probes got through", reached.len());
        return;
    }
    assert!(reached.is_empty(), "reached: {reached:#?}");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert_eq!(datagrams.load(Ordering::SeqCst), 0);
    assert_eq!(public_fetch, "status 200");
    assert!(
        policy_in_force || !extra_args.is_empty(),
        "the managed policy is not in force: {policies}"
    );
    assert_eq!(tistory[0], "harne1.tistory.com");
    assert!(
        tistory[1].as_str().is_some_and(|t| !t.is_empty()),
        "{tistory}"
    );
    // Neither a refused nor an allowed destination is in the log.
    for secret in [
        lan.to_string(),
        gateway.clone(),
        "lan.trss.test".to_owned(),
        OWN_GATEWAY.to_owned(),
        "169.254.169.254".to_owned(),
        "100.100.100.100".to_owned(),
        nip.clone(),
        "example.com".to_owned(),
        "tistory".to_owned(),
        format!(":{port}"),
    ] {
        assert!(!logs.contains(&secret), "the log names {secret}");
    }
    assert!(!logs.contains("panicked"), "{logs}");
}
