//! The library: what discovery found in the watch folders
//! (`docs/specs/library.md`, 작품 발견과 감시 폴더).
//!
//! A [`WatchFolder`] is a folder the user registered. The worker (every cycle,
//! and on `다시 확인`) and the web (when a folder is added) read it with
//! [`crate::discovery::scan`] and hand the result to this store, which keeps
//! the works, seasons, episodes and files it found.
//!
//! # What a scan changes
//!
//! - **Added times.** A file or work first seen by the scan that *baselines* a
//!   folder (the first one that read it without any error) has no added time
//!   (`None`: 미상): it may have been there long before the app looked. One that
//!   first appears in a later scan gets that scan's time. A time is never read
//!   from the file system and never changes once set.
//! - **Files that are gone** are dropped from the record, and an episode with no
//!   file left goes with them. A work whose folder is gone stays, with its ID
//!   and everything recorded for it, marked [`WorkRecord::missing`].
//! - **A scan that failed to read** the folder records the error on the folder
//!   and leaves every earlier record as it was. A work folder that could not be
//!   read (the rest of the folder was) keeps its records, and the folder's
//!   error says so; a folder with such an error is not yet baselined.
//!
//! # A work keeps its ID when its folder moves
//!
//! A work is identified by an app-issued ID and found again by its folder's
//! place. [`LibraryStore::follow_move`] is how the archive move
//! ([`crate::worker::commands::rule_archive`]) keeps the ID: it changes the
//! work's watch folder, or, when the destination already has a work of that
//! name, merges the moved work's records into it and keeps the destination's ID.
//! A folder moved by hand is not recognized by its name: the old work goes
//! missing and the new place is a new work.
//!
//! Removing a watch folder removes its works from the library and touches no
//! file.

mod detail;
mod overview;
mod repo;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

pub use detail::{EpisodeDetail, SeasonDetail, WorkDetail};
pub use overview::{EpisodeRange, SubtitleCoverage, WorkOverview};

use crate::{
    discovery::{FileKind, Reason, Scan, ScanError},
    store::{
        db::{Db, DbError},
        history::Millis,
    },
};

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error(transparent)]
    Db(#[from] DbError),
    /// A watch folder with this path is registered already.
    #[error("the watch folder is registered already")]
    Duplicate,
}

impl From<rusqlite::Error> for LibraryError {
    fn from(e: rusqlite::Error) -> Self {
        LibraryError::Db(DbError::Sqlite(e))
    }
}

/// A registered watch folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchFolder {
    pub id: String,
    /// As the user typed it (trailing slashes dropped).
    pub path: String,
    pub created_at: Millis,
    /// Whether a scan has read the folder without error yet; until then what
    /// scans find has no added time.
    pub baselined: bool,
    /// The last attempt to read the folder, successful or not.
    pub checked_at: Option<Millis>,
    /// A sentence while the last attempt could not read everything.
    pub error: Option<String>,
}

/// A watch folder with the counts its row shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSummary {
    pub folder: WatchFolder,
    /// All works of the folder, including those whose folder is gone.
    pub works: usize,
    /// Of those, the works whose folder is gone.
    pub missing_works: usize,
    /// The works first seen after the folder was baselined, at or after the
    /// time asked for.
    pub new_works: usize,
}

/// What recording one scan came to, for logs and answers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// Whether this scan was the folder's first clean one.
    pub baseline: bool,
    /// Works whose folder was read and is there.
    pub works_found: usize,
    /// Works recorded for the first time by this scan.
    pub works_added: usize,
    /// Works that are missing after this scan.
    pub works_missing: usize,
    pub files_added: usize,
    pub files_removed: usize,
    /// Work folders that could not be read and kept their records.
    pub works_unreadable: usize,
    /// The folder's error after this scan.
    pub error: Option<String>,
}

/// What [`LibraryStore::follow_move`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followed {
    /// No work was recorded for the moved folder.
    NotTracked,
    /// The work now belongs to the destination folder, same ID.
    Moved,
    /// The destination had a work of that name; the moved work's records were
    /// merged into it and its row is gone.
    Merged,
}

/// One file of an episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRecord {
    /// Relative to the work folder.
    pub path: String,
    pub kind: FileKind,
    /// `None`: unknown.
    pub added_at: Option<Millis>,
}

/// One recorded episode: a season and the episode as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeRecord {
    pub season: u32,
    pub episode: String,
    pub files: Vec<FileRecord>,
}

/// A file that could not be attached to an episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnrecognizedRecord {
    pub path: String,
    pub reason: Reason,
}

/// A recorded work with everything under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkRecord {
    pub id: String,
    pub watch_folder_id: String,
    pub dir_name: String,
    /// `None`: unknown.
    pub first_seen_at: Option<Millis>,
    pub missing: bool,
    pub seasons: Vec<u32>,
    pub episodes: Vec<EpisodeRecord>,
    pub unrecognized: Vec<UnrecognizedRecord>,
}

impl WorkRecord {
    /// The files of every episode, by path.
    pub fn files(&self) -> BTreeMap<&str, &FileRecord> {
        self.episodes
            .iter()
            .flat_map(|e| e.files.iter())
            .map(|f| (f.path.as_str(), f))
            .collect()
    }
}

/// Async access to the library. Cheap to clone.
#[derive(Clone)]
pub struct LibraryStore {
    db: Db,
}

impl LibraryStore {
    pub fn new(db: Db) -> Self {
        LibraryStore { db }
    }

    /// The watch folders in the order they were registered.
    pub async fn folders(&self) -> Result<Vec<WatchFolder>, LibraryError> {
        self.db.run(|c| Ok(repo::folders(c)?)).await
    }

    pub async fn folder(&self, id: &str) -> Result<Option<WatchFolder>, LibraryError> {
        let id = id.to_owned();
        self.db.run(move |c| Ok(repo::folder(c, &id)?)).await
    }

    /// The watch folders with their counts; `new_since` is the earliest first-seen
    /// time that still counts a work as newly found.
    pub async fn summaries(&self, new_since: Millis) -> Result<Vec<FolderSummary>, LibraryError> {
        self.db
            .run(move |c| Ok(repo::summaries(c, new_since)?))
            .await
    }

    /// Registers the folder at `path` and records `scan`, its first reading, in
    /// one transaction. [`LibraryError::Duplicate`] when `path` is registered.
    pub async fn add_folder(
        &self,
        path: String,
        scan: Scan,
        now: Millis,
    ) -> Result<(WatchFolder, ScanReport), LibraryError> {
        self.db
            .run(move |c| repo::add_folder(c, &path, &scan, now))
            .await
    }

    /// Removes the folder and its works from the library (no file is touched)
    /// and returns how many works went. `None` when there is no such folder.
    pub async fn remove_folder(&self, id: &str) -> Result<Option<usize>, LibraryError> {
        let id = id.to_owned();
        self.db.run(move |c| Ok(repo::remove_folder(c, &id)?)).await
    }

    /// Records the outcome of reading folder `id` at `now`. `None` when the
    /// folder is not registered (any more).
    pub async fn record_scan(
        &self,
        id: &str,
        scan: Result<Scan, ScanError>,
        now: Millis,
    ) -> Result<Option<ScanReport>, LibraryError> {
        let id = id.to_owned();
        self.db
            .run(move |c| Ok(repo::record_scan(c, &id, &scan, now)?))
            .await
    }

    /// The work folder `name` moved from watch folder `from` to watch folder
    /// `to`: the work keeps its ID (see the module docs).
    pub async fn follow_move(
        &self,
        from: &str,
        to: &str,
        name: &str,
    ) -> Result<Followed, LibraryError> {
        let (from, to, name) = (from.to_owned(), to.to_owned(), name.to_owned());
        self.db
            .run(move |c| Ok(repo::follow_move(c, &from, &to, &name)?))
            .await
    }

    /// The works of a watch folder with everything recorded for them, by folder name.
    pub async fn works(&self, folder_id: &str) -> Result<Vec<WorkRecord>, LibraryError> {
        let id = folder_id.to_owned();
        self.db.run(move |c| Ok(repo::works(c, &id)?)).await
    }

    /// Every work of every watch folder with the summary the library list
    /// shows (see [`WorkOverview`]), by folder name, in a fixed number of queries.
    pub async fn overview(&self) -> Result<Vec<WorkOverview>, LibraryError> {
        self.db.run(|c| Ok(overview::overview(c)?)).await
    }

    /// One work with its seasons, episodes and files (see [`WorkDetail`]), or
    /// `None` when there is no work with this ID.
    pub async fn work_detail(&self, id: &str) -> Result<Option<WorkDetail>, LibraryError> {
        let id = id.to_owned();
        self.db.run(move |c| Ok(detail::detail(c, &id)?)).await
    }
}
