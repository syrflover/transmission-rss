//! A local HTTP server shaped like Tistory, for the tests here and of the
//! crates above (feature `test-support`).
//!
//! Every name resolves to the server ([`TistoryServer::source`]), so the
//! addresses keep their real shape with the server's port:
//! `http://<blog>.tistory.com:<port>/<n>` for a post and
//! `http://blog.kakaocdn.net:<port>/dna/<id>/<name>?credential=…&signature=…`
//! for its files; any CDN host ([`tistory::cdn_host`]) serves the files. Each
//! post and file answers from a script a test sets, one answer per request,
//! the last one again and again. Every serving of a post signs its addresses
//! anew, as Tistory does. The source it gives takes plain `http`, which the
//! source over the network refuses.

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

use crate::tistory::{self, Limits, Reach, TistorySource};

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

/// A request the server saw.
#[derive(Debug, Clone)]
pub struct Seen {
    pub host: String,
    pub path: String,
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
    seen: Vec<Seen>,
    signed: u64,
}

/// The server, until it is dropped.
pub struct TistoryServer {
    port: u16,
    state: Arc<Mutex<State>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TistoryServer {
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

impl TistoryServer {
    pub async fn start() -> TistoryServer {
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
        TistoryServer { port, state, task }
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

    /// The source with these limits (a smaller file, a shorter deadline).
    pub fn source_with(&self, limits: Limits) -> TistorySource {
        let reach = Reach { plain_http: true };
        let builder = reqwest::Client::builder()
            .no_proxy()
            .dns_resolver(Arc::new(Loopback));
        TistorySource::over(tistory::client(builder, reach), limits, reach)
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

/// The body container of the posts seen (`tistory::BODY`).
pub const BODY_OPEN: &str = r#"<div class="tt_article_useless_p_margin contents_style">"#;

/// A post whose subtitle is a Google Drive link.
pub fn drive_page() -> String {
    r#"<!doctype html><html><body><div class="tt_article_useless_p_margin contents_style"><p>24화 자막</p>
<p><a href="https://drive.google.com/file/d/1AbCdEf/view?usp=drive_link">https://drive.google.com/file/d/1AbCdEf/view?usp=drive_link</a></p>
</div></body></html>"#
        .to_owned()
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

fn answer(state: &Mutex<State>, request: Request<Body>) -> Response {
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    let headers = request.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default().to_owned();
    let path = request.uri().path().to_owned();
    state.seen.push(Seen {
        host: host.clone(),
        path: path.clone(),
        at: Instant::now(),
        cookie: headers.contains_key(header::COOKIE),
        referer: headers.contains_key(header::REFERER),
    });

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
        None => page(404, 1952),
    }
}

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
        match source.open(post).await.unwrap() {
            Opened::Files(files) => files,
            other => panic!("files: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_zip_comes_whole_with_no_cookie_or_referer() {
        let server = TistoryServer::start().await;
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
        let server = TistoryServer::start().await;
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
        let server = TistoryServer::start().await;
        let source = Source::Tistory(server.source());
        let url = |n| Url::parse(&server.post_url("blog", n)).unwrap();
        server.post("blog", 2, vec![PostAnswer::Status(503)]);
        server.post("blog", 3, vec![PostAnswer::Page(drive_page())]);

        let missing = source.open(&url(99999)).await.unwrap_err();
        assert_eq!(
            (missing.kind, missing.status, missing.size),
            (FailureKind::Missing, Some(404), Some(1952))
        );
        assert_eq!(
            source.open(&url(2)).await.unwrap_err().kind,
            FailureKind::Network
        );
        assert!(matches!(
            source.open(&url(3)).await.unwrap(),
            Opened::Elsewhere { .. }
        ));
    }

    #[tokio::test]
    async fn a_redirect_is_followed_on_the_cdn_with_no_referer_and_stopped_elsewhere() {
        let server = TistoryServer::start().await;
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
        let server = TistoryServer::start().await;
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
        let server = TistoryServer::start().await;
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
        let server = TistoryServer::start().await;
        let specs: Vec<FileSpec> = (0..3)
            .map(|i| spec(&format!("f{i}"), &format!("{i}.srt"), "1KB"))
            .collect();
        server.post("blog", 1, vec![PostAnswer::Files(specs.clone())]);
        for s in &specs {
            server.file(&s.id, vec![FileAnswer::Bytes(SRT.to_vec())]);
        }
        let source = Source::Tistory(server.source_spaced(Duration::from_millis(150)));
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
            .all(|w| w[1] - w[0] >= Duration::from_millis(140)));
    }
}
