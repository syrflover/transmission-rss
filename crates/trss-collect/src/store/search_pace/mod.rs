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

use trss_core::{
    db::{Db, DbError},
    pace::RequestPace,
    Millis,
};

/// Access to the per-host pace. Cheap to clone.
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

    /// The host asked for no request before `until`.
    pub async fn block(&self, host: &str, until: Millis) -> Result<(), DbError> {
        self.host(host).block_requests(until).await
    }
}
