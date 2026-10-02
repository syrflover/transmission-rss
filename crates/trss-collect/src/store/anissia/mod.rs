//! Anissia's schedule as the app keeps it for the anime it subscribes to
//! (`docs/specs/collection.md`, 방영작 구독): the snapshot of each subscribed
//! anime. (The pace of requests to Anissia is kept by `trss-anissia`.)
//!
//! The subscription itself is part of a rule ([`crate::store::channels`]); this
//! store keeps what the rule's detail and the weekly schedule show of the anime
//! without asking Anissia, and what the worker's daily refresh needs to know.

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use trss_anissia::Anime;

use trss_core::{
    db::{Db, DbError},
    Millis,
};

/// A day: how old a snapshot may get before the worker asks Anissia again.
pub const REFRESH_AFTER_MS: i64 = 24 * 60 * 60 * 1000;

/// A subscribed anime whose snapshot is due to be received again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub anime_no: i64,
    /// The group the snapshot was listed in; `None` without a snapshot.
    pub week: Option<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum AnissiaStoreError {
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<rusqlite::Error> for AnissiaStoreError {
    fn from(e: rusqlite::Error) -> Self {
        AnissiaStoreError::Db(DbError::Sqlite(e))
    }
}

type Result<T> = std::result::Result<T, AnissiaStoreError>;

/// Async access to the snapshots. Cheap to clone.
#[derive(Clone)]
pub struct AnissiaStore {
    db: Db,
}

const COLUMNS: &str = "anime_no, subject, original_subject, week, air_time, start_date, end_date, \
     status, fetched_at";

fn anime_from_row(row: &Row<'_>) -> rusqlite::Result<Anime> {
    Ok(Anime {
        anime_no: row.get(0)?,
        subject: row.get(1)?,
        original_subject: row.get(2)?,
        week: row.get(3)?,
        air_time: row.get(4)?,
        start_date: row.get(5)?,
        end_date: row.get(6)?,
        status: row.get(7)?,
        fetched_at: row.get(8)?,
    })
}

/// Writes `anime` as the anime's snapshot, replacing an earlier one. A refresh
/// that failed earlier is forgotten: this one is the newest. Anissia listing
/// the anime also ends its having been found unlisted.
pub(crate) fn upsert_in(conn: &Connection, anime: &Anime) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO anissia_anime (anime_no, subject, original_subject, week, air_time,
                                    start_date, end_date, status, fetched_at, refresh_not_before)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)
         ON CONFLICT (anime_no) DO UPDATE SET
             subject = excluded.subject, original_subject = excluded.original_subject,
             week = excluded.week, air_time = excluded.air_time,
             start_date = excluded.start_date, end_date = excluded.end_date,
             status = excluded.status, fetched_at = excluded.fetched_at,
             refresh_not_before = NULL, unlisted_at = NULL",
        params![
            anime.anime_no,
            anime.subject,
            anime.original_subject,
            anime.week,
            anime.air_time,
            anime.start_date,
            anime.end_date,
            anime.status,
            anime.fetched_at,
        ],
    )?;
    Ok(())
}

impl AnissiaStore {
    pub fn new(db: Db) -> Self {
        AnissiaStore { db }
    }

    /// Stores `anime` as the snapshot of its anime.
    pub async fn put_anime(&self, anime: Anime) -> Result<()> {
        self.db
            .run(move |c| Ok::<_, AnissiaStoreError>(upsert_in(c, &anime)?))
            .await
    }

    /// The snapshot of anime `anime_no`, if the app has one.
    pub async fn anime(&self, anime_no: i64) -> Result<Option<Anime>> {
        self.db
            .run(move |c| {
                Ok::<_, AnissiaStoreError>(
                    c.query_row(
                        &format!("SELECT {COLUMNS} FROM anissia_anime WHERE anime_no = ?1"),
                        [anime_no],
                        anime_from_row,
                    )
                    .optional()?,
                )
            })
            .await
    }

    /// The snapshots of `anime_nos` that exist, by anime number.
    pub async fn animes(&self, anime_nos: Vec<i64>) -> Result<HashMap<i64, Anime>> {
        self.db
            .run(move |c| {
                let mut stmt = c.prepare(&format!(
                    "SELECT {COLUMNS} FROM anissia_anime WHERE anime_no = ?1"
                ))?;
                let mut out = HashMap::new();
                for no in anime_nos {
                    if let Some(anime) = stmt.query_row([no], anime_from_row).optional()? {
                        out.insert(no, anime);
                    }
                }
                Ok::<_, AnissiaStoreError>(out)
            })
            .await
    }

    /// Which of `anime_nos` Anissia was found not to list any more (see
    /// [`AnissiaStore::mark_unlisted`]); anime without a snapshot are not.
    pub async fn unlisted(&self, anime_nos: Vec<i64>) -> Result<HashSet<i64>> {
        self.db
            .run(move |c| {
                let mut stmt = c.prepare(
                    "SELECT unlisted_at IS NOT NULL FROM anissia_anime WHERE anime_no = ?1",
                )?;
                let mut out = HashSet::new();
                for no in anime_nos {
                    if stmt
                        .query_row([no], |r| r.get(0))
                        .optional()?
                        .unwrap_or(false)
                    {
                        out.insert(no);
                    }
                }
                Ok::<_, AnissiaStoreError>(out)
            })
            .await
    }

    /// The anime a collecting or paused rule subscribes to whose snapshot is a day old (or
    /// missing) and not held back, oldest first.
    pub async fn due(&self, now: Millis) -> Result<Vec<Due>> {
        self.db
            .run(move |c| {
                let mut stmt = c.prepare(
                    "SELECT s.anissia_anime_no, a.week
                       FROM rule_subscriptions s
                       JOIN rules r ON r.id = s.rule_id AND r.state IN ('active', 'paused')
                       LEFT JOIN anissia_anime a ON a.anime_no = s.anissia_anime_no
                      WHERE a.anime_no IS NULL
                         OR (a.fetched_at <= ?1 - ?2
                             AND (a.refresh_not_before IS NULL OR a.refresh_not_before <= ?1))
                      GROUP BY s.anissia_anime_no
                      ORDER BY coalesce(a.fetched_at, 0), s.anissia_anime_no",
                )?;
                let rows = stmt
                    .query_map(params![now, REFRESH_AFTER_MS], |r| {
                        Ok(Due {
                            anime_no: r.get(0)?,
                            week: r.get(1)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok::<_, AnissiaStoreError>(rows)
            })
            .await
    }

    /// Holds the refresh of `anime_nos` back until `until`.
    pub async fn refresh_later(&self, anime_nos: Vec<i64>, until: Millis) -> Result<()> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                for no in anime_nos {
                    tx.execute(
                        "UPDATE anissia_anime SET refresh_not_before = ?2 WHERE anime_no = ?1",
                        params![no, until],
                    )?;
                }
                tx.commit()?;
                Ok::<_, AnissiaStoreError>(())
            })
            .await
    }

    /// Records that Anissia answered for every week of its schedule at `at`
    /// and listed none of `anime_nos`, and holds their next refresh back until
    /// `until`. Only the worker's daily refresh may call this, and only after
    /// every week was answered: a refresh that failed or stopped halfway knows
    /// nothing about the anime it did not find. The first time is kept while the
    /// anime stays unlisted.
    pub async fn mark_unlisted(
        &self,
        anime_nos: Vec<i64>,
        at: Millis,
        until: Millis,
        asked_from: Millis,
    ) -> Result<()> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                for no in anime_nos {
                    tx.execute(
                        "UPDATE anissia_anime
                            SET unlisted_at = coalesce(unlisted_at, ?2), refresh_not_before = ?3
                          WHERE anime_no = ?1 AND fetched_at < ?4",
                        params![no, at, until, asked_from],
                    )?;
                }
                tx.commit()?;
                Ok::<_, AnissiaStoreError>(())
            })
            .await
    }
}
