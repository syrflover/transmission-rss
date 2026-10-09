//! The care of a job's remote screen: bringing an item to a site's check,
//! watching the run that shows it, and ending the run with the job. The
//! runner's loop decides the order and asks [`ScreenTender`]
//! ([`ScreenTender::through_check`], [`ScreenTender::release`]); the worker
//! asks it to look at the screens ([`ScreenTender::tend`]); the state of the
//! watches belongs to it alone, and so does the find job
//! ([`find`]), which is driven by the screen.
//!
//! # A site's check
//!
//! A post whose check a person passes in the server browser
//! ([`trss_subtitles::Opened::BrowserAuth`]) is first brought to the check by the runner's
//! [`AuthBrowser`] ([`super::Runner::with_auth`]), and the run that shows it is
//! bound to the job ([`crate::screen`]); one item of a job at a time. The
//! worker watches the bound run ([`super::Runner::tend_screens`]): the file the
//! browser downloads when the person passes the check is put in a folder of
//! the item's and the job goes back in line, and the item's next run
//! receives that file like any other (`받기`). Where the file should have
//! come from answering with a web page instead (an expired address, a
//! file gone; [`Waited::Refused`]) is kept in that folder the same way,
//! and the item's next run fails with it: nothing asks the address again,
//! and a new attempt goes through the check anew. Each opening of the
//! job's screen lets the browser bring a check that went away back to the
//! page ([`AuthBrowser::rearm`]). A person may also ask to start the run
//! anew when its page is stuck ([`screen::ScreenStore::request_restart`]):
//! the worker ends the run, after any download of it has ended, and puts
//! the job back in line to be brought to the check in a new run. The
//! item's folder goes once the item settles (received, failed or held). A
//! run that ends before leaves the job waiting with no run; a person's
//! next opening of the job's page asks for it again, and the job goes back
//! in line to be brought to the check anew. The runs the worker's own
//! shutdown ends leave the binding to the next worker's start.
//!
//! A clone of the runner shares the tender's state with the runner: the
//! tasks the worker spawns see the same watches, the same check being brought
//! back for a job and the same takers of a find job.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use trss_core::{Clock, Millis};
use trss_subtitles::{
    auth::{self, AuthBrowser, AuthPage, Waited},
    Failure, PostFile,
};
use url::Url;

use super::{described, NO_AUTH_BROWSER, OTHER_CHECK_FIRST};
use crate::{
    area::ReceiveArea,
    model::{ItemState, StepKind, StepState, Wait},
    screen::{self, Arrival, ScreenStore},
    store::{FileProblem, ItemRow, JobError, JobRun, JobViews},
};

pub mod find;
mod pages;

/// Cares for the remote screens of the jobs. Cheap to clone; every clone
/// shares the watches, the check being brought back and the find jobs'
/// takers, which the worker's tasks (cloned from the runner) rely on.
#[derive(Clone)]
pub(super) struct ScreenTender {
    store: JobRun,
    /// What the find job reads of its item ([`ScreenTender::run_find`]).
    views: JobViews,
    area: ReceiveArea,
    clock: Clock,
    screens: ScreenStore,
    /// Brings a post to a site's check in the server browser and takes the
    /// file a person's pass downloads; `None`: no server browser.
    auth: Option<Arc<dyn AuthBrowser>>,
    /// The bound run and item each job's watch waits on
    /// ([`ScreenTender::tend`]).
    watching: Arc<Mutex<HashMap<String, (String, i64)>>>,
    /// The jobs whose check is being brought back to the page
    /// ([`ScreenTender::tend`]): one at a time for each.
    rearming: Arc<Mutex<HashSet<String>>>,
    /// By find job: held by whoever takes its staged downloads or ends it
    /// ([`ScreenTender::watch_find`]), so two of them never judge the same
    /// file.
    find_takes: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

/// A job's check being brought back to its page; the job is free again when
/// this goes, whatever way the task ended.
struct Rearming {
    jobs: Arc<Mutex<HashSet<String>>>,
    job: String,
}

impl Drop for Rearming {
    fn drop(&mut self) {
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.remove(&self.job);
        }
    }
}

/// What an item whose post needs a person's check in the server browser
/// came to.
pub(super) enum Check {
    /// The browser downloaded the post's file: receive it.
    Arrived(Box<PostFile>),
    /// Where the file should have come from answered with a page instead:
    /// the item fails with it.
    Refused(Failure),
    /// The item waits (its state is written).
    Settled,
    Interrupted,
}

impl ScreenTender {
    pub(super) fn new(
        store: JobRun,
        views: JobViews,
        area: ReceiveArea,
        clock: Clock,
    ) -> ScreenTender {
        ScreenTender {
            screens: ScreenStore::new(store.db().clone()),
            store,
            views,
            area,
            clock,
            auth: None,
            watching: Arc::default(),
            rearming: Arc::default(),
            find_takes: Arc::default(),
        }
    }

    /// The same tender bringing posts to a site's check with `browser`.
    pub(super) fn with_auth(mut self, browser: Arc<dyn AuthBrowser>) -> ScreenTender {
        self.auth = Some(browser);
        self
    }

    pub(super) fn screens(&self) -> &ScreenStore {
        &self.screens
    }

    fn now(&self) -> Millis {
        (self.clock)()
    }

    /// The browser run a job used goes, and its screen: the run ended, and no
    /// check of the job waits for a person.
    pub(super) async fn release(&self, job: &str) -> Result<(), JobError> {
        if let Some(browser) = &self.auth {
            browser.release(job).await;
        }
        self.screens.clear(job).await
    }

    /// The folder the file of an item's check goes to when the browser
    /// downloads it ([`super::Runner::tend_screens`]): the item's own, so a file
    /// that came stays for the item's next run, a restart in between too.
    pub(super) fn check_staging(&self, job: &str, item: i64) -> PathBuf {
        self.area.at(&format!(".tmp/check-{job}-{item}"))
    }

    /// An item whose post needs a person's check in the server browser (see
    /// the module docs): the file the check let the browser download, or the
    /// item brought to the check and waiting.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn through_check(
        &self,
        job: &str,
        item: &ItemRow,
        ep: &str,
        post: &Url,
        reason: String,
        page: &AuthPage,
        cancel: &CancellationToken,
    ) -> Result<Check, JobError> {
        let now = self.now();
        if let Some(staged) = auth::staged(&self.check_staging(job, item.id)).await {
            self.store
                .set_step(job, StepKind::Open, StepState::Done, None, now)
                .await?;
            self.store
                .set_step(job, StepKind::Auth, StepState::Done, None, now)
                .await?;
            return Ok(match staged {
                auth::Staged::File { name, path } => {
                    Check::Arrived(Box::new(auth::arrived(post, page, &name, path).await))
                }
                auth::Staged::Refused(failure) => Check::Refused(failure),
            });
        }
        let Some(browser) = &self.auth else {
            self.wait_check(
                job,
                item.id,
                Wait::Subtitle,
                NO_AUTH_BROWSER.to_owned(),
                None,
            )
            .await?;
            self.store
                .event(
                    job,
                    format!("{ep}: 사이트 확인에 서버 브라우저가 필요해요"),
                    Some(reason),
                    now,
                )
                .await?;
            return Ok(Check::Settled);
        };
        // One check of a job on the screen at a time.
        if let Some(bound) = self.screens.bound(job).await? {
            if bound.item_id != item.id && browser.is_live(job, &bound.run_id) {
                self.wait_check(job, item.id, Wait::Auth, OTHER_CHECK_FIRST.to_owned(), None)
                    .await?;
                return Ok(Check::Settled);
            }
        }

        self.store.set_stage(job, StepKind::Auth, now).await?;
        self.store
            .event(
                job,
                format!("{ep}: 서버 브라우저에서 사이트 확인을 준비해요"),
                Some(reason.clone()),
                now,
            )
            .await?;
        let prepared = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(Check::Interrupted),
            prepared = browser.prepare(auth::PrepareRequest { job, post, page }) => prepared,
        };
        let now = self.now();
        match prepared {
            Ok(prepared) => {
                self.screens
                    .bind(job, item.id, &prepared.run_id, &prepared.target_id, now)
                    .await?;
                self.wait_check(
                    job,
                    item.id,
                    Wait::Auth,
                    reason.clone(),
                    Some(reason.clone()),
                )
                .await?;
                self.store
                    .event(
                        job,
                        format!("{ep}: 사이트 확인이 필요해요. 작업 화면에서 확인해 주세요"),
                        Some(reason),
                        now,
                    )
                    .await?;
            }
            Err(failure) => {
                let note = format!("확인 화면을 준비하지 못했어요: {}", failure.reason);
                self.screens
                    .bind_failed(job, item.id, note.clone(), now)
                    .await?;
                self.wait_check(job, item.id, Wait::Auth, reason, Some(note))
                    .await?;
                self.store
                    .event(
                        job,
                        format!("{ep}: 사이트 확인 화면을 준비하지 못했어요"),
                        Some(described(&FileProblem::from(&failure))),
                        now,
                    )
                    .await?;
            }
        }
        Ok(Check::Settled)
    }

    /// An item of a post that needs a check waits for `wait` (`why`), the
    /// step `auth` with `note` when it is the check.
    async fn wait_check(
        &self,
        job: &str,
        item: i64,
        wait: Wait,
        why: String,
        note: Option<String>,
    ) -> Result<(), JobError> {
        let now = self.now();
        self.store
            .set_step(job, StepKind::Open, StepState::Done, None, now)
            .await?;
        if wait == Wait::Auth {
            self.store
                .set_step(job, StepKind::Auth, StepState::Waiting, note, now)
                .await?;
        }
        self.store
            .set_item(item, ItemState::Waiting, Some(wait), Some(why), now)
            .await
    }

    /// Watches the runs bound to jobs that wait for a person's check, and
    /// answers the requests to prepare their screens ([`super::Runner::tend_screens`]
    /// says how the worker calls it).
    pub(super) async fn tend(
        &self,
        wake: &Arc<Notify>,
        shutdown: &CancellationToken,
    ) -> Result<(), JobError> {
        // A find job a person asked to finish with no run bound: its run
        // ended (idle, a restart) before its watch ended it, it never had
        // one, or the worker has no server browser. Only the worker ends it,
        // after taking what a run left in its folder. One failing job does
        // not hold back the others.
        for job in self.store.unbound_finishes().await? {
            match self.end_unbound_find(&job).await {
                // What it kept is analysed next.
                Ok(true) => wake.notify_one(),
                Ok(false) => {}
                Err(err) => eprintln!("Subtitle job {job}: finishing: {err}"),
            }
        }
        let Some(browser) = &self.auth else {
            return Ok(());
        };
        for binding in self.screens.bindings().await? {
            let fresh = {
                let mut watching = self.watching.lock().expect("watching lock");
                let watched = (binding.run_id.clone(), binding.item_id);
                match watching.get(&binding.job_id) {
                    Some(now) if *now == watched => false,
                    _ => {
                        watching.insert(binding.job_id.clone(), watched);
                        true
                    }
                }
            };
            if fresh && binding.find {
                tokio::spawn(self.clone().watch_find(
                    binding,
                    browser.clone(),
                    wake.clone(),
                    shutdown.clone(),
                ));
            } else if fresh {
                tokio::spawn(self.clone().watch(
                    binding,
                    browser.clone(),
                    wake.clone(),
                    shutdown.clone(),
                ));
            }
        }
        for request in self.screens.prepare_requests().await? {
            let now = self.now();
            if request.restart {
                // A person asked to start the stuck run anew. A download on
                // its way is not lost for it: the request stays unanswered
                // until the download ended.
                if let Some(run) = request
                    .run_id
                    .as_deref()
                    .filter(|run| browser.is_live(&request.job_id, run))
                {
                    if browser.downloading(&request.job_id, run) {
                        continue;
                    }
                    // The run is over before the job goes back in line.
                    browser.release(&request.job_id).await;
                }
                if self
                    .screens
                    .requeue_for_check(&request.job_id, request.asked_at, true, self.now())
                    .await?
                {
                    println!(
                        "Subtitle job {}: the server browser is started anew at a person's request",
                        request.job_id
                    );
                    wake.notify_one();
                }
                continue;
            }
            // A person opened the screen: the live run's idle time counts
            // from now.
            let live = request
                .run_id
                .as_deref()
                .is_some_and(|run| browser.touch(&request.job_id, run));
            if live {
                self.screens
                    .mark_prepared(&request.job_id, request.asked_at, now)
                    .await?;
                // A check that went away while no one looked comes back. One
                // at a time for a job: a screen opened while the card is being
                // clicked waits for nothing and starts nothing. A find job has
                // no check: its page is the person's as they left it.
                if let Some(run) = request.run_id.clone().filter(|_| !request.find) {
                    let job = request.job_id.clone();
                    let started = self
                        .rearming
                        .lock()
                        .expect("rearming lock")
                        .insert(job.clone());
                    if started {
                        let guard = Rearming {
                            jobs: self.rearming.clone(),
                            job: job.clone(),
                        };
                        let browser = browser.clone();
                        tokio::spawn(async move {
                            browser.rearm(&job, &run).await;
                            drop(guard);
                        });
                    }
                }
            } else if self
                .screens
                .requeue_for_check(&request.job_id, request.asked_at, false, now)
                .await?
            {
                println!(
                    "Subtitle job {}: {} is brought to the screen again",
                    request.job_id,
                    if request.find {
                        "the creator's post"
                    } else {
                        "the site's check"
                    }
                );
                wake.notify_one();
            }
        }
        Ok(())
    }

    /// Waits on the bound run of a job for the file of its check, until the
    /// file arrives or the run ends.
    async fn watch(
        self,
        binding: screen::Binding,
        browser: Arc<dyn AuthBrowser>,
        wake: Arc<Notify>,
        shutdown: CancellationToken,
    ) {
        let (job, run, item) = (
            binding.job_id.as_str(),
            binding.run_id.as_str(),
            binding.item_id,
        );
        let staging = self.check_staging(job, binding.item_id);
        let _stop_following = self.follow_pages_of(&binding, &browser);
        loop {
            let waited = browser.wait_file(job, run, &staging).await;
            let now = self.now();
            match waited {
                Waited::File { name, path } => {
                    match self
                        .screens
                        .arrival(job, run, item, screen::FILE_ARRIVED, &name, now)
                        .await
                    {
                        Ok(Arrival::Taken) => {
                            println!("Subtitle job {job}: the site's check was passed");
                            wake.notify_one();
                        }
                        // The binding changed meanwhile, but the item has yet
                        // to receive its file: its next run takes it from
                        // the folder (`through_check`).
                        Ok(Arrival::Stale { item_open: true }) => {}
                        Ok(Arrival::Stale { item_open: false }) => {
                            let _ = tokio::fs::remove_file(&path).await;
                            let _ = tokio::fs::remove_dir_all(&staging).await;
                        }
                        Err(err) => {
                            eprintln!("Subtitle job {job}: the file of the site's check: {err}")
                        }
                    }
                    break;
                }
                Waited::Refused(failure) => {
                    // Kept for the item's next run, which fails with it.
                    if let Err(err) = auth::record_refusal(&staging, &failure).await {
                        eprintln!(
                            "Subtitle job {job}: the refusal of the site's check could not be kept: {}",
                            err.kind()
                        );
                        break;
                    }
                    let detail = described(&FileProblem::from(&failure));
                    match self
                        .screens
                        .arrival(job, run, item, screen::FILE_REFUSED, &detail, now)
                        .await
                    {
                        Ok(Arrival::Taken) => {
                            println!(
                                "Subtitle job {job}: the site's check was passed but its file was refused"
                            );
                            wake.notify_one();
                        }
                        Ok(Arrival::Stale { item_open: true }) => {}
                        Ok(Arrival::Stale { item_open: false }) => {
                            let _ = tokio::fs::remove_dir_all(&staging).await;
                        }
                        Err(err) => {
                            eprintln!("Subtitle job {job}: the refusal of the site's check: {err}")
                        }
                    }
                    break;
                }
                Waited::NotTaken { reason } => {
                    let _ = self
                        .store
                        .event(
                            job,
                            "브라우저가 파일을 받지 못했어요".to_owned(),
                            Some(reason),
                            now,
                        )
                        .await;
                }
                // The worker's shutdown ended the run: the next start closes
                // the screen with its own note (`WORKER_RESTARTED`).
                Waited::Ended if shutdown.is_cancelled() => break,
                Waited::Ended => {
                    if let Ok(true) = self.screens.unbind(job, run, screen::RUN_ENDED, now).await {
                        let _ = self
                            .store
                            .event(
                                job,
                                "사이트 확인을 기다리던 서버 브라우저가 닫혔어요".to_owned(),
                                Some(screen::RUN_ENDED.to_owned()),
                                now,
                            )
                            .await;
                    }
                    break;
                }
            }
        }
        let mut watching = self.watching.lock().expect("watching lock");
        if watching
            .get(job)
            .is_some_and(|(r, i)| r == run && *i == item)
        {
            watching.remove(job);
        }
    }
}
