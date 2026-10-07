//! The records of storing and applying (`migrations/jobs/store_apply.sql` in
//! `trss-core`). The writes that follow a file effect are synced
//! ([`durable`]), so the record of an effect's intent is on disk before the
//! effect.

use std::collections::{btree_map::Entry, BTreeMap};

use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use trss_core::Millis;

use crate::{
    model::{
        AssetKind, Chosen, EffectKind, EffectState, Outcome, PlanAction, RemovalState,
        SubtitleFormat,
    },
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
    /// When a person confirmed the job's placement (배치 확인), for an
    /// upload or a find job.
    pub placement_confirmed_at: Option<Millis>,
}

pub fn job_facts(c: &Connection, job_id: &str) -> rusqlite::Result<Option<JobFacts>> {
    c.query_row(
        "SELECT origin, work_id, season, source_id, creator, placement_confirmed_at
           FROM subtitle_jobs WHERE id = ?1",
        [job_id],
        |r| {
            Ok(JobFacts {
                origin: r.get(0)?,
                work_id: r.get(1)?,
                season: r.get(2)?,
                source_id: r.get(3)?,
                creator: r.get(4)?,
                placement_confirmed_at: r.get(5)?,
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

impl Placed {
    /// Whether a stored subtitle on `episode`, put there by `assignment` on
    /// `basis`, is where this puts its file. A same-number link the decided
    /// mapping left on its episode is the same.
    pub fn held_by(
        &self,
        episode: Option<i64>,
        assignment: Option<Assignment>,
        basis: Option<Basis>,
    ) -> bool {
        episode == Some(self.episode)
            && (assignment == Some(self.assignment)
                || (self.assignment == Assignment::SameNumber
                    && assignment == Some(Assignment::Mapped)))
            && basis == self.basis
    }
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
    /// What a person chose to apply from it ([`choose_stored`]); never set by
    /// the job's own flow.
    pub chosen: Option<Chosen>,
}

impl PlanRow {
    /// Whether the row's file is kept: stored as a subtitle or as an asset.
    pub fn kept(&self) -> bool {
        self.stored_id.is_some() || self.asset_id.is_some()
    }
}

const PLAN_COLUMNS: &str = "job_id, position, file_id, member, name, kind, format, size, sha256,
     item_id, anissia_episode, attachment_episode, episode, assignment, basis, action, question,
     stored_id, outcome, note, applied_id, asset_id, chosen";

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
        chosen: r.get(22)?,
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
                         ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)"
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
                row.chosen,
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
    /// For a replacement's removal or import: the path it takes off or
    /// imports.
    pub source: Option<String>,
    /// The replacement plan it carries out.
    pub plan_id: Option<String>,
}

pub(crate) const EFFECT_COLUMNS: &str = "id, job_id, position, kind, state, folder, temp, target, \
     video, size, sha256, object, reason, source, plan_id";

pub(crate) fn effect(r: &Row<'_>) -> rusqlite::Result<Effect> {
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
        source: r.get(13)?,
        plan_id: r.get(14)?,
    })
}

/// The job's effects that did not end: `intended`, `prepared` or
/// `set_aside`.
pub fn unfinished_effects(c: &Connection, job_id: &str) -> rusqlite::Result<Vec<Effect>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {EFFECT_COLUMNS} FROM subtitle_file_effects
          WHERE job_id = ?1 AND state IN ('intended', 'prepared', 'set_aside')
          ORDER BY created_at, id"
    ))?;
    let rows = stmt.query_map([job_id], effect)?;
    rows.collect()
}

/// Inserts an effect as `intended`, in the caller's transaction.
pub(crate) fn insert_intended(c: &Connection, e: &Effect, now: Millis) -> rusqlite::Result<()> {
    c.execute(
        &format!(
            "INSERT INTO subtitle_file_effects ({EFFECT_COLUMNS}, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'intended', ?5, ?6, ?7, ?8, ?9, ?10, NULL, NULL, ?11, ?12,
                     ?13, ?13)"
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
            e.source,
            e.plan_id,
            now
        ],
    )?;
    Ok(())
}

/// Records an effect's intent, before anything is written.
pub fn intend(c: &mut Connection, e: &Effect, now: Millis) -> Result<(), JobError> {
    durable(c, |c| Ok(insert_intended(c, e, now)?))
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
/// job) but `except`, and the paths a replacement or a relocation under way
/// takes off ([`crate::place::relocate`]; `except` may be a removal's ID):
/// names another effect is about to take or change.
pub fn busy_targets(
    c: &Connection,
    folder: &str,
    except: Option<&str>,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = c.prepare(
        "SELECT lower(target) FROM subtitle_file_effects
          WHERE folder = ?1 AND state IN ('intended', 'prepared', 'set_aside') AND id IS NOT ?2
         UNION
         SELECT lower(source) FROM subtitle_file_effects
          WHERE folder = ?1 AND kind = 'remove' AND state IN ('intended', 'prepared', 'set_aside')
            AND id IS NOT ?2
         UNION
         SELECT lower(path) FROM subtitle_relocations
          WHERE folder = ?1 AND state IN ('intended', 'set_aside') AND id IS NOT ?2",
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
/// whatever its letters' case, but the removed ones ([`crate::place::cleanup`]):
/// their files are gone, and their paths are free again.
pub fn assets_under(
    c: &Connection,
    work_id: &str,
    base: &str,
    prefix: &str,
) -> rusqlite::Result<Vec<Asset>> {
    let mut stmt = c.prepare(
        "SELECT id, kind, relative_path, byte_size, sha256 FROM subtitle_assets
          WHERE work_id = ?1 AND base = ?3 AND removed_at IS NULL
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

/// The paths of the work's assets kept in `base` under `prefix`, whatever
/// its letters' case, removed ones too, the ones not removed first: a
/// folder's name stays the one first written ([`crate::place::Placer`]'s
/// creator folder), as the folder may outlive its recorded files.
pub fn asset_paths_under(
    c: &Connection,
    work_id: &str,
    base: &str,
    prefix: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = c.prepare(
        "SELECT relative_path FROM subtitle_assets
          WHERE work_id = ?1 AND base = ?3
            AND lower(substr(relative_path, 1, length(?2))) = lower(?2)
          ORDER BY removed_at IS NOT NULL, created_at, id",
    )?;
    let rows = stmt.query_map(params![work_id, prefix, base], |r| r.get(0))?;
    rows.collect()
}

/// The asset `id` while it is not removed ([`crate::place::cleanup`]).
pub fn asset(c: &Connection, id: &str) -> rusqlite::Result<Option<Asset>> {
    c.query_row(
        "SELECT id, kind, relative_path, byte_size, sha256 FROM subtitle_assets
          WHERE id = ?1 AND removed_at IS NULL",
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
/// same bytes, source and link, [`same_stored`]) and the row's link to it; a font,
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
        let same = same_stored(
            &tx,
            &asset_id,
            &facts.work_id,
            facts.season,
            facts.source_id.as_deref(),
            placed,
        )?;
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

/// The stored subtitle, not cleaned, of the asset's bytes from `source` on
/// the link `placed` (its episode, what put it there and on which basis).
/// The same bytes of the same source on the same link are one revision,
/// however often received; a link of another basis is another stored
/// subtitle, which a mapping change may move elsewhere. One a person
/// cleaned is no stored copy any more, so the bytes are stored anew.
pub fn same_stored(
    c: &Connection,
    asset_id: &str,
    work_id: &str,
    season: u32,
    source_id: Option<&str>,
    placed: Option<&Placed>,
) -> rusqlite::Result<Option<String>> {
    c.query_row(
        "SELECT id FROM subtitle_stored
          WHERE subtitle_asset_id = ?1 AND work_id = ?2 AND season = ?3
            AND source_id IS ?4 AND episode IS ?5 AND assignment IS ?6 AND basis IS ?7
            AND cleaned_at IS NULL",
        params![
            asset_id,
            work_id,
            season,
            source_id,
            placed.map(|p| p.episode),
            placed.map(|p| p.assignment.code()),
            placed.and_then(|p| p.basis).map(Basis::code),
        ],
        |r| r.get(0),
    )
    .optional()
}

/// A stored subtitle as [`relink`] and [`place_stored`] read it to give a
/// row one of its own link.
struct OnStored {
    work_id: String,
    season: u32,
    package_id: String,
    asset_id: String,
    source_id: Option<String>,
    anissia_episode: Option<String>,
    observation: Option<String>,
    /// Its link: the episode, what put it there and on which basis.
    episode: Option<i64>,
    assignment: Option<Assignment>,
    basis: Option<Basis>,
}

/// The stored subtitle `id`, unless a person cleaned it.
fn on_stored(c: &Connection, id: &str) -> rusqlite::Result<Option<OnStored>> {
    c.query_row(
        "SELECT work_id, season, package_id, subtitle_asset_id, source_id, anissia_episode,
                anissia_observation, episode, assignment, basis
           FROM subtitle_stored WHERE id = ?1 AND cleaned_at IS NULL",
        [id],
        |r| {
            let assignment: Option<String> = r.get(8)?;
            let basis: Option<String> = r.get(9)?;
            Ok(OnStored {
                work_id: r.get(0)?,
                season: r.get(1)?,
                package_id: r.get(2)?,
                asset_id: r.get(3)?,
                source_id: r.get(4)?,
                anissia_episode: r.get(5)?,
                observation: r.get(6)?,
                episode: r.get(7)?,
                assignment: assignment.as_deref().and_then(Assignment::parse),
                basis: basis.as_deref().and_then(Basis::parse),
            })
        },
    )
    .optional()
}

/// The row and the stored subtitle it is on, when that one holds another
/// link than the row's.
fn off_link(
    c: &Connection,
    job_id: &str,
    position: i64,
) -> rusqlite::Result<Option<(PlanRow, OnStored)>> {
    let Some(row) = plan_row_at(c, job_id, position)? else {
        return Ok(None);
    };
    let (Some(placed), Some(id)) = (row.placed.as_ref(), row.stored_id.as_deref()) else {
        return Ok(None);
    };
    let on = on_stored(c, id)?.filter(|on| !placed.held_by(on.episode, on.assignment, on.basis));
    Ok(on.map(|on| (row, on)))
}

/// The stored subtitle of the row on the link `placed` (`None`: no
/// episode), for a row now on `old` (read as `on`): the one of `on`'s bytes
/// and source on that link ([`same_stored`]), else a new one of the same
/// bytes, stored when `old` was so that the source's revisions keep their
/// order. Its package is the row's job's entry of the bytes when there is
/// one; the links of `old` are kept, and the link step of the job's next
/// run adds that package's files ([`link`]), if the job runs again (one
/// that ends with this row's first copy does not). `old` stays as it is.
fn of_link(
    c: &Connection,
    row: &PlanRow,
    old: &str,
    on: OnStored,
    placed: Option<&Placed>,
) -> rusqlite::Result<String> {
    let same = same_stored(
        c,
        &on.asset_id,
        &on.work_id,
        on.season,
        on.source_id.as_deref(),
        placed,
    )?;
    if let Some(id) = same {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let own: Option<String> = c
        .query_row(
            "SELECT e.package_id FROM subtitle_package_entries e
               JOIN subtitle_packages p ON p.id = e.package_id
              WHERE p.job_id = ?1 AND e.asset_id = ?2
              ORDER BY e.package_id LIMIT 1",
            params![row.job_id, on.asset_id],
            |r| r.get(0),
        )
        .optional()?;
    // A package of the row's job links its own files once more.
    let links_known = own.as_ref().map(|_| 0);
    let package = own.unwrap_or(on.package_id);
    // What Anissia said of the episode is the row's only when it is the same
    // episode.
    let observation = on
        .observation
        .filter(|_| row.anissia_episode == on.anissia_episode);
    c.execute(
        "INSERT INTO subtitle_stored
             (id, work_id, season, package_id, subtitle_asset_id, source_id,
              anissia_episode, attachment_episode, assignment, basis, episode,
              format, encoding, creator, language, purpose, release, revision_label,
              links_known, anissia_observation, job_id, stored_at)
         SELECT ?2, work_id, season, ?3, subtitle_asset_id, source_id,
                ?4, ?5, ?6, ?7, ?8,
                format, encoding, creator, language, purpose, release, revision_label,
                coalesce(?9, links_known), ?10, ?11, stored_at
           FROM subtitle_stored WHERE id = ?1",
        params![
            old,
            id,
            package,
            row.anissia_episode,
            row.attachment_episode,
            placed.map(|p| p.assignment.code()),
            placed.and_then(|p| p.basis).map(Basis::code),
            placed.map(|p| p.episode),
            links_known,
            observation,
            row.job_id,
        ],
    )?;
    c.execute(
        "INSERT INTO subtitle_stored_assets (stored_id, asset_id, role)
         SELECT ?2, asset_id, role FROM subtitle_stored_assets WHERE stored_id = ?1",
        params![old, id],
    )?;
    Ok(id)
}

/// Gives the row a stored subtitle of its own link when the one it is on
/// holds another ([`Placed::held_by`]), so that a plan made for the row
/// holds once approved. A row comes to such a one when a build that told
/// stored subtitles apart by episode and assignment alone let it share the
/// same bytes of another basis, or when, in a build before [`place_stored`],
/// a person's 배치 확인 of another job moved the stored subtitle they shared.
/// The row takes the stored subtitle of its link ([`of_link`]);
/// the one it had stays as it is for whatever else holds it. Reads first,
/// and writes one synced transaction only for a row to relink; returns the
/// row as relinked, or `None` when it was left (no link, its stored subtitle
/// cleaned, or holding its link).
pub fn relink(
    c: &mut Connection,
    job_id: &str,
    position: i64,
    now: Millis,
) -> Result<Option<PlanRow>, JobError> {
    if off_link(c, job_id, position)?.is_none() {
        return Ok(None);
    }
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((row, on)) = off_link(&tx, job_id, position)? else {
            return Ok(None);
        };
        let Some(placed) = row.placed.as_ref() else {
            return Ok(None);
        };
        let Some(old) = row.stored_id.as_deref() else {
            return Ok(None);
        };
        let stored = of_link(&tx, &row, old, on, Some(placed))?;
        tx.execute(
            "UPDATE subtitle_job_plan SET stored_id = ?3, updated_at = ?4
              WHERE job_id = ?1 AND position = ?2",
            params![row.job_id, row.position, stored, now],
        )?;
        let relinked = plan_row_at(&tx, job_id, position)?;
        tx.commit()?;
        Ok(relinked)
    })
}

/// A stored subtitle's asset, as the apply copies it; none for a stored
/// subtitle a person cleaned.
pub fn stored_asset(c: &Connection, stored_id: &str) -> rusqlite::Result<Option<Asset>> {
    let id: Option<String> = c
        .query_row(
            "SELECT subtitle_asset_id FROM subtitle_stored WHERE id = ?1 AND cleaned_at IS NULL",
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
/// files (`subtitle_stored_assets`), but those a cleanup removed, and marks
/// its links known. One synced transaction; links made already stay as they
/// are.
pub fn link(c: &mut Connection, links: &[(String, Vec<(String, Role)>)]) -> Result<(), JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (stored, assets) in links {
            for (asset, role) in assets {
                // A file a person's cleanup removed is linked to nothing.
                tx.execute(
                    "INSERT OR IGNORE INTO subtitle_stored_assets (stored_id, asset_id, role)
                     SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM subtitle_assets
                                                      WHERE id = ?2 AND removed_at IS NULL)",
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
/// `applied` on the copy's episode; the effect is `done`. One synced
/// transaction, which also plans the copy's relocation when the stored
/// subtitle is on another episode by then. Returns the applied copy's ID.
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
                SET outcome = 'applied', applied_id = ?3, note = ?4, episode = ?6, updated_at = ?5
              WHERE job_id = ?1 AND position = ?2",
            params![effect.job_id, effect.position, id, note, now, copy.episode],
        )?;
        // A mapping change committed after the run looked at the row's
        // episode moved the stored subtitle: the copy is on its old episode,
        // and the source's relocation plans to move it.
        let moved: Option<String> = tx
            .query_row(
                "SELECT source_id FROM subtitle_stored
                  WHERE id = ?1 AND episode IS NOT ?2 AND assignment = 'mapped'
                    AND source_id IS NOT NULL",
                params![copy.stored_id, copy.episode],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(source) = moved {
            crate::place::relocate::sync_in(&tx, &copy.work_id, copy.season, &source, now)?;
        }
        tx.commit()?;
        Ok(id)
    })
}

/// A file beside a video whose bytes are a stored subtitle's, to be recorded
/// as that subtitle's applied copy ([`adopt`]).
#[derive(Debug, Clone)]
pub struct Adopted {
    pub work_id: String,
    pub stored_id: String,
    pub season: u32,
    pub episode: i64,
    /// The video it is beside, relative to the work folder.
    pub video: String,
    /// The file, relative to the work folder, as it was read.
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub object: String,
    /// The plan row it settles.
    pub job_id: String,
    pub position: i64,
    /// A person chose the stored subtitle: the file is its copy even when
    /// the app recorded it as another stored subtitle's.
    pub take_over: bool,
}

/// Records the file beside the video as the stored subtitle's applied copy,
/// the file left as it is, and the row as `applied` with `note`; the new
/// record's ID. `None`, nothing recorded, when the file is a live applied copy
/// of the stored subtitle already, or of another stored subtitle's while
/// `take_over` is not set. A record at the path whose bytes are not the file's
/// is of a file that is gone: it ends as [`applied`] ends it. One synced
/// transaction.
pub fn adopt(
    c: &mut Connection,
    what: &Adopted,
    note: &str,
    now: Millis,
) -> Result<Option<String>, JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let live: Option<(String, String)> = tx
            .query_row(
                "SELECT stored_id, sha256 FROM subtitle_applied
                  WHERE work_id = ?1 AND path = ?2 AND removed_at IS NULL",
                params![what.work_id, what.path],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((stored, sha256)) = &live {
            let same_file = *sha256 == what.sha256;
            if same_file && (*stored == what.stored_id || !what.take_over) {
                return Ok(None);
            }
            tx.execute(
                "UPDATE subtitle_applied SET removed_at = ?3
                  WHERE work_id = ?1 AND path = ?2 AND removed_at IS NULL",
                params![what.work_id, what.path, now],
            )?;
        }
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO subtitle_applied
                 (id, work_id, stored_id, season, episode, video_path, path, byte_size, sha256,
                  object, job_id, applied_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                what.work_id,
                what.stored_id,
                what.season,
                what.episode,
                what.video,
                what.path,
                what.size as i64,
                what.sha256,
                what.object,
                what.job_id,
                now
            ],
        )?;
        tx.execute(
            "UPDATE subtitle_job_plan
                SET outcome = 'applied', applied_id = ?3, note = ?4, episode = ?6, updated_at = ?5
              WHERE job_id = ?1 AND position = ?2",
            params![what.job_id, what.position, id, note, now, what.episode],
        )?;
        tx.commit()?;
        Ok(Some(id))
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
    /// The row's own store published its file: a new asset, not one kept
    /// before with the same bytes (nor a font it did not receive).
    pub made: bool,
}

/// [`RowPaths`] of each row of the job's plan, by position. A file a cleanup
/// removed ([`crate::place::cleanup`]) is in none.
pub fn row_paths(c: &Connection, job_id: &str) -> rusqlite::Result<Vec<(i64, RowPaths)>> {
    let mut stmt = c.prepare(
        "SELECT p.position, coalesce(s.work_id, k.work_id), coalesce(a.relative_path, k.relative_path),
                ap.path, ap.video_path, coalesce(k.base = 'app_data', 0),
                EXISTS (SELECT 1 FROM subtitle_file_effects e
                         WHERE e.job_id = p.job_id AND e.position = p.position
                           AND e.kind = 'store' AND e.state = 'done')
           FROM subtitle_job_plan p
           LEFT JOIN subtitle_stored s ON s.id = p.stored_id
           LEFT JOIN subtitle_assets a
                  ON a.id = s.subtitle_asset_id AND a.base = 'work' AND a.removed_at IS NULL
           LEFT JOIN subtitle_assets k ON k.id = p.asset_id AND k.removed_at IS NULL
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
            r.get::<_, bool>(6)?,
        ))
    })?;
    let mut paths = Vec::new();
    for row in rows {
        let (position, work, stored, applied, video, in_app_data, made) = row?;
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
                made,
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
    /// The paths of the later volumes of the split archive it is the first
    /// of, which go with it.
    pub volumes: Vec<String>,
    /// It is an archive that was unpacked: its unpack folder goes too.
    pub unpacked: bool,
}

/// The job's receipts every plan row of which is stored or dropped, with the
/// ones already cleared.
pub fn clearable(c: &Connection, job_id: &str) -> rusqlite::Result<Vec<Clearable>> {
    let mut stmt = c.prepare(
        "SELECT f.id, f.path, f.cleared_at IS NOT NULL, f.unpacked_at IS NOT NULL
           FROM subtitle_job_files f
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
            volumes: Vec::new(),
            unpacked: r.get(3)?,
        })
    })?;
    let mut rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut volumes = c.prepare(
        "SELECT path FROM subtitle_job_files
          WHERE volume_of = ?1 AND same_as IS NULL AND path IS NOT NULL ORDER BY name",
    )?;
    for row in &mut rows {
        row.volumes = volumes
            .query_map([&row.id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(rows)
}

/// The receipt's bytes are to be removed from the receive area: written
/// before they are.
pub fn clearing(c: &mut Connection, file_id: &str, now: Millis) -> Result<(), JobError> {
    durable(c, |c| {
        // The receipts that share its bytes lose them too, and so do the
        // later volumes of a split archive and the receipts sharing theirs.
        c.execute(
            "UPDATE subtitle_job_files SET cleared_at = ?2, updated_at = ?2
             WHERE (id = ?1 OR same_as = ?1 OR volume_of = ?1
                    OR same_as IN (SELECT id FROM subtitle_job_files WHERE volume_of = ?1))
               AND cleared_at IS NULL",
            params![file_id, now],
        )?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Stored subtitles a person applies from an episode's row

/// A stored subtitle of a work on an episode with no applied copy of it
/// beside a video (보관만 한 자막), for the work's episode rows. One a person
/// cleaned is none, and so is one whose last replacement plan the person
/// decided `현재 유지` (`kept`) while the episode has a subtitle: the 자막 card
/// still offers it ([`work_copies`]), and choosing it again opens a new plan.
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
    /// The job whose plan row has it, which applies it when asked; for an
    /// imported copy, the job whose replacement imported it.
    pub job_id: Option<String>,
    /// A job applies it by itself once the episode's video comes (`영상
    /// 대기`): one that waits for the video or a source, ended partly failed,
    /// or is about to run. A job held or waiting for a person does not until
    /// the person acts, so its row is not told so.
    pub awaiting_video: bool,
    /// The job whose replacement plan to decide puts it beside the
    /// episode's video (`교체 승인`).
    pub awaiting_approval: Option<String>,
    /// The episode has a subtitle, so applying it is a replacement the
    /// person compares first ([`StoredOptions::apply`]).
    pub compare: bool,
}

pub fn stored_only(c: &Connection, work_id: &str) -> rusqlite::Result<Vec<StoredOnly>> {
    let mut stmt = c.prepare(
        "SELECT s.id, s.season, s.episode, a.relative_path, s.creator, s.format, s.stored_at,
                coalesce(
                    (SELECT p.job_id FROM subtitle_job_plan p
                      WHERE p.stored_id = s.id AND p.episode IS NOT NULL
                      ORDER BY p.updated_at DESC LIMIT 1),
                    -- an imported copy: the job whose replacement imported it
                    (SELECT j.id FROM subtitle_packages pk JOIN subtitle_jobs j ON j.id = s.job_id
                      WHERE pk.id = s.package_id AND pk.source_kind = 'existing'
                        AND j.origin <> ?2
                        AND EXISTS (SELECT 1 FROM subtitle_job_plan q WHERE q.job_id = j.id))),
                EXISTS (SELECT 1 FROM subtitle_job_plan p JOIN subtitle_jobs j ON j.id = p.job_id
                         WHERE p.stored_id = s.id AND p.action = 'apply'
                           AND p.outcome = 'no_video'
                           AND (j.state = 'waiting' AND j.wait IN ('video', 'subtitle')
                                OR j.state IN ('partial', 'pending', 'running'))),
                (SELECT r.job_id FROM subtitle_replacements r
                  WHERE r.stored_id = s.id AND r.state = 'open'
                  ORDER BY r.created_at LIMIT 1),
                coalesce(
                    (SELECT r.state = 'kept' FROM subtitle_replacements r
                      WHERE r.stored_id = s.id
                      ORDER BY r.created_at DESC, r.rowid DESC LIMIT 1),
                    0)
           FROM subtitle_stored s JOIN subtitle_assets a ON a.id = s.subtitle_asset_id
          WHERE s.work_id = ?1 AND s.episode IS NOT NULL AND s.cleaned_at IS NULL
            AND NOT EXISTS (SELECT 1 FROM subtitle_applied ap
                             WHERE ap.stored_id = s.id AND ap.removed_at IS NULL)
          ORDER BY s.season, s.episode, s.stored_at, s.id",
    )?;
    let rows = stmt.query_map(params![work_id, crate::store::RELOCATE], |r| {
        let path: String = r.get(3)?;
        let kept: bool = r.get(10)?;
        Ok((
            kept,
            StoredOnly {
                id: r.get(0)?,
                season: r.get(1)?,
                episode: r.get(2)?,
                name: path.rsplit('/').next().unwrap_or(&path).to_owned(),
                creator: r.get(4)?,
                format: r.get(5)?,
                stored_at: r.get(6)?,
                job_id: r.get(7)?,
                awaiting_video: r.get(8)?,
                awaiting_approval: r.get(9)?,
                compare: false,
            },
        ))
    })?;
    let rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut library = Library::of(work_id);
    let mut stored = Vec::with_capacity(rows.len());
    for (kept, mut one) in rows {
        let recorded = library
            .files(c, one.season, one.episode)?
            .is_some_and(|f| !f.subtitles.is_empty());
        one.compare = recorded || !applied_formats(c, work_id, one.season, one.episode)?.is_empty();
        // Kept over the episode's subtitle: decided, until the episode has none.
        if !(kept && one.compare) {
            stored.push(one);
        }
    }
    Ok(stored)
}

/// The library's files of a work, a season read once
/// ([`season_files`]): a page that asks about every stored subtitle of the
/// work does not read the season's files again for each.
struct Library<'a> {
    work_id: &'a str,
    seasons: BTreeMap<u32, BTreeMap<i64, EpisodeFiles>>,
}

impl<'a> Library<'a> {
    fn of(work_id: &'a str) -> Library<'a> {
        Library {
            work_id,
            seasons: BTreeMap::new(),
        }
    }

    /// The episode's files; none when the library has no file of it.
    fn files(
        &mut self,
        c: &Connection,
        season: u32,
        episode: i64,
    ) -> rusqlite::Result<Option<&EpisodeFiles>> {
        let files = match self.seasons.entry(season) {
            Entry::Occupied(read) => read.into_mut(),
            Entry::Vacant(unread) => unread.insert(season_files(c, self.work_id, season)?),
        };
        Ok(files.get(&episode))
    }
}

/// The creator and format of each copy the app applied to the episode and has
/// not removed.
fn applied_formats(
    c: &Connection,
    work_id: &str,
    season: u32,
    episode: i64,
) -> rusqlite::Result<Vec<(Option<String>, SubtitleFormat)>> {
    let mut stmt = c.prepare(
        "SELECT s.creator, s.format FROM subtitle_applied ap
           JOIN subtitle_stored s ON s.id = ap.stored_id
          WHERE ap.work_id = ?1 AND ap.season = ?2 AND ap.episode = ?3 AND ap.removed_at IS NULL",
    )?;
    let copies = stmt.query_map(params![work_id, season, episode], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?;
    copies.collect()
}

/// A copy the app applied beside a video and has not removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedPlace {
    /// Relative to the work folder.
    pub path: String,
    pub applied_at: Millis,
}

/// A stored subtitle of a work on an episode, applied or not, for the work's
/// 자막 card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCopy {
    pub id: String,
    pub season: u32,
    pub episode: i64,
    /// The stored file's name.
    pub name: String,
    /// The stored file's path, relative to the work folder.
    pub stored_path: String,
    pub creator: Option<String>,
    pub format: SubtitleFormat,
    pub stored_at: Millis,
    /// Its applied copies, oldest first.
    pub applied: Vec<AppliedPlace>,
    pub options: StoredOptions,
}

/// The work's stored subtitles on an episode that a person did not clean, by
/// season, episode and then newest stored first, with what a person can ask of
/// each ([`stored_options`]).
pub fn work_copies(c: &Connection, work_id: &str) -> rusqlite::Result<Vec<StoredCopy>> {
    let mut stmt = c.prepare(
        "SELECT s.id, s.season, s.episode, a.relative_path, s.creator, s.format, s.stored_at
           FROM subtitle_stored s JOIN subtitle_assets a ON a.id = s.subtitle_asset_id
          WHERE s.work_id = ?1 AND s.episode IS NOT NULL AND s.cleaned_at IS NULL
          ORDER BY s.season, s.episode, s.stored_at DESC, s.id",
    )?;
    let rows = stmt
        .query_map([work_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, SubtitleFormat>(5)?,
                r.get::<_, Millis>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut applied = c.prepare(
        "SELECT path, applied_at FROM subtitle_applied
          WHERE stored_id = ?1 AND removed_at IS NULL ORDER BY applied_at, id",
    )?;
    let mut copies = Vec::with_capacity(rows.len());
    let mut library = Library::of(work_id);
    for (id, season, episode, stored_path, creator, format, stored_at) in rows {
        let Some(options) = options_in(c, &mut library, &id)? else {
            continue;
        };
        let placed = applied
            .query_map([&id], |r| {
                Ok(AppliedPlace {
                    path: r.get(0)?,
                    applied_at: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        copies.push(StoredCopy {
            season,
            episode,
            name: stored_path
                .rsplit('/')
                .next()
                .unwrap_or(&stored_path)
                .to_owned(),
            stored_path,
            creator,
            format,
            stored_at,
            applied: placed,
            options,
            id,
        });
    }
    Ok(copies)
}

/// What came of asking to apply a stored subtitle ([`choose_stored`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredChoice {
    /// The job that applies it, queued again; `compare` when the job plans a
    /// replacement for the person to compare and approve before anything
    /// beside the video changes ([`StoredOptions`]).
    Queued {
        job_id: String,
        compare: bool,
    },
    NotFound,
    /// Why it is not applied, for the person.
    Refused(&'static str),
}

/// Why a stored subtitle cannot be added ([`Chosen::Add`]).
pub const ADD_REFUSED: &str =
    "추가로 적용할 수 있는 것은 이 회차에 적용한 제작자의 다른 형식이에요.";

/// What a person can ask of a stored subtitle now ([`stored_options`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOptions {
    /// Whether a live applied copy is it.
    pub applied: bool,
    /// Whether the episode has a subtitle: one beside the video the library
    /// recorded, or a copy the app applied and has not removed.
    pub has_subtitle: bool,
    /// Applying it as the episode's ([`Chosen::Apply`]): whether the person
    /// compares it with the episode's subtitle, or why not.
    pub apply: Result<bool, &'static str>,
    /// Applying it beside the applied copies ([`Chosen::Add`]): whether the
    /// person compares it with a subtitle the library recorded at the name
    /// it takes beside the video, or why not.
    pub add: Result<bool, &'static str>,
    /// The row of the job that would apply it, or why there is none.
    job: Result<(String, Option<i64>), &'static str>,
}

/// A job that may take a stored subtitle: its ID, the position of the row
/// that keeps it (none for an imported copy), the job's state and what it
/// waits for.
type Taker = (String, Option<i64>, String, Option<String>);

/// The job whose replacement imported the stored subtitle (a file found
/// beside the video and kept, in an `existing` package), as a row of
/// [`options_in`] with no position. None for a received copy, for one whose
/// job is gone, and for a job with no plan row to take a receipt from
/// ([`choose_stored`]).
fn imported_by(c: &Connection, stored_id: &str) -> rusqlite::Result<Option<Taker>> {
    c.query_row(
        "SELECT j.id, j.state, j.wait
           FROM subtitle_stored s JOIN subtitle_packages p ON p.id = s.package_id
                JOIN subtitle_jobs j ON j.id = s.job_id
          WHERE s.id = ?1 AND p.source_kind = 'existing' AND j.origin <> ?2
            AND EXISTS (SELECT 1 FROM subtitle_job_plan q WHERE q.job_id = j.id)",
        params![stored_id, crate::store::RELOCATE],
        |r| Ok((r.get(0)?, None, r.get(1)?, r.get(2)?)),
    )
    .optional()
}

/// What a person can ask of the stored subtitle `stored_id` of the work:
/// `None` for one of another work, one a person cleaned and one on no
/// episode. These are [`choose_stored`]'s rules.
pub fn stored_options(
    c: &Connection,
    work_id: &str,
    stored_id: &str,
) -> rusqlite::Result<Option<StoredOptions>> {
    options_in(c, &mut Library::of(work_id), stored_id)
}

/// [`stored_options`] with the library's files read through `library`.
fn options_in(
    c: &Connection,
    library: &mut Library<'_>,
    stored_id: &str,
) -> rusqlite::Result<Option<StoredOptions>> {
    let work_id = library.work_id;
    let stored: Option<(u32, Option<i64>, SubtitleFormat, Option<String>)> = c
        .query_row(
            "SELECT season, episode, format, creator FROM subtitle_stored
              WHERE id = ?1 AND work_id = ?2 AND cleaned_at IS NULL",
            params![stored_id, work_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((season, Some(episode), format, creator)) = stored else {
        return Ok(None);
    };
    let applied: i64 = c.query_row(
        "SELECT count(*) FROM subtitle_applied WHERE stored_id = ?1 AND removed_at IS NULL",
        [stored_id],
        |r| r.get(0),
    )?;
    let early = if format.extension().is_none() {
        Some("자동으로 적용하지 않는 형식이라 적용할 수 없어요.")
    } else if applied > 0 {
        Some("이 보관본은 이미 영상 옆에 적용했어요.")
    } else {
        None
    };
    let copies = applied_formats(c, work_id, season, episode)?;
    let files = library.files(c, season, episode)?;
    let has_subtitle = !copies.is_empty() || files.is_some_and(|f| !f.subtitles.is_empty());
    // The same creator's other format; a stored subtitle that names no
    // creator matches none.
    let addable = creator.is_some()
        && copies.iter().any(|(by, _)| *by == creator)
        && copies.iter().all(|(_, f)| *f != format);
    // A subtitle the library recorded at the name an added copy takes beside
    // a video (the placer's, any case): the job plans its replacement.
    let add_compare = format.extension().is_some_and(|ext| {
        files.is_some_and(|f| {
            f.videos.iter().any(|video| {
                let (dir, stem) = super::video_parts(video);
                let name = super::joined(dir, &format!("{stem}.{ext}")).to_lowercase();
                f.subtitles.iter().any(|s| s.to_lowercase() == name)
            })
        })
    });
    // The rows that keep it, newest first; a deduplicated file has one in
    // each job that received it. A relocation's rows move copies a person
    // confirmed, so they are not chosen from. An imported copy (a file found
    // beside a video, kept before a replacement) is in no job's plan: the job
    // whose replacement imported it takes it, with a row made when it is
    // chosen (no position yet).
    let mut rows: Vec<Taker> = {
        let mut stmt = c.prepare(
            "SELECT p.job_id, p.position, j.state, j.wait
               FROM subtitle_job_plan p JOIN subtitle_jobs j ON j.id = p.job_id
              WHERE p.stored_id = ?1 AND p.episode IS NOT NULL AND j.origin <> ?2
              ORDER BY p.updated_at DESC, j.seq DESC",
        )?;
        let found = stmt.query_map(params![stored_id, crate::store::RELOCATE], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        found.collect::<rusqlite::Result<_>>()?
    };
    if rows.is_empty() {
        rows.extend(imported_by(c, stored_id)?);
    }
    let free = |(_, _, state, wait): &&Taker| {
        state != "running" && state != "held" && wait.as_deref() != Some("approval")
    };
    let job = match rows.iter().find(free) {
        Some((job, position, ..)) => Ok((job.clone(), *position)),
        None => Err(match rows.first() {
            None => "이 보관본을 받은 작업의 기록이 없어 적용할 수 없어요.",
            Some((_, _, state, ..)) if state == "running" => {
                "이 보관본을 받은 작업이 진행 중이에요. 끝난 뒤에 다시 적용해 주세요."
            }
            Some((_, _, state, ..)) if state == "held" => {
                "이 보관본을 받은 작업이 보류 중이라 적용할 수 없어요."
            }
            Some(_) => {
                "이 보관본을 받은 작업이 교체 승인을 기다리고 있어요. 작업 상세에서 정해 주세요."
            }
        }),
    };
    let taken = job.clone().map(|_| ());
    let (apply, add) = match early {
        Some(reason) => (Err(reason), Err(reason)),
        None => (
            taken.map(|()| has_subtitle),
            match addable {
                true => taken.map(|()| add_compare),
                false => Err(ADD_REFUSED),
            },
        ),
    };
    Ok(Some(StoredOptions {
        applied: applied > 0,
        has_subtitle,
        apply,
        add,
        job,
    }))
}

/// Asks the job that stored `stored_id` (its latest plan row of it whose job
/// neither runs, is held nor waits for an approval) to apply it as `mode`
/// says: the row goes back to `apply` with no outcome, marked as chosen by a
/// person, and the job is queued again, in one synced transaction with its
/// log line. Choosing it answers the job's question of which file the
/// episode takes, if it asked one: the alternatives asked about are stored
/// only.
///
/// [`Chosen::Apply`] on an episode with a subtitle is a replacement the
/// worker plans for a person to compare and approve (`compare` in the
/// result); on one with none the copy is applied at once. [`Chosen::Add`]
/// applies the same creator's other format beside the episode's applied
/// copies and takes none off; a subtitle the library recorded at the name it
/// takes is a replacement to compare too. Refused for a format the app does not apply, a
/// subtitle applied already, an add that is not another format of a creator
/// whose copy is applied on the episode, and when no job can take it. A
/// stored subtitle a person cleaned is not found.
pub fn choose_stored(
    c: &mut Connection,
    work_id: &str,
    stored_id: &str,
    mode: Chosen,
    now: Millis,
) -> Result<StoredChoice, JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(options) = stored_options(&tx, work_id, stored_id)? else {
            return Ok(StoredChoice::NotFound);
        };
        let allowed = match mode {
            Chosen::Apply => options.apply,
            Chosen::Add => options.add,
        };
        let compare = match allowed {
            Ok(compare) => compare,
            Err(reason) => return Ok(StoredChoice::Refused(reason)),
        };
        let Ok((job, position)) = options.job else {
            unreachable!("a subtitle that can be chosen has a job to take it")
        };
        let episode: i64 = tx.query_row(
            "SELECT episode FROM subtitle_stored WHERE id = ?1",
            [stored_id],
            |r| r.get(0),
        )?;
        // An imported copy is in no plan yet: the job that imported it keeps
        // it as a row of its own, stored only, which the update below turns
        // into the apply. The row takes its receipt from one of the job's
        // rows, as a relocation's rows do; the file's name is the one it had
        // beside the video.
        let position = match position {
            Some(position) => position,
            None => tx.query_row(
                "INSERT INTO subtitle_job_plan
                     (job_id, position, file_id, name, kind, format, size, sha256, episode,
                      assignment, basis, action, outcome, note, stored_id, updated_at)
                 SELECT ?1,
                        (SELECT max(position) + 1 FROM subtitle_job_plan WHERE job_id = ?1),
                        (SELECT file_id FROM subtitle_job_plan WHERE job_id = ?1
                          ORDER BY position LIMIT 1),
                        coalesce((SELECT original_name FROM subtitle_package_entries e
                                   WHERE e.package_id = s.package_id AND e.asset_id = a.id), 'x'),
                        'subtitle', s.format, a.byte_size, a.sha256, s.episode, s.assignment,
                        s.basis, 'store', 'stored', NULL, s.id, ?3
                   FROM subtitle_stored s JOIN subtitle_assets a ON a.id = s.subtitle_asset_id
                  WHERE s.id = ?2
                 RETURNING position",
                params![job, stored_id, now],
                |r| r.get(0),
            )?,
        };
        // The row goes where the stored subtitle is now: a mapping change
        // may have moved it since the job planned the row.
        tx.execute(
            "UPDATE subtitle_job_plan
                SET action = 'apply', outcome = NULL, note = NULL, question = NULL,
                    applied_id = NULL, chosen = ?4, updated_at = ?3,
                    episode = s.episode, assignment = s.assignment, basis = s.basis
               FROM (SELECT episode, assignment, basis FROM subtitle_stored WHERE id = ?5) AS s
              WHERE job_id = ?1 AND position = ?2",
            params![job, position, now, mode, stored_id],
        )?;
        // The alternatives the job asked about for the episode are kept as
        // they are: this one answers which it takes.
        tx.execute(
            "UPDATE subtitle_job_plan
                SET action = 'store', question = NULL,
                    outcome = CASE WHEN stored_id IS NULL THEN NULL ELSE 'stored' END,
                    note = '다른 자막을 골라 보관만 해요', updated_at = ?4
              WHERE job_id = ?1 AND position <> ?2 AND episode = ?3
                AND question IS NOT NULL AND outcome IS NULL",
            params![job, position, episode, now],
        )?;
        let note = match mode {
            Chosen::Apply => "고른 보관본을 적용해요",
            Chosen::Add => "고른 보관본을 추가로 적용해요",
        };
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
        Ok(StoredChoice::Queued {
            job_id: job,
            compare,
        })
    })
}

/// A person's placing of one plan row at 배치 확인: on `episode` (none: on
/// no episode), applied or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowPlacing {
    pub position: i64,
    pub episode: Option<i64>,
    pub apply: bool,
}

/// What came of a person's 배치 확인 ([`confirm_placement`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirmed {
    /// The job is queued again to keep and apply its rows: how many are to
    /// be applied and how many stored only.
    Queued {
        applied: usize,
        stored: usize,
    },
    NotFound,
    /// The job does not wait for its placement now.
    NotWaiting,
    /// The rows placed are not the ones the job asks about now.
    Stale,
    /// Why the placing is not taken, for the person.
    Refused(String),
}

/// The rows of `job` a person places at its 배치 확인, and whether it is
/// the whole plan: an upload's or a find job's subtitles, or a relocation's
/// rows, before its first confirmation, else the rows it asks about (보류한
/// 줄).
pub fn placeable(facts: &JobFacts, rows: &[PlanRow]) -> (Vec<i64>, bool) {
    let whole = (facts.origin == crate::store::UPLOAD
        || facts.origin == crate::store::FIND
        || facts.origin == crate::store::RELOCATE)
        && facts.placement_confirmed_at.is_none();
    let positions = rows
        .iter()
        .filter(|r| match whole {
            true => r.kind == AssetKind::Subtitle && r.outcome.is_none(),
            false => r.question.is_some() && r.outcome != Some(Outcome::Dropped),
        })
        .map(|r| r.position)
        .collect();
    (positions, whole)
}

/// Applies a person's 배치 확인 of the job: `placings` places every row it
/// asks about ([`placeable`]), on an episode of the season (`1..=total`)
/// or none, applied or not. Of the rows applied on an episode, the first
/// format of the work's order is ([`crate::place::package::placing`]). A
/// row left on its planned episode keeps what put it there (a mapping's
/// `mapped`), a row it asked about too; one a person moved, or placed with
/// no planned episode, is `explicit`; a row already stored takes its
/// stored subtitle with it, or one of its new link when others use it
/// ([`place_stored`]). A relocation ([`crate::place::relocate`]) is
/// confirmed as planned: each row on its planned episode and applied (every
/// one of them, the format order aside), and `removals` naming each removal
/// it plans (another set is a plan the person did not see). In one synced transaction with its log line, the
/// rows are written, the job's placement is marked confirmed and the job
/// queued again. Taken only while the job waits for it: an upload, a find
/// job or a relocation waiting for its placement, or a job with rows it asks
/// about that neither runs, is held nor waits for an approval.
pub fn confirm_placement(
    c: &mut Connection,
    job_id: &str,
    placings: &[RowPlacing],
    removals: &[String],
    total: Option<u32>,
    now: Millis,
) -> Result<Confirmed, JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(facts) = job_facts(&tx, job_id)? else {
            return Ok(Confirmed::NotFound);
        };
        let (state, wait): (String, Option<String>) = tx.query_row(
            "SELECT state, wait FROM subtitle_jobs WHERE id = ?1",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let rows = plan(&tx, job_id)?;
        let (positions, whole) = placeable(&facts, &rows);
        let waiting = match whole {
            true => state == "waiting" && wait.as_deref() == Some("placement"),
            false => state != "running" && state != "held" && wait.as_deref() != Some("approval"),
        };
        // A package of fonts and attachments alone has no row to place, and
        // the person confirms keeping it all the same.
        if !waiting || (positions.is_empty() && !whole) {
            return Ok(Confirmed::NotWaiting);
        }
        let mut placed: Vec<i64> = placings.iter().map(|p| p.position).collect();
        placed.sort_unstable();
        let mut asked = positions.clone();
        asked.sort_unstable();
        if placed != asked {
            return Ok(Confirmed::Stale);
        }
        let relocation = facts.origin == crate::store::RELOCATE;
        let mut planned: Vec<String> = match relocation {
            true => crate::place::relocate::removals(&tx, job_id)?
                .into_iter()
                .filter(|r| r.state == RemovalState::Planned)
                .map(|r| r.id)
                .collect(),
            false => Vec::new(),
        };
        planned.sort_unstable();
        let mut named = removals.to_vec();
        named.sort_unstable();
        if planned != named {
            return Ok(Confirmed::Stale);
        }
        let row_at = |position: i64| rows.iter().find(|r| r.position == position).expect("asked");
        if relocation
            && placings.iter().any(|p| {
                !p.apply || p.episode != row_at(p.position).placed.as_ref().map(|q| q.episode)
            })
        {
            return Ok(Confirmed::Refused(
                "재배치는 표에 적힌 회차 그대로 확인해요.".to_owned(),
            ));
        }
        for p in placings {
            if let Some(episode) = p.episode {
                if episode < 1 || total.is_some_and(|n| episode > i64::from(n)) {
                    return Ok(Confirmed::Refused(match total {
                        Some(n) => format!("{episode}화는 이 시즌의 1–{n}화 밖이에요."),
                        None => format!("{episode}화는 회차가 될 수 없어요."),
                    }));
                }
            } else if p.apply {
                return Ok(Confirmed::Refused(format!(
                    "{}: 적용할 회차를 골라 주세요.",
                    row_at(p.position).name
                )));
            }
        }
        // Another row of the job applied, or to apply, on an episode a
        // placed row would be applied on.
        let fixed: Vec<i64> = rows
            .iter()
            .filter(|r| !positions.contains(&r.position) && r.action == PlanAction::Apply)
            .filter(|r| matches!(r.outcome, None | Some(Outcome::Applied | Outcome::NoVideo)))
            .filter_map(|r| r.placed.as_ref().map(|p| p.episode))
            .collect();
        if let Some(episode) = placings
            .iter()
            .filter(|p| p.apply)
            .filter_map(|p| p.episode)
            .find(|e| fixed.contains(e))
        {
            return Ok(Confirmed::Refused(format!(
                "이 작업이 {episode}화에 적용할 자막이 이미 있어요. 다른 회차로 두거나 적용하지 않음으로 둬 주세요."
            )));
        }
        let order = match &facts.work_id {
            Some(work) => format_order(&tx, work)?,
            None => Vec::new(),
        };
        // A relocation applies every row it shows: the copies it moves are
        // the ones the episodes had, whatever the format order says now.
        let decided = match relocation {
            true => placings.iter().map(|_| (PlanAction::Apply, None)).collect(),
            false => match crate::place::package::placing(
                &placings
                    .iter()
                    .map(|p| {
                        let row = row_at(p.position);
                        crate::place::package::Placing {
                            episode: p.episode,
                            apply: p.apply,
                            format: row.format.unwrap_or(SubtitleFormat::Other),
                            sha256: &row.sha256,
                        }
                    })
                    .collect::<Vec<_>>(),
                &order,
            ) {
                Ok(decided) => decided,
                Err(why) => return Ok(Confirmed::Refused(why)),
            },
        };
        let (mut applied, mut stored) = (0, 0);
        // The stored rows, and the link each is placed on.
        let mut moved: Vec<(&PlanRow, Option<Placed>)> = Vec::new();
        for (p, (action, note)) in placings.iter().zip(decided) {
            let row = row_at(p.position);
            let kept = row
                .placed
                .as_ref()
                .filter(|planned| Some(planned.episode) == p.episode);
            let (assignment, basis) = match (kept, p.episode) {
                (Some(planned), _) => (Some(planned.assignment), planned.basis),
                (None, Some(_)) => (Some(Assignment::Explicit), None),
                (None, None) => (None, None),
            };
            match action {
                PlanAction::Apply => applied += 1,
                _ => stored += 1,
            }
            tx.execute(
                "UPDATE subtitle_job_plan
                    SET episode = ?3, assignment = ?4, basis = ?5, action = ?6, question = NULL,
                        note = ?7,
                        outcome = CASE WHEN ?6 = 'store' AND stored_id IS NOT NULL
                                       THEN 'stored' END,
                        updated_at = ?8
                  WHERE job_id = ?1 AND position = ?2",
                params![
                    job_id,
                    p.position,
                    p.episode,
                    assignment.map(Assignment::code),
                    basis.map(Basis::code),
                    action,
                    note,
                    now
                ],
            )?;
            if row.stored_id.is_some() {
                let to = p
                    .episode
                    .zip(assignment)
                    .map(|(episode, assignment)| Placed {
                        episode,
                        assignment,
                        basis,
                    });
                moved.push((row, to));
            }
        }
        // A stored row's subtitle goes where the person placed it, once every
        // row is written: the rows of one stored subtitle placed on one link
        // move it as one.
        for (row, to) in &moved {
            if let Some(id) = &row.stored_id {
                place_stored(&tx, row, id, to.as_ref())?;
            }
        }
        let note = "배치를 확인했어요";
        let detail = match (relocation, placings.is_empty()) {
            (true, _) => format!(
                "옛 회차의 적용본 {}개를 지우고 새 회차에 {applied}개를 적용해요",
                removals.len()
            ),
            (false, true) => "폰트와 첨부만 보관해요".to_owned(),
            (false, false) => format!("적용 {applied}개 · 보관만 {stored}개"),
        };
        tx.execute(
            "UPDATE subtitle_jobs
                SET state = 'pending', wait = NULL, finished_at = NULL, note = ?2,
                    placement_confirmed_at = CASE WHEN ?4 THEN ?3 ELSE placement_confirmed_at END,
                    state_at = ?3, updated_at = ?3
              WHERE id = ?1",
            params![job_id, note, now, whole],
        )?;
        tx.execute(
            "INSERT INTO subtitle_job_steps (job_id, step, state, at, note)
             VALUES (?1, 'placement', 'done', ?2, ?3)
             ON CONFLICT (job_id, step) DO UPDATE
             SET state = 'done', at = excluded.at, note = excluded.note",
            params![job_id, now, detail],
        )?;
        tx.execute(
            "INSERT INTO subtitle_job_events (job_id, at, message, detail)
             VALUES (?1, ?2, ?3, ?4)",
            params![job_id, now, note, detail],
        )?;
        tx.commit()?;
        Ok(Confirmed::Queued { applied, stored })
    })
}

/// Puts the row's stored subtitle `stored` on the link `to` a person's 배치
/// 확인 gave the row (`None`: no episode), once the confirmation wrote every
/// row's link. One that no applied copy not removed names, and no other
/// plan row (of any job) on another link than `to`, is the row's alone and
/// moves, with the rows placed on `to` beside it. One that others name stays
/// on its link for them: a done row and an applied copy record what was put
/// where, and a replacement plan made from it for another job holds once
/// approved. The row takes the stored subtitle of its new link instead
/// ([`of_link`]), unless the one it has holds that link
/// ([`Placed::held_by`]), as a relocation's row confirmed as planned does.
fn place_stored(
    c: &Connection,
    row: &PlanRow,
    stored: &str,
    to: Option<&Placed>,
) -> rusqlite::Result<()> {
    let (episode, assignment, basis) = (
        to.map(|p| p.episode),
        to.map(|p| p.assignment.code()),
        to.and_then(|p| p.basis).map(Basis::code),
    );
    let shared: bool = c.query_row(
        "SELECT EXISTS (SELECT 1 FROM subtitle_job_plan
                         WHERE stored_id = ?1 AND (job_id <> ?2 OR position <> ?3)
                           AND NOT (episode IS ?4 AND assignment IS ?5 AND basis IS ?6))
             OR EXISTS (SELECT 1 FROM subtitle_applied
                         WHERE stored_id = ?1 AND removed_at IS NULL)",
        params![stored, row.job_id, row.position, episode, assignment, basis],
        |r| r.get(0),
    )?;
    if !shared {
        c.execute(
            "UPDATE subtitle_stored SET episode = ?2, assignment = ?3, basis = ?4
              WHERE id = ?1 AND cleaned_at IS NULL",
            params![stored, episode, assignment, basis],
        )?;
        return Ok(());
    }
    let Some(on) = on_stored(c, stored)? else {
        return Ok(());
    };
    let held = match to {
        Some(to) => to.held_by(on.episode, on.assignment, on.basis),
        None => on.episode.is_none(),
    };
    if held {
        return Ok(());
    }
    let id = of_link(c, row, stored, on, to)?;
    c.execute(
        "UPDATE subtitle_job_plan SET stored_id = ?3 WHERE job_id = ?1 AND position = ?2",
        params![row.job_id, row.position, id],
    )?;
    Ok(())
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

/// The library's files of one episode, as paths relative to the work
/// folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpisodeFiles {
    pub videos: Vec<String>,
    pub subtitles: Vec<String>,
}

/// The library's files of the season by episode (whatever zeros its text
/// has).
pub fn season_files(
    c: &Connection,
    work_id: &str,
    season: u32,
) -> rusqlite::Result<BTreeMap<i64, EpisodeFiles>> {
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
    let mut by_episode: BTreeMap<i64, EpisodeFiles> = BTreeMap::new();
    for row in rows {
        let (path, text, kind) = row?;
        let Some(episode) =
            trss_subtitles::episode::numeric_key(&text).and_then(|key| key.parse::<i64>().ok())
        else {
            continue;
        };
        let files = by_episode.entry(episode).or_default();
        match kind.as_str() {
            "video" => files.videos.push(path),
            _ => files.subtitles.push(path),
        }
    }
    Ok(by_episode)
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
