//! The worker's automatic artwork work: one job at a time, in request order,
//! each AniList request in its turn ([`trss_anilist::REQUEST_SPACING`]).
//!
//! It runs beside the collection loop, not under the worker lock (it touches
//! neither the media nor Transmission), so a long queue (the first reading of
//! a watch folder records hundreds of works at once) never holds up a cycle,
//! and the web keeps answering. One process runs the queue at a time
//! (`<db path>.artwork.lock`); jobs are rows in the database, so a restart
//! continues where the queue stopped.
//!
//! A search job looks the work's folder name up and records the decision
//! ([`trss_anilist::title::decide`]); a clear match becomes a fetch job for its image.
//! A job whose selection changed meanwhile (the user chose, cleared or asked
//! for something else) is dropped by the store's version check. A failure to
//! reach AniList is tried again later ([`RETRY_DELAYS`]); AniList's `429`
//! waits as long as it says.

use std::{path::PathBuf, time::Duration};

use tokio_util::sync::CancellationToken;

use trss_core::queue::{retry, run_item, Queue, Retry, LOCK_RETRY, POLL};

use crate::{
    artwork::{files, ActionError, Artwork},
    store::artwork::{ClaimedJob, JobKind, Note, Searched, Source},
};
use trss_anilist::{
    title::{decide, Decision},
    AnilistError, ImageFetchError,
};

/// The queue's name in its log lines.
pub(crate) const QUEUE: &str = "Artwork queue";
/// How often the queue recovers interrupted publishes and cleans up.
pub const TIDY_EVERY: Duration = Duration::from_secs(10 * 60);
/// The waits after the first, second and third failed attempt; after that the
/// job is given up with [`Note::Failed`] and waits for the user.
pub const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(60),
    Duration::from_secs(10 * 60),
    Duration::from_secs(60 * 60),
];

/// What running one job came to (tests and logs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ran {
    /// The job's outcome was recorded.
    Recorded,
    /// The selection changed while the job ran; its outcome was dropped.
    Dropped,
    /// The job waits for a later try.
    Later,
    /// The job was given up with this note.
    GaveUp(Note),
}

impl Artwork {
    /// [`Artwork::later`] for what AniList answered.
    async fn later_anilist(&self, job: &ClaimedJob, error: &AnilistError) -> Ran {
        let busy = match error {
            AnilistError::Busy { retry_after } => Some(*retry_after),
            _ => None,
        };
        self.later(job, error.to_string(), busy).await
    }

    /// Puts `job` off after `error`: for `busy` when AniList asked to wait (not
    /// a failure), else by the next of [`RETRY_DELAYS`] (a failure).
    async fn later(&self, job: &ClaimedJob, error: String, busy: Option<Duration>) -> Ran {
        let now = self.now();
        let Retry {
            at: retry_at,
            failed,
        } = retry(now, job.attempts as usize, &RETRY_DELAYS, busy);
        eprintln!(
            "Artwork {} for work {}: {error}",
            job.kind.code(),
            job.work_id
        );
        let written = self
            .store
            .job_later(
                &job.work_id,
                job.version,
                job.requested_at,
                retry_at,
                failed,
                Note::Failed,
                now,
            )
            .await;
        if let Err(e) = written {
            // Nothing holds the job back: pause the queue instead, so it is
            // not taken again at once.
            eprintln!("{QUEUE}: cannot put off work {}: {e}", job.work_id);
            tokio::time::sleep(POLL).await;
        }
        if retry_at.is_some() {
            Ran::Later
        } else {
            Ran::GaveUp(Note::Failed)
        }
    }

    async fn give_up(&self, job: &ClaimedJob, note: Note) -> Ran {
        let written = self
            .store
            .job_later(
                &job.work_id,
                job.version,
                job.requested_at,
                None,
                false,
                note,
                self.now(),
            )
            .await;
        if let Err(e) = written {
            eprintln!("{QUEUE}: cannot give up work {}: {e}", job.work_id);
            tokio::time::sleep(POLL).await;
        }
        Ran::GaveUp(note)
    }

    async fn run_search(&self, job: &ClaimedJob) -> Ran {
        let search = match self.anilist.search_all(&job.dir_name).await {
            Ok(search) => search,
            Err(e) => return self.later_anilist(job, &e).await,
        };
        let outcome = match decide(&job.dir_name, &search.candidates, search.complete) {
            Decision::Select(candidate) => Searched::Selected {
                anilist_media_id: candidate.id,
                image_url: candidate.cover_url.clone(),
            },
            Decision::NoMatch => Searched::Left(Note::NoMatch),
            Decision::Ambiguous => Searched::Left(Note::Ambiguous),
            Decision::Incomplete => Searched::Left(Note::Incomplete),
        };
        match self
            .store
            .searched(
                &job.work_id,
                job.version,
                job.requested_at,
                outcome,
                self.now(),
            )
            .await
        {
            Ok(true) => Ran::Recorded,
            Ok(false) => Ran::Dropped,
            // The outcome could not be recorded: try the job again later, not
            // at once (each try asks AniList again).
            Err(e) => self.later(job, e.to_string(), None).await,
        }
    }

    /// The cover URL AniList gives for `media_id` now; `Err` is how the job
    /// ended when there is none.
    async fn cover_url(&self, job: &ClaimedJob, media_id: i64) -> Result<String, Ran> {
        match self.anilist.media(media_id, None).await {
            Ok(Some(entry)) => match entry.cover_url {
                Some(url) => Ok(url),
                None => Err(self.give_up(job, Note::NoImage).await),
            },
            Ok(None) => Err(self.give_up(job, Note::Gone).await),
            Err(e) => Err(self.later_anilist(job, &e).await),
        }
    }

    /// How the job ends when its image could not be had.
    async fn not_fetched(&self, job: &ClaimedJob, error: ImageFetchError) -> Ran {
        match error {
            ImageFetchError::NotAllowed | ImageFetchError::TooLarge => {
                self.give_up(job, Note::Rejected).await
            }
            e => {
                self.later_anilist(job, &AnilistError::Unreachable(e.to_string()))
                    .await
            }
        }
    }

    async fn run_fetch(&self, job: &ClaimedJob) -> Ran {
        let Some(media_id) = job.anilist_media_id else {
            return self.give_up(job, Note::Gone).await;
        };
        let fetched = match &job.image_url {
            // The URL the search's answer gave. AniList moves cover images
            // now and then; when it is gone, the entry is asked for its
            // current URL, once.
            Some(stored) => match self.anilist.fetch_image(stored).await {
                Err(ImageFetchError::Status(404 | 410)) => {
                    match self.cover_url(job, media_id).await {
                        Ok(fresh) if &fresh != stored => self.anilist.fetch_image(&fresh).await,
                        Ok(_) => Err(ImageFetchError::Status(404)),
                        Err(ran) => return ran,
                    }
                }
                other => other,
            },
            None => match self.cover_url(job, media_id).await {
                Ok(url) => self.anilist.fetch_image(&url).await,
                Err(ran) => return ran,
            },
        };
        let bytes = match fetched {
            Ok(bytes) => bytes,
            Err(e) => return self.not_fetched(job, e).await,
        };
        let image = match self.store_image(bytes.into(), Source::Anilist, None).await {
            Ok(image) => image,
            Err(ActionError::Rejected(_)) => return self.give_up(job, Note::Rejected).await,
            Err(e) => {
                eprintln!("Artwork image for work {}: {e}", job.work_id);
                return self
                    .later_anilist(job, &AnilistError::Unreachable(e.to_string()))
                    .await;
            }
        };
        match self
            .store
            .fetched(&job.work_id, job.version, job.requested_at, media_id, image)
            .await
        {
            Ok(true) => {
                self.tidy().await;
                Ran::Recorded
            }
            Ok(false) => {
                self.tidy().await;
                Ran::Dropped
            }
            // The stored file is left to the recovery; the job waits.
            Err(e) => self.later(job, e.to_string(), None).await,
        }
    }

    /// Runs `job` to its outcome.
    pub async fn run_job(&self, job: &ClaimedJob) -> Ran {
        match job.kind {
            JobKind::Search => self.run_search(job).await,
            JobKind::Fetch => self.run_fetch(job).await,
        }
    }

    /// Runs the next job that is due, if any. A job that panics is put off as
    /// a failure is.
    pub async fn run_next(&self) -> Option<Ran> {
        let job = match self.store.next_job(self.now()).await {
            Ok(job) => job?,
            Err(e) => {
                eprintln!("{QUEUE}: {e}");
                return None;
            }
        };
        let item = format!("{} for work {}", job.kind.code(), job.work_id);
        Some(match run_item(QUEUE, &item, self.run_job(&job)).await {
            Ok(ran) => ran,
            Err(panic) => self.later(&job, format!("panicked: {panic}"), None).await,
        })
    }

    /// Recovers interrupted publishes and removes unreferenced files.
    pub async fn maintain(&self) {
        let Some(app) = self.app_data() else { return };
        if let Err(e) = files::recover(app, &self.store, self.now()).await {
            eprintln!("Artwork recovery failed: {e}");
        }
        self.tidy().await;
    }

    /// Runs the queue until `cancel` fires (see the module docs). A job cut
    /// short by the shutdown stays in the database and runs at the next start.
    pub async fn run_queue(&self, lock_path: PathBuf, cancel: CancellationToken) {
        Queue::new(QUEUE, lock_path, POLL, LOCK_RETRY)
            .run_with_upkeep(&cancel, TIDY_EVERY, || self.maintain(), || self.run_next())
            .await;
    }
}
