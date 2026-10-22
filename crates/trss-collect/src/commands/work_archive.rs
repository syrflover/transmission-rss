//! `work_archive`, shown on the work detail as `보관 폴더로 옮기기` and `다시
//! 옮기기`: moves a work folder from the collect folder into the archive folder
//! (`docs/requirements/library.md`, 작품 상세 화면, and
//! `docs/requirements/collection.md`, 보관과 수집 시작의 폴더 이동).
//!
//! The command is about a work folder, not a rule: the web deletes the rules
//! that save into the folder with it, in the transaction that stores it
//! ([`accept_deleting`]), so no new episode arrives at the old place while it
//! moves. Its payload is a [`WorkArchive`] (the folder's name).
//!
//! `보관` of a rule (the rule detail's, and each rule of an archive
//! suggestions' batch) is the same command for the rule's work folder, stored
//! with the rule's deletion ([`archive_rule`]): the screens follow the folder's
//! fate by it, and the work detail shows a move that did not go through. A
//! rule whose save folder is no work folder below the collect folder is only
//! deleted. The worker runs it with [`run`], holding the turn of the
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
    done, failed, held_reason, lexical_work_folder, move_folder, settings, work_folder,
    work_folder::{Disk, Hold, RealDisk, Request, Side},
    ArchiveContext, Finished, NoMove, Retry, Start, WorkFolder, KEPT,
};
use crate::store::channels::{
    delete_rules_in, ChannelError, ChannelStore, Rule, RuleState, Version,
};
use trss_library::store::library::{LibraryError, LibraryStore};

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

/// The library's work of the work folder `work`, wherever it is now: below the
/// collect folder `collect` or the archive folder `archive` (a work keeps its
/// ID when it moves). `None` when the library has no such work.
pub async fn work_in_library(
    library: &LibraryStore,
    collect: &str,
    archive: Option<&str>,
    work: &str,
) -> Result<Option<String>, LibraryError> {
    for folder in std::iter::once(collect).chain(archive) {
        let folder = folder.trim_end_matches('/');
        if let Some(id) = library.work_in_folder(folder, work).await? {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// The move `보관` of `rule` stores with the rule's deletion: its work folder,
/// the first part of its save folder below the collect folder `collect`.
/// `Err` when it has none, and the rule is only deleted: no collect folder is
/// set, or the rule saves into the collect folder itself or outside it (a save
/// folder with `..` counts as outside, as [`work_folder`] never moves a folder
/// its text cannot place). No archive folder is no `Err`: the move is stored
/// and ends `kept` saying so, the record the screens read of the folder.
pub fn rule_move(collect: Option<&str>, rule: &Rule) -> Result<WorkArchive, NoMove> {
    let Some(collect) = collect else {
        return Err(NoMove::NoCollectFolder);
    };
    match work_folder(Path::new(collect), &rule.directory) {
        WorkFolder::Named(work) => Ok(WorkArchive { work }),
        WorkFolder::CollectItself => Err(NoMove::CollectItself),
        WorkFolder::Outside => Err(NoMove::Outside),
    }
}

/// What `보관` of `rule` would do as of now, in a sentence for the screens to
/// say before it is pressed: the rule is deleted, and its work folder moves or
/// stays and why. It asks what the move asks (where the folders are, which
/// other rules still collect into the work folder), but the move decides again
/// when it runs. `rules` are the rules of every channel; `settings` the collect
/// folder and the archive folder.
pub fn forecast(settings: Option<(String, Option<String>)>, rules: &[Rule], rule: &Rule) -> String {
    let collect = settings.as_ref().map(|(collect, _)| collect.as_str());
    let name = match rule_move(collect, rule) {
        Ok(WorkArchive { work }) => work,
        Err(no_move) => return format!("규칙을 지워요. {}", no_move.forecast()),
    };
    if settings
        .as_ref()
        .is_some_and(|(_, archive)| archive.is_none())
    {
        return format!("규칙을 지워요. {}", NoMove::NoArchiveFolder.forecast());
    }
    let collect = Path::new(collect.expect("a work folder is below a collect folder"));
    let named = WorkFolder::Named(name.clone());
    let holding: Vec<&Rule> = rules
        .iter()
        .filter(|other| other.id != rule.id && other.state == RuleState::Active)
        .filter(|other| lexical_work_folder(collect, &other.directory) == named)
        .collect();
    let Some(first) = holding.first() else {
        return format!("규칙을 지우고 작품 폴더(`{name}`)를 보관 폴더로 옮겨요.");
    };
    let who = match holding.len() {
        1 => format!("‘{}’ 규칙이", first.directory),
        n => format!("‘{}’ 규칙 외 {}개가", first.directory, n - 1),
    };
    format!(
        "규칙을 지워요. {who} 아직 이 작품 폴더에 받고 있어서 폴더는 옮기지 않아요. 남은 규칙까지 보관할 때 옮겨요."
    )
}

/// How `보관` of a rule went ([`archive_rule`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleArchived {
    /// The rule is deleted and the move of its work folder is stored (now, or
    /// before under the same ID): its outcome says what became of the folder.
    Moving(Box<Command>),
    /// The rule is deleted; it had no work folder to move, for this reason.
    Deleted(NoMove),
}

/// Why `보관` of a rule changed nothing ([`archive_rule`]).
#[derive(Debug)]
pub enum NotArchived {
    /// The rule is gone ([`ChannelError::NotFound`]) or at another version
    /// ([`ChannelError::is_conflict`]).
    Changed(ChannelError),
    /// A move of its work folder is open (from the work detail, or the archive
    /// of another rule saving there).
    Busy(Box<Command>),
    /// The ID was used for another request: another kind or folder, or the
    /// `보관` of another rule of the same work folder.
    Mismatch(Box<Command>),
}

/// A store `보관` of a rule could not read or write.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error(transparent)]
    Channels(ChannelError),
    #[error(transparent)]
    Commands(#[from] CommandError),
}

/// `보관` of `rule`, as it was read: deletes it, if it is still at that
/// version, and stores the move of its work folder ([`rule_move`]) under `id`,
/// in one transaction; a rule with no work folder is only deleted. The worker
/// moves the folder unless a rule that collects still saves into it.
/// `collect` is the collect folder.
pub async fn archive_rule(
    channels: &ChannelStore,
    commands: &CommandStore,
    collect: Option<&str>,
    rule: &Rule,
    id: String,
    now: Millis,
) -> Result<Result<RuleArchived, NotArchived>, ArchiveError> {
    let deleted = vec![(rule.id.clone(), rule.version)];
    let refused = |e: ChannelError| {
        if e.is_conflict() || matches!(e, ChannelError::NotFound { .. }) {
            Ok(Err(NotArchived::Changed(e)))
        } else {
            Err(ArchiveError::Channels(e))
        }
    };
    let payload = match rule_move(collect, rule) {
        Ok(payload) => payload,
        Err(no_move) => {
            return match channels.delete_rule(&rule.id, rule.version).await {
                Ok(()) => Ok(Ok(RuleArchived::Deleted(no_move))),
                Err(e) => refused(e),
            }
        }
    };
    match accept_deleting(commands, payload.command(id), deleted, now).await? {
        Ok(Accepted::Created(command)) => Ok(Ok(RuleArchived::Moving(Box::new(command)))),
        // The move was stored with the deletion of the rule it archived, so a
        // rule that is still there is another one, which only shares its work
        // folder: the ID was used for another request.
        Ok(Accepted::Existing(command)) => Ok(
            match channels
                .get_rule(&rule.id)
                .await
                .map_err(ArchiveError::Channels)?
            {
                Some(_) => Err(NotArchived::Mismatch(Box::new(command))),
                None => Ok(RuleArchived::Moving(Box::new(command))),
            },
        ),
        Ok(Accepted::Busy(open)) => Ok(Err(NotArchived::Busy(Box::new(open)))),
        Ok(Accepted::Mismatch(stored)) => Ok(Err(NotArchived::Mismatch(Box::new(stored)))),
        Err(e) => refused(e),
    }
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
