//! A post whose file comes only after a person passes the site's check in the
//! server browser (`docs/specs/jobs.md`, 작업 화면 안의 인증과 브라우저 수명).
//!
//! # The seam
//!
//! A source that finds such a post answers [`crate::Opened::BrowserAuth`]
//! with the [`AuthPage`] that says how its page is brought to the check. The
//! job then asks its [`AuthBrowser`] to prepare it ([`AuthBrowser::prepare`]):
//! the job's browser run opens the post, the source's driver does what a
//! person would do up to the check (the fake source: clicks the download card,
//! centered on the screen) and stops there; it never touches the check
//! itself. The job waits (`인증 필요`) with the run bound to it, and a person
//! passes the check on the job's remote screen, which the web relays to the
//! same page. The site then starts a download in the browser:
//! [`AuthBrowser::wait_file`] takes it out of the run into a folder of the
//! job's, and [`arrived`] makes it the post's file, which the job receives
//! through the same steps as any other file (`docs/specs/jobs.md`, 공통 수신
//! 결과와 실패 분류).
//!
//! A source that adds a site (erulabo, ticket 0041) adds a variant to
//! [`AuthPage`] and its driver: what opens the post, where the download card
//! is, and how its file arrives are its own; the binding, the remote screen
//! and the receipt are the same.
//!
//! [`BrowserAuth`] is the [`AuthBrowser`] over a [`trss_browser::BrowserPool`].
//!
//! No address leaves this module: a post's download may be signed. The
//! browser's own words are not logged either, only the kind of a failure.

use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    time::Duration,
};

use serde_json::{json, Value};
use trss_browser::{BrowserError, BrowserPool, BrowserRun, DownloadState, Page};
use url::Url;

use crate::{fake, Failure, FailureKind, PostFile};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// How long a preparation waits for a slot of the server browser: a run of
/// another job that waits for a person holds its slot until it is idle.
pub const START_WAIT: Duration = Duration::from_secs(60);
/// How long the page has to reach the check.
pub const READY_TIMEOUT: Duration = Duration::from_secs(45);

/// How a post's page is brought to the site's check: the source's driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthPage {
    /// The fake source's post ([`fake`]): its pages are served inside the
    /// browser, with no network.
    Fake(fake::FakeCheck),
}

impl AuthPage {
    /// What the check is called on the job's screen and in its log.
    pub fn reason(&self) -> &'static str {
        match self {
            AuthPage::Fake(_) => fake::CHECK_REASON,
        }
    }
}

/// What a preparation is asked for.
#[derive(Debug, Clone, Copy)]
pub struct PrepareRequest<'a> {
    /// The job the browser run belongs to.
    pub job: &'a str,
    pub post: &'a Url,
    pub page: &'a AuthPage,
}

/// A page brought to the check: the run that shows it and the page itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub run_id: String,
    /// The DevTools target of the page, which the remote screen shows.
    pub target_id: String,
}

/// What waiting for the file of a prepared page came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Waited {
    /// The browser downloaded a file; it is now at `path` (in the folder the
    /// wait was given), named `name`.
    File { name: String, path: PathBuf },
    /// A download ended without its file (canceled, over a limit, or not a
    /// file the browser's folder could hand over): `reason` says which. The
    /// page is still there, so the wait can go on.
    NotTaken { reason: String },
    /// The run is over (idle, lost, ended): nothing more comes from it.
    Ended,
}

/// The server browser as a job that needs a person's check sees it. The one a
/// job has is chosen by the worker: the browser pool's, or none.
pub trait AuthBrowser: Send + Sync {
    /// Starts the job's run (or takes the one it has), opens the post in a
    /// new page and brings it to the check; earlier pages of the run are
    /// closed. A failure of the browser or the site is
    /// [`FailureKind::Network`], a page that is not what the driver knows
    /// [`FailureKind::Changed`].
    fn prepare<'a>(
        &'a self,
        request: PrepareRequest<'a>,
    ) -> BoxFuture<'a, Result<Prepared, Failure>>;

    /// Waits for the next download of the run `run_id` of `job` to end, and
    /// moves a completed one into `staging` (made if missing).
    fn wait_file<'a>(
        &'a self,
        job: &'a str,
        run_id: &'a str,
        staging: &'a Path,
    ) -> BoxFuture<'a, Waited>;

    /// Whether `run_id` is the live run of `job`. Looks only.
    fn is_live(&self, job: &str, run_id: &str) -> bool;

    /// A person opened the job's screen: the run's idle time counts from now.
    /// Whether the run is there.
    fn touch(&self, job: &str, run_id: &str) -> bool;

    /// The job needs no more of the browser: its run goes.
    fn release<'a>(&'a self, job: &'a str) -> BoxFuture<'a, ()>;
}

/// The key of a file a post gave through the browser: the post's host and
/// path and the file's name, free of any address the download came from.
pub fn file_key(post: &Url, name: &str) -> String {
    format!(
        "browser:{}{}#{name}",
        post.host_str().unwrap_or_default(),
        post.path()
    )
}

/// The post's file that a download made, now at `path`: the job receives it
/// from there ([`PostFile`]'s staged path).
pub fn arrived(post: &Url, name: &str, path: PathBuf) -> PostFile {
    PostFile::new(file_key(post, name), name.to_owned()).with_staged(path)
}

/// A failure of the browser, in words that are the same whatever the browser
/// said. Only the kind of the error is logged.
fn browser_failure(err: BrowserError) -> Failure {
    let (kind, reason) = match &err {
        BrowserError::RunEnded { .. } => ("run ended", "서버 브라우저의 실행이 끝났어요"),
        BrowserError::Launcher(_) => ("launcher", "서버 브라우저를 시작하지 못했어요"),
        BrowserError::Timeout(_) => ("timeout", "서버 브라우저가 시간 안에 답하지 않았어요"),
        BrowserError::PageGone(_) => ("page gone", "서버 브라우저의 페이지가 사라졌어요"),
        _ => ("other", "서버 브라우저를 쓰지 못했어요"),
    };
    eprintln!("trss-subtitles: the browser of a site's check failed: {kind}");
    Failure::new(FailureKind::Network, reason)
}

/// Scrolls the element `find` (a JavaScript expression that gives it, or
/// `null`) to the middle of the screen and clicks its middle with the mouse,
/// as a person does: the check a site shows in its place is then on the first
/// screen of a PC and of a phone. Whether the element was there.
pub async fn click_centered(page: &Page, find: &str) -> Result<bool, Failure> {
    let script = format!(
        "(() => {{ const el = ({find}); if (!el) return null;
           el.scrollIntoView({{block: 'center', inline: 'center'}});
           const r = el.getBoundingClientRect();
           return JSON.stringify({{x: r.left + r.width / 2, y: r.top + r.height / 2}}); }})()"
    );
    let point = match page.evaluate(&script).await.map_err(browser_failure)? {
        Value::String(text) => serde_json::from_str::<Value>(&text)
            .map_err(|_| Failure::new(FailureKind::Changed, "누를 자리를 읽지 못했어요"))?,
        _ => return Ok(false),
    };
    for (kind, button, count) in [
        ("mouseMoved", "none", 0),
        ("mousePressed", "left", 1),
        ("mouseReleased", "left", 1),
    ] {
        page.send(
            "Input.dispatchMouseEvent",
            json!({ "type": kind, "x": point["x"], "y": point["y"], "button": button, "clickCount": count }),
        )
        .await
        .map_err(browser_failure)?;
    }
    Ok(true)
}

/// Polls `script` in the page until it gives `true`, for up to `wait`.
pub async fn wait_until(page: &Page, script: &str, wait: Duration) -> Result<bool, Failure> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        // A page in the middle of loading may refuse an evaluation.
        if let Ok(Value::Bool(true)) = page.evaluate(script).await {
            return Ok(true);
        }
        if page.run().is_ended() {
            return Err(browser_failure(BrowserError::RunEnded {
                run: page.run().run_id().to_owned(),
            }));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The [`AuthBrowser`] over a server browser pool.
#[derive(Clone)]
pub struct BrowserAuth {
    pool: BrowserPool,
    start_wait: Duration,
}

impl BrowserAuth {
    pub fn new(pool: BrowserPool) -> BrowserAuth {
        BrowserAuth {
            pool,
            start_wait: START_WAIT,
        }
    }

    /// The browser as a job takes it.
    pub fn shared(pool: BrowserPool) -> std::sync::Arc<dyn AuthBrowser> {
        std::sync::Arc::new(BrowserAuth::new(pool))
    }

    fn live(&self, job: &str, run_id: &str) -> Option<BrowserRun> {
        self.pool
            .run_of_job(job)
            .filter(|run| run.run_id() == run_id)
    }

    async fn bring(&self, request: PrepareRequest<'_>) -> Result<Prepared, Failure> {
        let run = tokio::time::timeout(self.start_wait, self.pool.start(request.job))
            .await
            .map_err(|_| {
                Failure::new(
                    FailureKind::Network,
                    "서버 브라우저에 빈자리가 나지 않았어요",
                )
            })?
            .map_err(browser_failure)?;
        // Not ended for being idle while the page is brought to the check.
        let _busy = run.busy_guard().map_err(browser_failure)?;
        // What an earlier page of the run downloaded is not this one's.
        crate::winpng::drain(|| run.download_finished())
            .await
            .map_err(browser_failure)?;
        let page = run.new_page("about:blank").await.map_err(browser_failure)?;
        let driven = match request.page {
            AuthPage::Fake(check) => fake::drive_check(&page, request.post, check).await,
        };
        if let Err(failure) = driven {
            let _ = page.close().await;
            return Err(failure);
        }
        // The new page is the one shown; the ones before it go (after it is
        // there, so the browser always has a window).
        for other in run.pages() {
            if other.target_id() != page.target_id() {
                let _ = other.close().await;
            }
        }
        Ok(Prepared {
            run_id: run.run_id().to_owned(),
            target_id: page.target_id().to_owned(),
        })
    }

    async fn next_file(&self, job: &str, run_id: &str, staging: &Path) -> Waited {
        let Some(run) = self.live(job, run_id) else {
            return Waited::Ended;
        };
        let download = match run.download_finished().await {
            Ok(download) => download,
            Err(_) => return Waited::Ended,
        };
        if download.state != DownloadState::Completed {
            return Waited::NotTaken {
                reason: "브라우저의 다운로드가 끝나지 못했어요 (취소되었거나 크기 한도를 넘었어요)"
                    .to_owned(),
            };
        }
        match run.move_download(&download, staging).await {
            Ok(moved) => Waited::File {
                name: moved.name,
                path: moved.path,
            },
            Err(BrowserError::RunEnded { .. }) => Waited::Ended,
            Err(err) => {
                let failure = browser_failure(err);
                Waited::NotTaken {
                    reason: format!("브라우저가 받은 파일을 가져오지 못했어요: {failure}"),
                }
            }
        }
    }
}

impl AuthBrowser for BrowserAuth {
    fn prepare<'a>(
        &'a self,
        request: PrepareRequest<'a>,
    ) -> BoxFuture<'a, Result<Prepared, Failure>> {
        Box::pin(self.bring(request))
    }

    fn wait_file<'a>(
        &'a self,
        job: &'a str,
        run_id: &'a str,
        staging: &'a Path,
    ) -> BoxFuture<'a, Waited> {
        Box::pin(self.next_file(job, run_id, staging))
    }

    fn is_live(&self, job: &str, run_id: &str) -> bool {
        self.live(job, run_id).is_some_and(|run| !run.is_ended())
    }

    fn touch(&self, job: &str, run_id: &str) -> bool {
        self.live(job, run_id)
            .is_some_and(|run| run.touch().is_ok())
    }

    fn release<'a>(&'a self, job: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move { self.pool.end_job(job).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_came_through_the_browser_is_keyed_by_its_post_and_name_only() {
        let post = Url::parse("https://fake.trss.invalid/check/ep1?x=1").unwrap();
        assert_eq!(
            file_key(&post, "ep1.srt"),
            "browser:fake.trss.invalid/check/ep1#ep1.srt"
        );
        let file = arrived(&post, "ep1.srt", PathBuf::from("/area/.tmp/x/ep1.srt"));
        assert_eq!(file.name, "ep1.srt");
        assert_eq!(file.staged(), Some(Path::new("/area/.tmp/x/ep1.srt")));
    }
}
