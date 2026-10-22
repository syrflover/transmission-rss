//! The worker's automatic artwork work: one job at a time, in request order,
//! each AniList request in its turn ([`trss_anilist::REQUEST_SPACING`]).
//!
//! It runs beside the collection loop, not inside a cycle (it touches
//! neither the media nor Transmission), so a long queue (the first reading of
//! a watch folder records hundreds of works at once) never holds up a cycle,
//! and the web keeps answering. The queue never runs twice at once in the
//! process ([`trss_core::queue::Queue`]); jobs are rows in the database, so a restart
//! continues where the queue stopped.
//!
//! A search job looks the work's folder name up and records the decision
//! ([`trss_anilist::title::decide`]); a clear match becomes a fetch job for its image.
//! A job whose selection changed meanwhile (the user chose, cleared or asked
//! for something else) is dropped by the store's version check. A failure to
//! reach AniList is tried again later ([`RETRY_DELAYS`]); AniList's `429`
//! waits as long as it says.
//!
//! A subscription's cover ([`crate::store::artwork::subscription`]) runs the
//! same two jobs, ahead of the works': someone just subscribed and looks at
//! the schedule, while a first reading may have queued hundreds of works. Its
//! search looks the Anissia anime's original title up and keeps the one entry
//! whose native title and start date agree
//! ([`trss_anilist::title::decide_original`]).

use std::{fmt, time::Duration};

use tokio_util::sync::CancellationToken;

use trss_core::{
    queue::{retry, run_item, Retry, POLL},
    Millis,
};

use crate::{
    artwork::{files, ActionError, Artwork},
    store::artwork::{
        ArtworkError, ClaimedJob, ImageRef, JobKind, Note, Searched, Source, SubscriptionJob,
    },
};
use trss_anilist::{
    title::{decide, decide_original, Decision},
    AnilistError, FuzzyDate, ImageFetchError,
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

/// The job being run: a work's or a subscription's.
#[derive(Clone, Copy)]
enum Claim<'a> {
    Work(&'a ClaimedJob),
    Subscription(&'a SubscriptionJob),
}

impl<'a> Claim<'a> {
    fn kind(self) -> JobKind {
        match self {
            Claim::Work(job) => job.kind,
            Claim::Subscription(job) => job.kind,
        }
    }

    fn attempts(self) -> u32 {
        match self {
            Claim::Work(job) => job.attempts,
            Claim::Subscription(job) => job.attempts,
        }
    }

    fn media_id(self) -> Option<i64> {
        match self {
            Claim::Work(job) => job.anilist_media_id,
            Claim::Subscription(job) => job.anilist_media_id,
        }
    }

    fn image_url(self) -> Option<&'a str> {
        match self {
            Claim::Work(job) => job.image_url.as_deref(),
            Claim::Subscription(job) => job.image_url.as_deref(),
        }
    }
}

impl fmt::Display for Claim<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Claim::Work(job) => write!(f, "work {}", job.work_id),
            Claim::Subscription(job) => write!(f, "the subscription of rule {}", job.rule_id),
        }
    }
}

/// What a search decided, as the store records it.
fn searched(decision: Decision<'_>) -> Searched {
    match decision {
        Decision::Select(candidate) => Searched::Selected {
            anilist_media_id: candidate.id,
            image_url: candidate.cover_url.clone(),
        },
        Decision::NoMatch => Searched::Left(Note::NoMatch),
        Decision::Ambiguous => Searched::Left(Note::Ambiguous),
        Decision::Incomplete => Searched::Left(Note::Incomplete),
    }
}

impl Artwork {
    /// [`Artwork::later`] for what AniList answered.
    async fn later_anilist(&self, job: Claim<'_>, error: &AnilistError) -> Ran {
        let busy = match error {
            AnilistError::Busy { retry_after } => Some(*retry_after),
            _ => None,
        };
        self.later(job, error.to_string(), busy).await
    }

    /// Writes at `now` that `job` waits until `retry_at` or, with `None`, ends
    /// with `note`.
    async fn put_off(
        &self,
        job: Claim<'_>,
        retry_at: Option<Millis>,
        failed: bool,
        note: Note,
        now: Millis,
    ) -> Result<(), ArtworkError> {
        match job {
            Claim::Work(job) => {
                self.store
                    .job_later(
                        &job.work_id,
                        job.version,
                        job.requested_at,
                        retry_at,
                        failed,
                        note,
                        now,
                    )
                    .await
            }
            Claim::Subscription(job) => {
                self.store
                    .subscription_job_later(
                        &job.rule_id,
                        job.kind,
                        job.requested_at,
                        retry_at,
                        failed,
                        note,
                        now,
                    )
                    .await
            }
        }
    }

    /// Puts `job` off after `error`: for `busy` when AniList asked to wait (not
    /// a failure), else by the next of [`RETRY_DELAYS`] (a failure).
    async fn later(&self, job: Claim<'_>, error: String, busy: Option<Duration>) -> Ran {
        let now = self.now();
        let Retry {
            at: retry_at,
            failed,
        } = retry(now, job.attempts() as usize, &RETRY_DELAYS, busy);
        eprintln!("Artwork {} for {job}: {error}", job.kind().code());
        if let Err(e) = self.put_off(job, retry_at, failed, Note::Failed, now).await {
            // Nothing holds the job back: pause the queue instead, so it is
            // not taken again at once.
            eprintln!("{QUEUE}: cannot put off {job}: {e}");
            tokio::time::sleep(POLL).await;
        }
        if retry_at.is_some() {
            Ran::Later
        } else {
            Ran::GaveUp(Note::Failed)
        }
    }

    async fn give_up(&self, job: Claim<'_>, note: Note) -> Ran {
        if let Err(e) = self.put_off(job, None, false, note, self.now()).await {
            eprintln!("{QUEUE}: cannot give up {job}: {e}");
            tokio::time::sleep(POLL).await;
        }
        Ran::GaveUp(note)
    }

    /// How the job ends when its outcome could not be recorded: tried again
    /// later, not at once (each try asks AniList again).
    async fn recorded(&self, job: Claim<'_>, written: Result<bool, ArtworkError>) -> Ran {
        match written {
            Ok(true) => Ran::Recorded,
            Ok(false) => Ran::Dropped,
            Err(e) => self.later(job, e.to_string(), None).await,
        }
    }

    async fn run_search(&self, job: &ClaimedJob) -> Ran {
        let search = match self.anilist.search_all(&job.dir_name).await {
            Ok(search) => search,
            Err(e) => return self.later_anilist(Claim::Work(job), &e).await,
        };
        let outcome = searched(decide(&job.dir_name, &search.candidates, search.complete));
        let written = self
            .store
            .searched(
                &job.work_id,
                job.version,
                job.requested_at,
                outcome,
                self.now(),
            )
            .await;
        self.recorded(Claim::Work(job), written).await
    }

    async fn run_subscription_search(&self, job: &SubscriptionJob) -> Ran {
        let claim = Claim::Subscription(job);
        // Without an original title nothing can match; AniList is not asked.
        let Some(original) = job.original.as_deref().filter(|o| !o.trim().is_empty()) else {
            return self.give_up(claim, Note::NoMatch).await;
        };
        let search = match self.anilist.search_all(original).await {
            Ok(search) => search,
            Err(e) => return self.later_anilist(claim, &e).await,
        };
        let start = FuzzyDate::from_text(job.start_date.as_deref().unwrap_or_default());
        let outcome = searched(decide_original(
            original,
            start,
            &search.candidates,
            search.complete,
        ));
        let written = self
            .store
            .subscription_searched(&job.rule_id, job.requested_at, outcome, self.now())
            .await;
        self.recorded(claim, written).await
    }

    /// The cover URL AniList gives for `media_id` now; `Err` is how the job
    /// ended when there is none.
    async fn cover_url(&self, job: Claim<'_>, media_id: i64) -> Result<String, Ran> {
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
    async fn not_fetched(&self, job: Claim<'_>, error: ImageFetchError) -> Ran {
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

    /// Receives the image of the entry `job` is for and stores it, staged; the
    /// entry's ID with the image. `Err` is how the job ended when it could not.
    async fn receive(&self, job: Claim<'_>) -> Result<(i64, ImageRef), Ran> {
        let Some(media_id) = job.media_id() else {
            return Err(self.give_up(job, Note::Gone).await);
        };
        let fetched = match job.image_url() {
            // The URL the search's answer gave. AniList moves cover images
            // now and then; when it is gone, the entry is asked for its
            // current URL, once.
            Some(stored) => match self.anilist.fetch_image(stored).await {
                Err(ImageFetchError::Status(404 | 410)) => {
                    match self.cover_url(job, media_id).await {
                        Ok(fresh) if fresh != stored => self.anilist.fetch_image(&fresh).await,
                        Ok(_) => Err(ImageFetchError::Status(404)),
                        Err(ran) => return Err(ran),
                    }
                }
                other => other,
            },
            None => match self.cover_url(job, media_id).await {
                Ok(url) => self.anilist.fetch_image(&url).await,
                Err(ran) => return Err(ran),
            },
        };
        let bytes = match fetched {
            Ok(bytes) => bytes,
            Err(e) => return Err(self.not_fetched(job, e).await),
        };
        match self.store_image(bytes.into(), Source::Anilist, None).await {
            Ok(image) => Ok((media_id, image)),
            Err(ActionError::Rejected(_)) => Err(self.give_up(job, Note::Rejected).await),
            Err(e) => {
                eprintln!("Artwork image for {job}: {e}");
                Err(self
                    .later_anilist(job, &AnilistError::Unreachable(e.to_string()))
                    .await)
            }
        }
    }

    /// How a fetch ends once its image was offered to the store. A file
    /// nobody took goes at once; when the store could not be written, the
    /// stored file is left to the recovery and the job waits.
    async fn fetched(&self, job: Claim<'_>, written: Result<bool, ArtworkError>) -> Ran {
        if written.is_ok() {
            self.tidy().await;
        }
        self.recorded(job, written).await
    }

    async fn run_fetch(&self, job: &ClaimedJob) -> Ran {
        let claim = Claim::Work(job);
        let (media_id, image) = match self.receive(claim).await {
            Ok(received) => received,
            Err(ran) => return ran,
        };
        let written = self
            .store
            .fetched(&job.work_id, job.version, job.requested_at, media_id, image)
            .await;
        self.fetched(claim, written).await
    }

    async fn run_subscription_fetch(&self, job: &SubscriptionJob) -> Ran {
        let claim = Claim::Subscription(job);
        let (media_id, image) = match self.receive(claim).await {
            Ok(received) => received,
            Err(ran) => return ran,
        };
        let written = self
            .store
            .subscription_fetched(&job.rule_id, job.requested_at, media_id, image)
            .await;
        self.fetched(claim, written).await
    }

    /// Runs `job` to its outcome.
    pub async fn run_job(&self, job: &ClaimedJob) -> Ran {
        match job.kind {
            JobKind::Search => self.run_search(job).await,
            JobKind::Fetch => self.run_fetch(job).await,
        }
    }

    /// Runs the subscription cover's `job` to its outcome.
    pub async fn run_subscription_job(&self, job: &SubscriptionJob) -> Ran {
        match job.kind {
            JobKind::Search => self.run_subscription_search(job).await,
            JobKind::Fetch => self.run_subscription_fetch(job).await,
        }
    }

    /// Runs `job` to its outcome; one that panics is put off as a failure is.
    async fn run_claimed(&self, job: Claim<'_>) -> Ran {
        let item = format!("{} for {job}", job.kind().code());
        let ran = match job {
            Claim::Work(work) => run_item(QUEUE, &item, self.run_job(work)).await,
            Claim::Subscription(sub) => {
                run_item(QUEUE, &item, self.run_subscription_job(sub)).await
            }
        };
        match ran {
            Ok(ran) => ran,
            Err(panic) => self.later(job, format!("panicked: {panic}"), None).await,
        }
    }

    /// Runs the next job that is due, if any: a subscription's first.
    pub async fn run_next(&self) -> Option<Ran> {
        let now = self.now();
        match self.store.next_subscription_job(now).await {
            Ok(Some(job)) => return Some(self.run_claimed(Claim::Subscription(&job)).await),
            Ok(None) => {}
            Err(e) => eprintln!("{QUEUE}: {e}"),
        }
        let job = match self.store.next_job(now).await {
            Ok(job) => job?,
            Err(e) => {
                eprintln!("{QUEUE}: {e}");
                return None;
            }
        };
        Some(self.run_claimed(Claim::Work(&job)).await)
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
    pub async fn run_queue(&self, cancel: CancellationToken) {
        self.queue
            .run_with_upkeep(&cancel, TIDY_EVERY, || self.maintain(), || self.run_next())
            .await;
    }
}
