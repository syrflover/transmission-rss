//! The worker's side of the server browser: browser runs bound to jobs.
//!
//! A [`BrowserPool`] asks the launcher in the browser container
//! ([`crate::launcher`]) for runs, one for a job at a time. [`BrowserPool::start`]
//! waits for a free slot under the policy's cap on concurrent browser jobs,
//! starts a run, connects to its DevTools through the launcher's proxy and
//! prepares it: downloads are saved under the run's own folder (named by the
//! browser, finished ones announced), and the ad and tracker addresses
//! ([`crate::blocklist`]) are blocked in every page of the run, popups and
//! frames included.
//!
//! # A run belongs to its job
//!
//! A [`BrowserRun`] is the handle of one run of one job. The run's id is new
//! every time (`<job>-<random>`), so a handle of a run that ended can never
//! reach the run that follows it. Every operation on an ended run answers
//! [`BrowserError::RunEnded`]. A run ends when [`BrowserRun::end`] is called,
//! when the idle reaper ends it, when its DevTools connection is lost, or when
//! the launcher no longer has it (its Chromium exited). Ending a run leaves
//! the job and what it received alone: the pool only closes the browser.
//!
//! # Idle time
//!
//! A run is *busy* while a job step has called [`BrowserRun::mark_busy`] (and
//! not [`BrowserRun::mark_idle`] since) or a download is under way. A run that
//! is not busy and whose last activity is older than the policy's idle time
//! ends ([`BrowserPool::reap_once`], run every [`REAP_EVERY`] by
//! [`BrowserPool::run_reaper`]). Activity is the later of the last
//! [`BrowserRun::touch`] (a person using the screen), the last use reported
//! by the [`ActivitySource`] (a person's input the web relayed, read at each
//! reaper pass), the end of the last busy stretch, the end of the last
//! download, and the start. A screen that is only watched is not activity:
//! nothing reports it. The queries
//! ([`BrowserPool::status`], [`BrowserPool::run_of_job`]) never start a run and
//! never move the activity time.
//!
//! A step that [`BrowserRun::mark_busy`] and an idle end meet are ordered
//! under one lock: the step is counted before the reaper looks, or it finds
//! the run ended. [`BrowserPool::start`] giving back a run that is there
//! counts as using it.
//!
//! # Downloads
//!
//! The browser saves each download under the run's folder, named by its guid
//! (a UUID). The folder is the browser's to write, and the browser can be
//! compromised, so [`BrowserRun::move_download`] takes only a plain file with
//! one name from a plain folder ([`BrowserError::UnsafeDownload`] otherwise).
//! A download is canceled when it is over [`MAX_DOWNLOAD_BYTES`], when the
//! run's downloads together would pass [`MAX_RUN_DOWNLOAD_BYTES`], and when it
//! reports nothing for [`DOWNLOAD_STALL`]. One that is being canceled no
//! longer keeps the run busy.
//!
//! # Worker restarts
//!
//! A new pool resets the launcher (every run ends, every profile goes) and
//! empties its downloads folder once the reset has gone through, so nothing
//! of a run of the former worker stays. The jobs themselves are not touched
//! here.
//!
//! The pool takes the launcher as its own: it resets it, and its reaper ends
//! the runs it does not know. So one worker at a time may use one launcher
//! (the worker takes a lock for the process's life before it makes its
//! pool).

mod driver;
mod files;
mod run;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::future::BoxFuture;
use tokio_util::sync::CancellationToken;
use trss_core::{Clock, Millis};
use url::Url;

use crate::{
    cdp::CdpError,
    client::{LauncherClient, LauncherError},
};
pub use files::{safe_file_name, MovedFile};
use run::RunInner;
pub use run::{
    BrowserRun, DialogSeen, Dialogs, Download, DownloadState, Page, RunStatus, DOWNLOAD_STALL,
    MAX_DOWNLOAD_BYTES, MAX_RUN_DOWNLOAD_BYTES,
};

/// How often [`BrowserPool::run_reaper`] looks for idle runs.
pub const REAP_EVERY: Duration = Duration::from_secs(15);
/// How long a start waits before it reads the policy again for a free slot
/// (a slot freeing wakes it at once).
pub const SLOT_RECHECK: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    /// The run ended: nothing can be done with its handle any more.
    #[error("browser run {run} has ended")]
    RunEnded { run: String },
    #[error(transparent)]
    Launcher(#[from] LauncherError),
    #[error(transparent)]
    Cdp(CdpError),
    #[error("the page {0} is not there")]
    PageGone(String),
    #[error("{0} did not happen in time")]
    Timeout(&'static str),
    #[error("the download {0} did not complete")]
    DownloadNotCompleted(String),
    #[error("{0} exists in the folder already")]
    AlreadyExists(String),
    /// The browser's folder did not hold what a download is: the download is
    /// left where it is and not used.
    #[error("the download cannot be taken: {0}")]
    UnsafeDownload(&'static str),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("internal error: {0}")]
    Internal(String),
}

/// The part of the common policy the browser follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserPolicy {
    /// A run that is not busy ends this long after its last activity.
    pub idle: Duration,
    /// How many jobs may have a run at once.
    pub max_concurrent: usize,
}

impl Default for BrowserPolicy {
    /// The policy's defaults: five minutes, one job.
    fn default() -> BrowserPolicy {
        BrowserPolicy {
            idle: Duration::from_secs(300),
            max_concurrent: 1,
        }
    }
}

impl BrowserPolicy {
    pub fn from_policy(policy: &trss_core::settings::policy::Policy) -> BrowserPolicy {
        BrowserPolicy {
            idle: Duration::from_secs(u64::from(policy.idle_timeout_seconds)),
            max_concurrent: policy.max_concurrent_jobs.max(1) as usize,
        }
    }
}

/// Where the pool reads the policy: at each start and each reaper pass, so a
/// change applies without a restart.
#[derive(Clone)]
pub struct PolicySource(Arc<dyn Fn() -> BoxFuture<'static, BrowserPolicy> + Send + Sync>);

impl PolicySource {
    pub fn new<F>(read: F) -> PolicySource
    where
        F: Fn() -> BoxFuture<'static, BrowserPolicy> + Send + Sync + 'static,
    {
        PolicySource(Arc::new(read))
    }

    pub fn fixed(policy: BrowserPolicy) -> PolicySource {
        PolicySource::new(move || Box::pin(async move { policy }))
    }

    async fn get(&self) -> BrowserPolicy {
        (self.0)().await
    }
}

/// Where the pool reads the uses of its runs that happen outside the worker:
/// a person's input that the web relays to a run's page. Each reaper pass
/// reads it before it decides, and takes each `(run id, time)` as a use of
/// that run at that time ([`BrowserRun::touch`]). A run it does not name, or
/// one that has ended, is left as it is; the screen merely being watched is
/// never reported, so it is not activity.
#[derive(Clone)]
pub struct ActivitySource(Arc<ReadActivity>);

/// The uses outside the worker: `(run id, time)` each.
type ReadActivity = dyn Fn() -> BoxFuture<'static, Vec<(String, Millis)>> + Send + Sync;

impl ActivitySource {
    pub fn new<F>(read: F) -> ActivitySource
    where
        F: Fn() -> BoxFuture<'static, Vec<(String, Millis)>> + Send + Sync + 'static,
    {
        ActivitySource(Arc::new(read))
    }

    async fn get(&self) -> Vec<(String, Millis)> {
        (self.0)().await
    }
}

/// How the pool reaches the browser container and the files it saves.
#[derive(Clone)]
pub struct PoolConfig {
    /// The launcher's address inside the compose network.
    pub launcher: Url,
    pub token: String,
    /// The shared downloads folder, at the path the worker sees it. The
    /// browser container mounts the same folder at its own path; a run's
    /// downloads are in the folder named by its id.
    pub downloads_root: PathBuf,
    /// How often [`BrowserPool::run_reaper`] looks (default [`REAP_EVERY`]).
    pub reap_every: Duration,
    /// How long a start that waits for a slot goes without reading the policy
    /// again (default [`SLOT_RECHECK`]).
    pub slot_recheck: Duration,
    /// The uses of the runs outside the worker (default: none).
    pub activity: Option<ActivitySource>,
}

impl std::fmt::Debug for PoolConfig {
    /// Without the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolConfig")
            .field("launcher", &self.launcher.as_str())
            .field("downloads_root", &self.downloads_root)
            .field("reap_every", &self.reap_every)
            .field("slot_recheck", &self.slot_recheck)
            .finish_non_exhaustive()
    }
}

impl PoolConfig {
    pub fn new(
        launcher: Url,
        token: impl Into<String>,
        downloads_root: impl Into<PathBuf>,
    ) -> Self {
        PoolConfig {
            launcher,
            token: token.into(),
            downloads_root: downloads_root.into(),
            reap_every: REAP_EVERY,
            slot_recheck: SLOT_RECHECK,
            activity: None,
        }
    }

    /// The same configuration reading the uses of the runs outside the worker
    /// from `activity`.
    pub fn with_activity(mut self, activity: ActivitySource) -> Self {
        self.activity = Some(activity);
        self
    }
}

pub(crate) struct PoolInner {
    pub(crate) launcher: LauncherClient,
    pub(crate) downloads_root: PathBuf,
    pub(crate) clock: Clock,
    policy: PolicySource,
    activity: Option<ActivitySource>,
    reap_every: Duration,
    slot_recheck: Duration,
    /// The runs by id, those still being started and those ended but not yet
    /// confirmed gone by the launcher included: they all hold a slot.
    runs: Mutex<HashMap<String, Arc<RunInner>>>,
    /// Rung when a run is ready or gone.
    changed: tokio::sync::Notify,
    /// Whether the launcher has been reset since this pool began.
    reset_done: tokio::sync::Mutex<bool>,
}

/// The server browser of a worker. Cheap to clone.
#[derive(Clone)]
pub struct BrowserPool {
    inner: Arc<PoolInner>,
}

impl BrowserPool {
    /// A pool over the launcher `config` names. It makes the downloads folder
    /// writable for the browser, resets the launcher so that no run of an
    /// earlier worker is left, and empties the downloads folder. A launcher
    /// that cannot be reached yet is tried again at the first start.
    pub async fn new(
        config: PoolConfig,
        clock: Clock,
        policy: PolicySource,
    ) -> Result<BrowserPool, BrowserError> {
        let launcher = LauncherClient::new(config.launcher, config.token)
            .map_err(|e| BrowserError::Internal(format!("cannot build the HTTP client: {e}")))?;
        let pool = BrowserPool {
            inner: Arc::new(PoolInner {
                launcher,
                downloads_root: config.downloads_root,
                clock,
                policy,
                activity: config.activity,
                reap_every: config.reap_every,
                slot_recheck: config.slot_recheck,
                runs: Mutex::default(),
                changed: tokio::sync::Notify::new(),
                reset_done: tokio::sync::Mutex::new(false),
            }),
        };
        if let Err(err) = pool.inner.ensure_reset().await {
            eprintln!("Browser: the browser container is not ready yet ({err}); trying again at the first use");
        }
        Ok(pool)
    }

    /// The run of `job_id`, started if the job has none: waits (cancel by
    /// dropping the future) for a free slot under the policy's cap on
    /// concurrent browser jobs, read again at every attempt.
    ///
    /// A job has at most one live run, so a job that has one gets it back.
    /// A handle of a run that has ended is never given again; ask again for a
    /// new run.
    pub async fn start(&self, job_id: &str) -> Result<BrowserRun, BrowserError> {
        self.inner.ensure_reset().await?;
        let entry = loop {
            let policy = self.inner.policy.get().await;
            // Registered before the state is looked at, so a change between
            // the look and the wait is not missed.
            let changed = self.inner.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let reserved = {
                let mut runs = self.inner.runs.lock().expect("runs lock");
                match runs.values().find(|r| r.job_id == job_id && !r.is_ended()) {
                    // Asking for the run is using it: the idle time counts
                    // from here. Decided against an idle end under one lock,
                    // so a run that has just ended is not given back.
                    Some(own) if own.is_ready() && own.touch_live(self.inner.now()) => {
                        return Ok(BrowserRun::new(self.inner.clone(), own.clone()))
                    }
                    // This job's run is being started by another call.
                    Some(_) => None,
                    None if runs.len() < policy.max_concurrent => {
                        let run_id = new_run_id(job_id);
                        let entry = Arc::new(RunInner::new(
                            job_id,
                            &run_id,
                            self.inner.downloads_root.join(&run_id),
                            (self.inner.clock)(),
                        ));
                        runs.insert(run_id, entry.clone());
                        Some(entry)
                    }
                    None => None,
                }
            };
            if let Some(entry) = reserved {
                break entry;
            }
            tokio::select! {
                _ = &mut changed => {}
                _ = tokio::time::sleep(self.inner.slot_recheck) => {}
            }
        };

        // In a task of its own: a caller that gives up halfway must not leave
        // a half-started run behind.
        let setup = tokio::spawn({
            let (inner, entry) = (self.inner.clone(), entry.clone());
            async move { inner.set_up(&entry).await }
        });
        match setup.await {
            Ok(Ok(())) => Ok(BrowserRun::new(self.inner.clone(), entry)),
            Ok(Err(err)) => Err(err),
            Err(err) => {
                self.inner.end_entry(&entry).await;
                Err(BrowserError::Internal(format!(
                    "the start was cut short: {err}"
                )))
            }
        }
    }

    /// The live run of `job_id`, if it has one. Starts nothing and does not
    /// count as activity.
    pub fn run_of_job(&self, job_id: &str) -> Option<BrowserRun> {
        let runs = self.inner.runs.lock().expect("runs lock");
        runs.values()
            .find(|r| r.job_id == job_id && !r.is_ended() && r.is_ready())
            .map(|r| BrowserRun::new(self.inner.clone(), r.clone()))
    }

    /// The runs now, by start time. Starts nothing and does not count as
    /// activity.
    pub fn status(&self) -> Vec<RunStatus> {
        let runs = self.inner.runs.lock().expect("runs lock");
        let mut list: Vec<RunStatus> = runs.values().map(|r| r.status()).collect();
        list.sort_by(|a, b| (a.started_at, &a.run_id).cmp(&(b.started_at, &b.run_id)));
        list
    }

    /// Ends the live run of `job_id`, if any.
    pub async fn end_job(&self, job_id: &str) {
        let entries: Vec<Arc<RunInner>> = {
            let runs = self.inner.runs.lock().expect("runs lock");
            runs.values()
                .filter(|r| r.job_id == job_id && !r.is_ended())
                .cloned()
                .collect()
        };
        for entry in entries {
            self.inner.end_entry(&entry).await;
        }
    }

    /// One pass of the reaper: ends the runs that are idle past the policy's
    /// idle time, asks the launcher again to end those it has not confirmed,
    /// and reconciles with the launcher's list (a run it no longer has is
    /// ended here; a run only it has is ended there). Returns the ids of the
    /// runs ended for being idle.
    pub async fn reap_once(&self) -> Vec<String> {
        let inner = &self.inner;
        if let Err(err) = inner.ensure_reset().await {
            eprintln!("Browser: the browser container cannot be reached: {err}");
            return Vec::new();
        }
        let policy = inner.policy.get().await;
        // A person's input relayed by the web, read before the clock: a use
        // reported now is not older than the time it is judged at.
        let outside: HashMap<String, Millis> = match &inner.activity {
            Some(activity) => {
                activity
                    .get()
                    .await
                    .into_iter()
                    .fold(HashMap::new(), |mut uses, (run, at)| {
                        let last = uses.entry(run).or_insert(at);
                        *last = (*last).max(at);
                        uses
                    })
            }
            None => HashMap::new(),
        };
        let now = (inner.clock)();
        let idle_ms = i64::try_from(policy.idle.as_millis()).unwrap_or(i64::MAX);

        let snapshot: Vec<Arc<RunInner>> = inner
            .runs
            .lock()
            .expect("runs lock")
            .values()
            .cloned()
            .collect();
        let mut ended_idle = Vec::new();
        for entry in &snapshot {
            if entry.is_ended() {
                // The launcher did not confirm the end: ask again.
                inner.end_entry(entry).await;
                continue;
            }
            if !entry.is_ready() {
                continue;
            }
            // A use from outside counts like a touch, never later than now.
            if let Some(at) = outside.get(&entry.run_id) {
                entry.touch_live((*at).min(now));
            }
            // A download that has gone quiet is canceled, and no longer
            // keeps the run from being idle.
            for guid in entry.take_stalled_downloads(now, DOWNLOAD_STALL) {
                println!(
                    "Browser: a download of the run of job {} made no progress for {}s; canceling it",
                    entry.job_id,
                    DOWNLOAD_STALL.as_secs()
                );
                driver::cancel_download(entry, &guid).await;
            }
            // Looked at and ended in one step, against a job step that
            // starts meanwhile.
            if entry.end_if_idle(now, idle_ms) {
                println!(
                    "Browser: the run of job {} was idle for {}s; closing it",
                    entry.job_id,
                    policy.idle.as_secs()
                );
                inner.end_entry(entry).await;
                ended_idle.push(entry.run_id.clone());
            }
        }

        // What the launcher has, against what the pool has. The runs that
        // were ready before the launcher was asked: one made ready after its
        // answer was served is not in the answer, and is not gone.
        let ready_before: Vec<Arc<RunInner>> = inner
            .runs
            .lock()
            .expect("runs lock")
            .values()
            .filter(|r| r.is_ready() && !r.is_ended())
            .cloned()
            .collect();
        if let Ok(live) = inner.launcher.list().await {
            for entry in &ready_before {
                let there = live.iter().any(|r| r.run == entry.run_id);
                if !entry.is_ended() && !there {
                    println!(
                        "Browser: the run of job {} is gone from the browser container",
                        entry.job_id
                    );
                    inner.end_entry(entry).await;
                }
            }
            // A run is in the pool's table before the launcher starts it, so
            // one that is not there is not a start under way.
            for info in &live {
                let tracked = inner
                    .runs
                    .lock()
                    .expect("runs lock")
                    .contains_key(&info.run);
                if !tracked && crate::protocol::is_valid_run_id(&info.run) {
                    println!("Browser: ending run {}, which no job holds", info.run);
                    if inner.launcher.end(&info.run).await.is_ok() {
                        // Whatever it downloaded is of no job.
                        remove_tree(&inner.downloads_root.join(&info.run)).await;
                    }
                }
            }
        }
        ended_idle
    }

    /// Looks for idle runs every [`PoolConfig::reap_every`] until `cancel`
    /// fires.
    pub async fn run_reaper(&self, cancel: CancellationToken) {
        let mut ticker = tokio::time::interval(self.inner.reap_every);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = ticker.tick() => {}
            }
            // In its own task so that a panic ends the pass, not the loop.
            let pass = tokio::spawn({
                let pool = self.clone();
                async move { pool.reap_once().await }
            });
            if let Err(err) = pass.await {
                eprintln!("Browser: the reaper ended with an internal error: {err}");
            }
        }
    }

    /// Ends every run, for a worker that is stopping.
    pub async fn shutdown(&self) {
        let entries: Vec<Arc<RunInner>> = self
            .inner
            .runs
            .lock()
            .expect("runs lock")
            .values()
            .cloned()
            .collect();
        for entry in &entries {
            entry.mark_ended();
        }
        if let Err(err) = self.inner.launcher.reset().await {
            eprintln!("Browser: cannot end the runs at shutdown: {err}");
        }
        for entry in &entries {
            self.inner.end_entry(entry).await;
        }
    }
}

/// `<job>-<random>`: the job's id (its characters kept to the allowed ones,
/// at most 12) and 12 random hexadecimal digits.
fn new_run_id(job_id: &str) -> String {
    let job: String = job_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(12)
        .collect();
    let random = uuid::Uuid::new_v4().simple().to_string();
    format!("{job}-{}", &random[..12])
}

impl PoolInner {
    pub(crate) fn now(&self) -> Millis {
        (self.clock)()
    }

    /// Resets the launcher the first time, and empties the downloads folder
    /// after it.
    async fn ensure_reset(&self) -> Result<(), BrowserError> {
        let mut done = self.reset_done.lock().await;
        if *done {
            return Ok(());
        }
        self.prepare_downloads_root().await;
        self.launcher.reset().await?;
        self.clear_downloads_root().await;
        *done = true;
        Ok(())
    }

    /// The folder is shared with a browser that runs as another user and
    /// makes a folder in it for each run, like `/tmp`.
    async fn prepare_downloads_root(&self) {
        use std::os::unix::fs::PermissionsExt;
        if let Err(err) = tokio::fs::create_dir_all(&self.downloads_root).await {
            eprintln!(
                "Browser: cannot make {}: {err}",
                self.downloads_root.display()
            );
            return;
        }
        let mode = std::fs::Permissions::from_mode(0o1777);
        if let Err(err) = tokio::fs::set_permissions(&self.downloads_root, mode).await {
            eprintln!(
                "Browser: cannot open {} to the browser user: {err}",
                self.downloads_root.display()
            );
        }
    }

    async fn clear_downloads_root(&self) {
        let Ok(mut entries) = tokio::fs::read_dir(&self.downloads_root).await else {
            return;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            remove_tree(&entry.path()).await;
        }
    }

    /// Starts the launcher's run for `entry`, connects, and prepares it.
    async fn set_up(self: &Arc<Self>, entry: &Arc<RunInner>) -> Result<(), BrowserError> {
        let prepared = driver::prepare(self, entry).await;
        if prepared.is_err() {
            self.end_entry(entry).await;
        }
        prepared
    }

    /// Ends `entry`: nothing can be done with its handles from now on, and the
    /// launcher is asked to end the run. The entry leaves the table, and the
    /// slot is free, once the launcher confirms; until then the reaper asks
    /// again.
    pub(crate) async fn end_entry(&self, entry: &Arc<RunInner>) {
        entry.mark_ended();
        let _one_at_a_time = entry.end_lock.lock().await;
        if entry.is_removed() {
            return;
        }
        match self.launcher.end(&entry.run_id).await {
            Ok(()) => {
                remove_tree(&entry.downloads_dir).await;
                entry.mark_removed();
                self.runs.lock().expect("runs lock").remove(&entry.run_id);
                self.changed.notify_waiters();
            }
            Err(err) => eprintln!(
                "Browser: cannot end the run of job {} at the browser container yet: {err}",
                entry.job_id
            ),
        }
    }
}

async fn remove_tree(path: &std::path::Path) {
    let removed = match tokio::fs::symlink_metadata(path).await {
        Ok(meta) if meta.is_dir() => tokio::fs::remove_dir_all(path).await,
        Ok(_) => tokio::fs::remove_file(path).await,
        Err(_) => return,
    };
    if let Err(err) = removed {
        if err.kind() != std::io::ErrorKind::NotFound {
            eprintln!("Browser: cannot remove {}: {err}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_ids_are_new_valid_names() {
        let a = new_run_id("01J3-job/with odd:chars and a long tail");
        let b = new_run_id("01J3-job/with odd:chars and a long tail");
        assert_ne!(a, b);
        assert!(crate::protocol::is_valid_run_id(&a), "{a}");
        assert!(a.starts_with("01J3-job_wit-"), "{a}");
        assert!(crate::protocol::is_valid_run_id(&new_run_id("")));
    }

    #[test]
    fn the_policy_follows_the_settings() {
        let stored = trss_core::settings::policy::Policy {
            idle_timeout_seconds: 90,
            max_concurrent_jobs: 2,
            ..Default::default()
        };
        assert_eq!(
            BrowserPolicy::from_policy(&stored),
            BrowserPolicy {
                idle: Duration::from_secs(90),
                max_concurrent: 2
            }
        );
        assert_eq!(
            BrowserPolicy::from_policy(&trss_core::settings::policy::Policy::default()),
            BrowserPolicy::default()
        );
    }
}
