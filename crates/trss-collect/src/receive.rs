//! How one item's torrent goes to Transmission and into history, the same for
//! the rule cycle ([`crate::cycle`]) and the `다시 받기` command
//! ([`crate::commands::receive_once`], which `receive_past` goes through):
//!
//! 1. [`add`] adds the item's link with its labels, and says why an add failed
//!    in the words history keeps ([`failure_reason`]);
//! 2. [`record`] writes what came of it on the history item, with the
//!    replacement row of a video revision in the same transaction;
//! 3. [`rename`] gives the torrent's single file its `trname` name, trying again
//!    while Transmission does not have the file name yet, and notes on the
//!    item why a name stayed when the caller asks.
//!
//! The caller holds the item's turn at its work folder for all three (and a
//! command its torrent gate after the turn); nothing here takes a turn or a
//! gate. What the two callers do differently is theirs, given as values:
//! whether an add that got no answer counts as unconfirmed ([`AddFailure`]),
//! how a history write names its item ([`HistoryWrite`]), which name a file is
//! named from ([`Original`]), what becomes of a new torrent `trname` has no
//! name for ([`Underivable`]) and whether a kept name is noted
//! ([`RenameJob::note`]).

use std::path::Path;

use tokio_util::sync::CancellationToken;
use transmission_rpc::{types::Id, TransClient};
use trname::trname_raw;

use crate::{
    commands::receive_once::ReceiveContext,
    context::MAX_REASON_CHARS,
    release_name::name_for_trname,
    store::{
        history::{HistoryError, HistoryResult, Recorded},
        revisions::{HistoryWrite, Revision, RevisionError, RowWrite},
    },
};
use trss_core::Millis;
use trss_transmission::{add_item, get_torrent, AddError, AddLabels, AddedTorrent, Redactor};

#[cfg(test)]
mod tests;

/// Why the cycle recorded an item as failed whose task ended in a panic
/// before its result was written.
pub const ADD_PANICKED: &str = "토렌트를 추가하다 내부 오류가 났어요.";

/// Why an add failed, as history keeps it: one sentence per kind of failure,
/// with Transmission's own words after it, secret values replaced and cut to
/// [`MAX_REASON_CHARS`].
pub fn failure_reason(err: &AddError, redactor: &Redactor) -> String {
    let text = match err {
        AddError::Unreachable(err) => format!("Transmission에 연결하지 못했어요: {err}"),
        AddError::Rpc(err) => format!("Transmission이 응답하지 않았어요: {err}"),
        AddError::Rejected(result) => format!("Transmission이 토렌트를 받지 않았어요: {result}"),
    };
    redactor
        .apply(&text)
        .chars()
        .take(MAX_REASON_CHARS)
        .collect()
}

// --- add ---------------------------------------------------------------------

/// An add that did not leave the torrent in Transmission as far as the caller
/// knows.
#[derive(Debug)]
pub struct AddFailure {
    pub error: AddError,
    /// [`failure_reason`] of `error`.
    pub reason: String,
}

impl AddFailure {
    /// The request was sent and got no answer: Transmission may hold the
    /// torrent all the same.
    pub fn unanswered(&self) -> bool {
        matches!(self.error, AddError::Rpc(_))
    }

    /// Transmission answered and refused the torrent.
    pub fn refused(&self) -> bool {
        matches!(self.error, AddError::Rejected(_))
    }
}

/// Adds `link` to Transmission into `save_path`, with the bot's label and
/// `labels` ([`trss_transmission::add_item`]).
pub async fn add(
    transmission: &mut TransClient,
    link: &str,
    save_path: &Path,
    labels: AddLabels<'_>,
    redactor: &Redactor,
) -> Result<AddedTorrent, AddFailure> {
    add_item(transmission, link, save_path, labels, redactor)
        .await
        .map_err(|error| AddFailure {
            reason: failure_reason(&error, redactor),
            error,
        })
}

// --- record ------------------------------------------------------------------

/// What [`record`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    /// The history item written, when known: the item of a
    /// [`HistoryWrite::Outcome`], or of a sighting.
    pub item_id: Option<i64>,
    /// For [`HistoryWrite::Observe`]: what recording the sighting did.
    pub recorded: Option<Recorded>,
    /// For [`HistoryWrite::Outcome`]: the item's result afterwards.
    pub stored: Option<HistoryResult>,
    /// The item's replacement row afterwards, when one was written.
    pub row: Option<Revision>,
}

/// Why [`record`] wrote nothing.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Revisions(#[from] RevisionError),
}

/// Writes `history` at `at` and, with `row`, the item's replacement row in the
/// same transaction ([`crate::store::revisions::RevisionStore::write_with_history`]):
/// either both are written or neither.
pub async fn record(
    ctx: &ReceiveContext,
    at: Millis,
    history: HistoryWrite,
    row: Option<RowWrite>,
) -> Result<Stored, RecordError> {
    let item = match &history {
        HistoryWrite::Outcome { item_id, .. } => Some(*item_id),
        HistoryWrite::Observe(_) => None,
    };
    if let Some(row) = row {
        let written = ctx.revisions.write_with_history(at, history, row).await?;
        return Ok(Stored {
            item_id: item.or(written.row.as_ref().map(|row| row.item_id)),
            recorded: written.recorded,
            stored: written.stored,
            row: written.row,
        });
    }
    match history {
        HistoryWrite::Observe(observation) => {
            let (item_id, recorded) = ctx.history.record_one(at, observation).await?;
            Ok(Stored {
                item_id: Some(item_id),
                recorded: Some(recorded),
                stored: None,
                row: None,
            })
        }
        HistoryWrite::Outcome {
            item_id,
            result,
            rule_id,
            reason,
            torrent_hash,
        } => {
            let stored = ctx
                .history
                .record_outcome(item_id, at, result, rule_id, reason, torrent_hash)
                .await?;
            Ok(Stored {
                item_id: Some(item_id),
                recorded: None,
                stored,
                row: None,
            })
        }
    }
}

// --- rename ------------------------------------------------------------------

/// Which torrent [`rename`] is renaming, which decides how far it may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameMode {
    /// A torrent the caller has just added, or one its own earlier add put in.
    /// The single file is renamed; what becomes of it when `trname` has no
    /// name for it is [`RenameJob::underivable`]. A name the caller keeps
    /// ([`name_for_trname`]) is left as it is.
    Added,
    /// A torrent Transmission already had. Its file is renamed only while it
    /// sits in the save path and its name is not in the `trname` form yet (a
    /// rename that was cut short earlier). A name in that form is left alone,
    /// because applying the episode offset again would change the episode, and
    /// a torrent in another folder was named after another title. Nothing is
    /// ever removed.
    Existing,
}

/// Which name a file's `trname` name is derived from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Original {
    /// The name the file has when it is looked at, on every attempt (the rule
    /// cycle).
    Current,
    /// The name the file had before a command first renamed it, which the
    /// command records ([`trss_core::commands::Command::original_name`]):
    /// `name` as the command was claimed, else recorded here before the first
    /// rename. Run again over a file it named already, the rename finds the
    /// same name and leaves it. `added_before` says the torrent was put in by
    /// an earlier start of the command, so it may have been renamed since, by
    /// that start or by a cycle: with no name recorded, a name in the `trname`
    /// form is left as it is.
    Recorded {
        command_id: String,
        name: Option<String>,
        added_before: bool,
    },
}

/// What becomes of a torrent added in [`RenameMode::Added`] whose name the
/// caller reads as an episode and `trname` has no name for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Underivable {
    /// Removed with its data (the rule cycle's legacy treatment; 0128 ends it).
    Remove,
    /// Left under its received name ([`NAME_NOT_DERIVED`]).
    Keep,
}

/// One torrent to rename.
#[derive(Debug, Clone)]
pub struct RenameJob<'a> {
    pub hash: &'a str,
    /// The rule's folder (`.../<title>/Season NN`), which the torrent was
    /// added into.
    pub save_path: &'a Path,
    /// The rule's episode conversion.
    pub episode: isize,
    pub mode: RenameMode,
    pub original: Original,
    pub underivable: Underivable,
    /// The history item to note why the name stayed on
    /// ([`RenameResult::Kept`]), or none to note nothing.
    pub note: Option<i64>,
    /// Keeps trying until the file is renamed or the attempts run out, as the
    /// cycle's renaming did, instead of stopping once nothing more can come of
    /// it: a torrent that is gone or was removed, several files, a name that
    /// is right already; and a torrent without its file count or name panics.
    pub until_renamed: bool,
    pub redactor: &'a Redactor,
}

/// What [`rename`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameResult {
    /// The file has the `trname` name now.
    Renamed,
    /// Nothing to do: the name was already right, the torrent is gone, the
    /// torrent is one the rename leaves alone, or shutdown was asked for.
    Unchanged,
    /// The file keeps its original name; the note says why, for the history item.
    Kept(&'static str),
    /// The torrent was removed with its data ([`Underivable::Remove`]).
    Removed,
}

/// The file's name gave `trname` no title and episode to work with.
pub const NAME_NOT_DERIVED: &str =
    "파일 이름에서 작품과 회차를 알아내지 못해서 원래 이름 그대로 뒀어요.";
/// The torrent has more than one file; `trname` names a single file.
pub const SEVERAL_FILES: &str = "파일이 여러 개인 토렌트라 이름을 바꾸지 않았어요.";
/// The name `trname` gives is taken by another file in the folder.
pub const NAME_TAKEN: &str = "같은 회차 이름의 파일이 이미 있어서 원래 이름 그대로 뒀어요.";
/// Renaming was tried and did not go through.
pub const NAME_NOT_CHANGED: &str = "이름을 바꾸지 못해서 원래 이름 그대로 뒀어요.";

/// Gives the torrent's single file its `trname` name for the folder it was
/// saved in, with the rule's episode conversion, derived from the name
/// [`Original`] gives through [`name_for_trname`] (a name read as no episode
/// or as a batch keeps its name). A torrent with several files is left as it
/// is, and so is one whose new name is taken by a file in its folder (looked
/// up on this host's disk, which sees the folders at the paths Transmission
/// reports): Transmission would answer success and point the torrent at the
/// other file, leaving its own under the old name (libtransmission's
/// `renamePath` renames on disk only when the target is not there).
///
/// Attempts follow the context's [`RenamePolicy`](trss_transmission::RenamePolicy),
/// each after its delay, as a magnet link's file name is only known once
/// Transmission has its metadata. `cancel` stops it before the next attempt.
/// When the name stays ([`RenameResult::Kept`]) and the job names an item to
/// note, the note is written on it ([`crate::store::history::HistoryStore::note_received`]).
pub async fn rename(
    ctx: &ReceiveContext,
    job: &RenameJob<'_>,
    cancel: &CancellationToken,
) -> RenameResult {
    let result = rename_file(ctx, job, cancel).await;
    if let (RenameResult::Kept(note), Some(item_id)) = (result, job.note) {
        // Only reported: the item's result is written already, and a rerun
        // would not rename or note it.
        if let Err(err) = ctx.history.note_received(item_id, note).await {
            eprintln!("Cannot note the kept name on item {item_id}: {err}");
        }
    }
    result
}

async fn rename_file(
    ctx: &ReceiveContext,
    job: &RenameJob<'_>,
    cancel: &CancellationToken,
) -> RenameResult {
    let mut transmission = ctx.transmission.client();
    let mut recorded = match &job.original {
        Original::Recorded { name, .. } => name.clone(),
        Original::Current => None,
    };
    for _ in 0..ctx.rename.attempts {
        tokio::select! {
            _ = tokio::time::sleep(ctx.rename.delay) => {}
            _ = cancel.cancelled() => return RenameResult::Unchanged,
        }

        let torrent = match get_torrent(&mut transmission, job.hash).await {
            Ok(Some(torrent)) => torrent,
            Ok(None) if job.until_renamed && job.mode == RenameMode::Added => continue,
            Ok(None) => return RenameResult::Unchanged,
            Err(err) => {
                println!("{}", job.redactor.apply(&err.to_string()));
                continue;
            }
        };

        if job.mode == RenameMode::Existing
            && torrent.download_dir.as_deref().map(Path::new) != Some(job.save_path)
        {
            return RenameResult::Unchanged;
        }
        match torrent.file_count {
            Some(1) => {}
            // Transmission counts no files until a magnet link's metadata is in.
            Some(0) => continue,
            None if job.until_renamed => panic!("Transmission gave no file count"),
            None => continue,
            Some(_) if job.until_renamed && job.mode == RenameMode::Added => continue,
            Some(_) => return RenameResult::Kept(SEVERAL_FILES),
        }
        let current = match torrent.name {
            Some(name) => name,
            None if job.until_renamed => panic!("Transmission gave no name"),
            None => continue,
        };

        if job.mode == RenameMode::Existing && looks_renamed(&current, job.save_path) {
            return RenameResult::Unchanged;
        }

        // The name is derived from the name the file had before any rename
        // when a command recorded it, never from the name the file has now: a
        // name this command's earlier start or a cycle gave is then the same
        // name again and stays, so no episode is converted twice, while a
        // release that comes in the `trname` form of another season or
        // episode is still converted.
        let original = match &job.original {
            Original::Current => current.clone(),
            Original::Recorded {
                command_id,
                added_before,
                ..
            } => match &recorded {
                Some(name) => name.clone(),
                None => {
                    // An earlier start put the torrent in and recorded no name
                    // (the worker died between the two): a name in the
                    // `trname` form may be one it or a cycle gave.
                    if *added_before && has_trname_form(&current, job.save_path, job.episode) {
                        return RenameResult::Unchanged;
                    }
                    // Recorded before any rename, so a later start finds it.
                    match ctx.commands.note_original_name(command_id, &current).await {
                        Ok(Some(name)) => {
                            recorded = Some(name.clone());
                            name
                        }
                        Ok(None) => return RenameResult::Unchanged,
                        Err(err) => {
                            eprintln!(
                                "Cannot record the name of the torrent of command {command_id}: {err}"
                            );
                            continue;
                        }
                    }
                }
            },
        };

        // The caller keeps a name read as no episode: not renamed, and not
        // removed either.
        let Some(read_as) = name_for_trname(&original) else {
            return match job.mode {
                RenameMode::Added => RenameResult::Kept(NAME_NOT_DERIVED),
                RenameMode::Existing => RenameResult::Unchanged,
            };
        };
        let new_name = match (job.mode, trname_raw(job.save_path, &read_as, job.episode)) {
            (RenameMode::Existing, Some((_, file, _))) if file.already_formatted => {
                return RenameResult::Unchanged;
            }
            (_, Some((_, _, new_name))) => new_name,
            (RenameMode::Existing, None) => return RenameResult::Unchanged,
            (RenameMode::Added, None) => match job.underivable {
                Underivable::Keep => return RenameResult::Kept(NAME_NOT_DERIVED),
                Underivable::Remove => {
                    match transmission
                        .torrent_remove(vec![Id::Hash(job.hash.to_owned())], true)
                        .await
                    {
                        Ok(_) if job.until_renamed => {}
                        Ok(_) => return RenameResult::Removed,
                        Err(err) => println!("{}", job.redactor.apply(&err.to_string())),
                    }
                    continue;
                }
            },
        };
        if new_name == current && !job.until_renamed {
            return RenameResult::Unchanged;
        }
        // Never onto a name that is taken (see above). That is the old video
        // when a higher revision of a received episode comes in; its
        // replacement is the revisions' ([`crate::revisions`]).
        let folder = torrent
            .download_dir
            .as_deref()
            .map_or(job.save_path, Path::new);
        if std::fs::symlink_metadata(folder.join(&new_name)).is_ok() {
            println!(
                "Not renaming {current}: {new_name} is taken in {}",
                folder.display()
            );
            return RenameResult::Kept(NAME_TAKEN);
        }
        match transmission
            .torrent_rename_path(vec![Id::Hash(job.hash.to_owned())], current, new_name)
            .await
        {
            Ok(response) if response.result == "success" => return RenameResult::Renamed,
            Ok(_) => {}
            Err(err) => println!("{}", job.redactor.apply(&err.to_string())),
        }
    }
    RenameResult::Kept(NAME_NOT_CHANGED)
}

/// Whether the file `name` in `download_dir` has the `trname` form already,
/// read as [`RenameMode::Existing`] reads it: it looks renamed, or `trname`
/// finds it formatted. A rename of such a name would read its episode as a
/// release's and convert it a second time (`E13` with a conversion of `+12`
/// becoming `E25`), so a rename that may meet a name it or another rename
/// gave already asks this first.
pub fn has_trname_form(name: &str, download_dir: &Path, starts_episode_at: isize) -> bool {
    looks_renamed(name, download_dir)
        || name_for_trname(name)
            .and_then(|read_as| trname_raw(download_dir, &read_as, starts_episode_at))
            .is_some_and(|(_, file, _)| file.already_formatted)
}

/// Whether `name` is the name `trname` gives in `download_dir`
/// (`.../<title>/Season NN`): the folder's title and an episode
/// ([`trss_core::trname_names::is_trname_name`]).
fn looks_renamed(name: &str, download_dir: &Path) -> bool {
    download_dir
        .components()
        .rev()
        .nth(1)
        .and_then(|c| c.as_os_str().to_str())
        .is_some_and(|title| trss_core::trname_names::is_trname_name(name, title))
}
