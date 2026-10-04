//! A find job's page in the real browser image (ignored): the server browser
//! opens a fake blog's newest post with no network and clicks nothing
//! ([`AuthPage::Browse`]); a second DevTools client, as the web's remote
//! screen is, goes to the post before it and clicks its two attachments with
//! trusted clicks, and the run hands out both downloads. A click that opens a
//! new window makes a page of the run, which a person's request closes
//! ([`AuthBrowser::close_page`]); the page the run opened the post in is
//! never closed so. Such a window becomes a page within 2 seconds, and one
//! whose first request is a blocklisted address shows the browser's blocked
//! error page (ticket 0054).
//!
//! Needs Docker and the browser image (`TRSS_BROWSER_IMAGE`, default
//! `ghcr.io/syrflover/trss-browser:local`, built from `Dockerfile.browser`):
//!
//! ```sh
//! cargo test -p trss-subtitles --test find_sample -- --ignored --nocapture
//! ```
//!
//! It starts a throwaway container named `trss-find-sample-<pid>` (removed at
//! the end).

use std::{
    path::Path,
    process::{Command, Output},
    time::Duration,
};

use serde_json::{json, Value};
use trss_browser::{
    cdp::Connection, client::LauncherClient, BrowserPolicy, BrowserPool, PolicySource, PoolConfig,
};
use trss_subtitles::{
    auth::{AuthBrowser, AuthPage, BrowserAuth, PrepareRequest, Prepared, Waited},
    fake,
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
    let name = format!("trss-find-sample-{}", std::process::id());
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

/// The web's remote screen: its own connection and session on the page.
struct Screen {
    conn: Connection,
    session: String,
}

impl Screen {
    async fn send(&self, method: &str, params: Value) -> Value {
        self.conn
            .command(Some(&self.session), method, params)
            .await
            .unwrap()
    }

    async fn eval(&self, expression: &str) -> Value {
        let out = self
            .send(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await;
        out["result"]["value"].clone()
    }

    async fn title_is(&self, title: &str) {
        for _ in 0..40 {
            if self.eval("document.title").await == title {
                return;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        panic!(
            "the page is not {title}: {}",
            self.eval("document.title").await
        );
    }

    /// A person's click (a trusted one) in the middle of the element `id`.
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
}

/// A person's click on the link `id` opens a window: it is a page of the run
/// within 2 seconds. Returns its target.
async fn open_popup(screen: &Screen, auth: &BrowserAuth, prepared: &Prepared, id: &str) -> String {
    let clicked = std::time::Instant::now();
    screen.click(id).await;
    loop {
        let popup = auth
            .pages("job-1", &prepared.run_id)
            .into_iter()
            .find(|p| *p != prepared.target_id);
        if let Some(popup) = popup {
            println!(
                "the window of {id} became a page in {:?}",
                clicked.elapsed()
            );
            return popup;
        }
        assert!(
            clicked.elapsed() < Duration::from_secs(2),
            "no new window within 2 s of the click on {id}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A person's request closes the popup, and the post's own page is left.
async fn close_popup(auth: &BrowserAuth, prepared: &Prepared, popup: &str) {
    assert!(auth.close_page("job-1", &prepared.run_id, popup).await);
    for _ in 0..40 {
        if auth.pages("job-1", &prepared.run_id) == vec![prepared.target_id.clone()] {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!(
        "the popup is still a page: {:?}",
        auth.pages("job-1", &prepared.run_id)
    );
}

#[tokio::test]
#[ignore = "needs docker and the trss-browser image"]
async fn a_person_goes_to_a_past_post_and_its_attachments_are_the_runs_downloads() {
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

    let post = Url::parse(&format!("https://{}/blog/sample", fake::HOST)).unwrap();
    let prepared = auth
        .prepare(PrepareRequest {
            job: "job-1",
            post: &post,
            page: &AuthPage::Browse,
        })
        .await
        .unwrap();
    assert!(auth.is_live("job-1", &prepared.run_id));
    assert!(auth
        .pages("job-1", &prepared.run_id)
        .contains(&prepared.target_id));
    assert!(!auth.downloading("job-1", &prepared.run_id));

    let launcher = LauncherClient::new(base.parse().unwrap(), TOKEN).unwrap();
    let conn = Connection::connect(&launcher.cdp_url(&prepared.run_id), TOKEN)
        .await
        .unwrap();
    let attached = conn
        .command(
            None,
            "Target.attachToTarget",
            json!({ "targetId": prepared.target_id, "flatten": true }),
        )
        .await
        .unwrap();
    let screen = Screen {
        conn,
        session: attached["sessionId"].as_str().unwrap().to_owned(),
    };
    // The newest post, and nothing clicked by the browser.
    let newest = fake::BLOG_POSTS;
    screen
        .title_is(&format!("가짜 블로그 sample {newest}화"))
        .await;
    let none = tokio::time::timeout(
        Duration::from_secs(1),
        auth.wait_file("job-1", &prepared.run_id, staging.path()),
    )
    .await;
    assert!(none.is_err(), "a download came by itself: {none:?}");

    // A person goes to the post before it and clicks its attachments.
    screen.click("before").await;
    let past = newest - 1;
    screen
        .title_is(&format!("가짜 블로그 sample {past}화"))
        .await;
    let mut names = Vec::new();
    for id in ["srt", "txt"] {
        screen.click(id).await;
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
        assert!(path.exists());
        names.push(name);
    }
    assert_eq!(
        names,
        [format!("sample-{past}.srt"), format!("sample-{past}.txt")]
    );
    assert_eq!(
        std::fs::read(staging.path().join(&names[0])).unwrap(),
        fake::srt(&format!("sample-{past}"))
    );
    assert!(!auth.downloading("job-1", &prepared.run_id));

    // A frame of another site is a target of its own, set up the same way:
    // it asks for a blocklisted address and is refused, while an address that
    // is not on the list goes through.
    screen
        .eval(
            "document.body.append(Object.assign(document.createElement('iframe'), \
             { id: 'cross', src: 'https://example.com/' })); 0",
        )
        .await;
    let mut frame = None;
    for _ in 0..80 {
        let targets = screen
            .conn
            .command(None, "Target.getTargets", json!({}))
            .await
            .unwrap();
        frame = targets["targetInfos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| {
                t["type"] == "iframe"
                    && t["url"].as_str().is_some_and(|u| u.contains("example.com"))
            })
            .and_then(|t| t["targetId"].as_str().map(str::to_owned));
        if frame.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let frame = frame.expect("the frame of another site is not a target");
    let attached = screen
        .conn
        .command(
            None,
            "Target.attachToTarget",
            json!({ "targetId": frame, "flatten": true }),
        )
        .await
        .unwrap();
    let frame_view = Screen {
        conn: screen.conn.clone(),
        session: attached["sessionId"].as_str().unwrap().to_owned(),
    };
    let asked = frame_view
        .send(
            "Runtime.evaluate",
            json!({
                "expression": "const ask = url => fetch(url, { mode: 'no-cors' }).then(() => 'reached', () => 'blocked'); \
                    Promise.all([ask('https://pagead2.googlesyndication.com/pagead/trss-probe'), ask('https://example.com/')]) \
                    .then(([ad, other]) => 'ad ' + ad + ', other ' + other)",
                "awaitPromise": true,
                "returnByValue": true,
            }),
        )
        .await;
    assert_eq!(
        asked["result"]["value"], "ad blocked, other reached",
        "the frame of another site was not set up: {asked}"
    );

    // A click opens the post in a new window: a page of the run, at once.
    // The pool sets a popup up and lets it go without waiting for an answer of
    // the set-up (`Network.enable` is not answered while the popup waits for
    // the debugger; waiting for it held the popup for `cdp::COMMAND_TIMEOUT`,
    // 30 s, ticket 0054).
    let popup = open_popup(&screen, &auth, &prepared, "popup").await;
    // The post's own page stays; the popup closes.
    assert!(
        !auth
            .close_page("job-1", &prepared.run_id, &prepared.target_id)
            .await
    );
    close_popup(&auth, &prepared, &popup).await;

    // A window that asks for a blocklisted address as soon as its document
    // loads is set up before it: the request is blocked, and one for an
    // address that is not on the list goes through.
    let probe = open_popup(&screen, &auth, &prepared, "probe-popup").await;
    let attached = screen
        .conn
        .command(
            None,
            "Target.attachToTarget",
            json!({ "targetId": probe, "flatten": true }),
        )
        .await
        .unwrap();
    let probe_view = Screen {
        conn: screen.conn.clone(),
        session: attached["sessionId"].as_str().unwrap().to_owned(),
    };
    let mut title = Value::Null;
    for _ in 0..80 {
        title = probe_view.eval("document.title").await;
        if title.as_str().is_some_and(|t| t.starts_with("ad ")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(
        title.as_str().unwrap_or_default(),
        "ad blocked, other reached",
        "the popup was not set up before it loaded (or the network is out)"
    );
    close_popup(&auth, &prepared, &probe).await;

    auth.release("job-1").await;
    assert!(!auth.is_live("job-1", &prepared.run_id));
    assert!(auth.pages("job-1", &prepared.run_id).is_empty());
}
