//! The creator of a subtitle file (`docs/specs/library.md`, 머리와 시즌의 `제작자
//! 지정`): which creator of the season's Anissia anime made the file, as the
//! user said. The columns are in the migration `library/subtitle_creator.sql`.
//!
//! - A subtitle file the scan finds, or the user puts in a watch folder, has no
//!   creator (`제작자 알 수 없음`). The user names one for all of a season's
//!   unknown ones at once ([`LibraryStore::name_unknown_creators`]) or changes
//!   one file's ([`LibraryStore::set_file_creator`]).
//! - **An applied copy has its stored copy's creator.** A subtitle file the app
//!   applied beside a video (a live `subtitle_applied` row at the file's path)
//!   is by the creator of the stored copy it was made from (`제작자 알 수
//!   없음` when that has none), read from that relation when the work detail is
//!   read ([`crate::store::library::AppliedCopy`]) and written nowhere. It
//!   comes before a creator the user named for the same path, and a season's
//!   unknown files, which [`LibraryStore::name_unknown_creators`] names, leave
//!   applied copies out. Subscribed-creator receipt does not read it.
//! - The time the creator was named (`creator_set_at`) is kept with it: the
//!   subscribed creator's automatic receipt takes a line of that creator for the
//!   file's episode as a revision only when the app first observed it after
//!   that time. Changing the creator sets it again, putting the file back to
//!   unknown clears it, and a same-creator "change" leaves it alone.
//! - The creator is a subtitle source, never a name: the caller makes sure it
//!   is a creator of the anime the season is linked to.
//! - **File identity.** The attribution belongs to the file's row, which is
//!   found by the work and the path below the work folder. A rescan that finds
//!   the file as it was leaves it alone, one that reads the file's episode
//!   differently carries it to the row it records again, and an archive move of
//!   the work folder keeps it (the path below the folder does not change).
//!   A file that is renamed, or that leaves the folder and comes back, is a new
//!   file with an unknown creator: the app has no stronger identity for a
//!   file than its place.
//! - **Concurrency.** Every file has a version. Changing one file's creator
//!   names the version read; a stale one changes nothing
//!   ([`CreatorError::Conflict`]). The version goes up with every change of the
//!   creator, also one a scan makes (the file is read as a video and loses it),
//!   and a file the scan records as new starts at that scan's time in
//!   milliseconds, past any version an earlier file of the same path had (a
//!   screen that read a file that was then removed cannot change the file that
//!   comes back). The limit: a path that is removed and found again within one
//!   millisecond of a version it reached by many changes. When a merge of two
//!   works meets one path, the file with a creator wins, and the version goes
//!   past both. Naming the creator of a season's unknown
//!   files touches only the files that are unknown when it runs, so it never
//!   replaces a creator another screen named.
//! - Nothing here touches a file on disk, and it makes no job.

use rusqlite::{params, Connection, OptionalExtension};

use super::{FileCreator, LibraryStore};
use trss_core::{db::DbError, Millis};

#[derive(Debug, thiserror::Error)]
pub enum CreatorError {
    #[error(transparent)]
    Db(#[from] DbError),
    /// The work has no subtitle file at this path in this season.
    #[error("there is no such subtitle file")]
    NoFile,
    /// The file's creator is not at the version the caller read.
    #[error("the file's creator changed")]
    Conflict,
}

impl From<rusqlite::Error> for CreatorError {
    fn from(e: rusqlite::Error) -> Self {
        CreatorError::Db(DbError::Sqlite(e))
    }
}

/// A subtitle file whose creator the user named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributedSubtitle {
    /// Relative to the work folder.
    pub path: String,
    /// The episode as written in the file's name.
    pub episode: String,
    /// The creator's subtitle source.
    pub source_id: String,
    /// When the user named the creator.
    pub set_at: Millis,
}

/// A subtitle file's creator and the version it is at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatorSet {
    /// `None`: `제작자 알 수 없음`.
    pub creator: Option<FileCreator>,
    pub version: i64,
}

/// The creator a row's columns say, `None` when there is no source.
pub(super) fn creator_of(
    source_id: Option<String>,
    name: Option<String>,
    anime_no: Option<i64>,
) -> Option<FileCreator> {
    Some(FileCreator {
        source_id: source_id?,
        name: name?,
        anime_no: anime_no?,
    })
}

pub(super) fn attributed(
    conn: &Connection,
    work_id: &str,
    season: u32,
) -> rusqlite::Result<Vec<AttributedSubtitle>> {
    let mut stmt = conn.prepare_cached(
        "SELECT path, episode, creator_source_id, creator_set_at FROM media_files
          WHERE work_id = ?1 AND season = ?2 AND kind = 'subtitle'
            AND creator_source_id IS NOT NULL
          ORDER BY path",
    )?;
    let rows = stmt.query_map(params![work_id, season], |row| {
        Ok(AttributedSubtitle {
            path: row.get(0)?,
            episode: row.get(1)?,
            source_id: row.get(2)?,
            // Named before the time was kept: as old as it gets.
            set_at: row.get::<_, Option<Millis>>(3)?.unwrap_or(0),
        })
    })?;
    rows.collect()
}

pub(super) fn name_unknown(
    conn: &mut Connection,
    work_id: &str,
    season: u32,
    source_id: &str,
    now: Millis,
) -> rusqlite::Result<usize> {
    conn.prepare_cached(
        "UPDATE media_files
            SET creator_source_id = ?3, creator_set_at = ?4,
                creator_version = creator_version + 1
          WHERE work_id = ?1 AND season = ?2 AND kind = 'subtitle'
            AND creator_source_id IS NULL
            AND NOT EXISTS (SELECT 1 FROM subtitle_applied ap
                             WHERE ap.work_id = media_files.work_id
                               AND ap.path = media_files.path AND ap.removed_at IS NULL)",
    )?
    .execute(params![work_id, season, source_id, now])
}

pub(super) fn file_creator(
    conn: &Connection,
    work_id: &str,
    season: u32,
    path: &str,
) -> rusqlite::Result<Option<CreatorSet>> {
    conn.prepare_cached(
        "SELECT m.creator_source_id, s.creator_name, s.anime_no, m.creator_version
           FROM media_files m LEFT JOIN subtitle_sources s ON s.id = m.creator_source_id
          WHERE m.work_id = ?1 AND m.season = ?2 AND m.path = ?3 AND m.kind = 'subtitle'",
    )?
    .query_row(params![work_id, season, path], |row| {
        Ok(CreatorSet {
            creator: creator_of(row.get(0)?, row.get(1)?, row.get(2)?),
            version: row.get(3)?,
        })
    })
    .optional()
}

pub(super) fn set_file(
    conn: &mut Connection,
    work_id: &str,
    season: u32,
    path: &str,
    version: i64,
    source_id: Option<&str>,
    now: Millis,
) -> Result<CreatorSet, CreatorError> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let current: Option<(Option<String>, i64)> = tx
        .prepare_cached(
            "SELECT creator_source_id, creator_version FROM media_files
              WHERE work_id = ?1 AND season = ?2 AND path = ?3 AND kind = 'subtitle'",
        )?
        .query_row(params![work_id, season, path], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()?;
    let Some((held, held_version)) = current else {
        return Err(CreatorError::NoFile);
    };
    if held_version != version {
        return Err(CreatorError::Conflict);
    }
    // The same creator again is no change, so it does not move the version.
    if held.as_deref() != source_id {
        tx.prepare_cached(
            "UPDATE media_files
                SET creator_source_id = ?4, creator_version = creator_version + 1,
                    creator_set_at = CASE WHEN ?4 IS NULL THEN NULL ELSE ?5 END
              WHERE work_id = ?1 AND season = ?2 AND path = ?3",
        )?
        .execute(params![work_id, season, path, source_id, now])?;
    }
    tx.commit()?;
    file_creator(conn, work_id, season, path)?.ok_or(CreatorError::NoFile)
}

impl LibraryStore {
    /// The subtitle files of season `season` of the work whose creator the user
    /// named, by path.
    pub async fn attributed_subtitles(
        &self,
        work_id: &str,
        season: u32,
    ) -> Result<Vec<AttributedSubtitle>, DbError> {
        let work_id = work_id.to_owned();
        self.db
            .run(move |c| Ok::<_, DbError>(attributed(c, &work_id, season)?))
            .await
    }

    /// Names `source_id` the creator of every subtitle file of the season that
    /// has none yet, and returns how many. Files that have a creator are left
    /// as they are, and so are the app's applied copies, whose creator is that
    /// of their stored copy.
    pub async fn name_unknown_creators(
        &self,
        work_id: &str,
        season: u32,
        source_id: &str,
        now: Millis,
    ) -> Result<usize, DbError> {
        let (work_id, source_id) = (work_id.to_owned(), source_id.to_owned());
        self.db
            .run(move |c| Ok::<_, DbError>(name_unknown(c, &work_id, season, &source_id, now)?))
            .await
    }

    /// Changes the creator of one subtitle file to `source_id` (`None`: back to
    /// `제작자 알 수 없음`), provided the file's creator is still at `version`.
    pub async fn set_file_creator(
        &self,
        work_id: &str,
        season: u32,
        path: &str,
        version: i64,
        source_id: Option<&str>,
        now: Millis,
    ) -> Result<CreatorSet, CreatorError> {
        let (work_id, path, source_id) = (
            work_id.to_owned(),
            path.to_owned(),
            source_id.map(str::to_owned),
        );
        self.db
            .run(move |c| {
                set_file(
                    c,
                    &work_id,
                    season,
                    &path,
                    version,
                    source_id.as_deref(),
                    now,
                )
            })
            .await
    }

    /// A subtitle file's creator now, or `None` when the work has no such file.
    pub async fn file_creator(
        &self,
        work_id: &str,
        season: u32,
        path: &str,
    ) -> Result<Option<CreatorSet>, DbError> {
        let (work_id, path) = (work_id.to_owned(), path.to_owned());
        self.db
            .run(move |c| Ok::<_, DbError>(file_creator(c, &work_id, season, &path)?))
            .await
    }
}
