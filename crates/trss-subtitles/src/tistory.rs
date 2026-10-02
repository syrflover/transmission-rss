//! Tistory's general attachments (`docs/specs/subtitles.md`, 출처별 다운로드;
//! `docs/specs/jobs.md`, 출처별 단계 초안): a post on `<blog>.tistory.com`
//! read over HTTPS with no cookie, and each file of its `.fileblock`s received
//! from the signed address the post gives, also with no cookie and no
//! `Referer`, not even on a redirect.
//!
//! - Opening reads every `.fileblock a[href]` that is an `https` address on
//!   the CDN (`*.kakaocdn.net`, `*.daumcdn.net`; [`cdn_host`]); any other link
//!   there is no Tistory attachment. Each gives its name (`.filename .name`,
//!   else the address's last segment) and the size the post shows (`.size`).
//!   Every file the post offers is received (a series ZIP, the episodes'
//!   SMIs, a font ZIP alike); which one is an episode's is the analysis's
//!   question. The file's key is the address's host and path without its
//!   query, which holds the signature.
//! - A post with no such fileblock is read for where its subtitle is, inside
//!   its body ([`BODY`]) only, since the skin around it has its own links and
//!   thumbnails: a Google Drive file link, or an image on `*.kakaocdn.net`
//!   ending in `.png` (WinPNG), puts it elsewhere ([`Opened::Elsewhere`]).
//!   With neither, or with no body this module knows, the post has
//!   [`FailureKind::Changed`].
//! - A redirect is followed only to where the request may go itself: from a
//!   post to an `https` blog on `tistory.com`, from a file to the CDN, ten at
//!   most. Anything else is the redirect's answer, [`FailureKind::Changed`].
//! - A file is held to [`Limits`]: its bytes to [`crate::MAX_FILE_BYTES`]
//!   (past them it is [`FailureKind::NotAFile`]) and its whole receipt to
//!   [`crate::FILE_DEADLINE`] (a [`FailureKind::Network`] failure). An answer
//!   with a `Content-Encoding` other than `identity` is not taken as the file
//!   ([`FailureKind::NotAFile`]): its length and bytes are the encoding's.
//! - A signed address refused with a web page (`400`, `403`, `404` or `410`
//!   with `text/html`; its size says nothing, 150 and 552 bytes were both
//!   seen) has expired: the post is read once more for a new one. A file gone
//!   from the post then is [`FailureKind::Missing`]; a new address refused as
//!   well is [`FailureKind::Expired`].
//! - Requests to one host are at least [`SPACING`] apart, so a post of 25
//!   files does not come as a burst.
//!
//! No address leaves this module but in a [`PostFile`]'s hidden locator, and
//! no reason names one: an error of the HTTP client is told by its kind only,
//! since its text carries the address.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use percent_encoding::percent_decode_str;
use reqwest::{header, Response, StatusCode};
use scraper::{Html, Selector};
use url::Url;

use crate::{
    Body, Failure, FailureKind, Fetch, Opened, PostFile, Snapshot, FILE_DEADLINE, MAX_FILE_BYTES,
};

/// The hosts this source reads: `<blog>.tistory.com`.
pub fn reads(host: &str) -> bool {
    host.strip_suffix(".tistory.com")
        .is_some_and(|blog| !blog.is_empty() && blog != "www")
}

/// Whether `host` serves Tistory's attachments and images: `*.kakaocdn.net`
/// or `*.daumcdn.net`.
pub fn cdn_host(host: &str) -> bool {
    [".kakaocdn.net", ".daumcdn.net"]
        .iter()
        .any(|suffix| host.strip_suffix(suffix).is_some_and(|sub| !sub.is_empty()))
}

/// The least time between two requests to one host.
pub const SPACING: Duration = Duration::from_secs(1);

/// The most redirects one request follows.
const MAX_REDIRECTS: usize = 10;

/// A post's body, the containers seen on real posts (`docs/specs/jobs.md`,
/// 공통 수신 결과와 실패 분류): the editor's `tt_article_useless_p_margin
/// contents_style` in every skin seen, inside `#article-view` in some.
pub const BODY: &str = ".tt_article_useless_p_margin, .contents_style, #article-view";

/// How long a post may take to come.
const POST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the connection may stay silent while a file comes.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The most of a post's page that is read.
const MAX_PAGE: usize = 8 * 1024 * 1024;
/// The most of an error answer that is read to measure it.
const MAX_ERROR_BODY: usize = 1024 * 1024;

/// A browser's `User-Agent`: the blog's pages are made for browsers.
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                          Chrome/140.0.0.0 Safari/537.36";

/// The snapshot's names.
pub const POST_MODIFIED: &str = "article:modified_time";
pub const DECLARED_SIZE: &str = "declared_size";
pub const LAST_MODIFIED: &str = "last_modified";

/// What the source holds its requests to.
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

/// Where the source's requests may go. Over the network only `https`; the
/// tests' server ([`crate::testing`]) speaks plain `http` on its own port.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Reach {
    pub(crate) plain_http: bool,
}

impl Reach {
    pub(crate) const NETWORK: Reach = Reach { plain_http: false };

    fn scheme(self, url: &Url) -> bool {
        url.scheme() == "https" || (self.plain_http && url.scheme() == "http")
    }

    /// A file's address: on the CDN.
    pub(crate) fn file(self, url: &Url) -> bool {
        self.scheme(url) && url.domain().is_some_and(cdn_host)
    }

    /// A post's address: a blog on `tistory.com`.
    fn post(self, url: &Url) -> bool {
        self.scheme(url) && url.domain().is_some_and(reads)
    }
}

/// Reads Tistory posts and their attachments. Cheap to clone; the clones share
/// the client and the pace.
#[derive(Clone)]
pub struct TistorySource {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    limits: Limits,
    reach: Reach,
    /// When the next request to each host may go.
    next: Mutex<HashMap<String, Instant>>,
}

impl std::fmt::Debug for TistorySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TistorySource")
    }
}

impl Default for TistorySource {
    fn default() -> TistorySource {
        TistorySource::new()
    }
}

impl TistorySource {
    /// The source over the network.
    pub fn new() -> TistorySource {
        let reach = Reach::NETWORK;
        TistorySource::over(
            client(reqwest::Client::builder(), reach),
            Limits::default(),
            reach,
        )
    }

    /// The source with its own client, limits and reach (the tests'
    /// [`crate::testing`] server).
    pub(crate) fn over(http: reqwest::Client, limits: Limits, reach: Reach) -> TistorySource {
        TistorySource {
            inner: Arc::new(Inner {
                http,
                limits,
                reach,
                next: Mutex::default(),
            }),
        }
    }

    pub(crate) async fn open(&self, post: &Url) -> Result<Opened, Failure> {
        let page = self.read_post(post).await?;
        read_page(post, &page, self.inner.reach)
    }

    pub(crate) async fn fetch(&self, post: &Url, file: &PostFile) -> Result<Fetch, Failure> {
        let first = match file.locator() {
            Some(locator) => self.get_file(locator).await,
            None => Err(Failure::new(FailureKind::Expired, "받을 주소가 없어요")),
        };
        let expired = match first {
            Err(failure) if failure.kind == FailureKind::Expired => failure,
            other => return other,
        };
        // Once more, with the address the post gives now.
        let again = match self.open(post).await {
            Ok(Opened::Files(files)) => files.into_iter().find(|f| f.key == file.key),
            Err(failure) if failure.kind == FailureKind::Network => return Err(failure),
            _ => None,
        };
        let Some(locator) = again.as_ref().and_then(PostFile::locator) else {
            return Err(Failure::new(
                FailureKind::Missing,
                "주소가 거절돼 게시물을 다시 읽었지만 이 파일이 없어요",
            )
            .with_response(expired.status, expired.content_type, expired.size));
        };
        match self.get_file(locator).await {
            Err(failure) if failure.kind == FailureKind::Expired => Err(Failure {
                reason: format!(
                    "게시물을 다시 읽어 얻은 주소도 거절됐어요 (HTTP {})",
                    failure.status.unwrap_or_default()
                ),
                ..failure
            }),
            other => other,
        }
    }

    /// Waits until a request to `url`'s host may go, and books the next one.
    async fn pace(&self, url: &Url) {
        let host = url.host_str().unwrap_or_default().to_owned();
        let wait = {
            let mut next = self.inner.next.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            let at = next
                .get(&host)
                .copied()
                .filter(|at| *at > now)
                .unwrap_or(now);
            next.insert(host, at + self.inner.limits.spacing);
            at - now
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }

    async fn read_post(&self, post: &Url) -> Result<String, Failure> {
        self.pace(post).await;
        let response = self
            .inner
            .http
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

    async fn get_file(&self, locator: &Url) -> Result<Fetch, Failure> {
        self.pace(locator).await;
        let limits = self.inner.limits;
        let deadline = tokio::time::Instant::now() + limits.file_deadline;
        // No cookie (the client keeps none) and no `Referer` (the client is
        // built with none, which holds for its redirects as well).
        let response =
            tokio::time::timeout_at(deadline, self.inner.http.get(locator.clone()).send())
                .await
                .map_err(|_| deadline_failure(limits.file_deadline))?
                .map_err(|e| network_failure(&e, "파일 주소에 연결하지 못했어요"))?;
        let status = response.status();
        let content_type = media_type(&response);
        if status != StatusCode::OK {
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
            return Err(
                Failure::new(kind, format!("{reason} (HTTP {})", status.as_u16())).with_response(
                    Some(status.as_u16()),
                    content_type,
                    size,
                ),
            );
        }
        let expected_size: Option<u64> = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok()?.parse().ok());
        // The client decodes nothing: an encoded answer's bytes and length
        // are the encoding's, not the file's.
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
                Some(status.as_u16()),
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
            Some(status.as_u16()),
            content_type,
            snapshot,
            Body::Http(response),
            limits.max_file,
            Some((deadline, limits.file_deadline)),
        ))
    }
}

/// The client every request of the source goes through: no `Referer`, which
/// would carry a signed address to the next host, and redirects only to where
/// the first request may go ([`Reach`]).
pub(crate) fn client(builder: reqwest::ClientBuilder, reach: Reach) -> reqwest::Client {
    let redirects = reqwest::redirect::Policy::custom(move |attempt| {
        let from_file = attempt.previous().first().is_some_and(|u| reach.file(u));
        let allowed = match from_file {
            true => reach.file(attempt.url()),
            false => reach.post(attempt.url()),
        };
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
fn media_type(response: &Response) -> Option<String> {
    let value = response
        .headers()
        .get(header::CONTENT_TYPE)?
        .to_str()
        .ok()?;
    let essence = value.split(';').next()?.trim().to_ascii_lowercase();
    (!essence.is_empty()).then_some(essence.chars().take(100).collect())
}

/// How many bytes an error answer had, read up to a bound.
async fn error_size(response: Response) -> Option<u64> {
    let announced = response.content_length();
    match read_capped(response, MAX_ERROR_BODY).await {
        Ok(body) if body.len() < MAX_ERROR_BODY => Some(body.len() as u64),
        _ => announced,
    }
}

async fn read_capped(mut response: Response, cap: usize) -> Result<Vec<u8>, reqwest::Error> {
    let mut body = Vec::new();
    while let Some(piece) = response.chunk().await? {
        body.extend_from_slice(&piece);
        if body.len() >= cap {
            body.truncate(cap);
            break;
        }
    }
    Ok(body)
}

/// What a post's page offers (see the module docs).
pub(crate) fn read_page(post: &Url, page: &str, reach: Reach) -> Result<Opened, Failure> {
    let html = Html::parse_document(page);
    let select = |css: &str| Selector::parse(css).expect("a valid selector");
    let modified = html
        .select(&select(r#"meta[property="article:modified_time"]"#))
        .find_map(|m| m.value().attr("content"))
        .map(str::trim)
        .filter(|v| !v.is_empty());

    let text_of = |el: scraper::ElementRef<'_>, css: &str| {
        el.select(&select(css))
            .next()
            .map(|e| e.text().collect::<String>().trim().to_owned())
            .filter(|t| !t.is_empty())
    };
    let mut files: Vec<PostFile> = Vec::new();
    for link in html.select(&select(".fileblock a[href]")) {
        let Some(url) = link
            .value()
            .attr("href")
            .and_then(|href| post.join(href.trim()).ok())
            .filter(|u| reach.file(u))
        else {
            continue;
        };
        let key = format!(
            "tistory:{}{}",
            url.host_str().unwrap_or_default(),
            url.path()
        );
        if files.iter().any(|f| f.key == key) {
            continue;
        }
        let Some(name) = text_of(link, ".filename .name").or_else(|| last_segment(&url)) else {
            continue;
        };
        let mut file = PostFile::new(key, name);
        file.size_text = text_of(link, ".size");
        if let Some(modified) = modified {
            file.snapshot.push(POST_MODIFIED, modified);
        }
        if let Some(size) = &file.size_text {
            file.snapshot.push(DECLARED_SIZE, size.clone());
        }
        files.push(file.with_locator(url));
    }
    if !files.is_empty() {
        return Ok(Opened::Files(files));
    }

    // Only the body says where the subtitle is: the skin around it has
    // links and thumbnails of its own.
    let bodies: Vec<scraper::ElementRef<'_>> = html.select(&select(BODY)).collect();
    if bodies.is_empty() {
        return Err(Failure::new(
            FailureKind::Changed,
            "게시물에서 첨부 파일도 본문도 찾지 못했어요",
        ));
    }
    if bodies
        .iter()
        .any(|body| body.html().contains("drive.google.com/file/d/"))
    {
        return Ok(Opened::Elsewhere {
            reason:
                "자막이 Google Drive 링크로 올라와 있어요. Google Drive에서 받는 방법은 아직 없어요"
                    .to_owned(),
        });
    }
    let image = select("img");
    let winpng = bodies
        .iter()
        .flat_map(|body| body.select(&image))
        .any(|img| {
            ["src", "data-src"]
                .into_iter()
                .filter_map(|a| img.value().attr(a))
                .filter_map(|src| post.join(src.trim()).ok())
                .any(|u| {
                    u.domain().is_some_and(|h| h.ends_with(".kakaocdn.net"))
                        && u.path().to_ascii_lowercase().ends_with(".png")
                })
        });
    if winpng {
        return Ok(Opened::Elsewhere {
            reason: "자막이 게시물의 PNG 이미지(WinPNG)에 들어 있어요. 이미지에서 꺼내는 방법은 아직 없어요"
                .to_owned(),
        });
    }
    Err(Failure::new(
        FailureKind::Changed,
        "게시물에서 첨부 파일을 찾지 못했어요",
    ))
}

/// The address's last path segment, decoded: a name when the post gives none.
fn last_segment(url: &Url) -> Option<String> {
    let segment = url.path_segments()?.rev().find(|s| !s.is_empty())?;
    let name = percent_decode_str(segment)
        .decode_utf8_lossy()
        .trim()
        .to_owned();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post() -> Url {
        Url::parse("https://sumomomo.tistory.com/492").unwrap()
    }

    /// A fileblock as Tistory writes it, with its `&amp;`s.
    fn fileblock(path: &str, name: Option<&str>, size: &str) -> String {
        let name = name
            .map(|n| format!(r#"<div class="filename"><span class="name">{n}</span></div>"#))
            .unwrap_or_default();
        format!(
            r#"<figure class="fileblock" data-ke-align="alignCenter"><a href="https://blog.kakaocdn.net/dna/AAA/BBB/CCC/{path}?credential=CRED&amp;expires=1793458799&amp;allow_ip=&amp;allow_referer=&amp;signature=SIG%3D&amp;attach=1&amp;knm=tfile.zip" class=""><div class="image"></div><div class="desc">{name}<div class="size">{size}</div></div></a></figure>"#
        )
    }

    fn read_page_(post: &Url, page: &str) -> Result<Opened, Failure> {
        read_page(post, page, Reach::NETWORK)
    }

    /// A post's page as harne1 739 and felia 1187 are made: the body in the
    /// editor's container, a picture on the CDN in it.
    fn page(body: &str) -> String {
        format!(
            r#"<!doctype html><html><head><meta property="article:modified_time" content="2026-09-28T00:13:41+09:00"></head>
<body><div class="tt_article_useless_p_margin contents_style"><p><img src="https://blog.kakaocdn.net/dna/P/Q/R/img.png?credential=x&amp;signature=y"></p>{body}</div></body></html>"#
        )
    }

    /// A body with no picture, inside `#article-view` as sumomomo 491's is,
    /// with the skin's own links and thumbnails around it.
    fn plain_page(body: &str) -> String {
        format!(
            r#"<!doctype html><html><body>
<div class="header"><img src="https://blog.kakaocdn.net/dna/S/K/N/logo.png"><a href="https://drive.google.com/file/d/skin/view">skin</a></div>
<div id="article-view" class="article-view"><div class="tt_article_useless_p_margin contents_style">{body}</div></div>
<div class="related"><img src="https://img1.daumcdn.net/thumb/R750x0/?fname=https%3A%2F%2Fblog.kakaocdn.net%2Fdna%2Fa%2Fb%2Fimg.png"></div>
</body></html>"#
        )
    }

    #[test]
    fn only_blogs_on_tistory_are_read() {
        assert!(reads("sumomomo.tistory.com"));
        assert!(!reads("tistory.com"));
        assert!(!reads("www.tistory.com"));
        assert!(!reads("tistory.com.example"));
        assert!(!reads("blog.naver.com"));
    }

    #[test]
    fn a_fileblock_off_the_cdn_or_not_https_is_no_attachment() {
        assert!(cdn_host("blog.kakaocdn.net") && cdn_host("t1.daumcdn.net"));
        assert!(!cdn_host("kakaocdn.net") && !cdn_host("evilkakaocdn.net"));
        assert!(!cdn_host("blog.kakaocdn.net.example"));
        let link = |href: &str| {
            format!(
                r#"<figure class="fileblock"><a href="{href}"><div class="filename"><span class="name">a.zip</span></div></a></figure>"#
            )
        };
        let blocks = [
            link("https://example.com/dna/a.zip?signature=S"),
            link("http://blog.kakaocdn.net/dna/A/B/C/a.zip?signature=S"),
            link("https://kakaocdn.net.example/dna/a.zip"),
            link("ftp://blog.kakaocdn.net/a.zip"),
            link("/dna/a.zip"),
        ]
        .concat();
        // No fileblock left: the body is read as for a post with none.
        let failure = read_page_(&post(), &plain_page(&blocks)).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert!(matches!(
            read_page_(&post(), &page(&blocks)),
            Ok(Opened::Elsewhere { .. })
        ));
        // Beside one on the CDN, only that one is offered.
        let both = format!("{blocks}{}", fileblock("a.zip", Some("a.zip"), "1KB"));
        let Ok(Opened::Files(files)) = read_page_(&post(), &plain_page(&both)) else {
            panic!("files");
        };
        assert_eq!(files.len(), 1);
        assert!(files[0].key.starts_with("tistory:blog.kakaocdn.net/dna/"));
    }

    #[test]
    fn a_fileblock_gives_its_name_size_and_a_key_without_the_signature() {
        let html = page(&fileblock(
            "Seihantai%20-%2024.zip",
            Some("Seihantai - 24.zip"),
            "0.01MB",
        ));
        let Ok(Opened::Files(files)) = read_page_(&post(), &html) else {
            panic!("files");
        };
        assert_eq!(files.len(), 1);
        let file = &files[0];
        assert_eq!(file.name, "Seihantai - 24.zip");
        assert_eq!(
            file.key,
            "tistory:blog.kakaocdn.net/dna/AAA/BBB/CCC/Seihantai%20-%2024.zip"
        );
        assert_eq!(file.size_text.as_deref(), Some("0.01MB"));
        assert_eq!(
            file.snapshot.entries(),
            [
                (
                    POST_MODIFIED.to_owned(),
                    "2026-09-28T00:13:41+09:00".to_owned()
                ),
                (DECLARED_SIZE.to_owned(), "0.01MB".to_owned())
            ]
        );
        // The address is unescaped and whole, and kept out of sight.
        let locator = file.locator().unwrap().as_str();
        assert!(locator.contains("&expires=1793458799&") && locator.contains("signature=SIG%3D"));
        assert!(!format!("{file:?}").contains("signature"));
    }

    #[test]
    fn every_file_of_a_post_is_offered_once_with_a_name_from_its_address_when_none_is_shown() {
        let blocks = [
            fileblock(
                "Kami%20no%20Shizuku1-24.Zip",
                Some("Kami no Shizuku1-24.Zip"),
                "1.2MB",
            ),
            fileblock(
                "ep01.smi",
                Some("Kami no Shizuku - 01SubsPlease.smi"),
                "40KB",
            ),
            fileblock("ep01.smi", Some("again"), "40KB"),
            fileblock("Hotori%20font.zip", None, "3MB"),
        ];
        let Ok(Opened::Files(files)) = read_page_(&post(), &page(&blocks.concat())) else {
            panic!("files");
        };
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Kami no Shizuku1-24.Zip",
                "Kami no Shizuku - 01SubsPlease.smi",
                "Hotori font.zip"
            ]
        );
    }

    #[test]
    fn a_post_without_a_fileblock_points_elsewhere_or_has_changed() {
        let drive = page(
            r#"<p><a href="https://drive.google.com/file/d/1AbC/view?usp=drive_link">24화</a></p>"#,
        );
        let Ok(Opened::Elsewhere { reason }) = read_page_(&post(), &drive) else {
            panic!("elsewhere");
        };
        assert!(reason.contains("Google Drive"));

        // The page's body image on kakaocdn ending in `.png`: WinPNG.
        let Ok(Opened::Elsewhere { reason }) = read_page_(&post(), &page("")) else {
            panic!("elsewhere");
        };
        assert!(reason.contains("WinPNG"));

        let home = "<html><body><div class='entry'><img src='https://tistory1.daumcdn.net/x.jpg'></div></body></html>";
        let failure = read_page_(&post(), home).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
    }

    #[test]
    fn only_the_body_says_where_the_subtitle_is() {
        // A body with neither, among a skin with a Drive link, a PNG logo on
        // the CDN and PNG thumbnails: the post has changed.
        let failure = read_page_(&post(), &plain_page("<p>자막</p>")).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        // Inside `#article-view` as well.
        let drive = r#"<p><a href="https://drive.google.com/file/d/1AbC/view">24화</a></p>"#;
        let Ok(Opened::Elsewhere { reason }) = read_page_(&post(), &plain_page(drive)) else {
            panic!("elsewhere");
        };
        assert!(reason.contains("Google Drive"));
        let png =
            r#"<p><img data-src="https://blog.kakaocdn.net/dna/x/y/z/img.png?credential=c"></p>"#;
        let Ok(Opened::Elsewhere { reason }) = read_page_(&post(), &plain_page(png)) else {
            panic!("elsewhere");
        };
        assert!(reason.contains("WinPNG"));
        // No body this module knows: changed, whatever the page links to.
        let bodiless = format!(
            r#"<html><body><div class="entry-content">{drive}<img src="https://blog.kakaocdn.net/dna/x/y/z/img.png"></div></body></html>"#
        );
        let failure = read_page_(&post(), &bodiless).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert!(failure.reason.contains("본문"));
    }
}
