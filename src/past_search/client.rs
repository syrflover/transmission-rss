//! Sending a search to the tracker: one request for one page of its search
//! RSS, after the request pace of the host allows it.
//!
//! This is the one place that requests a search page, whichever process asks
//! (the web today; the worker may later), and the one place the pace is kept:
//! every request takes its slot from the database
//! ([`SearchPace::take_slot`]) and waits for it, so the requests of the two
//! processes and of searches running side by side stay [`REQUEST_SPACING`]
//! apart. A `429` answer blocks the host for as long as it asks.
//!
//! A search reads the RSS only. The page's HTML is never requested.

use std::time::Duration;

use super::query::search_url;
use crate::{
    store::{channels::Channel, search_pace::SearchPace},
    transmission::Redactor,
    worker::{
        feed::{self, FeedItem, FetchError},
        system_clock, Clock,
    },
};

/// The time between two requests to one host. A search of a long series sends
/// up to [`super::MAX_EXTRA_SEARCHES`] more after the first, so this is the
/// price of those: a minute at the most.
pub const REQUEST_SPACING: Duration = Duration::from_secs(3);

/// A page of results.
#[derive(Debug, Clone)]
pub struct Page {
    pub items: Vec<FeedItem>,
    /// How many items the RSS held, before items with the same identity were
    /// merged. A tracker that returns [`super::PAGE_LIMIT`] has probably more.
    pub raw_count: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// The channel's address is not one a search can be made from.
    #[error("the channel address is not a URL")]
    BadAddress,
    #[error("cannot use the request pace: {0}")]
    Pace(String),
    /// The tracker asks to wait. The host is blocked for that long.
    #[error("the tracker asks to wait {}s", .0.as_secs())]
    Busy(Duration),
    #[error("cannot read the search: {0}")]
    Read(String),
}

/// Requests search pages. Cheap to clone.
#[derive(Clone)]
pub struct SearchClient {
    http: reqwest::Client,
    pace: SearchPace,
    spacing: Duration,
    clock: Clock,
}

impl SearchClient {
    pub fn new(pace: SearchPace) -> Result<SearchClient, reqwest::Error> {
        Ok(SearchClient {
            http: feed::client()?,
            pace,
            spacing: REQUEST_SPACING,
            clock: system_clock(),
        })
    }

    /// Overrides the time between requests (tests).
    pub fn with_spacing(mut self, spacing: Duration) -> Self {
        self.spacing = spacing;
        self
    }

    /// Overrides the clock the pace is read by (tests).
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Reads one page of `query` in `channel`'s tracker. `redactor` knows the
    /// channel's secret values; no error holds the address.
    pub async fn page(
        &self,
        channel: &Channel,
        query: &str,
        redactor: &Redactor,
    ) -> Result<Page, SearchError> {
        let url = search_url(&channel.url, query).ok_or(SearchError::BadAddress)?;
        let host = url::Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .ok_or(SearchError::BadAddress)?;

        let now = (self.clock)();
        let slot = self
            .pace
            .take_slot(&host, now, self.spacing.as_millis() as i64)
            .await
            .map_err(|e| SearchError::Pace(e.to_string()))?;
        if slot > now {
            tokio::time::sleep(Duration::from_millis((slot - now) as u64)).await;
        }

        let read = match feed::fetch(&self.http, &url).await {
            Ok(read) => read,
            Err(FetchError::Busy(wait)) => {
                let until = (self.clock)() + wait.as_millis() as i64;
                if let Err(e) = self.pace.block(&host, until).await {
                    eprintln!("Search pace: cannot block {host}: {e}");
                }
                return Err(SearchError::Busy(wait));
            }
            Err(err) => return Err(SearchError::Read(redactor.apply(&err.to_string()))),
        };
        let raw_count = read.items().len();
        let items = feed::items(&read, &channel.secret_query, redactor);
        Ok(Page { items, raw_count })
    }
}
