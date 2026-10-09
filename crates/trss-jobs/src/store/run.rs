//! Claiming a job and the state it goes through: the job, its items and steps,
//! and the log.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use trss_core::Millis;
use trss_subtitles::FailureKind;

use super::{
    rows::{items, steps},
    ItemRow, JobError, JobStore, StepRow,
};
use crate::model::{ItemState, JobState, StepKind, StepState, Wait};

/// The note of a job a decision on its replacement put back in line
/// ([`crate::place::replace::records::decide`]), and the note and log line
/// of one decided on while it ran ([`JobStore::settle`]).
pub const DECIDED: &str = "결정한 교체를 이어가요";

impl JobStore {
    /// Whether a job is ready to run: `pending`, or `running` from a start
    /// that did not finish.
    pub async fn has_ready(&self) -> Result<bool, JobError> {
        self.db
            .run(|c| {
                Ok(c.prepare_cached(
                    "SELECT EXISTS (SELECT 1 FROM subtitle_jobs
                                    WHERE state IN ('pending', 'running'))",
                )?
                .query_row([], |r| r.get(0))?)
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
                    .prepare_cached(
                        "SELECT id, state, attempts FROM subtitle_jobs
                         WHERE state IN ('pending', 'running') ORDER BY seq LIMIT 1",
                    )?
                    .query_row([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .optional()?;
                let Some((id, state, attempts)) = found else {
                    return Ok(None);
                };
                tx.prepare_cached(
                    "UPDATE subtitle_jobs
                     SET state = 'running', wait = NULL, attempts = attempts + 1,
                         state_at = CASE state WHEN 'running' THEN state_at ELSE ?2 END,
                         updated_at = ?2
                     WHERE id = ?1",
                )?
                .execute(params![id, now])?;
                tx.commit()?;
                Ok(Some((id, state == JobState::Running, attempts + 1)))
            })
            .await
    }

    /// Puts the items and jobs that wait for a source back in line, so a
    /// build that knows more sources tries them again; so are the jobs whose
    /// work folder was not there (`영상 대기`). A worker calls it when it
    /// starts, which is also when an archive this machine failed to unpack
    /// is tried again without waiting for its hour
    /// ([`crate::place::unpack`]).
    pub async fn requeue_waiting_for_sources(&self, now: Millis) -> Result<usize, JobError> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.prepare_cached(
                    "UPDATE subtitle_job_files SET unpack_retry_at = NULL, updated_at = ?1
                     WHERE unpack_retry_at IS NOT NULL",
                )?
                .execute([now])?;
                tx.prepare_cached(
                    "UPDATE subtitle_job_items
                     SET state = 'pending', wait = NULL, reason = NULL, updated_at = ?1
                     WHERE state = 'waiting' AND wait = 'subtitle'
                       AND job_id IN (SELECT id FROM subtitle_jobs
                                      WHERE state = 'waiting' AND wait = 'subtitle')",
                )?
                .execute([now])?;
                let jobs = tx
                    .prepare_cached(
                        "UPDATE subtitle_jobs
                     SET state = 'pending', wait = NULL, note = NULL, state_at = ?1,
                         updated_at = ?1
                     WHERE state = 'waiting' AND wait IN ('subtitle', 'video')",
                    )?
                    .execute([now])?;
                tx.commit()?;
                Ok(jobs)
            })
            .await
    }

    /// Puts back in line the jobs that wait (`자막 대기`) for an archive's
    /// next try to unpack it once its time came ([`crate::place::unpack`]);
    /// how many. Every command poll asks, so a look that finds none takes
    /// no write lock.
    pub async fn requeue_unpack_retries(&self, now: Millis) -> Result<usize, JobError> {
        const DUE: &str = "SELECT f.job_id FROM subtitle_job_files f
                             JOIN subtitle_jobs j ON j.id = f.job_id
                            WHERE f.unpack_retry_at <= ?1
                              AND f.unpacked_at IS NULL AND f.unpack_error IS NULL
                              AND j.state = 'waiting' AND j.wait = 'subtitle'";
        self.db
            .run(move |c| {
                let due: bool = c
                    .prepare_cached(&format!("SELECT EXISTS ({DUE})"))?
                    .query_row([now], |r| r.get(0))?;
                if !due {
                    return Ok(0);
                }
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let jobs = tx
                    .prepare_cached(&format!(
                        "UPDATE subtitle_jobs
                            SET state = 'pending', wait = NULL, note = NULL, state_at = ?1,
                                updated_at = ?1
                          WHERE id IN ({DUE})"
                    ))?
                    .execute([now])?;
                tx.commit()?;
                Ok(jobs)
            })
            .await
    }

    /// Puts back in line the jobs a row of which waits for its episode's
    /// video (`영상 대기`) and now has one recorded, when the library's
    /// generation is not `seen`; the generation looked at, and how many jobs
    /// went back in line. The jobs are those that wait for the video or for a
    /// source, and those that ended partly failed: a job held or waiting for a
    /// person goes on when the person acts.
    pub async fn requeue_awaiting_video(
        &self,
        seen: Option<i64>,
        now: Millis,
    ) -> Result<(i64, usize), JobError> {
        fn generation(c: &Connection) -> rusqlite::Result<i64> {
            Ok(
                c.prepare_cached("SELECT generation FROM library_generation WHERE id = 1")?
                    .query_row([], |r| r.get(0))
                    .optional()?
                    .unwrap_or(0),
            )
        }
        self.db
            .run(move |c| {
                // Every command poll asks: a library that did not change takes
                // no write lock.
                if let Some(seen) = seen {
                    if generation(c)? == seen {
                        return Ok((seen, 0));
                    }
                }
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let generation = generation(&tx)?;
                let waiting: Vec<(String, String, u32, i64)> = {
                    let mut stmt = tx.prepare_cached(
                        "SELECT DISTINCT j.id, j.work_id, j.season, p.episode
                           FROM subtitle_jobs j JOIN subtitle_job_plan p ON p.job_id = j.id
                          WHERE (j.state = 'waiting' AND j.wait IN ('video', 'subtitle')
                                 OR j.state = 'partial')
                            AND j.work_id IS NOT NULL AND j.season IS NOT NULL
                            AND p.action = 'apply' AND p.outcome = 'no_video'
                            AND p.episode IS NOT NULL
                          ORDER BY j.seq, p.episode",
                    )?;
                    let rows =
                        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
                    rows.collect::<rusqlite::Result<_>>()?
                };
                let mut requeued: Vec<(String, i64)> = Vec::new();
                for (job, work, season, episode) in waiting {
                    if requeued.iter().any(|(j, _)| *j == job) {
                        continue;
                    }
                    let (videos, _) =
                        crate::place::records::episode_files(&tx, &work, season, episode)?;
                    if !videos.is_empty() {
                        requeued.push((job, episode));
                    }
                }
                for (job, episode) in &requeued {
                    tx.prepare_cached(
                        "UPDATE subtitle_jobs
                            SET state = 'pending', wait = NULL, note = NULL, finished_at = NULL,
                                state_at = ?2, updated_at = ?2
                          WHERE id = ?1",
                    )?
                    .execute(params![job, now])?;
                    tx.prepare_cached(
                        "INSERT INTO subtitle_job_events (job_id, at, message, detail)
                         VALUES (?1, ?2, '영상이 들어와 적용을 이어가요', ?3)",
                    )?
                    .execute(params![job, now, format!("{episode}화")])?;
                }
                tx.commit()?;
                Ok((generation, requeued.len()))
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
                c.prepare_cached(
                    "UPDATE subtitle_job_items
                     SET state = ?2, wait = ?3, reason = ?4, failure = NULL, updated_at = ?5
                     WHERE id = ?1",
                )?
                .execute(params![item_id, state, wait, reason, now])?;
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
                c.prepare_cached(
                    "UPDATE subtitle_job_items
                     SET state = 'failed', wait = NULL, reason = ?2, failure = ?3, updated_at = ?4
                     WHERE id = ?1",
                )?
                .execute(params![
                    item_id,
                    reason,
                    failure.map(FailureKind::code),
                    now
                ])?;
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
                c.prepare_cached(
                    "UPDATE subtitle_jobs SET stage = ?2, updated_at = ?3 WHERE id = ?1",
                )?
                .execute(params![id, stage, now])?;
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
                c.prepare_cached(
                    "INSERT INTO subtitle_job_steps (job_id, step, state, at, note)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (job_id, step) DO UPDATE
                     SET at = CASE WHEN state = excluded.state THEN at ELSE excluded.at END,
                         state = excluded.state, note = excluded.note",
                )?
                .execute(params![id, step, state, now, note])?;
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
                c.prepare_cached(
                    "INSERT OR IGNORE INTO subtitle_job_steps (job_id, step, state, at)
                     VALUES (?1, ?2, 'current', ?3)",
                )?
                .execute(params![id, step, now])?;
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
                c.prepare_cached(
                    "INSERT INTO subtitle_job_events (job_id, at, message, detail)
                     VALUES (?1, ?2, ?3, ?4)",
                )?
                .execute(params![id, now, message, detail])?;
                Ok(())
            })
            .await
    }

    /// Ends a run of the job in `state` (with `wait` and `note`) at `now`.
    ///
    /// A person may decide on a replacement while the run goes on, after it
    /// counted the plans: a job about to wait for an approval that has an
    /// approved plan, or no plan left to decide, goes back in line instead
    /// ([`DECIDED`]), in the same transaction as the decisions are
    /// read, so the next run carries them out. A job whose plan a mapping
    /// change rewrote during the run (`remapped_at`) goes back in line too
    /// ([`crate::place::relocate::REMAPPED`]), unless the run ends it held,
    /// failed or waiting for anything else than a person's placement, an
    /// approval or a video. Returns the note it went back in line with, if
    /// it did.
    pub async fn settle(
        &self,
        job_id: &str,
        state: JobState,
        wait: Option<Wait>,
        note: Option<String>,
        now: Millis,
    ) -> Result<Option<&'static str>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let decided = state == JobState::Waiting
                    && wait == Some(Wait::Approval)
                    && tx
                        .prepare_cached(
                            "SELECT EXISTS (SELECT 1 FROM subtitle_replacements
                                         WHERE job_id = ?1 AND state = 'approved')
                             OR NOT EXISTS (SELECT 1 FROM subtitle_replacements
                                             WHERE job_id = ?1 AND state = 'open')",
                        )?
                        .query_row([&id], |r| r.get::<_, bool>(0))?;
                let remapped = matches!(
                    (state, wait),
                    (JobState::Done | JobState::Partial, _)
                        | (
                            JobState::Waiting,
                            Some(Wait::Placement | Wait::Approval | Wait::Video)
                        )
                ) && tx
                    .prepare_cached(
                        "SELECT remapped_at IS NOT NULL FROM subtitle_jobs WHERE id = ?1",
                    )?
                    .query_row([&id], |r| r.get::<_, bool>(0))
                    .optional()?
                    .unwrap_or(false);
                // A mapping change that left no plan to decide says why.
                let again = match (decided, remapped) {
                    (_, true) => Some(crate::place::relocate::REMAPPED),
                    (true, false) => Some(DECIDED),
                    (false, false) => None,
                };
                let (state, wait, note) = match again {
                    Some(why) => (JobState::Pending, None, Some(why.to_owned())),
                    None => (state, wait, note),
                };
                let finished = state.is_finished().then_some(now);
                tx.prepare_cached(
                    "UPDATE subtitle_jobs
                     SET state = ?2, wait = ?3, note = ?4, stage = NULL, state_at = ?5,
                         updated_at = ?5, finished_at = ?6, attempts = 0, remapped_at = NULL
                     WHERE id = ?1",
                )?
                .execute(params![id, state, wait, note, now, finished])?;
                tx.commit()?;
                Ok(again)
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
                tx.prepare_cached(
                    "UPDATE subtitle_job_items
                     SET state = 'held', wait = NULL, reason = ?2, updated_at = ?3
                     WHERE job_id = ?1 AND state = 'running'",
                )?
                .execute(params![id, note, now])?;
                tx.prepare_cached(
                    "UPDATE subtitle_job_steps SET state = 'waiting', at = ?3, note = ?2
                     WHERE job_id = ?1 AND state = 'current'",
                )?
                .execute(params![id, note, now])?;
                // Its file effects under way are not known to have ended:
                // held with their rows and replacement plans, so their
                // targets are free again and the plans are not carried on.
                // A done plan's removals only wait for their clean-up: the
                // replacement stays done, and the clean-up runs when the job
                // runs again.
                let under_way = "SELECT * FROM subtitle_file_effects
                                  WHERE job_id = ?1
                                    AND state IN ('intended', 'prepared', 'set_aside')
                                    AND (plan_id IS NULL OR plan_id NOT IN
                                         (SELECT id FROM subtitle_replacements
                                           WHERE state = 'done'))";
                tx.prepare_cached(&format!(
                    "UPDATE subtitle_job_plan SET outcome = 'held', note = ?2, updated_at = ?3
                         WHERE job_id = ?1 AND position IN
                               (SELECT position FROM ({under_way}))"
                ))?
                .execute(params![id, note, now])?;
                tx.prepare_cached(&format!(
                    "UPDATE subtitle_replacements SET state = 'held', reason = ?2, updated_at = ?3
                         WHERE state = 'approved' AND id IN
                               (SELECT plan_id FROM ({under_way}) WHERE plan_id IS NOT NULL)"
                ))?
                .execute(params![id, note, now])?;
                tx.prepare_cached(&format!(
                    "UPDATE subtitle_file_effects SET state = 'held', reason = ?2, updated_at = ?3
                         WHERE id IN (SELECT id FROM ({under_way}))"
                ))?
                .execute(params![id, note, now])?;
                // So are a relocation's removals under way (the copy may be
                // aside); one not started keeps its copy, which a later
                // relocation may move.
                tx.prepare_cached(
                    "UPDATE subtitle_relocations
                        SET state = CASE state WHEN 'planned' THEN 'kept' ELSE 'held' END,
                            reason = ?2, updated_at = ?3
                      WHERE job_id = ?1 AND state IN ('planned', 'intended', 'set_aside')",
                )?
                .execute(params![id, note, now])?;
                tx.prepare_cached(
                    "UPDATE subtitle_jobs
                     SET state = 'held', wait = NULL, note = ?2, stage = NULL, state_at = ?3,
                         updated_at = ?3, attempts = 0
                     WHERE id = ?1",
                )?
                .execute(params![id, note, now])?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    /// Ends an item `done` and, in the same transaction, marks it when it is
    /// the item of a revision job (`revision_of`) whose files are the same
    /// bytes as the files of the earlier receipt of the episode: the latest
    /// done item of the revised observation before it. Both must have the same
    /// files (by key) with the same SHA-256 each. Returns that earlier
    /// receipt's job, or `None` when the item is not such an item or differs.
    pub async fn finish_item(&self, item_id: i64, now: Millis) -> Result<Option<String>, JobError> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.prepare_cached(
                    "UPDATE subtitle_job_items
                     SET state = 'done', wait = NULL, reason = NULL, failure = NULL,
                         updated_at = ?2
                     WHERE id = ?1",
                )?
                .execute(params![item_id, now])?;
                let unchanged = unchanged_from(&tx, item_id)?;
                if let Some(job) = &unchanged {
                    tx.prepare_cached(
                        "UPDATE subtitle_job_items SET unchanged_from = ?2 WHERE id = ?1",
                    )?
                    .execute(params![item_id, job])?;
                }
                tx.commit()?;
                Ok(unchanged)
            })
            .await
    }

    /// How the job was asked for (`pick`, [`AUTO`], [`UPLOAD`], [`FIND`]), or
    /// `None` when there is no such job.
    pub async fn origin(&self, job_id: &str) -> Result<Option<String>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                Ok(
                    c.prepare_cached("SELECT origin FROM subtitle_jobs WHERE id = ?1")?
                        .query_row([id], |r| r.get(0))
                        .optional()?,
                )
            })
            .await
    }

    /// Whether a person confirmed the job's placement (배치 확인).
    pub async fn placement_confirmed(&self, job_id: &str) -> Result<bool, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                Ok(c.prepare_cached(
                    "SELECT placement_confirmed_at IS NOT NULL FROM subtitle_jobs WHERE id = ?1",
                )?
                .query_row([id], |r| r.get::<_, bool>(0))
                .optional()?
                .unwrap_or(false))
            })
            .await
    }
}

/// The earlier receipt's job when the done item `item_id` of a revision job
/// has the same files, by key, with the same SHA-256 as that receipt
/// ([`JobStore::finish_item`]).
fn unchanged_from(tx: &Connection, item_id: i64) -> Result<Option<String>, JobError> {
    let revised: Option<i64> = tx
        .prepare_cached(
            "SELECT j.revision_of FROM subtitle_job_items i
               JOIN subtitle_jobs j ON j.id = i.job_id
              WHERE i.id = ?1 AND i.state = 'done'",
        )?
        .query_row([item_id], |r| r.get(0))
        .optional()?
        .flatten();
    let Some(revised) = revised else {
        return Ok(None);
    };
    let earlier: Option<(i64, String)> = tx
        .prepare_cached(
            "SELECT id, job_id FROM subtitle_job_items
              WHERE observation_id = ?1 AND state = 'done' AND id < ?2
              ORDER BY id DESC LIMIT 1",
        )?
        .query_row(params![revised, item_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let Some((earlier_item, earlier_job)) = earlier else {
        return Ok(None);
    };
    let hashes = |item: i64| -> Result<Vec<(String, Option<String>)>, JobError> {
        let mut stmt = tx.prepare_cached(
            "SELECT file_key, sha256 FROM subtitle_job_files
              WHERE item_id = ?1 AND state = 'done' ORDER BY file_key",
        )?;
        let rows = stmt
            .query_map([item], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    };
    let (now_files, was_files) = (hashes(item_id)?, hashes(earlier_item)?);
    let same = !now_files.is_empty()
        && now_files == was_files
        && now_files.iter().all(|(_, sha)| sha.is_some());
    Ok(same.then_some(earlier_job))
}
