//! The subtitle sources (`docs/specs/jobs.md`, 출처별 단계 초안): how the
//! app reads a creator's post and receives the files it offers.
//!
//! A job ([`trss-jobs`]) hands a candidate's post address to [`Sources`], which
//! names the [`Source`] that knows the address's site, or none. The source
//! opens the post ([`Source::open`]): the files it offers ([`PostFile`]), or
//! that a person has to pass the site's check first ([`Opened::NeedsAuth`]),
//! or why it cannot ([`Failure`]). It then receives one file at a time
//! ([`Source::fetch`]) as a stream of bytes with the length the site announced
//! before them, which the job writes to its own temporary file.
//!
//! Sources are an enum rather than trait objects: the set is closed and each
//! one is async. The real sites come with the tickets that implement them; for
//! now there is only [`fake::FakeSource`], a source with no network that lets
//! a job run from start to end in tests and in the development environment.
//!
//! No value here holds a cookie, a token or a signed download address: a
//! [`PostFile::key`] names a file within its post in words that stay the same
//! from one reading to the next, and the address the bytes come from lives
//! only inside the [`Fetch`] that reads them.
//!
//! [`trss-jobs`]: ../trss_jobs/index.html

pub mod fake;

use bytes::Bytes;
use url::Url;

use fake::{FakeBody, FakeSource};

/// A file a post offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostFile {
    /// What names this file within its source, the same at every reading and
    /// free of secret values. Two posts that offer one file give it the same
    /// key, so a job receives it once.
    pub key: String,
    /// The file's name as the site gives it.
    pub name: String,
}

/// What opening a post came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// The files the post offers, in the post's order.
    Files(Vec<PostFile>),
    /// A person has to pass the site's check before anything can be read;
    /// `reason` names it in a few words (`"CAPTCHA"`).
    NeedsAuth { reason: String },
}

/// Why a post or a file could not be read (`docs/specs/jobs.md`, 공통 수신
/// 결과와 실패 분류 초안).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{reason}")]
pub struct Failure {
    pub kind: FailureKind,
    /// One sentence for the job's log and its screen. It holds no cookie,
    /// token or signed address.
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The post or the file is gone (`원본 없음`).
    Missing,
    /// The post opens but holds nothing the source knows how to read
    /// (`출처 구조 바뀜`).
    Changed,
    /// What came back is not the file: an error page, nothing, or another
    /// length than announced (`파일 아님`).
    NotAFile,
    /// The site could not be reached or failed (`네트워크 실패`).
    Network,
}

impl Failure {
    pub fn new(kind: FailureKind, reason: impl Into<String>) -> Failure {
        Failure {
            kind,
            reason: reason.into(),
        }
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
    body: Body,
}

enum Body {
    Fake(FakeBody),
}

impl Fetch {
    /// The next piece of the file; `None` at its end.
    pub async fn chunk(&mut self) -> Result<Option<Bytes>, Failure> {
        match &mut self.body {
            Body::Fake(body) => Ok(body.chunk().await),
        }
    }
}

/// A site the app knows how to read.
#[derive(Debug, Clone)]
pub enum Source {
    Fake(FakeSource),
}

impl Source {
    /// Reads the post at `post` for the files it offers.
    pub async fn open(&self, post: &Url) -> Result<Opened, Failure> {
        match self {
            Source::Fake(source) => source.open(post).await,
        }
    }

    /// Starts receiving `file` of the post at `post`.
    pub async fn fetch(&self, post: &Url, file: &PostFile) -> Result<Fetch, Failure> {
        match self {
            Source::Fake(source) => {
                let body = source.fetch(post, file).await?;
                Ok(Fetch {
                    expected_size: body.declared_size(),
                    body: Body::Fake(body),
                })
            }
        }
    }
}

/// The sources this process knows, and which one reads an address.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    fake: Option<FakeSource>,
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

    /// The source that reads the post at `post`, if this process knows one.
    pub fn for_post(&self, post: &Url) -> Option<Source> {
        match post.host_str() {
            Some(fake::HOST) => self.fake.clone().map(Source::Fake),
            _ => None,
        }
    }
}
