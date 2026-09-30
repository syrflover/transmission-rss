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
//! such a command a second time does not add a second torrent. The one gap is
//! a process killed after Transmission took the torrent and before the result
//! was written: the command is then run again and ends as `duplicate` instead
//! of `received`.
//!
//! Each command kind has its own module below.

pub mod folder;
pub mod link;
pub mod receive_once;

use std::time::Duration;

use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::{CycleLock, Worker, WorkerError};
use crate::store::commands::{Command, CommandState, Outcome};

/// How often the worker looks for waiting commands.
pub const DEFAULT_COMMAND_POLL: Duration = Duration::from_secs(3);

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
        let Some(_lock) = lock else {
            return Ok(CommandsOutcome::Busy);
        };

        let mut ran = 0;
        while !cancel.is_cancelled() {
            let Some(command) = self.commands.claim_next((self.clock)()).await? else {
                break;
            };
            if !self.run_command(&command, cancel).await? {
                break;
            }
            ran += 1;
        }
        Ok(CommandsOutcome::Ran(ran))
    }

    /// Runs one claimed command and ends it. `false` means it could not be
    /// carried through now (shutdown, or the database failed) and stays
    /// `running` for the next look.
    async fn run_command(
        &self,
        command: &Command,
        cancel: &CancellationToken,
    ) -> Result<bool, WorkerError> {
        println!(
            "Command {} ({}), attempt {}",
            command.id, command.kind, command.attempts
        );

        // In its own task so that a panic ends the command, not the worker.
        let mut task = JoinSet::new();
        let (ctx, clock, owned) = (self.ctx.clone(), self.clock.clone(), command.clone());
        match command.kind.as_str() {
            receive_once::KIND => {
                task.spawn(async move { receive_once::execute(&ctx, &owned, || clock()).await });
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

        let finished = match task.join_next().await {
            Some(Ok(Ok(finished))) => finished,
            Some(Ok(Err(err))) => {
                eprintln!("Command {} not finished: {err}", command.id);
                return Ok(false);
            }
            Some(Err(err)) => {
                eprintln!("Command {} ended with an internal error: {err}", command.id);
                let outcome = Outcome {
                    result: "failed".to_owned(),
                    reason: Some("처리하다 내부 오류가 났어요.".to_owned()),
                };
                self.commands
                    .finish(&command.id, CommandState::Failed, outcome, (self.clock)())
                    .await?;
                return Ok(true);
            }
            None => return Ok(false),
        };

        self.commands
            .finish(
                &command.id,
                finished.state,
                finished.outcome.clone(),
                (self.clock)(),
            )
            .await?;
        println!(
            "Command {} {}: {}",
            command.id, finished.state, finished.outcome.result
        );

        if let Some(rename) = &finished.rename {
            if let receive_once::RenameResult::Kept(note) =
                receive_once::rename(&self.ctx, rename, cancel).await
            {
                // A note on the item; the command has ended already, so a
                // failure here is only reported.
                if let Err(err) = self.ctx.history.note_received(rename.item_id, note).await {
                    eprintln!(
                        "Cannot note the kept name on item {}: {err}",
                        rename.item_id
                    );
                }
            }
        }
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
