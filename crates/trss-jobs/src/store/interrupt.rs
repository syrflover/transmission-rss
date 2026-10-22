//! A job the process stopped in the middle of a step (`docs/requirements/jobs.md`,
//! 체크포인트와 중단 복구; `docs/adr/0020-interrupted-jobs-wait-for-a-restart.md`):
//! it is `interrupted` (`중단됨`) when the process starts, stays where it was
//! cut, and goes on from that step when a person restarts it.

use rusqlite::{params, OptionalExtension, TransactionBehavior};
use trss_core::Millis;

use super::{JobError, JobRequests, JobRun};
use crate::model::{JobState, StepKind};

/// The log line of a job the start found `running`.
pub const INTERRUPTED: &str = "중단됐어요";

/// The log line of a job a person restarted.
pub const RESTARTED: &str = "다시 시작했어요";

/// What a person's restart of a job did ([`JobRequests::restart`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restarted {
    /// No such job.
    Missing,
    /// The job is not `interrupted`; only such a job is restarted.
    NotInterrupted(JobState),
    /// The job is `running` again, from the step it was cut in; the worker's
    /// jobs loop has to be rung.
    Restarted,
}

fn cut_in(stage: Option<StepKind>) -> String {
    match stage {
        Some(stage) => format!("{} 단계에서 멈췄어요", stage.label()),
        None => "단계를 시작하기 전에 멈췄어요".to_owned(),
    }
}

impl JobRun {
    /// Makes every job a process left `running` `interrupted`, at the start
    /// of the process that runs the jobs, before it serves the web or claims
    /// any job: one process runs jobs, so none of them is really running
    /// then, and a person's restart cannot come before it. A job keeps
    /// its stage, so the screen can say which step was cut, and its records;
    /// the log says in which step. A job that waits for a person, is held or
    /// is in line is left as it is. Returns how many jobs it made so.
    pub async fn interrupt_running(&self, now: Millis) -> Result<usize, JobError> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let running: Vec<(String, Option<StepKind>)> = {
                    let mut stmt = tx.prepare_cached(
                        "SELECT id, stage FROM subtitle_jobs WHERE state = 'running' ORDER BY seq",
                    )?;
                    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                    rows.collect::<rusqlite::Result<_>>()?
                };
                for (id, stage) in &running {
                    tx.prepare_cached(
                        "UPDATE subtitle_jobs
                         SET state = 'interrupted', state_at = ?2, updated_at = ?2
                         WHERE id = ?1",
                    )?
                    .execute(params![id, now])?;
                    tx.prepare_cached(
                        "INSERT INTO subtitle_job_events (job_id, at, message, detail)
                         VALUES (?1, ?2, ?3, ?4)",
                    )?
                    .execute(params![id, now, INTERRUPTED, cut_in(*stage)])?;
                }
                tx.commit()?;
                Ok(running.len())
            })
            .await
    }
}

impl JobRequests {
    /// A person's restart of the `interrupted` job `job_id` (`다시 시작`): it
    /// is `running` again with its stage, and the jobs' loop claims it as it
    /// claims a job left `running`, which carries it on from the step it was
    /// cut in ([`JobRun::claim_next`]). `attempts` stay as they are.
    pub async fn restart(&self, job_id: &str, now: Millis) -> Result<Restarted, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let found: Option<(JobState, Option<StepKind>)> = tx
                    .prepare_cached("SELECT state, stage FROM subtitle_jobs WHERE id = ?1")?
                    .query_row([&id], |r| Ok((r.get(0)?, r.get(1)?)))
                    .optional()?;
                let Some((state, stage)) = found else {
                    return Ok(Restarted::Missing);
                };
                if state != JobState::Interrupted {
                    return Ok(Restarted::NotInterrupted(state));
                }
                tx.prepare_cached(
                    "UPDATE subtitle_jobs
                     SET state = 'running', state_at = ?2, updated_at = ?2
                     WHERE id = ?1",
                )?
                .execute(params![id, now])?;
                tx.prepare_cached(
                    "INSERT INTO subtitle_job_events (job_id, at, message, detail)
                     VALUES (?1, ?2, ?3, ?4)",
                )?
                .execute(params![
                    id,
                    now,
                    RESTARTED,
                    stage.map(|s| format!("{} 단계부터", s.label()))
                ])?;
                tx.commit()?;
                Ok(Restarted::Restarted)
            })
            .await
    }
}
