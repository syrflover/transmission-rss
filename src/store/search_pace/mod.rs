//! The pace of search requests to a feed host, shared by the web and the
//! worker through the database (ticket 0026).
//!
//! Two processes cannot share an in-memory timer, so the next allowed time of
//! each host is a row. [`SearchPace::take_slot`] reads it and moves it
//! forward in one write transaction, so the requests of both processes, and of
//! searches running at the same time in one, start at least `spacing` apart.
//! The caller sleeps until the slot it was given before it sends.

#[cfg(test)]
mod tests;

use rusqlite::{params, OptionalExtension, TransactionBehavior};

use super::{
    db::{Db, DbError},
    history::Millis,
};

#[derive(Debug, thiserror::Error)]
pub enum PaceError {
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<rusqlite::Error> for PaceError {
    fn from(e: rusqlite::Error) -> Self {
        PaceError::Db(DbError::Sqlite(e))
    }
}

type Result<T> = std::result::Result<T, PaceError>;

/// Async access to the per-host pace. Cheap to clone.
#[derive(Clone)]
pub struct SearchPace {
    db: Db,
}

impl SearchPace {
    pub fn new(db: Db) -> Self {
        SearchPace { db }
    }

    /// Takes the next slot for a request to `host` at or after `now` and keeps
    /// `spacing_ms` between this slot and the next one, whichever process asks.
    /// The slot is when the request may start: `now` or later.
    ///
    /// With `max_wait_ms`, a slot further away than that is not taken: the
    /// answer is `Err(wait_ms)` and the pace is left as it was, so a caller
    /// that gives up does not push the other requests back.
    pub async fn take_slot(
        &self,
        host: &str,
        now: Millis,
        spacing_ms: i64,
        max_wait_ms: Option<i64>,
    ) -> Result<std::result::Result<Millis, i64>> {
        let host = host.to_ascii_lowercase();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let pace: Option<(Millis, Option<Millis>)> = tx
                    .query_row(
                        "SELECT next_at, blocked_until FROM search_pace WHERE host = ?1",
                        params![host],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                let (next_at, blocked) = pace.unwrap_or((now, None));
                let slot = now.max(next_at).max(blocked.unwrap_or(now));
                if max_wait_ms.is_some_and(|max| slot - now > max) {
                    return Ok::<_, PaceError>(Err(slot - now));
                }
                tx.execute(
                    "INSERT INTO search_pace (host, next_at, blocked_until) VALUES (?1, ?2, ?3)
                     ON CONFLICT (host) DO UPDATE SET next_at = excluded.next_at",
                    params![host, slot + spacing_ms, blocked],
                )?;
                tx.commit()?;
                Ok::<_, PaceError>(Ok(slot))
            })
            .await
    }

    /// Until when the host asked for no request, if it did.
    pub async fn blocked_until(&self, host: &str) -> Result<Option<Millis>> {
        let host = host.to_ascii_lowercase();
        self.db
            .run(move |c| {
                let blocked: Option<Option<Millis>> = c
                    .query_row(
                        "SELECT blocked_until FROM search_pace WHERE host = ?1",
                        params![host],
                        |r| r.get(0),
                    )
                    .optional()?;
                Ok::<_, PaceError>(blocked.flatten())
            })
            .await
    }

    /// The host asked for no request before `until`.
    pub async fn block(&self, host: &str, until: Millis) -> Result<()> {
        let host = host.to_ascii_lowercase();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.execute(
                    "INSERT INTO search_pace (host, next_at, blocked_until) VALUES (?1, ?2, ?2)
                     ON CONFLICT (host) DO UPDATE SET
                         next_at = max(next_at, excluded.next_at),
                         blocked_until = max(coalesce(blocked_until, 0), excluded.blocked_until)",
                    params![host, until],
                )?;
                tx.commit()?;
                Ok::<_, PaceError>(())
            })
            .await
    }
}
