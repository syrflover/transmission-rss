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
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
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
/// handle is let go, which writes the last beat (clearing the hold) before the
/// lock is let go. [`WorkerHold::release`] does that in place. When the last
/// one goes some other way (a handle dropped because its task panicked or was
/// aborted, what [`WorkerHold::keep`] gave outliving the handles, or a hold
/// given up while its first beat was being written), a task
/// writes the last beat and then lets go of the lock; a hold taken meanwhile
/// waits for it, from the moment the last handle goes on whatever thread, and
/// a hold given up while it waits leaves the waiting to the next. Only without a runtime to run that task (the process is
/// ending) are the beats merely stopped, so the web sees a worker that
/// stopped.
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
    /// The task that ends a hold whose last handle went without
    /// [`WorkerHold::release`] (see [`Held`]'s `Drop`). A new hold waits
    /// for it, so the last beat of the old hold never lands after the first
    /// of the new one.
    closing: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Holds not yet dropped to the end. The last handle of a hold drops it
    /// on that handle's thread, and from the moment the hold can no longer
    /// be shared until its `Drop` has stored its ending task in
    /// [`Shared::closing`], neither the hold nor the task can be seen: a new
    /// hold waits for this to come back to zero.
    unfinished: AtomicUsize,
}

/// The lock while the worker holds it.
struct Held {
    /// `None` once the last beat is written.
    beat: Mutex<Option<Heartbeat>>,
    /// Always `Some` until the hold is dropped.
    lock: Option<CycleLock>,
    shared: Weak<Shared>,
}

impl Drop for Held {
    fn drop(&mut self) {
        let beat = self
            .beat
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let lock = self.lock.take();
        let task = match (beat, tokio::runtime::Handle::try_current()) {
            // The lock goes with the task and is let go only after the last
            // beat.
            (Some(beat), Ok(runtime)) => Some(runtime.spawn(async move {
                beat.stop().await;
                drop(lock);
            })),
            // The last handle was released, and the last beat is written; or
            // there is no runtime to write it.
            (beat, _) => {
                drop(beat);
                drop(lock);
                None
            }
        };
        if let Some(shared) = self.shared.upgrade() {
            if let Some(task) = task {
                *shared.closing.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
            }
            shared.unfinished.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// The ending task of the last hold while a new hold waits for it. When the
/// waiting is given up (the task that waits is aborted), the ending task goes
/// back to [`Shared::closing`] for the next hold to wait for.
struct Ending<'a> {
    task: Option<tokio::task::JoinHandle<()>>,
    slot: &'a Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Ending<'_> {
    async fn wait(mut self) {
        if let Some(task) = self.task.as_mut() {
            let _ = task.await;
        }
        self.task = None;
    }
}

/// A hold being taken while its first beat is written. When the taking is
/// given up there (the task that takes it is aborted), the beat may land all
/// the same and say the lock is held: a task waits for it, writes the last
/// beat and only then lets go of the lock, as when a hold ends without
/// [`WorkerHold::release`], and the next hold waits for that task.
struct Starting<'a> {
    task: Option<tokio::task::JoinHandle<Heartbeat>>,
    lock: Option<CycleLock>,
    slot: &'a Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Starting<'_> {
    async fn wait(mut self) -> io::Result<(Heartbeat, CycleLock)> {
        let started = match self.task.as_mut() {
            Some(task) => task.await,
            None => unreachable!("the start is waited for once"),
        };
        self.task = None;
        let lock = self
            .lock
            .take()
            .expect("the lock is kept until the start ends");
        match started {
            Ok(beat) => Ok((beat, lock)),
            Err(err) => Err(io::Error::other(err)),
        }
    }
}

impl Drop for Starting<'_> {
    fn drop(&mut self) {
        let (Some(task), lock) = (self.task.take(), self.lock.take()) else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let ending = runtime.spawn(async move {
            if let Ok(beat) = task.await {
                beat.stop().await;
            }
            drop(lock);
        });
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(earlier) = slot.replace(ending) {
            // Not expected: the hold that was being taken waited for any
            // earlier ending first.
            drop(earlier);
        }
    }
}

impl Drop for Ending<'_> {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
            if slot.is_none() {
                *slot = Some(task);
            }
        }
    }
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
                closing: Mutex::new(None),
                unfinished: AtomicUsize::new(0),
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
        // The last hold may be ending: dropped on another thread just now,
        // or its ending task still writing the last beat. Its lock is let go
        // only then, and the file is not refused for it in the meantime.
        for _ in 0..1000 {
            if self.inner.unfinished.load(Ordering::SeqCst) == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let task = self
            .inner
            .closing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        Ending {
            task,
            slot: &self.inner.closing,
        }
        .wait()
        .await;
        let Some(lock) = CycleLock::try_acquire(&self.inner.path)? else {
            return Ok(None);
        };
        // The first beat is written by a task of its own, so that a hold given
        // up meanwhile (its task aborted) still ends it (see [`Starting`]).
        let start = tokio::spawn(Heartbeat::start(
            self.inner.heartbeat.clone(),
            self.inner.clock.clone(),
            self.inner.every,
        ));
        let (beat, lock) = Starting {
            task: Some(start),
            lock: Some(lock),
            slot: &self.inner.closing,
        }
        .wait()
        .await?;
        self.inner.unfinished.fetch_add(1, Ordering::SeqCst);
        let new = Arc::new(Held {
            beat: Mutex::new(Some(beat)),
            lock: Some(lock),
            shared: Arc::downgrade(&self.inner),
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
    /// it is aborted (a blocking rename). It counts as a handle; when it is
    /// the last to go, the last beat is written by a task (see
    /// [`WorkerLock`]).
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

            /// Waits until the last beat cleared the hold.
            async fn wait_cleared(&self) {
                for _ in 0..500 {
                    if self.held_since().await.is_none() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                panic!("the hold was never cleared");
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
        async fn a_last_handle_dropped_without_letting_go_still_ends_the_hold() {
            let s = Scene::new().await;
            let worker = s.worker();
            let hold = worker.try_hold().await.unwrap().unwrap();

            // Its task was aborted: the last beat is written all the same,
            // and then the lock is free.
            drop(hold);
            s.wait_cleared().await;
            assert!(CycleLock::try_acquire(&s.path).unwrap().is_some());
        }

        #[tokio::test]
        async fn what_keeps_the_lock_ends_the_hold_when_it_outlives_the_handles() {
            let s = Scene::new().await;
            let worker = s.worker();
            let hold = worker.try_hold().await.unwrap().unwrap();
            let kept = hold.keep();

            hold.release().await;
            assert_eq!(s.held_since().await, Some(1_000), "still held");

            // The blocking work returns after its task was let go.
            s.now.store(9_000, Ordering::SeqCst);
            drop(kept);
            s.wait_cleared().await;
            assert!(CycleLock::try_acquire(&s.path).unwrap().is_some());

            // A new hold starts anew.
            let again = worker.try_hold().await.unwrap().expect("free");
            assert_eq!(s.held_since().await, Some(9_000));
            again.release().await;
            assert_eq!(s.held_since().await, None);
        }

        #[tokio::test]
        async fn a_hold_taken_while_the_last_one_ends_waits_for_its_last_beat() {
            let s = Scene::new().await;
            let worker = s.worker();
            let hold = worker.try_hold().await.unwrap().unwrap();
            drop(hold);

            // At once, before the ending task has run: not refused, and the
            // new hold's beat is not cleared by the old one's last.
            s.now.store(7_000, Ordering::SeqCst);
            let again = worker
                .try_hold()
                .await
                .unwrap()
                .expect("waits, not refused");
            assert_eq!(s.held_since().await, Some(7_000));
            again.release().await;
        }

        #[tokio::test]
        async fn a_hold_given_up_while_the_last_one_ends_leaves_the_wait_to_the_next() {
            let s = Scene::new().await;
            let worker = s.worker();
            drop(worker.try_hold().await.unwrap().unwrap());

            // A hold waiting for the ending task is given up there (its task
            // was aborted) before that task has run.
            let mut waiting = Box::pin(worker.try_hold());
            let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(std::future::Future::poll(waiting.as_mut(), &mut cx).is_pending());
            drop(waiting);

            // The next one waits for the ending task all the same.
            let again = worker
                .try_hold()
                .await
                .unwrap()
                .expect("waits, not refused");
            again.release().await;
        }

        #[tokio::test]
        async fn a_hold_taken_while_the_last_one_is_dropped_on_another_thread_waits_for_it() {
            let s = Scene::new().await;
            let worker = s.worker();
            // The last handle of a hold has gone on another thread, whose drop
            // of the hold has not yet let go of the lock or stored its ending
            // task: the hold can no longer be shared, and the lock is taken.
            let lock = CycleLock::try_acquire(&s.path).unwrap().unwrap();
            worker.inner.unfinished.fetch_add(1, Ordering::SeqCst);
            let shared = worker.inner.clone();
            let dropping = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                drop(lock);
                shared.unfinished.fetch_sub(1, Ordering::SeqCst);
            });

            let hold = worker
                .try_hold()
                .await
                .unwrap()
                .expect("waits, not refused");
            hold.release().await;
            dropping.join().unwrap();
        }

        #[tokio::test]
        async fn a_hold_given_up_while_its_first_beat_is_written_clears_it_before_the_lock_goes() {
            let s = Scene::new().await;
            let worker = s.worker();

            // Given up (its task aborted) while the first beat is on its way.
            let mut taking = Box::pin(worker.try_hold());
            let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(std::future::Future::poll(taking.as_mut(), &mut cx).is_pending());
            drop(taking);

            // The lock goes only once the hold the beat said is cleared: when
            // it can be taken, the hold is cleared already.
            // Looked for between any two steps of the tasks that end it.
            let mut free = None;
            for _ in 0..1_000_000 {
                free = CycleLock::try_acquire(&s.path).unwrap();
                if free.is_some() {
                    break;
                }
                tokio::task::yield_now().await;
            }
            assert!(free.is_some(), "the lock was never let go");
            assert_eq!(s.held_since().await, None);
            drop(free);

            // The worker takes its lock again.
            let again = worker.try_hold().await.unwrap().expect("free");
            again.release().await;
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

            // Let go once the last beat is written.
            drop(kept);
            s.wait_cleared().await;
            let theirs = s.worker().try_hold().await.unwrap().expect("free");
            theirs.release().await;
        }
    }
}
