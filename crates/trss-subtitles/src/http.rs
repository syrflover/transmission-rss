//! What the sources over HTTP share (`docs/specs/jobs.md`, 공통 수신 결과와
//! 실패 분류): a client with no cookie and no `Referer` that follows a
//! redirect only where the source allows, a pace per host, the reading of a
//! post's page, and the taking of a file's answer as its bytes.
//!
//! No address leaves here in a reason: an error of the HTTP client is told by
//! its kind only, since its text carries the address.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use reqwest::{header, Response, StatusCode};
use url::Url;

use crate::{Body, Failure, FailureKind, Fetch, FileInfo, Snapshot, FILE_DEADLINE, MAX_FILE_BYTES};

/// The least time between two requests to one host.
pub const SPACING: Duration = Duration::from_secs(1);

/// The most redirects one request follows.
const MAX_REDIRECTS: usize = 10;

/// How long a post may take to come, or a request for a file's information.
pub(crate) const POST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the connection may stay silent while a file comes.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The most of a post's page that is read.
const MAX_PAGE: usize = 8 * 1024 * 1024;
/// The most of an error answer that is read to measure it.
pub(crate) const MAX_ERROR_BODY: usize = 1024 * 1024;

/// A browser's `User-Agent`: the blogs' pages are made for browsers.
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                          Chrome/140.0.0.0 Safari/537.36";

/// The snapshot's name for an answer's `Last-Modified`.
pub const LAST_MODIFIED: &str = "last_modified";

/// What a source holds its requests to.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The least time between two requests to one host.
    pub spacing: Duration,
    /// The most bytes of one file.
    pub max_file: u64,
    /// How long one file may take, from its request to its last byte.
    pub file_deadline: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            spacing: SPACING,
            max_file: MAX_FILE_BYTES,
            file_deadline: FILE_DEADLINE,
        }
    }
}

/// Where a source's requests may go. Over the network only `https`; the
/// tests' server ([`crate::testing`]) speaks plain `http` on its own port.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Reach {
    pub(crate) plain_http: bool,
}

impl Reach {
    pub(crate) const NETWORK: Reach = Reach { plain_http: false };

    pub(crate) fn scheme(self, url: &Url) -> bool {
        url.scheme() == "https" || (self.plain_http && url.scheme() == "http")
    }
}

/// When the next request to each host may go.
#[derive(Debug)]
pub(crate) struct Pace {
    spacing: Duration,
    next: Mutex<HashMap<String, Instant>>,
}

impl Pace {
    pub(crate) fn new(spacing: Duration) -> Pace {
        Pace {
            spacing,
            next: Mutex::default(),
        }
    }

    /// Waits until a request to `url`'s host may go, and books the next one.
    pub(crate) async fn wait(&self, url: &Url) {
        let host = url.host_str().unwrap_or_default().to_owned();
        let wait = {
            let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            let at = next
                .get(&host)
                .copied()
                .filter(|at| *at > now)
                .unwrap_or(now);
            next.insert(host, at + self.spacing);
            at - now
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

/// A client with no `Referer`, which would carry an address to the next
/// host, and no cookie (it keeps none). A redirect is followed, ten at most,
/// only when `follow(first, next)` allows it, with `first` the address the
/// request was made to; any other is the redirect's answer.
pub(crate) fn client(
    builder: reqwest::ClientBuilder,
    follow: impl Fn(&Url, &Url) -> bool + Send + Sync + 'static,
) -> reqwest::Client {
    let redirects = reqwest::redirect::Policy::custom(move |attempt| {
        let allowed = attempt
            .previous()
            .first()
            .is_some_and(|first| follow(first, attempt.url()));
        match allowed && attempt.previous().len() <= MAX_REDIRECTS {
            true => attempt.follow(),
            false => attempt.stop(),
        }
    });
    builder
        .user_agent(USER_AGENT)
        .referer(false)
        .redirect(redirects)
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .expect("a client with timeouts builds")
}

/// Reads the page of the post at `post`: its text, or why not. `404` and
/// `410` are a post gone; `429` and `5xx` a network failure; a redirect the
/// client did not follow, or any other status, a post that has changed.
pub(crate) async fn get_page(
    http: &reqwest::Client,
    pace: &Pace,
    post: &Url,
) -> Result<String, Failure> {
    pace.wait(post).await;
    let response = http
        .get(post.clone())
        .timeout(POST_TIMEOUT)
        .send()
        .await
        .map_err(|e| network_failure(&e, "게시물에 연결하지 못했어요"))?;
    let status = response.status();
    let content_type = media_type(&response);
    if status != StatusCode::OK {
        let size = error_size(response).await;
        let (kind, reason) = match status.as_u16() {
            404 | 410 => (FailureKind::Missing, "게시물이 없어요"),
            300..=399 => (
                FailureKind::Changed,
                "게시물이 따라갈 수 없는 곳으로 넘기려 했어요",
            ),
            429 | 500..=599 => (FailureKind::Network, "사이트가 게시물을 주지 못했어요"),
            _ => (FailureKind::Changed, "게시물을 열 수 없어요"),
        };
        return Err(
            Failure::new(kind, format!("{reason} (HTTP {})", status.as_u16())).with_response(
                Some(status.as_u16()),
                content_type,
                size,
            ),
        );
    }
    let page = read_capped(response, MAX_PAGE)
        .await
        .map_err(|e| network_failure(&e, "게시물을 읽는 도중에 연결이 끊겼어요"))?;
    Ok(String::from_utf8_lossy(&page).into_owned())
}

/// Starts receiving the file at the signed address `locator`, with no cookie
/// (the client keeps none) and no `Referer` (the client sends none, on its
/// redirects as well), held to `limits`. A refusal with a web page (`400`,
/// `403`, `404` or `410` with `text/html`) is an expired address
/// ([`FailureKind::Expired`]), which the source reads its post again for.
pub(crate) async fn get_file(
    http: &reqwest::Client,
    pace: &Pace,
    limits: Limits,
    locator: &Url,
) -> Result<Fetch, Failure> {
    pace.wait(locator).await;
    let deadline = tokio::time::Instant::now() + limits.file_deadline;
    let response = tokio::time::timeout_at(deadline, http.get(locator.clone()).send())
        .await
        .map_err(|_| deadline_failure(limits.file_deadline))?
        .map_err(|e| network_failure(&e, "파일 주소에 연결하지 못했어요"))?;
    if response.status() != StatusCode::OK {
        return Err(refusal(response).await);
    }
    take_file(response, limits, deadline)
}

/// The failure a file's answer that is not `200` comes to: a refusal with a
/// web page (`400`, `403`, `404` or `410` with `text/html`) is an expired
/// address ([`FailureKind::Expired`]), which the source reads its post again
/// for.
pub(crate) async fn refusal(response: Response) -> Failure {
    let status = response.status();
    let content_type = media_type(&response);
    let html = content_type.as_deref() == Some("text/html");
    let size = error_size(response).await;
    let (kind, reason) = match status.as_u16() {
        400 | 403 | 404 | 410 if html => (FailureKind::Expired, "파일 주소가 거절됐어요"),
        404 | 410 => (FailureKind::Missing, "파일이 없어요"),
        300..=399 => (
            FailureKind::Changed,
            "파일 주소가 따라갈 수 없는 곳으로 넘기려 했어요",
        ),
        429 | 500..=599 => (FailureKind::Network, "사이트가 파일을 주지 못했어요"),
        _ => (FailureKind::Changed, "파일 주소가 뜻밖의 답을 줬어요"),
    };
    Failure::new(kind, format!("{reason} (HTTP {})", status.as_u16())).with_response(
        Some(status.as_u16()),
        content_type,
        size,
    )
}

/// The whole size of the file at the signed address `locator` without
/// receiving it: a `GET` with `Range: bytes=0-0`, which answers `206` with
/// the total in `Content-Range` (`bytes 0-0/11724`) and one byte. Tistory's
/// CDN answers its `HEAD` with `404` and gives neither `Last-Modified` nor
/// `ETag` (2026-10-03), so the total is all it tells. A server that ignores
/// the range and answers `200` gives the size in `Content-Length`; its body is
/// not read. The request has no cookie and no `Referer`, like a receipt.
pub(crate) async fn range_total(
    http: &reqwest::Client,
    pace: &Pace,
    locator: &Url,
) -> Result<FileInfo, Failure> {
    pace.wait(locator).await;
    let response = http
        .get(locator.clone())
        .header(header::RANGE, "bytes=0-0")
        .timeout(POST_TIMEOUT)
        .send()
        .await
        .map_err(|e| network_failure(&e, "파일 주소에 연결하지 못했어요"))?;
    let status = response.status();
    let size = match status {
        StatusCode::PARTIAL_CONTENT => response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(content_range_total),
        StatusCode::OK => header_length(&response),
        _ => return Err(refusal(response).await),
    };
    match size {
        Some(size) => Ok(FileInfo {
            size: Some(size),
            last_modified: None,
        }),
        None => Err(Failure::new(
            FailureKind::Changed,
            "사이트가 파일의 전체 크기를 알려주지 않았어요",
        )
        .with_response(Some(status.as_u16()), media_type(&response), None)),
    }
}

/// The total of a `Content-Range` (`bytes 0-0/11724`); none when the server
/// does not know it (`*`).
fn content_range_total(value: &str) -> Option<u64> {
    value.rsplit_once('/')?.1.trim().parse().ok()
}

/// The answer's `Content-Length` header (a `HEAD` answer has no body for
/// [`Response::content_length`] to measure), unless the body is encoded: its
/// length would be the encoding's.
pub(crate) fn header_length(response: &Response) -> Option<u64> {
    let encoded = response
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| {
            let v = String::from_utf8_lossy(v.as_bytes());
            !v.trim().is_empty() && !v.trim().eq_ignore_ascii_case("identity")
        });
    if encoded {
        return None;
    }
    response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok()?.trim().parse().ok())
}

/// Takes a `200` answer as the file's bytes, held to `limits` from
/// `deadline`: refused when it is encoded (its bytes and length would be the
/// encoding's; the client decodes nothing) or announces more than the file may
/// have. Its `Last-Modified` goes to the snapshot.
pub(crate) fn take_file(
    response: Response,
    limits: Limits,
    deadline: tokio::time::Instant,
) -> Result<Fetch, Failure> {
    let status = response.status().as_u16();
    let content_type = media_type(&response);
    let expected_size: Option<u64> = response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok()?.parse().ok());
    let encoded = response
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| {
            let v = String::from_utf8_lossy(v.as_bytes());
            let v = v.trim();
            !v.is_empty() && !v.eq_ignore_ascii_case("identity")
        });
    let not_a_file = |reason: String| {
        Failure::new(FailureKind::NotAFile, reason).with_response(
            Some(status),
            content_type.clone(),
            expected_size,
        )
    };
    if encoded {
        return Err(not_a_file(
            "사이트가 파일을 압축 등으로 인코딩해(Content-Encoding) 보내서 받은 바이트를 파일로 쓸 수 없어요"
                .to_owned(),
        ));
    }
    if let Some(size) = expected_size.filter(|s| *s > limits.max_file) {
        return Err(not_a_file(format!(
            "사이트가 알린 크기({size}바이트)가 받을 수 있는 크기({})를 넘어요",
            crate::size_limit_text(limits.max_file)
        )));
    }
    let mut snapshot = Snapshot::default();
    if let Some(modified) = response
        .headers()
        .get(header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
    {
        snapshot.push(LAST_MODIFIED, modified);
    }
    Ok(Fetch::new(
        expected_size,
        Some(status),
        content_type,
        snapshot,
        Body::Http(response),
        limits.max_file,
        Some((deadline, limits.file_deadline)),
    ))
}

/// A file that did not come whole within its deadline.
pub(crate) fn deadline_failure(deadline: Duration) -> Failure {
    let text = match deadline.as_secs() {
        s if s >= 60 => format!("{}분", s.div_ceil(60)),
        _ => format!("{}초", deadline.as_secs_f64().ceil().max(1.0) as u64),
    };
    Failure::new(
        FailureKind::Network,
        format!("파일을 {text} 안에 다 받지 못했어요"),
    )
}

/// A failure of the HTTP client as a network failure, told by its kind: its
/// own text names the address.
pub(crate) fn network_failure(err: &reqwest::Error, otherwise: &str) -> Failure {
    let reason = match () {
        _ if err.is_timeout() => "응답이 없어 시간이 다 됐어요",
        _ if err.is_connect() => "사이트에 연결하지 못했어요",
        _ => otherwise,
    };
    Failure::new(FailureKind::Network, reason)
}

/// The answer's media type, lower-case and without its parameters.
pub(crate) fn media_type(response: &Response) -> Option<String> {
    let value = response
        .headers()
        .get(header::CONTENT_TYPE)?
        .to_str()
        .ok()?;
    let essence = value.split(';').next()?.trim().to_ascii_lowercase();
    (!essence.is_empty()).then_some(essence.chars().take(100).collect())
}

/// How many bytes an error answer had, read up to a bound.
pub(crate) async fn error_size(response: Response) -> Option<u64> {
    let announced = response.content_length();
    match read_capped(response, MAX_ERROR_BODY).await {
        Ok(body) if body.len() < MAX_ERROR_BODY => Some(body.len() as u64),
        _ => announced,
    }
}

pub(crate) async fn read_capped(response: Response, cap: usize) -> Result<Vec<u8>, reqwest::Error> {
    trss_core::response::read_cut(cap, response, Response::chunk).await
}
