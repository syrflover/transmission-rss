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
        // A video the library recorded since the last look takes up the
        // subtitles waiting for it.
        match runner.requeue_awaiting_video().await {
            Ok(0) => {}
            Ok(n) => println!("Subtitle jobs: {n} waiting for a video are in line again"),
            Err(err) => eprintln!("Subtitle jobs: cannot look for arrived videos: {err}"),
        }
        // An archive whose unpacking failed on this machine an hour ago is
        // tried again.
        match runner.requeue_unpack_retries().await {
            Ok(0) => {}
            Ok(n) => {
                println!("Subtitle jobs: {n} waiting to unpack an archive again are in line again")
            }
            Err(err) => eprintln!("Subtitle jobs: cannot look for archives to unpack again: {err}"),
        }
        let ready = runner.has_ready().await.map_err(|e| e.to_string())?;
        if !ready && !runner.has_cleanups().await.map_err(|e| e.to_string())? {
            return Ok(Some(0));
        }
        let Some(hold) = self.hold().await.map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        // A person's cleanups of stored files go first, in this task: no
        // job stores or links a file between a cleanup's look at it and its
        // removal (`trss_jobs::place::cleanup`).
        if let Err(err) = runner.run_cleanups().await {
            eprintln!("Stored files: cannot carry out the cleanups: {err}");
        }
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
    use trss_jobs::{
        area::ReceiveArea, Created, JobRequests, JobRun, JobViews, NewItem, NewJob, PlaceStore,
        Runner, ScreenStore,
    };
    use trss_subtitles::Sources;

    use crate::{Worker, WorkerEnv};

    /// A worker with a job runner and one job whose remote screen is bound to
    /// a run of an earlier worker.
    async fn with_a_bound_screen() -> (tempfile::TempDir, Worker, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        let requests = JobRequests::new(db.clone());
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
        let Created::Created(id) = requests.create(job, 900).await.unwrap() else {
            panic!("the job was not created");
        };
        let item = JobViews::new(db.clone())
            .detail(&id)
            .await
            .unwrap()
            .unwrap()
            .items[0]
            .id;
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
                JobRun::new(db.clone()),
                Sources::none(),
                ReceiveArea::in_app_data(dir.path()),
                Arc::new(|| 2_000),
            ));
        (dir, worker, db)
    }

    /// A worker with a job runner and a job of the work `w1` whose episode 1
    /// row waits for its video (`영상 대기`).
    async fn with_a_job_waiting_for_a_video() -> (tempfile::TempDir, Worker, Db, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        let requests = JobRequests::new(db.clone());
        let job = NewJob {
            command_id: "c1".to_owned(),
            request: "{}".to_owned(),
            origin: "pick".to_owned(),
            work_id: Some("w1".to_owned()),
            season: Some(1),
            anime_no: None,
            source_id: None,
            creator: Some("creator".to_owned()),
            revision_of: None,
            revises_attributed: false,
            items: vec![NewItem {
                observation_id: None,
                episode: "1".to_owned(),
                post_url: "https://fake.trss.invalid/ok/ep1".to_owned(),
                found_at: 500,
            }],
        };
        let Created::Created(id) = requests.create(job, 900).await.unwrap() else {
            panic!("the job was not created");
        };
        let shows = dir.path().join("shows").to_string_lossy().into_owned();
        let job = id.clone();
        db.run(move |c| {
            c.execute(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', ?1, 0)",
                [shows],
            )?;
            c.execute_batch(
                "INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
                 INSERT INTO seasons (work_id, number) VALUES ('w1', 1);",
            )?;
            c.execute(
                "UPDATE subtitle_jobs SET state = 'waiting', wait = 'video' WHERE id = ?1",
                [&job],
            )?;
            c.execute(
                "INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state,
                                                 size, sha256, created_at, updated_at)
                 SELECT 'r1', job_id, id, 'k1', 'Show - 01.ass', 'done', 1, printf('%064d', 1),
                        0, 0
                   FROM subtitle_job_items WHERE job_id = ?1",
                [&job],
            )?;
            c.execute(
                "INSERT INTO subtitle_job_plan (job_id, position, file_id, name, kind, format,
                                                size, sha256, assignment, episode, action,
                                                outcome, updated_at)
                 VALUES (?1, 0, 'r1', 'Show - 01.ass', 'subtitle', 'ass', 1,
                         printf('%064d', 1), 'explicit', 1, 'apply', 'no_video', 0)",
                [&job],
            )?;
            Ok::<_, trss_core::DbError>(())
        })
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
                JobRun::new(db.clone()),
                Sources::none(),
                ReceiveArea::in_app_data(dir.path()),
                Arc::new(|| 2_000),
            ));
        (dir, worker, db, id)
    }

    /// How often the job's log says its video came.
    async fn video_came(db: &Db, job: &str) -> i64 {
        let job = job.to_owned();
        db.run(move |c| {
            Ok::<_, trss_core::DbError>(c.query_row(
                "SELECT COUNT(*) FROM subtitle_job_events
                  WHERE job_id = ?1 AND message = '영상이 들어와 적용을 이어가요'",
                [job],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_video_the_library_records_puts_the_job_waiting_for_it_back_in_line() {
        let (_dir, worker, db, job) = with_a_job_waiting_for_a_video().await;
        let cancel = tokio_util::sync::CancellationToken::new();
        worker.run_jobs_once(&cancel).await.unwrap();
        assert_eq!(video_came(&db, &job).await, 0);

        db.run(|c| {
            c.execute_batch(
                "INSERT INTO episodes (work_id, season, episode) VALUES ('w1', 1, '01');
                 INSERT INTO media_files (work_id, path, season, episode, kind)
                     VALUES ('w1', 'Season 01/Show S01E01.mkv', 1, '01', 'video');",
            )?;
            Ok::<_, trss_core::DbError>(())
        })
        .await
        .unwrap();
        worker.run_jobs_once(&cancel).await.unwrap();
        assert_eq!(video_came(&db, &job).await, 1);
    }

    /// A worker with a job runner, no job to run, and the done job `j1` of
    /// the work `w1` that stored `Show - 01.ass` (`s1`, asset `a1`, the
    /// bytes `abc`) without applying it.
    async fn with_a_stored_subtitle() -> (tempfile::TempDir, Worker, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        let shows = dir.path().join("shows");
        let stored = shows.join("Show/.trss/subtitles/하느");
        std::fs::create_dir_all(&stored).unwrap();
        std::fs::write(stored.join("Show - 01.ass"), b"abc").unwrap();
        let shows = shows.to_string_lossy().into_owned();
        db.run(move |c| {
            c.execute(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', ?1, 0)",
                [shows],
            )?;
            // SHA-256 of `abc`.
            let sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
            c.execute_batch(&format!(
                "INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
                 INSERT INTO seasons (work_id, number) VALUES ('w1', 1);
                 INSERT INTO subtitle_jobs (id, command_id, request, origin, work_id, season,
                                            state, created_at, updated_at, state_at)
                     VALUES ('j1', 'c1', '{{}}', 'pick', 'w1', 1, 'done', 0, 0, 0);
                 INSERT INTO subtitle_job_items (id, job_id, position, episode, post_url,
                                                 found_at, state, updated_at)
                     VALUES (1, 'j1', 0, '1', 'https://example.org/p', 0, 'done', 0);
                 INSERT INTO subtitle_packages (id, work_id, job_id, source_kind, created_at)
                     VALUES ('p1', 'w1', 'j1', 'post', 0);
                 INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state,
                                                 size, sha256, created_at, updated_at)
                     VALUES ('r1', 'j1', 1, 'k1', 'Show - 01.ass', 'done', 3, '{sha}', 0, 0);
                 INSERT INTO subtitle_assets (id, work_id, kind, base, relative_path,
                                              byte_size, sha256, created_at)
                     VALUES ('a1', 'w1', 'subtitle', 'work',
                             '.trss/subtitles/하느/Show - 01.ass', 3, '{sha}', 0);
                 INSERT INTO subtitle_stored (id, work_id, season, package_id,
                                              subtitle_asset_id, assignment, episode, format,
                                              creator, stored_at)
                     VALUES ('s1', 'w1', 1, 'p1', 'a1', 'explicit', 1, 'ass', '하느', 0);
                 INSERT INTO subtitle_job_plan (job_id, position, file_id, name, kind, format,
                                                size, sha256, item_id, assignment, episode,
                                                action, stored_id, outcome, updated_at)
                     VALUES ('j1', 0, 'r1', 'Show - 01.ass', 'subtitle', 'ass', 3, '{sha}', 1,
                             'explicit', 1, 'store', 's1', 'stored', 0);"
            ))?;
            Ok::<_, trss_core::DbError>(())
        })
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
                JobRun::new(db.clone()),
                Sources::none(),
                ReceiveArea::in_app_data(dir.path()),
                Arc::new(|| 2_000),
            ));
        (dir, worker, db)
    }

    #[tokio::test]
    async fn a_confirmed_cleanup_is_carried_out_when_no_job_is_ready() {
        // Also what a worker started after the confirmation does.
        let (dir, worker, db) = with_a_stored_subtitle().await;
        let asked = PlaceStore::new(db.clone())
            .clean_stored("w1", "s1", vec!["a1".to_owned()], 1_000)
            .await
            .unwrap();
        let trss_jobs::place::cleanup::Asked::Asked(cleanup) = asked else {
            panic!("not asked: {asked:?}");
        };
        assert!(!JobRun::new(db.clone()).has_ready().await.unwrap());

        let cancel = tokio_util::sync::CancellationToken::new();
        worker.run_jobs_once(&cancel).await.unwrap();

        let file = dir
            .path()
            .join("shows/Show/.trss/subtitles/하느/Show - 01.ass");
        assert!(!file.exists());
        let state: String = db
            .run(move |c| {
                Ok::<_, trss_core::DbError>(c.query_row(
                    "SELECT state FROM subtitle_cleanups WHERE id = ?1",
                    [cleanup],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(state, "done");
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
