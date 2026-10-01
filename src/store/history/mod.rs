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
//! [`HistoryResult`] has the stable codes `received` (추가함), `no_match`
//! (규칙 불일치), `excluded` (제외), `duplicate` (중복), `add_failed`
//! (추가 실패) and `version_unknown` (버전 미상: a higher revision of a video
//! the folder holds that the worker did not receive on its own, see
//! [`crate::worker::revisions`]). When an item is seen again with a different
//! result, the change is applied by the rules in [`Transition::between`]:
//! `no_match`, `excluded`, `add_failed` and `version_unknown` follow the newest
//! evaluation, while `received` (and
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
/// Shared with `store::revisions`, which writes a revision row in the same
/// transaction as its history item.
pub(in crate::store) mod repo;
mod rule_items;
#[cfg(test)]
mod tests;

pub use identity::{identity_key, stored_link};
pub use model::{
    CycleState, HistoryChange, HistoryCursor, HistoryItem, HistoryPage, HistoryQuery,
    HistoryResult, KnownItem, Millis, Observation, Recorded, Transition, DEFAULT_PAGE_SIZE,
    MAX_PAGE_SIZE,
};

use std::collections::HashSet;

use self::repo::Origin;
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
        self.record_from(at, Origin::Feed, observations).await
    }

    /// [`HistoryStore::record`] for sightings made somewhere other than the
    /// channel's feed (the past search's tracker read). They never establish
    /// the channel's first read: the first cycle that reads the feed still
    /// finds the channel unread and keeps its subscriptions out of what the
    /// feed already holds.
    pub async fn record_elsewhere(
        &self,
        at: Millis,
        observations: Vec<Observation>,
    ) -> Result<Vec<Recorded>, HistoryError> {
        self.record_from(at, Origin::Elsewhere, observations).await
    }

    async fn record_from(
        &self,
        at: Millis,
        origin: Origin,
        observations: Vec<Observation>,
    ) -> Result<Vec<Recorded>, HistoryError> {
        if observations.is_empty() {
            return Ok(Vec::new());
        }
        self.db
            .run(move |c| repo::record(c, at, origin, &observations))
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

    /// The record of the item `identity_key` of the channel `channel_id`.
    pub async fn item_by_key(
        &self,
        channel_id: String,
        identity_key: String,
    ) -> Result<Option<HistoryItem>, HistoryError> {
        self.db
            .run(move |c| repo::item_by_key(c, &channel_id, &identity_key))
            .await
    }

    /// The records of the torrent `hash`, newest first: what tells whether trss
    /// added a torrent (a `received` record) and which release it was.
    pub async fn items_of_hash(&self, hash: &str) -> Result<Vec<HistoryItem>, HistoryError> {
        let hash = hash.to_owned();
        self.db.run(move |c| repo::items_of_hash(c, &hash)).await
    }

    /// The ID and title of every record of the channel `channel_id`: the
    /// releases a video of unknown revision is compared with.
    pub async fn titles_of_channel(
        &self,
        channel_id: String,
    ) -> Result<Vec<(i64, String)>, HistoryError> {
        self.db
            .run(move |c| repo::titles_of_channel(c, &channel_id))
            .await
    }

    /// The ID and title of the `limit` newest records of the channel
    /// `channel_id`, newest first: [`Self::titles_of_channel`] for a channel
    /// whose history may be long.
    pub async fn recent_titles_of_channel(
        &self,
        channel_id: String,
        limit: usize,
    ) -> Result<Vec<(i64, String)>, HistoryError> {
        self.db
            .run(move |c| repo::recent_titles_of_channel(c, &channel_id, limit))
            .await
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

    /// When each of the given items of a channel was first seen, what became of
    /// it and whether the channel's first read recorded it, by identity key:
    /// what a cycle needs to tell the items a rule has not seen yet from the
    /// ones already in history.
    pub async fn known_items(
        &self,
        channel_id: String,
        keys: Vec<String>,
    ) -> Result<std::collections::HashMap<String, KnownItem>, HistoryError> {
        if keys.is_empty() {
            return Ok(Default::default());
        }
        self.db
            .run(move |c| repo::known_items(c, &channel_id, &keys))
            .await
    }

    /// When each of the given channels was first read, by channel ID; a channel
    /// with no record is left out. A channel's first record is its first read,
    /// whose time is stored when it is written and never moves afterwards, not
    /// even when a later record carries an earlier time (the clock went back).
    /// A channel without one is read for the first time by the next cycle
    /// ([`crate::worker::plan::ChannelPlan::for_first_read`]). Which items the
    /// feed already held then is told by the items themselves
    /// ([`HistoryItem::first_read`]), not by comparing times with this one.
    pub async fn first_sightings(
        &self,
        channel_ids: Vec<String>,
    ) -> Result<std::collections::HashMap<String, Millis>, HistoryError> {
        if channel_ids.is_empty() {
            return Ok(Default::default());
        }
        self.db
            .run(move |c| repo::first_sightings(c, &channel_ids))
            .await
    }

    /// When each of the given rules last got an item into Transmission, by
    /// rule ID; a rule that received nothing is left out. What an archive
    /// suggestion's weeks of "no new item" count from.
    pub async fn last_received_of_rules(
        &self,
        rule_ids: Vec<String>,
    ) -> Result<std::collections::HashMap<String, Millis>, HistoryError> {
        if rule_ids.is_empty() {
            return Ok(Default::default());
        }
        self.db
            .run(move |c| repo::last_received_of_rules(c, &rule_ids))
            .await
    }

    /// The titles of the channel's items first seen after `since`, newest
    /// first, at most `limit`, and whether the window held more than that.
    pub async fn titles_since(
        &self,
        channel_id: String,
        since: Millis,
        limit: usize,
    ) -> Result<(Vec<String>, bool), HistoryError> {
        self.db
            .run(move |c| repo::titles_since(c, &channel_id, since, limit))
            .await
    }

    /// Sets an item's result from something done to it outside a cycle (a
    /// command from the web), by the transition rules of [`Transition::between`],
    /// and returns the item's result afterwards (`None` for an unknown item).
    /// The item's `last_seen_at`, title and link are left alone, because it was
    /// not seen in a feed. `rule_id` is the rule that picked the item, which a
    /// retry for that rule keeps (a receive with no rule passes `None`).
    /// `reason` must be free of secret values.
    pub async fn record_outcome(
        &self,
        item_id: i64,
        at: Millis,
        result: HistoryResult,
        rule_id: Option<String>,
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
                    rule_id.as_deref(),
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

    /// The torrent hashes of the items each of the given rules received
    /// (`received` with the rule recorded on the item), by rule ID: what shows
    /// which videos a rule brought in. A rule that received nothing is absent.
    pub async fn received_hashes_of_rules(
        &self,
        rule_ids: Vec<String>,
    ) -> Result<std::collections::HashMap<String, Vec<String>>, HistoryError> {
        self.db
            .run(move |c| repo::received_hashes_of_rules(c, &rule_ids))
            .await
    }

    /// The torrent hash and release title of the items each of the given rules
    /// received whose torrent is one of `hashes`, by rule ID: what tells which
    /// episode a torrent is of.
    pub async fn received_titles_of_rules(
        &self,
        rule_ids: Vec<String>,
        hashes: Vec<String>,
    ) -> Result<std::collections::HashMap<String, Vec<(String, String)>>, HistoryError> {
        self.db
            .run(move |c| repo::received_titles_of_rules(c, &rule_ids, &hashes))
            .await
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
