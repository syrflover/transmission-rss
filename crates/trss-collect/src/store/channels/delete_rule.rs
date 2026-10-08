//! Deleting one rule.
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
            .run(move |c| delete_rule(c, &id, expected_version))
            .await
    }
}

fn delete_rule(
    conn: &mut Connection,
    id: &str,
    expected_version: Version,
) -> Result<(), ChannelError> {
    // The write lock is taken before the version check, like every mutation.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
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
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
#[path = "delete_rule_tests.rs"]
mod tests;
