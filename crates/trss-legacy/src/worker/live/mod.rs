//! Changes in the watch folders, taken from the kernel's inotify alerts instead
//! of a read of every folder each cycle (`docs/specs/library.md`, 작품 발견과
//! 감시 폴더; ticket 0016).
//!
//! The worker watches each watch folder's root, work folders and season
//! folders ([`tree`]), one inotify instance per watch folder. An alert only
//! says *which work to read again*: the alerts of one work are gathered for a
//! few seconds ([`LiveConfig::debounce`]), and then, under the same lock as the
//! cycles ([`trss_core::CycleLock`]), that one work is read and
//! recorded with the code every other reading uses
//! ([`watch::scan_works`](crate::worker::watch::scan_works)). A reading that
//! finds the lock taken waits and tries again, so alerts never interleave with
//! a cycle or a command, and the record is always what the disk says whatever
//! order the alerts came in. Added times stay what the readings say: the time
//! of the reading that first saw a file, never a file system time.
//!
//! # What the alerts cannot tell
//!
//! The cycle's read of every directory's modification time
//! ([`discovery::scan_incremental`](crate::discovery::scan_incremental)) still
//! fills in where alerts cannot:
//!
//! - **The worker's start.** What changed while it was off: the first cycle
//!   reads every folder (the watches are placed first, so what changes during
//!   that read is alerted).
//! - **A queue overflow.** The kernel dropped alerts of that watch folder (each
//!   has its own instance): its watches are placed again and the whole folder
//!   is read.
//! - **A directory without a watch.** The system's limit
//!   (`fs.inotify.max_user_watches`), no permission, or a link: its work is read
//!   by every cycle, with unchanged directories skipped. The folder's row says
//!   how many and why ([`WatchFolder::watch_note`](crate::store::library::WatchFolder)).
//!   A folder that cannot be watched at all is read whole by every cycle, as
//!   before.
//! - **A safety net, every hour ([`LiveConfig::safety_net`]).** One read of
//!   every folder, which also places the watches again: it catches what an
//!   alert never brought (a folder mounted over the network, whose changes by
//!   other machines the kernel does not report; a download that adds files deep
//!   inside a season's own folders).
//!
//! When every directory is watched, a cycle reads no watch folder at all except
//! for the hourly net. Until [`LiveWatch::start`] is called (a worker that is
//! only ticked, as tests do), nothing is watched and every cycle reads every
//! folder as it always did.
//!
//! # Folders and watches
//!
//! [`LiveWatch::sync_folders`] makes the watched folders the registered ones: a
//! folder registered after the start is watched and read once to catch what
//! happened before its watches; one that is unregistered or whose path changed
//! loses its watches. The archive move
//! ([`crate::worker::commands::rule_archive`]) asks the watches of the work
//! folder it moved to follow ([`LiveWatch::resync`]), though the alerts of the
//! move already do.
//!
//! # What this costs
//!
//! The kernel keeps about a kilobyte per watch, and the watched directories
//! stay in its inode cache, so a check that would have read a directory finds it
//! in memory. The row of the folders says when the limit is reached.

mod tree;
mod watcher;

use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::Duration,
};

use tokio::{io::unix::AsyncFd, sync::mpsc, task::JoinHandle};

use tree::WatchTree;
pub use tree::Why;
use watcher::{Control, SharedFd, Task};

use trss_core::Clock;

use super::CycleContext;
use crate::store::{history::Millis, library::WatchFolder};

/// How long placing the watches of one folder may hold up whoever attaches it
/// (the command poll). Past it the folder is read by every cycle instead.
const ATTACH_TIMEOUT: Duration = Duration::from_secs(15);

/// How the watches behave. The defaults are what the worker runs with; tests
/// shorten the times and set a small limit.
#[derive(Debug, Clone)]
pub struct LiveConfig {
    /// How long the alerts of one work are gathered before it is read.
    pub debounce: Duration,
    /// How long a reading that found the lock taken waits to try again.
    pub retry: Duration,
    /// How long a folder goes without a whole read before the safety net reads it.
    pub safety_net: Duration,
    /// The most watches one folder may hold (`None`: the system's limit decides).
    /// A seam for tests of a system that has run out of watches.
    pub max_watches: Option<usize>,
}

impl Default for LiveConfig {
    fn default() -> Self {
        LiveConfig {
            debounce: Duration::from_secs(3),
            retry: Duration::from_secs(1),
            safety_net: Duration::from_secs(60 * 60),
            max_watches: None,
        }
    }
}

/// What the cycle knows of one watched folder (see [`LiveWatch::status`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderStatus {
    /// The folder's own directory has a watch.
    pub root_watched: bool,
    /// How many directories have no watch.
    pub unwatched_dirs: usize,
    /// The works with a directory that has no watch.
    pub unwatched_works: BTreeSet<String>,
    /// How many watches the folder holds.
    pub watches: usize,
    /// The sentence on the folder's row about directories without a watch.
    pub note: Option<String>,
    /// When the folder was last read whole (the worker's clock).
    pub last_scan_at: Option<Millis>,
    /// The last reading found a problem (the folder or a work could not be
    /// read), so the cycle reads the folder again.
    pub scan_problem: bool,
    /// The folder was attached after the worker started and has not been read
    /// since: what happened before its watches is still to be found.
    pub needs_catchup: bool,
}

/// What a cycle has to read of a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    /// The whole folder, skipping directories that have not changed.
    Folder,
    /// Only these works (the ones with a directory that has no watch).
    Works(Vec<String>),
    /// Nothing: every directory is watched and alerts do the reading.
    Nothing,
}

/// One reading, as [`LiveWatch::readings`] lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// A whole watch folder (by ID).
    Folder(String),
    /// One work of a watch folder (by ID, and the work's folder name).
    Work(String, String),
}

const LOG_LIMIT: usize = 512;

#[derive(Default)]
struct Stats {
    folder_reads: AtomicU64,
    work_reads: AtomicU64,
    recent: Mutex<VecDeque<Reading>>,
}

/// What a folder's task needs from the worker.
#[derive(Clone)]
pub(super) struct Runtime {
    pub ctx: CycleContext,
    pub lock_path: PathBuf,
    pub clock: Clock,
    /// How often the heartbeat is written while a reading holds the lock.
    pub heartbeat_every: Duration,
}

struct Attached {
    path: String,
    controls: mpsc::UnboundedSender<Control>,
    status: Arc<Mutex<FolderStatus>>,
    task: JoinHandle<()>,
}

struct Inner {
    config: LiveConfig,
    runtime: Mutex<Option<Runtime>>,
    folders: Mutex<HashMap<String, Attached>>,
    /// Whether folders have been attached since the start (the first attach
    /// leaves the first reading to the cycle).
    attached_once: AtomicBool,
    /// Held while folders are attached.
    sync: tokio::sync::Mutex<()>,
    /// How many tasks hold the worker's lock to read now.
    flushing: AtomicIsize,
    stats: Stats,
}

/// The worker's inotify watches of its watch folders. Cheap to clone.
#[derive(Clone)]
pub struct LiveWatch {
    inner: Arc<Inner>,
}

impl Default for LiveWatch {
    fn default() -> Self {
        LiveWatch::new(LiveConfig::default())
    }
}

/// The paths whose watches are being placed on a blocking thread: a folder on a
/// mount that has stopped answering holds its thread, and must not get another.
static ATTACHING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);

struct Claim(String);

impl Drop for Claim {
    fn drop(&mut self) {
        ATTACHING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

fn millis(duration: Duration) -> Millis {
    Millis::try_from(duration.as_millis()).unwrap_or(Millis::MAX)
}

impl LiveWatch {
    pub fn new(config: LiveConfig) -> LiveWatch {
        LiveWatch {
            inner: Arc::new(Inner {
                config,
                runtime: Mutex::default(),
                folders: Mutex::default(),
                attached_once: AtomicBool::new(false),
                sync: tokio::sync::Mutex::new(()),
                flushing: AtomicIsize::new(0),
                stats: Stats::default(),
            }),
        }
    }

    pub fn config(&self) -> &LiveConfig {
        &self.inner.config
    }

    /// Lets the folders be watched: [`LiveWatch::sync_folders`] places the
    /// watches. `ctx` is the context the readings run with.
    pub fn start(
        &self,
        ctx: CycleContext,
        lock_path: PathBuf,
        clock: Clock,
        heartbeat_every: Duration,
    ) {
        *self.inner.runtime.lock().unwrap_or_else(|e| e.into_inner()) = Some(Runtime {
            ctx,
            lock_path,
            clock,
            heartbeat_every,
        });
    }

    /// Ends every watch. The folders are read whole by every cycle again.
    pub fn stop(&self) {
        *self.inner.runtime.lock().unwrap_or_else(|e| e.into_inner()) = None;
        for (_, attached) in self
            .inner
            .folders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
        {
            attached.task.abort();
        }
        self.inner.attached_once.store(false, Ordering::SeqCst);
    }

    fn runtime(&self) -> Option<Runtime> {
        self.inner
            .runtime
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Makes the watched folders the registered ones (see the module docs).
    pub async fn sync_folders(&self) {
        let Some(runtime) = self.runtime() else {
            return;
        };
        let _one_at_a_time = self.inner.sync.lock().await;
        let folders = match runtime.ctx.library.folders().await {
            Ok(folders) => folders,
            Err(error) => {
                eprintln!("Watch folders: cannot list them to watch them: {error}");
                return;
            }
        };
        let initial = !self.inner.attached_once.swap(true, Ordering::SeqCst);

        let mut missing = Vec::new();
        {
            let mut attached = self.inner.folders.lock().unwrap_or_else(|e| e.into_inner());
            attached.retain(|id, folder| {
                let keep = !folder.task.is_finished()
                    && folders.iter().any(|f| &f.id == id && f.path == folder.path);
                if !keep {
                    folder.task.abort();
                }
                keep
            });
            for folder in &folders {
                if !attached.contains_key(&folder.id) {
                    missing.push(folder.clone());
                }
            }
        }
        for folder in missing {
            if let Some(attached) = self.attach(&runtime, &folder, !initial).await {
                self.inner
                    .folders
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(folder.id.clone(), attached);
            }
        }
    }

    /// Places the watches of `folder` and starts its task. `None` when that
    /// could not be done now (the folder is then read by every cycle).
    async fn attach(
        &self,
        runtime: &Runtime,
        folder: &WatchFolder,
        catch_up: bool,
    ) -> Option<Attached> {
        let claimed = ATTACHING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(folder.path.clone());
        if !claimed {
            return None;
        }
        let claim = Claim(folder.path.clone());
        let fd = match tree::init() {
            Ok(fd) => Arc::new(fd),
            Err(error) => {
                eprintln!(
                    "Watch folder {}: cannot start watching it (the inotify instance limit?): {error}",
                    folder.path
                );
                return None;
            }
        };
        let started = std::time::Instant::now();
        let mut tree = WatchTree::new(
            PathBuf::from(&folder.path),
            fd.clone(),
            self.inner.config.max_watches,
            self.inner.config.debounce,
        );
        // The directories are listed on a blocking thread, which a mount that
        // does not answer would hold; the claim lasts as long as the thread.
        let placed = tokio::time::timeout(
            ATTACH_TIMEOUT,
            tokio::task::spawn_blocking(move || {
                let _claim = claim;
                tree.sync_all(None);
                tree
            }),
        )
        .await;
        let tree = match placed {
            Ok(Ok(tree)) => tree,
            Ok(Err(error)) => {
                eprintln!(
                    "Watch folder {}: placing its watches failed: {error}",
                    folder.path
                );
                return None;
            }
            Err(_) => {
                eprintln!(
                    "Watch folder {}: placing its watches took more than {} s; it is read by every cycle instead",
                    folder.path,
                    ATTACH_TIMEOUT.as_secs()
                );
                return None;
            }
        };
        let snapshot = tree.status();
        println!(
            "Watch folder {}: {} directories watched, {} not, placed in {} ms",
            folder.path,
            snapshot.watches,
            snapshot.unwatched_dirs,
            started.elapsed().as_millis()
        );
        let status = Arc::new(Mutex::new(FolderStatus {
            root_watched: snapshot.root_watched,
            unwatched_dirs: snapshot.unwatched_dirs,
            unwatched_works: snapshot.unwatched_works,
            watches: snapshot.watches,
            note: snapshot.note,
            last_scan_at: None,
            scan_problem: false,
            needs_catchup: catch_up,
        }));
        let async_fd = match AsyncFd::new(SharedFd(fd)) {
            Ok(fd) => fd,
            Err(error) => {
                eprintln!(
                    "Watch folder {}: cannot wait on its watches: {error}",
                    folder.path
                );
                return None;
            }
        };
        let (controls, receiver) = mpsc::unbounded_channel();
        let task = tokio::spawn(
            Task {
                folder_id: folder.id.clone(),
                runtime: runtime.clone(),
                config: self.inner.config.clone(),
                status: status.clone(),
                controls: receiver,
                fd: async_fd,
                tree: Some(tree),
            }
            .run(),
        );
        Some(Attached {
            path: folder.path.clone(),
            controls,
            status,
            task,
        })
    }

    /// What a cycle has to read of folder `id` at `now` (see [`Poll`]).
    pub fn poll_for(&self, id: &str, now: Millis) -> Poll {
        let folders = self.inner.folders.lock().unwrap_or_else(|e| e.into_inner());
        let Some(attached) = folders.get(id) else {
            return Poll::Folder;
        };
        if attached.task.is_finished() {
            return Poll::Folder;
        }
        let status = attached.status.lock().unwrap_or_else(|e| e.into_inner());
        let net = millis(self.inner.config.safety_net);
        let due = status
            .last_scan_at
            .is_none_or(|at| now.saturating_sub(at) >= net);
        if !status.root_watched || status.scan_problem || due {
            Poll::Folder
        } else if !status.unwatched_works.is_empty() {
            Poll::Works(status.unwatched_works.iter().cloned().collect())
        } else {
            Poll::Nothing
        }
    }

    /// What the cycle knows of folder `id` (`None` when it is not watched).
    pub fn status(&self, id: &str) -> Option<FolderStatus> {
        let folders = self.inner.folders.lock().unwrap_or_else(|e| e.into_inner());
        let attached = folders.get(id)?;
        let status = attached.status.lock().unwrap_or_else(|e| e.into_inner());
        Some(status.clone())
    }

    fn control(&self, id: &str, control: Control) {
        if let Some(attached) = self
            .inner
            .folders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
        {
            let _ = attached.controls.send(control);
        }
    }

    /// Whether a cycle that reads folder `id` should also bring its watches to
    /// the disk: some directory has no watch (it may be possible now), or the
    /// folder has been read before (the hourly net places the watches again).
    pub(super) fn wants_resync(&self, id: &str) -> bool {
        self.status(id).is_some_and(|status| {
            !status.root_watched || status.unwatched_dirs > 0 || status.last_scan_at.is_some()
        })
    }

    /// Asks folder `id`'s watches to be brought to the disk: the whole folder,
    /// or only the work folder `work`. Done by the folder's task shortly after.
    pub fn resync(&self, id: &str, work: Option<&str>) {
        self.control(id, Control::Resync(work.map(str::to_owned)));
    }

    /// Acts as if the kernel had dropped folder `id`'s events: the watches are
    /// placed again and the whole folder is read. A seam for tests.
    #[doc(hidden)]
    pub fn simulate_overflow(&self, id: &str) {
        self.control(id, Control::Overflow);
    }

    // --- what the readings tell ---------------------------------------------------------

    /// A whole read of folder `id` starts: what a catch-up wanted is done by it.
    pub(super) fn folder_scan_started(&self, id: &str) {
        self.inner.stats.folder_reads.fetch_add(1, Ordering::SeqCst);
        self.log(Reading::Folder(id.to_owned()));
        self.with_status(id, |status| status.needs_catchup = false);
    }

    /// A whole read of folder `id` ended at `now`; `clean` when nothing could
    /// not be read.
    pub(super) fn folder_scan_done(&self, id: &str, now: Millis, clean: bool) {
        self.with_status(id, |status| {
            status.last_scan_at = Some(now);
            status.scan_problem = !clean;
        });
    }

    /// A read of some works of folder `id` starts. Logged before anything is
    /// recorded, as a whole read is, so a reading whose record can be seen is
    /// always in [`LiveWatch::readings`] already.
    pub(super) fn works_scan_started(&self, id: &str, names: &[String]) {
        self.inner
            .stats
            .work_reads
            .fetch_add(names.len() as u64, Ordering::SeqCst);
        for name in names {
            self.log(Reading::Work(id.to_owned(), name.clone()));
        }
    }

    /// A read of some works of folder `id` ended, `clean` when nothing could
    /// not be read.
    pub(super) fn works_scan_done(&self, id: &str, clean: bool) {
        if !clean {
            self.with_status(id, |status| status.scan_problem = true);
        }
    }

    fn with_status(&self, id: &str, change: impl FnOnce(&mut FolderStatus)) {
        let folders = self.inner.folders.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(attached) = folders.get(id) {
            change(&mut attached.status.lock().unwrap_or_else(|e| e.into_inner()));
        }
    }

    fn log(&self, reading: Reading) {
        let mut recent = self
            .inner
            .stats
            .recent
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if recent.len() == LOG_LIMIT {
            recent.pop_front();
        }
        recent.push_back(reading);
    }

    /// How many whole folder reads and how many work reads have been made.
    pub fn read_counts(&self) -> (u64, u64) {
        (
            self.inner.stats.folder_reads.load(Ordering::SeqCst),
            self.inner.stats.work_reads.load(Ordering::SeqCst),
        )
    }

    /// The latest readings (at most 512), oldest first.
    pub fn readings(&self) -> Vec<Reading> {
        self.inner
            .stats
            .recent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    /// How many watches are placed, over all folders.
    pub fn watch_count(&self) -> usize {
        self.inner
            .folders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|a| a.status.lock().unwrap_or_else(|e| e.into_inner()).watches)
            .sum()
    }

    pub(super) fn flushing_delta(&self, delta: isize) {
        self.inner.flushing.fetch_add(delta, Ordering::SeqCst);
    }

    /// Whether a watch task holds the worker's lock for a reading right now.
    pub fn flushing(&self) -> bool {
        self.inner.flushing.load(Ordering::SeqCst) > 0
    }
}
