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
//!   ([`Runner::requeue_waiting_for_sources`]).
//! - A site that asks for a person's check makes the item wait (`인증 필요`).
//!   Nothing goes on until a later ticket's screen lets a person solve it.
//! - A file the job received for one item is not received again for another:
//!   the second item's receipt names the first (`same_as`).
//!
//! # One receipt
//!
//! Each effect on a file is preceded by a record of its intent and followed by
//! a record of its result:
//!
//! 1. `intended`: the attempt's ID and its own temporary folder, before
//!    anything is fetched; then the length the source announced.
//! 2. The bytes go to `.tmp/<attempt>/<name>`, hashed as they come, and the
//!    file is synced. A length other than the announced one fails the attempt
//!    and its bytes are removed.
//! 3. `fetched`: the length, the SHA-256, the temporary file's object
//!    (`dev:ino`) and the path it is to be published at.
//! 4. The file is published by a rename that replaces nothing, and its folder
//!    synced.
//! 5. `done`, and the temporary folder goes.
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
//! | `intended` | a temporary file of the announced length | taken as fetched, then published |
//! | `intended` | any other temporary file (no length was announced, or it is longer) | `held` |
//! | `fetched` | the temporary file, same object, length and hash | published |
//! | `fetched` | no temporary file, the planned path is the recorded object with its length and hash | `done` |
//! | `fetched` | anything else | `held` |
//! | `done` | the path has the recorded length and hash | reused |
//! | `done` | anything else | `held` |
//!
//! A held file holds its item, and a held item holds its job: the runner does
//! not take it up again by itself. Its temporary file and anything at its path
//! stay as they are.

use std::{io, path::Path};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use trss_core::{Clock, Millis};
use trss_subtitles::{Opened, PostFile, Sources};
use url::Url;

use crate::{
    area::{self, ReceiveArea},
    model::{FileState, ItemState, JobState, StepKind, StepState, Wait},
    store::{FileRow, ItemRow, JobError, JobStore},
};

/// What the job's log and screen say for a post no source reads.
pub const NO_SOURCE: &str = "이 출처에서 받는 방법을 아직 몰라요";

/// How many times a job is started without a run ending before the runner
/// holds it instead: a job whose runs keep stopping in an error or a crash
/// would otherwise block the line for good. A run that ends, waiting or not,
/// starts the count again.
pub const MAX_STARTS: i64 = 5;

/// Carries out jobs. Cheap to clone.
#[derive(Clone)]
pub struct Runner {
    store: JobStore,
    sources: Sources,
    area: ReceiveArea,
    clock: Clock,
}

/// How one file of an item came out.
enum Receipt {
    Received,
    Failed(String),
    Held(String),
    /// Shutdown was asked for in the middle; the item stays `running`.
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
        }
    }

    pub fn store(&self) -> &JobStore {
        &self.store
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
            if let ItemEnd::Interrupted = self.run_item(id, item, cancel).await? {
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
        // first, whatever the post says now.
        for unfinished in item
            .files
            .iter()
            .filter(|r| matches!(r.state, FileState::Intended | FileState::Fetched))
        {
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

        let fail = |reason: String| async move {
            let now = self.now();
            self.store
                .set_item(item.id, ItemState::Failed, None, Some(reason), now)
                .await
        };
        let Ok(post) = Url::parse(&item.post_url) else {
            fail("게시물 주소를 읽지 못했어요".to_owned()).await?;
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
        let opened = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(ItemEnd::Interrupted),
            opened = source.open(&post) => opened,
        };
        let files = match opened {
            Err(failure) => {
                self.store
                    .event(
                        job,
                        format!("{ep}: 게시물을 열지 못했어요"),
                        Some(failure.reason.clone()),
                        self.now(),
                    )
                    .await?;
                fail(failure.reason).await?;
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
            Ok(Opened::Files(files)) => files,
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
            fail("게시물에 받을 파일이 없어요".to_owned()).await?;
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
                Receipt::Failed(reason) => {
                    failed.get_or_insert(reason);
                }
                Receipt::Held(reason) => {
                    held.get_or_insert(reason);
                }
                Receipt::Interrupted => return Ok(ItemEnd::Interrupted),
            }
        }
        let (state, reason) = match (held, failed) {
            (Some(reason), _) => (ItemState::Held, Some(reason)),
            (None, Some(reason)) => (ItemState::Failed, Some(reason)),
            (None, None) => (ItemState::Done, None),
        };
        self.store
            .set_item(item.id, state, None, reason, self.now())
            .await?;
        Ok(ItemEnd::Settled)
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
        for unfinished in receipts
            .iter()
            .filter(|r| matches!(r.state, FileState::Intended | FileState::Fetched))
        {
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

        // A new attempt.
        let attempt = uuid::Uuid::new_v4().to_string();
        let temp_rel = ReceiveArea::temp_dir(&attempt);
        let now = self.now();
        self.store
            .file_intend(FileRow {
                id: attempt.clone(),
                item_id: item.id,
                file_key: file.key.clone(),
                name: name.clone(),
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
            })
            .await?;

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
                return self
                    .fail_file(job, ep, &attempt, &name, None, failure.reason)
                    .await;
            }
        };
        self.store
            .file_expect(&attempt, fetch.expected_size, self.now())
            .await?;

        let temp_dir = self.area.at(&temp_rel);
        let temp = temp_dir.join(&name);
        let written: io::Result<(u64, String)> = async {
            tokio::fs::create_dir_all(&temp_dir).await?;
            let mut out = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .await?;
            let mut hasher = Sha256::new();
            let mut size = 0u64;
            loop {
                let piece = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "shutdown"));
                    }
                    piece = fetch.chunk() => piece,
                };
                let piece = match piece {
                    Ok(Some(piece)) => piece,
                    Ok(None) => break,
                    Err(failure) => return Err(io::Error::other(failure.reason)),
                };
                hasher.update(&piece);
                size += piece.len() as u64;
                out.write_all(&piece).await?;
            }
            out.sync_all().await?;
            Ok((size, area::hex(&hasher.finalize())))
        }
        .await;
        let (size, sha256) = match written {
            Ok(written) => written,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {
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
            Err(err) => {
                return self
                    .fail_file(job, ep, &attempt, &name, Some(&temp_dir), err.to_string())
                    .await;
            }
        };
        if size == 0 {
            return self
                .fail_file(
                    job,
                    ep,
                    &attempt,
                    &name,
                    Some(&temp_dir),
                    "받은 파일이 비어 있어요".into(),
                )
                .await;
        }
        if let Some(expected) = fetch.expected_size.filter(|e| *e != size) {
            return self
                .fail_file(
                    job,
                    ep,
                    &attempt,
                    &name,
                    Some(&temp_dir),
                    format!(
                        "사이트가 알린 크기({expected}바이트)와 받은 크기({size}바이트)가 달라요"
                    ),
                )
                .await;
        }
        let object = match std::fs::metadata(&temp) {
            Ok(meta) => area::object_of(&meta),
            Err(err) => {
                return self
                    .fail_file(job, ep, &attempt, &name, Some(&temp_dir), err.to_string())
                    .await
            }
        };
        let path = self.plan_path(job, &name).await?;
        self.store
            .file_fetched(&attempt, size, sha256, object, path.clone(), self.now())
            .await?;
        self.publish(job, ep, &attempt, &temp_rel, &name, &path, size)
            .await
    }

    /// Publishes a fetched file's temporary file at `path` and records it done.
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
        let target = self.area.at(path);
        let published = (|| {
            let folder = target.parent().expect("a path in a job's folder");
            // Synced every time: a crash may have left a folder made before
            // its entry was synced.
            std::fs::create_dir_all(folder)?;
            if let Some(parent) = folder.parent() {
                area::sync_dir(parent)?;
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
        self.store
            .file_end(attempt, FileState::Done, None, self.now())
            .await?;
        let _ = std::fs::remove_dir_all(self.area.at(temp_rel));
        self.store
            .event(
                job,
                format!("{ep}: 파일을 받았어요"),
                Some(format!("{name} · {}", human_size(size))),
                self.now(),
            )
            .await?;
        Ok(Receipt::Received)
    }

    /// A free path for `name` in the job's folder: on the disk and among the
    /// job's receipts.
    async fn plan_path(&self, job: &str, name: &str) -> Result<String, JobError> {
        let taken = self.store.paths_of(job).await?;
        let dir = ReceiveArea::job_dir(job);
        for candidate in area::name_candidates(name) {
            let path = format!("{dir}/{candidate}");
            let on_disk = std::fs::symlink_metadata(self.area.at(&path)).is_ok();
            if !on_disk && !taken.contains(&path) {
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
                    let path = self.plan_path(job, &r.name).await?;
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
        let recorded = |facts: &(u64, String, String)| {
            Some(facts.0) == r.size
                && Some(&facts.1) == r.sha256.as_ref()
                && Some(&facts.2) == r.object.as_ref()
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
                self.store
                    .file_end(&r.id, FileState::Done, None, now)
                    .await?;
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

    async fn fail_file(
        &self,
        job: &str,
        ep: &str,
        attempt: &str,
        name: &str,
        temp_dir: Option<&Path>,
        reason: String,
    ) -> Result<Receipt, JobError> {
        if let Some(dir) = temp_dir {
            let _ = tokio::fs::remove_dir_all(dir).await;
        }
        let now = self.now();
        self.store
            .file_end(attempt, FileState::Failed, Some(reason.clone()), now)
            .await?;
        self.store
            .event(
                job,
                format!("{ep}: 파일을 받지 못했어요"),
                Some(format!("{name} · {reason}")),
                now,
            )
            .await?;
        Ok(Receipt::Failed(reason))
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
        } else if waits(Wait::Subtitle).is_some() {
            (
                JobState::Waiting,
                Some(Wait::Subtitle),
                Some(NO_SOURCE.to_owned()),
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
