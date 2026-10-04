//! The records of storing and applying (`migrations/jobs/store_apply.sql` in
//! `trss-core`). The writes that follow a file effect are synced
//! ([`durable`]), so the record of an effect's intent is on disk before the
//! effect.

use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use trss_core::Millis;

use crate::{
    model::{AssetKind, EffectKind, EffectState, Outcome, PlanAction, SubtitleFormat},
    place::episode::{Assignment, Basis},
    store::JobError,
};

/// Runs `write` with every commit synced to disk (`synchronous = FULL`).
pub(crate) fn durable<T>(
    c: &mut Connection,
    write: impl FnOnce(&mut Connection) -> Result<T, JobError>,
) -> Result<T, JobError> {
    c.pragma_update(None, "synchronous", "FULL")?;
    let written = write(c);
    c.pragma_update(None, "synchronous", "NORMAL")?;
    written
}

/// The job as the placement reads it.
#[derive(Debug, Clone)]
pub struct JobFacts {
    pub origin: String,
    pub work_id: Option<String>,
    pub season: Option<u32>,
    pub source_id: Option<String>,
    pub creator: Option<String>,
}

pub fn job_facts(c: &Connection, job_id: &str) -> rusqlite::Result<Option<JobFacts>> {
    c.query_row(
        "SELECT origin, work_id, season, source_id, creator FROM subtitle_jobs WHERE id = ?1",
        [job_id],
        |r| {
            Ok(JobFacts {
                origin: r.get(0)?,
                work_id: r.get(1)?,
                season: r.get(2)?,
                source_id: r.get(3)?,
                creator: r.get(4)?,
            })
        },
    )
    .optional()
}

/// Where a row of a plan goes: the episode and what puts it there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub episode: i64,
    pub assignment: Assignment,
    pub basis: Option<Basis>,
}

/// One row of a job's placement plan (see the schema's comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRow {
    pub job_id: String,
    pub position: i64,
    pub file_id: String,
    pub member: Option<String>,
    pub name: String,
    pub kind: AssetKind,
    pub format: Option<SubtitleFormat>,
    pub size: u64,
    pub sha256: String,
    pub item_id: Option<i64>,
    pub anissia_episode: Option<String>,
    pub attachment_episode: Option<String>,
    pub placed: Option<Placed>,
    pub action: PlanAction,
    pub question: Option<String>,
    pub stored_id: Option<String>,
    pub outcome: Option<Outcome>,
    pub note: Option<String>,
    pub applied_id: Option<String>,
    /// The asset a font, attachment or companion row was kept as.
    pub asset_id: Option<String>,
}

impl PlanRow {
    /// Whether the row's file is kept: stored as a subtitle or as an asset.
    pub fn kept(&self) -> bool {
        self.stored_id.is_some() || self.asset_id.is_some()
    }
}

const PLAN_COLUMNS: &str = "job_id, position, file_id, member, name, kind, format, size, sha256,
     item_id, anissia_episode, attachment_episode, episode, assignment, basis, action, question,
     stored_id, outcome, note, applied_id, asset_id";

fn plan_row(r: &Row<'_>) -> rusqlite::Result<PlanRow> {
    let episode: Option<i64> = r.get(12)?;
    let assignment: Option<String> = r.get(13)?;
    let basis: Option<String> = r.get(14)?;
    Ok(PlanRow {
        job_id: r.get(0)?,
        position: r.get(1)?,
        file_id: r.get(2)?,
        member: r.get(3)?,
        name: r.get(4)?,
        kind: r.get(5)?,
        format: r.get(6)?,
        size: r.get::<_, i64>(7)? as u64,
        sha256: r.get(8)?,
        item_id: r.get(9)?,
        anissia_episode: r.get(10)?,
        attachment_episode: r.get(11)?,
        placed: match (episode, assignment.as_deref().and_then(Assignment::parse)) {
            (Some(episode), Some(assignment)) => Some(Placed {
                episode,
                assignment,
                basis: basis.as_deref().and_then(Basis::parse),
            }),
            _ => None,
        },
        action: r.get(15)?,
        question: r.get(16)?,
        stored_id: r.get(17)?,
        outcome: r.get(18)?,
        note: r.get(19)?,
        applied_id: r.get(20)?,
        asset_id: r.get(21)?,
    })
}

pub fn plan(c: &Connection, job_id: &str) -> rusqlite::Result<Vec<PlanRow>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {PLAN_COLUMNS} FROM subtitle_job_plan WHERE job_id = ?1 ORDER BY position"
    ))?;
    let rows = stmt.query_map([job_id], plan_row)?;
    rows.collect()
}

pub fn plan_row_at(
    c: &Connection,
    job_id: &str,
    position: i64,
) -> rusqlite::Result<Option<PlanRow>> {
    c.query_row(
        &format!(
            "SELECT {PLAN_COLUMNS} FROM subtitle_job_plan WHERE job_id = ?1 AND position = ?2"
        ),
        params![job_id, position],
        plan_row,
    )
    .optional()
}

/// Adds rows to the job's plan after its last, numbering them; returns them
/// as numbered.
pub fn add_plan(
    c: &mut Connection,
    job_id: &str,
    rows: Vec<PlanRow>,
    now: Millis,
) -> Result<Vec<PlanRow>, JobError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let next: i64 = tx.query_row(
        "SELECT coalesce(max(position) + 1, 0) FROM subtitle_job_plan WHERE job_id = ?1",
        [job_id],
        |r| r.get(0),
    )?;
    let mut added = Vec::with_capacity(rows.len());
    for (offset, mut row) in rows.into_iter().enumerate() {
        row.job_id = job_id.to_owned();
        row.position = next + offset as i64;
        tx.execute(
            &format!(
                "INSERT INTO subtitle_job_plan ({PLAN_COLUMNS}, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                         ?17, ?18, ?19, ?20, ?21, ?22, ?23)"
            ),
            params![
                row.job_id,
                row.position,
                row.file_id,
                row.member,
                row.name,
                row.kind,
                row.format,
                row.size as i64,
                row.sha256,
                row.item_id,
                row.anissia_episode,
                row.attachment_episode,
                row.placed.as_ref().map(|p| p.episode),
                row.placed.as_ref().map(|p| p.assignment.code()),
                row.placed.as_ref().and_then(|p| p.basis).map(Basis::code),
                row.action,
                row.question,
                row.stored_id,
                row.outcome,
                row.note,
                row.applied_id,
                row.asset_id,
                now,
            ],
        )?;
        added.push(row);
    }
    tx.commit()?;
    Ok(added)
}

/// Writes what came of a row.
pub fn set_outcome(
    c: &mut Connection,
    job_id: &str,
    position: i64,
    outcome: Outcome,
    note: Option<String>,
    now: Millis,
) -> Result<(), JobError> {
    durable(c, |c| {
        c.execute(
            "UPDATE subtitle_job_plan SET outcome = ?3, note = ?4, updated_at = ?5
             WHERE job_id = ?1 AND position = ?2",
            params![job_id, position, outcome, note, now],
        )?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Effects

/// One effect on a work folder's file (see the schema's comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    pub id: String,
    pub job_id: String,
    pub position: i64,
    pub kind: EffectKind,
    pub state: EffectState,
    pub folder: String,
    pub temp: String,
    pub target: String,
    pub video: Option<String>,
    pub size: u64,
    pub sha256: String,
    pub object: Option<String>,
    pub reason: Option<String>,
}

const EFFECT_COLUMNS: &str =
    "id, job_id, position, kind, state, folder, temp, target, video, size, sha256, object, reason";

fn effect(r: &Row<'_>) -> rusqlite::Result<Effect> {
    Ok(Effect {
        id: r.get(0)?,
        job_id: r.get(1)?,
        position: r.get(2)?,
        kind: r.get(3)?,
        state: r.get(4)?,
        folder: r.get(5)?,
        temp: r.get(6)?,
        target: r.get(7)?,
        video: r.get(8)?,
        size: r.get::<_, i64>(9)? as u64,
        sha256: r.get(10)?,
        object: r.get(11)?,
        reason: r.get(12)?,
    })
}

/// The job's effects that did not end: `intended` or `prepared`.
pub fn unfinished_effects(c: &Connection, job_id: &str) -> rusqlite::Result<Vec<Effect>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {EFFECT_COLUMNS} FROM subtitle_file_effects
          WHERE job_id = ?1 AND state IN ('intended', 'prepared')
          ORDER BY created_at, id"
    ))?;
    let rows = stmt.query_map([job_id], effect)?;
    rows.collect()
}

/// Records an effect's intent, before anything is written.
pub fn intend(c: &mut Connection, e: &Effect, now: Millis) -> Result<(), JobError> {
    durable(c, |c| {
        c.execute(
            &format!(
                "INSERT INTO subtitle_file_effects ({EFFECT_COLUMNS}, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'intended', ?5, ?6, ?7, ?8, ?9, ?10, NULL, NULL, ?11, ?11)"
            ),
            params![
                e.id,
                e.job_id,
                e.position,
                e.kind,
                e.folder,
                e.temp,
                e.target,
                e.video,
                e.size as i64,
                e.sha256,
                now
            ],
        )?;
        Ok(())
    })
}

/// The temporary file of an intended effect is written and checked.
pub fn prepared(c: &mut Connection, id: &str, object: &str, now: Millis) -> Result<(), JobError> {
    durable(c, |c| {
        c.execute(
            "UPDATE subtitle_file_effects SET state = 'prepared', object = ?2, updated_at = ?3
             WHERE id = ?1 AND state = 'intended'",
            params![id, object, now],
        )?;
        Ok(())
    })
}

/// A prepared store's target was taken: it goes to `target` instead.
pub fn retarget(c: &mut Connection, id: &str, target: &str, now: Millis) -> Result<(), JobError> {
    durable(c, |c| {
        c.execute(
            "UPDATE subtitle_file_effects SET target = ?2, updated_at = ?3
             WHERE id = ?1 AND state = 'prepared'",
            params![id, target, now],
        )?;
        Ok(())
    })
}

/// Ends an effect `held`, `failed` or `abandoned`, with the reason.
pub fn end_effect(
    c: &mut Connection,
    id: &str,
    state: EffectState,
    reason: Option<String>,
    now: Millis,
) -> Result<(), JobError> {
    durable(c, |c| {
        c.execute(
            "UPDATE subtitle_file_effects SET state = ?2, reason = ?3, updated_at = ?4
             WHERE id = ?1",
            params![id, state, reason, now],
        )?;
        Ok(())
    })
}

/// Ends the effect and its row `held` together.
pub fn hold(
    c: &mut Connection,
    effect: &Effect,
    reason: &str,
    now: Millis,
) -> Result<(), JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE subtitle_file_effects SET state = 'held', reason = ?2, updated_at = ?3
             WHERE id = ?1",
            params![effect.id, reason, now],
        )?;
        tx.execute(
            "UPDATE subtitle_job_plan SET outcome = 'held', note = ?3, updated_at = ?4
             WHERE job_id = ?1 AND position = ?2",
            params![effect.job_id, effect.position, reason, now],
        )?;
        tx.commit()?;
        Ok(())
    })
}

/// An apply that something on disk overtook before it was published: the
/// effect is abandoned (its temporary file is gone) and its row held for a
/// person, in one synced transaction.
pub fn withdrawn(
    c: &mut Connection,
    effect: &Effect,
    reason: &str,
    now: Millis,
) -> Result<(), JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE subtitle_file_effects SET state = 'abandoned', reason = ?2, updated_at = ?3
             WHERE id = ?1",
            params![effect.id, reason, now],
        )?;
        tx.execute(
            "UPDATE subtitle_job_plan SET outcome = 'held', note = ?3, updated_at = ?4
             WHERE job_id = ?1 AND position = ?2",
            params![effect.job_id, effect.position, reason, now],
        )?;
        tx.commit()?;
        Ok(())
    })
}

/// The targets in `folder`, lower-cased, of the effects under way (of any
/// job) but `except`: names another effect is about to take.
pub fn busy_targets(
    c: &Connection,
    folder: &str,
    except: Option<&str>,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = c.prepare(
        "SELECT lower(target) FROM subtitle_file_effects
          WHERE folder = ?1 AND state IN ('intended', 'prepared') AND id IS NOT ?2",
    )?;
    let rows = stmt.query_map(params![folder, except], |r| r.get::<_, String>(0))?;
    rows.collect()
}

// ---------------------------------------------------------------------------
// Assets, packages, stored subtitles

/// A kept file of a work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub id: String,
    pub kind: AssetKind,
    pub relative_path: String,
    pub size: u64,
    pub sha256: String,
}

/// Where an asset of `kind` is kept: the work folder (`work`) for a subtitle
/// or a font, the app data folder (`app_data`) for anything else.
pub fn base_of(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Subtitle | AssetKind::Font => "work",
        _ => "app_data",
    }
}

/// The work's assets kept in `base` whose path starts with `prefix`,
/// whatever its letters' case.
pub fn assets_under(
    c: &Connection,
    work_id: &str,
    base: &str,
    prefix: &str,
) -> rusqlite::Result<Vec<Asset>> {
    let mut stmt = c.prepare(
        "SELECT id, kind, relative_path, byte_size, sha256 FROM subtitle_assets
          WHERE work_id = ?1 AND base = ?3
            AND lower(substr(relative_path, 1, length(?2))) = lower(?2)",
    )?;
    let rows = stmt.query_map(params![work_id, prefix, base], |r| {
        Ok(Asset {
            id: r.get(0)?,
            kind: r.get(1)?,
            relative_path: r.get(2)?,
            size: r.get::<_, i64>(3)? as u64,
            sha256: r.get(4)?,
        })
    })?;
    rows.collect()
}

pub fn asset(c: &Connection, id: &str) -> rusqlite::Result<Option<Asset>> {
    c.query_row(
        "SELECT id, kind, relative_path, byte_size, sha256 FROM subtitle_assets WHERE id = ?1",
        [id],
        |r| {
            Ok(Asset {
                id: r.get(0)?,
                kind: r.get(1)?,
                relative_path: r.get(2)?,
                size: r.get::<_, i64>(3)? as u64,
                sha256: r.get(4)?,
            })
        },
    )
    .optional()
}

/// What a stored subtitle is made of, besides its asset.
#[derive(Debug, Clone)]
pub struct NewStored {
    pub work_id: String,
    pub season: u32,
    pub job_id: String,
    pub source_kind: &'static str,
    pub source_page: Option<String>,
    pub received_at: Option<Millis>,
    pub source_id: Option<String>,
    pub creator: Option<String>,
    pub encoding: Option<String>,
    pub observation: Option<String>,
}

/// The asset a store makes or takes.
#[derive(Debug, Clone)]
pub enum Kept {
    /// A file this effect published at `relative_path`.
    New {
        effect_id: String,
        relative_path: String,
    },
    /// A file stored before with the same name and bytes.
    Reused(String),
}

/// Records the row's file as stored: its package (made once for the job's
/// post), the asset (new, or the one reused), the package's entry for it,
/// and for a subtitle the stored subtitle (or the one already stored for the
/// same bytes, source and episode) and the row's link to it; a font,
/// attachment or companion row links to its asset and is `stored`. The
/// effect, if one made the file, is `done`. One synced transaction. Returns
/// the stored subtitle's ID, or the asset's for a row that is no subtitle.
/// The stored subtitle's links to the package's other files are made once
/// the package is stored ([`link`]).
pub fn stored(
    c: &mut Connection,
    row: &PlanRow,
    facts: &NewStored,
    kept: Kept,
    now: Millis,
) -> Result<String, JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let package: Option<String> = tx
            .query_row(
                "SELECT id FROM subtitle_packages
                  WHERE job_id = ?1 AND source_page IS ?2",
                params![facts.job_id, facts.source_page],
                |r| r.get(0),
            )
            .optional()?;
        let package = match package {
            Some(id) => id,
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO subtitle_packages
                         (id, work_id, job_id, source_kind, source_page, received_at, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        id,
                        facts.work_id,
                        facts.job_id,
                        facts.source_kind,
                        facts.source_page,
                        facts.received_at,
                        now
                    ],
                )?;
                id
            }
        };
        let asset_id = match &kept {
            Kept::Reused(id) => id.clone(),
            Kept::New {
                effect_id,
                relative_path,
            } => {
                let id = uuid::Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO subtitle_assets
                         (id, work_id, kind, base, relative_path, byte_size, sha256, created_at)
                     VALUES (?1, ?2, ?3, ?8, ?4, ?5, ?6, ?7)",
                    params![
                        id,
                        facts.work_id,
                        row.kind,
                        relative_path,
                        row.size as i64,
                        row.sha256,
                        now,
                        base_of(row.kind),
                    ],
                )?;
                tx.execute(
                    "UPDATE subtitle_file_effects SET state = 'done', target = ?2, updated_at = ?3
                     WHERE id = ?1",
                    params![effect_id, relative_path, now],
                )?;
                id
            }
        };
        let entry: i64 = tx.query_row(
            "SELECT coalesce(max(position) + 1, 0) FROM subtitle_package_entries
              WHERE package_id = ?1",
            [&package],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO subtitle_package_entries (package_id, position, asset_id, original_name)
             VALUES (?1, ?2, ?3, ?4)",
            params![package, entry, asset_id, row.name],
        )?;
        if row.kind != AssetKind::Subtitle {
            tx.execute(
                "UPDATE subtitle_job_plan SET asset_id = ?3, outcome = 'stored', updated_at = ?4
                  WHERE job_id = ?1 AND position = ?2",
                params![row.job_id, row.position, asset_id, now],
            )?;
            tx.commit()?;
            return Ok(asset_id);
        }
        let placed = row.placed.as_ref();
        // The same bytes of the same source on the same episode are one
        // revision, however often received.
        let same: Option<String> = tx
            .query_row(
                "SELECT id FROM subtitle_stored
                  WHERE subtitle_asset_id = ?1 AND work_id = ?2 AND season = ?3
                    AND source_id IS ?4 AND episode IS ?5 AND assignment IS ?6",
                params![
                    asset_id,
                    facts.work_id,
                    facts.season,
                    facts.source_id,
                    placed.map(|p| p.episode),
                    placed.map(|p| p.assignment.code()),
                ],
                |r| r.get(0),
            )
            .optional()?;
        let stored = match same {
            Some(id) => id,
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO subtitle_stored
                         (id, work_id, season, package_id, subtitle_asset_id, source_id,
                          anissia_episode, attachment_episode, assignment, basis, episode,
                          format, encoding, creator, links_known, anissia_observation, job_id,
                          stored_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 0, ?15,
                             ?16, ?17)",
                    params![
                        id,
                        facts.work_id,
                        facts.season,
                        package,
                        asset_id,
                        facts.source_id,
                        row.anissia_episode,
                        row.attachment_episode,
                        placed.map(|p| p.assignment.code()),
                        placed.and_then(|p| p.basis).map(Basis::code),
                        placed.map(|p| p.episode),
                        row.format.unwrap_or(SubtitleFormat::Other),
                        facts.encoding,
                        facts.creator,
                        facts.observation,
                        facts.job_id,
                        now
                    ],
                )?;
                id
            }
        };
        // A row only stored is settled by its store.
        tx.execute(
            "UPDATE subtitle_job_plan
                SET stored_id = ?3, updated_at = ?4,
                    outcome = CASE action WHEN 'store' THEN 'stored' ELSE outcome END
              WHERE job_id = ?1 AND position = ?2",
            params![row.job_id, row.position, stored, now],
        )?;
        tx.commit()?;
        Ok(stored)
    })
}

/// A stored subtitle's asset, as the apply copies it.
pub fn stored_asset(c: &Connection, stored_id: &str) -> rusqlite::Result<Option<Asset>> {
    let id: Option<String> = c
        .query_row(
            "SELECT subtitle_asset_id FROM subtitle_stored WHERE id = ?1",
            [stored_id],
            |r| r.get(0),
        )
        .optional()?;
    match id {
        Some(id) => asset(c, &id),
        None => Ok(None),
    }
}

/// How a stored subtitle uses another file of its package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Font,
    Attachment,
    Companion,
}

impl Role {
    fn code(self) -> &'static str {
        match self {
            Role::Font => "font",
            Role::Attachment => "attachment",
            Role::Companion => "companion",
        }
    }
}

/// Links each stored subtitle of a stored package to the package's other
/// files (`subtitle_stored_assets`) and marks its links known. One synced
/// transaction; links made already stay as they are.
pub fn link(c: &mut Connection, links: &[(String, Vec<(String, Role)>)]) -> Result<(), JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (stored, assets) in links {
            for (asset, role) in assets {
                tx.execute(
                    "INSERT OR IGNORE INTO subtitle_stored_assets (stored_id, asset_id, role)
                     VALUES (?1, ?2, ?3)",
                    params![stored, asset, role.code()],
                )?;
            }
            tx.execute(
                "UPDATE subtitle_stored SET links_known = 1 WHERE id = ?1",
                [stored],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
}

/// Of the stored subtitles `ids`, those whose links are not known yet.
pub fn links_unknown(c: &Connection, ids: &[String]) -> rusqlite::Result<Vec<String>> {
    let mut stmt = c.prepare("SELECT links_known FROM subtitle_stored WHERE id = ?1")?;
    let mut unknown = Vec::new();
    for id in ids {
        let known: Option<bool> = stmt.query_row([id], |r| r.get(0)).optional()?;
        if known == Some(false) {
            unknown.push(id.clone());
        }
    }
    Ok(unknown)
}

/// The formats in the order the work's first apply takes them: the work's
/// own order, else the common policy's, else ASS, SRT, SMI.
pub fn format_order(c: &Connection, work_id: &str) -> rusqlite::Result<Vec<SubtitleFormat>> {
    let text: String = c.query_row(
        "SELECT coalesce(
                    (SELECT format_order FROM work_subtitle_policy WHERE work_id = ?1),
                    (SELECT format_order FROM policy_settings LIMIT 1),
                    'ass,srt,smi')",
        [work_id],
        |r| r.get(0),
    )?;
    let order: Vec<SubtitleFormat> = text
        .split(',')
        .filter_map(|code| match code.trim() {
            "ass" => Some(SubtitleFormat::Ass),
            "srt" => Some(SubtitleFormat::Srt),
            "smi" => Some(SubtitleFormat::Smi),
            _ => None,
        })
        .collect();
    Ok(match order.is_empty() {
        true => vec![
            SubtitleFormat::Ass,
            SubtitleFormat::Srt,
            SubtitleFormat::Smi,
        ],
        false => order,
    })
}

/// What an applied copy is.
#[derive(Debug, Clone)]
pub struct NewApplied {
    pub work_id: String,
    pub stored_id: String,
    pub season: u32,
    pub episode: i64,
}

/// Records the effect's published copy as applied, and the row as
/// `applied`; the effect is `done`. One synced transaction. Returns the
/// applied copy's ID.
pub fn applied(
    c: &mut Connection,
    effect: &Effect,
    copy: &NewApplied,
    object: &str,
    note: Option<String>,
    now: Millis,
) -> Result<String, JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // The rename replaced nothing, so an earlier copy recorded at this
        // path is gone: a person removed it.
        tx.execute(
            "UPDATE subtitle_applied SET removed_at = ?3
              WHERE work_id = ?1 AND path = ?2 AND removed_at IS NULL",
            params![copy.work_id, effect.target, now],
        )?;
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO subtitle_applied
                 (id, work_id, stored_id, season, episode, video_path, path, byte_size, sha256,
                  object, job_id, applied_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                copy.work_id,
                copy.stored_id,
                copy.season,
                copy.episode,
                effect.video,
                effect.target,
                effect.size as i64,
                effect.sha256,
                object,
                effect.job_id,
                now
            ],
        )?;
        tx.execute(
            "UPDATE subtitle_file_effects SET state = 'done', updated_at = ?2 WHERE id = ?1",
            params![effect.id, now],
        )?;
        tx.execute(
            "UPDATE subtitle_job_plan
                SET outcome = 'applied', applied_id = ?3, note = ?4, updated_at = ?5
              WHERE job_id = ?1 AND position = ?2",
            params![effect.job_id, effect.position, id, note, now],
        )?;
        tx.commit()?;
        Ok(id)
    })
}

/// Where a plan row's files are, for a job's detail: the work folder and,
/// relative to it, the stored file, the applied copy (while it was not
/// removed) and the video it was put beside. A row kept in the app data
/// folder (`in_app_data`) has its stored file relative to that folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowPaths {
    pub folder: Option<String>,
    pub stored: Option<String>,
    pub applied: Option<String>,
    pub video: Option<String>,
    pub in_app_data: bool,
}

/// [`RowPaths`] of each row of the job's plan, by position.
pub fn row_paths(c: &Connection, job_id: &str) -> rusqlite::Result<Vec<(i64, RowPaths)>> {
    let mut stmt = c.prepare(
        "SELECT p.position, coalesce(s.work_id, k.work_id), coalesce(a.relative_path, k.relative_path),
                ap.path, ap.video_path, coalesce(k.base = 'app_data', 0)
           FROM subtitle_job_plan p
           LEFT JOIN subtitle_stored s ON s.id = p.stored_id
           LEFT JOIN subtitle_assets a ON a.id = s.subtitle_asset_id AND a.base = 'work'
           LEFT JOIN subtitle_assets k ON k.id = p.asset_id
           LEFT JOIN subtitle_applied ap ON ap.id = p.applied_id AND ap.removed_at IS NULL
          WHERE p.job_id = ?1
          ORDER BY p.position",
    )?;
    let rows = stmt.query_map([job_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, bool>(5)?,
        ))
    })?;
    let mut paths = Vec::new();
    for row in rows {
        let (position, work, stored, applied, video, in_app_data) = row?;
        let folder = match &work {
            Some(work) => work_folder(c, work)?,
            None => None,
        };
        paths.push((
            position,
            RowPaths {
                folder,
                stored,
                applied,
                video,
                in_app_data,
            },
        ));
    }
    Ok(paths)
}

// ---------------------------------------------------------------------------
// Receipts

/// A receipt of the job whose file is in the receive area: its ID and path,
/// whether it was cleared, and whether every row of the plan made from it is
/// stored (or dropped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clearable {
    pub id: String,
    pub path: String,
    pub cleared: bool,
}

/// The job's receipts every plan row of which is stored or dropped, with the
/// ones already cleared.
pub fn clearable(c: &Connection, job_id: &str) -> rusqlite::Result<Vec<Clearable>> {
    let mut stmt = c.prepare(
        "SELECT f.id, f.path, f.cleared_at IS NOT NULL FROM subtitle_job_files f
          WHERE f.job_id = ?1 AND f.state = 'done' AND f.same_as IS NULL AND f.path IS NOT NULL
            AND EXISTS (SELECT 1 FROM subtitle_job_plan p WHERE p.file_id = f.id)
            AND NOT EXISTS (SELECT 1 FROM subtitle_job_plan p
                             WHERE p.file_id = f.id AND p.stored_id IS NULL
                               AND p.asset_id IS NULL AND p.action <> 'drop')",
    )?;
    let rows = stmt.query_map([job_id], |r| {
        Ok(Clearable {
            id: r.get(0)?,
            path: r.get(1)?,
            cleared: r.get(2)?,
        })
    })?;
    rows.collect()
}

/// The receipt's bytes are to be removed from the receive area: written
/// before they are.
pub fn clearing(c: &mut Connection, file_id: &str, now: Millis) -> Result<(), JobError> {
    durable(c, |c| {
        // The receipts that share its bytes lose them too.
        c.execute(
            "UPDATE subtitle_job_files SET cleared_at = ?2, updated_at = ?2
             WHERE (id = ?1 OR same_as = ?1) AND cleared_at IS NULL",
            params![file_id, now],
        )?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Stored subtitles a person applies from an episode's row

/// A stored subtitle of a work on an episode with no applied copy of it
/// beside a video (보관만 한 자막), for the work's episode rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOnly {
    pub id: String,
    pub season: u32,
    pub episode: i64,
    /// The stored file's name.
    pub name: String,
    pub creator: Option<String>,
    pub format: SubtitleFormat,
    pub stored_at: Millis,
    /// The job whose plan row has it, which applies it when asked.
    pub job_id: Option<String>,
    /// A job applies it by itself once the episode's video comes (`영상
    /// 대기`): one that waits for the video or a source, ended partly failed,
    /// or is about to run. A job held or waiting for a person does not until
    /// the person acts, so its row is not told so.
    pub awaiting_video: bool,
}

pub fn stored_only(c: &Connection, work_id: &str) -> rusqlite::Result<Vec<StoredOnly>> {
    let mut stmt = c.prepare(
        "SELECT s.id, s.season, s.episode, a.relative_path, s.creator, s.format, s.stored_at,
                (SELECT p.job_id FROM subtitle_job_plan p
                  WHERE p.stored_id = s.id AND p.episode IS NOT NULL
                  ORDER BY p.updated_at DESC LIMIT 1),
                EXISTS (SELECT 1 FROM subtitle_job_plan p JOIN subtitle_jobs j ON j.id = p.job_id
                         WHERE p.stored_id = s.id AND p.action = 'apply'
                           AND p.outcome = 'no_video'
                           AND (j.state = 'waiting' AND j.wait IN ('video', 'subtitle')
                                OR j.state IN ('partial', 'pending', 'running')))
           FROM subtitle_stored s JOIN subtitle_assets a ON a.id = s.subtitle_asset_id
          WHERE s.work_id = ?1 AND s.episode IS NOT NULL
            AND NOT EXISTS (SELECT 1 FROM subtitle_applied ap
                             WHERE ap.stored_id = s.id AND ap.removed_at IS NULL)
          ORDER BY s.season, s.episode, s.stored_at, s.id",
    )?;
    let rows = stmt.query_map([work_id], |r| {
        let path: String = r.get(3)?;
        Ok(StoredOnly {
            id: r.get(0)?,
            season: r.get(1)?,
            episode: r.get(2)?,
            name: path.rsplit('/').next().unwrap_or(&path).to_owned(),
            creator: r.get(4)?,
            format: r.get(5)?,
            stored_at: r.get(6)?,
            job_id: r.get(7)?,
            awaiting_video: r.get(8)?,
        })
    })?;
    rows.collect()
}

/// What came of asking to apply a stored subtitle ([`choose_stored`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredChoice {
    /// The job that applies it, queued again.
    Queued(String),
    NotFound,
    /// Why it is not applied, for the person.
    Refused(&'static str),
}

/// Asks the job that stored `stored_id` (its latest plan row of it whose job
/// neither runs, is held nor waits for an approval) to apply it: the row goes
/// back to `apply` with no outcome and the job is queued again, in one synced
/// transaction with its log line. Choosing it answers the job's question of
/// which file the episode takes, if it asked one: the alternatives asked
/// about are stored only. Refused for a format the app does not apply, a subtitle
/// applied already, an episode with a subtitle (whose change is a
/// replacement's), and when no job can take it.
pub fn choose_stored(
    c: &mut Connection,
    work_id: &str,
    stored_id: &str,
    now: Millis,
) -> Result<StoredChoice, JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored: Option<(u32, Option<i64>, SubtitleFormat)> = tx
            .query_row(
                "SELECT season, episode, format FROM subtitle_stored
                  WHERE id = ?1 AND work_id = ?2",
                params![stored_id, work_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((season, Some(episode), format)) = stored else {
            return Ok(StoredChoice::NotFound);
        };
        if format.extension().is_none() {
            return Ok(StoredChoice::Refused(
                "자동으로 적용하지 않는 형식이라 적용할 수 없어요.",
            ));
        }
        let live = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> rusqlite::Result<bool> {
            tx.query_row(sql, p, |r| r.get::<_, i64>(0)).map(|n| n > 0)
        };
        if live(
            "SELECT count(*) FROM subtitle_applied WHERE stored_id = ?1 AND removed_at IS NULL",
            &[&stored_id],
        )? {
            return Ok(StoredChoice::Refused(
                "이 보관본은 이미 영상 옆에 적용했어요.",
            ));
        }
        let (_, subtitles) = episode_files(&tx, work_id, season, episode)?;
        if !subtitles.is_empty()
            || live(
                "SELECT count(*) FROM subtitle_applied
                  WHERE work_id = ?1 AND season = ?2 AND episode = ?3 AND removed_at IS NULL",
                &[&work_id, &season, &episode],
            )?
        {
            return Ok(StoredChoice::Refused(
                "이 회차에는 자막이 있어요. 다른 자막으로 바꾸려면 교체 비교를 거쳐요.",
            ));
        }
        // The rows that keep it, newest first; a deduplicated file has one in
        // each job that received it.
        let rows: Vec<(String, i64, String, Option<String>)> = {
            let mut stmt = tx.prepare(
                "SELECT p.job_id, p.position, j.state, j.wait
                   FROM subtitle_job_plan p JOIN subtitle_jobs j ON j.id = p.job_id
                  WHERE p.stored_id = ?1 AND p.episode IS NOT NULL
                  ORDER BY p.updated_at DESC, j.seq DESC",
            )?;
            let found = stmt.query_map([stored_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?;
            found.collect::<rusqlite::Result<_>>()?
        };
        let free = |(_, _, state, wait): &&(String, i64, String, Option<String>)| {
            state != "running" && state != "held" && wait.as_deref() != Some("approval")
        };
        let Some((job, position, ..)) = rows.iter().find(free).cloned() else {
            return Ok(StoredChoice::Refused(match rows.first() {
                None => "이 보관본을 받은 작업의 기록이 없어 적용할 수 없어요.",
                Some((_, _, state, ..)) if state == "running" => {
                    "이 보관본을 받은 작업이 진행 중이에요. 끝난 뒤에 다시 적용해 주세요."
                }
                Some((_, _, state, ..)) if state == "held" => {
                    "이 보관본을 받은 작업이 보류 중이라 적용할 수 없어요."
                }
                Some(_) => "이 보관본을 받은 작업이 교체 승인을 기다리고 있어요. 작업 상세에서 정해 주세요.",
            }));
        };
        tx.execute(
            "UPDATE subtitle_job_plan
                SET action = 'apply', outcome = NULL, note = NULL, question = NULL,
                    applied_id = NULL, updated_at = ?3
              WHERE job_id = ?1 AND position = ?2",
            params![job, position, now],
        )?;
        // The alternatives the job asked about for the episode are kept as
        // they are: this one answers which it takes.
        tx.execute(
            "UPDATE subtitle_job_plan
                SET action = 'store', question = NULL,
                    outcome = CASE WHEN stored_id IS NULL THEN NULL ELSE 'stored' END,
                    note = '회차 줄에서 다른 자막을 골라 보관만 해요', updated_at = ?4
              WHERE job_id = ?1 AND position <> ?2 AND episode = ?3
                AND question IS NOT NULL AND outcome IS NULL",
            params![job, position, episode, now],
        )?;
        let note = "회차 줄에서 고른 보관본을 적용해요";
        tx.execute(
            "UPDATE subtitle_jobs
                SET state = 'pending', wait = NULL, finished_at = NULL, note = ?2,
                    state_at = ?3, updated_at = ?3
              WHERE id = ?1",
            params![job, note, now],
        )?;
        tx.execute(
            "INSERT INTO subtitle_job_events (job_id, at, message, detail)
             VALUES (?1, ?2, ?3, ?4)",
            params![job, now, note, format!("{}화", episode)],
        )?;
        tx.commit()?;
        Ok(StoredChoice::Queued(job))
    })
}

// ---------------------------------------------------------------------------
// The library

/// A work's folder, while the library has it in a registered watch folder
/// and the folder is there.
pub fn work_folder(c: &Connection, work_id: &str) -> rusqlite::Result<Option<String>> {
    let found: Option<(String, String)> = c
        .query_row(
            "SELECT f.path, w.dir_name FROM works w
               JOIN watch_folders f ON f.id = w.watch_folder_id
              WHERE w.id = ?1 AND f.unregistered_at IS NULL AND w.missing = 0",
            [work_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(found.map(|(folder, dir)| format!("{}/{dir}", folder.trim_end_matches('/'))))
}

/// The library's files of episode `episode` of the season (whatever zeros
/// its text has), by kind: paths relative to the work folder.
pub fn episode_files(
    c: &Connection,
    work_id: &str,
    season: u32,
    episode: i64,
) -> rusqlite::Result<(Vec<String>, Vec<String>)> {
    let mut stmt = c.prepare(
        "SELECT path, episode, kind FROM media_files WHERE work_id = ?1 AND season = ?2
          ORDER BY path",
    )?;
    let rows = stmt.query_map(params![work_id, season], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let key = episode.to_string();
    let (mut videos, mut subtitles) = (Vec::new(), Vec::new());
    for row in rows {
        let (path, text, kind) = row?;
        if trss_subtitles::episode::numeric_key(&text).as_deref() != Some(key.as_str()) {
            continue;
        }
        match kind.as_str() {
            "video" => videos.push(path),
            _ => subtitles.push(path),
        }
    }
    Ok((videos, subtitles))
}

/// The candidate's line as observed, for a stored subtitle
/// (`anissia_observation`): `anime_no`, `episode`, `creator`, `website` and
/// `updDt_raw`, in JSON.
pub fn observation(c: &Connection, observation_id: i64) -> rusqlite::Result<Option<String>> {
    c.query_row(
        "SELECT json_object('anime_no', s.anime_no, 'episode', o.episode,
                            'creator', s.creator_name, 'website', o.post_url,
                            'updDt_raw', o.updated)
           FROM caption_observations o JOIN subtitle_sources s ON s.id = o.source_id
          WHERE o.id = ?1",
        [observation_id],
        |r| r.get(0),
    )
    .optional()
}
