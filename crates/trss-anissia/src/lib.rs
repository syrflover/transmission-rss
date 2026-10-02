//! The Anissia client (`docs/specs/collection.md`, 방영작 구독): the weekly
//! anime schedule and the subtitle creators of an anime, read from
//! `api.anissia.net`.
//!
//! - `GET /anime/schedule/<week>`: `0` (Sunday) to `6` (Saturday), `7` (`기타`)
//!   and `8` (`신작`, not started yet). Times are Asia/Seoul.
//! - `GET /anime/caption/animeNo/<n>`: the captions (subtitle releases) of an
//!   anime, with the creator's name.
//! - `GET /anime/list/<page>?q=<text>`: the full anime list, searched by title
//!   (not in Anissia's documentation, see [`parse::anime_page`]). `page` counts
//!   from 0, a page has up to 30 anime, and finished anime are in it.
//!
//! [`parse`] reads the answers. [`Anissia`] asks for them:
//!
//! - **Pace.** Every request takes a slot from the database
//!   ([`RequestPace::take_request_slot`]), so the web and the worker together
//!   send at most one request every [`REQUEST_SPACING`]. A `429` answer blocks
//!   every request until its `Retry-After` has passed.
//! - **Bounded answers.** An answer is read up to [`MAX_ANSWER_BYTES`]; a longer
//!   one is refused before it is parsed. Redirects are not followed.
//! - **Short cache.** The web asks Anissia when the user looks at a schedule,
//!   and keeps what it was told for [`CACHE_TTL`] in memory, so tabs flipped
//!   back and forth cost no request. The pages of a title search are kept the
//!   same way (the last [`MAX_CACHED_PAGES`] of them), so that saving the anime
//!   a user picked from a page is checked against that page without asking
//!   again. The cache is per process and never read by the worker.
//! - **Configurable for tests.** The base URL comes from [`AnissiaConfig`]
//!   (`TRSS_ANISSIA_URL`), so tests point it at a local fake.
//!
//! The schedule of a subscribed anime is kept in the database by
//! `trss-collect` so the app shows its weekday and time without Anissia; this
//! crate holds what it keeps ([`Anime`]) and how the schedule's days and slots
//! are read ([`calendar`], [`slot`]).

mod anime;
pub mod calendar;
pub mod pace;
pub mod parse;
pub mod slot;

#[cfg(any(test, feature = "test-support"))]
pub mod fake;
#[cfg(test)]
mod tests;

pub use anime::{Anime, WEEK_OTHER, WEEK_UPCOMING};

use std::{
    collections::HashMap,
    future::Future,
    hash::Hash,
    sync::{Arc, Mutex},
    time::Duration,
};

use reqwest::{header, redirect, StatusCode};
use url::Url;

pub use parse::{AnimePage, Caption, Creator, ScheduleEntry};

use pace::RequestPace;
use trss_core::{system_clock, Clock, Db, DbError, Millis};

/// Anissia's API.
pub const DEFAULT_URL: &str = "https://api.anissia.net";
/// Environment variable overriding [`DEFAULT_URL`] (tests and local checks).
pub const URL_VAR: &str = "TRSS_ANISSIA_URL";

/// The time between two requests of the app: 30 a minute, the pace AniList's
/// client keeps.
pub const REQUEST_SPACING: Duration = Duration::from_secs(2);
/// How long one request may take.
pub const API_TIMEOUT: Duration = Duration::from_secs(20);
/// The largest answer read. A week's schedule is about 55 KB (182 entries
/// listed as upcoming) and an anime's captions a few KB.
pub const MAX_ANSWER_BYTES: usize = 2 * 1024 * 1024;
/// How long the web keeps an answer it was given.
pub const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
/// How many pages of title searches the client keeps.
pub const MAX_CACHED_PAGES: usize = 32;
/// How long to wait when a `429` answer names no time.
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(60);
/// The longest `Retry-After` honoured as given.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(3600);

/// The highest week number: `8`, `신작`.
pub const LAST_WEEK: u8 = WEEK_UPCOMING;

/// Where the client goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnissiaConfig {
    pub base_url: Url,
}

impl Default for AnissiaConfig {
    fn default() -> Self {
        AnissiaConfig {
            base_url: Url::parse(DEFAULT_URL).expect("valid default URL"),
        }
    }
}

impl AnissiaConfig {
    /// Reads [`URL_VAR`] through `lookup`; an empty value counts as unset.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let mut config = AnissiaConfig::default();
        if let Some(url) = lookup(URL_VAR).filter(|v| !v.trim().is_empty()) {
            let parsed = Url::parse(url.trim())
                .ok()
                .filter(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
                .ok_or_else(|| format!("{URL_VAR} must be an http(s) URL, got {url:?}"))?;
            config.base_url = parsed;
        }
        Ok(config)
    }

    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url.as_str().trim_end_matches('/'))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AnissiaError {
    /// Anissia asked to wait, or the app's own pace would make the caller wait
    /// longer than it may.
    #[error("Anissia asks to wait {}s", retry_after.as_secs())]
    Busy { retry_after: Duration },
    #[error("cannot reach Anissia: {0}")]
    Unreachable(String),
    #[error("Anissia answered HTTP {0}")]
    Status(u16),
    #[error("Anissia's answer is not usable: {0}")]
    Invalid(String),
    #[error("{0} is not a week of the schedule")]
    NoSuchWeek(u8),
    #[error(transparent)]
    Store(#[from] DbError),
}

/// What a request gave and how old it is.
#[derive(Debug, Clone)]
pub struct Fetched<T> {
    pub value: Arc<Vec<T>>,
    /// When Anissia was asked (Unix ms).
    pub fetched_at: Millis,
    /// Whether this is the cached answer of an earlier request.
    pub cached: bool,
}

/// A page of the full anime list as the client asked for it.
#[derive(Debug, Clone)]
pub struct FetchedPage {
    pub page: Arc<AnimePage>,
    /// When Anissia was asked (Unix ms).
    pub fetched_at: Millis,
    /// Whether this is the cached answer of an earlier request.
    pub cached: bool,
}

/// The pages of title searches kept, by what was asked (the title text and the
/// page, from 0).
type Pages = Mutex<HashMap<(String, u32), (Millis, Arc<AnimePage>)>>;

type Slot<V> = Arc<tokio::sync::Mutex<Option<(Millis, Arc<Vec<V>>)>>>;

/// What a process remembers of Anissia's answers, by what was asked.
struct Slots<K, V> {
    slots: Mutex<HashMap<K, Slot<V>>>,
}

impl<K, V> Default for Slots<K, V> {
    fn default() -> Self {
        Slots {
            slots: Mutex::default(),
        }
    }
}

impl<K: Hash + Eq, V> Slots<K, V> {
    fn slot(&self, key: K) -> Slot<V> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.entry(key).or_default().clone()
    }
}

/// The Anissia client. Cheap to clone.
#[derive(Clone)]
pub struct Anissia {
    config: Arc<AnissiaConfig>,
    http: reqwest::Client,
    pace: RequestPace,
    clock: Clock,
    spacing: Duration,
    schedules: Arc<Slots<u8, ScheduleEntry>>,
    captions: Arc<Slots<i64, Caption>>,
    pages: Arc<Pages>,
}

impl Anissia {
    pub fn new(db: Db, config: AnissiaConfig, clock: Clock) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(trss_core::USER_AGENT)
            .timeout(API_TIMEOUT)
            .redirect(redirect::Policy::none())
            .build()
            .expect("a client with a timeout and no redirects builds");
        Anissia {
            config: Arc::new(config),
            http,
            pace: RequestPace::new(db),
            clock,
            spacing: REQUEST_SPACING,
            schedules: Arc::default(),
            captions: Arc::default(),
            pages: Arc::default(),
        }
    }

    /// The client over `db` with the system clock.
    pub fn with_defaults(db: Db, config: AnissiaConfig) -> Self {
        Anissia::new(db, config, system_clock())
    }

    /// Overrides the time between requests (tests).
    pub fn with_spacing(mut self, spacing: Duration) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn config(&self) -> &AnissiaConfig {
        &self.config
    }

    pub fn now(&self) -> Millis {
        (self.clock)()
    }

    /// Waits for this process's turn to send a request. With `max_wait`, a
    /// turn further away than that is not taken: [`AnissiaError::Busy`].
    async fn turn(&self, max_wait: Option<Duration>) -> Result<(), AnissiaError> {
        let now = self.now();
        let slot = self
            .pace
            .take_request_slot(
                now,
                self.spacing.as_millis() as i64,
                max_wait.map(|d| d.as_millis() as i64),
            )
            .await?;
        match slot {
            Ok(at) => {
                if at > now {
                    tokio::time::sleep(Duration::from_millis((at - now) as u64)).await;
                }
                Ok(())
            }
            Err(wait) => Err(AnissiaError::Busy {
                retry_after: Duration::from_millis(wait.max(0) as u64),
            }),
        }
    }

    /// Sends one request and returns its answer's body.
    async fn get_body(
        &self,
        path: &str,
        query: &[(&str, &str)],
        max_wait: Option<Duration>,
    ) -> Result<Vec<u8>, AnissiaError> {
        let mut url = Url::parse(&self.config.url(path))
            .map_err(|e| AnissiaError::Unreachable(e.to_string()))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        self.turn(max_wait).await?;
        let response = self
            .http
            .get(url)
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| AnissiaError::Unreachable(e.without_url().to_string()))?;

        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(Duration::from_secs)
                .unwrap_or(DEFAULT_RETRY_AFTER)
                .min(MAX_RETRY_AFTER);
            let until = self.now() + retry_after.as_millis() as i64;
            self.pace.block_requests(until).await?;
            return Err(AnissiaError::Busy { retry_after });
        }
        if !status.is_success() {
            return Err(AnissiaError::Status(status.as_u16()));
        }
        read_answer(response).await
    }

    /// Sends one request and reads the list of its answer.
    async fn get_list(
        &self,
        path: &str,
        max_wait: Option<Duration>,
    ) -> Result<Vec<serde_json::Value>, AnissiaError> {
        let body = self.get_body(path, &[], max_wait).await?;
        parse::list_of(&body).map_err(|e| AnissiaError::Invalid(e.0))
    }

    /// Asks Anissia for the schedule of `week`, bypassing the cache.
    pub async fn fetch_schedule(
        &self,
        week: u8,
        max_wait: Option<Duration>,
    ) -> Result<Vec<ScheduleEntry>, AnissiaError> {
        if week > LAST_WEEK {
            return Err(AnissiaError::NoSuchWeek(week));
        }
        let list = self
            .get_list(&format!("/anime/schedule/{week}"), max_wait)
            .await?;
        parse::schedule(&list, week).map_err(|e| AnissiaError::Invalid(e.0))
    }

    /// Asks Anissia for the captions of anime `anime_no`, bypassing the cache.
    pub async fn fetch_captions(
        &self,
        anime_no: i64,
        max_wait: Option<Duration>,
    ) -> Result<Vec<Caption>, AnissiaError> {
        let list = self
            .get_list(&format!("/anime/caption/animeNo/{anime_no}"), max_wait)
            .await?;
        Ok(parse::captions(&list))
    }

    /// Asks Anissia for page `page` (from 0) of the full anime list matching
    /// `q`, bypassing the cache. A search text is sent as it is; the caller
    /// decides what is worth asking.
    pub async fn fetch_anime_page(
        &self,
        q: &str,
        page: u32,
        max_wait: Option<Duration>,
    ) -> Result<AnimePage, AnissiaError> {
        let body = self
            .get_body(&format!("/anime/list/{page}"), &[("q", q)], max_wait)
            .await?;
        parse::anime_page(&body).map_err(|e| AnissiaError::Invalid(e.0))
    }

    /// The cached answer for `key`, or the one `load` gets and caches. Two
    /// calls for one key do not both ask: the second waits for the first.
    async fn cached<K, V, Fut>(
        &self,
        slots: &Slots<K, V>,
        key: K,
        load: impl FnOnce() -> Fut,
    ) -> Result<Fetched<V>, AnissiaError>
    where
        K: Hash + Eq,
        Fut: Future<Output = Result<Vec<V>, AnissiaError>>,
    {
        let slot = slots.slot(key);
        let mut held = slot.lock().await;
        let now = self.now();
        if let Some((at, value)) = held.as_ref() {
            if now - *at < CACHE_TTL.as_millis() as i64 {
                return Ok(Fetched {
                    value: value.clone(),
                    fetched_at: *at,
                    cached: true,
                });
            }
        }
        let value = Arc::new(load().await?);
        let at = self.now();
        *held = Some((at, value.clone()));
        Ok(Fetched {
            value,
            fetched_at: at,
            cached: false,
        })
    }

    /// The schedule of `week`: the cached answer while it is younger than
    /// [`CACHE_TTL`], otherwise Anissia's.
    pub async fn schedule(
        &self,
        week: u8,
        max_wait: Option<Duration>,
    ) -> Result<Fetched<ScheduleEntry>, AnissiaError> {
        if week > LAST_WEEK {
            return Err(AnissiaError::NoSuchWeek(week));
        }
        self.cached(&self.schedules, week, || {
            self.fetch_schedule(week, max_wait)
        })
        .await
    }

    /// The captions of anime `anime_no`, cached like [`Anissia::schedule`].
    pub async fn captions(
        &self,
        anime_no: i64,
        max_wait: Option<Duration>,
    ) -> Result<Fetched<Caption>, AnissiaError> {
        self.cached(&self.captions, anime_no, || {
            self.fetch_captions(anime_no, max_wait)
        })
        .await
    }

    /// Page `page` (from 0) of the full anime list matching `q`: the cached
    /// answer while it is younger than [`CACHE_TTL`], otherwise Anissia's.
    pub async fn search_anime(
        &self,
        q: &str,
        page: u32,
        max_wait: Option<Duration>,
    ) -> Result<FetchedPage, AnissiaError> {
        let key = (q.to_owned(), page);
        let now = self.now();
        if let Some((at, value)) = self.kept_page(&key, now) {
            return Ok(FetchedPage {
                page: value,
                fetched_at: at,
                cached: true,
            });
        }
        let value = Arc::new(self.fetch_anime_page(q, page, max_wait).await?);
        let at = self.now();
        self.keep_page(key, at, value.clone());
        Ok(FetchedPage {
            page: value,
            fetched_at: at,
            cached: false,
        })
    }

    fn kept_page(&self, key: &(String, u32), now: Millis) -> Option<(Millis, Arc<AnimePage>)> {
        let pages = self.pages.lock().unwrap_or_else(|e| e.into_inner());
        pages
            .get(key)
            .filter(|(at, _)| now - *at < CACHE_TTL.as_millis() as i64)
            .cloned()
    }

    fn keep_page(&self, key: (String, u32), at: Millis, value: Arc<AnimePage>) {
        let mut pages = self.pages.lock().unwrap_or_else(|e| e.into_inner());
        pages.retain(|_, (kept, _)| at - *kept < CACHE_TTL.as_millis() as i64);
        while pages.len() >= MAX_CACHED_PAGES {
            let Some(oldest) = pages
                .iter()
                .min_by_key(|(_, (kept, _))| *kept)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            pages.remove(&oldest);
        }
        pages.insert(key, (at, value));
    }
}

/// The body of an answer, refused once it passes [`MAX_ANSWER_BYTES`].
async fn read_answer(mut response: reqwest::Response) -> Result<Vec<u8>, AnissiaError> {
    let too_large = || AnissiaError::Invalid(format!("larger than {MAX_ANSWER_BYTES} bytes"));
    if response
        .content_length()
        .is_some_and(|n| n > MAX_ANSWER_BYTES as u64)
    {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| AnissiaError::Unreachable(e.without_url().to_string()))?
    {
        if body.len() + chunk.len() > MAX_ANSWER_BYTES {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
