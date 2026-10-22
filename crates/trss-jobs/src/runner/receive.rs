//! Receiving and publishing the files of an item (`docs/requirements/jobs.md`,
//! 체크포인트와 중단 복구). The runner's loop decides the order and asks
//! [`Receiver`] for one file ([`Receiver::receive`]), once it has cleared what
//! an earlier run left ([`Receiver::start_over`]); everything a file's receipt
//! goes through is here, and nothing of it is anywhere else.
//!
//! # One receipt
//!
//! 1. `intended`: the attempt's ID, before anything is fetched, so the job's
//!    detail shows the file as being received; then what the answer says (its
//!    status and media type, and the file's name when the answer gives one
//!    the post did not: Drive's).
//! 2. The bytes go to `.partial/<attempt>/<name>`, hashed as they come, and
//!    the file is synced. A length other than the one the source announced
//!    fails the attempt, and its bytes are removed.
//! 3. The bytes are checked ([`trss_subtitles::verify`]): bytes that are not
//!    a file (nothing, a web page, a ZIP whose CRC fails, a name's format the
//!    bytes are not) fail the receipt as `not_a_file`, and they are removed.
//!    A check that breaks off (a panic) holds the receipt; its bytes stay in
//!    `.partial/` until `trss` starts again.
//! 4. The file is published at a path that no file on the disk and no receipt
//!    of the job takes, by a rename that replaces nothing, and its folders
//!    are synced.
//! 5. `done` with the path, the length, the SHA-256 and the format the check
//!    found, synced; then the attempt's folder goes.
//!
//! A Google Drive font the work keeps already is first read with a `HEAD`,
//! and is not received when its size and `Last-Modified` are those it was
//! kept with ([`crate::place::unchanged`]): nothing is fetched, so nothing is
//! intended; the receipt is written `done` at once, with no path, naming the
//! font it uses (`unchanged_asset`). A `HEAD` that fails or gives other
//! values leaves the file to the steps above.
//!
//! # A run cut short
//!
//! A run can stop before a receipt is `done`: `trss` dies (the job is then
//! `interrupted` until a person restarts it, [`JobRun::interrupt_running`]),
//! or an error ends the run and the same process takes the job up again
//! ([`JobRun::claim_next`]). Nothing such a run left is compared with the
//! disk. Before a job receives ([`Receiver::start_over`]):
//!
//! - its receipts still `intended` are removed with their attempts' folders,
//!   and their files are received from the start;
//! - every file in the job's folder that no receipt names is removed: a
//!   rename done before its `done` was recorded leaves one. The members of
//!   unpacked archives (`.unpack/`) are the unpacking's, and stay.
//!
//! A `done` receipt is never received again: its bytes are checked and the
//! file is used as it is ([`Receiver::reuse`]); bytes other than the recorded
//! ones hold it. `trss` empties `.partial/` when it starts
//! ([`ReceiveArea::empty_partial`]).
//!
//! A held file holds its item, and a held item holds its job: the runner does
//! not take it up again by itself.

use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use trss_core::{Clock, Millis};
use trss_subtitles::{verify, Failure, FailureKind, PostFile};
use url::Url;

use super::{described, pause, wait_text};
use crate::{
    area::{self, ReceiveArea},
    model::FileState,
    place::Placer,
    store::{snapshot_json, FileProblem, FileRow, ItemRow, JobError, JobRun, Received},
};

/// Receives and publishes files. Cheap to clone; a clone is the same
/// receiver, with the same waits.
#[derive(Clone)]
pub(super) struct Receiver {
    store: JobRun,
    /// Asked for the Drive font the work keeps already
    /// ([`Receiver::unchanged`]).
    placer: Placer,
    area: ReceiveArea,
    clock: Clock,
    /// The waits before each retry of a network failure ([`RETRY_WAITS`]
    /// unless set otherwise); the runner uses the same ones to open a post.
    ///
    /// [`RETRY_WAITS`]: super::RETRY_WAITS
    retry_waits: Arc<[Duration]>,
}

/// How one file of an item came out.
pub(super) enum Receipt {
    Received,
    Failed(FileProblem),
    /// A network failure a next attempt may get past: this attempt is
    /// `abandoned`.
    Retry(FileProblem),
    Held(String),
    /// Shutdown was asked for in the middle; the item stays `running`.
    Interrupted,
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

/// How a write of the bytes to the temporary file ended badly.
enum Written {
    Io(io::Error),
    Source(Failure),
    Interrupted,
}

impl Receiver {
    pub(super) fn new(store: JobRun, placer: Placer, area: ReceiveArea, clock: Clock) -> Receiver {
        Receiver {
            store,
            placer,
            area,
            clock,
            retry_waits: Arc::new(super::RETRY_WAITS),
        }
    }

    /// The same receiver with other waits before a retry; as many retries as
    /// waits.
    pub(super) fn with_retry_waits(mut self, waits: Vec<Duration>) -> Receiver {
        self.retry_waits = waits.into();
        self
    }

    /// The waits before each retry of a network failure.
    pub(super) fn retry_waits(&self) -> &[Duration] {
        &self.retry_waits
    }

    fn now(&self) -> Millis {
        (self.clock)()
    }

    /// Clears what an earlier run of `job` left before it receives again (see
    /// the module docs): the receipts still being received, with their
    /// attempts' folders, and the files in the job's folder that no receipt
    /// names. Logged when it found any.
    pub(super) async fn start_over(&self, job: &str) -> Result<(), JobError> {
        let forgotten = self.store.forget_unfinished(job).await?;
        for id in &forgotten {
            let _ = std::fs::remove_dir_all(self.area.at(&ReceiveArea::partial_dir(id)));
        }
        let named = self.store.named_paths(job).await?;
        let (area, job_id) = (self.area.clone(), job.to_owned());
        let swept = tokio::task::spawn_blocking(move || sweep_unnamed(&area, &job_id, &named))
            .await
            .map_err(|err| JobError::Other(err.to_string()))?;
        let mut detail = Vec::new();
        if !forgotten.is_empty() {
            detail.push(format!("받던 파일 {}개", forgotten.len()));
        }
        if swept.removed > 0 {
            detail.push(format!("기록에 없는 파일 {}개 지움", swept.removed));
        }
        if !detail.is_empty() {
            self.store
                .event(
                    job,
                    "끊긴 받기를 처음부터 다시 해요".to_owned(),
                    Some(detail.join(" · ")),
                    self.now(),
                )
                .await?;
        }
        for (path, err) in swept.failed {
            self.store
                .event(
                    job,
                    "기록에 없는 받은 파일을 지우지 못했어요".to_owned(),
                    Some(format!("{path}: {err}")),
                    self.now(),
                )
                .await?;
        }
        Ok(())
    }

    /// Receives one file of `item`, or finds it received (see the module docs).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn receive(
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

        if receipts.iter().any(|r| r.state == FileState::Held) {
            return Ok(Receipt::Held("같은 파일이 보류돼 있어요".to_owned()));
        }
        if let Some(original) = receipts
            .iter()
            .find(|r| r.state == FileState::Done && r.same_as.is_none())
        {
            return self.reuse(job, item, ep, original, &receipts).await;
        }
        if let Some(receipt) = self
            .unchanged(job, item, ep, source, post, file, &name, cancel)
            .await?
        {
            return Ok(receipt);
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

    /// A Google Drive font the work keeps already, left unreceived when a
    /// `HEAD` gives the size and `Last-Modified` it was kept with
    /// ([`crate::place::unchanged`]): its receipt is `done` at once, naming
    /// the font. `None`: the file is received. No `HEAD` is asked for a file
    /// with no such font.
    #[allow(clippy::too_many_arguments)]
    async fn unchanged(
        &self,
        job: &str,
        item: &ItemRow,
        ep: &str,
        source: &trss_subtitles::Source,
        post: &Url,
        file: &PostFile,
        name: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<Receipt>, JobError> {
        if file.is_staged() || trss_subtitles::drive::id_of(&file.key).is_none() {
            return Ok(None);
        }
        // A folder refused is the receipt's to record (`attempt`).
        let folder = match file.folder.as_deref().map(area::safe_folder) {
            None => None,
            Some(Ok(folder)) => folder,
            Some(Err(_)) => return Ok(None),
        };
        let Some(kept) = self.placer.unchanged_font(job, &file.key).await? else {
            return Ok(None);
        };
        let keys = [file.key.clone()];
        let answers = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(Some(Receipt::Interrupted)),
            answers = source.recheck(post, &keys) => answers,
        };
        let info = answers
            .into_iter()
            .find(|(key, _)| *key == file.key)
            .map(|(_, info)| info);
        let (message, detail) = match &info {
            Some(Ok(info))
                if info.size == Some(kept.size)
                    && info.last_modified.as_deref() == Some(kept.last_modified.as_str()) =>
            {
                ("", None)
            }
            Some(Ok(_)) => (
                "보관한 폰트와 크기나 수정 시각이 달라 받아요",
                Some(name.to_owned()),
            ),
            Some(Err(failure)) => (
                "폰트가 바뀌었는지 확인하지 못해 받아요",
                Some(format!("{name} · {}", described(&failure.into()))),
            ),
            None => (
                "폰트가 바뀌었는지 확인하지 못해 받아요",
                Some(name.to_owned()),
            ),
        };
        if !message.is_empty() {
            self.store
                .event(job, format!("{ep}: {message}"), detail, self.now())
                .await?;
            return Ok(None);
        }
        let mut snapshot = file.snapshot.clone();
        snapshot.push(
            trss_subtitles::http::LAST_MODIFIED,
            kept.last_modified.clone(),
        );
        snapshot.push(trss_subtitles::drive::CONTENT_LENGTH, kept.size.to_string());
        let now = self.now();
        self.store
            .file_unchanged(FileRow {
                id: uuid::Uuid::new_v4().to_string(),
                item_id: item.id,
                file_key: file.key.clone(),
                name: kept.name.clone(),
                state: FileState::Done,
                same_as: None,
                size: Some(kept.size),
                sha256: Some(kept.sha256.clone()),
                path: None,
                reason: None,
                created_at: now,
                format: Some(verify::Format::Other),
                failure: None,
                http_status: None,
                content_type: None,
                response_size: None,
                snapshot: snapshot_json(&snapshot),
                kind: None,
                archive: None,
                folder,
                cleared_at: None,
                volume_of: None,
                unpacked_at: None,
                unpack_error: None,
                unpack_tries: 0,
                unpack_failure: None,
                unpack_retry_at: None,
                unchanged_asset: Some(kept.asset_id),
            })
            .await?;
        self.store
            .event(
                job,
                format!("{ep}: 바뀌지 않은 폰트라 받지 않았어요"),
                Some(format!(
                    "{} · {} · 크기와 수정 시각이 보관한 폰트와 같아요",
                    kept.name,
                    human_size(kept.size)
                )),
                now,
            )
            .await?;
        Ok(Some(Receipt::Received))
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
                size: None,
                sha256: None,
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
                cleared_at: None,
                volume_of: None,
                unpacked_at: None,
                unpack_error: None,
                unpack_tries: 0,
                unpack_failure: None,
                unpack_retry_at: None,
                unchanged_asset: None,
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
        // on.
        let answered = fetch.name.as_deref().map(area::safe_name);
        self.store
            .file_answer(
                &attempt,
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

        let partial = self.area.at(&ReceiveArea::partial_dir(&attempt));
        let temp = partial.join(name);
        let written: Result<(u64, String), Written> = async {
            tokio::fs::create_dir_all(&partial)
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
                // The bytes are known to be incomplete; the next run receives
                // the file again.
                let _ = tokio::fs::remove_dir_all(&partial).await;
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
                        Some(&partial),
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
                        Some(&partial),
                        FileProblem::local(err.to_string()),
                        false,
                    )
                    .await;
            }
        };
        if size == 0 {
            let problem = not_a_file("받은 파일이 비어 있어요".into(), 0);
            return self
                .fail_file(job, ep, &attempt, name, Some(&partial), problem, false)
                .await;
        }
        if let Some(expected) = fetch.expected_size.filter(|e| *e != size) {
            let problem = not_a_file(
                format!("사이트가 알린 크기({expected}바이트)와 받은 크기({size}바이트)가 달라요"),
                size,
            );
            return self
                .fail_file(job, ep, &attempt, name, Some(&partial), problem, false)
                .await;
        }
        let format = match check(temp.clone(), name.to_owned()).await {
            Checked::File(format) => format,
            Checked::NotAFile(problem) => {
                let problem = FileProblem {
                    size: Some(size),
                    ..problem
                };
                return self
                    .fail_file(job, ep, &attempt, name, Some(&partial), problem, false)
                    .await;
            }
            Checked::Broken(reason) => {
                return self.hold_file(job, ep, &attempt, name, reason).await
            }
        };
        let path = self.plan_path(job, folder.as_deref(), name).await?;
        if let Err(err) = self.publish(&temp, &path) {
            let reason = match err.kind() {
                io::ErrorKind::AlreadyExists => "받을 자리에 다른 파일이 있어요".to_owned(),
                _ => format!("받은 파일을 옮기지 못했어요: {err}"),
            };
            return self.hold_file(job, ep, &attempt, name, reason).await;
        }
        let received = Received {
            size,
            sha256,
            path,
            format,
        };
        self.store.file_done(&attempt, received, self.now()).await?;
        let _ = std::fs::remove_dir_all(&partial);
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

    /// Publishes the checked bytes at `temp` at `path` (in the area) by a
    /// rename that replaces nothing, and syncs the folders.
    fn publish(&self, temp: &Path, path: &str) -> io::Result<()> {
        let target = self.area.at(path);
        let folder = target.parent().expect("a path in a job's folder");
        // Synced every time: a crash may have left a folder made before
        // its entry was synced. Every level down from the area has its
        // entry synced, a file's own folders included.
        std::fs::create_dir_all(folder)?;
        let mut level = folder;
        while let Some(parent) = level.parent() {
            trss_core::files::sync_dir(parent)?;
            if parent == self.area.root() {
                break;
            }
            level = parent;
        }
        trss_core::files::rename_noreplace(temp, &target)?;
        trss_core::files::sync_dir(folder)
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
        // A receipt already stored left the receive area: its bytes were
        // checked when they came and are kept in the work folder now. One
        // that was not received has no bytes here: its font is looked at when
        // it is stored ([`crate::place::unchanged`]).
        let matches = original.cleared_at.is_some()
            || original.unchanged_asset.is_some()
            || facts.as_ref().is_some_and(|(size, sha, _)| {
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
        if original.item_id == item.id && original.unchanged_asset.is_some() {
            // A run cut after the font was found unchanged: nothing was
            // received, and the font is looked at when it is stored.
            self.store
                .event(
                    job,
                    format!("{ep}: 바뀌지 않은 폰트라 받지 않았어요"),
                    Some(name.clone()),
                    self.now(),
                )
                .await?;
        } else if original.item_id == item.id {
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

    /// Removes an attempt's folder of bytes and ends it `failed` for
    /// `problem`; or, when `retry`, `abandoned` for the next attempt (the
    /// caller logs the retry).
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
}

/// What [`sweep_unnamed`] did.
struct Swept {
    removed: usize,
    /// The paths it could not remove, relative to the area, with why.
    failed: Vec<(String, io::Error)>,
}

/// Removes every file in `job`'s folder of the area whose path (relative to
/// the area) is not in `named`, and the folders that are left empty, the
/// job's own aside. `.unpack/` is the unpacking's and is not looked into. A
/// link is removed as itself, never followed.
fn sweep_unnamed(area: &ReceiveArea, job: &str, named: &HashSet<String>) -> Swept {
    let mut swept = Swept {
        removed: 0,
        failed: Vec::new(),
    };
    let root = area.at(&ReceiveArea::job_dir(job));
    sweep_dir(&root, &ReceiveArea::job_dir(job), true, named, &mut swept);
    swept
}

fn sweep_dir(dir: &Path, relative: &str, top: bool, named: &HashSet<String>, swept: &mut Swept) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return,
        Err(err) => {
            swept.failed.push((relative.to_owned(), err));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                swept.failed.push((relative.to_owned(), err));
                continue;
            }
        };
        let name = entry.file_name();
        // A name that is not UTF-8 is no receipt's.
        let path = name
            .to_str()
            .map(|name| format!("{relative}/{name}"))
            .unwrap_or_default();
        if top && name == ".unpack" {
            continue;
        }
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if is_dir {
            let shown = format!("{relative}/{}", name.to_string_lossy());
            sweep_dir(&entry.path(), &shown, false, named, swept);
            // Only an empty folder goes.
            let _ = std::fs::remove_dir(entry.path());
        } else if !named.contains(&path) {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => swept.removed += 1,
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => swept
                    .failed
                    .push((format!("{relative}/{}", name.to_string_lossy()), err)),
            }
        }
    }
}

fn human_size(size: u64) -> String {
    match size {
        s if s < 1024 => format!("{s} B"),
        s if s < 1024 * 1024 => format!("{:.1} KB", s as f64 / 1024.0),
        s => format!("{:.1} MB", s as f64 / 1024.0 / 1024.0),
    }
}
