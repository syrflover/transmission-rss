//! The records of the files a job receives: an intent before anything is
//! fetched, then what came, so a restarted worker can reuse or hold them.

use rusqlite::{params, Connection};
use trss_core::Millis;
use trss_subtitles::{verify::Format, FailureKind};

use super::{
    rows::{file_row, FILE_COLUMNS},
    FileRow, JobError, JobStore,
};
use crate::model::FileState;

/// Runs a write of a file receipt with every commit synced: the record of an
/// intent must outlive a power loss as surely as the file effect after it,
/// which is synced too. The connection's usual `NORMAL` comes back after.
fn durable(
    c: &Connection,
    write: impl FnOnce(&Connection) -> rusqlite::Result<usize>,
) -> Result<usize, JobError> {
    c.pragma_update(None, "synchronous", "FULL")?;
    let written = write(c);
    c.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(written?)
}

/// Why an attempt to receive a file failed, with the facts of the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileProblem {
    pub reason: String,
    /// `None` for a failure of this app (its disk), not of the source.
    pub class: Option<FailureKind>,
    pub status: Option<u16>,
    pub content_type: Option<String>,
    pub size: Option<u64>,
}

impl FileProblem {
    /// A failure of this app, not of the source.
    pub fn local(reason: impl Into<String>) -> FileProblem {
        FileProblem {
            reason: reason.into(),
            class: None,
            status: None,
            content_type: None,
            size: None,
        }
    }
}

impl From<&trss_subtitles::Failure> for FileProblem {
    fn from(f: &trss_subtitles::Failure) -> FileProblem {
        FileProblem {
            reason: f.reason.clone(),
            class: Some(f.kind),
            status: f.status,
            content_type: f.content_type.clone(),
            size: f.size,
        }
    }
}

/// A status the records keep: a real HTTP one (the schema checks 100–599).
fn kept_status(status: Option<u16>) -> Option<u16> {
    status.filter(|s| (100..=599).contains(s))
}

/// A size as the records keep it: one past SQLite's integers is dropped
/// rather than wrapped.
fn stored_size(size: u64) -> Option<i64> {
    i64::try_from(size).ok()
}

/// A snapshot as the records keep it: a JSON array of `[name, value]` pairs,
/// or nothing when it is empty.
pub fn snapshot_json(snapshot: &trss_subtitles::Snapshot) -> Option<String> {
    if snapshot.is_empty() {
        return None;
    }
    let pairs: Vec<[&str; 2]> = snapshot
        .entries()
        .iter()
        .map(|(name, value)| [name.as_str(), value.as_str()])
        .collect();
    Some(serde_json::to_string(&pairs).expect("strings serialize"))
}

impl JobStore {
    /// The receipts of `file_key` in the job, oldest first.
    pub async fn files_for_key(
        &self,
        job_id: &str,
        file_key: &str,
    ) -> Result<Vec<FileRow>, JobError> {
        let (id, key) = (job_id.to_owned(), file_key.to_owned());
        self.db
            .run(move |c| {
                let mut stmt = c.prepare_cached(&format!(
                    "{FILE_COLUMNS} WHERE job_id = ?1 AND file_key = ?2 ORDER BY created_at, id"
                ))?;
                let rows = stmt
                    .query_map(params![id, key], file_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }

    /// The paths in the receive area the job's receipts name or plan to.
    pub async fn paths_of(&self, job_id: &str) -> Result<Vec<String>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT path FROM subtitle_job_files
                     WHERE job_id = ?1 AND path IS NOT NULL AND state <> 'abandoned'",
                )?;
                let rows = stmt
                    .query_map([id], |r| r.get(0))?
                    .collect::<Result<Vec<String>, _>>()?;
                Ok(rows)
            })
            .await
    }

    /// Records a Google Drive font that is not received because it did not
    /// change ([`crate::place::unchanged`]): `done` at once, with no path and
    /// no temporary folder, naming the stored font it uses
    /// (`unchanged_asset`) with that font's size and SHA-256, and the snapshot
    /// with what the `HEAD` said. Nothing is fetched, so no intent comes
    /// before it.
    pub async fn file_unchanged(&self, file: FileRow) -> Result<(), JobError> {
        self.db
            .run(move |c| {
                durable(c, |c| {
                    c.prepare_cached(
                        "INSERT INTO subtitle_job_files
                         (id, job_id, item_id, file_key, name, state, expected_size, size,
                          sha256, format, snapshot, created_at, updated_at, folder,
                          unchanged_asset)
                     SELECT ?1, job_id, ?2, ?3, ?4, 'done', ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?11, ?12
                     FROM subtitle_job_items WHERE id = ?2",
                    )?
                    .execute(params![
                        file.id,
                        file.item_id,
                        file.file_key,
                        file.name,
                        file.expected_size.map(|s| s as i64),
                        file.size.map(|s| s as i64),
                        file.sha256,
                        file.format.map(Format::code),
                        file.snapshot,
                        file.created_at,
                        file.folder,
                        file.unchanged_asset,
                    ])
                })
                .and_then(|rows| match rows {
                    1 => Ok(()),
                    _ => Err(JobError::Missing("the item of a file not received")),
                })
            })
            .await
    }

    /// Records the intent to receive a file before anything is fetched.
    pub async fn file_intend(&self, file: FileRow) -> Result<(), JobError> {
        self.db
            .run(move |c| {
                durable(c, |c| {
                    c.prepare_cached(
                        "INSERT INTO subtitle_job_files
                         (id, job_id, item_id, file_key, name, state, temp_dir, snapshot,
                          created_at, updated_at, folder)
                     SELECT ?1, job_id, ?2, ?3, ?4, 'intended', ?5, ?7, ?6, ?6, ?8
                     FROM subtitle_job_items WHERE id = ?2",
                    )?
                    .execute(params![
                        file.id,
                        file.item_id,
                        file.file_key,
                        file.name,
                        file.temp_dir,
                        file.created_at,
                        file.snapshot,
                        file.folder
                    ])
                })
                .and_then(|rows| match rows {
                    1 => Ok(()),
                    _ => Err(JobError::Missing("the item of a file to receive")),
                })
            })
            .await
    }

    /// The length the source announced, before the bytes come.
    pub async fn file_expect(
        &self,
        id: &str,
        size: Option<u64>,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = id.to_owned();
        self.db
            .run(move |c| {
                durable(c, |c| {
                    c.prepare_cached(
                        "UPDATE subtitle_job_files SET expected_size = ?2, updated_at = ?3
                          WHERE id = ?1",
                    )?
                    .execute(params![id, size.and_then(stored_size), now])
                })
                .map(|_| ())
            })
            .await
    }

    /// The answer the bytes come in: the length it announced, its status and
    /// media type, what it added to the snapshot (`None` keeps it), and the
    /// name it gave the file when the post gave none (`None` keeps the
    /// post's). It is recorded before any byte is written, so the temporary
    /// file a restart looks for has the recorded name.
    #[allow(clippy::too_many_arguments)]
    pub async fn file_answer(
        &self,
        id: &str,
        size: Option<u64>,
        status: Option<u16>,
        content_type: Option<String>,
        snapshot: Option<String>,
        name: Option<String>,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = id.to_owned();
        self.db
            .run(move |c| {
                durable(c, |c| {
                    c.prepare_cached(
                        "UPDATE subtitle_job_files
                         SET expected_size = ?2, http_status = ?3, content_type = ?4,
                             snapshot = coalesce(?5, snapshot), name = coalesce(?7, name),
                             updated_at = ?6
                         WHERE id = ?1",
                    )?
                    .execute(params![
                        id,
                        size.and_then(stored_size),
                        kept_status(status),
                        content_type,
                        snapshot,
                        now,
                        name
                    ])
                })
                .map(|_| ())
            })
            .await
    }

    /// The bytes are in the temporary file and synced: their length, hash, the
    /// file's object and the path it is to be published at.
    pub async fn file_fetched(
        &self,
        id: &str,
        size: u64,
        sha256: String,
        object: String,
        path: String,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = id.to_owned();
        self.db
            .run(move |c| {
                durable(c, |c| {
                    c.prepare_cached(
                        "UPDATE subtitle_job_files
                     SET state = 'fetched', size = ?2, sha256 = ?3, object = ?4, path = ?5,
                         updated_at = ?6
                     WHERE id = ?1",
                    )?
                    .execute(params![
                        id,
                        stored_size(size),
                        sha256,
                        object,
                        path,
                        now
                    ])
                })
                .map(|_| ())
            })
            .await
    }

    /// Ends a receipt in `state` (`done`, `held`, `failed`, `abandoned`).
    pub async fn file_end(
        &self,
        id: &str,
        state: FileState,
        reason: Option<String>,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = id.to_owned();
        self.db
            .run(move |c| {
                durable(c, |c| {
                    c.prepare_cached(
                        "UPDATE subtitle_job_files SET state = ?2, reason = ?3, updated_at = ?4
                     WHERE id = ?1",
                    )?
                    .execute(params![id, state, reason, now])
                })
                .map(|_| ())
            })
            .await
    }

    /// Ends a receipt `done`, its bytes checked to be `format`.
    pub async fn file_done(&self, id: &str, format: Format, now: Millis) -> Result<(), JobError> {
        let id = id.to_owned();
        self.db
            .run(move |c| {
                durable(c, |c| {
                    c.prepare_cached(
                        "UPDATE subtitle_job_files
                         SET state = 'done', reason = NULL, format = ?2, updated_at = ?3
                         WHERE id = ?1",
                    )?
                    .execute(params![id, format.code(), now])
                })
                .map(|_| ())
            })
            .await
    }

    /// Ends a receipt `failed`, or `abandoned` when it is to be tried again,
    /// for `problem`. The answer's facts it does not have stay as recorded.
    /// A planned path stays until its bytes are gone ([`JobStore::file_clear_path`]):
    /// a crash in between leaves a record that still names them.
    pub async fn file_fail(
        &self,
        id: &str,
        state: FileState,
        problem: FileProblem,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = id.to_owned();
        self.db
            .run(move |c| {
                durable(c, |c| {
                    c.prepare_cached(
                        "UPDATE subtitle_job_files
                         SET state = ?2, reason = ?3, failure = ?4,
                             http_status = coalesce(?5, http_status),
                             content_type = coalesce(?6, content_type),
                             response_size = ?7, updated_at = ?8
                         WHERE id = ?1",
                    )?
                    .execute(params![
                        id,
                        state,
                        problem.reason,
                        problem.class.map(FailureKind::code),
                        kept_status(problem.status),
                        problem.content_type,
                        problem.size.and_then(stored_size),
                        now
                    ])
                })
                .map(|_| ())
            })
            .await
    }

    /// Frees a failed receipt's planned path once its bytes are gone, so the
    /// name is free for the next attempt.
    pub async fn file_clear_path(&self, id: &str, now: Millis) -> Result<(), JobError> {
        let id = id.to_owned();
        self.db
            .run(move |c| {
                durable(c, |c| {
                    c.prepare_cached(
                        "UPDATE subtitle_job_files SET path = NULL, updated_at = ?2 WHERE id = ?1",
                    )?
                    .execute(params![id, now])
                })
                .map(|_| ())
            })
            .await
    }

    /// Records for another item that its file is `original`'s receipt.
    pub async fn file_share(
        &self,
        id: &str,
        item_id: i64,
        original: &FileRow,
        now: Millis,
    ) -> Result<(), JobError> {
        let (id, original) = (id.to_owned(), original.clone());
        self.db
            .run(move |c| {
                durable(c, |c| {
                    c.prepare_cached(
                        "INSERT INTO subtitle_job_files
                         (id, job_id, item_id, file_key, name, state, same_as, size, sha256,
                          object, path, format, http_status, content_type, snapshot,
                          created_at, updated_at, folder, cleared_at, unchanged_asset)
                     SELECT ?1, job_id, ?2, file_key, name, 'done', id, size, sha256, object,
                            path, format, http_status, content_type, snapshot, ?3, ?3, folder,
                            cleared_at, unchanged_asset
                     FROM subtitle_job_files WHERE id = ?4",
                    )?
                    .execute(params![id, item_id, now, original.id])
                })
                .map(|_| ())
            })
            .await
    }
}
