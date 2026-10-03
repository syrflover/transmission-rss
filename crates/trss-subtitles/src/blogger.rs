//! Blogger posts (`docs/specs/subtitles.md`, 출처별 다운로드; `docs/specs/
//! jobs.md`, 출처별 단계 초안): a post on `<blog>.blogspot.com` read over
//! HTTPS with no cookie, whose subtitle is a Google Drive file linked in its
//! body ([`drive`]).
//!
//! - The body is the post's one `.post-body` (every theme seen, 2026-10-03:
//!   `post-body entry-content`). Links outside it (the blog's own folder of
//!   all its subtitles, beside every post of 별명따위) are not read. A page
//!   with no body, or with more than one (a blog's front page), has
//!   [`FailureKind::Changed`].
//! - Of the Drive files the body links, those that serve the candidate's
//!   episode are offered ([`crate::episode`]); a body that links only Drive
//!   folders has its subtitle elsewhere; a body with neither has
//!   [`FailureKind::Changed`].
//! - The post's `dateModified` (its JSON-LD, in the themes that have one) goes
//!   to each file's snapshot.
//! - A redirect of the post is followed only to an `https` blog on
//!   `blogspot.com`; a post asked for over `http` is asked for over `https`.
//!   Requests to one host are at least [`crate::http::SPACING`] apart.

use std::sync::Arc;

use scraper::{Html, Selector};
use url::Url;

use crate::{
    drive::{self, Drive},
    http::{self, Limits, Pace, Reach},
    Failure, FailureKind, Fetch, FileInfo, Opened, PostFile, Snapshot,
};

/// The hosts this source reads: `<blog>.blogspot.com`.
pub fn reads(host: &str) -> bool {
    host.strip_suffix(".blogspot.com")
        .is_some_and(|blog| !blog.is_empty() && blog != "www" && !blog.contains('.'))
}

/// A post's body.
pub const BODY: &str = ".post-body";

/// The snapshot's name for the post's `dateModified`.
pub const POST_MODIFIED: &str = "dateModified";

/// Reads Blogger posts and the Drive files they link. Cheap to clone; the
/// clones share the clients and the pace.
#[derive(Clone)]
pub struct BloggerSource {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    reach: Reach,
    pace: Pace,
    drive: Drive,
}

impl std::fmt::Debug for BloggerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BloggerSource")
    }
}

impl BloggerSource {
    /// The source over the network, receiving the Drive files its posts link
    /// through `drive`, which the other sources share.
    pub fn new(drive: Drive) -> BloggerSource {
        BloggerSource::over(
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
    ) -> BloggerSource {
        let follow =
            move |_: &Url, next: &Url| reach.scheme(next) && next.domain().is_some_and(reads);
        BloggerSource {
            inner: Arc::new(Inner {
                http: http::client(builder, follow),
                reach,
                pace: Pace::new(limits.spacing),
                drive,
            }),
        }
    }

    pub(crate) async fn open(&self, post: &Url, episode: &str) -> Result<Opened, Failure> {
        let mut post = post.clone();
        if !self.inner.reach.plain_http && post.scheme() == "http" {
            let _ = post.set_scheme("https");
        }
        let page = http::get_page(&self.inner.http, &self.inner.pace, &post).await?;
        read_page(&post, &page, episode)
    }

    /// The Drive files `keys` name, read again with a `HEAD` each (see
    /// [`crate::Source::recheck`]). The post is not read: a Drive file's
    /// address does not change.
    pub(crate) async fn recheck(
        &self,
        _post: &Url,
        keys: &[String],
    ) -> Vec<(String, Result<FileInfo, Failure>)> {
        let mut answers = Vec::new();
        for key in keys {
            let info = match drive::id_of(key) {
                Some(id) => self.inner.drive.head(id).await,
                None => Err(Failure::new(
                    FailureKind::Changed,
                    "Blogger 게시물이 Google Drive 밖의 파일을 가리켜요",
                )),
            };
            answers.push((key.clone(), info));
        }
        answers
    }

    pub(crate) async fn fetch(&self, _post: &Url, file: &PostFile) -> Result<Fetch, Failure> {
        match drive::id_of(&file.key) {
            Some(id) => self.inner.drive.fetch(id).await,
            None => Err(Failure::new(
                FailureKind::Changed,
                "Blogger 게시물이 Google Drive 밖의 파일을 가리켜요",
            )),
        }
    }
}

/// What a post's page offers for `episode` (see the module docs).
pub(crate) fn read_page(post: &Url, page: &str, episode: &str) -> Result<Opened, Failure> {
    let html = Html::parse_document(page);
    let select = |css: &str| Selector::parse(css).expect("a valid selector");
    let bodies: Vec<_> = html.select(&select(BODY)).collect();
    match bodies.len() {
        0 => {
            return Err(Failure::new(
                FailureKind::Changed,
                "게시물에서 본문을 찾지 못했어요",
            ))
        }
        1 => {}
        n => {
            return Err(Failure::new(
                FailureKind::Changed,
                format!("게시물 하나가 아니라 글 {n}개가 있는 페이지예요"),
            ))
        }
    }
    let mut snapshot = Snapshot::default();
    if let Some(modified) = date_modified(&html) {
        snapshot.push(POST_MODIFIED, modified);
    }
    let (files, folder) = drive::links_in(&bodies, post);
    drive::offered(&files, folder, episode, &snapshot).unwrap_or_else(|| {
        Err(Failure::new(
            FailureKind::Changed,
            "게시물 본문에서 Google Drive 파일 링크를 찾지 못했어요",
        ))
    })
}

/// The post's `dateModified` from its JSON-LD, when the theme writes one.
fn date_modified(html: &Html) -> Option<String> {
    let scripts =
        Selector::parse(r#"script[type="application/ld+json"]"#).expect("a valid selector");
    html.select(&scripts).find_map(|script| {
        let text = script.text().collect::<String>();
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        let modified = value.get("dateModified")?.as_str()?.trim();
        (!modified.is_empty() && modified.len() <= 64).then(|| modified.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post() -> Url {
        Url::parse("https://csora556.blogspot.com/2026/07/2.html").unwrap()
    }

    fn drive(id: &str) -> String {
        format!("https://drive.google.com/file/d/{id}/view?usp=sharing")
    }

    /// A post as the newer themes write it (C소라, 별명따위): its JSON-LD, the
    /// body, and the blog's folder link beside it.
    fn page(body: &str) -> String {
        format!(
            r#"<!DOCTYPE html><html><head><script type="application/ld+json">{{
  "@context": "http://schema.org",
  "@type": "BlogPosting",
  "headline": "정반대의 너와 나 2기",
  "datePublished": "2026-07-05T22:58:00+09:00",
  "dateModified": "2026-09-27T22:55:03+09:00"
}}</script></head><body>
<div class='post-body-container'><div class='post-body entry-content float-container' id='post-body-1'>{body}</div></div>
<div class='widget LinkList'><a href='https://drive.google.com/drive/folders/10YFO-jkkgsybQnPpl5TVAwPdx2P0SE-y'>자막 모음(작업 중)</a></div>
</body></html>"#
        )
    }

    #[test]
    fn only_blogs_on_blogspot_are_read() {
        assert!(reads("csora556.blogspot.com"));
        assert!(!reads("blogspot.com"));
        assert!(!reads("www.blogspot.com"));
        assert!(!reads("a.b.blogspot.com"));
        assert!(!reads("csora556.blogspot.com.example"));
    }

    #[test]
    fn the_episodes_file_and_the_fonts_are_offered_with_the_posts_modified_time() {
        let mut links = format!(
            r#"<a href="{}">폰트</a><br />"#,
            drive("129b_0l8KfVERXsp3eMTQgysAXNNKpQJH")
        );
        for n in 13..=24 {
            links.push_str(&format!(
                r#"<a href="{}" target="_blank">{n}화</a><br />"#,
                drive(&format!("1nHhIm1tRIs9lrdRe-89PAfVuKJrfx{n:02}"))
            ));
        }
        let Ok(Opened::Files(files)) = read_page(&post(), &page(&links), "24") else {
            panic!("files");
        };
        let shown: Vec<(&str, &str)> = files
            .iter()
            .map(|f| (f.key.as_str(), f.name.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("drive:129b_0l8KfVERXsp3eMTQgysAXNNKpQJH", "폰트"),
                ("drive:1nHhIm1tRIs9lrdRe-89PAfVuKJrfx24", "24화"),
            ]
        );
        assert_eq!(
            files[1].snapshot.entries(),
            [(
                POST_MODIFIED.to_owned(),
                "2026-09-27T22:55:03+09:00".to_owned()
            )]
        );
        let failure = read_page(&post(), &page(&links), "25").unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert!(failure.reason.contains("25화"));
    }

    #[test]
    fn a_picture_link_alone_is_the_posts_file_and_a_folder_alone_is_elsewhere() {
        // 카이란: a picture links the one file; an older theme with no JSON-LD.
        let picture = format!(
            r#"<div class="separator"><a href="{}" style="margin-left: 1em;"><img border="0" src="https://blogger.googleusercontent.com/img/b/x.png" /></a></div>"#,
            drive("1C13Daai_8VJqwkal9YtbEfZ3chenHlSt")
        );
        let older = format!(
            "<html><body><div class='post-body entry-content' id='post-body-3'>{picture}</div></body></html>"
        );
        let Ok(Opened::Files(files)) = read_page(&post(), &older, "19") else {
            panic!("files");
        };
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].name,
            "Google Drive 1C13Daai_8VJqwkal9YtbEfZ3chenHlSt"
        );
        assert!(files[0].snapshot.is_empty());

        // Only the blog's folder, inside the body this time.
        let folder = page(
            r#"<a href="https://drive.google.com/drive/folders/10YFO-jkkgsybQnPpl5TVAwPdx2P0SE-y">자막 모음</a>"#,
        );
        assert!(matches!(
            read_page(&post(), &folder, "1"),
            Ok(Opened::Elsewhere { .. })
        ));
        // The folder beside the body is not read.
        let failure = read_page(&post(), &page("<p>자막은 내일 올려요</p>"), "1").unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
    }

    #[test]
    fn a_page_with_no_body_or_many_has_changed() {
        let none = "<html><body><div class='entry'>x</div></body></html>";
        assert_eq!(
            read_page(&post(), none, "1").unwrap_err().kind,
            FailureKind::Changed
        );
        let many = format!(
            "{}{}",
            page(&format!(
                r#"<a href="{}">1화</a>"#,
                drive("1C13Daai_8VJqwkal9YtbEfZ3chenHlSt")
            )),
            "<div class='post-body'>another</div>"
        );
        let failure = read_page(&post(), &many, "1").unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert!(failure.reason.contains("2개"));
    }
}
