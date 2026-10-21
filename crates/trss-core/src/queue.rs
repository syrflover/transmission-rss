//! What the worker's background queues share (`docs/adr/0016-shared-parts-in-core.md`).
//!
//! A queue runs in a task of its own and takes one item at a time. [`Queue`]
//! is the loop of that task: look for the next item, and look again after a
//! pause when there is none. The queue hands over what it does to find and
//! run an item, and its own intervals. One process runs on an app database
//! ([`crate::InstanceLock`]), and a [`Queue`] runs its loop once at a time, so
//! one task runs each queue.
//!
//! A panic in one item would end that task, and the queue would stand still
//! until the worker restarted, so each item runs through [`run_item`], which
//! catches the panic and hands it back to the queue to put the item off like
//! a failure. [`retry`] and [`after`] are the arithmetic of putting an item off.

use std::{
    any::Any,
    future::Future,
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use futures::FutureExt;
use tokio_util::sync::CancellationToken;

use crate::Millis;

/// How often an idle queue looks for new items.
pub const POLL: Duration = Duration::from_secs(5);

/// The loop of one background queue, until cancelled. Cheap to clone; clones
/// are the same queue, and only one of them runs the loop at a time. Two
/// `Queue::new` of one name are two queues: the process makes each queue
/// that runs once (`trss`'s `main`), so tests can make their own.
#[derive(Debug, Clone)]
pub struct Queue {
    name: &'static str,
    poll: Duration,
    /// Whether the loop runs now ([`Queue::run`]).
    running: Arc<AtomicBool>,
}

/// Lets the queue's loop be run again once the run that holds it ends,
/// however it ends.
struct Running(Arc<AtomicBool>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Queue {
    /// The queue `name` (the name its log lines use). An idle queue looks for
    /// an item every `poll`.
    pub fn new(name: &'static str, poll: Duration) -> Queue {
        Queue {
            name,
            poll,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Runs the queue until `cancel` fires: calls `next` over and over, at
    /// once after an item and after the poll interval after `None`. A second
    /// run of the same queue while one runs returns at once and says so. An
    /// item cut short by `cancel` is dropped where it stands, so it must be
    /// one that the next start finds again.
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
    /// runs as soon as the queue starts and then every `upkeep_every`, between
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
        if self.running.swap(true, Ordering::SeqCst) {
            eprintln!("{}: runs already in this process", self.name);
            return;
        }
        let _running = Running(self.running.clone());
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

/// When an item that could not be done is tried again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retry {
    /// The moment of the next try; `None` when the item is given up.
    pub at: Option<Millis>,
    /// Whether this counts as a failed attempt (a wait the service asked for
    /// does not).
    pub failed: bool,
}

/// How an item is put off at `now`: for `busy` when the service asked to wait
/// (a `429`; not a failure and not counted against the item), else by the
/// delay in `delays` for the item's `attempts` so far (`delays[0]` after the
/// first failed attempt), a failure; after the last delay it is given up.
pub fn retry(now: Millis, attempts: usize, delays: &[Duration], busy: Option<Duration>) -> Retry {
    match busy {
        Some(wait) => Retry {
            at: Some(after(now, wait)),
            failed: false,
        },
        None => Retry {
            at: delays.get(attempts).map(|delay| after(now, *delay)),
            failed: true,
        },
    }
}

/// The moment `wait` after `now`.
pub fn after(now: Millis, wait: Duration) -> Millis {
    now + wait.as_millis() as Millis
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
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    use super::*;

    const SHORT: Duration = Duration::from_millis(20);
    const HOUR: Duration = Duration::from_secs(3600);

    fn queue(poll: Duration) -> Queue {
        Queue::new("Test queue", poll)
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
    async fn a_second_run_of_one_queue_returns_at_once_while_the_first_goes_on() {
        let queue = queue(SHORT);
        let cancel = CancellationToken::new();
        let (first_calls, first) = spawn_counting(&queue, &cancel, 0);
        tokio::time::sleep(Duration::from_millis(100)).await;

        // A clone is the same queue.
        let (second_calls, second) = spawn_counting(&queue.clone(), &cancel, 0);
        stops(second).await;
        assert_eq!(second_calls.load(Ordering::SeqCst), 0);

        let before = first_calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(before > 0);
        assert!(first_calls.load(Ordering::SeqCst) > before);
        cancel.cancel();
        stops(first).await;
    }

    #[tokio::test]
    async fn a_queue_runs_again_once_its_run_stopped_or_was_aborted() {
        let queue = queue(SHORT);

        let cancel = CancellationToken::new();
        let (_, task) = spawn_counting(&queue, &cancel, 0);
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
        stops(task).await;

        let cancel = CancellationToken::new();
        let (calls, task) = spawn_counting(&queue, &cancel, 0);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(calls.load(Ordering::SeqCst) > 0, "it did not run again");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());

        let (calls, task) = spawn_counting(&queue, &cancel, 0);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            calls.load(Ordering::SeqCst) > 0,
            "it did not run after an abort"
        );
        cancel.cancel();
        stops(task).await;
    }

    #[tokio::test]
    async fn an_item_is_followed_by_the_next_at_once_and_an_empty_look_by_a_pause() {
        let cancel = CancellationToken::new();
        // The poll interval is an hour: five items in a row cannot have waited
        // for it, and the look that finds none is the sixth.
        let (calls, task) = spawn_counting(&queue(HOUR), &cancel, 5);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 6);
        cancel.cancel();
        stops(task).await;
    }

    #[tokio::test]
    async fn an_idle_queue_looks_again_after_the_poll_interval() {
        let cancel = CancellationToken::new();
        let (calls, task) = spawn_counting(&queue(SHORT), &cancel, 0);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let looks = calls.load(Ordering::SeqCst);
        assert!((3..=16).contains(&looks), "{looks} looks in 300 ms");
        cancel.cancel();
        stops(task).await;
    }

    #[tokio::test]
    async fn cancel_ends_a_queue_in_each_of_its_waits() {
        // In the pause after an empty look.
        let cancel = CancellationToken::new();
        let (calls, task) = spawn_counting(&queue(HOUR), &cancel, 0);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cancel.cancel();
        stops(task).await;

        // In an item that does not end.
        let cancel = CancellationToken::new();
        let task = tokio::spawn({
            let (queue, cancel) = (queue(HOUR), cancel.clone());
            async move {
                queue.run(&cancel, std::future::pending::<Option<()>>).await;
            }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
        stops(task).await;
    }

    #[tokio::test]
    async fn upkeep_runs_at_the_start_and_then_every_period_between_items() {
        let events = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let cancel = CancellationToken::new();
        let task = tokio::spawn({
            let (queue, cancel, events) = (queue(SHORT), cancel.clone(), events.clone());
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
        let cancel = CancellationToken::new();
        let finished = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn({
            let (queue, cancel) = (queue(SHORT), cancel.clone());
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

    #[test]
    fn an_item_is_put_off_by_the_next_delay_or_by_the_wait_it_was_asked_for() {
        let delays = [
            Duration::from_secs(60),
            Duration::from_secs(600),
            Duration::from_secs(3600),
        ];
        let now = 1_000;
        let failed = |at| Retry {
            at: Some(now + at),
            failed: true,
        };
        assert_eq!(retry(now, 0, &delays, None), failed(60_000));
        assert_eq!(retry(now, 1, &delays, None), failed(600_000));
        assert_eq!(retry(now, 2, &delays, None), failed(3_600_000));
        // After the last delay the item is given up, still as a failure.
        assert_eq!(
            retry(now, 3, &delays, None),
            Retry {
                at: None,
                failed: true
            }
        );
        // The wait a service asks for is no failure, whatever the attempts.
        let asked = Retry {
            at: Some(now + 90_500),
            failed: false,
        };
        let wait = Some(Duration::from_millis(90_500));
        assert_eq!(retry(now, 0, &delays, wait), asked);
        assert_eq!(retry(now, 9, &delays, wait), asked);
        assert_eq!(retry(now, 0, &[], None).at, None);
    }

    #[test]
    fn after_adds_the_wait_in_milliseconds() {
        assert_eq!(after(5, Duration::from_millis(1_500)), 1_505);
        assert_eq!(after(5, Duration::ZERO), 5);
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
