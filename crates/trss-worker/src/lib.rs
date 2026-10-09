//! The long-running collection worker (`trss-worker`).
//!
//! Every interval (five minutes by default) the worker runs one *cycle*
//! ([`cycle::run_cycle`]): it reads the channels and their rules from the app
//! database, reads each channel's RSS feed, judges every item with the shared
//! evaluation in [`trss_collect::rss`], adds the selected ones to Transmission and
//! records every item it saw in the collection history
//! ([`trss_collect::store::history`]). The Transmission handling is the legacy
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
//! 1. **An OS advisory lock** ([`trss_core::WorkerLock`], `flock` on
//!    `<db path>.worker.lock`) is held while the worker works: for a cycle, for
//!    the web's commands, for a reading that a watch folder's alert asked for.
//!    Everything one worker runs at the same time shares its hold; another
//!    worker that finds the lock taken skips its cycle and leaves the commands
//!    alone. The kernel releases it when the holder dies, and a slow holder
//!    keeps it for as long as it runs, so no timeout can let a second worker
//!    in while the first is still working. No SQLite transaction is held
//!    meanwhile, so the web keeps writing.
//! 2. **A start marker** in the database ([`HistoryStore::try_begin_cycle`])
//!    refuses a start less than half an interval after the previous start.
//!    Without it, two workers whose timers are out of phase would each run a
//!    cycle every interval, one after the other. With it, the later one keeps
//!    skipping and the period stays one cycle.
//!
//! One worker runs one cycle at a time ([`Worker::tick`] says
//! [`TickOutcome::Busy`] to a second).
//!
//! While it holds the lock the worker also leaves a heartbeat in the database
//! ([`heartbeat`]), which is how the web tells a busy worker from a dead one
//! without touching the lock. (The Anissia, artwork and season queues and the
//! 30-minute observation of Anissia's subtitle lines hold locks of their own,
//! not this one.)
//!
//! # Side by side
//!
//! Within one worker, the cycle, the commands and the readings of the watch
//! folders run at the same time, and two things keep them from treading on
//! each other:
//!
//! - **Turns at the folders** ([`trss_core::folder_locks`]). Each piece of
//!   work names the folders it touches and how: a cycle's item reads its work
//!   folder for its add and rename, a revision replacement writes its folder,
//!   a reading of a watch folder reads the folder (one reading per folder at a
//!   time), a command reads or writes the folders it works in ([`commands`]):
//!   a retry reads its work folder and takes its item alone, so it and the
//!   cycle's add of the same item go in turn while other items go on beside,
//!   and the receives that may decide a rule's episode offset take the rule
//!   alone, so they go in the order they were accepted.
//!   Work on the same folder runs in the order it came, work on other folders
//!   side by side.
//! - **The torrent gate** ([`removal`]). The cycle's removal of departed
//!   torrents never runs between a command's add and the record of it.
//!
//! # Watch folders
//!
//! The worker watches the watch folders with inotify ([`live`]) and reads the
//! works an alert names a few seconds after it, holding the lock. After the RSS
//! work of each cycle it reads the folders the alerts could not cover
//! ([`watch`]): all of them at the start, the ones with a directory that has no
//! watch, a folder that has not been read whole for an hour, and, when it is
//! not watching at all, every folder. Reading only looks at the disk and
//! changes nothing.
//!
//! After the watch folders, the worker connects the subscriptions that have no
//! season yet to the season their received videos appeared in
//! ([`season_link`]).
//!
//! # Commands
//!
//! Beside the cycles, the worker carries out the commands the web accepted as
//! soon as the web wakes it ([`trss_core::wake`]), or at its own look every
//! few seconds, several at a time ([`commands`]).
//!
//! # Subtitle jobs
//!
//! Beside the commands, the worker carries out the subtitle jobs the web made
//! ([`trss_jobs`]), one at a time, holding the lock like a command: at its
//! start (after putting the jobs that wait for a source back in line), when
//! the web wakes it, and every few seconds. A job cut short by a shutdown or a
//! kill stays `running` and goes on at the next start from its records. The
//! real sources (Tistory's attachments) are always on; the fake one only with
//! [`env::FAKE_SUBTITLE_SOURCE_VAR`].
//!
//! The cleanups of stored files a person confirmed on a work's page
//! ([`trss_jobs::place::cleanup`]) are carried out in the same task, under
//! the same hold, before the ready jobs run: no job stores or links a file
//! between a cleanup's look at it and its removal.
//!
//! The worker also makes the jobs of the subscribed creators' new episodes and
//! revisions itself ([`trss_jobs::follow`]): at its start, whenever a reading
//! of Anissia's lines added observations, when the season link connected a
//! rule's season, when the season queue stored a season's AniList entry
//! ([`Worker::with_season_info`]), and after a run of jobs ended.
//!
//! Its own task also reads the files of the episodes received from the
//! subscribed creator again, once a day for 14 days after the receipt, and
//! makes a revision job when a file differs ([`trss_jobs::recheck`]). The task
//! looks for what is due at its start and every hour
//! ([`Worker::with_recheck_every`]), and the database remembers each reading,
//! so a restart reads nothing twice in a day.
//!
//! # The server browser
//!
//! With `TRSS_BROWSER_URL` (and its token and downloads folder, [`env`]) the
//! worker holds a [`trss_browser::BrowserPool`] over the browser container
//! ([`browser`]): it resets the container at its start, so a run an earlier
//! worker left open is gone, ends the runs that are idle past the policy's
//! idle time while [`Worker::run`] runs, and ends all of them at shutdown.
//! Without the variable the worker runs as before. The binary also gives the
//! subtitle jobs a reader over the pool, which takes the files out of the
//! WinPNG images of a Tistory post (`trss_subtitles::winpng`), and a browser
//! that brings a post to a site's check for a person to pass on the job's
//! remote screen (`trss_subtitles::auth`); a worker without the pool leaves
//! such posts waiting for a source.
//!
//! The web shows that screen and relays the person's input to the run's page
//! itself, through the launcher; only this worker's pool starts, resets and
//! ends runs. They meet in the database (`trss_jobs::screen`): the worker
//! binds the run to the job, watches it for the file, and clears the binding
//! when the run ends, all of them at its start; the web asks it to prepare
//! the screen when a person opens the job's page, which wakes the worker's
//! look at the screens, and records the person's input, which the pool's idle
//! end counts as use ([`browser::screen_activity`]).
//!
//! One worker at a time uses one container, since a pool resets it and its
//! reaper ends the runs it does not know: the binary takes
//! [`browser::take_lock`] for its life before it makes the pool, and runs
//! without a server browser when another worker has it.
//!
//! # Shutdown
//!
//! Cancelling the token ([`Worker::run`]) stops the loop between cycles and
//! winds a running cycle down: no new item is started, items already handed to
//! Transmission are recorded (each item is recorded as one transaction right
//! after Transmission answered), and the removal of departed torrents is
//! skipped. Nothing is written for items that were not started; the next cycle
//! sees them as new. No new command is started either, and the ones under way
//! get the same grace as the cycle (see [`commands`]).
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

pub mod browser;
pub mod commands;
pub mod cycle;
pub mod env;
mod jobs;
pub mod removal;

use std::{path::PathBuf, sync::Arc, time::Duration};

use tokio_util::sync::CancellationToken;
use trss_core::{
    commands::{CommandError, CommandStore},
    folder_locks::FolderLocks,
    heartbeat::{self, HeartbeatStore},
    settings::SettingsStore,
    system_clock, Clock, Db, WorkerHold, WorkerLock,
};

pub use commands::{CommandsOutcome, DEFAULT_COMMAND_POLL, MAX_COMMANDS_AT_ONCE};
pub use cycle::{run_cycle, CommandsAtRemoval, CycleError, CycleReport};
pub use env::{EnvError, WorkerEnv};

use trss_collect::{
    anissia::captions::CaptionObserver,
    commands::anissia_captions,
    commands::rule_archive::work_folder::MovePolicy,
    context::{CollectContext, TransmissionLink},
    feed, season_link,
    store::{
        channels::ChannelStore,
        history::{HistoryError, HistoryStore},
        revisions::RevisionStore,
        status::StatusStore,
    },
};
use trss_library::{
    live,
    store::{library::LibraryStore, seasons::SeasonStore},
    watch::{self, WatchContext},
};
use trss_transmission::{Redactor, RenamePolicy, SessionConfig};

use removal::{InFlight, Removal, TorrentGate};

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
    /// Another worker holds the lock, or this worker is running a cycle already.
    Busy,
    /// A cycle started less than the minimum gap ago (by any worker).
    TooSoon,
}

/// A collection worker. Cheap to clone; clones share the lock, the turns and
/// the commands under way.
#[derive(Clone)]
pub struct Worker {
    /// What the collection work (cycle, commands, season link) shares.
    ctx: CollectContext,
    /// What the reading of the watch folders shares.
    watch: WatchContext,
    /// Transmission's session settings, applied at the start of every cycle.
    session: SessionConfig,
    /// Commands the web accepted; see [`commands`].
    commands: CommandStore,
    interval: Duration,
    command_poll: Duration,
    min_gap: Duration,
    shutdown_grace: Duration,
    /// How often the heartbeat is written while the lock is held.
    heartbeat_every: Duration,
    lock_path: PathBuf,
    /// The lock everything this worker runs shares (see the module docs).
    lock: WorkerLock,
    /// Between the commands' adds and the cycle's removal (see [`removal`]).
    torrents: TorrentGate,
    /// The commands this worker is carrying out now.
    in_flight: InFlight,
    /// Held while a cycle runs: one at a time.
    cycling: Arc<tokio::sync::Mutex<()>>,
    /// Where the web's wake-ups arrive ([`trss_core::wake`]); `None`: only the
    /// worker's own looks.
    wake_path: Option<PathBuf>,
    /// Reads one anime's subtitle lines for `anissia_captions` commands; `None`
    /// fails them.
    captions: Option<CaptionObserver>,
    /// Carries out the subtitle jobs; `None`: the worker leaves them alone.
    jobs: Option<trss_jobs::Runner>,
    /// Makes the subscribed creators' jobs; set with `jobs`.
    follow: Option<trss_jobs::Follow>,
    /// Reads the received episodes' files again for 14 days; set with `jobs`.
    recheck: Option<trss_jobs::Recheck>,
    /// How often the due rechecks are looked for.
    recheck_every: Duration,
    /// Rung when the web wakes the worker, so the jobs are looked at too.
    job_wake: Arc<tokio::sync::Notify>,
    /// Rung when the web wakes the worker, so the remote screens are looked
    /// at too (a person opened a job's page).
    screen_wake: Arc<tokio::sync::Notify>,
    /// Rung when the season queue stored a season's entry
    /// ([`trss_library::seasons::Seasons::stored`]); `None`: not heard.
    season_stored: Option<Arc<tokio::sync::Notify>>,
    /// Held while subtitle jobs run: one run at a time, since the worker
    /// lock does not keep two runs of one worker apart.
    jobs_running: Arc<tokio::sync::Mutex<()>>,
    /// The server browser ([`trss_browser`]); `None`: the worker has none.
    browser: Option<trss_browser::BrowserPool>,
    /// The worker holds the server browser's lock but runs without the browser
    /// ([`Worker::with_unused_browser_lock`]).
    browser_lock_unused: bool,
    clock: Clock,
}

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

        let library = LibraryStore::new(db.clone());
        let live = live::LiveWatch::default();
        let folders = FolderLocks::new();
        let clock = system_clock();

        Ok(Worker {
            commands: CommandStore::new(db.clone()),
            watch: WatchContext {
                library: library.clone(),
                settings: SettingsStore::new(db.clone()),
                folders: folders.clone(),
                scan_cache: watch::ScanCaches::default(),
                live: live.clone(),
            },
            session: env.session.clone(),
            lock: WorkerLock::new(
                lock_path.clone(),
                HeartbeatStore::new(db.clone()),
                clock.clone(),
                heartbeat::BEAT_EVERY,
            ),
            ctx: CollectContext {
                channels: ChannelStore::new(db.clone()),
                settings: SettingsStore::new(db.clone()),
                history: HistoryStore::new(db.clone()),
                revisions: RevisionStore::new(db.clone()),
                commands: CommandStore::new(db.clone()),
                seasons: SeasonStore::new(db),
                season_link: season_link::Memory::default(),
                library,
                live,
                transmission: TransmissionLink {
                    url: env.transmission_url.clone(),
                    http: trss_transmission::http_client(trss_transmission::REQUEST_TIMEOUT)
                        .map_err(WorkerError::TransmissionHttp)?,
                },
                http: feed::client()?,
                rename: RenamePolicy::default(),
                moves: MovePolicy::default(),
                redactor,
                folders,
            },
            interval: env.interval,
            command_poll: DEFAULT_COMMAND_POLL,
            min_gap: env.interval / 2,
            shutdown_grace: SHUTDOWN_GRACE,
            heartbeat_every: heartbeat::BEAT_EVERY,
            lock_path,
            torrents: TorrentGate::default(),
            in_flight: InFlight::default(),
            cycling: Arc::default(),
            wake_path: None,
            captions: None,
            jobs: None,
            follow: None,
            recheck: None,
            recheck_every: jobs::RECHECK_EVERY,
            job_wake: Arc::default(),
            screen_wake: Arc::default(),
            season_stored: None,
            jobs_running: Arc::default(),
            browser: None,
            browser_lock_unused: false,
            clock,
        })
    }

    /// The lock again, for a clock or a beat that changed. Builders call it
    /// before the worker is used.
    fn rebuild_lock(&mut self) {
        self.lock = WorkerLock::new(
            self.lock_path.clone(),
            HeartbeatStore::new(self.ctx.channels.db().clone()),
            self.clock.clone(),
            self.heartbeat_every,
        );
    }

    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self.rebuild_lock();
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
        let live = live::LiveWatch::new(config);
        self.ctx.live = live.clone();
        self.watch.live = live;
        self
    }

    /// Reads the Anissia anime `anissia_captions` commands name with
    /// `observer` (the 30-minute reading of the recent list is the observer's
    /// own queue, [`CaptionObserver::run_queue`], run beside the worker).
    pub fn with_captions(mut self, observer: CaptionObserver) -> Self {
        self.captions = Some(observer);
        self
    }

    /// Gives the worker the server browser. While [`Worker::run`] runs, the
    /// pool's reaper ends the runs that are idle past the policy's idle time,
    /// and at shutdown every run ends. The jobs use the pool through the
    /// runner's reader, which the caller gives the runner
    /// (`trss_jobs::Runner::with_winpng`); the pool is also there for the
    /// sources that need a person's authentication ([`Worker::browser`]).
    pub fn with_browser(mut self, pool: trss_browser::BrowserPool) -> Self {
        self.browser = Some(pool);
        self
    }

    /// Tells the worker it holds the server browser's lock (the lock of
    /// `browser::take_lock`) but runs without the browser, because it could not prepare the
    /// downloads folder. No other worker uses the browser then, so the remote
    /// screens still bound to runs of an earlier worker are no one's and are
    /// closed at the start, as for a worker that has the browser.
    pub fn with_unused_browser_lock(mut self) -> Self {
        self.browser_lock_unused = true;
        self
    }

    /// The server browser; `None` when the worker was given none.
    pub fn browser(&self) -> Option<&trss_browser::BrowserPool> {
        self.browser.as_ref()
    }

    /// Listens for the web's wake-ups at `path` (see [`trss_core::wake`]) while
    /// [`Worker::run`] runs. Without it the worker looks for commands at its
    /// own pace only.
    pub fn with_wake_socket(mut self, path: PathBuf) -> Self {
        self.wake_path = Some(path);
        self
    }

    /// The inotify watches of the watch folders.
    pub fn live(&self) -> &live::LiveWatch {
        &self.watch.live
    }

    /// Starts watching the watch folders for changes, so that cycles stop
    /// reading the folders that alerts cover (see [`live`]). [`Worker::run`]
    /// does this itself; a worker that is only ticked is not watching, and its
    /// cycles read every folder.
    pub async fn start_watching(&self) {
        self.watch
            .live
            .start(self.watch.clone(), self.lock.clone(), self.clock.clone());
        self.watch.live.sync_folders().await;
    }

    /// One pass of the season link, which a cycle runs after the watch folder
    /// scan (see [`season_link`]). It takes no lock, so it is for callers that
    /// know no cycle is running, such as tests.
    pub async fn link_seasons(&self) -> season_link::Linked {
        let linked = season_link::link_seasons(&self.ctx.link()).await;
        self.ask_captions(&linked).await;
        // A subscribed creator's season may have just become one to receive.
        if linked.linked > 0 && self.follow_logged().await {
            self.job_wake.notify_one();
        }
        linked
    }

    /// Has the subtitle lines of the anime that seasons were just connected to
    /// read at once (`anissia_captions`, [`commands`]), like a season linked
    /// from the work detail. A pass that connected nothing asks for nothing, so
    /// a cycle that changes no link makes no command. A worker that reads no
    /// Anissia ([`Worker::with_captions`] not given) asks for nothing, since
    /// the command could only fail.
    async fn ask_captions(&self, linked: &season_link::Linked) {
        if self.captions.is_none() {
            return;
        }
        let mut stored = false;
        for anime_no in &linked.anime_nos {
            match anissia_captions::ask(&self.commands, *anime_no, (self.clock)()).await {
                Ok(created) => stored |= created,
                Err(err) => eprintln!(
                    "Season link: cannot ask for the subtitle lines of anime {anime_no}: {err}"
                ),
            }
        }
        if stored {
            if let Some(path) = &self.wake_path {
                trss_core::wake::wake_worker(path);
            }
        }
    }

    /// Ends every watch.
    pub fn stop_watching(&self) {
        self.watch.live.stop();
    }

    /// Overrides the minimum time between cycle starts (default: half the interval).
    pub fn with_min_gap(mut self, gap: Duration) -> Self {
        self.min_gap = gap;
        self
    }

    /// Overrides how long a running cycle or command may take to wind down
    /// after shutdown was asked for (default: [`SHUTDOWN_GRACE`]).
    pub fn with_shutdown_grace(mut self, grace: Duration) -> Self {
        self.shutdown_grace = grace;
        self
    }

    /// Overrides how often the heartbeat is written while the lock is held
    /// (default: [`heartbeat::BEAT_EVERY`]).
    pub fn with_heartbeat_every(mut self, every: Duration) -> Self {
        self.heartbeat_every = every;
        self.rebuild_lock();
        self
    }

    /// Overrides how long one request to Transmission may take (default:
    /// [`trss_transmission::REQUEST_TIMEOUT`]).
    pub fn with_transmission_timeout(mut self, timeout: Duration) -> Self {
        self.ctx.transmission.http = trss_transmission::http_client(timeout)
            .expect("a client with only timeouts set builds");
        self
    }

    /// A handle to this worker's hold of its lock; `None` when another worker
    /// holds it.
    pub(crate) async fn hold(&self) -> Result<Option<WorkerHold>, WorkerError> {
        self.lock
            .try_hold()
            .await
            .map_err(|source| WorkerError::Lock {
                path: self.lock_path.clone(),
                source,
            })
    }

    /// Tries to run one cycle now: takes the lock, checks the start marker,
    /// runs the cycle, and lets go of the lock. The commands and the readings
    /// of this worker go on meanwhile.
    pub async fn tick(&self, cancel: &CancellationToken) -> Result<TickOutcome, WorkerError> {
        let Ok(_one_cycle) = self.cycling.try_lock() else {
            return Ok(TickOutcome::Busy);
        };
        let Some(hold) = self.hold().await? else {
            return Ok(TickOutcome::Busy);
        };
        let outcome = self.tick_held(cancel).await;
        hold.release().await;
        outcome
    }

    /// One cycle and what follows it, with the lock held.
    async fn tick_held(&self, cancel: &CancellationToken) -> Result<TickOutcome, WorkerError> {
        // The previous start is read before this cycle replaces it.
        let previous_start = self.ctx.history.last_cycle().await?.map(|c| c.started_at);

        let started = (self.clock)();
        let min_gap = i64::try_from(self.min_gap.as_millis()).unwrap_or(i64::MAX);
        if !self.ctx.history.try_begin_cycle(started, min_gap).await? {
            return Ok(TickOutcome::TooSoon);
        }

        let removal = Removal {
            gate: self.torrents.clone(),
            commands: self.commands.clone(),
            in_flight: self.in_flight.clone(),
            previous_start,
        };
        let report = run_cycle(&self.ctx, &self.session, started, &removal, cancel).await?;

        // The `start` commands the cycle stored for rules whose work folder
        // was in the archive folder: the command runner looks at once, not at
        // its next poll.
        if report.moves_asked > 0 {
            if let Some(path) = &self.wake_path {
                trss_core::wake::wake_worker(path);
            }
        }

        // An interrupted cycle stays unfinished in the marker.
        if !report.interrupted {
            self.ctx.history.finish_cycle((self.clock)()).await?;
        }

        // The watch folders are read after the RSS work, and one that cannot be
        // read neither stops the others nor fails the tick.
        if !report.interrupted && !cancel.is_cancelled() {
            watch::scan_all(&self.watch, &self.clock, cancel).await;
            // The videos the rules received are in the library now (or not yet).
            self.link_seasons().await;
        }

        Ok(TickOutcome::Ran(report))
    }

    /// Runs a cycle at start and then every interval until `cancel` fires, and
    /// carries out the web's commands beside the cycles. A failed or panicking
    /// cycle is logged and the loop carries on.
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
        // Before any job runs: the runs bound to the jobs that wait for a
        // site's check were an earlier worker's, closed by the pool's reset.
        self.clear_screens().await;
        let jobs = tokio::spawn({
            let (worker, cancel) = (self.clone(), cancel.clone());
            async move { worker.run_jobs(cancel).await }
        });
        let screens = tokio::spawn({
            let (worker, cancel) = (self.clone(), cancel.clone());
            async move { worker.run_screens(cancel).await }
        });
        let rechecks = tokio::spawn({
            let (worker, cancel) = (self.clone(), cancel.clone());
            async move { worker.run_rechecks(cancel).await }
        });
        let commands = tokio::spawn({
            let (worker, cancel, wake) = (self.clone(), cancel.clone(), self.listen_for_wakes());
            async move { worker.dispatch(cancel, wake).await }
        });
        let browser_reaper = self.browser.clone().map(|pool| {
            let cancel = cancel.clone();
            tokio::spawn(async move { pool.run_reaper(cancel).await })
        });
        self.run_loop(cancel).await;
        if let (Some(reaper), Some(pool)) = (browser_reaper, &self.browser) {
            let _ = reaper.await;
            // Whatever the jobs held open goes with the worker; the jobs
            // themselves wait for authentication as before.
            if tokio::time::timeout(self.shutdown_grace, pool.shutdown())
                .await
                .is_err()
            {
                eprintln!("Browser: the runs did not all end before the shutdown grace ran out");
            }
        }
        if let Err(err) = commands.await {
            eprintln!("Commands stopped: {err}");
        }
        if let Err(err) = jobs.await {
            eprintln!("Subtitle jobs stopped: {err}");
        }
        if let Err(err) = screens.await {
            eprintln!("Remote screens stopped: {err}");
        }
        if let Err(err) = rechecks.await {
            eprintln!("Subtitle rechecks stopped: {err}");
        }
        self.stop_watching();
    }

    async fn run_loop(&self, cancel: CancellationToken) {
        let mut ticker = tokio::time::interval(self.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut folder_ticker = tokio::time::interval(self.command_poll);
        folder_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = ticker.tick() => {}
                _ = folder_ticker.tick() => {
                    // A watch folder registered meanwhile is watched from now on.
                    self.watch.live.sync_folders().await;
                    continue;
                }
            }

            // In its own task so that a panic ends the cycle, not the worker.
            let worker = self.clone();
            let token = cancel.clone();
            let mut cycle = tokio::spawn(async move { worker.tick(&token).await });

            // After shutdown was asked for, the cycle gets a grace period to wind
            // down. Past it, the cycle is aborted: dropping it aborts its item
            // tasks and releases its hold, and what Transmission had not yet
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
