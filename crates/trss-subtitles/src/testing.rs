//! A local HTTP server shaped like Tistory, Blogger, Naver blogs and Google
//! Drive, for the tests here and of the crates above (feature
//! `test-support`).
//!
//! Every name resolves to the server ([`SourceServer::source`]), so the
//! addresses keep their real shape with the server's port:
//! `http://<blog>.tistory.com:<port>/<n>` for a Tistory post,
//! `http://blog.kakaocdn.net:<port>/dna/<id>/<name>?credential=…&signature=…`
//! for its files (any CDN host, [`tistory::cdn_host`], serves them),
//! `http://<blog>.blogspot.com:<port>/<path>` for a Blogger post, and
//! `http://drive.usercontent.google.com:<port>/download?id=<id>&export=download`
//! for a Drive file (`drive.google.com/uc` redirects there as Drive does),
//! `http://erulabo.com:<port>/<number>` for an erulabo post (its download is
//! the server browser's, not served here),
//! `http://blog.naver.com:<port>/<blog>/<logNo>` for a Naver post's frame
//! (its inner page at `/PostView.naver?blogId=…&logNo=…`) and
//! `http://download.blog.naver.com:<port>/open/<file>/<token>/<name>` for
//! its attachments.
//! The posts link Drive files by their real addresses (`https://drive.google.com/file/d/<id>/view`):
//! the sources take only the ID from them. Each post and file answers from a
//! script a test sets, one answer per request, the last one again and again.
//! Every serving of a Tistory post or a Naver post's inner page signs its
//! addresses anew, as both sites do.
//! The sources it gives take plain `http`, which the sources over the network
//! refuse.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

use url::Url;

use crate::{
    blogger::BloggerSource,
    drive::Drive,
    erulabo::ErulaboSource,
    http::{Limits, Reach},
    naver::{self, NaverSource},
    tistory::{self, TistorySource},
};

/// The host of the files.
pub const CDN: &str = "blog.kakaocdn.net";

/// How a post answers one request.
#[derive(Debug, Clone)]
pub enum PostAnswer {
    /// A page with a fileblock for each file, signed anew.
    Files(Vec<FileSpec>),
    /// The same, the post's `article:modified_time` being this.
    FilesAt(Vec<FileSpec>, String),
    /// This page, `200`.
    Page(String),
    /// This status with a small web page.
    Status(u16),
    /// A Naver post's inner page attaching these files, signed anew.
    Naver(Vec<NaverFile>),
}

/// A file a Naver post attaches.
#[derive(Debug, Clone)]
pub struct NaverFile {
    pub name: String,
    /// The size the post gives (`attachFileSize`).
    pub size: usize,
    /// `maliciousCodeYn`, and `punishType` (`"0"` for none).
    pub malicious: bool,
    pub punish: String,
}

pub fn naver_file(name: &str, size: usize) -> NaverFile {
    NaverFile {
        name: name.to_owned(),
        size,
        malicious: false,
        punish: "0".to_owned(),
    }
}

/// A file a post offers.
#[derive(Debug, Clone)]
pub struct FileSpec {
    /// Its place on the CDN (`/dna/<id>/…`), the same at every serving.
    pub id: String,
    pub name: String,
    pub size_text: String,
}

pub fn spec(id: &str, name: &str, size_text: &str) -> FileSpec {
    FileSpec {
        id: id.to_owned(),
        name: name.to_owned(),
        size_text: size_text.to_owned(),
    }
}

/// How a file answers one request.
#[derive(Debug, Clone)]
pub enum FileAnswer {
    /// The bytes, `200` `application/octet-stream`.
    Bytes(Vec<u8>),
    /// An error page, `200` `text/html`.
    Page,
    /// A refused signature: `404` `text/html`, 150 bytes.
    Refused,
    /// This status with a small web page.
    Status(u16),
    /// The bytes, `200`, sent in pieces with no `Content-Length`.
    Streamed(Vec<u8>),
    /// The bytes' first piece, then nothing more, the connection left open.
    Stalled(Vec<u8>),
    /// The bytes, `200`, with `Content-Encoding: gzip` (not gzipped).
    Encoded(Vec<u8>),
    /// `302` to file `id`'s signed address on `host` (any host resolves to
    /// the server).
    Redirect { host: String, id: String },
}

/// How a Drive file answers one request.
#[derive(Debug, Clone)]
pub enum DriveAnswer {
    /// The file, `200` `application/octet-stream`, its name in
    /// `Content-Disposition` as UTF-8 bytes, its `Content-Length` and
    /// `Last-Modified`.
    File { name: String, bytes: Vec<u8> },
    /// The same file with this `Last-Modified`.
    FileAt {
        name: String,
        bytes: Vec<u8>,
        modified: String,
    },
    /// The same file with no `Last-Modified`.
    Undated { name: String, bytes: Vec<u8> },
    /// No such file: `404` `text/html`, 1,652 bytes.
    Missing,
    /// The page that asks to confirm the download of a file too large to
    /// scan: `200` `text/html`.
    Confirm,
    /// The quota is spent: `200` `text/html`.
    Quota,
    /// `302` to a sign-in on `accounts.google.com`.
    SignIn,
    /// `302` to the same file on `host` (any host resolves to the server).
    Redirect { host: String },
    /// This status with a small web page.
    Status(u16),
}

/// The `Last-Modified` of every Drive file.
pub const DRIVE_MODIFIED: &str = "Fri, 02 Oct 2026 02:11:00 GMT";

/// A request the server saw.
#[derive(Debug, Clone)]
pub struct Seen {
    /// `GET`, `HEAD`.
    pub method: String,
    /// The `Range` header, when the request had one.
    pub range: Option<String>,
    pub host: String,
    pub path: String,
    pub query: String,
    pub at: Instant,
    pub cookie: bool,
    pub referer: bool,
}

struct Script<T> {
    answers: Vec<T>,
    served: usize,
}

impl<T: Clone> Script<T> {
    fn next(&mut self) -> Option<T> {
        let answer = self
            .answers
            .get(self.served.min(self.answers.len().checked_sub(1)?))
            .cloned();
        self.served += 1;
        answer
    }
}

#[derive(Default)]
struct State {
    port: u16,
    /// By `<host>/<path>` without the port.
    posts: HashMap<String, Script<PostAnswer>>,
    files: HashMap<String, Script<FileAnswer>>,
    drive: HashMap<String, Script<DriveAnswer>>,
    seen: Vec<Seen>,
    signed: u64,
}

/// The server, until it is dropped.
pub struct SourceServer {
    port: u16,
    state: Arc<Mutex<State>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for SourceServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Resolves every name to the loopback address.
struct Loopback;

impl reqwest::dns::Resolve for Loopback {
    fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async {
            let addrs: reqwest::dns::Addrs =
                Box::new(std::iter::once(SocketAddr::from(([127, 0, 0, 1], 0))));
            Ok(addrs)
        })
    }
}

impl SourceServer {
    pub async fn start() -> SourceServer {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State {
            port,
            ..State::default()
        }));
        let app = Router::new().fallback({
            let state = state.clone();
            move |request: Request<Body>| {
                let state = state.clone();
                async move { answer(&state, request) }
            }
        });
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        SourceServer { port, state, task }
    }

    /// The Tistory source, reaching this server for every name, with no
    /// spacing between requests.
    pub fn source(&self) -> TistorySource {
        self.source_spaced(Duration::ZERO)
    }

    pub fn source_spaced(&self, spacing: Duration) -> TistorySource {
        self.source_with(Limits {
            spacing,
            ..Limits::default()
        })
    }

    /// The source with these limits (a smaller file, a shorter deadline),
    /// and a Drive of its own with them.
    pub fn source_with(&self, limits: Limits) -> TistorySource {
        self.source_over(limits, self.drive_with(limits))
    }

    /// The source with these limits, receiving Drive files through `drive`.
    pub fn source_over(&self, limits: Limits, drive: Drive) -> TistorySource {
        TistorySource::over(builder(), limits, REACH, drive)
    }

    /// The Blogger source, reaching this server for every name, with no
    /// spacing between requests.
    pub fn blogger(&self) -> BloggerSource {
        self.blogger_with(Limits {
            spacing: Duration::ZERO,
            ..Limits::default()
        })
    }

    pub fn blogger_with(&self, limits: Limits) -> BloggerSource {
        self.blogger_over(limits, self.drive_with(limits))
    }

    /// The Blogger source with these limits, receiving Drive files through
    /// `drive`.
    pub fn blogger_over(&self, limits: Limits, drive: Drive) -> BloggerSource {
        BloggerSource::over(builder(), limits, REACH, drive)
    }

    /// Drive on this server with these limits, for sources to share.
    pub fn drive_with(&self, limits: Limits) -> Drive {
        let base = Url::parse(&format!("http://{DRIVE_FILES}:{}/", self.port)).unwrap();
        Drive::over(builder(), limits, REACH, base)
    }

    /// The Naver source, reaching this server for every name, with no
    /// spacing between requests.
    pub fn naver(&self) -> NaverSource {
        let limits = Limits {
            spacing: Duration::ZERO,
            ..Limits::default()
        };
        self.naver_over(limits, self.drive_with(limits))
    }

    /// The Naver source with these limits, receiving Drive files through
    /// `drive`.
    pub fn naver_over(&self, limits: Limits, drive: Drive) -> NaverSource {
        let base = Url::parse(&format!("http://blog.naver.com:{}/", self.port)).unwrap();
        NaverSource::over(builder(), limits, REACH, drive, base)
    }

    /// The erulabo source, reaching this server for every name, with no
    /// spacing between requests, observing Drive files on this server.
    pub fn erulabo(&self) -> ErulaboSource {
        let limits = Limits {
            spacing: Duration::ZERO,
            ..Limits::default()
        };
        ErulaboSource::over(builder(), limits, REACH, self.drive_with(limits))
    }

    /// The address of erulabo post `number`.
    pub fn erulabo_url(&self, number: u32) -> String {
        format!("http://erulabo.com:{}/{number}", self.port)
    }

    /// How erulabo post `number` answers, request after request (a
    /// [`PostAnswer::Page`] of [`erulabo_page`]).
    pub fn erulabo_post(&self, number: u32, answers: Vec<PostAnswer>) {
        self.lock().posts.insert(
            format!("erulabo.com/{number}"),
            Script { answers, served: 0 },
        );
    }

    /// The address of Naver post `log_no` of `blog`: its frame.
    pub fn naver_url(&self, blog: &str, log_no: &str) -> String {
        format!("http://blog.naver.com:{}/{blog}/{log_no}", self.port)
    }

    /// How the inner page of Naver post `log_no` of `blog` answers, request
    /// after request.
    pub fn naver_post(&self, blog: &str, log_no: &str, answers: Vec<PostAnswer>) {
        self.lock().posts.insert(
            format!("naver:{blog}/{log_no}"),
            Script { answers, served: 0 },
        );
    }

    /// How many times the inner page of Naver post `log_no` was asked for.
    pub fn naver_read(&self, log_no: &str) -> usize {
        let number = format!("logNo={log_no}&");
        self.lock()
            .seen
            .iter()
            .filter(|s| s.path == "/PostView.naver" && s.query.contains(&number))
            .count()
    }

    /// How the Naver attachment named `name` answers, request after request.
    pub fn naver_attachment(&self, name: &str, answers: Vec<FileAnswer>) {
        self.lock()
            .files
            .insert(format!("naver:{name}"), Script { answers, served: 0 });
    }

    /// The address of Blogger post `path` (`2026/07/2.html`) of `blog`.
    pub fn blogger_url(&self, blog: &str, path: &str) -> String {
        format!("http://{blog}.blogspot.com:{}/{path}", self.port)
    }

    /// How Blogger post `path` of `blog` answers, request after request.
    pub fn blogger_post(&self, blog: &str, path: &str, answers: Vec<PostAnswer>) {
        self.lock().posts.insert(
            format!("{blog}.blogspot.com/{path}"),
            Script { answers, served: 0 },
        );
    }

    /// How the Drive file with ID `id` answers, request after request.
    pub fn drive(&self, id: &str, answers: Vec<DriveAnswer>) {
        self.lock()
            .drive
            .insert(id.to_owned(), Script { answers, served: 0 });
    }

    /// How many times the Drive file with ID `id` was asked for.
    pub fn drive_asked(&self, id: &str) -> usize {
        let query = format!("id={id}&");
        self.lock()
            .seen
            .iter()
            .filter(|s| s.host == DRIVE_FILES && s.query.starts_with(&query))
            .count()
    }

    /// The address of post `number` of `blog`.
    pub fn post_url(&self, blog: &str, number: u32) -> String {
        format!("http://{blog}.tistory.com:{}/{number}", self.port)
    }

    /// How post `number` of `blog` answers, request after request.
    pub fn post(&self, blog: &str, number: u32, answers: Vec<PostAnswer>) {
        self.lock().posts.insert(
            format!("{blog}.tistory.com/{number}"),
            Script { answers, served: 0 },
        );
    }

    /// How file `id` answers, request after request.
    pub fn file(&self, id: &str, answers: Vec<FileAnswer>) {
        self.lock()
            .files
            .insert(id.to_owned(), Script { answers, served: 0 });
    }

    /// The requests so far, in order.
    pub fn seen(&self) -> Vec<Seen> {
        self.lock().seen.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The sources' reach here: plain `http`.
const REACH: Reach = Reach { plain_http: true };

/// The host Drive files come from.
pub const DRIVE_FILES: &str = "drive.usercontent.google.com";

fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .no_proxy()
        .dns_resolver(Arc::new(Loopback))
}

/// A Drive file's address as a post links it.
pub fn drive_link(id: &str) -> String {
    format!("https://drive.google.com/file/d/{id}/view?usp=sharing")
}

/// A Blogger post as the newer themes write it (C소라, 별명따위): its JSON-LD
/// with `dateModified`, the body (`.post-body`) with a link of these words to
/// each address, and the blog's folder of all its subtitles outside it.
pub fn blogger_page(links: &[(&str, &str)]) -> String {
    let links: String = links
        .iter()
        .map(|(words, href)| {
            format!(
                r#"<a href="{}" target="_blank">{}</a><br />"#,
                escape(href),
                escape(words)
            )
        })
        .collect();
    format!(
        r#"<!DOCTYPE html><html><head><meta charset="UTF-8"><script type="application/ld+json">{{
  "@context": "http://schema.org",
  "@type": "BlogPosting",
  "headline": "정반대의 너와 나 2기",
  "datePublished": "2026-07-05T22:58:00+09:00",
  "dateModified": "{BLOGGER_MODIFIED}"
}}</script></head><body>
<div class='post-body-container'><div class='post-body entry-content float-container' id='post-body-1'><p>자막이에요.</p><p>{links}</p></div></div>
<div class='widget LinkList'><a href='https://drive.google.com/drive/folders/10YFO-jkkgsybQnPpl5TVAwPdx2P0SE-y'>자막 모음(작업 중)</a></div>
</body></html>"#
    )
}

/// An erulabo post as the site writes it (2026-10-03): its JSON-LD with
/// `dateModified` `modified`, and the body with a download card of each
/// `(file, title)`.
pub fn erulabo_page(cards: &[(&str, &str)], modified: &str) -> String {
    let cards: String = cards
        .iter()
        .map(|(file, title)| {
            format!(
                r#"<button type="button" class="og-link og-link-button" data-file-url="{file}"><span class="og-preview"><span class="og-body"><span class="og-title">{}</span></span></span></button>"#,
                escape(title)
            )
        })
        .collect();
    format!(
        r#"<!doctype html><html><head><title>자막</title>
<script type="application/ld+json">{{"@context":"https://schema.org","@type":"BlogPosting","datePublished":"2026-09-22T11:20:00+09:00","dateModified":"{modified}"}}</script>
</head><body><div id="post-body" class="fr-view"><p>본문</p>{cards}</div></body></html>"#
    )
}

/// The `dateModified` of every [`blogger_page`].
pub const BLOGGER_MODIFIED: &str = "2026-09-27T22:55:03+09:00";

/// The `article:modified_time` of every [`PostAnswer::Files`] page.
pub const TISTORY_MODIFIED: &str = "2026-09-28T00:13:41+09:00";

/// The body container of the posts seen (`tistory::BODY`).
pub const BODY_OPEN: &str = r#"<div class="tt_article_useless_p_margin contents_style">"#;

/// A Tistory post whose subtitle is a Google Drive link in its body, as felia
/// 1187: one link to file `id` whose words say nothing of the episode.
pub fn drive_page(id: &str) -> String {
    format!(
        r#"<!doctype html><html><head><meta property="article:modified_time" content="2026-10-01T23:10:30+09:00"></head><body><div class="tt_article_useless_p_margin contents_style"><p>1화 자막</p>
<table><tbody><tr><td><a href="https://drive.google.com/file/d/{id}/view?usp=drive_link"><span><b>자막 다운로드</b></span></a></td></tr></tbody></table>
</div></body></html>"#
    )
}

fn page(status: u16, size: usize) -> Response {
    let mut body =
        "<!DOCTYPE html><html><head><title>Error</title></head><body>error</body></html>"
            .to_owned();
    while body.len() < size {
        body.push(' ');
    }
    (
        StatusCode::from_u16(status).unwrap(),
        [(header::CONTENT_TYPE, "text/html;charset=UTF-8")],
        body,
    )
        .into_response()
}

/// A file's signed address on `host`, as the server signs it.
fn file_address(host: &str, port: u16, id: &str, name: &str, signed: u64) -> String {
    format!(
        "http://{host}:{port}/dna/{id}/x/y/{name}?credential=C{signed}&expires=1793458799&allow_ip=&allow_referer=&signature=S{signed}&attach=1&knm=tfile.zip"
    )
}

fn octets(body: Body) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CONTENT_DISPOSITION, "attachment;"),
        ],
        body,
    )
        .into_response()
}

fn pieces(bytes: Vec<u8>) -> Vec<Result<bytes::Bytes, std::convert::Infallible>> {
    bytes
        .chunks(1024)
        .map(|c| Ok(bytes::Bytes::copy_from_slice(c)))
        .collect()
}

/// The server's answer to `request`. Drive's and the CDN's answers also set a
/// cookie for their whole domain, as the real hosts do, so a client that kept
/// cookies would send one on its next request there (`Seen::cookie`).
fn answer(state: &Mutex<State>, request: Request<Body>) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.split(':').next())
        .unwrap_or_default()
        .to_owned();
    let mut response = respond(state, request);
    if crate::drive::HOSTS.contains(&host.as_str()) || tistory::cdn_host(&host) {
        // The last two labels: `google.com`, `daumcdn.net`.
        let at = host.rmatch_indices('.').nth(1).map_or(0, |(i, _)| i + 1);
        let cookie = format!(
            "NID=511=test-cookie; expires=Sun, 04-Apr-2027 00:00:00 GMT; path=/; domain=.{}; HttpOnly",
            &host[at..]
        );
        response.headers_mut().append(
            header::SET_COOKIE,
            header::HeaderValue::from_str(&cookie).unwrap(),
        );
    }
    response
}

fn respond(state: &Mutex<State>, request: Request<Body>) -> Response {
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    let headers = request.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default().to_owned();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    let head = request.method() == axum::http::Method::HEAD;
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    state.seen.push(Seen {
        method: request.method().as_str().to_owned(),
        range: range.clone(),
        host: host.clone(),
        path: path.clone(),
        query: query.clone(),
        at: Instant::now(),
        cookie: headers.contains_key(header::COOKIE),
        referer: headers.contains_key(header::REFERER),
    });

    if host == "drive.google.com" && path == "/uc" {
        let to = format!("http://{DRIVE_FILES}:{}/download?{query}", state.port);
        return (StatusCode::SEE_OTHER, [(header::LOCATION, to)]).into_response();
    }
    if crate::drive::HOSTS.contains(&host.as_str()) && path == "/download" {
        let id = url::form_urlencoded::parse(query.as_bytes())
            .find(|(k, _)| k == "id")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default();
        let port = state.port;
        return match state.drive.get_mut(&id).and_then(Script::next) {
            Some(
                answer @ (DriveAnswer::File { .. }
                | DriveAnswer::FileAt { .. }
                | DriveAnswer::Undated { .. }),
            ) => {
                let (name, bytes, modified) = match answer {
                    DriveAnswer::File { name, bytes } => {
                        (name, bytes, Some(DRIVE_MODIFIED.to_owned()))
                    }
                    DriveAnswer::FileAt {
                        name,
                        bytes,
                        modified,
                    } => (name, bytes, Some(modified)),
                    DriveAnswer::Undated { name, bytes } => (name, bytes, None),
                    _ => unreachable!("matched above"),
                };
                let length = bytes.len();
                let mut response = octets(Body::from(bytes));
                let headers = response.headers_mut();
                let disposition = format!("attachment; filename=\"{name}\"");
                headers.insert(
                    header::CONTENT_DISPOSITION,
                    header::HeaderValue::from_bytes(disposition.as_bytes()).unwrap(),
                );
                if let Some(modified) = modified {
                    headers.insert(
                        header::LAST_MODIFIED,
                        header::HeaderValue::from_str(&modified).unwrap(),
                    );
                }
                // A `HEAD` has no body but the same `Content-Length`.
                headers.insert(header::CONTENT_LENGTH, length.into());
                response
            }
            Some(DriveAnswer::Confirm) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                format!(
                    r#"<!DOCTYPE html><html><head><title>Google Drive - Virus scan warning</title></head><body><form id="download-form" action="https://{DRIVE_FILES}/download" method="get"><input type="submit" id="uc-download-link" value="Download anyway"/><input type="hidden" name="id" value="{id}"><input type="hidden" name="export" value="download"><input type="hidden" name="confirm" value="t"></form></body></html>"#
                ),
            )
                .into_response(),
            Some(DriveAnswer::Quota) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                "<!DOCTYPE html><html><head><title>Google Drive - Quota exceeded</title></head><body>Too many users have viewed or downloaded this file recently.</body></html>",
            )
                .into_response(),
            Some(DriveAnswer::SignIn) => (
                StatusCode::FOUND,
                [(
                    header::LOCATION,
                    "https://accounts.google.com/ServiceLogin?continue=https://drive.google.com/",
                )],
            )
                .into_response(),
            Some(DriveAnswer::Redirect { host }) => {
                let to = format!("http://{host}:{port}/download?{query}");
                (StatusCode::FOUND, [(header::LOCATION, to)]).into_response()
            }
            Some(DriveAnswer::Status(status)) => page(status, 0),
            Some(DriveAnswer::Missing) | None => page(404, 1652),
        };
    }

    if naver::reads(&host) && path != "/PostView.naver" {
        // The frame around the post: its inner page, and nothing of it.
        let mut parts = path.split('/').filter(|p| !p.is_empty());
        let (blog, log_no) = (
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
        );
        let inner = format!("/PostView.naver?blogId={blog}&amp;logNo={log_no}&amp;redirect=Dlog&amp;widgetTypeCall=true&amp;noTrackingCode=true&amp;directAccess=false");
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html;charset=UTF-8")],
            format!(
                r#"<html><head><title>네이버 블로그</title></head><body><iframe id="mainFrame" name="mainFrame" src="{inner}"></iframe></body></html>"#
            ),
        )
            .into_response();
    }
    if naver::reads(&host) {
        let pairs: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        let log_no = pairs.get("logNo").cloned().unwrap_or_default();
        let key = format!(
            "naver:{}/{log_no}",
            pairs.get("blogId").map(String::as_str).unwrap_or_default(),
        );
        return match state.posts.get_mut(&key).and_then(Script::next) {
            Some(PostAnswer::Naver(files)) => {
                state.signed += 1;
                let html = naver_page(state.port, state.signed, &log_no, &files);
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html;charset=UTF-8")],
                    html,
                )
                    .into_response()
            }
            Some(PostAnswer::Page(html)) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html;charset=UTF-8")],
                html,
            )
                .into_response(),
            Some(PostAnswer::Status(status)) => page(status, 3228),
            Some(PostAnswer::Files(_) | PostAnswer::FilesAt(..)) | None => page(404, 3228),
        };
    }
    if host == naver::FILE_HOST {
        let name = path.rsplit('/').next().unwrap_or_default();
        let name = percent_encoding::percent_decode_str(name).decode_utf8_lossy();
        return match state
            .files
            .get_mut(&format!("naver:{name}"))
            .and_then(Script::next)
        {
            Some(FileAnswer::Bytes(bytes)) => octets(Body::from(bytes)),
            Some(FileAnswer::Streamed(bytes)) => {
                octets(Body::from_stream(futures::stream::iter(pieces(bytes))))
            }
            Some(FileAnswer::Refused) => page(400, 3452),
            Some(FileAnswer::Page) => page(200, 0),
            Some(FileAnswer::Status(status)) => page(status, 0),
            Some(other) => panic!("not a Naver answer: {other:?}"),
            None => page(400, 3452),
        };
    }

    if tistory::cdn_host(&host) {
        let id = path.split('/').nth(2).unwrap_or_default().to_owned();
        // The CDN refuses a `HEAD` (2026-10-03).
        if head {
            return page(404, 150);
        }
        return match state.files.get_mut(&id).and_then(Script::next) {
            Some(FileAnswer::Bytes(bytes)) if range.as_deref() == Some("bytes=0-0") => {
                // The one byte asked for, and the whole size in `Content-Range`.
                let total = bytes.len();
                let mut response = (
                    StatusCode::PARTIAL_CONTENT,
                    [
                        (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
                        (header::CONTENT_RANGE, format!("bytes 0-0/{total}")),
                    ],
                    Body::from(bytes[..1.min(total)].to_vec()),
                )
                    .into_response();
                response
                    .headers_mut()
                    .insert(header::CONTENT_LENGTH, 1.into());
                response
            }
            Some(FileAnswer::Bytes(bytes)) => octets(Body::from(bytes)),
            Some(FileAnswer::Streamed(bytes)) => {
                octets(Body::from_stream(futures::stream::iter(pieces(bytes))))
            }
            Some(FileAnswer::Stalled(bytes)) => {
                use futures::StreamExt;
                let first = pieces(bytes).into_iter().take(1);
                octets(Body::from_stream(
                    futures::stream::iter(first).chain(futures::stream::pending()),
                ))
            }
            Some(FileAnswer::Encoded(bytes)) => {
                let mut response = octets(Body::from(bytes));
                response.headers_mut().insert(
                    header::CONTENT_ENCODING,
                    header::HeaderValue::from_static("gzip"),
                );
                response
            }
            Some(FileAnswer::Redirect { host, id }) => {
                state.signed += 1;
                let to = file_address(&host, state.port, &id, "f.zip", state.signed);
                (StatusCode::FOUND, [(header::LOCATION, to)]).into_response()
            }
            Some(FileAnswer::Page) => page(200, 0),
            Some(FileAnswer::Refused) => page(404, 150),
            Some(FileAnswer::Status(status)) => page(status, 0),
            None => page(404, 150),
        };
    }

    let key = format!("{host}{path}");
    match state.posts.get_mut(&key).and_then(Script::next) {
        Some(answer @ (PostAnswer::Files(_) | PostAnswer::FilesAt(..))) => {
            state.signed += 1;
            let (files, modified) = match answer {
                PostAnswer::FilesAt(files, modified) => (files, modified),
                PostAnswer::Files(files) => (files, TISTORY_MODIFIED.to_owned()),
                _ => unreachable!("matched above"),
            };
            let html = files_page(state.port, state.signed, &files, &modified);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html;charset=UTF-8")],
                html,
            )
                .into_response()
        }
        Some(PostAnswer::Page(html)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html;charset=UTF-8")],
            html,
        )
            .into_response(),
        Some(PostAnswer::Status(status)) => page(status, 1952),
        Some(PostAnswer::Naver(_)) | None => page(404, 1952),
    }
}

/// A Naver post's inner page as Naver writes it (2026-10-03): the body with a
/// save button per file, the guestbook's CAPTCHA frame, and the `aPostFiles`
/// list, each address with a new token.
fn naver_page(port: u16, signed: u64, log_no: &str, files: &[NaverFile]) -> String {
    let address = |f: &NaverFile| {
        format!(
            "http://{}:{port}/open/F{:x}/T{signed}/{}",
            naver::FILE_HOST,
            f.name.len(),
            utf8_percent_encode(&f.name, NON_ALPHANUMERIC)
        )
    };
    let records: Vec<String> = files
        .iter()
        .map(|f| {
            let size = f.size.to_string();
            // Thousands with commas, as the page writes them.
            let mut grouped = String::new();
            for (i, digit) in size.chars().enumerate() {
                if i > 0 && (size.len() - i) % 3 == 0 {
                    grouped.push(',');
                }
                grouped.push(digit);
            }
            let json = |text: &str| serde_json::to_string(text).unwrap();
            format!(
                r#"{{"encodedAttachFileName": {name},"encodedAttachFileNameByTruncate": {name},"encodedAttachFileUrl": {url},"encodedAttachFileUrlByMS949": {url},"licenseyn": "T","maliciousCodeYn": "{malicious}","punishType": "{punish}","attachFileSize": "{grouped}","ahfLicenseYn" : "false"}}"#,
                name = json(&f.name),
                url = json(&address(f)),
                malicious = f.malicious,
                punish = f.punish,
            )
        })
        .collect();
    // Inside a JavaScript string in single quotes.
    let list = format!("[{}]", records.join(" , "))
        .replace('\\', "\\\\")
        .replace('\'', "\\'");
    let buttons: String = files
        .iter()
        .map(|f| {
            format!(
                r#"<div class="se-module se-module-file"><span class="se-file-name">{}</span><a href="{}" class="se-file-save-button __se_link" role="button" target="_blank">저장</a></div>"#,
                escape(&f.name),
                escape(&address(f))
            )
        })
        .collect();
    format!(
        r#"<!DOCTYPE html><html><body><div id="postListBody"><div id="post-view{log_no}"><div class="se-main-container"><p>자막이에요.</p>{buttons}</div>
<span class="se_publishDate pcol2">{NAVER_PUBLISHED}</span>
</div><div class="frame_wrap"><iframe id="captchalayeredframe" src="about:blank"></iframe></div></div>
<script>
var aPostFiles = [];
		aPostFiles[1] = JSON.parse('{list}'.replace(/\\'/g, ''));
	aPostBaseInfo[1] = "{log_no}|0|1|1|339|0|false|4|MYLOG";
</script></body></html>"#
    )
}

/// The date every Naver post shows.
pub const NAVER_PUBLISHED: &str = "2026. 6. 23. 5:19";

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A post's page as Tistory writes it: a picture, then the fileblocks.
fn files_page(port: u16, signed: u64, files: &[FileSpec], modified: &str) -> String {
    let blocks: String = files
        .iter()
        .map(|f| {
            let name = utf8_percent_encode(&f.name, NON_ALPHANUMERIC);
            format!(
                r#"<figure class="fileblock" data-ke-align="alignCenter"><a href="{href}" class=""><div class="image"></div><div class="desc"><div class="filename"><span class="name">{shown}</span></div><div class="size">{size}</div></div></a></figure>"#,
                href = escape(&file_address(CDN, port, &f.id, &name.to_string(), signed)),
                shown = escape(&f.name),
                size = escape(&f.size_text),
            )
        })
        .collect();
    format!(
        r#"<!doctype html><html lang="ko"><head><meta charset="utf-8">
<meta property="article:modified_time" content="{modified}"><title>자막</title></head>
<body>{BODY_OPEN}<p><img src="http://{CDN}:{port}/dna/pic/1/2/img.png?credential=P&amp;signature=Q"></p>
{blocks}</div></body></html>"#
    )
}

#[cfg(test)]
mod tests;
