//! A source's episode mapping changes: its stored subtitles and the unfinished
//! rows of its jobs go to their new episodes, and the copies the app applied
//! move with a person's confirmation (재배치; `docs/specs/library.md`, 자막의
//! 회차 대응; `docs/specs/subtitles.md`, 배치 확인; `docs/specs/jobs.md`,
//! 체크포인트와 중단 복구).
//!
//! # Re-evaluation
//!
//! Every write that changes a mapping calls [`reevaluate_in`] in its own
//! transaction, so a mapping change and a file effect are ordered by which
//! committed first. Only a decided mapping (the app's `auto` or the user's)
//! moves anything: while a mapping is undecided or the source has none, each
//! subtitle keeps the episode its last decided mapping gave it, and the next
//! decided one is taken from the episode as it was observed.
//!
//! What follows the mapping (`assignment = mapped`) is taken from its
//! observed episode (`basis`: the candidate's episode as Anissia wrote it,
//! or the one the file's name says) through the new mapping
//! ([`crate::mapping::Mapping::season_episode`]), within the season's
//! episodes when their count is known. What was put on its observed
//! episode's own number for want of a mapping (`same_number`) is taken
//! through the first decided one the same way, and is `mapped` from then on
//! (an unfinished row or a stored subtitle; a row done is a record). The
//! observed episodes and the records of what was applied where are never
//! rewritten. A person's choice (`explicit`) is not touched.
//!
//! | What | New episode found | No episode found (an exception that does not receive it, past the season, a text the mapping does not take) |
//! | --- | --- | --- |
//! | a stored subtitle | its `episode` is the new one | it stays where it is |
//! | an unfinished row of a job (not done yet, or waiting for its video) | its `episode` is the new one | it asks a person ([`HELD`]), on the episode it had |
//!
//! Either way an open replacement plan of a changed row goes stale
//! (`회차 대응이 바뀌었어요`): an approval is never used for another target.
//! A job that waits for a person, a video or an approval goes back in line;
//! a job a worker runs goes back in line when that run ends
//! (`subtitle_jobs.remapped_at`).
//!
//! # The relocation job
//!
//! A stored subtitle whose applied copy (적용본) is now on another episode
//! than the stored subtitle is moved by the source's relocation job
//! ([`sync_in`], origin [`RELOCATE`]): a plan row applies the stored
//! subtitle on its new episode and its removals take the copies off the old
//! one, shown and confirmed together (배치 확인, `회차 확인 필요`). Nothing
//! moves until then; a copy left unconfirmed stays where it is. The source
//! has one such job waiting at a time: a later change plans it anew (a
//! screen that showed the earlier plan is refused), and a change that puts
//! every copy back ends it. A copy a confirmed relocation is taking off is
//! not planned again.
//!
//! Once confirmed, the job takes off every copy first ([`Placer::relocate`]),
//! then applies its rows as any job does: a new episode that has a subtitle
//! waits for a replacement's approval, one with no video for its video.
//! Taking the copies off first keeps a mapping moved by one episode from
//! asking to replace each copy with the next.
//!
//! # Taking a copy off
//!
//! A removal looks again first: a copy already removed is done; a copy whose
//! stored subtitle is back on its episode, a path another effect under way
//! uses, a copy whose stored subtitle's file is not in the work folder as
//! recorded (the copy may be the last of its bytes; an empty mount point
//! has none, so its copies are not taken for gone) or a copy a person
//! changed (its bytes are not the applied ones) is kept where it is. Then
//! the copy is renamed, replacing nothing, to `.trss/tmp/<removal>.aside` (`intended` before, `set_aside` once it is
//! found to be the applied bytes there), that file is removed, and the
//! removal is `done` with the applied copy recorded removed. The stored
//! subtitle keeps its bytes, so no copy of them is made first, and the
//! bytes alone say a file is the applied copy (see [`Found::Copy`]).
//!
//! | Record | On disk | Then |
//! | --- | --- | --- |
//! | `intended` | the aside file, the applied bytes | `set_aside`, then removed |
//! | `intended` | the aside file, anything else | put back on its path (`kept`), or `held` when that path is taken |
//! | `intended` | no aside file, the copy on its path | taken off anew |
//! | `intended` | no aside file, nothing on its path | `done`: it is gone (once the stored file is found as recorded; `kept` otherwise) |
//! | `intended` | no aside file, another file on its path | `kept` |
//! | `set_aside` | the aside file or none | removed, `done` |
//!
//! A held removal holds the rows it touches, with its files as they are: its
//! own row (the copy may still be on its old episode) and every row applied
//! on its copy's episode (the copy may still be there). The other rows are
//! applied, so the copies already taken off are not left applied nowhere,
//! and the job ends held once none of them waits for a person, a
//! replacement's approval, a video or the work folder
//! ([`crate::place::Standing::held_removal`]).

use std::{collections::BTreeMap, path::Path};

use rusqlite::{params, Connection, OptionalExtension};
use trss_core::{files::rename_noreplace, Millis};

use crate::{
    area::sync_dir,
    mapping::{self, Mapped, Mapping},
    model::{JobState, RemovalState, StepKind},
    place::{
        blocking,
        episode::Basis,
        files,
        records::{self, durable},
        Placer,
    },
    store::{JobError, RELOCATE},
};

/// What an unfinished row asks when the new mapping gives it no episode.
pub const HELD: &str = "바뀐 회차 대응으로 회차를 정하지 못했어요";

/// Why an open replacement plan of a row the mapping moved goes stale.
pub const STALE: &str = "회차 대응이 바뀌었어요";

/// The note of a job the mapping change puts back in line.
pub const REMAPPED: &str = "회차 대응이 바뀌어 다시 살펴봐요";

/// The note of a relocation job no copy is left to move by.
pub const NOTHING_TO_MOVE: &str = "회차 대응이 다시 바뀌어 옮길 적용본이 없어요";

/// What a mapping change came to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Remapped {
    /// Stored subtitles given another episode.
    pub stored: usize,
    /// Unfinished rows given another episode, or asked about.
    pub rows: usize,
    /// The jobs that go back in line, now or when their run ends.
    pub jobs: Vec<String>,
    /// The source's relocation job that waits for a person, if it has one.
    pub relocation: Option<String>,
}

/// The season episode `basis` gives through `mapping`, within the season's
/// `total` episodes when known; why none, as a sentence, otherwise.
fn target(
    mapping: &Mapping,
    basis: Option<Basis>,
    anissia: Option<&str>,
    attachment: Option<&str>,
    total: Option<u32>,
) -> Result<i64, String> {
    let text = match basis {
        Some(Basis::Anissia) => anissia,
        Some(Basis::Attachment) => attachment,
        None => None,
    };
    let Some(text) = text.map(str::trim).filter(|t| !t.is_empty()) else {
        return Err("원래 회차의 기록이 없어요".to_owned());
    };
    let label = match trss_subtitles::episode::numeric_key(text) {
        Some(_) => format!("{text}화"),
        None => text.to_owned(),
    };
    match mapping.season_episode(text) {
        Mapped::Episode(n) if n >= 1 && total.is_none_or(|t| n <= i64::from(t)) => Ok(n),
        Mapped::Episode(n) => Err(format!(
            "{label}를 옮긴 시즌 {n}화가 시즌의 {} 밖이에요",
            match total {
                Some(t) => format!("1–{t}화"),
                None => "1화부터".to_owned(),
            }
        )),
        Mapped::NotReceived => Err(format!("회차 대응이 {label}를 받지 않는 회차로 정했어요")),
        Mapped::Unmapped => Err(format!("{label}는 회차 대응으로 옮길 수 없는 회차예요")),
    }
}

/// Re-evaluates what follows the source's mapping in the season after a
/// write that changed it (see the module docs), then brings the source's
/// relocation job up to date ([`sync_in`]). `total` is the season's episode
/// count, when known. Call it in the transaction of the write.
pub fn reevaluate_in(
    c: &Connection,
    work_id: &str,
    season: u32,
    source_id: &str,
    total: Option<u32>,
    now: Millis,
) -> rusqlite::Result<Remapped> {
    let mut out = Remapped::default();
    let Some(mapping) = mapping::read_in(c, work_id, season)?.remove(source_id) else {
        return Ok(out);
    };
    if mapping.decided_offset().is_none() {
        return Ok(out);
    }
    let parse_basis = |code: Option<String>| code.as_deref().and_then(Basis::parse);

    // The stored subtitles.
    struct Stored {
        id: String,
        episode: i64,
        basis: Option<Basis>,
        anissia: Option<String>,
        attachment: Option<String>,
    }
    let stored: Vec<Stored> = {
        let mut stmt = c.prepare_cached(
            "SELECT id, episode, basis, anissia_episode, attachment_episode FROM subtitle_stored
              WHERE work_id = ?1 AND season = ?2 AND source_id = ?3
                AND assignment IN ('mapped', 'same_number') AND cleaned_at IS NULL
              ORDER BY id",
        )?;
        let rows = stmt.query_map(params![work_id, season, source_id], |r| {
            Ok(Stored {
                id: r.get(0)?,
                episode: r.get(1)?,
                basis: parse_basis(r.get(2)?),
                anissia: r.get(3)?,
                attachment: r.get(4)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for s in stored {
        let found = target(
            &mapping,
            s.basis,
            s.anissia.as_deref(),
            s.attachment.as_deref(),
            total,
        );
        if let Some(n) = found.ok().filter(|n| *n != s.episode) {
            c.prepare_cached("UPDATE subtitle_stored SET episode = ?2 WHERE id = ?1")?
                .execute(params![s.id, n])?;
            out.stored += 1;
        }
    }
    c.prepare_cached(
        "UPDATE subtitle_stored SET assignment = 'mapped'
          WHERE work_id = ?1 AND season = ?2 AND source_id = ?3 AND assignment = 'same_number'
            AND cleaned_at IS NULL",
    )?
    .execute(params![work_id, season, source_id])?;

    // The unfinished rows of the source's jobs; a relocation that waits for
    // its confirmation is planned anew below instead.
    struct Unfinished {
        job_id: String,
        position: i64,
        episode: Option<i64>,
        basis: Option<Basis>,
        anissia: Option<String>,
        attachment: Option<String>,
        question: Option<String>,
        state: JobState,
        wait: Option<String>,
    }
    let unfinished: Vec<Unfinished> = {
        let mut stmt = c.prepare_cached(
            "SELECT p.job_id, p.position, p.episode, p.basis, p.anissia_episode,
                    p.attachment_episode, p.question, j.state, j.wait
               FROM subtitle_job_plan p JOIN subtitle_jobs j ON j.id = p.job_id
              WHERE j.work_id = ?1 AND j.season = ?2 AND j.source_id = ?3
                AND p.assignment IN ('mapped', 'same_number')
                AND (p.outcome IS NULL OR p.outcome = 'no_video')
                AND NOT (j.origin = ?4 AND j.placement_confirmed_at IS NULL)
              ORDER BY j.seq, p.position",
        )?;
        let rows = stmt.query_map(params![work_id, season, source_id, RELOCATE], |r| {
            Ok(Unfinished {
                job_id: r.get(0)?,
                position: r.get(1)?,
                episode: r.get(2)?,
                basis: parse_basis(r.get(3)?),
                anissia: r.get(4)?,
                attachment: r.get(5)?,
                question: r.get(6)?,
                state: r.get(7)?,
                wait: r.get(8)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut touched: BTreeMap<String, (JobState, Option<String>)> = BTreeMap::new();
    for row in unfinished {
        let held = row.question.as_deref().is_some_and(|q| q.starts_with(HELD));
        let found = target(
            &mapping,
            row.basis,
            row.anissia.as_deref(),
            row.attachment.as_deref(),
            total,
        );
        let changed = match found {
            Ok(n) if Some(n) != row.episode || held => c
                .prepare_cached(
                    "UPDATE subtitle_job_plan
                    SET episode = ?3, question = CASE WHEN ?4 THEN NULL ELSE question END,
                        updated_at = ?5
                  WHERE job_id = ?1 AND position = ?2",
                )?
                .execute(params![row.job_id, row.position, n, held, now])?,
            Err(why) if row.question.is_none() => c
                .prepare_cached(
                    "UPDATE subtitle_job_plan SET question = ?3, updated_at = ?4
                  WHERE job_id = ?1 AND position = ?2",
                )?
                .execute(params![
                    row.job_id,
                    row.position,
                    format!("{HELD}: {why}"),
                    now
                ])?,
            _ => 0,
        };
        if changed == 0 {
            continue;
        }
        out.rows += 1;
        c.prepare_cached(
            "UPDATE subtitle_replacements SET state = 'stale', reason = ?3, updated_at = ?4
              WHERE job_id = ?1 AND position = ?2 AND state = 'open'",
        )?
        .execute(params![row.job_id, row.position, STALE, now])?;
        touched.insert(row.job_id, (row.state, row.wait));
    }
    // The same target, now the mapping's: an open replacement plan of a row
    // left on its episode holds ([`crate::place::replace`]). Not a change of
    // the row's placement, so `updated_at` (which orders a stored subtitle's
    // rows) stays.
    c.prepare_cached(
        "UPDATE subtitle_job_plan SET assignment = 'mapped'
          WHERE assignment = 'same_number' AND (outcome IS NULL OR outcome = 'no_video')
            AND job_id IN (SELECT id FROM subtitle_jobs
                            WHERE work_id = ?1 AND season = ?2 AND source_id = ?3)",
    )?
    .execute(params![work_id, season, source_id])?;
    for (job, (state, wait)) in touched {
        let back = match state {
            JobState::Running => {
                c.prepare_cached("UPDATE subtitle_jobs SET remapped_at = ?2 WHERE id = ?1")?
                    .execute(params![job, now])?;
                true
            }
            JobState::Waiting
                if matches!(wait.as_deref(), Some("placement" | "approval" | "video")) =>
            {
                requeue(c, &job, now)?
            }
            JobState::Partial | JobState::Done => requeue(c, &job, now)?,
            _ => false,
        };
        if back {
            out.jobs.push(job);
        }
    }
    out.relocation = sync_in(c, work_id, season, source_id, now)?;
    Ok(out)
}

/// Puts the job back in line for its rows' new episodes, with a log line.
fn requeue(c: &Connection, job: &str, now: Millis) -> rusqlite::Result<bool> {
    let changed = c
        .prepare_cached(
            "UPDATE subtitle_jobs
            SET state = 'pending', wait = NULL, finished_at = NULL, note = ?2, state_at = ?3,
                updated_at = ?3
          WHERE id = ?1",
        )?
        .execute(params![job, REMAPPED, now])?;
    event(c, job, REMAPPED, None, now)?;
    Ok(changed > 0)
}

fn event(
    c: &Connection,
    job: &str,
    message: &str,
    detail: Option<&str>,
    now: Millis,
) -> rusqlite::Result<()> {
    c.prepare_cached(
        "INSERT INTO subtitle_job_events (job_id, at, message, detail) VALUES (?1, ?2, ?3, ?4)",
    )?
    .execute(params![job, now, message, detail])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// The relocation job

/// One stored subtitle a relocation moves: applied on `to` (`None`: it is
/// applied there already), its copies on other episodes taken off.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Move {
    stored_id: String,
    to: Option<i64>,
    /// `(applied copy, its episode, its path)`, by ID.
    off: Vec<(String, i64, String)>,
}

/// The moves the source's applied copies need now: each live copy of a
/// mapped stored subtitle that is on another episode than the stored
/// subtitle, but one another relocation than `waiting` (the source's job
/// that waits for its confirmation) plans or is taking off, or one whose
/// removal is held. A stored subtitle no job row names (there is nothing to
/// apply it from) is left out. A copy on the stored subtitle's episode that a
/// removal has started taking off (its episode changed back after the start)
/// does not count as applied there: it is going.
fn wanted(
    c: &Connection,
    work_id: &str,
    season: u32,
    source_id: &str,
    waiting: Option<&str>,
) -> rusqlite::Result<Vec<Move>> {
    let mut stmt = c.prepare_cached(
        "SELECT s.id, s.episode, a.id, a.episode, a.path,
                EXISTS (SELECT 1 FROM subtitle_applied b
                         WHERE b.stored_id = s.id AND b.removed_at IS NULL
                           AND b.episode = s.episode
                           AND NOT EXISTS (
                               SELECT 1 FROM subtitle_relocations r
                                WHERE r.applied_id = b.id
                                  AND r.state IN ('intended', 'set_aside'))),
                EXISTS (SELECT 1 FROM subtitle_job_plan p WHERE p.stored_id = s.id)
           FROM subtitle_applied a JOIN subtitle_stored s ON s.id = a.stored_id
          WHERE s.work_id = ?1 AND s.season = ?2 AND s.source_id = ?3
            AND s.assignment = 'mapped' AND s.cleaned_at IS NULL
            AND a.removed_at IS NULL AND a.episode <> s.episode
            AND NOT EXISTS (
                SELECT 1 FROM subtitle_relocations r
                 WHERE r.applied_id = a.id
                   AND (r.state IN ('intended', 'set_aside', 'held')
                        OR (r.state = 'planned' AND r.job_id IS NOT ?4)))
          ORDER BY s.episode, s.id, a.id",
    )?;
    let rows = stmt.query_map(params![work_id, season, source_id, waiting], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, bool>(5)?,
            r.get::<_, bool>(6)?,
        ))
    })?;
    let mut moves: Vec<Move> = Vec::new();
    for row in rows {
        let (stored_id, episode, applied, on, path, there, named) = row?;
        if !there && !named {
            continue;
        }
        match moves.last_mut().filter(|m| m.stored_id == stored_id) {
            Some(m) => m.off.push((applied, on, path)),
            None => moves.push(Move {
                stored_id,
                to: (!there).then_some(episode),
                off: vec![(applied, on, path)],
            }),
        }
    }
    Ok(moves)
}

/// The source's relocation job that waits for its confirmation.
fn waiting_job(
    c: &Connection,
    work_id: &str,
    season: u32,
    source_id: &str,
) -> rusqlite::Result<Option<String>> {
    c.prepare_cached(
        "SELECT id FROM subtitle_jobs
          WHERE origin = ?4 AND work_id = ?1 AND season = ?2 AND source_id = ?3
            AND placement_confirmed_at IS NULL AND state = 'waiting'
          ORDER BY seq DESC LIMIT 1",
    )?
    .query_row(params![work_id, season, source_id, RELOCATE], |r| r.get(0))
    .optional()
}

/// What the job plans now, as [`wanted`] says it.
fn planned(c: &Connection, job: &str) -> rusqlite::Result<Vec<Move>> {
    let mut stmt = c.prepare_cached(
        "SELECT a.stored_id, p.episode, r.applied_id, r.episode, r.path
           FROM subtitle_relocations r JOIN subtitle_applied a ON a.id = r.applied_id
           LEFT JOIN subtitle_job_plan p ON p.job_id = r.job_id AND p.position = r.position
          WHERE r.job_id = ?1 AND r.state = 'planned'
          ORDER BY coalesce(p.episode, (SELECT episode FROM subtitle_stored WHERE id = a.stored_id)),
                   a.stored_id, r.applied_id",
    )?;
    let rows = stmt.query_map([job], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    let mut moves: Vec<Move> = Vec::new();
    for row in rows {
        let (stored_id, to, applied, on, path) = row?;
        match moves.last_mut().filter(|m| m.stored_id == stored_id) {
            Some(m) => m.off.push((applied, on, path)),
            None => moves.push(Move {
                stored_id,
                to,
                off: vec![(applied, on, path)],
            }),
        }
    }
    Ok(moves)
}

/// The note of a relocation job waiting for its confirmation.
fn confirm_note(moves: &[Move]) -> String {
    let off: usize = moves.iter().map(|m| m.off.len()).sum();
    format!("회차 대응이 바뀌어 적용본 {off}개를 옮길 계획을 확인해 주세요")
}

/// Brings the source's relocation job up to date with the moves its applied
/// copies need now (see the module docs): makes one when none waits, plans
/// the waiting one anew when they changed, or ends it when none is left.
/// Returns the job that waits, if one does. Call it in a write transaction.
pub fn sync_in(
    c: &Connection,
    work_id: &str,
    season: u32,
    source_id: &str,
    now: Millis,
) -> rusqlite::Result<Option<String>> {
    let job = waiting_job(c, work_id, season, source_id)?;
    let moves = wanted(c, work_id, season, source_id, job.as_deref())?;
    match (job, moves.is_empty()) {
        (None, true) => Ok(None),
        (Some(job), true) => {
            nothing_to_move(c, &job, now)?;
            Ok(None)
        }
        (Some(job), false) if planned(c, &job)? == moves => Ok(Some(job)),
        (Some(job), false) => {
            if plan(c, &job, &moves, now)? == 0 {
                nothing_to_move(c, &job, now)?;
                return Ok(None);
            }
            event(
                c,
                &job,
                "회차 대응이 다시 바뀌어 재배치 계획을 새로 만들었어요",
                Some(&waiting_note(c, &job)?),
                now,
            )?;
            Ok(Some(job))
        }
        (None, false) => {
            let job = uuid::Uuid::new_v4().to_string();
            let note = confirm_note(&moves);
            c.prepare_cached(
                "INSERT INTO subtitle_jobs
                     (id, command_id, request, origin, work_id, season, anime_no, source_id,
                      creator, state, wait, stage, note, created_at, updated_at, state_at)
                 SELECT ?1, 'relocate:' || ?1,
                        json_object('work_id', ?2, 'season', ?3, 'source_id', ?4), ?5, ?2, ?3,
                        s.anime_no, s.id, s.creator_name, 'waiting', 'placement', 'placement',
                        ?6, ?7, ?7, ?7
                   FROM subtitle_sources s WHERE s.id = ?4",
            )?
            .execute(params![
                job, work_id, season, source_id, RELOCATE, note, now
            ])?;
            if plan(c, &job, &moves, now)? == 0 {
                c.prepare_cached("DELETE FROM subtitle_jobs WHERE id = ?1")?
                    .execute([&job])?;
                return Ok(None);
            }
            event(
                c,
                &job,
                "회차 대응이 바뀌어 적용본을 옮길 계획을 만들었어요",
                Some(&waiting_note(c, &job)?),
                now,
            )?;
            Ok(Some(job))
        }
    }
}

/// Ends the waiting relocation job: no copy is left to move by it.
fn nothing_to_move(c: &Connection, job: &str, now: Millis) -> rusqlite::Result<()> {
    c.prepare_cached("DELETE FROM subtitle_job_plan WHERE job_id = ?1")?
        .execute([job])?;
    c.prepare_cached("DELETE FROM subtitle_relocations WHERE job_id = ?1")?
        .execute([job])?;
    c.prepare_cached(
        "UPDATE subtitle_jobs
            SET state = 'done', wait = NULL, stage = NULL, note = ?2, finished_at = ?3,
                state_at = ?3, updated_at = ?3
          WHERE id = ?1",
    )?
    .execute(params![job, NOTHING_TO_MOVE, now])?;
    c.prepare_cached(
        "UPDATE subtitle_job_steps SET state = 'done', at = ?2, note = ?3
          WHERE job_id = ?1 AND step = 'placement'",
    )?
    .execute(params![job, now, NOTHING_TO_MOVE])?;
    event(c, job, NOTHING_TO_MOVE, None, now)
}

/// Writes `moves` as the job's plan and removals, in place of what it
/// planned (none of which was confirmed, so none of it was done). Rows are
/// numbered after the ones they replace, so a screen's placing of an earlier
/// plan names none of them.
///
/// Each row moves what is applied, so it is applied as a person's choice
/// (`chosen`): a newer revision of its source does not keep it stored only.
/// A copy that was added beside another format (`add`) is added on the new
/// episode too, after the rows that put a first copy there. Returns how many
/// moves it wrote: one whose stored subtitle no row is left to copy is not.
fn plan(c: &Connection, job: &str, moves: &[Move], now: Millis) -> rusqlite::Result<usize> {
    let first: i64 = c
        .prepare_cached(
            "SELECT coalesce(max(position) + 1, 0) FROM subtitle_job_plan WHERE job_id = ?1",
        )?
        .query_row([job], |r| r.get(0))?;
    let mut next = first;
    let mut written = 0;
    let mut ordered: Vec<(&Move, bool)> = Vec::with_capacity(moves.len());
    for m in moves {
        let mut added = false;
        for (applied, _, _) in &m.off {
            added |= c
                .prepare_cached(
                    "SELECT EXISTS (SELECT 1 FROM subtitle_job_plan
                                 WHERE applied_id = ?1 AND chosen = 'add')",
                )?
                .query_row([applied], |r| r.get::<_, bool>(0))?;
        }
        ordered.push((m, added));
    }
    ordered.sort_by_key(|(_, added)| *added);
    // The earlier rows go once the new ones are in: a stored subtitle only
    // they name is still found to copy.
    c.prepare_cached("DELETE FROM subtitle_relocations WHERE job_id = ?1")?
        .execute([job])?;
    for (m, added) in ordered {
        let position = match m.to {
            None => None,
            Some(to) => {
                // The row is the stored subtitle's, as a job that received
                // or moved it planned it, on its new episode.
                let inserted = c
                    .prepare_cached(
                        "INSERT INTO subtitle_job_plan
                         (job_id, position, file_id, member, name, kind, format, size, sha256,
                          anissia_episode, attachment_episode, episode, assignment, basis,
                          action, stored_id, chosen, updated_at)
                     SELECT ?1, ?2, p.file_id, p.member, p.name, 'subtitle', s.format,
                            a.byte_size, a.sha256, s.anissia_episode, s.attachment_episode, ?4,
                            'mapped', s.basis, 'apply', s.id,
                            CASE WHEN ?6 THEN 'add' ELSE 'apply' END, ?5
                       FROM subtitle_stored s
                       JOIN subtitle_assets a ON a.id = s.subtitle_asset_id
                       JOIN subtitle_job_plan p ON p.stored_id = s.id
                      WHERE s.id = ?3
                      ORDER BY p.updated_at, p.job_id, p.position LIMIT 1",
                    )?
                    .execute(params![job, next, m.stored_id, to, now, added])?;
                // [`wanted`] moves only what a row names. Should none be
                // left, the copies stay where they are rather than be taken
                // off with nothing to apply.
                if inserted == 0 {
                    continue;
                }
                next += 1;
                Some(next - 1)
            }
        };
        written += 1;
        for (applied, episode, path) in &m.off {
            c.prepare_cached(
                "INSERT INTO subtitle_relocations
                     (id, job_id, position, applied_id, episode, path, state, created_at,
                      updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'planned', ?7, ?7)",
            )?
            .execute(params![
                uuid::Uuid::new_v4().to_string(),
                job,
                position,
                applied,
                episode,
                path,
                now
            ])?;
        }
    }
    c.prepare_cached("DELETE FROM subtitle_job_plan WHERE job_id = ?1 AND position < ?2")?
        .execute(params![job, first])?;
    let note = confirm_note(&planned(c, job)?);
    c.prepare_cached("UPDATE subtitle_jobs SET note = ?2, updated_at = ?3 WHERE id = ?1")?
        .execute(params![job, note, now])?;
    c.prepare_cached(
        "INSERT INTO subtitle_job_steps (job_id, step, state, at, note)
         VALUES (?1, 'placement', 'waiting', ?2, ?3)
         ON CONFLICT (job_id, step) DO UPDATE
         SET state = 'waiting', at = excluded.at, note = excluded.note",
    )?
    .execute(params![job, now, note])?;
    Ok(written)
}

// ---------------------------------------------------------------------------
// The records of removals

/// One removal of a relocation job (see the schema's comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub id: String,
    pub job_id: String,
    /// The row that applies the same stored subtitle on its new episode.
    pub position: Option<i64>,
    pub applied_id: String,
    pub episode: i64,
    pub path: String,
    pub state: RemovalState,
    pub folder: Option<String>,
    pub aside: Option<String>,
    pub reason: Option<String>,
}

/// The job's removals, by the episode they take a copy off.
pub fn removals(c: &Connection, job_id: &str) -> rusqlite::Result<Vec<Removal>> {
    let mut stmt = c.prepare_cached(
        "SELECT id, job_id, position, applied_id, episode, path, state, folder, aside, reason
           FROM subtitle_relocations WHERE job_id = ?1 ORDER BY episode, path, id",
    )?;
    let rows = stmt.query_map([job_id], |r| {
        Ok(Removal {
            id: r.get(0)?,
            job_id: r.get(1)?,
            position: r.get(2)?,
            applied_id: r.get(3)?,
            episode: r.get(4)?,
            path: r.get(5)?,
            state: r.get(6)?,
            folder: r.get(7)?,
            aside: r.get(8)?,
            reason: r.get(9)?,
        })
    })?;
    rows.collect()
}

/// What an applied copy's record says of its file.
#[derive(Clone)]
struct Applied {
    size: u64,
    sha256: String,
    removed: bool,
    /// Its stored subtitle is on the copy's episode again.
    back: bool,
}

fn applied_of(c: &Connection, applied_id: &str) -> rusqlite::Result<Option<Applied>> {
    c.prepare_cached(
        "SELECT a.byte_size, a.sha256, a.removed_at IS NOT NULL, s.episode IS a.episode
           FROM subtitle_applied a JOIN subtitle_stored s ON s.id = a.stored_id
          WHERE a.id = ?1",
    )?
    .query_row([applied_id], |r| {
        Ok(Applied {
            size: r.get::<_, i64>(0)? as u64,
            sha256: r.get(1)?,
            removed: r.get(2)?,
            back: r.get(3)?,
        })
    })
    .optional()
}

fn set_state(
    c: &mut Connection,
    id: &str,
    state: RemovalState,
    reason: Option<&str>,
    now: Millis,
) -> Result<(), JobError> {
    durable(c, |c| {
        c.prepare_cached(
            "UPDATE subtitle_relocations SET state = ?2, reason = ?3, updated_at = ?4
              WHERE id = ?1",
        )?
        .execute(params![id, state, reason, now])?;
        Ok(())
    })
}

/// Marks the removal `intended`, unless its copy is recorded removed or its
/// stored subtitle is on the copy's episode now: a mapping change committed
/// after the looks before it is seen in the statement that marks it.
/// Whether it was marked.
fn intend(
    c: &mut Connection,
    id: &str,
    folder: &str,
    aside: &str,
    now: Millis,
) -> Result<bool, JobError> {
    durable(c, |c| {
        let marked = c
            .prepare_cached(
                "UPDATE subtitle_relocations
                SET state = 'intended', folder = ?2, aside = ?3, reason = NULL, updated_at = ?4
              WHERE id = ?1 AND state IN ('planned', 'intended')
                AND EXISTS (
                    SELECT 1 FROM subtitle_applied a JOIN subtitle_stored s ON s.id = a.stored_id
                     WHERE a.id = subtitle_relocations.applied_id AND a.removed_at IS NULL
                       AND s.episode IS NOT a.episode)",
            )?
            .execute(params![id, folder, aside, now])?;
        Ok(marked == 1)
    })
}

/// The removal is done, and its applied copy recorded removed (unless it
/// was already), in one synced transaction.
fn done(c: &mut Connection, removal: &Removal, now: Millis) -> Result<(), JobError> {
    durable(c, |c| {
        let tx = c.transaction()?;
        tx.prepare_cached(
            "UPDATE subtitle_relocations SET state = 'done', reason = NULL, updated_at = ?2
              WHERE id = ?1",
        )?
        .execute(params![removal.id, now])?;
        tx.prepare_cached(
            "UPDATE subtitle_applied SET removed_at = ?2 WHERE id = ?1 AND removed_at IS NULL",
        )?
        .execute(params![removal.applied_id, now])?;
        tx.commit()?;
        Ok(())
    })
}

/// What a look at a removal's files found.
enum Found {
    /// The applied bytes on its path. The file's object is not compared: a
    /// file system mounted again may give the same file another device
    /// number, and a file of the applied bytes loses nothing when removed,
    /// since its stored subtitle keeps them.
    Copy,
    /// Nothing on its path.
    Gone,
    /// Another file there, or one that could not be read.
    Other(String),
}

fn found(path: &Path, copy: &Applied) -> Found {
    match files::facts(path) {
        Ok(None) => Found::Gone,
        Ok(Some((size, sha, _))) if size == copy.size && sha == copy.sha256 => Found::Copy,
        Ok(Some(_)) => Found::Other("적용한 뒤 바뀐 파일이라 그대로 뒀어요".to_owned()),
        Err(err) => Found::Other(format!("적용본을 확인하지 못해 그대로 뒀어요: {err}")),
    }
}

// ---------------------------------------------------------------------------
// Taking the copies off

/// Why a row whose own removal is held is not applied.
pub const OLD_COPY_HELD: &str = "옛 회차의 적용본을 지우지 못해 적용하지 않았어요";

/// Why a row on the episode of a held removal's copy is not applied.
pub const COPY_HERE_HELD: &str = "이 회차의 적용본을 지우지 못해 적용하지 않았어요";

/// Why the job's first held removal is held, if one is.
pub(super) fn held_reason(c: &Connection, job: &str) -> rusqlite::Result<Option<String>> {
    c.prepare_cached(
        "SELECT reason FROM subtitle_relocations WHERE job_id = ?1 AND state = 'held'
          ORDER BY updated_at, id LIMIT 1",
    )?
    .query_row([job], |r| r.get(0))
    .optional()
}

/// Holds the job's unapplied rows a held removal touches (see the module
/// docs). Returns how many it held.
pub(super) fn hold_rows(c: &mut Connection, job: &str, now: Millis) -> Result<usize, JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut held = 0;
        for (note, touched) in [
            (OLD_COPY_HELD, "p.position = r.position"),
            (COPY_HERE_HELD, "p.episode = r.episode"),
        ] {
            held += tx
                .prepare_cached(&format!(
                    "UPDATE subtitle_job_plan AS p SET outcome = 'held', note = ?2, updated_at = ?3
                      WHERE p.job_id = ?1 AND (p.outcome IS NULL OR p.outcome = 'no_video')
                        AND EXISTS (SELECT 1 FROM subtitle_relocations r
                                     WHERE r.job_id = ?1 AND r.state = 'held' AND {touched})"
                ))?
                .execute(params![job, note, now])?;
        }
        tx.commit()?;
        Ok(held)
    })
}

/// Why a removal keeps a copy whose stored subtitle's file is not there as
/// recorded: the copy may be the last of its bytes.
pub const STORED_MISSING: &str = "보관본 파일이 없거나 기록과 달라 적용본을 그대로 뒀어요";

/// The asset of the applied copy's stored subtitle; none when it was cleaned.
fn stored_of(c: &Connection, applied_id: &str) -> rusqlite::Result<Option<records::Asset>> {
    let stored: Option<String> = c
        .prepare_cached("SELECT stored_id FROM subtitle_applied WHERE id = ?1")?
        .query_row([applied_id], |r| r.get(0))
        .optional()?;
    match stored {
        Some(id) => records::stored_asset(c, &id),
        None => Ok(None),
    }
}

/// Why a removal keeps a copy whose stored subtitle is on its episode again.
const BACK_HERE: &str = "회차 대응이 다시 바뀌어 보관본이 이 회차의 것이라 그대로 뒀어요";

/// What came of renaming a copy aside.
enum Aside {
    /// It is aside, the applied bytes.
    Done,
    /// Nothing was left changed: the removal ends `kept` (or `done` when the
    /// copy is gone), with why.
    Kept(String),
    Gone,
    /// What became of it is not known.
    Unsure(String),
}

impl Placer {
    /// Takes off the copies the job's confirmed relocation moves, before any
    /// of its rows is applied, and holds the rows a held removal touches (see
    /// the module docs).
    pub(super) async fn relocate(&self, job: &str, folder: &str) -> Result<(), JobError> {
        let id = job.to_owned();
        let all = self.read(move |c| removals(c, &id)).await?;
        let mut any = false;
        for removal in all {
            let state = removal.state;
            if !matches!(
                state,
                RemovalState::Planned | RemovalState::Intended | RemovalState::SetAside
            ) {
                continue;
            }
            if !any {
                self.begin(job, StepKind::Apply).await?;
                any = true;
            }
            self.take_off(folder, removal).await?;
        }
        let id = job.to_owned();
        if self.read(move |c| held_reason(c, &id)).await?.is_some() {
            let (id, now) = (job.to_owned(), self.now());
            self.write(move |c| hold_rows(c, &id, now)).await?;
        }
        Ok(())
    }

    async fn take_off(&self, folder: &str, removal: Removal) -> Result<(), JobError> {
        let applied_id = removal.applied_id.clone();
        let Some(copy) = self.read(move |c| applied_of(c, &applied_id)).await? else {
            return self
                .end(
                    &removal,
                    RemovalState::Held,
                    "적용본의 기록을 찾지 못했어요",
                )
                .await;
        };
        // Where an earlier start left it.
        let (folder, aside) = match (removal.state, &removal.folder, &removal.aside) {
            (RemovalState::Intended | RemovalState::SetAside, Some(f), Some(a)) => {
                (f.clone(), a.clone())
            }
            _ => (
                folder.to_owned(),
                format!("{}/{}.aside", files::TEMP_DIR, removal.id),
            ),
        };
        let base = Path::new(&folder).to_path_buf();
        let at = files::within(&base, &removal.path);
        let aside_at = files::within(&base, &aside);
        let label = format!("{}화 적용본 {}", removal.episode, removal.path);

        if removal.state == RemovalState::SetAside {
            return self.remove_aside(&removal, &aside_at, &label).await;
        }
        if removal.state == RemovalState::Intended {
            let a = aside_at.clone();
            if blocking(move || files::occupied(&a)).await.unwrap_or(true) {
                // Renamed before the record that it was found aside.
                return match self.check_aside(&copy, &at, &aside_at, &label).await {
                    Aside::Done => {
                        self.mark_aside(&removal).await?;
                        self.remove_aside(&removal, &aside_at, &label).await
                    }
                    Aside::Kept(why) => self.end(&removal, RemovalState::Kept, &why).await,
                    Aside::Gone => self.gone(&removal, &label).await,
                    Aside::Unsure(why) => self.end(&removal, RemovalState::Held, &why).await,
                };
            }
            // Not renamed: looked at anew below.
        }

        if copy.removed {
            return self.gone(&removal, &label).await;
        }
        if copy.back {
            return self.end(&removal, RemovalState::Kept, BACK_HERE).await;
        }
        let (f, lower, own) = (
            folder.clone(),
            removal.path.to_lowercase(),
            removal.id.clone(),
        );
        if self
            .read(move |c| records::busy_targets(c, &f, Some(&own)))
            .await?
            .contains(&lower)
        {
            return self
                .end(
                    &removal,
                    RemovalState::Kept,
                    "다른 작업이 이 경로의 파일을 바꾸는 중이라 그대로 뒀어요",
                )
                .await;
        }
        // The stored subtitle keeps the bytes taken off: its file must be
        // there as recorded, in this folder (an empty mount point has none).
        let applied_id = removal.applied_id.clone();
        let stored = self.read(move |c| stored_of(c, &applied_id)).await?;
        let (b, kept) = (base.clone(), stored);
        let intact = blocking(move || {
            kept.is_some_and(|asset| {
                matches!(files::facts(&files::within(&b, &asset.relative_path)),
                    Ok(Some((size, sha, _))) if size == asset.size && sha == asset.sha256)
            })
        })
        .await;
        if !intact {
            return self.end(&removal, RemovalState::Kept, STORED_MISSING).await;
        }
        let a = at.clone();
        let seen = blocking(move || found(&a, &copy)).await;
        match seen {
            Found::Gone => return self.gone(&removal, &label).await,
            Found::Other(why) => return self.end(&removal, RemovalState::Kept, &why).await,
            Found::Copy => {}
        }
        let (id, f, a, now) = (
            removal.id.clone(),
            folder.clone(),
            aside.clone(),
            self.now(),
        );
        let marked = self.write(move |c| intend(c, &id, &f, &a, now)).await?;
        let applied_id = removal.applied_id.clone();
        let Some(copy) = self.read(move |c| applied_of(c, &applied_id)).await? else {
            return self
                .end(
                    &removal,
                    RemovalState::Held,
                    "적용본의 기록을 찾지 못했어요",
                )
                .await;
        };
        if !marked {
            return match (copy.removed, copy.back) {
                (true, _) => self.gone(&removal, &label).await,
                (false, true) => self.end(&removal, RemovalState::Kept, BACK_HERE).await,
                // The mapping moved away and back while it looked: the copy
                // stays, and a later relocation may plan it again.
                (false, false) => {
                    let why = "회차 대응이 바뀌는 사이라 그대로 뒀어요";
                    self.end(&removal, RemovalState::Kept, why).await
                }
            };
        }
        let (from, to) = (at.clone(), aside_at.clone());
        let renamed = blocking(move || -> std::io::Result<()> {
            if let Some(dir) = to.parent() {
                std::fs::create_dir_all(dir)?;
            }
            rename_noreplace(&from, &to)?;
            for dir in [from.parent(), to.parent()].into_iter().flatten() {
                sync_dir(dir)?;
            }
            Ok(())
        })
        .await;
        match renamed {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return self.gone(&removal, &label).await;
            }
            Err(err) => {
                // Either nothing moved, or the move is not known to be on
                // disk: the next run looks at both paths again.
                let a = aside_at.clone();
                if blocking(move || files::occupied(&a)).await.unwrap_or(true) {
                    return self
                        .end(
                            &removal,
                            RemovalState::Held,
                            &format!("{label}을 옮긴 뒤 폴더를 동기화하지 못했어요: {err}"),
                        )
                        .await;
                }
                return self
                    .end(
                        &removal,
                        RemovalState::Kept,
                        &format!("적용본을 옮기지 못해 그대로 뒀어요: {err}"),
                    )
                    .await;
            }
        }
        match self.check_aside(&copy, &at, &aside_at, &label).await {
            Aside::Done => {
                self.mark_aside(&removal).await?;
                self.remove_aside(&removal, &aside_at, &label).await
            }
            Aside::Kept(why) => self.end(&removal, RemovalState::Kept, &why).await,
            Aside::Gone => self.gone(&removal, &label).await,
            Aside::Unsure(why) => self.end(&removal, RemovalState::Held, &why).await,
        }
    }

    /// Whether the file renamed aside is the applied copy; one that is not
    /// goes back on its path.
    async fn check_aside(&self, copy: &Applied, at: &Path, aside: &Path, label: &str) -> Aside {
        let copy = copy.clone();
        let (at, aside, label) = (at.to_path_buf(), aside.to_path_buf(), label.to_owned());
        blocking(move || match found(&aside, &copy) {
            Found::Copy => Aside::Done,
            Found::Gone => Aside::Gone,
            Found::Other(_) => match rename_noreplace(&aside, &at) {
                Ok(()) => {
                    for dir in [at.parent(), aside.parent()].into_iter().flatten() {
                        if let Err(err) = sync_dir(dir) {
                            return Aside::Unsure(format!(
                                "{label}을 되돌린 뒤 폴더를 동기화하지 못했어요: {err}"
                            ));
                        }
                    }
                    Aside::Kept("적용한 뒤 바뀐 파일이라 그대로 뒀어요".to_owned())
                }
                Err(err) => Aside::Unsure(format!(
                    "옮긴 {label}이 적용한 파일과 달라 되돌리려 했지만 하지 못했어요: {err}"
                )),
            },
        })
        .await
    }

    async fn mark_aside(&self, removal: &Removal) -> Result<(), JobError> {
        let (id, now) = (removal.id.clone(), self.now());
        self.write(move |c| set_state(c, &id, RemovalState::SetAside, None, now))
            .await
    }

    /// Removes the copy set aside; the removal is done.
    async fn remove_aside(
        &self,
        removal: &Removal,
        aside: &Path,
        label: &str,
    ) -> Result<(), JobError> {
        let a = aside.to_path_buf();
        let removed = blocking(move || -> std::io::Result<()> {
            files::remove_known(&a)?;
            match a.parent() {
                Some(dir) if dir.is_dir() => sync_dir(dir),
                _ => Ok(()),
            }
        })
        .await;
        if let Err(err) = removed {
            return self
                .end(
                    removal,
                    RemovalState::Held,
                    &format!("{label}을 지우지 못했어요: {err}"),
                )
                .await;
        }
        let (r, now) = (removal.clone(), self.now());
        self.write(move |c| done(c, &r, now)).await?;
        self.event(&removal.job_id, format!("{label}을 지웠어요"), None)
            .await
    }

    /// The copy is not on the disk any more: a person removed it.
    async fn gone(&self, removal: &Removal, label: &str) -> Result<(), JobError> {
        let (r, now) = (removal.clone(), self.now());
        self.write(move |c| done(c, &r, now)).await?;
        self.event(&removal.job_id, format!("{label}은 이미 없어요"), None)
            .await
    }

    async fn end(&self, removal: &Removal, state: RemovalState, why: &str) -> Result<(), JobError> {
        let (id, reason, now) = (removal.id.clone(), why.to_owned(), self.now());
        self.write(move |c| set_state(c, &id, state, Some(&reason), now))
            .await?;
        self.event(
            &removal.job_id,
            format!("{}화 적용본 {}: {why}", removal.episode, removal.path),
            None,
        )
        .await
    }
}

/// The note of a relocation job that waits for its confirmation, from what
/// it plans.
pub fn waiting_note(c: &Connection, job_id: &str) -> rusqlite::Result<String> {
    Ok(confirm_note(&planned(c, job_id)?))
}

/// The live applied copy of the stored subtitle on `episode`, if it has one.
pub fn applied_on(
    c: &Connection,
    stored_id: &str,
    episode: i64,
) -> rusqlite::Result<Option<String>> {
    c.prepare_cached(
        "SELECT id FROM subtitle_applied
          WHERE stored_id = ?1 AND episode = ?2 AND removed_at IS NULL
          ORDER BY applied_at LIMIT 1",
    )?
    .query_row(params![stored_id, episode], |r| r.get(0))
    .optional()
}

/// Settles a relocation's row whose stored subtitle is applied on its
/// episode already (a later mapping change put it back): `applied`, with
/// that copy.
pub fn already_applied(
    c: &mut Connection,
    job_id: &str,
    position: i64,
    applied_id: &str,
    now: Millis,
) -> Result<(), JobError> {
    durable(c, |c| {
        c.prepare_cached(
            "UPDATE subtitle_job_plan
                SET outcome = 'applied', applied_id = ?3, note = ?4, updated_at = ?5
              WHERE job_id = ?1 AND position = ?2",
        )?
        .execute(params![job_id, position, applied_id, ALREADY_APPLIED, now])?;
        Ok(())
    })
}

/// The note of a relocation's row whose stored subtitle is applied on its
/// episode already.
pub const ALREADY_APPLIED: &str = "이미 이 회차에 적용돼 있어요";

/// Marks the job to go back in line once its run ends (`remapped_at`): what
/// a row of it was to be applied by no longer holds.
pub fn look_again(c: &mut Connection, job_id: &str, now: Millis) -> Result<(), JobError> {
    c.prepare_cached("UPDATE subtitle_jobs SET remapped_at = ?2 WHERE id = ?1")?
        .execute(params![job_id, now])?;
    Ok(())
}

/// What a relocation job came to, for its last note: how many copies it took
/// off and how many it left where they are, and whether it applied any on
/// the new episodes (one whose new episodes had them already only took off).
pub fn outcome_note(c: &Connection, job_id: &str) -> rusqlite::Result<Option<String>> {
    let (done, kept, applied): (i64, i64, bool) = c
        .prepare_cached(
            "SELECT count(*) FILTER (WHERE state = 'done'), count(*) FILTER (WHERE state = 'kept'),
                EXISTS (SELECT 1 FROM subtitle_job_plan
                         WHERE job_id = ?1 AND outcome = 'applied' AND note IS NOT ?2)
           FROM subtitle_relocations WHERE job_id = ?1",
        )?
        .query_row(params![job_id, ALREADY_APPLIED], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    Ok(match (done, kept) {
        (0, 0) => None,
        (done, 0) if applied => Some(format!(
            "옛 회차의 적용본 {done}개를 지우고 새 회차에 적용했어요"
        )),
        (done, 0) => Some(format!("옛 회차의 적용본 {done}개를 지웠어요")),
        (0, kept) => Some(format!("옛 회차의 적용본 {kept}개를 그대로 뒀어요")),
        (done, kept) => Some(format!(
            "옛 회차의 적용본 {done}개를 지우고 {kept}개는 그대로 뒀어요"
        )),
    })
}
