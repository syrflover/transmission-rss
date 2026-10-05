//! erulabo's posts in the real browser image (ignored).
//!
//! - [`a_fake_erulabo_post_is_brought_to_its_check_and_its_answer_is_kept`]
//!   needs no network: the post and its download are answered from inside
//!   the browser by a page shaped like erulabo's (2026-10-03: the cards, the
//!   check put in the card's place, the site's own time limit, then a Google
//!   Drive download). It checks the card of the episode is clicked, the check
//!   is on the first screen of a PC and a phone, a check that went away comes
//!   back, a person's click gets the file with what its answer said, and a
//!   web page instead of the file is a refusal.
//! - [`a_real_erulabo_post_waits_at_its_check_on_the_first_screen`] opens a
//!   real post (`ERULABO_POST`, default `https://erulabo.com/859`, episode
//!   `ERULABO_EPISODE`, default `1`) and stops at the site's check. Nothing
//!   touches the check: it is left to go away by itself.
//!
//! Needs Docker and the browser image (`TRSS_BROWSER_IMAGE`, default
//! `ghcr.io/syrflover/trss-browser:local`, built from `Dockerfile.browser`):
//!
//! ```sh
//! cargo test -p trss-subtitles --features test-hooks --test it erulabo_sample:: -- --ignored --nocapture
//! ```
//!
//! Each test starts a throwaway container named
//! `trss-erulabo-sample-<pid>-<test>` (removed at the end).

use std::{
    path::Path,
    process::{Command, Output},
    sync::Arc,
    time::Duration,
};

use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use trss_browser::{
    cdp::Connection, client::LauncherClient, BrowserPolicy, BrowserPool, Page, PolicySource,
    PoolConfig,
};
use trss_subtitles::{
    auth::{self, AuthBrowser, AuthPage, BrowserAuth, PrepareRequest, Waited},
    erulabo::{self, ErulaboCheck, ErulaboSource},
    FailureKind, Opened, Snapshot, Sources,
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

async fn start(test: &str, downloads: &Path) -> (Container, String) {
    let name = format!("trss-erulabo-sample-{}-{test}", std::process::id());
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

async fn pool(base: &str, downloads: &Path) -> BrowserPool {
    BrowserPool::new(
        PoolConfig::new(base.parse().unwrap(), TOKEN, downloads),
        trss_core::system_clock(),
        PolicySource::fixed(BrowserPolicy::default()),
    )
    .await
    .unwrap()
}

/// A second client of the run, as the web's remote screen is: its own
/// connection and its own session on the shown page.
struct Screen {
    conn: Connection,
    session: String,
}

impl Screen {
    async fn open(base: &str, run_id: &str, target_id: &str) -> Screen {
        let launcher = LauncherClient::new(base.parse().unwrap(), TOKEN).unwrap();
        let conn = Connection::connect(&launcher.cdp_url(run_id), TOKEN)
            .await
            .unwrap();
        let attached = conn
            .command(
                None,
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
            )
            .await
            .unwrap();
        let session = attached["sessionId"].as_str().unwrap().to_owned();
        Screen { conn, session }
    }

    async fn send(&self, method: &str, params: Value) -> Value {
        self.conn
            .command(Some(&self.session), method, params)
            .await
            .unwrap()
    }

    /// The size of a PC's window, or of a phone (its touch too).
    async fn device(&self, width: u32, height: u32, phone: bool) {
        self.send(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": width,
                "height": height,
                "deviceScaleFactor": if phone { 3 } else { 1 },
                "mobile": phone,
            }),
        )
        .await;
        self.send(
            "Emulation.setTouchEmulationEnabled",
            json!({ "enabled": phone, "maxTouchPoints": if phone { 5 } else { 1 } }),
        )
        .await;
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

    /// Where the check's frame is on the screen (`null` when the page shows
    /// no check), as `{x, y, width, height}` of the frame, else of the place
    /// the site keeps for it.
    async fn check_box(&self) -> Option<(f64, f64, f64, f64)> {
        let rect = self
            .eval(
                "(() => { const w = document.querySelector('[data-file-download-turnstile-wrap]');
                   if (!w || w.hidden || !w.getClientRects().length) return null;
                   const f = w.querySelector('iframe, #box') || w;
                   const r = f.getBoundingClientRect();
                   return JSON.stringify({what: f.tagName, x: r.x, y: r.y, width: r.width, height: r.height}); })()",
            )
            .await;
        let rect: Value = serde_json::from_str(rect.as_str()?).ok()?;
        println!("  the check's {} at {rect}", rect["what"]);
        let n = |k: &str| rect[k].as_f64().unwrap();
        Some((n("x"), n("y"), n("width"), n("height")))
    }

    /// Asserts the check's box is on the first screen of a `width`×`height`
    /// screen: all of its height, and its left part where the box a person
    /// presses is.
    async fn assert_check_on_screen(&self, width: u32, height: u32) {
        let mut found = None;
        for _ in 0..20 {
            // The size changed: the page settles first.
            tokio::time::sleep(Duration::from_millis(250)).await;
            found = self.check_box().await;
            if let Some((x, y, w, h)) = found {
                let inside = x >= 0.0
                    && y >= 0.0
                    && x + w.min(80.0) <= f64::from(width)
                    && y + h <= f64::from(height);
                if inside {
                    return;
                }
            }
        }
        panic!("the check is not on the first screen of {width}x{height}: {found:?}");
    }

    /// A person's click (a trusted one) in the middle of the box.
    async fn click(&self, x: f64, y: f64) {
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

/// The Drive IDs the fake page's cards download from.
const SERIES_ID: &str = "1FakeDriveIdOfTheSeries000abcd";
const EPISODE_ID: &str = "1FakeDriveIdOfEpisode12000abcd";

/// A post shaped like erulabo's (2026-10-03), cut to what the driver uses:
/// two cards far apart (the series' first), the check put in the clicked
/// card's place and scrolled to smoothly, and the site's own time limit,
/// after which the card comes back under a notice over the whole page for 2
/// seconds. A trusted click on the box downloads from
/// Drive, as the site's `window.open` does.
fn fake_post(limit_ms: u64) -> String {
    let card = |file: &str, title: &str| {
        format!(
            r#"<button type="button" class="og-link og-link-button" data-file-url="{file}"><span class="og-preview"><span class="og-body"><span class="og-title">{title}</span><span class="og-description">여기를 클릭하면 보안 확인 후 다운로드가 시작됩니다.</span></span></span></button>"#
        )
    };
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<style>
:root {{ scroll-behavior: smooth; }}
body {{ margin: 0; font: 16px sans-serif; }}
.filler {{ height: 1800px; background: linear-gradient(#eee, #ccc); }}
.og-link {{ display: block; box-sizing: border-box; width: 92%; max-width: 680px; margin: 24px auto; padding: 16px; text-align: left; }}
.og-body {{ display: block; }}
[data-file-download-turnstile-wrap] {{ margin-top: 12px; min-height: 65px; }}
#box {{ width: 300px; height: 65px; background: #fafafa; border: 1px solid #888; }}
.toast-backdrop {{ position: fixed; inset: 0; z-index: 10; background: rgba(0, 0, 0, 0.3); }}
</style>
<script type="application/ld+json">{{"@type":"BlogPosting","dateModified":"2026-10-01T10:00:00+09:00"}}</script>
<script>
window.turnstile = {{
  render(el, options) {{
    const box = document.createElement('div');
    box.id = 'box';
    box.addEventListener('click', (e) => {{ if (e.isTrusted) options.callback('token'); }});
    el.appendChild(box);
    return 'widget';
  }},
  remove() {{}},
}};
</script></head><body>
<div class="filler"></div>
<div id="post-body">
{series}
<div class="filler"></div>
{episode}
<div class="filler"></div>
</div>
<script>
const DRIVE = {{ '/file/card-all': '{SERIES_ID}', '/file/card-12': '{EPISODE_ID}' }};
document.getElementById('post-body').addEventListener('click', (ev) => {{
  const button = ev.target.closest('button[data-file-url]');
  if (!button) return;
  const file = button.dataset.fileUrl;
  const holder = document.createElement('div');
  holder.className = 'og-link';
  holder.innerHTML = button.innerHTML;
  button.replaceWith(holder);
  const wrap = document.createElement('div');
  wrap.setAttribute('data-file-download-turnstile-wrap', '');
  holder.querySelector('.og-body').appendChild(wrap);
  wrap.scrollIntoView({{ block: 'center', behavior: 'smooth' }});
  const timer = setTimeout(() => {{
    holder.replaceWith(button);
    const notice = document.createElement('div');
    notice.className = 'toast-backdrop';
    document.body.appendChild(notice);
    setTimeout(() => notice.remove(), 2000);
  }}, {limit_ms});
  turnstile.render(wrap, {{ callback: () => {{
    clearTimeout(timer);
    location.assign('https://drive.usercontent.google.com/download?id=' + DRIVE[file] + '&export=download');
  }} }});
}});
</script></body></html>"#,
        series = card("/file/card-all", "테스트 (1-12)"),
        episode = card("/file/card-12", "테스트 (12)"),
    )
}

/// How the fake Drive answers the download.
#[derive(Clone, Copy)]
enum Drive {
    File,
    /// What a file that is not (or no longer) available gets: `403` and a
    /// page. From Drive that is a file not shared.
    Refused,
}

const SRT: &[u8] = b"1\n00:00:01,000 --> 00:00:02,000\n12\n";

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Answers the page's requests to erulabo and Drive from inside the browser.
fn serve(page: Page, limit_ms: u64, drive: Drive) -> auth::BoxFuture<'static, ()> {
    Box::pin(async move {
        let session = page.session_id().unwrap();
        let run = page.run().clone();
        let mut events = run.events().unwrap();
        page.send(
            "Fetch.enable",
            json!({ "patterns": [
                { "urlPattern": "https://erulabo.com/*", "requestStage": "Request" },
                { "urlPattern": "https://drive.usercontent.google.com/*", "requestStage": "Request" },
            ] }),
        )
        .await
        .unwrap();
        tokio::spawn(async move {
            loop {
                let event = tokio::select! {
                    _ = run.ended() => break,
                    event = events.recv() => event,
                };
                let event = match event {
                    Ok(event) => event,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                };
                if event.session_id.as_deref() != Some(session.as_str())
                    || event.method != "Fetch.requestPaused"
                {
                    continue;
                }
                let id = event.params["requestId"].as_str().unwrap().to_owned();
                let url = Url::parse(event.params["request"]["url"].as_str().unwrap()).unwrap();
                let (status, mut headers, body): (u16, Vec<(&str, String)>, Vec<u8>) =
                    match (url.host_str(), drive) {
                        (Some("erulabo.com"), _) => (
                            200,
                            vec![("Content-Type", "text/html; charset=utf-8".to_owned())],
                            fake_post(limit_ms).into_bytes(),
                        ),
                        (_, Drive::File) => (
                            200,
                            vec![
                                ("Content-Type", "application/octet-stream".to_owned()),
                                (
                                    "Content-Disposition",
                                    "attachment; filename=\"ep12.srt\"".to_owned(),
                                ),
                                ("Last-Modified", "Fri, 02 Oct 2026 02:11:11 GMT".to_owned()),
                            ],
                            SRT.to_vec(),
                        ),
                        (_, Drive::Refused) => (
                            403,
                            vec![("Content-Type", "text/html; charset=utf-8".to_owned())],
                            b"<html><body>403. That's an error.</body></html>".to_vec(),
                        ),
                    };
                headers.push(("Content-Length", body.len().to_string()));
                headers.push(("Cache-Control", "no-store".to_owned()));
                let headers: Vec<Value> = headers
                    .into_iter()
                    .map(|(name, value)| json!({ "name": name, "value": value }))
                    .collect();
                let _ = page
                    .send(
                        "Fetch.fulfillRequest",
                        json!({
                            "requestId": id,
                            "responseCode": status,
                            "responseHeaders": headers,
                            "body": base64(&body),
                        }),
                    )
                    .await;
            }
        });
    })
}

/// A person passes the fake check while the job waits for its file, as the
/// worker's watch does: what the wait came to.
async fn pass_check(
    auth: &BrowserAuth,
    job: &str,
    run_id: &str,
    staging: &Path,
    screen: &Screen,
) -> Waited {
    let waiting = tokio::spawn({
        let (auth, job, run_id, staging) = (
            auth.clone(),
            job.to_owned(),
            run_id.to_owned(),
            staging.to_owned(),
        );
        async move { auth.wait_file(&job, &run_id, &staging).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (x, y, w, h) = screen.check_box().await.unwrap();
    screen.click(x + w / 2.0, y + h / 2.0).await;
    tokio::time::timeout(Duration::from_secs(30), waiting)
        .await
        .expect("nothing came in time")
        .unwrap()
}

/// The check of the fake post's episode 12, by the rules the source uses.
fn fake_check() -> AuthPage {
    let titles = ["테스트 (1-12)", "테스트 (12)"];
    let files = ["/file/card-all", "/file/card-12"];
    let chosen = erulabo::choose_card("12", &titles).unwrap();
    AuthPage::Erulabo(ErulaboCheck::new(
        files[chosen],
        titles[chosen],
        Snapshot::default(),
    ))
}

#[tokio::test]
#[ignore = "needs docker and the trss-browser image"]
async fn a_fake_erulabo_post_is_brought_to_its_check_and_its_answer_is_kept() {
    let downloads = tempfile::tempdir().unwrap();
    let (_container, base) = start("fake", downloads.path()).await;
    let pool = pool(&base, downloads.path()).await;
    let post = Url::parse("https://erulabo.com/900").unwrap();
    let page = fake_check();

    // The file comes, with what its answer said.
    let limit_ms = 4_000;
    let auth = BrowserAuth::new(pool.clone())
        .with_page_setup(Arc::new(move |page| serve(page, limit_ms, Drive::File)));
    let prepared = auth
        .prepare(PrepareRequest {
            job: "job-file",
            post: &post,
            page: &page,
        })
        .await
        .unwrap();
    let screen = Screen::open(&base, &prepared.run_id, &prepared.target_id).await;
    // The episode's card was clicked, not the series' before it.
    assert_eq!(
        screen
            .eval("[!!document.querySelector('button[data-file-url=\"/file/card-all\"]'), !!document.querySelector('button[data-file-url=\"/file/card-12\"]')]")
            .await,
        json!([true, false])
    );
    for (width, height, phone) in [(1440, 900, false), (390, 844, true)] {
        println!("a {width}x{height} screen:");
        screen.device(width, height, phone).await;
        screen.assert_check_on_screen(width, height).await;
    }

    // The site's time ran out: the card is back under the site's notice, and
    // the next opening of the screen brings the check back once the notice
    // is gone.
    tokio::time::sleep(Duration::from_millis(limit_ms + 300)).await;
    assert_eq!(screen.check_box().await, None, "the site's time ran out");
    assert_eq!(
        screen
            .eval("!!document.querySelector('.toast-backdrop')")
            .await,
        json!(true)
    );
    auth.rearm("job-file", &prepared.run_id).await;
    println!("brought back:");
    screen.assert_check_on_screen(390, 844).await;

    let staging = tempfile::tempdir().unwrap();
    let waited = pass_check(&auth, "job-file", &prepared.run_id, staging.path(), &screen).await;
    let Waited::File { name, path } = waited else {
        panic!("{waited:?}");
    };
    assert_eq!(name, "ep12.srt");
    assert_eq!(std::fs::read(&path).unwrap(), SRT);
    let file = auth::arrived(&post, &page, &name, path).await;
    let mut expected = Snapshot::default();
    expected.push(auth::DRIVE_ID, EPISODE_ID);
    expected.push("last_modified", "Fri, 02 Oct 2026 02:11:11 GMT");
    expected.push("content_length", SRT.len().to_string());
    assert_eq!(file.snapshot, expected);
    let source = Sources::none()
        .with_erulabo(ErulaboSource::new(trss_subtitles::drive::Drive::new()))
        .for_post(&post)
        .unwrap();
    let fetch = source.fetch(&post, &file).await.unwrap();
    assert_eq!(
        (
            fetch.status,
            fetch.content_type.as_deref(),
            fetch.expected_size
        ),
        (
            Some(200),
            Some("application/octet-stream"),
            Some(SRT.len() as u64)
        )
    );
    auth.release("job-file").await;

    // Drive answers with a page: a refusal, no file. Drive's `403` is a file
    // that is not available to everyone (`원본 없음`), not an address past
    // its time.
    let auth = BrowserAuth::new(pool.clone())
        .with_page_setup(Arc::new(move |page| serve(page, 30_000, Drive::Refused)));
    let prepared = auth
        .prepare(PrepareRequest {
            job: "job-expired",
            post: &post,
            page: &page,
        })
        .await
        .unwrap();
    let screen = Screen::open(&base, &prepared.run_id, &prepared.target_id).await;
    screen.device(1440, 900, false).await;
    screen.assert_check_on_screen(1440, 900).await;
    let staging = tempfile::tempdir().unwrap();
    let waited = pass_check(
        &auth,
        "job-expired",
        &prepared.run_id,
        staging.path(),
        &screen,
    )
    .await;
    let Waited::Refused(failure) = waited else {
        panic!("{waited:?}");
    };
    println!("refused: {failure:?}");
    assert_eq!(failure.kind, FailureKind::Missing);
    assert_eq!(failure.status, Some(403));
    assert!(!failure.reason.contains(EPISODE_ID));
    assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 0);
    auth.release("job-expired").await;
}

#[tokio::test]
#[ignore = "needs docker, the trss-browser image and the network"]
async fn a_real_erulabo_post_waits_at_its_check_on_the_first_screen() {
    let post = Url::parse(
        &std::env::var("ERULABO_POST").unwrap_or_else(|_| "https://erulabo.com/859".to_owned()),
    )
    .unwrap();
    let episode = std::env::var("ERULABO_EPISODE").unwrap_or_else(|_| "1".to_owned());
    let downloads = tempfile::tempdir().unwrap();
    let (_container, base) = start("real", downloads.path()).await;
    let pool = pool(&base, downloads.path()).await;
    let auth = BrowserAuth::new(pool.clone());

    let source = Sources::none()
        .with_erulabo(ErulaboSource::new(trss_subtitles::drive::Drive::new()))
        .for_post(&post)
        .unwrap();
    let Opened::BrowserAuth { reason, page } = source.open(&post, &episode).await.unwrap() else {
        panic!("not a check post");
    };
    let AuthPage::Erulabo(check) = &page else {
        panic!("{page:?}");
    };
    println!(
        "{post} episode {episode}: {reason}, card {:?} ({:?})",
        check.title(),
        check.snapshot()
    );
    let prepared = auth
        .prepare(PrepareRequest {
            job: "job-real",
            post: &post,
            page: &page,
        })
        .await
        .unwrap();
    let screen = Screen::open(&base, &prepared.run_id, &prepared.target_id).await;
    for (width, height, phone) in [(1440, 900, false), (390, 844, true)] {
        println!("a {width}x{height} screen:");
        screen.device(width, height, phone).await;
        screen.assert_check_on_screen(width, height).await;
    }

    // No one passes it: the site takes the check away after its time (30
    // seconds, 2026-10-03), and the next opening of the screen brings it back.
    let mut gone = false;
    for _ in 0..90 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if screen.check_box().await.is_none() {
            gone = true;
            break;
        }
    }
    assert!(gone, "the site kept its check past 45 seconds");
    auth.rearm("job-real", &prepared.run_id).await;
    println!("brought back:");
    screen.assert_check_on_screen(390, 844).await;

    auth.release("job-real").await;
    assert!(!auth.is_live("job-real", &prepared.run_id));
}
