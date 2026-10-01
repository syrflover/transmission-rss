//! The worker's daily refresh of the schedule snapshots of subscribed anime,
//! each Anissia request in its turn ([`super::REQUEST_SPACING`], shared with
//! the web through the database).
//!
//! A snapshot is due when it is a day old ([`REFRESH_AFTER_MS`]) and an active
//! rule still subscribes to the anime. Anissia has no request for one anime,
//! only for a week's schedule, so a refresh asks for the weeks the due
//! snapshots were listed in (an anime moves from `신작` to its weekday once it
//! starts), and for the other weeks only while some anime is still not found.
//! An anime that no week lists any more keeps its snapshot and is looked for
//! again a day later.
//!
//! A failure to reach Anissia puts every anime that was still to be found off
//! for an hour, and `429` for as long as Anissia says. One process runs the
//! queue at a time (`<db path>.anissia.lock`), outside the collection cycle's
//! lock: it touches neither the media nor Transmission.

use std::{path::PathBuf, time::Duration};

use tokio_util::sync::CancellationToken;

use super::{Anissia, AnissiaError, LAST_WEEK};
use crate::{
    artwork::queue::{LOCK_RETRY, POLL},
    store::anissia::{Due, REFRESH_AFTER_MS},
    worker::CycleLock,
};

/// How long a refresh that failed waits before the anime are tried again.
pub const REFRESH_RETRY: Duration = Duration::from_secs(60 * 60);

/// The lock file's path for a database file.
pub fn lock_path_for(db_path: &std::path::Path) -> PathBuf {
    let mut name = db_path.as_os_str().to_owned();
    name.push(".anissia.lock");
    PathBuf::from(name)
}

/// What one refresh came to (tests and logs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// Snapshots received again.
    pub refreshed: usize,
    /// Anime no week of the schedule listed; they wait a day.
    pub missing: usize,
    /// Anime put off for [`REFRESH_RETRY`] (or as long as Anissia asked)
    /// because a request failed.
    pub failed: usize,
}

impl Anissia {
    /// Puts the refresh of `anime_nos` off by `wait`; when even that cannot be
    /// written, pauses the queue so they are not taken again at once.
    async fn put_off(&self, anime_nos: Vec<i64>, wait: Duration) {
        let until = self.now() + wait.as_millis() as i64;
        if let Err(e) = self.store.refresh_later(anime_nos, until).await {
            eprintln!("Anissia queue: cannot put off the refresh: {e}");
            tokio::time::sleep(POLL).await;
        }
    }

    /// Receives the snapshots of `due` again.
    pub async fn refresh(&self, due: Vec<Due>) -> Ran {
        let mut pending: Vec<i64> = due.iter().map(|d| d.anime_no).collect();
        let mut weeks: Vec<u8> = Vec::new();
        for week in due.iter().filter_map(|d| d.week) {
            if !weeks.contains(&week) {
                weeks.push(week);
            }
        }
        for week in 0..=LAST_WEEK {
            if !weeks.contains(&week) {
                weeks.push(week);
            }
        }

        let mut refreshed = 0;
        for week in weeks {
            if pending.is_empty() {
                break;
            }
            let entries = match self.fetch_schedule(week, None).await {
                Ok(entries) => entries,
                Err(e) => {
                    eprintln!("Anissia refresh of week {week}: {e}");
                    let wait = match &e {
                        AnissiaError::Busy { retry_after } => *retry_after,
                        _ => REFRESH_RETRY,
                    };
                    let failed = pending.len();
                    self.put_off(pending, wait).await;
                    return Ran {
                        refreshed,
                        missing: 0,
                        failed,
                    };
                }
            };
            let at = self.now();
            for entry in entries {
                let Some(index) = pending.iter().position(|no| *no == entry.anime_no) else {
                    continue;
                };
                match self.store.put_anime(entry.snapshot(at)).await {
                    Ok(()) => {
                        pending.remove(index);
                        refreshed += 1;
                    }
                    Err(e) => {
                        // Not recorded: tried again later, not at once (each try
                        // asks Anissia again).
                        eprintln!("Anissia refresh of anime {}: {e}", entry.anime_no);
                        let failed = pending.len();
                        self.put_off(pending, REFRESH_RETRY).await;
                        return Ran {
                            refreshed,
                            missing: 0,
                            failed,
                        };
                    }
                }
            }
        }

        let missing = pending.len();
        if missing > 0 {
            self.put_off(pending, Duration::from_millis(REFRESH_AFTER_MS as u64))
                .await;
        }
        Ran {
            refreshed,
            missing,
            failed: 0,
        }
    }

    /// Runs the refresh if anything is due.
    pub async fn run_next(&self) -> Option<Ran> {
        match self.store.due(self.now()).await {
            Ok(due) if due.is_empty() => None,
            Ok(due) => Some(self.refresh(due).await),
            Err(e) => {
                eprintln!("Anissia queue: {e}");
                None
            }
        }
    }

    /// Runs the queue until `cancel` fires. A refresh cut short by the shutdown
    /// is due again at the next start.
    pub async fn run_queue(&self, lock_path: PathBuf, cancel: CancellationToken) {
        loop {
            let lock = match CycleLock::try_acquire(&lock_path) {
                Ok(lock) => lock,
                Err(e) => {
                    eprintln!("Anissia queue: cannot take {}: {e}", lock_path.display());
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
