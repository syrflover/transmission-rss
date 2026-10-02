//! The lock that keeps two workers from working at the same time.
//!
//! [`CycleLock`] is the operating system's lock on a file next to the
//! database. [`WorkerLock`] shares one such lock among everything one worker
//! runs at the same time (its cycle, the web's commands, the readings its
//! folder watches ask for), and keeps the worker's heartbeat
//! ([`crate::heartbeat`]) for as long as any of them holds it.

use std::{
    any::Any,
    ffi::OsString,
    fs::{File, OpenOptions, TryLockError},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use crate::{
    heartbeat::{Heartbeat, HeartbeatStore},
    Clock,
};

/// The lock file's path for a database file: the database path plus
/// `.worker.lock`, so it sits next to the database on the same local volume.
pub fn lock_path_for(db_path: &Path) -> PathBuf {
    let mut name: OsString = db_path.as_os_str().to_owned();
    name.push(".worker.lock");
    PathBuf::from(name)
}

/// An exclusive advisory lock (`flock`) on a file, held while a worker works.
///
/// The operating system, not the worker, owns the lock: it is released when the
/// guard is dropped and also when the process dies for any reason, so a
/// crashed worker never leaves a stale lock, and a worker that is merely slow
/// keeps its lock for as long as it runs. No time limit is involved, which is
/// what stops a second worker from entering while the first is still working.
///
/// Every `try_acquire` opens its own file description, so two guards on the
/// same path exclude each other even inside one process.
///
/// Dropping the guard unlocks before it closes. Closing alone would not be
/// enough: the lock belongs to the open file description, and a child process
/// that another thread is starting holds a copy of every descriptor from its
/// fork until its `exec` closes them, which would keep the lock taken for that
/// moment after the guard is gone.
#[derive(Debug)]
pub struct CycleLock {
    file: File,
}

impl Drop for CycleLock {
    fn drop(&mut self) {
        // Closing the file releases it anyway once no copy is left.
        let _ = self.file.unlock();
    }
}

impl CycleLock {
    /// Takes the lock if nobody holds it. `Ok(None)` means another worker does.
    pub fn try_acquire(path: &Path) -> io::Result<Option<CycleLock>> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;

        match file.try_lock() {
            Ok(()) => Ok(Some(CycleLock { file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(err)) => Err(err),
        }
    }
}

/// The worker's lock as everything one worker runs shares it. Cheap to clone;
/// clones share the hold.
///
/// [`WorkerLock::try_hold`] takes the [`CycleLock`] when nothing of this worker
/// holds it yet, and otherwise hands out another handle to the hold it has.
/// So the worker's own work never finds the lock taken, while another worker
/// (another process, or another [`WorkerLock`] on the same file) finds it
/// taken for as long as any work of this one runs. Two workers therefore
/// never work at the same time, as before, and one worker runs its pieces of
/// work side by side; what keeps those from treading on each other is the
/// worker's business (see the `trss-worker` crate).
///
/// The heartbeat beats from the moment the lock is taken until the last
/// handle is let go with [`WorkerHold::release`], which writes the last beat
/// (clearing the hold) before the lock is let go. A last handle dropped
/// without it (its task panicked or was aborted) lets go of the lock all the
/// same, but only stops the beats, so the web sees a worker that stopped.
#[derive(Clone)]
pub struct WorkerLock {
    inner: Arc<Shared>,
}

struct Shared {
    path: PathBuf,
    heartbeat: HeartbeatStore,
    clock: Clock,
    every: Duration,
    /// The hold while anything holds it. Taking a hold and letting go of the
    /// last one happen under this, one at a time.
    held: tokio::sync::Mutex<Weak<Held>>,
}

/// The lock while the worker holds it.
struct Held {
    // Fields drop in order: the beats end before the lock is let go.
    beat: Mutex<Option<Heartbeat>>,
    _lock: CycleLock,
}

/// One handle to the worker's hold of its lock (see [`WorkerLock`]).
pub struct WorkerHold {
    held: Arc<Held>,
    lock: WorkerLock,
}

impl WorkerLock {
    /// The lock on the file at `path`, beating into `heartbeat` every `every`
    /// while held, with the times `clock` gives.
    pub fn new(path: PathBuf, heartbeat: HeartbeatStore, clock: Clock, every: Duration) -> Self {
        WorkerLock {
            inner: Arc::new(Shared {
                path,
                heartbeat,
                clock,
                every,
                held: tokio::sync::Mutex::new(Weak::new()),
            }),
        }
    }

    /// The lock file's path.
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// A handle to this worker's hold of the lock, taking the lock (and
    /// starting the heartbeat) when nothing of this worker holds it.
    /// `Ok(None)` means another worker holds it.
    pub async fn try_hold(&self) -> io::Result<Option<WorkerHold>> {
        let mut held = self.inner.held.lock().await;
        if let Some(existing) = held.upgrade() {
            return Ok(Some(WorkerHold {
                held: existing,
                lock: self.clone(),
            }));
        }
        let Some(lock) = CycleLock::try_acquire(&self.inner.path)? else {
            return Ok(None);
        };
        let beat = Heartbeat::start(
            self.inner.heartbeat.clone(),
            self.inner.clock.clone(),
            self.inner.every,
        )
        .await;
        let new = Arc::new(Held {
            beat: Mutex::new(Some(beat)),
            _lock: lock,
        });
        *held = Arc::downgrade(&new);
        Ok(Some(WorkerHold {
            held: new,
            lock: self.clone(),
        }))
    }
}

impl Clone for WorkerHold {
    fn clone(&self) -> Self {
        WorkerHold {
            held: self.held.clone(),
            lock: self.lock.clone(),
        }
    }
}

impl WorkerHold {
    /// Lets go of this handle. The last one writes the heartbeat's last beat
    /// and then lets go of the lock.
    pub async fn release(self) {
        let _one_at_a_time = self.lock.inner.held.lock().await;
        if Arc::strong_count(&self.held) == 1 {
            let beat = self
                .held
                .beat
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if let Some(beat) = beat {
                beat.stop().await;
            }
        }
        drop(self.held);
    }

    /// Something that keeps the lock held for as long as it lives, for work
    /// that must keep it until it returns even when the task that waits for
    /// it is aborted (a blocking rename). It counts as a handle, but dropping
    /// it never writes the last beat: the handle it was made from does.
    pub fn keep(&self) -> Arc<dyn Any + Send + Sync> {
        self.held.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_path_sits_next_to_the_database() {
        assert_eq!(
            lock_path_for(Path::new("/data/trss/app.db")),
            Path::new("/data/trss/app.db.worker.lock")
        );
    }

    #[test]
    fn a_second_holder_is_refused_until_the_first_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db.worker.lock");

        let first = CycleLock::try_acquire(&path).unwrap().expect("free lock");
        assert!(CycleLock::try_acquire(&path).unwrap().is_none());
        assert!(CycleLock::try_acquire(&path).unwrap().is_none());

        drop(first);
        assert!(CycleLock::try_acquire(&path).unwrap().is_some());
    }

    #[test]
    fn a_dropped_guard_is_free_even_while_a_copy_of_its_descriptor_lives_on() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db.worker.lock");

        let first = CycleLock::try_acquire(&path).unwrap().expect("free lock");
        // What a child process being started holds until its `exec`.
        let copy = first.file.try_clone().unwrap();
        drop(first);
        assert!(CycleLock::try_acquire(&path).unwrap().is_some());
        drop(copy);
    }

    #[test]
    fn a_missing_directory_is_an_error_not_a_free_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("app.db.worker.lock");
        assert!(CycleLock::try_acquire(&path).is_err());
    }

    mod shared {
        use std::sync::atomic::{AtomicI64, Ordering};

        use super::*;
        use crate::Db;

        struct Scene {
            _dir: tempfile::TempDir,
            path: PathBuf,
            heartbeat: HeartbeatStore,
            now: Arc<AtomicI64>,
        }

        impl Scene {
            async fn new() -> Scene {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("app.db.worker.lock");
                let db = Db::open(dir.path().join("app.db")).await.unwrap();
                Scene {
                    _dir: dir,
                    path,
                    heartbeat: HeartbeatStore::new(db),
                    now: Arc::new(AtomicI64::new(1_000)),
                }
            }

            /// A worker's lock, as a worker builds it.
            fn worker(&self) -> WorkerLock {
                let now = self.now.clone();
                WorkerLock::new(
                    self.path.clone(),
                    self.heartbeat.clone(),
                    Arc::new(move || now.load(Ordering::SeqCst)),
                    Duration::from_secs(3600),
                )
            }

            async fn held_since(&self) -> Option<i64> {
                self.heartbeat
                    .read()
                    .await
                    .unwrap()
                    .and_then(|b| b.held_since)
            }
        }

        #[tokio::test]
        async fn one_workers_work_shares_the_hold_and_another_worker_waits() {
            let s = Scene::new().await;
            let (worker, other) = (s.worker(), s.worker());

            let cycle = worker.try_hold().await.unwrap().expect("a free lock");
            let command = worker.clone().try_hold().await.unwrap();
            assert!(command.is_some(), "the worker's own work shares its hold");
            assert!(other.try_hold().await.unwrap().is_none());
            // Nor does a bare lock on the file get in.
            assert!(CycleLock::try_acquire(&s.path).unwrap().is_none());

            // Letting go of one handle keeps the hold for the other.
            cycle.release().await;
            assert!(other.try_hold().await.unwrap().is_none());

            command.unwrap().release().await;
            let theirs = other
                .try_hold()
                .await
                .unwrap()
                .expect("free once all let go");
            assert!(worker.try_hold().await.unwrap().is_none());
            theirs.release().await;
        }

        #[tokio::test]
        async fn the_heartbeat_holds_from_the_first_handle_until_the_last_lets_go() {
            let s = Scene::new().await;
            let worker = s.worker();
            assert_eq!(s.held_since().await, None);

            let first = worker.try_hold().await.unwrap().unwrap();
            assert_eq!(s.held_since().await, Some(1_000));
            s.now.store(5_000, Ordering::SeqCst);
            let second = worker.try_hold().await.unwrap().unwrap();
            // Still the hold that began first.
            assert_eq!(s.held_since().await, Some(1_000));

            first.release().await;
            assert_eq!(s.held_since().await, Some(1_000));
            second.release().await;
            assert_eq!(s.held_since().await, None);

            // A new hold is a new start.
            let again = worker.try_hold().await.unwrap().unwrap();
            assert_eq!(s.held_since().await, Some(5_000));
            again.release().await;
        }

        #[tokio::test]
        async fn a_last_handle_dropped_without_letting_go_frees_the_lock_but_leaves_the_hold() {
            let s = Scene::new().await;
            let worker = s.worker();
            let hold = worker.try_hold().await.unwrap().unwrap();

            // Its task was aborted: the lock is free, and no last beat clears
            // the hold, so the web sees a worker that stopped.
            drop(hold);
            assert_eq!(s.held_since().await, Some(1_000));
            assert!(CycleLock::try_acquire(&s.path).unwrap().is_some());
        }

        #[tokio::test]
        async fn what_keeps_the_lock_outlives_its_handle() {
            let s = Scene::new().await;
            let worker = s.worker();
            let hold = worker.try_hold().await.unwrap().unwrap();
            let kept = hold.keep();

            hold.release().await;
            assert!(s.worker().try_hold().await.unwrap().is_none());
            // The worker's own work still shares it.
            let mine = worker.try_hold().await.unwrap().expect("still held");
            mine.release().await;
            assert!(s.worker().try_hold().await.unwrap().is_none());

            drop(kept);
            assert!(s.worker().try_hold().await.unwrap().is_some());
        }
    }
}
