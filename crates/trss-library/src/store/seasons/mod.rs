//! Season info: which AniList entries each local season is linked to, and the
//! entries as AniList described them (`docs/specs/library.md`, 시즌 정보). The
//! rules that look entries up, link them and turn them into what the screen
//! shows live in [`crate::seasons`]; this store keeps the state and its
//! invariants.
//!
//! # The link and its version
//!
//! A local season has an ordered list of AniList entries ([`SeasonLink`]). The
//! link has a `version` (0 while nothing was ever recorded for the season) that
//! grows with every change of the list. A user's change names the version it
//! was made from and fails with [`SeasonError::Conflict`], changing nothing,
//! when the link moved on first; the automatic link names the version its job
//! was taken at and is dropped the same way, so a late search never undoes a
//! user's newer choice. A refresh of an entry's data is no change of the link.
//!
//! # The Anissia link
//!
//! A season also links one Anissia anime, kept apart from the AniList entries
//! with a version of its own ([`anissia`]).
//!
//! # Jobs
//!
//! The one automatic job is [`ClaimedSearch`]: look the work's folder name up
//! on AniList and link the entry whose title it equals exactly. It is set for
//! a first season (the lowest-numbered season that is not season 0) the moment
//! the app records it, by a trigger on `seasons`, and for a user's request to
//! find the entry again. A rescan, a restart or opening a screen sets none, and
//! nothing is set for what is recorded when the migration runs.
//!
//! # Entries
//!
//! An entry that is not yet released or is releasing is refreshed once a day
//! while a season links it ([`SeasonStore::next_refresh`]); a finished one only
//! when the user asks.

pub mod anissia;
#[cfg(test)]
mod anissia_tests;
mod repo;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

pub use repo::LinkedFact;

use trss_anilist::Entry;
use trss_core::{
    db::{Db, DbError},
    Millis,
};

/// Who made the current links of a season.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The app linked the entry itself, or nothing is linked yet.
    Auto,
    /// The user chose (and a user who unlinked everything is a user).
    User,
}

impl Origin {
    pub fn code(self) -> &'static str {
        match self {
            Origin::Auto => "auto",
            Origin::User => "user",
        }
    }

    pub fn from_code(code: &str) -> Option<Origin> {
        match code {
            "auto" => Some(Origin::Auto),
            "user" => Some(Origin::User),
            _ => None,
        }
    }
}

/// Why the last automatic search linked nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    /// No AniList title equals the folder name.
    NoMatch,
    /// More than one AniList entry has a title equal to the folder name.
    Ambiguous,
    /// The search had more results than the app reads.
    Incomplete,
    /// AniList could not be reached or answered something unusable, again and
    /// again.
    Failed,
}

impl Note {
    pub fn code(self) -> &'static str {
        match self {
            Note::NoMatch => "no_match",
            Note::Ambiguous => "ambiguous",
            Note::Incomplete => "incomplete",
            Note::Failed => "failed",
        }
    }

    fn from_code(code: &str) -> Option<Note> {
        [
            Note::NoMatch,
            Note::Ambiguous,
            Note::Incomplete,
            Note::Failed,
        ]
        .into_iter()
        .find(|n| n.code() == code)
    }

    pub fn message(self) -> &'static str {
        match self {
            Note::NoMatch => {
                "AniList에서 폴더 이름과 제목이 같은 작품을 찾지 못해 잇지 않았어요. 직접 찾아 이어 주세요."
            }
            Note::Ambiguous => {
                "AniList에 폴더 이름과 제목이 같은 작품이 여럿이라 잇지 않았어요. 직접 골라 주세요."
            }
            Note::Incomplete => {
                "AniList 검색 결과가 너무 많아 하나뿐인지 확인하지 못해 잇지 않았어요. 직접 골라 주세요."
            }
            Note::Failed => "AniList에 여러 번 연결하지 못해 시즌 정보를 잇지 못했어요.",
        }
    }
}

/// The automatic search still to do for a season.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub requested_at: Millis,
    /// Failed attempts so far.
    pub attempts: u32,
    pub not_before: Option<Millis>,
}

/// A local season's link to AniList entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeasonLink {
    pub work_id: String,
    pub season: u32,
    /// 0 while nothing was ever recorded for the season.
    pub version: i64,
    pub origin: Origin,
    /// The linked entries in order.
    pub entries: Vec<Entry>,
    pub job: Option<Job>,
    pub note: Option<Note>,
}

/// A search job the worker took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedSearch {
    pub work_id: String,
    pub season: u32,
    /// The work's folder name: the title observation the search compares.
    pub dir_name: String,
    /// The link's version when the job was taken.
    pub version: i64,
    pub attempts: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum SeasonError {
    #[error(transparent)]
    Db(#[from] DbError),
    /// The work has no such season recorded.
    #[error("no such season")]
    NotFound,
    /// The link is not at the version the change was made from; nothing was
    /// changed. Carries the current link.
    #[error("the season's link was changed by someone else")]
    Conflict(Box<SeasonLink>),
    /// The change does not apply to the season as it is.
    #[error("{0}")]
    Invalid(&'static str),
    /// An entry to link has no row (it was not received first).
    #[error("entry {0} is not known")]
    MissingEntry(i64),
}

impl From<rusqlite::Error> for SeasonError {
    fn from(e: rusqlite::Error) -> Self {
        SeasonError::Db(DbError::Sqlite(e))
    }
}

/// Async access to the season links and the entries. Cheap to clone.
#[derive(Clone)]
pub struct SeasonStore {
    db: Db,
}

impl SeasonStore {
    pub fn new(db: Db) -> Self {
        SeasonStore { db }
    }

    /// The link of a season, with its entries; a season never touched has
    /// version 0 and no entries. [`SeasonError::NotFound`] when the work has no
    /// such season recorded.
    pub async fn link(&self, work_id: &str, season: u32) -> Result<SeasonLink, SeasonError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| repo::link_of_recorded(c, &id, season))
            .await
    }

    /// [`SeasonStore::link`] for several seasons at once, in the order given;
    /// `None` for a season the work has no record of.
    pub async fn links_of_seasons(
        &self,
        seasons: Vec<(String, u32)>,
    ) -> Result<Vec<Option<SeasonLink>>, SeasonError> {
        self.db
            .run(move |c| {
                seasons
                    .iter()
                    .map(
                        |(id, season)| match repo::link_of_recorded(c, id, *season) {
                            Ok(link) => Ok(Some(link)),
                            Err(SeasonError::NotFound) => Ok(None),
                            Err(e) => Err(e),
                        },
                    )
                    .collect()
            })
            .await
    }

    /// The links of every season of a work that has one, by season number.
    pub async fn links_of(&self, work_id: &str) -> Result<BTreeMap<u32, SeasonLink>, SeasonError> {
        let id = work_id.to_owned();
        self.db.run(move |c| Ok(repo::links_of(c, &id)?)).await
    }

    /// The lowest-numbered recorded season that is not season 0 (specials) of
    /// the work: the season the automatic search is for.
    pub async fn first_season(&self, work_id: &str) -> Result<Option<u32>, SeasonError> {
        let id = work_id.to_owned();
        self.db.run(move |c| Ok(repo::first_season(c, &id)?)).await
    }

    pub async fn entry(&self, id: i64) -> Result<Option<Entry>, SeasonError> {
        self.db.run(move |c| Ok(repo::entry(c, id)?)).await
    }

    /// Stores what AniList answered for an entry, replacing the earlier copy.
    pub async fn put_entry(&self, entry: Entry) -> Result<(), SeasonError> {
        self.db.run(move |c| Ok(repo::put_entry(c, &entry)?)).await
    }

    /// Makes `ids` the season's entries, in order, as the user's choice made
    /// from `expected`. Every entry must have been stored with
    /// [`SeasonStore::put_entry`] first.
    pub async fn set_links(
        &self,
        work_id: &str,
        season: u32,
        expected: i64,
        ids: Vec<i64>,
    ) -> Result<SeasonLink, SeasonError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| repo::set_links(c, &id, season, expected, &ids))
            .await
    }

    /// Unlinks the season and asks for a new automatic search, as the user's
    /// request made from `expected`. Only the work's first season has one.
    pub async fn restart_auto(
        &self,
        work_id: &str,
        season: u32,
        expected: i64,
        now: Millis,
    ) -> Result<SeasonLink, SeasonError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| repo::restart_auto(c, &id, season, expected, now))
            .await
    }

    /// The next search that may run at `now`, oldest request first. A search
    /// for a season that is not the work's first any more is dropped.
    pub async fn next_search(&self, now: Millis) -> Result<Option<ClaimedSearch>, SeasonError> {
        self.db.run(move |c| Ok(repo::next_search(c, now)?)).await
    }

    /// Links `entry_id` (stored already) as the result of the search taken at
    /// `version`, unless the link changed since. Whether it was recorded.
    pub async fn auto_linked(
        &self,
        work_id: &str,
        season: u32,
        version: i64,
        entry_id: i64,
    ) -> Result<bool, SeasonError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| repo::auto_linked(c, &id, season, version, entry_id))
            .await
    }

    /// The search taken at `version` did not finish: try again at `retry_at`
    /// (counting a failed attempt when `failed`), or, with `retry_at` `None`,
    /// give up and leave `note`. Whether it was recorded.
    pub async fn search_later(
        &self,
        work_id: &str,
        season: u32,
        version: i64,
        retry_at: Option<Millis>,
        failed: bool,
        note: Note,
    ) -> Result<bool, SeasonError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| {
                Ok(repo::search_later(
                    c, &id, season, version, retry_at, failed, note,
                )?)
            })
            .await
    }

    /// The entry linked by a season whose daily refresh is due at `now`
    /// (active, last received a day ago or more), oldest first.
    pub async fn next_refresh(&self, now: Millis) -> Result<Option<i64>, SeasonError> {
        self.db.run(move |c| Ok(repo::next_refresh(c, now)?)).await
    }

    /// A refresh did not finish: the next try is not before `retry_at`.
    pub async fn refresh_later(&self, id: i64, retry_at: Millis) -> Result<(), SeasonError> {
        self.db
            .run(move |c| Ok(repo::refresh_later(c, id, retry_at)?))
            .await
    }

    /// AniList has no such entry any more: what is kept stays, and the next
    /// try waits a day.
    pub async fn refresh_gone(&self, id: i64, now: Millis) -> Result<(), SeasonError> {
        self.db
            .run(move |c| Ok(repo::refresh_gone(c, id, now)?))
            .await
    }
}

pub(crate) use anissia::{follow_subscriptions, merge_links as merge_anissia_links};
pub(crate) use repo::{cover_target, facts as library_facts, merge_links};
