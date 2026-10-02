//! Google Drive files linked from a post (`docs/specs/jobs.md`, 공통 수신
//! 결과와 실패 분류), for every source whose posts link them: Blogger's, and
//! Tistory's and Naver's when the subtitle is in the body rather than
//! attached.
//!
//! - A link to a file (`drive.google.com/file/d/<id>/…`, `open?id=<id>`,
//!   `uc?id=<id>`, each also under `/u/<n>/` or `/a/<domain>/`) names it by its ID. The ID is public and stays the same, so
//!   the file's key is `drive:<id>` and there is no address to read again. A
//!   link to a folder (`drive/folders/<id>`) is no file: a post that links
//!   only folders has its subtitle elsewhere ([`Opened::Elsewhere`]).
//! - Which linked files serve the candidate's episode is
//!   [`episode::choose`]'s, by the words of each link.
//! - A file is received from `drive.usercontent.google.com/download` with no
//!   cookie and no `Referer`, following a redirect only within
//!   [`HOSTS`] (`drive.google.com/uc` answers `303` there), under the
//!   sources' byte limit and deadline. The name is the one the answer gives
//!   (`Content-Disposition`); the post gives none, only the link's words.
//! - Drive gives the file as `application/octet-stream`. Any web page instead
//!   is never the file: `404` is a file gone ([`FailureKind::Missing`], 1,652
//!   bytes of `text/html` on 2026-10-03), as is a page that asks to sign in
//!   (the file is not shared); a page that asks to confirm the download (a
//!   file too large to scan) or says the quota is spent is
//!   [`FailureKind::NotAFile`]. No confirmation is passed.

use std::sync::Arc;

use reqwest::{header, StatusCode};
use scraper::{ElementRef, Selector};
use url::Url;

use crate::{
    episode,
    http::{self, Limits, Pace, Reach},
    Failure, FailureKind, Fetch, Opened, PostFile, Snapshot,
};

/// The hosts a Drive file is received from, and the only ones its requests
/// are redirected to.
pub const HOSTS: [&str; 2] = ["drive.google.com", "drive.usercontent.google.com"];

/// Where a file is asked for: `<base>download?id=<id>&export=download`.
const DOWNLOAD: &str = "https://drive.usercontent.google.com/";

/// The snapshot's name for the length the answer announced.
pub const CONTENT_LENGTH: &str = "content_length";

/// What a Drive link points to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    File(String),
    Folder,
}

/// What `url` points to on Drive, if it is a Drive link at all.
pub fn link(url: &Url) -> Option<Link> {
    if !matches!(url.scheme(), "https" | "http") || !url.host_str().is_some_and(drive_host) {
        return None;
    }
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let query_id = || {
        url.query_pairs()
            .find(|(k, _)| k == "id")
            .map(|(_, v)| v.into_owned())
    };
    // An account's (`/u/0/…`) or a Workspace domain's (`/a/<domain>/…`)
    // address of the same file.
    let segments = match segments[..] {
        ["u", n, ref rest @ ..] if n.bytes().all(|b| b.is_ascii_digit()) => rest,
        ["a", _, ref rest @ ..] => rest,
        ref all => all,
    };
    let id = match *segments {
        ["file", "d", id, ..] | ["file", "u", _, "d", id, ..] => Some(id.to_owned()),
        ["open"] | ["uc"] | ["download"] => query_id(),
        ["drive", .., "folders", _] | ["folderview"] => return Some(Link::Folder),
        _ => None,
    }?;
    valid_id(&id).then_some(Link::File(id))
}

fn drive_host(host: &str) -> bool {
    HOSTS.contains(&host)
}

/// An ID as Drive makes them: letters, digits, `-` and `_`.
fn valid_id(id: &str) -> bool {
    (10..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The key of the file with Drive ID `id`.
pub fn key(id: &str) -> String {
    format!("drive:{id}")
}

/// The Drive ID of a file's key, when it is a Drive file's.
pub(crate) fn id_of(key: &str) -> Option<&str> {
    key.strip_prefix("drive:").filter(|id| valid_id(id))
}

/// The Drive links inside `bodies`, in order, each file once with the words
/// of its first link that has any; and whether a folder is linked.
pub(crate) fn links_in(bodies: &[ElementRef<'_>], post: &Url) -> (Vec<(String, String)>, bool) {
    let anchor = Selector::parse("a[href]").expect("a valid selector");
    let mut files: Vec<(String, String)> = Vec::new();
    let mut folder = false;
    for a in bodies.iter().flat_map(|body| body.select(&anchor)) {
        let Some(found) = a
            .value()
            .attr("href")
            .and_then(|href| post.join(href.trim()).ok())
            .as_ref()
            .and_then(link)
        else {
            continue;
        };
        let id = match found {
            Link::Folder => {
                folder = true;
                continue;
            }
            Link::File(id) => id,
        };
        let words = a.text().collect::<Vec<_>>().join(" ");
        let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
        match files.iter_mut().find(|(seen, _)| *seen == id) {
            Some((_, text)) if text.is_empty() => *text = words,
            Some(_) => {}
            None => files.push((id, words)),
        }
    }
    (files, folder)
}

/// What a post whose subtitle is in Drive offers for `episode`: the linked
/// files that serve it, each with the post's `snapshot`; the folder it links
/// instead; or `None` when it links neither.
pub(crate) fn offered(
    files: &[(String, String)],
    folder: bool,
    episode: &str,
    snapshot: &Snapshot,
) -> Option<Result<Opened, Failure>> {
    if files.is_empty() {
        return folder.then(|| {
            Ok(Opened::Elsewhere {
                reason: "자막이 Google Drive 폴더 링크로 올라와 있어요. 폴더에서 받는 방법은 아직 없어요"
                    .to_owned(),
            })
        });
    }
    let texts: Vec<&str> = files.iter().map(|(_, text)| text.as_str()).collect();
    Some(match episode::choose(episode, &texts) {
        Ok(chosen) => Ok(Opened::Files(
            chosen
                .into_iter()
                .map(|i| {
                    let (id, text) = &files[i];
                    // Until the answer names it, the file is called by the
                    // words that linked it.
                    let name = match text.is_empty() {
                        true => format!("Google Drive {id}"),
                        false => text.clone(),
                    };
                    let mut file = PostFile::new(key(id), name);
                    file.snapshot = snapshot.clone();
                    file
                })
                .collect(),
        )),
        Err(reason) => Err(Failure::new(FailureKind::Changed, reason)),
    })
}

/// Receives Drive files. Cheap to clone: the clones share the client and the
/// pace, so the sources that link Drive files (Blogger's, Tistory's, Naver's) are
/// given one and space their requests to Drive's hosts together.
#[derive(Clone)]
pub struct Drive {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    limits: Limits,
    pace: Pace,
    base: Url,
}

impl std::fmt::Debug for Drive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Drive")
    }
}

impl Default for Drive {
    fn default() -> Drive {
        Drive::new()
    }
}

impl Drive {
    /// Over the network.
    pub fn new() -> Drive {
        Drive::over(
            reqwest::Client::builder(),
            Limits::default(),
            Reach::NETWORK,
            Url::parse(DOWNLOAD).expect("a valid address"),
        )
    }

    /// With this client, limits, reach and download address (the tests'
    /// [`crate::testing`] server).
    pub(crate) fn over(
        builder: reqwest::ClientBuilder,
        limits: Limits,
        reach: Reach,
        base: Url,
    ) -> Drive {
        let follow = move |_: &Url, next: &Url| {
            reach.scheme(next) && next.host_str().is_some_and(drive_host)
        };
        Drive {
            inner: Arc::new(Inner {
                http: http::client(builder, follow),
                limits,
                pace: Pace::new(limits.spacing),
                base,
            }),
        }
    }

    /// Starts receiving the file with Drive ID `id`.
    pub(crate) async fn fetch(&self, id: &str) -> Result<Fetch, Failure> {
        let mut url = self.inner.base.join("download").expect("a valid address");
        url.query_pairs_mut()
            .append_pair("id", id)
            .append_pair("export", "download");
        self.inner.pace.wait(&url).await;
        let limits = self.inner.limits;
        let deadline = tokio::time::Instant::now() + limits.file_deadline;
        let response = tokio::time::timeout_at(deadline, self.inner.http.get(url).send())
            .await
            .map_err(|_| http::deadline_failure(limits.file_deadline))?
            .map_err(|e| http::network_failure(&e, "Google Drive에 연결하지 못했어요"))?;
        let status = response.status();
        let content_type = http::media_type(&response);
        let failure = |kind, reason: &str, size| {
            Failure::new(kind, format!("{reason} (HTTP {})", status.as_u16())).with_response(
                Some(status.as_u16()),
                content_type.clone(),
                size,
            )
        };
        if status == StatusCode::OK && content_type.as_deref() == Some("text/html") {
            let page = http::read_capped(response, http::MAX_ERROR_BODY)
                .await
                .unwrap_or_default();
            let (kind, reason) = page_says(&String::from_utf8_lossy(&page));
            return Err(failure(kind, reason, Some(page.len() as u64)));
        }
        if status != StatusCode::OK {
            // A sign-in is where Drive sends a request for a file it does not
            // share: the redirect the client did not follow.
            let to_sign_in = response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|l| Url::parse(l).ok())
                .is_some_and(|l| l.host_str() == Some("accounts.google.com"));
            let size = http::error_size(response).await;
            let (kind, reason) = match status.as_u16() {
                404 | 410 => (FailureKind::Missing, "Google Drive에 파일이 없어요"),
                401 | 403 => (
                    FailureKind::Missing,
                    "Google Drive 파일이 공개돼 있지 않아요",
                ),
                300..=399 if to_sign_in => (
                    FailureKind::Missing,
                    "Google Drive가 로그인을 요구해요. 파일이 공개돼 있지 않아요",
                ),
                300..=399 => (
                    FailureKind::Changed,
                    "Google Drive가 따라갈 수 없는 곳으로 넘기려 했어요",
                ),
                429 | 500..=599 => (FailureKind::Network, "Google Drive가 파일을 주지 못했어요"),
                _ => (FailureKind::Changed, "Google Drive가 뜻밖의 답을 줬어요"),
            };
            return Err(failure(kind, reason, size));
        }
        let name = response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .and_then(|v| disposition_name(v.as_bytes()));
        let mut fetch = http::take_file(response, limits, deadline)?;
        if let Some(length) = fetch.expected_size {
            fetch.snapshot.push(CONTENT_LENGTH, length.to_string());
        }
        fetch.name = name;
        Ok(fetch)
    }
}

/// What a web page Drive gave instead of the file says, as a failure.
fn page_says(page: &str) -> (FailureKind, &'static str) {
    match () {
        _ if ["download-form", "uc-download-link", "Virus scan warning"]
            .iter()
            .any(|m| page.contains(m)) =>
        {
            (
                FailureKind::NotAFile,
                "Google Drive가 파일 대신 다운로드 확인 페이지를 줬어요. 확인을 건너뛰어 받지 않아요",
            )
        }
        _ if ["Quota exceeded", "Too many users"]
            .iter()
            .any(|m| page.contains(m)) =>
        {
            (
                FailureKind::NotAFile,
                "Google Drive가 파일 대신 다운로드 한도를 넘었다는 페이지를 줬어요",
            )
        }
        _ if ["ServiceLogin", "accounts.google.com/v3/signin"]
            .iter()
            .any(|m| page.contains(m)) =>
        {
            (
                FailureKind::Missing,
                "Google Drive가 파일 대신 로그인 페이지를 줬어요. 파일이 공개돼 있지 않아요",
            )
        }
        _ => (
            FailureKind::NotAFile,
            "Google Drive가 파일 대신 웹 페이지를 줬어요",
        ),
    }
}

/// The file name of a `Content-Disposition`: `filename*=UTF-8''…` when it is
/// there, else `filename="…"`, whose bytes Drive sends as UTF-8 as they are.
fn disposition_name(value: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(value);
    // ASCII lower-casing keeps every byte where it is.
    let lower = text.to_ascii_lowercase();
    let value_at = |key: &str| {
        lower
            .find(key)
            .map(|at| text[at + key.len()..].trim_start())
    };
    let name = if let Some(extended) = value_at("filename*=") {
        let raw = extended.split(';').next().unwrap_or_default().trim();
        let encoded = raw.rsplit_once('\'').map_or(raw, |(_, v)| v);
        percent_encoding::percent_decode_str(encoded)
            .decode_utf8_lossy()
            .into_owned()
    } else {
        let plain = value_at("filename=")?;
        match plain.strip_prefix('"') {
            Some(quoted) => {
                let mut name = String::new();
                let mut chars = quoted.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => name.extend(chars.next()),
                        c => name.push(c),
                    }
                }
                name
            }
            None => plain.split(';').next().unwrap_or_default().to_owned(),
        }
    };
    let name = name.trim().to_owned();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn the_links_seen_on_posts_name_a_file_or_a_folder() {
        let id = "1W0LRBy-gxCLtmDpQBd8kGYb79oh19YRe";
        let file = Some(Link::File(id.to_owned()));
        for address in [
            format!("https://drive.google.com/file/d/{id}/view?usp=sharing"),
            format!("https://drive.google.com/file/d/{id}/view?usp=drive_link"),
            format!("https://drive.google.com/uc?authuser=0&id={id}&export=download"),
            format!("https://drive.google.com/open?id={id}"),
            format!("https://drive.usercontent.google.com/download?id={id}&export=download"),
            format!("http://drive.google.com/file/d/{id}"),
            format!("https://drive.google.com/u/0/uc?id={id}&export=download"),
            format!("https://drive.google.com/u/1/open?id={id}"),
            format!("https://drive.google.com/u/0/file/d/{id}/view"),
            format!("https://drive.google.com/a/example.ac.kr/file/d/{id}/view"),
            format!("https://drive.google.com/a/example.ac.kr/uc?id={id}"),
        ] {
            assert_eq!(link(&url(&address)), file, "{address}");
        }
        for folder in [
            "https://drive.google.com/drive/folders/10YFO-jkkgsybQnPpl5TVAwPdx2P0SE-y",
            "https://drive.google.com/drive/u/0/folders/10YFO-jkkgsybQnPpl5TVAwPdx2P0SE-y",
            "https://drive.google.com/a/example.ac.kr/drive/folders/10YFO-jkkgsybQnPpl5TVAwPdx2P0SE-y",
        ] {
            assert_eq!(link(&url(folder)), Some(Link::Folder), "{folder}");
        }
        for other in [
            "https://drive.google.com/",
            "https://drive.google.com/u/x/uc?id=1W0LRBy-gxCLtmDpQBd8kGYb79oh19YRe",
            "https://drive.google.com/file/d/short/view",
            "https://drive.google.com/file/d/1W0LRBy%2Fgx..CLtmDp/view",
            "https://evil.example/file/d/1W0LRBy-gxCLtmDpQBd8kGYb79oh19YRe/view",
            "https://docs.google.com/document/d/1W0LRBy-gxCLtmDpQBd8kGYb79oh19YRe/edit",
            "ftp://drive.google.com/file/d/1W0LRBy-gxCLtmDpQBd8kGYb79oh19YRe/view",
        ] {
            assert_eq!(link(&url(other)), None, "{other}");
        }
        assert_eq!(id_of(&key(id)), Some(id));
        assert_eq!(id_of("tistory:blog.kakaocdn.net/dna/x"), None);
    }

    #[test]
    fn the_name_of_the_answer_is_read_as_drive_sends_it() {
        // The bytes of UTF-8 as they are, as Drive sent `그랑블루3 1-12.zip`.
        assert_eq!(
            disposition_name("attachment; filename=\"그랑블루3 1-12.zip\"".as_bytes()).as_deref(),
            Some("그랑블루3 1-12.zip")
        );
        assert_eq!(
            disposition_name(
                b"attachment; filename=\"Koori no Jouheki (The Ramparts of Ice) 15.ass\""
            )
            .as_deref(),
            Some("Koori no Jouheki (The Ramparts of Ice) 15.ass")
        );
        assert_eq!(
            disposition_name(
                b"attachment; filename=\"a.zip\"; filename*=UTF-8''%EA%B7%B8%EB%9E%91.zip"
            )
            .as_deref(),
            Some("그랑.zip")
        );
        assert_eq!(
            disposition_name(br#"attachment; filename="say \"hi\"; ok.ass""#).as_deref(),
            Some("say \"hi\"; ok.ass")
        );
        assert_eq!(
            disposition_name(b"attachment; filename=plain.srt; size=3").as_deref(),
            Some("plain.srt")
        );
        assert_eq!(disposition_name(b"attachment"), None);
        assert_eq!(disposition_name(b"attachment; filename=\"\""), None);
    }

    #[test]
    fn a_page_instead_of_the_file_is_never_the_file() {
        for (page, kind) in [
            (
                r#"<title>Google Drive - Virus scan warning</title><form id="download-form" action="https://drive.usercontent.google.com/download">"#,
                FailureKind::NotAFile,
            ),
            (
                "<title>Google Drive - Quota exceeded</title>Too many users have viewed or downloaded this file recently.",
                FailureKind::NotAFile,
            ),
            (
                r#"<a href="https://accounts.google.com/ServiceLogin?continue=x">Sign in</a>"#,
                FailureKind::Missing,
            ),
            ("<html><body>Error</body></html>", FailureKind::NotAFile),
        ] {
            assert_eq!(page_says(page).0, kind, "{page}");
        }
    }

    #[test]
    fn a_post_of_folders_only_points_elsewhere_and_the_chosen_files_keep_its_snapshot() {
        let mut snapshot = Snapshot::default();
        snapshot.push("dateModified", "2026-09-27T22:55:03+09:00");
        let Some(Ok(Opened::Elsewhere { reason })) = offered(&[], true, "1", &snapshot) else {
            panic!("elsewhere");
        };
        assert!(reason.contains("폴더"));
        assert!(offered(&[], false, "1", &snapshot).is_none());

        let files = vec![
            (
                "1yiOdJ6YbwrbGmJ4gVMGeLfhdk4w0dAwM".to_owned(),
                "폰트".to_owned(),
            ),
            (
                "10EFK_9-G9VPVFy08Zuit88H0ciLJMFKq".to_owned(),
                "1 ~ 12화".to_owned(),
            ),
            (
                "1C13Daai_8VJqwkal9YtbEfZ3chenHlSt".to_owned(),
                "13화".to_owned(),
            ),
        ];
        let Some(Ok(Opened::Files(chosen))) = offered(&files, true, "12", &snapshot) else {
            panic!("files");
        };
        let names: Vec<&str> = chosen.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["폰트", "1 ~ 12화"]);
        assert_eq!(chosen[1].key, "drive:10EFK_9-G9VPVFy08Zuit88H0ciLJMFKq");
        assert_eq!(chosen[1].snapshot, snapshot);
        let Some(Err(failure)) = offered(&files, false, "14", &snapshot) else {
            panic!("failure");
        };
        assert_eq!(failure.kind, FailureKind::Changed);
        assert_eq!(failure.reason, "게시물에 14화 파일이 없어요");
    }
}
