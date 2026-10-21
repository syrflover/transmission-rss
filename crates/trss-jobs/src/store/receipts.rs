//! The records of the files a job receives: an `intended` receipt while a
//! file is being received, so the job's detail shows it, then what came. A
//! file that is `done` is never received again; one an earlier run left
//! unfinished is forgotten and received anew ([`JobRun::forget_unfinished`]).

use std::collections::HashSet;

use rusqlite::{params, Connection};
use trss_core::Millis;
use trss_subtitles::{verify::Format, FailureKind};

use super::{
    rows::{file_row, FILE_COLUMNS},
    FileRow, JobError, JobRun,
};
use crate::model::FileState;

/// Runs a write of a file receipt with every commit synced: the record of a
/// published file must outlive a power loss as surely as the file, which is
/// synced too. The connection's usual `NORMAL` comes back after.
fn durable(
    c: &Connection,
    write: impl FnOnce(&Connection) -> rusqlite::Result<usize>,
) -> Result<usize, JobError> {
    c.pragma_update(None, "synchronous", "FULL")?;
    let written = write(c);
    c.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(written?)
}

/// What a receipt records when its file is published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    pub size: u64,
    pub sha256: String,
    /// Where the file is, relative to the receive area.
    pub path: String,
    pub format: Format,
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

impl JobRun {
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

    /// The paths in the receive area the job's receipts name.
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
    /// change ([`crate::place::unchanged`]): `done` at once, with no path,
    /// naming the stored font it uses (`unchanged_asset`) with that font's
    /// size and SHA-256, and the snapshot with what the `HEAD` said. Nothing
    /// is fetched, so nothing is `intended` before it.
    pub async fn file_unchanged(&self, file: FileRow) -> Result<(), JobError> {
        self.db
            .run(move |c| {
                durable(c, |c| {
                    c.prepare_cached(
                        "INSERT INTO subtitle_job_files
                         (id, job_id, item_id, file_key, name, state, size, sha256, format,
                          snapshot, created_at, updated_at, folder, unchanged_asset)
                     SELECT ?1, job_id, ?2, ?3, ?4, 'done', ?5, ?6, ?7, ?8, ?9, ?9, ?10, ?11
                     FROM subtitle_job_items WHERE id = ?2",
                    )?
                    .execute(params![
                        file.id,
                        file.item_id,
                        file.file_key,
                        file.name,
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

    /// Records that a file is being received, before anything is fetched:
    /// the job's detail shows it as `받는 중` while its episode runs. Not
    /// synced: the next run removes such a receipt before it receives
    /// ([`JobRun::forget_unfinished`]), and nothing else reads it after a
    /// stop.
    pub async fn file_intend(&self, file: FileRow) -> Result<(), JobError> {
        self.db
            .run(move |c| {
                let rows = c
                    .prepare_cached(
                        "INSERT INTO subtitle_job_files
                         (id, job_id, item_id, file_key, name, state, snapshot, created_at,
                          updated_at, folder)
                     SELECT ?1, job_id, ?2, ?3, ?4, 'intended', ?6, ?5, ?5, ?7
                     FROM subtitle_job_items WHERE id = ?2",
                    )?
                    .execute(params![
                        file.id,
                        file.item_id,
                        file.file_key,
                        file.name,
                        file.created_at,
                        file.snapshot,
                        file.folder
                    ])?;
                match rows {
                    1 => Ok(()),
                    _ => Err(JobError::Missing("the item of a file to receive")),
                }
            })
            .await
    }

    /// The answer the bytes come in: its status and media type, what it
    /// added to the snapshot (`None` keeps it), and the name it gave the file
    /// when the post gave none (`None` keeps the post's). Written on an
    /// `intended` receipt, so not synced either ([`Self::file_intend`]).
    pub async fn file_answer(
        &self,
        id: &str,
        status: Option<u16>,
        content_type: Option<String>,
        snapshot: Option<String>,
        name: Option<String>,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = id.to_owned();
        self.db
            .run(move |c| {
                c.prepare_cached(
                    "UPDATE subtitle_job_files
                     SET http_status = ?2, content_type = ?3,
                         snapshot = coalesce(?4, snapshot), name = coalesce(?6, name),
                         updated_at = ?5
                     WHERE id = ?1",
                )?
                .execute(params![
                    id,
                    kept_status(status),
                    content_type,
                    snapshot,
                    now,
                    name
                ])?;
                Ok(())
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

    /// Ends a receipt `done` once its bytes are published at `path`: their
    /// length, SHA-256 and the `format` the check found. Synced before it
    /// returns, as the file is.
    pub async fn file_done(
        &self,
        id: &str,
        received: Received,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = id.to_owned();
        self.db
            .run(move |c| {
                durable(c, |c| {
                    c.prepare_cached(
                        "UPDATE subtitle_job_files
                         SET state = 'done', reason = NULL, size = ?2, sha256 = ?3, path = ?4,
                             format = ?5, updated_at = ?6
                         WHERE id = ?1",
                    )?
                    .execute(params![
                        id,
                        stored_size(received.size),
                        received.sha256,
                        received.path,
                        received.format.code(),
                        now
                    ])
                })
                .map(|_| ())
            })
            .await
    }

    /// Ends a receipt `failed`, or `abandoned` when it is to be tried again,
    /// for `problem`. The answer's facts it does not have stay as recorded.
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

    /// Forgets the receipts of the job's files that an earlier run left
    /// being received (`intended`): their files are received anew. Returns their IDs, which name their
    /// attempts' folders ([`crate::area::ReceiveArea::partial_dir`]).
    pub async fn forget_unfinished(&self, job_id: &str) -> Result<Vec<String>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let mut stmt = c.prepare_cached(
                    "DELETE FROM subtitle_job_files
                     WHERE job_id = ?1 AND state = 'intended'
                     RETURNING id",
                )?;
                let rows = stmt
                    .query_map([id], |r| r.get(0))?
                    .collect::<Result<Vec<String>, _>>()?;
                Ok(rows)
            })
            .await
    }

    /// The paths in `job_id`'s folder of the receive area that a receipt
    /// names, whatever its job and state.
    pub async fn named_paths(&self, job_id: &str) -> Result<HashSet<String>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT path FROM subtitle_job_files
                     WHERE path IS NOT NULL
                       AND (job_id = ?1 OR substr(path, 1, length(?1) + 1) = ?1 || '/')",
                )?;
                let rows = stmt
                    .query_map([id], |r| r.get(0))?
                    .collect::<Result<HashSet<String>, _>>()?;
                Ok(rows)
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
                          path, format, http_status, content_type, snapshot, created_at,
                          updated_at, folder, cleared_at, unchanged_asset)
                     SELECT ?1, job_id, ?2, file_key, name, 'done', id, size, sha256, path,
                            format, http_status, content_type, snapshot, ?3, ?3, folder,
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
