//! The worker's automatic season work, one job at a time and each AniList
//! request in its turn ([`trss_anilist::REQUEST_SPACING`], shared
//! with the cover queue through the database).
//!
//! - **Linking a first season.** A search job looks the work's folder name up
//!   and links the entry it names exactly ([`trss_anilist::title::decide`],
//!   the rule the cover uses: the search read to its last page, one candidate
//!   whose title equals the name after NFC, case folding and space folding).
//!   A linked entry is what an `auto` cover follows ([`Seasons::follow_cover`]).
//!   Anything else links nothing and leaves a note. The result is dropped when
//!   the season's link changed meanwhile (the store checks the version).
//! - **The daily refresh.** An entry a season links that is not yet released or
//!   is releasing is received again once a day. A finished entry never is, until
//!   the user asks.
//!
//! One process runs the queue at a time (`<db path>.seasons.lock`). Searches
//! come before refreshes. A failure to reach AniList is tried again later
//! ([`crate::artwork::queue::RETRY_DELAYS`]); a refresh that fails waits an hour,
//! and AniList's `429` waits as long as it says.

use std::{path::PathBuf, time::Duration};

use tokio_util::sync::CancellationToken;

use trss_core::CycleLock;

use crate::{
    artwork::queue::{LOCK_RETRY, POLL, RETRY_DELAYS},
    seasons::Seasons,
    store::seasons::{ClaimedSearch, Note, SeasonError},
};
use trss_anilist::{
    season::fetch_entry,
    title::{decide, Decision},
    AnilistError,
};

/// How long a refresh that failed waits before the entry is tried again.
pub const REFRESH_RETRY: Duration = Duration::from_secs(60 * 60);

/// The lock file's path for a database file.
pub fn lock_path_for(db_path: &std::path::Path) -> PathBuf {
    let mut name = db_path.as_os_str().to_owned();
    name.push(".seasons.lock");
    PathBuf::from(name)
}

/// What running one job came to (tests and logs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ran {
    /// A search linked an entry.
    Linked(i64),
    /// A search left the season unlinked, with this note.
    Left(Note),
    /// The season's link changed (or the season stopped being the first) while
    /// the search ran; its outcome was dropped.
    Dropped,
    /// The search waits for a later try.
    Later,
    /// An entry was received again.
    Refreshed(i64),
    /// A refresh did not happen; the entry waits.
    RefreshLater(i64),
}

impl Seasons {
    async fn later(&self, job: &ClaimedSearch, error: &AnilistError) -> Ran {
        let busy = match error {
            AnilistError::Busy { retry_after } => Some(*retry_after),
            _ => None,
        };
        self.put_off(job, busy, error).await
    }

    /// Puts the search off: by `busy` (AniList's own wait, not counted as a
    /// failure) or by the next of [`RETRY_DELAYS`], after which it is given up.
    async fn put_off(
        &self,
        job: &ClaimedSearch,
        busy: Option<Duration>,
        why: &(dyn std::fmt::Display + Sync),
    ) -> Ran {
        let now = self.now();
        let (retry_at, failed) = match busy {
            Some(wait) => (Some(now + wait.as_millis() as i64), false),
            None => match RETRY_DELAYS.get(job.attempts as usize) {
                Some(delay) => (Some(now + delay.as_millis() as i64), true),
                None => (None, true),
            },
        };
        eprintln!(
            "Season search for work {} season {}: {why}",
            job.work_id, job.season
        );
        let written = self
            .store
            .search_later(
                &job.work_id,
                job.season,
                job.version,
                retry_at,
                failed,
                Note::Failed,
            )
            .await;
        if let Err(e) = written {
            // Nothing holds the search back: pause the queue instead, so it is
            // not taken again at once.
            eprintln!("Season queue: cannot put off work {}: {e}", job.work_id);
            tokio::time::sleep(POLL).await;
        }
        if retry_at.is_some() {
            Ran::Later
        } else {
            Ran::Left(Note::Failed)
        }
    }

    /// The search's outcome could not be recorded: it is tried again later,
    /// not at once (each try asks AniList again).
    async fn not_recorded(&self, job: &ClaimedSearch, error: SeasonError) -> Ran {
        self.put_off(job, None, &format!("cannot record the outcome: {error}"))
            .await
    }

    async fn leave(&self, job: &ClaimedSearch, note: Note) -> Ran {
        match self
            .store
            .search_later(&job.work_id, job.season, job.version, None, false, note)
            .await
        {
            Ok(true) => Ran::Left(note),
            Ok(false) => Ran::Dropped,
            Err(e) => self.not_recorded(job, e).await,
        }
    }

    pub async fn run_search(&self, job: &ClaimedSearch) -> Ran {
        let search = match self.anilist.search_all(&job.dir_name).await {
            Ok(search) => search,
            Err(e) => return self.later(job, &e).await,
        };
        let candidate = match decide(&job.dir_name, &search.candidates, search.complete) {
            Decision::Select(candidate) => candidate,
            Decision::NoMatch => return self.leave(job, Note::NoMatch).await,
            Decision::Ambiguous => return self.leave(job, Note::Ambiguous).await,
            Decision::Incomplete => return self.leave(job, Note::Incomplete).await,
        };
        let entry = match fetch_entry(&self.anilist, candidate.id, None, self.now()).await {
            Ok(Some(entry)) => entry,
            // The entry the search just named is gone: try again later.
            Ok(None) => {
                return self
                    .later(job, &AnilistError::Invalid("the entry vanished".to_owned()))
                    .await
            }
            Err(e) => return self.later(job, &e).await,
        };
        let id = entry.id;
        if let Err(e) = self.store.put_entry(entry).await {
            return self.not_recorded(job, e).await;
        }
        match self
            .store
            .auto_linked(&job.work_id, job.season, job.version, id)
            .await
        {
            Ok(true) => {
                self.follow_cover(&job.work_id).await;
                self.stored.notify_one();
                Ran::Linked(id)
            }
            Ok(false) => Ran::Dropped,
            Err(e) => self.not_recorded(job, e).await,
        }
    }

    /// Puts the refresh of entry `id` off by `wait`; when even that cannot be
    /// written, pauses the queue so the entry is not taken again at once.
    async fn refresh_off(&self, id: i64, wait: Duration) -> Ran {
        let retry_at = self.now() + wait.as_millis() as i64;
        if let Err(e) = self.store.refresh_later(id, retry_at).await {
            eprintln!("Season queue: cannot put off entry {id}: {e}");
            tokio::time::sleep(POLL).await;
        }
        Ran::RefreshLater(id)
    }

    async fn run_refresh(&self, id: i64) -> Ran {
        let now = self.now();
        match fetch_entry(&self.anilist, id, None, now).await {
            Ok(Some(entry)) => match self.store.put_entry(entry).await {
                Ok(()) => {
                    self.stored.notify_one();
                    Ran::Refreshed(id)
                }
                Err(e) => {
                    eprintln!("Season refresh of entry {id}: {e}");
                    self.refresh_off(id, REFRESH_RETRY).await
                }
            },
            Ok(None) => match self.store.refresh_gone(id, now).await {
                Ok(()) => Ran::RefreshLater(id),
                Err(e) => {
                    eprintln!("Season refresh of entry {id}: {e}");
                    self.refresh_off(id, REFRESH_RETRY).await
                }
            },
            Err(e) => {
                eprintln!("Season refresh of entry {id}: {e}");
                let wait = match &e {
                    AnilistError::Busy { retry_after } => *retry_after,
                    _ => REFRESH_RETRY,
                };
                self.refresh_off(id, wait).await
            }
        }
    }

    /// Runs the next job that is due, if any: a search first, then a refresh.
    pub async fn run_next(&self) -> Option<Ran> {
        let now = self.now();
        match self.store.next_search(now).await {
            Ok(Some(job)) => return Some(self.run_search(&job).await),
            Ok(None) => {}
            Err(e) => {
                eprintln!("Season queue: {e}");
                return None;
            }
        }
        match self.store.next_refresh(now).await {
            Ok(Some(id)) => Some(self.run_refresh(id).await),
            Ok(None) => None,
            Err(e) => {
                eprintln!("Season queue: {e}");
                None
            }
        }
    }

    /// Runs the queue until `cancel` fires. A job cut short by the shutdown
    /// stays in the database and runs at the next start.
    pub async fn run_queue(&self, lock_path: PathBuf, cancel: CancellationToken) {
        loop {
            let lock = match CycleLock::try_acquire(&lock_path) {
                Ok(lock) => lock,
                Err(e) => {
                    eprintln!("Season queue: cannot take {}: {e}", lock_path.display());
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
                    ran = self.run_next() => ran,
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
