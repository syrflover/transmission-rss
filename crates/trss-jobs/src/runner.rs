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
//!   with the source's reason.
//! - A post whose subtitle is in WinPNG images ([`Opened::WinPng`]) is read by
//!   the runner's [`WinpngReader`] (a server browser; [`Runner::with_winpng`]):
//!   the files it takes out are put in a folder of the job's, which is the
//!   step `open`, and are then received like any other file, under their
//!   folders ([`FileRow::folder`]). A post whose images hold no subtitle fails
//!   as `no_subtitle`, and one whose image needs a key as `needs_input`.
//!   A runner with no reader leaves the item waiting for a source, as for a
//!   Drive folder. The reader's browser run is let go when a run ends, unless
//!   the job waits for a person's check on the site.
//! - The source opens the post for the item's episode: a post that says which
//!   file is which episode (Blogger's Drive links) offers that episode's.
//! - A site that asks for a person's check makes the item wait (`인증 필요`).
//!   Nothing goes on until a later ticket's screen lets a person solve it.
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
//!   that there is nothing to replace ([`JobStore::finish_item`]); no
//!   replacement is to be approved for it.
//!
//! # One receipt
//!
//! Each effect on a file is preceded by a record of its intent and followed by
//! a record of its result:
//!
//! 1. `intended`: the attempt's ID and its own temporary folder, before
//!    anything is fetched; then the length the source announced, and the
//!    file's name when the answer gives one the post did not (Drive's).
//! 2. The bytes go to `.tmp/<attempt>/<name>`, hashed as they come, and the
//!    file is synced. A length other than the announced one fails the attempt
//!    and its bytes are removed.
//! 3. `fetched`: the length, the SHA-256, the temporary file's object
//!    (`dev:ino`) and the path it is to be published at.
//! 4. The bytes are checked ([`trss_subtitles::verify`]): bytes that are not
//!    a file (nothing, a web page, a ZIP whose CRC fails, a name's format the
//!    bytes are not) fail the receipt as `not_a_file`. The failure is recorded
//!    with the path still named, then the bytes are removed, then the path.
//!    A check that breaks off (a panic) holds the receipt with its bytes.
//! 5. The file is published by a rename that replaces nothing, and its folder
//!    synced.
//! 6. `done` with the format the check found, and the temporary folder goes.
//!
//! # Restart
//!
//! A worker that dies leaves its job `running`; the next one to hold the
//! worker lock claims it again ([`JobStore::claim_next`]). The lock is the
//! kernel's and goes with the process, so no earlier worker still writes.
//! Before receiving a file again the runner compares its unfinished receipt
//! with the disk ([`Runner::recover`]):
//!
//! | Record | On disk | Then |
//! | --- | --- | --- |
//! | `intended` | no temporary file | no bytes came: `abandoned`, a new attempt |
//! | `intended` | a temporary file shorter than announced | known to be incomplete: `abandoned` (its bytes removed), a new attempt |
//! | `intended` | a temporary file of the announced length | taken as fetched, then checked and published |
//! | `intended` | any other temporary file (no length was announced, or it is longer) | `held` |
//! | `fetched` | the temporary file, same object, length and hash | checked and published |
//! | `fetched` | no temporary file, the planned path is the recorded object with its length and hash | checked, then `done` |
//! | `fetched` | anything else | `held` |
//! | `done` | the path has the recorded length and hash | reused |
//! | `done` | anything else | `held` |
//! | `failed` that still names its path | its temporary file or its path is the recorded object with its length and hash | that file removed (a removal that fails: `held`), then the path cleared |
//!
//! A held file holds its item, and a held item holds its job: the runner does
//! not take it up again by itself. Its temporary file and anything at its path
//! stay as they are. A recovered file the check finds not to be one fails, its
//! bytes are removed as in step 4, and the item receives the file anew. Only
//! a removal that succeeds or finds the file gone counts; any other error
//! holds the receipt.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use trss_core::{Clock, Millis};
use trss_subtitles::{
    verify,
    winpng::{self, ViewRequest, Viewed, WinpngReader},
    Failure, FailureKind, Opened, PostFile, Snapshot, Sources,
};
use url::Url;

use crate::{
    area::{self, ReceiveArea},
    model::{FileState, ItemState, JobState, StepKind, StepState, Wait},
    store::{snapshot_json, FileProblem, FileRow, ItemRow, JobError, JobStore},
};

/// What the job's log and screen say for a post no source reads.
pub const NO_SOURCE: &str = "이 출처에서 받는 방법을 아직 몰라요";

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
    store: JobStore,
    sources: Sources,
    area: ReceiveArea,
    clock: Clock,
    retry_waits: Arc<[Duration]>,
    /// Reads the WinPNG images of a post; `None`: the worker has no server
    /// browser.
    winpng: Option<Arc<dyn WinpngReader>>,
}

/// How one file of an item came out.
enum Receipt {
    Received,
    Failed(FileProblem),
    /// A network failure a next attempt may get past: this attempt is
    /// `abandoned`.
    Retry(FileProblem),
    Held(String),
    /// Shutdown was asked for in the middle; the item stays `running`.
    Interrupted,
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

/// What the check of received bytes found.
enum Checked {
    File(verify::Format),
    NotAFile(FileProblem),
    /// The check itself broke off (a panic): the bytes are neither passed nor
    /// failed, so the receipt is held with them.
    Broken(String),
}

/// What a receipt held for a broken check says.
const CHECK_BROKEN: &str = "받은 파일을 확인하는 도중에 확인이 비정상으로 끝났어요";

/// Checks the received bytes at `path`, received as `name`, off the runtime.
async fn check(path: PathBuf, name: String) -> Checked {
    match tokio::task::spawn_blocking(move || verify::check(&path, &name)).await {
        Ok(Ok(format)) => Checked::File(format),
        Ok(Err(failure)) => Checked::NotAFile(FileProblem::from(&failure)),
        Err(_) => Checked::Broken(CHECK_BROKEN.to_owned()),
    }
}

/// Removes a file: whether it is gone, as removed now or found missing.
fn remove_known(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// How a write of the bytes to the temporary file ended badly.
enum Written {
    Io(io::Error),
    Source(Failure),
    Interrupted,
}

/// How one item came out.
enum ItemEnd {
    Settled,
    Interrupted,
}

impl Runner {
    pub fn new(store: JobStore, sources: Sources, area: ReceiveArea, clock: Clock) -> Runner {
        Runner {
            store,
            sources,
            area,
            clock,
            retry_waits: Arc::new(RETRY_WAITS),
            winpng: None,
        }
    }

    /// The same runner reading WinPNG images with `reader`.
    pub fn with_winpng(mut self, reader: Arc<dyn WinpngReader>) -> Runner {
        self.winpng = Some(reader);
        self
    }

    /// The same runner with other waits before a retry ([`RETRY_WAITS`]); as
    /// many retries as waits.
    pub fn with_retry_waits(mut self, waits: Vec<Duration>) -> Runner {
        self.retry_waits = waits.into();
        self
    }

    pub fn store(&self) -> &JobStore {
        &self.store
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

    pub async fn has_ready(&self) -> Result<bool, JobError> {
        self.store.has_ready().await
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
        let message = match resumed {
            true => "멈췄던 작업을 이어가요",
            false => "작업을 시작했어요",
        };
        println!("Subtitle job {id}: {message}");
        self.store
            .event(id, message.to_owned(), None, self.now())
            .await?;

        for item in self.store.items(id).await? {
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
        self.settle(id).await?;
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
            if let Some(reason) = self.recover(job, &ep, unfinished).await? {
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
                Err(failure) if failure.kind.retryable() && tries < self.retry_waits.len() => {
                    let wait = self.retry_waits[tries];
                    tries += 1;
                    self.store
                        .event(
                            job,
                            format!("{ep}: 게시물을 열지 못해 다시 시도해요"),
                            Some(format!(
                                "{} · {} 뒤 ({tries}/{})",
                                described(&FileProblem::from(&failure)),
                                wait_text(wait),
                                self.retry_waits.len()
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

    /// Receives one file of `item`, or finds it received (see the module docs).
    #[allow(clippy::too_many_arguments)]
    async fn receive(
        &self,
        job: &str,
        item: &ItemRow,
        ep: &str,
        source: &trss_subtitles::Source,
        post: &Url,
        file: &PostFile,
        cancel: &CancellationToken,
    ) -> Result<Receipt, JobError> {
        let name = area::safe_name(&file.name);
        let receipts = self.store.files_for_key(job, &file.key).await?;

        // An attempt an earlier start did not finish, compared with the disk.
        for unfinished in receipts.iter().filter(|r| {
            matches!(r.state, FileState::Intended | FileState::Fetched)
                || (r.state == FileState::Failed && r.path.is_some())
        }) {
            if let Some(reason) = self.recover(job, ep, unfinished).await? {
                return Ok(Receipt::Held(reason));
            }
        }
        let receipts = self.store.files_for_key(job, &file.key).await?;

        if receipts.iter().any(|r| r.state == FileState::Held) {
            return Ok(Receipt::Held("같은 파일이 보류돼 있어요".to_owned()));
        }
        if let Some(original) = receipts
            .iter()
            .find(|r| r.state == FileState::Done && r.same_as.is_none())
        {
            return self.reuse(job, item, ep, original, &receipts).await;
        }

        let mut tries = 0;
        loop {
            let retry = tries < self.retry_waits.len();
            match self
                .attempt(job, item, ep, source, post, file, &name, retry, cancel)
                .await?
            {
                Receipt::Retry(problem) => {
                    let wait = self.retry_waits[tries];
                    tries += 1;
                    self.store
                        .event(
                            job,
                            format!("{ep}: 파일을 받지 못해 다시 시도해요"),
                            Some(format!(
                                "{name} · {} · {} 뒤 ({tries}/{})",
                                described(&problem),
                                wait_text(wait),
                                self.retry_waits.len()
                            )),
                            self.now(),
                        )
                        .await?;
                    if !pause(wait, cancel).await {
                        return Ok(Receipt::Interrupted);
                    }
                }
                receipt => return Ok(receipt),
            }
        }
    }

    /// One attempt to receive `file` (see the module docs). A network failure
    /// comes back as [`Receipt::Retry`] when `retry` allows one.
    #[allow(clippy::too_many_arguments)]
    async fn attempt(
        &self,
        job: &str,
        item: &ItemRow,
        ep: &str,
        source: &trss_subtitles::Source,
        post: &Url,
        file: &PostFile,
        name: &str,
        retry: bool,
        cancel: &CancellationToken,
    ) -> Result<Receipt, JobError> {
        let attempt = uuid::Uuid::new_v4().to_string();
        let temp_rel = ReceiveArea::temp_dir(&attempt);
        // A folder too deep or too long is refused below, after the receipt is
        // recorded.
        let (folder, folder_refused) = match file.folder.as_deref().map(area::safe_folder) {
            None => (None, None),
            Some(Ok(folder)) => (folder, None),
            Some(Err(reason)) => (None, Some(reason)),
        };
        let now = self.now();
        self.store
            .file_intend(FileRow {
                id: attempt.clone(),
                item_id: item.id,
                file_key: file.key.clone(),
                name: name.to_owned(),
                state: FileState::Intended,
                same_as: None,
                temp_dir: Some(temp_rel.clone()),
                expected_size: None,
                size: None,
                sha256: None,
                object: None,
                path: None,
                reason: None,
                created_at: now,
                format: None,
                failure: None,
                http_status: None,
                content_type: None,
                response_size: None,
                snapshot: snapshot_json(&file.snapshot),
                kind: None,
                archive: None,
                folder: folder.clone(),
            })
            .await?;

        if let Some(reason) = folder_refused {
            let problem = FileProblem {
                reason,
                class: Some(FailureKind::NotAFile),
                status: None,
                content_type: None,
                size: None,
            };
            return self
                .fail_file(job, ep, &attempt, name, None, problem, false)
                .await;
        }

        let fetched = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                self.store
                    .file_end(
                        &attempt,
                        FileState::Abandoned,
                        Some("종료 요청으로 멈췄어요".to_owned()),
                        self.now(),
                    )
                    .await?;
                return Ok(Receipt::Interrupted);
            }
            fetched = source.fetch(post, file) => fetched,
        };
        let mut fetch = match fetched {
            Ok(fetch) => fetch,
            Err(failure) => {
                let retry = retry && failure.kind.retryable();
                return self
                    .fail_file(job, ep, &attempt, name, None, (&failure).into(), retry)
                    .await;
            }
        };
        let snapshot = match fetch.snapshot.is_empty() {
            true => None,
            false => {
                let mut whole = file.snapshot.clone();
                whole.extend(&fetch.snapshot);
                snapshot_json(&whole)
            }
        };
        // The name the answer gives (a Drive file's) is the file's from here
        // on, recorded before its temporary file is made.
        let answered = fetch.name.as_deref().map(area::safe_name);
        self.store
            .file_answer(
                &attempt,
                fetch.expected_size,
                fetch.status,
                fetch.content_type.clone(),
                snapshot,
                answered.clone(),
                self.now(),
            )
            .await?;
        let name = answered.as_deref().unwrap_or(name);
        // What a failure of the bytes themselves says about the answer.
        let (status, content_type) = (fetch.status, fetch.content_type.clone());
        let not_a_file = |reason: String, size: u64| FileProblem {
            reason,
            class: Some(FailureKind::NotAFile),
            status,
            content_type: content_type.clone(),
            size: Some(size),
        };

        let temp_dir = self.area.at(&temp_rel);
        let temp = temp_dir.join(name);
        let written: Result<(u64, String), Written> = async {
            tokio::fs::create_dir_all(&temp_dir)
                .await
                .map_err(Written::Io)?;
            let mut out = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .await
                .map_err(Written::Io)?;
            let mut hasher = Sha256::new();
            let mut size = 0u64;
            loop {
                let piece = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(Written::Interrupted),
                    piece = fetch.chunk() => piece,
                };
                let piece = match piece {
                    Ok(Some(piece)) => piece,
                    Ok(None) => break,
                    Err(failure) => return Err(Written::Source(failure)),
                };
                hasher.update(&piece);
                size += piece.len() as u64;
                out.write_all(&piece).await.map_err(Written::Io)?;
            }
            out.sync_all().await.map_err(Written::Io)?;
            Ok((size, area::hex(&hasher.finalize())))
        }
        .await;
        let (size, sha256) = match written {
            Ok(written) => written,
            Err(Written::Interrupted) => {
                // The bytes are known to be incomplete; the next start receives
                // the file again.
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                self.store
                    .file_end(
                        &attempt,
                        FileState::Abandoned,
                        Some("종료 요청으로 멈췄어요".to_owned()),
                        self.now(),
                    )
                    .await?;
                return Ok(Receipt::Interrupted);
            }
            Err(Written::Source(failure)) => {
                let retry = retry && failure.kind.retryable();
                return self
                    .fail_file(
                        job,
                        ep,
                        &attempt,
                        name,
                        Some(&temp_dir),
                        (&failure).into(),
                        retry,
                    )
                    .await;
            }
            Err(Written::Io(err)) => {
                return self
                    .fail_file(
                        job,
                        ep,
                        &attempt,
                        name,
                        Some(&temp_dir),
                        FileProblem::local(err.to_string()),
                        false,
                    )
                    .await;
            }
        };
        if size == 0 {
            let problem = not_a_file("받은 파일이 비어 있어요".into(), 0);
            return self
                .fail_file(job, ep, &attempt, name, Some(&temp_dir), problem, false)
                .await;
        }
        if let Some(expected) = fetch.expected_size.filter(|e| *e != size) {
            let problem = not_a_file(
                format!("사이트가 알린 크기({expected}바이트)와 받은 크기({size}바이트)가 달라요"),
                size,
            );
            return self
                .fail_file(job, ep, &attempt, name, Some(&temp_dir), problem, false)
                .await;
        }
        let object = match std::fs::metadata(&temp) {
            Ok(meta) => area::object_of(&meta),
            Err(err) => {
                return self
                    .fail_file(
                        job,
                        ep,
                        &attempt,
                        name,
                        Some(&temp_dir),
                        FileProblem::local(err.to_string()),
                        false,
                    )
                    .await
            }
        };
        let path = self.plan_path(job, folder.as_deref(), name).await?;
        self.store
            .file_fetched(&attempt, size, sha256, object, path.clone(), self.now())
            .await?;
        self.publish(job, ep, &attempt, &temp_rel, name, &path, size)
            .await
    }

    /// Checks a fetched file's bytes, then publishes its temporary file at
    /// `path` and records it done; bytes that are not a file fail it.
    #[allow(clippy::too_many_arguments)]
    async fn publish(
        &self,
        job: &str,
        ep: &str,
        attempt: &str,
        temp_rel: &str,
        name: &str,
        path: &str,
        size: u64,
    ) -> Result<Receipt, JobError> {
        let temp = self.area.at(temp_rel).join(name);
        let format = match check(temp.clone(), name.to_owned()).await {
            Checked::File(format) => format,
            Checked::NotAFile(problem) => {
                let problem = FileProblem {
                    size: Some(size),
                    ..problem
                };
                return self
                    .fail_fetched(job, ep, attempt, name, temp_rel, &temp, problem)
                    .await;
            }
            Checked::Broken(reason) => return self.hold_file(job, ep, attempt, name, reason).await,
        };
        let target = self.area.at(path);
        let published = (|| {
            let folder = target.parent().expect("a path in a job's folder");
            // Synced every time: a crash may have left a folder made before
            // its entry was synced. Every level down from the area has its
            // entry synced, a file's own folders included.
            std::fs::create_dir_all(folder)?;
            let mut level = folder;
            while let Some(parent) = level.parent() {
                area::sync_dir(parent)?;
                if parent == self.area.root() {
                    break;
                }
                level = parent;
            }
            trss_core::files::rename_noreplace(&temp, &target)?;
            area::sync_dir(folder)
        })();
        if let Err(err) = published {
            let reason = match err.kind() {
                io::ErrorKind::AlreadyExists => "받을 자리에 다른 파일이 있어요".to_owned(),
                _ => format!("받은 파일을 옮기지 못했어요: {err}"),
            };
            return self.hold_file(job, ep, attempt, name, reason).await;
        }
        self.store.file_done(attempt, format, self.now()).await?;
        let _ = std::fs::remove_dir_all(self.area.at(temp_rel));
        self.store
            .event(
                job,
                format!("{ep}: 파일을 받았어요"),
                Some(format!(
                    "{name} · {} · {}",
                    human_size(size),
                    format.label()
                )),
                self.now(),
            )
            .await?;
        Ok(Receipt::Received)
    }

    /// A free path for `name` in the job's folder, within `folder` when the
    /// file has one: on the disk and among the job's receipts.
    async fn plan_path(
        &self,
        job: &str,
        folder: Option<&str>,
        name: &str,
    ) -> Result<String, JobError> {
        let taken = self.store.paths_of(job).await?;
        // A name is out of the way when nothing of the job is at it, on the
        // disk or among the receipts, and when it is not a folder that files
        // are at (`X` the file against `X/` the folder).
        let free = |path: &str| {
            let on_disk = std::fs::symlink_metadata(self.area.at(path)).is_ok();
            let is_folder = taken.iter().any(|t| {
                t.strip_prefix(path)
                    .is_some_and(|rest| rest.starts_with('/'))
            });
            !on_disk && !is_folder && !taken.iter().any(|t| t == path)
        };
        // A folder is out of the way when what is at its name is a folder (or
        // nothing). One that is a file of the job's, or anything that is not a
        // folder on the disk, is passed over for `X (2)`, as a file would be.
        let usable_folder = |path: &str| {
            let on_disk = match std::fs::symlink_metadata(self.area.at(path)) {
                Ok(meta) => meta.is_dir(),
                Err(_) => true,
            };
            on_disk && !taken.iter().any(|t| t == path)
        };
        let mut dir = ReceiveArea::job_dir(job);
        for part in folder.into_iter().flat_map(|f| f.split('/')) {
            let candidate = std::iter::once(part.to_owned())
                .chain((2..).map(|n| format!("{part} ({n})")))
                .find(|c| usable_folder(&format!("{dir}/{c}")))
                .expect("the names go on");
            dir = format!("{dir}/{candidate}");
        }
        for candidate in area::name_candidates(name) {
            let path = format!("{dir}/{candidate}");
            if free(&path) {
                return Ok(path);
            }
        }
        unreachable!("the names go on")
    }

    /// The file was received for this job before: its bytes are checked and
    /// it is used again.
    async fn reuse(
        &self,
        job: &str,
        item: &ItemRow,
        ep: &str,
        original: &FileRow,
        receipts: &[FileRow],
    ) -> Result<Receipt, JobError> {
        let path = original.path.as_deref().unwrap_or_default();
        let facts = area::read_facts(&self.area.at(path)).ok();
        let matches = facts.as_ref().is_some_and(|(size, sha, _)| {
            Some(*size) == original.size && Some(sha) == original.sha256.as_ref()
        });
        if !matches {
            let reason = "받은 파일이 기록과 달라요".to_owned();
            self.store
                .file_end(
                    &original.id,
                    FileState::Held,
                    Some(reason.clone()),
                    self.now(),
                )
                .await?;
            self.store
                .event(
                    job,
                    format!("{ep}: 파일을 보류했어요"),
                    Some(format!("{} · {reason}", original.name)),
                    self.now(),
                )
                .await?;
            return Ok(Receipt::Held(reason));
        }
        let name = &original.name;
        if original.item_id == item.id {
            self.store
                .event(
                    job,
                    format!("{ep}: 이미 받은 파일이라 다시 받지 않았어요"),
                    Some(format!("{name} · 바이트 확인")),
                    self.now(),
                )
                .await?;
        } else if !receipts
            .iter()
            .any(|r| r.item_id == item.id && r.state == FileState::Done)
        {
            self.store
                .file_share(
                    &uuid::Uuid::new_v4().to_string(),
                    item.id,
                    original,
                    self.now(),
                )
                .await?;
            self.store
                .event(
                    job,
                    format!("{ep}: 앞 회차에서 받은 파일과 같아요"),
                    Some(name.clone()),
                    self.now(),
                )
                .await?;
        }
        Ok(Receipt::Received)
    }

    /// Compares an unfinished receipt with the disk (see the module docs).
    /// Returns why it is held, or `None` when it was settled (published,
    /// found done, or abandoned).
    async fn recover(&self, job: &str, ep: &str, r: &FileRow) -> Result<Option<String>, JobError> {
        // A receipt with no folder of its own cannot be compared with the disk,
        // and nothing outside its folder is touched.
        let Some(temp_rel) = r.temp_dir.clone().filter(|d| d.starts_with(".tmp/")) else {
            return self
                .hold_reason(job, ep, r, "임시 폴더 기록이 없어요")
                .await;
        };
        let temp = self.area.at(&temp_rel).join(&r.name);
        let temp_meta = std::fs::symlink_metadata(&temp).ok();
        let now = self.now();
        let recorded = |facts: &(u64, String, String)| {
            Some(facts.0) == r.size
                && Some(&facts.1) == r.sha256.as_ref()
                && Some(&facts.2) == r.object.as_ref()
        };

        if r.state == FileState::Failed {
            return self.finish_failed(job, ep, r, &temp_rel, recorded).await;
        }

        if r.state == FileState::Intended {
            // No bytes came (an empty file is none either).
            let Some(meta) = temp_meta.filter(|m| !(m.is_file() && m.len() == 0)) else {
                let _ = std::fs::remove_file(&temp);
                self.store
                    .file_end(
                        &r.id,
                        FileState::Abandoned,
                        Some("받은 바이트가 없어요".into()),
                        now,
                    )
                    .await?;
                let _ = std::fs::remove_dir(self.area.at(&temp_rel));
                return Ok(None);
            };
            let len = meta.len();
            match r.expected_size {
                Some(expected) if len < expected => {
                    let _ = std::fs::remove_dir_all(self.area.at(&temp_rel));
                    self.store
                        .file_end(
                            &r.id,
                            FileState::Abandoned,
                            Some(format!("{expected}바이트 중 {len}바이트에서 멈췄어요")),
                            now,
                        )
                        .await?;
                    self.store
                        .event(
                            job,
                            format!("{ep}: 끝까지 받지 못한 파일을 다시 받아요"),
                            Some(format!("{} · {len}/{expected}바이트", r.name)),
                            now,
                        )
                        .await?;
                    return Ok(None);
                }
                Some(expected) if len == expected && meta.is_file() => {
                    // Synced before it is published, as the first start would have.
                    let synced = std::fs::File::open(&temp).and_then(|f| f.sync_all());
                    let (Ok(()), Ok((size, sha, object))) = (synced, area::read_facts(&temp))
                    else {
                        return self
                            .hold_reason(job, ep, r, "임시 파일을 읽지 못했어요")
                            .await;
                    };
                    let path = self.plan_path(job, r.folder.as_deref(), &r.name).await?;
                    self.store
                        .file_fetched(&r.id, size, sha, object, path.clone(), now)
                        .await?;
                    self.store
                        .event(
                            job,
                            format!("{ep}: 다 받아 둔 파일을 이어서 옮겨요"),
                            Some(format!("{} · 알린 크기와 같아요", r.name)),
                            now,
                        )
                        .await?;
                    return self
                        .publish_recovered(job, ep, r, &temp_rel, &path, size)
                        .await;
                }
                _ => {
                    return self
                        .hold_reason(job, ep, r, "받은 바이트가 완전한지 확인할 수 없어요")
                        .await
                }
            }
        }

        // `fetched`: the bytes were complete and recorded.
        let Some(path) = r.path.clone() else {
            return self
                .hold_reason(job, ep, r, "공개할 경로 기록이 없어요")
                .await;
        };
        if temp_meta.is_some() {
            return match area::read_facts(&temp) {
                Ok(facts) if recorded(&facts) => {
                    self.publish_recovered(job, ep, r, &temp_rel, &path, facts.0)
                        .await
                }
                _ => {
                    self.hold_reason(job, ep, r, "임시 파일이 기록과 달라요")
                        .await
                }
            };
        }
        match area::read_facts(&self.area.at(&path)) {
            Ok(facts) if recorded(&facts) => {
                // This attempt's own file, checked as one published now would be.
                let format = match check(self.area.at(&path), r.name.clone()).await {
                    Checked::File(format) => format,
                    Checked::NotAFile(problem) => {
                        let problem = FileProblem {
                            size: Some(facts.0),
                            ..problem
                        };
                        let at = self.area.at(&path);
                        return match self
                            .fail_fetched(job, ep, &r.id, &r.name, &temp_rel, &at, problem)
                            .await?
                        {
                            Receipt::Held(reason) => Ok(Some(reason)),
                            _ => Ok(None),
                        };
                    }
                    Checked::Broken(reason) => return self.hold_reason(job, ep, r, &reason).await,
                };
                self.store.file_done(&r.id, format, now).await?;
                let _ = std::fs::remove_dir(self.area.at(&temp_rel));
                self.store
                    .event(
                        job,
                        format!("{ep}: 옮긴 뒤 기록 전에 멈췄던 파일을 확인했어요"),
                        Some(format!("{} · 파일 객체와 바이트 확인", r.name)),
                        now,
                    )
                    .await?;
                Ok(None)
            }
            _ => {
                self.hold_reason(job, ep, r, "받은 파일을 확인할 수 없어요")
                    .await
            }
        }
    }

    async fn publish_recovered(
        &self,
        job: &str,
        ep: &str,
        r: &FileRow,
        temp_rel: &str,
        path: &str,
        size: u64,
    ) -> Result<Option<String>, JobError> {
        match self
            .publish(job, ep, &r.id, temp_rel, &r.name, path, size)
            .await?
        {
            Receipt::Held(reason) => Ok(Some(reason)),
            _ => Ok(None),
        }
    }

    async fn hold_reason(
        &self,
        job: &str,
        ep: &str,
        r: &FileRow,
        reason: &str,
    ) -> Result<Option<String>, JobError> {
        self.hold_file(job, ep, &r.id, &r.name, reason.to_owned())
            .await?;
        Ok(Some(reason.to_owned()))
    }

    async fn hold_file(
        &self,
        job: &str,
        ep: &str,
        attempt: &str,
        name: &str,
        reason: String,
    ) -> Result<Receipt, JobError> {
        let now = self.now();
        self.store
            .file_end(attempt, FileState::Held, Some(reason.clone()), now)
            .await?;
        self.store
            .event(
                job,
                format!("{ep}: 파일을 보류했어요"),
                Some(format!("{name} · {reason}")),
                now,
            )
            .await?;
        Ok(Receipt::Held(reason))
    }

    /// Ends an attempt that has not reached `fetched` (it names no path)
    /// `failed` for `problem` and removes its temporary folder; or, when
    /// `retry`, `abandoned` for the next attempt (the caller logs the retry).
    /// A fetched attempt fails through [`Runner::fail_fetched`].
    #[allow(clippy::too_many_arguments)]
    async fn fail_file(
        &self,
        job: &str,
        ep: &str,
        attempt: &str,
        name: &str,
        temp_dir: Option<&Path>,
        problem: FileProblem,
        retry: bool,
    ) -> Result<Receipt, JobError> {
        if let Some(dir) = temp_dir {
            let _ = tokio::fs::remove_dir_all(dir).await;
        }
        let now = self.now();
        let state = match retry {
            true => FileState::Abandoned,
            false => FileState::Failed,
        };
        self.store
            .file_fail(attempt, state, problem.clone(), now)
            .await?;
        if retry {
            return Ok(Receipt::Retry(problem));
        }
        self.store
            .event(
                job,
                format!("{ep}: 파일을 받지 못했어요"),
                Some(format!("{name} · {}", described(&problem))),
                now,
            )
            .await?;
        Ok(Receipt::Failed(problem))
    }

    /// Fails a fetched attempt whose bytes at `bytes` (its temporary file, or
    /// the file it published) are not a file. The failure is recorded first,
    /// with the path still named, then the bytes go, then the path: a crash in
    /// between leaves a `failed` receipt that names its bytes, which the next
    /// start removes ([`Runner::finish_failed`]). Bytes that cannot be removed
    /// hold the receipt.
    #[allow(clippy::too_many_arguments)]
    async fn fail_fetched(
        &self,
        job: &str,
        ep: &str,
        attempt: &str,
        name: &str,
        temp_rel: &str,
        bytes: &Path,
        problem: FileProblem,
    ) -> Result<Receipt, JobError> {
        let now = self.now();
        self.store
            .file_fail(attempt, FileState::Failed, problem.clone(), now)
            .await?;
        if let Err(err) = remove_known(bytes) {
            let reason = format!("파일이 아닌 바이트를 지우지 못했어요: {}", err.kind());
            return self.hold_file(job, ep, attempt, name, reason).await;
        }
        let _ = std::fs::remove_dir(self.area.at(temp_rel));
        self.store.file_clear_path(attempt, self.now()).await?;
        self.store
            .event(
                job,
                format!("{ep}: 파일을 받지 못했어요"),
                Some(format!("{name} · {}", described(&problem))),
                now,
            )
            .await?;
        Ok(Receipt::Failed(problem))
    }

    /// A `failed` receipt that still names its path: a start stopped after
    /// recording the failure and before its bytes went. Its bytes go from its
    /// temporary file and from its path, each only when it is the recorded
    /// object with the recorded length and hash; anything else there is left
    /// as it is. Then the path goes. Bytes that cannot be read or removed
    /// hold the receipt.
    async fn finish_failed(
        &self,
        job: &str,
        ep: &str,
        r: &FileRow,
        temp_rel: &str,
        recorded: impl Fn(&(u64, String, String)) -> bool,
    ) -> Result<Option<String>, JobError> {
        let Some(path) = r.path.as_deref() else {
            return Ok(None);
        };
        for at in [self.area.at(temp_rel).join(&r.name), self.area.at(path)] {
            match std::fs::symlink_metadata(&at) {
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Ok(meta) if !meta.is_file() => continue,
                _ => {}
            }
            match area::read_facts(&at) {
                Ok(facts) if recorded(&facts) => {
                    if remove_known(&at).is_err() {
                        return self
                            .hold_reason(job, ep, r, "실패한 파일의 바이트를 지우지 못했어요")
                            .await;
                    }
                }
                Ok(_) => {}
                Err(_) => {
                    return self
                        .hold_reason(job, ep, r, "실패한 파일의 바이트를 확인할 수 없어요")
                        .await
                }
            }
        }
        let _ = std::fs::remove_dir(self.area.at(temp_rel));
        let now = self.now();
        self.store.file_clear_path(&r.id, now).await?;
        self.store
            .event(
                job,
                format!("{ep}: 받지 못한 파일을 마저 지웠어요"),
                Some(format!("{} · 파일 객체와 바이트 확인", r.name)),
                now,
            )
            .await?;
        Ok(None)
    }

    /// Writes the job's state, steps and last log line from its items.
    async fn settle(&self, job: &str) -> Result<(), JobError> {
        let items = self.store.items(job).await?;
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
            (JobState::Done, None, None, "작업을 마쳤어요")
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
        if step_of(StepKind::Receive).is_some() {
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
        self.store
            .settle(job, state, wait, note.clone(), now)
            .await?;
        self.store
            .event(job, message.to_owned(), detail.or(note), now)
            .await?;
        // The run ended, so the browser run it used goes too; a job that waits
        // for a person's check on the site keeps it for that check.
        if let (Some(reader), false) = (&self.winpng, wait == Some(Wait::Auth)) {
            reader.release(job).await;
        }
        println!("Subtitle job {job}: {} ({done}/{total})", state.code());
        Ok(())
    }
}

/// `"11"` as `11화`; a text that is not a number as it is.
pub fn episode_label(episode: &str) -> String {
    match episode.parse::<f64>() {
        Ok(_) => format!("{episode}화"),
        Err(_) => episode.to_owned(),
    }
}

fn human_size(size: u64) -> String {
    match size {
        s if s < 1024 => format!("{s} B"),
        s if s < 1024 * 1024 => format!("{:.1} KB", s as f64 / 1024.0),
        s => format!("{:.1} MB", s as f64 / 1024.0 / 1024.0),
    }
}
