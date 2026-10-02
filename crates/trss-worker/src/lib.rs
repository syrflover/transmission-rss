//! The long-running collection worker (`trss-worker`).
//!
//! Every interval (five minutes by default) the worker runs one *cycle*
//! ([`cycle::run_cycle`]): it reads the channels and their rules from the app
//! database, reads each channel's RSS feed, judges every item with the shared
//! evaluation in [`trss_legacy::rss`], adds the selected ones to Transmission and
//! records every item it saw in the collection history
//! ([`trss_legacy::store::history`]). The Transmission handling is the legacy
//! binary's, shared through [`trss_transmission`].
//!
//! # The collect folder
//!
//! Every torrent is saved under the app's collect folder
//! ([`trss_core::settings`]) plus its rule's directory. While no collect
//! folder is set the worker has nowhere to save anything: it still reads the
//! feeds and records the items no rule picks, but the items a rule picked are
//! neither added nor recorded (the cycle counts them as
//! `waiting_for_collect_folder`), and departed torrents are left alone. The
//! next cycle after the folder is chosen judges them as new. Recording them as
//! `add_failed` instead would pile up failures that are not failures and need
//! a retry per item.
//!
//! # Exclusivity
//!
//! Two layers keep two workers from running the same cycle:
//!
//! 1. **An OS advisory lock** ([`trss_core::CycleLock`], `flock` on
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
//! While it holds the lock, for a cycle, for commands, or for a reading that a
//! watch folder's alert asked for, the worker also leaves a heartbeat in the
//! database ([`heartbeat`]), which is how the web tells a busy worker from a
//! dead one without touching the lock. (The Anissia, artwork and season
//! queues hold locks of their own, not this one.)
//!
//! # Watch folders
//!
//! The worker watches the watch folders with inotify ([`live`]) and reads the
//! works an alert names a few seconds after it, under the same lock as the
//! cycles. After the RSS work of each cycle, still under the lock, it reads the
//! folders the alerts could not cover ([`watch`]): all of them at the start, the
//! ones with a directory that has no watch, a folder that has not been read whole
//! for an hour, and, when it is not watching at all, every folder. Reading only
//! looks at the disk and changes nothing.
//!
//! After the watch folders, the worker connects the subscriptions that have no
//! season yet to the season their received videos appeared in
//! ([`season_link`]).
//!
//! # Commands
//!
//! Between cycles the loop also looks, every few seconds, for commands the web
//! accepted and runs them under the same lock ([`commands`]).
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
//! Every request to Transmission times out ([`trss_transmission::REQUEST_TIMEOUT`],
//! connecting [`trss_transmission::CONNECT_TIMEOUT`]), so a Transmission that
//! stops answering fails the items of the cycle instead of holding the lock
//! for good. A cycle that has not wound down [`SHUTDOWN_GRACE`] after the
//! shutdown request is aborted, which aborts its item tasks and releases the
//! lock; the process then exits without waiting for the hung call. What
//! Transmission had not answered by then is not recorded (as after a kill,
//! see below).
//!
//! A process killed outright (SIGKILL, power loss) between Transmission
//! taking a torrent and the record being written leaves that item recorded as
//! `duplicate` instead of `received` after the next cycle.

pub mod commands;
pub mod cycle;
pub mod env;

use std::{path::PathBuf, time::Duration};

use tokio_util::sync::CancellationToken;
use trss_core::{
    commands::{CommandError, CommandStore},
    heartbeat::{self, HeartbeatStore},
    settings::SettingsStore,
    system_clock, Clock, CycleLock, Db,
};

pub use commands::{CommandsOutcome, DEFAULT_COMMAND_POLL};
pub use cycle::{run_cycle, CommandsAtStart, CycleError, CycleReport};
pub use env::{EnvError, WorkerEnv};

use trss_legacy::{
    store::{
        channels::ChannelStore,
        history::{HistoryError, HistoryStore},
        library::LibraryStore,
        revisions::RevisionStore,
        seasons::SeasonStore,
        status::StatusStore,
    },
    worker::{feed, live, season_link, watch, CycleContext, MovePolicy},
};
use trss_transmission::{Redactor, RenamePolicy};

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
    #[error("commands: {0}")]
    Command(#[from] CommandError),
    #[error(transparent)]
    Cycle(#[from] CycleError),
    #[error("cannot build the HTTP client: {0}")]
    Http(#[from] reqwest::Error),
    #[error("cannot build the Transmission HTTP client: {0}")]
    TransmissionHttp(reqwest012::Error),
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
    /// Commands the web accepted; see [`commands`].
    commands: CommandStore,
    interval: Duration,
    command_poll: Duration,
    min_gap: Duration,
    shutdown_grace: Duration,
    /// How often the heartbeat is written while a cycle holds the lock.
    heartbeat_every: Duration,
    lock_path: PathBuf,
    clock: Clock,
}

/// The longest a cycle waits for a reading of one of the worker's own watches
/// to let go of the lock.
const LIVE_LOCK_WAIT: Duration = Duration::from_secs(20);

/// How long a running cycle may take to wind down after shutdown was asked
/// for. It normally finishes within milliseconds (it starts nothing new and
/// records what Transmission has answered), but a Transmission call that hangs
/// would otherwise hold the process until the call's own timeout. Container
/// runtimes send SIGKILL after ten seconds by default; five leaves room.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

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
            commands: CommandStore::new(db.clone()),
            ctx: CycleContext {
                channels: ChannelStore::new(db.clone()),
                settings: SettingsStore::new(db.clone()),
                history: HistoryStore::new(db.clone()),
                revisions: RevisionStore::new(db.clone()),
                library: LibraryStore::new(db.clone()),
                seasons: SeasonStore::new(db),
                scan_cache: watch::ScanCaches::default(),
                season_link: season_link::Memory::default(),
                live: live::LiveWatch::default(),
                transmission_url: env.transmission_url.clone(),
                transmission_http: trss_transmission::http_client(
                    trss_transmission::REQUEST_TIMEOUT,
                )
                .map_err(WorkerError::TransmissionHttp)?,
                session: env.session.clone(),
                http: feed::client()?,
                rename: RenamePolicy::default(),
                moves: MovePolicy::default(),
                redactor,
            },
            interval: env.interval,
            command_poll: DEFAULT_COMMAND_POLL,
            min_gap: env.interval / 2,
            shutdown_grace: SHUTDOWN_GRACE,
            heartbeat_every: heartbeat::BEAT_EVERY,
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

    /// Overrides how a work folder move waits for Transmission (default:
    /// [`MovePolicy::default`]).
    pub fn with_move_policy(mut self, policy: MovePolicy) -> Self {
        self.ctx.moves = policy;
        self
    }

    /// Overrides how the inotify watches behave (default: [`live::LiveConfig::default`]).
    pub fn with_live_config(mut self, config: live::LiveConfig) -> Self {
        self.ctx.live = live::LiveWatch::new(config);
        self
    }

    /// The inotify watches of the watch folders.
    pub fn live(&self) -> &live::LiveWatch {
        &self.ctx.live
    }

    /// Starts watching the watch folders for changes, so that cycles stop
    /// reading the folders that alerts cover (see [`live`]). [`Worker::run`]
    /// does this itself; a worker that is only ticked is not watching, and its
    /// cycles read every folder.
    pub async fn start_watching(&self) {
        self.ctx.live.start(
            self.ctx.clone(),
            self.lock_path.clone(),
            self.clock.clone(),
            self.heartbeat_every,
        );
        self.ctx.live.sync_folders().await;
    }

    /// One pass of the season link, which a cycle runs after the watch folder
    /// scan (see [`season_link`]). It takes no lock, so it is for callers that
    /// know no cycle is running, such as tests.
    pub async fn link_seasons(&self) -> season_link::Linked {
        season_link::link_seasons(&self.ctx).await
    }

    /// Ends every watch.
    pub fn stop_watching(&self) {
        self.ctx.live.stop();
    }

    /// Overrides the minimum time between cycle starts (default: half the interval).
    pub fn with_min_gap(mut self, gap: Duration) -> Self {
        self.min_gap = gap;
        self
    }

    /// Overrides how long a running cycle may take to wind down after shutdown
    /// was asked for (default: [`SHUTDOWN_GRACE`]).
    pub fn with_shutdown_grace(mut self, grace: Duration) -> Self {
        self.shutdown_grace = grace;
        self
    }

    /// Overrides how often the heartbeat is written while a cycle holds the lock
    /// (default: [`heartbeat::BEAT_EVERY`]).
    pub fn with_heartbeat_every(mut self, every: Duration) -> Self {
        self.heartbeat_every = every;
        self
    }

    /// Overrides how long one request to Transmission may take (default:
    /// [`trss_transmission::REQUEST_TIMEOUT`]).
    pub fn with_transmission_timeout(mut self, timeout: Duration) -> Self {
        self.ctx.transmission_http = trss_transmission::http_client(timeout)
            .expect("a client with only timeouts set builds");
        self
    }

    /// Tries to run one cycle now: takes the lock, checks the start marker,
    /// runs the cycle, and releases the lock.
    pub async fn tick(&self, cancel: &CancellationToken) -> Result<TickOutcome, WorkerError> {
        let lock = self.acquire_cycle_lock().await?;
        let Some(_lock) = lock else {
            return Ok(TickOutcome::Busy);
        };

        // The web tells a busy worker from a dead one by this pulse, for as long
        // as the lock is held: the cycle and the folder reading after it.
        self.beating(self.tick_locked(cancel)).await
    }

    /// Runs `work` with the cycle lock held by this worker and its heartbeat
    /// beating ([`heartbeat::while_holding`]). Every holder of the lock does its
    /// work through this.
    pub(crate) async fn beating<T>(&self, work: impl std::future::Future<Output = T>) -> T {
        heartbeat::while_holding(
            HeartbeatStore::new(self.ctx.channels.db().clone()),
            self.clock.clone(),
            self.heartbeat_every,
            work,
        )
        .await
    }

    /// One cycle and what follows it, with the cycle lock held.
    async fn tick_locked(&self, cancel: &CancellationToken) -> Result<TickOutcome, WorkerError> {
        // Read under the lock, before the cycle starts: no command runs
        // meanwhile. The previous start is read before this cycle replaces it.
        let previous_start = self.ctx.history.last_cycle().await?.map(|c| c.started_at);
        let commands = CommandsAtStart {
            running: self.commands.running_count().await?,
            unconfirmed_adds: self.commands.unconfirmed_adds_since(previous_start).await?,
        };

        let started = (self.clock)();
        let min_gap = i64::try_from(self.min_gap.as_millis()).unwrap_or(i64::MAX);
        if !self.ctx.history.try_begin_cycle(started, min_gap).await? {
            return Ok(TickOutcome::TooSoon);
        }

        let report = run_cycle(&self.ctx, started, commands, cancel).await?;

        // An interrupted cycle stays unfinished in the marker.
        if !report.interrupted {
            self.ctx.history.finish_cycle((self.clock)()).await?;
        }

        // The watch folders are read after the RSS work, under the same lock,
        // and one that cannot be read neither stops the others nor fails the tick.
        if !report.interrupted && !cancel.is_cancelled() {
            watch::scan_all(&self.ctx, &self.clock, cancel).await;
            // The videos the rules received are in the library now (or not yet).
            season_link::link_seasons(&self.ctx).await;
        }

        Ok(TickOutcome::Ran(report))
    }

    /// Takes the cycle lock. A lock that one of the worker's own watches holds
    /// for a reading of a work (a moment) is waited for rather than reported
    /// as another worker's cycle, which would skip the whole cycle.
    async fn acquire_cycle_lock(&self) -> Result<Option<CycleLock>, WorkerError> {
        let started = tokio::time::Instant::now();
        loop {
            let lock =
                CycleLock::try_acquire(&self.lock_path).map_err(|source| WorkerError::Lock {
                    path: self.lock_path.clone(),
                    source,
                })?;
            if lock.is_some() || !self.ctx.live.flushing() || started.elapsed() > LIVE_LOCK_WAIT {
                return Ok(lock);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Runs a cycle at start and then every interval until `cancel` fires. A
    /// failed or panicking cycle is logged and the loop carries on.
    pub async fn run(&self, cancel: CancellationToken) {
        // The web shows when the next check is due and cannot read this worker's settings.
        let status = StatusStore::new(self.ctx.channels.db().clone());
        if let Err(err) = status
            .record_cycle_interval(self.interval.as_millis() as i64)
            .await
        {
            eprintln!("Cannot record the cycle interval: {err}");
        }
        self.start_watching().await;
        self.run_loop(cancel).await;
        self.stop_watching();
    }

    async fn run_loop(&self, cancel: CancellationToken) {
        let mut ticker = tokio::time::interval(self.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut command_ticker = tokio::time::interval(self.command_poll);
        command_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = ticker.tick() => {}
                _ = command_ticker.tick() => {
                    // A watch folder registered meanwhile is watched from now on.
                    self.ctx.live.sync_folders().await;
                    self.poll_commands(&cancel).await;
                    continue;
                }
            }

            // In its own task so that a panic ends the cycle, not the worker.
            let worker = self.clone();
            let token = cancel.clone();
            let mut cycle = tokio::spawn(async move { worker.tick(&token).await });

            // After shutdown was asked for, the cycle gets a grace period to wind
            // down. Past it, the cycle is aborted: dropping it aborts its item
            // tasks and releases the lock, and what Transmission had not yet
            // answered is left for the next start.
            let joined = tokio::select! {
                joined = &mut cycle => joined,
                _ = async {
                    cancel.cancelled().await;
                    tokio::time::sleep(self.shutdown_grace).await;
                } => {
                    cycle.abort();
                    let _ = cycle.await;
                    println!(
                        "Cycle abandoned: it did not wind down within {}s of the shutdown request",
                        self.shutdown_grace.as_secs()
                    );
                    continue;
                }
            };

            match joined {
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
