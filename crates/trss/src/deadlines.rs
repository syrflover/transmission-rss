//! The moments at which what a screen shows changes with the clock alone, told
//! to the screens' event stream when they come.
//!
//! The web decides some states from the time at which it reads: whether the
//! collection has stalled, whether a look at Transmission is still believed,
//! whether an episode has aired, which Seoul day it is, whether a rule has
//! been quiet for four weeks. Nothing is written when the time passes, so the
//! database connection (`trss_core::changes`) announces nothing, and a screen
//! that read before the moment would go on showing the state it read. This
//! service tells the screens: [`Deadlines::run`] announces
//! [`ChangeKind::CollectStatus`] (no ID) at the first of the moments below
//! that comes, and the screens that read these states listen to that kind
//! (`web/src/screens/schedule/listens.ts`, `collect/status/listens.ts`,
//! `todo/listens.ts`).
//!
//! Each state's rule sits next to its `next` function, which names the first
//! moment after a given time at which the rule answers otherwise with nothing
//! written, and is tested with the rule:
//!
//! | state | moment | function |
//! | ----- | ------ | -------- |
//! | the collection has stalled | the cycle outlives its bound, or its next check is more than one interval overdue | [`CycleState::next_change`](trss_collect::store::history::CycleState::next_change) |
//! | the look at Transmission is no longer believed | the look is older than three intervals | [`TransmissionCounts::next_stale`](trss_collect::store::status::TransmissionCounts::next_stale) |
//! | an episode's video line leaves `방영 전` | the air time | [`schedule_api::next_video_change`] |
//! | a rule has been quiet for four weeks | four weeks after the moment it counts from | [`archive_api::next_quiet_deadline`] |
//! | the worker's list of Transmission's torrents is too old to say a torrent is there, which the `다시 받기` offers depend on | the list is older than 30 minutes | [`in_place::listing_stale_at`] |
//! | the Seoul day, week and quarter change | the next Seoul midnight | [`next_day_start`] |
//!
//! The service sleeps until the earliest of them. It does not look at the
//! database on a timer: it computes the moments from the database when it
//! starts, and again only when a change that can move one of them is announced
//! (the cycle, the rules and subscriptions, the Anissia snapshots, the season
//! links, the collection history) or when a moment has come. The longest it
//! sleeps is [`LONGEST_SLEEP`], after which it checks the clock against the
//! moment again, so a wall clock that was set or a machine that was suspended
//! is noticed.
//!
//! The list of torrents is written by the cycle in the step after the
//! Transmission snapshot, which announces `collect_status`, and the cycle's end
//! is announced after it, so the service has always computed again by the time
//! a refreshed list is in the database.
//!
//! A moment is a bound, never a promise that the screen changes: the screen
//! reads again and may find the same. A moment missing from the list is a
//! state that the next change announces anyway or that no screen shows; the
//! ones left out on purpose are in `docs/specs/web-app.md`.

use std::time::Duration;

use tokio::{
    sync::{
        broadcast::error::{RecvError, TryRecvError},
        watch,
    },
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use trss_core::{
    calendar::next_day_start,
    changes::{Change, ChangeKind},
    Clock, Millis,
};

use crate::web::{archive_api, in_place, schedule_api, ApiError, AppState};

/// The longest the service sleeps before it compares the clock with the moment
/// it waits for again.
pub const LONGEST_SLEEP: Duration = Duration::from_secs(60 * 60);

/// How long the service waits to try again after the database could not be
/// read.
pub const RETRY_AFTER: Duration = Duration::from_secs(60);

/// The service. Cheap to build; [`Deadlines::run`] does the work.
pub struct Deadlines {
    state: AppState,
    clock: Clock,
    waiting: watch::Sender<Option<Millis>>,
}

impl Deadlines {
    /// A service over the stores of `state`, announcing on `state.changes`,
    /// with `clock` as the time every moment is counted from.
    pub fn new(state: AppState, clock: Clock) -> Self {
        Deadlines {
            state,
            clock,
            waiting: watch::Sender::new(None),
        }
    }

    /// The moment the service waits for, `None` while it has none or has not
    /// worked it out yet: it changes each time the service has computed the
    /// moments again, which is how a test tells that a change was taken in.
    pub fn waiting_for(&self) -> watch::Receiver<Option<Millis>> {
        self.waiting.subscribe()
    }

    /// The first moment after `now` at which a state the web decides from the
    /// clock changes, if any: the earliest of the moments in the module docs.
    pub async fn next(&self, now: Millis) -> Result<Option<Millis>, ApiError> {
        let state = &self.state;
        let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
        let mut moments = vec![next_day_start(now)];

        if let Some(interval) = state
            .status
            .cycle_interval()
            .await
            .map_err(|e| internal(&e))?
        {
            let last = state.history.last_cycle().await.map_err(|e| internal(&e))?;
            if let Some(cycle) = &last {
                moments.extend(cycle.next_change(interval, now));
            }
            if let Some(counts) = state
                .status
                .transmission()
                .await
                .map_err(|e| internal(&e))?
            {
                let busy = last
                    .filter(|cycle| cycle.is_running(interval, now))
                    .map(|cycle| cycle.started_at);
                moments.extend(counts.next_stale(interval, now, busy));
            }
        }
        if let Some(taken_at) = state
            .status
            .torrent_listing_taken_at()
            .await
            .map_err(|e| internal(&e))?
        {
            moments.extend(in_place::listing_stale_at(taken_at, now));
        }
        moments.extend(schedule_api::next_video_change(state, now).await?);
        moments.extend(archive_api::next_quiet_deadline(state, now).await?);
        Ok(moments.into_iter().min())
    }

    /// Announces each moment as it comes, until `cancel` is cancelled.
    pub async fn run(self, cancel: CancellationToken) {
        // Heard from before the first computation, so a change made while the
        // moments are computed is not lost.
        let mut heard = self.state.changes.subscribe();
        // The moment waited for. It outlives a computation that failed, so a
        // moment worked out before the failure still comes.
        let mut due: Option<Millis> = None;
        loop {
            let now = (self.clock)();
            // The sleep counts from before the computation, which takes a
            // while, so it ends when the moment comes and not that much later.
            let started = Instant::now();
            let until = |at: Millis| Duration::from_millis(at.saturating_sub(now).max(0) as u64);
            let wait = match self.next(now).await {
                Ok(next) => {
                    due = next;
                    due.map_or(LONGEST_SLEEP, |at| until(at).min(LONGEST_SLEEP))
                }
                Err(e) => {
                    eprintln!("trss: cannot work out the next deadline of a screen: {e:?}");
                    due.map_or(RETRY_AFTER, |at| until(at).min(RETRY_AFTER))
                }
            };
            self.waiting.send_replace(due);
            let sleep = tokio::time::sleep_until(started + wait);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    () = cancel.cancelled() => return,
                    () = &mut sleep => break,
                    change = heard.recv() => match change {
                        Ok(change) if !may_move_a_moment(&change) => {}
                        Ok(_) | Err(RecvError::Lagged(_)) => break,
                        Err(RecvError::Closed) => return,
                    },
                }
            }
            // Announced before the moments are worked out again, whichever
            // woke the service: worked out from a time past it, the moment is
            // gone.
            if due.is_some_and(|at| (self.clock)() >= at) {
                self.state
                    .changes
                    .announce_target(ChangeKind::CollectStatus, None);
                due = None;
            }
            // What was heard so far, this announcement included, is in what
            // the next computation reads, so it counts once.
            while matches!(heard.try_recv(), Ok(_) | Err(TryRecvError::Lagged(_))) {}
        }
    }
}

/// Whether a change can move one of the moments: the cycle and Transmission's
/// look (`collect_status`), the rules, their subscriptions and the grounds
/// of the suggestions (`rules`), the Anissia snapshots the air times come
/// from (`anissia`), the season links (`season_info`) and the history the quiet
/// weeks count from (`history`). The others, such as a subtitle job's steps,
/// move none.
fn may_move_a_moment(change: &Change) -> bool {
    matches!(
        change.kind,
        ChangeKind::CollectStatus
            | ChangeKind::Rules
            | ChangeKind::Anissia
            | ChangeKind::SeasonInfo
            | ChangeKind::History
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(kind: ChangeKind) -> Change {
        Change { kind, id: None }
    }

    #[test]
    fn the_changes_of_what_the_moments_are_made_of_move_them_and_the_others_do_not() {
        for kind in [
            ChangeKind::CollectStatus,
            ChangeKind::Rules,
            ChangeKind::Anissia,
            ChangeKind::SeasonInfo,
            ChangeKind::History,
        ] {
            assert!(may_move_a_moment(&change(kind)), "{kind:?}");
        }
        for kind in [
            ChangeKind::Command,
            ChangeKind::PastSearch,
            ChangeKind::SubtitleJob,
            ChangeKind::SubtitleScreen,
            ChangeKind::SubtitleStore,
            ChangeKind::SubtitleMapping,
            ChangeKind::Library,
            ChangeKind::Artwork,
            ChangeKind::Settings,
        ] {
            assert!(!may_move_a_moment(&change(kind)), "{kind:?}");
        }
    }
}
