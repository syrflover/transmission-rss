//! A source with no network, for the tests and the development environment:
//! posts on [`HOST`] (a reserved name that resolves nowhere) whose path says
//! what the post does.
//!
//! | Path | What opening and receiving it does |
//! | --- | --- |
//! | `/ok/<name>` | one file, `<name>.ass` |
//! | `/shared/<series>/<anything>` | one file, `<series>.ass`, the same file for every post of the series |
//! | `/auth/<anything>` | a person has to pass a check (`CAPTCHA`) |
//! | `/missing/<anything>` | the post is gone |
//! | `/empty/<anything>` | the post offers no file |
//! | `/short/<name>` | one file, `<name>.ass`, whose bytes stop 16 short of the length announced |
//!
//! `?delay_ms=<n>` waits `n` milliseconds before each [`CHUNK`] bytes of a file
//! (about 30 of them), so a test can stop the worker in the middle of one.
//!
//! The bytes are a small valid ASS file made from the name, the same every
//! time, so a second receipt of a file has the first one's SHA-256.

use std::time::Duration;

use bytes::Bytes;
use url::Url;

use crate::{Failure, FailureKind, Opened, PostFile};

/// The host of the fake posts.
pub const HOST: &str = "fake.trss.invalid";

/// How many bytes one piece of a fake file has.
pub const CHUNK: usize = 64;

/// How much shorter than announced a `/short/` file is.
const SHORT_BY: u64 = 16;

#[derive(Debug, Clone, Default)]
pub struct FakeSource;

/// A fake post's path, read.
enum Post {
    Ok(String),
    Shared(String),
    Auth,
    Missing,
    Empty,
    Short(String),
}

fn read(post: &Url) -> Result<Post, Failure> {
    let segments: Vec<&str> = post
        .path_segments()
        .map(|s| s.filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let name = |i: usize| segments.get(i).map(|s| (*s).to_owned());
    let changed = || Failure::new(FailureKind::Changed, "가짜 출처가 모르는 주소예요");
    Ok(match segments.first().copied() {
        Some("ok") => Post::Ok(name(1).ok_or_else(changed)?),
        Some("shared") => Post::Shared(name(1).ok_or_else(changed)?),
        Some("short") => Post::Short(name(1).ok_or_else(changed)?),
        Some("auth") => Post::Auth,
        Some("missing") => Post::Missing,
        Some("empty") => Post::Empty,
        _ => return Err(changed()),
    })
}

fn delay(post: &Url) -> Duration {
    post.query_pairs()
        .find(|(k, _)| k == "delay_ms")
        .and_then(|(_, v)| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or_default()
}

impl FakeSource {
    pub(crate) async fn open(&self, post: &Url) -> Result<Opened, Failure> {
        let file = |key: String, name: &str| PostFile::new(key, format!("{name}.ass"));
        Ok(match read(post)? {
            Post::Ok(name) => Opened::Files(vec![file(format!("ok/{name}"), &name)]),
            Post::Short(name) => Opened::Files(vec![file(format!("short/{name}"), &name)]),
            Post::Shared(series) => Opened::Files(vec![file(format!("shared/{series}"), &series)]),
            Post::Auth => Opened::NeedsAuth {
                reason: "CAPTCHA".to_owned(),
            },
            Post::Missing => {
                return Err(Failure::new(FailureKind::Missing, "게시물이 없어요 (404)"))
            }
            Post::Empty => Opened::Files(Vec::new()),
        })
    }

    pub(crate) async fn fetch(&self, post: &Url, file: &PostFile) -> Result<FakeBody, Failure> {
        let short = matches!(read(post)?, Post::Short(_));
        let name = file.name.strip_suffix(".ass").unwrap_or(&file.name);
        let mut bytes = ass(name);
        let declared = bytes.len() as u64;
        if short {
            bytes.truncate(bytes.len() - SHORT_BY as usize);
        }
        Ok(FakeBody {
            bytes: Bytes::from(bytes),
            at: 0,
            declared,
            delay: delay(post),
        })
    }
}

/// A fake file's bytes, a piece at a time.
pub struct FakeBody {
    bytes: Bytes,
    at: usize,
    declared: u64,
    delay: Duration,
}

impl FakeBody {
    pub(crate) fn declared_size(&self) -> Option<u64> {
        Some(self.declared)
    }

    pub(crate) async fn chunk(&mut self) -> Option<Bytes> {
        if self.at >= self.bytes.len() {
            return None;
        }
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        let end = (self.at + CHUNK).min(self.bytes.len());
        let piece = self.bytes.slice(self.at..end);
        self.at = end;
        Some(piece)
    }
}

/// The bytes of the fake file named `name`: a valid ASS file, the same for
/// the same name.
pub fn ass(name: &str) -> Vec<u8> {
    let mut text = format!(
        "\u{feff}[Script Info]\nTitle: {name}\nScriptType: v4.00+\n\n\
         [V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, Bold, Italic, Alignment, MarginV\n\
         Style: Default,Arial,48,&H00FFFFFF,0,0,2,30\n\n\
         [Events]\n\
         Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
    );
    for i in 0..24 {
        text.push_str(&format!(
            "Dialogue: 0,0:00:{:02}.00,0:00:{:02}.50,Default,,0,0,0,,가짜 자막 {name} {}\n",
            i * 2,
            i * 2 + 1,
            i + 1
        ));
    }
    text.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Source, Sources};

    async fn receive(source: &Source, post: &Url, file: &PostFile) -> (Option<u64>, Vec<u8>) {
        let mut fetch = source.fetch(post, file).await.unwrap();
        let mut bytes = Vec::new();
        while let Some(piece) = fetch.chunk().await.unwrap() {
            bytes.extend_from_slice(&piece);
        }
        (fetch.expected_size, bytes)
    }

    #[tokio::test]
    async fn a_post_says_what_it_does_and_its_file_is_the_same_every_time() {
        let sources = Sources::none().with_fake(FakeSource);
        let post = Url::parse("https://fake.trss.invalid/ok/ep1").unwrap();
        let source = sources.for_post(&post).unwrap();

        let Opened::Files(files) = source.open(&post, "1").await.unwrap() else {
            panic!("files");
        };
        assert_eq!(files, [PostFile::new("ok/ep1", "ep1.ass")]);
        let (expected, first) = receive(&source, &post, &files[0]).await;
        let (_, second) = receive(&source, &post, &files[0]).await;
        assert_eq!(expected, Some(first.len() as u64));
        assert_eq!(first, second);
        assert!(first.starts_with("\u{feff}[Script Info]".as_bytes()));
        assert!(first.len() > 20 * CHUNK);
    }

    #[tokio::test]
    async fn posts_of_a_series_share_one_file_and_the_other_paths_wait_fail_or_fall_short() {
        let sources = Sources::none().with_fake(FakeSource);
        let url = |path: &str| Url::parse(&format!("https://fake.trss.invalid{path}")).unwrap();
        let source = sources.for_post(&url("/ok/x")).unwrap();

        let keys = |opened: Opened| match opened {
            Opened::Files(files) => files.into_iter().map(|f| f.key).collect::<Vec<_>>(),
            _ => panic!("files"),
        };
        assert_eq!(
            keys(source.open(&url("/shared/s/1"), "1").await.unwrap()),
            keys(source.open(&url("/shared/s/2"), "1").await.unwrap())
        );
        assert_eq!(
            source.open(&url("/auth/1"), "1").await.unwrap(),
            Opened::NeedsAuth {
                reason: "CAPTCHA".into()
            }
        );
        assert_eq!(
            source.open(&url("/missing/1"), "1").await.unwrap_err().kind,
            FailureKind::Missing
        );
        assert_eq!(
            keys(source.open(&url("/empty/1"), "1").await.unwrap()),
            Vec::<String>::new()
        );

        let post = url("/short/x");
        let Opened::Files(files) = source.open(&post, "1").await.unwrap() else {
            panic!("files");
        };
        let (expected, bytes) = receive(&source, &post, &files[0]).await;
        assert_eq!(expected, Some(bytes.len() as u64 + SHORT_BY));
    }

    #[test]
    fn only_the_fake_host_has_a_source_and_only_when_it_is_on() {
        let post = Url::parse("https://fake.trss.invalid/ok/1").unwrap();
        let other = Url::parse("https://example.tistory.com/1").unwrap();
        assert!(Sources::none().for_post(&post).is_none());
        let sources = Sources::none().with_fake(FakeSource);
        assert!(sources.for_post(&post).is_some());
        assert!(sources.for_post(&other).is_none());
    }
}
