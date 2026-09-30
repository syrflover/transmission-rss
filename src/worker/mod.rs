//! The long-running collection worker (`trss-worker`).
//!
//! Every interval (five minutes by default) the worker runs one *cycle*
//! ([`cycle::run_cycle`]): it reads the channels and their rules from the app
//! database, reads each channel's RSS feed, judges every item with the shared
//! evaluation in [`crate::rss`], adds the selected ones to Transmission and
//! records every item it saw in the collection history
//! ([`crate::store::history`]). The Transmission handling is the legacy
//! binary's, shared through [`crate::transmission`].
//!
//! # Exclusivity
//!
//! Two layers keep two workers from running the same cycle:
//!
//! 1. **An OS advisory lock** ([`lock::CycleLock`], `flock` on
//!    `<db path>.worker.lock`) is held for the whole cycle. A worker that finds
//!    it taken skips the cycle. The kernel releases it when the holder dies,
//!    and a slow holder keeps it for as long as it runs, so no timeout can let a
//!    second worker in while the first is still working. No SQLite transaction
//!    is held during the cycle, so the web keeps writing.
//! 2. **A start marker** in the database ([`HistoryStore::try_begin_cycle`])
//!    refuses a start less than half an interval after the previous start.
//!    Without it, two workers whose timers are out of phase would each run a
//!    cycle every interval, one after the other. With it, the later one keeps
//!    skipping and the period stays one cycle.
//!
//! # Shutdown
//!
//! Cancelling the token ([`Worker::run`]) stops the loop between cycles and
//! winds a running cycle down: no new item is started, items already handed to
//! Transmission are recorded (each item is recorded as one transaction right
//! after Transmission answered), and the removal of departed torrents is
//! skipped. Nothing is written for items that were not started; the next cycle
//! sees them as new.
//!
//! A process killed outright (SIGKILL, power loss) between Transmission
//! taking a torrent and the record being written leaves that item recorded as
//! `duplicate` instead of `received` after the next cycle.

pub mod cycle;
pub mod env;
pub mod feed;
pub mod lock;
pub mod plan;

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio_util::sync::CancellationToken;

pub use cycle::{run_cycle, CycleContext, CycleError, CycleReport};
pub use env::{EnvError, WorkerEnv};
pub use lock::{lock_path_for, CycleLock};

use crate::{
    store::{
        channels::ChannelStore,
        history::{HistoryError, HistoryStore, Millis},
        Db,
    },
    transmission::{Redactor, RenamePolicy},
};

/// Source of the current time in Unix milliseconds.
pub type Clock = Arc<dyn Fn() -> Millis + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as Millis)
    })
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("cannot take the worker lock {path}: {source}")]
    Lock {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cycle marker: {0}")]
    History(#[from] HistoryError),
    #[error(transparent)]
    Cycle(#[from] CycleError),
    #[error("cannot build the HTTP client: {0}")]
    Http(#[from] reqwest::Error),
}

/// What one attempt to run a cycle came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickOutcome {
    Ran(CycleReport),
    /// Another worker holds the lock and is running a cycle.
    Busy,
    /// A cycle started less than the minimum gap ago (by any worker).
    TooSoon,
}

/// A collection worker. Cheap to clone.
#[derive(Clone)]
pub struct Worker {
    ctx: CycleContext,
    interval: Duration,
    min_gap: Duration,
    lock_path: PathBuf,
    clock: Clock,
}

impl Worker {
    /// A worker on `db`, taking its settings from `env` and its lock file from
    /// `lock_path`. The minimum gap between cycle starts is half the interval.
    pub fn new(db: Db, env: &WorkerEnv, lock_path: PathBuf) -> Result<Worker, WorkerError> {
        // Credentials in the Transmission URL must not reach logs or history.
        let mut redactor = Redactor::none();
        if !env.transmission_url.username().is_empty() {
            redactor.add(env.transmission_url.username());
        }
        if let Some(password) = env.transmission_url.password() {
            redactor.add(password);
        }

        Ok(Worker {
            ctx: CycleContext {
                channels: ChannelStore::new(db.clone()),
                history: HistoryStore::new(db),
                transmission_url: env.transmission_url.clone(),
                session: env.session.clone(),
                http: feed::client()?,
                rename: RenamePolicy::default(),
                redactor,
            },
            interval: env.interval,
            min_gap: env.interval / 2,
            lock_path,
            clock: system_clock(),
        })
    }

    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_rename_policy(mut self, policy: RenamePolicy) -> Self {
        self.ctx.rename = policy;
        self
    }

    /// Overrides the minimum time between cycle starts (default: half the interval).
    pub fn with_min_gap(mut self, gap: Duration) -> Self {
        self.min_gap = gap;
        self
    }

    /// Tries to run one cycle now: takes the lock, checks the start marker,
    /// runs the cycle, and releases the lock.
    pub async fn tick(&self, cancel: &CancellationToken) -> Result<TickOutcome, WorkerError> {
        let lock = CycleLock::try_acquire(&self.lock_path).map_err(|source| WorkerError::Lock {
            path: self.lock_path.clone(),
            source,
        })?;
        let Some(_lock) = lock else {
            return Ok(TickOutcome::Busy);
        };

        let started = (self.clock)();
        let min_gap = i64::try_from(self.min_gap.as_millis()).unwrap_or(i64::MAX);
        if !self.ctx.history.try_begin_cycle(started, min_gap).await? {
            return Ok(TickOutcome::TooSoon);
        }

        let report = run_cycle(&self.ctx, started, cancel).await?;

        // An interrupted cycle stays unfinished in the marker.
        if !report.interrupted {
            self.ctx.history.finish_cycle((self.clock)()).await?;
        }

        Ok(TickOutcome::Ran(report))
    }

    /// Runs a cycle at start and then every interval until `cancel` fires. A
    /// failed or panicking cycle is logged and the loop carries on.
    pub async fn run(&self, cancel: CancellationToken) {
        let mut ticker = tokio::time::interval(self.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = ticker.tick() => {}
            }

            // In its own task so that a panic ends the cycle, not the worker.
            let worker = self.clone();
            let token = cancel.clone();
            let cycle = tokio::spawn(async move { worker.tick(&token).await });

            match cycle.await {
                Ok(Ok(TickOutcome::Ran(report))) if report.interrupted => {
                    println!("Cycle interrupted by shutdown");
                }
                Ok(Ok(TickOutcome::Ran(_))) => {}
                Ok(Ok(TickOutcome::Busy)) => {
                    println!("Another worker is running a cycle; skipping this one");
                }
                Ok(Ok(TickOutcome::TooSoon)) => {
                    println!("A cycle started recently; skipping this one");
                }
                Ok(Err(err)) => eprintln!("Cycle failed: {err}"),
                Err(err) => eprintln!("Cycle aborted: {err}"),
            }
        }
    }
}
