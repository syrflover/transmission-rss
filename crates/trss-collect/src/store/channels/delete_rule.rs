//! Deleting rules.
//!
//! Received files and the collection history are not owned by the rule: the
//! history keeps the rule's ID as a plain value with no foreign key, so its
//! records stay and simply point at a rule that no longer exists.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::store::channels::{ChannelError, ChannelStore, Version};

impl ChannelStore {
    /// Deletes a rule if it is still at `expected_version`. The other rules of
    /// the channel keep their order (and their versions: nothing about them
    /// changed).
    pub async fn delete_rule(
        &self,
        id: &str,
        expected_version: Version,
    ) -> Result<(), ChannelError> {
        let id = id.to_owned();
        self.db
            .run(move |c| delete_rules(c, &[(id, expected_version)]))
            .await
    }

    /// Deletes the rules `rules`, each if it is still at the version given
    /// with it, in one transaction: all of them, or none when one of them is
    /// gone or at another version (the error is about the first such rule).
    pub async fn delete_rules(&self, rules: Vec<(String, Version)>) -> Result<(), ChannelError> {
        self.db.run(move |c| delete_rules(c, &rules)).await
    }
}

fn delete_rules(conn: &mut Connection, rules: &[(String, Version)]) -> Result<(), ChannelError> {
    // The write lock is taken before the version checks, like every mutation.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    delete_rules_in(&tx, rules)?;
    tx.commit()?;
    Ok(())
}

/// [`ChannelStore::delete_rules`] inside the write transaction `tx`, for a
/// write that goes with another (a command stored with it): an `Err` leaves
/// `tx` to be rolled back.
pub(crate) fn delete_rules_in(
    tx: &rusqlite::Transaction<'_>,
    rules: &[(String, Version)],
) -> Result<(), ChannelError> {
    for (id, expected_version) in rules {
        delete_in(tx, id, *expected_version)?;
    }
    Ok(())
}

fn delete_in(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    expected_version: Version,
) -> Result<(), ChannelError> {
    let version: Version = tx
        .prepare_cached("SELECT version FROM rules WHERE id = ?1")?
        .query_row([id], |r| r.get(0))
        .optional()?
        .ok_or_else(|| ChannelError::NotFound {
            kind: "rule",
            id: id.to_owned(),
        })?;
    if version != expected_version {
        return Err(ChannelError::Conflict {
            kind: "rule",
            id: id.to_owned(),
            expected: expected_version,
            actual: version,
        });
    }
    tx.prepare_cached("DELETE FROM rules WHERE id = ?1")?
        .execute([id])?;
    Ok(())
}

#[cfg(test)]
#[path = "delete_rule_tests.rs"]
mod tests;
