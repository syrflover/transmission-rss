//! The job records: what the web asks for, what the runner writes as it goes,
//! and what the screens read.

use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use trss_core::{Db, DbError, Millis};
use trss_subtitles::{verify::Format, FailureKind};

use crate::model::{FileState, ItemState, JobState, StepKind, StepState, Wait};

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("no record of {0}")]
    Missing(&'static str),
}

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

/// A job to make: the candidates a person picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    /// The ID the browser made for the action.
    pub command_id: String,
    /// The request's content in canonical JSON, to tell a repeat from another
    /// request with the same ID.
    pub request: String,
    /// How it was asked for (`pick`).
    pub origin: String,
    pub work_id: Option<String>,
    pub season: Option<i64>,
    pub anime_no: Option<i64>,
    pub source_id: Option<String>,
    pub creator: Option<String>,
    /// In the order to receive them.
    pub items: Vec<NewItem>,
}

/// One candidate of a job, as it was picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewItem {
    pub observation_id: Option<i64>,
    pub episode: String,
    pub post_url: String,
    pub found_at: Millis,
}

/// What [`JobStore::create`] did, with the job's ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Created {
    Created(String),
    /// The browser's ID had made this job with the same request before.
    Existing(String),
    /// The browser's ID made another request before; nothing was stored.
    Mismatch(String),
}

/// A job as the lists show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRow {
    pub seq: i64,
    pub id: String,
    pub state: JobState,
    pub wait: Option<Wait>,
    pub stage: Option<StepKind>,
    pub note: Option<String>,
    pub state_at: Millis,
    pub created_at: Millis,
    pub finished_at: Option<Millis>,
    pub work_id: Option<String>,
    /// The work's folder name, while the library has the work.
    pub work_name: Option<String>,
    pub season: Option<i64>,
    pub anime_no: Option<i64>,
    /// The Anissia title of the anime, while its snapshot is kept.
    pub anime_title: Option<String>,
    pub creator: Option<String>,
    /// The items' episodes, in order.
    pub episodes: Vec<String>,
    /// The first item's post host.
    pub source: Option<String>,
    pub progress: Progress,
    /// The class of the first failed item's failure, when it has one.
    pub failure: Option<FailureKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    pub done: usize,
    pub failed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRow {
    pub step: StepKind,
    pub state: StepState,
    pub at: Millis,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    pub id: i64,
    pub position: i64,
    pub observation_id: Option<i64>,
    pub episode: String,
    pub post_url: String,
    pub state: ItemState,
    pub wait: Option<Wait>,
    pub reason: Option<String>,
    /// The class of a failed item's failure (`docs/specs/jobs.md`, 공통 수신
    /// 결과와 실패 분류), when it has one.
    pub failure: Option<FailureKind>,
    pub files: Vec<FileRow>,
}

/// A candidate a job took: an item that names its observation, with how the
/// item and its job stand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub observation_id: i64,
    pub source_id: Option<String>,
    pub episode: String,
    pub post_url: String,
    pub item_state: ItemState,
    pub item_wait: Option<Wait>,
    pub job_id: String,
    pub job_state: JobState,
    pub updated_at: Millis,
}

/// One receipt of a file (see the schema's comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub id: String,
    pub item_id: i64,
    pub file_key: String,
    pub name: String,
    pub state: FileState,
    pub same_as: Option<String>,
    pub temp_dir: Option<String>,
    pub expected_size: Option<u64>,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    pub object: Option<String>,
    pub path: Option<String>,
    pub reason: Option<String>,
    pub created_at: Millis,
    /// What the received bytes are, found when they were checked.
    pub format: Option<Format>,
    /// The class of a failed (or retried) attempt.
    pub failure: Option<FailureKind>,
    /// The answer's status and media type, and for a failure the size of the
    /// answer that showed it.
    pub http_status: Option<u16>,
    pub content_type: Option<String>,
    pub response_size: Option<u64>,
    /// What the source read about the file, as a JSON array of `[name,
    /// value]` pairs ([`snapshot_json`]).
    pub snapshot: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub at: Millis,
    pub message: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobDetail {
    pub row: JobRow,
    pub steps: Vec<StepRow>,
    pub items: Vec<ItemRow>,
    /// Newest first.
    pub events: Vec<EventRow>,
}

/// A page of done jobs, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DonePage {
    pub items: Vec<JobRow>,
    /// The cursor of the next page; `None` at the end.
    pub next: Option<String>,
    pub total: usize,
}

/// Async access to the job records. Cheap to clone.
#[derive(Clone)]
pub struct JobStore {
    db: Db,
}

impl JobStore {
    pub fn new(db: Db) -> JobStore {
        JobStore { db }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Stores `job` as `pending` with its `found` step done at `now`, unless
    /// its browser ID is known. The check and the insert are one write
    /// transaction, so two deliveries at once store one job.
    pub async fn create(&self, job: NewJob, now: Millis) -> Result<Created, JobError> {
        self.db.run(move |c| create(c, &job, now)).await
    }

    /// The jobs that are not done, oldest first.
    pub async fn open_jobs(&self) -> Result<Vec<JobRow>, JobError> {
        self.db
            .run(|c| rows(c, "WHERE j.state <> 'done' ORDER BY j.seq", []))
            .await
    }

    /// The jobs waiting for a person's check, oldest first.
    pub async fn auth_waits(&self) -> Result<Vec<JobRow>, JobError> {
        self.db
            .run(|c| {
                rows(
                    c,
                    "WHERE j.state = 'waiting' AND j.wait = 'auth' ORDER BY j.seq",
                    [],
                )
            })
            .await
    }

    /// Up to `limit` done jobs after `after` (a page's `next`), newest first.
    /// A cursor that cannot be read starts from the newest.
    pub async fn done_page(
        &self,
        after: Option<String>,
        limit: usize,
    ) -> Result<DonePage, JobError> {
        self.db
            .run(move |c| done_page(c, after.as_deref(), limit))
            .await
    }

    pub async fn detail(&self, id: &str) -> Result<Option<JobDetail>, JobError> {
        let id = id.to_owned();
        self.db.run(move |c| detail(c, &id)).await
    }

    /// The candidates the jobs of Anissia anime `anime_no` took, in the order
    /// they were taken.
    pub async fn picks_of_anime(&self, anime_no: i64) -> Result<Vec<Pick>, JobError> {
        self.db
            .run(move |c| {
                let mut stmt = c.prepare(
                    "SELECT i.observation_id, j.source_id, i.episode, i.post_url, i.state,
                            i.wait, j.id, j.state, i.updated_at
                       FROM subtitle_job_items i
                       JOIN subtitle_jobs j ON j.id = i.job_id
                      WHERE j.anime_no = ?1 AND i.observation_id IS NOT NULL
                      ORDER BY i.id",
                )?;
                let rows = stmt
                    .query_map([anime_no], |r| {
                        Ok(Pick {
                            observation_id: r.get(0)?,
                            source_id: r.get(1)?,
                            episode: r.get(2)?,
                            post_url: r.get(3)?,
                            item_state: r.get(4)?,
                            item_wait: r.get(5)?,
                            job_id: r.get(6)?,
                            job_state: r.get(7)?,
                            updated_at: r.get(8)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }

    // -----------------------------------------------------------------------
    // What the runner writes

    /// Whether a job is ready to run: `pending`, or `running` from a start
    /// that did not finish.
    pub async fn has_ready(&self) -> Result<bool, JobError> {
        self.db
            .run(|c| {
                Ok(c.query_row(
                    "SELECT EXISTS (SELECT 1 FROM subtitle_jobs
                                    WHERE state IN ('pending', 'running'))",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
    }

    /// Takes the oldest ready job, marks it `running` and counts the start.
    /// Returns its ID, whether an earlier start had left it `running`, and how
    /// many times it has been started since its last run ended.
    pub async fn claim_next(&self, now: Millis) -> Result<Option<(String, bool, i64)>, JobError> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let found: Option<(String, JobState, i64)> = tx
                    .query_row(
                        "SELECT id, state, attempts FROM subtitle_jobs
                         WHERE state IN ('pending', 'running') ORDER BY seq LIMIT 1",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let Some((id, state, attempts)) = found else {
                    return Ok(None);
                };
                tx.execute(
                    "UPDATE subtitle_jobs
                     SET state = 'running', wait = NULL, attempts = attempts + 1,
                         state_at = CASE state WHEN 'running' THEN state_at ELSE ?2 END,
                         updated_at = ?2
                     WHERE id = ?1",
                    params![id, now],
                )?;
                tx.commit()?;
                Ok(Some((id, state == JobState::Running, attempts + 1)))
            })
            .await
    }

    /// Puts the items and jobs that wait for a source back in line, so a
    /// build that knows more sources tries them again.
    pub async fn requeue_waiting_for_sources(&self, now: Millis) -> Result<usize, JobError> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.execute(
                    "UPDATE subtitle_job_items
                     SET state = 'pending', wait = NULL, reason = NULL, updated_at = ?1
                     WHERE state = 'waiting' AND wait = 'subtitle'
                       AND job_id IN (SELECT id FROM subtitle_jobs
                                      WHERE state = 'waiting' AND wait = 'subtitle')",
                    [now],
                )?;
                let jobs = tx.execute(
                    "UPDATE subtitle_jobs
                     SET state = 'pending', wait = NULL, note = NULL, state_at = ?1,
                         updated_at = ?1
                     WHERE state = 'waiting' AND wait = 'subtitle'",
                    [now],
                )?;
                tx.commit()?;
                Ok(jobs)
            })
            .await
    }

    pub async fn items(&self, job_id: &str) -> Result<Vec<ItemRow>, JobError> {
        let id = job_id.to_owned();
        self.db.run(move |c| items(c, &id)).await
    }

    pub async fn set_item(
        &self,
        item_id: i64,
        state: ItemState,
        wait: Option<Wait>,
        reason: Option<String>,
        now: Millis,
    ) -> Result<(), JobError> {
        self.db
            .run(move |c| {
                c.execute(
                    "UPDATE subtitle_job_items
                     SET state = ?2, wait = ?3, reason = ?4, failure = NULL, updated_at = ?5
                     WHERE id = ?1",
                    params![item_id, state, wait, reason, now],
                )?;
                Ok(())
            })
            .await
    }

    /// Ends an item `failed` for `reason`, of class `failure` when the source
    /// gave one.
    pub async fn fail_item(
        &self,
        item_id: i64,
        reason: String,
        failure: Option<FailureKind>,
        now: Millis,
    ) -> Result<(), JobError> {
        self.db
            .run(move |c| {
                c.execute(
                    "UPDATE subtitle_job_items
                     SET state = 'failed', wait = NULL, reason = ?2, failure = ?3, updated_at = ?4
                     WHERE id = ?1",
                    params![item_id, reason, failure.map(FailureKind::code), now],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn set_stage(
        &self,
        job_id: &str,
        stage: StepKind,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                c.execute(
                    "UPDATE subtitle_jobs SET stage = ?2, updated_at = ?3 WHERE id = ?1",
                    params![id, stage, now],
                )?;
                Ok(())
            })
            .await
    }

    /// Writes `step` as `state` since `now`, or keeps its time when it is in
    /// `state` already.
    pub async fn set_step(
        &self,
        job_id: &str,
        step: StepKind,
        state: StepState,
        note: Option<String>,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                c.execute(
                    "INSERT INTO subtitle_job_steps (job_id, step, state, at, note)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (job_id, step) DO UPDATE
                     SET at = CASE WHEN state = excluded.state THEN at ELSE excluded.at END,
                         state = excluded.state, note = excluded.note",
                    params![id, step, state, now, note],
                )?;
                Ok(())
            })
            .await
    }

    /// Writes `step` as `current` since `now` unless the job reached it before.
    pub async fn begin_step(
        &self,
        job_id: &str,
        step: StepKind,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                c.execute(
                    "INSERT OR IGNORE INTO subtitle_job_steps (job_id, step, state, at)
                     VALUES (?1, ?2, 'current', ?3)",
                    params![id, step, now],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn steps(&self, job_id: &str) -> Result<Vec<StepRow>, JobError> {
        let id = job_id.to_owned();
        self.db.run(move |c| steps(c, &id)).await
    }

    pub async fn event(
        &self,
        job_id: &str,
        message: String,
        detail: Option<String>,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                c.execute(
                    "INSERT INTO subtitle_job_events (job_id, at, message, detail)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![id, now, message, detail],
                )?;
                Ok(())
            })
            .await
    }

    /// Ends a run of the job in `state` (with `wait` and `note`) at `now`.
    pub async fn settle(
        &self,
        job_id: &str,
        state: JobState,
        wait: Option<Wait>,
        note: Option<String>,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let finished = state.is_finished().then_some(now);
                c.execute(
                    "UPDATE subtitle_jobs
                     SET state = ?2, wait = ?3, note = ?4, stage = NULL, state_at = ?5,
                         updated_at = ?5, finished_at = ?6, attempts = 0
                     WHERE id = ?1",
                    params![id, state, wait, note, now, finished],
                )?;
                Ok(())
            })
            .await
    }

    /// Holds a job whose runs keep being cut short: its running items and
    /// its current steps stop with `note`, so nothing looks under way.
    pub async fn hold_stuck(
        &self,
        job_id: &str,
        note: String,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.execute(
                    "UPDATE subtitle_job_items
                     SET state = 'held', wait = NULL, reason = ?2, updated_at = ?3
                     WHERE job_id = ?1 AND state = 'running'",
                    params![id, note, now],
                )?;
                tx.execute(
                    "UPDATE subtitle_job_steps SET state = 'waiting', at = ?3, note = ?2
                     WHERE job_id = ?1 AND state = 'current'",
                    params![id, note, now],
                )?;
                tx.execute(
                    "UPDATE subtitle_jobs
                     SET state = 'held', wait = NULL, note = ?2, stage = NULL, state_at = ?3,
                         updated_at = ?3, attempts = 0
                     WHERE id = ?1",
                    params![id, note, now],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    // -----------------------------------------------------------------------
    // File receipts

    /// The receipts of `file_key` in the job, oldest first.
    pub async fn files_for_key(
        &self,
        job_id: &str,
        file_key: &str,
    ) -> Result<Vec<FileRow>, JobError> {
        let (id, key) = (job_id.to_owned(), file_key.to_owned());
        self.db
            .run(move |c| {
                let mut stmt = c.prepare(&format!(
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
                let mut stmt = c.prepare(
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

    /// Records the intent to receive a file before anything is fetched.
    pub async fn file_intend(&self, file: FileRow) -> Result<(), JobError> {
        self.db
            .run(move |c| {
                durable(c, |c| {
                    c.execute(
                        "INSERT INTO subtitle_job_files
                         (id, job_id, item_id, file_key, name, state, temp_dir, snapshot,
                          created_at, updated_at)
                     SELECT ?1, job_id, ?2, ?3, ?4, 'intended', ?5, ?7, ?6, ?6
                     FROM subtitle_job_items WHERE id = ?2",
                        params![
                            file.id,
                            file.item_id,
                            file.file_key,
                            file.name,
                            file.temp_dir,
                            file.created_at,
                            file.snapshot
                        ],
                    )
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
                durable(c, |c| c.execute(
                    "UPDATE subtitle_job_files SET expected_size = ?2, updated_at = ?3 WHERE id = ?1",
                    params![id, size.and_then(stored_size), now],
                )).map(|_| ())
            })
            .await
    }

    /// The answer the bytes come in: the length it announced, its status and
    /// media type, and what it added to the snapshot (`None` keeps it).
    pub async fn file_answer(
        &self,
        id: &str,
        size: Option<u64>,
        status: Option<u16>,
        content_type: Option<String>,
        snapshot: Option<String>,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = id.to_owned();
        self.db
            .run(move |c| {
                durable(c, |c| {
                    c.execute(
                        "UPDATE subtitle_job_files
                         SET expected_size = ?2, http_status = ?3, content_type = ?4,
                             snapshot = coalesce(?5, snapshot), updated_at = ?6
                         WHERE id = ?1",
                        params![
                            id,
                            size.and_then(stored_size),
                            kept_status(status),
                            content_type,
                            snapshot,
                            now
                        ],
                    )
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
                    c.execute(
                        "UPDATE subtitle_job_files
                     SET state = 'fetched', size = ?2, sha256 = ?3, object = ?4, path = ?5,
                         updated_at = ?6
                     WHERE id = ?1",
                        params![id, stored_size(size), sha256, object, path, now],
                    )
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
                    c.execute(
                        "UPDATE subtitle_job_files SET state = ?2, reason = ?3, updated_at = ?4
                     WHERE id = ?1",
                        params![id, state, reason, now],
                    )
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
                    c.execute(
                        "UPDATE subtitle_job_files
                         SET state = 'done', reason = NULL, format = ?2, updated_at = ?3
                         WHERE id = ?1",
                        params![id, format.code(), now],
                    )
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
                    c.execute(
                        "UPDATE subtitle_job_files
                         SET state = ?2, reason = ?3, failure = ?4,
                             http_status = coalesce(?5, http_status),
                             content_type = coalesce(?6, content_type),
                             response_size = ?7, updated_at = ?8
                         WHERE id = ?1",
                        params![
                            id,
                            state,
                            problem.reason,
                            problem.class.map(FailureKind::code),
                            kept_status(problem.status),
                            problem.content_type,
                            problem.size.and_then(stored_size),
                            now
                        ],
                    )
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
                    c.execute(
                        "UPDATE subtitle_job_files SET path = NULL, updated_at = ?2 WHERE id = ?1",
                        params![id, now],
                    )
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
                    c.execute(
                        "INSERT INTO subtitle_job_files
                         (id, job_id, item_id, file_key, name, state, same_as, size, sha256,
                          object, path, format, http_status, content_type, snapshot,
                          created_at, updated_at)
                     SELECT ?1, job_id, ?2, file_key, name, 'done', id, size, sha256, object,
                            path, format, http_status, content_type, snapshot, ?3, ?3
                     FROM subtitle_job_files WHERE id = ?4",
                        params![id, item_id, now, original.id],
                    )
                })
                .map(|_| ())
            })
            .await
    }
}

fn create(c: &mut Connection, job: &NewJob, now: Millis) -> Result<Created, JobError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let known: Option<(String, String)> = tx
        .query_row(
            "SELECT id, request FROM subtitle_jobs WHERE command_id = ?1",
            [&job.command_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((id, request)) = known {
        return Ok(match request == job.request {
            true => Created::Existing(id),
            false => Created::Mismatch(id),
        });
    }
    let id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO subtitle_jobs
             (id, command_id, request, origin, work_id, season, anime_no, source_id, creator,
              state, created_at, updated_at, state_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', ?10, ?10, ?10)",
        params![
            id,
            job.command_id,
            job.request,
            job.origin,
            job.work_id,
            job.season,
            job.anime_no,
            job.source_id,
            job.creator,
            now
        ],
    )?;
    for (position, item) in job.items.iter().enumerate() {
        tx.execute(
            "INSERT INTO subtitle_job_items
                 (job_id, position, observation_id, episode, post_url, found_at, state,
                  updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
            params![
                id,
                position as i64,
                item.observation_id,
                item.episode,
                item.post_url,
                item.found_at,
                now
            ],
        )?;
    }
    let count = job.items.len();
    tx.execute(
        "INSERT INTO subtitle_job_steps (job_id, step, state, at, note)
         VALUES (?1, 'found', 'done', ?2, ?3)",
        params![id, now, format!("후보 {count}개")],
    )?;
    tx.execute(
        "INSERT INTO subtitle_job_events (job_id, at, message, detail)
         VALUES (?1, ?2, '작업을 만들었어요', ?3)",
        params![id, now, format!("후보 {count}개")],
    )?;
    tx.commit()?;
    Ok(Created::Created(id))
}

const JOB_COLUMNS: &str = "
    SELECT j.seq, j.id, j.state, j.wait, j.stage, j.note, j.state_at, j.created_at,
           j.finished_at, j.work_id, w.dir_name, j.season, j.anime_no, a.subject, j.creator
    FROM subtitle_jobs j
    LEFT JOIN works w ON w.id = j.work_id
    LEFT JOIN anissia_anime a ON a.anime_no = j.anime_no";

fn job_row(r: &Row<'_>) -> rusqlite::Result<JobRow> {
    Ok(JobRow {
        seq: r.get(0)?,
        id: r.get(1)?,
        state: r.get(2)?,
        wait: r.get(3)?,
        stage: r.get(4)?,
        note: r.get(5)?,
        state_at: r.get(6)?,
        created_at: r.get(7)?,
        finished_at: r.get(8)?,
        work_id: r.get(9)?,
        work_name: r.get(10)?,
        season: r.get(11)?,
        anime_no: r.get(12)?,
        anime_title: r.get(13)?,
        creator: r.get(14)?,
        episodes: Vec::new(),
        source: None,
        progress: Progress::default(),
        failure: None,
    })
}

/// The jobs `tail` picks, with their items' episodes, source and progress.
fn rows<P: rusqlite::Params>(c: &Connection, tail: &str, p: P) -> Result<Vec<JobRow>, JobError> {
    let mut stmt = c.prepare(&format!("{JOB_COLUMNS} {tail}"))?;
    let mut jobs = stmt.query_map(p, job_row)?.collect::<Result<Vec<_>, _>>()?;
    let mut items = c.prepare(
        "SELECT episode, post_url, state, failure FROM subtitle_job_items
         WHERE job_id = ?1 ORDER BY position",
    )?;
    for job in &mut jobs {
        let mut first_failed_seen = false;
        let rows = items.query_map([&job.id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, ItemState>(2)?,
                failure_at(r, 3)?,
            ))
        })?;
        for row in rows {
            let (episode, post, state, failure) = row?;
            // The first failed item's class, as the note is its reason: a
            // later item's class would name another failure.
            if state == ItemState::Failed && !first_failed_seen {
                first_failed_seen = true;
                job.failure = failure;
            }
            if job.source.is_none() {
                job.source = url::Url::parse(&post)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned));
            }
            job.episodes.push(episode);
            job.progress.total += 1;
            match state {
                ItemState::Done => job.progress.done += 1,
                ItemState::Failed => job.progress.failed += 1,
                _ => {}
            }
        }
    }
    Ok(jobs)
}

fn done_page(c: &Connection, after: Option<&str>, limit: usize) -> Result<DonePage, JobError> {
    let cursor = after.and_then(|a| {
        let (at, seq) = a.split_once('.')?;
        Some((at.parse::<i64>().ok()?, seq.parse::<i64>().ok()?))
    });
    let (at, seq) = cursor.unwrap_or((i64::MAX, i64::MAX));
    let limit = limit.max(1);
    let mut items = rows(
        c,
        "WHERE j.state = 'done' AND (j.finished_at, j.seq) < (?1, ?2)
         ORDER BY j.finished_at DESC, j.seq DESC LIMIT ?3",
        params![at, seq, (limit + 1) as i64],
    )?;
    let next = match items.len() > limit {
        true => {
            items.truncate(limit);
            items
                .last()
                .map(|j| format!("{}.{}", j.finished_at.unwrap_or(0), j.seq))
        }
        false => None,
    };
    let total: i64 = c.query_row(
        "SELECT count(*) FROM subtitle_jobs WHERE state = 'done'",
        [],
        |r| r.get(0),
    )?;
    Ok(DonePage {
        items,
        next,
        total: total as usize,
    })
}

const FILE_COLUMNS: &str = "
    SELECT id, item_id, file_key, name, state, same_as, temp_dir, expected_size, size, sha256,
           object, path, reason, created_at, format, failure, http_status, content_type,
           response_size, snapshot
    FROM subtitle_job_files";

/// A failure class column.
fn failure_at(r: &Row<'_>, i: usize) -> rusqlite::Result<Option<FailureKind>> {
    let code: Option<String> = r.get(i)?;
    code.map(|code| {
        FailureKind::parse(&code).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                i,
                rusqlite::types::Type::Text,
                format!("unknown failure class {code:?}").into(),
            )
        })
    })
    .transpose()
}

fn format_at(r: &Row<'_>, i: usize) -> rusqlite::Result<Option<Format>> {
    let code: Option<String> = r.get(i)?;
    code.map(|code| {
        Format::parse(&code).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                i,
                rusqlite::types::Type::Text,
                format!("unknown format {code:?}").into(),
            )
        })
    })
    .transpose()
}

fn file_row(r: &Row<'_>) -> rusqlite::Result<FileRow> {
    Ok(FileRow {
        id: r.get(0)?,
        item_id: r.get(1)?,
        file_key: r.get(2)?,
        name: r.get(3)?,
        state: r.get(4)?,
        same_as: r.get(5)?,
        temp_dir: r.get(6)?,
        expected_size: r
            .get::<_, Option<i64>>(7)?
            .and_then(|s| u64::try_from(s).ok()),
        size: r
            .get::<_, Option<i64>>(8)?
            .and_then(|s| u64::try_from(s).ok()),
        sha256: r.get(9)?,
        object: r.get(10)?,
        path: r.get(11)?,
        reason: r.get(12)?,
        created_at: r.get(13)?,
        format: format_at(r, 14)?,
        failure: failure_at(r, 15)?,
        http_status: r.get(16)?,
        content_type: r.get(17)?,
        response_size: r
            .get::<_, Option<i64>>(18)?
            .and_then(|s| u64::try_from(s).ok()),
        snapshot: r.get(19)?,
    })
}

fn items(c: &Connection, job_id: &str) -> Result<Vec<ItemRow>, JobError> {
    let mut stmt = c.prepare(
        "SELECT id, position, observation_id, episode, post_url, state, wait, reason, failure
         FROM subtitle_job_items WHERE job_id = ?1 ORDER BY position",
    )?;
    let mut items = stmt
        .query_map([job_id], |r| {
            Ok(ItemRow {
                id: r.get(0)?,
                position: r.get(1)?,
                observation_id: r.get(2)?,
                episode: r.get(3)?,
                post_url: r.get(4)?,
                state: r.get(5)?,
                wait: r.get(6)?,
                reason: r.get(7)?,
                failure: failure_at(r, 8)?,
                files: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut files = c.prepare(&format!(
        "{FILE_COLUMNS} WHERE item_id = ?1 ORDER BY created_at, id"
    ))?;
    for item in &mut items {
        item.files = files
            .query_map([item.id], file_row)?
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(items)
}

fn steps(c: &Connection, job_id: &str) -> Result<Vec<StepRow>, JobError> {
    let mut stmt =
        c.prepare("SELECT step, state, at, note FROM subtitle_job_steps WHERE job_id = ?1")?;
    let rows = stmt
        .query_map([job_id], |r| {
            Ok(StepRow {
                step: r.get(0)?,
                state: r.get(1)?,
                at: r.get(2)?,
                note: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn detail(c: &mut Connection, id: &str) -> Result<Option<JobDetail>, JobError> {
    // One snapshot, so the steps, items and log agree with the row.
    let tx = c.transaction()?;
    let c = &*tx;
    let Some(row) = rows(c, "WHERE j.id = ?1", [id])?.pop() else {
        return Ok(None);
    };
    let mut stmt = c.prepare(
        "SELECT at, message, detail FROM subtitle_job_events
         WHERE job_id = ?1 ORDER BY id DESC",
    )?;
    let events = stmt
        .query_map([id], |r| {
            Ok(EventRow {
                at: r.get(0)?,
                message: r.get(1)?,
                detail: r.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let detail = JobDetail {
        steps: steps(c, id)?,
        items: items(c, id)?,
        events,
        row,
    };
    drop(stmt);
    tx.commit()?;
    Ok(Some(detail))
}
