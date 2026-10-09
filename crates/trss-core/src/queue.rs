//! What the worker's background queues share (`docs/adr/0016-shared-parts-in-core.md`).
//!
//! A queue runs in a task of its own and takes one item at a time. [`Queue`]
//! is the loop of that task: take the queue's lock file, then look for the
//! next item, and look again after a pause when there is none. The queue
//! hands over what it does to find and run an item, and its own intervals.
//!
//! A panic in one item would end that task, and the queue would stand still
//! until the worker restarted, so each item runs through [`run_item`], which
//! catches the panic and hands it back to the queue to put the item off like
//! a failure.

use std::{any::Any, future::Future, panic::AssertUnwindSafe, path::PathBuf, time::Duration};

use futures::FutureExt;
use tokio_util::sync::CancellationToken;

use crate::CycleLock;

/// How often an idle queue looks for new items.
pub const POLL: Duration = Duration::from_secs(5);
/// How long a queue that finds another process running it waits to look again.
pub const LOCK_RETRY: Duration = Duration::from_secs(60);

/// The loop of one background queue: it runs under the queue's lock file
/// (one of the queue locks of [`crate::LockFile`]) until cancelled.
#[derive(Debug, Clone)]
pub struct Queue {
    name: &'static str,
    lock_path: PathBuf,
    poll: Duration,
    lock_retry: Duration,
}

impl Queue {
    /// The queue `name` (the name its log lines use) under the lock file
    /// `lock_path`. An idle queue looks for an item every `poll`, and one that
    /// finds the lock taken tries it again after `lock_retry`.
    pub fn new(
        name: &'static str,
        lock_path: PathBuf,
        poll: Duration,
        lock_retry: Duration,
    ) -> Queue {
        Queue {
            name,
            lock_path,
            poll,
            lock_retry,
        }
    }

    /// Runs the queue until `cancel` fires. While another process holds the
    /// lock it only waits; with the lock it calls `next` over and over, at once
    /// after an item and after the poll interval after `None`. An item cut
    /// short by `cancel` is dropped where it stands, so it must be one that
    /// the next start finds again.
    pub async fn run<T, N, NF>(&self, cancel: &CancellationToken, next: N)
    where
        N: FnMut() -> NF,
        NF: Future<Output = Option<T>>,
    {
        // Without upkeep: one that is never due, and does nothing.
        self.run_with_upkeep(cancel, Duration::MAX, || async {}, next)
            .await;
    }

    /// [`Queue::run`] for a queue with upkeep to do besides its items: `upkeep`
    /// runs as soon as the lock is taken and then every `upkeep_every`, between
    /// two items. It is not cut short by `cancel`; the queue stops after it.
    pub async fn run_with_upkeep<T, U, UF, N, NF>(
        &self,
        cancel: &CancellationToken,
        upkeep_every: Duration,
        mut upkeep: U,
        mut next: N,
    ) where
        U: FnMut() -> UF,
        UF: Future<Output = ()>,
        N: FnMut() -> NF,
        NF: Future<Output = Option<T>>,
    {
        let name = self.name;
        loop {
            // Every queue keeps its own lock file although one worker runs at
            // a time: the worker lock stops a second worker process only
            // from running its cycles, and that process still starts and runs
            // its queues. The queue's lock is what keeps the two from running
            // the same queue side by side (user decision, 2026-10-08).
            let lock = match CycleLock::try_acquire(&self.lock_path) {
                Ok(lock) => lock,
                Err(e) => {
                    eprintln!("{name}: cannot take {}: {e}", self.lock_path.display());
                    None
                }
            };
            let Some(_lock) = lock else {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(self.lock_retry) => continue,
                }
            };
            upkeep().await;
            let mut upkept = tokio::time::Instant::now();
            loop {
                if upkept.elapsed() >= upkeep_every {
                    upkeep().await;
                    upkept = tokio::time::Instant::now();
                }
                let ran = tokio::select! {
                    _ = cancel.cancelled() => return,
                    ran = next() => ran,
                };
                if ran.is_none() {
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = tokio::time::sleep(self.poll) => {}
                    }
                }
            }
        }
    }
}

/// Runs one item of the queue `queue` (the name its log lines use; `item`
/// says which one). A panic in `work` is caught and logged with both names,
/// and its text comes back as `Err`; the queue then puts the item off as it
/// does a failure, so the same item does not panic again at once.
pub async fn run_item<T>(
    queue: &str,
    item: &str,
    work: impl Future<Output = T>,
) -> Result<T, String> {
    let work = async {
        #[cfg(any(test, feature = "test-support"))]
        testing::panic_if_asked(queue, item);
        work.await
    };
    match AssertUnwindSafe(work).catch_unwind().await {
        Ok(out) => Ok(out),
        Err(payload) => {
            let text = panic_text(payload.as_ref());
            eprintln!("{}", panic_line(queue, item, &text));
            Err(text)
        }
    }
}

/// The message a panic was raised with.
fn panic_text(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "no message".to_owned()
    }
}

fn panic_line(queue: &str, item: &str, text: &str) -> String {
    format!("{queue}: {item} panicked: {text}")
}

/// Panics on request, for the tests of the queues above.
#[cfg(any(test, feature = "test-support"))]
pub mod testing {
    use std::cell::RefCell;

    thread_local! {
        static ASKED: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
    }

    /// The next [`super::run_item`] of `item` in `queue` on this thread
    /// panics, once. A `#[tokio::test]` runs its tasks on its own thread.
    pub fn panic_next(queue: &str, item: &str) {
        ASKED.with(|asked| asked.borrow_mut().push((queue.to_owned(), item.to_owned())));
    }

    pub(super) fn panic_if_asked(queue: &str, item: &str) {
        let asked = ASKED.with(|asked| {
            let mut asked = asked.borrow_mut();
            let at = asked.iter().position(|(q, i)| q == queue && i == item);
            at.map(|at| asked.remove(at))
        });
        if asked.is_some() {
            panic!("a test asked {item} of {queue} to panic");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
    };

    use super::*;

    const SHORT: Duration = Duration::from_millis(20);
    const HOUR: Duration = Duration::from_secs(3600);

    fn queue(lock: &Path, poll: Duration, lock_retry: Duration) -> Queue {
        Queue::new("Test queue", lock.to_owned(), poll, lock_retry)
    }

    /// Runs `queue` over `next`, which says `Some` for the first `items` calls
    /// and `None` after. Returns the count of calls so far and the task.
    fn spawn_counting(
        queue: &Queue,
        cancel: &CancellationToken,
        items: usize,
    ) -> (Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn({
            let (queue, cancel, calls) = (queue.clone(), cancel.clone(), calls.clone());
            async move {
                queue
                    .run(&cancel, || {
                        let n = calls.fetch_add(1, Ordering::SeqCst);
                        async move { (n < items).then_some(n) }
                    })
                    .await;
            }
        });
        (calls, task)
    }

    async fn stops(task: tokio::task::JoinHandle<()>) {
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the queue did not stop on cancel")
            .unwrap();
    }

    #[tokio::test]
    async fn a_queue_holds_its_lock_while_it_runs_and_lets_go_when_it_stops() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("q.lock");
        let cancel = CancellationToken::new();
        let (calls, task) = spawn_counting(&queue(&lock, SHORT, HOUR), &cancel, 0);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(calls.load(Ordering::SeqCst) > 0);
        assert!(
            CycleLock::try_acquire(&lock).unwrap().is_none(),
            "the lock was free while the queue ran"
        );
        cancel.cancel();
        stops(task).await;
        assert!(CycleLock::try_acquire(&lock).unwrap().is_some());
    }

    #[tokio::test]
    async fn an_item_is_followed_by_the_next_at_once_and_an_empty_look_by_a_pause() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("q.lock");
        let cancel = CancellationToken::new();
        // The poll interval is an hour: five items in a row cannot have waited
        // for it, and the look that finds none is the sixth.
        let (calls, task) = spawn_counting(&queue(&lock, HOUR, HOUR), &cancel, 5);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 6);
        cancel.cancel();
        stops(task).await;
    }

    #[tokio::test]
    async fn an_idle_queue_looks_again_after_the_poll_interval() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("q.lock");
        let cancel = CancellationToken::new();
        let (calls, task) = spawn_counting(&queue(&lock, SHORT, HOUR), &cancel, 0);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let looks = calls.load(Ordering::SeqCst);
        assert!((3..=16).contains(&looks), "{looks} looks in 300 ms");
        cancel.cancel();
        stops(task).await;
    }

    #[tokio::test]
    async fn a_queue_whose_lock_is_taken_elsewhere_waits_and_starts_when_it_is_let_go() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("q.lock");
        let held = CycleLock::try_acquire(&lock).unwrap().unwrap();
        let cancel = CancellationToken::new();
        let (calls, task) = spawn_counting(&queue(&lock, SHORT, SHORT), &cancel, 0);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0, "it ran without its lock");

        drop(held);
        for _ in 0..100 {
            if calls.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(calls.load(Ordering::SeqCst) > 0, "it did not start");
        cancel.cancel();
        stops(task).await;
    }

    #[tokio::test]
    async fn two_queues_on_one_lock_file_do_not_run_side_by_side() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("q.lock");
        let cancel = CancellationToken::new();
        let (first_calls, first) = spawn_counting(&queue(&lock, SHORT, SHORT), &cancel, 0);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (second_calls, second) = spawn_counting(&queue(&lock, SHORT, SHORT), &cancel, 0);
        let before = first_calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(before > 0);
        assert!(first_calls.load(Ordering::SeqCst) > before);
        assert_eq!(second_calls.load(Ordering::SeqCst), 0);
        cancel.cancel();
        stops(first).await;
        stops(second).await;
    }

    #[tokio::test]
    async fn a_lock_file_that_cannot_be_opened_is_tried_again_like_one_taken_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("missing").join("q.lock");
        let cancel = CancellationToken::new();
        let (calls, task) = spawn_counting(&queue(&lock, SHORT, SHORT), &cancel, 0);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        // The folder shows up (a volume mounted late): the next try works.
        std::fs::create_dir(dir.path().join("missing")).unwrap();
        for _ in 0..100 {
            if calls.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(calls.load(Ordering::SeqCst) > 0);
        cancel.cancel();
        stops(task).await;
    }

    #[tokio::test]
    async fn cancel_ends_a_queue_in_each_of_its_waits() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("q.lock");

        // In the pause after an empty look.
        let cancel = CancellationToken::new();
        let (calls, task) = spawn_counting(&queue(&lock, HOUR, HOUR), &cancel, 0);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cancel.cancel();
        stops(task).await;

        // In the wait for a lock held elsewhere.
        let held = CycleLock::try_acquire(&lock).unwrap().unwrap();
        let cancel = CancellationToken::new();
        let (_, task) = spawn_counting(&queue(&lock, HOUR, HOUR), &cancel, 0);
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
        stops(task).await;
        drop(held);

        // In an item that does not end.
        let cancel = CancellationToken::new();
        let task = tokio::spawn({
            let (queue, cancel) = (queue(&lock, HOUR, HOUR), cancel.clone());
            async move {
                queue.run(&cancel, std::future::pending::<Option<()>>).await;
            }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
        stops(task).await;
        assert!(CycleLock::try_acquire(&lock).unwrap().is_some());
    }

    #[tokio::test]
    async fn upkeep_runs_once_the_lock_is_taken_and_then_every_period_between_items() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("q.lock");
        let held = CycleLock::try_acquire(&lock).unwrap().unwrap();
        let events = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let cancel = CancellationToken::new();
        let task = tokio::spawn({
            let (queue, cancel, events) =
                (queue(&lock, SHORT, SHORT), cancel.clone(), events.clone());
            async move {
                let (for_upkeep, for_items) = (events.clone(), events);
                queue
                    .run_with_upkeep(
                        &cancel,
                        Duration::from_millis(100),
                        || {
                            let events = for_upkeep.clone();
                            async move { events.lock().unwrap().push("upkeep") }
                        },
                        || {
                            let events = for_items.clone();
                            async move {
                                events.lock().unwrap().push("item");
                                None::<()>
                            }
                        },
                    )
                    .await;
            }
        });
        // Without the lock there is no upkeep either.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(events.lock().unwrap().is_empty());

        drop(held);
        tokio::time::sleep(Duration::from_millis(450)).await;
        cancel.cancel();
        stops(task).await;
        let events = events.lock().unwrap().clone();
        assert_eq!(events.first(), Some(&"upkeep"), "{events:?}");
        let upkeeps = events.iter().filter(|e| **e == "upkeep").count();
        assert!((2..=6).contains(&upkeeps), "{upkeeps} upkeeps: {events:?}");
        assert!(events.contains(&"item"), "{events:?}");
    }

    #[tokio::test]
    async fn upkeep_that_cancel_finds_under_way_is_finished_before_the_queue_stops() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("q.lock");
        let cancel = CancellationToken::new();
        let finished = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn({
            let (queue, cancel) = (queue(&lock, SHORT, SHORT), cancel.clone());
            let finished = finished.clone();
            async move {
                queue
                    .run_with_upkeep(
                        &cancel,
                        HOUR,
                        || {
                            let finished = finished.clone();
                            async move {
                                tokio::time::sleep(Duration::from_millis(200)).await;
                                finished.fetch_add(1, Ordering::SeqCst);
                            }
                        },
                        || async { None::<()> },
                    )
                    .await;
            }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
        stops(task).await;
        assert_eq!(finished.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_item_that_panics_is_told_with_its_queue_and_the_next_one_runs() {
        let out = run_item("Test queue", "item 1", async { 7 }).await;
        assert_eq!(out, Ok(7));
        let out: Result<(), String> =
            run_item("Test queue", "item 2", async { panic!("broken {}", 2) }).await;
        assert_eq!(out, Err("broken 2".to_owned()));
        let out: Result<(), String> = run_item("Test queue", "item 3", async {
            std::panic::panic_any(3_u8)
        })
        .await;
        assert_eq!(out, Err("no message".to_owned()));
        assert_eq!(run_item("Test queue", "item 4", async { 8 }).await, Ok(8));
        assert_eq!(
            panic_line("Test queue", "item 2", "broken 2"),
            "Test queue: item 2 panicked: broken 2"
        );
    }

    #[tokio::test]
    async fn a_test_panic_comes_once_for_its_queue_and_item() {
        testing::panic_next("Test queue", "item 1");
        assert_eq!(run_item("Other queue", "item 1", async { 1 }).await, Ok(1));
        assert_eq!(run_item("Test queue", "item 2", async { 2 }).await, Ok(2));
        assert!(run_item("Test queue", "item 1", async { 1 }).await.is_err());
        assert_eq!(run_item("Test queue", "item 1", async { 1 }).await, Ok(1));
    }
}
