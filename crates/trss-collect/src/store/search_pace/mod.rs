//! The pace of search requests to a feed host, shared through the database
//! (ticket 0026).
//!
//! Processes cannot share an in-memory timer, so the next allowed time of each
//! host is a row of `search_pace`, read and moved forward by
//! [`trss_core::pace::RequestPace`] in one write transaction. The requests of
//! every web process, and of searches running at the same time in one, start
//! at least `spacing` apart. Only searches take slots; the worker's feed reads
//! do not use the row. The caller waits for its turn
//! ([`RequestPace::wait_for_turn`]) before it sends.

#[cfg(test)]
mod tests;

use trss_core::{
    db::{Db, DbError},
    pace::RequestPace,
    Millis,
};

#[derive(Debug, thiserror::Error)]
pub enum PaceError {
    #[error(transparent)]
    Db(#[from] DbError),
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

    /// The pace of `host`'s requests.
    pub(crate) fn host(&self, host: &str) -> RequestPace {
        RequestPace::host(self.db.clone(), host)
    }

    /// See [`RequestPace::take_request_slot`].
    pub async fn take_slot(
        &self,
        host: &str,
        now: Millis,
        spacing_ms: i64,
        max_wait_ms: Option<i64>,
    ) -> Result<std::result::Result<Millis, i64>> {
        Ok(self
            .host(host)
            .take_request_slot(now, spacing_ms, max_wait_ms)
            .await?)
    }

    /// Until when the host asked for no request, if it did.
    pub async fn blocked_until(&self, host: &str) -> Result<Option<Millis>> {
        Ok(self.host(host).blocked_until().await?)
    }

    /// The host asked for no request before `until`.
    pub async fn block(&self, host: &str, until: Millis) -> Result<()> {
        Ok(self.host(host).block_requests(until).await?)
    }
}
