//! The worker's heartbeat while it holds the cycle lock.
//!
//! The web cannot ask the worker whether it is alive, and must not look at the
//! cycle lock: even a try-lock from the web could make the worker's own
//! try-lock fail and skip a cycle. So while the worker holds the lock, whatever
//! for (a cycle: the RSS work, then the watch folder reading and the season
//! link that follow under the same lock; the commands the web accepted; a
//! reading that a watch folder's alert asked for), a task writes a timestamp to
//! the database every [`BEAT_EVERY`] ([`HeartbeatStore::record`]). Every holder
//! runs its work through [`while_holding`] once it has the lock.
//! A timestamp that stops ageing means the worker is busy; one that has aged
//! for a minute means it died or is stopped, whatever the cycle's own marker
//! says. The web reads it with [`HeartbeatStore::read`] (`status_api` in
//! `trss-web`).

use std::{future::Future, time::Duration};

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use rusqlite::OptionalExtension;

use crate::{Clock, Db, DbError, Millis};

/// How often the heartbeat is written. The web calls the worker busy for a
/// minute after a beat, so this leaves room for three missed beats.
pub const BEAT_EVERY: Duration = Duration::from_secs(15);

/// The worker's pulse while it holds the cycle lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerHeartbeat {
    /// The last time the worker wrote it: every few seconds while it holds the
    /// lock, and once more when it lets go.
    pub beat_at: Millis,
    /// When the worker took the lock; `None` once it has let go. A worker that
    /// died while holding it leaves this set and a `beat_at` that ages.
    pub held_since: Option<Millis>,
}

/// Async access to the heartbeat the worker writes and the web reads. Cheap
/// to clone.
#[derive(Clone)]
pub struct HeartbeatStore {
    db: Db,
}

impl HeartbeatStore {
    pub fn new(db: Db) -> Self {
        HeartbeatStore { db }
    }

    /// Records that the worker is alive at `beat_at` and, with `held_since`, has
    /// held the cycle lock since then (`None`: it has let go of it).
    pub async fn record(&self, beat_at: Millis, held_since: Option<Millis>) -> Result<(), DbError> {
        self.db
            .run(move |c| {
                c.execute(
                    "INSERT INTO worker_heartbeat (id, beat_at, held_since) VALUES (1, ?1, ?2)
                     ON CONFLICT (id) DO UPDATE
                     SET beat_at = excluded.beat_at, held_since = excluded.held_since",
                    rusqlite::params![beat_at, held_since],
                )?;
                Ok::<_, DbError>(())
            })
            .await
    }

    /// The worker's last heartbeat; `None` while no worker of this version has
    /// run a cycle (an older worker writes none).
    pub async fn read(&self) -> Result<Option<WorkerHeartbeat>, DbError> {
        self.db
            .run(|c| {
                Ok::<_, DbError>(
                    c.query_row(
                        "SELECT beat_at, held_since FROM worker_heartbeat WHERE id = 1",
                        [],
                        |r| {
                            Ok(WorkerHeartbeat {
                                beat_at: r.get(0)?,
                                held_since: r.get(1)?,
                            })
                        },
                    )
                    .optional()?,
                )
            })
            .await
    }
}

/// Runs `work`, which the caller does with the cycle lock held, with the
/// heartbeat beating, and lets go of the hold when it returns. The caller drops
/// the lock after this returns, so the last beat is written under it. If `work`
/// panics or is dropped the heartbeat is dropped with it: no clean end is
/// written and the timestamp ages.
pub async fn while_holding<T>(
    store: HeartbeatStore,
    clock: Clock,
    every: Duration,
    work: impl Future<Output = T>,
) -> T {
    let beat = Heartbeat::start(store, clock, every).await;
    let out = work.await;
    beat.stop().await;
    out
}

/// A running heartbeat. Writes the first beat when it starts, one more every
/// [`BEAT_EVERY`], and a last one that clears the hold when [`Heartbeat::stop`]
/// is awaited. Dropping it without that (the cycle panicked or was aborted)
/// only stops the beats, so the timestamp ages and the web sees a worker that
/// has stopped.
pub struct Heartbeat {
    store: HeartbeatStore,
    clock: Clock,
    stop: CancellationToken,
    task: Option<JoinHandle<()>>,
}

impl Heartbeat {
    /// Starts beating for a lock taken now. A beat that cannot be written is
    /// logged and does not fail the cycle: the worst it does is show the worker
    /// as stopped.
    pub async fn start(store: HeartbeatStore, clock: Clock, every: Duration) -> Heartbeat {
        let held_since = clock();
        write(&store, clock(), Some(held_since)).await;

        let stop = CancellationToken::new();
        let task = tokio::spawn({
            let (store, clock, stop) = (store.clone(), clock.clone(), stop.clone());
            async move {
                loop {
                    tokio::select! {
                        biased;
                        _ = stop.cancelled() => break,
                        _ = tokio::time::sleep(every) => {}
                    }
                    write(&store, clock(), Some(held_since)).await;
                }
            }
        });
        Heartbeat {
            store,
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
        write(&self.store, (self.clock)(), None).await;
    }
}

impl Drop for Heartbeat {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn write(store: &HeartbeatStore, at: Millis, held_since: Option<Millis>) {
    if let Err(err) = store.record(at, held_since).await {
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

    /// A clock that moves one second at each reading, so that every beat has its own time.
    fn ticking_clock() -> Clock {
        let now = Arc::new(AtomicI64::new(1_000_000));
        Arc::new(move || now.fetch_add(1_000, Ordering::SeqCst))
    }

    async fn heartbeat_of(store: &HeartbeatStore) -> Option<WorkerHeartbeat> {
        store.read().await.unwrap()
    }

    #[tokio::test]
    async fn the_heartbeat_is_whatever_the_worker_last_wrote() {
        let store = HeartbeatStore::new(Db::open(":memory:").await.unwrap());
        assert_eq!(store.read().await.unwrap(), None);

        store.record(1_000, Some(900)).await.unwrap();
        store.record(16_000, Some(900)).await.unwrap();
        assert_eq!(
            store.read().await.unwrap(),
            Some(WorkerHeartbeat {
                beat_at: 16_000,
                held_since: Some(900)
            })
        );

        // Letting go of the lock keeps the time and clears the hold.
        store.record(20_000, None).await.unwrap();
        assert_eq!(
            store.read().await.unwrap(),
            Some(WorkerHeartbeat {
                beat_at: 20_000,
                held_since: None
            })
        );
    }

    #[tokio::test]
    async fn the_first_beat_is_written_before_start_returns() {
        let store = HeartbeatStore::new(Db::open(":memory:").await.unwrap());
        let beat =
            Heartbeat::start(store.clone(), ticking_clock(), Duration::from_secs(3600)).await;

        let written = heartbeat_of(&store).await.expect("a first beat");
        assert_eq!(written.held_since, Some(1_000_000));
        beat.stop().await;
    }

    #[tokio::test]
    async fn it_beats_while_held_and_clears_the_hold_when_stopped() {
        let store = HeartbeatStore::new(Db::open(":memory:").await.unwrap());
        let beat =
            Heartbeat::start(store.clone(), ticking_clock(), Duration::from_millis(10)).await;
        let first = heartbeat_of(&store).await.unwrap();

        // The beat moves on while the lock is held, and the hold keeps its start.
        let mut later = first;
        for _ in 0..200 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            later = heartbeat_of(&store).await.unwrap();
            if later.beat_at > first.beat_at {
                break;
            }
        }
        assert!(later.beat_at > first.beat_at, "{first:?} then {later:?}");
        assert_eq!(later.held_since, first.held_since);

        beat.stop().await;
        let released = heartbeat_of(&store).await.unwrap();
        assert_eq!(released.held_since, None);
        assert!(released.beat_at >= later.beat_at);

        // Nothing beats after the stop.
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(heartbeat_of(&store).await.unwrap(), released);
    }

    #[tokio::test]
    async fn a_dropped_heartbeat_stops_beating_and_keeps_the_hold() {
        let store = HeartbeatStore::new(Db::open(":memory:").await.unwrap());
        let beat =
            Heartbeat::start(store.clone(), ticking_clock(), Duration::from_millis(10)).await;
        tokio::time::sleep(Duration::from_millis(50)).await;

        // The cycle panicked or was aborted: no clean end is written, so the
        // timestamp ages and the web sees a stopped worker.
        drop(beat);
        tokio::time::sleep(Duration::from_millis(30)).await;
        let stopped = heartbeat_of(&store).await.unwrap();
        assert!(stopped.held_since.is_some());
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(heartbeat_of(&store).await.unwrap(), stopped);
    }
}
