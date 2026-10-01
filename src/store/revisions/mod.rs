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
//!   one) already; nothing is received.
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
//! did not go through yet; the worker tries again while the name is free.

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};

use super::{
    db::{Db, DbError},
    history::Millis,
};

#[derive(Debug, thiserror::Error)]
pub enum RevisionError {
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<rusqlite::Error> for RevisionError {
    fn from(e: rusqlite::Error) -> Self {
        RevisionError::Db(DbError::Sqlite(e))
    }
}

type Result<T> = std::result::Result<T, RevisionError>;

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
}

impl RevisionState {
    pub const ALL: [RevisionState; 9] = [
        RevisionState::Unknown,
        RevisionState::Skipped,
        RevisionState::Receiving,
        RevisionState::Verified,
        RevisionState::Removing,
        RevisionState::Removed,
        RevisionState::Done,
        RevisionState::Failed,
        RevisionState::Cleared,
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
            RevisionState::Removing | RevisionState::Removed | RevisionState::Done
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
    /// The CRC32 the new release's name carries; `None` when the person
    /// confirmed the replacement with `다시 받기` instead.
    pub expected_crc: Option<String>,
    pub torrent_hash: Option<String>,
    /// The new file's name in `folder` as it was received.
    pub received_name: Option<String>,
    /// The new file's CRC32 as read.
    pub file_crc: Option<String>,
    pub state: RevisionState,
    /// Why the replacement failed or waits; free of secret values.
    pub reason: Option<String>,
    pub created_at: Millis,
    pub updated_at: Millis,
    /// When the new video got the episode name.
    pub replaced_at: Option<Millis>,
}

impl Revision {
    /// A `받기 실패`: a failure that holds, or a rename after the old video was
    /// removed that has not gone through yet.
    pub fn is_failure(&self) -> bool {
        self.state == RevisionState::Failed
            || (self.state == RevisionState::Removed && self.reason.is_some())
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
    Verified {
        received_name: String,
        file_crc: String,
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
}

/// What a history item is to a cycle that would receive it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// A replacement row exists for it: the worker decided about it already,
    /// and the replacement steps (not the cycle's add) carry it on.
    Revision(RevisionState),
    /// It is the old video of a replacement that removed (or is removing) its
    /// torrent: receiving it again would bring the old video back.
    Superseded,
}

/// A work the library knows at a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkRef {
    pub id: String,
    pub name: String,
}

const COLUMNS: &str = "id, item_id, old_item_id, rule_id, folder, episode_name, old_version, \
     new_version, expected_crc, torrent_hash, received_name, file_crc, state, reason, \
     created_at, updated_at, replaced_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<Revision> {
    let state: String = row.get(12)?;
    let state = RevisionState::parse(&state).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            12,
            rusqlite::types::Type::Text,
            format!("unknown revision state {state:?}").into(),
        )
    })?;
    Ok(Revision {
        id: row.get(0)?,
        item_id: row.get(1)?,
        old_item_id: row.get(2)?,
        rule_id: row.get(3)?,
        folder: row.get(4)?,
        episode_name: row.get(5)?,
        old_version: row.get(6)?,
        new_version: row.get(7)?,
        expected_crc: row.get(8)?,
        torrent_hash: row.get(9)?,
        received_name: row.get(10)?,
        file_crc: row.get(11)?,
        state,
        reason: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
        replaced_at: row.get(16)?,
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
    /// item once.
    pub async fn create(&self, at: Millis, new: NewRevision) -> Result<Revision> {
        self.db
            .run(move |c| {
                c.execute(
                    "INSERT INTO video_revisions (item_id, old_item_id, rule_id, folder,
                         episode_name, old_version, new_version, expected_crc, torrent_hash,
                         state, reason, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)
                     ON CONFLICT (item_id) DO NOTHING",
                    params![
                        new.item_id,
                        new.old_item_id,
                        new.rule_id,
                        new.folder,
                        new.episode_name,
                        new.old_version,
                        new.new_version,
                        new.expected_crc,
                        new.torrent_hash,
                        new.state.code(),
                        new.reason,
                        at
                    ],
                )?;
                Ok(query(c, "WHERE item_id = ?1", &[&new.item_id])?
                    .pop()
                    .expect("the row just written"))
            })
            .await
    }

    /// The row of the history item `item_id`.
    pub async fn by_item(&self, item_id: i64) -> Result<Option<Revision>> {
        self.db
            .run(move |c| Ok(query(c, "WHERE item_id = ?1", &[&item_id])?.pop()))
            .await
    }

    /// The person received the item `item_id` with `다시 받기`: a `버전 미상`
    /// row becomes a replacement under way with the torrent `hash`. The CRC32
    /// check is skipped when the name carries none (`expected_crc` `None`).
    /// Returns the row afterwards; a row in any other state is left alone.
    pub async fn confirm(
        &self,
        item_id: i64,
        at: Millis,
        hash: String,
        expected_crc: Option<String>,
    ) -> Result<Option<Revision>> {
        self.db
            .run(move |c| {
                c.execute(
                    "UPDATE video_revisions
                        SET state = 'receiving', torrent_hash = ?2, expected_crc = ?3,
                            reason = NULL, updated_at = ?4
                      WHERE item_id = ?1 AND state = 'unknown'",
                    params![item_id, hash, expected_crc, at],
                )?;
                Ok(query(c, "WHERE item_id = ?1", &[&item_id])?.pop())
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
                let mut out = HashMap::new();
                let mut stmt = c.prepare(
                    "SELECT h.identity_key, r.state, 1 FROM video_revisions r
                       JOIN history_items h ON h.id = r.item_id WHERE h.channel_id = ?1
                     UNION ALL
                     SELECT h.identity_key, r.state, 0 FROM video_revisions r
                       JOIN history_items h ON h.id = r.old_item_id WHERE h.channel_id = ?1",
                )?;
                let wanted: std::collections::HashSet<&String> = keys.iter().collect();
                let mut rows = stmt.query([&channel_id])?;
                while let Some(row) = rows.next()? {
                    let key: String = row.get(0)?;
                    if !wanted.contains(&key) {
                        continue;
                    }
                    let state: String = row.get(1)?;
                    let Some(state) = RevisionState::parse(&state) else {
                        continue;
                    };
                    let is_new: bool = row.get(2)?;
                    if is_new {
                        out.insert(key, Mark::Revision(state));
                    } else if state.supersedes_old() {
                        out.entry(key).or_insert(Mark::Superseded);
                    }
                }
                Ok(out)
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
                    "WHERE state IN ('receiving', 'verified', 'removing', 'removed', 'failed')",
                    &[],
                )
            })
            .await
    }

    /// The new torrents the replacements under way still act on.
    pub async fn held_hashes(&self) -> Result<Vec<String>> {
        self.db
            .run(|c| {
                let mut stmt = c.prepare(
                    "SELECT torrent_hash FROM video_revisions
                      WHERE torrent_hash IS NOT NULL
                        AND state IN ('receiving', 'verified', 'removing', 'removed')",
                )?;
                let hashes = stmt
                    .query_map([], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
                Ok(hashes)
            })
            .await
    }

    /// Writes `step` on the row `id` at `at`.
    pub async fn advance(&self, id: i64, at: Millis, step: Step) -> Result<()> {
        self.db
            .run(move |c| {
                match step {
                    Step::Verified {
                        received_name,
                        file_crc,
                    } => c.execute(
                        "UPDATE video_revisions SET state = 'verified', received_name = ?2,
                             file_crc = ?3, reason = NULL, updated_at = ?4 WHERE id = ?1",
                        params![id, received_name, file_crc, at],
                    )?,
                    Step::Removing => c.execute(
                        "UPDATE video_revisions SET state = 'removing', updated_at = ?2
                          WHERE id = ?1",
                        params![id, at],
                    )?,
                    Step::Removed { reason } => c.execute(
                        "UPDATE video_revisions SET state = 'removed', reason = ?2,
                             updated_at = ?3 WHERE id = ?1",
                        params![id, reason, at],
                    )?,
                    Step::Done => c.execute(
                        "UPDATE video_revisions SET state = 'done', reason = NULL,
                             replaced_at = ?2, updated_at = ?2 WHERE id = ?1",
                        params![id, at],
                    )?,
                    Step::Failed {
                        reason,
                        received_name,
                    } => c.execute(
                        "UPDATE video_revisions SET state = 'failed', reason = ?2,
                             received_name = COALESCE(?4, received_name), updated_at = ?3
                          WHERE id = ?1",
                        params![id, reason, at, received_name],
                    )?,
                    Step::Cleared => c.execute(
                        "UPDATE video_revisions SET state = 'cleared', updated_at = ?2
                          WHERE id = ?1",
                        params![id, at],
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
                let mut rows = query(
                    c,
                    "WHERE state = 'failed' OR (state = 'removed' AND reason IS NOT NULL)",
                    &[],
                )?;
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
                    "WHERE state IN ('done', 'failed', 'removed')
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
