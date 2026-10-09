//! The observation of Anissia's subtitle lines (`docs/specs/subtitles.md`,
//! 자막 후보 조회).
//!
//! Anissia keeps one line per anime and creator and overwrites it, so the app
//! keeps every state it sees ([`crate::store::anissia::captions`]). Two things
//! read:
//!
//! - **Every [`OBSERVE_EVERY`]** ([`CaptionObserver::run_queue`]) the worker
//!   reads the recent list (`/anime/caption/recent/<page>`, pages from 0) to
//!   the empty page and observes every line of every anime, linked to a
//!   season or not, so the episodes that pass before a season is linked are
//!   there when it is. The reading has its own lock
//!   (`<db path>.anissia-captions.lock`), outside the collection cycle's, and
//!   its requests take their turns at the same pace as the rest of the app's
//!   Anissia requests ([`trss_anissia::REQUEST_SPACING`], through the
//!   database). It is not driven by the RSS cycle's interval.
//! - **On request** ([`CaptionObserver::read_anime`]) one anime's lines
//!   (`/anime/caption/animeNo/<n>`) are read at once, which the web asks for
//!   when a season is linked and when the user refreshes
//!   ([`crate::commands::anissia_captions`]). That also reaches the lines the
//!   recent list does not hold (older than 90 days, or not listed).
//!
//! A page is observed as soon as it is read, so a failure part way through
//! keeps what the earlier pages gave. The next reading is due [`OBSERVE_EVERY`]
//! after this one started whatever came of it (a `429`, a failure, or the end),
//! and a `429` that asks for longer holds it off until then. The schedule is
//! written to the database *before* the first request, so a reading cut short
//! (the process killed, or stopped for good) waits for its period instead of
//! starting over at every restart. A schedule further off than a period plus the
//! longest wait a `429` can ask for ([`LONGEST_WAIT`]) is not believed (the clock
//! jumped back): the reading is due.
//!
//! When one answer holds two lines of the same anime and creator name, only
//! the newest is observed ([`collapse`]), so two lines that differ cannot
//! each count as a change at every reading. Every reading that ends logs one
//! line with the pages it read and what it added and skipped.
//! Whatever a reading does not reach (a failed page, or a line that changed
//! twice between two readings) is seen at the next, as its last state.
//!
//! A reading that added observations rings [`CaptionObserver::observed`], so
//! the worker looks at what the subscribed creators posted (the auto receipt
//! of `trss-jobs`, which this crate does not see).

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio_util::sync::CancellationToken;

use trss_anissia::{Anissia, AnissiaError, CaptionLine};
use trss_core::{queue::run_item, CycleLock, LockFile, Millis};

use crate::store::anissia::{AnissiaStore, Line, Observed};
use trss_library::artwork::queue::{LOCK_RETRY, POLL};

/// The queue's name in its log lines.
pub(crate) const QUEUE: &str = "Anissia caption observation";
/// How often the recent list is read.
pub const OBSERVE_EVERY: Duration = Duration::from_secs(30 * 60);
/// The most pages one reading takes: a list that does not end is a list that
/// is not Anissia's. Seventy lines were four pages on 2026-10-02.
pub const MAX_RECENT_PAGES: u32 = 200;
/// How long a reading of the recent list may wait for its turn at the pace;
/// a longer wait (a `429`'s) ends it, and the next period reads again.
pub const RECENT_MAX_WAIT: Duration = Duration::from_secs(5 * 60);
/// How long a request of one anime's lines may wait for its turn.
pub const ANIME_MAX_WAIT: Duration = Duration::from_secs(30);
/// The longest wait a `429` can make the next reading keep (the client honours
/// a `Retry-After` up to an hour).
pub const LONGEST_WAIT: Duration = Duration::from_secs(60 * 60);

/// The lock file's path for a database file.
pub fn lock_path_for(db_path: &std::path::Path) -> PathBuf {
    LockFile::AnissiaCaptions.path_for(db_path)
}

/// How a reading ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    /// Everything asked for was read.
    Complete,
    /// Anissia asked to wait (a `429`), or the app's own pace would make the
    /// reading wait longer than it may. What was read before is kept.
    Busy(Duration),
    /// A request failed or its answer was not usable. What was read before is
    /// kept.
    Failed(String),
}

/// What one reading came to (tests, logs and the command's outcome).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    /// Pages that held lines (1 for one anime's lines).
    pub pages: u32,
    /// Lines that became observations.
    pub added: usize,
    /// Lines the previous observation already said.
    pub unchanged: usize,
    /// Lines that were not observed: ones that could not be used (no address,
    /// a bound passed, ...) and the older of two lines for one creator in one
    /// answer.
    pub skipped: usize,
    pub end: End,
}

impl Read {
    fn new() -> Read {
        Read {
            pages: 0,
            added: 0,
            unchanged: 0,
            skipped: 0,
            end: End::Complete,
        }
    }

    fn take(&mut self, observed: Observed, skipped: usize) {
        self.added += observed.added;
        self.unchanged += observed.unchanged;
        self.skipped += skipped;
    }

    fn stop(mut self, error: AnissiaError) -> Read {
        self.end = match error {
            AnissiaError::Busy { retry_after } => End::Busy(retry_after),
            other => End::Failed(other.to_string()),
        };
        self
    }
}

/// The lines of one answer with the duplicates taken out: of the lines for one
/// anime and creator name only the one with the newest update moment stays
/// (the last of them when moments are missing or equal). The order of the
/// rest is kept. Also returns how many lines were taken out.
fn collapse(lines: Vec<Line>) -> (Vec<Line>, usize) {
    let mut keep: HashMap<(i64, &str), usize> = HashMap::new();
    for (index, line) in lines.iter().enumerate() {
        keep.entry((line.anime_no, line.creator.as_str()))
            .and_modify(|kept| {
                if line.updated_at >= lines[*kept].updated_at {
                    *kept = index;
                }
            })
            .or_insert(index);
    }
    let kept: HashSet<usize> = keep.into_values().collect();
    let total = lines.len();
    let lines: Vec<Line> = lines
        .into_iter()
        .enumerate()
        .filter(|(index, _)| kept.contains(index))
        .map(|(_, line)| line)
        .collect();
    let dropped = total - lines.len();
    (lines, dropped)
}

/// A line as the store keeps it.
fn line_of(line: CaptionLine, anime_no: i64) -> Line {
    Line {
        anime_no,
        creator: line.creator,
        episode: line.episode,
        post_url: line.website,
        updated: line.updated,
        updated_at: line.updated_at,
    }
}

/// The observation of the subtitle lines: the client asks Anissia and the
/// store keeps the answers. Cheap to clone.
#[derive(Clone)]
pub struct CaptionObserver {
    anissia: Anissia,
    store: AnissiaStore,
    /// When this process next reads the recent list, whatever the database
    /// says: a reading whose schedule could not be written is not read again
    /// at once.
    held_until: Arc<Mutex<Millis>>,
    /// Rung whenever a reading added observations.
    observed: Arc<tokio::sync::Notify>,
}

impl CaptionObserver {
    pub fn new(anissia: Anissia, store: AnissiaStore) -> Self {
        CaptionObserver {
            anissia,
            store,
            held_until: Arc::default(),
            observed: Arc::default(),
        }
    }

    /// Rung (once for any number of readings in between) whenever a reading
    /// added observations, by this observer or a clone of it.
    pub fn observed(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.observed)
    }

    fn took(&self, observed: Observed) {
        if observed.added > 0 {
            self.observed.notify_one();
        }
    }

    fn now(&self) -> Millis {
        self.anissia.now()
    }

    /// Reads the recent list from page 0 to the empty page, observing each
    /// page as it is read.
    pub async fn read_recent(&self) -> Read {
        let mut read = Read::new();
        for page in 0..MAX_RECENT_PAGES {
            let answer = match self
                .anissia
                .fetch_recent_captions(page, Some(RECENT_MAX_WAIT))
                .await
            {
                Ok(answer) => answer,
                Err(e) => return read.stop(e),
            };
            if answer.rows == 0 {
                return read;
            }
            read.pages += 1;
            let mut skipped = answer.skipped();
            let (lines, dropped) = collapse(
                answer
                    .lines
                    .into_iter()
                    .filter_map(|l| {
                        let no = l.anime_no?;
                        Some(line_of(l, no))
                    })
                    .collect(),
            );
            skipped += dropped;
            // The list is newest first; observing oldest first gives the
            // observations the order the lines changed in.
            let lines: Vec<Line> = lines.into_iter().rev().collect();
            match self.store.observe(lines, self.now()).await {
                Ok(observed) => {
                    self.took(observed);
                    read.take(observed, skipped)
                }
                Err(e) => {
                    read.end = End::Failed(e.to_string());
                    return read;
                }
            }
        }
        read.end = End::Failed(format!(
            "the list did not end within {MAX_RECENT_PAGES} pages"
        ));
        read
    }

    /// Reads the lines of anime `anime_no` and observes them.
    pub async fn read_anime(&self, anime_no: i64) -> Read {
        let mut read = Read::new();
        let (lines, rows) = match self
            .anissia
            .fetch_caption_lines(anime_no, Some(ANIME_MAX_WAIT))
            .await
        {
            Ok(answer) => answer,
            Err(e) => return read.stop(e),
        };
        read.pages = u32::from(rows > 0);
        let (lines, _) = collapse(lines.into_iter().map(|l| line_of(l, anime_no)).collect());
        let skipped = rows - lines.len();
        let lines: Vec<Line> = lines.into_iter().rev().collect();
        match self.store.observe(lines, self.now()).await {
            Ok(observed) => {
                self.took(observed);
                read.take(observed, skipped)
            }
            Err(e) => read.end = End::Failed(e.to_string()),
        }
        read
    }

    /// Whether a reading due at `next_at` is due at `now`. A schedule further
    /// off than a period plus [`LONGEST_WAIT`] cannot be one this process made
    /// (the clock went back), so it is due.
    fn is_due(now: Millis, next_at: Millis) -> bool {
        let furthest = (OBSERVE_EVERY + LONGEST_WAIT).as_millis() as i64;
        now >= next_at || next_at - now > furthest
    }

    /// Reads the recent list if it is due, and schedules the next reading.
    /// `None` when it is not due yet.
    ///
    /// The next reading is written before the first request, so a reading that
    /// is cut short is not started over at the next start; a `429` that asks
    /// for longer pushes it further when the reading ends.
    pub async fn run_due(&self) -> Option<Read> {
        let now = self.now();
        let held = *self.held_until.lock().unwrap_or_else(|e| e.into_inner());
        if !Self::is_due(now, held) {
            return None;
        }
        match self.store.caption_poll().await {
            Ok(Some((next_at, _))) if !Self::is_due(now, next_at) => return None,
            Ok(_) => {}
            Err(e) => {
                eprintln!("{QUEUE}: {e}");
                return None;
            }
        }
        let next_at = now + OBSERVE_EVERY.as_millis() as i64;
        *self.held_until.lock().unwrap_or_else(|e| e.into_inner()) = next_at;
        if let Err(e) = self.store.schedule_caption_poll(next_at, None).await {
            eprintln!("{QUEUE}: cannot write the schedule: {e}");
        }

        // A reading that panics ends as a failed one: the next is due a
        // period after this one started.
        let read = match run_item(QUEUE, "reading of the recent list", self.read_recent()).await {
            Ok(read) => read,
            Err(panic) => {
                let mut read = Read::new();
                read.end = End::Failed(format!("panicked: {panic}"));
                read
            }
        };

        let counts = format!(
            "{} page(s) read, {} added, {} unchanged, {} skipped",
            read.pages, read.added, read.unchanged, read.skipped
        );
        let mut next_at = next_at;
        let mut read_at = None;
        match &read.end {
            End::Complete => {
                read_at = Some(self.now());
                println!("{QUEUE}: {counts}");
            }
            End::Busy(wait) => {
                eprintln!("{QUEUE}: asked to wait {}s; {counts}", wait.as_secs());
                next_at = next_at.max(self.now() + wait.as_millis() as i64);
            }
            End::Failed(why) => eprintln!("{QUEUE} failed: {why}; {counts}"),
        }
        *self.held_until.lock().unwrap_or_else(|e| e.into_inner()) = next_at;
        if let Err(e) = self.store.schedule_caption_poll(next_at, read_at).await {
            eprintln!("{QUEUE}: cannot write the schedule: {e}");
        }
        Some(read)
    }

    /// Runs the observation until `cancel` fires: waits for the lock (another
    /// worker may hold it) and reads whenever the period is up. A reading cut
    /// short by the shutdown is due again at the next start.
    pub async fn run_queue(&self, lock_path: PathBuf, cancel: CancellationToken) {
        loop {
            let lock = match CycleLock::try_acquire(&lock_path) {
                Ok(lock) => lock,
                Err(e) => {
                    eprintln!("{QUEUE}: cannot take {}: {e}", lock_path.display());
                    None
                }
            };
            let Some(_lock) = lock else {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(LOCK_RETRY) => continue,
                }
            };
            loop {
                let ran = tokio::select! {
                    _ = cancel.cancelled() => return,
                    ran = self.run_due() => ran,
                };
                if ran.is_none() {
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = tokio::time::sleep(POLL) => {}
                    }
                }
            }
        }
    }
}
