//! The worker's loop over the subtitle jobs (see the crate docs, Subtitle
//! jobs, and [`trss_jobs::runner`]), and the subscribed creators' receipts
//! that make jobs of their own ([`trss_jobs::follow`]).

use tokio_util::sync::CancellationToken;

use super::Worker;

impl Worker {
    /// Carries out the subtitle jobs with `runner` (see the crate docs), and
    /// makes the subscribed creators' jobs.
    pub fn with_jobs(mut self, runner: trss_jobs::Runner) -> Self {
        self.jobs = Some(runner);
        self.follow = Some(trss_jobs::Follow::new(self.ctx.channels.db().clone()));
        self
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
