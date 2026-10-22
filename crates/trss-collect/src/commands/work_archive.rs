//! `work_archive`, shown on the work detail as `보관 폴더로 옮기기` and `다시
//! 옮기기`: moves a work folder from the collect folder into the archive folder
//! (`docs/requirements/library.md`, 작품 상세 화면, and
//! `docs/requirements/collection.md`, 보관과 복원의 폴더 이동).
//!
//! The command is about a work folder, not a rule: the web deletes the rules
//! that save into the folder with it, in the transaction that stores it
//! ([`accept_deleting`]), so no new episode arrives at the old place while it
//! moves. Its payload is a [`WorkArchive`] (the folder's name). The worker runs it with [`run`], holding the turn of the
//! work folder in the collect folder and in the archive folder ([`section`]),
//! and moves it as `rule_archive` moves a rule's work folder
//! ([`super::rule_archive::work_folder`]): Transmission's torrents first, `.trss/`
//! along, into a folder of the same name when the archive folder has one,
//! nothing at all when a file is on both sides, renames within one file system
//! only. The library follows the folder, so the work keeps its ID
//! ([`trss_library::watch::follow_move`]).
//!
//! The command ends:
//!
//! - `moved` when the folder is in the archive folder now, or was found there
//!   already;
//! - `kept`, with the reason, when no collect or archive folder is set any
//!   more, when an active rule of any channel saves into the work folder again
//!   (one made after the web deleted the others), or when the collect folder
//!   has no such folder. A paused rule does not keep it: turning it on brings
//!   the folder back first (`rule_archive`'s `resume`);
//! - `failed`, with the reason, when the move fails. Nothing of the rules is
//!   left to retry from, so `다시 옮기기` is a new command of the same folder.
//!
//! Every step is safe to repeat, and where the folder is comes from the disk
//! each time, so a command cut short is claimed again by the next worker and
//! moves what is left, as `rule_archive` does.

use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use trss_core::{
    commands::{Accepted, Command, CommandError, CommandStore, NewCommand, MAX_ATTEMPTS},
    folder_locks::Section,
    Clock, Millis,
};

use super::rule_archive::{
    done, failed, held_reason, lexical_work_folder, move_folder, settings,
    work_folder::{Disk, Hold, RealDisk, Request, Side},
    ArchiveContext, Finished, NoMove, Retry, Start, WorkFolder, KEPT,
};
use crate::store::channels::{delete_rules_in, ChannelError, Rule, RuleState, Version};

/// The `kind` of the command.
pub const KIND: &str = "work_archive";

/// The content of a `work_archive` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkArchive {
    /// The work folder's name: the folder directly in the collect folder.
    pub work: String,
}

impl WorkArchive {
    /// The canonical text stored with the command.
    pub fn canonical(&self) -> String {
        serde_json::to_string(self).expect("a payload serializes")
    }

    /// The subject stored with the command: the work folder. One folder has
    /// at most one open command of this kind.
    pub fn subject(&self) -> String {
        self.work.clone()
    }

    /// The command to accept, under the ID the person's action made.
    pub fn command(&self, id: String) -> NewCommand {
        NewCommand {
            id,
            kind: KIND.to_owned(),
            payload: self.canonical(),
            subject: Some(self.subject()),
        }
    }
}

/// Stores the move `command` and deletes `rules`, each if it is still at the
/// version given with it, in one transaction: both, or neither. The rules are
/// the ones that saved into the work folder, so none of them adds into it
/// while it moves. A command whose ID is known, or a work folder with another
/// move open, deletes nothing ([`Accepted::Existing`], [`Accepted::Mismatch`],
/// [`Accepted::Busy`]); a rule that is gone or at another version stores
/// nothing (the inner `Err`). `commands` keeps its commands in the database
/// the rules are in.
pub async fn accept_deleting(
    commands: &CommandStore,
    command: NewCommand,
    rules: Vec<(String, Version)>,
    now: Millis,
) -> Result<Result<Accepted, ChannelError>, CommandError> {
    commands
        .accept_with(command, now, move |tx| delete_rules_in(tx, &rules))
        .await
}

/// The payload of `command`, when it names one folder by a plain name: a name
/// with a separator, `..` or nothing at all names no work folder.
fn read(command: &Command) -> Option<WorkArchive> {
    let payload: WorkArchive = serde_json::from_str(&command.payload).ok()?;
    let mut parts = Path::new(&payload.work).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(name)), None) if name == payload.work.as_str() => Some(payload),
        _ => None,
    }
}

/// The turn the command takes before it runs: a write of the work folder in the
/// collect folder and in the archive folder (the ones that are set). Empty when
/// the request cannot be read or no collect folder is set: the command then
/// ends by itself.
pub async fn section(ctx: &ArchiveContext, command: &Command) -> Result<Section, Retry> {
    let Some(payload) = read(command) else {
        return Ok(Section::new());
    };
    let Some((collect, archive)) = settings(ctx).await? else {
        return Ok(Section::new());
    };
    let section = Section::new().write(Path::new(&collect).join(&payload.work));
    Ok(match archive {
        Some(archive) => section.write(Path::new(&archive).join(&payload.work)),
        None => section,
    })
}

/// Runs a `work_archive` command to its end, with its turn ([`section`])
/// taken. See the module docs. `hold` keeps the folders' turn and the torrent
/// gate until the move's blocking work returns.
pub async fn run(
    ctx: &ArchiveContext,
    command: &Command,
    hold: Hold,
    clock: &Clock,
    cancel: &CancellationToken,
) -> Result<Finished, Retry> {
    run_on(ctx, command, Arc::new(RealDisk), hold, clock, cancel).await
}

/// [`run`] with the filesystems told by `disk`.
pub async fn run_on(
    ctx: &ArchiveContext,
    command: &Command,
    disk: Arc<dyn Disk>,
    hold: Hold,
    clock: &Clock,
    cancel: &CancellationToken,
) -> Result<Finished, Retry> {
    let start = Start {
        ctx,
        disk,
        hold,
        last: command.attempts >= MAX_ATTEMPTS,
        cancel,
        clock,
        requested_at: command.created_at,
    };
    let Some(payload) = read(command) else {
        return Ok(failed("요청 내용을 읽지 못했어요."));
    };
    let request = match plan(settings(ctx).await?, payload.work) {
        Ok(request) => request,
        Err(no_move) => return Ok(done(KEPT, Some(no_move.reason()))),
    };

    let rules: Vec<Rule> = ctx
        .channels
        .list_channels_with_rules()
        .await
        .map_err(|e| Retry::Store(e.to_string()))?
        .into_iter()
        .flat_map(|cwr| cwr.rules)
        .collect();
    let collecting = WorkFolder::Named(request.name.clone());
    let holding: Vec<&Rule> = rules
        .iter()
        .filter(|rule| rule.state == RuleState::Active)
        .filter(|rule| lexical_work_folder(&request.from_root, &rule.directory) == collecting)
        .collect();
    if !holding.is_empty() {
        return Ok(done(KEPT, Some(held_reason(&holding))));
    }

    Ok(move_folder(&start, &request)
        .await?
        .unwrap_or_else(|failed| failed))
}

/// The move of the work folder `name` into the archive folder, or why there is
/// none.
fn plan(settings: Option<(String, Option<String>)>, name: String) -> Result<Request, NoMove> {
    let Some((collect, archive)) = settings else {
        return Err(NoMove::NoCollectFolder);
    };
    let Some(archive) = archive else {
        return Err(NoMove::NoArchiveFolder);
    };
    Ok(Request {
        from_root: PathBuf::from(collect),
        from: Side::Collect,
        to_root: PathBuf::from(archive),
        to: Side::Archive,
        name,
    })
}

#[cfg(test)]
mod tests;
