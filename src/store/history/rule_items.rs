//! What history says of the first items a rule picked (ticket 0024: the
//! release a rule's episode offset is decided from).

use std::collections::HashSet;

use rusqlite::{params, params_from_iter, Connection};

use super::{HistoryError, HistoryStore};

type Result<T> = std::result::Result<T, HistoryError>;

/// How many rule IDs one query binds.
const CHUNK: usize = 400;

/// The rules among `rule_ids` that have an item recorded, of any result. The
/// record carries the rule once the rule picked the item, so a rule that is
/// absent has not picked anything yet. Reads through `history_items_by_rule`.
fn with_items(conn: &Connection, rule_ids: &[String]) -> Result<HashSet<String>> {
    let mut found = HashSet::new();
    for chunk in rule_ids.chunks(CHUNK) {
        let marks = vec!["?"; chunk.len()].join(", ");
        let mut stmt = conn.prepare(&format!(
            "SELECT DISTINCT rule_id FROM history_items WHERE rule_id IN ({marks})"
        ))?;
        let rows = stmt
            .query_map(params_from_iter(chunk), |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        found.extend(rows);
    }
    Ok(found)
}

/// The titles of the items the rule picked first: those whose result for the
/// rule was recorded at the earliest moment any was. A cycle records what it
/// picked at one moment, so these are the items of the cycle (or of the one
/// `다시 받기`) that first picked something for the rule.
fn first_titles(conn: &Connection, rule_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT title FROM history_items
          WHERE rule_id = ?1
            AND result_at = (SELECT MIN(result_at) FROM history_items WHERE rule_id = ?1)
          ORDER BY id",
    )?;
    let titles = stmt
        .query_map(params![rule_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(titles)
}

impl HistoryStore {
    /// The rules among `rule_ids` that have picked an item, of any result.
    pub async fn rules_with_items(&self, rule_ids: Vec<String>) -> Result<HashSet<String>> {
        self.db.run(move |c| with_items(c, &rule_ids)).await
    }

    /// The titles of the items the rule picked first (see [`first_titles`]);
    /// empty for a rule that has picked nothing.
    pub async fn first_titles_of_rule(&self, rule_id: &str) -> Result<Vec<String>> {
        let rule_id = rule_id.to_owned();
        self.db.run(move |c| first_titles(c, &rule_id)).await
    }
}
