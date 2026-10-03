//! erulabo's posts (`docs/specs/subtitles.md`, 출처별 다운로드; `docs/specs/
//! jobs.md`, 출처별 단계 초안): a post's subtitle comes only after a person
//! passes the site's check (Cloudflare Turnstile) in the server browser
//! ([`crate::auth`]).
//!
//! - The post (`https://erulabo.com/<number>`) is read over HTTPS with no
//!   cookie (a page of about 170KB, rendered by the server). Its download
//!   cards are the `button[data-file-url="/file/<id>"]` of `#post-body`, each
//!   with a title (`.og-title`) that ends in the episode: `전생귀족3 (1)`, and
//!   on the last post of a series a second card for all of them,
//!   `코코오레 (1-12)` (2026-10-03). Which card serves the candidate's episode
//!   is [`choose_card`]'s. The post's `dateModified` (its JSON-LD) goes to the
//!   snapshot.
//! - In the browser ([`drive_check`]) the driver opens the post, waits for the
//!   site's download script (it loads Turnstile's script as it starts, so
//!   `window.turnstile` says it is there), scrolls the chosen card to the
//!   middle of the screen and clicks it. The site then shows the check inside
//!   the card ([`WRAP`]). The driver keeps it in the middle when the screen's
//!   size changes (a phone opening the job's screen); it never touches the
//!   check.
//! - The site gives the check 30 seconds: then it logs the failure to itself,
//!   hides the check, puts the card back and shows a notice over the whole
//!   page for about 2 seconds (2026-10-03). So a person who opens the job's
//!   screen later finds the card again; the driver clicks it again when they
//!   do, once the notice is gone ([`rearm`]).
//! - Once the check is passed, the site asks for a download address that
//!   lives 60 seconds and opens it, in a new tab or in the page; the file came
//!   from Google Drive (2026-10-02). The browser's download is the file
//!   ([`crate::auth`] records what its answer said, and the Drive file's ID).
//!   An answer that is a web page instead ([`download_refusal`]) ends the
//!   item; the address is never asked for again, and a new receipt goes
//!   through the check again.
//! - Nothing is read again without the check: a recheck answers
//!   [`FailureKind::Changed`], which the recheck records as unreadable.

use std::{sync::Arc, time::Duration};

use scraper::{Html, Selector};
use trss_browser::Page;
use url::Url;

use crate::{
    auth::{self, AuthPage},
    blogger, drive,
    episode::{self, Holds},
    http::{self, Limits, Pace, Reach},
    Failure, FailureKind, FileInfo, Opened, Snapshot,
};

/// The site's host.
pub const HOST: &str = "erulabo.com";

/// The hosts this source reads.
pub fn reads(host: &str) -> bool {
    host == HOST || host == "www.erulabo.com"
}

/// What the check is called on the job's screen and in its log.
pub const CHECK_REASON: &str = "erulabo 보안 확인(Cloudflare Turnstile)";

/// The snapshot's name for the post's `dateModified`.
pub const POST_MODIFIED: &str = blogger::POST_MODIFIED;

/// Where the site shows the check, inside the clicked card.
pub const WRAP: &str = "document.querySelector('[data-file-download-turnstile-wrap]')";

/// How long the check has to appear after the card is clicked (0.5 seconds
/// on 2026-10-03).
const CHECK_SHOWS: Duration = Duration::from_secs(15);

/// How long the site's notice that the check's time ran out, which covers
/// the page, is waited for to go before the card is clicked (it went after
/// about 2 seconds on 2026-10-03). A card still covered is not clicked.
const NOTICE_GOES: Duration = Duration::from_secs(10);

/// A post brought to its check: the card that serves the episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErulaboCheck {
    /// The card's `data-file-url`, `/file/<id>`.
    file: String,
    /// The card's title, as the post wrote it.
    title: String,
    /// What the post said about itself (`dateModified`).
    snapshot: Snapshot,
}

impl ErulaboCheck {
    pub fn new(file: impl Into<String>, title: impl Into<String>, snapshot: Snapshot) -> Self {
        ErulaboCheck {
            file: file.into(),
            title: title.into(),
            snapshot,
        }
    }

    pub fn file(&self) -> &str {
        &self.file
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// The card, as a JavaScript expression. The file's path is one
    /// [`is_file_path`] lets through, so it goes in as it is.
    fn card(&self) -> String {
        format!(
            "document.querySelector('#post-body button[data-file-url=\"{}\"]')",
            self.file
        )
    }
}

/// Whether `path` is a card's file as the site's script takes it
/// (`/file/<id>`), of characters that go into a selector as they are.
fn is_file_path(path: &str) -> bool {
    path.strip_prefix("/file/").is_some_and(|id| {
        !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

/// The post's own address, `https://erulabo.com/<number>`, or why `post` is
/// not one.
fn post_address(post: &Url, reach: Reach) -> Result<Url, Failure> {
    let not_a_post = || Failure::new(FailureKind::Changed, "erulabo 게시물 주소가 아니에요");
    if !post.host_str().is_some_and(reads) || !matches!(post.scheme(), "https" | "http") {
        return Err(not_a_post());
    }
    let segments: Vec<&str> = post
        .path_segments()
        .map(|s| s.filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    match segments[..] {
        [number]
            if !number.is_empty()
                && number.len() <= 12
                && number.bytes().all(|b| b.is_ascii_digit()) =>
        {
            let scheme = if reach.plain_http && post.scheme() == "http" {
                "http"
            } else {
                "https"
            };
            let mut address = post.clone();
            let _ = address.set_scheme(scheme);
            address.set_path(&format!("/{number}"));
            address.set_query(None);
            address.set_fragment(None);
            Ok(address)
        }
        _ => Err(not_a_post()),
    }
}

/// Which of a post's cards, by their titles, serves `episode`: the first that
/// names that one episode (`해골기사님2 (12)`), else the first whose range
/// holds it (`해골기사님2 (1-12)`), else the only card when it names no
/// episode. One card only: each is a check of its own.
pub fn choose_card(episode: &str, titles: &[&str]) -> Result<usize, String> {
    let key = episode::numeric_key(episode.trim());
    let held: Vec<Holds> = titles.iter().map(|t| episode::holds(t)).collect();
    let spans = |h: &Holds| match h {
        Holds::Episodes(spans) => spans.clone(),
        _ => Vec::new(),
    };
    if let Some(key) = key.as_deref() {
        let exact = held
            .iter()
            .position(|h| spans(h).iter().any(|s| s.from == key && s.to == key));
        let within = || {
            held.iter()
                .position(|h| spans(h).iter().any(|s| s.holds(key)))
        };
        if let Some(found) = exact.or_else(within) {
            return Ok(found);
        }
    }
    if let [Holds::Nothing | Holds::Bundle] = held[..] {
        return Ok(0);
    }
    let label = match &key {
        Some(key) => format!("{key}화"),
        None => episode.trim().to_owned(),
    };
    Err(match titles.len() {
        0 => "게시물에 받기 카드가 없어요".to_owned(),
        _ => format!("게시물에 {label} 받기 카드가 없어요"),
    })
}

/// What a post's page offers for `episode` (see the module docs).
pub(crate) fn read_page(page: &str, episode: &str) -> Result<Opened, Failure> {
    let html = Html::parse_document(page);
    let select = |css: &str| Selector::parse(css).expect("a valid selector");
    if html.select(&select("#post-body")).next().is_none() {
        return Err(Failure::new(
            FailureKind::Changed,
            "게시물에서 본문을 찾지 못했어요",
        ));
    }
    let title_of = select(".og-title");
    let cards: Vec<(String, String)> = html
        .select(&select("#post-body button[data-file-url]"))
        .filter_map(|card| {
            let file = card.value().attr("data-file-url")?.trim();
            if !is_file_path(file) {
                return None;
            }
            let title = card
                .select(&title_of)
                .next()
                .map(|t| t.text().collect::<String>())
                .unwrap_or_else(|| card.text().collect::<String>());
            let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
            Some((file.to_owned(), title))
        })
        .collect();
    let titles: Vec<&str> = cards.iter().map(|(_, t)| t.as_str()).collect();
    let chosen =
        choose_card(episode, &titles).map_err(|why| Failure::new(FailureKind::Changed, why))?;
    let mut snapshot = Snapshot::default();
    if let Some(modified) = blogger::date_modified(&html) {
        snapshot.push(POST_MODIFIED, modified);
    }
    let (file, title) = cards[chosen].clone();
    Ok(Opened::BrowserAuth {
        reason: CHECK_REASON.to_owned(),
        page: AuthPage::Erulabo(ErulaboCheck::new(file, title, snapshot)),
    })
}

/// Reads erulabo's posts. Cheap to clone; the clones share the client and the
/// pace.
#[derive(Clone)]
pub struct ErulaboSource {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    reach: Reach,
    pace: Pace,
}

impl std::fmt::Debug for ErulaboSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ErulaboSource")
    }
}

impl Default for ErulaboSource {
    fn default() -> Self {
        ErulaboSource::new()
    }
}

impl ErulaboSource {
    /// The source over the network.
    pub fn new() -> ErulaboSource {
        ErulaboSource::over(
            reqwest::Client::builder(),
            Limits::default(),
            Reach::NETWORK,
        )
    }

    /// The source with its own client, limits and reach (the tests' server).
    pub(crate) fn over(
        builder: reqwest::ClientBuilder,
        limits: Limits,
        reach: Reach,
    ) -> ErulaboSource {
        // A post is followed only to the site's own posts.
        let follow =
            move |_: &Url, next: &Url| reach.scheme(next) && next.host_str().is_some_and(reads);
        ErulaboSource {
            inner: Arc::new(Inner {
                http: http::client(builder, follow),
                reach,
                pace: Pace::new(limits.spacing),
            }),
        }
    }

    pub(crate) async fn open(&self, post: &Url, episode: &str) -> Result<Opened, Failure> {
        let address = post_address(post, self.inner.reach)?;
        let page = http::get_page(&self.inner.http, &self.inner.pace, &address).await?;
        read_page(&page, episode)
    }

    /// Nothing of a file is told without the check (`docs/specs/subtitles.md`,
    /// 구독 제작자 자동 수신): every key answers [`FailureKind::Changed`].
    pub(crate) async fn recheck(
        &self,
        _post: &Url,
        keys: &[String],
    ) -> Vec<(String, Result<FileInfo, Failure>)> {
        keys.iter()
            .map(|key| {
                (
                    key.clone(),
                    Err(Failure::new(
                        FailureKind::Changed,
                        "erulabo의 파일은 사이트 확인 없이 다시 읽을 수 없어요",
                    )),
                )
            })
            .collect()
    }

    /// A file comes only through the browser, already on this machine: a
    /// file that is not there was never downloaded, and no address is kept
    /// to ask for it again.
    pub(crate) fn fetch(&self) -> Failure {
        Failure::new(
            FailureKind::Expired,
            "erulabo의 파일은 사이트 확인을 거쳐 브라우저로만 받아요",
        )
    }
}

fn browser_trouble() -> Failure {
    Failure::new(FailureKind::Network, "서버 브라우저를 쓰지 못했어요")
}

/// Whether the check shows on the page, as a JavaScript expression.
fn check_shown() -> String {
    format!(
        "(() => {{ const w = {WRAP}; return !!w && !w.hidden && w.getClientRects().length > 0; }})()"
    )
}

/// Keeps the check in the middle of the screen when its size changes (the
/// site does not: a phone's size put its top off the screen, 2026-10-03).
/// Once per document.
const KEEP_CENTERED: &str = "(() => { const mark = Symbol.for('trss.keepCentered');
  if (window[mark]) return true; window[mark] = true;
  window.addEventListener('resize', () => {
    const w = document.querySelector('[data-file-download-turnstile-wrap]');
    if (w && !w.hidden) w.scrollIntoView({block: 'center', inline: 'center', behavior: 'instant'});
  });
  return true; })()";

/// Brings the post at `post` to the check of `check`'s card in `page` (a
/// blank page of the job's run): see the module docs. The check itself is
/// left to a person.
pub(crate) async fn drive_check(
    page: &Page,
    post: &Url,
    check: &ErulaboCheck,
) -> Result<(), Failure> {
    let address = post_address(post, Reach::NETWORK)?;
    page.navigate(address.as_str())
        .await
        .map_err(|_| browser_trouble())?;
    let card = check.card();
    let ready = format!("document.readyState === 'complete' && !!{card} && !!window.turnstile");
    if !auth::wait_until(page, &ready, auth::READY_TIMEOUT).await? {
        let found = page
            .evaluate(&format!("!!{card}"))
            .await
            .map_err(|_| browser_trouble())?;
        return Err(match found.as_bool() {
            Some(true) => Failure::new(
                FailureKind::Network,
                "게시물의 보안 확인을 불러오지 못했어요",
            ),
            _ => Failure::new(
                FailureKind::Changed,
                format!("게시물에서 받기 카드({})를 찾지 못했어요", check.title),
            ),
        });
    }
    bring_to_check(page, check).await
}

/// Clicks the card, centered, and waits for the check to show.
async fn bring_to_check(page: &Page, check: &ErulaboCheck) -> Result<(), Failure> {
    if !auth::click_centered(page, &check.card()).await? {
        return Err(Failure::new(
            FailureKind::Changed,
            "게시물의 받기 카드가 사라졌어요",
        ));
    }
    if !auth::wait_until(page, &check_shown(), CHECK_SHOWS).await? {
        return Err(Failure::new(
            FailureKind::Changed,
            "받기 카드를 눌렀지만 보안 확인이 나오지 않았어요",
        ));
    }
    page.evaluate(KEEP_CENTERED)
        .await
        .map_err(|_| browser_trouble())?;
    Ok(())
}

/// A person opened the job's screen: when the check went away (its time ran
/// out and the site put the card back), the card is clicked again. A page
/// that shows the check, or that the person took elsewhere, is left as it is.
pub(crate) async fn rearm(page: &Page, post: &Url, check: &ErulaboCheck) -> Result<(), Failure> {
    let address = post_address(post, Reach::NETWORK)?;
    let state = format!(
        "JSON.stringify({{shown: {}, here: location.pathname === {:?}, card: !!{}}})",
        check_shown(),
        address.path(),
        check.card()
    );
    let state = page.evaluate(&state).await.map_err(|_| browser_trouble())?;
    let state: serde_json::Value = state
        .as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    if state["shown"] == true || state["here"] != true || state["card"] != true {
        return Ok(());
    }
    // When its time runs out the site says so in a notice over the whole
    // page, which goes about 2 seconds later (2026-10-03): the card is
    // clicked once nothing covers it.
    let uncovered = format!(
        "(() => {{ const el = {card}; if (!el) return true;
           const r = el.getBoundingClientRect();
           const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
           return !hit || hit === el || el.contains(hit); }})()",
        card = check.card()
    );
    auth::wait_until(page, &uncovered, NOTICE_GOES).await?;
    bring_to_check(page, check).await
}

/// Google's sign-in, which Drive sends a file that is not shared to.
const SIGN_IN_HOST: &str = "accounts.google.com";

/// Whether `host` is one a page of this source's download is expected to
/// come from or go through ([`download_refusal`] judges these).
pub fn known_host(host: &str) -> bool {
    drive::HOSTS.contains(&host) || reads(host) || host == SIGN_IN_HOST
}

/// Whether a document a page of the browser was answered with, after the
/// check, is a refusal of the download instead of the file: a web page from
/// the hosts the file comes from (Google Drive, the site's `/file/…`
/// addresses, Google's sign-in). A download is no document answer; the
/// person's other pages (another post, an ad) are none of these. `None` when
/// it is not one. A refusal ends the item at once, whatever it is: a new
/// address comes only from a new check, so nothing is tried again.
pub fn download_refusal(url: &Url, status: u16, content_type: Option<&str>) -> Option<Failure> {
    let host = url.host_str()?;
    let html = content_type.is_some_and(|t| t.eq_ignore_ascii_case("text/html"));
    let failure = |kind, reason: &str| {
        Some(
            Failure::new(kind, format!("{reason} (HTTP {status})")).with_response(
                Some(status),
                content_type.map(str::to_owned),
                None,
            ),
        )
    };
    let on_drive = drive::HOSTS.contains(&host);
    let site_file = reads(host) && url.path().starts_with("/file/");
    if host == SIGN_IN_HOST {
        return failure(
            FailureKind::Missing,
            "Google Drive가 파일 대신 로그인을 요구했어요. 파일이 공개돼 있지 않아요",
        );
    }
    if !on_drive && !site_file {
        return None;
    }
    match status {
        // A redirect is followed by the browser; its end is what counts.
        300..=399 => None,
        // Drive answers a file gone with `404` (2026-10-03), and a file that
        // is not shared with a demand to sign in.
        404 if on_drive => failure(FailureKind::Missing, "Google Drive에 파일이 없어요"),
        401 | 403 if on_drive => failure(
            FailureKind::Missing,
            "Google Drive가 로그인을 요구했어요. 파일이 공개돼 있지 않아요",
        ),
        // The address lives 60 seconds; past them, `403` and a page
        // (2026-09-26).
        400 | 403 | 404 | 410 => failure(
            FailureKind::Expired,
            "다운로드 주소가 거절됐어요. 주소의 유효 시간이 지났을 수 있어요",
        ),
        429 | 500..=599 => failure(
            FailureKind::Network,
            "파일을 주는 사이트가 답하지 못했어요. 확인을 다시 거쳐야 해요",
        ),
        200 if html && on_drive => failure(
            FailureKind::NotAFile,
            "Google Drive가 파일 대신 웹 페이지를 줬어요",
        ),
        200 if html => failure(
            FailureKind::NotAFile,
            "사이트가 파일 대신 웹 페이지를 줬어요",
        ),
        200 => None,
        _ => failure(FailureKind::Changed, "다운로드 주소가 뜻밖의 답을 줬어요"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A post as erulabo wrote it on 2026-10-03 (859, 850), cut to what is
    /// read.
    fn page(cards: &[(&str, &str)]) -> String {
        let cards: String = cards
            .iter()
            .map(|(file, title)| {
                format!(
                    r#"<button type="button" class="og-link og-link-button" data-file-url="{file}"><span class="og-preview"><span class="og-media"><img src="https://cdn.erulabo.com/editor/x.webp" alt="{title}"></span><span class="og-body"><span class="og-title">{title}</span><span class="og-description">여기를 클릭하면 보안 확인 후 다운로드가 시작됩니다.<br><span class="og-description-release">SubsPlease 1080p, Pretendard</span></span><span class="og-url">erulabo.com</span></span></span></button>"#
                )
            })
            .collect();
        format!(
            r#"<!doctype html><html><head><title>자막 (12)</title>
<script type="application/ld+json">{{"@context":"https://schema.org","@type":"BlogPosting","datePublished":"2026-09-22T11:20:00+09:00","dateModified":"2026-09-22T11:39:10+09:00"}}</script>
</head><body><div id="post-body" class="fr-view"><p>본문</p>{cards}</div>
<div id="post-config" hidden data-has-file-download="1"></div>
<button data-file-url="/file/outside">본문 밖</button></body></html>"#
        )
    }

    fn opened(page: &str, episode: &str) -> Result<ErulaboCheck, Failure> {
        match read_page(page, episode)? {
            Opened::BrowserAuth {
                reason,
                page: AuthPage::Erulabo(check),
            } => {
                assert_eq!(reason, CHECK_REASON);
                Ok(check)
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_card_of_the_episode_is_chosen_before_the_card_of_the_series() {
        let both = page(&[
            ("/file/aaaa-1", "코코오레 (12)"),
            ("/file/bbbb-2", "코코오레 (1-12)"),
        ]);
        let check = opened(&both, "12").unwrap();
        assert_eq!(
            (check.file(), check.title()),
            ("/file/aaaa-1", "코코오레 (12)")
        );
        assert_eq!(
            check.snapshot().entries(),
            [(
                "dateModified".to_owned(),
                "2026-09-22T11:39:10+09:00".to_owned()
            )]
        );
        // Another episode of the series is in the series card.
        assert_eq!(opened(&both, "05").unwrap().file(), "/file/bbbb-2");
        // The order of the cards does not matter.
        let reversed = page(&[
            ("/file/bbbb-2", "코코오레 (1-12)"),
            ("/file/aaaa-1", "코코오레 (12)"),
        ]);
        assert_eq!(opened(&reversed, "12").unwrap().file(), "/file/aaaa-1");
        // An episode no card holds is the post changed from the candidate.
        let failure = opened(&both, "13").unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
        assert_eq!(failure.reason, "게시물에 13화 받기 카드가 없어요");
    }

    #[test]
    fn the_titles_seen_on_real_posts_name_their_episode() {
        // The season glued to the title is no episode (859, 853).
        assert_eq!(choose_card("1", &["전생귀족3 (1)"]), Ok(0));
        assert_eq!(
            choose_card("2", &["전생귀족3 (1)"]).unwrap_err(),
            "게시물에 2화 받기 카드가 없어요"
        );
        assert_eq!(
            choose_card("12", &["해골기사님2 (12)", "해골기사님2 (1-12)"]),
            Ok(0)
        );
        assert_eq!(choose_card("13", &["헬모드2 (13)"]), Ok(0));
        assert_eq!(
            choose_card("012", &["데가라시 (1-12)", "데가라시 (12)"]),
            Ok(1)
        );
        // A lone card that says no episode is the post's file.
        assert_eq!(choose_card("3", &["자막 받기"]), Ok(0));
        assert!(choose_card("3", &["자막 받기", "폰트"]).is_err());
        assert_eq!(
            choose_card("3", &[]).unwrap_err(),
            "게시물에 받기 카드가 없어요"
        );
    }

    #[test]
    fn a_page_without_its_body_or_cards_has_changed_and_odd_cards_are_not_cards() {
        let failure = opened("<html><body><p>점검 중</p></body></html>", "1").unwrap_err();
        assert_eq!(
            (failure.kind, failure.reason.as_str()),
            (FailureKind::Changed, "게시물에서 본문을 찾지 못했어요")
        );
        let odd = page(&[
            ("/file/a&quot;b", "x (1)"),
            ("/file/a b", "v (1)"),
            ("https://elsewhere.example/file/1", "y (1)"),
            ("/file/", "z (1)"),
            ("/file/ok/token", "w (1)"),
        ]);
        let failure = opened(&odd, "1").unwrap_err();
        assert_eq!(failure.reason, "게시물에 받기 카드가 없어요");
        // The card outside the body is not read either.
        assert!(!page(&[]).is_empty());
    }

    #[test]
    fn only_a_post_address_of_the_site_is_opened() {
        let ok =
            |s: &str| post_address(&Url::parse(s).unwrap(), Reach::NETWORK).map(|u| u.to_string());
        assert_eq!(
            ok("https://erulabo.com/859").unwrap(),
            "https://erulabo.com/859"
        );
        assert_eq!(
            ok("http://www.erulabo.com/859/?x=1#c").unwrap(),
            "https://www.erulabo.com/859"
        );
        for bad in [
            "https://erulabo.com/",
            "https://erulabo.com/file/abc",
            "https://erulabo.com/859/edit",
            "https://erulabo.com.evil.example/859",
            "ftp://erulabo.com/859",
        ] {
            assert_eq!(ok(bad).unwrap_err().kind, FailureKind::Changed, "{bad}");
        }
    }

    #[test]
    fn a_page_where_the_file_should_be_is_classified_and_other_pages_are_not() {
        let at = |s: &str| Url::parse(s).unwrap();
        let drive =
            at("https://drive.usercontent.google.com/download?id=ABCDEFGHIJKLMNOP&export=download");
        let site = at("https://erulabo.com/file/6cd2c4cf/download?signature=S");
        let kind =
            |url: &Url, status, ty: Option<&str>| download_refusal(url, status, ty).map(|f| f.kind);
        // The expired address: `403` and a page (2026-09-26).
        assert_eq!(
            kind(&site, 403, Some("text/html")),
            Some(FailureKind::Expired)
        );
        assert_eq!(
            kind(&drive, 410, Some("text/html")),
            Some(FailureKind::Expired)
        );
        assert_eq!(
            kind(&drive, 400, Some("text/html")),
            Some(FailureKind::Expired)
        );
        assert_eq!(kind(&site, 410, None), Some(FailureKind::Expired));
        assert_eq!(
            kind(&site, 404, Some("text/html")),
            Some(FailureKind::Expired)
        );
        // Drive's own answers are the file gone or not shared: no address
        // is to blame (the table of `docs/specs/jobs.md`).
        for status in [401, 403, 404] {
            assert_eq!(
                kind(&drive, status, Some("text/html")),
                Some(FailureKind::Missing),
                "{status}"
            );
        }
        assert_eq!(
            kind(&drive, 200, Some("text/html")),
            Some(FailureKind::NotAFile)
        );
        assert_eq!(
            kind(&site, 200, Some("text/html")),
            Some(FailureKind::NotAFile)
        );
        // Too many requests and a failing server end the item as well, as a
        // network failure; the person passes the check again.
        for (url, status) in [(&drive, 503), (&drive, 429), (&site, 500), (&site, 429)] {
            assert_eq!(
                kind(url, status, Some("text/html")),
                Some(FailureKind::Network),
                "{status}"
            );
        }
        assert_eq!(
            kind(
                &at("https://accounts.google.com/v3/signin/identifier?continue=x"),
                200,
                Some("text/html")
            ),
            Some(FailureKind::Missing)
        );
        // The file itself, a redirect on the way, and the person's other pages.
        assert_eq!(kind(&drive, 200, Some("application/octet-stream")), None);
        assert_eq!(kind(&site, 302, Some("text/html")), None);
        assert_eq!(
            kind(&at("https://erulabo.com/860"), 404, Some("text/html")),
            None
        );
        assert_eq!(
            kind(&at("https://ads.example/x"), 500, Some("text/html")),
            None
        );
        // The reason says no address.
        let failure = download_refusal(&site, 403, Some("text/html")).unwrap();
        assert!(!failure.reason.contains("signature") && !failure.reason.contains("erulabo.com"));
        assert_eq!(
            (failure.status, failure.content_type.as_deref()),
            (Some(403), Some("text/html"))
        );
    }

    #[test]
    fn the_hosts_a_download_comes_from_are_known_and_others_are_not() {
        for host in [
            "drive.google.com",
            "drive.usercontent.google.com",
            "erulabo.com",
            "www.erulabo.com",
            "accounts.google.com",
        ] {
            assert!(known_host(host), "{host}");
        }
        for host in ["cdn.erulabo.com", "ads.example", "erulabo.com.evil.example"] {
            assert!(!known_host(host), "{host}");
        }
    }

    #[tokio::test]
    async fn nothing_is_fetched_or_read_again_without_the_check() {
        let source = ErulaboSource::new();
        let post = Url::parse("https://erulabo.com/859").unwrap();
        let answers = source
            .recheck(&post, &["browser:erulabo.com/859#a.zip".to_owned()])
            .await;
        assert_eq!(
            answers[0].1.as_ref().unwrap_err().kind,
            FailureKind::Changed
        );
        assert_eq!(source.fetch().kind, FailureKind::Expired);
    }
}
