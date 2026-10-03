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
//! A site adds a variant to [`AuthPage`] and its driver: what opens the post,
//! where the download card is, and how its file arrives are its own; the
//! binding, the remote screen and the receipt are the same. erulabo
//! ([`crate::erulabo`]) is the real one: its check goes away after 30
//! seconds, so a person's opening of the job's screen brings it back
//! ([`AuthBrowser::rearm`]), and an answer that is a web page instead of its
//! file ends the wait ([`Waited::Refused`]).
//!
//! # What the download's answer said
//!
//! The browser reports the answer of the navigation it turned into the
//! download ([`trss_browser::DownloadAnswer`]): its status, media type,
//! `Content-Length` and `Last-Modified`, and the address, which stays in
//! memory. When the address is a Google Drive file's, its ID is taken from
//! it (user decision, 2026-10-02: to learn how a creator revises a file).
//! These go to a file beside the downloaded one in the item's folder
//! ([`ANSWER`]), and from there to the file's snapshot and its receipt
//! ([`arrived`]); the folder goes when the item settles. A refusal is kept
//! the same way ([`REFUSED`]) until the item's next run fails it.
//!
//! # A page a person browses
//!
//! A find job (`docs/specs/subtitles.md`, 직접 찾기와 자막 올리기) asks for
//! [`AuthPage::Browse`]: the post is opened and nothing on it is clicked. The
//! person goes to an earlier post on the job's remote screen, opens its
//! popups, passes a site's check there, and starts the downloads. Every
//! download the run completes is handed out by [`AuthBrowser::wait_file`] as
//! for a check; nothing judges the run's documents as refusals, since what
//! came is judged by its bytes afterwards. [`AuthBrowser::downloading`] tells
//! whether a download is still on its way, and [`AuthBrowser::pages`] which
//! pages the run has open, so the screen can follow a popup.
//!
//! [`BrowserAuth`] is the [`AuthBrowser`] over a [`trss_browser::BrowserPool`].
//!
//! No address leaves this module: a post's download may be signed. The
//! browser's own words are not logged either, only the kind of a failure.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use trss_browser::{
    BrowserError, BrowserPool, BrowserRun, Download, DownloadState, Page, PageDocument,
};
use url::Url;

use crate::{drive, erulabo, fake, http, Failure, FailureKind, PostFile, Snapshot};

/// The file beside a downloaded one that says what its answer said. A
/// download's name never starts with a dot ([`trss_browser::safe_file_name`]),
/// so the two never meet.
pub const ANSWER: &str = ".answer";
/// The file in an item's folder that says the download was refused.
pub const REFUSED: &str = ".refused";
/// The snapshot's name for a Google Drive file's ID.
pub const DRIVE_ID: &str = "drive_id";

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
    /// An erulabo post and the card of the episode ([`erulabo`]).
    Erulabo(erulabo::ErulaboCheck),
    /// A post a person browses from (a find job): opened, and nothing on it
    /// clicked. A fake post ([`fake::HOST`]) is served inside the browser.
    Browse,
}

/// What a page a person browses is called on the job's screen and in its log.
pub const BROWSE_REASON: &str = "직접 찾기";

impl AuthPage {
    /// What the check is called on the job's screen and in its log.
    pub fn reason(&self) -> &'static str {
        match self {
            AuthPage::Fake(_) => fake::CHECK_REASON,
            AuthPage::Erulabo(_) => erulabo::CHECK_REASON,
            AuthPage::Browse => BROWSE_REASON,
        }
    }

    /// What the post said about itself when it was opened, for the file's
    /// snapshot.
    pub fn snapshot(&self) -> Snapshot {
        match self {
            AuthPage::Fake(_) | AuthPage::Browse => Snapshot::default(),
            AuthPage::Erulabo(check) => check.snapshot().clone(),
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
    /// Where the file should have come from answered with a web page instead
    /// (an expired address, a file gone): the item fails with it. Nothing
    /// asks the address again.
    Refused(Failure),
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

    /// A person opened the job's screen of the live run `run_id`: a page
    /// whose check went away is brought back to it. Nothing by default.
    fn rearm<'a>(&'a self, job: &'a str, run_id: &'a str) -> BoxFuture<'a, ()> {
        let _ = (job, run_id);
        Box::pin(async {})
    }

    /// Whether a download of the run `run_id` of `job` is on its way: under
    /// way in the browser, ended and not yet handed out, or handed out and
    /// still being moved by [`AuthBrowser::wait_file`]. Looks only. A
    /// [`AuthBrowser::wait_file`] given up while this is `false` loses no
    /// file. `false` by default.
    fn downloading(&self, job: &str, run_id: &str) -> bool {
        let _ = (job, run_id);
        false
    }

    /// The DevTools targets of the pages the run `run_id` of `job` has open
    /// now (a popup is one), none when it is not live. Looks only. None by
    /// default.
    fn pages(&self, job: &str, run_id: &str) -> Vec<String> {
        let _ = (job, run_id);
        Vec::new()
    }

    /// A person asked to close the page `target` of the run `run_id` of
    /// `job` (a popup the post opened): it is closed, unless it is the page
    /// the run was prepared with, which never is. Whether it was closed.
    /// `false` by default.
    fn close_page<'a>(
        &'a self,
        job: &'a str,
        run_id: &'a str,
        target: &'a str,
    ) -> BoxFuture<'a, bool> {
        let _ = (job, run_id, target);
        Box::pin(async { false })
    }
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

/// What a download's answer said, as [`ANSWER`] keeps it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answered {
    pub status: Option<u16>,
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub last_modified: Option<String>,
    /// The Google Drive file's ID, when the download came from one.
    pub drive_id: Option<String>,
}

impl Answered {
    /// What `download` tells: its answer, and the Drive ID of its address.
    /// The address itself is not kept.
    pub fn of(download: &Download) -> Answered {
        let answer = download.answer.clone().unwrap_or_default();
        let drive_id =
            download
                .source
                .as_ref()
                .and_then(|source| match drive::link(source.url()) {
                    Some(drive::Link::File(id)) => Some(id),
                    _ => None,
                });
        Answered {
            status: download.answer.as_ref().and_then(|a| a.status),
            content_type: answer.content_type,
            content_length: answer.content_length,
            last_modified: answer.last_modified,
            drive_id,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "status": self.status,
            "content_type": self.content_type,
            "content_length": self.content_length,
            "last_modified": self.last_modified,
            "drive_id": self.drive_id,
        })
    }

    fn from_json(value: &Value) -> Answered {
        let text = |key: &str| value[key].as_str().map(str::to_owned);
        Answered {
            status: value["status"].as_u64().and_then(|s| u16::try_from(s).ok()),
            content_type: text("content_type"),
            content_length: value["content_length"].as_u64(),
            last_modified: text("last_modified"),
            drive_id: text("drive_id"),
        }
    }
}

/// Keeps what `answered` says beside the file that is to come into
/// `staging` (written whole, then renamed).
pub async fn record_answer(staging: &Path, answered: &Answered) -> std::io::Result<()> {
    write_whole(staging, ANSWER, answered.to_json().to_string().as_bytes()).await
}

/// Keeps `failure`, the refusal of the item's download, in its folder
/// `staging` for the item's next run ([`staged`]).
pub async fn record_refusal(staging: &Path, failure: &Failure) -> std::io::Result<()> {
    let kept = json!({
        "kind": failure.kind.code(),
        "reason": failure.reason,
        "status": failure.status,
        "content_type": failure.content_type,
    });
    write_whole(staging, REFUSED, kept.to_string().as_bytes()).await
}

async fn write_whole(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    let part = dir.join(format!("{name}.part"));
    tokio::fs::write(&part, bytes).await?;
    tokio::fs::rename(&part, dir.join(name)).await
}

/// What an item's folder holds from its check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Staged {
    /// The file the browser downloaded.
    File { name: String, path: PathBuf },
    /// The answer that refused it.
    Refused(Failure),
}

/// What the item's folder `dir` holds: a refusal, or the downloaded file (the
/// one file whose name does not start with a dot), or nothing.
pub async fn staged(dir: &Path) -> Option<Staged> {
    if let Ok(bytes) = tokio::fs::read(dir.join(REFUSED)).await {
        let kept: Value = serde_json::from_slice(&bytes).unwrap_or_default();
        let kind = kept["kind"]
            .as_str()
            .and_then(FailureKind::parse)
            .unwrap_or(FailureKind::NotAFile);
        let failure = Failure::new(
            kind,
            kept["reason"]
                .as_str()
                .unwrap_or("파일 대신 웹 페이지가 왔어요"),
        )
        .with_response(
            kept["status"].as_u64().and_then(|s| u16::try_from(s).ok()),
            kept["content_type"].as_str().map(str::to_owned),
            None,
        );
        return Some(Staged::Refused(failure));
    }
    let mut entries = tokio::fs::read_dir(dir).await.ok()?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if entry.file_type().await.is_ok_and(|t| t.is_file()) {
            return Some(Staged::File {
                name,
                path: entry.path(),
            });
        }
    }
    None
}

/// The post's file that a download made, now at `path`: the job receives it
/// from there ([`PostFile`]'s staged path). Its snapshot is what the post
/// said about itself (`page`) and what the download's answer said
/// ([`ANSWER`], beside the file); the answer's status and media type go with
/// the receipt. The length it announced goes only into the snapshot
/// (`content_length`): the job holds the file to its own length.
pub async fn arrived(post: &Url, page: &AuthPage, name: &str, path: PathBuf) -> PostFile {
    let answered = match path.parent() {
        Some(dir) => tokio::fs::read(dir.join(ANSWER))
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .map(|value| Answered::from_json(&value)),
        None => None,
    };
    let mut snapshot = page.snapshot();
    let mut file = PostFile::new(file_key(post, name), name.to_owned());
    if let Some(answered) = &answered {
        if let Some(id) = &answered.drive_id {
            snapshot.push(DRIVE_ID, id.clone());
        }
        if let Some(modified) = &answered.last_modified {
            snapshot.push(http::LAST_MODIFIED, modified.clone());
        }
        if let Some(length) = answered.content_length {
            snapshot.push(drive::CONTENT_LENGTH, length.to_string());
        }
        file = file.with_answer(crate::StagedAnswer {
            status: answered.status,
            content_type: answered.content_type.clone(),
        });
    }
    file.snapshot = snapshot;
    file.with_staged(path)
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
/// `null`) to the middle of the screen at once and clicks its middle with the
/// mouse, as a person does: the check a site shows in its place is then on
/// the first screen of a PC and of a phone. Whether the element was there;
/// an element something else covers is [`FailureKind::Changed`] and is not
/// clicked.
pub async fn click_centered(page: &Page, find: &str) -> Result<bool, Failure> {
    // `instant`: a page that scrolls smoothly (`scroll-behavior: smooth`, as
    // erulabo's does) would still be on its way, and the point read now would
    // be where the element was, not where it ends up. The point counts only
    // if the element itself is what a click there reaches (nothing covers it).
    let script = format!(
        "(() => {{ const el = ({find}); if (!el) return null;
           el.scrollIntoView({{block: 'center', inline: 'center', behavior: 'instant'}});
           const r = el.getBoundingClientRect();
           const x = r.left + r.width / 2, y = r.top + r.height / 2;
           const hit = document.elementFromPoint(x, y);
           return JSON.stringify({{x, y, covered: !(hit && (hit === el || el.contains(hit)))}}); }})()"
    );
    let point = match page.evaluate(&script).await.map_err(browser_failure)? {
        Value::String(text) => serde_json::from_str::<Value>(&text)
            .map_err(|_| Failure::new(FailureKind::Changed, "누를 자리를 읽지 못했어요"))?,
        _ => return Ok(false),
    };
    if point["covered"] != Value::Bool(false) {
        return Err(Failure::new(
            FailureKind::Changed,
            "누를 자리를 다른 것이 가리고 있어요",
        ));
    }
    let (Some(x), Some(y)) = (point["x"].as_f64(), point["y"].as_f64()) else {
        return Err(Failure::new(
            FailureKind::Changed,
            "누를 자리를 읽지 못했어요",
        ));
    };
    let mouse = |kind: &'static str, button: &'static str, count: u8| {
        page.send(
            "Input.dispatchMouseEvent",
            json!({ "type": kind, "x": x, "y": y, "button": button, "clickCount": count }),
        )
    };
    mouse("mouseMoved", "none", 0)
        .await
        .map_err(browser_failure)?;
    // The page may have moved on since the point was read (the site's notice
    // slides over it, the layout shifts): the press is made only where the
    // element still is and nothing covers it.
    let still = format!(
        "(() => {{ const el = ({find}); if (!el) return false;
           const r = el.getBoundingClientRect();
           if ({x} < r.left || {x} > r.right || {y} < r.top || {y} > r.bottom) return false;
           const hit = document.elementFromPoint({x}, {y});
           return !!hit && (hit === el || el.contains(hit)); }})()"
    );
    if page.evaluate(&still).await.map_err(browser_failure)? != Value::Bool(true) {
        return Err(Failure::new(
            FailureKind::Changed,
            "누르기 직전에 누를 자리가 바뀌어 누르지 않았어요",
        ));
    }
    for (kind, button) in [("mousePressed", "left"), ("mouseReleased", "left")] {
        mouse(kind, button, 1).await.map_err(browser_failure)?;
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

/// The page of a job brought to its check: in memory, as the runs are.
#[derive(Debug, Clone)]
struct Shown {
    run_id: String,
    target_id: String,
    post: Url,
    page: AuthPage,
}

/// The [`AuthBrowser`] over a server browser pool.
#[derive(Clone)]
pub struct BrowserAuth {
    pool: BrowserPool,
    start_wait: Duration,
    /// By job.
    shown: Arc<Mutex<HashMap<String, Shown>>>,
    /// The jobs whose page is being brought back to its check.
    rearming: Arc<Mutex<HashSet<String>>>,
    /// The runs that logged the host of a page they did not know.
    unknown_hosts: Arc<FirstPerRun>,
    /// The jobs whose download [`AuthBrowser::wait_file`] has taken from the
    /// run and is moving now ([`AuthBrowser::downloading`]).
    taking: Arc<Mutex<HashSet<String>>>,
    #[cfg(feature = "test-hooks")]
    page_setup: Option<PageSetup>,
}

/// What [`BrowserAuth::with_page_setup`] runs on each new page.
#[cfg(feature = "test-hooks")]
pub type PageSetup = Arc<dyn Fn(Page) -> BoxFuture<'static, ()> + Send + Sync>;

impl BrowserAuth {
    pub fn new(pool: BrowserPool) -> BrowserAuth {
        BrowserAuth {
            pool,
            start_wait: START_WAIT,
            shown: Arc::default(),
            rearming: Arc::default(),
            unknown_hosts: Arc::default(),
            taking: Arc::default(),
            #[cfg(feature = "test-hooks")]
            page_setup: None,
        }
    }

    /// The same, running `setup` on each page it opens before the page goes
    /// to the post: for the tests of a site's page in the real browser image,
    /// which answer the site's addresses from inside the browser. Only with
    /// the `test-hooks` feature.
    #[cfg(feature = "test-hooks")]
    pub fn with_page_setup(mut self, setup: PageSetup) -> BrowserAuth {
        self.page_setup = Some(setup);
        self
    }

    fn shown(&self, job: &str, run_id: &str) -> Option<Shown> {
        self.shown
            .lock()
            .expect("shown lock")
            .get(job)
            .filter(|s| s.run_id == run_id)
            .cloned()
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
        #[cfg(feature = "test-hooks")]
        if let Some(setup) = &self.page_setup {
            setup(page.clone()).await;
        }
        let driven = match request.page {
            AuthPage::Fake(check) => fake::drive_check(&page, request.post, check).await,
            AuthPage::Erulabo(check) => erulabo::drive_check(&page, request.post, check).await,
            AuthPage::Browse => browse(&page, request.post).await,
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
        self.shown.lock().expect("shown lock").insert(
            request.job.to_owned(),
            Shown {
                run_id: run.run_id().to_owned(),
                target_id: page.target_id().to_owned(),
                post: request.post.clone(),
                page: request.page.clone(),
            },
        );
        Ok(Prepared {
            run_id: run.run_id().to_owned(),
            target_id: page.target_id().to_owned(),
        })
    }

    async fn next_file(&self, job: &str, run_id: &str, staging: &Path) -> Waited {
        let Some(run) = self.live(job, run_id) else {
            return Waited::Ended;
        };
        // A site whose file can be refused with a page is watched for one.
        let watched = self
            .shown(job, run_id)
            .is_some_and(|s| matches!(s.page, AuthPage::Erulabo(_)));
        let download = tokio::select! {
            download = run.download_finished() => match download {
                Ok(download) => download,
                Err(_) => return Waited::Ended,
            },
            refused = self.refusal(job, &run), if watched => return Waited::Refused(refused),
        };
        // Set in the same poll the download was taken in, so whoever asks
        // `downloading` sees it as on its way until it is moved (or not).
        let _taking = Taking::new(&self.taking, job);
        if download.state != DownloadState::Completed {
            return Waited::NotTaken {
                reason: "브라우저의 다운로드가 끝나지 못했어요 (취소되었거나 크기 한도를 넘었어요)"
                    .to_owned(),
            };
        }
        // What the answer said goes first, so the file is never there
        // without it. A later download writes its own over it.
        if let Err(err) = record_answer(staging, &Answered::of(&download)).await {
            eprintln!(
                "trss-subtitles: what a download's answer said could not be kept: {}",
                err.kind()
            );
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

/// A download of a job taken from its run and not yet moved: the job is in
/// [`BrowserAuth::taking`] until this goes.
struct Taking {
    jobs: Arc<Mutex<HashSet<String>>>,
    job: String,
}

impl Taking {
    fn new(jobs: &Arc<Mutex<HashSet<String>>>, job: &str) -> Taking {
        jobs.lock().expect("taking lock").insert(job.to_owned());
        Taking {
            jobs: jobs.clone(),
            job: job.to_owned(),
        }
    }
}

impl Drop for Taking {
    fn drop(&mut self) {
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.remove(&self.job);
        }
    }
}

/// Opens the post a person browses from in `page` (a blank page of the job's
/// run) and waits until it has loaded, clicking nothing. A fake post is served
/// inside the browser ([`fake::HOST`]).
async fn browse(page: &Page, post: &Url) -> Result<(), Failure> {
    if !matches!(post.scheme(), "http" | "https") {
        return Err(Failure::new(
            FailureKind::Changed,
            "게시물 주소가 웹 주소가 아니에요",
        ));
    }
    if post.host_str() == Some(fake::HOST) {
        fake::serve_page(page.clone()).await?;
    }
    page.navigate(post.as_str())
        .await
        .map_err(browser_failure)?;
    // A page that never finishes loading (a stalled ad) is still the
    // person's to use: only a page that does not answer at all is a failure.
    if !wait_until(page, "document.readyState !== 'loading'", READY_TIMEOUT).await? {
        return Err(Failure::new(
            FailureKind::Network,
            "게시물이 시간 안에 열리지 않았어요",
        ));
    }
    Ok(())
}

/// What a document a page of the run was answered with means to the wait.
#[derive(Debug)]
enum Judged {
    /// It refuses the download instead of giving the file.
    Refused(Failure),
    /// A host the source does not know (`host` only, never the address): the
    /// wait goes on, and the host is told so a real check can learn it.
    UnknownHost(String),
    Nothing,
}

/// Judges a document of one of the run's pages ([`erulabo::download_refusal`],
/// [`erulabo::known_host`]).
fn judge(document: &PageDocument) -> Judged {
    let url = document.source.url();
    let Some(host) = url.host_str() else {
        return Judged::Nothing;
    };
    if let Some(status) = document.answer.status {
        if let Some(failure) =
            erulabo::download_refusal(url, status, document.answer.content_type.as_deref())
        {
            return Judged::Refused(failure);
        }
    }
    if erulabo::known_host(host) {
        Judged::Nothing
    } else {
        Judged::UnknownHost(host.to_owned())
    }
}

/// Which jobs' runs did something already: once per run.
#[derive(Debug, Default)]
struct FirstPerRun(Mutex<HashMap<String, String>>);

impl FirstPerRun {
    /// Whether this is the first time for the run `run_id` of `job`.
    fn first(&self, job: &str, run_id: &str) -> bool {
        let mut seen = self.0.lock().expect("first lock");
        if seen.get(job).is_some_and(|run| run == run_id) {
            return false;
        }
        seen.insert(job.to_owned(), run_id.to_owned());
        true
    }

    fn forget(&self, job: &str) {
        self.0.lock().expect("first lock").remove(job);
    }
}

impl BrowserAuth {
    /// The first document of the run's pages that refuses its download
    /// instead of giving the file ([`erulabo::download_refusal`]). Only the
    /// pages' own documents count: the frames inside a page (Google's
    /// sign-in, a Drive player) are not where the file comes from. Never, if
    /// the run's events end.
    async fn refusal(&self, job: &str, run: &BrowserRun) -> Failure {
        let Ok(mut documents) = run.page_documents() else {
            return std::future::pending().await;
        };
        loop {
            match documents.recv().await {
                Ok(document) => match judge(&document) {
                    Judged::Refused(failure) => {
                        eprintln!(
                            "trss-subtitles: a download of a site's check was refused: {}",
                            failure.kind.code()
                        );
                        return failure;
                    }
                    Judged::UnknownHost(host) => {
                        if self.unknown_hosts.first(job, run.run_id()) {
                            eprintln!(
                                "trss-subtitles: a page of a site's check went to a host the source does not know: {host}"
                            );
                        }
                    }
                    Judged::Nothing => {}
                },
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return std::future::pending().await,
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
        Box::pin(async move {
            self.shown.lock().expect("shown lock").remove(job);
            self.unknown_hosts.forget(job);
            self.pool.end_job(job).await
        })
    }

    fn downloading(&self, job: &str, run_id: &str) -> bool {
        self.taking.lock().expect("taking lock").contains(job)
            || self
                .live(job, run_id)
                .is_some_and(|run| run.download_active())
    }

    fn pages(&self, job: &str, run_id: &str) -> Vec<String> {
        match self.live(job, run_id).filter(|run| !run.is_ended()) {
            Some(run) => run
                .pages()
                .iter()
                .map(|page| page.target_id().to_owned())
                .collect(),
            None => Vec::new(),
        }
    }

    fn close_page<'a>(
        &'a self,
        job: &'a str,
        run_id: &'a str,
        target: &'a str,
    ) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            // The page the run was prepared with stays: the browser always
            // has a window, and the post is where the person came from.
            if self
                .shown(job, run_id)
                .is_none_or(|shown| shown.target_id == target)
            {
                return false;
            }
            let Some(run) = self.live(job, run_id).filter(|run| !run.is_ended()) else {
                return false;
            };
            let Some(page) = run.pages().into_iter().find(|p| p.target_id() == target) else {
                return false;
            };
            page.close().await.is_ok()
        })
    }

    fn rearm<'a>(&'a self, job: &'a str, run_id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(shown) = self.shown(job, run_id) else {
                return;
            };
            let AuthPage::Erulabo(check) = &shown.page else {
                return;
            };
            let Some(run) = self.live(job, run_id) else {
                return;
            };
            // A person who passed the check has a download coming (or not yet
            // taken): clicking the card then would start the check anew.
            if run.download_active() {
                return;
            }
            // One at a time: two screens opened together click once.
            if !self
                .rearming
                .lock()
                .expect("rearming lock")
                .insert(job.to_owned())
            {
                return;
            }
            if let (Ok(_busy), Some(page)) = (
                run.busy_guard(),
                run.pages()
                    .into_iter()
                    .find(|p| p.target_id() == shown.target_id),
            ) {
                // The reasons are the driver's own words, with no address.
                if let Err(failure) = erulabo::rearm(&page, &shown.post, check).await {
                    eprintln!(
                        "trss-subtitles: a site's check could not be brought back ({}): {}",
                        failure.kind.code(),
                        failure.reason
                    );
                }
            }
            self.rearming.lock().expect("rearming lock").remove(job);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_page() -> AuthPage {
        AuthPage::Fake(fake::FakeCheck {
            name: "ep1".to_owned(),
        })
    }

    #[tokio::test]
    async fn a_file_that_came_through_the_browser_is_keyed_by_its_post_and_name_only() {
        let post = Url::parse("https://fake.trss.invalid/check/ep1?x=1").unwrap();
        assert_eq!(
            file_key(&post, "ep1.srt"),
            "browser:fake.trss.invalid/check/ep1#ep1.srt"
        );
        let file = arrived(
            &post,
            &fake_page(),
            "ep1.srt",
            PathBuf::from("/area/.tmp/x/ep1.srt"),
        )
        .await;
        assert_eq!(file.name, "ep1.srt");
        assert_eq!(file.staged(), Some(Path::new("/area/.tmp/x/ep1.srt")));
        assert_eq!(file.snapshot, Snapshot::default());
    }

    #[tokio::test]
    async fn what_the_answer_said_goes_with_the_file_and_its_address_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let answered = Answered {
            status: Some(200),
            content_type: Some("application/octet-stream".to_owned()),
            content_length: Some(5),
            last_modified: Some("Fri, 02 Oct 2026 02:11:11 GMT".to_owned()),
            drive_id: Some("1AbC_d-E".to_owned()),
        };
        record_answer(dir.path(), &answered).await.unwrap();
        let path = dir.path().join("ep1.zip");
        std::fs::write(&path, b"12345").unwrap();
        assert_eq!(
            staged(dir.path()).await,
            Some(Staged::File {
                name: "ep1.zip".to_owned(),
                path: path.clone()
            })
        );

        let post = Url::parse("https://erulabo.com/859").unwrap();
        let mut page_snapshot = Snapshot::default();
        page_snapshot.push(erulabo::POST_MODIFIED, "2026-10-01T00:00:00+09:00");
        let page = AuthPage::Erulabo(erulabo::ErulaboCheck::new(
            "/file/abc".to_owned(),
            "전생귀족3 (1)".to_owned(),
            page_snapshot.clone(),
        ));
        let file = arrived(&post, &page, "ep1.zip", path).await;
        let mut expected = page_snapshot;
        expected.push(DRIVE_ID, "1AbC_d-E");
        expected.push(http::LAST_MODIFIED, "Fri, 02 Oct 2026 02:11:11 GMT");
        expected.push(drive::CONTENT_LENGTH, "5");
        assert_eq!(file.snapshot, expected);
        let fetch = crate::Source::Fake(fake::FakeSource)
            .fetch(&post, &file)
            .await
            .unwrap();
        assert_eq!(fetch.status, Some(200));
        assert_eq!(
            fetch.content_type.as_deref(),
            Some("application/octet-stream")
        );
        assert_eq!(fetch.expected_size, Some(5));
    }

    #[tokio::test]
    async fn a_refusal_kept_in_the_folder_comes_before_any_file_in_it() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(staged(dir.path()).await, None);
        std::fs::write(dir.path().join("half.zip"), b"1").unwrap();
        let refused = Failure::new(FailureKind::Expired, "받기 주소가 만료됐어요 (HTTP 403)")
            .with_response(Some(403), Some("text/html".to_owned()), None);
        record_refusal(dir.path(), &refused).await.unwrap();
        assert_eq!(staged(dir.path()).await, Some(Staged::Refused(refused)));
    }
    fn document(url: &str, status: Option<u16>, content_type: Option<&str>) -> PageDocument {
        PageDocument {
            source: trss_browser::DownloadSource::new(Url::parse(url).unwrap()),
            answer: trss_browser::DownloadAnswer {
                status,
                content_type: content_type.map(str::to_owned),
                ..Default::default()
            },
        }
    }

    #[test]
    fn a_page_document_is_a_refusal_an_unknown_host_or_nothing() {
        let kind = |d: PageDocument| match judge(&d) {
            Judged::Refused(f) => format!("refused {}", f.kind.code()),
            Judged::UnknownHost(host) => format!("unknown {host}"),
            Judged::Nothing => "nothing".to_owned(),
        };
        // Google's sign-in as a page: Drive wants the person to sign in.
        assert_eq!(
            kind(document(
                "https://accounts.google.com/v3/signin?continue=SECRET",
                Some(200),
                Some("text/html")
            )),
            "refused missing"
        );
        assert_eq!(
            kind(document(
                "https://drive.usercontent.google.com/download?id=ABCDEFGHIJKL&at=SECRET",
                Some(403),
                Some("text/html")
            )),
            "refused missing"
        );
        assert_eq!(
            kind(document(
                "https://erulabo.com/file/abc/download?signature=SECRET",
                Some(429),
                Some("text/html")
            )),
            "refused network"
        );
        // The file itself, the post, and a site the source never heard of: the
        // host alone is named, and the wait goes on.
        assert_eq!(
            kind(document(
                "https://drive.usercontent.google.com/download?id=A&at=SECRET",
                Some(200),
                Some("application/octet-stream")
            )),
            "nothing"
        );
        assert_eq!(
            kind(document(
                "https://erulabo.com/860",
                Some(200),
                Some("text/html")
            )),
            "nothing"
        );
        assert_eq!(
            kind(document(
                "https://download.example/f/SECRET?token=SECRET",
                Some(403),
                Some("text/html")
            )),
            "unknown download.example"
        );
        // An answer with no status says nothing about a refusal.
        assert_eq!(
            kind(document("https://accounts.google.com/signin", None, None)),
            "nothing"
        );
    }

    #[test]
    fn an_unknown_host_is_told_once_per_run_of_a_job() {
        let first = FirstPerRun::default();
        assert!(first.first("j", "run-1"));
        assert!(!first.first("j", "run-1"));
        assert!(first.first("other", "run-1"));
        assert!(first.first("j", "run-2"));
        first.forget("j");
        assert!(first.first("j", "run-2"));
    }

    #[test]
    fn an_answer_with_no_status_is_kept_with_none_and_never_as_zero() {
        let answered = Answered::of(&Download {
            guid: "g".to_owned(),
            file_name: "x.zip".to_owned(),
            host: None,
            source: None,
            answer: Some(trss_browser::DownloadAnswer::default()),
            state: DownloadState::Completed,
            path: PathBuf::from("/x"),
            received_bytes: 1,
        });
        assert_eq!(answered.status, None);
        let again = Answered::from_json(&answered.to_json());
        assert_eq!(again.status, None);
        assert_eq!(again, answered);
    }
}
