//! The worker's heartbeat while it holds the worker lock.
//!
//! The web cannot ask the worker whether it is alive, and must not look at the
//! worker lock: even a try-lock from the web could make the worker's own
//! try-lock fail and skip a cycle. So while the worker holds the lock, whatever
//! for (a cycle: the RSS work, then the watch folder reading and the season
//! link that follow it; the commands the web accepted; a reading that a watch
//! folder's alert asked for; several of them at once), a task writes a
//! timestamp to the database every [`BEAT_EVERY`] ([`HeartbeatStore::record`]).
//! The worker's lock ([`crate::WorkerLock`]) starts the beat when the first of
//! them takes the lock and stops it when the last lets go.
//! A timestamp that stops ageing means the worker is busy; one that has aged
//! for [`FRESH_FOR_MS`] means it died or is stopped, whatever the cycle's own
//! marker says. The web reads it with [`HeartbeatStore::read`] and judges it
//! with [`worker_busy`] and [`cycle_stalled`] below.

use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use rusqlite::OptionalExtension;

use crate::{Clock, Db, DbError, Millis};

/// How often the heartbeat is written. A beat shows the worker alive for
/// [`FRESH_FOR_MS`] (a minute), so this leaves room for three missed beats.
pub const BEAT_EVERY: Duration = Duration::from_secs(15);

/// How old the worker's heartbeat may be and still show it busy: four beats.
/// This allows three missed beats, and one slow database write, before a
/// worker that died is told from one that works, which is why a killed worker
/// shows as stalled within about a minute.
pub const FRESH_FOR_MS: i64 = BEAT_EVERY.as_millis() as i64 * 4;

/// The shortest bound on how long a cycle may run and still be taken for
/// running.
const RUNNING_FLOOR_MS: i64 = 30 * 60_000;
/// How many intervals a cycle may run and still be taken for running, when that
/// is longer than [`RUNNING_FLOOR_MS`].
const RUNNING_INTERVALS: i64 = 10;

/// The worker's pulse while it holds the worker lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerHeartbeat {
    /// The last time the worker wrote it: every few seconds while it holds the
    /// lock, and once more when it lets go.
    pub beat_at: Millis,
    /// When the worker took the lock; `None` once it has let go. A worker that
    /// died while holding it leaves this set and a `beat_at` that ages.
    pub held_since: Option<Millis>,
}

impl WorkerHeartbeat {
    /// Whether the heartbeat says the worker is alive as of `now`: it was
    /// written within [`FRESH_FOR_MS`]. (A beat dated after `now` is a clock a
    /// little ahead and counts as fresh.)
    pub fn is_beating(&self, now: Millis) -> bool {
        now.saturating_sub(self.beat_at) <= FRESH_FOR_MS
    }

    /// Whether the worker has held the worker lock past [`running_bound`] while
    /// it goes on beating: alive, but stuck (in a cycle, or a command, or work
    /// that overlapped without a break).
    pub fn is_hung(&self, interval_ms: i64, now: Millis) -> bool {
        self.held_since
            .is_some_and(|since| now.saturating_sub(since) > running_bound(interval_ms))
    }
}

/// How long a worker may hold the worker lock, or (with no heartbeat) a cycle
/// run without an end, and still be taken for busy rather than hung or dead:
/// the larger of 30 minutes and ten intervals, so that a short interval does not
/// call a slow cycle stopped.
pub fn running_bound(interval_ms: i64) -> i64 {
    RUNNING_FLOOR_MS.max(interval_ms.saturating_mul(RUNNING_INTERVALS))
}

/// When the worker last started a cycle and whether it ended: the cycle's own
/// marker, which is all there is to go on without a heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleMarks {
    pub started_at: Millis,
    /// `None` while that cycle is running or if the worker died in it.
    pub finished_at: Option<Millis>,
}

/// Whether the worker is busy as of `now`, so that what it left (the look at
/// Transmission, the cycle's marker) is expected to be replaced soon.
///
/// With a heartbeat (a worker of this version has run), the worker is busy
/// while it beats, and until it has held the lock past [`running_bound`]. That
/// covers the whole time under the worker lock, including the watch folder
/// reading after the cycle's RSS work ended, and a worker killed in a cycle
/// stops being busy as soon as its beat is stale. Without one (an older
/// worker), the cycle's own marker is all there is: a cycle that has started
/// and not ended, and has not gone on past [`running_bound`].
pub fn worker_busy(
    cycle: CycleMarks,
    beat: Option<&WorkerHeartbeat>,
    interval_ms: i64,
    now: Millis,
) -> bool {
    match beat {
        Some(beat) => beat.is_beating(now) && !beat.is_hung(interval_ms, now),
        None => {
            cycle.finished_at.is_none()
                && now.saturating_sub(cycle.started_at) <= running_bound(interval_ms)
        }
    }
}

/// Whether the worker is taken to have stopped checking as of `now`.
///
/// While the heartbeat is fresh the worker is alive: it is stopped only when it
/// has held the lock past [`running_bound`]. Once the heartbeat is stale, a
/// cycle that never ended means the worker died in it, and otherwise the
/// next check is stopped once it is more than one interval overdue. Without a
/// heartbeat (an older worker) a cycle that has no end is running until it has
/// outlived [`running_bound`].
pub fn cycle_stalled(
    cycle: CycleMarks,
    beat: Option<&WorkerHeartbeat>,
    interval_ms: i64,
    now: Millis,
) -> bool {
    let overdue = || {
        now > cycle
            .started_at
            .saturating_add(interval_ms.saturating_mul(2))
    };
    match beat {
        Some(beat) if beat.is_beating(now) => beat.is_hung(interval_ms, now),
        Some(_) => cycle.finished_at.is_none() || overdue(),
        None if cycle.finished_at.is_none() => !worker_busy(cycle, None, interval_ms, now),
        None => overdue(),
    }
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
    /// held the worker lock since then (`None`: it has let go of it).
    pub async fn record(&self, beat_at: Millis, held_since: Option<Millis>) -> Result<(), DbError> {
        self.db
            .run(move |c| {
                c.prepare_cached(
                    "INSERT INTO worker_heartbeat (id, beat_at, held_since) VALUES (1, ?1, ?2)
                     ON CONFLICT (id) DO UPDATE
                     SET beat_at = excluded.beat_at, held_since = excluded.held_since",
                )?
                .execute(rusqlite::params![beat_at, held_since])?;
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
                    c.prepare_cached(
                        "SELECT beat_at, held_since FROM worker_heartbeat WHERE id = 1",
                    )?
                    .query_row([], |r| {
                        Ok(WorkerHeartbeat {
                            beat_at: r.get(0)?,
                            held_since: r.get(1)?,
                        })
                    })
                    .optional()?,
                )
            })
            .await
    }
}

/// A running heartbeat. Writes the first beat when it starts, one more every
/// [`BEAT_EVERY`], and a last one that clears the hold when [`Heartbeat::stop`]
/// is awaited. Dropping it without that (the work panicked or was aborted)
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
    /// logged and does not fail the work: the worst it does is show the worker
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

    #[test]
    fn a_beat_is_fresh_for_a_minute_and_a_cycle_runs_for_half_an_hour_at_least() {
        const MINUTE: i64 = 60_000;
        assert_eq!(FRESH_FOR_MS, MINUTE);
        assert_eq!(running_bound(MINUTE), 30 * MINUTE);
        assert_eq!(running_bound(3 * MINUTE), 30 * MINUTE);
        assert_eq!(running_bound(10 * MINUTE), 100 * MINUTE);
        assert_eq!(running_bound(i64::MAX), i64::MAX);
    }

    const MINUTE: i64 = 60_000;
    /// 2026-09-30 12:00:00 UTC.
    const NOON: i64 = 1_790_769_600_000;

    fn cycle(started_at: Millis, finished_at: Option<Millis>) -> CycleMarks {
        CycleMarks {
            started_at,
            finished_at,
        }
    }

    fn beat(beat_at: Millis, held_since: Option<Millis>) -> Option<WorkerHeartbeat> {
        Some(WorkerHeartbeat {
            beat_at,
            held_since,
        })
    }

    /// One judgement: the cycle, the heartbeat and the interval the worker
    /// recorded, as of `now`.
    struct Case {
        what: &'static str,
        cycle: CycleMarks,
        beat: Option<WorkerHeartbeat>,
        interval: i64,
        now: Millis,
        expected: bool,
    }

    fn run(cases: &[Case], judge: fn(CycleMarks, Option<&WorkerHeartbeat>, i64, Millis) -> bool) {
        for case in cases {
            assert_eq!(
                judge(case.cycle, case.beat.as_ref(), case.interval, case.now),
                case.expected,
                "{}",
                case.what
            );
        }
    }

    #[test]
    fn a_finished_cycle_is_stalled_once_the_next_check_is_more_than_one_interval_overdue() {
        let started = NOON - 60 * MINUTE;
        let finished = cycle(started, Some(started + 1_000));
        let case = |what, now, expected| Case {
            what,
            cycle: finished,
            beat: None,
            interval: 20 * MINUTE,
            now,
            expected,
        };
        run(
            &[
                // Due at 11:20; late but within one more interval until 11:40.
                case("due 20 minutes ago", NOON - 20 * MINUTE, false),
                case(
                    "one more interval and a millisecond",
                    NOON - 20 * MINUTE + 1,
                    true,
                ),
                case("40 minutes overdue", NOON, true),
                Case {
                    what: "a new cycle starting puts it back on time",
                    cycle: cycle(NOON - MINUTE, None),
                    beat: None,
                    interval: 20 * MINUTE,
                    now: NOON,
                    expected: false,
                },
                Case {
                    what: "three intervals overdue",
                    cycle: cycle(NOON - 4 * MINUTE, Some(NOON - 4 * MINUTE + 1_000)),
                    beat: None,
                    interval: MINUTE,
                    now: NOON,
                    expected: true,
                },
            ],
            cycle_stalled,
        );
    }

    #[test]
    fn without_a_heartbeat_an_unfinished_cycle_runs_until_the_larger_of_half_an_hour_and_ten_intervals(
    ) {
        let case = |what, started_ago, interval, now_after_start, expected| Case {
            what,
            cycle: cycle(NOON - started_ago, None),
            beat: None,
            interval,
            now: NOON - started_ago + now_after_start,
            expected,
        };
        run(
            &[
                // A cycle three intervals old that has not ended is the worker busy.
                case("three intervals in", 3 * MINUTE, MINUTE, 3 * MINUTE, false),
                case(
                    "at the 30 minute floor",
                    3 * MINUTE,
                    MINUTE,
                    30 * MINUTE,
                    false,
                ),
                case("past the floor", 3 * MINUTE, MINUTE, 30 * MINUTE + 1, true),
                // With a long interval the bound is ten intervals.
                case(
                    "at ten intervals",
                    3 * MINUTE,
                    10 * MINUTE,
                    100 * MINUTE,
                    false,
                ),
                case(
                    "past ten intervals",
                    3 * MINUTE,
                    10 * MINUTE,
                    100 * MINUTE + 1,
                    true,
                ),
                // A five-minute interval has a bound of fifty minutes, from ten minutes in.
                case(
                    "ten minutes in",
                    10 * MINUTE,
                    5 * MINUTE,
                    10 * MINUTE,
                    false,
                ),
                case(
                    "at fifty minutes",
                    10 * MINUTE,
                    5 * MINUTE,
                    50 * MINUTE,
                    false,
                ),
                case(
                    "past fifty minutes",
                    10 * MINUTE,
                    5 * MINUTE,
                    50 * MINUTE + 1,
                    true,
                ),
            ],
            cycle_stalled,
        );
    }

    #[test]
    fn a_stale_heartbeat_tells_a_worker_killed_in_a_cycle_from_a_slow_one() {
        // The cycle began ten minutes ago and never ended: well within the half
        // hour a cycle may take, so only the heartbeat tells it from a slow one.
        let unfinished = cycle(NOON - 10 * MINUTE, None);
        let held = Some(NOON - 10 * MINUTE);
        let case = |what, beat_at, now, expected| Case {
            what,
            cycle: unfinished,
            beat: beat(beat_at, held),
            interval: 5 * MINUTE,
            now,
            expected,
        };
        run(
            &[
                case("beating", NOON - 10_000, NOON, false),
                case(
                    "killed: the beat stopped two minutes ago",
                    NOON - 2 * MINUTE,
                    NOON,
                    true,
                ),
                // The heartbeat is fresh for a minute.
                case("a minute after the last beat", NOON, NOON + MINUTE, false),
                case(
                    "a minute and a millisecond after",
                    NOON,
                    NOON + MINUTE + 1,
                    true,
                ),
            ],
            cycle_stalled,
        );
    }

    #[test]
    fn a_worker_that_beats_through_a_long_folder_scan_is_not_stalled_but_one_that_does_not_come_back_is(
    ) {
        // The cycle began fifteen minutes ago (three intervals) and its RSS work
        // ended a minute later; the worker still holds the lock reading the watch
        // folders and beats every few seconds.
        let ended = cycle(NOON - 15 * MINUTE, Some(NOON - 14 * MINUTE));
        let case = |what, beat: Option<WorkerHeartbeat>, now, expected| Case {
            what,
            cycle: ended,
            beat,
            interval: 5 * MINUTE,
            now,
            expected,
        };
        run(
            &[
                case(
                    "scanning",
                    beat(NOON - 5_000, Some(NOON - 15 * MINUTE)),
                    NOON,
                    false,
                ),
                // The scan ends and the worker lets go; the next cycle is about to
                // start, so a minute passes before the next check is overdue.
                case(
                    "let go a moment ago",
                    beat(NOON, None),
                    NOON + 30_000,
                    false,
                ),
                case(
                    "did not come back",
                    beat(NOON, None),
                    NOON + MINUTE + 1,
                    true,
                ),
            ],
            cycle_stalled,
        );
    }

    #[test]
    fn a_worker_that_beats_but_holds_the_lock_past_the_bound_is_stalled() {
        let held = NOON - 50 * MINUTE;
        let unfinished = cycle(held, None);
        let case = |what, interval, now: Millis, expected| Case {
            what,
            cycle: unfinished,
            // Beating five seconds before `now`.
            beat: beat(now - 5_000, Some(held)),
            interval,
            now,
            expected,
        };
        run(
            &[
                // Fifty minutes (ten intervals) is the bound for a five-minute interval.
                case("at the bound", 5 * MINUTE, NOON, false),
                case("past the bound", 5 * MINUTE, NOON + 1, true),
                // With a long interval the bound is ten intervals (100 minutes).
                case(
                    "long interval at the bound",
                    10 * MINUTE,
                    held + 100 * MINUTE,
                    false,
                ),
                case(
                    "long interval past the bound",
                    10 * MINUTE,
                    held + 100 * MINUTE + 1,
                    true,
                ),
            ],
            cycle_stalled,
        );
    }

    #[test]
    fn an_idle_worker_with_a_stale_heartbeat_is_stalled_only_when_the_next_check_is_overdue() {
        let idle = cycle(NOON - 6 * MINUTE, Some(NOON - 5 * MINUTE));
        let case = |what, now, expected| Case {
            what,
            cycle: idle,
            // Between cycles the heartbeat is old by design.
            beat: beat(NOON - 5 * MINUTE, None),
            interval: 5 * MINUTE,
            now,
            expected,
        };
        run(
            &[
                case("late but within one more interval", NOON, false),
                case(
                    "past one more interval",
                    NOON - 6 * MINUTE + 10 * MINUTE + 1,
                    true,
                ),
            ],
            cycle_stalled,
        );
    }

    #[test]
    fn the_worker_is_busy_while_it_beats_until_it_holds_the_lock_past_the_bound() {
        // A one-minute interval: the bound is the 30 minute floor.
        let unfinished = cycle(NOON - 3 * MINUTE, None);
        let scanning = cycle(NOON - 3 * MINUTE, Some(NOON - 2 * MINUTE));
        let case = |what, cycle, beat, expected| Case {
            what,
            cycle,
            beat,
            interval: MINUTE,
            now: NOON,
            expected,
        };
        run(
            &[
                // Without a heartbeat only the cycle's own marker is known.
                case("a cycle three intervals in", unfinished, None, true),
                case(
                    "a cycle that ended",
                    cycle(NOON - 3 * MINUTE, Some(NOON - 1_000)),
                    None,
                    false,
                ),
                case(
                    "a cycle past the floor",
                    cycle(NOON - 31 * MINUTE, None),
                    None,
                    false,
                ),
                // With one, the whole time under the worker lock counts.
                case(
                    "scanning after the cycle ended",
                    scanning,
                    beat(NOON - 5_000, Some(NOON - 3 * MINUTE)),
                    true,
                ),
                case(
                    "let go a moment ago",
                    scanning,
                    beat(NOON - 5_000, None),
                    true,
                ),
                case(
                    "the beat stopped (died in the scan)",
                    scanning,
                    beat(NOON - 2 * MINUTE, Some(NOON - 3 * MINUTE)),
                    false,
                ),
                case(
                    "beating but hung past the bound",
                    scanning,
                    beat(NOON - 5_000, Some(NOON - 31 * MINUTE)),
                    false,
                ),
                case(
                    "beating in a cycle",
                    unfinished,
                    beat(NOON - 5_000, Some(NOON - 3 * MINUTE)),
                    true,
                ),
                case(
                    "killed in a cycle",
                    unfinished,
                    beat(NOON - 2 * MINUTE, Some(NOON - 3 * MINUTE)),
                    false,
                ),
            ],
            worker_busy,
        );
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
