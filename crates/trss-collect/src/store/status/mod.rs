//! What the worker last saw of the outside world, for the status board.
//!
//! The web does not read RSS feeds and does not call Transmission. Instead the
//! worker leaves a small snapshot in the database at each cycle:
//!
//! - per channel, whether the feed could be read ([`ChannelRead`]);
//! - Transmission's downloading and seeding torrent counts ([`TransmissionCounts`])
//!   and the hashes of the torrents that were downloading;
//! - the hashes of every torrent it held ([`TorrentListing`]), which the past
//!   episode search needs to tell a removed torrent from one that is still there;
//! - per channel, the days on which a read of the feed worked
//!   ([`StatusStore::read_day_floors`]), which the archive suggestions count the
//!   weeks without a new item in;
//! - the worker's cycle interval, which the web cannot read from its own
//!   environment, so it can tell when the next check is due;
//!
//! A snapshot is a fact about the time it was taken, so it carries that time
//! and the screen shows it; a failed look at Transmission leaves the older
//! counts (and their time) in place instead of writing zeros.
//!
//! This module also answers the two questions the board asks of the collection
//! history ([`StatusStore::received_since`], [`StatusStore::problems_since`]);
//! it only reads the history table.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use trss_core::{
    db::{Db, DbError},
    Millis,
};

#[cfg(test)]
mod tests;

/// How many of a channel's newest read days are kept: the 28 days of the
/// `새 항목 없음` ground of the archive suggestions.
pub const READ_DAYS_KEPT: usize = 28;

const DAY_MS: Millis = 24 * 60 * 60 * 1000;

/// The day a moment belongs to: Unix milliseconds divided by a day. Read days
/// are told apart by this and nothing more, so the time zone does not matter.
pub fn read_day(at: Millis) -> i64 {
    at.div_euclid(DAY_MS)
}

#[derive(Debug, thiserror::Error)]
pub enum StatusError {
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<rusqlite::Error> for StatusError {
    fn from(e: rusqlite::Error) -> Self {
        StatusError::Db(DbError::Sqlite(e))
    }
}

/// What one cycle found reading one channel's feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelReadResult {
    pub channel_id: String,
    pub ok: bool,
}

/// The stored read status of a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRead {
    pub channel_id: String,
    /// Whether the most recent attempt worked.
    pub ok: bool,
    /// When the most recent attempt was made.
    pub read_at: Millis,
    /// When a read last worked; `None` if none has.
    pub ok_at: Option<Millis>,
}

/// Torrent counts from the worker's last successful look at Transmission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransmissionCounts {
    pub downloading: u32,
    pub seeding: u32,
    pub taken_at: Millis,
}

/// Every torrent Transmission held when the worker last looked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentListing {
    /// When the worker took the list; a torrent added after it is not in it.
    pub taken_at: Millis,
    /// The torrents' hashes, lowercase.
    pub hashes: HashSet<String>,
}

impl TorrentListing {
    /// Whether the torrent `hash` was in Transmission when the list was taken.
    pub fn holds(&self, hash: &str) -> bool {
        self.hashes.contains(&hash.to_ascii_lowercase())
    }
}

/// Async access to the worker's snapshots. Cheap to clone.
#[derive(Clone)]
pub struct StatusStore {
    db: Db,
}

impl StatusStore {
    pub fn new(db: Db) -> Self {
        StatusStore { db }
    }

    /// Records the feed reads of a cycle that ran at `at`, in one transaction.
    /// A channel keeps its last success time when this read failed. Rows of
    /// channels not in `existing` (channels deleted since) are removed; a
    /// channel that exists but was not read this time keeps its row.
    pub async fn record_reads(
        &self,
        at: Millis,
        reads: Vec<ChannelReadResult>,
        existing: Vec<String>,
    ) -> Result<(), StatusError> {
        self.db
            .run(move |c| record_reads(c, at, &reads, &existing))
            .await
    }

    /// For each of the given channels with at least [`READ_DAYS_KEPT`] days up
    /// to `now` on which a read of its feed worked, the [`read_day`] of the
    /// oldest of its newest [`READ_DAYS_KEPT`]; a channel with fewer is left
    /// out. Reading the feed on every one of those days is what a quiet stretch
    /// of that many days takes. Days after `now` (written by a clock that was
    /// ahead) do not count. One lookup of the table's key per channel.
    pub async fn read_day_floors(
        &self,
        channel_ids: Vec<String>,
        now: Millis,
    ) -> Result<std::collections::HashMap<String, i64>, StatusError> {
        self.db
            .run(move |c| read_day_floors(c, &channel_ids, now))
            .await
    }

    /// The read status of every channel the worker has tried.
    pub async fn channel_reads(&self) -> Result<Vec<ChannelRead>, StatusError> {
        self.db.run(|c| channel_reads(c)).await
    }

    /// Replaces the Transmission counts and the hashes of the torrents that
    /// were downloading (or queued to download) when they were taken.
    pub async fn record_transmission(
        &self,
        counts: TransmissionCounts,
        downloading: Vec<String>,
    ) -> Result<(), StatusError> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.prepare_cached(
                    "INSERT INTO transmission_snapshot (id, downloading, seeding, taken_at)
                     VALUES (1, ?1, ?2, ?3)
                     ON CONFLICT (id) DO UPDATE SET
                         downloading = excluded.downloading,
                         seeding = excluded.seeding,
                         taken_at = excluded.taken_at",
                )?
                .execute(params![
                    counts.downloading,
                    counts.seeding,
                    counts.taken_at
                ])?;
                tx.prepare_cached("DELETE FROM transmission_downloading")?
                    .execute([])?;
                let mut insert = tx.prepare_cached(
                    "INSERT OR IGNORE INTO transmission_downloading (hash) VALUES (?1)",
                )?;
                for hash in downloading.iter().filter(|h| !h.is_empty()) {
                    insert.execute([hash])?;
                }
                drop(insert);
                tx.commit()?;
                Ok::<_, StatusError>(())
            })
            .await
    }

    /// Replaces the list of every torrent in Transmission, taken at `at`.
    pub async fn record_listing(&self, at: Millis, hashes: Vec<String>) -> Result<(), StatusError> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.prepare_cached(
                    "INSERT INTO transmission_listing (id, taken_at) VALUES (1, ?1)
                     ON CONFLICT (id) DO UPDATE SET taken_at = excluded.taken_at",
                )?
                .execute([at])?;
                tx.prepare_cached("DELETE FROM transmission_torrents")?
                    .execute([])?;
                let mut insert = tx.prepare_cached(
                    "INSERT OR IGNORE INTO transmission_torrents (hash) VALUES (?1)",
                )?;
                for hash in hashes.iter().filter(|h| !h.is_empty()) {
                    insert.execute([hash.to_ascii_lowercase()])?;
                }
                drop(insert);
                tx.commit()?;
                Ok::<_, StatusError>(())
            })
            .await
    }

    /// The list of every torrent in Transmission the worker last wrote, or
    /// `None` while it has written none.
    pub async fn torrent_listing(&self) -> Result<Option<TorrentListing>, StatusError> {
        self.db
            .run(|c| {
                let Some(taken_at) = c
                    .prepare_cached("SELECT taken_at FROM transmission_listing WHERE id = 1")?
                    .query_row([], |r| r.get(0))
                    .optional()?
                else {
                    return Ok::<_, StatusError>(None);
                };
                let mut stmt = c.prepare_cached("SELECT hash FROM transmission_torrents")?;
                let hashes = stmt
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<HashSet<String>>>()?;
                Ok(Some(TorrentListing { taken_at, hashes }))
            })
            .await
    }

    /// The hashes of the torrents Transmission was downloading when the worker
    /// last looked.
    pub async fn downloading_hashes(&self) -> Result<HashSet<String>, StatusError> {
        self.db
            .run(|c| {
                let mut stmt = c.prepare_cached("SELECT hash FROM transmission_downloading")?;
                let hashes = stmt
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<HashSet<String>>>()?;
                Ok::<_, StatusError>(hashes)
            })
            .await
    }

    /// Records the time between two collection cycles of this worker.
    pub async fn record_cycle_interval(&self, interval_ms: i64) -> Result<(), StatusError> {
        self.db
            .run(move |c| {
                c.execute(
                    "INSERT INTO worker_info (id, cycle_interval_ms) VALUES (1, ?1)
                     ON CONFLICT (id) DO UPDATE SET cycle_interval_ms = excluded.cycle_interval_ms",
                    [interval_ms.max(1)],
                )?;
                Ok::<_, StatusError>(())
            })
            .await
    }

    /// The time between two collection cycles the worker last recorded, in
    /// milliseconds; `None` while no worker has started.
    pub async fn cycle_interval(&self) -> Result<Option<i64>, StatusError> {
        self.db
            .run(|c| {
                Ok::<_, StatusError>(
                    c.prepare_cached("SELECT cycle_interval_ms FROM worker_info WHERE id = 1")?
                        .query_row([], |r| r.get(0))
                        .optional()?,
                )
            })
            .await
    }

    /// The counts from the last successful look at Transmission, if any.
    pub async fn transmission(&self) -> Result<Option<TransmissionCounts>, StatusError> {
        self.db
            .run(|c| {
                Ok::<_, StatusError>(
                    c.prepare_cached(
                        "SELECT downloading, seeding, taken_at
                         FROM transmission_snapshot WHERE id = 1",
                    )?
                    .query_row([], |r| {
                        Ok(TransmissionCounts {
                            downloading: r.get(0)?,
                            seeding: r.get(1)?,
                            taken_at: r.get(2)?,
                        })
                    })
                    .optional()?,
                )
            })
            .await
    }

    /// When each item that Transmission took (`received`) got that result, for
    /// results at or after `since`, oldest first.
    pub async fn received_since(&self, since: Millis) -> Result<Vec<Millis>, StatusError> {
        self.db
            .run(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT result_at FROM history_items
                     WHERE result = 'received' AND result_at >= ?1
                     ORDER BY result_at",
                )?;
                let times = stmt
                    .query_map([since], |r| r.get(0))?
                    .collect::<rusqlite::Result<Vec<Millis>>>()?;
                Ok::<_, StatusError>(times)
            })
            .await
    }

    /// How many items ended as `add_failed`, `version_unknown` or `duplicate`
    /// at or after `since`.
    pub async fn problems_since(&self, since: Millis) -> Result<u32, StatusError> {
        self.db
            .run(move |c| {
                Ok::<_, StatusError>(c.prepare_cached("SELECT count(*) FROM history_items
                     WHERE result IN ('add_failed', 'version_unknown', 'duplicate') AND result_at >= ?1")?.query_row(
                    [since],
                    |r| r.get(0),
                )?)
            })
            .await
    }
}

fn record_reads(
    conn: &mut Connection,
    at: Millis,
    reads: &[ChannelReadResult],
    existing: &[String],
) -> Result<(), StatusError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for read in reads {
        tx.prepare_cached(
            "INSERT INTO channel_read_status (channel_id, ok, read_at, ok_at)
             VALUES (?1, ?2, ?3, CASE WHEN ?2 THEN ?3 END)
             ON CONFLICT (channel_id) DO UPDATE SET
                 ok = excluded.ok,
                 read_at = excluded.read_at,
                 ok_at = CASE WHEN excluded.ok THEN excluded.read_at ELSE ok_at END",
        )?
        .execute(params![read.channel_id, read.ok, at])?;
    }
    for read in reads.iter().filter(|read| read.ok) {
        // A day after this read's own is from a clock that was ahead and has
        // come back: it was not a day the feed was read, and it must not take
        // the place of the real days among the newest ones.
        tx.prepare_cached("DELETE FROM channel_read_days WHERE channel_id = ?1 AND day > ?2")?
            .execute(params![read.channel_id, read_day(at)])?;
        // The newest `READ_DAYS_KEPT` days are all that is asked for; an older
        // one (a clock that went back) is gone as soon as it is written.
        tx.prepare_cached(
            "INSERT OR IGNORE INTO channel_read_days (channel_id, day) VALUES (?1, ?2)",
        )?
        .execute(params![read.channel_id, read_day(at)])?;
        tx.prepare_cached(
            "DELETE FROM channel_read_days
             WHERE channel_id = ?1 AND day < (
                 SELECT day FROM channel_read_days WHERE channel_id = ?1
                 ORDER BY day DESC LIMIT 1 OFFSET ?2)",
        )?
        .execute(params![read.channel_id, READ_DAYS_KEPT as i64 - 1])?;
    }
    let keep: HashSet<&str> = existing.iter().map(String::as_str).collect();
    let stored: Vec<String> = {
        let mut stmt = tx.prepare_cached("SELECT channel_id FROM channel_read_status")?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        ids
    };
    for id in stored.iter().filter(|id| !keep.contains(id.as_str())) {
        tx.prepare_cached("DELETE FROM channel_read_status WHERE channel_id = ?1")?
            .execute([id])?;
    }
    let days_of: Vec<String> = {
        let mut stmt = tx.prepare_cached("SELECT DISTINCT channel_id FROM channel_read_days")?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        ids
    };
    for id in days_of.iter().filter(|id| !keep.contains(id.as_str())) {
        tx.prepare_cached("DELETE FROM channel_read_days WHERE channel_id = ?1")?
            .execute([id])?;
    }
    tx.commit()?;
    Ok(())
}

fn read_day_floors(
    conn: &Connection,
    channel_ids: &[String],
    now: Millis,
) -> Result<std::collections::HashMap<String, i64>, StatusError> {
    // The days up to today only: a day after it was written by a clock that was
    // ahead, and is no day the feed was read.
    let mut stmt = conn.prepare_cached(
        "SELECT MIN(day), COUNT(*) FROM (
             SELECT day FROM channel_read_days WHERE channel_id = ?1 AND day <= ?2
             ORDER BY day DESC LIMIT ?3)",
    )?;
    let mut floors = std::collections::HashMap::new();
    for id in channel_ids {
        let (floor, days): (Option<i64>, i64) = stmt
            .query_row(params![id, read_day(now), READ_DAYS_KEPT as i64], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        if let (Some(floor), true) = (floor, days >= READ_DAYS_KEPT as i64) {
            floors.insert(id.clone(), floor);
        }
    }
    Ok(floors)
}

fn channel_reads(conn: &Connection) -> Result<Vec<ChannelRead>, StatusError> {
    let mut stmt = conn.prepare_cached(
        "SELECT channel_id, ok, read_at, ok_at FROM channel_read_status ORDER BY channel_id",
    )?;
    let reads = stmt
        .query_map([], |r| {
            Ok(ChannelRead {
                channel_id: r.get(0)?,
                ok: r.get(1)?,
                read_at: r.get(2)?,
                ok_at: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(reads)
}
