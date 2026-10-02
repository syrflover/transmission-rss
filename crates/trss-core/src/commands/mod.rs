//! Web commands: user actions the web accepts and the worker carries out.
//!
//! A command is the first thing that crosses from the web to the worker. The
//! contract (`docs/specs/web-app.md`, 웹 명령과 상태 갱신) is built here so
//! later commands reuse it:
//!
//! - **The browser makes the ID.** One user action has one command ID, sent
//!   together with the request's content. The web stores the command before it
//!   answers, and the answer only says *accepted*: the work has not been done.
//! - **A repeat is harmless.** [`CommandStore::accept`] returns the stored
//!   command when the same ID comes again with the same kind and content, and
//!   refuses ([`Accepted::Mismatch`]) when the content differs. The work is
//!   queued once however often the request is delivered.
//! - **A lost answer is looked up, not resent under a new ID.** The command is
//!   read by its ID ([`CommandStore::get`]); it is either there (with its
//!   state) or was never stored.
//! - **The worker executes under the worker lock.** [`CommandStore::claim_next`]
//!   hands out the oldest open command and marks it `running`. A worker claims
//!   only while it holds the worker lock ([`crate::WorkerLock`]), which one
//!   worker at a time holds, and it passes over the commands it is running
//!   itself ([`CommandStore::claim_next_excluding`]). So a command still
//!   `running` when a worker claims it is one an earlier start did not finish
//!   (its worker died or was stopped, or left it for a later look), and it is
//!   handed out again. A command that keeps ending its worker is given up
//!   after [`MAX_ATTEMPTS`] tries.
//! - **The end is recorded once**, as `done` or `failed`, with an
//!   [`Outcome`] the screen can show. Outcomes and everything else stored here
//!   are free of secret values.
//!
//! Kinds and payloads are opaque here: a kind's meaning (its payload shape,
//! validation and execution) lives with the code that accepts and runs it.

mod repo;
#[cfg(test)]
mod tests;

use std::{collections::HashMap, fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{
    db::{Db, DbError},
    Millis,
};

/// How many times a command may be started. A command that was started this
/// often and never finished is failed instead of started again, so one that
/// stops its worker every time cannot keep the worker from restarting.
pub const MAX_ATTEMPTS: i64 = 5;

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("the stored outcome of command {0} cannot be read")]
    Outcome(String),
}

impl From<rusqlite::Error> for CommandError {
    fn from(e: rusqlite::Error) -> Self {
        CommandError::Db(DbError::Sqlite(e))
    }
}

/// Where a command is. `Done` and `Failed` are final.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandState {
    /// Accepted; no worker has started it.
    Pending,
    /// A worker started it and has not finished.
    Running,
    /// It ran to its end (the outcome says how).
    Done,
    /// It ended without doing what was asked (the outcome says why).
    Failed,
}

impl CommandState {
    pub fn code(self) -> &'static str {
        match self {
            CommandState::Pending => "pending",
            CommandState::Running => "running",
            CommandState::Done => "done",
            CommandState::Failed => "failed",
        }
    }

    /// True until the command has ended.
    pub fn is_open(self) -> bool {
        matches!(self, CommandState::Pending | CommandState::Running)
    }
}

impl fmt::Display for CommandState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for CommandState {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "pending" => Ok(CommandState::Pending),
            "running" => Ok(CommandState::Running),
            "done" => Ok(CommandState::Done),
            "failed" => Ok(CommandState::Failed),
            _ => Err(()),
        }
    }
}

/// How a command ended. `result` is a code of the command's kind (for
/// `receive_once`, a history result code); `reason` explains a failure in a
/// sentence the screen may show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub result: String,
    pub reason: Option<String>,
}

/// A stored command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub id: String,
    pub kind: String,
    /// The request's content in canonical JSON.
    pub payload: String,
    pub subject: Option<String>,
    pub state: CommandState,
    /// How many times a worker started it.
    pub attempts: i64,
    pub created_at: Millis,
    pub updated_at: Millis,
    pub finished_at: Option<Millis>,
    pub outcome: Option<Outcome>,
    /// A request of this command to add a torrent got no answer from
    /// Transmission, so Transmission may hold a torrent whose hash history did
    /// not learn. Set on a running command by
    /// [`CommandStore::note_unconfirmed_add`], and kept or cleared when it ends.
    pub add_unconfirmed: bool,
    /// The name Transmission reported for the single file of the torrent this
    /// command put in, before the command first renamed it. Set once by
    /// [`CommandStore::note_original_name`].
    pub original_name: Option<String>,
}

/// A command to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCommand {
    /// Made by the browser, one per user action.
    pub id: String,
    pub kind: String,
    /// Canonical JSON: two requests with the same content must have the same
    /// text, since it is compared to tell a repeat from a different request.
    pub payload: String,
    /// What the command is about (for `receive_once`, the history item's ID).
    pub subject: Option<String>,
}

/// What [`CommandStore::accept`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accepted {
    /// Stored now, `pending`.
    Created(Command),
    /// The same ID with the same kind and content had been accepted before;
    /// this is the stored command, in whatever state it has reached.
    Existing(Command),
    /// The ID belongs to a command with a different kind or content. Nothing
    /// was stored; this is the stored command.
    Mismatch(Command),
    /// Another command of the same kind and subject is still open. Nothing was
    /// stored; this is that command.
    Busy(Command),
}

/// Async access to the commands. Cheap to clone.
#[derive(Clone)]
pub struct CommandStore {
    db: Db,
}

impl CommandStore {
    pub fn new(db: Db) -> Self {
        CommandStore { db }
    }

    /// Stores `command` as `pending` at time `now`, unless its ID is known or
    /// another command of the same kind and subject is still open. The check
    /// and the insert are one write transaction, so two deliveries of the same
    /// ID at once store one command.
    pub async fn accept(&self, command: NewCommand, now: Millis) -> Result<Accepted, CommandError> {
        self.db.run(move |c| repo::accept(c, &command, now)).await
    }

    pub async fn get(&self, id: &str) -> Result<Option<Command>, CommandError> {
        let id = id.to_owned();
        self.db.run(move |c| repo::get(c, &id)).await
    }

    /// The open command of `kind` for each of the given subjects, if any.
    pub async fn open_for_subjects(
        &self,
        kind: &str,
        subjects: Vec<String>,
    ) -> Result<HashMap<String, Command>, CommandError> {
        let kind = kind.to_owned();
        self.db
            .run(move |c| repo::open_for_subjects(c, &kind, &subjects))
            .await
    }

    /// The latest command of `kind` for each of the given subjects, open or
    /// ended, if any: what a screen shows as the last thing done to a subject.
    pub async fn latest_for_subjects(
        &self,
        kind: &str,
        subjects: Vec<String>,
    ) -> Result<HashMap<String, Command>, CommandError> {
        let kind = kind.to_owned();
        self.db
            .run(move |c| repo::latest_for_subjects(c, &kind, &subjects))
            .await
    }

    /// Whether any command is waiting for a worker (a cheap check the worker
    /// makes before it bothers taking the lock).
    pub async fn has_open(&self) -> Result<bool, CommandError> {
        self.db.run(|c| repo::has_open(c)).await
    }

    /// How many commands are `running`. Asked while holding the worker lock,
    /// these are commands a worker started and did not end: it died in them,
    /// or stopped them to retry later, or is running them now.
    pub async fn running_count(&self) -> Result<usize, CommandError> {
        self.running_count_excluding(Vec::new()).await
    }

    /// [`CommandStore::running_count`] without the commands whose IDs are in
    /// `excluded` (the ones the asking worker is running now and knows about).
    pub async fn running_count_excluding(
        &self,
        excluded: Vec<String>,
    ) -> Result<usize, CommandError> {
        self.db
            .run(move |c| repo::running_count_excluding(c, &excluded))
            .await
    }

    /// Hands out the oldest open command to run and marks it `running`, one
    /// more attempt. Call it only while holding the worker lock (see the module
    /// docs). A command already started [`MAX_ATTEMPTS`] times is failed on the
    /// way instead of handed out.
    pub async fn claim_next(&self, now: Millis) -> Result<Option<Command>, CommandError> {
        self.claim_next_excluding(now, Vec::new()).await
    }

    /// [`CommandStore::claim_next`], passing over the open commands whose IDs
    /// are in `excluded`: the ones the worker is running now, and the ones it
    /// leaves for a later look. They are neither handed out nor given up.
    pub async fn claim_next_excluding(
        &self,
        now: Millis,
        excluded: Vec<String>,
    ) -> Result<Option<Command>, CommandError> {
        self.db
            .run(move |c| repo::claim_next_excluding(c, now, &excluded))
            .await
    }

    /// Ends a command. Only an open command changes; the return value tells
    /// whether this call ended it.
    pub async fn finish(
        &self,
        id: &str,
        state: CommandState,
        outcome: Outcome,
        now: Millis,
    ) -> Result<bool, CommandError> {
        self.end(id, state, outcome, now, false).await
    }

    /// Ends a command, like [`CommandStore::finish`], whose request to add a
    /// torrent got no answer from Transmission: Transmission may have taken the
    /// torrent without history learning its hash. See
    /// [`CommandStore::unconfirmed_adds_since`].
    pub async fn finish_with_unconfirmed_add(
        &self,
        id: &str,
        state: CommandState,
        outcome: Outcome,
        now: Millis,
    ) -> Result<bool, CommandError> {
        self.end(id, state, outcome, now, true).await
    }

    /// Records on a running command that its request to add a torrent got no
    /// answer, before the command is left for another start (see
    /// [`Command::add_unconfirmed`]). The return value tells whether the
    /// command was running.
    pub async fn note_unconfirmed_add(&self, id: &str, now: Millis) -> Result<bool, CommandError> {
        let id = id.to_owned();
        self.db
            .run(move |c| repo::note_unconfirmed_add(c, &id, now))
            .await
    }

    /// Records the name `name` the running command's torrent had before the
    /// command renamed it, unless a name is recorded already, and returns the
    /// recorded name (`None` when the command is not running). Leaves
    /// `updated_at` alone: nothing the screen shows changes.
    pub async fn note_original_name(
        &self,
        id: &str,
        name: &str,
    ) -> Result<Option<String>, CommandError> {
        let (id, name) = (id.to_owned(), name.to_owned());
        self.db
            .run(move |c| repo::note_original_name(c, &id, &name))
            .await
    }

    async fn end(
        &self,
        id: &str,
        state: CommandState,
        outcome: Outcome,
        now: Millis,
        add_unconfirmed: bool,
    ) -> Result<bool, CommandError> {
        assert!(!state.is_open(), "a command ends as done or failed");
        let id = id.to_owned();
        self.db
            .run(move |c| repo::finish(c, &id, state, &outcome, now, add_unconfirmed))
            .await
    }

    /// How many commands with an unconfirmed add (see
    /// [`CommandStore::finish_with_unconfirmed_add`]) ended at or after
    /// `since`, or at all when `since` is `None`.
    pub async fn unconfirmed_adds_since(
        &self,
        since: Option<Millis>,
    ) -> Result<usize, CommandError> {
        self.db
            .run(move |c| repo::unconfirmed_adds_since(c, since))
            .await
    }
}
