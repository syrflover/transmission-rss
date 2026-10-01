//! The task that watches one watch folder: it reads the kernel's events into
//! its [`WatchTree`], and when something has been due for its debounce it
//! reads that part of the folder again under the worker's lock.

use std::{
    os::fd::{AsRawFd, RawFd},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use rustix::fd::OwnedFd;
use tokio::{io::unix::AsyncFd, sync::mpsc};

use super::{
    tree::{self, Due, RawEvent, WatchTree},
    FolderStatus, LiveConfig, Runtime,
};
use crate::{
    store::status::StatusStore,
    worker::{
        heartbeat,
        lock::CycleLock,
        watch::{self, ScanMode, WorksMode},
    },
};

/// What the rest of the worker can ask of a folder's task.
#[derive(Debug)]
pub(super) enum Control {
    /// Bring the watches to the disk: the whole folder, or one work folder.
    Resync(Option<String>),
    /// Act as if the kernel had dropped events (a seam for tests).
    Overflow,
}

/// The inotify descriptor as the reactor wants it, shared with the tree.
pub(super) struct SharedFd(pub Arc<OwnedFd>);

impl AsRawFd for SharedFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

/// What a folder's task needs from the worker.
pub(super) struct Task {
    pub folder_id: String,
    pub runtime: Runtime,
    pub config: LiveConfig,
    pub status: Arc<Mutex<FolderStatus>>,
    pub controls: mpsc::UnboundedReceiver<Control>,
    pub fd: AsyncFd<SharedFd>,
    pub tree: Option<WatchTree>,
}

/// Counts the lock the task holds, so that a cycle that finds it taken can
/// wait the moment out instead of skipping its turn.
struct Flushing<'a>(&'a super::LiveWatch);

impl<'a> Flushing<'a> {
    fn begin(live: &'a super::LiveWatch) -> Self {
        live.flushing_delta(1);
        Flushing(live)
    }
}

impl Drop for Flushing<'_> {
    fn drop(&mut self) {
        self.0.flushing_delta(-1);
    }
}

/// Reads the events waiting on the descriptor, waiting until there are some.
async fn read_events(fd: &AsyncFd<SharedFd>) -> std::io::Result<Vec<RawEvent>> {
    loop {
        let mut guard = fd.readable().await?;
        match guard.try_io(|inner| tree::drain(&inner.get_ref().0)) {
            Ok(result) => return result,
            Err(_would_block) => continue,
        }
    }
}

/// Runs `work` on the tree on a blocking thread (it looks at the disk, which
/// may be slow) and takes the tree back. `None` when the work panicked: the
/// tree is lost and the task ends.
async fn on_tree<R, F>(slot: &mut Option<WatchTree>, work: F) -> Option<R>
where
    R: Send + 'static,
    F: FnOnce(&mut WatchTree) -> R + Send + 'static,
{
    let mut tree = slot.take()?;
    match tokio::task::spawn_blocking(move || {
        let result = work(&mut tree);
        (tree, result)
    })
    .await
    {
        Ok((tree, result)) => {
            *slot = Some(tree);
            Some(result)
        }
        Err(error) => {
            eprintln!("Watch folder: the watches could not be kept up: {error}");
            None
        }
    }
}

impl Task {
    pub(super) async fn run(mut self) {
        let mut tree = self.tree.take();
        // When a reading that could not run (the lock was taken) may be tried again.
        let mut retry_at: Option<Instant> = None;
        // What was last written to the folder's row (`None`: nothing yet).
        let mut written: Option<Option<String>> = None;
        publish(
            &self.runtime,
            &self.folder_id,
            &self.status,
            tree.as_ref(),
            &mut written,
        )
        .await;

        loop {
            let wake = {
                let due = tree.as_ref().and_then(WatchTree::next_deadline);
                let catch_up = self
                    .catching_up()
                    .then(|| retry_at.unwrap_or_else(Instant::now));
                match (due, catch_up) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                }
            };
            tokio::select! {
                biased;
                control = self.controls.recv() => {
                    let Some(control) = control else { return };
                    let now = Instant::now();
                    let ok = match control {
                        Control::Resync(None) => {
                            on_tree(&mut tree, move |t| t.sync_all(Some(now))).await.is_some()
                        }
                        Control::Resync(Some(work)) => on_tree(&mut tree, move |t| {
                            t.sync_work(&work, Some(now));
                        })
                        .await
                        .is_some(),
                        Control::Overflow => {
                            on_tree(&mut tree, move |t| t.overflow(now)).await.is_some()
                        }
                    };
                    if !ok {
                        return;
                    }
                }
                events = read_events(&self.fd) => {
                    let events = match events {
                        Ok(events) => events,
                        Err(error) => {
                            eprintln!("Watch folder {}: cannot read its watch events: {error}", self.folder_id);
                            return;
                        }
                    };
                    let now = Instant::now();
                    if on_tree(&mut tree, move |t| t.handle(events, now)).await.is_none() {
                        return;
                    }
                }
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(wake.unwrap_or_else(Instant::now))), if wake.is_some() => {
                    retry_at = self.flush(&mut tree).await;
                }
            }
            publish(
                &self.runtime,
                &self.folder_id,
                &self.status,
                tree.as_ref(),
                &mut written,
            )
            .await;
        }
    }

    /// Reads again what is due, if the worker's lock is free. When it could not
    /// be done now, the time to try again.
    async fn flush(&self, slot: &mut Option<WatchTree>) -> Option<Instant> {
        let tree = slot.as_mut()?;
        let mut due = tree.take_due(Instant::now());
        let catching_up = self.catching_up();
        if due.is_empty() && !catching_up {
            return None;
        }
        let retry = Instant::now() + self.config.retry;
        let ctx = &self.runtime.ctx;
        // Announced before the lock is tried, so a cycle that finds it taken
        // waits for this reading instead of skipping itself.
        let _flushing = Flushing::begin(&ctx.live);
        let lock = match CycleLock::try_acquire(&self.runtime.lock_path) {
            Ok(Some(lock)) => lock,
            Ok(None) => {
                tree.defer(due, retry);
                return Some(retry);
            }
            Err(error) => {
                eprintln!(
                    "Watch folder {}: cannot take the worker lock: {error}",
                    self.folder_id
                );
                let later = Instant::now() + self.config.retry.max(Duration::from_secs(5));
                tree.defer(due, later);
                return Some(later);
            }
        };

        // The web sees the worker busy, not stopped, for as long as a reading
        // takes (a whole folder on a slow disk takes minutes); the beat ends
        // before the lock is let go.
        let read = async {
            let folder = match ctx.library.folder(&self.folder_id).await {
                Ok(Some(folder)) => folder,
                // Unregistered meanwhile: the worker's next look at the folders ends this task.
                Ok(None) => return Some(retry),
                Err(error) => {
                    eprintln!(
                        "Watch folder {}: cannot read it from the database: {error}",
                        self.folder_id
                    );
                    tree.defer(due, retry);
                    return Some(retry);
                }
            };
            // The cycle may have read the whole folder since the catch-up was asked for.
            let whole = due.folder || !folder.baselined || (catching_up && self.catching_up());
            let now = (self.runtime.clock)();
            let works = std::mem::take(&mut due.works);
            let outcome = if whole {
                watch::scan_folder(ctx, &folder, now, ScanMode::Periodic)
                    .await
                    .map(|_| ())
            } else if !works.is_empty() {
                watch::scan_works(ctx, &folder, works.clone(), now, WorksMode::Fresh)
                    .await
                    .map(|_| ())
            } else {
                Ok(())
            };
            if let Err(error) = outcome {
                eprintln!(
                    "Watch folder {}: cannot record the reading: {error}",
                    folder.path
                );
                // Nothing was recorded: what was due is read again shortly.
                let again = Due {
                    folder: whole,
                    works: if whole { Vec::new() } else { works },
                };
                tree.defer(again, retry);
                return Some(retry);
            }
            None
        };
        let next = heartbeat::while_holding(
            StatusStore::new(ctx.channels.db().clone()),
            self.runtime.clock.clone(),
            self.runtime.heartbeat_every,
            read,
        )
        .await;
        drop(lock);
        next
    }

    fn catching_up(&self) -> bool {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .needs_catchup
    }
}

/// Publishes the tree's status for the cycle, and writes the folder's row
/// when its sentence about directories without a watch changed.
async fn publish(
    runtime: &Runtime,
    folder_id: &str,
    status: &Mutex<FolderStatus>,
    tree: Option<&WatchTree>,
    written: &mut Option<Option<String>>,
) {
    let Some(tree) = tree else { return };
    let snapshot = tree.status();
    {
        let mut shared = status.lock().unwrap_or_else(|e| e.into_inner());
        shared.root_watched = snapshot.root_watched;
        shared.unwatched_dirs = snapshot.unwatched_dirs;
        shared.unwatched_works = snapshot.unwatched_works.clone();
        shared.watches = snapshot.watches;
        shared.note = snapshot.note.clone();
    }
    if written.as_ref() != Some(&snapshot.note) {
        match runtime
            .ctx
            .library
            .set_watch_note(folder_id, snapshot.note.clone())
            .await
        {
            Ok(()) => *written = Some(snapshot.note),
            Err(error) => {
                eprintln!("Watch folder {folder_id}: cannot write its watch note: {error}")
            }
        }
    }
}
