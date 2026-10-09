//! A find job: a person browses a creator's posts on its remote screen and every
//! download becomes a file of its one package, until the person finishes its
//! 받기.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use trss_core::Millis;
use trss_subtitles::upload::Archive;

use super::{
    create::{Created, UploadedFile},
    rows::{dropped_rows, found_note, items, upload_note, upload_summary},
    DroppedRow, FileRow, JobError, JobStore, FIND,
};
use crate::model::{FileState, JobState};

/// A find job to make: one item for its package, whose post is where the
/// browser starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFind {
    pub command_id: String,
    /// The request's content in canonical JSON.
    pub request: String,
    pub work_id: String,
    pub season: i64,
    pub anime_no: i64,
    pub source_id: String,
    pub creator: String,
    /// The creator's newest post the app observed for the anime.
    pub post_url: String,
}

/// What a find job has received so far: its item, the files it kept and the
/// names of those it dropped, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub item_id: i64,
    pub kept: Vec<FileRow>,
    pub dropped: Vec<DroppedRow>,
}

/// What a person's request to finish a find job did ([`JobStore::ask_finish`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskedFinish {
    /// No such job.
    Missing,
    /// The job is not a find job.
    NotFind,
    /// The job's 받기 had ended already.
    Ended,
    /// The request is written; the worker ends the job once no download of
    /// its run is on its way, or at its next look when no run is bound.
    Asked,
}

/// The note of a find job that ended with no file kept.
pub const NOTHING_FOUND: &str = "받은 파일 없음";

impl JobStore {
    // What the web asks for.

    /// Stores a find job as `pending`, unless its command ID is known. The
    /// check and the insert are one write transaction, so two deliveries at
    /// once store one job.
    pub async fn create_find(&self, find: NewFind, now: Millis) -> Result<Created, JobError> {
        self.db.run(move |c| create_find(c, &find, now)).await
    }

    /// A person asked the find job to finish receiving. The request is
    /// written and nothing else: the worker ends the job
    /// ([`JobStore::end_find`]), since only it sees a download a run left in
    /// the job's folder (a restart cut its watch short) and whether one is
    /// on its way. Until then the job is finishing ([`JobRow::finishing`]).
    pub async fn ask_finish(&self, job_id: &str, now: Millis) -> Result<AskedFinish, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let found: Option<(String, JobState)> = tx
                    .prepare_cached("SELECT origin, state FROM subtitle_jobs WHERE id = ?1")?
                    .query_row([&id], |r| Ok((r.get(0)?, r.get(1)?)))
                    .optional()?;
                let Some((origin, state)) = found else {
                    return Ok(AskedFinish::Missing);
                };
                if origin != FIND {
                    return Ok(AskedFinish::NotFind);
                }
                if state.is_finished() || received(&tx, &id)? {
                    return Ok(AskedFinish::Ended);
                }
                tx.prepare_cached(
                    "UPDATE subtitle_jobs SET finish_at = COALESCE(finish_at, ?2), updated_at = ?2
                     WHERE id = ?1",
                )?
                .execute(params![id, now])?;
                tx.commit()?;
                Ok(AskedFinish::Asked)
            })
            .await
    }

    // What the worker does.

    /// What the find job has received so far, or `None` when it has no item.
    pub async fn found(&self, job_id: &str) -> Result<Option<Found>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let Some(item) = items(c, &id)?.into_iter().next() else {
                    return Ok(None);
                };
                Ok(Some(Found {
                    item_id: item.id,
                    kept: item
                        .files
                        .into_iter()
                        .filter(|f| f.state == FileState::Done)
                        .collect(),
                    dropped: dropped_rows(c, &id)?,
                }))
            })
            .await
    }

    /// Records a file the find job's browser run downloaded and the job kept,
    /// already in the job's folder, as a receipt at `done` of its item, with
    /// a line in its log.
    pub async fn add_found(
        &self,
        job_id: &str,
        item_id: i64,
        file: UploadedFile,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.prepare_cached(
                    "INSERT INTO subtitle_job_files
                         (id, job_id, item_id, file_key, name, state, size, sha256, object,
                          path, created_at, updated_at, format, kind, archive_type)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'done', ?6, ?7, ?8, ?9, ?10, ?10, ?11, ?12, ?13)",
                )?
                .execute(params![
                    file.id,
                    id,
                    item_id,
                    file.file_key,
                    file.name,
                    i64::try_from(file.size).unwrap_or(i64::MAX),
                    file.sha256,
                    file.object,
                    file.path,
                    now,
                    file.format.code(),
                    file.kind.code(),
                    file.archive.map(Archive::code)
                ])?;
                tx.prepare_cached(
                    "INSERT INTO subtitle_job_events (job_id, at, message, detail)
                     VALUES (?1, ?2, '서버 브라우저가 받은 파일을 남겼어요', ?3)",
                )?
                .execute(params![
                    id,
                    now,
                    format!("{} · {}", file.name, file.kind.label())
                ])?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    /// Records a file the find job's browser run downloaded and the job did
    /// not keep, with why (its bytes are gone), with a line in its log.
    pub async fn add_dropped(
        &self,
        job_id: &str,
        dropped: crate::upload::Dropped,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.prepare_cached(
                    "INSERT INTO subtitle_job_dropped (job_id, position, name, reason)
                     VALUES (?1, (SELECT COALESCE(MAX(position), -1) + 1
                                  FROM subtitle_job_dropped WHERE job_id = ?1), ?2, ?3)",
                )?
                .execute(params![id, dropped.name, dropped.reason])?;
                tx.prepare_cached(
                    "INSERT INTO subtitle_job_events (job_id, at, message, detail)
                     VALUES (?1, ?2, '서버 브라우저가 받은 파일을 뺐어요', ?3)",
                )?
                .execute(params![
                    id,
                    now,
                    format!("{} · {}", dropped.name, dropped.reason)
                ])?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    /// Whether a person asked the find job to finish ([`JobStore::ask_finish`]).
    pub async fn finish_asked(&self, job_id: &str) -> Result<bool, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                Ok(c.prepare_cached(
                    "SELECT finish_at IS NOT NULL FROM subtitle_jobs WHERE id = ?1",
                )?
                .query_row([id], |r| r.get(0))
                .optional()?
                .unwrap_or(false))
            })
            .await
    }

    /// Ends the find job's 받기 when it has not ended and the run bound to
    /// it is `run` (`None`: no run is bound): its item is done, its step
    /// `receive` too, a step `open` left unfinished (its screen could not be
    /// prepared) is done when a run once opened the post and goes when none
    /// did, its screen goes, and its note and log say what it kept. A job
    /// that kept files is `pending` again for the worker's analysis and a
    /// person's 배치 확인; one that kept none ends `done` with 받은 파일 없음.
    /// The worker removes its folder of downloads. Whether it ended now.
    ///
    /// The end of a run's watch (`Some`) waits while the job is `running`:
    /// the job's run settles it first, and the next watch ends it.
    pub async fn end_find(
        &self,
        job_id: &str,
        run: Option<&str>,
        now: Millis,
    ) -> Result<bool, JobError> {
        let (id, run) = (job_id.to_owned(), run.map(str::to_owned));
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let ended = end_find(&tx, &id, run.as_deref(), now)?;
                tx.commit()?;
                Ok(ended)
            })
            .await
    }

    /// The note of the upload or find job `job_id` once what it kept is
    /// placed, as it was made or ended with ([`upload_note`],
    /// [`found_note`]); a wait in between leaves it another.
    pub async fn kept_note(&self, job_id: &str) -> Result<String, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let origin: Option<String> = c
                    .prepare_cached("SELECT origin FROM subtitle_jobs WHERE id = ?1")?
                    .query_row([&id], |r| r.get(0))
                    .optional()?;
                let summary = upload_summary(c, &id)?;
                Ok(match origin.as_deref() {
                    Some(FIND) => found_note(&summary),
                    _ => upload_note(&summary.counts()),
                })
            })
            .await
    }

    /// The find jobs a person asked to finish that wait (or are held) with no
    /// browser run bound to them: the worker ends them.
    pub async fn unbound_finishes(&self) -> Result<Vec<String>, JobError> {
        self.db
            .run(|c| {
                let mut stmt = c.prepare_cached(
                    "SELECT j.id FROM subtitle_jobs j
                     WHERE j.origin = 'find' AND j.finish_at IS NOT NULL
                       AND j.state IN ('waiting', 'held')
                       AND EXISTS (SELECT 1 FROM subtitle_job_items i
                                   WHERE i.job_id = j.id AND i.state <> 'done')
                       AND NOT EXISTS (SELECT 1 FROM subtitle_job_screens s
                                       WHERE s.job_id = j.id AND s.run_id IS NOT NULL)
                     ORDER BY j.seq",
                )?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
            .await
    }
}

fn create_find(c: &mut Connection, find: &NewFind, now: Millis) -> Result<Created, JobError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let known: Option<(String, String)> = tx
        .prepare_cached("SELECT id, request FROM subtitle_jobs WHERE command_id = ?1")?
        .query_row([&find.command_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    if let Some((id, request)) = known {
        return Ok(match request == find.request {
            true => Created::Existing(id),
            false => Created::Mismatch(id),
        });
    }
    let id = uuid::Uuid::new_v4().to_string();
    tx.prepare_cached(
        "INSERT INTO subtitle_jobs
             (id, command_id, request, origin, work_id, season, anime_no, source_id, creator,
              state, created_at, updated_at, state_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', ?10, ?10, ?10)",
    )?
    .execute(params![
        id,
        find.command_id,
        find.request,
        FIND,
        find.work_id,
        find.season,
        find.anime_no,
        find.source_id,
        find.creator,
        now
    ])?;
    // The item stands for the package: no episode, no observation (it is no
    // candidate the person picked, and nothing reads it as one).
    tx.prepare_cached(
        "INSERT INTO subtitle_job_items
             (job_id, position, observation_id, episode, post_url, found_at, state, updated_at)
         VALUES (?1, 0, NULL, '', ?2, ?3, 'pending', ?3)",
    )?
    .execute(params![id, find.post_url, now])?;
    let host = url::Url::parse(&find.post_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned));
    tx.prepare_cached(
        "INSERT INTO subtitle_job_events (job_id, at, message, detail)
         VALUES (?1, ?2, '직접 찾기 작업을 만들었어요', ?3)",
    )?
    .execute(params![
        id,
        now,
        match host {
            Some(host) => format!("{} · {host}", find.creator),
            None => find.creator.clone(),
        }
    ])?;
    tx.commit()?;
    Ok(Created::Created(id))
}

/// Ends the find job `id` in `tx` (see [`JobStore::end_find`]). Whether it
/// ended now.
fn end_find(tx: &Connection, id: &str, run: Option<&str>, now: Millis) -> Result<bool, JobError> {
    let open: Option<JobState> = tx
        .prepare_cached("SELECT state FROM subtitle_jobs WHERE id = ?1 AND origin = 'find'")?
        .query_row([id], |r| r.get(0))
        .optional()?;
    if open.is_none_or(JobState::is_finished) || received(tx, id)? {
        return Ok(false);
    }
    // A run of the job holds it while it is `running` and writes the job's
    // wait when it settles: a watch's end leaves the job to it, or that wait
    // would be written over the ended 받기. The worker watches only jobs that
    // wait for their check; this keeps the order without counting on it.
    if run.is_some() && open == Some(JobState::Running) {
        return Ok(false);
    }
    let bound: Option<String> = tx
        .prepare_cached("SELECT run_id FROM subtitle_job_screens WHERE job_id = ?1")?
        .query_row([id], |r| r.get(0))
        .optional()?
        .flatten();
    if bound.as_deref() != run {
        return Ok(false);
    }
    let summary = upload_summary(tx, id)?;
    let dropped = match summary.dropped {
        0 => String::new(),
        n => format!(" · 뺀 파일 {n}개"),
    };
    let kept = summary.subtitles + summary.fonts + summary.archives;
    // What it kept goes on to the worker's analysis and a person's
    // 배치 확인: the job is back in line.
    let (note, message, state, finished_at) = match kept {
        0 => (
            format!("{NOTHING_FOUND}{dropped}"),
            "받은 파일 없이 받기를 끝냈어요",
            JobState::Done,
            Some(now),
        ),
        _ => (
            found_note(&summary),
            "받기를 끝냈어요",
            JobState::Pending,
            None,
        ),
    };
    tx.prepare_cached(
        "UPDATE subtitle_job_items
         SET state = 'done', wait = NULL, reason = NULL, failure = NULL, updated_at = ?2
         WHERE job_id = ?1",
    )?
    .execute(params![id, now])?;
    // A step `open` the last run left waiting (its screen was not prepared)
    // says nothing of a job that ended: done when an earlier run opened the
    // post (the step `receive` began), gone when none did.
    tx.prepare_cached(
        "UPDATE subtitle_job_steps SET state = 'done', note = NULL
         WHERE job_id = ?1 AND step = 'open' AND state <> 'done'
           AND EXISTS (SELECT 1 FROM subtitle_job_steps
                       WHERE job_id = ?1 AND step = 'receive')",
    )?
    .execute([id])?;
    tx.prepare_cached(
        "DELETE FROM subtitle_job_steps WHERE job_id = ?1 AND step = 'open' AND state <> 'done'",
    )?
    .execute([id])?;
    tx.prepare_cached(
        "INSERT INTO subtitle_job_steps (job_id, step, state, at, note)
         VALUES (?1, 'receive', 'done', ?2, ?3)
         ON CONFLICT (job_id, step) DO UPDATE
         SET state = 'done', at = excluded.at, note = excluded.note",
    )?
    .execute(params![id, now, note])?;
    tx.prepare_cached(
        "UPDATE subtitle_jobs
         SET state = ?4, wait = NULL, note = ?2, stage = NULL, state_at = ?3,
             updated_at = ?3, finished_at = ?5, attempts = 0
         WHERE id = ?1",
    )?
    .execute(params![id, note, now, state, finished_at])?;
    tx.prepare_cached("DELETE FROM subtitle_job_screens WHERE job_id = ?1")?
        .execute([id])?;
    tx.prepare_cached(
        "INSERT INTO subtitle_job_events (job_id, at, message, detail)
         VALUES (?1, ?2, ?3, ?4)",
    )?
    .execute(params![id, now, message, note])?;
    Ok(true)
}

/// Whether the find job `id`'s 받기 ended: [`end_find`] made its item done,
/// and it went on to its placement.
fn received(tx: &Connection, id: &str) -> rusqlite::Result<bool> {
    tx.prepare_cached(
        "SELECT NOT EXISTS (SELECT 1 FROM subtitle_job_items
                            WHERE job_id = ?1 AND state <> 'done')",
    )?
    .query_row([id], |r| r.get(0))
}
