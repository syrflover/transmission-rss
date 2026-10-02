//! Season info (`docs/specs/library.md`, 시즌 정보): each local season is
//! linked to one or more AniList entries, and the season's airing, amount,
//! studios, genres, original title and synopsis are those entries'.
//!
//! - `trss_anilist::season`: asking AniList for an entry (through the artwork client, so
//!   with its request pace and `429` wait).
//! - [`combine`]: several entries as one season, the air dates of episodes and
//!   the sequels offered for a season.
//! - [`describe`]: AniList's description as plain text.
//! - [`queue`]: the worker's automatic work: linking a first season whose
//!   folder name names exactly one AniList entry, and the daily refresh of
//!   entries that are not finished.
//!
//! Saving a season's link also lets the work's cover follow it
//! ([`Seasons::follow_cover`]), whoever saves it: the user, or the worker's
//! automatic link of a first season.
//!
//! [`Seasons`] puts them together for both processes. The web carries out the
//! user's choices itself (search, link, unlink, order, ask again) because the
//! user waits for the answer and needs the reason when AniList refuses; the
//! worker runs the queue, outside the cycle lock. Both write the database
//! side by side safely: every change of a season's links is checked against the
//! link's version in its transaction, so a search that finishes late never
//! undoes a user's choice.

pub mod combine;
pub mod describe;
pub mod queue;

#[cfg(test)]
mod cover_tests;
#[cfg(test)]
mod tests;

use std::{sync::Arc, time::Duration};

use trss_core::{Clock, Db, Millis};

use crate::{
    artwork::{Artwork, USER_MAX_WAIT},
    store::{
        artwork::{ArtworkError, ArtworkStore},
        seasons::{SeasonError, SeasonLink, SeasonStore},
    },
};
use trss_anilist::{Anilist, AnilistError, Entry};

/// The most entries one season links.
pub const MAX_ENTRIES: usize = 8;

/// Why a user's season action did not happen. Nothing was changed.
#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    #[error(transparent)]
    Store(#[from] SeasonError),
    #[error(transparent)]
    Anilist(#[from] AnilistError),
    /// AniList has no anime entry with this ID.
    #[error("no such AniList entry: {0}")]
    NoEntry(i64),
}

fn clock_of(artwork: &Artwork) -> Clock {
    let artwork = artwork.clone();
    Arc::new(move || artwork.now())
}

/// The season services of one process. Cheap to clone.
#[derive(Clone)]
pub struct Seasons {
    pub store: SeasonStore,
    artwork: ArtworkStore,
    pub anilist: Anilist,
    clock: Clock,
}

impl Seasons {
    pub fn new(db: Db, anilist: Anilist, clock: Clock) -> Self {
        Seasons {
            artwork: ArtworkStore::new(db.clone()),
            store: SeasonStore::new(db),
            anilist,
            clock,
        }
    }

    /// The season services over `db` that ask AniList through `artwork`'s
    /// client (so both share one request pace) and read its clock.
    pub fn over(db: Db, artwork: &Artwork) -> Self {
        Seasons::new(db, artwork.anilist.clone(), clock_of(artwork))
    }

    /// The same services using `artwork`'s AniList client and clock instead.
    pub fn alongside(&self, artwork: &Artwork) -> Self {
        Seasons {
            store: self.store.clone(),
            artwork: artwork.store.clone(),
            anilist: artwork.anilist.clone(),
            clock: clock_of(artwork),
        }
    }

    pub fn now(&self) -> Millis {
        (self.clock)()
    }

    /// Lets the work's cover follow its seasons' links after one was saved: an
    /// `auto` cover is changed to the image of the earliest season's first
    /// entry, which the worker receives and verifies like any cover (the cover
    /// shown stays until then). The link is saved already, so a failure here is
    /// logged and never the link's.
    pub async fn follow_cover(&self, work_id: &str) {
        match self.artwork.follow_season_link(work_id, self.now()).await {
            Ok(_) | Err(ArtworkError::NotFound) => {}
            Err(e) => eprintln!("Cover of work {work_id} cannot follow its seasons: {e}"),
        }
    }

    /// Asks AniList for entry `id` and stores the answer.
    async fn receive(&self, id: i64, max_wait: Option<Duration>) -> Result<Entry, ActionError> {
        let entry = trss_anilist::season::fetch_entry(&self.anilist, id, max_wait, self.now())
            .await?
            .ok_or(ActionError::NoEntry(id))?;
        self.store.put_entry(entry.clone()).await?;
        Ok(entry)
    }

    /// Makes `ids` the season's entries in this order, as the user's choice
    /// made from the link's `expected` version. An entry already stored is
    /// used as it is; the others are asked of AniList first. Nothing changes
    /// when the version is not current, AniList does not answer, or an entry
    /// does not exist.
    pub async fn set_links(
        &self,
        work_id: &str,
        season: u32,
        expected: i64,
        ids: Vec<i64>,
    ) -> Result<SeasonLink, ActionError> {
        if ids.len() > MAX_ENTRIES {
            return Err(
                SeasonError::Invalid("한 시즌에는 항목을 여덟 개까지 이을 수 있어요.").into(),
            );
        }
        // A stale version is refused before any request is made.
        let current = self.store.link(work_id, season).await?;
        if current.version != expected {
            return Err(SeasonError::Conflict(Box::new(current)).into());
        }
        for id in &ids {
            if *id > 0 && self.store.entry(*id).await?.is_none() {
                self.receive(*id, Some(USER_MAX_WAIT)).await?;
            }
        }
        let link = self.store.set_links(work_id, season, expected, ids).await?;
        self.follow_cover(work_id).await;
        Ok(link)
    }

    /// Unlinks the first season and asks for a new automatic search.
    pub async fn restart_auto(
        &self,
        work_id: &str,
        season: u32,
        expected: i64,
    ) -> Result<SeasonLink, ActionError> {
        let link = self
            .store
            .restart_auto(work_id, season, expected, self.now())
            .await?;
        self.follow_cover(work_id).await;
        Ok(link)
    }

    /// Asks AniList again for every entry the season links, finished or not.
    /// The links stay as they are.
    pub async fn refresh_season(
        &self,
        work_id: &str,
        season: u32,
    ) -> Result<SeasonLink, ActionError> {
        let link = self.store.link(work_id, season).await?;
        for entry in &link.entries {
            self.receive(entry.id, Some(USER_MAX_WAIT)).await?;
        }
        Ok(self.store.link(work_id, season).await?)
    }
}
