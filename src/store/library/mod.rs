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
//! # Reading only some works
//!
//! The worker's inotify watches say which work folders changed, and
//! [`LibraryStore::record_works`] records the reading of just those: the rules
//! above hold for them (added times, files that are gone, a work folder that is
//! gone is marked missing), the other works are not looked at, and the folder's
//! own error and baseline are left to the readings of the whole folder.
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
//! Unregistering a watch folder takes its works out of the library and touches
//! no file, but forgets nothing: the folder's row stays, marked unregistered,
//! with its works and everything linked to them by ID (covers, season links).
//! An unregistered folder is not read or listed, and its works are in no list,
//! screen or queue. Registering the same path again (by hand or as the collect
//! or archive folder) brings the folder back under its ID, so its works are
//! found again by folder name with their IDs, and nothing new is searched for.
//!
//! # Automatic watch folders
//!
//! The collect folder and the archive folder of the collection settings are
//! always watch folders, so that a work an archive move takes from one to the
//! other stays in the library. Their watch folders are *automatic*
//! ([`WatchFolder::automatic`]): registered by the app, not removable by hand.
//! An [`AutomaticPlan`] says how the registered folders become the ones the
//! settings call for: a folder the user had registered at the same place turns
//! automatic and keeps its records, a new one is registered (with its first
//! scan when the caller has one), and an automatic folder at a path the
//! settings no longer use is unregistered like one the user unregisters. A plan is made
//! from the folders as they were read, and applying it fails with
//! [`LibraryError::Changed`] if they are not the registered folders any more.

mod detail;
mod overview;
mod page;
mod repo;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

pub use detail::{EpisodeDetail, SeasonDetail, WorkDetail};
use rusqlite::Transaction;

pub use overview::{EpisodeRange, SubtitleCoverage, WorkOverview};
pub use page::{Cursor, Filter, ListQuery, Page, Sort};

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
    /// The folder is the collect or archive folder, which stays a watch folder
    /// while the settings use it.
    #[error("the watch folder is the collect or archive folder")]
    Automatic,
    /// The registered watch folders are not the ones the caller checked
    /// against; nothing was changed. Read them again and check again.
    #[error("the watch folders changed while they were being checked")]
    Changed,
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
    /// Registered by the app for the collect or archive folder: it cannot be
    /// unregistered by hand.
    pub automatic: bool,
    /// Whether a scan has read the folder without error yet; until then what
    /// scans find has no added time.
    pub baselined: bool,
    /// The last attempt to read the folder, successful or not.
    pub checked_at: Option<Millis>,
    /// A sentence while the last attempt could not read everything.
    pub error: Option<String>,
    /// A sentence while the worker could not watch every directory of the
    /// folder for changes (how many and why); the worker then checks those
    /// directories itself every cycle. `None` when none is missing, or when the
    /// worker does not watch the folder (see [`crate::worker::live`]).
    pub watch_note: Option<String>,
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

/// A watch folder to register for a folder of the collection settings.
#[derive(Debug, Clone)]
pub struct NewAutomatic {
    pub path: String,
    /// The folder's first reading. Without one the folder is registered
    /// unread, and the worker's next cycle makes the first reading (which then
    /// baselines it, as a manual add's first scan does).
    pub scan: Option<Scan>,
}

/// How the watch folders become the ones the collection settings call for;
/// made by [`crate::automatic_watch::plan`].
#[derive(Debug, Clone, Default)]
pub struct AutomaticPlan {
    /// The registered folders the plan was made from (id, path, automatic).
    pub(crate) based_on: Vec<(String, String, bool)>,
    /// Folders that become automatic, with the path the settings give them.
    pub keep: Vec<(String, String)>,
    /// Automatic folders whose path the settings no longer use: unregistered.
    pub remove: Vec<String>,
    pub add: Vec<NewAutomatic>,
}

impl AutomaticPlan {
    /// An empty plan over `folders`.
    pub fn over(folders: &[WatchFolder]) -> AutomaticPlan {
        AutomaticPlan {
            based_on: folders
                .iter()
                .map(|f| (f.id.clone(), f.path.clone(), f.automatic))
                .collect(),
            ..AutomaticPlan::default()
        }
    }

    /// Whether applying the plan changes nothing.
    pub fn is_empty(&self) -> bool {
        self.keep.is_empty() && self.remove.is_empty() && self.add.is_empty()
    }
}

/// What applying an [`AutomaticPlan`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutomaticApplied {
    /// Folders registered.
    pub added: usize,
    /// Folders that were registered by hand and became automatic, or whose
    /// path text changed.
    pub converted: usize,
    /// Folders unregistered, and the works that left the library with them.
    pub removed: usize,
    pub removed_works: usize,
}

/// Applies `plan` inside `tx`, which the caller commits. Fails with
/// [`LibraryError::Changed`] if the registered folders differ from the ones the
/// plan was made from.
pub fn apply_automatic_in(
    tx: &Transaction<'_>,
    plan: &AutomaticPlan,
    now: Millis,
) -> Result<AutomaticApplied, LibraryError> {
    repo::apply_automatic(tx, plan, now)
}

/// Sets the automatic watch folder of `path` inside `tx`, which the caller
/// commits, when nothing has the settings' folders to compare with yet (the
/// legacy import, which sets the collect folder on a database that has none).
/// A folder registered at the same path turns automatic; otherwise one is
/// registered unread.
pub fn ensure_automatic_in(
    tx: &Transaction<'_>,
    path: &str,
    now: Millis,
) -> Result<(), LibraryError> {
    Ok(repo::ensure_automatic(tx, path, now)?)
}

/// What [`LibraryStore::follow_move`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followed {
    /// No work was recorded for the moved folder.
    NotTracked,
    /// The work now belongs to the destination folder, same ID.
    Moved,
    /// The destination had a work of that name; the moved work's records were
    /// merged into it and its row is gone. The moved work's cover choice and
    /// season links were carried over where the kept work had none (see
    /// [`crate::store::artwork::merge_selection`] and
    /// [`crate::store::seasons::merge_links`]).
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
    ///
    /// `checked_against` is the registered folders the caller checked that the
    /// new folder does not overlap; if they are not the registered folders when
    /// the transaction starts, nothing is added and the answer is
    /// [`LibraryError::Changed`].
    pub async fn add_folder(
        &self,
        path: String,
        scan: Scan,
        now: Millis,
        checked_against: &[WatchFolder],
    ) -> Result<(WatchFolder, ScanReport), LibraryError> {
        let checked = AutomaticPlan::over(checked_against).based_on;
        self.db
            .run(move |c| repo::add_folder(c, &path, &scan, now, &checked))
            .await
    }

    /// Unregisters the folder at `now`: its works leave the library, kept with
    /// everything linked to them for the folder's return (see the module docs;
    /// no file is touched). Returns how many works left. `None` when no such
    /// folder is registered; [`LibraryError::Automatic`] for the collect or
    /// archive folder.
    pub async fn remove_folder(
        &self,
        id: &str,
        now: Millis,
    ) -> Result<Option<usize>, LibraryError> {
        let id = id.to_owned();
        self.db.run(move |c| repo::remove_folder(c, &id, now)).await
    }

    /// Applies `plan` in one transaction, provided the collection settings are
    /// still at `settings_version` (0 when none are stored): the plan was made
    /// from the settings of that version, and a newer save has its own plan.
    /// [`LibraryError::Changed`] when they are not, or when the watch folders
    /// are not the ones the plan was made from.
    pub async fn sync_automatic(
        &self,
        plan: AutomaticPlan,
        settings_version: i64,
        now: Millis,
    ) -> Result<AutomaticApplied, LibraryError> {
        self.db
            .run(move |c| repo::sync_automatic(c, &plan, settings_version, now))
            .await
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

    /// Records the reading of only the work folders called `names` of folder
    /// `id` (see [`crate::discovery::scan_works`]): the works of the scan are
    /// brought up to date, and a work of one of those names that the scan does
    /// not have is marked missing. The other works, and the folder's error
    /// unless one of these work folders could not be read, stay as they were.
    /// `None` when the folder is not registered (any more).
    pub async fn record_works(
        &self,
        id: &str,
        names: Vec<String>,
        scan: Scan,
        now: Millis,
    ) -> Result<Option<ScanReport>, LibraryError> {
        let id = id.to_owned();
        self.db
            .run(move |c| Ok(repo::record_works(c, &id, &names, &scan, now)?))
            .await
    }

    /// Sets the sentence on folder `id`'s row about directories the worker
    /// could not watch (see [`WatchFolder::watch_note`]); `None` clears it.
    pub async fn set_watch_note(&self, id: &str, note: Option<String>) -> Result<(), LibraryError> {
        let id = id.to_owned();
        self.db
            .run(move |c| Ok(repo::set_watch_note(c, &id, note.as_deref())?))
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

    /// One page of the library list: the works that pass the query's filter and
    /// search, in its sort's order, after its cursor (see [`Page`]).
    pub async fn list(&self, query: ListQuery) -> Result<Page, LibraryError> {
        self.db
            .run(move |c| Ok(page::page(overview::overview(c)?, &query)))
            .await
    }

    /// One work with its seasons, episodes and files (see [`WorkDetail`]), or
    /// `None` when there is no work with this ID.
    pub async fn work_detail(&self, id: &str) -> Result<Option<WorkDetail>, LibraryError> {
        let id = id.to_owned();
        self.db.run(move |c| Ok(detail::detail(c, &id)?)).await
    }
}
