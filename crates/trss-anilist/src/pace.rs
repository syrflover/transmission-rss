//! The pace of the requests to AniList, kept in the database so the web and
//! the worker together send at most one request every spacing.

use rusqlite::{params, OptionalExtension, TransactionBehavior};
use trss_core::{Db, DbError, Millis};

/// Async access to the request pace. Cheap to clone.
#[derive(Clone)]
pub struct RequestPace {
    db: Db,
}

impl RequestPace {
    pub fn new(db: Db) -> Self {
        RequestPace { db }
    }

    /// Takes the next slot for an AniList request at or after `now`, keeping
    /// `spacing_ms` between the requests of both processes. `Err(wait)`
    /// without taking one when the slot is more than `max_wait_ms` away.
    pub async fn take_request_slot(
        &self,
        now: Millis,
        spacing_ms: i64,
        max_wait_ms: Option<i64>,
    ) -> Result<Result<Millis, i64>, DbError> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let pace: Option<(Millis, Option<Millis>)> = tx
                    .prepare_cached("SELECT next_at, blocked_until FROM anilist_pace WHERE id = 1")?
                    .query_row([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .optional()?;
                let (next_at, blocked) = pace.unwrap_or((now, None));
                let slot = now.max(next_at).max(blocked.unwrap_or(now));
                if let Some(max) = max_wait_ms {
                    if slot - now > max {
                        return Ok::<_, DbError>(Err(slot - now));
                    }
                }
                tx.prepare_cached(
                    "INSERT INTO anilist_pace (id, next_at, blocked_until) VALUES (1, ?1, ?2)
                     ON CONFLICT (id) DO UPDATE SET next_at = excluded.next_at",
                )?
                .execute(params![slot + spacing_ms, blocked])?;
                tx.commit()?;
                Ok(Ok(slot))
            })
            .await
    }

    /// Until when AniList asked for no request, if it did.
    pub async fn blocked_until(&self) -> Result<Option<Millis>, DbError> {
        self.db
            .run(|c| {
                let blocked: Option<Option<Millis>> = c
                    .prepare_cached("SELECT blocked_until FROM anilist_pace WHERE id = 1")?
                    .query_row([], |r| r.get(0))
                    .optional()?;
                Ok::<_, DbError>(blocked.flatten())
            })
            .await
    }

    /// AniList asked for no request before `until`.
    pub async fn block_requests(&self, until: Millis) -> Result<(), DbError> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.prepare_cached(
                    "INSERT INTO anilist_pace (id, next_at, blocked_until) VALUES (1, ?1, ?1)
                     ON CONFLICT (id) DO UPDATE SET
                         next_at = max(next_at, excluded.next_at),
                         blocked_until = max(coalesce(blocked_until, 0), excluded.blocked_until)",
                )?
                .execute(params![until])?;
                tx.commit()?;
                Ok::<_, DbError>(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn requests_take_turns_and_a_block_holds_them_all() {
        let store = RequestPace::new(Db::open(":memory:").await.unwrap());
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
}
