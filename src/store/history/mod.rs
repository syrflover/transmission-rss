//! Collection history: every RSS item the worker has seen, with what became of it.
//!
//! The worker records each item of each channel it reads, including the ones no
//! rule selected, and never deletes a record. This module also keeps the time
//! of the worker's last collection cycle, which the worker uses so that two
//! worker processes do not both run the same period's cycle.
//!
//! # Records
//!
//! One record per `(channel_id, identity_key)`; see [`identity_key`]. Seeing an
//! item again creates no record and keeps `first_seen_at`; it only refreshes
//! `last_seen_at`, the title and the link.
//!
//! # Results
//!
//! [`HistoryResult`] has the stable codes `received` (받음), `no_match`
//! (규칙 불일치), `excluded` (제외), `duplicate` (중복) and `add_failed`
//! (추가 실패). When an item is seen again with a different result, the change
//! is applied by the rules in [`Transition::between`]: `no_match`, `excluded`
//! and `add_failed` follow the newest evaluation, while `received` (and
//! `duplicate`, except that it may become `received`) is never undone. Each
//! applied change updates the record (`result`, `result_at`, rule, reason,
//! hash) and appends a row to the item's change trail ([`HistoryStore::changes`]),
//! so a record shows the current state and the trail shows how it got there.
//!
//! # Lifetime and secrets
//!
//! `channel_id` and `rule_id` are plain IDs without foreign keys, so records
//! outlive the channel and rules they refer to; `channel_label` keeps the
//! channel's masked URL for such records. Callers pass only masked URLs and
//! redacted failure reasons; this module stores what it is given.

mod identity;
mod model;
mod repo;
#[cfg(test)]
mod tests;

pub use identity::{identity_key, stored_link};
pub use model::{
    CycleState, HistoryChange, HistoryCursor, HistoryItem, HistoryPage, HistoryQuery,
    HistoryResult, Millis, Observation, Recorded, Transition, DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE,
};

use std::collections::HashSet;

use super::db::{Db, DbError};

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<rusqlite::Error> for HistoryError {
    fn from(e: rusqlite::Error) -> Self {
        HistoryError::Db(DbError::Sqlite(e))
    }
}

/// Async access to the collection history. Cheap to clone.
#[derive(Clone)]
pub struct HistoryStore {
    db: Db,
}

impl HistoryStore {
    pub fn new(db: Db) -> Self {
        HistoryStore { db }
    }

    /// Records sightings made at time `at`, all in one transaction. The
    /// returned list matches `observations`. A new item gets `first_seen_at =
    /// at`; a known one keeps its first-seen time.
    pub async fn record(
        &self,
        at: Millis,
        observations: Vec<Observation>,
    ) -> Result<Vec<Recorded>, HistoryError> {
        if observations.is_empty() {
            return Ok(Vec::new());
        }
        self.db
            .run(move |c| repo::record(c, at, &observations))
            .await
    }

    /// One page of history, newest first (by first-seen time, ties by record
    /// order), optionally limited to a result and a channel.
    pub async fn list(&self, query: HistoryQuery) -> Result<HistoryPage, HistoryError> {
        self.db.run(move |c| repo::list(c, &query)).await
    }

    /// One record by its ID.
    pub async fn get(&self, item_id: i64) -> Result<Option<HistoryItem>, HistoryError> {
        self.db.run(move |c| repo::get(c, item_id)).await
    }

    /// How many records have each result (results with none are left out),
    /// optionally within one channel.
    pub async fn counts(
        &self,
        channel_id: Option<String>,
    ) -> Result<Vec<(HistoryResult, i64)>, HistoryError> {
        self.db
            .run(move |c| repo::counts(c, channel_id.as_deref()))
            .await
    }

    /// The torrent hashes of the records among the given `(channel_id,
    /// identity_key)` pairs that Transmission holds a torrent for (`received`
    /// or `duplicate`): what a cycle keeps in Transmission for the items that
    /// are still in the feeds it read.
    pub async fn held_hashes_of_items(
        &self,
        items: Vec<(String, String)>,
    ) -> Result<HashSet<String>, HistoryError> {
        if items.is_empty() {
            return Ok(HashSet::new());
        }
        self.db
            .run(move |c| repo::held_hashes_of_items(c, &items))
            .await
            .map(|hashes| hashes.into_iter().collect())
    }

    /// Sets an item's result from something done to it outside a cycle (a
    /// command from the web), by the transition rules of [`Transition::between`],
    /// and returns the item's result afterwards (`None` for an unknown item).
    /// The item's `last_seen_at`, title and link are left alone, because it was
    /// not seen in a feed. `reason` must be free of secret values.
    pub async fn record_outcome(
        &self,
        item_id: i64,
        at: Millis,
        result: HistoryResult,
        reason: Option<String>,
        torrent_hash: Option<String>,
    ) -> Result<Option<HistoryResult>, HistoryError> {
        self.db
            .run(move |c| {
                repo::record_outcome(
                    c,
                    item_id,
                    at,
                    result,
                    reason.as_deref(),
                    torrent_hash.as_deref(),
                )
            })
            .await
    }

    /// Adds a note (for example, that a received file kept its original name)
    /// to a `received` item that has none. It does not change the result and
    /// leaves no entry in the item's changes. `note` must be free of secret
    /// values. Returns whether the note was written.
    pub async fn note_received(&self, item_id: i64, note: &str) -> Result<bool, HistoryError> {
        let note = note.to_owned();
        self.db
            .run(move |c| repo::note_received(c, item_id, &note))
            .await
    }

    /// Whether some item records the torrent `hash` as received by hand: a
    /// `received` item without a rule (see [`HistoryStore::record_outcome`]).
    /// The rule path leaves such a torrent's name and data alone.
    pub async fn received_by_hand(&self, hash: &str) -> Result<bool, HistoryError> {
        let hash = hash.to_owned();
        self.db.run(move |c| repo::received_by_hand(c, &hash)).await
    }

    /// The changes of an item's result, oldest first.
    pub async fn changes(&self, item_id: i64) -> Result<Vec<HistoryChange>, HistoryError> {
        self.db.run(move |c| repo::changes(c, item_id)).await
    }

    /// The torrent hashes recorded for items of the given channels (received or
    /// found already in Transmission), which tells where a torrent came from.
    pub async fn torrent_hashes_of_channels(
        &self,
        channel_ids: Vec<String>,
    ) -> Result<HashSet<String>, HistoryError> {
        self.db
            .run(move |c| repo::torrent_hashes_of_channels(c, &channel_ids))
            .await
            .map(|hashes| hashes.into_iter().collect())
    }

    /// Marks a collection cycle as started at `now`, unless the previous cycle
    /// started less than `min_gap` milliseconds ago; returns whether it did.
    pub async fn try_begin_cycle(
        &self,
        now: Millis,
        min_gap: Millis,
    ) -> Result<bool, HistoryError> {
        self.db
            .run(move |c| repo::try_begin_cycle(c, now, min_gap))
            .await
    }

    pub async fn finish_cycle(&self, now: Millis) -> Result<(), HistoryError> {
        self.db.run(move |c| repo::finish_cycle(c, now)).await
    }

    /// When the worker last started and finished a cycle.
    pub async fn last_cycle(&self) -> Result<Option<CycleState>, HistoryError> {
        self.db.run(|c| repo::last_cycle(c)).await
    }
}
