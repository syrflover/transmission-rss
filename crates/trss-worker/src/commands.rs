//! Carrying out the commands the web accepted (see [`trss_core::commands`]).
//!
//! # When
//!
//! The web wakes the worker when it has stored a command
//! ([`trss_core::wake`]), and the worker also looks every
//! [`DEFAULT_COMMAND_POLL`] by itself, for a wake that was lost and for the
//! commands it left for a later look. A look claims the waiting commands,
//! oldest first, and starts each in its own task, up to
//! [`MAX_COMMANDS_AT_ONCE`]; one that ends lets the next in. A command does
//! not wait for a cycle, a reading of the watch folders, or another command,
//! except where they touch the same folders or the removal of departed
//! torrents is under way (below).
//!
//! The worker claims only while it holds the worker lock
//! ([`trss_core::WorkerLock`]), which everything it runs shares and another
//! worker never holds at the same time:
//!
//! - two workers never run one command twice, because only the holder claims,
//!   and a worker passes over the commands it runs itself
//!   ([`trss_core::commands::CommandStore::claim_next_excluding`]);
//! - a lock another worker holds is not an error: the next look tries again.
//!
//! # Turns
//!
//! Before a command starts, the look names the folders it works in and takes
//! its place in line for them ([`trss_core::folder_locks`]), in the order the
//! commands were claimed: `receive_once` and `receive_past` write the work
//! folder of their rule (their add and rename must not meet a cycle's or
//! another receive's of the same item), `rule_archive` writes the rule's work
//! folder in the collect and the archive folder, `episode_undo` writes the
//! rule's work folder, and `watch_rescan` reads its watch folder as every
//! reading does. The command waits for its turn and runs; the command modules
//! take no turn themselves (a task never waits for a second one). So two
//! commands on one work folder run one after the other in the order they were
//! accepted, and a cycle's add into a work folder that is moving or receiving
//! waits for it.
//!
//! The folders are named when the command is claimed, from the rule and the
//! collect folder as they are then; a change to either before the command
//! runs is not seen, nor is a link that reaches the same folder by another
//! name. Each command checks the rule and the folders again when it runs, so
//! a stale turn only orders work that did not need ordering, or orders less.
//!
//! The commands that may add, move, rename or take labels off torrents (all
//! but `watch_rescan`) then take the torrent gate ([`crate::removal`]) and
//! hold it until their end is written, so the cycle's removal of departed
//! torrents never runs between their add and its record.
//!
//! # Starts that do not finish
//!
//! A command claimed by a worker that then died stays `running`; the next
//! worker to hold the lock claims it again (see [`trss_core::commands::CommandStore::claim_next`]).
//! Adding a torrent Transmission already has answers `duplicate`, so running
//! such a command a second time does not add a second torrent.
//!
//! A process killed after Transmission took the torrent and before the result
//! was written leaves a torrent whose hash history does not know. Until the
//! command has run again and recorded it, a cycle's removal finds the command
//! `running` from an earlier start and removes nothing (see [`crate::removal`]).
//! The rerun then meets the torrent, which carries the command's label, and
//! takes it as its own add (`received`, renamed and noted).
//!
//! A request to add a torrent that was sent and got no answer (it timed out,
//! say) may have been taken all the same. The command is not ended then: it is
//! marked ([`trss_core::commands::CommandStore::note_unconfirmed_add`]) and stays `running` for a
//! later look (at the earliest the next look of the worker's own,
//! [`DEFAULT_COMMAND_POLL`] later), and removals in between remove nothing, as
//! above. The next start adds the item again; Transmission answers `duplicate`
//! with the hash, and a torrent carrying the command's label counts as this
//! command's own (`received`, renamed and noted). A connection that could not
//! be made at all sent nothing and fails the command at once. Once marked, a
//! start that fails for a reason no later start can get past (the channel was
//! deleted, the folder is refused) ends the command at once too; the torrent's
//! item label keeps it while its item is in a feed. A start that could not be
//! carried through for another reason (the database failed) is left for a
//! later look the same way.
//!
//! The last start ([`trss_core::commands::MAX_ATTEMPTS`]) ends the command
//! instead, and so does a task that ended in a panic, with the unanswered add
//! recorded ([`trss_core::commands::CommandStore::finish_with_unconfirmed_add`]): Transmission may
//! hold its torrent under a hash history never learned, and the next cycle
//! removes no departed torrents either (see [`crate::removal`]).
//!
//! # Shutdown
//!
//! Once shutdown is asked for no command is claimed, and the ones under way
//! get [`crate::SHUTDOWN_GRACE`] to wind down before they are aborted; an
//! aborted command stays `running` and runs again at the next start.
//!
//! # The kinds
//!
//! A `receive_past` command (`받기` of a past episode search's result) is a
//! `receive_once` of a result no feed showed: it records the result as a past
//! item and receives it the same way ([`receive_past`]).
//!
//! A `rule_archive` command (보관·복원) moves a work folder on disk. It holds
//! the folder's turn, so no cycle adds a torrent into the folder while it
//! moves, and a start cut short leaves it `running`: the next start looks at
//! the disk and Transmission again and moves what is left (see
//! [`rule_archive`]). Its blocking renames keep the worker's lock themselves
//! ([`trss_core::WorkerHold::keep`]), so a shutdown that aborts the command's
//! task lets go of the lock only once they have returned.
//!
//! A `watch_rescan` command (`다시 확인` of a watch folder) reads the folder
//! again, like the cycles do, and only reads ([`watch_rescan`]); it runs
//! beside a move in another folder.
//!
//! An `episode_undo` command (`되돌리기` of an episode offset the app set) puts
//! the rule's previous offset back and renames the videos it named, through
//! Transmission or on disk without replacing anything ([`episode_undo`]). A
//! start cut short leaves it `running`, and the next start carries on with the
//! files still to rename. A start that cannot reach Transmission or the
//! database once the value is back ends the command with those files still
//! to rename, rather than holding up the folder for the commands behind it; a
//! new request (`이어서 되돌리기`) carries them on.
//!
//! Each command kind has its own module below.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use tokio::{net::UnixDatagram, task::JoinSet};
use tokio_util::sync::CancellationToken;

use trss_core::{
    commands::{Command, CommandState, Outcome},
    folder_locks::{Reservation, Section},
    WorkerHold,
};

use trss_collect::commands::{episode_undo, receive_once, receive_past, rule_archive};
use trss_library::watch_rescan;

use super::{removal, Worker, WorkerError};

/// How often the worker looks for waiting commands without being woken.
pub const DEFAULT_COMMAND_POLL: Duration = Duration::from_secs(3);

/// How many commands one worker carries out at the same time.
pub const MAX_COMMANDS_AT_ONCE: usize = 8;

/// How one start of a command came out, whatever its kind.
enum Ran {
    /// It ended; the command is ended with this.
    Ended {
        state: CommandState,
        outcome: Outcome,
        add_unconfirmed: bool,
    },
    /// A `receive_once` add got no answer; see the module docs.
    AddUnanswered,
    /// It could not be carried through now (the database failed, or shutdown
    /// was asked for) and stays `running` for a later look.
    NotNow(String),
}

/// What one look for commands came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandsOutcome {
    /// Nothing was waiting.
    Idle,
    /// Something is waiting, but another worker holds the lock (a cycle or
    /// a command is running there).
    Busy,
    /// This many commands were carried out to their end.
    Ran(usize),
}

/// The kinds of command this worker knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    ReceiveOnce,
    ReceivePast,
    RuleArchive,
    WatchRescan,
    EpisodeUndo,
}

impl Kind {
    fn of(command: &Command) -> Option<Kind> {
        Some(match command.kind.as_str() {
            receive_once::KIND => Kind::ReceiveOnce,
            receive_past::KIND => Kind::ReceivePast,
            rule_archive::KIND => Kind::RuleArchive,
            watch_rescan::KIND => Kind::WatchRescan,
            episode_undo::KIND => Kind::EpisodeUndo,
            _ => return None,
        })
    }

    /// Whether the command may add, move, rename or take labels off torrents,
    /// and so takes the torrent gate ([`crate::removal`]).
    fn touches_torrents(self) -> bool {
        self != Kind::WatchRescan
    }
}

/// How a command's task came out, for the look that started it.
enum Carried {
    /// The command ended (its end is written).
    Ended,
    /// It stays `running` for a later look.
    Later(String),
}

/// The commands one look (or the worker's long-running loop) has started.
#[derive(Default)]
struct Batch {
    tasks: JoinSet<Carried>,
    /// Commands left for a later look, and since when. Passed over by the
    /// claims until [`Batch::forget_deferred`] lets them go.
    deferred: HashMap<String, Instant>,
    /// How many commands ended.
    ended: usize,
}

impl Batch {
    fn settle(&mut self, joined: Result<Carried, tokio::task::JoinError>) {
        match joined {
            Ok(Carried::Ended) => self.ended += 1,
            Ok(Carried::Later(id)) => {
                self.deferred.insert(id, Instant::now());
            }
            // The task records its own end; a panic outside the command's
            // own task leaves the command `running` for the next start.
            Err(err) => eprintln!("Command task ended with an internal error: {err}"),
        }
    }

    /// Lets the commands left for later at least `after` ago be claimed again.
    fn forget_deferred(&mut self, after: Duration) {
        self.deferred.retain(|_, since| since.elapsed() < after);
    }
}

impl Worker {
    /// Overrides how often the worker looks for waiting commands without being
    /// woken (default: [`DEFAULT_COMMAND_POLL`]).
    pub fn with_command_poll(mut self, every: Duration) -> Self {
        self.command_poll = every;
        self
    }

    /// One look for commands: if this worker can take the lock, claims the
    /// waiting commands, carries them out side by side (each in its turn), and
    /// returns once all have come out. Returns without taking the lock when
    /// nothing is waiting. A command that is left for a later look is not
    /// claimed again by the same look and does not count.
    pub async fn run_commands(
        &self,
        cancel: &CancellationToken,
    ) -> Result<CommandsOutcome, WorkerError> {
        // Looking first keeps the lock free for another worker when there is
        // nothing to do.
        if !self.commands.has_open().await? {
            return Ok(CommandsOutcome::Idle);
        }
        let Some(hold) = self.hold().await? else {
            return Ok(CommandsOutcome::Busy);
        };
        let mut batch = Batch::default();
        let mut failed = None;
        loop {
            if failed.is_none() {
                if let Err(err) = self.fill(&mut batch, &hold, cancel).await {
                    failed = Some(err);
                }
            }
            let Some(joined) = batch.tasks.join_next().await else {
                break;
            };
            batch.settle(joined);
        }
        hold.release().await;
        match failed {
            Some(err) => Err(err),
            None => Ok(CommandsOutcome::Ran(batch.ended)),
        }
    }

    /// Where the web's wake-ups arrive, if this worker listens for them. The
    /// path is bound anew: a socket left by an earlier worker is replaced.
    pub(crate) fn listen_for_wakes(&self) -> Option<UnixDatagram> {
        let path = self.wake_path.as_ref()?;
        let _ = std::fs::remove_file(path);
        match UnixDatagram::bind(path) {
            Ok(socket) => Some(socket),
            Err(err) => {
                eprintln!(
                    "Cannot listen for the web's wake-ups at {}: {err}; \
                     looking for commands every {}s",
                    path.display(),
                    self.command_poll.as_secs()
                );
                None
            }
        }
    }

    /// The worker's commands until `cancel` fires: a look whenever the web
    /// wakes the worker, a command ends, or [`DEFAULT_COMMAND_POLL`] passes.
    /// Then the commands under way get the grace period and are aborted.
    pub(crate) async fn dispatch(&self, cancel: CancellationToken, mut wake: Option<UnixDatagram>) {
        let mut batch = Batch::default();
        let mut ticker = tokio::time::interval(self.command_poll);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut buf = [0u8; 16];

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                Some(joined) = batch.tasks.join_next(), if !batch.tasks.is_empty() => {
                    batch.settle(joined);
                }
                received = async { wake.as_ref().expect("guarded").recv(&mut buf).await },
                    if wake.is_some() =>
                {
                    if let Err(err) = received {
                        eprintln!(
                            "Cannot hear the web's wake-ups: {err}; looking for commands every {}s",
                            self.command_poll.as_secs()
                        );
                        wake = None;
                    }
                }
                _ = ticker.tick() => batch.forget_deferred(self.command_poll),
            }
            if let Err(err) = self.look(&mut batch, &cancel).await {
                eprintln!("Commands failed: {err}");
            }
        }

        let wind_down = async {
            while let Some(joined) = batch.tasks.join_next().await {
                batch.settle(joined);
            }
        };
        if tokio::time::timeout(self.shutdown_grace, wind_down)
            .await
            .is_err()
        {
            batch.tasks.abort_all();
            while batch.tasks.join_next().await.is_some() {}
            println!(
                "Commands abandoned: they did not wind down in time after the shutdown request"
            );
        }
    }

    /// One look of the long-running loop: starts what is waiting, if there is
    /// room and the lock can be had.
    async fn look(&self, batch: &mut Batch, cancel: &CancellationToken) -> Result<(), WorkerError> {
        if cancel.is_cancelled()
            || batch.tasks.len() >= MAX_COMMANDS_AT_ONCE
            || !self.commands.has_open().await?
        {
            return Ok(());
        }
        // Another worker holds the lock: the next look tries again.
        let Some(hold) = self.hold().await? else {
            return Ok(());
        };
        let filled = self.fill(batch, &hold, cancel).await;
        hold.release().await;
        filled
    }

    /// Claims waiting commands and starts them, oldest first, while there is
    /// room. Each takes its place in line for its folders before the next is
    /// claimed, so their turns come in the order they were accepted.
    async fn fill(
        &self,
        batch: &mut Batch,
        hold: &WorkerHold,
        cancel: &CancellationToken,
    ) -> Result<(), WorkerError> {
        while batch.tasks.len() < MAX_COMMANDS_AT_ONCE && !cancel.is_cancelled() {
            let mut excluded = self.in_flight.ids();
            excluded.extend(batch.deferred.keys().cloned());
            let Some(command) = self
                .commands
                .claim_next_excluding((self.clock)(), excluded)
                .await?
            else {
                break;
            };
            println!(
                "Command {} ({}), attempt {}",
                command.id, command.kind, command.attempts
            );
            let entry = self.in_flight.enter(&command);

            let Some(kind) = Kind::of(&command) else {
                let outcome = Outcome {
                    result: "failed".to_owned(),
                    reason: Some(format!("모르는 명령이에요: {}", command.kind)),
                };
                self.commands
                    .finish(&command.id, CommandState::Failed, outcome, (self.clock)())
                    .await?;
                batch.ended += 1;
                drop(entry);
                continue;
            };
            let section = match self.section(kind, &command).await {
                Ok(section) => section,
                Err(err) => {
                    eprintln!("Command {} not finished: {err}", command.id);
                    batch.deferred.insert(command.id.clone(), Instant::now());
                    continue;
                }
            };
            let turn = self.ctx.folders.reserve(section);
            batch.tasks.spawn(self.clone().carry_out(
                command,
                kind,
                turn,
                entry,
                hold.clone(),
                cancel.clone(),
            ));
        }
        Ok(())
    }

    /// The folders `command` works in (see the module docs).
    async fn section(&self, kind: Kind, command: &Command) -> Result<Section, String> {
        let text = |err: &dyn std::fmt::Display| err.to_string();
        match kind {
            Kind::ReceiveOnce => receive_once::section(&self.ctx.receive(), command)
                .await
                .map_err(|e| text(&e)),
            Kind::ReceivePast => receive_past::section(&self.ctx.receive(), command)
                .await
                .map_err(|e| text(&e)),
            Kind::RuleArchive => rule_archive::section(&self.ctx.archive(), command)
                .await
                .map_err(|e| text(&e)),
            Kind::EpisodeUndo => episode_undo::section(&self.ctx.undo(), command)
                .await
                .map_err(|e| text(&e)),
            Kind::WatchRescan => watch_rescan::section(&self.watch, command)
                .await
                .map_err(|e| text(&e)),
        }
    }

    /// A claimed command's task: waits for its turn (and the torrent gate),
    /// runs the command, and writes how it came out.
    async fn carry_out(
        self,
        command: Command,
        kind: Kind,
        turn: Reservation,
        entry: removal::Entry,
        hold: WorkerHold,
        cancel: CancellationToken,
    ) -> Carried {
        let _turn = turn.ready().await;
        let gate = match kind.touches_torrents() {
            true => Some(self.torrents.command().await),
            false => None,
        };
        // Dropped in this order, whatever ends the task: the command leaves
        // the in-flight set before the gate is let go, so a removal never
        // finds it neither running here nor `running` from an earlier start.
        let started = (entry, gate);

        let ran = self.run_kind(&command, kind, &hold, &cancel).await;
        let carried = match self.record(&command, ran).await {
            Ok(carried) => carried,
            Err(err) => {
                eprintln!("Command {} not finished: {err}", command.id);
                Carried::Later(command.id.clone())
            }
        };
        drop(started);
        drop(_turn);
        hold.release().await;
        carried
    }

    /// Runs one start of a command in its own task, so that a panic ends the
    /// command, not the worker. `None`: the task panicked.
    async fn run_kind(
        &self,
        command: &Command,
        kind: Kind,
        hold: &WorkerHold,
        cancel: &CancellationToken,
    ) -> Option<Ran> {
        let mut task = JoinSet::new();
        let (watch, clock, owned) = (self.watch.clone(), self.clock.clone(), command.clone());
        let cancel = cancel.clone();
        match kind {
            Kind::ReceiveOnce => {
                let ctx = self.ctx.receive();
                task.spawn(async move {
                    match receive_once::run(&ctx, &owned, || clock(), &cancel).await {
                        Ok(finished) => Ran::Ended {
                            state: finished.state,
                            outcome: finished.outcome,
                            add_unconfirmed: finished.add_unconfirmed,
                        },
                        Err(receive_once::Retry::AddUnanswered) => Ran::AddUnanswered,
                        Err(err) => Ran::NotNow(err.to_string()),
                    }
                });
            }
            Kind::ReceivePast => {
                let ctx = self.ctx.receive();
                task.spawn(async move {
                    match receive_past::run(&ctx, &owned, || clock(), &cancel).await {
                        Ok(finished) => Ran::Ended {
                            state: finished.state,
                            outcome: finished.outcome,
                            add_unconfirmed: finished.add_unconfirmed,
                        },
                        Err(receive_once::Retry::AddUnanswered) => Ran::AddUnanswered,
                        Err(err) => Ran::NotNow(err.to_string()),
                    }
                });
            }
            Kind::RuleArchive => {
                // Kept by the move's blocking renames until they return.
                let keep = hold.keep();
                let ctx = self.ctx.archive();
                task.spawn(async move {
                    match rule_archive::run(&ctx, &owned, keep, &clock, &cancel).await {
                        Ok(finished) => Ran::Ended {
                            state: finished.state,
                            outcome: finished.outcome,
                            add_unconfirmed: false,
                        },
                        Err(err) => Ran::NotNow(err.to_string()),
                    }
                });
            }
            Kind::WatchRescan => {
                task.spawn(async move {
                    match watch_rescan::run(&watch, &owned, &clock).await {
                        Ok(finished) => Ran::Ended {
                            state: finished.state,
                            outcome: finished.outcome,
                            add_unconfirmed: false,
                        },
                        Err(err) => Ran::NotNow(err.to_string()),
                    }
                });
            }
            Kind::EpisodeUndo => {
                let ctx = self.ctx.undo();
                task.spawn(async move {
                    match episode_undo::run(&ctx, &owned, &clock).await {
                        Ok(finished) => Ran::Ended {
                            state: finished.state,
                            outcome: finished.outcome,
                            add_unconfirmed: false,
                        },
                        Err(err) => Ran::NotNow(err.to_string()),
                    }
                });
            }
        }
        match task.join_next().await {
            Some(Ok(ran)) => Some(ran),
            Some(Err(err)) => {
                eprintln!("Command {} ended with an internal error: {err}", command.id);
                None
            }
            None => None,
        }
    }

    /// Writes how one start of `command` came out (`None`: its task panicked).
    async fn record(&self, command: &Command, ran: Option<Ran>) -> Result<Carried, WorkerError> {
        let (state, outcome, add_unconfirmed) = match ran {
            Some(Ran::Ended {
                state,
                outcome,
                add_unconfirmed,
            }) => (state, outcome, add_unconfirmed),
            Some(Ran::AddUnanswered) => {
                println!(
                    "Command {}: Transmission did not answer the add; trying again at a later look",
                    command.id
                );
                self.commands
                    .note_unconfirmed_add(&command.id, (self.clock)())
                    .await?;
                return Ok(Carried::Later(command.id.clone()));
            }
            Some(Ran::NotNow(err)) => {
                eprintln!("Command {} not finished: {err}", command.id);
                return Ok(Carried::Later(command.id.clone()));
            }
            None => {
                let outcome = Outcome {
                    result: "failed".to_owned(),
                    reason: Some("처리하다 내부 오류가 났어요.".to_owned()),
                };
                // The task may have ended after Transmission took a torrent and
                // before its hash was recorded, as an unconfirmed add may.
                self.commands
                    .finish_with_unconfirmed_add(
                        &command.id,
                        CommandState::Failed,
                        outcome,
                        (self.clock)(),
                    )
                    .await?;
                return Ok(Carried::Ended);
            }
        };

        let now = (self.clock)();
        let result = outcome.result.clone();
        if add_unconfirmed {
            self.commands
                .finish_with_unconfirmed_add(&command.id, state, outcome, now)
                .await?;
        } else {
            self.commands
                .finish(&command.id, state, outcome, now)
                .await?;
        }
        println!("Command {} {}: {}", command.id, state, result);
        Ok(Carried::Ended)
    }
}
