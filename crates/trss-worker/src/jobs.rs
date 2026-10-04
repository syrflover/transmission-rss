//! The worker's loop over the subtitle jobs (see the crate docs, Subtitle
//! jobs, and [`trss_jobs::runner`]), and the subscribed creators' receipts
//! that make jobs of their own ([`trss_jobs::follow`]).

use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::Worker;

/// How often the worker looks for received episodes to read again. Each item
/// is read once a day ([`trss_jobs::recheck::INTERVAL`]), so this only bounds
/// how late a due reading starts.
pub(crate) const RECHECK_EVERY: Duration = Duration::from_secs(60 * 60);

/// How often the worker looks at the remote screens besides the web's
/// wake-ups: the runs bound to jobs that wait for a site's check are watched
/// from the first look after the job settled.
pub(crate) const SCREEN_POLL: Duration = Duration::from_secs(2);

impl Worker {
    /// Carries out the subtitle jobs with `runner` (see the crate docs), makes
    /// the subscribed creators' jobs, and reads their received episodes' files
    /// again (with the runner's sources, so the requests keep the same pace
    /// per host).
    pub fn with_jobs(mut self, runner: trss_jobs::Runner) -> Self {
        let db = self.ctx.channels.db().clone();
        self.recheck = Some(trss_jobs::Recheck::new(
            db.clone(),
            runner.sources().clone(),
        ));
        self.jobs = Some(runner);
        self.follow = Some(trss_jobs::Follow::new(db));
        self
    }

    /// Overrides how often the due rechecks are looked for (default: an hour).
    pub fn with_recheck_every(mut self, every: Duration) -> Self {
        self.recheck_every = every;
        self
    }

    /// One pass of the recheck at the clock's time (see
    /// [`trss_jobs::recheck`]); the jobs it makes are run at once. 0 for a
    /// worker that runs no jobs.
    pub async fn recheck_once(&self, cancel: &CancellationToken) -> Result<usize, String> {
        let Some(recheck) = &self.recheck else {
            return Ok(0);
        };
        let report = recheck
            .run(&self.clock, cancel)
            .await
            .map_err(|e| e.to_string())?;
        if report.read() > 0 {
            println!(
                "Subtitle recheck: {} read ({} changed, {} same, {} missing, {} failed, {} unreadable)",
                report.read(),
                report.changed,
                report.same,
                report.missing,
                report.failed,
                report.unreadable
            );
        }
        if !report.jobs.is_empty() {
            self.job_wake.notify_one();
        }
        Ok(report.read())
    }

    /// The recheck until `cancel` fires: a pass at the start and one every
    /// [`Worker::with_recheck_every`]. A pass that fails is logged and the
    /// next one goes on.
    pub(crate) async fn run_rechecks(&self, cancel: CancellationToken) {
        if self.recheck.is_none() {
            return;
        }
        let mut ticker = tokio::time::interval(self.recheck_every);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = ticker.tick() => {}
            }
            // In its own task so that a panic ends the pass, not the loop.
            let run = tokio::spawn({
                let (worker, cancel) = (self.clone(), cancel.clone());
                async move { worker.recheck_once(&cancel).await }
            });
            match run.await {
                Ok(Ok(_)) => {}
                Ok(Err(err)) => eprintln!("Subtitle recheck failed: {err}"),
                Err(err) => eprintln!("Subtitle recheck ended with an internal error: {err}"),
            }
        }
    }

    /// Looks at the subscribed creators again whenever `stored` rings: the
    /// season queue's [`trss_library::seasons::Seasons::stored`], since a
    /// season's episode count can make a creator's posts receivable.
    pub fn with_season_info(mut self, stored: std::sync::Arc<tokio::sync::Notify>) -> Self {
        self.season_stored = Some(stored);
        self
    }

    /// Makes the jobs of the subscribed creators' episodes to receive
    /// ([`trss_jobs::Follow::evaluate`]). Returns how many it made; 0 for a
    /// worker that runs no jobs.
    pub async fn follow_once(&self) -> Result<usize, String> {
        let Some(follow) = &self.follow else {
            return Ok(0);
        };
        let made = follow
            .evaluate((self.clock)())
            .await
            .map_err(|e| e.to_string())?;
        if !made.is_empty() {
            println!(
                "Subtitle jobs: {} made for the subscribed creators",
                made.len()
            );
        }
        Ok(made.len())
    }

    /// [`Self::follow_once`], logged; whether it made a job.
    pub(crate) async fn follow_logged(&self) -> bool {
        match self.follow_once().await {
            Ok(made) => made > 0,
            Err(err) => {
                eprintln!("Subtitle jobs: cannot look at the subscribed creators: {err}");
                false
            }
        }
    }

    /// Clears the bindings of the remote screens when this worker holds the
    /// server browser's lock, with the browser or without it (see
    /// [`Worker::with_unused_browser_lock`]): the runs they name were an
    /// earlier worker's ([`trss_jobs::screen`]), and a job that waits for a
    /// check would otherwise show a screen for a run that is gone. A worker
    /// that does not hold the lock leaves them, since they may be the other
    /// worker's.
    pub(crate) async fn clear_screens(&self) {
        let Some(runner) = &self.jobs else {
            return;
        };
        if self.browser.is_none() && !self.browser_lock_unused {
            return;
        }
        match runner.screens().unbind_all((self.clock)()).await {
            Ok(0) => {}
            Ok(n) => {
                println!("Remote screens: {n} bound to an earlier worker's browser are closed")
            }
            Err(err) => eprintln!("Remote screens: cannot close the earlier ones: {err}"),
        }
    }

    /// The remote screens of the jobs that wait for a site's check, until
    /// `cancel` fires: whenever the web wakes the worker (a person opened a
    /// job's page) and every [`SCREEN_POLL`], the runner watches the bound
    /// runs and answers the requests to prepare a screen
    /// ([`trss_jobs::Runner::tend_screens`]). A worker without the server
    /// browser has no runs to watch; it still ends the find jobs a person
    /// asked to finish, which only a worker does.
    pub(crate) async fn run_screens(&self, cancel: CancellationToken) {
        let Some(runner) = self.jobs.clone() else {
            return;
        };
        let mut ticker = tokio::time::interval(SCREEN_POLL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = self.screen_wake.notified() => {}
                _ = ticker.tick() => {}
            }
            if let Err(err) = runner.tend_screens(&self.job_wake, &cancel).await {
                eprintln!("Remote screens: {err}");
            }
        }
    }

    /// Runs the ready subtitle jobs now, if this worker can take the lock.
    /// Returns how many runs ended; `None` when another worker holds the lock
    /// or this worker is running jobs already.
    pub async fn run_jobs_once(&self, cancel: &CancellationToken) -> Result<Option<usize>, String> {
        let Some(runner) = &self.jobs else {
            return Ok(Some(0));
        };
        let Ok(_one_run) = self.jobs_running.try_lock() else {
            return Ok(None);
        };
        if !runner.has_ready().await.map_err(|e| e.to_string())? {
            return Ok(Some(0));
        }
        let Some(hold) = self.hold().await.map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        let ran = runner.run_ready(cancel).await;
        hold.release().await;
        ran.map(Some).map_err(|e| e.to_string())
    }

    /// The jobs until `cancel` fires: a look at the start, whenever the web
    /// wakes the worker, and every command poll. The subscribed creators are
    /// looked at again when Anissia's lines added observations and when a
    /// season's entry was stored. A run under way at shutdown
    /// gets the grace period, then is aborted; its job stays `running`.
    pub(crate) async fn run_jobs(&self, cancel: CancellationToken) {
        let Some(runner) = self.jobs.clone() else {
            return;
        };
        match runner.requeue_waiting_for_sources().await {
            Ok(0) => {}
            Ok(n) => println!("Subtitle jobs: {n} waiting for a source are in line again"),
            Err(err) => {
                eprintln!("Subtitle jobs: cannot put the ones waiting for a source in line: {err}")
            }
        }
        // What the subscribed creators posted while the worker was away.
        self.follow_logged().await;
        let observed = self.captions.as_ref().map(|c| c.observed());
        let mut ticker = tokio::time::interval(self.command_poll);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = self.job_wake.notified() => {}
                _ = async {
                    match &observed {
                        Some(observed) => observed.notified().await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.follow_logged().await;
                }
                _ = async {
                    match &self.season_stored {
                        Some(stored) => stored.notified().await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.follow_logged().await;
                }
                _ = ticker.tick() => {}
            }
            // In its own task so that a panic ends the run, not the loop.
            let mut run = tokio::spawn({
                let (worker, cancel) = (self.clone(), cancel.clone());
                async move { worker.run_jobs_once(&cancel).await }
            });
            let joined = tokio::select! {
                joined = &mut run => joined,
                _ = async {
                    cancel.cancelled().await;
                    tokio::time::sleep(self.shutdown_grace).await;
                } => {
                    run.abort();
                    let _ = run.await;
                    println!("Subtitle job abandoned: it did not wind down in time after the shutdown request");
                    break;
                }
            };
            match joined {
                // A job that held a creator's episode may have ended; what it
                // makes is run at once.
                Ok(Ok(Some(ran))) if ran > 0 => {
                    if self.follow_logged().await {
                        self.job_wake.notify_one();
                    }
                }
                Ok(Ok(_)) => {}
                Ok(Err(err)) => eprintln!("Subtitle jobs failed: {err}"),
                Err(err) => eprintln!("Subtitle job task ended with an internal error: {err}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use trss_core::Db;
    use trss_jobs::{area::ReceiveArea, Created, JobStore, NewItem, NewJob, Runner, ScreenStore};
    use trss_subtitles::Sources;

    use crate::{Worker, WorkerEnv};

    /// A worker with a job runner and one job whose remote screen is bound to
    /// a run of an earlier worker.
    async fn with_a_bound_screen() -> (tempfile::TempDir, Worker, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        let store = JobStore::new(db.clone());
        let job = NewJob {
            command_id: "c1".to_owned(),
            request: "{}".to_owned(),
            origin: "pick".to_owned(),
            work_id: None,
            season: Some(1),
            anime_no: None,
            source_id: None,
            creator: Some("creator".to_owned()),
            revision_of: None,
            revises_attributed: false,
            items: vec![NewItem {
                observation_id: None,
                episode: "1".to_owned(),
                post_url: "https://fake.trss.invalid/check/ep1".to_owned(),
                found_at: 500,
            }],
        };
        let Created::Created(id) = store.create(job, 900).await.unwrap() else {
            panic!("the job was not created");
        };
        let item = store.detail(&id).await.unwrap().unwrap().items[0].id;
        ScreenStore::new(db.clone())
            .bind(&id, item, "run-1", "target-1", 1_000)
            .await
            .unwrap();
        let env = WorkerEnv::from_lookup(|key| {
            (key == "TRANSMISSION_URL").then(|| "http://127.0.0.1:1/transmission/rpc".to_owned())
        })
        .unwrap();
        let worker = Worker::new(db.clone(), &env, dir.path().join("app.db.worker.lock"))
            .unwrap()
            .with_clock(Arc::new(|| 2_000))
            .with_jobs(Runner::new(
                store,
                Sources::none(),
                ReceiveArea::in_app_data(dir.path()),
                Arc::new(|| 2_000),
            ));
        (dir, worker, db)
    }

    /// How many remote screens are bound to a run.
    async fn bound(db: &Db) -> i64 {
        db.run(|c| {
            Ok::<_, trss_core::DbError>(c.query_row(
                "SELECT COUNT(*) FROM subtitle_job_screens WHERE run_id IS NOT NULL",
                [],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_worker_that_holds_the_browser_lock_without_the_browser_closes_the_leftover_screens()
    {
        let (_dir, worker, db) = with_a_bound_screen().await;
        assert_eq!(bound(&db).await, 1);
        let worker = worker.with_unused_browser_lock();

        worker.clear_screens().await;

        assert_eq!(bound(&db).await, 0);
    }

    #[tokio::test]
    async fn a_worker_that_does_not_hold_the_browser_lock_leaves_the_screens_alone() {
        // They may be the screens of the worker that does hold it.
        let (_dir, worker, db) = with_a_bound_screen().await;
        assert_eq!(bound(&db).await, 1);

        worker.clear_screens().await;

        assert_eq!(bound(&db).await, 1);
    }
}
