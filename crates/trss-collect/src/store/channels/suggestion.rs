//! What archive suggestions keep in the database (`archive_suggestion.sql`):
//! when a rule started, and the grounds the user chose to keep collecting on.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, TransactionBehavior};

use crate::store::channels::ChannelError;
use trss_core::Millis;

type Result<T> = std::result::Result<T, ChannelError>;

/// When the app first had each rule (Unix ms), by rule ID. A rule from before
/// the stamp existed is absent.
pub fn rule_starts(conn: &Connection) -> Result<HashMap<String, Millis>> {
    let mut stmt = conn.prepare_cached("SELECT rule_id, started_at FROM rule_started")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Remembers that the user chose to keep collecting for `rule_id` on each of
/// `grounds`. A ground kept twice keeps its first time. A rule that is gone is
/// [`ChannelError::NotFound`].
pub fn keep_archive_grounds(
    conn: &mut Connection,
    rule_id: &str,
    grounds: &[String],
    at: Millis,
) -> Result<()> {
    if grounds.iter().any(|ground| ground.is_empty()) {
        return Err(ChannelError::Invalid("a ground must not be empty"));
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let exists: bool = tx
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM rules WHERE id = ?1)")?
        .query_row([rule_id], |row| row.get(0))?;
    if !exists {
        return Err(ChannelError::NotFound {
            kind: "rule",
            id: rule_id.to_owned(),
        });
    }
    for ground in grounds {
        tx.prepare_cached(
            "INSERT OR IGNORE INTO archive_suggestion_kept (rule_id, ground, kept_at)
             VALUES (?1, ?2, ?3)",
        )?
        .execute(params![rule_id, ground, at])?;
    }
    tx.commit()?;
    Ok(())
}

/// Every ground the user chose to keep collecting on, as `(rule ID, ground)`.
pub fn kept_archive_grounds(conn: &Connection) -> Result<HashSet<(String, String)>> {
    let mut stmt = conn.prepare_cached("SELECT rule_id, ground FROM archive_suggestion_kept")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
