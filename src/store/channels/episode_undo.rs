//! `되돌리기` of an automatic episode offset (`docs/specs/collection.md`, 영상
//! 회차 변환) in the store: putting the previous value back together with the
//! plan of the renames, and recording each rename with the video revision rows
//! it moves. The worker's side is [`crate::worker::commands::episode_undo`].
//!
//! # Why the plan and the value go together
//!
//! [`ChannelStore::begin_episode_undo`] writes, in one transaction, the value
//! the rule had before the app's (with `자동` off and nothing kept of the
//! app's), the undo's own row and one row per video to rename. So a command
//! that stopped half-way finds its undo begun and carries on with the files
//! left `pending`; and the value is the user's from the moment the plan is
//! made, so whatever the rule receives afterwards is named with it and is not
//! in the plan.
//!
//! # Video revisions
//!
//! A replacement of a video revision (`store::revisions`) keeps rows keyed by
//! the folder and the episode's file name. A rename moves the episode's file,
//! so the rows of that name move to the new one in the same transaction as the
//! file's record ([`ChannelStore::finish_undo_file`]): a `다시 받기` of a
//! `버전 미상` row then names its video as the rule names it now, and the work
//! detail's version line follows the file. A replacement still under way
//! (receiving, checked, removing the old video, renaming the new one) acts on
//! the files of its name across cycles, so an undo that would rename one of
//! them, or rename a file onto its name, is refused before anything changes
//! ([`UndoBegun::Busy`]); it can be asked again once the replacement ended.
//!
//! A cycle can run between two starts of an undo, so each file is looked at
//! again right before its rename ([`ChannelStore::undo_file_hold`]): a
//! replacement that began meanwhile keeps the file, and so do rows the new
//! name has of its own (an ended replacement of a file that is gone), which
//! would otherwise be merged with the file's. Only rows of ended
//! replacements move.

use std::collections::HashMap;

use rusqlite::{params, params_from_iter, Connection, OptionalExtension, TransactionBehavior};

use super::{ChannelError, ChannelStore};
use crate::store::{history::Millis, revisions::RevisionState};

type Result<T> = std::result::Result<T, ChannelError>;

/// A video to rename, as planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewUndoFile {
    /// The history item the video was received for.
    pub item_id: i64,
    /// The folder the video is in.
    pub folder: String,
    /// Its name under the automatic offset, which it has now.
    pub from_name: String,
    /// The name it takes with the offset put back.
    pub to_name: String,
    /// The torrent that held it when planned: renamed through Transmission.
    pub torrent_hash: Option<String>,
    /// The file's identity when planned
    /// ([`FileIdentity::to_text`](crate::revision::FileIdentity::to_text)).
    pub identity: Option<String>,
    /// Why the file is left as it is from the start, when planning could not
    /// tell which file the item's is. Only for planning: an undo read back
    /// has it as [`UndoFile::reason`].
    pub kept: Option<String>,
}

/// Where the rename of one video is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoFileState {
    Pending,
    Renamed,
    /// Left as it is; the reason says why.
    Kept,
}

impl UndoFileState {
    pub fn code(self) -> &'static str {
        match self {
            UndoFileState::Pending => "pending",
            UndoFileState::Renamed => "renamed",
            UndoFileState::Kept => "kept",
        }
    }

    fn parse(code: &str) -> Option<UndoFileState> {
        [
            UndoFileState::Pending,
            UndoFileState::Renamed,
            UndoFileState::Kept,
        ]
        .into_iter()
        .find(|s| s.code() == code)
    }
}

/// A video of an undo and how far its rename has come.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoFile {
    pub file: NewUndoFile,
    pub state: UndoFileState,
    pub reason: Option<String>,
}

/// An undo that began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeUndo {
    pub command_id: String,
    pub rule_id: String,
    /// The automatic value.
    pub from: i64,
    /// The value put back.
    pub to: i64,
    /// In the order they were planned.
    pub files: Vec<UndoFile>,
}

/// What [`ChannelStore::begin_episode_undo`] came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoBegun {
    /// Begun now, or by an earlier start of the same command.
    Begun(EpisodeUndo),
    /// The rule is gone, or its offset is no longer the automatic `from` with
    /// a known previous value. Nothing was changed.
    Changed,
    /// A video revision replacement is under way for these episode names in
    /// the plan. Nothing was changed.
    Busy(Vec<String>),
}

/// Why a file waits, still `pending`: a replacement acts on it or on its new
/// name.
pub const REVISION_UNDER_WAY: &str =
    "수정본으로 대체하는 중인 영상이에요. 대체가 끝난 뒤 이어서 되돌릴 수 있어요.";
/// Why a file keeps its name: the new name has video revision rows of its own.
pub const REVISION_ROWS_THERE: &str =
    "새 이름에 다른 영상의 수정본 기록이 있어서 이름을 바꾸지 않았어요.";

/// The states of a replacement still acting on its episode's files.
fn under_way() -> Vec<&'static str> {
    RevisionState::ALL
        .into_iter()
        .filter(|s| s.holds_torrent())
        .map(RevisionState::code)
        .collect()
}

fn read_undo(conn: &Connection, command_id: &str) -> Result<Option<EpisodeUndo>> {
    let head = conn
        .query_row(
            "SELECT rule_id, from_offset, to_offset FROM episode_undos WHERE command_id = ?1",
            params![command_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((rule_id, from, to)) = head else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT item_id, folder, from_name, to_name, torrent_hash, identity, state, reason
           FROM episode_undo_files WHERE command_id = ?1 ORDER BY rowid",
    )?;
    let files = stmt
        .query_map(params![command_id], |row| {
            let state: String = row.get(6)?;
            Ok(UndoFile {
                file: NewUndoFile {
                    item_id: row.get(0)?,
                    folder: row.get(1)?,
                    from_name: row.get(2)?,
                    to_name: row.get(3)?,
                    torrent_hash: row.get(4)?,
                    identity: row.get(5)?,
                    kept: None,
                },
                state: UndoFileState::parse(&state).unwrap_or(UndoFileState::Kept),
                reason: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(EpisodeUndo {
        command_id: command_id.to_owned(),
        rule_id,
        from,
        to,
        files,
    }))
}

#[allow(clippy::too_many_arguments)]
fn begin(
    conn: &mut Connection,
    command_id: &str,
    rule_id: &str,
    from: i64,
    to: i64,
    files: &[NewUndoFile],
    at: Millis,
) -> Result<UndoBegun> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(undo) = read_undo(&tx, command_id)? {
        tx.commit()?;
        return Ok(UndoBegun::Begun(undo));
    }
    let previous: Option<Option<i64>> = tx
        .query_row(
            "SELECT episode_previous FROM rules
              WHERE id = ?1 AND episode_auto = 1 AND episode = ?2",
            params![rule_id, from],
            |row| row.get(0),
        )
        .optional()?;
    // The plan was made for `to`; another previous value is another undo.
    if previous != Some(Some(to)) {
        return Ok(UndoBegun::Changed);
    }

    let states = under_way();
    let marks = vec!["?"; states.len()].join(", ");
    let mut busy: Vec<String> = Vec::new();
    {
        let mut stmt = tx.prepare(&format!(
            "SELECT DISTINCT episode_name FROM video_revisions
              WHERE folder = ? AND episode_name IN (?, ?) AND state IN ({marks})"
        ))?;
        for file in files.iter().filter(|f| f.kept.is_none()) {
            let mut args: Vec<&str> = vec![&file.folder, &file.from_name, &file.to_name];
            args.extend(states.iter().copied());
            let names = stmt
                .query_map(params_from_iter(args), |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for name in names {
                if !busy.contains(&name) {
                    busy.push(name);
                }
            }
        }
    }
    if !busy.is_empty() {
        return Ok(UndoBegun::Busy(busy));
    }

    tx.execute(
        "UPDATE rules
            SET episode = ?2, episode_auto = 0, episode_basis = NULL, episode_previous = NULL,
                version = version + 1
          WHERE id = ?1",
        params![rule_id, to],
    )?;
    tx.execute(
        "INSERT INTO episode_undos (command_id, rule_id, from_offset, to_offset, started_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![command_id, rule_id, from, to, at],
    )?;
    for file in files {
        tx.execute(
            "INSERT OR IGNORE INTO episode_undo_files
                 (command_id, item_id, folder, from_name, to_name, torrent_hash, identity,
                  state, reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                command_id,
                file.item_id,
                file.folder,
                file.from_name,
                file.to_name,
                file.torrent_hash,
                file.identity,
                if file.kept.is_some() {
                    "kept"
                } else {
                    "pending"
                },
                file.kept,
            ],
        )?;
    }
    let undo = read_undo(&tx, command_id)?.expect("the undo was just written");
    tx.commit()?;
    Ok(UndoBegun::Begun(undo))
}

fn finish_file(
    conn: &mut Connection,
    command_id: &str,
    item_id: i64,
    kept: Option<&str>,
    at: Millis,
) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let file: Option<(String, String, String)> = tx
        .query_row(
            "SELECT folder, from_name, to_name FROM episode_undo_files
              WHERE command_id = ?1 AND item_id = ?2 AND state = 'pending'",
            params![command_id, item_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((folder, from_name, to_name)) = file else {
        return Ok(());
    };
    let state = if kept.is_some() {
        UndoFileState::Kept
    } else {
        UndoFileState::Renamed
    };
    tx.execute(
        "UPDATE episode_undo_files SET state = ?3, reason = ?4
          WHERE command_id = ?1 AND item_id = ?2",
        params![command_id, item_id, state.code(), kept],
    )?;
    if kept.is_none() {
        let states = under_way();
        let marks = (5..5 + states.len())
            .map(|n| format!("?{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut args: Vec<&dyn rusqlite::ToSql> = vec![&folder, &from_name, &to_name, &at];
        args.extend(states.iter().map(|s| s as &dyn rusqlite::ToSql));
        tx.execute(
            &format!(
                "UPDATE video_revisions SET episode_name = ?3, updated_at = ?4
                  WHERE folder = ?1 AND episode_name = ?2 AND state NOT IN ({marks})"
            ),
            args.as_slice(),
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn wait_file(conn: &Connection, command_id: &str, item_id: i64, reason: &str) -> Result<()> {
    conn.execute(
        "UPDATE episode_undo_files SET reason = ?3
          WHERE command_id = ?1 AND item_id = ?2 AND state = 'pending'",
        params![command_id, item_id, reason],
    )?;
    Ok(())
}

fn note_identity(conn: &Connection, command_id: &str, item_id: i64, identity: &str) -> Result<()> {
    conn.execute(
        "UPDATE episode_undo_files SET identity = ?3
          WHERE command_id = ?1 AND item_id = ?2 AND state = 'pending' AND identity IS NULL",
        params![command_id, item_id, identity],
    )?;
    Ok(())
}

fn file_hold(
    conn: &Connection,
    folder: &str,
    from_name: &str,
    to_name: &str,
) -> Result<Option<&'static str>> {
    let mut stmt = conn.prepare(
        "SELECT episode_name, state FROM video_revisions
          WHERE folder = ?1 AND episode_name IN (?2, ?3)",
    )?;
    let rows = stmt
        .query_map(params![folder, from_name, to_name], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let states = under_way();
    if rows
        .iter()
        .any(|(_, state)| states.contains(&state.as_str()))
    {
        return Ok(Some(REVISION_UNDER_WAY));
    }
    if rows.iter().any(|(name, _)| name == to_name) {
        return Ok(Some(REVISION_ROWS_THERE));
    }
    Ok(None)
}

/// The latest undo of `rule_id` that still has files to rename.
fn unfinished(conn: &Connection, rule_id: &str) -> Result<Option<EpisodeUndo>> {
    let command_id: Option<String> = conn
        .query_row(
            "SELECT u.command_id FROM episode_undos u
              WHERE u.rule_id = ?1
                AND EXISTS (SELECT 1 FROM episode_undo_files f
                             WHERE f.command_id = u.command_id AND f.state = 'pending')
              ORDER BY u.started_at DESC, u.command_id DESC LIMIT 1",
            params![rule_id],
            |row| row.get(0),
        )
        .optional()?;
    match command_id {
        Some(id) => read_undo(conn, &id),
        None => Ok(None),
    }
}

fn adopt(
    conn: &mut Connection,
    from_command: &str,
    to_command: &str,
) -> Result<Option<EpisodeUndo>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(undo) = read_undo(&tx, to_command)? {
        tx.commit()?;
        return Ok(Some(undo));
    }
    let copied = tx.execute(
        "INSERT INTO episode_undos (command_id, rule_id, from_offset, to_offset, started_at)
         SELECT ?2, rule_id, from_offset, to_offset, started_at
           FROM episode_undos WHERE command_id = ?1",
        params![from_command, to_command],
    )?;
    if copied == 0 {
        return Ok(None);
    }
    tx.execute(
        "UPDATE episode_undo_files SET command_id = ?2 WHERE command_id = ?1",
        params![from_command, to_command],
    )?;
    tx.execute(
        "DELETE FROM episode_undos WHERE command_id = ?1",
        params![from_command],
    )?;
    let undo = read_undo(&tx, to_command)?;
    tx.commit()?;
    Ok(undo)
}

impl ChannelStore {
    /// The latest undo of rule `rule_id` that has files still to rename: one
    /// whose command ended before it was done (given up, or a panic). A new
    /// request for it carries it on ([`ChannelStore::adopt_episode_undo`]).
    pub async fn unfinished_episode_undo(&self, rule_id: &str) -> Result<Option<EpisodeUndo>> {
        let rule_id = rule_id.to_owned();
        self.db.run(move |c| unfinished(c, &rule_id)).await
    }

    /// Hands the undo of command `from_command`, with its files as they
    /// stand, to command `to_command`, which carries it on. The undo as it is
    /// then; `None` when `from_command` has none. Done once: an undo
    /// `to_command` has already is returned as it is.
    pub async fn adopt_episode_undo(
        &self,
        from_command: &str,
        to_command: &str,
    ) -> Result<Option<EpisodeUndo>> {
        let (from_command, to_command) = (from_command.to_owned(), to_command.to_owned());
        self.db
            .run(move |c| adopt(c, &from_command, &to_command))
            .await
    }

    /// Begins the undo `command_id` of rule `rule_id`'s automatic offset
    /// `from`: puts the previous value back as the user's own and writes the
    /// plan `files` (see the module docs). An undo the command began before
    /// is returned as it stands, whatever `files` says now.
    /// `to` is the previous value the plan was made for; the undo begins only
    /// while it is still the rule's.
    pub async fn begin_episode_undo(
        &self,
        command_id: &str,
        rule_id: &str,
        from: i64,
        to: i64,
        files: Vec<NewUndoFile>,
        at: Millis,
    ) -> Result<UndoBegun> {
        let (command_id, rule_id) = (command_id.to_owned(), rule_id.to_owned());
        self.db
            .run(move |c| begin(c, &command_id, &rule_id, from, to, &files, at))
            .await
    }

    /// Why the video at `from_name` in `folder` keeps its name, as the video
    /// revision rows have it now, or `None` when it may take `to_name` (see
    /// the module docs).
    pub async fn undo_file_hold(
        &self,
        folder: &str,
        from_name: &str,
        to_name: &str,
    ) -> Result<Option<&'static str>> {
        let (folder, from_name, to_name) =
            (folder.to_owned(), from_name.to_owned(), to_name.to_owned());
        self.db
            .run(move |c| file_hold(c, &folder, &from_name, &to_name))
            .await
    }

    /// The undo of command `command_id`, once it began.
    pub async fn episode_undo(&self, command_id: &str) -> Result<Option<EpisodeUndo>> {
        let command_id = command_id.to_owned();
        self.db.run(move |c| read_undo(c, &command_id)).await
    }

    /// The undos of the given commands that began, by command ID.
    pub async fn episode_undos(
        &self,
        command_ids: Vec<String>,
    ) -> Result<HashMap<String, EpisodeUndo>> {
        self.db
            .run(move |c| {
                let mut found = HashMap::new();
                for id in command_ids {
                    if let Some(undo) = read_undo(c, &id)? {
                        found.insert(id, undo);
                    }
                }
                Ok(found)
            })
            .await
    }

    /// Leaves the video of `item_id` in the undo `pending`, with why it waits
    /// (still downloading, a replacement under way): the next request for
    /// the undo looks at it again.
    pub async fn wait_undo_file(&self, command_id: &str, item_id: i64, reason: &str) -> Result<()> {
        let (command_id, reason) = (command_id.to_owned(), reason.to_owned());
        self.db
            .run(move |c| wait_file(c, &command_id, item_id, &reason))
            .await
    }

    /// Keeps the identity of the video of `item_id`, found once its torrent
    /// completed (it was planned while still downloading, with none), before
    /// it is renamed: a start cut short then knows the file it renamed.
    pub async fn note_undo_file_identity(
        &self,
        command_id: &str,
        item_id: i64,
        identity: String,
    ) -> Result<()> {
        let command_id = command_id.to_owned();
        self.db
            .run(move |c| note_identity(c, &command_id, item_id, &identity))
            .await
    }

    /// Records how the rename of the video of `item_id` in the undo ended:
    /// renamed (`kept` is `None`), and then the video revision rows of its old
    /// name that ended take the new one in the same transaction; or left as it is, with
    /// the reason. A file that is not `pending` any more is left as recorded.
    pub async fn finish_undo_file(
        &self,
        command_id: &str,
        item_id: i64,
        kept: Option<String>,
        at: Millis,
    ) -> Result<()> {
        let command_id = command_id.to_owned();
        self.db
            .run(move |c| finish_file(c, &command_id, item_id, kept.as_deref(), at))
            .await
    }
}
