//! A find job ([`crate::store::FIND`]): the job has no source to read. Its one item stands
//! for the package, its post the chosen creator's newest post. The runner
//! opens that post in the job's server browser run, clicking nothing
//! ([`AuthPage::Browse`]), binds the run to the job's remote screen and lets
//! the job wait (`waiting`, `auth`, so the screen machinery of a site's check
//! serves it as it is). A person browses there: past posts, popups, and any
//! check the site puts up, which only they pass.
//!
//! The job's watch ([`ScreenTender::watch_find`]) takes every download the run
//! completes: each is judged as an uploaded file is ([`sort_arrival`]) and
//! kept as a file of the job's package, or dropped with why. One taker at a
//! time for a job ([`ScreenTender::find_take`]): the watch, a next run's start and
//! the end of a job with no run never judge the same staged file. The screen
//! follows the pages of the run like a site's check does ([`super::pages`]):
//! a post a popup opens is the one a person sees, and a person may switch
//! between the pages and close any but the post the run opened.
//! The run ends as any other: an idle run closes and the files received stay;
//! a person reopening the screen puts the job back in line and its next run
//! opens the same post anew.
//!
//! A person ends the job (`받기 끝내기`, [`crate::JobRequests::ask_finish`]): the web
//! only writes the request, since a download a run left in the job's folder
//! is seen by the worker alone. The watch ends a job with a run once no
//! download of the run is on its way: a download under way is waited for (it
//! ends within the browser's stall and size limits), never cut off. A job
//! with no run bound is ended by the worker's next look at the screens
//! ([`crate::Runner::tend_screens`], with or without a server browser) or by its run
//! starting. Either end takes what is in the job's folder first.

use std::{path::Path, sync::Arc, time::Duration};

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use trss_subtitles::{
    auth::{self, AuthBrowser, AuthPage, Waited},
    upload::INFLATE_BUDGET,
};
use url::Url;

use super::ScreenTender;
use crate::runner::{described, NO_AUTH_BROWSER};
use crate::{
    model::{ItemState, JobState, StepKind, StepState, Wait},
    screen,
    store::{FileProblem, JobError},
    upload::{sort_arrival, Arrived, Earlier},
};

/// What a find job waits for while its run is open: a person browsing it.
pub const FIND_NOTE: &str = "원격 화면에서 게시물을 찾아 첨부 파일을 받아 주세요";

/// How often the watch looks for a request to finish.
const FINISH_POLL: Duration = Duration::from_millis(500);

impl ScreenTender {
    /// One run of the find job `id` (see the module docs). Returns whether
    /// the run ended (the job waits or is done).
    pub(in crate::runner) async fn run_find(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<bool, JobError> {
        let Some(item) = self.views.items(id).await?.into_iter().next() else {
            return Ok(false);
        };
        {
            let _taking = self.find_take(id).await;
            let mut budget = INFLATE_BUDGET;
            // What a run downloaded before a restart is the job's too.
            self.take_found(id, item.id, &mut budget).await?;
            if self.store.finish_asked(id).await? {
                if let Some(browser) = &self.auth {
                    browser.release(id).await;
                }
                self.screens.clear(id).await?;
                self.store.end_find(id, None, self.now()).await?;
                self.forget_staging(id, item.id).await;
                return Ok(true);
            }
        }
        let now = self.now();
        self.store
            .set_item(item.id, ItemState::Running, None, None, now)
            .await?;
        let post = Url::parse(&item.post_url).ok();
        let host = post.as_ref().and_then(|p| p.host_str().map(str::to_owned));
        let (Some(post), Some(browser)) = (post, &self.auth) else {
            // Nothing to open, or nothing to open it with: the job waits for
            // a server browser, as a post that needs a check does.
            self.store
                .set_item(
                    item.id,
                    ItemState::Waiting,
                    Some(Wait::Subtitle),
                    Some(NO_AUTH_BROWSER.to_owned()),
                    now,
                )
                .await?;
            self.store
                .settle(
                    id,
                    JobState::Waiting,
                    Some(Wait::Subtitle),
                    Some(NO_AUTH_BROWSER.to_owned()),
                    now,
                )
                .await?;
            self.store
                .event(
                    id,
                    "직접 찾기에 서버 브라우저가 필요해요".to_owned(),
                    host,
                    now,
                )
                .await?;
            return self.after_find_settled(id).await;
        };

        self.store.set_stage(id, StepKind::Open, now).await?;
        self.store.begin_step(id, StepKind::Open, now).await?;
        self.store
            .event(
                id,
                "서버 브라우저에서 제작자의 게시물을 열어요".to_owned(),
                host,
                now,
            )
            .await?;
        let page = AuthPage::Browse;
        let prepared = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(false),
            prepared = browser.prepare(auth::PrepareRequest { job: id, post: &post, page: &page }) => prepared,
        };
        let now = self.now();
        let note = match prepared {
            Ok(prepared) => {
                self.screens
                    .bind(id, item.id, &prepared.run_id, &prepared.target_id, now)
                    .await?;
                self.store
                    .set_step(id, StepKind::Open, StepState::Done, None, now)
                    .await?;
                self.store
                    .set_step(id, StepKind::Receive, StepState::Current, None, now)
                    .await?;
                self.store
                    .event(
                        id,
                        "원격 화면에서 게시물을 볼 수 있어요".to_owned(),
                        None,
                        now,
                    )
                    .await?;
                FIND_NOTE.to_owned()
            }
            Err(failure) => {
                let note = format!("원격 화면을 준비하지 못했어요: {}", failure.reason);
                self.screens
                    .bind_failed(id, item.id, note.clone(), now)
                    .await?;
                self.store
                    .set_step(
                        id,
                        StepKind::Open,
                        StepState::Waiting,
                        Some(note.clone()),
                        now,
                    )
                    .await?;
                self.store
                    .event(
                        id,
                        "직접 찾기 화면을 준비하지 못했어요".to_owned(),
                        Some(described(&FileProblem::from(&failure))),
                        now,
                    )
                    .await?;
                note
            }
        };
        self.store
            .set_item(
                item.id,
                ItemState::Waiting,
                Some(Wait::Auth),
                Some(note.clone()),
                now,
            )
            .await?;
        self.store
            .settle(id, JobState::Waiting, Some(Wait::Auth), Some(note), now)
            .await?;
        self.after_find_settled(id).await
    }

    /// A finish asked while the run was being prepared: a job with no run
    /// bound ends now (it would at the next look at the screens too), one
    /// with a run is its watch's to end.
    async fn after_find_settled(&self, id: &str) -> Result<bool, JobError> {
        if self.store.finish_asked(id).await? {
            self.end_unbound_find(id).await?;
        }
        Ok(true)
    }

    /// Ends the find job `job` that a person asked to finish and that has no
    /// run bound, after taking what its last run left in its folder. Says
    /// whether it ended (a job that kept files is back in line).
    pub(super) async fn end_unbound_find(&self, job: &str) -> Result<bool, JobError> {
        let _taking = self.find_take(job).await;
        let Some(found) = self.store.found(job).await? else {
            return Ok(false);
        };
        let mut budget = INFLATE_BUDGET;
        self.take_found(job, found.item_id, &mut budget).await?;
        let ended = self.store.end_find(job, None, self.now()).await?;
        if ended {
            println!("Subtitle job {job}: finished receiving");
            if let Some(browser) = &self.auth {
                browser.release(job).await;
            }
            self.forget_staging(job, found.item_id).await;
        }
        Ok(ended)
    }

    /// The job ended: its folder of downloads goes (with what its last
    /// answer said), and its taker's lock.
    async fn forget_staging(&self, job: &str, item: i64) {
        let _ = tokio::fs::remove_dir_all(self.check_staging(job, item)).await;
        self.find_takes.lock().expect("find takes lock").remove(job);
    }

    /// Waits for the find job `job`'s staged downloads to be the caller's
    /// alone: held while taking them ([`ScreenTender::take_found`],
    /// [`ScreenTender::take_one`]) and while ending the job.
    async fn find_take(&self, job: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self
            .find_takes
            .lock()
            .expect("find takes lock")
            .entry(job.to_owned())
            .or_default()
            .clone();
        lock.lock_owned().await
    }

    /// Takes the files a run left in the item's folder (a restart cut its
    /// watch short after the download moved). The caller holds
    /// [`ScreenTender::find_take`].
    async fn take_found(&self, job: &str, item: i64, budget: &mut u64) -> Result<(), JobError> {
        let staging = self.check_staging(job, item);
        let Ok(mut entries) = tokio::fs::read_dir(&staging).await else {
            return Ok(());
        };
        let mut files = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with('.') && entry.file_type().await.is_ok_and(|t| t.is_file()) {
                files.push((name, entry.path()));
            }
        }
        files.sort();
        for (name, path) in files {
            self.take_one(job, &path, &name, budget).await?;
        }
        let _ = tokio::fs::remove_file(staging.join(auth::ANSWER)).await;
        Ok(())
    }

    /// Judges the file the run downloaded to `path`, named `name`, and
    /// records it as kept or dropped. The caller holds [`ScreenTender::find_take`].
    async fn take_one(
        &self,
        job: &str,
        path: &Path,
        name: &str,
        budget: &mut u64,
    ) -> Result<(), JobError> {
        let Some(found) = self.store.found(job).await? else {
            let _ = tokio::fs::remove_file(path).await;
            return Ok(());
        };
        let earlier = Earlier::of(&found);
        let n = found.kept.len();
        let (area, id, staged, raw, left) = (
            self.area.clone(),
            job.to_owned(),
            path.to_owned(),
            name.to_owned(),
            *budget,
        );
        let sorted = tokio::task::spawn_blocking(move || {
            let mut left = left;
            let sorted = sort_arrival(&area, &id, &staged, &raw, &earlier, n, &mut left);
            (sorted, left)
        })
        .await;
        let now = self.now();
        match sorted {
            Ok((Ok(Arrived::Kept(file)), left)) => {
                *budget = left;
                println!("Subtitle job {job}: the server browser's download was kept");
                self.store.add_found(job, found.item_id, file, now).await
            }
            Ok((Ok(Arrived::Dropped(dropped)), left)) => {
                *budget = left;
                println!("Subtitle job {job}: the server browser's download was dropped");
                self.store.add_dropped(job, dropped, now).await
            }
            Ok((Err(err), _)) => {
                eprintln!("Subtitle job {job}: a download could not be taken: {err}");
                let _ = tokio::fs::remove_file(path).await;
                self.store
                    .event(
                        job,
                        "서버 브라우저가 받은 파일을 남기지 못했어요".to_owned(),
                        Some(name.to_owned()),
                        now,
                    )
                    .await
            }
            Err(_) => {
                eprintln!("Subtitle job {job}: judging a download broke off");
                let _ = tokio::fs::remove_file(path).await;
                self.store
                    .event(
                        job,
                        "서버 브라우저가 받은 파일을 확인하지 못했어요".to_owned(),
                        Some(name.to_owned()),
                        now,
                    )
                    .await
            }
        }
    }

    /// Waits on the run bound to the find job for its downloads, until the
    /// run ends or a person finishes the job (see the module docs).
    pub(super) async fn watch_find(
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
        let staging = self.check_staging(job, item);
        let mut budget = INFLATE_BUDGET;
        {
            let _taking = self.find_take(job).await;
            if let Err(err) = self.take_found(job, item, &mut budget).await {
                eprintln!("Subtitle job {job}: the files left by the run: {err}");
            }
        }
        let _stop_following = self.follow_pages_of(&binding, &browser);
        loop {
            let next = tokio::select! {
                biased;
                waited = browser.wait_file(job, run, &staging) => Some(waited),
                () = self.finish_ready(job, run, browser.as_ref()) => None,
            };
            let now = self.now();
            match next {
                Some(Waited::File { name, path }) => {
                    let _taking = self.find_take(job).await;
                    if let Err(err) = self.take_one(job, &path, &name, &mut budget).await {
                        eprintln!("Subtitle job {job}: a download of the run: {err}");
                    }
                    let _ = tokio::fs::remove_file(staging.join(auth::ANSWER)).await;
                }
                Some(Waited::NotTaken { reason }) => {
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
                Some(Waited::Refused(failure)) => {
                    let _ = self
                        .store
                        .event(
                            job,
                            "파일 대신 웹 페이지가 왔어요".to_owned(),
                            Some(described(&FileProblem::from(&failure))),
                            now,
                        )
                        .await;
                }
                Some(Waited::Ended) if shutdown.is_cancelled() => break,
                Some(Waited::Ended) => {
                    if let Ok(true) = self.screens.unbind(job, run, screen::RUN_ENDED, now).await {
                        let _ = self
                            .store
                            .event(
                                job,
                                "직접 찾기 서버 브라우저가 닫혔어요".to_owned(),
                                Some(screen::RUN_ENDED.to_owned()),
                                now,
                            )
                            .await;
                    }
                    break;
                }
                None => {
                    let _taking = self.find_take(job).await;
                    // A download the browser moved in after the finish found
                    // none on its way is the job's too, and the folder goes
                    // with the job's end.
                    if let Err(err) = self.take_found(job, item, &mut budget).await {
                        // The next watch of the run takes it and ends the job.
                        eprintln!("Subtitle job {job}: finishing: {err}");
                        break;
                    }
                    match self.store.end_find(job, Some(run), self.now()).await {
                        Ok(true) => {
                            println!("Subtitle job {job}: finished receiving");
                            browser.release(job).await;
                            self.forget_staging(job, item).await;
                            // What it kept is analysed next.
                            wake.notify_one();
                        }
                        Ok(false) => {}
                        Err(err) => eprintln!("Subtitle job {job}: finishing: {err}"),
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

    /// Returns once a person asked the job to finish and no download of the
    /// run `run` is on its way. Gives up nothing when dropped.
    async fn finish_ready(&self, job: &str, run: &str, browser: &dyn AuthBrowser) {
        loop {
            if let Ok(true) = self.store.finish_asked(job).await {
                if !browser.downloading(job, run) {
                    return;
                }
            }
            tokio::time::sleep(FINISH_POLL).await;
        }
    }
}
