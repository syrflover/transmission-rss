//! A rule's episode offset when the app sets it (`docs/specs/collection.md`,
//! 영상 회차 변환), and the user's `적용` of a suggestion.
//!
//! The offset is `rules.episode`, and `rules.episode_auto` says the app chose
//! it. The sentence that says why is kept beside it (`episode_basis`, see the
//! migration), not on [`Rule`]: only the rule's detail reads it.
//!
//! The app sets an offset only on a rule that still has the value a new rule
//! has (`0` or `1`, both of which leave a release's number as it is) and was
//! not set by the app already, and only if the rule is at the version the
//! caller read. A rule the user has edited since is left alone: the user's
//! value is never overwritten by the app's logic.

use std::collections::HashMap;

use rusqlite::{params, params_from_iter, Connection, TransactionBehavior};

use super::{repo, ChannelError, ChannelStore, Rule, Version};

type Result<T> = std::result::Result<T, ChannelError>;

/// How many rule IDs one query binds.
const CHUNK: usize = 400;

/// Sets the offset of rule `id` as the app's own if the rule is still at
/// `expected`, has not had an offset set by the app and still has an offset
/// that changes nothing. `None` when any of that is not so.
fn set_auto(
    conn: &mut Connection,
    id: &str,
    expected: Version,
    offset: i64,
    basis: &str,
) -> Result<Option<Rule>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        "UPDATE rules
            SET episode = ?3, episode_auto = 1, episode_basis = ?4, version = version + 1
          WHERE id = ?1 AND version = ?2 AND episode_auto = 0 AND episode IN (0, 1)",
        params![id, expected, offset, basis],
    )?;
    let rule = if changed == 1 {
        repo::get_rule(&tx, id)?
    } else {
        None
    };
    tx.commit()?;
    Ok(rule)
}

/// The user's offset for rule `id`, if the rule is still at `expected`: it is
/// the user's own value from then on (`episode_auto` off, no grounds kept).
fn set_manual(conn: &mut Connection, id: &str, expected: Version, episode: i64) -> Result<Rule> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let rule = repo::get_rule(&tx, id)?.ok_or_else(|| ChannelError::NotFound {
        kind: "rule",
        id: id.to_owned(),
    })?;
    if rule.version != expected {
        return Err(ChannelError::Conflict {
            kind: "rule",
            id: id.to_owned(),
            expected,
            actual: rule.version,
        });
    }
    if rule.episode != episode || rule.episode_auto {
        tx.execute(
            "UPDATE rules
                SET episode = ?2, episode_auto = 0, episode_basis = NULL, version = version + 1
              WHERE id = ?1",
            params![id, episode],
        )?;
    }
    let updated = repo::get_rule(&tx, id)?.expect("the rule still exists");
    tx.commit()?;
    Ok(updated)
}

/// The grounds kept for the automatic offsets of the given rules, by rule ID.
fn bases(conn: &Connection, rule_ids: &[String]) -> Result<HashMap<String, String>> {
    let mut found = HashMap::new();
    for chunk in rule_ids.chunks(CHUNK) {
        let marks = vec!["?"; chunk.len()].join(", ");
        let mut stmt = conn.prepare(&format!(
            "SELECT id, episode_basis FROM rules
              WHERE episode_basis IS NOT NULL AND episode_auto = 1 AND id IN ({marks})"
        ))?;
        let rows = stmt
            .query_map(params_from_iter(chunk), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        found.extend(rows);
    }
    Ok(found)
}

impl ChannelStore {
    /// Sets the episode offset of rule `id` for the app, with the sentence that
    /// says why: only a rule at `expected_version` whose offset the app has
    /// not set and that still changes nothing (`0` or `1`) takes it. The
    /// rule as it is now, or `None` when it was not taken.
    pub async fn set_auto_episode(
        &self,
        id: &str,
        expected_version: Version,
        offset: i64,
        basis: &str,
    ) -> Result<Option<Rule>> {
        let (id, basis) = (id.to_owned(), basis.to_owned());
        self.db
            .run(move |c| set_auto(c, &id, expected_version, offset, &basis))
            .await
    }

    /// Sets the episode offset of rule `id` as the user's own value, if the
    /// rule is still at `expected_version` (`적용` of a suggestion).
    pub async fn set_episode(
        &self,
        id: &str,
        expected_version: Version,
        episode: i64,
    ) -> Result<Rule> {
        let id = id.to_owned();
        self.db
            .run(move |c| set_manual(c, &id, expected_version, episode))
            .await
    }

    /// The sentences that say why the app set the offsets of the given rules,
    /// by rule ID. A rule whose offset the app did not set is absent.
    pub async fn episode_bases(&self, rule_ids: Vec<String>) -> Result<HashMap<String, String>> {
        self.db.run(move |c| bases(c, &rule_ids)).await
    }
}
