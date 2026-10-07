//! What the worker's background queues share (`docs/adr/0016-shared-parts-in-core.md`).
//!
//! A queue runs in a task of its own and takes one item at a time. A panic in
//! one item would end that task, and the queue would stand still until the
//! worker restarted, so each item runs through [`run_item`], which catches the
//! panic and hands it back to the queue to put the item off like a failure.

use std::{any::Any, future::Future, panic::AssertUnwindSafe};

use futures::FutureExt;

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
    use super::*;

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
