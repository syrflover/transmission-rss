//! The worker's loop over the subtitle jobs (see the crate docs, Subtitle
//! jobs, and [`trss_jobs::runner`]).

use tokio_util::sync::CancellationToken;

use super::Worker;

impl Worker {
    /// Carries out the subtitle jobs with `runner` (see the crate docs).
    pub fn with_jobs(mut self, runner: trss_jobs::Runner) -> Self {
        self.jobs = Some(runner);
        self
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
    /// wakes the worker, and every command poll. A run under way at shutdown
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
        let mut ticker = tokio::time::interval(self.command_poll);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = self.job_wake.notified() => {}
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
                Ok(Ok(_)) => {}
                Ok(Err(err)) => eprintln!("Subtitle jobs failed: {err}"),
                Err(err) => eprintln!("Subtitle job task ended with an internal error: {err}"),
            }
        }
    }
}
