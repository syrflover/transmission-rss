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

/// The `dateModified` of every [`blogger_page`].
pub const BLOGGER_MODIFIED: &str = "2026-09-27T22:55:03+09:00";

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
    state.seen.push(Seen {
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
            Some(DriveAnswer::File { name, bytes }) => {
                let mut response = octets(Body::from(bytes));
                let headers = response.headers_mut();
                let disposition = format!("attachment; filename=\"{name}\"");
                headers.insert(
                    header::CONTENT_DISPOSITION,
                    header::HeaderValue::from_bytes(disposition.as_bytes()).unwrap(),
                );
                headers.insert(
                    header::LAST_MODIFIED,
                    header::HeaderValue::from_static(DRIVE_MODIFIED),
                );
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
            Some(PostAnswer::Files(_)) | None => page(404, 3228),
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
        return match state.files.get_mut(&id).and_then(Script::next) {
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
        Some(PostAnswer::Files(files)) => {
            state.signed += 1;
            let html = files_page(state.port, state.signed, &files);
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
fn files_page(port: u16, signed: u64, files: &[FileSpec]) -> String {
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
<meta property="article:modified_time" content="2026-09-28T00:13:41+09:00"><title>자막</title></head>
<body>{BODY_OPEN}<p><img src="http://{CDN}:{port}/dna/pic/1/2/img.png?credential=P&amp;signature=Q"></p>
{blocks}</div></body></html>"#
    )
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};
    use url::Url;

    use super::*;
    use crate::{verify, FailureKind, Opened, Source};

    const SRT: &[u8] = b"1\n00:00:01,000 --> 00:00:02,000\nHi\n";

    async fn bytes_of(
        source: &Source,
        post: &Url,
        file: &crate::PostFile,
    ) -> (Option<u64>, Vec<u8>) {
        let mut fetch = source.fetch(post, file).await.unwrap();
        let mut bytes = Vec::new();
        while let Some(piece) = fetch.chunk().await.unwrap() {
            bytes.extend_from_slice(&piece);
        }
        (fetch.expected_size, bytes)
    }

    async fn files(source: &Source, post: &Url) -> Vec<crate::PostFile> {
        match source.open(post, "24").await.unwrap() {
            Opened::Files(files) => files,
            other => panic!("files: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_zip_comes_whole_with_no_cookie_or_referer() {
        let server = SourceServer::start().await;
        let zip = verify::zip_of(&[("Seihantai - 24.srt", SRT)]);
        server.post(
            "sumomomo",
            492,
            vec![PostAnswer::Files(vec![spec(
                "z1",
                "Seihantai - 24.zip",
                "0.01MB",
            )])],
        );
        server.file("z1", vec![FileAnswer::Bytes(zip.clone())]);
        let source = Source::Tistory(server.source());
        let post = Url::parse(&server.post_url("sumomomo", 492)).unwrap();

        let files = files(&source, &post).await;
        assert_eq!(files[0].name, "Seihantai - 24.zip");
        let (expected, bytes) = bytes_of(&source, &post, &files[0]).await;
        assert_eq!(expected, Some(zip.len() as u64));
        assert_eq!(Sha256::digest(&bytes), Sha256::digest(&zip));
        assert!(server.seen().iter().all(|s| !s.cookie && !s.referer));
    }

    #[tokio::test]
    async fn a_refused_address_is_read_again_once_then_expired_or_missing() {
        let server = SourceServer::start().await;
        let source = Source::Tistory(server.source());
        let post = Url::parse(&server.post_url("blog", 1)).unwrap();
        let a = spec("a", "a.zip", "1KB");

        // Refused once: the post is read again and the new address works.
        server.post("blog", 1, vec![PostAnswer::Files(vec![a.clone()])]);
        server.file(
            "a",
            vec![FileAnswer::Refused, FileAnswer::Bytes(b"PK".to_vec())],
        );
        let first = files(&source, &post).await;
        let (_, bytes) = bytes_of(&source, &post, &first[0]).await;
        assert_eq!(bytes, b"PK");
        let posts = server
            .seen()
            .iter()
            .filter(|s| s.host.ends_with("tistory.com"))
            .count();
        assert_eq!(posts, 2);

        // Refused again: expired, with what answered.
        server.file("a", vec![FileAnswer::Refused]);
        let first = files(&source, &post).await;
        let failure = source.fetch(&post, &first[0]).await.err().unwrap();
        assert_eq!(failure.kind, FailureKind::Expired);
        assert_eq!(failure.status, Some(404));
        assert_eq!(failure.content_type.as_deref(), Some("text/html"));
        assert_eq!(failure.size, Some(150));

        // Gone from the post when it is read again: missing.
        server.post(
            "blog",
            1,
            vec![
                PostAnswer::Files(vec![a]),
                PostAnswer::Files(vec![spec("b", "b.zip", "1KB")]),
            ],
        );
        let first = files(&source, &post).await;
        let failure = source.fetch(&post, &first[0]).await.err().unwrap();
        assert_eq!(failure.kind, FailureKind::Missing);
    }

    #[tokio::test]
    async fn a_missing_post_a_failing_site_and_a_drive_post_each_say_so() {
        let server = SourceServer::start().await;
        let source = Source::Tistory(server.source());
        let url = |n| Url::parse(&server.post_url("blog", n)).unwrap();
        server.post("blog", 2, vec![PostAnswer::Status(503)]);
        server.post(
            "blog",
            3,
            vec![PostAnswer::Page(drive_page(
                "1zqWESZSANz8Uj2aaW9u1oSHKurLuTqQw",
            ))],
        );

        let missing = source.open(&url(99999), "1").await.unwrap_err();
        assert_eq!(
            (missing.kind, missing.status, missing.size),
            (FailureKind::Missing, Some(404), Some(1952))
        );
        assert_eq!(
            source.open(&url(2), "1").await.unwrap_err().kind,
            FailureKind::Network
        );
        // A Drive link in the body: the file, from Drive, under the name
        // Drive gives it.
        let ass = crate::fake::ass("FX Senshi Kurumi 01");
        server.drive(
            "1zqWESZSANz8Uj2aaW9u1oSHKurLuTqQw",
            vec![DriveAnswer::File {
                name: "FX 전사 쿠루미 01.ass".into(),
                bytes: ass.clone(),
            }],
        );
        let drive = files(&source, &url(3)).await;
        assert_eq!(drive.len(), 1);
        assert_eq!(drive[0].key, "drive:1zqWESZSANz8Uj2aaW9u1oSHKurLuTqQw");
        let mut fetch = source.fetch(&url(3), &drive[0]).await.unwrap();
        assert_eq!(fetch.name.as_deref(), Some("FX 전사 쿠루미 01.ass"));
        assert_eq!(fetch.expected_size, Some(ass.len() as u64));
        let mut bytes = Vec::new();
        while let Some(piece) = fetch.chunk().await.unwrap() {
            bytes.extend_from_slice(&piece);
        }
        assert_eq!(bytes, ass);
        assert!(server.seen().iter().all(|s| !s.cookie && !s.referer));
    }

    /// A Blogger post of C소라's shape: the fonts, then 13–24화.
    fn csora(server: &SourceServer) -> Url {
        let ids: Vec<(String, String)> =
            std::iter::once(("폰트".to_owned(), "1font0000000".to_owned()))
                .chain((13..=24).map(|n| (format!("{n}화"), format!("1episode{n:04}"))))
                .collect();
        let links: Vec<(String, String)> = ids
            .iter()
            .map(|(w, id)| (w.clone(), drive_link(id)))
            .collect();
        let links: Vec<(&str, &str)> = links
            .iter()
            .map(|(w, h)| (w.as_str(), h.as_str()))
            .collect();
        server.blogger_post(
            "csora556",
            "2026/07/2.html",
            vec![PostAnswer::Page(blogger_page(&links))],
        );
        Url::parse(&server.blogger_url("csora556", "2026/07/2.html")).unwrap()
    }

    #[tokio::test]
    async fn a_blogger_post_offers_its_episodes_file_and_fonts_from_drive() {
        let server = SourceServer::start().await;
        let source = Source::Blogger(server.blogger());
        let post = csora(&server);
        let chosen = match source.open(&post, "24").await.unwrap() {
            Opened::Files(files) => files,
            other => panic!("files: {other:?}"),
        };
        let keys: Vec<&str> = chosen.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["drive:1font0000000", "drive:1episode0024"]);
        assert_eq!(
            chosen[1].snapshot.entries(),
            [(
                crate::blogger::POST_MODIFIED.to_owned(),
                BLOGGER_MODIFIED.to_owned()
            )]
        );
        let ass = crate::fake::ass("Seihantai 24");
        server.drive(
            "1episode0024",
            vec![DriveAnswer::File {
                name: "Seihantai 24.ass".into(),
                bytes: ass.clone(),
            }],
        );
        let (expected, bytes) = bytes_of(&source, &post, &chosen[1]).await;
        assert_eq!((expected, bytes), (Some(ass.len() as u64), ass.clone()));
        let fetch = source.fetch(&post, &chosen[1]).await.unwrap();
        assert_eq!(
            fetch.snapshot.entries(),
            [
                (
                    crate::http::LAST_MODIFIED.to_owned(),
                    DRIVE_MODIFIED.to_owned()
                ),
                (
                    crate::drive::CONTENT_LENGTH.to_owned(),
                    ass.len().to_string()
                ),
            ]
        );
        // A second file from Drive, after Drive set its cookie twice.
        let font = b"OTTO font".to_vec();
        server.drive(
            "1font0000000",
            vec![DriveAnswer::File {
                name: "fonts.zip".into(),
                bytes: font.clone(),
            }],
        );
        let (_, bytes) = bytes_of(&source, &post, &chosen[0]).await;
        assert_eq!(bytes, font);
        assert_eq!(server.drive_asked("1font0000000"), 1);
        // Only the chosen files were asked for, and no other episode's; none
        // of the requests carried Drive's cookie or a `Referer`.
        assert_eq!(server.drive_asked("1episode0023"), 0);
        assert!(server.seen().iter().all(|s| !s.cookie && !s.referer));

        let failure = source.open(&post, "12").await.unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert_eq!(failure.reason, "게시물에 12화 파일이 없어요");
    }

    #[tokio::test]
    async fn a_page_from_drive_instead_of_the_file_is_a_classified_failure() {
        let server = SourceServer::start().await;
        let source = Source::Blogger(server.blogger());
        let post = csora(&server);
        let Opened::Files(chosen) = source.open(&post, "13").await.unwrap() else {
            panic!("files");
        };
        let file = &chosen[1];
        let cases = [
            (
                DriveAnswer::Confirm,
                FailureKind::NotAFile,
                Some(200),
                "확인 페이지",
            ),
            (DriveAnswer::Quota, FailureKind::NotAFile, Some(200), "한도"),
            (
                DriveAnswer::Missing,
                FailureKind::Missing,
                Some(404),
                "없어요",
            ),
            (
                DriveAnswer::SignIn,
                FailureKind::Missing,
                Some(302),
                "로그인",
            ),
            (
                DriveAnswer::Status(403),
                FailureKind::Missing,
                Some(403),
                "공개",
            ),
            (
                DriveAnswer::Status(503),
                FailureKind::Network,
                Some(503),
                "주지 못했어요",
            ),
            (
                DriveAnswer::Redirect {
                    host: "collect.example".into(),
                },
                FailureKind::Changed,
                Some(302),
                "따라갈 수 없는",
            ),
        ];
        for (answer, kind, status, says) in cases {
            server.drive("1episode0013", vec![answer.clone()]);
            let failure = source.fetch(&post, file).await.err().unwrap();
            assert_eq!((failure.kind, failure.status), (kind, status), "{answer:?}");
            assert!(
                failure.reason.contains(says),
                "{answer:?}: {}",
                failure.reason
            );
        }
        let failure = {
            server.drive("1episode0013", vec![DriveAnswer::Confirm]);
            source.fetch(&post, file).await.err().unwrap()
        };
        assert_eq!(failure.content_type.as_deref(), Some("text/html"));
        assert!(failure.size.is_some_and(|s| s > 0));
        // No request left Drive's hosts, and none went to sign in.
        assert!(server
            .seen()
            .iter()
            .all(|s| s.host != "collect.example" && s.host != "accounts.google.com"));

        // Within Drive's hosts a redirect is followed.
        server.drive(
            "1episode0013",
            vec![
                DriveAnswer::Redirect {
                    host: "drive.google.com".into(),
                },
                DriveAnswer::File {
                    name: "13.ass".into(),
                    bytes: crate::fake::ass("13"),
                },
            ],
        );
        let (_, bytes) = bytes_of(&source, &post, file).await;
        assert_eq!(bytes, crate::fake::ass("13"));
        // The cookie Drive set on the first answer is not sent on the
        // redirect to its other host, nor on any later request.
        let seen = server.seen();
        let followed = seen
            .iter()
            .find(|s| s.host == "drive.google.com" && s.path == "/download")
            .unwrap();
        assert!(!followed.cookie && !followed.referer);
        assert!(seen.iter().all(|s| !s.cookie && !s.referer));
    }

    #[tokio::test]
    async fn a_redirect_is_followed_on_the_cdn_with_no_referer_and_stopped_elsewhere() {
        let server = SourceServer::start().await;
        let zip = verify::zip_of(&[("a.srt", SRT)]);
        server.post(
            "blog",
            1,
            vec![PostAnswer::Files(vec![
                spec("r1", "a.zip", "1KB"),
                spec("r2", "b.zip", "1KB"),
            ])],
        );
        server.file(
            "r1",
            vec![FileAnswer::Redirect {
                host: "t1.daumcdn.net".into(),
                id: "z1".into(),
            }],
        );
        server.file("z1", vec![FileAnswer::Bytes(zip.clone())]);
        server.file(
            "r2",
            vec![FileAnswer::Redirect {
                host: "collect.example".into(),
                id: "z1".into(),
            }],
        );
        let source = Source::Tistory(server.source());
        let post = Url::parse(&server.post_url("blog", 1)).unwrap();
        let files = files(&source, &post).await;

        // To another CDN host: followed, and the signed address it came from
        // goes along in no `Referer`.
        let (_, bytes) = bytes_of(&source, &post, &files[0]).await;
        assert_eq!(bytes, zip);
        let seen = server.seen();
        let followed = seen.iter().find(|s| s.host == "t1.daumcdn.net").unwrap();
        assert!(!followed.referer && !followed.cookie);

        // To any other host: not followed, and the answer is the redirect's.
        let failure = source.fetch(&post, &files[1]).await.err().unwrap();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert_eq!(failure.status, Some(302));
        assert!(!failure.reason.contains("signature") && !failure.reason.contains("example"));
        assert!(server.seen().iter().all(|s| s.host != "collect.example"));
        assert!(server.seen().iter().all(|s| !s.referer && !s.cookie));
    }

    #[tokio::test]
    async fn a_file_past_its_byte_limit_or_encoded_is_not_a_file() {
        let server = SourceServer::start().await;
        let big = vec![b'x'; 4096];
        server.post(
            "blog",
            1,
            vec![PostAnswer::Files(vec![
                spec("said", "a.srt", "4KB"),
                spec("sent", "b.srt", "4KB"),
                spec("small", "c.srt", "1KB"),
                spec("gz", "d.srt", "1KB"),
            ])],
        );
        server.file("said", vec![FileAnswer::Bytes(big.clone())]);
        server.file("sent", vec![FileAnswer::Streamed(big.clone())]);
        server.file("small", vec![FileAnswer::Streamed(SRT.to_vec())]);
        server.file("gz", vec![FileAnswer::Encoded(SRT.to_vec())]);
        let source = Source::Tistory(server.source_with(tistory::Limits {
            spacing: Duration::ZERO,
            max_file: 2048,
            ..tistory::Limits::default()
        }));
        let post = Url::parse(&server.post_url("blog", 1)).unwrap();
        let files = files(&source, &post).await;

        // Announced past the limit: refused before a byte is read.
        let failure = source.fetch(&post, &files[0]).await.err().unwrap();
        assert_eq!(
            (failure.kind, failure.status, failure.size),
            (FailureKind::NotAFile, Some(200), Some(4096))
        );
        assert!(failure.reason.contains("2048바이트"));

        // Not announced: the bytes stop at the limit.
        let mut fetch = source.fetch(&post, &files[1]).await.unwrap();
        assert_eq!(fetch.expected_size, None);
        let failure = loop {
            match fetch.chunk().await {
                Ok(Some(_)) => continue,
                Ok(None) => panic!("the bytes went past the limit"),
                Err(failure) => break failure,
            }
        };
        assert_eq!(failure.kind, FailureKind::NotAFile);
        assert!(failure.size.is_some_and(|s| s > 2048 && s <= 3072));

        // Within it: whole.
        let (_, bytes) = bytes_of(&source, &post, &files[2]).await;
        assert_eq!(bytes, SRT);

        // Encoded: not the file, whatever its length says.
        let failure = source.fetch(&post, &files[3]).await.err().unwrap();
        assert_eq!(failure.kind, FailureKind::NotAFile);
        assert!(failure.reason.contains("Content-Encoding"));
    }

    #[tokio::test]
    async fn a_file_that_stalls_past_its_deadline_is_a_network_failure() {
        let server = SourceServer::start().await;
        server.post(
            "blog",
            1,
            vec![PostAnswer::Files(vec![spec("s", "a.srt", "4KB")])],
        );
        server.file("s", vec![FileAnswer::Stalled(vec![b'x'; 4096])]);
        let source = Source::Tistory(server.source_with(tistory::Limits {
            spacing: Duration::ZERO,
            file_deadline: Duration::from_millis(300),
            ..tistory::Limits::default()
        }));
        let post = Url::parse(&server.post_url("blog", 1)).unwrap();
        let files = files(&source, &post).await;
        let started = Instant::now();
        let mut fetch = source.fetch(&post, &files[0]).await.unwrap();
        let failure = loop {
            match fetch.chunk().await {
                Ok(Some(_)) => continue,
                Ok(None) => panic!("a stalled answer has no end"),
                Err(failure) => break failure,
            }
        };
        assert_eq!(failure.kind, FailureKind::Network);
        assert!(failure.reason.contains("1초 안에 다 받지 못했어요"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn requests_to_one_host_are_spaced() {
        let server = SourceServer::start().await;
        let specs: Vec<FileSpec> = (0..3)
            .map(|i| spec(&format!("f{i}"), &format!("{i}.srt"), "1KB"))
            .collect();
        server.post("blog", 1, vec![PostAnswer::Files(specs.clone())]);
        for s in &specs {
            server.file(&s.id, vec![FileAnswer::Bytes(SRT.to_vec())]);
        }
        // The server sees each request a little after it is sent, by an
        // amount a loaded machine can stretch: a wide margin under the spacing.
        let source = Source::Tistory(server.source_spaced(Duration::from_millis(300)));
        let post = Url::parse(&server.post_url("blog", 1)).unwrap();
        for file in files(&source, &post).await {
            bytes_of(&source, &post, &file).await;
        }
        let cdn: Vec<Instant> = server
            .seen()
            .iter()
            .filter(|s| s.host == CDN)
            .map(|s| s.at)
            .collect();
        assert_eq!(cdn.len(), 3);
        assert!(cdn
            .windows(2)
            .all(|w| w[1] - w[0] >= Duration::from_millis(200)));
    }

    #[tokio::test]
    async fn sources_sharing_one_drive_space_their_drive_requests_together() {
        let server = SourceServer::start().await;
        let unspaced = Limits {
            spacing: Duration::ZERO,
            ..Limits::default()
        };
        let drive = server.drive_with(Limits {
            spacing: Duration::from_millis(300),
            ..Limits::default()
        });
        let tistory = Source::Tistory(server.source_over(unspaced, drive.clone()));
        let blogger = Source::Blogger(server.blogger_over(unspaced, drive));
        server.post(
            "felia",
            1187,
            vec![PostAnswer::Page(drive_page("1tistoryfile0001"))],
        );
        let tistory_post = Url::parse(&server.post_url("felia", 1187)).unwrap();
        let blogger_post = csora(&server);
        for (id, name) in [("1tistoryfile0001", "01.ass"), ("1episode0013", "13.ass")] {
            server.drive(
                id,
                vec![DriveAnswer::File {
                    name: name.into(),
                    bytes: crate::fake::ass(name),
                }],
            );
        }
        let Opened::Files(from_tistory) = tistory.open(&tistory_post, "1").await.unwrap() else {
            panic!("files");
        };
        let Opened::Files(from_blogger) = blogger.open(&blogger_post, "13").await.unwrap() else {
            panic!("files");
        };

        for _ in 0..2 {
            bytes_of(&tistory, &tistory_post, &from_tistory[0]).await;
            bytes_of(&blogger, &blogger_post, &from_blogger[1]).await;
        }
        let drive: Vec<Instant> = server
            .seen()
            .iter()
            .filter(|s| crate::drive::HOSTS.contains(&s.host.as_str()))
            .map(|s| s.at)
            .collect();
        assert_eq!(drive.len(), 4);
        assert!(drive
            .windows(2)
            .all(|w| w[1] - w[0] >= Duration::from_millis(200)));
    }

    #[tokio::test]
    async fn drive_and_the_cdn_set_a_cookie_the_sources_never_send() {
        // The cookie checks above mean something only while these hosts
        // set one.
        let server = SourceServer::start().await;
        let client = builder().build().unwrap();
        for (url, domain) in [
            (
                format!("http://{DRIVE_FILES}:{}/download?id=x", server.port),
                "domain=.google.com",
            ),
            (
                format!("http://{CDN}:{}/dna/x/y/z/a.zip", server.port),
                "domain=.kakaocdn.net",
            ),
        ] {
            let response = client.get(&url).send().await.unwrap();
            let cookie = response.headers().get(header::SET_COOKIE).unwrap();
            let cookie = cookie.to_str().unwrap();
            assert!(
                cookie.starts_with("NID=") && cookie.contains(domain),
                "{cookie}"
            );
        }
    }

    /// 공룡이's post of 네죽사 8화 (2026-10-03): a ZIP of 1~8화 and the ASS
    /// of 8화, inside the inner frame.
    fn elaina(server: &SourceServer) -> (Url, Vec<u8>, Vec<u8>) {
        let zip = verify::zip_of(&[("네죽사 08.ass", crate::fake::ass("08").as_slice())]);
        let ass = crate::fake::ass("Kimi ga Shinu 08");
        server.naver_post(
            "elainalove1017",
            "224324105274",
            vec![PostAnswer::Naver(vec![
                naver_file("네죽사 1~8화 자막.zip", zip.len()),
                naver_file(ELAINA_ASS, ass.len()),
            ])],
        );
        (
            Url::parse(&server.naver_url("elainalove1017", "224324105274")).unwrap(),
            zip,
            ass,
        )
    }

    const ELAINA_ASS: &str =
        "[SubsPlease] Kimi ga Shinu made Koi wo Shitai - 08 (1080p) [F3B053C5].ass";

    #[tokio::test]
    async fn a_naver_posts_attachments_in_its_inner_frame_are_received() {
        let server = SourceServer::start().await;
        let (post, zip, ass) = elaina(&server);
        server.naver_attachment(ELAINA_ASS, vec![FileAnswer::Bytes(ass.clone())]);
        server.naver_attachment(
            "네죽사 1~8화 자막.zip",
            vec![FileAnswer::Bytes(zip.clone())],
        );
        let source = Source::Naver(server.naver());

        let Opened::Files(files) = source.open(&post, "8").await.unwrap() else {
            panic!("files");
        };
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["네죽사 1~8화 자막.zip", ELAINA_ASS]);
        assert_eq!(
            files[1].snapshot.entries(),
            [
                (naver::PUBLISH_DATE.to_owned(), NAVER_PUBLISHED.to_owned()),
                (naver::ATTACH_FILE_SIZE.to_owned(), ass.len().to_string()),
            ]
        );
        for (file, bytes) in files.iter().zip([zip, ass]) {
            let (expected, got) = bytes_of(&source, &post, file).await;
            assert_eq!((expected, got), (Some(bytes.len() as u64), bytes));
        }
        // The inner page was read, never the frame around it; nothing carried
        // a cookie or a `Referer`.
        let seen = server.seen();
        assert_eq!(server.naver_read("224324105274"), 1);
        assert!(seen
            .iter()
            .all(|s| s.path != "/elainalove1017/224324105274"));
        assert_eq!(
            seen.iter().filter(|s| s.host == naver::FILE_HOST).count(),
            2
        );
        assert!(seen.iter().all(|s| !s.cookie && !s.referer));

        // Episode 9: neither file.
        let failure = source.open(&post, "9").await.unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert!(failure.reason.contains("9화"));
    }

    #[tokio::test]
    async fn a_refused_naver_address_is_read_again_once_then_expired_or_missing() {
        let server = SourceServer::start().await;
        let (post, _, ass) = elaina(&server);
        let source = Source::Naver(server.naver());
        let Opened::Files(files) = source.open(&post, "8").await.unwrap() else {
            panic!("files");
        };
        let file = &files[1];

        // Refused once: the post is read again, and the new address works.
        server.naver_attachment(
            ELAINA_ASS,
            vec![FileAnswer::Refused, FileAnswer::Bytes(ass.clone())],
        );
        let (_, bytes) = bytes_of(&source, &post, file).await;
        assert_eq!(bytes, ass);
        assert_eq!(server.naver_read("224324105274"), 2);
        let tokens: Vec<String> = server
            .seen()
            .iter()
            .filter(|s| s.host == naver::FILE_HOST)
            .map(|s| s.path.split('/').nth(3).unwrap().to_owned())
            .collect();
        assert_eq!(tokens.len(), 2);
        assert_ne!(tokens[0], tokens[1], "the second address is the new one");

        // Refused again: expired.
        server.naver_attachment(ELAINA_ASS, vec![FileAnswer::Refused]);
        let failure = source.fetch(&post, file).await.err().unwrap();
        assert_eq!(failure.kind, FailureKind::Expired);
        assert_eq!(failure.status, Some(400));
        assert!(failure.reason.contains("다시 읽어"));
        assert!(!failure.reason.contains("open") && !failure.reason.contains("T1"));

        // Gone from the post read again: missing.
        server.naver_post(
            "elainalove1017",
            "224324105274",
            vec![PostAnswer::Naver(vec![naver_file(
                "네죽사 1~8화 자막.zip",
                10,
            )])],
        );
        let failure = source.fetch(&post, file).await.err().unwrap();
        assert_eq!(failure.kind, FailureKind::Missing);
        assert!(failure.reason.contains("파일이 없어요"));
    }

    #[tokio::test]
    async fn a_flagged_naver_file_is_never_asked_for() {
        let server = SourceServer::start().await;
        let mut flagged = naver_file("Title 08.ass", 100);
        flagged.malicious = true;
        let mut punished = naver_file("Title 08.smi", 100);
        punished.punish = "2".to_owned();
        server.naver_post(
            "blog",
            "1",
            vec![PostAnswer::Naver(vec![flagged, punished])],
        );
        let post = Url::parse(&server.naver_url("blog", "1")).unwrap();
        let source = Source::Naver(server.naver());
        let Opened::Files(files) = source.open(&post, "8").await.unwrap() else {
            panic!("files");
        };
        let failure = source.fetch(&post, &files[0]).await.err().unwrap();
        assert_eq!(failure.kind, FailureKind::Missing);
        assert!(failure.reason.contains("악성 코드"));
        let failure = source.fetch(&post, &files[1]).await.err().unwrap();
        assert_eq!(failure.kind, FailureKind::Missing);
        assert!(failure.reason.contains("제한"));
        assert!(server.seen().iter().all(|s| s.host != naver::FILE_HOST));
        assert_eq!(server.naver_read("1"), 1);
    }

    #[tokio::test]
    async fn a_naver_answer_is_held_to_the_size_the_post_gave() {
        let server = SourceServer::start().await;
        server.naver_post(
            "blog",
            "2",
            vec![PostAnswer::Naver(vec![
                naver_file("Title 03.srt", SRT.len() + 1),
                naver_file("Title 04.srt", SRT.len()),
            ])],
        );
        server.naver_attachment("Title 03.srt", vec![FileAnswer::Bytes(SRT.to_vec())]);
        server.naver_attachment("Title 04.srt", vec![FileAnswer::Streamed(SRT.to_vec())]);
        let post = Url::parse(&server.naver_url("blog", "2")).unwrap();
        let source = Source::Naver(server.naver());
        let open = |episode: &'static str| {
            let source = source.clone();
            let post = post.clone();
            async move {
                match source.open(&post, episode).await.unwrap() {
                    Opened::Files(files) => files,
                    other => panic!("files: {other:?}"),
                }
            }
        };
        // Another length announced: not the file.
        let file = &open("3").await[0];
        let failure = source.fetch(&post, file).await.err().unwrap();
        assert_eq!(failure.kind, FailureKind::NotAFile);
        assert!(failure.reason.contains(&format!("{}바이트", SRT.len() + 1)));
        // No length announced: the post's is the one the bytes are held to.
        let file = &open("4").await[0];
        let (expected, bytes) = bytes_of(&source, &post, file).await;
        assert_eq!((expected, bytes), (Some(SRT.len() as u64), SRT.to_vec()));
    }

    #[tokio::test]
    async fn a_page_instead_of_the_naver_post_is_a_classified_failure() {
        let server = SourceServer::start().await;
        let source = Source::Naver(server.naver());
        let cases = [
            (
                PostAnswer::Page(
                    r#"<html><body><form><img id="captchaimg" src="x"><p>자동입력 방지 문자를 입력해 주세요</p></form></body></html>"#
                        .to_owned(),
                ),
                FailureKind::Changed,
                "CAPTCHA",
            ),
            (
                PostAnswer::Page("<html><body>점검 중이에요</body></html>".to_owned()),
                FailureKind::Changed,
                "다른 페이지",
            ),
            (PostAnswer::Status(404), FailureKind::Missing, "없어요"),
            (PostAnswer::Status(503), FailureKind::Network, "주지 못했어요"),
        ];
        let post = Url::parse(&server.naver_url("blog", "3")).unwrap();
        for (answer, kind, says) in cases {
            server.naver_post("blog", "3", vec![answer.clone()]);
            let failure = source.open(&post, "1").await.unwrap_err();
            assert_eq!(failure.kind, kind, "{answer:?}");
            assert!(
                failure.reason.contains(says),
                "{answer:?}: {}",
                failure.reason
            );
        }
        // An address of no post is asked for nothing.
        let home = Url::parse(&format!("http://blog.naver.com:{}/blog", server.port)).unwrap();
        let before = server.seen().len();
        let failure = source.open(&home, "1").await.unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert_eq!(server.seen().len(), before);
        assert!(server.seen().iter().all(|s| s.host != naver::FILE_HOST));
    }
}
