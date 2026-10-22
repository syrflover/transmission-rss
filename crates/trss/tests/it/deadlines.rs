//! The moments at which what the web decides from the clock changes, announced
//! to the event stream (ticket lighter-structure/0156). The rule of each moment is tested in the
//! crate that owns the rule; here the service sleeps to the moments on a clock
//! the test sets, in paused time, and announces them.

use std::{
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;
use trss::{
    deadlines::Deadlines,
    web::{status_api, AppState},
};
use trss_anissia::Anissia;
use trss_collect::store::status::TransmissionCounts;
use trss_core::{
    calendar::{day_start, days_from_civil, next_day_start, DAY_MS},
    changes::{Change, ChangeKind, Changes},
    Clock, Db, DbError, Millis,
};

const MINUTE: Millis = 60_000;
/// 2026-10-01 12:00 in Seoul.
const NOON: Millis = 1_790_780_400_000 + 12 * 60 * MINUTE;

struct App {
    state: AppState,
    db: Db,
    now: Arc<AtomicI64>,
    clock: Clock,
    /// Hears what the service announces, from before it runs.
    heard: broadcast::Receiver<Change>,
    _dir: tempfile::TempDir,
}

impl App {
    /// A web state on a database that announces its writes, over a clock that
    /// reads `now` until a test moves it.
    async fn at(now: Millis) -> App {
        let dir = tempfile::tempdir().unwrap();
        let changes = Changes::default();
        let db = Db::open_announcing(dir.path().join("app.db"), changes.clone())
            .await
            .unwrap();
        let now = Arc::new(AtomicI64::new(now));
        let clock: Clock = {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        };
        let anissia = Anissia::new(db.clone(), Default::default(), clock.clone());
        let state = AppState::new(db.clone())
            .with_anissia(anissia)
            .with_changes(changes.clone());
        App {
            state,
            db,
            now,
            clock,
            heard: changes.subscribe(),
            _dir: dir,
        }
    }

    fn set(&self, now: Millis) {
        self.now.store(now, Ordering::SeqCst);
    }

    /// Runs the service; returns what it waits for and how to stop it.
    fn serve(&self) -> (watch::Receiver<Option<Millis>>, CancellationToken) {
        let service = Deadlines::new(self.state.clone(), self.clock.clone());
        let waiting = service.waiting_for();
        let cancel = CancellationToken::new();
        tokio::spawn(service.run(cancel.clone()));
        (waiting, cancel)
    }

    /// The first change the service announces, within a day of paused time.
    async fn announced(&mut self) -> Option<Change> {
        loop {
            match tokio::time::timeout(Duration::from_secs(24 * 60 * 60), self.heard.recv()).await {
                Ok(Ok(change)) => return Some(change),
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
                Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => return None,
            }
        }
    }

    /// Drops what was announced so far (the writes of the setup).
    fn forget_heard(&mut self) {
        while self.heard.try_recv().is_ok() {}
    }

    async fn stalled(&self) -> bool {
        status_api::board(&self.state, (self.clock)(), 0)
            .await
            .unwrap()
            .cycle
            .unwrap()
            .stalled
    }
}

async fn waits_for(waiting: &mut watch::Receiver<Option<Millis>>, moment: Option<Millis>) {
    let timed_out =
        tokio::time::timeout(Duration::from_secs(60), waiting.wait_for(|w| *w == moment))
            .await
            .is_err();
    if timed_out {
        panic!(
            "the service never waited for {moment:?}: it waits for {:?}",
            *waiting.borrow()
        );
    }
}

/// A cycle that began at `NOON` and has not ended, with the interval `interval`.
async fn running_cycle(app: &App, interval: Millis) {
    app.state
        .status
        .record_cycle_interval(interval)
        .await
        .unwrap();
    app.state.history.begin_cycle(NOON).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_collection_that_outlives_its_bound_is_announced_when_it_stalls() {
    let mut app = App::at(NOON + MINUTE).await;
    running_cycle(&app, 5 * MINUTE).await;
    app.forget_heard();
    assert!(!app.stalled().await);
    let (mut waiting, cancel) = app.serve();
    // A cycle without an end is believed to run for ten intervals, and for at
    // least half an hour (`CycleState::next_change`).
    let stalls = NOON + 50 * MINUTE + 1;
    waits_for(&mut waiting, Some(stalls)).await;
    // Nothing was announced on the way to it.
    assert!(app.heard.try_recv().is_err());

    app.set(stalls);
    assert_eq!(
        app.announced().await,
        Some(Change {
            kind: ChangeKind::CollectStatus,
            id: None
        })
    );
    // What the screen reads next says so.
    assert!(app.stalled().await);
    cancel.cancel();
}

#[tokio::test(start_paused = true)]
async fn a_moment_that_has_come_when_a_change_is_heard_is_announced() {
    let mut app = App::at(NOON + MINUTE).await;
    running_cycle(&app, 5 * MINUTE).await;
    app.forget_heard();
    let (mut waiting, cancel) = app.serve();
    let stalls = NOON + 50 * MINUTE + 1;
    waits_for(&mut waiting, Some(stalls)).await;

    // The clock reaches the moment, and a change that can move a moment is
    // heard before the sleep to it ends. Worked out again from now, the moment
    // is behind and would be gone.
    app.set(stalls);
    app.state.changes.announce_target(ChangeKind::Rules, None);
    assert_eq!(
        app.announced().await,
        Some(Change {
            kind: ChangeKind::Rules,
            id: None
        })
    );
    assert_eq!(
        app.announced().await,
        Some(Change {
            kind: ChangeKind::CollectStatus,
            id: None
        })
    );
    assert!(app.stalled().await);
    cancel.cancel();
}

#[tokio::test(start_paused = true)]
async fn a_moment_worked_out_before_the_database_failed_is_still_announced() {
    let mut app = App::at(NOON + MINUTE).await;
    running_cycle(&app, 5 * MINUTE).await;
    app.forget_heard();
    let (mut waiting, cancel) = app.serve();
    let stalls = NOON + 50 * MINUTE + 1;
    waits_for(&mut waiting, Some(stalls)).await;

    // The moments cannot be worked out again: what a change makes the service
    // read is gone. It has tried once the moment it waits for is told again.
    app.db
        .run(|c| Ok::<_, DbError>(c.execute_batch("DROP TABLE worker_info")?))
        .await
        .unwrap();
    waiting.borrow_and_update();
    app.state.changes.announce_target(ChangeKind::Rules, None);
    waiting.changed().await.unwrap();
    app.forget_heard();
    app.set(stalls);
    assert_eq!(
        app.announced().await,
        Some(Change {
            kind: ChangeKind::CollectStatus,
            id: None
        })
    );
    cancel.cancel();
}

#[tokio::test(start_paused = true)]
async fn a_change_that_moves_the_moment_makes_the_service_work_it_out_again() {
    let mut app = App::at(NOON + MINUTE).await;
    running_cycle(&app, MINUTE).await;
    app.forget_heard();
    let (mut waiting, cancel) = app.serve();
    // Half an hour: a cycle without an end is believed to run that long.
    waits_for(&mut waiting, Some(NOON + 30 * MINUTE + 1)).await;

    // The cycle ends, and a finished one is late after two intervals instead.
    app.state.history.finish_cycle(NOON + 2_000).await.unwrap();
    let stalls = NOON + 2 * MINUTE + 1;
    waits_for(&mut waiting, Some(stalls)).await;

    // Past the new moment, and far short of the old one. What the write of the
    // cycle's end announced is not what is waited for.
    app.forget_heard();
    app.set(stalls);
    assert_eq!(
        app.announced().await,
        Some(Change {
            kind: ChangeKind::CollectStatus,
            id: None
        })
    );
    assert!(app.stalled().await);
    cancel.cancel();
}

#[tokio::test(start_paused = true)]
async fn the_seoul_midnight_is_announced_and_the_next_one_waited_for() {
    // A minute before the Seoul day ends.
    let midnight = day_start(days_from_civil(2026, 10, 2));
    let mut app = App::at(midnight - MINUTE).await;
    app.forget_heard();
    let (mut waiting, cancel) = app.serve();
    waits_for(&mut waiting, Some(midnight)).await;
    assert_eq!(next_day_start(midnight - MINUTE), midnight);

    app.set(midnight);
    assert_eq!(
        app.announced().await,
        Some(Change {
            kind: ChangeKind::CollectStatus,
            id: None
        })
    );
    waits_for(&mut waiting, Some(midnight + DAY_MS)).await;
    cancel.cancel();
}

#[tokio::test(start_paused = true)]
async fn the_service_waits_for_the_earliest_of_the_moments_there_are() {
    let app = App::at(NOON + MINUTE).await;
    let service = Deadlines::new(app.state.clone(), app.clock.clone());
    // Only the Seoul midnight, twelve hours away, while nothing else is known.
    let midnight = next_day_start(NOON);
    assert_eq!(service.next(NOON + MINUTE).await.unwrap(), Some(midnight));

    // A finished cycle is late after two intervals, and the look at
    // Transmission it left is believed for three.
    app.state
        .status
        .record_cycle_interval(5 * MINUTE)
        .await
        .unwrap();
    app.state.history.begin_cycle(NOON).await.unwrap();
    app.state.history.finish_cycle(NOON + 1_000).await.unwrap();
    app.state
        .status
        .record_transmission(
            TransmissionCounts {
                downloading: 1,
                seeding: 0,
                taken_at: NOON,
            },
            vec!["aa".into()],
        )
        .await
        .unwrap();
    let stalls = NOON + 10 * MINUTE + 1;
    let stale = NOON + 15 * MINUTE + 1;
    assert_eq!(service.next(NOON + MINUTE).await.unwrap(), Some(stalls));
    assert_eq!(service.next(stalls - 1).await.unwrap(), Some(stalls));
    assert_eq!(service.next(stalls).await.unwrap(), Some(stale));
    assert_eq!(service.next(stale).await.unwrap(), Some(midnight));
}

#[tokio::test(start_paused = true)]
async fn an_old_list_of_torrents_is_announced_and_a_refreshed_one_moves_the_moment() {
    let mut app = App::at(NOON + MINUTE).await;
    let list = || vec!["aa".to_owned()];
    app.state.status.record_listing(NOON, list()).await.unwrap();
    app.forget_heard();
    let (mut waiting, cancel) = app.serve();
    // The offers believe a list for half an hour (`in_place::LISTING_FRESH_FOR`).
    waits_for(&mut waiting, Some(NOON + 30 * MINUTE + 1)).await;

    // The next cycle writes a new list, and its end is announced after it.
    app.state
        .status
        .record_listing(NOON + 20 * MINUTE, list())
        .await
        .unwrap();
    app.state
        .status
        .record_cycle_interval(5 * MINUTE)
        .await
        .unwrap();
    let old = NOON + 50 * MINUTE + 1;
    waits_for(&mut waiting, Some(old)).await;

    app.forget_heard();
    app.set(old);
    assert_eq!(
        app.announced().await,
        Some(Change {
            kind: ChangeKind::CollectStatus,
            id: None
        })
    );
    cancel.cancel();
}
