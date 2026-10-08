//! A rule's episode offset when the app sets it (`docs/specs/collection.md`,
//! 영상 회차 변환), and the user's `적용` of a suggestion.
//!
//! The offset is `rules.episode`, and `rules.episode_auto` says the app chose
//! it. What goes with an automatic value is kept beside it (see the
//! migration), not on [`Rule`]: the sentence that says why
//! (`episode_basis`), the value it replaced (`episode_previous`, for
//! `되돌리기`), and that the app has decided the rule once
//! (`episode_decided`). Only the rule's detail and the worker read them.
//!
//! The app decides a rule once: it sets an offset only on a rule whose offset
//! it has never set ([`EpisodeMark::decided`]) and that is not automatic
//! already, whatever value the field holds, and only if the rule is at the
//! version the caller read. A rule the user has saved since is left alone, and
//! a value the user changed or undid after the app set one is never replaced.

use std::collections::HashMap;

use rusqlite::{params, params_from_iter, Connection, TransactionBehavior};

use crate::store::channels::{repo, ChannelError, ChannelStore, Rule, Version};

type Result<T> = std::result::Result<T, ChannelError>;

/// How many rule IDs one query binds.
const CHUNK: usize = 400;

/// What is kept of a rule's offset beside the value (see the module docs).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpisodeMark {
    /// Why the app set the offset; only while it is automatic.
    pub basis: Option<String>,
    /// The offset before the app set its own; only while it is automatic.
    pub previous: Option<i64>,
    /// The app has set the rule's offset once and does not decide it again.
    pub decided: bool,
}

/// Sets the offset of rule `id` as the app's own if the rule is still at
/// `expected`, is not automatic and the app has not decided it before; the
/// value it had is kept as the one `되돌리기` puts back. `None` when any of
/// that is not so.
fn set_auto(
    conn: &mut Connection,
    id: &str,
    expected: Version,
    offset: i64,
    basis: &str,
) -> Result<Option<Rule>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = tx
        .prepare_cached(
            "UPDATE rules
            SET episode_previous = episode, episode = ?3, episode_auto = 1, episode_decided = 1,
                episode_basis = ?4, version = version + 1
          WHERE id = ?1 AND version = ?2 AND episode_auto = 0 AND episode_decided = 0",
        )?
        .execute(params![id, expected, offset, basis])?;
    let rule = if changed == 1 {
        repo::get_rule(&tx, id)?
    } else {
        None
    };
    tx.commit()?;
    Ok(rule)
}

/// The user's offset for rule `id`, if the rule is still at `expected`: it is
/// the user's own value from then on (`episode_auto` off, nothing kept with it).
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
        tx.prepare_cached(
            "UPDATE rules
                SET episode = ?2, episode_auto = 0, episode_basis = NULL, episode_previous = NULL,
                    version = version + 1
              WHERE id = ?1",
        )?
        .execute(params![id, episode])?;
    }
    let updated = repo::get_rule(&tx, id)?.expect("the rule still exists");
    tx.commit()?;
    Ok(updated)
}

/// What is kept of the offsets of the given rules, by rule ID. A rule that is
/// not stored is absent.
fn marks(conn: &Connection, rule_ids: &[String]) -> Result<HashMap<String, EpisodeMark>> {
    let mut found = HashMap::new();
    for chunk in rule_ids.chunks(CHUNK) {
        let marks = vec!["?"; chunk.len()].join(", ");
        let mut stmt = conn.prepare(&format!(
            "SELECT id, episode_auto, episode_basis, episode_previous, episode_decided FROM rules
              WHERE id IN ({marks})"
        ))?;
        let rows = stmt
            .query_map(params_from_iter(chunk), |row| {
                let auto: bool = row.get(1)?;
                Ok((
                    row.get::<_, String>(0)?,
                    EpisodeMark {
                        basis: if auto { row.get(2)? } else { None },
                        previous: if auto { row.get(3)? } else { None },
                        decided: row.get(4)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        found.extend(rows);
    }
    Ok(found)
}

impl ChannelStore {
    /// Sets the episode offset of rule `id` for the app, with the sentence that
    /// says why: only a rule at `expected_version` that is not automatic and
    /// that the app has not decided before takes it, whatever value it holds
    /// (which is kept for `되돌리기`). The rule as it is now, or `None` when it
    /// was not taken.
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

    /// What is kept of the offsets of the given rules, by rule ID (see
    /// [`EpisodeMark`]).
    pub async fn episode_marks(
        &self,
        rule_ids: Vec<String>,
    ) -> Result<HashMap<String, EpisodeMark>> {
        self.db.run(move |c| marks(c, &rule_ids)).await
    }
}
