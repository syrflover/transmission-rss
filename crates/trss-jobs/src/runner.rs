//! Carrying out the jobs (`docs/specs/jobs.md`, 체크포인트와 중단 복구).
//!
//! # Order
//!
//! The runner takes the oldest ready job and carries out its items one after
//! the other, then the next job: one job at a time per worker. An item goes
//! through `open` (the source reads the post), `auth` when the site asks a
//! person first, and `receive` (each file the post offers). The job's state
//! follows from its items' when the run ends ([`settle`]).
//!
//! - A post no source of this build reads waits for one (`자막 대기`); the
//!   worker puts such items back in line when it starts
//!   ([`Runner::requeue_waiting_for_sources`]). So does a post whose source
//!   finds its subtitle somewhere it cannot read yet (a Google Drive folder),
//!   with the source's reason, a job whose received files wait for their
//!   work folder to be there again (`영상 대기`, [`crate::place`]), and one
//!   whose archive waits for the next try to unpack it, which also goes back
//!   in line an hour after the failed try
//!   ([`Runner::requeue_unpack_retries`], [`crate::place::unpack`]).
//! - A post whose subtitle is in WinPNG images ([`Opened::WinPng`]) is read by
//!   the runner's [`WinpngReader`] (a server browser; [`Runner::with_winpng`]):
//!   the files it takes out are put in a folder of the job's, which is the
//!   step `open`, and are then received like any other file, under their
//!   folders ([`crate::store::FileRow::folder`]). A post whose images hold no subtitle fails
//!   as `no_subtitle`, and one whose image needs a key as `needs_input`.
//!   A runner with no reader leaves the item waiting for a source, as for a
//!   Drive folder. The reader's browser run is let go when a run ends, unless
//!   the job waits for a person's check on the site.
//! - The source opens the post for the item's episode: a post that says which
//!   file is which episode (Blogger's Drive links) offers that episode's.
//! - A site that asks for a person's check makes the item wait (`인증 필요`).
//!   A post whose check a person passes in the server browser
//!   ([`Opened::BrowserAuth`]) is first brought to the check by the runner's
//!   [`AuthBrowser`] ([`Runner::with_auth`]), and the run that shows it is
//!   bound to the job ([`crate::screen`]); the worker watches that run
//!   ([`Runner::tend_screens`]) and the job goes back in line when the person
//!   has passed the check. The care of the screen (the check, the watch, the
//!   pages, the end of the run) is the `tender` module's. Without an
//!   [`AuthBrowser`] such a post waits for a source.
//!   A site's check that needs no browser (a protected post) only waits.
//! - A find job ([`crate::store::FIND`]) has no source to read: the browser
//!   opens its creator's post for a person to browse on the job's remote
//!   screen, and every file the run downloads becomes a file of the job's
//!   one package, judged as an upload's files are (see [`find`]).
//! - A file the job received for one item is not received again for another:
//!   the second item's receipt names the first (`same_as`).
//! - A network failure while opening a post or receiving a file is tried
//!   again, after each of [`RETRY_WAITS`], before the item fails. The other
//!   failure classes are not (`docs/specs/jobs.md`, 공통 수신 결과와 실패
//!   분류); an expired address is the source's to read again.
//! - A failed item and a failed receipt keep the class of their failure, and
//!   a receipt the answer's status, media type and size.
//! - An item of a job that receives a revision (`revision_of`) whose files
//!   are, by key, the same bytes (SHA-256) as the earlier receipt's records
//!   that there is nothing to replace ([`JobRun::finish_item`]); no
//!   replacement is to be approved for it.
//!
//! # One receipt, and a restart
//!
//! Each effect on a file is preceded by a record of its intent and followed by
//! a record of its result, and a worker that dies leaves its job `running`:
//! the next one to hold the worker lock claims it again
//! ([`JobRun::claim_next`]). The steps of a receipt, and what a restart does
//! with an unfinished one (the table of records against the disk), are the
//! `receive` module's.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use trss_core::{episode::episode_label, Clock, Millis};
use trss_subtitles::{
    auth::AuthBrowser,
    winpng::{self, ViewRequest, Viewed, WinpngReader},
    Failure, FailureKind, Opened, Snapshot, Sources,
};
use url::Url;

use crate::{
    area::ReceiveArea,
    model::{FileState, ItemState, JobState, StepKind, StepState, Wait},
    place::{PlaceStore, Placement, Placer},
    screen::ScreenStore,
    store::{FileProblem, ItemRow, JobError, JobRun, JobViews, FIND, RELOCATE, UPLOAD},
};

mod receive;
mod tender;

use receive::{Receipt, Receiver};
pub use tender::find;
use tender::{Check, ScreenTender};

/// What the job's log and screen say for a post no source reads.
pub const NO_SOURCE: &str = "이 출처에서 받는 방법을 아직 몰라요";

/// What an item whose post needs a person's check in the server browser
/// says when the worker has no server browser.
pub const NO_AUTH_BROWSER: &str =
    "사이트 확인에 서버 브라우저가 필요한데, 작업기에 서버 브라우저가 없어요";

/// What an item says while another item of its job has its check on the
/// screen: a job has one browser run, which shows one check at a time.
pub const OTHER_CHECK_FIRST: &str = "같은 작업의 다른 회차 사이트 확인을 먼저 마쳐야 해요";

/// How many times a job is started without a run ending before the runner
/// holds it instead: a job whose runs keep stopping in an error or a crash
/// would otherwise block the line for good. A run that ends, waiting or not,
/// starts the count again.
pub const MAX_STARTS: i64 = 5;

/// How long the runner waits before it tries a post or a file again after a
/// network failure, one wait per try: two more tries, then the item fails.
pub const RETRY_WAITS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(10)];

/// Carries out jobs. Cheap to clone.
#[derive(Clone)]
pub struct Runner {
    store: JobRun,
    /// What the runner reads of the job's items.
    views: JobViews,
    /// The outcome note of a relocation job ([`PlaceStore::relocation_note`]).
    place: PlaceStore,
    /// Stores and applies what the items received ([`crate::place`]).
    placer: Placer,
    sources: Sources,
    /// Where a reading of images puts its files ([`Runner::staging`]).
    area: ReceiveArea,
    clock: Clock,
    /// Receives, publishes and recovers the files of an item.
    receiver: Receiver,
    /// Reads the WinPNG images of a post; `None`: the worker has no server
    /// browser.
    winpng: Option<Arc<dyn WinpngReader>>,
    /// Cares for the remote screens: the checks, their watches, the find
    /// jobs.
    tender: ScreenTender,
    /// The library's generation the last look for arrived videos saw
    /// ([`Runner::requeue_awaiting_video`]).
    video_seen: Arc<Mutex<Option<i64>>>,
}

/// What a failure says in the log: its class, then its reason.
fn described(problem: &FileProblem) -> String {
    match problem.class {
        Some(class) => format!("{} · {}", class.label(), problem.reason),
        None => problem.reason.clone(),
    }
}

/// `2초`: a wait as the log says it.
fn wait_text(wait: Duration) -> String {
    format!("{}초", wait.as_secs_f64().ceil() as u64)
}

/// Waits `wait`, or less when `cancel` fires: whether it waited it out.
async fn pause(wait: Duration, cancel: &CancellationToken) -> bool {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => false,
        _ = tokio::time::sleep(wait) => true,
    }
}

/// How a job stands from its items alone ([`Runner::received`]).
struct Received {
    state: JobState,
    wait: Option<Wait>,
    note: Option<String>,
    message: &'static str,
    /// The log line's detail when it is not the note.
    detail: Option<String>,
}

/// How one item came out.
enum ItemEnd {
    Settled,
    Interrupted,
}

impl Runner {
    pub fn new(store: JobRun, sources: Sources, area: ReceiveArea, clock: Clock) -> Runner {
        let placer = Placer::new(store.clone(), area.clone(), clock.clone());
        let views = JobViews::new(store.db().clone());
        Runner {
            receiver: Receiver::new(store.clone(), placer.clone(), area.clone(), clock.clone()),
            tender: ScreenTender::new(store.clone(), views.clone(), area.clone(), clock.clone()),
            placer,
            sources,
            area,
            clock,
            winpng: None,
            video_seen: Arc::default(),
            views,
            place: PlaceStore::new(store.db().clone()),
            store,
        }
    }

    /// The same runner unpacking received archives with `unpacker`
    /// ([`crate::place::unpack`]); without one, a package with an archive
    /// waits.
    pub fn with_unpacker(mut self, unpacker: trss_archive::run::Unpacker) -> Runner {
        self.placer = self.placer.with_unpacker(unpacker);
        self
    }

    /// The same runner bringing posts to a site's check with `browser`.
    pub fn with_auth(mut self, browser: Arc<dyn AuthBrowser>) -> Runner {
        self.tender = self.tender.with_auth(browser);
        self
    }

    pub fn screens(&self) -> &ScreenStore {
        self.tender.screens()
    }

    /// The same runner reading WinPNG images with `reader`.
    pub fn with_winpng(mut self, reader: Arc<dyn WinpngReader>) -> Runner {
        self.winpng = Some(reader);
        self
    }

    /// The same runner with other waits before a retry ([`RETRY_WAITS`]); as
    /// many retries as waits.
    pub fn with_retry_waits(mut self, waits: Vec<Duration>) -> Runner {
        self.receiver = self.receiver.with_retry_waits(waits);
        self
    }

    /// The sources this runner reads. The recheck ([`crate::recheck`]) asks the
    /// same ones, so its requests share their pace per host.
    pub fn sources(&self) -> &Sources {
        &self.sources
    }

    pub fn clock(&self) -> &Clock {
        &self.clock
    }

    fn now(&self) -> Millis {
        (self.clock)()
    }

    /// Puts the jobs that wait for a source back in line (see the module docs).
    pub async fn requeue_waiting_for_sources(&self) -> Result<usize, JobError> {
        self.store.requeue_waiting_for_sources(self.now()).await
    }

    /// Puts the jobs waiting for a video (`영상 대기`) back in line once the
    /// library has a video for an episode one of their rows waits on; looks
    /// only when the library changed since the last look.
    pub async fn requeue_awaiting_video(&self) -> Result<usize, JobError> {
        let seen = *self.video_seen.lock().expect("not poisoned");
        let (generation, requeued) = self.store.requeue_awaiting_video(seen, self.now()).await?;
        *self.video_seen.lock().expect("not poisoned") = Some(generation);
        Ok(requeued)
    }

    /// Puts the jobs whose archive waits for its next try to unpack it back
    /// in line once the try is due ([`crate::place::unpack`]). A runner that
    /// does not unpack leaves them: its runs would not try.
    pub async fn requeue_unpack_retries(&self) -> Result<usize, JobError> {
        if !self.placer.unpacks() {
            return Ok(0);
        }
        self.store.requeue_unpack_retries(self.now()).await
    }

    pub async fn has_ready(&self) -> Result<bool, JobError> {
        self.store.has_ready().await
    }

    /// Whether a person's cleanup of stored files waits for the worker
    /// ([`crate::place::cleanup`]).
    pub async fn has_cleanups(&self) -> Result<bool, JobError> {
        self.placer.has_cleanups().await
    }

    /// Carries out the asked cleanups of stored files
    /// ([`crate::place::cleanup`]). The caller holds the worker lock and
    /// calls it from the task that runs the jobs, between runs: no store or
    /// link of a job comes between a cleanup's look at a file and its
    /// removal. Returns how many ended.
    pub async fn run_cleanups(&self) -> Result<usize, JobError> {
        self.placer.run_cleanups().await
    }

    /// Watches the runs bound to jobs that wait for a person's check, and
    /// answers the requests to prepare their screens (see the module docs).
    /// A file that arrives, or a job put back in line, notifies `wake` (the
    /// worker's job loop). A person's request to start a run anew ends that
    /// run first (never while it has a download on its way: the request
    /// waits for a later round), and only then puts the job back in line, so
    /// the next run of the job cannot take the same run back. The worker
    /// calls it whenever it is woken and every few seconds. Without an [`AuthBrowser`] it only ends the find jobs a
    /// person asked to finish that have no run bound. `shutdown` is the
    /// worker's: a run that ends after it fired was ended by the shutdown,
    /// which closes no screen (the next start does).
    pub async fn tend_screens(
        &self,
        wake: &Arc<Notify>,
        shutdown: &CancellationToken,
    ) -> Result<(), JobError> {
        self.tender.tend(wake, shutdown).await
    }

    /// Runs the ready jobs one after the other until none is left or `cancel`
    /// fires. Returns how many runs ended. The caller holds the worker lock.
    pub async fn run_ready(&self, cancel: &CancellationToken) -> Result<usize, JobError> {
        let mut ran = 0;
        while !cancel.is_cancelled() {
            let Some((id, resumed, starts)) = self.store.claim_next(self.now()).await? else {
                break;
            };
            if starts > MAX_STARTS {
                let note = format!("{MAX_STARTS}번 시작했지만 한 번도 끝내지 못했어요");
                self.store.hold_stuck(&id, note.clone(), self.now()).await?;
                self.store
                    .event(&id, "보류했어요".to_owned(), Some(note), self.now())
                    .await?;
                continue;
            }
            if self.run_job(&id, resumed, cancel).await? {
                ran += 1;
            }
        }
        Ok(ran)
    }

    /// One run of a claimed job. Returns whether it ended (and was settled).
    async fn run_job(
        &self,
        id: &str,
        resumed: bool,
        cancel: &CancellationToken,
    ) -> Result<bool, JobError> {
        let items = self.views.items(id).await?;
        let received = !items
            .iter()
            .any(|i| matches!(i.state, ItemState::Pending | ItemState::Running));
        let origin = self.store.origin(id).await?;
        let upload = origin.as_deref() == Some(UPLOAD);
        let find = origin.as_deref() == Some(FIND);
        let relocation = origin.as_deref() == Some(RELOCATE);
        // A find job is received once a person ended it: its one item is
        // done.
        let found = find && items.iter().all(|i| i.state == ItemState::Done);
        let confirmed =
            (upload || found || relocation) && self.store.placement_confirmed(id).await?;
        let message = match (resumed, received) {
            (true, _) => "멈췄던 작업을 이어가요",
            (false, _) if relocation && confirmed => "확인한 재배치로 적용본을 옮겨요",
            (false, _) if relocation => "재배치 계획을 살펴봐요",
            (false, true) if confirmed => "배치를 확인한 파일을 보관하고 적용해요",
            // An upload or a found package: analysed for a person's
            // 배치 확인.
            (false, true) if upload || found => "받은 파일의 배치를 준비해요",
            // Received before storing was made: stored and applied now.
            (false, true) => "받아 둔 파일의 보관과 적용을 이어가요",
            (false, false) => "작업을 시작했어요",
        };
        println!("Subtitle job {id}: {message}");
        self.store
            .event(id, message.to_owned(), None, self.now())
            .await?;
        if find && !found {
            return self.tender.run_find(id, cancel).await;
        }

        for item in items {
            if !matches!(item.state, ItemState::Pending | ItemState::Running) {
                continue;
            }
            if cancel.is_cancelled() {
                return Ok(false);
            }
            let end = self.run_item(id, item, cancel).await;
            // What a reading of images left in the job's staging folder is
            // not the next item's.
            let _ = tokio::fs::remove_dir_all(self.staging(id)).await;
            if let ItemEnd::Interrupted = end? {
                return Ok(false);
            }
        }
        // An upload's or a find job's note says what it kept, whatever a
        // wait said in between.
        let kept = match upload || find {
            true => Some(self.store.kept_note(id).await?),
            false => None,
        };
        let mut received = self.received(id, kept).await?;
        let Some(placement) = self.placer.run(id, cancel).await? else {
            return Ok(false);
        };
        // A relocation's note says what came of the copies it moved.
        if relocation {
            received.note = self.place.relocation_note(id).await?;
        }
        self.settle(id, received, placement).await?;
        Ok(true)
    }

    async fn run_item(
        &self,
        job: &str,
        item: ItemRow,
        cancel: &CancellationToken,
    ) -> Result<ItemEnd, JobError> {
        let ep = episode_label(&item.episode);
        let now = self.now();
        self.store
            .set_item(item.id, ItemState::Running, None, None, now)
            .await?;

        // What an earlier start left unfinished is compared with the disk
        // first, whatever the post says now: a failure whose bytes were not
        // yet gone is finished here too, even when the post no longer opens.
        for unfinished in item.files.iter().filter(|r| {
            matches!(r.state, FileState::Intended | FileState::Fetched)
                || (r.state == FileState::Failed && r.path.is_some())
        }) {
            if let Some(reason) = self.receiver.recover(job, &ep, unfinished).await? {
                let now = self.now();
                self.store
                    .set_step(
                        job,
                        StepKind::Receive,
                        StepState::Waiting,
                        Some(reason.clone()),
                        now,
                    )
                    .await?;
                self.store
                    .set_item(item.id, ItemState::Held, None, Some(reason), now)
                    .await?;
                return Ok(ItemEnd::Settled);
            }
        }

        let fail = |reason: String, class: Option<FailureKind>| async move {
            let now = self.now();
            self.store.fail_item(item.id, reason, class, now).await
        };
        let Ok(post) = Url::parse(&item.post_url) else {
            fail("게시물 주소를 읽지 못했어요".to_owned(), None).await?;
            return Ok(ItemEnd::Settled);
        };
        let host = post.host_str().map(str::to_owned);
        let Some(source) = self.sources.for_post(&post) else {
            self.store
                .set_item(
                    item.id,
                    ItemState::Waiting,
                    Some(Wait::Subtitle),
                    Some(NO_SOURCE.to_owned()),
                    now,
                )
                .await?;
            self.store
                .event(job, format!("{ep}: {NO_SOURCE}"), host, now)
                .await?;
            return Ok(ItemEnd::Settled);
        };

        self.store.set_stage(job, StepKind::Open, now).await?;
        self.store.begin_step(job, StepKind::Open, now).await?;
        let mut tries = 0;
        let opened = loop {
            let opened = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Ok(ItemEnd::Interrupted),
                opened = source.open(&post, &item.episode) => opened,
            };
            // The images of the post, read by the browser, are the files it
            // offers.
            let opened = match opened {
                Ok(Opened::WinPng {
                    snapshot,
                    elsewhere,
                }) => tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Ok(ItemEnd::Interrupted),
                    opened = self.read_winpng(job, &post, &snapshot, elsewhere.as_deref()) => opened,
                },
                other => other,
            };
            match opened {
                Err(failure)
                    if failure.kind.retryable() && tries < self.receiver.retry_waits().len() =>
                {
                    let wait = self.receiver.retry_waits()[tries];
                    tries += 1;
                    self.store
                        .event(
                            job,
                            format!("{ep}: 게시물을 열지 못해 다시 시도해요"),
                            Some(format!(
                                "{} · {} 뒤 ({tries}/{})",
                                described(&FileProblem::from(&failure)),
                                wait_text(wait),
                                self.receiver.retry_waits().len()
                            )),
                            self.now(),
                        )
                        .await?;
                    if !pause(wait, cancel).await {
                        return Ok(ItemEnd::Interrupted);
                    }
                }
                opened => break opened,
            }
        };
        // The folder of a file a person's check let the browser download,
        // which goes once the item has received it.
        let mut checked = None;
        let files = match opened {
            Err(failure) => {
                let message = match failure.kind {
                    FailureKind::NoSubtitle | FailureKind::NeedsInput => {
                        "게시물의 이미지에서 자막을 꺼내지 못했어요"
                    }
                    _ => "게시물을 열지 못했어요",
                };
                self.store
                    .event(
                        job,
                        format!("{ep}: {message}"),
                        Some(described(&FileProblem::from(&failure))),
                        self.now(),
                    )
                    .await?;
                fail(failure.reason, Some(failure.kind)).await?;
                return Ok(ItemEnd::Settled);
            }
            Ok(Opened::NeedsAuth { reason }) => {
                let now = self.now();
                self.store
                    .set_step(job, StepKind::Open, StepState::Done, None, now)
                    .await?;
                self.store
                    .set_step(
                        job,
                        StepKind::Auth,
                        StepState::Waiting,
                        Some(reason.clone()),
                        now,
                    )
                    .await?;
                self.store
                    .set_item(
                        item.id,
                        ItemState::Waiting,
                        Some(Wait::Auth),
                        Some(reason.clone()),
                        now,
                    )
                    .await?;
                self.store
                    .event(
                        job,
                        format!("{ep}: 사이트 확인이 필요해요"),
                        Some(reason),
                        now,
                    )
                    .await?;
                return Ok(ItemEnd::Settled);
            }
            Ok(Opened::Elsewhere { reason }) => {
                let now = self.now();
                self.store
                    .set_step(job, StepKind::Open, StepState::Done, None, now)
                    .await?;
                self.store
                    .set_item(
                        item.id,
                        ItemState::Waiting,
                        Some(Wait::Subtitle),
                        Some(reason.clone()),
                        now,
                    )
                    .await?;
                self.store
                    .event(
                        job,
                        format!("{ep}: 자막이 아직 받을 수 없는 곳에 있어요"),
                        Some(reason),
                        now,
                    )
                    .await?;
                return Ok(ItemEnd::Settled);
            }
            Ok(Opened::Files(files)) => files,
            Ok(Opened::BrowserAuth { reason, page }) => {
                match self
                    .tender
                    .through_check(job, &item, &ep, &post, reason, &page, cancel)
                    .await?
                {
                    Check::Arrived(file) => {
                        checked = Some(self.tender.check_staging(job, item.id));
                        vec![*file]
                    }
                    Check::Refused(failure) => {
                        // A site that could not answer (`429`, `5xx`) did not
                        // send a page in place of the file, but it ends the
                        // item the same: a new address comes only from a new
                        // check.
                        let message = match failure.kind {
                            FailureKind::Network => "사이트가 파일을 주지 못했어요",
                            _ => "사이트가 파일 대신 웹 페이지를 보냈어요",
                        };
                        self.store
                            .event(
                                job,
                                format!("{ep}: {message}"),
                                Some(described(&FileProblem::from(&failure))),
                                self.now(),
                            )
                            .await?;
                        fail(failure.reason, Some(failure.kind)).await?;
                        let _ = tokio::fs::remove_dir_all(self.tender.check_staging(job, item.id))
                            .await;
                        return Ok(ItemEnd::Settled);
                    }
                    Check::Settled => return Ok(ItemEnd::Settled),
                    Check::Interrupted => return Ok(ItemEnd::Interrupted),
                }
            }
            // Taken to the files it offers before this match.
            Ok(Opened::WinPng { .. }) => unreachable!("WinPNG images are read above"),
        };
        let now = self.now();
        self.store
            .set_step(job, StepKind::Open, StepState::Done, None, now)
            .await?;
        self.store
            .event(
                job,
                format!("{ep}: 게시물을 열었어요"),
                Some(match &host {
                    Some(host) => format!("{host} · 파일 {}개", files.len()),
                    None => format!("파일 {}개", files.len()),
                }),
                now,
            )
            .await?;
        if files.is_empty() {
            fail(
                "게시물에 받을 파일이 없어요".to_owned(),
                Some(FailureKind::Changed),
            )
            .await?;
            return Ok(ItemEnd::Settled);
        }

        self.store.set_stage(job, StepKind::Receive, now).await?;
        self.store.begin_step(job, StepKind::Receive, now).await?;
        let (mut failed, mut held) = (None, None);
        for file in &files {
            if cancel.is_cancelled() {
                return Ok(ItemEnd::Interrupted);
            }
            match self
                .receiver
                .receive(job, &item, &ep, &source, &post, file, cancel)
                .await?
            {
                Receipt::Received => {}
                Receipt::Failed(problem) | Receipt::Retry(problem) => {
                    failed.get_or_insert(problem);
                }
                Receipt::Held(reason) => {
                    held.get_or_insert(reason);
                }
                Receipt::Interrupted => return Ok(ItemEnd::Interrupted),
            }
        }
        let now = self.now();
        match (held, failed) {
            (Some(reason), _) => {
                self.store
                    .set_item(item.id, ItemState::Held, None, Some(reason), now)
                    .await?
            }
            (None, Some(problem)) => {
                self.store
                    .fail_item(item.id, problem.reason, problem.class, now)
                    .await?
            }
            (None, None) => {
                // A revision whose files are the earlier receipt's bytes
                // replaces nothing; that is recorded with the item's end.
                if self.store.finish_item(item.id, now).await?.is_some() {
                    self.store
                        .event(
                            job,
                            format!("{ep}: 받은 파일이 지난번과 바이트가 같아 바꿀 것이 없어요"),
                            Some(format!("파일 {}개", files.len())),
                            self.now(),
                        )
                        .await?;
                }
            }
        }
        if let Some(dir) = checked {
            let _ = tokio::fs::remove_dir_all(dir).await;
        }
        Ok(ItemEnd::Settled)
    }

    /// The folder a reading of images puts its files in: the job's own, in the
    /// area's temporary folder, though not named as an attempt (nothing else
    /// removes it).
    fn staging(&self, job: &str) -> PathBuf {
        self.area.at(&format!(".tmp/winpng-{job}"))
    }

    /// The files the post's WinPNG images hold, read by the runner's reader
    /// into the job's staging folder (see the module docs). Without a reader
    /// the post waits as one whose subtitle is somewhere unreadable. When the
    /// images hold nothing but the post links a subtitle somewhere else too
    /// (`elsewhere`, a Drive folder), it waits for that, as it did before the
    /// images were read.
    async fn read_winpng(
        &self,
        job: &str,
        post: &Url,
        snapshot: &Snapshot,
        elsewhere: Option<&str>,
    ) -> Result<Opened, Failure> {
        let Some(reader) = &self.winpng else {
            return Ok(Opened::Elsewhere {
                reason: winpng::NO_READER.to_owned(),
            });
        };
        let staging = self.staging(job);
        let _ = tokio::fs::remove_dir_all(&staging).await;
        tokio::fs::create_dir_all(&staging).await.map_err(|e| {
            Failure::new(
                FailureKind::Network,
                format!(
                    "이미지에서 꺼낸 파일을 둘 폴더를 만들지 못했어요: {}",
                    e.kind()
                ),
            )
        })?;
        let viewed = reader
            .read(ViewRequest {
                job,
                post,
                staging: &staging,
            })
            .await?;
        if let (Viewed::NoSubtitle | Viewed::NoImages, Some(reason)) = (&viewed, elsewhere) {
            return Ok(Opened::Elsewhere {
                reason: reason.to_owned(),
            });
        }
        match viewed {
            Viewed::Files(staged) => Ok(Opened::Files(winpng::offered(snapshot, &staged))),
            Viewed::NoSubtitle => Err(Failure::new(
                FailureKind::NoSubtitle,
                "게시물의 이미지를 열어 봤지만 자막이 든 WinPNG 이미지가 없어요",
            )),
            // The page the browser opened has none of the images the source
            // saw (loaded late, or the page changed): nothing is known of the
            // subtitle, so it is not `자막 없음`.
            Viewed::NoImages => Err(Failure::new(
                FailureKind::Changed,
                "브라우저로 연 게시물에서 WinPNG 이미지를 찾지 못했어요",
            )),
            Viewed::NeedsKey => Err(Failure::new(
                FailureKind::NeedsInput,
                "자막이 든 WinPNG 이미지를 열려면 키가 필요해요. 키를 넣는 방법은 아직 없어요",
            )),
        }
    }
    /// How the job stands from its items, with its opening and receiving
    /// steps written, before what it received is stored and applied. `kept`
    /// is an upload's note (`올린 파일: …`): the note of the job, all of
    /// whose files were received when it was made, with its receiving step
    /// left as it was written then (what it kept and dropped).
    async fn received(&self, job: &str, kept: Option<String>) -> Result<Received, JobError> {
        let items = self.views.items(job).await?;
        // The file of a settled item's check is not needed any more.
        for item in items.iter().filter(|i| {
            matches!(
                i.state,
                ItemState::Done | ItemState::Failed | ItemState::Held
            )
        }) {
            let dir = self.tender.check_staging(job, item.id);
            if tokio::fs::try_exists(&dir).await.unwrap_or(false) {
                let _ = tokio::fs::remove_dir_all(dir).await;
            }
        }
        let now = self.now();
        let count = |s: ItemState| items.iter().filter(|i| i.state == s).count();
        let first_reason = |s: ItemState| {
            items
                .iter()
                .find(|i| i.state == s)
                .and_then(|i| i.reason.clone())
        };
        let waits = |w: Wait| {
            items
                .iter()
                .find(|i| i.state == ItemState::Waiting && i.wait == Some(w))
        };
        let (total, done, failed) = (
            items.len(),
            count(ItemState::Done),
            count(ItemState::Failed),
        );
        let steps = self.store.steps(job).await?;
        let step_of = |kind: StepKind| steps.iter().find(|s| s.step == kind).map(|s| s.state);

        // The log line's detail: the note, or for a site's check its name.
        let mut detail = None;
        let (state, wait, note, message) = if let Some(item) = waits(Wait::Auth) {
            let reason = item.reason.clone().unwrap_or_default();
            detail = Some(reason.clone());
            (
                JobState::Waiting,
                Some(Wait::Auth),
                Some(format!("사이트 확인을 기다려요 ({reason})")),
                "사이트 확인을 기다려요",
            )
        } else if count(ItemState::Held) > 0 {
            (
                JobState::Held,
                None,
                first_reason(ItemState::Held),
                "확인할 수 없는 파일이 있어 보류했어요",
            )
        } else if let Some(item) = waits(Wait::Subtitle) {
            (
                JobState::Waiting,
                Some(Wait::Subtitle),
                Some(item.reason.clone().unwrap_or_else(|| NO_SOURCE.to_owned())),
                "받을 방법을 기다려요",
            )
        } else if done == total {
            (JobState::Done, None, kept.clone(), "작업을 마쳤어요")
        } else if failed == total {
            (
                JobState::Failed,
                None,
                first_reason(ItemState::Failed),
                "받지 못했어요",
            )
        } else {
            (
                JobState::Partial,
                None,
                Some(format!("{total}개 중 {failed}개를 받지 못했어요")),
                "일부를 받지 못했어요",
            )
        };

        // No post opened.
        if step_of(StepKind::Open) == Some(StepState::Current) && state == JobState::Failed {
            self.store
                .set_step(job, StepKind::Open, StepState::Failed, note.clone(), now)
                .await?;
        }
        if step_of(StepKind::Receive).is_some() && kept.is_none() {
            let step = match state {
                JobState::Done => Some(StepState::Done),
                JobState::Partial => Some(StepState::Partial),
                JobState::Failed => Some(StepState::Failed),
                JobState::Held => Some(StepState::Waiting),
                _ => None,
            };
            if let Some(step) = step {
                let note = (state != JobState::Done).then(|| note.clone()).flatten();
                self.store
                    .set_step(job, StepKind::Receive, step, note, now)
                    .await?;
            }
        }
        Ok(Received {
            state,
            wait,
            note,
            message,
            detail,
        })
    }

    /// Writes the job's state and last log line from its items
    /// ([`Runner::received`]), then from its plan ([`crate::place`]): a
    /// received job whose files wait for a person, a later build or a check
    /// is not `done`.
    async fn settle(
        &self,
        job: &str,
        received: Received,
        placement: Placement,
    ) -> Result<(), JobError> {
        let Received {
            state,
            wait,
            note,
            message,
            mut detail,
        } = received;
        let now = self.now();
        let (done, total, again) = {
            let items = self.views.items(job).await?;
            let done = items.iter().filter(|i| i.state == ItemState::Done).count();
            // An item the placement put back in line: a font it did not
            // receive went away before it was stored
            // ([`crate::place::unchanged`]). A job that waits for a person's
            // check on the site keeps waiting, with its screen; the run after
            // the check receives that item too.
            let again = items.iter().any(|i| i.state == ItemState::Pending);
            (done, items.len(), again)
        };
        // What was received goes on to the plan's rows.
        let (state, wait, note, message) = match state {
            _ if again && wait != Some(Wait::Auth) => (
                JobState::Pending,
                None,
                Some(crate::place::unchanged::RECEIVE_AGAIN.to_owned()),
                crate::place::unchanged::RECEIVE_AGAIN,
            ),
            JobState::Done | JobState::Partial => {
                let standing = self.placer.standing(job).await?;
                // A relocation's held removal waits for the rest of the job.
                let waits = placement.no_folder.is_some()
                    || standing.questions > 0
                    || standing.approved > 0
                    || standing.approvals > 0
                    || (standing.awaiting_video > 0 && state == JobState::Done);
                let held = match standing.held_removal.clone() {
                    Some(_) if waits => None,
                    Some(reason) => Some(reason),
                    None => placement.blocked.or(standing.held.clone()),
                };
                if let Some(reason) = held {
                    (
                        JobState::Held,
                        None,
                        Some(reason),
                        "보관하거나 적용하지 못한 파일이 있어 보류했어요",
                    )
                } else if let Some(reason) = placement.unanalysed {
                    (
                        JobState::Waiting,
                        Some(Wait::Subtitle),
                        Some(reason),
                        "받은 묶음의 분석을 기다려요",
                    )
                } else if let Some(reason) = placement.confirm {
                    (
                        JobState::Waiting,
                        Some(Wait::Placement),
                        Some(reason),
                        "배치 확인을 기다려요",
                    )
                } else if let Some(reason) = placement.no_folder {
                    (
                        JobState::Waiting,
                        Some(Wait::Video),
                        Some(reason),
                        "작품 폴더를 기다려요",
                    )
                } else if standing.questions > 0 {
                    (
                        JobState::Waiting,
                        Some(Wait::Placement),
                        Some(format!(
                            "회차를 확인할 파일이 {}개 있어요",
                            standing.questions
                        )),
                        "회차 확인을 기다려요",
                    )
                } else if standing.approved > 0 {
                    // Decided while the run went on: carried out by the next.
                    (
                        JobState::Pending,
                        None,
                        Some("승인한 교체를 반영해요".to_owned()),
                        "승인한 교체를 반영해요",
                    )
                } else if standing.approvals > 0 {
                    (
                        JobState::Waiting,
                        Some(Wait::Approval),
                        Some(match standing.approvals {
                            1 => crate::place::replace::AWAITING_APPROVAL.to_owned(),
                            n => format!("교체를 기다리는 회차가 {n}개 있어요"),
                        }),
                        "교체 승인을 기다려요",
                    )
                } else if standing.failed > 0 || standing.unpack_failed > 0 {
                    let all = standing.failed == standing.rows
                        && !standing.unplanned
                        && state == JobState::Done;
                    (
                        match all {
                            true => JobState::Failed,
                            false => JobState::Partial,
                        },
                        None,
                        standing.failure.or(standing.unpack_failure),
                        match standing.failed {
                            0 => "압축 파일을 풀지 못했어요",
                            _ => "보관하거나 적용하지 못한 파일이 있어요",
                        },
                    )
                } else if let Some(reason) = standing.missing {
                    (
                        JobState::Partial,
                        None,
                        Some(reason),
                        "받은 묶음에 후보의 회차 파일이 없어요",
                    )
                } else if standing.awaiting_video > 0 && state == JobState::Done {
                    (
                        JobState::Waiting,
                        Some(Wait::Video),
                        Some(match standing.awaiting_video {
                            1 => crate::place::AWAITING_VIDEO.to_owned(),
                            n => format!("영상이 없는 회차 {n}개의 영상을 기다려요"),
                        }),
                        "영상을 기다려요",
                    )
                } else {
                    // A partial receipt stays partial: its rows waiting for a
                    // video go on when it comes
                    // ([`JobRun::requeue_awaiting_video`]).
                    (state, wait, note, message)
                }
            }
            _ => (state, wait, note, message),
        };
        if detail.is_none() && wait == Some(Wait::Placement) {
            detail = note.clone();
        }
        let requeued = self
            .store
            .settle(job, state, wait, note.clone(), now)
            .await?;
        let (state, message, detail) = match requeued {
            Some(why) => (JobState::Pending, why, None),
            None => (state, message, detail.or(note)),
        };
        self.store
            .event(job, message.to_owned(), detail, now)
            .await?;
        // The run ended, so the browser run it used goes too; a job that waits
        // for a person's check on the site keeps it for that check, and its
        // screen.
        if wait != Some(Wait::Auth) {
            if let Some(reader) = &self.winpng {
                reader.release(job).await;
            }
            self.tender.release(job).await?;
        }
        println!("Subtitle job {job}: {} ({done}/{total})", state.code());
        Ok(())
    }
}
