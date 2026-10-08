//! The videos a person is asked about (`docs/specs/library.md`, 할 일과 회차
//! 목록; `docs/specs/jobs.md`, 할 일): `회차 확인 필요` for a video directly in a
//! season folder other than `Season 00` whose name gives no episode, or another
//! season's ([`crate::discovery`]). The app does not put such a video on an
//! episode, and does not count it missing either: it asks.
//!
//! - The scan records each such video's size and modification time with its
//!   unrecognized row (migration `library/video_check.sql`). One whose size and
//!   time the scan could not read is not asked about until a scan reads them.
//! - A person's `확인함` ([`LibraryStore::check_video`]) names the file as the
//!   screen saw it ([`SeenFile`]), and holds while the scan finds a video of
//!   that size and time at the path. Renaming or moving the video ends the
//!   question too (the path is no longer one the scan could not attach); a
//!   different video put at the path is asked about again
//!   ([`super::repo`] drops the mark). A video that changed since the screen
//!   read it is not marked ([`CheckError::Changed`]).
//! - Only the works the library lists are asked about: not one whose folder is
//!   gone, nor one of an unregistered watch folder.
//! - Nothing here touches a file on disk, and it makes no job.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::LibraryStore;
use crate::discovery::{season_of_folder, Reason, SeenFile};
use trss_core::{db::DbError, Millis};

#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error(transparent)]
    Db(#[from] DbError),
    /// The work has no video asked about at this path.
    #[error("there is no such video to check")]
    NoFile,
    /// The video at the path is not the one the caller saw.
    #[error("the video changed")]
    Changed,
}

impl From<rusqlite::Error> for CheckError {
    fn from(e: rusqlite::Error) -> Self {
        CheckError::Db(DbError::Sqlite(e))
    }
}

/// A video a person is asked about and has not checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoCheck {
    pub work_id: String,
    /// The work folder's name.
    pub work_name: String,
    /// The season of the folder it is in.
    pub season: u32,
    /// Relative to the work folder: the season folder and the file name.
    pub path: String,
    /// [`Reason::NoEpisode`] or [`Reason::SeasonMismatch`].
    pub reason: Reason,
    pub identity: SeenFile,
}

/// A work's unrecognized files, by path: the path, the reason, and whether a
/// person's `확인함` holds for it.
pub(super) const UNRECOGNIZED_OF_WORK: &str = "
    SELECT u.path, u.reason,
           EXISTS (SELECT 1 FROM unrecognized_checks c
                    WHERE c.work_id = u.work_id AND c.path = u.path
                      AND c.size = u.size AND c.mtime_ns = u.mtime_ns)
      FROM unrecognized_files u WHERE u.work_id = ?1 ORDER BY u.path";

/// The size and time columns as an identity, when both are there.
pub(super) fn identity_of(size: Option<i64>, mtime_ns: Option<i64>) -> Option<SeenFile> {
    Some(SeenFile {
        size: u64::try_from(size?).ok()?,
        mtime_ns: mtime_ns?,
    })
}

pub(super) fn open_checks(conn: &Connection) -> rusqlite::Result<Vec<VideoCheck>> {
    let mut stmt = conn.prepare_cached(
        "SELECT u.work_id, w.dir_name, u.path, u.reason, u.size, u.mtime_ns
           FROM unrecognized_files u
           JOIN works w ON w.id = u.work_id
           JOIN watch_folders f ON f.id = w.watch_folder_id
          WHERE u.size IS NOT NULL AND u.mtime_ns IS NOT NULL
            AND w.missing = 0 AND f.unregistered_at IS NULL
            AND NOT EXISTS (
                SELECT 1 FROM unrecognized_checks c
                 WHERE c.work_id = u.work_id AND c.path = u.path
                   AND c.size = u.size AND c.mtime_ns = u.mtime_ns)
          ORDER BY w.dir_name, u.work_id, u.path",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, Option<i64>>(5)?,
        ))
    })?;
    let mut checks = Vec::new();
    for row in rows {
        let (work_id, work_name, path, reason, size, mtime_ns) = row?;
        // What the scan wrote, so each part is there; a row that is not is
        // left out rather than shown wrong.
        let season = path
            .split_once('/')
            .and_then(|(folder, _)| season_of_folder(folder));
        let (Some(season), Some(reason), Some(identity)) = (
            season,
            Reason::from_code(&reason),
            identity_of(size, mtime_ns),
        ) else {
            continue;
        };
        checks.push(VideoCheck {
            work_id,
            work_name,
            season,
            path,
            reason,
            identity,
        });
    }
    Ok(checks)
}

pub(super) fn check(
    conn: &mut Connection,
    work_id: &str,
    path: &str,
    seen: SeenFile,
    now: Millis,
) -> Result<(), CheckError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = tx
        .prepare_cached(
            "SELECT size, mtime_ns FROM unrecognized_files WHERE work_id = ?1 AND path = ?2",
        )?
        .query_row(params![work_id, path], |row| {
            Ok(identity_of(row.get(0)?, row.get(1)?))
        })
        .optional()?;
    let Some(Some(current)) = current else {
        return Err(CheckError::NoFile);
    };
    if current != seen {
        return Err(CheckError::Changed);
    }
    tx.prepare_cached(
        "INSERT OR REPLACE INTO unrecognized_checks (work_id, path, size, mtime_ns, checked_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?
    .execute(params![
        work_id,
        path,
        current.size as i64,
        current.mtime_ns,
        now
    ])?;
    tx.commit()?;
    Ok(())
}

impl LibraryStore {
    /// The videos the library's works have that a person is asked about and
    /// has not checked, by work name and path.
    pub async fn video_checks(&self) -> Result<Vec<VideoCheck>, DbError> {
        self.db.run(|c| Ok::<_, DbError>(open_checks(c)?)).await
    }

    /// A person's `확인함` on the video at `path` of the work, which the screen
    /// saw as `seen`: it is no longer asked about while that video is there.
    /// Checking it again keeps one mark, with the later time.
    pub async fn check_video(
        &self,
        work_id: &str,
        path: &str,
        seen: SeenFile,
        now: Millis,
    ) -> Result<(), CheckError> {
        let (work_id, path) = (work_id.to_owned(), path.to_owned());
        self.db
            .run(move |c| check(c, &work_id, &path, seen, now))
            .await
    }
}
