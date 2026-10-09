//! One run of one job: its handle, its activity, its pages and its downloads.

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};

use serde_json::{json, Value};
use tokio::sync::{broadcast, Notify};
use tokio_util::sync::CancellationToken;
use trss_core::Millis;

use super::{files, BrowserError, MovedFile, PoolInner};
use crate::cdp::{CdpError, Connection, Event};

/// How long a new page has to be ready for use.
const PAGE_READY_TIMEOUT: Duration = Duration::from_secs(15);

/// The most one download may be: a file longer than this is canceled while it
/// comes (Tistory's and Naver's downloads have the same limit).
pub const MAX_DOWNLOAD_BYTES: u64 = trss_core::response::MAX_DOWNLOAD_FILE_BYTES;
/// The most the downloads of one run may come to, those under way included.
/// Downloads share the disk of the app's database, so a page that keeps
/// offering files cannot fill it.
pub const MAX_RUN_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
/// How long a download may go without a report of progress before it is
/// canceled. Chromium reports as bytes arrive, so a stalled one is silent.
pub const DOWNLOAD_STALL: Duration = Duration::from_secs(120);

/// What the pool tracks of one run.
pub(crate) struct RunInner {
    pub(crate) job_id: String,
    pub(crate) run_id: String,
    /// The run's downloads folder at the path the worker sees it.
    pub(crate) downloads_dir: PathBuf,
    started_at: Millis,
    /// Cancelled when no operation may go on.
    ended: CancellationToken,
    /// Cancelled when the run is prepared for use.
    ready: CancellationToken,
    /// Set when the launcher confirmed the run gone.
    removed: AtomicBool,
    /// One ending at a time.
    pub(crate) end_lock: tokio::sync::Mutex<()>,
    pub(crate) conn: OnceLock<Connection>,
    /// Where the browser saves the run's downloads, as it sees the folder.
    pub(crate) container_downloads: OnceLock<String>,
    activity: Mutex<Activity>,
    targets: Mutex<HashMap<String, TargetEntry>>,
    targets_changed: Notify,
    downloads: Mutex<DownloadTable>,
    downloads_changed: Notify,
    /// By frame: the last answer of a document the frame loaded, with its
    /// address (in memory only). A download is the answer of a navigation
    /// the browser turned into one, so its facts are there.
    documents: Mutex<HashMap<String, (String, DownloadAnswer)>>,
    /// The documents the run's pages (not the frames inside them) were
    /// answered with, in the order the events came ([`BrowserRun::page_documents`]).
    page_documents: broadcast::Sender<PageDocument>,
}

/// The most frames whose last document answer is kept; past it they are
/// forgotten all at once (a run has a handful of pages and frames).
const MAX_DOCUMENTS: usize = 64;

/// How many answers of page documents a slow reader of
/// [`BrowserRun::page_documents`] may fall behind by before it loses the
/// oldest.
const PAGE_DOCUMENTS_KEPT: usize = 32;

struct Activity {
    /// Job steps that said they are running.
    busy: u32,
    /// The later of the last use of the screen, the end of the last busy
    /// stretch or download, and the start.
    last: Millis,
}

#[derive(Debug, Clone)]
struct TargetEntry {
    session_id: String,
    kind: String,
    ready: bool,
}

#[derive(Default)]
struct DownloadTable {
    /// By guid: the ones under way.
    in_progress: HashMap<String, DownloadItem>,
    finished: VecDeque<Download>,
    /// The bytes of the downloads that ended.
    finished_bytes: u64,
}

struct DownloadItem {
    file_name: String,
    host: Option<String>,
    source: Option<DownloadSource>,
    answer: Option<DownloadAnswer>,
    received: u64,
    /// When the download began or last reported progress.
    last_progress: Millis,
    /// Asked to be canceled (over a limit, or stalled): it no longer keeps
    /// the run busy, and its end, if it ever comes, is reported as usual.
    cancelling: bool,
}

impl DownloadTable {
    fn busy(&self) -> bool {
        self.in_progress.values().any(|d| !d.cancelling)
    }
}

impl RunInner {
    pub(crate) fn new(job_id: &str, run_id: &str, downloads_dir: PathBuf, now: Millis) -> RunInner {
        RunInner {
            job_id: job_id.to_owned(),
            run_id: run_id.to_owned(),
            downloads_dir,
            started_at: now,
            ended: CancellationToken::new(),
            ready: CancellationToken::new(),
            removed: AtomicBool::new(false),
            end_lock: tokio::sync::Mutex::new(()),
            conn: OnceLock::new(),
            container_downloads: OnceLock::new(),
            activity: Mutex::new(Activity { busy: 0, last: now }),
            targets: Mutex::default(),
            targets_changed: Notify::new(),
            downloads: Mutex::default(),
            downloads_changed: Notify::new(),
            documents: Mutex::default(),
            page_documents: broadcast::channel(PAGE_DOCUMENTS_KEPT).0,
        }
    }

    pub(crate) fn is_ended(&self) -> bool {
        self.ended.is_cancelled()
    }

    pub(crate) fn mark_ended(&self) {
        self.ended.cancel();
        if let Some(conn) = self.conn.get() {
            conn.close();
        }
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.ready.is_cancelled()
    }

    pub(crate) fn mark_ready(&self) {
        self.ready.cancel();
    }

    pub(crate) fn is_removed(&self) -> bool {
        self.removed.load(Ordering::Acquire)
    }

    pub(crate) fn mark_removed(&self) {
        self.removed.store(true, Ordering::Release);
    }

    pub(crate) fn is_busy(&self) -> bool {
        let busy = self.activity.lock().expect("activity lock").busy > 0;
        busy || self.downloads.lock().expect("downloads lock").busy()
    }

    /// Counts a job step as running, unless the run has ended. Decided under
    /// the lock [`RunInner::end_if_idle`] decides under, so a step is either
    /// counted before an idle end looks, or sees the run ended.
    pub(crate) fn try_busy(&self) -> bool {
        let mut activity = self.activity.lock().expect("activity lock");
        if self.is_ended() {
            return false;
        }
        activity.busy += 1;
        true
    }

    /// Counts a use of the run at `now`, unless it has ended (same lock as
    /// [`RunInner::try_busy`]).
    pub(crate) fn touch_live(&self, now: Millis) -> bool {
        let mut activity = self.activity.lock().expect("activity lock");
        if self.is_ended() {
            return false;
        }
        activity.last = activity.last.max(now);
        true
    }

    /// Ends the run if it is ready, not busy and idle for `idle_ms`, and says
    /// whether it did. The look and the end are one step under the activity
    /// lock: a step that starts meanwhile is refused as ended or is seen as
    /// busy, never both accepted and ended.
    pub(crate) fn end_if_idle(&self, now: Millis, idle_ms: i64) -> bool {
        let activity = self.activity.lock().expect("activity lock");
        let busy = activity.busy > 0 || self.downloads.lock().expect("downloads lock").busy();
        if self.is_ended() || !self.is_ready() || busy || now - activity.last < idle_ms {
            return false;
        }
        self.mark_ended();
        true
    }

    pub(crate) fn last_activity(&self) -> Millis {
        self.activity.lock().expect("activity lock").last
    }

    fn touch_at(&self, now: Millis) {
        let mut activity = self.activity.lock().expect("activity lock");
        activity.last = activity.last.max(now);
    }

    pub(crate) fn status(&self) -> RunStatus {
        RunStatus {
            job_id: self.job_id.clone(),
            run_id: self.run_id.clone(),
            started_at: self.started_at,
            ready: self.is_ready(),
            ended: self.is_ended(),
            busy: self.is_busy(),
            last_activity: self.last_activity(),
            downloads_in_progress: self
                .downloads
                .lock()
                .expect("downloads lock")
                .in_progress
                .values()
                .filter(|d| !d.cancelling)
                .count(),
        }
    }

    // --- targets (pages and frames the connection is attached to) ---

    pub(crate) fn target_attached(&self, target_id: &str, session_id: &str, kind: &str) {
        self.targets.lock().expect("targets lock").insert(
            target_id.to_owned(),
            TargetEntry {
                session_id: session_id.to_owned(),
                kind: kind.to_owned(),
                ready: false,
            },
        );
    }

    pub(crate) fn target_ready(&self, target_id: &str, session_id: &str) {
        if let Some(target) = self
            .targets
            .lock()
            .expect("targets lock")
            .get_mut(target_id)
        {
            if target.session_id == session_id {
                target.ready = true;
            }
        }
        self.targets_changed.notify_waiters();
    }

    pub(crate) fn session_detached(&self, session_id: &str) {
        self.targets
            .lock()
            .expect("targets lock")
            .retain(|_, t| t.session_id != session_id);
        self.targets_changed.notify_waiters();
    }

    pub(crate) fn target_destroyed(&self, target_id: &str) {
        self.targets.lock().expect("targets lock").remove(target_id);
        self.targets_changed.notify_waiters();
    }

    /// Whether `target_id` is a page of the run (a window or a popup), not a
    /// frame of another site inside one. A page's main frame has its target's
    /// ID as its frame ID.
    fn is_page_target(&self, target_id: &str) -> bool {
        self.targets
            .lock()
            .expect("targets lock")
            .get(target_id)
            .is_some_and(|t| t.kind == "page")
    }

    // --- downloads ---

    pub(crate) fn download_active(&self) -> bool {
        let table = self.downloads.lock().expect("downloads lock");
        table.busy() || !table.finished.is_empty()
    }

    /// A `Network.responseReceived` of the run's connection: the answer of a
    /// document is kept for the download it may become, and told to the
    /// readers of [`BrowserRun::page_documents`] when it is a page's own.
    /// Decided here, in the order the events came, so a popup's first
    /// document is never taken for a frame's.
    pub(crate) fn response_received(&self, params: &Value) {
        if params["type"] != "Document" {
            return;
        }
        let (Some(frame), Some(url)) = (
            params["frameId"].as_str(),
            params["response"]["url"].as_str(),
        ) else {
            return;
        };
        let answer = DownloadAnswer::of_response(&params["response"]);
        if self.is_page_target(frame) {
            if let Ok(source) = url::Url::parse(url) {
                // No reader is no error.
                let _ = self.page_documents.send(PageDocument {
                    source: DownloadSource::new(source),
                    answer: answer.clone(),
                });
            }
        }
        self.document_answered(frame, url, answer);
    }

    /// The answer of the document frame `frame_id` loaded from `url`.
    pub(crate) fn document_answered(&self, frame_id: &str, url: &str, answer: DownloadAnswer) {
        let mut documents = self.documents.lock().expect("documents lock");
        if documents.len() >= MAX_DOCUMENTS && !documents.contains_key(frame_id) {
            documents.clear();
        }
        documents.insert(frame_id.to_owned(), (url.to_owned(), answer));
    }

    /// A download began from `url` in frame `frame_id` (when the browser
    /// said which): the frame's last document answer is the download's when
    /// it was answered from the same address.
    pub(crate) fn download_began(
        &self,
        guid: &str,
        url: &str,
        frame_id: Option<&str>,
        suggested: &str,
        now: Millis,
    ) {
        let parsed = url::Url::parse(url).ok();
        let host = parsed
            .as_ref()
            .and_then(|u| u.host_str().map(str::to_owned));
        let answer = frame_id.and_then(|frame| {
            let documents = self.documents.lock().expect("documents lock");
            documents
                .get(frame)
                .filter(|(answered, _)| answered == url)
                .map(|(_, answer)| answer.clone())
        });
        // The address can carry a signature: only the host is ever logged.
        println!(
            "Browser: a download began in the run of job {} from {}",
            self.job_id,
            host.as_deref().unwrap_or("an unknown host")
        );
        self.downloads
            .lock()
            .expect("downloads lock")
            .in_progress
            .insert(
                guid.to_owned(),
                DownloadItem {
                    file_name: suggested.to_owned(),
                    host,
                    source: parsed.map(DownloadSource),
                    answer,
                    received: 0,
                    last_progress: now,
                    cancelling: false,
                },
            );
    }

    /// A report of the browser on download `guid`. Says whether the download
    /// is to be canceled, for being over a size limit.
    pub(crate) fn download_progress(
        &self,
        guid: &str,
        state: &str,
        received: u64,
        now: Millis,
    ) -> bool {
        let state = match state {
            "inProgress" => return self.download_grew(guid, received, now),
            "completed" => DownloadState::Completed,
            "canceled" => DownloadState::Canceled,
            _ => return false,
        };
        // A file over the limit that came whole is not kept.
        let state = if state == DownloadState::Completed && received > MAX_DOWNLOAD_BYTES {
            DownloadState::Canceled
        } else {
            state
        };
        {
            let mut table = self.downloads.lock().expect("downloads lock");
            let item = table.in_progress.remove(guid);
            let (file_name, host, source, answer) = item.map_or_else(
                || (String::new(), None, None, None),
                |i| (i.file_name, i.host, i.source, i.answer),
            );
            table.finished_bytes = table.finished_bytes.saturating_add(received);
            table.finished.push_back(Download {
                guid: guid.to_owned(),
                file_name,
                host,
                source,
                answer,
                state,
                path: self.downloads_dir.join(guid),
                received_bytes: received,
            });
        }
        // The end of a download is activity: the idle time counts from here.
        self.touch_at(now);
        self.downloads_changed.notify_waiters();
        false
    }

    fn download_grew(&self, guid: &str, received: u64, now: Millis) -> bool {
        let mut table = self.downloads.lock().expect("downloads lock");
        let finished = table.finished_bytes;
        let Some(item) = table.in_progress.get_mut(guid) else {
            return false;
        };
        item.received = received;
        item.last_progress = now;
        if item.cancelling {
            return false;
        }
        let under_way: u64 = table
            .in_progress
            .values()
            .filter(|d| !d.cancelling)
            .map(|d| d.received)
            .fold(0, u64::saturating_add);
        let over = received > MAX_DOWNLOAD_BYTES
            || finished.saturating_add(under_way) > MAX_RUN_DOWNLOAD_BYTES;
        if over {
            if let Some(item) = table.in_progress.get_mut(guid) {
                item.cancelling = true;
            }
        }
        over
    }

    /// The downloads under way that have reported nothing for `stall`, now
    /// marked as being canceled. Asking the browser to cancel them is the
    /// caller's. The idle time counts from here, as from any end.
    pub(crate) fn take_stalled_downloads(&self, now: Millis, stall: Duration) -> Vec<String> {
        let stall_ms = i64::try_from(stall.as_millis()).unwrap_or(i64::MAX);
        let stalled: Vec<String> = {
            let mut table = self.downloads.lock().expect("downloads lock");
            table
                .in_progress
                .iter_mut()
                .filter(|(_, d)| !d.cancelling && now - d.last_progress >= stall_ms)
                .map(|(guid, d)| {
                    d.cancelling = true;
                    guid.clone()
                })
                .collect()
        };
        if !stalled.is_empty() {
            self.touch_at(now);
        }
        stalled
    }
}

/// A run as the pool reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStatus {
    pub job_id: String,
    pub run_id: String,
    /// Unix milliseconds.
    pub started_at: Millis,
    /// Prepared for use: false while it is being started.
    pub ready: bool,
    pub ended: bool,
    pub busy: bool,
    /// Unix milliseconds; the idle time counts from here.
    pub last_activity: Millis,
    pub downloads_in_progress: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadState {
    Completed,
    Canceled,
}

/// A download that has ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub guid: String,
    /// The name the site suggested; the file itself is named by `guid`.
    pub file_name: String,
    /// The host it came from (never the address, which can be signed).
    pub host: Option<String>,
    /// The address it came from, in memory only ([`DownloadSource`]).
    pub source: Option<DownloadSource>,
    /// What the answer that became the download said, when the browser
    /// reported it for the frame the download began in.
    pub answer: Option<DownloadAnswer>,
    pub state: DownloadState,
    /// Where the file is, at the path the worker sees.
    pub path: PathBuf,
    pub received_bytes: u64,
}

/// The address a download came from. It can be signed, so it stays in
/// memory: its `Debug` hides it, and nothing here logs or stores it. A reader
/// takes from it what is not secret (a Google Drive file's ID).
#[derive(Clone, PartialEq, Eq)]
pub struct DownloadSource(url::Url);

impl DownloadSource {
    pub fn new(url: url::Url) -> DownloadSource {
        DownloadSource(url)
    }

    pub fn url(&self) -> &url::Url {
        &self.0
    }
}

impl std::fmt::Debug for DownloadSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DownloadSource(<hidden>)")
    }
}

/// What the answer a download came of said about the file: the facts of a
/// `Network.responseReceived` of the document that became the download.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadAnswer {
    /// The HTTP status, when the answer said one.
    pub status: Option<u16>,
    /// The media type, without its parameters.
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    /// `Last-Modified`, as the answer wrote it.
    pub last_modified: Option<String>,
}

impl DownloadAnswer {
    /// The facts of a DevTools `Response` object.
    pub fn of_response(response: &Value) -> DownloadAnswer {
        let header = |name: &str| {
            response["headers"].as_object().and_then(|headers| {
                headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(name))
                    .and_then(|(_, v)| v.as_str())
                    .map(|v| v.trim().to_owned())
            })
        };
        let media = |v: &str| {
            let media = v
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            (!media.is_empty()).then_some(media)
        };
        DownloadAnswer {
            status: response["status"]
                .as_u64()
                .and_then(|s| u16::try_from(s).ok()),
            content_type: header("content-type")
                .as_deref()
                .and_then(media)
                .or_else(|| response["mimeType"].as_str().and_then(media)),
            content_length: header("content-length").and_then(|v| v.parse().ok()),
            last_modified: header("last-modified").filter(|v| !v.is_empty() && v.len() <= 64),
        }
    }
}

/// A document a page of the run (its main frame; not a frame of another site
/// inside it) was answered with. The address stays in memory like a
/// download's ([`DownloadSource`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageDocument {
    pub source: DownloadSource,
    pub answer: DownloadAnswer,
}

/// The handle of one run of one job. Cheap to clone.
#[derive(Clone)]
pub struct BrowserRun {
    pool: Arc<PoolInner>,
    entry: Arc<RunInner>,
}

impl std::fmt::Debug for BrowserRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserRun")
            .field("job_id", &self.entry.job_id)
            .field("run_id", &self.entry.run_id)
            .field("ended", &self.entry.is_ended())
            .finish()
    }
}

/// Keeps a run busy until dropped.
pub struct BusyGuard {
    run: BrowserRun,
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.run.mark_idle();
    }
}

impl BrowserRun {
    pub(crate) fn new(pool: Arc<PoolInner>, entry: Arc<RunInner>) -> BrowserRun {
        BrowserRun { pool, entry }
    }

    pub fn job_id(&self) -> &str {
        &self.entry.job_id
    }

    pub fn run_id(&self) -> &str {
        &self.entry.run_id
    }

    /// The run's downloads folder, at the path the worker sees.
    pub fn downloads_dir(&self) -> &Path {
        &self.entry.downloads_dir
    }

    pub fn is_ended(&self) -> bool {
        self.entry.is_ended()
    }

    /// Resolves when the run has ended (at once for one that has). A task that
    /// serves a page of the run for as long as it lives waits on this: the
    /// run's events go on being sent while any handle holds the run.
    pub async fn ended(&self) {
        self.entry.ended.cancelled().await
    }

    /// Reads the run's state. Not activity.
    pub fn status(&self) -> RunStatus {
        self.entry.status()
    }

    fn ended_error(&self) -> BrowserError {
        BrowserError::RunEnded {
            run: self.entry.run_id.clone(),
        }
    }

    fn check(&self) -> Result<(), BrowserError> {
        if self.entry.is_ended() {
            Err(self.ended_error())
        } else {
            Ok(())
        }
    }

    // --- activity ---

    /// A person used the screen: the idle time counts from now. Decided under
    /// the lock an idle end decides under ([`RunInner::touch_live`]), so the
    /// use is either counted before the reaper looks, or refused as ended;
    /// never counted on a run the reaper ends meanwhile.
    pub fn touch(&self) -> Result<(), BrowserError> {
        if self.entry.touch_live(self.pool.now()) {
            Ok(())
        } else {
            Err(self.ended_error())
        }
    }

    /// A job step is running on the run: it does not end for being idle until
    /// the matching [`BrowserRun::mark_idle`]. Steps may overlap.
    pub fn mark_busy(&self) -> Result<(), BrowserError> {
        if self.entry.try_busy() {
            Ok(())
        } else {
            Err(self.ended_error())
        }
    }

    /// A job step on the run ended. The idle time counts from now once no
    /// step and no download is left. Nothing happens on an ended run.
    pub fn mark_idle(&self) {
        let mut activity = self.entry.activity.lock().expect("activity lock");
        activity.busy = activity.busy.saturating_sub(1);
        if activity.busy == 0 {
            activity.last = activity.last.max(self.pool.now());
        }
    }

    /// [`BrowserRun::mark_busy`] until the guard is dropped.
    pub fn busy_guard(&self) -> Result<BusyGuard, BrowserError> {
        self.mark_busy()?;
        Ok(BusyGuard { run: self.clone() })
    }

    // --- DevTools ---

    async fn send(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, BrowserError> {
        self.check()?;
        let conn = self.entry.conn.get().ok_or_else(|| self.ended_error())?;
        let answer = tokio::select! {
            answer = conn.command(session, method, params) => answer,
            _ = self.entry.ended.cancelled() => return Err(self.ended_error()),
        };
        answer.map_err(|err| match err {
            CdpError::Closed => self.ended_error(),
            other => BrowserError::Cdp(other),
        })
    }

    /// A browser-level DevTools command.
    pub async fn command(&self, method: &str, params: Value) -> Result<Value, BrowserError> {
        self.send(None, method, params).await
    }

    /// The events of the browser and all its sessions from now on. A reader
    /// that falls far behind loses the oldest ones.
    pub fn events(&self) -> Result<broadcast::Receiver<Event>, BrowserError> {
        self.check()?;
        self.entry
            .conn
            .get()
            .map(Connection::events)
            .ok_or_else(|| self.ended_error())
    }

    /// The documents the run's pages are answered with from now on, in the
    /// order the browser said them: those of a window or a popup, not of the
    /// frames inside them (a sign-in or a player another site shows in a
    /// page is no navigation of the page). A reader that falls far behind
    /// loses the oldest ones.
    pub fn page_documents(&self) -> Result<broadcast::Receiver<PageDocument>, BrowserError> {
        self.check()?;
        Ok(self.entry.page_documents.subscribe())
    }

    /// Opens a page and opens `url` in it. The page's set-up (ads blocked)
    /// is sent before it is let go, so it comes before anything the page
    /// asks for over the same session.
    pub async fn new_page(&self, url: &str) -> Result<Page, BrowserError> {
        let created = self
            .command("Target.createTarget", json!({ "url": "about:blank" }))
            .await?;
        let target_id = created["targetId"]
            .as_str()
            .ok_or_else(|| BrowserError::Internal("createTarget gave no targetId".into()))?
            .to_owned();
        self.wait_for_target(&target_id).await?;
        let page = Page {
            run: self.clone(),
            target_id,
        };
        if url != "about:blank" {
            page.navigate(url).await?;
        }
        Ok(page)
    }

    /// The pages open now (those the connection is attached to and has let
    /// go, their set-up sent). Looks only.
    pub fn pages(&self) -> Vec<Page> {
        let targets = self.entry.targets.lock().expect("targets lock");
        let mut ids: Vec<&String> = targets
            .iter()
            .filter(|(_, t)| t.kind == "page" && t.ready)
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        ids.into_iter()
            .map(|id| Page {
                run: self.clone(),
                target_id: id.clone(),
            })
            .collect()
    }

    async fn wait_for_target(&self, target_id: &str) -> Result<(), BrowserError> {
        let deadline = tokio::time::Instant::now() + PAGE_READY_TIMEOUT;
        loop {
            let changed = self.entry.targets_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            self.check()?;
            let ready = self
                .entry
                .targets
                .lock()
                .expect("targets lock")
                .get(target_id)
                .is_some_and(|t| t.ready);
            if ready {
                return Ok(());
            }
            tokio::select! {
                _ = &mut changed => {}
                _ = self.entry.ended.cancelled() => return Err(self.ended_error()),
                _ = tokio::time::sleep_until(deadline) => return Err(BrowserError::Timeout("the new page to be set up")),
            }
        }
    }

    fn session_of(&self, target_id: &str) -> Result<String, BrowserError> {
        self.entry
            .targets
            .lock()
            .expect("targets lock")
            .get(target_id)
            .filter(|t| t.ready)
            .map(|t| t.session_id.clone())
            .ok_or_else(|| BrowserError::PageGone(target_id.to_owned()))
    }

    // --- downloads ---

    /// Waits for the next download of the run that ended (completed or
    /// canceled) and has not been handed out. Give up by dropping the future
    /// (or wrap it in a timeout).
    pub async fn download_finished(&self) -> Result<Download, BrowserError> {
        loop {
            let changed = self.entry.downloads_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            self.check()?;
            let next = self
                .entry
                .downloads
                .lock()
                .expect("downloads lock")
                .finished
                .pop_front();
            if let Some(download) = next {
                return Ok(download);
            }
            tokio::select! {
                _ = &mut changed => {}
                _ = self.entry.ended.cancelled() => return Err(self.ended_error()),
            }
        }
    }

    /// Whether a download of the run is under way, or ended and not yet
    /// handed out by [`BrowserRun::download_finished`]. Looks only.
    pub fn download_active(&self) -> bool {
        self.entry.download_active()
    }

    /// Moves a completed download into `dir` (made if missing), named by the
    /// name the site suggested, made safe ([`files::safe_file_name`]). A file
    /// of that name in `dir` is never replaced
    /// ([`BrowserError::AlreadyExists`]), and the download then stays where it
    /// is.
    pub async fn move_download(
        &self,
        download: &Download,
        dir: &Path,
    ) -> Result<MovedFile, BrowserError> {
        self.check()?;
        if download.state != DownloadState::Completed {
            return Err(BrowserError::DownloadNotCompleted(download.guid.clone()));
        }
        // Only a file this run saved. The guid is a name, not a path, and what
        // is in the folder is the browser's to write: `move_into` takes the
        // file only if it is a plain file of that name.
        if download.path != self.entry.downloads_dir.join(&download.guid) {
            return Err(BrowserError::Internal(
                "the download is not of this run".into(),
            ));
        }
        let name = files::safe_file_name(&download.file_name);
        let (run_dir, guid, dir) = (
            self.entry.downloads_dir.clone(),
            download.guid.clone(),
            dir.to_owned(),
        );
        tokio::task::spawn_blocking(move || files::move_into(&run_dir, &guid, &dir, &name))
            .await
            .map_err(|e| BrowserError::Internal(e.to_string()))?
    }

    /// Ends the run. Its handles answer [`BrowserError::RunEnded`] from now
    /// on, here and everywhere.
    pub async fn end(&self) {
        self.pool.end_entry(&self.entry).await;
    }
}

/// A page of a run.
#[derive(Clone)]
pub struct Page {
    run: BrowserRun,
    target_id: String,
}

impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Page")
            .field("run_id", &self.run.entry.run_id)
            .field("target_id", &self.target_id)
            .finish()
    }
}

impl Page {
    pub fn target_id(&self) -> &str {
        &self.target_id
    }

    pub fn run(&self) -> &BrowserRun {
        &self.run
    }

    /// The DevTools session of the page.
    pub fn session_id(&self) -> Result<String, BrowserError> {
        self.run.check()?;
        self.run.session_of(&self.target_id)
    }

    /// A DevTools command in the page's session.
    pub async fn send(&self, method: &str, params: Value) -> Result<Value, BrowserError> {
        let session = self.session_id()?;
        self.run.send(Some(&session), method, params).await
    }

    /// Starts opening `url`; does not wait for the load. A page that cannot
    /// be reached is an error (its text has no address).
    pub async fn navigate(&self, url: &str) -> Result<(), BrowserError> {
        let answer = self.send("Page.navigate", json!({ "url": url })).await?;
        match answer.get("errorText").and_then(Value::as_str) {
            Some(text) => Err(BrowserError::Cdp(CdpError::Command {
                method: "Page.navigate".to_owned(),
                code: 0,
                message: text.to_owned(),
            })),
            None => Ok(()),
        }
    }

    /// Evaluates a JavaScript expression in the page (waiting for a promise)
    /// and gives its value.
    pub async fn evaluate(&self, expression: &str) -> Result<Value, BrowserError> {
        let mut answer = self
            .send(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
            )
            .await?;
        if let Some(details) = answer.get("exceptionDetails") {
            return Err(BrowserError::Cdp(CdpError::Command {
                method: "Runtime.evaluate".to_owned(),
                code: 0,
                message: details
                    .pointer("/exception/description")
                    .or_else(|| details.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("the script threw")
                    .to_owned(),
            }));
        }
        Ok(answer["result"]["value"].take())
    }

    pub async fn close(&self) -> Result<(), BrowserError> {
        self.run
            .command("Target.closeTarget", json!({ "targetId": self.target_id }))
            .await
            .map(|_| ())
    }

    /// Answers every JavaScript dialog (`alert`, `confirm`, `prompt`) the page
    /// opens from now on by dismissing it, as a person who cancels does, so a
    /// page script that waits on one never blocks the run. What was opened is
    /// kept in the returned [`Dialogs`], which stops answering when dropped.
    /// A watch that falls behind the browser's events ends the run, since a
    /// dialog it missed would hold the page.
    pub async fn dismiss_dialogs(&self) -> Result<Dialogs, BrowserError> {
        let session = self.session_id()?;
        let mut events = self.run.events()?;
        self.send("Page.enable", json!({})).await?;
        let seen: Arc<Mutex<Vec<DialogSeen>>> = Arc::default();
        let task = tokio::spawn({
            let (page, seen) = (self.clone(), seen.clone());
            async move {
                loop {
                    match events.recv().await {
                        Ok(event)
                            if event.method == "Page.javascriptDialogOpening"
                                && event.session_id.as_deref() == Some(session.as_str()) =>
                        {
                            let text = |key: &str| {
                                event.params[key]
                                    .as_str()
                                    .unwrap_or_default()
                                    .chars()
                                    .take(200)
                                    .collect::<String>()
                            };
                            seen.lock().expect("dialogs lock").push(DialogSeen {
                                kind: text("type"),
                                message: text("message"),
                            });
                            let _ = page
                                .send("Page.handleJavaScriptDialog", json!({ "accept": false }))
                                .await;
                        }
                        Ok(_) => {}
                        // A dialog may have been among what was missed, and a
                        // page that waits on it never goes on: as when the
                        // driver falls behind, the run ends and the job asks
                        // for a new one.
                        Err(broadcast::error::RecvError::Lagged(missed)) => {
                            eprintln!(
                                "Browser: ending the run of job {}: its dialog watch fell behind by {missed} events of the browser",
                                page.run.entry.job_id
                            );
                            page.run.end().await;
                            break;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        });
        Ok(Dialogs { seen, task })
    }
}

/// A dialog a page opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogSeen {
    /// `alert`, `confirm`, `prompt` or `beforeunload`.
    pub kind: String,
    /// Its text, cut at 200 characters.
    pub message: String,
}

/// The dialogs a page opened since [`Page::dismiss_dialogs`]; dropping it stops
/// the dismissing.
pub struct Dialogs {
    seen: Arc<Mutex<Vec<DialogSeen>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Dialogs {
    /// The dialogs opened so far, in order.
    pub fn seen(&self) -> Vec<DialogSeen> {
        self.seen.lock().expect("dialogs lock").clone()
    }
}

impl Drop for Dialogs {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;
    const IDLE_MS: i64 = 300_000;

    fn ready_run(now: Millis) -> RunInner {
        let run = RunInner::new("job", "job-1", PathBuf::from("/downloads/job-1"), now);
        run.mark_ready();
        run
    }

    #[test]
    fn an_idle_end_waits_for_a_step_and_a_step_cannot_join_an_ended_run() {
        let run = ready_run(0);
        // Not idle long enough.
        assert!(!run.end_if_idle(IDLE_MS - 1, IDLE_MS));
        // A step is running: not idle however long.
        assert!(run.try_busy());
        assert!(!run.end_if_idle(10 * IDLE_MS, IDLE_MS));
        assert!(!run.is_ended());
        // Idle: ended, and then no step is taken.
        let run = ready_run(0);
        assert!(run.end_if_idle(IDLE_MS, IDLE_MS));
        assert!(run.is_ended());
        assert!(!run.try_busy());
        assert!(!run.touch_live(IDLE_MS));
        // Ending twice is not ending again.
        assert!(!run.end_if_idle(2 * IDLE_MS, IDLE_MS));
        // A run that is not ready is not ended for being idle.
        let starting = RunInner::new("job", "job-2", PathBuf::new(), 0);
        assert!(!starting.end_if_idle(10 * IDLE_MS, IDLE_MS));
    }

    #[test]
    fn a_step_and_an_idle_end_are_never_both_accepted() {
        for _ in 0..2000 {
            let run = Arc::new(ready_run(0));
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let ending = std::thread::spawn({
                let (run, barrier) = (run.clone(), barrier.clone());
                move || {
                    barrier.wait();
                    run.end_if_idle(IDLE_MS, IDLE_MS)
                }
            });
            barrier.wait();
            let stepped = run.try_busy();
            let ended = ending.join().unwrap();
            assert!(
                stepped != ended,
                "the step was accepted ({stepped}) on a run that was ended ({ended})"
            );
        }
    }

    #[test]
    fn touching_a_run_that_is_there_moves_its_idle_time_and_an_ended_one_refuses() {
        let run = ready_run(0);
        assert!(run.touch_live(200_000));
        assert!(!run.end_if_idle(IDLE_MS, IDLE_MS));
        assert!(run.end_if_idle(500_000, IDLE_MS));
        assert!(!run.touch_live(600_000));
    }

    fn began(run: &RunInner, guid: &str, now: Millis) {
        run.download_began(guid, "https://h.example/f?sig=SECRET", None, "f.zip", now);
    }

    #[test]
    fn a_download_over_the_size_limit_is_canceled_and_stops_keeping_the_run_busy() {
        let run = ready_run(0);
        began(&run, "g", 0);
        assert!(run.is_busy());
        assert!(!run.download_progress("g", "inProgress", MAX_DOWNLOAD_BYTES, 1_000));
        assert!(run.is_busy());
        assert!(run.download_progress("g", "inProgress", MAX_DOWNLOAD_BYTES + 1, 2_000));
        assert!(
            !run.is_busy(),
            "a download being canceled keeps the run busy"
        );
        assert_eq!(run.status().downloads_in_progress, 0);
        // Asked once.
        assert!(!run.download_progress("g", "inProgress", MAX_DOWNLOAD_BYTES + 9, 3_000));
        // When the browser reports the cancellation it is reported as usual.
        assert!(!run.download_progress("g", "canceled", MAX_DOWNLOAD_BYTES + 9, 4_000));
        let ended = run.downloads.lock().unwrap().finished.pop_front().unwrap();
        assert_eq!(ended.state, DownloadState::Canceled);
        assert_eq!(ended.file_name, "f.zip");
    }

    #[test]
    fn a_file_over_the_limit_that_arrived_whole_is_not_kept() {
        let run = ready_run(0);
        began(&run, "g", 0);
        run.download_progress("g", "completed", MAX_DOWNLOAD_BYTES + 1, 1_000);
        let ended = run.downloads.lock().unwrap().finished.pop_front().unwrap();
        assert_eq!(ended.state, DownloadState::Canceled);
        // And one at the limit is.
        began(&run, "h", 0);
        run.download_progress("h", "completed", MAX_DOWNLOAD_BYTES, 1_000);
        let ended = run.downloads.lock().unwrap().finished.pop_front().unwrap();
        assert_eq!(ended.state, DownloadState::Completed);
    }

    #[test]
    fn the_downloads_of_a_run_together_have_a_limit() {
        let run = ready_run(0);
        // Five files just under the limit of one: 995 MiB.
        for n in 0..5 {
            let guid = format!("g{n}");
            began(&run, &guid, 0);
            assert!(!run.download_progress(&guid, "completed", 199 * MIB, 1_000));
        }
        // 29 MiB more is fine (1024 MiB), 30 MiB more is not.
        began(&run, "more", 0);
        assert!(!run.download_progress("more", "inProgress", 29 * MIB, 2_000));
        assert!(run.download_progress("more", "inProgress", 30 * MIB, 3_000));
        assert!(!run.is_busy());
    }

    #[test]
    fn a_download_that_reports_nothing_is_canceled_after_the_stall_time() {
        let run = ready_run(0);
        let stall = Duration::from_secs(120);
        began(&run, "quiet", 0);
        began(&run, "moving", 0);
        run.download_progress("moving", "inProgress", MIB, 100_000);

        assert!(run.take_stalled_downloads(119_000, stall).is_empty());
        assert_eq!(run.take_stalled_downloads(120_000, stall), ["quiet"]);
        // The other one is still going, and the stalled one is asked once.
        assert!(run.is_busy());
        assert_eq!(run.status().downloads_in_progress, 1);
        assert!(run.take_stalled_downloads(121_000, stall).is_empty());
        // The idle time counts from the cancellation.
        assert_eq!(run.last_activity(), 120_000);
        // When the last one stops reporting, the run is not busy any more.
        assert_eq!(run.take_stalled_downloads(300_000, stall), ["moving"]);
        assert!(!run.is_busy());
    }

    #[test]
    fn a_download_takes_the_answer_of_the_document_its_frame_loaded_from_its_address() {
        let run = ready_run(0);
        let url = "https://drive.usercontent.google.com/download?id=ABCDEFGHIJKL&export=download&at=SECRET";
        let response = json!({
            "url": url, "status": 200, "mimeType": "application/octet-stream",
            "headers": {
                "content-type": "application/octet-stream",
                "Content-Length": "14245",
                "last-modified": "Fri, 02 Oct 2026 02:11:11 GMT",
            },
        });
        run.document_answered("F1", url, DownloadAnswer::of_response(&response));
        // Another frame's document is not this download's.
        run.document_answered("F2", "https://erulabo.com/859", DownloadAnswer::default());
        run.download_began("g", url, Some("F1"), "x.zip", 0);
        // A download of another address in the same frame has no answer.
        run.download_began("h", "https://elsewhere.example/x", Some("F1"), "y.zip", 0);
        run.download_progress("g", "completed", 14_245, 1);
        run.download_progress("h", "completed", 1, 1);
        let mut table = run.downloads.lock().unwrap();
        let g = table.finished.pop_front().unwrap();
        let h = table.finished.pop_front().unwrap();
        assert_eq!(
            g.answer,
            Some(DownloadAnswer {
                status: Some(200),
                content_type: Some("application/octet-stream".into()),
                content_length: Some(14_245),
                last_modified: Some("Fri, 02 Oct 2026 02:11:11 GMT".into()),
            })
        );
        assert_eq!(g.source.as_ref().map(|s| s.url().as_str()), Some(url));
        assert_eq!(h.answer, None);
        // The address never shows.
        let shown = format!("{g:?}");
        assert!(
            !shown.contains("SECRET") && !shown.contains("ABCDEFGHIJKL"),
            "{shown}"
        );
    }
    fn document(frame: &str, url: &str, status: u64) -> Value {
        json!({
            "type": "Document", "frameId": frame,
            "response": { "url": url, "status": status, "mimeType": "text/html",
                          "headers": { "Content-Type": "text/html; charset=utf-8" } },
        })
    }

    #[test]
    fn only_the_documents_of_the_runs_pages_are_told_and_those_of_frames_are_not() {
        let run = ready_run(0);
        run.target_attached("P1", "s1", "page");
        // A popup is a page of the run as well; a frame of another site is a
        // target of its own, of another kind.
        run.target_attached("P2", "s2", "page");
        run.target_attached("F1", "s3", "iframe");
        let mut told = run.page_documents.subscribe();

        run.response_received(&document("F1", "https://accounts.google.com/gsi/x", 200));
        run.response_received(&document("unknown", "https://drive.google.com/x", 200));
        run.response_received(&json!({
            "type": "Image", "frameId": "P1",
            "response": { "url": "https://erulabo.com/a.png", "status": 200 },
        }));
        run.response_received(&document("P1", "https://accounts.google.com/signin", 200));
        run.response_received(&document("P2", "https://erulabo.com/860", 404));

        let first = told.try_recv().unwrap();
        assert_eq!(
            first.source.url().as_str(),
            "https://accounts.google.com/signin"
        );
        assert_eq!(first.answer.status, Some(200));
        assert_eq!(first.answer.content_type.as_deref(), Some("text/html"));
        let second = told.try_recv().unwrap();
        assert_eq!(second.source.url().host_str(), Some("erulabo.com"));
        assert_eq!(second.answer.status, Some(404));
        assert!(told.try_recv().is_err(), "nothing else was a page's");
        // The address does not show in the debug text.
        assert!(!format!("{first:?}").contains("signin"));
    }

    #[test]
    fn a_page_that_is_gone_is_no_page_and_an_answer_with_no_status_has_none() {
        let run = ready_run(0);
        run.target_attached("P1", "s1", "page");
        run.target_destroyed("P1");
        let mut told = run.page_documents.subscribe();
        run.response_received(&document("P1", "https://erulabo.com/860", 200));
        assert!(told.try_recv().is_err());

        let answer = DownloadAnswer::of_response(&json!({
            "url": "https://erulabo.com/860", "mimeType": "text/html",
        }));
        assert_eq!(answer.status, None);
        let answer = DownloadAnswer::of_response(&json!({ "status": 70_000 }));
        assert_eq!(answer.status, None);
        let answer = DownloadAnswer::of_response(&json!({ "status": 429 }));
        assert_eq!(answer.status, Some(429));
    }

    #[test]
    fn a_download_is_active_from_its_beginning_until_it_is_handed_out() {
        let run = ready_run(0);
        assert!(!run.download_active());
        began(&run, "g", 0);
        assert!(run.download_active());
        run.download_progress("g", "completed", 1, 1);
        // Ended, and not taken yet.
        assert!(run.download_active());
        run.downloads.lock().unwrap().finished.clear();
        assert!(!run.download_active());
    }
}
