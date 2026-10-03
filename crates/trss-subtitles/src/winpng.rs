//! WinPNG: subtitles inside a post's PNG images (`docs/specs/subtitles.md`,
//! 출처별 다운로드; `docs/specs/jobs.md`, 출처별 단계 초안).
//!
//! A Tistory skin that carries the WinPNG viewer opens a `.png` image of the
//! post in a panel that takes the files out of it (SMI and ASS files, or a
//! Jamaker project the viewer turns into SMI and ASS) and lists them as links
//! that download from `blob:` or `data:` addresses. This app does not read the
//! image format itself and does not carry the viewer's code: a server browser
//! ([`trss_browser`]) opens the post, lets the viewer's own page script open
//! the image and clicks the links it makes, and the files come as the browser's
//! downloads.
//!
//! # The seam
//!
//! A source that finds such images answers [`crate::Opened::WinPng`]. The job
//! then asks its [`WinpngReader`] to read the post ([`WinpngReader::read`]):
//! the files it took out are in a folder of the job's, under the names and
//! folders the viewer showed ([`Viewed::Files`]); [`offered`] makes them the
//! post's files, which the source then gives back from there
//! ([`crate::PostFile`]'s staged path), so the job receives them through the
//! same steps as any other file. A job with no reader leaves the item waiting
//! for a source ([`NO_READER`]).
//!
//! [`BrowserReader`] is the reader over a [`trss_browser::BrowserPool`]. It
//! starts the job's browser run and drives the page through [`ViewerPage`]; the
//! driving itself ([`drive`]) does not know the browser, so it is tested with a
//! page that has no browser behind it, and the scripts that [`BrowserPage`]
//! gives the page are tested against a real viewer (`tests/winpng_sample.rs`).
//!
//! # Driving the viewer
//!
//! 1. The page is opened and waited for until the viewer's panel and its
//!    scripts are there ([`READY_TIMEOUT`]); a skin without them has
//!    [`FailureKind::Changed`].
//! 2. Each `.png` image of the post's body on the CDN (`*.kakaocdn.net`, the
//!    ones [`crate::tistory`] takes for WinPNG; the address is the image's
//!    `currentSrc`, `src` or `data-src`) is tried. A picture the viewer has
//!    nothing to read in (the viewer's own probe of the image finds no hidden
//!    data in it) is skipped, and so is one the probe cannot fetch or decode:
//!    only a post none of whose images could be read fails for that
//!    ([`FailureKind::Network`]). One that holds something is opened.
//! 3. The viewer is done when it is not busy, lists at least one file, and no
//!    file of it is still being made, twice in a row ([`VIEW_TIMEOUT`]).
//!    A dialog the viewer opens is dismissed: the one that asks for a key (a
//!    `prompt`) means the image needs one ([`Viewed::NeedsKey`]), and one
//!    such image makes the post need input, whatever its other images gave.
//! 4. Each link is clicked in the order of the list and its download is moved
//!    into a folder of its own in the staging folder. A click whose download
//!    does not begin is clicked again ([`CLICK_ATTEMPTS`] times in all): the
//!    browser drops a download now and then when many follow one another. A
//!    download is the link's only if the name the page gave it is the link's:
//!    one that comes late, after its click was given up, is thrown away
//!    instead of being taken for the next link's.
//! 5. A post whose images hold no subtitle is [`Viewed::NoSubtitle`]; one in
//!    which the browser finds no image the source saw is [`Viewed::NoImages`].
//!
//! No address leaves this module but the host of an image's: a post's images
//! may be signed.

use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::Value;
use trss_browser::{
    BrowserError, BrowserPool, BrowserRun, DialogSeen, Dialogs, Download, DownloadState, Page,
};
use url::Url;

use crate::{Failure, FailureKind, PostFile, Snapshot};

/// What an item waits with when the job has no reader (no server browser).
pub const NO_READER: &str =
    "자막이 게시물의 PNG 이미지(WinPNG)에 들어 있어요. 이미지에서 꺼내려면 서버 브라우저가 있어야 해요";

/// How long the page has to show the viewer.
pub const READY_TIMEOUT: Duration = Duration::from_secs(45);
/// How long the viewer has to read an image and make its files.
pub const VIEW_TIMEOUT: Duration = Duration::from_secs(90);
/// How long a click on a link has to begin and finish its download.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(20);
/// How many times a link is clicked before it is given up.
pub const CLICK_ATTEMPTS: u32 = 4;
/// The most files one post may give: far more than any post has.
pub const MAX_FILES: usize = 500;

/// The snapshot name of the image a file came out of (its host and path).
pub const IMAGE: &str = "winpng_image";

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a reader is asked to read.
#[derive(Debug, Clone, Copy)]
pub struct ViewRequest<'a> {
    /// The job the browser run belongs to.
    pub job: &'a str,
    /// The post whose images are to be opened.
    pub post: &'a Url,
    /// An empty folder of the job's, where the files are put (the reader
    /// makes the folders inside it).
    pub staging: &'a Path,
}

/// A file taken out of an image, now on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    /// The image it came out of: its host and path, without the query.
    pub image: String,
    /// The folders the viewer showed it in (`회차/2화`), if any.
    pub folder: Option<String>,
    /// The name the viewer gave it.
    pub name: String,
    /// Where it is now.
    pub path: PathBuf,
}

/// What reading a post's images came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Viewed {
    /// The files the viewer made, in its order.
    Files(Vec<Staged>),
    /// The post has no image with a subtitle in it (`자막 없음`).
    NoSubtitle,
    /// The browser found none of the images the source saw in the post's page
    /// (they may load late): nothing is known of the subtitle.
    NoImages,
    /// An image needs a key the app does not have (`추가 입력 필요`).
    NeedsKey,
}

/// Reads the WinPNG images of a post. The one a job has is chosen by the
/// worker: the browser pool's, or none.
pub trait WinpngReader: Send + Sync {
    /// Opens the post and takes the files out of its images. A reading that
    /// fails for the browser's or the site's trouble is
    /// [`FailureKind::Network`].
    fn read<'a>(&'a self, request: ViewRequest<'a>) -> BoxFuture<'a, Result<Viewed, Failure>>;

    /// The job needs no more of the reader (it ended or failed): the browser
    /// run it kept goes. A job that waits for a person's check keeps it.
    fn release<'a>(&'a self, job: &'a str) -> BoxFuture<'a, ()>;
}

/// The post's files that [`Viewed::Files`] made: a key that names each by its
/// image and its place in it, and the snapshot of the post and the image.
pub fn offered(post: &Snapshot, staged: &[Staged]) -> Vec<PostFile> {
    let mut files: Vec<PostFile> = Vec::new();
    for file in staged {
        let folder = file.folder.as_deref().and_then(clean_folder);
        let place = match &folder {
            Some(folder) => format!("{folder}/{}", file.name),
            None => file.name.clone(),
        };
        let key = format!("winpng:{}#{place}", file.image);
        // A name the image holds twice is one file; the viewer lists it once
        // for each link it makes of it.
        if files.iter().any(|f| f.key == key) {
            continue;
        }
        let mut made = PostFile::new(key, file.name.clone());
        made.folder = folder;
        made.snapshot.extend(post);
        made.snapshot.push(IMAGE, file.image.clone());
        files.push(made.with_staged(file.path.clone()));
    }
    files
}

/// The folders of a viewer's path, without empty parts or any that walks out:
/// `회차/2화/` as `회차/2화`; `None` for none.
fn clean_folder(folder: &str) -> Option<String> {
    let parts: Vec<&str> = folder
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|p| !p.is_empty() && *p != "." && *p != "..")
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

// --- Driving the viewer ---

/// What the viewer's own probe says of an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// Nothing hidden in it: a picture.
    Plain,
    /// Something is hidden in it that the viewer can read, with a key or not.
    Winpng,
    /// The probe could not fetch or decode it (another host's refusal, a
    /// broken file): it is not read, and nothing is known of it.
    Failed,
}

/// A file the viewer lists, as it shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewerLink {
    /// The folders it shows before the name (`회차/2화/`).
    pub folder: String,
    /// The name it downloads as.
    pub name: String,
}

/// What the viewer's panel shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewerState {
    /// Reading an image.
    pub working: bool,
    /// How many of the files it lists are still being made.
    pub processing: usize,
    /// The files it lists that can be downloaded now, of the image opened last.
    pub links: Vec<ViewerLink>,
}

/// A download the page's click began and finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    pub path: PathBuf,
}

/// The page of a post with the viewer in it. [`BrowserPage`] drives a real one.
pub trait ViewerPage: Send + Sync {
    /// Waits until the viewer is there. A page that never shows it has
    /// [`FailureKind::Changed`].
    fn wait_viewer(&self) -> impl Future<Output = Result<(), Failure>> + Send;
    /// The images of the post the viewer opens, in order: each one's host and
    /// path.
    fn images(&self) -> impl Future<Output = Result<Vec<String>, Failure>> + Send;
    /// Asks the viewer whether image `index` holds anything.
    fn probe(&self, index: usize) -> impl Future<Output = Result<Probe, Failure>> + Send;
    /// Opens image `index` in the viewer. What was listed before it is not the
    /// new image's, whatever the viewer still shows of it.
    fn open(&self, index: usize) -> impl Future<Output = Result<(), Failure>> + Send;
    /// What the panel shows now.
    fn state(&self) -> impl Future<Output = Result<ViewerState, Failure>> + Send;
    /// How many key prompts (`prompt` dialogs) the page has opened, and had
    /// dismissed, since the start. Other dialogs are dismissed and not
    /// counted.
    fn prompts(&self) -> usize;
    /// Clicks link `index` of [`ViewerState::links`].
    fn click(&self, index: usize) -> impl Future<Output = Result<(), Failure>> + Send;
    /// The next download that has ended, waiting up to `wait` for one. `None`:
    /// none came in time. Which click it came of is not known, only the name
    /// the page gave it.
    fn next_download(
        &self,
        wait: Duration,
    ) -> impl Future<Output = Result<Option<Arrived>, Failure>> + Send;
    /// Puts a download that is the link's in `dir`, under the name the viewer
    /// gave it.
    fn keep(
        &self,
        arrived: &Arrived,
        dir: &Path,
    ) -> impl Future<Output = Result<Taken, Failure>> + Send;
    /// Throws a download away (`dir` is where there is room to put it down).
    fn discard(
        &self,
        arrived: &Arrived,
        dir: &Path,
    ) -> impl Future<Output = Result<(), Failure>> + Send;
}

/// A download that ended, before it is kept or thrown away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrived {
    /// The name the page gave it (the link's `download`).
    pub name: String,
    /// Whether it ended whole.
    pub completed: bool,
    /// What the page needs to find it again.
    pub token: String,
}

/// Whether a download named `got` is the one a link named `link` makes: the
/// browser may have turned characters a file cannot have in a name into `_`.
fn same_name(link: &str, got: &str) -> bool {
    let fold = |name: &str| -> String {
        name.trim()
            .chars()
            .map(|c| {
                if c.is_control() || "/\\:*?\"<>|".contains(c) {
                    '_'
                } else {
                    c
                }
            })
            .collect()
    };
    fold(link) == fold(got)
}

/// Takes what has already ended off `next` without waiting for more, until it
/// gives nothing at once. A source that has ended gives its error at once, and
/// that is the end of the draining, not a reason to go on.
async fn drain<T, E, F>(mut next: impl FnMut() -> F) -> Result<usize, E>
where
    F: Future<Output = Result<T, E>>,
{
    let mut drained = 0;
    while let Ok(item) = tokio::time::timeout(Duration::ZERO, next()).await {
        item?;
        drained += 1;
    }
    Ok(drained)
}

/// The waits of [`drive`]; [`Timing::default`] is what a real viewer gets.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub poll: Duration,
    pub view: Duration,
    pub download: Duration,
    pub attempts: u32,
}

impl Default for Timing {
    fn default() -> Timing {
        Timing {
            poll: Duration::from_millis(500),
            view: VIEW_TIMEOUT,
            download: DOWNLOAD_TIMEOUT,
            attempts: CLICK_ATTEMPTS,
        }
    }
}

enum Waited {
    Links(Vec<ViewerLink>),
    Prompted,
}

/// Drives the viewer of `page` over the post's images (see the module docs)
/// and puts what it makes in `staging`.
pub async fn drive<P: ViewerPage>(
    page: &P,
    staging: &Path,
    timing: Timing,
) -> Result<Viewed, Failure> {
    page.wait_viewer().await?;
    let images = page.images().await?;
    let mut staged: Vec<Staged> = Vec::new();
    let (mut readable, mut unread, mut needs_key, mut sequence) = (0usize, 0usize, false, 0usize);
    let mut seen: Vec<&String> = Vec::new();
    for (index, image) in images.iter().enumerate() {
        // An image the post shows twice is read once.
        if seen.contains(&image) {
            continue;
        }
        seen.push(image);
        match page.probe(index).await? {
            Probe::Plain => continue,
            // One image the probe cannot read does not end the post's other
            // images.
            Probe::Failed => {
                unread += 1;
                continue;
            }
            Probe::Winpng => {}
        }
        readable += 1;
        let prompts = page.prompts();
        page.open(index).await?;
        let links = match wait_for_files(page, prompts, timing).await? {
            Waited::Links(links) => links,
            Waited::Prompted => {
                needs_key = true;
                continue;
            }
        };
        for (position, link) in links.into_iter().enumerate() {
            if staged.len() >= MAX_FILES {
                return Err(Failure::new(
                    FailureKind::NotAFile,
                    format!("이미지에서 나온 파일이 {MAX_FILES}개를 넘어 멈췄어요"),
                ));
            }
            sequence += 1;
            let dir = staging.join(sequence.to_string());
            let taken = download(page, position, &link, &dir, timing).await?;
            staged.push(Staged {
                image: image.clone(),
                folder: clean_folder(&link.folder),
                name: link.name,
                path: taken.path,
            });
        }
    }
    if needs_key {
        return Ok(Viewed::NeedsKey);
    }
    if readable == 0 {
        if images.is_empty() {
            return Ok(Viewed::NoImages);
        }
        // Nothing was read, and some could not be: that no image holds a
        // subtitle is not known.
        if unread > 0 {
            return Err(Failure::new(
                FailureKind::Network,
                "게시물의 이미지를 읽지 못해서 자막이 있는지 알 수 없어요",
            ));
        }
        return Ok(Viewed::NoSubtitle);
    }
    Ok(Viewed::Files(staged))
}

/// Waits until the viewer has made the files of the image just opened, or
/// has asked for a key.
async fn wait_for_files<P: ViewerPage>(
    page: &P,
    prompts_before: usize,
    timing: Timing,
) -> Result<Waited, Failure> {
    let deadline = tokio::time::Instant::now() + timing.view;
    let mut last: Option<Vec<ViewerLink>> = None;
    loop {
        if page.prompts() > prompts_before {
            return Ok(Waited::Prompted);
        }
        let state = page.state().await?;
        let ready = !state.working && state.processing == 0 && !state.links.is_empty();
        if ready {
            // The same list twice: a list that is still growing is not done.
            if last.as_ref() == Some(&state.links) {
                return Ok(Waited::Links(state.links));
            }
            last = Some(state.links);
        } else {
            last = None;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Failure::new(
                FailureKind::Network,
                "WinPNG 뷰어가 시간 안에 이미지를 다 읽지 못했어요",
            ));
        }
        tokio::time::sleep(timing.poll).await;
    }
}

/// Clicks link `index` (`link` as the viewer lists it) until its download
/// comes.
async fn download<P: ViewerPage>(
    page: &P,
    index: usize,
    link: &ViewerLink,
    dir: &Path,
    timing: Timing,
) -> Result<Taken, Failure> {
    for _ in 0..timing.attempts {
        page.click(index).await?;
        let deadline = tokio::time::Instant::now() + timing.download;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let Some(arrived) = page.next_download(left).await? else {
                break;
            };
            // A download that is not this link's came late from a click that
            // was given up: it is not this link's bytes.
            if !same_name(&link.name, &arrived.name) {
                page.discard(&arrived, dir).await?;
                continue;
            }
            if !arrived.completed {
                return Err(Failure::new(
                    FailureKind::NotAFile,
                    "뷰어가 만든 파일의 다운로드가 취소됐어요",
                ));
            }
            return page.keep(&arrived, dir).await;
        }
    }
    Err(Failure::new(
        FailureKind::Network,
        format!(
            "WinPNG 뷰어의 파일 {}의 다운로드가 시작되지 않았어요",
            link.name
        ),
    ))
}

// --- The browser's page ---

/// The images of the post's body that the viewer opens on a click: the ones
/// whose address, without its query, ends in `.png`. The `__` words are
/// replaced before a script is sent.
const IMAGES_JS: &str = r#"(() => {
  // The address of an image the source takes for WinPNG: on the CDN, ending in
  // `.png`, from where the browser loaded it or where a lazy loader keeps it.
  const srcOf = i => {
    for (const s of [i.currentSrc, i.src, i.getAttribute('data-src')]) {
      if (!s || s.startsWith('data:')) continue;
      let u; try { u = new URL(s, document.baseURI); } catch (e) { continue; }
      if (u.hostname.endsWith('.kakaocdn.net') && u.pathname.toLowerCase().endsWith('.png')) return u.href;
    }
    return null;
  };
  const bodies = [...document.querySelectorAll('.tt_article_useless_p_margin, .contents_style, #article-view')];
  const images = [];
  for (const body of bodies) for (const el of body.querySelectorAll('img')) {
    if (images.some(x => x.el === el)) continue;
    const href = srcOf(el);
    if (href) images.push({el, href});
  }
  __BODY__
})()"#;

const READY_JS: &str = "document.readyState === 'complete' \
    && !!document.getElementById('viewFileList') && !!document.getElementById('winPNG') \
    && typeof window.downloadZip === 'function' && typeof WithTarget !== 'undefined' \
    && typeof BufferedImage !== 'undefined'";

/// The `__BODY__` of [`IMAGES_JS`] that lists the images' hosts and paths.
const LIST_BODY: &str = "return JSON.stringify(images.map(x => { const u = new URL(x.href); return u.host + u.pathname; }));";

/// ... that asks the viewer's probe about image `__INDEX__`. The viewer's own
/// code reads the image: 0 is a picture, more is something hidden in it.
const PROBE_BODY: &str = r#"const x = images[__INDEX__]; if (!x) return Promise.resolve('gone');
  return (async () => {
    try {
      const res = await fetch(x.href);
      if (!res.ok) return 'error';
      const url = URL.createObjectURL(await res.blob());
      try {
        const el = new Image();
        await new Promise((ok, no) => { el.onload = ok; el.onerror = no; el.src = url; });
        return String(WithTarget.possibility(new BufferedImage(el)));
      } finally { URL.revokeObjectURL(url); }
    } catch (e) { return 'error'; }
  })();"#;

/// ... that marks what the viewer lists now as the previous image's, then
/// clicks image `__INDEX__` as a reader does.
const OPEN_BODY: &str = r#"const x = images[__INDEX__]; if (!x) return 'gone';
  const i = x.el;
  // A lazy loader may not have put the address where the viewer reads it.
  if (!i.src.split('?')[0].toLowerCase().endsWith('.png')) i.src = x.href;
  for (const a of document.querySelectorAll('#viewFileList a')) a.setAttribute('data-trss-old', '');
  i.scrollIntoView({block: 'center'});
  i.click();
  return 'clicked';"#;

/// The files the viewer lists and can download now, of the image opened last.
const LINKS_JS: &str = "const links = () => [...document.querySelectorAll('#viewFileList a[href]')].filter(a => !a.hasAttribute('data-trss-old') && /^(blob:|data:)/.test(a.getAttribute('href') || ''));";

const STATE_JS: &str = r#"(() => {
  __LINKS__
  const list = document.getElementById('viewFileList');
  return JSON.stringify({
    working: document.getElementById('winPNG').classList.contains('progress'),
    processing: list.querySelectorAll('a.processing:not([data-trss-old])').length,
    links: links().map(a => { const top = a.closest('#viewFileList > a'); const dir = top && top.children[0] ? top.children[0].innerText : ''; return {folder: dir || '', name: a.getAttribute('download') || ''}; }),
  });
})()"#;

/// A point on link `__INDEX__` that a click on it reaches (not on a link
/// inside it), or a click made by the script when there is none.
const LOCATE_JS: &str = r#"(() => {
  __LINKS__
  const el = links()[__INDEX__]; if (!el) return null;
  el.scrollIntoView({block: 'center'});
  for (const r of el.getClientRects()) {
    if (r.width < 2 || r.height < 2) continue;
    for (const f of [0.5, 0.25, 0.75, 0.1, 0.9]) {
      const x = r.left + r.width * f, y = r.top + r.height / 2;
      const hit = document.elementFromPoint(x, y);
      if (hit && hit.closest('a[href]') === el) return JSON.stringify({x, y});
    }
  }
  el.click();
  return 'clicked';
})()"#;

fn images_script(body: &str, index: usize) -> String {
    IMAGES_JS
        .replace("__BODY__", body)
        .replace("__INDEX__", &index.to_string())
}

fn links_script(script: &str, index: usize) -> String {
    script
        .replace("__LINKS__", LINKS_JS)
        .replace("__INDEX__", &index.to_string())
}

/// The page of a run, with the dialogs it opens dismissed.
pub struct BrowserPage {
    run: BrowserRun,
    page: Page,
    dialogs: Dialogs,
    ready_timeout: Duration,
    /// A script run once the viewer is there, before the images are looked
    /// at (the tests of a real viewer change the page with it).
    prepare: Option<Arc<str>>,
    /// The downloads [`ViewerPage::next_download`] gave and nobody has kept or
    /// thrown away yet, by [`Arrived::token`].
    arrived: Mutex<HashMap<String, Download>>,
}

/// A failure of the browser, in words that are the same whatever the browser
/// said: its own text can hold the page's addresses and exception texts.
/// Only the kind of the error is logged.
fn browser_failure(err: BrowserError) -> Failure {
    let (kind, reason) = match &err {
        BrowserError::RunEnded { .. } => ("run ended", "서버 브라우저의 실행이 끝났어요"),
        BrowserError::Launcher(_) => ("launcher", "서버 브라우저를 시작하지 못했어요"),
        BrowserError::Timeout(_) => ("timeout", "서버 브라우저가 시간 안에 답하지 않았어요"),
        BrowserError::PageGone(_) => ("page gone", "서버 브라우저의 페이지가 사라졌어요"),
        BrowserError::Cdp(_) => ("cdp", "서버 브라우저를 쓰지 못했어요"),
        _ => ("other", "서버 브라우저를 쓰지 못했어요"),
    };
    eprintln!("trss-subtitles: the WinPNG reader's browser failed: {kind}");
    Failure::new(FailureKind::Network, reason)
}

impl BrowserPage {
    async fn text(&self, script: &str) -> Result<String, Failure> {
        match self.page.evaluate(script).await.map_err(browser_failure)? {
            Value::String(text) => Ok(text),
            other => Err(Failure::new(
                FailureKind::Changed,
                format!("WinPNG 뷰어가 뜻밖의 답을 했어요: {}", kind_of(&other)),
            )),
        }
    }
}

fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

impl ViewerPage for BrowserPage {
    async fn wait_viewer(&self) -> Result<(), Failure> {
        let deadline = tokio::time::Instant::now() + self.ready_timeout;
        loop {
            if self
                .page
                .evaluate(READY_JS)
                .await
                .map_err(browser_failure)?
                == Value::Bool(true)
            {
                if let Some(script) = &self.prepare {
                    self.page.evaluate(script).await.map_err(browser_failure)?;
                }
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Failure::new(
                    FailureKind::Changed,
                    "게시물에서 WinPNG 뷰어를 찾지 못했어요",
                ));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn images(&self) -> Result<Vec<String>, Failure> {
        let text = self.text(&images_script(LIST_BODY, 0)).await?;
        serde_json::from_str(&text)
            .map_err(|_| Failure::new(FailureKind::Changed, "게시물의 이미지 목록을 읽지 못했어요"))
    }

    async fn probe(&self, index: usize) -> Result<Probe, Failure> {
        let answer = self.text(&images_script(PROBE_BODY, index)).await?;
        match answer.as_str() {
            "gone" => Err(Failure::new(
                FailureKind::Changed,
                "게시물의 이미지가 사라졌어요",
            )),
            // The viewer's script threw or the image could not be fetched or
            // decoded (another host's refusal, a broken file).
            "error" => Ok(Probe::Failed),
            text => match text.parse::<f64>() {
                Ok(p) if p > 0.0 => Ok(Probe::Winpng),
                Ok(_) => Ok(Probe::Plain),
                Err(_) => Err(Failure::new(
                    FailureKind::Changed,
                    "WinPNG 뷰어의 이미지 검사가 뜻밖의 답을 했어요",
                )),
            },
        }
    }

    async fn open(&self, index: usize) -> Result<(), Failure> {
        match self.text(&images_script(OPEN_BODY, index)).await?.as_str() {
            "clicked" => Ok(()),
            _ => Err(Failure::new(
                FailureKind::Changed,
                "게시물의 이미지가 사라졌어요",
            )),
        }
    }

    async fn state(&self) -> Result<ViewerState, Failure> {
        let text = self.text(&links_script(STATE_JS, 0)).await?;
        let bad = || Failure::new(FailureKind::Changed, "WinPNG 뷰어의 목록을 읽지 못했어요");
        let value: Value = serde_json::from_str(&text).map_err(|_| bad())?;
        let links = value["links"]
            .as_array()
            .ok_or_else(bad)?
            .iter()
            .map(|l| {
                Some(ViewerLink {
                    folder: l["folder"].as_str()?.to_owned(),
                    name: l["name"].as_str()?.to_owned(),
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(bad)?;
        Ok(ViewerState {
            working: value["working"].as_bool().ok_or_else(bad)?,
            processing: value["processing"].as_u64().ok_or_else(bad)? as usize,
            links,
        })
    }

    fn prompts(&self) -> usize {
        prompt_count(&self.dialogs.seen())
    }

    async fn click(&self, index: usize) -> Result<(), Failure> {
        let located = self.text(&links_script(LOCATE_JS, index)).await;
        let located = match located {
            Ok(text) => text,
            // `null` (the link is gone) is not a string.
            Err(_) => {
                return Err(Failure::new(
                    FailureKind::Changed,
                    "WinPNG 뷰어의 링크가 사라졌어요",
                ))
            }
        };
        if located == "clicked" {
            return Ok(());
        }
        let point: Value = serde_json::from_str(&located).map_err(|_| {
            Failure::new(
                FailureKind::Changed,
                "WinPNG 뷰어의 링크 자리를 읽지 못했어요",
            )
        })?;
        for (kind, button, count) in [
            ("mouseMoved", "none", 0),
            ("mousePressed", "left", 1),
            ("mouseReleased", "left", 1),
        ] {
            self.page
                .send(
                    "Input.dispatchMouseEvent",
                    serde_json::json!({
                        "type": kind, "x": point["x"], "y": point["y"],
                        "button": button, "clickCount": count,
                    }),
                )
                .await
                .map_err(browser_failure)?;
        }
        Ok(())
    }

    async fn next_download(&self, wait: Duration) -> Result<Option<Arrived>, Failure> {
        let download = match tokio::time::timeout(wait, self.run.download_finished()).await {
            Err(_) => return Ok(None),
            Ok(download) => download.map_err(browser_failure)?,
        };
        let arrived = Arrived {
            name: download.file_name.clone(),
            completed: download.state == DownloadState::Completed,
            token: download.guid.clone(),
        };
        self.arrived
            .lock()
            .expect("arrived lock")
            .insert(arrived.token.clone(), download);
        Ok(Some(arrived))
    }

    async fn keep(&self, arrived: &Arrived, dir: &Path) -> Result<Taken, Failure> {
        let download = self.owned(arrived)?;
        let moved = self
            .run
            .move_download(&download, dir)
            .await
            .map_err(browser_failure)?;
        Ok(Taken { path: moved.path })
    }

    async fn discard(&self, arrived: &Arrived, dir: &Path) -> Result<(), Failure> {
        let download = self.owned(arrived)?;
        if download.state != DownloadState::Completed {
            return Ok(());
        }
        // Moved out of the browser's folder, then removed with its folder.
        let scratch = dir.join(".discarded");
        self.run
            .move_download(&download, &scratch)
            .await
            .map_err(browser_failure)?;
        let _ = tokio::fs::remove_dir_all(&scratch).await;
        Ok(())
    }
}

/// How many of the dialogs are key prompts: the viewer asks for a key with a
/// `prompt`; an `alert` or a `confirm` of the page is not that.
fn prompt_count(seen: &[DialogSeen]) -> usize {
    seen.iter().filter(|d| d.kind == "prompt").count()
}

impl BrowserPage {
    fn owned(&self, arrived: &Arrived) -> Result<Download, Failure> {
        self.arrived
            .lock()
            .expect("arrived lock")
            .remove(&arrived.token)
            .ok_or_else(|| Failure::new(FailureKind::Changed, "받은 파일의 기록을 찾지 못했어요"))
    }
}

/// The reader over a server browser pool.
#[derive(Clone)]
pub struct BrowserReader {
    pool: BrowserPool,
    prepare: Option<Arc<str>>,
}

impl BrowserReader {
    pub fn new(pool: BrowserPool) -> BrowserReader {
        BrowserReader {
            pool,
            prepare: None,
        }
    }

    /// The same reader running `script` in each page once its viewer is there,
    /// before the images are looked at: for the tests of a real viewer, which
    /// change what the page fetches or what its viewer makes of an image.
    /// Only with the `test-hooks` feature.
    #[cfg(feature = "test-hooks")]
    pub fn with_script_after_ready(mut self, script: &str) -> BrowserReader {
        self.prepare = Some(script.into());
        self
    }

    /// The reader as a job takes it.
    pub fn shared(pool: BrowserPool) -> Arc<dyn WinpngReader> {
        Arc::new(BrowserReader::new(pool))
    }

    async fn view(&self, request: ViewRequest<'_>) -> Result<Viewed, Failure> {
        let run = self
            .pool
            .start(request.job)
            .await
            .map_err(browser_failure)?;
        // The run is not ended for being idle while the viewer is read.
        let _busy = run.busy_guard().map_err(browser_failure)?;
        // What an earlier reading left unclaimed is not this one's. A run that
        // has ended is a failure, not something to drain.
        drain(|| run.download_finished())
            .await
            .map_err(browser_failure)?;
        // The page is made blank, the dialogs are dismissed, and only then is
        // the post opened, so a dialog the page opens as it loads is answered.
        let page = run.new_page("about:blank").await.map_err(browser_failure)?;
        let viewed = self.drive_page(&run, &page, request).await;
        let _ = page.close().await;
        viewed
    }

    async fn drive_page(
        &self,
        run: &BrowserRun,
        page: &Page,
        request: ViewRequest<'_>,
    ) -> Result<Viewed, Failure> {
        let dialogs = page.dismiss_dialogs().await.map_err(browser_failure)?;
        page.navigate(request.post.as_str())
            .await
            .map_err(browser_failure)?;
        let browser_page = BrowserPage {
            run: run.clone(),
            page: page.clone(),
            dialogs,
            ready_timeout: READY_TIMEOUT,
            prepare: self.prepare.clone(),
            arrived: Mutex::default(),
        };
        drive(&browser_page, request.staging, Timing::default()).await
    }
}

impl WinpngReader for BrowserReader {
    fn read<'a>(&'a self, request: ViewRequest<'a>) -> BoxFuture<'a, Result<Viewed, Failure>> {
        Box::pin(self.view(request))
    }

    fn release<'a>(&'a self, job: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move { self.pool.end_job(job).await })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn link(folder: &str, name: &str) -> ViewerLink {
        ViewerLink {
            folder: folder.to_owned(),
            name: name.to_owned(),
        }
    }

    /// One image of a scripted post.
    struct Image {
        id: &'static str,
        probe: Probe,
        /// The files the viewer makes of it, with their bytes.
        links: Vec<(ViewerLink, &'static str)>,
        /// The viewer asks for a key (a dialog) instead.
        prompts: bool,
    }

    /// A page with no browser: the viewer's panel as a state machine.
    struct FakePage {
        images: Vec<Image>,
        /// The first click on this link (by name) is dropped, as the browser
        /// drops the eleventh download.
        drop_first_click: Option<&'static str>,
        /// The first click on this link (by name) is answered late: its
        /// download comes after the next click, as one that took longer than
        /// the wait would.
        late_first_click: Option<&'static str>,
        /// A viewer that never finishes.
        never_done: bool,
        /// No viewer in the page.
        no_viewer: bool,
        state: Mutex<Fake>,
    }

    #[derive(Default)]
    struct Fake {
        opened: Option<usize>,
        polls_since_open: u32,
        prompts: usize,
        /// Downloads that ended, by token: (name, bytes); in the order they
        /// ended.
        finished: std::collections::VecDeque<(String, &'static str)>,
        /// The late download, until the next click.
        held: Option<(String, &'static str)>,
        dropped: Vec<String>,
        late: Vec<String>,
        clicks: Vec<String>,
        opens: Vec<usize>,
        discarded: Vec<String>,
    }

    impl FakePage {
        fn new(images: Vec<Image>) -> FakePage {
            FakePage {
                images,
                drop_first_click: None,
                late_first_click: None,
                never_done: false,
                no_viewer: false,
                state: Mutex::default(),
            }
        }
    }

    impl ViewerPage for FakePage {
        async fn wait_viewer(&self) -> Result<(), Failure> {
            match self.no_viewer {
                true => Err(Failure::new(FailureKind::Changed, "no viewer")),
                false => Ok(()),
            }
        }

        async fn images(&self) -> Result<Vec<String>, Failure> {
            Ok(self.images.iter().map(|i| i.id.to_owned()).collect())
        }

        async fn probe(&self, index: usize) -> Result<Probe, Failure> {
            Ok(self.images[index].probe)
        }

        async fn open(&self, index: usize) -> Result<(), Failure> {
            let mut s = self.state.lock().unwrap();
            s.opens.push(index);
            s.opened = Some(index);
            s.polls_since_open = 0;
            if self.images[index].prompts {
                s.prompts += 1;
            }
            Ok(())
        }

        async fn state(&self) -> Result<ViewerState, Failure> {
            let mut s = self.state.lock().unwrap();
            s.polls_since_open += 1;
            let Some(index) = s.opened else {
                return Ok(ViewerState::default());
            };
            let image = &self.images[index];
            // Busy at the first look, a file still made at the second, done
            // from the third on.
            Ok(match s.polls_since_open {
                _ if self.never_done || image.prompts => ViewerState {
                    working: true,
                    ..ViewerState::default()
                },
                1 => ViewerState {
                    working: true,
                    ..ViewerState::default()
                },
                2 => ViewerState {
                    processing: 1,
                    links: vec![image.links[0].0.clone()],
                    ..ViewerState::default()
                },
                _ => ViewerState {
                    links: image.links.iter().map(|(l, _)| l.clone()).collect(),
                    ..ViewerState::default()
                },
            })
        }

        fn prompts(&self) -> usize {
            self.state.lock().unwrap().prompts
        }

        async fn click(&self, index: usize) -> Result<(), Failure> {
            let mut s = self.state.lock().unwrap();
            let image = &self.images[s.opened.expect("an image is open")];
            let (link, bytes) = &image.links[index];
            s.clicks.push(link.name.clone());
            // What was held back by an earlier click ends now.
            if let Some(held) = s.held.take() {
                s.finished.push_back(held);
            }
            let name = link.name.clone();
            if self.drop_first_click == Some(name.as_str()) && !s.dropped.contains(&name) {
                s.dropped.push(name);
            } else if self.late_first_click == Some(name.as_str()) && !s.late.contains(&name) {
                s.late.push(name.clone());
                s.held = Some((name, *bytes));
            } else {
                s.finished.push_back((name, *bytes));
            }
            Ok(())
        }

        async fn next_download(&self, _wait: Duration) -> Result<Option<Arrived>, Failure> {
            let s = self.state.lock().unwrap();
            Ok(s.finished.front().map(|(name, _)| Arrived {
                name: name.clone(),
                completed: true,
                token: name.clone(),
            }))
        }

        async fn keep(&self, arrived: &Arrived, dir: &Path) -> Result<Taken, Failure> {
            let mut s = self.state.lock().unwrap();
            let (name, bytes) = s.finished.pop_front().unwrap();
            assert_eq!(name, arrived.token);
            std::fs::create_dir_all(dir).unwrap();
            let path = dir.join(&name);
            std::fs::write(&path, bytes).unwrap();
            Ok(Taken { path })
        }

        async fn discard(&self, arrived: &Arrived, _dir: &Path) -> Result<(), Failure> {
            let mut s = self.state.lock().unwrap();
            let (name, _) = s.finished.pop_front().unwrap();
            assert_eq!(name, arrived.token);
            s.discarded.push(name);
            Ok(())
        }
    }

    fn quick() -> Timing {
        Timing {
            poll: Duration::from_millis(1),
            view: Duration::from_millis(300),
            download: Duration::from_millis(1),
            attempts: 3,
        }
    }

    fn winpng(id: &'static str, links: Vec<(ViewerLink, &'static str)>) -> Image {
        Image {
            id,
            probe: Probe::Winpng,
            links,
            prompts: false,
        }
    }

    #[tokio::test]
    async fn the_files_of_an_image_are_taken_in_the_viewers_order_with_their_folders() {
        let dir = tempfile::tempdir().unwrap();
        let page = FakePage::new(vec![winpng(
            "blog.kakaocdn.net/dna/a/img.png",
            vec![
                (link("", "readme.smi"), "<SAMI>top"),
                (link("회차/", "01.smi"), "<SAMI>one"),
                (link("회차/2기/", "02.smi"), "<SAMI>two"),
            ],
        )]);
        let Viewed::Files(files) = drive(&page, dir.path(), quick()).await.unwrap() else {
            panic!("files");
        };
        let shown: Vec<(Option<&str>, &str)> = files
            .iter()
            .map(|f| (f.folder.as_deref(), f.name.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                (None, "readme.smi"),
                (Some("회차"), "01.smi"),
                (Some("회차/2기"), "02.smi")
            ]
        );
        assert_eq!(
            std::fs::read_to_string(&files[2].path).unwrap(),
            "<SAMI>two"
        );
        // Each file is in a folder of its own, so equal names cannot meet.
        assert!(files.iter().all(|f| f.path.starts_with(dir.path())));
        let paths: std::collections::HashSet<_> = files.iter().map(|f| &f.path).collect();
        assert_eq!(paths.len(), 3);
    }

    #[tokio::test]
    async fn a_click_whose_download_does_not_begin_is_clicked_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut page = FakePage::new(vec![winpng(
            "h/x.png",
            vec![
                (link("", "a.smi"), "<SAMI>a"),
                (link("", "b.smi"), "<SAMI>b"),
            ],
        )]);
        page.drop_first_click = Some("b.smi");
        let Viewed::Files(files) = drive(&page, dir.path(), quick()).await.unwrap() else {
            panic!("files");
        };
        assert_eq!(files.len(), 2);
        let clicks = page.state.lock().unwrap().clicks.clone();
        assert_eq!(clicks, ["a.smi", "b.smi", "b.smi"]);
    }

    #[tokio::test]
    async fn a_link_that_never_downloads_fails_the_reading_as_a_network_failure() {
        let dir = tempfile::tempdir().unwrap();
        let mut page = FakePage::new(vec![winpng("h/x.png", vec![(link("", "a.smi"), "x")])]);
        page.drop_first_click = Some("a.smi");
        let timing = Timing {
            attempts: 1,
            ..quick()
        };
        let failure = drive(&page, dir.path(), timing).await.unwrap_err();
        assert_eq!(failure.kind, FailureKind::Network);
        assert!(failure.reason.contains("a.smi"));
    }

    #[tokio::test]
    async fn pictures_are_skipped_and_a_post_of_nothing_else_has_no_subtitle() {
        let dir = tempfile::tempdir().unwrap();
        let plain = |id| Image {
            id,
            probe: Probe::Plain,
            links: Vec::new(),
            prompts: false,
        };
        let page = FakePage::new(vec![plain("h/a.png"), plain("h/b.png")]);
        assert_eq!(
            drive(&page, dir.path(), quick()).await.unwrap(),
            Viewed::NoSubtitle
        );
        assert!(page.state.lock().unwrap().opens.is_empty());

        // A picture beside a WinPNG image is passed over.
        let page = FakePage::new(vec![
            plain("h/a.png"),
            winpng("h/b.png", vec![(link("", "1.smi"), "<SAMI>")]),
        ]);
        let Viewed::Files(files) = drive(&page, dir.path(), quick()).await.unwrap() else {
            panic!("files");
        };
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].image, "h/b.png");
        assert_eq!(page.state.lock().unwrap().opens, [1]);
    }

    #[tokio::test]
    async fn an_image_that_asks_for_a_key_needs_input_whatever_else_the_post_has() {
        let dir = tempfile::tempdir().unwrap();
        let page = FakePage::new(vec![
            winpng("h/a.png", vec![(link("", "1.smi"), "<SAMI>")]),
            Image {
                id: "h/b.png",
                probe: Probe::Winpng,
                links: Vec::new(),
                prompts: true,
            },
        ]);
        assert_eq!(
            drive(&page, dir.path(), quick()).await.unwrap(),
            Viewed::NeedsKey
        );
    }

    #[tokio::test]
    async fn a_viewer_that_never_finishes_and_a_page_without_one_fail() {
        let dir = tempfile::tempdir().unwrap();
        let mut page = FakePage::new(vec![winpng("h/a.png", vec![(link("", "1.smi"), "x")])]);
        page.never_done = true;
        let failure = drive(&page, dir.path(), quick()).await.unwrap_err();
        assert_eq!(failure.kind, FailureKind::Network);

        let mut page = FakePage::new(Vec::new());
        page.no_viewer = true;
        let failure = drive(&page, dir.path(), quick()).await.unwrap_err();
        assert_eq!(failure.kind, FailureKind::Changed);
    }

    #[tokio::test]
    async fn an_image_shown_twice_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let page = FakePage::new(vec![
            winpng("h/a.png", vec![(link("", "1.smi"), "x")]),
            winpng("h/a.png", vec![(link("", "1.smi"), "x")]),
        ]);
        let Viewed::Files(files) = drive(&page, dir.path(), quick()).await.unwrap() else {
            panic!("files");
        };
        assert_eq!(files.len(), 1);
        assert_eq!(page.state.lock().unwrap().opens, [0]);
    }

    #[tokio::test]
    async fn a_download_that_comes_late_is_not_taken_for_the_next_links() {
        let dir = tempfile::tempdir().unwrap();
        let mut page = FakePage::new(vec![winpng(
            "h/x.png",
            vec![
                (link("", "a.smi"), "<SAMI>a"),
                (link("", "b.smi"), "<SAMI>b"),
                (link("", "c.smi"), "<SAMI>c"),
            ],
        )]);
        // b's first download ends only after b was clicked again; c's click
        // then finds the second b ahead of its own bytes.
        page.late_first_click = Some("b.smi");
        let Viewed::Files(files) = drive(&page, dir.path(), quick()).await.unwrap() else {
            panic!("files");
        };
        let got: Vec<(String, String)> = files
            .iter()
            .map(|f| (f.name.clone(), std::fs::read_to_string(&f.path).unwrap()))
            .collect();
        assert_eq!(
            got,
            [
                ("a.smi".to_owned(), "<SAMI>a".to_owned()),
                ("b.smi".to_owned(), "<SAMI>b".to_owned()),
                ("c.smi".to_owned(), "<SAMI>c".to_owned()),
            ]
        );
        assert_eq!(page.state.lock().unwrap().discarded, ["b.smi"]);
    }

    #[test]
    fn the_name_a_download_has_is_the_links_even_if_the_browser_cleaned_it() {
        assert!(same_name("01.smi", "01.smi"));
        assert!(same_name("1:2화?.smi", "1_2화_.smi"));
        assert!(!same_name("01.smi", "02.smi"));
    }

    #[test]
    fn only_a_prompt_asks_for_a_key() {
        let dialog = |kind: &str| DialogSeen {
            kind: kind.to_owned(),
            message: "m".to_owned(),
        };
        assert_eq!(prompt_count(&[dialog("alert"), dialog("confirm")]), 0);
        assert_eq!(
            prompt_count(&[dialog("alert"), dialog("prompt"), dialog("prompt")]),
            2
        );
    }

    #[tokio::test]
    async fn an_image_the_probe_cannot_read_is_skipped_and_fails_the_post_only_if_none_was_read() {
        let dir = tempfile::tempdir().unwrap();
        let broken = |id| Image {
            id,
            probe: Probe::Failed,
            links: Vec::new(),
            prompts: false,
        };
        // Beside a good image, a broken one is passed over.
        let page = FakePage::new(vec![
            broken("h/a.png"),
            winpng("h/b.png", vec![(link("", "1.smi"), "<SAMI>")]),
        ]);
        let Viewed::Files(files) = drive(&page, dir.path(), quick()).await.unwrap() else {
            panic!("files");
        };
        assert_eq!(files.len(), 1);
        assert_eq!(page.state.lock().unwrap().opens, [1]);
        // Nothing read, one broken: not known to hold no subtitle.
        let page = FakePage::new(vec![broken("h/a.png")]);
        let failure = drive(&page, dir.path(), quick()).await.unwrap_err();
        assert_eq!(failure.kind, FailureKind::Network);
        // The same beside a picture.
        let plain = Image {
            id: "h/p.png",
            probe: Probe::Plain,
            links: Vec::new(),
            prompts: false,
        };
        let page = FakePage::new(vec![plain, broken("h/a.png")]);
        let failure = drive(&page, dir.path(), quick()).await.unwrap_err();
        assert_eq!(failure.kind, FailureKind::Network);
    }

    #[tokio::test]
    async fn a_page_where_the_browser_finds_no_image_is_no_images_not_no_subtitle() {
        let dir = tempfile::tempdir().unwrap();
        // `pictures_are_skipped...` shows the same for pictures: NoSubtitle.
        let page = FakePage::new(Vec::new());
        assert_eq!(
            drive(&page, dir.path(), quick()).await.unwrap(),
            Viewed::NoImages
        );
    }

    #[tokio::test]
    async fn draining_stops_when_nothing_is_ready_and_fails_when_the_source_has_ended() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // Two ready, then nothing: both are taken.
        let calls = AtomicUsize::new(0);
        let drained = drain(|| {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    Ok::<usize, &'static str>(n)
                } else {
                    std::future::pending().await
                }
            }
        })
        .await;
        assert_eq!(drained, Ok(2));
        // A source that has ended answers with its error at once, again and
        // again: the draining ends with it instead of going on for ever.
        let ended = tokio::time::timeout(
            Duration::from_secs(5),
            drain(|| async { Err::<(), &'static str>("ended") }),
        )
        .await
        .expect("the draining does not spin");
        assert_eq!(ended, Err("ended"));
    }

    #[test]
    fn what_the_browser_said_does_not_reach_the_reason() {
        let failure = browser_failure(BrowserError::Internal(
            "https://blog.kakaocdn.net/dna/x/img.png?signature=SECRET threw".into(),
        ));
        assert_eq!(failure.kind, FailureKind::Network);
        assert!(!failure.reason.contains("SECRET") && !failure.reason.contains("kakaocdn"));
        let ended = browser_failure(BrowserError::RunEnded { run: "r1".into() });
        assert!(!ended.reason.contains("r1"));
    }

    #[test]
    fn staged_files_are_offered_with_keys_that_name_their_image_and_place() {
        let mut post = Snapshot::default();
        post.push("article:modified_time", "2026-09-28T00:13:41+09:00");
        let staged = |image: &str, folder: Option<&str>, name: &str| Staged {
            image: image.to_owned(),
            folder: folder.map(str::to_owned),
            name: name.to_owned(),
            path: PathBuf::from("/staging/1/x"),
        };
        let files = offered(
            &post,
            &[
                staged("blog.kakaocdn.net/dna/a/img.png", None, "1.smi"),
                staged(
                    "blog.kakaocdn.net/dna/a/img.png",
                    Some("회차/2화/"),
                    "2.smi",
                ),
                staged(
                    "blog.kakaocdn.net/dna/a/img.png",
                    Some("../../etc/"),
                    "3.smi",
                ),
                // The same place twice is one file.
                staged("blog.kakaocdn.net/dna/a/img.png", None, "1.smi"),
            ],
        );
        let keys: Vec<&str> = files.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "winpng:blog.kakaocdn.net/dna/a/img.png#1.smi",
                "winpng:blog.kakaocdn.net/dna/a/img.png#회차/2화/2.smi",
                "winpng:blog.kakaocdn.net/dna/a/img.png#etc/3.smi",
            ]
        );
        assert_eq!(files[1].folder.as_deref(), Some("회차/2화"));
        assert_eq!(files[2].folder.as_deref(), Some("etc"));
        assert_eq!(files[0].staged(), Some(Path::new("/staging/1/x")));
        let entries: Vec<&str> = files[0]
            .snapshot
            .entries()
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(entries, ["article:modified_time", IMAGE]);
    }
}
