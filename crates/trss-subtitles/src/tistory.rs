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
//!   thumbnails. Google Drive file links there are received as Blogger's are
//!   ([`crate::drive`]): those that serve the candidate's episode. A Drive
//!   folder, or an image on `*.kakaocdn.net` ending in `.png` (WinPNG), puts
//!   the subtitle elsewhere ([`Opened::Elsewhere`]). With none of these, or
//!   with no body this module knows, the post has [`FailureKind::Changed`].
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

use std::sync::Arc;

use percent_encoding::percent_decode_str;
use scraper::{Html, Selector};
use url::Url;

pub use crate::http::{Limits, LAST_MODIFIED, SPACING};
use crate::{
    drive::{self, Drive},
    http::{self, Pace, Reach},
    Failure, FailureKind, Fetch, FileInfo, Opened, PostFile, Snapshot,
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

/// A post's body, the containers seen on real posts (`docs/specs/jobs.md`,
/// 공통 수신 결과와 실패 분류): the editor's `tt_article_useless_p_margin
/// contents_style` in every skin seen, inside `#article-view` in some.
pub const BODY: &str = ".tt_article_useless_p_margin, .contents_style, #article-view";

/// The snapshot's names.
pub const POST_MODIFIED: &str = "article:modified_time";
pub const DECLARED_SIZE: &str = "declared_size";

/// A file's address: on the CDN.
fn file_address(reach: Reach, url: &Url) -> bool {
    reach.scheme(url) && url.domain().is_some_and(cdn_host)
}

/// A post's address: a blog on `tistory.com`.
fn post_address(reach: Reach, url: &Url) -> bool {
    reach.scheme(url) && url.domain().is_some_and(reads)
}

/// Reads Tistory posts and their attachments. Cheap to clone; the clones share
/// the clients and the pace.
#[derive(Clone)]
pub struct TistorySource {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    limits: Limits,
    reach: Reach,
    pace: Pace,
    /// The Drive files a body links.
    drive: Drive,
}

impl std::fmt::Debug for TistorySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TistorySource")
    }
}

impl TistorySource {
    /// The source over the network, receiving the Drive files its posts link
    /// through `drive`, which the other sources share.
    pub fn new(drive: Drive) -> TistorySource {
        TistorySource::over(
            reqwest::Client::builder(),
            Limits::default(),
            Reach::NETWORK,
            drive,
        )
    }

    /// The source with its own client, limits, reach and Drive (the tests'
    /// [`crate::testing`] server).
    pub(crate) fn over(
        builder: reqwest::ClientBuilder,
        limits: Limits,
        reach: Reach,
        drive: Drive,
    ) -> TistorySource {
        let follow = move |first: &Url, next: &Url| match file_address(reach, first) {
            true => file_address(reach, next),
            false => post_address(reach, next),
        };
        TistorySource {
            inner: Arc::new(Inner {
                http: http::client(builder, follow),
                limits,
                reach,
                pace: Pace::new(limits.spacing),
                drive,
            }),
        }
    }

    pub(crate) async fn open(&self, post: &Url, episode: &str) -> Result<Opened, Failure> {
        let page = http::get_page(&self.inner.http, &self.inner.pace, post).await?;
        read_page(post, &page, self.inner.reach, episode)
    }

    pub(crate) async fn fetch(&self, post: &Url, file: &PostFile) -> Result<Fetch, Failure> {
        // A Drive file has no signed address to read again.
        if let Some(id) = drive::id_of(&file.key) {
            return self.inner.drive.fetch(id).await;
        }
        let first = match file.locator() {
            Some(locator) => self.get_file(locator).await,
            None => Err(Failure::new(FailureKind::Expired, "받을 주소가 없어요")),
        };
        let expired = match first {
            Err(failure) if failure.kind == FailureKind::Expired => failure,
            other => return other,
        };
        // Once more, with the address the post gives now.
        let again = match http::get_page(&self.inner.http, &self.inner.pace, post).await {
            Ok(page) => attachments(post, &Html::parse_document(&page), self.inner.reach)
                .into_iter()
                .find(|f| f.key == file.key),
            Err(failure) if failure.kind == FailureKind::Network => return Err(failure),
            Err(_) => None,
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

    /// The files `keys` name, read again without receiving them (see
    /// [`crate::Source::recheck`]). An attachment's signed address is never
    /// kept, so the post is read again for it (once, whatever the number of
    /// files), and the address is asked for one byte: the total in the answer
    /// is the file's size ([`http::range_total`]). A Drive file in the body
    /// is a `HEAD` of its fixed address, with no post read.
    pub(crate) async fn recheck(
        &self,
        post: &Url,
        keys: &[String],
    ) -> Vec<(String, Result<FileInfo, Failure>)> {
        let mut answers = Vec::new();
        // The attachments the post offers now, read when the first one needs it.
        let mut offered: Option<Result<Vec<PostFile>, Failure>> = None;
        for key in keys {
            let info = if let Some(id) = drive::id_of(key) {
                self.inner.drive.head(id).await
            } else {
                if offered.is_none() {
                    offered = Some(
                        http::get_page(&self.inner.http, &self.inner.pace, post)
                            .await
                            .map(|page| {
                                attachments(post, &Html::parse_document(&page), self.inner.reach)
                            }),
                    );
                }
                match offered.as_ref().expect("read above") {
                    Err(failure) => Err(failure.clone()),
                    Ok(files) => match files.iter().find(|f| f.key == *key) {
                        None => Err(Failure::new(
                            FailureKind::Missing,
                            "게시물을 다시 읽었지만 이 파일이 없어요",
                        )),
                        Some(file) => match file.locator() {
                            Some(locator) => {
                                http::range_total(&self.inner.http, &self.inner.pace, locator).await
                            }
                            None => Err(Failure::new(FailureKind::Expired, "받을 주소가 없어요")),
                        },
                    },
                }
            };
            answers.push((key.clone(), info));
        }
        answers
    }

    async fn get_file(&self, locator: &Url) -> Result<Fetch, Failure> {
        http::get_file(
            &self.inner.http,
            &self.inner.pace,
            self.inner.limits,
            locator,
        )
        .await
    }
}

/// The attachments of a post's page: its fileblocks on the CDN.
fn attachments(post: &Url, html: &Html, reach: Reach) -> Vec<PostFile> {
    let select = |css: &str| Selector::parse(css).expect("a valid selector");
    let modified = post_modified(html);
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
            .filter(|u| file_address(reach, u))
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
        if let Some(modified) = &modified {
            file.snapshot.push(POST_MODIFIED, modified.clone());
        }
        if let Some(size) = &file.size_text {
            file.snapshot.push(DECLARED_SIZE, size.clone());
        }
        files.push(file.with_locator(url));
    }
    files
}

/// The post's `article:modified_time`.
fn post_modified(html: &Html) -> Option<String> {
    let meta =
        Selector::parse(r#"meta[property="article:modified_time"]"#).expect("a valid selector");
    html.select(&meta)
        .find_map(|m| m.value().attr("content"))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// What a post's page offers for `episode` (see the module docs).
pub(crate) fn read_page(
    post: &Url,
    page: &str,
    reach: Reach,
    episode: &str,
) -> Result<Opened, Failure> {
    let html = Html::parse_document(page);
    let files = attachments(post, &html, reach);
    if !files.is_empty() {
        return Ok(Opened::Files(files));
    }

    // Only the body says where the subtitle is: the skin around it has
    // links and thumbnails of its own.
    let select = |css: &str| Selector::parse(css).expect("a valid selector");
    let bodies: Vec<scraper::ElementRef<'_>> = html.select(&select(BODY)).collect();
    if bodies.is_empty() {
        return Err(Failure::new(
            FailureKind::Changed,
            "게시물에서 첨부 파일도 본문도 찾지 못했어요",
        ));
    }
    let mut snapshot = Snapshot::default();
    if let Some(modified) = post_modified(&html) {
        snapshot.push(POST_MODIFIED, modified);
    }
    let (drive_files, folder) = drive::links_in(&bodies, post);
    if !drive_files.is_empty() {
        return drive::offered(&drive_files, false, episode, &snapshot).expect("files are offered");
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
    if let Some(elsewhere) = drive::offered(&[], folder, episode, &snapshot) {
        return elsewhere;
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
        read_page(post, page, Reach::NETWORK, "24")
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
<div class="header"><img src="https://blog.kakaocdn.net/dna/S/K/N/logo.png"><a href="https://drive.google.com/file/d/1SkinSkinSkin/view">1화</a></div>
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
    fn a_post_without_a_fileblock_offers_its_drive_files_points_elsewhere_or_has_changed() {
        // felia 1187: one Drive link that says nothing of the episode, beside
        // a picture on the CDN.
        let drive = page(
            r#"<table><tr><td><a href="https://drive.google.com/file/d/1zqWESZSANz8Uj2aaW9u1oSHKurLuTqQw/view?usp=drive_link"><span><b>자막 다운로드</b></span></a></td></tr></table>"#,
        );
        let Ok(Opened::Files(files)) = read_page_(&post(), &drive) else {
            panic!("files");
        };
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].key, "drive:1zqWESZSANz8Uj2aaW9u1oSHKurLuTqQw");
        assert_eq!(files[0].name, "자막 다운로드");
        assert_eq!(
            files[0].snapshot.entries(),
            [(
                POST_MODIFIED.to_owned(),
                "2026-09-28T00:13:41+09:00".to_owned()
            )]
        );
        // Drive links of other episodes only: the post has changed.
        let other = page(
            r#"<p><a href="https://drive.google.com/file/d/1zqWESZSANz8Uj2aaW9u1oSHKurLuTqQw/view">23화</a></p>"#,
        );
        let failure = read_page_(&post(), &other).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert!(failure.reason.contains("24화"));
        // A Drive folder alone: elsewhere.
        let folder = plain_page(
            r#"<p><a href="https://drive.google.com/drive/folders/10YFO-jkkgsybQnPpl5TVAwPdx2P0SE-y">자막 모음</a></p>"#,
        );
        let Ok(Opened::Elsewhere { reason }) = read_page_(&post(), &folder) else {
            panic!("elsewhere");
        };
        assert!(reason.contains("Google Drive 폴더"));

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
        // Inside `#article-view` as well, and the skin's Drive link is not
        // one of the post's.
        let drive =
            r#"<p><a href="https://drive.google.com/file/d/1AbCdEfGhIjK/view">24화</a></p>"#;
        let Ok(Opened::Files(files)) = read_page_(&post(), &plain_page(drive)) else {
            panic!("files");
        };
        let keys: Vec<&str> = files.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["drive:1AbCdEfGhIjK"]);
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
