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
//! and a `429` that asks for longer holds it off until then; the schedule is in
//! the database, so a restart does not read again within the period.
//! Whatever a reading does not reach (a failed page, or a line that changed
//! twice between two readings) is seen at the next, as its last state.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio_util::sync::CancellationToken;

use trss_anissia::{Anissia, AnissiaError, CaptionLine};
use trss_core::{CycleLock, Millis};

use crate::store::anissia::{AnissiaStore, Line, Observed};
use trss_library::artwork::queue::{LOCK_RETRY, POLL};

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

/// The lock file's path for a database file.
pub fn lock_path_for(db_path: &std::path::Path) -> PathBuf {
    let mut name = db_path.as_os_str().to_owned();
    name.push(".anissia-captions.lock");
    PathBuf::from(name)
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
    /// Lines that could not be used (no address, a bound passed, ...).
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
}

impl CaptionObserver {
    pub fn new(anissia: Anissia, store: AnissiaStore) -> Self {
        CaptionObserver {
            anissia,
            store,
            held_until: Arc::default(),
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
            let skipped = answer.skipped();
            // The list is newest first; observing oldest first gives the
            // observations the order the lines changed in.
            let lines: Vec<Line> = answer
                .lines
                .into_iter()
                .rev()
                .filter_map(|l| {
                    let no = l.anime_no?;
                    Some(line_of(l, no))
                })
                .collect();
            match self.store.observe(lines, self.now()).await {
                Ok(observed) => read.take(observed, skipped),
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
        let skipped = rows - lines.len();
        let lines = lines
            .into_iter()
            .rev()
            .map(|l| line_of(l, anime_no))
            .collect();
        match self.store.observe(lines, self.now()).await {
            Ok(observed) => read.take(observed, skipped),
            Err(e) => read.end = End::Failed(e.to_string()),
        }
        read
    }

    /// Reads the recent list if it is due, and schedules the next reading.
    /// `None` when it is not due yet.
    pub async fn run_due(&self) -> Option<Read> {
        let now = self.now();
        let held = *self.held_until.lock().unwrap_or_else(|e| e.into_inner());
        if now < held {
            return None;
        }
        match self.store.caption_poll().await {
            Ok(Some((next_at, _))) if now < next_at => return None,
            Ok(_) => {}
            Err(e) => {
                eprintln!("Anissia caption observation: {e}");
                return None;
            }
        }
        let read = self.read_recent().await;
        let every = OBSERVE_EVERY.as_millis() as i64;
        let mut next_at = now + every;
        let mut read_at = None;
        match &read.end {
            End::Complete => read_at = Some(self.now()),
            End::Busy(wait) => {
                eprintln!(
                    "Anissia caption observation: asked to wait {}s",
                    wait.as_secs()
                );
                next_at = next_at.max(self.now() + wait.as_millis() as i64);
            }
            End::Failed(why) => eprintln!("Anissia caption observation: {why}"),
        }
        *self.held_until.lock().unwrap_or_else(|e| e.into_inner()) = next_at;
        if let Err(e) = self.store.schedule_caption_poll(next_at, read_at).await {
            eprintln!("Anissia caption observation: cannot write the schedule: {e}");
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
                    eprintln!(
                        "Anissia caption observation: cannot take {}: {e}",
                        lock_path.display()
                    );
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
