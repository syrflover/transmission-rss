//! The replacement of video revisions (`docs/specs/collection.md`, 영상
//! 수정본의 대체): what the worker decided about each higher revision a rule
//! selected for an episode whose folder already holds a video, and how far
//! the replacement has come. The worker ([`crate::worker::revisions`]) is the
//! only writer of media folders and of the steps here; the web reads them for
//! the work detail's episode rows and for the `받기 실패` to-do source.
//!
//! # States
//!
//! A row starts as one of:
//!
//! - [`RevisionState::Unknown`] (버전 미상): not received; the history item is
//!   `version_unknown`, and `다시 받기` turns the row into `receiving`.
//! - [`RevisionState::Skipped`]: the folder holds this revision (or a higher
//!   one) already, or the torrent is another row's (the same release through
//!   another channel); nothing is replaced.
//! - [`RevisionState::Receiving`]: the new torrent is in Transmission and
//!   keeps the name it was received under.
//!
//! and moves on, each step written before the next one acts:
//! `receiving` → `verified` (the download is complete and its CRC32 matched
//! the name, or the person confirmed) → `removing` (the old video is about to
//! be removed) → `removed` (it is gone; the new video is to take the episode
//! name, which happens only while that name is free) → `done`.
//! [`RevisionState::Failed`] ends a replacement before the old video was
//! removed: both files stay and the reason says why. A failure the person has
//! since resolved (one of the two files is gone) becomes
//! [`RevisionState::Cleared`]. A `removed` row with a reason is a rename that
//! did not go through yet; the worker tries again while the name is free. A
//! `removing` row with a reason ([`Step::RemovalWaits`]) removed the old
//! torrent but the episode's file is still there, and waits for that file to
//! go. A `verified` or `removing` row with a reason ([`Step::NewMissing`])
//! found, before removing the old video, that the new video was not the one
//! whose CRC32 was checked, and removed nothing. All of them are listed with
//! the failures. A row whose new video is gone (or not the checked one) on
//! two looks in a row ([`Revision::new_missing_at`]) ends as
//! [`RevisionState::Abandoned`]: nothing is removed or renamed (one that
//! ended after the old video was removed is listed with the failures until
//! the episode name holds a video again; one that ended beside the old
//! video's file left after its torrent was removed, [`OLD_FILE_WATCHED`],
//! is listed once that file goes too), it holds up
//! no other replacement of the episode nor any lower revision of its release
//! (the rows skipped for it start over, as when it fails), and the old
//! release stays superseded (its torrent was removed for this replacement,
//! or is still there with its video).
//!
//! A rule folder that is away decides nothing; one away for
//! [`FOLDER_GONE_AFTER`] ([`Revision::folder_away_since`]) is not waited for
//! any more: failures leave the list, and replacements under way say so
//! ([`Step::FolderGone`]) until it is back.
//!
//! A step is written only from the state it was decided from
//! ([`RevisionStore::advance`]), and a row the worker decides together with
//! its history item's result is written in the same transaction
//! ([`RevisionStore::write_with_history`]).
//!
//! A failure before the new video was received (no `received_name`, see
//! [`Revision::not_received`]) is not final: the worker looks at its torrent
//! again every cycle, and a cycle that receives its item again (the torrent
//! had gone), or `다시 받기` of it, starts it over ([`RevisionStore::reopen`]).
//! So does `다시 받기` of a replacement that ended with no video under the
//! episode name; its old release stays superseded meanwhile.
//!
//! # One episode, one replacement at a time
//!
//! Rows are unique per history item, so two revisions of one release (`14v2`
//! and `14v3`) can each have a row for the same episode file. Only one of them
//! removes the old video at a time ([`RevisionStore::claim`]), a lower one
//! never replaces a higher one, and a row whose torrent is another row's is
//! skipped when it is written.
//!
//! A lower revision skipped while a higher one was on its way
//! ([`Step::Overtaken`]) keeps that row ([`Revision::overtaken_by`]). If the
//! higher one fails or is abandoned, the lower one goes back to `receiving`
//! in the same transaction and replaces the video after all, from its first
//! step; a lower revision skipped for any other reason stays skipped.

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row, TransactionBehavior};

use super::{
    db::{Db, DbError},
    history::{repo as history_repo, HistoryError, HistoryResult, Millis, Observation, Recorded},
};

#[derive(Debug, thiserror::Error)]
pub enum RevisionError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    History(#[from] HistoryError),
}

impl From<rusqlite::Error> for RevisionError {
    fn from(e: rusqlite::Error) -> Self {
        RevisionError::Db(DbError::Sqlite(e))
    }
}

type Result<T> = std::result::Result<T, RevisionError>;

/// Why a row was skipped because another row of the episode has the same
/// torrent: the same release through another channel.
pub const SAME_TORRENT: &str = "같은 토렌트가 이미 이 회차를 대체하고 있어요.";
/// Why a replacement ended with the old video's file still under the episode
/// name after its torrent was removed for it ([`Step::RemovalWaits`]), and
/// the new video gone: no failure while that file is there, but the worker
/// watches it, and the row becomes one if the file goes too.
pub const OLD_FILE_WATCHED: &str = "이전 영상의 토렌트를 지운 뒤 받은 새 영상 파일이 없어져서 대체를 끝냈어요. 회차 이름에 남은 이전 영상 파일이 없어지면 알려요.";
/// How long the rule's folder of a row may be away (a mount that is not
/// there) before the worker stops waiting for it: a failure, or a
/// replacement that ended with no video, is no failure any more
/// ([`RevisionState::Cleared`], [`Step::Abandoned`] without a reason), and a
/// replacement under way, which still holds its torrent, is listed as a
/// failure with [`FOLDER_AWAY`] until the folder is back. A week.
pub const FOLDER_GONE_AFTER: Millis = 7 * 24 * 60 * 60 * 1000;
/// Why a replacement under way waits: its folder has been away for
/// [`FOLDER_GONE_AFTER`].
pub const FOLDER_AWAY: &str = "작품 폴더가 보이지 않아요. 저장 폴더를 7일 넘게 찾지 못해서 대체가 멈춰 있어요. 폴더가 돌아오면 이어가요.";
/// Why a row was skipped because a higher revision of the episode replaced
/// the old video, or is about to.
pub const OVERTAKEN: &str =
    "같은 회차의 더 높은 수정본이 이전 영상을 대체해서 이 수정본은 받은 이름 그대로 뒀어요.";

/// Where a replacement is (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RevisionState {
    Unknown,
    Skipped,
    Receiving,
    Verified,
    Removing,
    Removed,
    Done,
    Failed,
    Cleared,
    Abandoned,
}

impl RevisionState {
    pub const ALL: [RevisionState; 10] = [
        RevisionState::Unknown,
        RevisionState::Skipped,
        RevisionState::Receiving,
        RevisionState::Verified,
        RevisionState::Removing,
        RevisionState::Removed,
        RevisionState::Done,
        RevisionState::Failed,
        RevisionState::Cleared,
        RevisionState::Abandoned,
    ];

    pub fn code(self) -> &'static str {
        match self {
            RevisionState::Unknown => "unknown",
            RevisionState::Skipped => "skipped",
            RevisionState::Receiving => "receiving",
            RevisionState::Verified => "verified",
            RevisionState::Removing => "removing",
            RevisionState::Removed => "removed",
            RevisionState::Done => "done",
            RevisionState::Failed => "failed",
            RevisionState::Cleared => "cleared",
            RevisionState::Abandoned => "abandoned",
        }
    }

    pub fn parse(code: &str) -> Option<RevisionState> {
        RevisionState::ALL.into_iter().find(|s| s.code() == code)
    }

    /// The new torrent is in Transmission for this replacement, which still
    /// has to act on it: a cycle keeps it however its item fares in the feeds.
    pub fn holds_torrent(self) -> bool {
        matches!(
            self,
            RevisionState::Receiving
                | RevisionState::Verified
                | RevisionState::Removing
                | RevisionState::Removed
        )
    }

    /// The old video's torrent may be gone for this replacement: its item is
    /// not to be received again.
    pub fn supersedes_old(self) -> bool {
        matches!(
            self,
            RevisionState::Removing
                | RevisionState::Removed
                | RevisionState::Done
                | RevisionState::Abandoned
        )
    }
}

/// A stored replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub id: i64,
    /// The history item of the new revision.
    pub item_id: i64,
    /// The history item of the video it replaces, when known.
    pub old_item_id: Option<i64>,
    pub rule_id: String,
    /// The rule's save folder, absolute.
    pub folder: String,
    /// The episode's file name in `folder`.
    pub episode_name: String,
    /// The old video's revision, when known.
    pub old_version: Option<u32>,
    pub new_version: u32,
    /// The CRC32 of the episode's file when the worker decided, if it read it
    /// then (a file no torrent of known revision holds).
    pub old_crc: Option<String>,
    /// The torrent removed with the old video, once removed.
    pub old_torrent_hash: Option<String>,
    /// The CRC32 the new release's name carries; `None` when the person
    /// confirmed the replacement with `다시 받기` instead.
    pub expected_crc: Option<String>,
    pub torrent_hash: Option<String>,
    /// The new file's name in `folder` as it was received.
    pub received_name: Option<String>,
    /// The new file's CRC32 as read.
    pub file_crc: Option<String>,
    /// What told the new file apart when its CRC32 was read
    /// ([`crate::revision::FileIdentity::to_text`]): the old video is removed
    /// only while the file under `received_name` is still that one.
    pub file_identity: Option<String>,
    /// When a look, with the folder there, last found the new video missing
    /// (or, before the old video is removed, not the checked file):
    /// [`Step::NewMissing`]. `None` once a look finds it, or finds the folder
    /// away ([`RevisionStore::forget_miss`]). The next such look in a row
    /// ends the replacement.
    pub new_missing_at: Option<Millis>,
    /// When a look first found the rule's folder itself away (a mount that
    /// is not there); `None` once a look finds it
    /// ([`RevisionStore::folder_looked_at`]). See [`FOLDER_GONE_AFTER`].
    pub folder_away_since: Option<Millis>,
    pub state: RevisionState,
    /// Why the replacement failed or waits; free of secret values.
    pub reason: Option<String>,
    pub created_at: Millis,
    pub updated_at: Millis,
    /// When the new video got the episode name.
    pub replaced_at: Option<Millis>,
    /// The row of the higher revision this `skipped` row was skipped for
    /// while that one was on its way ([`Step::Overtaken`]); `None` for any
    /// other skip.
    pub overtaken_by: Option<i64>,
}

impl Revision {
    /// A `받기 실패`: a failure that holds, a rename after the old video was
    /// removed that has not gone through yet, a removal that waits
    /// ([`Step::RemovalWaits`], [`Step::NewMissing`]), a replacement under
    /// way whose folder has been away for long ([`FOLDER_AWAY`]), or one that
    /// ended after the old video was removed while the episode has no video
    /// under its name ([`Step::Abandoned`] with a reason other than
    /// [`OLD_FILE_WATCHED`]).
    pub fn is_failure(&self) -> bool {
        match self.state {
            RevisionState::Failed => true,
            RevisionState::Verified | RevisionState::Removed | RevisionState::Removing => {
                self.reason.is_some()
            }
            // Only its folder away for long has a reason ([`FOLDER_AWAY`]).
            RevisionState::Receiving => self.reason.is_some(),
            RevisionState::Abandoned => self
                .reason
                .as_deref()
                .is_some_and(|reason| reason != OLD_FILE_WATCHED),
            _ => false,
        }
    }

    /// A failure before the new video was received in the rule's folder: its
    /// torrent stopped, reported an error, is elsewhere or is not one file.
    /// The worker keeps looking at it (see the module docs).
    pub fn not_received(&self) -> bool {
        self.state == RevisionState::Failed && self.received_name.is_none()
    }
}

/// A row to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRevision {
    pub item_id: i64,
    pub old_item_id: Option<i64>,
    pub rule_id: String,
    pub folder: String,
    pub episode_name: String,
    pub old_version: Option<u32>,
    pub new_version: u32,
    pub old_crc: Option<String>,
    pub expected_crc: Option<String>,
    pub torrent_hash: Option<String>,
    /// [`RevisionState::Unknown`], [`RevisionState::Skipped`] or
    /// [`RevisionState::Receiving`].
    pub state: RevisionState,
    pub reason: Option<String>,
}

/// One step of a replacement (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A failure before the new video was received is under way again: its
    /// torrent is back without an error, in the rule's folder.
    Receiving,
    Verified {
        received_name: String,
        file_crc: String,
        /// [`Revision::file_identity`].
        file_identity: String,
    },
    Removing,
    /// The old video is gone; `reason` says why the rename has not gone
    /// through, `None` before it was tried.
    Removed {
        reason: Option<String>,
    },
    Done,
    /// `received_name` is the new file's name when the failure learned it
    /// (it is kept otherwise).
    Failed {
        reason: String,
        received_name: Option<String>,
    },
    Cleared,
    /// Before the old video was touched: the folder holds this revision or a
    /// higher one by now. Not for a higher revision of the episode in a row
    /// (that is [`Step::Overtaken`]).
    Skipped {
        reason: String,
    },
    /// Before the old video was touched: a higher revision of the episode
    /// is on its way or replaced the video ([`Claim::Overtaken`]), so the
    /// row is `skipped` ([`OVERTAKEN`]). The store works out which, in the
    /// same transaction: a row skipped for one on its way keeps that row
    /// ([`Revision::overtaken_by`]) and goes back to `receiving` if it fails
    /// ([`Step::Failed`]). Not written when nothing overtakes the row any
    /// more.
    Overtaken,
    /// A `removing` row whose old torrent Transmission took out while the
    /// old video's file is still there: it stays `removing` (the old release
    /// stays superseded) and `reason` says why it waits.
    RemovalWaits {
        reason: String,
    },
    /// A look, with the folder there, found the new video missing (or, before
    /// the old video is removed, not the checked file): the row keeps its
    /// state, `reason` says why it waits, and [`Revision::new_missing_at`]
    /// marks the look. The next such look in a row ends the replacement
    /// ([`Step::Abandoned`]).
    NewMissing {
        reason: String,
    },
    /// A row whose new video was gone (or, before the old video was
    /// removed, not the checked one) on two looks in a row: the replacement
    /// ends as [`RevisionState::Abandoned`]. The rows skipped for it while it
    /// was on its way start over, as when it fails. A `reason` makes it a
    /// `받기 실패`: the old video was removed, so the episode has no video
    /// under its name; written again without one once it has.
    /// [`OLD_FILE_WATCHED`] is no failure: the old video's file is still
    /// there, and the worker watches it ([`Revision::is_failure`]).
    Abandoned {
        reason: Option<String>,
    },
    /// A replacement under way whose folder has been away for
    /// [`FOLDER_GONE_AFTER`]: it keeps its state (and its torrent), and the
    /// reason [`FOLDER_AWAY`] lists it with the failures until a look finds
    /// the folder ([`RevisionStore::folder_looked_at`]).
    FolderGone,
}

/// A history write that goes with a row's write, in one transaction
/// ([`RevisionStore::write_with_history`]).
#[derive(Debug, Clone)]
pub enum HistoryWrite {
    /// A cycle's sighting of an item, which may be new: the row is that item's.
    Observe(Observation),
    /// The outcome of the item `item_id`
    /// ([`crate::store::history::HistoryStore::record_outcome`]).
    Outcome {
        item_id: i64,
        result: HistoryResult,
        rule_id: Option<String>,
        reason: Option<String>,
        torrent_hash: Option<String>,
    },
}

/// What becomes of the row of the history write's item.
#[derive(Debug, Clone)]
pub enum RowWrite {
    /// [`RevisionStore::create`] (the item ID is the history write's). With
    /// `reopen`, a row that failed before its new video was received starts
    /// over with `new.torrent_hash` ([`RevisionStore::reopen`]).
    Create { new: NewRevision, reopen: bool },
    /// [`RevisionStore::confirm`] with the torrent `hash`.
    Confirm {
        hash: String,
        expected_crc: Option<String>,
    },
    /// [`RevisionStore::reopen`] with the torrent `hash`.
    Reopen { hash: String },
}

/// What [`RevisionStore::write_with_history`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The item's row afterwards; `None` when there is none (or no item).
    pub row: Option<Revision>,
    /// For [`HistoryWrite::Observe`]: what recording the sighting did.
    pub recorded: Option<Recorded>,
    /// For [`HistoryWrite::Outcome`]: the item's result afterwards.
    pub stored: Option<HistoryResult>,
}

/// What a history item is to a cycle that would receive it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    /// A replacement row exists for it: the worker decided about it already,
    /// and the replacement steps (not the cycle's add) carry it on.
    Revision(RevisionState),
    /// It is the old video of a replacement that removed (or is removing) its
    /// torrent, or the same torrent through another channel: receiving it
    /// again would bring the old video back.
    Superseded,
    /// Its replacement failed before the new video was received
    /// ([`Revision::not_received`]): the cycle receives it again as decided
    /// in the row.
    Retry(Box<Revision>),
}

/// A release that replaced (or is replacing) the video of an episode: the
/// lower revisions of it are not received into `folder` again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub folder: String,
    /// The new revision's release name.
    pub title: String,
    pub new_version: u32,
}

/// Whether a replacement may remove the old video now ([`RevisionStore::claim`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// It may: the row is `removing` now.
    Go,
    /// Another replacement of the episode is removing the old video or
    /// naming its new one; this one waits for it.
    Wait,
    /// A higher revision of the episode is in place or on its way, or the same
    /// one is in place: this one is to be skipped ([`OVERTAKEN`]).
    Overtaken,
}

/// The old video a replacement removes, as the worker found it just before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OldVideo {
    pub item_id: Option<i64>,
    pub version: Option<u32>,
    /// The torrent removed with it; `None` for a file of no torrent.
    pub torrent_hash: Option<String>,
}

/// A work the library knows at a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkRef {
    pub id: String,
    pub name: String,
}

const COLUMNS: &str = "id, item_id, old_item_id, rule_id, folder, episode_name, old_version, \
     new_version, expected_crc, torrent_hash, received_name, file_crc, state, reason, \
     created_at, updated_at, replaced_at, old_crc, old_torrent_hash, overtaken_by, \
     file_identity, new_missing_at, folder_away_since";

fn from_row(row: &Row<'_>) -> rusqlite::Result<Revision> {
    from_row_at(row, 0)
}

/// A row whose [`COLUMNS`] start at column `at`.
fn from_row_at(row: &Row<'_>, at: usize) -> rusqlite::Result<Revision> {
    let state: String = row.get(at + 12)?;
    let state = RevisionState::parse(&state).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            at + 12,
            rusqlite::types::Type::Text,
            format!("unknown revision state {state:?}").into(),
        )
    })?;
    Ok(Revision {
        id: row.get(at)?,
        item_id: row.get(at + 1)?,
        old_item_id: row.get(at + 2)?,
        rule_id: row.get(at + 3)?,
        folder: row.get(at + 4)?,
        episode_name: row.get(at + 5)?,
        old_version: row.get(at + 6)?,
        new_version: row.get(at + 7)?,
        expected_crc: row.get(at + 8)?,
        torrent_hash: row.get(at + 9)?,
        received_name: row.get(at + 10)?,
        file_crc: row.get(at + 11)?,
        state,
        reason: row.get(at + 13)?,
        created_at: row.get(at + 14)?,
        updated_at: row.get(at + 15)?,
        replaced_at: row.get(at + 16)?,
        old_crc: row.get(at + 17)?,
        old_torrent_hash: row.get(at + 18)?,
        overtaken_by: row.get(at + 19)?,
        file_identity: row.get(at + 20)?,
        new_missing_at: row.get(at + 21)?,
        folder_away_since: row.get(at + 22)?,
    })
}

fn query(
    conn: &Connection,
    filter: &str,
    values: &[&dyn rusqlite::ToSql],
) -> Result<Vec<Revision>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM video_revisions {filter} ORDER BY id"
    ))?;
    let rows = stmt
        .query_map(values, from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn by_id(conn: &Connection, id: i64) -> Result<Option<Revision>> {
    Ok(query(conn, "WHERE id = ?1", &[&id])?.pop())
}

/// Whether another row than `id` that is under way or done has the torrent
/// `hash`: the same release reached the worker through another channel.
fn torrent_taken(conn: &Connection, id: Option<i64>, hash: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM video_revisions
           WHERE torrent_hash = ?1 AND id IS NOT ?2
             AND state IN ('receiving', 'verified', 'removing', 'removed', 'done'))",
        params![hash, id],
        |row| row.get(0),
    )?)
}

/// The rows skipped for the row `id` while it was on its way
/// ([`Revision::overtaken_by`]), which has failed or was abandoned (its video
/// never took the episode name): each starts over as
/// `receiving`, having forgotten what it found, unless another row under way
/// or done has its torrent by now (the same release through another channel,
/// or another row skipped for `id` that started over first), which skips it
/// as that torrent's ([`SAME_TORRENT`]), as [`create_in`] would.
fn revive_overtaken(tx: &Connection, id: i64, at: Millis) -> Result<()> {
    for row in query(tx, "WHERE state = 'skipped' AND overtaken_by = ?1", &[&id])? {
        let taken = match &row.torrent_hash {
            Some(hash) => torrent_taken(tx, Some(row.id), hash)?,
            None => false,
        };
        if taken {
            tx.execute(
                "UPDATE video_revisions SET reason = ?2, overtaken_by = NULL, updated_at = ?3
                  WHERE id = ?1",
                params![row.id, SAME_TORRENT, at],
            )?;
        } else {
            tx.execute(
                "UPDATE video_revisions
                    SET state = 'receiving', reason = NULL, received_name = NULL,
                        file_crc = NULL, file_identity = NULL, overtaken_by = NULL,
                        new_missing_at = NULL, updated_at = ?2
                  WHERE id = ?1",
                params![row.id, at],
            )?;
        }
    }
    Ok(())
}

/// [`RevisionStore::create`] inside the transaction `tx`.
fn create_in(tx: &Connection, at: Millis, new: NewRevision) -> Result<Revision> {
    if let Some(row) = query(tx, "WHERE item_id = ?1", &[&new.item_id])?.pop() {
        return Ok(row);
    }
    let (state, reason) = match &new.torrent_hash {
        Some(hash) if new.state == RevisionState::Receiving && torrent_taken(tx, None, hash)? => {
            (RevisionState::Skipped, Some(SAME_TORRENT.to_owned()))
        }
        _ => (new.state, new.reason),
    };
    tx.execute(
        "INSERT INTO video_revisions (item_id, old_item_id, rule_id, folder,
             episode_name, old_version, new_version, old_crc, expected_crc,
             torrent_hash, state, reason, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)",
        params![
            new.item_id,
            new.old_item_id,
            new.rule_id,
            new.folder,
            new.episode_name,
            new.old_version,
            new.new_version,
            new.old_crc,
            new.expected_crc,
            new.torrent_hash,
            state.code(),
            reason,
            at
        ],
    )?;
    Ok(query(tx, "WHERE item_id = ?1", &[&new.item_id])?
        .pop()
        .expect("the row just written"))
}

/// [`RevisionStore::reopen`] inside the transaction `tx`: a failure before
/// the new video was received ([`Revision::not_received`]), or a replacement
/// that ended with no video under the episode name (an abandoned
/// [`Revision::is_failure`]), starts over having forgotten the file it had
/// checked.
fn reopen_in(tx: &Connection, id: i64, at: Millis, hash: &str) -> Result<Option<Revision>> {
    let (state, reason) = if torrent_taken(tx, Some(id), hash)? {
        ("skipped", Some(SAME_TORRENT))
    } else {
        ("receiving", None)
    };
    tx.execute(
        "UPDATE video_revisions
            SET state = ?2, torrent_hash = ?3, reason = ?4, received_name = NULL,
                file_crc = NULL, file_identity = NULL, new_missing_at = NULL,
                updated_at = ?5
          WHERE id = ?1
            AND ((state = 'failed' AND received_name IS NULL)
                 OR (state = 'abandoned' AND reason IS NOT NULL AND reason <> ?6))",
        params![id, state, hash, reason, at, OLD_FILE_WATCHED],
    )?;
    by_id(tx, id)
}

/// [`RevisionStore::confirm`] inside the transaction `tx`.
fn confirm_in(
    tx: &Connection,
    item_id: i64,
    at: Millis,
    hash: &str,
    expected_crc: Option<&str>,
) -> Result<Option<Revision>> {
    let Some(row) = query(tx, "WHERE item_id = ?1", &[&item_id])?.pop() else {
        return Ok(None);
    };
    if row.state == RevisionState::Unknown {
        let (state, reason) = if torrent_taken(tx, Some(row.id), hash)? {
            ("skipped", Some(SAME_TORRENT))
        } else {
            ("receiving", None)
        };
        tx.execute(
            "UPDATE video_revisions
                SET state = ?2, torrent_hash = ?3, expected_crc = ?4,
                    reason = ?5, updated_at = ?6
              WHERE id = ?1",
            params![row.id, state, hash, expected_crc, reason, at],
        )?;
    }
    by_id(tx, row.id)
}

/// The ID of the history item of `observation`'s channel and identity key.
fn item_of(tx: &Connection, observation: &Observation) -> Result<Option<i64>> {
    Ok(tx
        .query_row(
            "SELECT id FROM history_items WHERE channel_id = ?1 AND identity_key = ?2",
            params![observation.channel_id, observation.identity_key],
            |r| r.get(0),
        )
        .optional()?)
}

/// The other rows of `row`'s episode that are under way or done.
fn siblings(conn: &Connection, row: &Revision) -> Result<Vec<Revision>> {
    query(
        conn,
        "WHERE folder = ?1 AND episode_name = ?2 AND id <> ?3
           AND state IN ('receiving', 'verified', 'removing', 'removed', 'done')",
        &[&row.folder, &row.episode_name, &row.id],
    )
}

/// What keeps a row from replacing the video, of its `siblings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overtaker {
    None,
    /// A replacement of the same or a higher revision is done.
    InPlace,
    /// A higher revision is on its way: the row of the highest one.
    OnItsWay(i64),
}

fn overtaker(siblings: &[Revision], row: &Revision) -> Overtaker {
    if siblings
        .iter()
        .any(|s| s.state == RevisionState::Done && s.new_version >= row.new_version)
    {
        return Overtaker::InPlace;
    }
    siblings
        .iter()
        .filter(|s| s.state != RevisionState::Done && s.new_version > row.new_version)
        .max_by_key(|s| (s.new_version, s.id))
        .map_or(Overtaker::None, |s| Overtaker::OnItsWay(s.id))
}

/// What the other rows of `row`'s episode say about it removing the old video.
fn verdict(conn: &Connection, row: &Revision) -> Result<Claim> {
    let siblings = siblings(conn, row)?;
    if overtaker(&siblings, row) != Overtaker::None {
        return Ok(Claim::Overtaken);
    }
    if siblings
        .iter()
        .any(|s| matches!(s.state, RevisionState::Removing | RevisionState::Removed))
    {
        return Ok(Claim::Wait);
    }
    Ok(Claim::Go)
}

/// Async access to the replacements. Cheap to clone.
#[derive(Clone)]
pub struct RevisionStore {
    db: Db,
}

impl RevisionStore {
    pub fn new(db: Db) -> Self {
        RevisionStore { db }
    }

    /// Creates the row of `new.item_id` at `at`. A row that exists already for
    /// the item is kept as it is and returned: the worker decides about an
    /// item once. A replacement under way whose torrent another row under way
    /// or done has is written as [`RevisionState::Skipped`] ([`SAME_TORRENT`]).
    pub async fn create(&self, at: Millis, new: NewRevision) -> Result<Revision> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let row = create_in(&tx, at, new)?;
                tx.commit()?;
                Ok(row)
            })
            .await
    }

    /// Writes `row` and `history` in one transaction at `at`: a row the
    /// worker decided is never without its history item's record, nor that
    /// record without the row (a `received` item whose replacement was never
    /// written would not be received again and would replace nothing, and a
    /// `버전 미상` one would be received with `다시 받기` and named as an
    /// ordinary item). The row is written first; only a sighting of an item
    /// history does not hold yet is recorded first, as the row needs its ID.
    /// Either both are written or neither.
    pub async fn write_with_history(
        &self,
        at: Millis,
        history: HistoryWrite,
        row: RowWrite,
    ) -> Result<Written> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut written = Written {
                    row: None,
                    recorded: None,
                    stored: None,
                };
                let item_id = match &history {
                    HistoryWrite::Observe(observation) => match item_of(&tx, observation)? {
                        Some(id) => Some(id),
                        None => {
                            written.recorded = history_repo::record_in(
                                &tx,
                                at,
                                history_repo::Origin::Feed,
                                std::slice::from_ref(observation),
                            )?
                            .pop();
                            item_of(&tx, observation)?
                        }
                    },
                    HistoryWrite::Outcome { item_id, .. } => {
                        let exists: bool = tx.query_row(
                            "SELECT EXISTS (SELECT 1 FROM history_items WHERE id = ?1)",
                            [item_id],
                            |r| r.get(0),
                        )?;
                        exists.then_some(*item_id)
                    }
                };
                if let Some(item_id) = item_id {
                    written.row = match row {
                        RowWrite::Create { mut new, reopen } => {
                            new.item_id = item_id;
                            let hash = new.torrent_hash.clone();
                            let row = create_in(&tx, at, new)?;
                            match hash {
                                Some(hash) if reopen && row.not_received() => {
                                    reopen_in(&tx, row.id, at, &hash)?
                                }
                                _ => Some(row),
                            }
                        }
                        RowWrite::Confirm { hash, expected_crc } => {
                            confirm_in(&tx, item_id, at, &hash, expected_crc.as_deref())?
                        }
                        RowWrite::Reopen { hash } => {
                            match query(&tx, "WHERE item_id = ?1", &[&item_id])?.pop() {
                                Some(row) => reopen_in(&tx, row.id, at, &hash)?,
                                None => None,
                            }
                        }
                    };
                }
                match history {
                    HistoryWrite::Observe(observation) => {
                        if written.recorded.is_none() {
                            written.recorded = history_repo::record_in(
                                &tx,
                                at,
                                history_repo::Origin::Feed,
                                &[observation],
                            )?
                            .pop();
                        }
                    }
                    HistoryWrite::Outcome {
                        item_id,
                        result,
                        rule_id,
                        reason,
                        torrent_hash,
                    } => {
                        written.stored = history_repo::record_outcome_in(
                            &tx,
                            item_id,
                            at,
                            result,
                            rule_id.as_deref(),
                            reason.as_deref(),
                            torrent_hash.as_deref(),
                        )?;
                    }
                }
                tx.commit()?;
                Ok(written)
            })
            .await
    }

    /// The row of the history item `item_id`.
    pub async fn by_item(&self, item_id: i64) -> Result<Option<Revision>> {
        self.db
            .run(move |c| Ok(query(c, "WHERE item_id = ?1", &[&item_id])?.pop()))
            .await
    }

    /// A cycle added the torrent `hash` again for the row `id`, a failure
    /// before its new video was received ([`Revision::not_received`]), or
    /// `다시 받기` received it again (such a failure, or a replacement that
    /// ended with no video under the episode name): the replacement starts
    /// over (or is skipped when another row has that torrent). Returns the
    /// row afterwards; a row in any other state is left alone.
    pub async fn reopen(&self, id: i64, at: Millis, hash: String) -> Result<Option<Revision>> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let row = reopen_in(&tx, id, at, &hash)?;
                tx.commit()?;
                Ok(row)
            })
            .await
    }

    /// The person received the item `item_id` with `다시 받기`: a `버전 미상`
    /// row becomes a replacement under way with the torrent `hash` (or is
    /// skipped when another row has that torrent). The CRC32 check is skipped
    /// when the name carries none (`expected_crc` `None`). Returns the row
    /// afterwards; a row in any other state is left alone.
    pub async fn confirm(
        &self,
        item_id: i64,
        at: Millis,
        hash: String,
        expected_crc: Option<String>,
    ) -> Result<Option<Revision>> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let row = confirm_in(&tx, item_id, at, &hash, expected_crc.as_deref())?;
                tx.commit()?;
                Ok(row)
            })
            .await
    }

    /// What the other rows of the episode say about the row `id` removing
    /// the old video, without writing anything ([`RevisionStore::claim`]
    /// decides for good).
    pub async fn verdict(&self, id: i64) -> Result<Claim> {
        self.db
            .run(move |c| match by_id(c, id)? {
                Some(row) => verdict(c, &row),
                None => Ok(Claim::Wait),
            })
            .await
    }

    /// The row `id` (`verified`, or `removing` after a restart) is about to
    /// remove `old`: written as `removing` with what it removes, unless the
    /// other rows of the episode say otherwise ([`Claim`]). Only one row of an
    /// episode is past `verified` and not `done` at a time.
    pub async fn claim(&self, id: i64, at: Millis, old: OldVideo) -> Result<Claim> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let Some(row) = by_id(&tx, id)? else {
                    return Ok(Claim::Wait);
                };
                let claim = match row.state {
                    RevisionState::Removing => Claim::Go,
                    RevisionState::Verified => verdict(&tx, &row)?,
                    _ => Claim::Wait,
                };
                if claim == Claim::Go {
                    tx.execute(
                        "UPDATE video_revisions
                            SET state = 'removing', old_item_id = COALESCE(?2, old_item_id),
                                old_version = COALESCE(?3, old_version),
                                old_torrent_hash = COALESCE(?4, old_torrent_hash),
                                updated_at = ?5
                          WHERE id = ?1",
                        params![id, old.item_id, old.version, old.torrent_hash, at],
                    )?;
                }
                tx.commit()?;
                Ok(claim)
            })
            .await
    }

    /// What the rows say of the items `keys` (identity keys) of the channel
    /// `channel_id`, by identity key; items with nothing to say are absent.
    pub async fn marks(
        &self,
        channel_id: String,
        keys: Vec<String>,
    ) -> Result<HashMap<String, Mark>> {
        if keys.is_empty() {
            return Ok(HashMap::new());
        }
        self.db
            .run(move |c| {
                let wanted: std::collections::HashSet<&String> = keys.iter().collect();
                let mut out = HashMap::new();
                let mut stmt = c.prepare(&format!(
                    "SELECT h.identity_key, {} FROM video_revisions r
                       JOIN history_items h ON h.id = r.item_id WHERE h.channel_id = ?1",
                    COLUMNS
                        .split(", ")
                        .map(|column| format!("r.{}", column.trim()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))?;
                let mut rows = stmt.query([&channel_id])?;
                while let Some(row) = rows.next()? {
                    let key: String = row.get(0)?;
                    if !wanted.contains(&key) {
                        continue;
                    }
                    let revision = from_row_at(row, 1)?;
                    let mark = if revision.not_received() {
                        Mark::Retry(Box::new(revision))
                    } else {
                        Mark::Revision(revision.state)
                    };
                    out.insert(key, mark);
                }
                // The old video's item, and every item of the torrent removed
                // with it (the same release through another channel). A
                // replacement received again after it ended with no video
                // ([`RevisionStore::reopen`]) removed that torrent already.
                let mut stmt = c.prepare(
                    "SELECT h.identity_key FROM video_revisions r
                       JOIN history_items h
                         ON h.id = r.old_item_id
                         OR (r.old_torrent_hash IS NOT NULL AND h.torrent_hash = r.old_torrent_hash)
                      WHERE h.channel_id = ?1
                        AND (r.state IN ('removing', 'removed', 'done', 'abandoned')
                             OR (r.state IN ('receiving', 'verified')
                                 AND r.old_torrent_hash IS NOT NULL))",
                )?;
                let mut rows = stmt.query([&channel_id])?;
                while let Some(row) = rows.next()? {
                    let key: String = row.get(0)?;
                    if wanted.contains(&key) {
                        out.entry(key).or_insert(Mark::Superseded);
                    }
                }
                Ok(out)
            })
            .await
    }

    /// The releases that replaced (or are replacing) a video: their lower
    /// revisions are not received into the folder again. Not one abandoned
    /// ([`RevisionState::Abandoned`]), whose video never took the episode
    /// name: a lower revision may still replace the video there.
    pub async fn replacements(&self) -> Result<Vec<Replacement>> {
        self.db
            .run(|c| {
                let mut stmt = c.prepare(
                    "SELECT r.folder, h.title, r.new_version FROM video_revisions r
                       JOIN history_items h ON h.id = r.item_id
                      WHERE r.state IN ('removing', 'removed', 'done')",
                )?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok(Replacement {
                            folder: row.get(0)?,
                            title: row.get(1)?,
                            new_version: row.get(2)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// The rows of the episode file `episode_name` in `folder`.
    pub async fn of_episode(&self, folder: String, episode_name: String) -> Result<Vec<Revision>> {
        self.db
            .run(move |c| {
                query(
                    c,
                    "WHERE folder = ?1 AND episode_name = ?2",
                    &[&folder, &episode_name],
                )
            })
            .await
    }

    /// The rows the worker still acts on: the replacements under way and the
    /// failures it watches for being resolved.
    pub async fn open(&self) -> Result<Vec<Revision>> {
        self.db
            .run(|c| {
                query(
                    c,
                    "WHERE state IN ('receiving', 'verified', 'removing', 'removed', 'failed')
                        OR (state = 'abandoned' AND reason IS NOT NULL)",
                    &[],
                )
            })
            .await
    }

    /// The torrents the replacements under way still act on: their new
    /// torrents, and the old torrent of one that is removing (it asked for
    /// it to go with its data; taking the torrent out of Transmission without
    /// the data leaves the old video for the person to delete).
    pub async fn held_hashes(&self) -> Result<Vec<String>> {
        self.db
            .run(|c| {
                let mut stmt = c.prepare(
                    "SELECT torrent_hash FROM video_revisions
                      WHERE torrent_hash IS NOT NULL
                        AND state IN ('receiving', 'verified', 'removing', 'removed')
                      UNION
                     SELECT old_torrent_hash FROM video_revisions
                      WHERE old_torrent_hash IS NOT NULL AND state = 'removing'",
                )?;
                let hashes = stmt
                    .query_map([], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
                Ok(hashes)
            })
            .await
    }

    /// Writes `step` on the row `id` at `at` if the row is still in the state
    /// `from` the step was decided on, and returns whether it was written. A
    /// row another write moved on since (a higher revision's `done` skipped it
    /// earlier in the same pass, say) keeps what that write made of it, and
    /// [`Step::Overtaken`] is not written once nothing overtakes the row. `done`
    /// also skips the lower (or equal) revisions of the episode still
    /// receiving or checked: they would replace the video that just took the
    /// name. `failed` puts the rows skipped for this one while it was on its
    /// way ([`Revision::overtaken_by`]) back to `receiving`, with what they
    /// had checked forgotten: the lower revision replaces the video after
    /// all, from its first step, unless another higher one is on its way.
    pub async fn advance(
        &self,
        id: i64,
        at: Millis,
        from: RevisionState,
        step: Step,
    ) -> Result<bool> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let state: Option<String> = tx
                    .query_row(
                        "SELECT state FROM video_revisions WHERE id = ?1",
                        [id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if state.as_deref() != Some(from.code()) {
                    return Ok(false);
                }
                match step {
                    Step::Receiving => tx.execute(
                        "UPDATE video_revisions SET state = 'receiving', reason = NULL,
                             updated_at = ?2 WHERE id = ?1",
                        params![id, at],
                    )?,
                    Step::Verified {
                        received_name,
                        file_crc,
                        file_identity,
                    } => tx.execute(
                        "UPDATE video_revisions SET state = 'verified', received_name = ?2,
                             file_crc = ?3, file_identity = ?5, new_missing_at = NULL,
                             reason = NULL, updated_at = ?4
                          WHERE id = ?1",
                        params![id, received_name, file_crc, at, file_identity],
                    )?,
                    Step::Removing => tx.execute(
                        "UPDATE video_revisions SET state = 'removing', updated_at = ?2
                          WHERE id = ?1",
                        params![id, at],
                    )?,
                    Step::Removed { reason } => tx.execute(
                        "UPDATE video_revisions SET state = 'removed', reason = ?2,
                             updated_at = ?3 WHERE id = ?1",
                        params![id, reason, at],
                    )?,
                    Step::Done => {
                        tx.execute(
                            "UPDATE video_revisions
                                SET state = 'skipped', reason = ?2, overtaken_by = NULL,
                                    updated_at = ?3
                              WHERE id IN (
                                SELECT o.id FROM video_revisions o, video_revisions r
                                 WHERE r.id = ?1 AND o.id <> r.id
                                   AND o.folder = r.folder AND o.episode_name = r.episode_name
                                   AND o.new_version <= r.new_version
                                   AND o.state IN ('receiving', 'verified'))",
                            params![id, OVERTAKEN, at],
                        )?;
                        tx.execute(
                            "UPDATE video_revisions SET state = 'done', reason = NULL,
                                 replaced_at = ?2, updated_at = ?2 WHERE id = ?1",
                            params![id, at],
                        )?
                    }
                    Step::Failed {
                        reason,
                        received_name,
                    } => {
                        let written = tx.execute(
                            "UPDATE video_revisions SET state = 'failed', reason = ?2,
                                 received_name = COALESCE(?4, received_name), updated_at = ?3
                              WHERE id = ?1",
                            params![id, reason, at, received_name],
                        )?;
                        revive_overtaken(&tx, id, at)?;
                        written
                    }
                    Step::Cleared => tx.execute(
                        "UPDATE video_revisions SET state = 'cleared', updated_at = ?2
                          WHERE id = ?1",
                        params![id, at],
                    )?,
                    Step::Skipped { reason } => tx.execute(
                        "UPDATE video_revisions SET state = 'skipped', reason = ?2,
                             overtaken_by = NULL, updated_at = ?3 WHERE id = ?1",
                        params![id, reason, at],
                    )?,
                    Step::Overtaken => {
                        let row = by_id(&tx, id)?.expect("the row whose state was read");
                        let by = match overtaker(&siblings(&tx, &row)?, &row) {
                            Overtaker::None => return Ok(false),
                            Overtaker::InPlace => None,
                            Overtaker::OnItsWay(by) => Some(by),
                        };
                        tx.execute(
                            "UPDATE video_revisions SET state = 'skipped', reason = ?2,
                                 overtaken_by = ?3, updated_at = ?4 WHERE id = ?1",
                            params![id, OVERTAKEN, by, at],
                        )?
                    }
                    Step::Abandoned { reason } => {
                        let written = tx.execute(
                            "UPDATE video_revisions SET state = 'abandoned', reason = ?2,
                                 updated_at = ?3 WHERE id = ?1",
                            params![id, reason, at],
                        )?;
                        revive_overtaken(&tx, id, at)?;
                        written
                    }
                    Step::RemovalWaits { reason } => tx.execute(
                        "UPDATE video_revisions SET reason = ?2, updated_at = ?3
                          WHERE id = ?1 AND state = 'removing'",
                        params![id, reason, at],
                    )?,
                    Step::FolderGone => tx.execute(
                        "UPDATE video_revisions SET reason = ?2, updated_at = ?3
                          WHERE id = ?1",
                        params![id, FOLDER_AWAY, at],
                    )?,
                    Step::NewMissing { reason } => tx.execute(
                        "UPDATE video_revisions SET reason = ?2, new_missing_at = ?3,
                             updated_at = ?3 WHERE id = ?1",
                        params![id, reason, at],
                    )?,
                };
                tx.commit()?;
                Ok(true)
            })
            .await
    }

    /// A look at the new video of the row `id` found it, or found its folder
    /// away: an earlier miss ([`Step::NewMissing`]) no longer counts toward
    /// two in a row. A `reason` that was the miss's (`miss_reason`) goes with
    /// it. Nothing else of the row changes.
    pub async fn forget_miss(&self, id: i64, miss_reason: Option<String>) -> Result<()> {
        self.db
            .run(move |c| {
                c.execute(
                    "UPDATE video_revisions
                        SET new_missing_at = NULL,
                            reason = CASE WHEN reason = ?2 THEN NULL ELSE reason END
                      WHERE id = ?1 AND new_missing_at IS NOT NULL",
                    params![id, miss_reason],
                )?;
                Ok(())
            })
            .await
    }

    /// The new video of the row `id`, `verified` or `removing`, was told by
    /// its CRC32 after its identity had changed (a remount, a copy put
    /// back): `file_identity` is the identity it has now, which the next
    /// look compares instead of reading the whole file again.
    pub async fn keep_identity(&self, id: i64, file_identity: String) -> Result<()> {
        self.db
            .run(move |c| {
                c.execute(
                    "UPDATE video_revisions SET file_identity = ?2
                      WHERE id = ?1 AND state IN ('verified', 'removing')",
                    params![id, file_identity],
                )?;
                Ok(())
            })
            .await
    }

    /// A look at the row `id` found its folder there (`away` `None`), or away
    /// at `away`: [`Revision::folder_away_since`] is the first look of a run
    /// that found it away. A folder found there takes [`FOLDER_AWAY`] away
    /// too; the next look says why the row waits, if it does.
    pub async fn folder_looked_at(&self, id: i64, away: Option<Millis>) -> Result<()> {
        self.db
            .run(move |c| {
                match away {
                    Some(at) => c.execute(
                        "UPDATE video_revisions SET folder_away_since = ?2
                          WHERE id = ?1 AND folder_away_since IS NULL",
                        params![id, at],
                    )?,
                    None => c.execute(
                        "UPDATE video_revisions
                            SET folder_away_since = NULL,
                                reason = CASE WHEN reason = ?2 THEN NULL ELSE reason END
                          WHERE id = ?1 AND folder_away_since IS NOT NULL",
                        params![id, FOLDER_AWAY],
                    )?,
                };
                Ok(())
            })
            .await
    }

    /// The `받기 실패` of replacements (see [`Revision::is_failure`]), newest first.
    pub async fn failures(&self) -> Result<Vec<Revision>> {
        self.db
            .run(|c| {
                let mut rows: Vec<Revision> = query(
                    c,
                    "WHERE state = 'failed'
                        OR (state IN ('receiving', 'verified', 'removed', 'removing',
                                      'abandoned')
                            AND reason IS NOT NULL)",
                    &[],
                )?
                .into_iter()
                .filter(Revision::is_failure)
                .collect();
                rows.sort_by_key(|r| std::cmp::Reverse((r.updated_at, r.id)));
                Ok(rows)
            })
            .await
    }

    /// The rows whose folder is inside `work_folder` (an absolute path) that
    /// an episode row shows: replacements that are done and failures.
    pub async fn in_work_folder(&self, work_folder: String) -> Result<Vec<Revision>> {
        self.db
            .run(move |c| {
                let prefix = format!("{}/", work_folder.trim_end_matches('/'));
                let rows = query(
                    c,
                    "WHERE state IN ('done', 'failed', 'receiving', 'verified', 'removed',
                                     'removing', 'abandoned')
                       AND substr(folder, 1, length(?1)) = ?1",
                    &[&prefix],
                )?;
                Ok(rows
                    .into_iter()
                    .filter(|r| r.state == RevisionState::Done || r.is_failure())
                    .collect())
            })
            .await
    }

    /// The library work whose folder holds the season folder `folder`, in a
    /// registered watch folder.
    pub async fn work_at(&self, folder: String) -> Result<Option<WorkRef>> {
        self.db
            .run(move |c| {
                let path = std::path::Path::new(&folder);
                let Some(parent) = path.parent().and_then(|p| p.to_str()) else {
                    return Ok(None);
                };
                let parent = parent.trim_end_matches('/').to_owned();
                Ok(c.query_row(
                    "SELECT w.id, w.dir_name FROM works w
                       JOIN watch_folders f ON f.id = w.watch_folder_id
                      WHERE f.unregistered_at IS NULL
                        AND rtrim(f.path, '/') || '/' || w.dir_name = ?1",
                    [parent],
                    |row| {
                        Ok(WorkRef {
                            id: row.get(0)?,
                            name: row.get(1)?,
                        })
                    },
                )
                .optional()?)
            })
            .await
    }

    /// The rows of the history items `ids`, by item ID.
    pub async fn of_items(&self, ids: Vec<i64>) -> Result<HashMap<i64, Revision>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.db
            .run(move |c| {
                let marks = vec!["?"; ids.len()].join(", ");
                let mut stmt = c.prepare(&format!(
                    "SELECT {COLUMNS} FROM video_revisions WHERE item_id IN ({marks})"
                ))?;
                let rows = stmt
                    .query_map(params_from_iter(ids.iter()), from_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows.into_iter().map(|r| (r.item_id, r)).collect())
            })
            .await
    }
}
