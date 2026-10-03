//! The subtitle sources (`docs/specs/jobs.md`, 출처별 단계 초안): how the
//! app reads a creator's post and receives the files it offers.
//!
//! A job ([`trss-jobs`]) hands a candidate's post address to [`Sources`], which
//! names the [`Source`] that knows the address's site, or none. The source
//! opens the post for the candidate's episode ([`Source::open`]): the files it
//! offers for it ([`PostFile`]), that
//! a person has to pass the site's check first ([`Opened::NeedsAuth`]), that
//! the subtitle is somewhere this app cannot read yet ([`Opened::Elsewhere`]),
//! or why it cannot ([`Failure`]). It then receives one file at a time
//! ([`Source::fetch`]) as a stream of bytes with the length the site announced
//! before them, which the job writes to its own temporary file and checks
//! ([`verify`]) before it counts as received (`docs/specs/jobs.md`, 공통 수신
//! 결과와 실패 분류).
//!
//! Sources are an enum rather than trait objects: the set is closed and each
//! one is async. [`tistory::TistorySource`] reads Tistory's attachments over
//! HTTP, [`naver::NaverSource`] Naver blogs' attachments, and
//! [`blogger::BloggerSource`] Blogger's posts; all of them receive the Google
//! Drive files a post links ([`drive`]). Where a post says which file is
//! which episode, they offer those that serve the episode ([`episode`]). [`fake::FakeSource`] is a source with no network
//! that lets a job run from start to end in tests and in the development
//! environment.
//!
//! No value here keeps a cookie, a token or a signed download address where it
//! could be stored, logged or shown: a [`PostFile::key`] names a file within
//! its post in words that stay the same from one reading to the next, and the
//! signed address a source found when it opened the post lives only in memory
//! until it fetches the file (a [`PostFile`]'s locator, which its `Debug`
//! hides and its equality ignores), then inside the [`Fetch`] that reads it.
//!
//! [`trss-jobs`]: ../trss_jobs/index.html

pub mod blogger;
pub mod drive;
pub mod episode;
pub mod fake;
pub mod http;
pub mod naver;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod tistory;
pub mod upload;
pub mod verify;

use std::time::Duration;

use bytes::Bytes;
use url::Url;

use blogger::BloggerSource;
use fake::{FakeBody, FakeSource};
use naver::NaverSource;
use tistory::TistorySource;

/// The most bytes one file may have: far more than any subtitle or its
/// series ZIP. A source stops receiving past it ([`FailureKind::NotAFile`]),
/// and the check refuses a larger file ([`verify::check`]).
pub const MAX_FILE_BYTES: u64 = 200 * 1024 * 1024;

/// How long one file may take to come, from its request to its last byte.
pub const FILE_DEADLINE: Duration = Duration::from_secs(15 * 60);

/// A file a post offers.
#[derive(Debug, Clone)]
pub struct PostFile {
    /// What names this file within its source, the same at every reading and
    /// free of secret values. Two posts that offer one file give it the same
    /// key, so a job receives it once.
    pub key: String,
    /// The file's name as the site gives it.
    pub name: String,
    /// The size the site shows beside the file, as it shows it (Tistory's
    /// `0.01MB`). It is coarse: a value of the snapshot, never the length the
    /// bytes are held to.
    pub size_text: Option<String>,
    /// What the site said about the file and its post when it was read.
    pub snapshot: Snapshot,
    /// Where the bytes are, as the post gave it: a signed address. Only the
    /// source reads it.
    locator: Option<Locator>,
}

impl PostFile {
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> PostFile {
        PostFile {
            key: key.into(),
            name: name.into(),
            size_text: None,
            snapshot: Snapshot::default(),
            locator: None,
        }
    }

    pub(crate) fn locator(&self) -> Option<&Url> {
        self.locator.as_ref().map(|l| &l.0)
    }

    pub(crate) fn with_locator(mut self, url: Url) -> PostFile {
        self.locator = Some(Locator(url));
        self
    }
}

/// Two readings of one file are equal whatever address each was given.
impl PartialEq for PostFile {
    fn eq(&self, other: &PostFile) -> bool {
        self.key == other.key
            && self.name == other.name
            && self.size_text == other.size_text
            && self.snapshot == other.snapshot
    }
}

impl Eq for PostFile {}

/// A signed download address, kept in memory between opening a post and
/// fetching its file. Its `Debug` never shows it.
#[derive(Clone)]
struct Locator(Url);

impl std::fmt::Debug for Locator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Locator(<hidden>)")
    }
}

/// What a source read about a file and its post, as named values (Tistory's
/// `article:modified_time` and the size it shows, Blogger's `dateModified`;
/// an answer's `Last-Modified` and Drive's `Content-Length`). Each is compared only with the next snapshot of the same
/// path. No value holds a secret.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot(Vec<(String, String)>);

impl Snapshot {
    pub fn push(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.0.push((name.into(), value.into()));
    }

    pub fn extend(&mut self, other: &Snapshot) {
        self.0.extend(other.0.iter().cloned());
    }

    pub fn entries(&self) -> &[(String, String)] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What a source reads again about a file it received before, without
/// receiving it ([`Source::recheck`]; `docs/specs/subtitles.md`, 구독 제작자
/// 자동 수신). Each value is what the site gives with no sign-in; one the site
/// does not give is `None`, and a missing value is never a difference.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileInfo {
    /// The file's whole size in bytes: a Drive file's `Content-Length` (a
    /// `HEAD` answer), a Tistory attachment's total in the `Content-Range` of
    /// a one-byte range request, a Naver attachment's `attachFileSize`.
    pub size: Option<u64>,
    /// The `Last-Modified` the answer gave, as it wrote it. Only Drive's
    /// answers have one: Tistory's and Naver's files have no modified time to
    /// read, so a same-size edit shows nothing there.
    pub last_modified: Option<String>,
}

/// What opening a post came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// The files the post offers, in the post's order.
    Files(Vec<PostFile>),
    /// A person has to pass the site's check before anything can be read;
    /// `reason` names it in a few words (`"CAPTCHA"`).
    NeedsAuth { reason: String },
    /// The post's subtitle is somewhere this app cannot read yet (a Google
    /// Drive folder, a WinPNG image): the item waits for a source (`자막 대기`)
    /// as a post of an unknown site does. `reason` says where, in a sentence.
    Elsewhere { reason: String },
}

/// Why a post or a file could not be read (`docs/specs/jobs.md`, 공통 수신
/// 결과와 실패 분류).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{reason}")]
pub struct Failure {
    pub kind: FailureKind,
    /// One sentence for the job's log and its screen. It holds no cookie,
    /// token or address.
    pub reason: String,
    /// The answer's HTTP status, when there was an answer.
    pub status: Option<u16>,
    /// The answer's media type (`text/html`), without its parameters.
    pub content_type: Option<String>,
    /// How many bytes the answer had: its body as read, or what it announced.
    pub size: Option<u64>,
}

/// The classes of a failed receipt, the same for every source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureKind {
    /// The post or the file is gone (`원본 없음`).
    Missing,
    /// The signed address of a file was refused, and reading the post again
    /// did not give one that works (`만료`).
    Expired,
    /// What came back is not the file: an error page, nothing, another length
    /// than announced, or bytes that are not what they claim to be
    /// (`파일 아님`).
    NotAFile,
    /// The post opens but holds nothing the source knows how to read
    /// (`출처 구조 바뀜`).
    Changed,
    /// The site could not be reached or failed (`네트워크 실패`).
    Network,
}

impl FailureKind {
    pub const ALL: [FailureKind; 5] = [
        FailureKind::Missing,
        FailureKind::Expired,
        FailureKind::NotAFile,
        FailureKind::Changed,
        FailureKind::Network,
    ];

    /// The class's code in the records and the API.
    pub fn code(self) -> &'static str {
        match self {
            FailureKind::Missing => "missing",
            FailureKind::Expired => "expired",
            FailureKind::NotAFile => "not_a_file",
            FailureKind::Changed => "changed",
            FailureKind::Network => "network",
        }
    }

    pub fn parse(code: &str) -> Option<FailureKind> {
        FailureKind::ALL.into_iter().find(|k| k.code() == code)
    }

    /// What the screens call it.
    pub fn label(self) -> &'static str {
        match self {
            FailureKind::Missing => "원본 없음",
            FailureKind::Expired => "만료",
            FailureKind::NotAFile => "파일 아님",
            FailureKind::Changed => "출처 구조 바뀜",
            FailureKind::Network => "네트워크 실패",
        }
    }

    /// Whether the same request may succeed a little later: only a network
    /// failure. An expired address is not retried as it is; the source reads
    /// the post again for a new one, once, before it reports `Expired`.
    pub fn retryable(self) -> bool {
        self == FailureKind::Network
    }
}

impl Failure {
    pub fn new(kind: FailureKind, reason: impl Into<String>) -> Failure {
        Failure {
            kind,
            reason: reason.into(),
            status: None,
            content_type: None,
            size: None,
        }
    }

    /// The same failure with the facts of the answer that showed it.
    pub fn with_response(
        mut self,
        status: Option<u16>,
        content_type: Option<String>,
        size: Option<u64>,
    ) -> Failure {
        self.status = status;
        self.content_type = content_type;
        self.size = size;
        self
    }
}

/// One file's bytes as they come.
pub struct Fetch {
    /// The length the site announced before the bytes (`Content-Length`, a
    /// size in the post), when it did. It must be the length of what
    /// [`Fetch::chunk`] yields: a source whose client decodes a compressed
    /// answer gives `None` rather than the encoded length. A restarted job
    /// takes a temporary file of this length as received whole.
    pub expected_size: Option<u64>,
    /// The file's name as the answer gives it (Drive's
    /// `Content-Disposition`), when the post did not: the job receives the
    /// file under it.
    pub name: Option<String>,
    /// The answer's HTTP status and media type, when the bytes come over HTTP:
    /// kept with the receipt, so a file the bytes turn out not to be says
    /// what answered.
    pub status: Option<u16>,
    pub content_type: Option<String>,
    /// What the answer said about the file (`Last-Modified`), added to the
    /// post's snapshot of it.
    pub snapshot: Snapshot,
    body: Body,
    /// The bytes yielded so far, and the most there may be.
    received: u64,
    max_bytes: u64,
    /// When the last byte must have come, and the deadline it is from.
    deadline: Option<(tokio::time::Instant, Duration)>,
}

/// `200MiB`, or a smaller limit in bytes.
pub(crate) fn size_limit_text(bytes: u64) -> String {
    match bytes % (1 << 20) {
        0 => format!("{}MiB", bytes >> 20),
        _ => format!("{bytes}바이트"),
    }
}

enum Body {
    Fake(FakeBody),
    Http(reqwest::Response),
}

impl Fetch {
    pub(crate) fn new(
        expected_size: Option<u64>,
        status: Option<u16>,
        content_type: Option<String>,
        snapshot: Snapshot,
        body: Body,
        max_bytes: u64,
        deadline: Option<(tokio::time::Instant, Duration)>,
    ) -> Fetch {
        Fetch {
            expected_size,
            name: None,
            status,
            content_type,
            snapshot,
            body,
            received: 0,
            max_bytes,
            deadline,
        }
    }

    /// The next piece of the file; `None` at its end. Bytes past the file's
    /// limit are [`FailureKind::NotAFile`]; the deadline passing is
    /// [`FailureKind::Network`].
    pub async fn chunk(&mut self) -> Result<Option<Bytes>, Failure> {
        let piece = match &mut self.body {
            Body::Fake(body) => body.chunk().await,
            Body::Http(response) => {
                let next = response.chunk();
                let next = match self.deadline {
                    Some((at, deadline)) => tokio::time::timeout_at(at, next)
                        .await
                        .map_err(|_| http::deadline_failure(deadline))?,
                    None => next.await,
                };
                next.map_err(|e| http::network_failure(&e, "받는 도중에 연결이 끊겼어요"))?
            }
        };
        if let Some(piece) = &piece {
            self.received += piece.len() as u64;
            if self.received > self.max_bytes {
                return Err(Failure::new(
                    FailureKind::NotAFile,
                    format!(
                        "파일이 받을 수 있는 크기({})를 넘어 받기를 멈췄어요",
                        size_limit_text(self.max_bytes)
                    ),
                )
                .with_response(
                    self.status,
                    self.content_type.clone(),
                    Some(self.received),
                ));
            }
        }
        Ok(piece)
    }
}

/// A site the app knows how to read.
#[derive(Debug, Clone)]
pub enum Source {
    Fake(FakeSource),
    Tistory(TistorySource),
    Blogger(BloggerSource),
    Naver(NaverSource),
}

impl Source {
    /// Reads the post at `post` for the files it offers for `episode`, the
    /// candidate's episode as Anissia wrote it. A post that says which file is
    /// which episode offers the episode's (and the fonts); one that does not
    /// offers them all.
    pub async fn open(&self, post: &Url, episode: &str) -> Result<Opened, Failure> {
        match self {
            Source::Fake(source) => source.open(post).await,
            Source::Tistory(source) => source.open(post, episode).await,
            Source::Blogger(source) => source.open(post, episode).await,
            Source::Naver(source) => source.open(post, episode).await,
        }
    }

    /// Reads again what the site tells about the files `keys` name
    /// ([`PostFile::key`]) of the post at `post`, without receiving any: a
    /// request that gets no body for each file (a Drive file's `HEAD`, a
    /// Tistory attachment's one-byte range) after reading the post again for a
    /// signed address, or the post's own list of attachments (Naver). The post
    /// is read once, and only when a key needs it (a Drive file's address is
    /// always the same).
    ///
    /// One answer per key, in the order of `keys`: the file's information, or
    /// why the site could not give it. A post that cannot be read gives that
    /// failure for each key that needs it; a file the post no longer offers is
    /// [`FailureKind::Missing`]. Requests keep the sources' spacing per host.
    pub async fn recheck(
        &self,
        post: &Url,
        keys: &[String],
    ) -> Vec<(String, Result<FileInfo, Failure>)> {
        match self {
            Source::Fake(source) => source.recheck(post, keys).await,
            Source::Tistory(source) => source.recheck(post, keys).await,
            Source::Blogger(source) => source.recheck(post, keys).await,
            Source::Naver(source) => source.recheck(post, keys).await,
        }
    }

    /// Starts receiving `file` of the post at `post`.
    pub async fn fetch(&self, post: &Url, file: &PostFile) -> Result<Fetch, Failure> {
        match self {
            Source::Fake(source) => {
                let body = source.fetch(post, file).await?;
                Ok(Fetch::new(
                    body.declared_size(),
                    None,
                    None,
                    Snapshot::default(),
                    Body::Fake(body),
                    MAX_FILE_BYTES,
                    None,
                ))
            }
            Source::Tistory(source) => source.fetch(post, file).await,
            Source::Blogger(source) => source.fetch(post, file).await,
            Source::Naver(source) => source.fetch(post, file).await,
        }
    }
}

/// The sources this process knows, and which one reads an address.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    fake: Option<FakeSource>,
    tistory: Option<TistorySource>,
    blogger: Option<BloggerSource>,
    naver: Option<NaverSource>,
}

impl Sources {
    /// No source: every post waits for one (`자막 대기`).
    pub fn none() -> Sources {
        Sources::default()
    }

    /// Adds the fake source, for the posts on [`fake::HOST`].
    pub fn with_fake(mut self, source: FakeSource) -> Sources {
        self.fake = Some(source);
        self
    }

    /// Adds the Tistory source, for the posts on a blog of
    /// [`tistory::reads`].
    pub fn with_tistory(mut self, source: TistorySource) -> Sources {
        self.tistory = Some(source);
        self
    }

    /// Adds the Blogger source, for the posts on a blog of
    /// [`blogger::reads`].
    pub fn with_blogger(mut self, source: BloggerSource) -> Sources {
        self.blogger = Some(source);
        self
    }

    /// Adds the Naver source, for the posts on a blog of [`naver::reads`].
    pub fn with_naver(mut self, source: NaverSource) -> Sources {
        self.naver = Some(source);
        self
    }

    /// The source that reads the post at `post`, if this process knows one.
    pub fn for_post(&self, post: &Url) -> Option<Source> {
        match post.host_str() {
            Some(fake::HOST) => self.fake.clone().map(Source::Fake),
            Some(host) if tistory::reads(host) => self.tistory.clone().map(Source::Tistory),
            Some(host) if blogger::reads(host) => self.blogger.clone().map(Source::Blogger),
            Some(host) if naver::reads(host) => self.naver.clone().map(Source::Naver),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_post_files_debug_hides_its_address_and_equality_ignores_it() {
        let signed =
            Url::parse("https://blog.kakaocdn.net/dna/a/b/c/x.zip?credential=C&signature=S")
                .unwrap();
        let file = PostFile::new("tistory:blog.kakaocdn.net/dna/a/b/c/x.zip", "x.zip")
            .with_locator(signed.clone());
        let shown = format!("{file:?}");
        assert!(!shown.contains("signature") && !shown.contains("credential"));
        assert!(shown.contains("<hidden>"));
        assert_eq!(file.locator(), Some(&signed));
        assert_eq!(
            file,
            PostFile::new("tistory:blog.kakaocdn.net/dna/a/b/c/x.zip", "x.zip")
        );
    }

    #[test]
    fn only_a_network_failure_is_retried_and_every_class_has_a_code() {
        for kind in FailureKind::ALL {
            assert_eq!(FailureKind::parse(kind.code()), Some(kind));
            assert_eq!(kind.retryable(), kind == FailureKind::Network);
        }
    }
}
