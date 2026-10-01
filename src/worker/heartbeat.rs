//! The worker's heartbeat while it holds the cycle lock.
//!
//! The web cannot ask the worker whether it is alive, and must not look at the
//! cycle lock: even a try-lock from the web could make the worker's own
//! try-lock fail and skip a cycle. So while the worker holds the lock for a
//! cycle (the RSS work, then the watch folder reading and the season link that
//! follow under the same lock) a task writes a timestamp to the database every
//! [`BEAT_EVERY`] ([`crate::store::status::StatusStore::record_heartbeat`]).
//! A timestamp that stops ageing means the worker is busy; one that has aged
//! for a minute means it died or is stopped, whatever the cycle's own marker
//! says. See `crate::web::status_api` for how the web reads it.

use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::Clock;
use crate::store::{history::Millis, status::StatusStore};

/// How often the heartbeat is written. The web calls the worker busy for a
/// minute after a beat, so this leaves room for three missed beats.
pub const BEAT_EVERY: Duration = Duration::from_secs(15);

/// A running heartbeat. Writes the first beat when it starts, one more every
/// [`BEAT_EVERY`], and a last one that clears the hold when [`Heartbeat::stop`]
/// is awaited. Dropping it without that (the cycle panicked or was aborted)
/// only stops the beats, so the timestamp ages and the web sees a worker that
/// has stopped.
pub struct Heartbeat {
    status: StatusStore,
    clock: Clock,
    stop: CancellationToken,
    task: Option<JoinHandle<()>>,
}

impl Heartbeat {
    /// Starts beating for a lock taken now. A beat that cannot be written is
    /// logged and does not fail the cycle: the worst it does is show the worker
    /// as stopped.
    pub async fn start(status: StatusStore, clock: Clock, every: Duration) -> Heartbeat {
        let held_since = clock();
        write(&status, clock(), Some(held_since)).await;

        let stop = CancellationToken::new();
        let task = tokio::spawn({
            let (status, clock, stop) = (status.clone(), clock.clone(), stop.clone());
            async move {
                loop {
                    tokio::select! {
                        biased;
                        _ = stop.cancelled() => break,
                        _ = tokio::time::sleep(every) => {}
                    }
                    write(&status, clock(), Some(held_since)).await;
                }
            }
        });
        Heartbeat {
            status,
            clock,
            stop,
            task: Some(task),
        }
    }

    /// Ends the beats and records that the lock is let go, so the web does not
    /// take the minute after it for a worker that is still busy holding it.
    pub async fn stop(mut self) {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            // Wait for a beat in flight, so that it cannot land after this one.
            let _ = task.await;
        }
        write(&self.status, (self.clock)(), None).await;
    }
}

impl Drop for Heartbeat {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn write(status: &StatusStore, at: Millis, held_since: Option<Millis>) {
    if let Err(err) = status.record_heartbeat(at, held_since).await {
        eprintln!("Cannot record the worker heartbeat: {err}");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    };

    use super::*;
    use crate::store::{status::WorkerHeartbeat, Db};

    /// A clock that moves one second at each reading, so that every beat has its own time.
    fn ticking_clock() -> Clock {
        let now = Arc::new(AtomicI64::new(1_000_000));
        Arc::new(move || now.fetch_add(1_000, Ordering::SeqCst))
    }

    async fn heartbeat_of(status: &StatusStore) -> Option<WorkerHeartbeat> {
        status.heartbeat().await.unwrap()
    }

    #[tokio::test]
    async fn the_first_beat_is_written_before_start_returns() {
        let status = StatusStore::new(Db::open(":memory:").await.unwrap());
        let beat =
            Heartbeat::start(status.clone(), ticking_clock(), Duration::from_secs(3600)).await;

        let written = heartbeat_of(&status).await.expect("a first beat");
        assert_eq!(written.held_since, Some(1_000_000));
        beat.stop().await;
    }

    #[tokio::test]
    async fn it_beats_while_held_and_clears_the_hold_when_stopped() {
        let status = StatusStore::new(Db::open(":memory:").await.unwrap());
        let beat =
            Heartbeat::start(status.clone(), ticking_clock(), Duration::from_millis(10)).await;
        let first = heartbeat_of(&status).await.unwrap();

        // The beat moves on while the lock is held, and the hold keeps its start.
        let mut later = first;
        for _ in 0..200 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            later = heartbeat_of(&status).await.unwrap();
            if later.beat_at > first.beat_at {
                break;
            }
        }
        assert!(later.beat_at > first.beat_at, "{first:?} then {later:?}");
        assert_eq!(later.held_since, first.held_since);

        beat.stop().await;
        let released = heartbeat_of(&status).await.unwrap();
        assert_eq!(released.held_since, None);
        assert!(released.beat_at >= later.beat_at);

        // Nothing beats after the stop.
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(heartbeat_of(&status).await.unwrap(), released);
    }

    #[tokio::test]
    async fn a_dropped_heartbeat_stops_beating_and_keeps_the_hold() {
        let status = StatusStore::new(Db::open(":memory:").await.unwrap());
        let beat =
            Heartbeat::start(status.clone(), ticking_clock(), Duration::from_millis(10)).await;
        tokio::time::sleep(Duration::from_millis(50)).await;

        // The cycle panicked or was aborted: no clean end is written, so the
        // timestamp ages and the web sees a stopped worker.
        drop(beat);
        tokio::time::sleep(Duration::from_millis(30)).await;
        let stopped = heartbeat_of(&status).await.unwrap();
        assert!(stopped.held_since.is_some());
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(heartbeat_of(&status).await.unwrap(), stopped);
    }
}
