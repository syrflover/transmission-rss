//! The fake check post in the real browser image (ignored): the server
//! browser opens the post with no network, its driver clicks the download
//! card, and a click a second DevTools client gives at the check box (as the
//! web's remote screen does) makes the browser download the file.
//!
//! Needs Docker and the browser image (`TRSS_BROWSER_IMAGE`, default
//! `ghcr.io/syrflover/trss-browser:local`, built from `Dockerfile.browser`):
//!
//! ```sh
//! cargo test -p trss-subtitles --test auth_sample -- --ignored --nocapture
//! ```
//!
//! It starts a throwaway container named `trss-auth-sample-<pid>` (removed at
//! the end).

use std::{
    path::Path,
    process::{Command, Output},
    time::Duration,
};

use serde_json::json;
use trss_browser::{
    cdp::Connection, client::LauncherClient, BrowserPolicy, BrowserPool, PolicySource, PoolConfig,
};
use trss_subtitles::{
    auth::{AuthBrowser, BrowserAuth, PrepareRequest, Waited},
    fake::{self, FakeSource},
    verify, Opened, Sources,
};
use url::Url;

const TOKEN: &str = "sample-token";

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

async fn start(downloads: &Path) -> (Container, String) {
    let name = format!("trss-auth-sample-{}", std::process::id());
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
    let base = format!("http://{ip}:9230");
    let client = reqwest::Client::new();
    for _ in 0..80 {
        let up = client
            .get(format!("{base}/runs"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if up {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    (container, base)
}

#[tokio::test]
#[ignore = "needs docker and the trss-browser image"]
async fn a_persons_click_at_the_fake_check_makes_the_browser_download_the_file() {
    let downloads = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    let (_container, base) = start(downloads.path()).await;
    let pool = BrowserPool::new(
        PoolConfig::new(base.parse().unwrap(), TOKEN, downloads.path()),
        trss_core::system_clock(),
        PolicySource::fixed(BrowserPolicy::default()),
    )
    .await
    .unwrap();
    let auth = BrowserAuth::new(pool.clone());

    let post = Url::parse("https://fake.trss.invalid/check/sample").unwrap();
    let source = Sources::none()
        .with_fake(FakeSource)
        .for_post(&post)
        .unwrap();
    let Opened::BrowserAuth { page, .. } = source.open(&post, "1").await.unwrap() else {
        panic!("not a check post");
    };
    let prepared = auth
        .prepare(PrepareRequest {
            job: "job-1",
            post: &post,
            page: &page,
        })
        .await
        .unwrap();
    println!(
        "prepared: run {} target {}",
        prepared.run_id, prepared.target_id
    );
    assert!(auth.is_live("job-1", &prepared.run_id));
    // The blank page the run started with is closed (its end comes as an event).
    let mut pages = 0;
    for _ in 0..20 {
        pages = pool.run_of_job("job-1").unwrap().pages().len();
        if pages == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(pages, 1);

    // A second client, as the web is: its own connection to the run, its own
    // session on the page, a device's size, and a trusted click in the middle.
    let launcher = LauncherClient::new(base.parse().unwrap(), TOKEN).unwrap();
    let web = Connection::connect(&launcher.cdp_url(&prepared.run_id), TOKEN)
        .await
        .unwrap();
    let attached = web
        .command(
            None,
            "Target.attachToTarget",
            json!({ "targetId": prepared.target_id, "flatten": true }),
        )
        .await
        .unwrap();
    let session = attached["sessionId"].as_str().unwrap().to_owned();
    let (width, height) = (402, 666);
    web.command(
        Some(&session),
        "Emulation.setDeviceMetricsOverride",
        json!({ "width": width, "height": height, "deviceScaleFactor": 3, "mobile": true }),
    )
    .await
    .unwrap();
    web.command(
        Some(&session),
        "Emulation.setTouchEmulationEnabled",
        json!({ "enabled": true, "maxTouchPoints": 5 }),
    )
    .await
    .unwrap();
    // A page script's click is not a person's: nothing is downloaded.
    web.command(
        Some(&session),
        "Runtime.evaluate",
        json!({ "expression": "document.getElementById('check').click()" }),
    )
    .await
    .unwrap();
    // The page keeps the box in the middle after the size changed.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let rect = web
        .command(
            Some(&session),
            "Runtime.evaluate",
            json!({ "expression": "JSON.stringify(document.getElementById('check').getBoundingClientRect())", "returnByValue": true }),
        )
        .await
        .unwrap();
    println!("the box at {width}x{height}: {}", rect["result"]["value"]);
    let rect: serde_json::Value =
        serde_json::from_str(rect["result"]["value"].as_str().unwrap()).unwrap();
    let (x, y) = (
        rect["x"].as_f64().unwrap() + rect["width"].as_f64().unwrap() / 2.0,
        rect["y"].as_f64().unwrap() + rect["height"].as_f64().unwrap() / 2.0,
    );
    assert!(
        (0.0..width as f64).contains(&x) && (0.0..height as f64).contains(&y),
        "the box is off the first screen: {x},{y}"
    );
    for (kind, button, count) in [
        ("mouseMoved", "none", 0),
        ("mousePressed", "left", 1),
        ("mouseReleased", "left", 1),
    ] {
        web.command(
            Some(&session),
            "Input.dispatchMouseEvent",
            json!({ "type": kind, "x": x, "y": y, "button": button, "clickCount": count }),
        )
        .await
        .unwrap();
    }

    let waited = tokio::time::timeout(
        Duration::from_secs(30),
        auth.wait_file("job-1", &prepared.run_id, staging.path()),
    )
    .await
    .expect("no download in time");
    let Waited::File { name, path } = waited else {
        panic!("{waited:?}");
    };
    println!("downloaded {name}");
    assert_eq!(name, "sample.srt");
    assert_eq!(std::fs::read(&path).unwrap(), fake::srt("sample"));
    assert_eq!(verify::check(&path, &name).unwrap(), verify::Format::Srt);

    // Only one file came: the script's click before did not count.
    let none = tokio::time::timeout(
        Duration::from_secs(2),
        auth.wait_file("job-1", &prepared.run_id, staging.path()),
    )
    .await;
    assert!(none.is_err(), "another download came: {none:?}");

    auth.release("job-1").await;
    assert!(!auth.is_live("job-1", &prepared.run_id));
    assert!(
        web.is_closed() || {
            tokio::time::sleep(Duration::from_secs(2)).await;
            web.is_closed()
        }
    );
}
