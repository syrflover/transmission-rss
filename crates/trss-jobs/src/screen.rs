//! The remote screen of a job that waits for a person's check on the site
//! (`docs/specs/jobs.md`, 작업 화면 안의 인증과 브라우저 수명):
//! `subtitle_job_screens` (see `migrations/jobs/remote_screen.sql` in
//! `trss-core`).
//!
//! The worker and the web are separate processes over the same database.
//!
//! - The worker's runner binds the browser run that brought an item to the
//!   check ([`ScreenStore::bind`]), watches it for the file, and clears the
//!   binding when the run ends ([`ScreenStore::unbind`]) or the file arrives
//!   ([`ScreenStore::arrival`]).
//! - The web reads the binding to show the run's page ([`ScreenStore::screen`],
//!   a read that changes nothing), records that a person opened the job's page
//!   ([`ScreenStore::request_prepare`], then wakes the worker), and records a
//!   person's input on the screen ([`ScreenStore::record_input`]), which the
//!   worker's idle end of browser runs counts as use
//!   ([`ScreenStore::live_inputs`]).
//! - A find job's screen follows a page the post opened (a popup,
//!   [`ScreenStore::retarget`]). The web records a person's request to close
//!   such a page ([`ScreenStore::request_close`]); the worker closes it in the
//!   browser, never the run's first page, and the screen goes back to the
//!   page before it ([`ScreenStore::take_close`]).
//!
//! No token, cookie or address is written here.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use trss_core::{Db, Millis};

use crate::{
    model::{JobState, Wait},
    store::JobError,
};

/// What the job's screen says when its browser run ended before the check
/// was passed.
pub const RUN_ENDED: &str =
    "서버 브라우저가 쉬는 동안 닫혔어요. 작업 화면을 다시 열면 다시 준비해요";
/// The same, when the worker restarted.
pub const WORKER_RESTARTED: &str =
    "작업기가 다시 시작해 서버 브라우저가 닫혔어요. 작업 화면을 다시 열면 다시 준비해요";
/// The job's log when the check was passed and the browser downloaded the
/// file ([`ScreenStore::arrival`]).
pub const FILE_ARRIVED: &str = "사이트 확인을 마쳐 브라우저가 파일을 받았어요";
/// The same, when the answer was a web page instead of the file.
pub const FILE_REFUSED: &str = "사이트 확인을 마쳤지만 파일 대신 웹 페이지가 왔어요";
/// The job's log when a person reopened the screen of a run that had ended
/// ([`ScreenStore::requeue_for_check`]).
pub const CHECK_PREPARED_AGAIN: &str = "사이트 확인 화면을 다시 준비해요";
/// The same, for a find job.
pub const FIND_PREPARED_AGAIN: &str = "제작자의 게시물을 다시 열어요";

/// How far a job's remote screen is, as the web shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenState {
    /// The job waits for the check and a browser run shows it: the screen can
    /// connect to `run`.
    Ready,
    /// The worker is bringing the page to the check (again): asked to, or the
    /// job is back in line for it.
    Preparing,
    /// The job waits for the check but no run shows it (it ended, or the
    /// preparation failed: `note`). Opening the job's page prepares it again.
    Closed,
}

impl ScreenState {
    pub fn code(self) -> &'static str {
        match self {
            ScreenState::Ready => "ready",
            ScreenState::Preparing => "preparing",
            ScreenState::Closed => "closed",
        }
    }
}

/// A job's remote screen as the web reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub state: ScreenState,
    /// Whether the job waits for its check (`인증 필요`) now; a screen that is
    /// [`ScreenState::Preparing`] may be of a job back in line.
    pub waiting: bool,
    /// The bound run, when [`ScreenState::Ready`].
    pub run_id: Option<String>,
    /// The page of the run that shows the check, with `run_id`.
    pub target_id: Option<String>,
    /// When the run was bound, with `run_id`: tells a binding from an
    /// earlier one of the same run and page (another check of the job).
    pub bound_at: Option<Millis>,
    /// Why no run shows it, in a sentence.
    pub note: Option<String>,
    /// With `run_id`: the page shown is not the one the run was bound with
    /// (a find job's popup), so a person may close it
    /// ([`ScreenStore::request_close`]).
    pub popup: bool,
}

/// A binding of a run to a job that waits for its check: what the worker
/// watches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub job_id: String,
    pub item_id: i64,
    pub run_id: String,
    /// The page of the run the screen shows.
    pub target_id: String,
    /// The job is a find job: a person browses on the screen and every
    /// download of the run is the job's ([`crate::runner`]).
    pub find: bool,
}

/// A person opened the page of a job that waits for its check: the worker
/// answers it ([`ScreenStore::prepare_requests`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareRequest {
    pub job_id: String,
    /// The bound run, if any: the worker checks whether it is still live.
    pub run_id: Option<String>,
    /// When it was asked: what [`ScreenStore::mark_prepared`] and
    /// [`ScreenStore::requeue_for_check`] answer.
    pub asked_at: Millis,
    /// The job is a find job: its page has no check to bring back.
    pub find: bool,
}

/// A file the browser downloaded arrived for the bound run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arrival {
    /// The job is back in line to receive it (`받기`).
    Taken,
    /// The run is no longer the job's (or the job is gone): the arrival
    /// changes nothing. `item_open`: the item still has to receive its file
    /// (pending, running or waiting), so the file is kept for its next run.
    Stale { item_open: bool },
}

/// Async access to the remote screens. Cheap to clone.
#[derive(Clone)]
pub struct ScreenStore {
    db: Db,
}

/// `subtitle_jobs.state` and `.wait` of a waiting-for-check job, in SQL.
const WAITS_FOR_CHECK: &str = "j.state = 'waiting' AND j.wait = 'auth'";

impl ScreenStore {
    pub fn new(db: Db) -> ScreenStore {
        ScreenStore { db }
    }

    /// The job's screen, or `None` when it has none (no item of it was
    /// brought to a check in the server browser). Reads only.
    pub async fn screen(&self, job_id: &str) -> Result<Option<Screen>, JobError> {
        let id = job_id.to_owned();
        self.db.run(move |c| screen(c, &id)).await
    }

    /// A person opened the page of the job: when it waits for its check and
    /// has a screen, the request is written for the worker (the caller wakes
    /// it). Returns the screen as it is now, or `None` when there is none.
    pub async fn request_prepare(
        &self,
        job_id: &str,
        now: Millis,
    ) -> Result<Option<Screen>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.execute(
                    &format!(
                        "UPDATE subtitle_job_screens
                         SET prepare_at = MAX(?2, COALESCE(prepare_at, 0) + 1,
                                              COALESCE(prepared_at, 0) + 1),
                             updated_at = ?2
                         WHERE job_id = ?1
                           AND EXISTS (SELECT 1 FROM subtitle_jobs j
                                       WHERE j.id = ?1 AND {WAITS_FOR_CHECK})"
                    ),
                    params![id, now],
                )?;
                let screen = screen(&tx, &id)?;
                tx.commit()?;
                Ok(screen)
            })
            .await
    }

    /// A person's input reached the run `run_id` of the job: written when the
    /// run is still bound to it. Whether it was.
    pub async fn record_input(
        &self,
        job_id: &str,
        run_id: &str,
        now: Millis,
    ) -> Result<bool, JobError> {
        let (id, run) = (job_id.to_owned(), run_id.to_owned());
        self.db
            .run(move |c| {
                Ok(c.execute(
                    "UPDATE subtitle_job_screens
                     SET input_at = MAX(?3, COALESCE(input_at, 0))
                     WHERE job_id = ?1 AND run_id = ?2",
                    params![id, run, now],
                )? > 0)
            })
            .await
    }

    /// A person asked to close the page the find job's screen shows, of the
    /// binding (`run_id`, `bound_at`) they see: written for the worker
    /// ([`ScreenStore::take_close`]) when that is still the binding and its
    /// page is not the run's first ([`Screen::popup`]). Whether it was.
    pub async fn request_close(
        &self,
        job_id: &str,
        run_id: &str,
        bound_at: Millis,
        now: Millis,
    ) -> Result<bool, JobError> {
        let (id, run) = (job_id.to_owned(), run_id.to_owned());
        self.db
            .run(move |c| {
                Ok(c.execute(
                    &format!(
                        "UPDATE subtitle_job_screens
                         SET close_target_id = target_id, updated_at = ?4
                         WHERE job_id = ?1 AND run_id = ?2 AND bound_at = ?3
                           AND target_id <> first_target_id
                           AND EXISTS (SELECT 1 FROM subtitle_jobs j
                                       WHERE j.id = ?1 AND j.origin = 'find'
                                         AND {WAITS_FOR_CHECK})"
                    ),
                    params![id, run, bound_at, now],
                )? > 0)
            })
            .await
    }

    // -----------------------------------------------------------------------
    // What the worker writes

    /// The page of the run `run_id` a person asked to close
    /// ([`ScreenStore::request_close`]), if any: the request is answered by
    /// this (taken once).
    pub async fn take_close(&self, job_id: &str, run_id: &str) -> Result<Option<String>, JobError> {
        let (id, run) = (job_id.to_owned(), run_id.to_owned());
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let asked: Option<String> = tx
                    .query_row(
                        "SELECT close_target_id FROM subtitle_job_screens
                         WHERE job_id = ?1 AND run_id = ?2",
                        params![id, run],
                        |r| r.get(0),
                    )
                    .optional()?
                    .flatten();
                if asked.is_some() {
                    tx.execute(
                        "UPDATE subtitle_job_screens SET close_target_id = NULL
                         WHERE job_id = ?1 AND run_id = ?2",
                        params![id, run],
                    )?;
                }
                tx.commit()?;
                Ok(asked)
            })
            .await
    }

    /// The run `run_id` shows the check of `item_id`, at its page
    /// `target_id`: bound to the job, any earlier binding and note replaced.
    /// A request to prepare it is answered by this.
    pub async fn bind(
        &self,
        job_id: &str,
        item_id: i64,
        run_id: &str,
        target_id: &str,
        now: Millis,
    ) -> Result<(), JobError> {
        let (id, run, target) = (job_id.to_owned(), run_id.to_owned(), target_id.to_owned());
        self.db
            .run(move |c| {
                c.execute(
                    "INSERT INTO subtitle_job_screens
                         (job_id, item_id, run_id, target_id, bound_at, note, prepared_at,
                          input_at, updated_at, first_target_id, close_target_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?5, NULL, ?5, ?4, NULL)
                     ON CONFLICT (job_id) DO UPDATE
                     SET item_id = excluded.item_id, run_id = excluded.run_id,
                         target_id = excluded.target_id, bound_at = excluded.bound_at,
                         note = NULL, input_at = NULL,
                         prepared_at = MAX(excluded.prepared_at, COALESCE(prepare_at, 0)),
                         updated_at = excluded.updated_at,
                         first_target_id = excluded.target_id, close_target_id = NULL",
                    params![id, item_id, run, target, now],
                )?;
                Ok(())
            })
            .await
    }

    /// The check of `item_id` could not be brought to the screen (`note`):
    /// the job has a screen with no run, which a person's next opening of the
    /// page prepares again.
    pub async fn bind_failed(
        &self,
        job_id: &str,
        item_id: i64,
        note: String,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                c.execute(
                    "INSERT INTO subtitle_job_screens
                         (job_id, item_id, run_id, target_id, bound_at, note, prepared_at,
                          updated_at)
                     VALUES (?1, ?2, NULL, NULL, NULL, ?3, ?4, ?4)
                     ON CONFLICT (job_id) DO UPDATE
                     SET item_id = excluded.item_id, run_id = NULL, target_id = NULL,
                         bound_at = NULL, note = excluded.note, input_at = NULL,
                         prepared_at = MAX(excluded.prepared_at, COALESCE(prepare_at, 0)),
                         updated_at = excluded.updated_at,
                         first_target_id = NULL, close_target_id = NULL",
                    params![id, item_id, note, now],
                )?;
                Ok(())
            })
            .await
    }

    /// The screen of the run `run_id` shows its page `target_id` from now on
    /// (a find job's popup, or the page left when it closed): a new binding
    /// of the same run, so the screens open on the old page end and connect
    /// anew. The person's last input stays. Whether it changed.
    pub async fn retarget(
        &self,
        job_id: &str,
        run_id: &str,
        target_id: &str,
        now: Millis,
    ) -> Result<bool, JobError> {
        let (id, run, target) = (job_id.to_owned(), run_id.to_owned(), target_id.to_owned());
        self.db
            .run(move |c| {
                Ok(c.execute(
                    "UPDATE subtitle_job_screens
                     SET target_id = ?3, bound_at = MAX(?4, bound_at + 1), updated_at = ?4
                     WHERE job_id = ?1 AND run_id = ?2 AND target_id <> ?3",
                    params![id, run, target, now],
                )? > 0)
            })
            .await
    }

    /// The run `run_id` ended before its file came: the binding is cleared
    /// (with `note`) when it is still that run's, and the job keeps waiting
    /// for its check. Whether it was.
    pub async fn unbind(
        &self,
        job_id: &str,
        run_id: &str,
        note: &str,
        now: Millis,
    ) -> Result<bool, JobError> {
        let (id, run, note) = (job_id.to_owned(), run_id.to_owned(), note.to_owned());
        self.db
            .run(move |c| {
                Ok(c.execute(
                    "UPDATE subtitle_job_screens
                     SET run_id = NULL, target_id = NULL, bound_at = NULL, note = ?3,
                         input_at = NULL, updated_at = ?4,
                         first_target_id = NULL, close_target_id = NULL
                     WHERE job_id = ?1 AND run_id = ?2",
                    params![id, run, note, now],
                )? > 0)
            })
            .await
    }

    /// Every binding is cleared: the worker that held the runs restarted, so
    /// none of them is its any more. Returns how many there were.
    pub async fn unbind_all(&self, now: Millis) -> Result<usize, JobError> {
        self.db
            .run(move |c| {
                Ok(c.execute(
                    "UPDATE subtitle_job_screens
                     SET run_id = NULL, target_id = NULL, bound_at = NULL, note = ?2,
                         input_at = NULL, updated_at = ?1,
                         first_target_id = NULL, close_target_id = NULL
                     WHERE run_id IS NOT NULL",
                    params![now, WORKER_RESTARTED],
                )?)
            })
            .await
    }

    /// The job's screen is over: it no longer waits for its check.
    pub async fn clear(&self, job_id: &str) -> Result<(), JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                c.execute("DELETE FROM subtitle_job_screens WHERE job_id = ?1", [id])?;
                Ok(())
            })
            .await
    }

    /// The run bound to the job, whatever the job's state.
    pub async fn bound(&self, job_id: &str) -> Result<Option<Binding>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                Ok(c.query_row(
                    "SELECT s.job_id, s.item_id, s.run_id, s.target_id,
                            COALESCE(j.origin = 'find', 0)
                     FROM subtitle_job_screens s LEFT JOIN subtitle_jobs j ON j.id = s.job_id
                     WHERE s.job_id = ?1 AND s.run_id IS NOT NULL",
                    [id],
                    binding_row,
                )
                .optional()?)
            })
            .await
    }

    /// The runs bound to jobs that wait for their check: what the worker
    /// watches for the file.
    pub async fn bindings(&self) -> Result<Vec<Binding>, JobError> {
        self.db
            .run(|c| {
                let mut stmt = c.prepare(&format!(
                    "SELECT s.job_id, s.item_id, s.run_id, s.target_id, j.origin = 'find'
                     FROM subtitle_job_screens s JOIN subtitle_jobs j ON j.id = s.job_id
                     WHERE s.run_id IS NOT NULL AND {WAITS_FOR_CHECK}
                     ORDER BY s.job_id"
                ))?;
                let rows = stmt.query_map([], binding_row)?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
            .await
    }

    /// The unanswered requests to prepare a screen of jobs that wait for
    /// their check.
    pub async fn prepare_requests(&self) -> Result<Vec<PrepareRequest>, JobError> {
        self.db
            .run(|c| {
                let mut stmt = c.prepare(&format!(
                    "SELECT s.job_id, s.run_id, s.prepare_at, j.origin = 'find'
                     FROM subtitle_job_screens s JOIN subtitle_jobs j ON j.id = s.job_id
                     WHERE s.prepare_at > COALESCE(s.prepared_at, 0) AND {WAITS_FOR_CHECK}
                     ORDER BY s.prepare_at"
                ))?;
                let rows = stmt.query_map([], |r| {
                    Ok(PrepareRequest {
                        job_id: r.get(0)?,
                        run_id: r.get(1)?,
                        asked_at: r.get(2)?,
                        find: r.get(3)?,
                    })
                })?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
            .await
    }

    /// The request asked at `asked_at` is answered: the bound run is live.
    pub async fn mark_prepared(
        &self,
        job_id: &str,
        asked_at: Millis,
        now: Millis,
    ) -> Result<(), JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                c.execute(
                    "UPDATE subtitle_job_screens
                     SET prepared_at = MAX(?2, COALESCE(prepared_at, 0)), updated_at = ?3
                     WHERE job_id = ?1",
                    params![id, asked_at, now],
                )?;
                Ok(())
            })
            .await
    }

    /// The request asked at `asked_at` is answered by bringing the page to
    /// the check anew: the binding is cleared, and the job and its items that
    /// wait for a check go back in line, so the runner's next run of the job
    /// prepares it. Whether the job was put back (it still waited).
    pub async fn requeue_for_check(
        &self,
        job_id: &str,
        asked_at: Millis,
        now: Millis,
    ) -> Result<bool, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let back = requeue(&tx, &id, now)?;
                if back {
                    tx.execute(
                        "UPDATE subtitle_job_screens
                         SET run_id = NULL, target_id = NULL, bound_at = NULL, note = NULL,
                             input_at = NULL, prepared_at = MAX(?2, COALESCE(prepared_at, 0)),
                             updated_at = ?3, first_target_id = NULL, close_target_id = NULL
                         WHERE job_id = ?1",
                        params![id, asked_at, now],
                    )?;
                    tx.execute(
                        "INSERT INTO subtitle_job_events (job_id, at, message, detail)
                         SELECT ?1, ?2, CASE origin WHEN 'find' THEN ?3 ELSE ?4 END, NULL
                         FROM subtitle_jobs WHERE id = ?1",
                        params![id, now, FIND_PREPARED_AGAIN, CHECK_PREPARED_AGAIN],
                    )?;
                }
                tx.commit()?;
                Ok(back)
            })
            .await
    }

    /// The bound run `run_id` downloaded the file of the job's check, or was
    /// refused it: the check is passed (`인증` done), the screen is over, the
    /// job's log says `message` with `detail` (the file's name, or why), and
    /// the job goes back in line to receive the file (`받기`) or fail with
    /// the refusal. The runner finds either where the watch put it.
    /// `item_id`: the item the watch was for, which a stale arrival reports
    /// on ([`Arrival::Stale`]).
    pub async fn arrival(
        &self,
        job_id: &str,
        run_id: &str,
        item_id: i64,
        message: &'static str,
        detail: &str,
        now: Millis,
    ) -> Result<Arrival, JobError> {
        let (id, run, detail) = (job_id.to_owned(), run_id.to_owned(), detail.to_owned());
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let bound: Option<String> = tx
                    .query_row(
                        "SELECT run_id FROM subtitle_job_screens WHERE job_id = ?1",
                        [&id],
                        |r| r.get(0),
                    )
                    .optional()?
                    .flatten();
                if bound.as_deref() != Some(run.as_str()) || !requeue(&tx, &id, now)? {
                    let item_open = tx
                        .query_row(
                            "SELECT 1 FROM subtitle_job_items
                             WHERE id = ?1 AND job_id = ?2
                               AND state IN ('pending', 'running', 'waiting')",
                            params![item_id, id],
                            |_| Ok(()),
                        )
                        .optional()?
                        .is_some();
                    return Ok(Arrival::Stale { item_open });
                }
                tx.execute("DELETE FROM subtitle_job_screens WHERE job_id = ?1", [&id])?;
                tx.execute(
                    "UPDATE subtitle_job_steps SET state = 'done', at = ?2, note = NULL
                     WHERE job_id = ?1 AND step = 'auth'",
                    params![id, now],
                )?;
                tx.execute(
                    "INSERT INTO subtitle_job_events (job_id, at, message, detail)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![id, now, message, detail],
                )?;
                tx.commit()?;
                Ok(Arrival::Taken)
            })
            .await
    }

    /// The person's last input of every bound run, for the browser pool's
    /// idle end.
    pub async fn live_inputs(&self) -> Result<Vec<(String, Millis)>, JobError> {
        self.db
            .run(|c| {
                let mut stmt = c.prepare(
                    "SELECT run_id, input_at FROM subtitle_job_screens
                     WHERE run_id IS NOT NULL AND input_at IS NOT NULL",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
            .await
    }
}

fn binding_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Binding> {
    Ok(Binding {
        job_id: r.get(0)?,
        item_id: r.get(1)?,
        run_id: r.get(2)?,
        target_id: r.get(3)?,
        find: r.get(4)?,
    })
}

/// Puts a job that waits for its check, and its items that do, back in line.
/// Whether it waited.
fn requeue(c: &Connection, job_id: &str, now: Millis) -> Result<bool, JobError> {
    let state: Option<(JobState, Option<Wait>)> = c
        .query_row(
            "SELECT state, wait FROM subtitle_jobs WHERE id = ?1",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if state != Some((JobState::Waiting, Some(Wait::Auth))) {
        return Ok(false);
    }
    c.execute(
        "UPDATE subtitle_job_items
         SET state = 'pending', wait = NULL, reason = NULL, updated_at = ?2
         WHERE job_id = ?1 AND state = 'waiting' AND wait = 'auth'",
        params![job_id, now],
    )?;
    c.execute(
        "UPDATE subtitle_jobs
         SET state = 'pending', wait = NULL, note = NULL, state_at = ?2, updated_at = ?2
         WHERE id = ?1",
        params![job_id, now],
    )?;
    Ok(true)
}

/// A screen's row with its job's state: run, page, binding time, note,
/// whether a request is unanswered, the job's state and wait, and whether
/// the page shown is not the run's first.
type ScreenRow = (
    Option<String>,
    Option<String>,
    Option<Millis>,
    Option<String>,
    bool,
    JobState,
    Option<Wait>,
    bool,
);

fn screen(c: &Connection, job_id: &str) -> Result<Option<Screen>, JobError> {
    let row: Option<ScreenRow> = c
        .query_row(
            "SELECT s.run_id, s.target_id, s.bound_at, s.note,
                    COALESCE(s.prepare_at, 0) > COALESCE(s.prepared_at, 0), j.state, j.wait,
                    COALESCE(s.target_id <> s.first_target_id, 0)
             FROM subtitle_job_screens s JOIN subtitle_jobs j ON j.id = s.job_id
             WHERE s.job_id = ?1",
            [job_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )
        .optional()?;
    let Some((run_id, target_id, bound_at, note, asked, state, wait, popup)) = row else {
        return Ok(None);
    };
    let waits = state == JobState::Waiting && wait == Some(Wait::Auth);
    let back_in_line = matches!(state, JobState::Pending | JobState::Running);
    let screen = match run_id {
        Some(run_id) if waits => Screen {
            state: ScreenState::Ready,
            waiting: waits,
            run_id: Some(run_id),
            target_id,
            bound_at,
            note: None,
            popup,
        },
        None if waits && !asked => Screen {
            state: ScreenState::Closed,
            waiting: waits,
            run_id: None,
            target_id: None,
            bound_at: None,
            note,
            popup: false,
        },
        // Asked, or back in line to be brought to the check again.
        _ if waits || back_in_line => Screen {
            state: ScreenState::Preparing,
            waiting: waits,
            run_id: None,
            target_id: None,
            bound_at: None,
            note: None,
            popup: false,
        },
        // A screen of a job that ended, which the runner clears.
        _ => return Ok(None),
    };
    Ok(Some(screen))
}
