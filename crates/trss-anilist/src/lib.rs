//! The AniList client: title searches and single entries over AniList's
//! GraphQL API, and cover images from the image origins AniList's answers
//! point to (`docs/adr/0007-anilist-work-artwork.md`).
//!
//! - **Pace.** Every API request takes a slot from the database
//!   ([`RequestPace::take_request_slot`]), so the web and the worker together
//!   send at most one request every [`REQUEST_SPACING`] (30 a minute, AniList's
//!   lowest published limit). A `429` answer blocks every request until its
//!   `Retry-After` has passed.
//! - **Images only from allowed origins.** An image is fetched only from a URL
//!   an AniList answer gave whose origin is one of
//!   [`AnilistConfig::image_origins`], without following redirects, within
//!   [`MAX_IMAGE_BYTES`] and [`FETCH_TIMEOUT`]. No URL a user
//!   typed is ever fetched.
//! - **Bounded answers.** An API answer is read up to [`MAX_ANSWER_BYTES`];
//!   a longer one is refused before it is parsed.
//! - **Configurable for tests.** The API URL and the image origins come from
//!   [`AnilistConfig`], so tests point them at a local fake.

use std::time::Duration;

use reqwest::{header, redirect, StatusCode};
use serde::Deserialize;
use serde_json::json;
use url::Url;

use trss_core::{Clock, Db, DbError};

use pace::RequestPace;
use title::Candidate;

mod entry;
mod pace;
pub mod season;
pub mod title;

#[cfg(any(test, feature = "test-support"))]
pub mod fake;

pub use entry::{Airing, Entry, FuzzyDate, Sequel};

/// The largest image file accepted, uploaded or fetched: 10 MiB.
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
/// How long fetching one image may take in total.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// AniList's GraphQL endpoint.
pub const DEFAULT_API_URL: &str = "https://graphql.anilist.co";
/// Where AniList serves cover images.
pub const DEFAULT_IMAGE_ORIGINS: &[&str] = &["https://s4.anilist.co"];
/// Environment variable overriding [`DEFAULT_API_URL`] (tests and local checks).
pub const API_URL_VAR: &str = "TRSS_ANILIST_URL";
/// Environment variable overriding [`DEFAULT_IMAGE_ORIGINS`]: origins
/// (`scheme://host[:port]`) separated by commas.
pub const IMAGE_ORIGINS_VAR: &str = "TRSS_ANILIST_IMAGE_ORIGINS";

/// The time between two API requests of the app: 30 a minute.
pub const REQUEST_SPACING: Duration = Duration::from_secs(2);
/// How long one API request may take.
pub const API_TIMEOUT: Duration = Duration::from_secs(20);
/// Entries per search page (AniList's maximum).
pub const SEARCH_PAGE_SIZE: u32 = 50;
/// Pages read for one automatic search. A search with more results than this
/// is not read to its end, so it never selects automatically.
pub const MAX_SEARCH_PAGES: u32 = 4;
/// The largest API answer read. A search page of 50 entries or one entry
/// with its airing schedule is far below it.
pub const MAX_ANSWER_BYTES: usize = 2 * 1024 * 1024;
/// How long to wait when a `429` answer names no time.
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(60);
/// The longest `Retry-After` honoured as given.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(3600);

/// Where the client goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnilistConfig {
    pub api_url: Url,
    /// Origins images may be fetched from, as `scheme://host[:port]`.
    pub image_origins: Vec<String>,
}

impl Default for AnilistConfig {
    fn default() -> Self {
        AnilistConfig {
            api_url: Url::parse(DEFAULT_API_URL).expect("valid default URL"),
            image_origins: DEFAULT_IMAGE_ORIGINS
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        }
    }
}

impl AnilistConfig {
    /// Reads [`API_URL_VAR`] and [`IMAGE_ORIGINS_VAR`] through `lookup`; empty
    /// values count as unset.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let get = |key: &str| lookup(key).filter(|v| !v.trim().is_empty());
        let mut config = AnilistConfig::default();
        if let Some(url) = get(API_URL_VAR) {
            config.api_url = Url::parse(url.trim())
                .map_err(|_| format!("{API_URL_VAR} must be a URL, got {url:?}"))?;
        }
        if let Some(origins) = get(IMAGE_ORIGINS_VAR) {
            let mut list = Vec::new();
            for text in origins.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                let url = Url::parse(text)
                    .map_err(|_| format!("{IMAGE_ORIGINS_VAR} must list origins, got {text:?}"))?;
                list.push(url.origin().ascii_serialization());
            }
            config.image_origins = list;
        }
        Ok(config)
    }

    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Whether an image may be fetched from `url`.
    pub fn allows(&self, url: &Url) -> bool {
        matches!(url.scheme(), "https" | "http")
            && url.username().is_empty()
            && url.password().is_none()
            && self
                .image_origins
                .iter()
                .any(|o| *o == url.origin().ascii_serialization())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AnilistError {
    /// AniList asked to wait, or the app's own pace would make the caller wait
    /// longer than it may.
    #[error("AniList asks to wait {}s", retry_after.as_secs())]
    Busy { retry_after: Duration },
    #[error("cannot reach AniList: {0}")]
    Unreachable(String),
    #[error("AniList answered HTTP {0}")]
    Status(u16),
    #[error("AniList's answer is not usable: {0}")]
    Invalid(String),
    #[error(transparent)]
    Store(#[from] DbError),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ImageFetchError {
    #[error("the image URL is not from an allowed origin")]
    NotAllowed,
    #[error("the image is larger than the limit")]
    TooLarge,
    #[error("cannot fetch the image: {0}")]
    Unreachable(String),
    #[error("the image answered HTTP {0}")]
    Status(u16),
}

/// One page of a title search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPage {
    pub candidates: Vec<Candidate>,
    pub has_next: bool,
}

/// A whole title search, as far as it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    pub candidates: Vec<Candidate>,
    /// Whether the last page was reached.
    pub complete: bool,
}

const MEDIA_FIELDS: &str = "id title { romaji english native } synonyms format seasonYear \
     coverImage { extraLarge large medium }";

fn search_query() -> String {
    format!(
        "query ($search: String, $page: Int, $perPage: Int) {{ \
           Page(page: $page, perPage: $perPage) {{ \
             pageInfo {{ hasNextPage }} \
             media(search: $search, type: ANIME) {{ {MEDIA_FIELDS} }} }} }}"
    )
}

fn media_query() -> String {
    format!("query ($id: Int) {{ Media(id: $id, type: ANIME) {{ {MEDIA_FIELDS} }} }}")
}

#[derive(Deserialize)]
struct Title {
    romaji: Option<String>,
    english: Option<String>,
    native: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoverImage {
    extra_large: Option<String>,
    large: Option<String>,
    medium: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Media {
    id: i64,
    title: Option<Title>,
    #[serde(default)]
    synonyms: Option<Vec<Option<String>>>,
    format: Option<String>,
    season_year: Option<i32>,
    cover_image: Option<CoverImage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageData {
    page_info: PageInfo,
    media: Vec<Option<Media>>,
}

#[derive(Deserialize)]
struct SearchData {
    #[serde(rename = "Page")]
    page: PageData,
}

#[derive(Deserialize)]
struct MediaData {
    #[serde(rename = "Media")]
    media: Option<Media>,
}

#[derive(Deserialize)]
struct Answer<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Option<Vec<serde_json::Value>>,
}

/// The AniList client. Cheap to clone.
#[derive(Clone)]
pub struct Anilist {
    config: std::sync::Arc<AnilistConfig>,
    http: reqwest::Client,
    images: reqwest::Client,
    pace: RequestPace,
    clock: Clock,
    spacing: Duration,
}

impl Anilist {
    pub fn new(config: AnilistConfig, db: Db, clock: Clock) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(trss_core::USER_AGENT)
            .timeout(API_TIMEOUT)
            .redirect(redirect::Policy::none())
            .build()
            .expect("a client with a timeout and no redirects builds");
        let images = reqwest::Client::builder()
            .user_agent(trss_core::USER_AGENT)
            .timeout(FETCH_TIMEOUT)
            .redirect(redirect::Policy::none())
            .build()
            .expect("a client with a timeout and no redirects builds");
        Anilist {
            config: std::sync::Arc::new(config),
            http,
            images,
            pace: RequestPace::new(db),
            clock,
            spacing: REQUEST_SPACING,
        }
    }

    /// Overrides the time between requests (tests).
    pub fn with_spacing(mut self, spacing: Duration) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn config(&self) -> &AnilistConfig {
        &self.config
    }

    /// Waits for this process's turn to send an API request. With `max_wait`,
    /// a turn further away than that is not taken: [`AnilistError::Busy`].
    async fn turn(&self, max_wait: Option<Duration>) -> Result<(), AnilistError> {
        let now = (self.clock)();
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
            Err(wait) => Err(AnilistError::Busy {
                retry_after: Duration::from_millis(wait.max(0) as u64),
            }),
        }
    }

    /// Sends one GraphQL request. `Ok(None)` for a `404` (AniList's answer for
    /// an ID it does not have).
    pub(crate) async fn post<T: for<'de> Deserialize<'de>>(
        &self,
        query: String,
        variables: serde_json::Value,
        max_wait: Option<Duration>,
    ) -> Result<Option<T>, AnilistError> {
        self.turn(max_wait).await?;
        let response = self
            .http
            .post(self.config.api_url.clone())
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/json")
            .body(json!({ "query": query, "variables": variables }).to_string())
            .send()
            .await
            .map_err(|e| AnilistError::Unreachable(e.without_url().to_string()))?;

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
            let until = (self.clock)() + retry_after.as_millis() as i64;
            self.pace.block_requests(until).await?;
            return Err(AnilistError::Busy { retry_after });
        }
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(AnilistError::Status(status.as_u16()));
        }
        let body = read_answer(response).await?;
        let answer: Answer<T> = serde_json::from_slice(&body)
            .map_err(|e| AnilistError::Invalid(format!("unexpected shape: {e}")))?;
        if answer.errors.as_ref().is_some_and(|e| !e.is_empty()) {
            return Err(AnilistError::Invalid(
                "the answer carries errors".to_owned(),
            ));
        }
        answer
            .data
            .map(Some)
            .ok_or_else(|| AnilistError::Invalid("the answer has no data".to_owned()))
    }

    /// An allowed image URL, or `None`.
    fn allowed(&self, url: Option<String>) -> Option<String> {
        let url = url?;
        let parsed = Url::parse(&url).ok()?;
        self.config.allows(&parsed).then_some(url)
    }

    fn candidate(&self, media: Media) -> Candidate {
        let title = media.title.unwrap_or(Title {
            romaji: None,
            english: None,
            native: None,
        });
        let (large, medium) = match media.cover_image {
            Some(c) => (c.extra_large.or(c.large), c.medium),
            None => (None, None),
        };
        let cover_url = self.allowed(large);
        Candidate {
            id: media.id,
            romaji: title.romaji,
            english: title.english,
            native: title.native,
            synonyms: media
                .synonyms
                .unwrap_or_default()
                .into_iter()
                .flatten()
                .collect(),
            format: media.format,
            season_year: media.season_year,
            thumb_url: self.allowed(medium).or_else(|| cover_url.clone()),
            cover_url,
        }
    }

    /// One page (1-based) of the anime whose titles match `text`.
    pub async fn search_page(
        &self,
        text: &str,
        page: u32,
        max_wait: Option<Duration>,
    ) -> Result<SearchPage, AnilistError> {
        let data: SearchData = self
            .post(
                search_query(),
                json!({ "search": text, "page": page, "perPage": SEARCH_PAGE_SIZE }),
                max_wait,
            )
            .await?
            .ok_or(AnilistError::Status(404))?;
        let has_next = data
            .page
            .page_info
            .has_next_page
            .ok_or_else(|| AnilistError::Invalid("no hasNextPage".to_owned()))?;
        let candidates = data
            .page
            .media
            .into_iter()
            .flatten()
            .filter(|m| m.id > 0)
            .map(|m| self.candidate(m))
            .collect();
        Ok(SearchPage {
            candidates,
            has_next,
        })
    }

    /// The search for `text`, page after page up to [`MAX_SEARCH_PAGES`].
    pub async fn search_all(&self, text: &str) -> Result<Search, AnilistError> {
        let mut candidates = Vec::new();
        for page in 1..=MAX_SEARCH_PAGES {
            let answer = self.search_page(text, page, None).await?;
            candidates.extend(answer.candidates);
            if !answer.has_next {
                return Ok(Search {
                    candidates,
                    complete: true,
                });
            }
        }
        Ok(Search {
            candidates,
            complete: false,
        })
    }

    /// The anime with AniList ID `id`; `None` when AniList has none.
    pub async fn media(
        &self,
        id: i64,
        max_wait: Option<Duration>,
    ) -> Result<Option<Candidate>, AnilistError> {
        let data: Option<MediaData> = self
            .post(media_query(), json!({ "id": id }), max_wait)
            .await?;
        Ok(data
            .and_then(|d| d.media)
            .filter(|m| m.id == id)
            .map(|m| self.candidate(m)))
    }

    /// The bytes at an image URL an AniList answer gave, within the limits.
    pub async fn fetch_image(&self, url: &str) -> Result<Vec<u8>, ImageFetchError> {
        let url = Url::parse(url).map_err(|_| ImageFetchError::NotAllowed)?;
        if !self.config.allows(&url) {
            return Err(ImageFetchError::NotAllowed);
        }
        let download = async {
            let mut response = self
                .images
                .get(url)
                .send()
                .await
                .map_err(|e| ImageFetchError::Unreachable(e.without_url().to_string()))?;
            if !response.status().is_success() {
                return Err(ImageFetchError::Status(response.status().as_u16()));
            }
            if response
                .content_length()
                .is_some_and(|n| n > MAX_IMAGE_BYTES as u64)
            {
                return Err(ImageFetchError::TooLarge);
            }
            // Reserved once (the declared length, else the limit, of which only
            // what arrives is touched), so the buffer is never grown by
            // copying into a larger one beside the old.
            let declared = response.content_length();
            let mut bytes = Vec::with_capacity(declared.map_or(MAX_IMAGE_BYTES, |n| n as usize));
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|e| ImageFetchError::Unreachable(e.without_url().to_string()))?
            {
                if bytes.len() + chunk.len() > MAX_IMAGE_BYTES {
                    return Err(ImageFetchError::TooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            // A body that ended short of the length the response announced is
            // a cut image, not a smaller one.
            if declared.is_some_and(|n| n != bytes.len() as u64) {
                return Err(ImageFetchError::Unreachable(
                    "the body is not as long as announced".to_owned(),
                ));
            }
            Ok(bytes)
        };
        tokio::time::timeout(FETCH_TIMEOUT, download)
            .await
            .map_err(|_| ImageFetchError::Unreachable("timed out".to_owned()))?
    }
}

/// The body of an API answer, refused once it passes [`MAX_ANSWER_BYTES`].
async fn read_answer(mut response: reqwest::Response) -> Result<Vec<u8>, AnilistError> {
    let too_large = || AnilistError::Invalid(format!("larger than {MAX_ANSWER_BYTES} bytes"));
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
        .map_err(|e| AnilistError::Unreachable(e.without_url().to_string()))?
    {
        if body.len() + chunk.len() > MAX_ANSWER_BYTES {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_reads_overrides_and_allows_only_its_origins() {
        let config = AnilistConfig::from_lookup(|_| None).unwrap();
        assert_eq!(config, AnilistConfig::default());
        let allowed = |c: &AnilistConfig, u: &str| c.allows(&Url::parse(u).unwrap());
        assert!(allowed(
            &config,
            "https://s4.anilist.co/file/anilistcdn/media/anime/cover/large/bx1.jpg"
        ));
        assert!(!allowed(&config, "http://s4.anilist.co/x.jpg"));
        assert!(!allowed(
            &config,
            "https://s4.anilist.co.evil.example/x.jpg"
        ));
        assert!(!allowed(&config, "https://user@s4.anilist.co/x.jpg"));
        assert!(!allowed(&config, "https://example.org/x.jpg"));
        assert!(!allowed(&config, "file:///etc/passwd"));

        let config = AnilistConfig::from_lookup(|key| match key {
            API_URL_VAR => Some("http://127.0.0.1:9/graphql".to_owned()),
            IMAGE_ORIGINS_VAR => Some("http://127.0.0.1:9, https://img.example".to_owned()),
            _ => None,
        })
        .unwrap();
        assert_eq!(config.api_url.as_str(), "http://127.0.0.1:9/graphql");
        assert!(allowed(&config, "http://127.0.0.1:9/a.png"));
        assert!(allowed(&config, "https://img.example/a.png"));
        assert!(!allowed(&config, "http://127.0.0.1:10/a.png"));
        assert!(
            AnilistConfig::from_lookup(|k| (k == API_URL_VAR).then(|| "nope".to_owned())).is_err()
        );
    }
}
