//! Sending a search to the tracker: one request for one page of its search
//! RSS, after the request pace of the host allows it.
//!
//! This is the one place that requests a search page (only the web asks), and
//! the one place the pace is kept: every request takes its slot from the
//! database ([`SearchPace::take_slot`]) and waits for it, so the requests of
//! the web processes and of searches running side by side stay
//! [`REQUEST_SPACING`] apart. A `429` answer blocks the host for as long as it asks.
//!
//! A search reads the RSS only. The page's HTML is never requested.

#[cfg(test)]
mod tests;

use std::time::Duration;

use trss_core::{system_clock, Clock};

use crate::{
    feed::{self, FeedItem, FetchError},
    past_search::query::search_url,
    store::{channels::Channel, search_pace::SearchPace},
};
use trss_transmission::Redactor;

/// The time between two requests to one host. A search of a long series sends
/// up to [`super::MAX_EXTRA_SEARCHES`] more after the first, so this is the
/// price of those: a minute at the most.
pub const REQUEST_SPACING: Duration = Duration::from_secs(3);

/// The longest a request waits for its turn. A turn further away (the host
/// asked for no request for a long while, or the clock moved) fails the search
/// with the time to try again, rather than leaving the screen on `검색하는
/// 중` for as long. The queue of searches running side by side is far shorter:
/// [`super::service::MAX_SEARCHES`] × [`REQUEST_SPACING`].
pub const MAX_WAIT: Duration = Duration::from_secs(60);

/// How long to wait, for a sentence: `45초`, `12분`, `2시간` (rounded up).
pub fn wait_phrase(wait: Duration) -> String {
    let secs = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    match secs {
        0..=59 => format!("{}초", secs.max(1)),
        60..=3599 => format!("{}분", secs.div_ceil(60)),
        _ => format!("{}시간", secs.div_ceil(3600)),
    }
}

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
    /// The request pace of the host allows no request before this long, which
    /// is more than a search waits ([`MAX_WAIT`]). Nothing was sent.
    #[error("the host allows no request for {}s", .0.as_secs())]
    Wait(Duration),
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
            .take_slot(
                &host,
                now,
                self.spacing.as_millis() as i64,
                Some(MAX_WAIT.as_millis() as i64),
            )
            .await
            .map_err(|e| SearchError::Pace(e.to_string()))?
            .map_err(|wait| SearchError::Wait(Duration::from_millis(wait.max(0) as u64)))?;
        if slot > now {
            tokio::time::sleep(Duration::from_millis((slot - now) as u64)).await;
            // A request of another search may have been answered `429` while
            // this one waited: its turn was taken before the block was.
            let until = self
                .pace
                .blocked_until(&host)
                .await
                .map_err(|e| SearchError::Pace(e.to_string()))?;
            let now = (self.clock)();
            if let Some(until) = until.filter(|until| *until > now) {
                return Err(SearchError::Wait(Duration::from_millis(
                    (until - now) as u64,
                )));
            }
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
