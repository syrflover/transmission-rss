//! Naver blog attachments (`docs/specs/subtitles.md`, 출처별 다운로드;
//! `docs/specs/jobs.md`, 출처별 단계 초안): a post on `blog.naver.com` read
//! over HTTPS with no cookie, and the files attached to it received from the
//! signed addresses it gives, also with no cookie and no `Referer`.
//!
//! - A post's address names its blog and its number (`blog.naver.com/<blog>/
//!   <logNo>`, `PostView.naver?blogId=…&logNo=…`, the same on
//!   `m.blog.naver.com`; [`post_id`]). Its page there is a frame of 2.9KB
//!   whose inner frame is the post itself: this source reads that inner page,
//!   `PostView.naver`, directly ([`post_view`]).
//! - The post's attachments are the list the inner page assigns it
//!   (`aPostFiles[<i>] = JSON.parse('…')`, the `<i>` its `aPostBaseInfo[<i>]`
//!   gives the post's number): each file's name, its exact size
//!   (`attachFileSize`), whether Naver flags it (`maliciousCodeYn`,
//!   `punishType`, read so that only the values that say no are no flag),
//!   and its signed address on [`FILE_HOST`]. The name, the size and the
//!   address are one record there, which the page's save buttons only repeat.
//!   The file's key is the blog, the post and the name; the address holds a
//!   token that changes at every reading.
//! - Which attachments serve the candidate's episode is
//!   [`episode::choose`]'s, by their names: an episode's file, a ZIP of a
//!   range that holds it, the fonts (`.ttf` and the like) always.
//! - A flagged file is never asked for: it is [`FailureKind::Missing`], the
//!   original being out of reach whatever is retried.
//! - A page that assigns no list and no base information is not the post,
//!   however it looks (the bare `var aPostFiles = [];` says nothing): one
//!   that says the post is deleted (`게시물이 삭제되었거나…`) is
//!   [`FailureKind::Missing`]; a security check (CAPTCHA), another alert or
//!   any other page is [`FailureKind::Changed`]. So is a page whose base
//!   information names other posts only. Every real post carries a hidden CAPTCHA frame
//!   for its guestbook, so a CAPTCHA says nothing on a post's page.
//! - A post with no attachment is read for Google Drive links in its body,
//!   as a Tistory post is ([`crate::drive`]).
//! - A file's answer is held to [`Limits`] like Tistory's, and its length to
//!   the size the post gave: an answer that announces another is
//!   [`FailureKind::NotAFile`], and one that announces none is held to it. A
//!   signed address refused with a web page (`400` `text/html`, 3,452 bytes)
//!   has expired: the post is read once more for a new one.
//! - A redirect is followed only to where the request may go itself: a post
//!   to an `https` page on `blog.naver.com` or `m.blog.naver.com`, a file to
//!   [`FILE_HOST`]. Requests to one host are at least [`crate::http::SPACING`]
//!   apart.

use std::sync::Arc;

use scraper::{Html, Selector};
use url::Url;

use crate::{
    drive::{self, Drive},
    episode,
    http::{self, Limits, Pace, Reach},
    Failure, FailureKind, Fetch, FileInfo, Opened, PostFile, Snapshot,
};

/// The hosts of the posts this source reads.
pub fn reads(host: &str) -> bool {
    matches!(host, "blog.naver.com" | "m.blog.naver.com")
}

/// The host of the attachments.
pub const FILE_HOST: &str = "download.blog.naver.com";

/// Where the inner pages are: `<base>PostView.naver?…`.
const BLOG: &str = "https://blog.naver.com/";

/// A post's body, for the Drive links of a post with no attachment: the
/// editor's container (every post seen, 2026-10-03) or the older editor's.
pub const BODY: &str = ".se-main-container, #postViewArea";

/// The snapshot's names.
pub const ATTACH_FILE_SIZE: &str = "attach_file_size";
pub const PUBLISH_DATE: &str = "publish_date";
pub const BLOCKED: &str = "blocked";

/// The blog and the number of the post at `url`, if it is a Naver post's
/// address.
pub fn post_id(url: &Url) -> Option<(String, String)> {
    if !url.host_str().is_some_and(reads) {
        return None;
    }
    let query = |name: &str| {
        url.query_pairs()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_owned())
    };
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let (blog, log_no) = match segments[..] {
        // `/<blog>/<logNo>`
        [blog, log_no] => (blog.to_owned(), log_no.to_owned()),
        // `/PostView.naver?blogId=…&logNo=…`, `/<blog>?Redirect=Log&logNo=…`
        [page] => {
            let blog =
                query("blogId").or_else(|| (!page.contains('.')).then(|| page.to_owned()))?;
            (blog, query("logNo")?)
        }
        _ => return None,
    };
    let blog_ok = (1..=50).contains(&blog.len())
        && blog
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    let log_ok = (1..=20).contains(&log_no.len()) && log_no.bytes().all(|b| b.is_ascii_digit());
    (blog_ok && log_ok).then_some((blog, log_no))
}

/// The inner page of post `log_no` of `blog` under `base`, as the frame
/// points to it.
pub fn post_view(base: &Url, blog: &str, log_no: &str) -> Url {
    let mut url = base.join("PostView.naver").expect("a valid address");
    url.query_pairs_mut()
        .append_pair("blogId", blog)
        .append_pair("logNo", log_no)
        .append_pair("redirect", "Dlog")
        .append_pair("widgetTypeCall", "true")
        .append_pair("noTrackingCode", "true")
        .append_pair("directAccess", "false");
    url
}

fn file_address(reach: Reach, url: &Url) -> bool {
    reach.scheme(url) && url.domain() == Some(FILE_HOST)
}

fn post_address(reach: Reach, url: &Url) -> bool {
    reach.scheme(url) && url.domain().is_some_and(reads)
}

/// Reads Naver posts and their attachments. Cheap to clone; the clones share
/// the client and the pace.
#[derive(Clone)]
pub struct NaverSource {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    limits: Limits,
    reach: Reach,
    pace: Pace,
    drive: Drive,
    base: Url,
}

impl std::fmt::Debug for NaverSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NaverSource")
    }
}

impl NaverSource {
    /// The source over the network, receiving the Drive files its posts link
    /// through `drive`, which the other sources share.
    pub fn new(drive: Drive) -> NaverSource {
        NaverSource::over(
            reqwest::Client::builder(),
            Limits::default(),
            Reach::NETWORK,
            drive,
            Url::parse(BLOG).expect("a valid address"),
        )
    }

    /// The source with its own client, limits, reach, Drive and blog address
    /// (the tests' [`crate::testing`] server).
    pub(crate) fn over(
        builder: reqwest::ClientBuilder,
        limits: Limits,
        reach: Reach,
        drive: Drive,
        base: Url,
    ) -> NaverSource {
        let follow = move |first: &Url, next: &Url| match file_address(reach, first) {
            true => file_address(reach, next),
            false => post_address(reach, next),
        };
        NaverSource {
            inner: Arc::new(Inner {
                http: http::client(builder, follow),
                limits,
                reach,
                pace: Pace::new(limits.spacing),
                drive,
                base,
            }),
        }
    }

    /// The inner page of the post at `post`.
    async fn page(&self, post: &Url) -> Result<(String, String, String), Failure> {
        let Some((blog, log_no)) = post_id(post) else {
            return Err(Failure::new(
                FailureKind::Changed,
                "Naver 블로그 글의 주소가 아니에요",
            ));
        };
        let view = post_view(&self.inner.base, &blog, &log_no);
        let page = http::get_page(&self.inner.http, &self.inner.pace, &view).await?;
        Ok((blog, log_no, page))
    }

    pub(crate) async fn open(&self, post: &Url, episode: &str) -> Result<Opened, Failure> {
        let (blog, log_no, page) = self.page(post).await?;
        read_page(post, &blog, &log_no, &page, self.inner.reach, episode)
    }

    /// The files `keys` name, read again (see [`crate::Source::recheck`]).
    /// The post's inner page lists every attachment with its exact size
    /// (`attachFileSize`), so one reading of it answers for all of them and no
    /// request goes to the file host: the size is the file's value. Naver
    /// gives no modified time. A Drive file in the body is a `HEAD` of its
    /// fixed address, with no post read.
    pub(crate) async fn recheck(
        &self,
        post: &Url,
        keys: &[String],
    ) -> Vec<(String, Result<FileInfo, Failure>)> {
        let mut answers = Vec::new();
        let mut offered: Option<Result<Vec<PostFile>, Failure>> = None;
        for key in keys {
            let info = if let Some(id) = drive::id_of(key) {
                self.inner.drive.head(id).await
            } else {
                if offered.is_none() {
                    offered = Some(match self.page(post).await {
                        Ok((blog, log_no, page)) => {
                            attachments(&blog, &log_no, &page, self.inner.reach)
                        }
                        Err(failure) => Err(failure),
                    });
                }
                match offered.as_ref().expect("read above") {
                    Err(failure) => Err(failure.clone()),
                    Ok(files) => match files.iter().find(|f| f.key == *key) {
                        None => Err(Failure::new(
                            FailureKind::Missing,
                            "게시물을 다시 읽었지만 이 첨부가 없어요",
                        )),
                        Some(file) => match snapshot_value(&file.snapshot, BLOCKED) {
                            Some(blocked) => Err(blocked_failure(blocked)),
                            None => {
                                match snapshot_value(&file.snapshot, ATTACH_FILE_SIZE)
                                    .and_then(|s| s.parse().ok())
                                {
                                    Some(size) => Ok(FileInfo {
                                        size: Some(size),
                                        last_modified: None,
                                    }),
                                    None => Err(Failure::new(
                                        FailureKind::Changed,
                                        "게시물이 첨부의 크기(attachFileSize)를 알려주지 않았어요",
                                    )),
                                }
                            }
                        },
                    },
                }
            };
            answers.push((key.clone(), info));
        }
        answers
    }

    pub(crate) async fn fetch(&self, post: &Url, file: &PostFile) -> Result<Fetch, Failure> {
        if let Some(id) = drive::id_of(&file.key) {
            return self.inner.drive.fetch(id).await;
        }
        if let Some(blocked) = snapshot_value(&file.snapshot, BLOCKED) {
            return Err(blocked_failure(blocked));
        }
        let first = match file.locator() {
            Some(locator) => self.get_file(locator, file).await,
            None => Err(Failure::new(FailureKind::Expired, "받을 주소가 없어요")),
        };
        let expired = match first {
            Err(failure) if failure.kind == FailureKind::Expired => failure,
            other => return other,
        };
        // Once more, with the address the post gives now.
        let again = match self.page(post).await {
            Ok((blog, log_no, page)) => attachments(&blog, &log_no, &page, self.inner.reach)?
                .into_iter()
                .find(|f| f.key == file.key),
            Err(failure) if failure.kind == FailureKind::Network => return Err(failure),
            Err(_) => None,
        };
        let gone = || {
            Failure::new(
                FailureKind::Missing,
                "주소가 거절돼 게시물을 다시 읽었지만 이 파일이 없어요",
            )
            .with_response(expired.status, expired.content_type.clone(), expired.size)
        };
        let Some(again) = again else {
            return Err(gone());
        };
        if let Some(blocked) = snapshot_value(&again.snapshot, BLOCKED) {
            return Err(blocked_failure(blocked));
        }
        let Some(locator) = again.locator() else {
            return Err(Failure::new(
                FailureKind::Changed,
                "게시물이 준 첨부 주소가 Naver 첨부 호스트가 아니에요",
            ));
        };
        match self.get_file(locator, &again).await {
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

    /// Receives `file` from `locator`, held to the size the post gave.
    async fn get_file(&self, locator: &Url, file: &PostFile) -> Result<Fetch, Failure> {
        let mut fetch = http::get_file(
            &self.inner.http,
            &self.inner.pace,
            self.inner.limits,
            locator,
        )
        .await?;
        let declared: Option<u64> =
            snapshot_value(&file.snapshot, ATTACH_FILE_SIZE).and_then(|s| s.parse().ok());
        match (declared, fetch.expected_size) {
            (Some(declared), Some(announced)) if declared != announced => Err(Failure::new(
                FailureKind::NotAFile,
                format!(
                    "게시물이 알린 크기({declared}바이트)와 응답이 알린 길이({announced}바이트)가 달라요"
                ),
            )
            .with_response(fetch.status, fetch.content_type.clone(), Some(announced))),
            (Some(declared), None) => {
                fetch.expected_size = Some(declared);
                Ok(fetch)
            }
            _ => Ok(fetch),
        }
    }
}

fn snapshot_value<'a>(snapshot: &'a Snapshot, name: &str) -> Option<&'a str> {
    snapshot
        .entries()
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

fn blocked_failure(blocked: &str) -> Failure {
    let reason = match blocked {
        "malicious_code" => "Naver가 이 첨부를 악성 코드가 든 파일로 표시해서 받지 않아요",
        _ => "Naver가 이 첨부를 작성자 말고는 받을 수 없게 제한했어요",
    };
    Failure::new(FailureKind::Missing, reason)
}

/// What the inner page of a post offers for `episode` (see the module docs).
pub(crate) fn read_page(
    post: &Url,
    blog: &str,
    log_no: &str,
    page: &str,
    reach: Reach,
    episode: &str,
) -> Result<Opened, Failure> {
    let files = attachments(blog, log_no, page, reach)?;
    if !files.is_empty() {
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        return match episode::choose(episode, &names) {
            Ok(chosen) => Ok(Opened::Files(
                chosen.into_iter().map(|i| files[i].clone()).collect(),
            )),
            Err(reason) => Err(Failure::new(FailureKind::Changed, reason)),
        };
    }
    let html = Html::parse_document(page);
    let bodies: Vec<_> = html
        .select(&Selector::parse(BODY).expect("a valid selector"))
        .collect();
    let mut snapshot = Snapshot::default();
    if let Some(published) = publish_date(page, log_no) {
        snapshot.push(PUBLISH_DATE, published);
    }
    let (drive_files, folder) = drive::links_in(&bodies, post);
    drive::offered(&drive_files, folder, episode, &snapshot).unwrap_or_else(|| {
        Err(Failure::new(
            FailureKind::Changed,
            "게시물에서 첨부 파일을 찾지 못했어요",
        ))
    })
}

/// The files attached to post `log_no` of `blog`, as its inner page lists
/// them; none for a post with no attachment. A page that is not the post's
/// is a failure (see the module docs).
fn attachments(
    blog: &str,
    log_no: &str,
    page: &str,
    reach: Reach,
) -> Result<Vec<PostFile>, Failure> {
    let lists = file_lists(page);
    let posts = assignments(page, "aPostBaseInfo");
    // A post's page assigns its lists and its base information; the bare
    // declaration (`var aPostFiles = [];`) may be in other pages as well.
    if lists.is_empty() && posts.is_empty() {
        return Err(not_a_post(page));
    }
    let unreadable = || {
        Failure::new(
            FailureKind::Changed,
            "게시물의 첨부 파일 목록(aPostFiles)을 읽지 못했어요",
        )
    };
    // The lists of this post: those of the index its base information has,
    // or the only list of a page that names no post.
    let index = match (posts.is_empty(), lists.len()) {
        (false, _) => Some(
            posts
                .iter()
                .find(|(_, value)| post_of(value) == Some(log_no))
                .map(|(i, _)| *i)
                .ok_or_else(|| {
                    Failure::new(
                        FailureKind::Changed,
                        format!("Naver가 준 페이지에 이 글({log_no})이 없어요"),
                    )
                })?,
        ),
        (true, 0 | 1) => None,
        (true, n) => {
            return Err(Failure::new(
                FailureKind::Changed,
                format!("페이지에 첨부 파일 목록이 {n}개 있는데 어느 글의 것인지 알 수 없어요"),
            ))
        }
    };
    let published = publish_date(page, log_no);
    let mut files: Vec<PostFile> = Vec::new();
    for (i, list) in lists {
        if index.is_some_and(|index| index != i) {
            continue;
        }
        let list = list.ok_or_else(unreadable)?;
        let records: Vec<serde_json::Map<String, serde_json::Value>> =
            serde_json::from_str(&list).map_err(|_| unreadable())?;
        for record in records {
            let text = |name: &str| {
                record
                    .get(name)
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
            };
            let Some(name) = text("encodedAttachFileName") else {
                continue;
            };
            let key = format!("naver:{blog}/{log_no}/{name}");
            if files.iter().any(|f| f.key == key) {
                continue;
            }
            let mut file = PostFile::new(key, name);
            if let Some(published) = &published {
                file.snapshot.push(PUBLISH_DATE, published.clone());
            }
            let size = text("attachFileSize").map(|s| s.replace(',', ""));
            if let Some(size) = size.filter(|s| s.parse::<u64>().is_ok()) {
                file.snapshot.push(ATTACH_FILE_SIZE, size);
            }
            // Both flags fail closed: any value but the ones that say no.
            let malicious = text("maliciousCodeYn").is_some_and(|v| {
                !["false", "n", "0"]
                    .iter()
                    .any(|no| v.eq_ignore_ascii_case(no))
            });
            let punished = text("punishType").filter(|v| *v != "0");
            let blocked = match (malicious, punished) {
                (true, _) => Some("malicious_code".to_owned()),
                (false, Some(kind)) => {
                    // The page's text, kept short and plain.
                    let kind: String = kind
                        .chars()
                        .filter(char::is_ascii_alphanumeric)
                        .take(16)
                        .collect();
                    let kind = match kind.is_empty() {
                        true => "unknown".to_owned(),
                        false => kind,
                    };
                    Some(format!("punish_type={kind}"))
                }
                (false, None) => None,
            };
            match blocked {
                Some(blocked) => file.snapshot.push(BLOCKED, blocked),
                None => {
                    if let Some(url) = text("encodedAttachFileUrl")
                        .and_then(|u| Url::parse(u).ok())
                        .filter(|u| file_address(reach, u))
                    {
                        file = file.with_locator(url);
                    }
                }
            }
            files.push(file);
        }
    }
    Ok(files)
}

/// Why a page that is no post's is not the post.
fn not_a_post(page: &str) -> Failure {
    // The page Naver gives for a post deleted: `var msg = '…'; alert(msg)`.
    // Only the wording seen for that is a post gone; another alert says
    // something this source does not know.
    if let Some(message) = alert_message(page) {
        let kind = match message.contains("삭제되었거나") {
            true => FailureKind::Missing,
            false => FailureKind::Changed,
        };
        return Failure::new(
            kind,
            format!("Naver가 게시물 대신 알림을 줬어요: {message}"),
        );
    }
    let lower = page.to_lowercase();
    if ["captcha", "자동입력 방지", "자동 입력 방지"]
        .iter()
        .any(|w| lower.contains(w))
    {
        return Failure::new(
            FailureKind::Changed,
            "Naver가 게시물 대신 보안 확인(CAPTCHA) 페이지를 줬어요",
        );
    }
    Failure::new(
        FailureKind::Changed,
        "Naver가 게시물 대신 다른 페이지를 줬어요",
    )
}

/// The message of `var msg = '…';`, when it has one.
fn alert_message(page: &str) -> Option<String> {
    let at = page.find("var msg")?;
    let rest = page[at + "var msg".len()..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('\'')?;
    let message = js_string(rest)?.0;
    let message = message.split_whitespace().collect::<Vec<_>>().join(" ");
    (!message.is_empty()).then(|| message.chars().take(100).collect())
}

/// Every assignment `aPostFiles[<i>] = …` of a page: the index, and the
/// list's text when it is `JSON.parse('<list>'…)` (or `[]`), `None` when it
/// is something else. Any other mention (`aPostFiles[0].length`, a post's
/// own words) is not one.
fn file_lists(page: &str) -> Vec<(u32, Option<String>)> {
    assignments(page, "aPostFiles")
        .into_iter()
        .map(|(index, value)| {
            let list = match value.strip_prefix("JSON.parse(") {
                Some(rest) => rest
                    .trim_start()
                    .strip_prefix('\'')
                    .and_then(js_string)
                    // As the page does: `.replace(/\\'/g, '')`.
                    .map(|(list, _)| list.replace("\\'", "")),
                None => value.starts_with("[]").then(|| "[]".to_owned()),
            };
            (index, list)
        })
        .collect()
}

/// Every assignment `<name>[<i>] = <value>` of a page (not a comparison,
/// `==`), with the text from its value on.
fn assignments<'a>(page: &'a str, name: &str) -> Vec<(u32, &'a str)> {
    let opening = format!("{name}[");
    page.match_indices(&opening)
        .filter_map(|(at, found)| {
            // Not the end of a longer name.
            let before = page[..at].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$') {
                return None;
            }
            let rest = &page[at + found.len()..];
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            let index = rest[..digits].parse().ok()?;
            let rest = rest[digits..].strip_prefix(']')?.trim_start();
            let rest = rest.strip_prefix('=')?;
            (!rest.starts_with('=')).then(|| (index, rest.trim_start()))
        })
        .collect()
}

/// The post number a base information value (`"<logNo>|…"`) starts with.
fn post_of(value: &str) -> Option<&str> {
    let value = value.strip_prefix('"')?;
    let number = &value[..value.find('|')?];
    (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())).then_some(number)
}

/// Post `log_no`'s date as the page shows it (`2026. 6. 23. 5:19`), when it
/// is a date: a post of the last day shows how long ago (`3시간 전`) instead,
/// and has none. The date is the one inside the post's own container
/// (`id="post-view<logNo>"`), or the only one of a page with no container.
fn publish_date(page: &str, log_no: &str) -> Option<String> {
    let own = format!("id=\"post-view{log_no}\"");
    let scope = match page.find(&own) {
        Some(at) => {
            let rest = &page[at + own.len()..];
            &rest[..rest.find("id=\"post-view").unwrap_or(rest.len())]
        }
        None if page.contains("id=\"post-view") => return None,
        None if page.matches("se_publishDate").count() == 1 => page,
        None => return None,
    };
    let at = scope.find("se_publishDate")?;
    let rest = &scope[at..];
    let text = &rest[rest.find('>')? + 1..];
    let text = text[..text.find('<')?].trim();
    let parts: Vec<&str> = text
        .split(['.', ' ', ':'])
        .filter(|p| !p.is_empty())
        .collect();
    let date = parts.len() == 5
        && parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
        && parts[0].len() == 4;
    date.then(|| text.to_owned())
}

/// The JavaScript string in single quotes that `text` starts within (just
/// after its opening quote), unescaped, and the rest after its closing quote.
fn js_string(text: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut units: Vec<u16> = Vec::new();
    let mut chars = text.char_indices();
    let flush = |units: &mut Vec<u16>, out: &mut String| {
        out.extend(char::decode_utf16(units.drain(..)).map(|c| c.unwrap_or('\u{FFFD}')));
    };
    while let Some((at, c)) = chars.next() {
        let escaped = match c {
            '\'' => {
                flush(&mut units, &mut out);
                return Some((out, &text[at + 1..]));
            }
            '\n' => return None,
            '\\' => chars.next()?.1,
            c => {
                flush(&mut units, &mut out);
                out.push(c);
                continue;
            }
        };
        let hex = |chars: &mut std::str::CharIndices<'_>, n: usize| {
            let digits: String = chars.take(n).map(|(_, c)| c).collect();
            (digits.len() == n)
                .then(|| u32::from_str_radix(&digits, 16).ok())
                .flatten()
        };
        match escaped {
            'u' => units.push(hex(&mut chars, 4)? as u16),
            'x' => {
                flush(&mut units, &mut out);
                out.push(char::from_u32(hex(&mut chars, 2)?)?);
            }
            other => {
                flush(&mut units, &mut out);
                match other {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'v' => out.push('\u{b}'),
                    '0' => out.push('\0'),
                    '\n' => {}
                    other => out.push(other),
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn a_posts_address_names_its_blog_and_number_in_every_form() {
        let id = |text: &str| post_id(&url(text));
        let elaina = Some(("elainalove1017".to_owned(), "224324105274".to_owned()));
        for form in [
            "https://blog.naver.com/elainalove1017/224324105274",
            "https://m.blog.naver.com/elainalove1017/224324105274",
            "http://blog.naver.com/elainalove1017/224324105274?fromRss=true",
            "https://blog.naver.com/PostView.naver?blogId=elainalove1017&logNo=224324105274&redirect=Dlog&widgetTypeCall=true&noTrackingCode=true&directAccess=false",
            "https://blog.naver.com/PostView.nhn?blogId=elainalove1017&logNo=224324105274",
            "https://m.blog.naver.com/PostView.naver?blogId=elainalove1017&logNo=224324105274",
            "https://blog.naver.com/elainalove1017?Redirect=Log&logNo=224324105274",
        ] {
            assert_eq!(id(form), elaina, "{form}");
        }
        for not_a_post in [
            "https://blog.naver.com/elainalove1017",
            "https://blog.naver.com/PostList.naver?blogId=elainalove1017",
            "https://blog.naver.com/elainalove1017/224324105274/x",
            "https://blog.naver.com/elainalove1017/22432410527x",
            "https://blog.naver.com/elaina%20love/224324105274",
            "https://cafe.naver.com/elainalove1017/224324105274",
            "https://blog.naver.com.example/elainalove1017/224324105274",
        ] {
            assert_eq!(id(not_a_post), None, "{not_a_post}");
        }
        assert_eq!(
            post_view(&url(BLOG), "elainalove1017", "224324105274").as_str(),
            "https://blog.naver.com/PostView.naver?blogId=elainalove1017&logNo=224324105274&redirect=Dlog&widgetTypeCall=true&noTrackingCode=true&directAccess=false"
        );
    }

    #[test]
    fn a_javascript_string_is_read_as_the_page_reads_it() {
        assert_eq!(
            js_string(r#"a\'b\\cA😀\x41\/d' rest"#),
            Some(("a'b\\cA\u{1F600}A/d".to_owned(), " rest"))
        );
        assert_eq!(js_string("no end"), None);
        assert_eq!(js_string("line\nbreak'"), None);
    }

    /// One record of `aPostFiles`, as the page writes it.
    fn record(name: &str, path: &str, size: &str, malicious: &str, punish: &str) -> String {
        format!(
            r#"{{"encodedAttachFileName": "{name}","encodedAttachFileNameByTruncate": "{name}","encodedAttachFileUrl": "https://download.blog.naver.com/open/AAAA/TOKEN/{path}","encodedAttachFileUrlByMS949": "https://download.blog.naver.com/open/AAAA/TOKEN/{path}","licenseyn": "T","maliciousCodeYn": "{malicious}","punishType": "{punish}","attachFileSize": "{size}","encodedAttachFileNameByUTF8": "{path}","ahfLicenseYn" : "false"}}"#
        )
    }

    /// An inner page as Naver writes it (2026-10-03), with its guestbook's
    /// CAPTCHA frame.
    fn page(records: &[String], body: &str) -> String {
        format!(
            r#"<html><body><div id="postListBody"><div class="se-main-container">{body}</div>
<span class="se_publishDate pcol2">2026. 6. 23. 5:19</span>
<div class="frame_wrap"><iframe id="captchalayeredframe" src="about:blank"></iframe></div></div>
<script>
var aPostFiles = [];
	gdidTag[1] = "90000003_000000000000003440B28842";
		aPostFiles[1] = JSON.parse('[{}]'.replace(/\\'/g, ''));
	aPostBaseInfo[1] = "224324105274|0|1|1|339|0|false|4|MYLOG";
</script></body></html>"#,
            records.join(" , ")
        )
    }

    fn elaina() -> Vec<String> {
        vec![
            record(
                "네죽사 1~8화 자막.zip",
                "%EB%84%A4%EC%A3%BD%EC%82%AC%201%7E8%ED%99%94%20%EC%9E%90%EB%A7%89.zip",
                "106,408",
                "false",
                "0",
            ),
            record(
                "[SubsPlease] Kimi ga Shinu made Koi wo Shitai - 08 (1080p) [F3B053C5].ass",
                "%5BSubsPlease%5D%20Kimi%20ga%20Shinu%20made%20Koi%20wo%20Shitai%20-%2008%20%281080p%29%20%5BF3B053C5%5D.ass",
                "44,480",
                "false",
                "0",
            ),
        ]
    }

    fn read(page: &str, episode: &str) -> Result<Opened, Failure> {
        let post = url("https://blog.naver.com/elainalove1017/224324105274");
        read_page(
            &post,
            "elainalove1017",
            "224324105274",
            page,
            Reach::NETWORK,
            episode,
        )
    }

    #[test]
    fn the_attachments_give_names_exact_sizes_and_a_key_without_the_token() {
        let Ok(Opened::Files(files)) = read(&page(&elaina(), ""), "8") else {
            panic!("files");
        };
        let keys: Vec<&str> = files.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "naver:elainalove1017/224324105274/네죽사 1~8화 자막.zip",
                "naver:elainalove1017/224324105274/[SubsPlease] Kimi ga Shinu made Koi wo Shitai - 08 (1080p) [F3B053C5].ass",
            ]
        );
        assert_eq!(
            files[1].snapshot.entries(),
            [
                (PUBLISH_DATE.to_owned(), "2026. 6. 23. 5:19".to_owned()),
                (ATTACH_FILE_SIZE.to_owned(), "44480".to_owned()),
            ]
        );
        let locator = files[1].locator().unwrap();
        assert_eq!(locator.host_str(), Some(FILE_HOST));
        assert!(locator.path().contains("/TOKEN/"));
        assert!(!format!("{files:?}").contains("TOKEN"));
        // Another episode: neither the ZIP of 1~8 nor 08.
        let failure = read(&page(&elaina(), ""), "9").unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert!(failure.reason.contains("9화"));
    }

    #[test]
    fn the_fonts_come_with_the_episodes_file() {
        // 수퍼소닉EX 224428424028: three fonts and the episode's SMI, named
        // with its season and episode.
        let records: Vec<String> = [
            ("H2MPRB.TTF", "1,432,836"),
            ("H2MKPB.TTF", "2,098,368"),
            ("a옛날목욕탕L.ttf", "2,166,920"),
            ("Reincarnated.as.a.Sword.S02E01.The.Island.Floating.in.the.Sky.1080p.BILI.WEB-DL.JPN.AAC2.0.H.265.MSubs-ToonsHub.smi", "76,408"),
        ]
        .iter()
        .map(|(name, size)| record(name, "x", size, "false", "0"))
        .collect();
        let Ok(Opened::Files(files)) = read(&page(&records, ""), "1") else {
            panic!("files");
        };
        assert_eq!(files.len(), 4);
        let failure = read(&page(&records, ""), "2").unwrap_err();
        assert!(failure.reason.contains("2화"), "{}", failure.reason);
    }

    #[test]
    fn the_real_posts_sampled_choose_their_files() {
        // The attachments of the dev candidates on 2026-10-03, and the
        // episode each is for.
        for (episode, names, chosen) in [
            (
                "8",
                vec![
                    "네죽사 1~8화 자막.zip",
                    "[SubsPlease] Kimi ga Shinu made Koi wo Shitai - 08 (1080p) [F3B053C5].ass",
                ],
                vec![0, 1],
            ),
            (
                "12",
                vec![
                    "[ak-Submarines] Girls und Panzer - MLLSD - 12 [WEB 1080p][09A3B4BB].ass",
                    "1~12.zip",
                ],
                vec![0, 1],
            ),
            (
                "20",
                vec!["[EruPii-Raws] Rockman.EXE Beast+ - 20 [DVD 768x576 x264-10bit AC3][1DFE24E7].smi"],
                vec![0],
            ),
            ("35", vec!["명탐정 프리큐어 35화.ass"], vec![0]),
            ("49", vec!["디지몬 비트브레이크 통합자막.zip"], vec![0]),
            (
                "35",
                vec!["[SubsPlease] Meitantei Precure! - 35 (720p) [A70A665B].smi"],
                vec![0],
            ),
            (
                "1",
                vec![
                    "Reincarnated.as.a.Sword.S02E01.The.Island.Floating.in.the.Sky.1080p.BILI.WEB-DL.JPN.AAC2.0.H.265.MSubs-ToonsHub.smi",
                    "H2MPRB.TTF",
                    "H2MKPB.TTF",
                    "a옛날목욕탕L.ttf",
                ],
                vec![0, 1, 2, 3],
            ),
        ] {
            assert_eq!(episode::choose(episode, &names), Ok(chosen), "{names:?}");
        }
    }

    #[test]
    fn a_flagged_file_is_offered_without_an_address_and_never_fetched() {
        let records = vec![
            record("Title 08.ass", "a.ass", "1,000", "true", "0"),
            record("Title 08.smi", "a.smi", "1,000", "false", "2"),
        ];
        let Ok(Opened::Files(files)) = read(&page(&records, ""), "8") else {
            panic!("files");
        };
        assert!(files.iter().all(|f| f.locator().is_none()));
        assert_eq!(
            snapshot_value(&files[0].snapshot, BLOCKED),
            Some("malicious_code")
        );
        assert_eq!(
            snapshot_value(&files[1].snapshot, BLOCKED),
            Some("punish_type=2")
        );
        assert!(blocked_failure("malicious_code")
            .reason
            .contains("악성 코드"));
        assert_eq!(blocked_failure("punish_type=2").kind, FailureKind::Missing);
    }

    #[test]
    fn an_address_off_the_file_host_is_not_kept() {
        let records = vec![record("Title 08.ass", "a.ass", "1,000", "false", "0")
            .replace("download.blog.naver.com", "download.example.com")];
        let Ok(Opened::Files(files)) = read(&page(&records, ""), "8") else {
            panic!("files");
        };
        assert!(files[0].locator().is_none());
    }

    #[test]
    fn a_page_that_is_not_the_post_is_a_classified_failure() {
        let deleted = "<html><body><script>var msg = '게시물이 삭제되었거나 다른 페이지로 변경되었습니다.';\nif( msg != '' ) alert(msg);</script></body></html>";
        let failure = read(deleted, "8").unwrap_err();
        assert_eq!(failure.kind, FailureKind::Missing);
        assert!(failure.reason.contains("삭제"));
        let captcha = r#"<html><body><form><img id="captchaimg" src="x"><p>자동입력 방지 문자를 입력해 주세요</p></form></body></html>"#;
        let failure = read(captcha, "8").unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert!(failure.reason.contains("CAPTCHA"));
        let other = "<html><body><h1>블로그 :: 네이버</h1></body></html>";
        let failure = read(other, "8").unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert!(failure.reason.contains("다른 페이지"));
        // The bare declaration does not make a page the post's.
        let declared = format!("<script>var aPostFiles = [];</script>{captcha}");
        let failure = read(&declared, "8").unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert!(failure.reason.contains("CAPTCHA"));
        // Another alert: not a post gone, and said in at most 100 letters.
        let long = "가".repeat(300);
        let alert = format!("<script>var msg = '잠시 후 다시 이용해 주세요 {long}';</script>");
        let failure = read(&alert, "8").unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert!(failure.reason.contains("잠시 후 다시 이용해 주세요"));
        assert!(failure.reason.chars().filter(|c| *c == '가').count() < 100);
        // A list it cannot read.
        let broken = page(&elaina(), "").replace("JSON.parse('[", "JSON.parse(\"[");
        assert_eq!(read(&broken, "8").unwrap_err().kind, FailureKind::Changed);
    }

    #[test]
    fn a_post_with_no_attachment_offers_its_drive_files_or_has_changed() {
        let drive = page(
            &[],
            r#"<p><a href="https://drive.google.com/file/d/1AbCdEfGhIjKlMn/view" class="se-link">8화 자막</a></p>"#,
        );
        let Ok(Opened::Files(files)) = read(&drive, "8") else {
            panic!("files");
        };
        assert_eq!(files[0].key, "drive:1AbCdEfGhIjKlMn");
        let empty = page(&[], "<p>자막은 내일</p>");
        let failure = read(&empty, "8").unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
    }

    #[test]
    fn only_the_posts_own_list_is_read() {
        let mut two = page(&elaina(), "");
        two = two.replace(
            "</script>",
            &format!(
                "aPostFiles[2] = JSON.parse('[{}]'.replace(/\\\\'/g, ''));\naPostBaseInfo[2] = \"111|0\";</script>",
                record("Other 08.ass", "o.ass", "10", "false", "0")
            ),
        );
        let Ok(Opened::Files(files)) = read(&two, "8") else {
            panic!("files");
        };
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|f| !f.name.starts_with("Other")));
    }

    #[test]
    fn only_the_posts_own_absolute_date_is_its_publish_date() {
        let date = |text: &str| format!(r#"<span class="se_publishDate pcol2">{text}</span>"#);
        assert_eq!(
            publish_date(&date("2026. 10. 1. 16:48"), "1"),
            Some("2026. 10. 1. 16:48".to_owned())
        );
        // Under a day old: how long ago, which is no date.
        assert_eq!(publish_date(&date("3시간 전"), "1"), None);
        // Two posts' containers: the date of this post's only.
        let two = format!(
            r#"<div id="post-view111">{}</div><div id="post-view222">{}</div>"#,
            date("2026. 9. 1. 1:00"),
            date("5분 전")
        );
        assert_eq!(
            publish_date(&two, "111"),
            Some("2026. 9. 1. 1:00".to_owned())
        );
        assert_eq!(publish_date(&two, "222"), None);
        assert_eq!(publish_date(&two, "333"), None);
        // No container and two dates: neither is surely this post's.
        let loose = format!("{}{}", date("2026. 9. 1. 1:00"), date("2026. 9. 2. 1:00"));
        assert_eq!(publish_date(&loose, "1"), None);
    }

    #[test]
    fn other_mentions_of_the_list_are_not_its_assignment() {
        // A script reading the list, and a post's words naming it.
        let page = page(&elaina(), "<p>aPostFiles[1] 같은 글자도 본문에 있어요</p>").replace(
            "</script>",
            "if (aPostFiles[0].length == 0) {}\nif (aPostFiles[1] == null) {}\n</script>",
        );
        let Ok(Opened::Files(files)) = read(&page, "8") else {
            panic!("files");
        };
        assert_eq!(files.len(), 2);
        // An assignment of something else than a list it can read.
        let other = page.replace(
            "aPostFiles[1] = JSON.parse(",
            "aPostFiles[1] = loadFiles(JSON.parse(",
        );
        let failure = read(&other, "8").unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert!(failure.reason.contains("aPostFiles"));
        // `[]` is a list of nothing.
        assert_eq!(
            file_lists("aPostFiles[3] = [];"),
            [(3, Some("[]".to_owned()))]
        );
    }

    #[test]
    fn a_page_of_another_post_or_of_lists_of_no_post_has_changed() {
        let other = page(&elaina(), "").replace("\"224324105274|", "\"999|");
        let failure = read(&other, "8").unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert!(failure.reason.contains("224324105274"));
        // No base information: one list is the post's, two are no one's.
        let unnamed = page(&elaina(), "").replace("aPostBaseInfo[1]", "somethingElse[1]");
        assert!(matches!(read(&unnamed, "8"), Ok(Opened::Files(_))));
        let two = unnamed.replace(
            "</script>",
            &format!(
                "aPostFiles[2] = JSON.parse('[{}]');</script>",
                record("Other 08.ass", "o.ass", "10", "false", "0")
            ),
        );
        let failure = read(&two, "8").unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert!(failure.reason.contains("2개"));
    }

    #[test]
    fn the_flags_fail_closed_and_are_kept_short() {
        let blocked = |malicious: &str, punish: &str| {
            let records = vec![record("Title 08.ass", "a.ass", "1", malicious, punish)];
            let Ok(Opened::Files(files)) = read(&page(&records, ""), "8") else {
                panic!("files");
            };
            snapshot_value(&files[0].snapshot, BLOCKED).map(str::to_owned)
        };
        for no in ["false", "FALSE", "n", "N", "0", ""] {
            assert_eq!(blocked(no, "0"), None, "{no}");
        }
        for yes in ["true", "Y", "1", "unknown"] {
            assert_eq!(
                blocked(yes, "0").as_deref(),
                Some("malicious_code"),
                "{yes}"
            );
        }
        assert_eq!(blocked("false", "").as_deref(), None);
        assert_eq!(
            blocked("false", "<b>abc 12345678901234567890").as_deref(),
            Some("punish_type=babc123456789012")
        );
        assert_eq!(
            blocked("false", "!!").as_deref(),
            Some("punish_type=unknown")
        );
    }
}
