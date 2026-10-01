//! Carrying out the commands the web accepted (see [`crate::store::commands`]).
//!
//! The worker looks for waiting commands every [`DEFAULT_COMMAND_POLL`] (a few
//! seconds, not the cycle interval) and runs them **only while holding the
//! same lock as the collection cycles** ([`super::lock::CycleLock`]):
//!
//! - two workers never run one command twice, because only the holder claims;
//! - a command never interleaves with a cycle, which matters because a cycle
//!   removes Transmission torrents it cannot account for;
//! - a busy lock is not an error: the poll after next tries again. A cycle in
//!   progress therefore delays a command until it ends.
//!
//! A command claimed by a worker that then died stays `running`; the next
//! worker to hold the lock claims it again (see [`CommandStore::claim_next`]).
//! Adding a torrent Transmission already has answers `duplicate`, so running
//! such a command a second time does not add a second torrent.
//!
//! A process killed after Transmission took the torrent and before the result
//! was written leaves a torrent whose hash history does not know. A restarted
//! worker runs a collection cycle before it looks for commands, and that cycle
//! would remove the torrent as departed; so a cycle that starts while any
//! command is `running` removes nothing (see [`super::CommandsAtStart`]). The
//! rerun then meets the torrent, which carries the command's label, and takes
//! it as its own add (`received`, renamed and noted).
//!
//! A request to add a torrent that was sent and got no answer (it timed out,
//! say) may have been taken all the same. The command is not ended then: it is
//! marked ([`CommandStore::note_unconfirmed_add`]) and stays `running` for the
//! next look, and cycles in between remove nothing, as above. The next start
//! adds the item again; Transmission answers `duplicate` with the hash, and a
//! torrent carrying the command's label counts as this command's own
//! (`received`, renamed and noted). A connection that
//! could not be made at all sent nothing and fails the command at once. Once
//! marked, a start that fails for a reason no later start can get past (the
//! channel was deleted, the folder is refused) ends the command at once too;
//! the torrent's item label keeps it while its item is in a feed.
//!
//! The last start ([`crate::store::commands::MAX_ATTEMPTS`]) ends the command
//! instead, and so does a task that ended in a panic, with the unanswered add
//! recorded ([`CommandStore::finish_with_unconfirmed_add`]): Transmission may
//! hold its torrent under a hash history never learned, and the next cycle
//! removes no departed torrents either (see [`super::CommandsAtStart`]).
//!
//! A `receive_past` command (`받기` of a past episode search's result) is a
//! `receive_once` of a result no feed showed: it records the result as a past
//! item and receives it the same way ([`receive_past`]).
//!
//! A `rule_archive` command (보관·복원) moves a work folder on disk. It runs
//! under the same lock, so no cycle adds a torrent into the folder while it
//! moves, and a start cut short leaves it `running`: the next start looks at
//! the disk and Transmission again and moves what is left (see
//! [`rule_archive`]). Its blocking renames hold the lock themselves, so a
//! shutdown that aborts the command's task releases the lock only once they
//! have returned.
//!
//! A `watch_rescan` command (`다시 확인` of a watch folder) reads the folder
//! again, like the cycles do, and only reads ([`watch_rescan`]).
//!
//! An `episode_undo` command (`되돌리기` of an episode offset the app set) puts
//! the rule's previous offset back and renames the videos it named, through
//! Transmission or on disk without replacing anything ([`episode_undo`]). A
//! start cut short leaves it `running`, and the next start carries on with the
//! files still to rename. A start that cannot reach Transmission or the
//! database once the value is back ends the command with those files still
//! to rename, rather than holding up the commands behind it; a new request
//! (`이어서 되돌리기`) carries them on.
//!
//! Each command kind has its own module below.

pub mod episode_undo;
pub mod link;
pub mod receive_once;
pub mod receive_past;
pub mod rule_archive;
pub mod watch_rescan;

use std::{sync::Arc, time::Duration};

use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::{CycleLock, Worker, WorkerError};
use crate::store::commands::{Command, CommandState, Outcome};

/// How often the worker looks for waiting commands.
pub const DEFAULT_COMMAND_POLL: Duration = Duration::from_secs(3);

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
    /// was asked for) and stays `running` for the next look.
    NotNow(String),
}

/// What one look for commands came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandsOutcome {
    /// Nothing was waiting.
    Idle,
    /// Something is waiting, but another worker holds the lock (a cycle or
    /// another command is running there).
    Busy,
    /// This many commands were carried out.
    Ran(usize),
}

impl Worker {
    /// Overrides how often the worker looks for waiting commands (default:
    /// [`DEFAULT_COMMAND_POLL`]).
    pub fn with_command_poll(mut self, every: Duration) -> Self {
        self.command_poll = every;
        self
    }

    /// Carries out the waiting commands, oldest first, if this worker can take
    /// the lock. Returns without taking it when nothing is waiting.
    pub async fn run_commands(
        &self,
        cancel: &CancellationToken,
    ) -> Result<CommandsOutcome, WorkerError> {
        // Looking first keeps the lock free for the cycle when there is nothing to do.
        if !self.commands.has_open().await? {
            return Ok(CommandsOutcome::Idle);
        }
        let lock = CycleLock::try_acquire(&self.lock_path).map_err(|source| WorkerError::Lock {
            path: self.lock_path.clone(),
            source,
        })?;
        let Some(lock) = lock else {
            return Ok(CommandsOutcome::Busy);
        };
        // Shared with work that must keep the lock until it returns, even
        // after this task is aborted (a `rule_archive` rename in progress).
        let lock: rule_archive::work_folder::Hold = Arc::new(lock);

        // The web sees the worker busy, not stopped, for as long as a command
        // runs; the beat ends before the lock is let go.
        self.beating(async {
            let mut ran = 0;
            while !cancel.is_cancelled() {
                let Some(command) = self.commands.claim_next((self.clock)()).await? else {
                    break;
                };
                if !self.run_command(&command, &lock, cancel).await? {
                    break;
                }
                ran += 1;
            }
            Ok(CommandsOutcome::Ran(ran))
        })
        .await
    }

    /// Runs one claimed command and ends it. `false` means it could not be
    /// carried through now (shutdown, or the database failed) and stays
    /// `running` for the next look.
    async fn run_command(
        &self,
        command: &Command,
        lock: &rule_archive::work_folder::Hold,
        cancel: &CancellationToken,
    ) -> Result<bool, WorkerError> {
        println!(
            "Command {} ({}), attempt {}",
            command.id, command.kind, command.attempts
        );

        // In its own task so that a panic ends the command, not the worker.
        let mut task = JoinSet::new();
        let (ctx, clock, owned) = (self.ctx.clone(), self.clock.clone(), command.clone());
        let cancel = cancel.clone();
        match command.kind.as_str() {
            receive_once::KIND => {
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
            receive_past::KIND => {
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
            rule_archive::KIND => {
                let lock = lock.clone();
                task.spawn(async move {
                    match rule_archive::run(&ctx, &owned, lock, &clock, &cancel).await {
                        Ok(finished) => Ran::Ended {
                            state: finished.state,
                            outcome: finished.outcome,
                            add_unconfirmed: false,
                        },
                        Err(err) => Ran::NotNow(err.to_string()),
                    }
                });
            }
            watch_rescan::KIND => {
                task.spawn(async move {
                    match watch_rescan::run(&ctx, &owned, &clock).await {
                        Ok(finished) => Ran::Ended {
                            state: finished.state,
                            outcome: finished.outcome,
                            add_unconfirmed: false,
                        },
                        Err(err) => Ran::NotNow(err.to_string()),
                    }
                });
            }
            episode_undo::KIND => {
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
            other => {
                let outcome = Outcome {
                    result: "failed".to_owned(),
                    reason: Some(format!("모르는 명령이에요: {other}")),
                };
                self.commands
                    .finish(&command.id, CommandState::Failed, outcome, (self.clock)())
                    .await?;
                return Ok(true);
            }
        }

        let (state, outcome, add_unconfirmed) = match task.join_next().await {
            Some(Ok(Ran::Ended {
                state,
                outcome,
                add_unconfirmed,
            })) => (state, outcome, add_unconfirmed),
            Some(Ok(Ran::AddUnanswered)) => {
                println!(
                    "Command {}: Transmission did not answer the add; trying again at the next look",
                    command.id
                );
                self.commands
                    .note_unconfirmed_add(&command.id, (self.clock)())
                    .await?;
                return Ok(false);
            }
            Some(Ok(Ran::NotNow(err))) => {
                eprintln!("Command {} not finished: {err}", command.id);
                return Ok(false);
            }
            Some(Err(err)) => {
                eprintln!("Command {} ended with an internal error: {err}", command.id);
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
                return Ok(true);
            }
            None => return Ok(false),
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
        Ok(true)
    }

    /// One look for commands from the worker's loop: the work runs in its own
    /// task, and after shutdown was asked for it gets [`super::SHUTDOWN_GRACE`]
    /// to wind down before it is aborted (an aborted command stays `running`
    /// and is run again at the next start).
    pub(super) async fn poll_commands(&self, cancel: &CancellationToken) {
        let worker = self.clone();
        let token = cancel.clone();
        let mut work = tokio::spawn(async move { worker.run_commands(&token).await });

        let joined = tokio::select! {
            joined = &mut work => joined,
            _ = async {
                cancel.cancelled().await;
                tokio::time::sleep(self.shutdown_grace).await;
            } => {
                work.abort();
                let _ = work.await;
                println!("Commands abandoned: they did not wind down in time after the shutdown request");
                return;
            }
        };
        match joined {
            Ok(Ok(_)) => {}
            Ok(Err(err)) => eprintln!("Commands failed: {err}"),
            Err(err) => eprintln!("Commands aborted: {err}"),
        }
    }
}
