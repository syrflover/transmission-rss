//! The records of replacements (`migrations/jobs/replacement.sql` in
//! `trss-core`): the plans, their paths, and what a person decided. The
//! writes that come before or after a file effect are synced
//! ([`durable`]).

use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use trss_core::Millis;
// What the screens read of a comparison.
pub use trss_subtitles::compare::{Diff, Encoding, Format, Item, NotCompared, Side};

use crate::{
    model::{EffectState, PathAction, PlanState, SubtitleFormat},
    place::{
        episode::{Assignment, Basis},
        records::{self as place_records, durable, Effect, EFFECT_COLUMNS},
    },
    store::{JobError, DECIDED},
};

/// A file beside a video as a plan saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSeen {
    pub size: u64,
    pub sha256: String,
    pub object: String,
    /// Its change time, in nanoseconds since the epoch.
    pub mtime: i64,
    /// Its dialogue lines, when the app can count them.
    pub lines: Option<u64>,
}

/// What a plan does to one path (see the schema's comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanPath {
    pub path: String,
    pub action: PathAction,
    /// The file there, `None` for `add`.
    pub file: Option<FileSeen>,
    /// The app's applied copy it is.
    pub applied_id: Option<String>,
}

/// The video a plan is bound to: no hash of its bytes, only what identifies
/// the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoSeen {
    pub path: String,
    pub object: String,
    pub size: u64,
    /// Nanoseconds since the epoch.
    pub mtime: i64,
}

/// One replacement plan (see the schema's comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub id: String,
    pub job_id: String,
    pub position: i64,
    pub version: i64,
    pub state: PlanState,
    pub reason: Option<String>,
    pub work_id: String,
    pub season: u32,
    pub episode: i64,
    pub assignment: Assignment,
    pub basis: Option<Basis>,
    pub folder: String,
    pub video: VideoSeen,
    pub stored_id: String,
    pub asset_id: String,
    pub asset_path: String,
    pub asset_size: u64,
    pub asset_sha256: String,
    pub asset_lines: Option<u64>,
    pub target: String,
    pub created_at: Millis,
    pub decided_at: Option<Millis>,
    pub paths: Vec<PlanPath>,
}

impl Plan {
    /// The paths the plan changes: the one it replaces, then the ones it
    /// removes.
    pub fn taken_off(&self) -> impl Iterator<Item = &PlanPath> {
        let replaced = self
            .paths
            .iter()
            .filter(|p| p.action == PathAction::Replace);
        let removed = self.paths.iter().filter(|p| p.action == PathAction::Remove);
        replaced.chain(removed)
    }

    pub fn path(&self, path: &str) -> Option<&PlanPath> {
        self.paths.iter().find(|p| p.path == path)
    }

    /// The subtitle the episode has now, the one a person compares the new
    /// one with: the file the new copy replaces, else the first applied copy
    /// it removes, else the first file it keeps beside the video; with the
    /// file as the plan saw it. A plan has one whenever it was made, as a
    /// plan exists only for an episode that has a subtitle.
    pub fn current(&self) -> Option<(&PlanPath, &FileSeen)> {
        [PathAction::Replace, PathAction::Remove, PathAction::Keep]
            .into_iter()
            .find_map(|action| {
                self.paths
                    .iter()
                    .filter(|p| p.action == action)
                    .find_map(|p| Some((p, p.file.as_ref()?)))
            })
    }
}

/// What comparing the current subtitle of a plan with the new one came to
/// (`subtitle_replacement_diffs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    /// The current file compared, as in the plan's paths.
    pub path: String,
    pub result: Compared,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compared {
    /// What differs. Read from the records, it has no dialogue or timing
    /// lines in it (they are apart, [`comparison_lines`]); its counts are
    /// whole.
    Diff(Box<Diff>),
    /// Why the contents were not compared, in Korean for the screen: never
    /// shown as no difference.
    Unreadable(String),
}

/// What a comparison whose stored summary cannot be read now says.
const UNSTORED: &str = "저장된 비교를 읽지 못했어요";

const PLAN_COLUMNS: &str = "id, job_id, position, version, state, reason, work_id, season, \
     episode, assignment, basis, folder, video_path, video_object, video_size, video_mtime, \
     stored_id, asset_id, asset_path, asset_size, asset_sha256, asset_lines, target, created_at, \
     decided_at";

fn plan_head(r: &Row<'_>) -> rusqlite::Result<Plan> {
    let assignment: String = r.get(9)?;
    let basis: Option<String> = r.get(10)?;
    Ok(Plan {
        id: r.get(0)?,
        job_id: r.get(1)?,
        position: r.get(2)?,
        version: r.get(3)?,
        state: r.get(4)?,
        reason: r.get(5)?,
        work_id: r.get(6)?,
        season: r.get(7)?,
        episode: r.get(8)?,
        assignment: Assignment::parse(&assignment).unwrap_or(Assignment::Explicit),
        basis: basis.as_deref().and_then(Basis::parse),
        folder: r.get(11)?,
        video: VideoSeen {
            path: r.get(12)?,
            object: r.get(13)?,
            size: r.get::<_, i64>(14)? as u64,
            mtime: r.get(15)?,
        },
        stored_id: r.get(16)?,
        asset_id: r.get(17)?,
        asset_path: r.get(18)?,
        asset_size: r.get::<_, i64>(19)? as u64,
        asset_sha256: r.get(20)?,
        asset_lines: r.get::<_, Option<i64>>(21)?.map(|n| n as u64),
        target: r.get(22)?,
        created_at: r.get(23)?,
        decided_at: r.get(24)?,
        paths: Vec::new(),
    })
}

fn with_paths(c: &Connection, mut plan: Plan) -> rusqlite::Result<Plan> {
    let mut stmt = c.prepare_cached(
        "SELECT path, action, byte_size, sha256, object, mtime, lines, applied_id
           FROM subtitle_replacement_paths WHERE plan_id = ?1
          ORDER BY CASE action WHEN 'replace' THEN 0 WHEN 'add' THEN 0 WHEN 'remove' THEN 1
                               ELSE 2 END, path",
    )?;
    let rows = stmt.query_map([&plan.id], |r| {
        let sha: Option<String> = r.get(3)?;
        Ok(PlanPath {
            path: r.get(0)?,
            action: r.get(1)?,
            file: match sha {
                Some(sha256) => Some(FileSeen {
                    size: r.get::<_, i64>(2)? as u64,
                    sha256,
                    object: r.get(4)?,
                    mtime: r.get::<_, Option<i64>>(5)?.unwrap_or_default(),
                    lines: r.get::<_, Option<i64>>(6)?.map(|n| n as u64),
                }),
                None => None,
            },
            applied_id: r.get(7)?,
        })
    })?;
    plan.paths = rows.collect::<rusqlite::Result<_>>()?;
    Ok(plan)
}

pub fn plan(c: &Connection, id: &str) -> rusqlite::Result<Option<Plan>> {
    let head = c
        .prepare_cached(&format!(
            "SELECT {PLAN_COLUMNS} FROM subtitle_replacements WHERE id = ?1"
        ))?
        .query_row([id], plan_head)
        .optional()?;
    head.map(|p| with_paths(c, p)).transpose()
}

/// The row's plan to decide or carry out (`open` or `approved`).
pub fn live_plan(c: &Connection, job_id: &str, position: i64) -> rusqlite::Result<Option<Plan>> {
    let head = c
        .prepare_cached(&format!(
            "SELECT {PLAN_COLUMNS} FROM subtitle_replacements
                  WHERE job_id = ?1 AND position = ?2 AND state IN ('open', 'approved')"
        ))?
        .query_row(params![job_id, position], plan_head)
        .optional()?;
    head.map(|p| with_paths(c, p)).transpose()
}

/// The latest plan of each row of the job that has one, by position.
pub fn latest_plans(c: &Connection, job_id: &str) -> rusqlite::Result<Vec<Plan>> {
    let heads: Vec<Plan> = {
        let mut stmt = c.prepare_cached(&format!(
            "SELECT {PLAN_COLUMNS} FROM subtitle_replacements r
              WHERE job_id = ?1
                AND version = (SELECT max(version) FROM subtitle_replacements
                                WHERE job_id = r.job_id AND position = r.position)
              ORDER BY position"
        ))?;
        let rows = stmt.query_map([job_id], plan_head)?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    heads.into_iter().map(|p| with_paths(c, p)).collect()
}

/// The reason the version before `plan` went stale, if it did: why the
/// person compares again.
pub fn stale_before(c: &Connection, plan: &Plan) -> rusqlite::Result<Option<String>> {
    c.prepare_cached(
        "SELECT reason FROM subtitle_replacements
          WHERE job_id = ?1 AND position = ?2 AND version = ?3 AND state = 'stale'",
    )?
    .query_row(params![plan.job_id, plan.position, plan.version - 1], |r| {
        r.get::<_, Option<String>>(0)
    })
    .optional()
    .map(Option::flatten)
}

/// How many of the job's rows have a plan `open` and `approved`.
pub fn live_counts(c: &Connection, job_id: &str) -> rusqlite::Result<(usize, usize)> {
    c.prepare_cached(
        "SELECT count(*) FILTER (WHERE state = 'open'), count(*) FILTER (WHERE state = 'approved')
           FROM subtitle_replacements WHERE job_id = ?1",
    )?
    .query_row([job_id], |r| {
        Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as usize))
    })
}

/// Records a new plan for its row, the next version, `open`, with its
/// `comparison` and a note on the row; returns its version. One synced
/// transaction.
pub fn make_plan(
    c: &mut Connection,
    plan: &Plan,
    comparison: Comparison,
    note: &str,
    now: Millis,
) -> Result<i64, JobError> {
    let stored = Stored::of(comparison)?;
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx
            .prepare_cached(
                "SELECT coalesce(max(version), 0) + 1 FROM subtitle_replacements
              WHERE job_id = ?1 AND position = ?2",
            )?
            .query_row(params![plan.job_id, plan.position], |r| r.get(0))?;
        tx.prepare_cached(&format!(
            "INSERT INTO subtitle_replacements ({PLAN_COLUMNS}, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'open', NULL, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                         ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, NULL, ?22)"
        ))?
        .execute(params![
            plan.id,
            plan.job_id,
            plan.position,
            version,
            plan.work_id,
            plan.season,
            plan.episode,
            plan.assignment.code(),
            plan.basis.map(Basis::code),
            plan.folder,
            plan.video.path,
            plan.video.object,
            plan.video.size as i64,
            plan.video.mtime,
            plan.stored_id,
            plan.asset_id,
            plan.asset_path,
            plan.asset_size as i64,
            plan.asset_sha256,
            plan.asset_lines.map(|n| n as i64),
            plan.target,
            now,
        ])?;
        for path in &plan.paths {
            let file = path.file.as_ref();
            tx.prepare_cached(
                "INSERT INTO subtitle_replacement_paths
                     (plan_id, path, action, byte_size, sha256, object, mtime, lines, applied_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?
            .execute(params![
                plan.id,
                path.path,
                path.action,
                file.map(|f| f.size as i64),
                file.map(|f| f.sha256.as_str()),
                file.map(|f| f.object.as_str()),
                file.map(|f| f.mtime),
                file.and_then(|f| f.lines).map(|n| n as i64),
                path.applied_id,
            ])?;
        }
        tx.prepare_cached(
            "INSERT INTO subtitle_replacement_diffs (plan_id, path, diff, lines, unreadable)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?
        .execute(params![
            plan.id,
            stored.path,
            stored.diff,
            stored.lines,
            stored.unreadable
        ])?;
        // The row waits for the person: whatever it came to before (the
        // video it waited for, say) is over.
        tx.prepare_cached(
            "UPDATE subtitle_job_plan SET outcome = NULL, note = ?3, updated_at = ?4
              WHERE job_id = ?1 AND position = ?2",
        )?
        .execute(params![plan.job_id, plan.position, note, now])?;
        tx.commit()?;
        Ok(version)
    })
}

/// A comparison as its table keeps it.
struct Stored {
    path: String,
    diff: Option<String>,
    lines: Option<String>,
    unreadable: Option<String>,
}

impl Stored {
    /// The counts and the lines apart: reading a job's detail never reads a
    /// whole file's change.
    fn of(comparison: Comparison) -> Result<Self, JobError> {
        let json = |e: serde_json::Error| JobError::Other(format!("비교를 저장하지 못했어요: {e}"));
        let Comparison { path, result } = comparison;
        Ok(match result {
            Compared::Unreadable(reason) => Self {
                path,
                diff: None,
                lines: None,
                unreadable: Some(reason),
            },
            Compared::Diff(mut diff) => {
                let dialogue = std::mem::take(&mut diff.dialogue.lines);
                let timing = std::mem::take(&mut diff.timing.lines);
                let lines = format!(
                    "{{\"dialogue\":{},\"timing\":{}}}",
                    serde_json::to_string(&dialogue).map_err(json)?,
                    serde_json::to_string(&timing).map_err(json)?
                );
                Self {
                    path,
                    diff: Some(serde_json::to_string(&*diff).map_err(json)?),
                    lines: Some(lines),
                    unreadable: None,
                }
            }
        })
    }
}

/// The comparison of the plan, without the lines of a difference, if the plan
/// was made with one.
pub fn comparison(c: &Connection, plan_id: &str) -> rusqlite::Result<Option<Comparison>> {
    let found: Option<(String, Option<String>, Option<String>)> = c
        .prepare_cached(
            "SELECT path, diff, unreadable FROM subtitle_replacement_diffs WHERE plan_id = ?1",
        )?
        .query_row([plan_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    Ok(found.map(|(path, diff, unreadable)| Comparison {
        path,
        result: match (diff, unreadable) {
            (Some(json), _) => match serde_json::from_str::<Diff>(&json) {
                Ok(diff) => Compared::Diff(Box::new(diff)),
                Err(_) => Compared::Unreadable(UNSTORED.to_owned()),
            },
            (None, reason) => Compared::Unreadable(reason.unwrap_or_else(|| UNSTORED.to_owned())),
        },
    }))
}

/// The lines of the plan's difference as JSON text, `{"dialogue": [...],
/// "timing": [...]}` as the engine writes them: `None` unless the plan is
/// the job's and its comparison was made. The text is not parsed, as a
/// rewritten file's lines are large.
pub fn comparison_lines(
    c: &Connection,
    job_id: &str,
    plan_id: &str,
) -> rusqlite::Result<Option<String>> {
    c.prepare_cached(
        "SELECT d.lines FROM subtitle_replacement_diffs d
           JOIN subtitle_replacements r ON r.id = d.plan_id
          WHERE r.id = ?1 AND r.job_id = ?2 AND d.lines IS NOT NULL",
    )?
    .query_row(params![plan_id, job_id], |r| r.get(0))
    .optional()
}

/// Moves the plan from `from` to `to` with the reason; whether it was at
/// `from`.
pub fn move_plan(
    c: &mut Connection,
    id: &str,
    from: PlanState,
    to: PlanState,
    reason: Option<&str>,
    now: Millis,
) -> Result<bool, JobError> {
    durable(c, |c| {
        let n = c
            .prepare_cached(
                "UPDATE subtitle_replacements SET state = ?3, reason = coalesce(?4, reason),
                    updated_at = ?5
              WHERE id = ?1 AND state = ?2",
            )?
            .execute(params![id, from, to, reason, now])?;
        Ok(n == 1)
    })
}

/// What came of a person's decision on a plan ([`decide`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decided {
    /// Written; the job is in line again to carry it out or settle.
    Done(PlanState),
    NotFound,
    /// The plan is not the row's plan to decide any more: a newer version
    /// replaced it, or it was decided.
    Stale,
}

/// A person's decision on one plan of a job ([`decide_all`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub plan_id: String,
    pub version: i64,
    /// `true` approves the plan; `false` keeps the episode's subtitle.
    pub replace: bool,
}

/// A person's decision on the job's plan `plan_id` of `version`: `replace`
/// approves it, else the episode keeps its subtitle (the row is settled
/// `existing`). Refused unless it is the row's latest version and `open`.
/// A job waiting for the approval goes back in line; one that waits for
/// something else (held, a check, a site) keeps waiting and carries the
/// decision out once it runs again, and a run under way ends `pending` and
/// runs again ([`crate::store::JobRun::settle`]). With its log line. One
/// synced transaction. [`decide_all`] with one decision.
pub fn decide(
    c: &mut Connection,
    job_id: &str,
    plan_id: &str,
    version: i64,
    replace: bool,
    now: Millis,
) -> Result<Decided, JobError> {
    let decision = Decision {
        plan_id: plan_id.to_owned(),
        version,
        replace,
    };
    let mut decided = decide_all(c, job_id, std::slice::from_ref(&decision), now)?;
    Ok(decided.remove(0))
}

/// A person's decisions on several plans of the job at once, one per
/// episode: each is checked and written as [`decide`] does, in one synced
/// transaction, and the job is put back in line once. A plan that is no
/// longer the row's to decide is `Stale` and left as it is, the others are
/// written. If any plan is not the job's, nothing is written and that
/// plan's place holds `NotFound` and the others hold `Stale` (none of them
/// was written). The results are in the order of the decisions, and each is
/// checked against what the ones before it wrote, so a plan named twice is
/// written at most once.
pub fn decide_all(
    c: &mut Connection,
    job_id: &str,
    decisions: &[Decision],
    now: Millis,
) -> Result<Vec<Decided>, JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut results = Vec::with_capacity(decisions.len());
        let mut written = false;
        for (index, decision) in decisions.iter().enumerate() {
            let Decision {
                plan_id,
                version,
                replace,
            } = decision;
            let found: Option<(i64, i64, PlanState, i64, i64)> = tx
                .prepare_cached(
                    "SELECT position, version, state, episode,
                            (SELECT max(version) FROM subtitle_replacements o
                              WHERE o.job_id = r.job_id AND o.position = r.position)
                       FROM subtitle_replacements r WHERE id = ?1 AND job_id = ?2",
                )?
                .query_row(params![plan_id, job_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })
                .optional()?;
            let Some((position, found_version, state, episode, latest)) = found else {
                // Dropping the transaction writes nothing of the others.
                let mut refused = vec![Decided::Stale; decisions.len()];
                refused[index] = Decided::NotFound;
                return Ok(refused);
            };
            if found_version != *version || latest != *version || state != PlanState::Open {
                results.push(Decided::Stale);
                continue;
            }
            let to = match replace {
                true => PlanState::Approved,
                false => PlanState::Kept,
            };
            tx.prepare_cached(
                "UPDATE subtitle_replacements SET state = ?2, decided_at = ?3, updated_at = ?3
                  WHERE id = ?1",
            )?
            .execute(params![plan_id, to, now])?;
            let message = match replace {
                true => "새 자막으로 교체하기로 했어요",
                false => {
                    tx.prepare_cached("UPDATE subtitle_job_plan
                            SET outcome = 'existing', note = '현재 자막을 그대로 두고 보관만 했어요',
                                updated_at = ?3
                          WHERE job_id = ?1 AND position = ?2")?.execute(
                        params![job_id, position, now],
                    )?;
                    "현재 자막을 그대로 두기로 했어요"
                }
            };
            tx.prepare_cached(
                "INSERT INTO subtitle_job_events (job_id, at, message, detail)
                 VALUES (?1, ?2, ?3, ?4)",
            )?
            .execute(params![job_id, now, message, format!("{episode}화")])?;
            written = true;
            results.push(Decided::Done(to));
        }
        if written {
            tx.prepare_cached(
                "UPDATE subtitle_jobs
                    SET state = 'pending', wait = NULL, note = ?3, finished_at = NULL,
                        state_at = ?2
                  WHERE id = ?1 AND state = 'waiting' AND wait = 'approval'",
            )?
            .execute(params![job_id, now, DECIDED])?;
            tx.prepare_cached("UPDATE subtitle_jobs SET updated_at = ?2 WHERE id = ?1")?
                .execute(params![job_id, now])?;
        }
        tx.commit()?;
        Ok(results)
    })
}

/// What came of claiming the paths of an approved plan ([`claim`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claimed {
    /// Its effects are recorded `intended`.
    Yes,
    /// Another effect under way aims at one of its paths: that one.
    Busy(String),
    /// The plan is not approved any more.
    NotApproved,
}

/// Records the approved plan's effects as `intended` in one synced
/// transaction, unless another effect under way (of another plan, or no
/// plan) aims at or takes off one of their paths: the paths beside the
/// video change one replacement at a time.
pub fn claim(
    c: &mut Connection,
    plan: &Plan,
    effects: &[Effect],
    now: Millis,
) -> Result<Claimed, JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: Option<PlanState> = tx
            .prepare_cached("SELECT state FROM subtitle_replacements WHERE id = ?1")?
            .query_row([&plan.id], |r| r.get(0))
            .optional()?;
        if state != Some(PlanState::Approved) {
            return Ok(Claimed::NotApproved);
        }
        let busy = place_records::busy_targets(&tx, &plan.folder, None)?;
        let ours: Vec<String> = {
            let mut stmt = tx.prepare_cached(
                "SELECT lower(target) FROM subtitle_file_effects
                  WHERE plan_id = ?1 AND state IN ('intended', 'prepared', 'set_aside')
                 UNION
                 SELECT lower(source) FROM subtitle_file_effects
                  WHERE plan_id = ?1 AND kind = 'remove'
                    AND state IN ('intended', 'prepared', 'set_aside')",
            )?;
            let rows = stmt.query_map([&plan.id], |r| r.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let wanted = effects
            .iter()
            .flat_map(|e| [Some(&e.target), e.source.as_ref()])
            .flatten();
        for path in wanted {
            let lower = path.to_lowercase();
            if busy.contains(&lower) && !ours.contains(&lower) {
                return Ok(Claimed::Busy(path.clone()));
            }
        }
        for effect in effects {
            place_records::insert_intended(&tx, effect, now)?;
        }
        tx.commit()?;
        Ok(Claimed::Yes)
    })
}

/// The plan's effects that did not end.
pub fn unfinished_of(c: &Connection, plan_id: &str) -> rusqlite::Result<Vec<Effect>> {
    let mut stmt = c.prepare_cached(&format!(
        "SELECT {EFFECT_COLUMNS} FROM subtitle_file_effects
          WHERE plan_id = ?1 AND state IN ('intended', 'prepared', 'set_aside')
          ORDER BY created_at, id"
    ))?;
    let rows = stmt.query_map([plan_id], place_records::effect)?;
    rows.collect()
}

/// The plan's effects, every state.
pub fn effects_of(c: &Connection, plan_id: &str) -> rusqlite::Result<Vec<Effect>> {
    let mut stmt = c.prepare_cached(&format!(
        "SELECT {EFFECT_COLUMNS} FROM subtitle_file_effects WHERE plan_id = ?1
          ORDER BY created_at, id"
    ))?;
    let rows = stmt.query_map([plan_id], place_records::effect)?;
    rows.collect()
}

/// A removal's file is renamed aside: the effect is `set_aside`, and the
/// applied copy it was (for a removal of one) is recorded as removed, in one
/// synced transaction. Nothing is written of an effect that is not
/// `prepared` any more (held meanwhile).
pub fn set_aside(
    c: &mut Connection,
    effect_id: &str,
    removed: Option<&str>,
    now: Millis,
) -> Result<(), JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let moved = tx
            .prepare_cached(
                "UPDATE subtitle_file_effects SET state = 'set_aside', updated_at = ?2
              WHERE id = ?1 AND state = 'prepared'",
            )?
            .execute(params![effect_id, now])?;
        if let Some(applied) = removed.filter(|_| moved == 1) {
            tx.prepare_cached(
                "UPDATE subtitle_applied SET removed_at = ?2 WHERE id = ?1 AND removed_at IS NULL",
            )?
            .execute(params![applied, now])?;
        }
        tx.commit()?;
        Ok(())
    })
}

/// Ends the plan `held`, its effects that did not end too, and its row,
/// with the reason; one synced transaction.
pub fn hold_plan(
    c: &mut Connection,
    plan: &Plan,
    reason: &str,
    now: Millis,
) -> Result<(), JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.prepare_cached(
            "UPDATE subtitle_replacements SET state = 'held', reason = ?2, updated_at = ?3
              WHERE id = ?1 AND state = 'approved'",
        )?
        .execute(params![plan.id, reason, now])?;
        tx.prepare_cached(
            "UPDATE subtitle_file_effects SET state = 'held', reason = ?2, updated_at = ?3
              WHERE plan_id = ?1 AND state IN ('intended', 'prepared', 'set_aside')",
        )?
        .execute(params![plan.id, reason, now])?;
        tx.prepare_cached(
            "UPDATE subtitle_job_plan SET outcome = 'held', note = ?3, updated_at = ?4
              WHERE job_id = ?1 AND position = ?2",
        )?
        .execute(params![plan.job_id, plan.position, reason, now])?;
        tx.commit()?;
        Ok(())
    })
}

/// Ends the effect, which left nothing behind, `abandoned`.
pub fn abandon(c: &mut Connection, effect_id: &str, now: Millis) -> Result<(), JobError> {
    place_records::end_effect(c, effect_id, EffectState::Abandoned, None, now)
}

/// A newer stored subtitle than `stored_id` of the same source and episode
/// (a revision received since), if one is that a person did not clean.
pub fn newer_revision(c: &Connection, stored_id: &str) -> rusqlite::Result<Option<String>> {
    c.prepare_cached(
        "SELECT n.id FROM subtitle_stored s JOIN subtitle_stored n
             ON n.work_id = s.work_id AND n.season = s.season AND n.episode = s.episode
            AND n.source_id = s.source_id AND n.id <> s.id
            AND n.subtitle_asset_id <> s.subtitle_asset_id AND n.cleaned_at IS NULL
            AND (n.stored_at > s.stored_at OR (n.stored_at = s.stored_at AND n.id > s.id))
          WHERE s.id = ?1 AND s.source_id IS NOT NULL
          ORDER BY n.stored_at DESC LIMIT 1",
    )?
    .query_row([stored_id], |r| r.get(0))
    .optional()
}

/// Where a stored subtitle is now: its episode, what puts it there, and its
/// asset. A stored subtitle a person cleaned is nowhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPlace {
    pub episode: Option<i64>,
    pub assignment: Option<Assignment>,
    pub basis: Option<Basis>,
    pub asset_id: String,
}

pub fn stored_place(c: &Connection, stored_id: &str) -> rusqlite::Result<Option<StoredPlace>> {
    c.prepare_cached(
        "SELECT episode, assignment, basis, subtitle_asset_id FROM subtitle_stored
          WHERE id = ?1 AND cleaned_at IS NULL",
    )?
    .query_row([stored_id], |r| {
        let assignment: Option<String> = r.get(1)?;
        let basis: Option<String> = r.get(2)?;
        Ok(StoredPlace {
            episode: r.get(0)?,
            assignment: assignment.as_deref().and_then(Assignment::parse),
            basis: basis.as_deref().and_then(Basis::parse),
            asset_id: r.get(3)?,
        })
    })
    .optional()
}

/// The live applied copies of the episode, by path: the files the app put
/// beside its video and did not take away.
pub fn applied_on(
    c: &Connection,
    work_id: &str,
    season: u32,
    episode: i64,
) -> rusqlite::Result<Vec<(String, String, String)>> {
    let mut stmt = c.prepare_cached(
        "SELECT id, path, sha256 FROM subtitle_applied
          WHERE work_id = ?1 AND season = ?2 AND episode = ?3 AND removed_at IS NULL
          ORDER BY applied_at, id",
    )?;
    let rows = stmt.query_map(params![work_id, season, episode], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    rows.collect()
}

/// What an imported subtitle is: the bytes of a file the app found beside
/// the episode's video and did not manage.
#[derive(Debug, Clone)]
pub struct Imported {
    pub work_id: String,
    pub season: u32,
    pub episode: i64,
    pub job_id: String,
    /// The file's name beside the video.
    pub original_name: String,
    pub format: SubtitleFormat,
    pub encoding: Option<String>,
}

/// Records an import: its package (`existing`), the asset (the effect's
/// published file, or `reused`), the package's entry, and the stored
/// subtitle of the creator nobody named on the episode (or the one stored
/// already for the same bytes there, while a person did not clean it); the
/// effect, if it made the file, is
/// `done`. One synced transaction; returns the stored subtitle's ID.
pub fn imported(
    c: &mut Connection,
    what: &Imported,
    effect: Option<&Effect>,
    reused: Option<&str>,
    now: Millis,
) -> Result<String, JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let asset_id = match (effect, reused) {
            (_, Some(id)) => id.to_owned(),
            (Some(effect), None) => {
                let id = uuid::Uuid::new_v4().to_string();
                tx.prepare_cached(
                    "INSERT INTO subtitle_assets
                         (id, work_id, kind, base, relative_path, byte_size, sha256, created_at)
                     VALUES (?1, ?2, 'subtitle', 'work', ?3, ?4, ?5, ?6)",
                )?
                .execute(params![
                    id,
                    what.work_id,
                    effect.target,
                    effect.size as i64,
                    effect.sha256,
                    now
                ])?;
                tx.prepare_cached(
                    "UPDATE subtitle_file_effects SET state = 'done', updated_at = ?2
                      WHERE id = ?1",
                )?
                .execute(params![effect.id, now])?;
                id
            }
            (None, None) => return Err(JobError::Missing("the asset of an import")),
        };
        let same: Option<String> = tx
            .prepare_cached(
                "SELECT id FROM subtitle_stored
                  WHERE subtitle_asset_id = ?1 AND work_id = ?2 AND season = ?3
                    AND source_id IS NULL AND creator IS NULL AND episode = ?4
                    AND cleaned_at IS NULL",
            )?
            .query_row(
                params![asset_id, what.work_id, what.season, what.episode],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = same {
            tx.commit()?;
            return Ok(id);
        }
        let package = uuid::Uuid::new_v4().to_string();
        tx.prepare_cached(
            "INSERT INTO subtitle_packages (id, work_id, job_id, source_kind, created_at)
             VALUES (?1, ?2, ?3, 'existing', ?4)",
        )?
        .execute(params![package, what.work_id, what.job_id, now])?;
        tx.prepare_cached(
            "INSERT INTO subtitle_package_entries (package_id, position, asset_id, original_name)
             VALUES (?1, 0, ?2, ?3)",
        )?
        .execute(params![package, asset_id, what.original_name])?;
        let id = uuid::Uuid::new_v4().to_string();
        tx.prepare_cached(
            "INSERT INTO subtitle_stored
                 (id, work_id, season, package_id, subtitle_asset_id, assignment, episode,
                  format, encoding, links_known, job_id, stored_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'explicit', ?6, ?7, ?8, 0, ?9, ?10)",
        )?
        .execute(params![
            id,
            what.work_id,
            what.season,
            package,
            asset_id,
            what.episode,
            what.format,
            what.encoding,
            what.job_id,
            now
        ])?;
        tx.commit()?;
        Ok(id)
    })
}

/// What the screens show of the stored subtitle a plan names or the applied
/// copy at a path is of: its creator, format, the post it came from, when it
/// was received (else stored), and its stored file (relative to the work
/// folder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFacts {
    pub creator: Option<String>,
    pub format: SubtitleFormat,
    pub source_kind: String,
    pub post: Option<String>,
    pub received_at: Millis,
    pub encoding: Option<String>,
    pub asset_path: String,
}

pub fn stored_facts(c: &Connection, stored_id: &str) -> rusqlite::Result<Option<StoredFacts>> {
    c.prepare_cached(
        "SELECT s.creator, s.format, p.source_kind, p.source_page,
                coalesce(p.received_at, s.stored_at), s.encoding, a.relative_path
           FROM subtitle_stored s JOIN subtitle_packages p ON p.id = s.package_id
                JOIN subtitle_assets a ON a.id = s.subtitle_asset_id
          WHERE s.id = ?1",
    )?
    .query_row([stored_id], |r| {
        Ok(StoredFacts {
            creator: r.get(0)?,
            format: r.get(1)?,
            source_kind: r.get(2)?,
            post: r.get(3)?,
            received_at: r.get(4)?,
            encoding: r.get(5)?,
            asset_path: r.get(6)?,
        })
    })
    .optional()
}

/// The stored subtitle an applied copy is of.
pub fn applied_stored(c: &Connection, applied_id: &str) -> rusqlite::Result<Option<String>> {
    c.prepare_cached("SELECT stored_id FROM subtitle_applied WHERE id = ?1")?
        .query_row([applied_id], |r| r.get(0))
        .optional()
}

/// A plan as the job's detail shows it: with why the version before went
/// stale, and what is known of the new subtitle and of each applied copy it
/// takes off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanView {
    pub plan: Plan,
    /// Why the version before went stale (`다시 비교 필요`).
    pub previous: Option<String>,
    pub new: Option<StoredFacts>,
    /// For each path of an applied copy: what it is a copy of.
    pub applied: Vec<(String, StoredFacts)>,
    /// What differs between the current subtitle and the new one, without
    /// the lines ([`comparison_lines`]); `None` for a plan made before the
    /// app compared contents.
    pub comparison: Option<Comparison>,
}

/// The latest plan of each row of the job, for its detail.
pub fn views(c: &Connection, job_id: &str) -> rusqlite::Result<Vec<PlanView>> {
    let mut views = Vec::new();
    for plan in latest_plans(c, job_id)? {
        let previous = stale_before(c, &plan)?;
        let new = stored_facts(c, &plan.stored_id)?;
        let mut applied = Vec::new();
        for path in &plan.paths {
            let Some(id) = &path.applied_id else {
                continue;
            };
            if let Some(stored) = applied_stored(c, id)? {
                if let Some(facts) = stored_facts(c, &stored)? {
                    applied.push((path.path.clone(), facts));
                }
            }
        }
        let comparison = comparison(c, &plan.id)?;
        views.push(PlanView {
            plan,
            previous,
            new,
            applied,
            comparison,
        });
    }
    Ok(views)
}
