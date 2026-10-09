//! The stored episode mappings of the subtitle sources (`docs/specs/library.md`,
//! 자막의 회차 대응): the rows of `subtitle_episode_mappings`, their exceptions
//! and conflicts, and the writes that decide, set and revert them.
//!
//! What a mapping is, how the app decides one from the times the creator
//! posted, and which episodes do not fit it is `trss_library::mapping`, which
//! this module calls and re-exports, so that the callers of a stored mapping
//! read one module.
//!
//! # The version
//!
//! Every write that changes a row gives it the next value of one counter
//! shared by all rows (`subtitle_mapping_clock`); a save or a revert carries the
//! version the screen read and is refused with the row now stored when that is
//! another. A source with no row is version 0. The counter never gives a value
//! twice, so a row deleted by a revert and made again by the app is not the
//! version a screen still holds.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use trss_core::Millis;
pub use trss_library::mapping::{
    conflicts, decide, offsets, whole, Conflict, Decided, Exception, InvalidMapping, Mapped,
    Mapping, MappingKind, Posted, Season, UserMapping, MAX_EXCEPTIONS, USER_EVIDENCE,
};
use trss_library::mapping::{reconcile, Reconciled};

const MAPPING_COLUMNS: &str =
    "source_id, kind, episode_offset, evidence, decided_at, retired_offset, version";

fn mapping_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<(String, Mapping, Option<i64>)> {
    let code: String = r.get(1)?;
    let kind = MappingKind::parse(&code).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            format!("unknown mapping kind {code:?}").into(),
        )
    })?;
    Ok((
        r.get(0)?,
        Mapping {
            kind,
            offset: r.get(2)?,
            evidence: r.get(3)?,
            decided_at: r.get(4)?,
            version: r.get(6)?,
            exceptions: Vec::new(),
        },
        r.get(5)?,
    ))
}

/// The exceptions of season `season` of the work, by source, each ordered by
/// its text.
fn exceptions_in(
    c: &Connection,
    work_id: &str,
    season: u32,
) -> rusqlite::Result<HashMap<String, Vec<Exception>>> {
    let mut stmt = c.prepare_cached(
        "SELECT source_id, episode_key, episode, target FROM subtitle_episode_exceptions
          WHERE work_id = ?1 AND season = ?2 ORDER BY episode_key",
    )?;
    let rows = stmt.query_map(params![work_id, season], |r| {
        Ok((
            r.get::<_, String>(0)?,
            Exception {
                key: r.get(1)?,
                episode: r.get(2)?,
                target: r.get(3)?,
            },
        ))
    })?;
    let mut by_source: HashMap<String, Vec<Exception>> = HashMap::new();
    for row in rows {
        let (source, exception) = row?;
        by_source.entry(source).or_default().push(exception);
    }
    Ok(by_source)
}

/// The mappings of season `season` of the work, by source.
pub fn read_in(
    c: &Connection,
    work_id: &str,
    season: u32,
) -> rusqlite::Result<HashMap<String, Mapping>> {
    let mut stmt = c.prepare_cached(&format!(
        "SELECT {MAPPING_COLUMNS} FROM subtitle_episode_mappings
          WHERE work_id = ?1 AND season = ?2"
    ))?;
    let rows = stmt.query_map(params![work_id, season], mapping_of)?;
    let mut mappings: HashMap<String, Mapping> = rows
        .map(|row| row.map(|(source, mapping, _)| (source, mapping)))
        .collect::<rusqlite::Result<_>>()?;
    for (source, exceptions) in exceptions_in(c, work_id, season)? {
        if let Some(mapping) = mappings.get_mut(&source) {
            mapping.exceptions = exceptions;
        }
    }
    Ok(mappings)
}

/// The mapping of one source in the season, with the offset it retired.
fn read_one(
    c: &Connection,
    work_id: &str,
    season: u32,
    source_id: &str,
) -> rusqlite::Result<Option<(Mapping, Option<i64>)>> {
    let row = c
        .prepare_cached(&format!(
            "SELECT {MAPPING_COLUMNS} FROM subtitle_episode_mappings
                  WHERE work_id = ?1 AND season = ?2 AND source_id = ?3"
        ))?
        .query_row(params![work_id, season, source_id], mapping_of)
        .optional()?;
    let Some((_, mut mapping, retired)) = row else {
        return Ok(None);
    };
    if let Some(exceptions) = exceptions_in(c, work_id, season)?.remove(source_id) {
        mapping.exceptions = exceptions;
    }
    Ok(Some((mapping, retired)))
}

/// The next value of the version counter (see the module docs).
fn next_version(c: &Connection) -> rusqlite::Result<i64> {
    c.prepare_cached("UPDATE subtitle_mapping_clock SET version = version + 1 RETURNING version")?
        .query_row([], |r| r.get(0))
}

/// Writes what the app decided for the source, unless the user set its
/// mapping or the same is stored already. Returns the mapping now stored.
///
/// The app never takes up another offset by itself: when the stored mapping is
/// `auto` and what is decided now is another offset, the mapping goes
/// `undecided` and says both. When it goes `undecided` from `auto` (that or any
/// other reason), the offset it had is kept as `retired_offset`, and while
/// that is set only the same offset is decided again; any other is refused the
/// same way.
///
/// A write that changes the row gives it the next version. Call it in a
/// write transaction when the user may save at the same time: the user's row
/// that is there then stays whatever was decided before.
pub fn store_in(
    c: &Connection,
    work_id: &str,
    season: u32,
    source_id: &str,
    decided: &Decided,
    now: Millis,
) -> rusqlite::Result<Mapping> {
    let stored = read_one(c, work_id, season, source_id)?;
    let mut retired: Option<i64> = None;
    if let Some((m, kept)) = &stored {
        match m.kind {
            MappingKind::User => return Ok(m.clone()),
            MappingKind::Auto => retired = m.offset,
            MappingKind::Undecided => retired = *kept,
        }
    }
    let Reconciled {
        kind,
        offset,
        evidence,
        retired,
    } = reconcile(retired, decided);
    if let Some((m, kept)) = &stored {
        if m.kind == kind && m.offset == offset && m.evidence == evidence && *kept == retired {
            return Ok(m.clone());
        }
    }
    let version = next_version(c)?;
    c.prepare_cached(
        "INSERT INTO subtitle_episode_mappings
             (work_id, season, source_id, kind, episode_offset, evidence, decided_at,
              retired_offset, version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (work_id, season, source_id) DO UPDATE SET
             kind = excluded.kind, episode_offset = excluded.episode_offset,
             evidence = excluded.evidence, decided_at = excluded.decided_at,
             retired_offset = excluded.retired_offset, version = excluded.version
         WHERE subtitle_episode_mappings.kind <> 'user'",
    )?
    .execute(params![
        work_id,
        season,
        source_id,
        kind.code(),
        offset,
        evidence,
        now,
        retired,
        version
    ])?;
    // The row as it is now: a user's mapping that came in between stays.
    Ok(read_one(c, work_id, season, source_id)?
        .map(|(m, _)| m)
        .unwrap_or(Mapping {
            kind,
            offset,
            evidence,
            decided_at: now,
            version,
            exceptions: Vec::new(),
        }))
}

/// What a save or a revert of the user's came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    /// Done; the mapping now stored (`None` after a revert).
    Done(Option<Mapping>),
    /// The row was not the version the screen read: what is stored now.
    Stale(Option<Mapping>),
    /// A revert of a mapping the user did not set: nothing to take back.
    NotTheUsers(Option<Mapping>),
}

/// Saves the mapping the user set for the source in the season, as one unit
/// (the offset with its exceptions): a `user` row that clears the offset the
/// app retired, with the next version. `version` is what the screen read (0 for
/// a source with no row); another is refused with the stored mapping. The
/// source's stored conflicts go, since they were found against the mapping
/// that was there; the follower writes them again at its next look. What
/// follows the mapping moves with it in the same transaction
/// ([`crate::place::relocate::reevaluate_in`]), within the season's `total`
/// episodes when known.
#[allow(clippy::too_many_arguments)]
pub fn set_user_in(
    c: &mut Connection,
    work_id: &str,
    season: u32,
    source_id: &str,
    version: i64,
    user: &UserMapping,
    total: Option<u32>,
    now: Millis,
) -> rusqlite::Result<Saved> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let stored = read_one(&tx, work_id, season, source_id)?.map(|(m, _)| m);
    if stored.as_ref().map_or(0, |m| m.version) != version {
        return Ok(Saved::Stale(stored));
    }
    let next = next_version(&tx)?;
    tx.prepare_cached(
        "INSERT INTO subtitle_episode_mappings
             (work_id, season, source_id, kind, episode_offset, evidence, decided_at,
              retired_offset, version)
         VALUES (?1, ?2, ?3, 'user', ?4, ?5, ?6, NULL, ?7)
         ON CONFLICT (work_id, season, source_id) DO UPDATE SET
             kind = 'user', episode_offset = excluded.episode_offset,
             evidence = excluded.evidence, decided_at = excluded.decided_at,
             retired_offset = NULL, version = excluded.version",
    )?
    .execute(params![
        work_id,
        season,
        source_id,
        user.offset,
        USER_EVIDENCE,
        now,
        next
    ])?;
    tx.prepare_cached(
        "DELETE FROM subtitle_episode_exceptions
          WHERE work_id = ?1 AND season = ?2 AND source_id = ?3",
    )?
    .execute(params![work_id, season, source_id])?;
    for e in &user.exceptions {
        tx.prepare_cached(
            "INSERT INTO subtitle_episode_exceptions
                 (work_id, season, source_id, episode_key, episode, target)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![
            work_id, season, source_id, e.key, e.episode, e.target
        ])?;
    }
    tx.prepare_cached(
        "DELETE FROM subtitle_mapping_conflicts
          WHERE work_id = ?1 AND season = ?2 AND source_id = ?3",
    )?
    .execute(params![work_id, season, source_id])?;
    crate::place::relocate::reevaluate_in(&tx, work_id, season, source_id, total, now)?;
    let saved = read_one(&tx, work_id, season, source_id)?.map(|(m, _)| m);
    tx.commit()?;
    Ok(Saved::Done(saved))
}

/// `자동으로 되돌리기`: deletes the user's mapping of the source with its
/// exceptions and the offset the app had retired, and the conflicts found
/// against it, so the app decides again at its next look. `version` is what the
/// screen read. A mapping the user did not set is not taken back.
pub fn revert_in(
    c: &mut Connection,
    work_id: &str,
    season: u32,
    source_id: &str,
    version: i64,
) -> rusqlite::Result<Saved> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let stored = read_one(&tx, work_id, season, source_id)?.map(|(m, _)| m);
    if stored.as_ref().map_or(0, |m| m.version) != version {
        return Ok(Saved::Stale(stored));
    }
    if stored.as_ref().is_none_or(|m| m.kind != MappingKind::User) {
        return Ok(Saved::NotTheUsers(stored));
    }
    // The exceptions go with the row.
    tx.prepare_cached(
        "DELETE FROM subtitle_episode_mappings
          WHERE work_id = ?1 AND season = ?2 AND source_id = ?3",
    )?
    .execute(params![work_id, season, source_id])?;
    tx.prepare_cached(
        "DELETE FROM subtitle_mapping_conflicts
          WHERE work_id = ?1 AND season = ?2 AND source_id = ?3",
    )?
    .execute(params![work_id, season, source_id])?;
    tx.commit()?;
    Ok(Saved::Done(None))
}

/// Makes the stored conflicts of the source in the season exactly `found`: an
/// episode that was a conflict already keeps its `found_at`.
pub fn store_conflicts(
    c: &Connection,
    work_id: &str,
    season: u32,
    source_id: &str,
    found: &[Conflict],
    now: Millis,
) -> rusqlite::Result<()> {
    let before: HashMap<String, (String, Millis)> = {
        let mut stmt = c.prepare_cached(
            "SELECT episode, reason, found_at FROM subtitle_mapping_conflicts
              WHERE work_id = ?1 AND season = ?2 AND source_id = ?3",
        )?;
        let rows = stmt.query_map(params![work_id, season, source_id], |r| {
            Ok((r.get(0)?, (r.get(1)?, r.get(2)?)))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let same = before.len() == found.len()
        && found.iter().all(|f| {
            before
                .get(&f.episode)
                .is_some_and(|(reason, _)| *reason == f.reason)
        });
    if same {
        return Ok(());
    }
    c.prepare_cached(
        "DELETE FROM subtitle_mapping_conflicts
          WHERE work_id = ?1 AND season = ?2 AND source_id = ?3",
    )?
    .execute(params![work_id, season, source_id])?;
    for f in found {
        c.prepare_cached(
            "INSERT INTO subtitle_mapping_conflicts
                 (work_id, season, source_id, episode, reason, found_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![
            work_id,
            season,
            source_id,
            f.episode,
            f.reason,
            before.get(&f.episode).map_or(now, |(_, at)| *at)
        ])?;
    }
    Ok(())
}

/// The stored conflicts of the source in the season, by episode text.
pub fn conflicts_in(
    c: &Connection,
    work_id: &str,
    season: u32,
    source_id: &str,
) -> rusqlite::Result<Vec<Conflict>> {
    let mut stmt = c.prepare_cached(
        "SELECT episode, reason FROM subtitle_mapping_conflicts
          WHERE work_id = ?1 AND season = ?2 AND source_id = ?3 ORDER BY episode",
    )?;
    let rows = stmt.query_map(params![work_id, season, source_id], |r| {
        Ok(Conflict {
            episode: r.get(0)?,
            reason: r.get(1)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests;
