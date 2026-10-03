//! A source with no network, for the tests and the development environment:
//! posts on [`HOST`] (a reserved name that resolves nowhere) whose path says
//! what the post does.
//!
//! | Path | What opening and receiving it does |
//! | --- | --- |
//! | `/ok/<name>` | one file, `<name>.ass` |
//! | `/shared/<series>/<anything>` | one file, `<series>.ass`, the same file for every post of the series |
//! | `/auth/<anything>` | a person has to pass a check (`CAPTCHA`) that nothing shows |
//! | `/check/<name>` | a person has to pass a check in the server browser; then the browser downloads `<name>.srt` |
//! | `/missing/<anything>` | the post is gone |
//! | `/empty/<anything>` | the post offers no file |
//! | `/short/<name>` | one file, `<name>.ass`, whose bytes stop 16 short of the length announced |
//!
//! `?delay_ms=<n>` waits `n` milliseconds before each [`CHUNK`] bytes of a file
//! (about 30 of them), so a test can stop the worker in the middle of one.
//!
//! The bytes are a small valid ASS file made from the name, the same every
//! time, so a second receipt of a file has the first one's SHA-256.
//!
//! # The check in the browser
//!
//! A `/check/<name>` post ([`crate::Opened::BrowserAuth`], [`crate::auth`])
//! is a page in the server browser, served inside the browser with no
//! network: the driver ([`drive_check`]) intercepts the page's requests to
//! [`HOST`] (DevTools' `Fetch` domain) and answers them itself for as long as
//! the run lives. The page has a download card far down; the driver scrolls
//! it to the middle of the screen and clicks it, as erulabo's driver will, and
//! a fake check box appears in the card. Only a person's click on the box (a
//! trusted event: a remote screen's input, not a page script) passes it; the
//! page then starts a real browser download of `/files/<name>.srt`, a small
//! valid SRT file ([`srt`]). The card follows the middle of the screen when
//! the screen's size changes, so the box stays on the first screen.

use std::time::Duration;

use bytes::Bytes;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use trss_browser::Page;
use url::Url;

use crate::{
    auth::{self, AuthPage},
    Failure, FailureKind, FileInfo, Opened, PostFile,
};

/// The host of the fake posts.
pub const HOST: &str = "fake.trss.invalid";

/// What the fake check is called on the job's screen and in its log.
pub const CHECK_REASON: &str = "가짜 사람 확인";

/// A fake post that needs the check in the browser: the name of its file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeCheck {
    name: String,
}

impl FakeCheck {
    /// The name of the file the page downloads, without `.srt`.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Whether `name` is a name a check post may have: it goes into the page as
/// it is.
fn is_check_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

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
    Check(String),
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
        Some("check") => Post::Check(name(1).filter(|n| is_check_name(n)).ok_or_else(changed)?),
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
            Post::Check(name) => Opened::BrowserAuth {
                reason: CHECK_REASON.to_owned(),
                page: AuthPage::Fake(FakeCheck { name }),
            },
            Post::Missing => {
                return Err(Failure::new(FailureKind::Missing, "게시물이 없어요 (404)"))
            }
            Post::Empty => Opened::Files(Vec::new()),
        })
    }

    /// The size of each file, which never changes: the file's name is its
    /// bytes (see [`ass`]).
    pub(crate) async fn recheck(
        &self,
        post: &Url,
        keys: &[String],
    ) -> Vec<(String, Result<FileInfo, Failure>)> {
        let post = read(post);
        keys.iter()
            .map(|key| {
                let info = match &post {
                    Err(failure) => Err(failure.clone()),
                    Ok(Post::Missing) => {
                        Err(Failure::new(FailureKind::Missing, "게시물이 없어요 (404)"))
                    }
                    // What the browser downloaded after the check.
                    Ok(Post::Check(name)) => Ok(FileInfo {
                        size: Some(srt(name).len() as u64),
                        last_modified: None,
                    }),
                    Ok(_) => {
                        let name = key.rsplit('/').next().unwrap_or_default();
                        Ok(FileInfo {
                            size: Some(ass(name).len() as u64),
                            last_modified: None,
                        })
                    }
                };
                (key.clone(), info)
            })
            .collect()
    }

    pub(crate) async fn fetch(&self, post: &Url, file: &PostFile) -> Result<FakeBody, Failure> {
        let post_kind = read(post)?;
        // Its file comes only through the browser, already on this machine.
        if matches!(post_kind, Post::Check(_)) {
            return Err(Failure::new(
                FailureKind::Expired,
                "이 게시물의 파일은 사이트 확인을 거쳐 브라우저로만 받아요",
            ));
        }
        let short = matches!(post_kind, Post::Short(_));
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

/// The bytes of the file a check post's page downloads: a valid SRT file, the
/// same for the same name.
pub fn srt(name: &str) -> Vec<u8> {
    let mut text = String::new();
    for i in 0..12 {
        text.push_str(&format!(
            "{}\r\n00:00:{:02},000 --> 00:00:{:02},500\r\n가짜 확인 자막 {name} {}\r\n\r\n",
            i + 1,
            i * 2,
            i * 2 + 1,
            i + 1
        ));
    }
    text.into_bytes()
}

/// The page of the check post whose file is `name` (a name [`is_check_name`]
/// lets through, so it goes in as it is).
fn check_page(name: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="ko"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>가짜 확인 게시물 {name}</title>
<style>
body {{ font-family: sans-serif; margin: 0; padding: 16px; }}
.filler {{ height: 1600px; background: linear-gradient(#f4f4f4, #d8d8d8); }}
#card {{ border: 2px solid #3366cc; border-radius: 8px; margin: 16px 0; padding: 16px;
        min-height: 180px; display: flex; flex-direction: column; align-items: center;
        justify-content: center; gap: 12px; cursor: pointer; }}
#check {{ display: none; width: 240px; height: 72px; border: 2px solid #888; border-radius: 6px;
         align-items: center; justify-content: center; gap: 10px; background: #fafafa;
         font-size: 18px; cursor: pointer; }}
#check.shown {{ display: flex; }}
#box {{ width: 28px; height: 28px; border: 2px solid #555; border-radius: 4px; }}
#check.done #box {{ background: #22aa22; }}
</style></head>
<body>
<h1>가짜 확인 게시물</h1>
<p>자막은 아래 카드를 눌러 사람 확인을 거친 뒤에 받아요.</p>
<div class="filler"></div>
<div id="card" data-trss-download>
  <strong>{name}.srt 받기</strong>
  <span id="hint">눌러서 받기</span>
  <div id="check" role="checkbox" aria-checked="false"><span id="box"></span><span>사람입니다</span></div>
</div>
<div class="filler"></div>
<script>
const card = document.getElementById('card');
const check = document.getElementById('check');
card.addEventListener('click', () => {{
  check.classList.add('shown');
  document.getElementById('hint').textContent = '확인해 주세요';
}});
check.addEventListener('click', (event) => {{
  event.stopPropagation();
  // Only a person's click passes: a page script's click is not trusted.
  if (!event.isTrusted || check.classList.contains('done')) return;
  check.classList.add('done');
  check.setAttribute('aria-checked', 'true');
  const link = document.createElement('a');
  link.href = '/files/{name}.srt';
  link.download = '{name}.srt';
  document.body.appendChild(link);
  link.click();
}});
window.addEventListener('resize', () => {{
  if (check.classList.contains('shown')) card.scrollIntoView({{block: 'center'}});
}});
</script>
</body></html>
"#
    )
}

/// What the fake site answers for a request to `url`: status, media type,
/// whether it is an attachment, and the body.
fn answer(url: &str) -> (u16, &'static str, Option<String>, Vec<u8>) {
    let Ok(url) = Url::parse(url) else {
        return (404, "text/plain", None, Vec::new());
    };
    if url.host_str() != Some(HOST) {
        return (404, "text/plain", None, Vec::new());
    }
    let segments: Vec<&str> = url
        .path_segments()
        .map(|s| s.filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    match segments.as_slice() {
        ["check", name] if is_check_name(name) => (
            200,
            "text/html; charset=utf-8",
            None,
            check_page(name).into_bytes(),
        ),
        ["files", file] => match file.strip_suffix(".srt").filter(|n| is_check_name(n)) {
            Some(name) => (
                200,
                "application/x-subrip",
                Some(format!("attachment; filename=\"{name}.srt\"")),
                srt(name),
            ),
            None => (404, "text/plain", None, Vec::new()),
        },
        _ => (404, "text/plain", None, Vec::new()),
    }
}

/// Standard base64, for a body DevTools takes.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(chunk.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Answers the page's requests to [`HOST`] for as long as the run lives (or
/// the page is gone): the page and its file come from here, not a network.
async fn serve(page: Page) -> Result<(), Failure> {
    let session = page.session_id().map_err(|_| browser_trouble())?;
    let run = page.run().clone();
    let mut events = run.events().map_err(|_| browser_trouble())?;
    page.send(
        "Fetch.enable",
        json!({ "patterns": [{ "urlPattern": format!("https://{HOST}/*"), "requestStage": "Request" }] }),
    )
    .await
    .map_err(|_| browser_trouble())?;
    tokio::spawn(async move {
        loop {
            let event = tokio::select! {
                _ = run.ended() => break,
                event = events.recv() => event,
            };
            let event = match event {
                Ok(event) => event,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            if event.session_id.as_deref() != Some(session.as_str()) {
                if event.method == "Target.detachedFromTarget"
                    && event.params["sessionId"].as_str() == Some(session.as_str())
                {
                    break;
                }
                continue;
            }
            if event.method != "Fetch.requestPaused" {
                continue;
            }
            let Some(request_id) = event.params["requestId"].as_str() else {
                continue;
            };
            let (status, media, attachment, body) =
                answer(event.params["request"]["url"].as_str().unwrap_or_default());
            let mut headers = vec![
                json!({ "name": "Content-Type", "value": media }),
                json!({ "name": "Content-Length", "value": body.len().to_string() }),
                json!({ "name": "Cache-Control", "value": "no-store" }),
            ];
            if let Some(disposition) = attachment {
                headers.push(json!({ "name": "Content-Disposition", "value": disposition }));
            }
            let _ = page
                .send(
                    "Fetch.fulfillRequest",
                    json!({
                        "requestId": request_id,
                        "responseCode": status,
                        "responseHeaders": headers,
                        "body": base64(&body),
                    }),
                )
                .await;
        }
    });
    Ok(())
}

fn browser_trouble() -> Failure {
    Failure::new(FailureKind::Network, "서버 브라우저를 쓰지 못했어요")
}

/// Brings the check post at `post` to its check in `page` (a blank page of
/// the job's run): serves the post's pages inside the browser, opens the
/// post, waits for its download card, scrolls it to the middle and clicks it,
/// and waits for the check box. The box itself is left to a person.
pub(crate) async fn drive_check(page: &Page, post: &Url, check: &FakeCheck) -> Result<(), Failure> {
    // The address of the page is the post's own, whatever it was given with.
    let mut address = Url::parse(&format!("https://{HOST}/check/{}", check.name))
        .map_err(|_| Failure::new(FailureKind::Changed, "가짜 출처가 모르는 주소예요"))?;
    address.set_query(post.query());
    serve(page.clone()).await?;
    page.navigate(address.as_str())
        .await
        .map_err(|_| browser_trouble())?;
    let card = "document.querySelector('[data-trss-download]')";
    if !auth::wait_until(
        page,
        &format!("document.readyState === 'complete' && !!{card}"),
        auth::READY_TIMEOUT,
    )
    .await?
    {
        return Err(Failure::new(
            FailureKind::Changed,
            "게시물에서 받기 카드를 찾지 못했어요",
        ));
    }
    if !auth::click_centered(page, card).await? {
        return Err(Failure::new(
            FailureKind::Changed,
            "게시물의 받기 카드가 사라졌어요",
        ));
    }
    let shown = "document.getElementById('check')?.classList.contains('shown') === true";
    if !auth::wait_until(page, shown, Duration::from_secs(10)).await? {
        return Err(Failure::new(
            FailureKind::Changed,
            "받기 카드를 눌렀지만 사람 확인이 나오지 않았어요",
        ));
    }
    Ok(())
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

    #[tokio::test]
    async fn a_check_post_needs_the_browser_and_its_file_comes_only_through_it() {
        let sources = Sources::none().with_fake(FakeSource);
        let post = Url::parse("https://fake.trss.invalid/check/ep1").unwrap();
        let source = sources.for_post(&post).unwrap();
        let opened = source.open(&post, "1").await.unwrap();
        let Opened::BrowserAuth { reason, page } = opened else {
            panic!("{opened:?}");
        };
        assert_eq!(reason, CHECK_REASON);
        assert_eq!(page.reason(), CHECK_REASON);
        let AuthPage::Fake(check) = &page;
        assert_eq!(check.name(), "ep1");

        // Nothing to fetch without the browser.
        let file = PostFile::new("browser:x", "ep1.srt");
        assert_eq!(
            source.fetch(&post, &file).await.err().unwrap().kind,
            FailureKind::Expired
        );
        // What the browser downloaded is read from where it was put.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ep1.srt");
        std::fs::write(&path, srt("ep1")).unwrap();
        let arrived = crate::auth::arrived(&post, "ep1.srt", path.clone());
        let (expected, bytes) = receive(&source, &post, &arrived).await;
        assert_eq!(bytes, srt("ep1"));
        assert_eq!(expected, Some(bytes.len() as u64));
        assert_eq!(
            crate::verify::check(&path, "ep1.srt").unwrap(),
            crate::verify::Format::Srt
        );

        // A name that would not go into the page as it is is not a post.
        for bad in ["/check/", "/check/a%22b", "/check/a.b"] {
            let post = Url::parse(&format!("https://fake.trss.invalid{bad}")).unwrap();
            assert_eq!(
                source.open(&post, "1").await.unwrap_err().kind,
                FailureKind::Changed,
                "{bad}"
            );
        }
    }

    #[test]
    fn the_check_posts_pages_are_answered_inside_the_browser() {
        let (status, media, attachment, body) = answer("https://fake.trss.invalid/check/ep1");
        assert_eq!((status, attachment), (200, None));
        assert!(media.starts_with("text/html"));
        let page = String::from_utf8(body).unwrap();
        assert!(page.contains("data-trss-download") && page.contains("/files/ep1.srt"));
        assert!(page.contains("event.isTrusted"));

        let (status, _, attachment, body) = answer("https://fake.trss.invalid/files/ep1.srt");
        assert_eq!(status, 200);
        assert_eq!(
            attachment.as_deref(),
            Some("attachment; filename=\"ep1.srt\"")
        );
        assert_eq!(body, srt("ep1"));

        for other in [
            "https://fake.trss.invalid/files/x.zip",
            "https://fake.trss.invalid/other",
            "https://example.com/check/ep1",
            "not a url",
        ] {
            assert_eq!(answer(other).0, 404, "{other}");
        }
    }

    #[test]
    fn base64_is_the_standard_one() {
        for (raw, coded) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foobar", "Zm9vYmFy"),
            (b"\xff\xfe\x00", "//4A"),
        ] {
            assert_eq!(base64(raw), coded);
        }
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
