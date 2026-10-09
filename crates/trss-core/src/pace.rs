//! The pace of the requests to an outside service, kept in the database so the
//! web and the worker, two processes, together send at most one request every
//! spacing.
//!
//! A service's pace is one row of a table: when the next request may start
//! (`next_at`) and until when the service asked for none (`blocked_until`, set
//! from a `429` answer's `Retry-After`). AniList and Anissia have a table of
//! their own with a single row; the feed hosts share `search_pace`, a row per
//! host. [`RequestPace`] reads and moves any of the three rows the same way.

use std::time::Duration;

use rusqlite::{params, types::Value, OptionalExtension, TransactionBehavior};

use crate::{Clock, Db, DbError, Millis};

/// Async access to the pace of one service. Cheap to clone.
#[derive(Clone)]
pub struct RequestPace {
    db: Db,
    row: Row,
}

/// Where the pace of one service is kept: the table, its key column and the
/// key. Only the constructors of [`RequestPace`] make one, so the table and
/// column names written into the SQL are the fixed ones below.
#[derive(Clone)]
struct Row {
    table: &'static str,
    column: &'static str,
    key: Value,
}

/// Why a turn was not given.
#[derive(Debug, thiserror::Error)]
pub enum TurnError {
    #[error(transparent)]
    Db(#[from] DbError),
    /// The service allows no request for this long: either the turn is more
    /// than the longest wait away, or a `429` came while the request waited.
    /// Nothing was sent.
    #[error("no request is allowed for {}s", .0.as_secs())]
    Wait(Duration),
}

impl RequestPace {
    /// The pace of the requests to AniList (`anilist_pace`).
    pub fn anilist(db: Db) -> Self {
        RequestPace::single(db, "anilist_pace")
    }

    /// The pace of the requests to Anissia (`anissia_pace`).
    pub fn anissia(db: Db) -> Self {
        RequestPace::single(db, "anissia_pace")
    }

    /// The pace of the requests to a feed host (`search_pace`). The host's
    /// letter case does not matter.
    pub fn host(db: Db, host: &str) -> Self {
        RequestPace {
            db,
            row: Row {
                table: "search_pace",
                column: "host",
                key: Value::Text(host.to_ascii_lowercase()),
            },
        }
    }

    fn single(db: Db, table: &'static str) -> Self {
        RequestPace {
            db,
            row: Row {
                table,
                column: "id",
                key: Value::Integer(1),
            },
        }
    }

    /// Takes the next slot for a request at or after `now`, keeping
    /// `spacing_ms` between this slot and the next one, whichever process
    /// asks. The slot is when the request may start: `now` or later.
    ///
    /// With `max_wait_ms`, a slot further away than that is not taken: the
    /// answer is `Err(wait_ms)` and the pace is left as it was, so a caller
    /// that gives up does not push the other requests back.
    pub async fn take_request_slot(
        &self,
        now: Millis,
        spacing_ms: i64,
        max_wait_ms: Option<i64>,
    ) -> Result<Result<Millis, i64>, DbError> {
        let Row { table, column, key } = self.row.clone();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let pace: Option<(Millis, Option<Millis>)> = tx
                    .prepare_cached(&format!(
                        "SELECT next_at, blocked_until FROM {table} WHERE {column} = ?1"
                    ))?
                    .query_row(params![key], |r| Ok((r.get(0)?, r.get(1)?)))
                    .optional()?;
                let (next_at, blocked) = pace.unwrap_or((now, None));
                let slot = now.max(next_at).max(blocked.unwrap_or(now));
                if max_wait_ms.is_some_and(|max| slot - now > max) {
                    return Ok::<_, DbError>(Err(slot - now));
                }
                tx.prepare_cached(&format!(
                    "INSERT INTO {table} ({column}, next_at, blocked_until) VALUES (?1, ?2, ?3)
                     ON CONFLICT ({column}) DO UPDATE SET next_at = excluded.next_at"
                ))?
                .execute(params![key, slot + spacing_ms, blocked])?;
                tx.commit()?;
                Ok(Ok(slot))
            })
            .await
    }

    /// Until when the service asked for no request, if it did.
    pub async fn blocked_until(&self) -> Result<Option<Millis>, DbError> {
        let Row { table, column, key } = self.row.clone();
        self.db
            .run(move |c| {
                let blocked: Option<Option<Millis>> = c
                    .prepare_cached(&format!(
                        "SELECT blocked_until FROM {table} WHERE {column} = ?1"
                    ))?
                    .query_row(params![key], |r| r.get(0))
                    .optional()?;
                Ok::<_, DbError>(blocked.flatten())
            })
            .await
    }

    /// The service asked for no request before `until`. A shorter block does
    /// not shorten a longer one.
    pub async fn block_requests(&self, until: Millis) -> Result<(), DbError> {
        let Row { table, column, key } = self.row.clone();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.prepare_cached(&format!(
                    "INSERT INTO {table} ({column}, next_at, blocked_until) VALUES (?1, ?2, ?2)
                     ON CONFLICT ({column}) DO UPDATE SET
                         next_at = max(next_at, excluded.next_at),
                         blocked_until = max(coalesce(blocked_until, 0), excluded.blocked_until)"
                ))?
                .execute(params![key, until])?;
                tx.commit()?;
                Ok::<_, DbError>(())
            })
            .await
    }

    /// Waits for this process's turn to send a request: takes a slot `spacing`
    /// apart from the others and sleeps until it. With `max_wait`, a turn
    /// further away than that is not taken ([`TurnError::Wait`]).
    ///
    /// Another request may have been answered `429` while this one waited, its
    /// turn having been taken before the block was, so the block is read again
    /// after the wait and holds the request back ([`TurnError::Wait`] with the
    /// time left).
    pub async fn wait_for_turn(
        &self,
        clock: &Clock,
        spacing: Duration,
        max_wait: Option<Duration>,
    ) -> Result<(), TurnError> {
        let now = clock();
        let slot = self
            .take_request_slot(
                now,
                spacing.as_millis() as i64,
                max_wait.map(|d| d.as_millis() as i64),
            )
            .await?;
        let at = slot.map_err(|wait| TurnError::Wait(Duration::from_millis(wait.max(0) as u64)))?;
        if at > now {
            tokio::time::sleep(Duration::from_millis((at - now) as u64)).await;
            let now = clock();
            if let Some(until) = self.blocked_until().await?.filter(|u| *u > now) {
                return Err(TurnError::Wait(Duration::from_millis((until - now) as u64)));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// A clock that stays put while real time passes.
    fn fixed(at: Millis) -> Clock {
        Arc::new(move || at)
    }

    #[tokio::test]
    async fn requests_take_turns_and_a_block_holds_them_all() {
        let store = RequestPace::anilist(Db::open(":memory:").await.unwrap());
        assert_eq!(
            store.take_request_slot(1000, 2000, None).await.unwrap(),
            Ok(1000)
        );
        assert_eq!(
            store.take_request_slot(1000, 2000, None).await.unwrap(),
            Ok(3000)
        );
        // A caller that may wait 1 s is told the wait instead of taking the slot.
        assert_eq!(
            store
                .take_request_slot(1000, 2000, Some(1000))
                .await
                .unwrap(),
            Err(4000)
        );
        assert_eq!(
            store.take_request_slot(1000, 2000, None).await.unwrap(),
            Ok(5000)
        );

        store.block_requests(60_000).await.unwrap();
        assert_eq!(
            store.take_request_slot(7000, 2000, None).await.unwrap(),
            Ok(60_000)
        );
        // A shorter block does not shorten a longer one.
        store.block_requests(10_000).await.unwrap();
        assert_eq!(
            store.take_request_slot(8000, 2000, None).await.unwrap(),
            Ok(62_000)
        );
    }

    #[tokio::test]
    async fn request_slots_keep_their_spacing_and_a_block_holds_every_request() {
        let store = RequestPace::anissia(Db::open(":memory:").await.unwrap());
        assert_eq!(
            store.take_request_slot(1000, 2000, None).await.unwrap(),
            Ok(1000)
        );
        assert_eq!(
            store.take_request_slot(1000, 2000, None).await.unwrap(),
            Ok(3000)
        );
        // A caller that may wait 1 s is told the wait instead of taking the slot.
        assert_eq!(
            store
                .take_request_slot(1000, 2000, Some(1000))
                .await
                .unwrap(),
            Err(4000)
        );
        assert_eq!(
            store.take_request_slot(1000, 2000, None).await.unwrap(),
            Ok(5000)
        );

        store.block_requests(60_000).await.unwrap();
        assert_eq!(
            store.take_request_slot(7000, 2000, None).await.unwrap(),
            Ok(60_000)
        );
        // A shorter block does not shorten a longer one.
        store.block_requests(10_000).await.unwrap();
        assert_eq!(
            store.take_request_slot(8000, 2000, None).await.unwrap(),
            Ok(62_000)
        );
    }

    #[tokio::test]
    async fn the_three_services_and_each_host_have_their_own_pace_and_case_does_not_matter() {
        let db = Db::open(":memory:").await.unwrap();
        let rows = [
            RequestPace::anilist(db.clone()),
            RequestPace::anissia(db.clone()),
            RequestPace::host(db.clone(), "Nyaa.si"),
            RequestPace::host(db.clone(), "other.test"),
        ];
        for row in &rows {
            assert_eq!(
                row.take_request_slot(1000, 3000, None).await.unwrap(),
                Ok(1000)
            );
        }
        let again = RequestPace::host(db.clone(), "nyaa.SI");
        assert_eq!(
            again.take_request_slot(1000, 3000, None).await.unwrap(),
            Ok(4000)
        );
        // A block of one row holds the others' requests not at all.
        again.block_requests(60_000).await.unwrap();
        assert_eq!(rows[0].blocked_until().await.unwrap(), None);
        assert_eq!(rows[2].blocked_until().await.unwrap(), Some(60_000));
        assert_eq!(rows[3].blocked_until().await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_slot_further_away_than_the_longest_wait_is_not_taken() {
        let pace = RequestPace::host(Db::open(":memory:").await.unwrap(), "nyaa.si");
        pace.block_requests(3_600_000).await.unwrap();
        // The wait is told, and the pace stays as the block left it.
        for _ in 0..2 {
            assert_eq!(
                pace.take_request_slot(1_000, 3_000, Some(60_000))
                    .await
                    .unwrap(),
                Err(3_599_000)
            );
        }
        // Once the block has passed, the first request is not behind the refused ones.
        assert_eq!(
            pace.take_request_slot(3_600_000, 3_000, Some(60_000))
                .await
                .unwrap(),
            Ok(3_600_000)
        );
    }

    #[tokio::test]
    async fn two_handles_on_one_file_share_the_pace() {
        // The web and the worker are separate processes with their own connection.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        let web = RequestPace::anilist(Db::open(&path).await.unwrap());
        let worker = RequestPace::anilist(Db::open(&path).await.unwrap());
        assert_eq!(
            web.take_request_slot(1000, 3000, None).await.unwrap(),
            Ok(1000)
        );
        assert_eq!(
            worker.take_request_slot(1000, 3000, None).await.unwrap(),
            Ok(4000)
        );
        assert_eq!(
            web.take_request_slot(1000, 3000, None).await.unwrap(),
            Ok(7000)
        );
    }

    #[tokio::test]
    async fn a_request_waits_for_its_turn_and_a_far_turn_is_refused_with_its_wait() {
        const NOW: Millis = 1_000_000;
        let pace = RequestPace::anissia(Db::open(":memory:").await.unwrap());
        let clock = fixed(NOW);
        let spacing = Duration::from_millis(300);

        let started = std::time::Instant::now();
        pace.wait_for_turn(&clock, spacing, None).await.unwrap();
        assert!(started.elapsed() < Duration::from_millis(250));
        // The first request holds the turn: the next waits for it.
        pace.wait_for_turn(&clock, spacing, None).await.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(250));

        // Two spacings are queued now; a caller who may wait one spacing is refused.
        match pace
            .wait_for_turn(&clock, spacing, Some(Duration::from_millis(300)))
            .await
        {
            Err(TurnError::Wait(wait)) => assert_eq!(wait, Duration::from_millis(600)),
            other => panic!("expected the wait, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_block_that_comes_while_a_request_waits_for_its_turn_stops_the_request() {
        const NOW: Millis = 1_000_000;
        let pace = RequestPace::host(Db::open(":memory:").await.unwrap(), "127.0.0.1");
        let clock = fixed(NOW);
        let spacing = Duration::from_millis(600);
        // Another request holds the turn before the test's.
        assert_eq!(
            pace.take_request_slot(NOW, 600, None).await.unwrap(),
            Ok(NOW)
        );
        let blocker = tokio::spawn({
            let pace = pace.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(150)).await;
                pace.block_requests(NOW + 10_000).await.unwrap();
            }
        });
        let answer = pace.wait_for_turn(&clock, spacing, None).await;
        blocker.await.unwrap();
        match answer {
            Err(TurnError::Wait(wait)) => assert_eq!(wait, Duration::from_secs(10)),
            other => panic!("expected the wait, got {other:?}"),
        }
    }
}
