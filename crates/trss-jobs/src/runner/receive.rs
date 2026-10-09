//! Receiving, publishing and recovering the files of an item
//! (`docs/specs/jobs.md`, 체크포인트와 중단 복구). The runner's loop decides
//! the order and asks [`Receiver`] for one file ([`Receiver::receive`]) or for
//! what an earlier start left unfinished ([`Receiver::recover`]); everything a
//! file's receipt goes through is here, and nothing of it is anywhere else.
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
//!    ([`area::object_of`]) and the path it is to be published at.
//! 4. The bytes are checked ([`trss_subtitles::verify`]): bytes that are not
//!    a file (nothing, a web page, a ZIP whose CRC fails, a name's format the
//!    bytes are not) fail the receipt as `not_a_file`. The failure is recorded
//!    with the path still named, then the bytes are removed, then the path.
//!    A check that breaks off (a panic) holds the receipt with its bytes.
//! 5. The file is published by a rename that replaces nothing, and its folder
//!    synced.
//! 6. `done` with the format the check found, and the temporary folder goes.
//!
//! A Google Drive font the work keeps already is first read with a `HEAD`,
//! and is not received when its size and `Last-Modified` are those it was
//! kept with ([`crate::place::unchanged`]): nothing is fetched, so nothing is
//! intended; the receipt is written `done` at once, with no path and no
//! temporary folder, naming the font it uses (`unchanged_asset`). A `HEAD`
//! that fails or gives other values leaves the file to the steps above.
//!
//! # Restart
//!
//! A worker that dies leaves its job `running`; the next one to hold the
//! worker lock claims it again ([`JobRun::claim_next`]). The lock is the
//! kernel's and goes with the process, so no earlier worker still writes.
//! Before receiving a file again the runner compares its unfinished receipt
//! with the disk ([`Receiver::recover`]):
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
//! | nothing (cut between a font's `HEAD` and its record) | nothing of it | the font is looked at again: a new `HEAD` |
//! | `done`, not received (`unchanged_asset`) | no file of its own; the font not removed, its file with its recorded length and hash | the package uses the font when its row is stored |
//! | `done`, not received | the font removed by a cleanup, its file gone or other bytes | at its row's store: `abandoned` with the receipts that share it, their rows not kept gone, their items `pending`; the job goes back in line and receives the file |
//!
//! A file is the recorded object by its inode ([`trss_core::file_id::same_recorded_file`]): a
//! machine restarted meanwhile may have mounted its file system with another
//! device number.
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
use trss_core::{file_id::same_recorded_file, Clock, Millis};
use trss_subtitles::{verify, Failure, FailureKind, PostFile};
use url::Url;

use super::{described, pause, wait_text};
use crate::{
    area::{self, ReceiveArea},
    model::FileState,
    place::{files::remove_known, Placer},
    store::{snapshot_json, FileProblem, FileRow, ItemRow, JobError, JobRun},
};

/// Receives, publishes and recovers files. Cheap to clone; a clone is the same
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
                temp_dir: None,
                expected_size: Some(kept.size),
                size: Some(kept.size),
                sha256: Some(kept.sha256.clone()),
                object: None,
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
                trss_core::files::sync_dir(parent)?;
                if parent == self.area.root() {
                    break;
                }
                level = parent;
            }
            trss_core::files::rename_noreplace(&temp, &target)?;
            trss_core::files::sync_dir(folder)
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

    /// Compares an unfinished receipt with the disk (see the module docs).
    /// Returns why it is held, or `None` when it was settled (published,
    /// found done, or abandoned).
    pub(super) async fn recover(
        &self,
        job: &str,
        ep: &str,
        r: &FileRow,
    ) -> Result<Option<String>, JobError> {
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
                && r.object
                    .as_ref()
                    .is_some_and(|o| same_recorded_file(o, &facts.2))
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
                    let synced =
                        std::fs::File::open(&temp).and_then(|f| trss_core::files::sync_file(&f));
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
    /// A fetched attempt fails through [`Receiver::fail_fetched`].
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
    /// start removes ([`Receiver::finish_failed`]). Bytes that cannot be removed
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
}

fn human_size(size: u64) -> String {
    match size {
        s if s < 1024 => format!("{s} B"),
        s if s < 1024 * 1024 => format!("{:.1} KB", s as f64 / 1024.0),
        s => format!("{:.1} MB", s as f64 / 1024.0 / 1024.0),
    }
}
