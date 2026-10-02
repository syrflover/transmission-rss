//! The worker's daily refresh of the schedule snapshots of subscribed anime,
//! each Anissia request in its turn ([`trss_anissia::REQUEST_SPACING`], shared with
//! the web through the database).
//!
//! A snapshot is due when it is a day old ([`REFRESH_AFTER_MS`]) and an active
//! rule still subscribes to the anime. Anissia has no request for one anime,
//! only for a week's schedule, so a refresh asks for the weeks the due
//! snapshots were listed in (an anime moves from `신작` to its weekday once it
//! starts), and for the other weeks only while some anime is still not found.
//! An anime that no week lists any more keeps its snapshot and is looked for
//! again a day later. When every week was asked, all answered, and the
//! schedule listed something, that anime is recorded as unlisted
//! ([`AnissiaStore::mark_unlisted`]), which is how the weekly schedule learns
//! that an anime without an end date has ended. A refresh that failed, was
//! refused or was cut short records nothing of the kind, and neither does one
//! whose every week came back empty (an answer that lists nothing at all says
//! more about Anissia than about the anime). Likewise a week that came back
//! empty although a due snapshot was last listed in it is not believed: Anissia
//! may have answered that one week wrongly, so the anime whose snapshot is in
//! that week are not recorded as unlisted by this refresh (they are looked for
//! again a day later), while the other weeks decide theirs as usual.
//!
//! A failure to reach Anissia puts every anime that was still to be found off
//! for an hour, and `429` for as long as Anissia says. One process runs the
//! queue at a time (`<db path>.anissia.lock`), outside the collection cycle's
//! lock: it touches neither the media nor Transmission.

use std::{path::PathBuf, time::Duration};

use tokio_util::sync::CancellationToken;

use trss_core::{CycleLock, Millis};

use trss_anissia::{Anissia, AnissiaError, LAST_WEEK};

use crate::{
    artwork::queue::{LOCK_RETRY, POLL},
    store::anissia::{AnissiaStore, Due, REFRESH_AFTER_MS},
};

#[cfg(test)]
mod tests;

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

/// The refresh of the snapshots: the client asks Anissia and the store keeps
/// the answers. Cheap to clone.
#[derive(Clone)]
pub struct AnissiaQueue {
    anissia: Anissia,
    store: AnissiaStore,
}

impl AnissiaQueue {
    pub fn new(anissia: Anissia, store: AnissiaStore) -> Self {
        AnissiaQueue { anissia, store }
    }

    fn now(&self) -> Millis {
        self.anissia.now()
    }

    /// Puts the refresh of `anime_nos` off by `wait`; when even that cannot be
    /// written, pauses the queue so they are not taken again at once.
    async fn put_off(&self, anime_nos: Vec<i64>, wait: Duration) {
        let until = self.now() + wait.as_millis() as i64;
        if let Err(e) = self.store.refresh_later(anime_nos, until).await {
            eprintln!("Anissia queue: cannot put off the refresh: {e}");
            tokio::time::sleep(POLL).await;
        }
    }

    /// Records that Anissia answered for every week and listed none of
    /// `anime_nos`, and puts their refresh off by `wait`.
    async fn leave_unlisted(&self, anime_nos: Vec<i64>, wait: Duration, asked_from: Millis) {
        let at = self.now();
        let until = at + wait.as_millis() as i64;
        if let Err(e) = self
            .store
            .mark_unlisted(anime_nos, at, until, asked_from)
            .await
        {
            eprintln!("Anissia queue: cannot record the unlisted anime: {e}");
            tokio::time::sleep(POLL).await;
        }
    }

    /// Receives the snapshots of `due` again.
    pub async fn refresh(&self, due: Vec<Due>) -> Ran {
        // A snapshot written after this (by the web, which found the anime
        // listed) is newer than the answers below and is not marked unlisted.
        let asked_from = self.now();
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

        // The week each anime's snapshot was listed in.
        let weeks_of: Vec<(i64, Option<u8>)> = due.iter().map(|d| (d.anime_no, d.week)).collect();
        let mut refreshed = 0;
        // Whether any week listed any anime at all.
        let mut anything_listed = false;
        // The weeks that answered with nothing although a due snapshot is in
        // them.
        let mut doubtful_weeks: Vec<u8> = Vec::new();
        for week in weeks {
            if pending.is_empty() {
                break;
            }
            let entries = match self.anissia.fetch_schedule(week, None).await {
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
            anything_listed |= !entries.is_empty();
            if entries.is_empty() {
                let held = weeks_of
                    .iter()
                    .filter(|(no, w)| *w == Some(week) && pending.contains(no))
                    .count();
                if held > 0 {
                    eprintln!(
                        "Anissia refresh of week {week}: the list is empty but {held} anime \
                         were last listed in it; not recording them as unlisted"
                    );
                    doubtful_weeks.push(week);
                }
            }
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
            let wait = Duration::from_millis(REFRESH_AFTER_MS as u64);
            if anything_listed {
                // Every week answered and these were in none of them, except
                // those whose own week came back empty: that answer is doubted.
                let (doubted, gone): (Vec<i64>, Vec<i64>) = pending.into_iter().partition(|no| {
                    weeks_of
                        .iter()
                        .any(|(n, w)| n == no && w.is_some_and(|w| doubtful_weeks.contains(&w)))
                });
                if !gone.is_empty() {
                    self.leave_unlisted(gone, wait, asked_from).await;
                }
                if !doubted.is_empty() {
                    self.put_off(doubted, wait).await;
                }
            } else {
                self.put_off(pending, wait).await;
            }
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
