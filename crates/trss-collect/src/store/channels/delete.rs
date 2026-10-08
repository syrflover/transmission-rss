//! Deleting a channel together with its rules.
//!
//! Received files and the collection history are not owned by the channel, so
//! they stay: the history table has no foreign key to `channels`, and nothing
//! here touches files.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::store::channels::{ChannelError, ChannelStore, Version};

impl ChannelStore {
    /// Deletes a channel and all its rules in one transaction, if the channel
    /// is still at `expected_version` and still has exactly `expected_rules`
    /// rules (archived ones included). Returns how many rules went with it.
    ///
    /// `expected_rules` is the number the caller showed the user when asking
    /// for confirmation. A rule added since then changes the count, and the
    /// delete is refused with [`ChannelError::OrderMismatch`] (`kind: "rule"`,
    /// a conflict), so the confirmation never understates what is removed.
    pub async fn delete_channel(
        &self,
        id: &str,
        expected_version: Version,
        expected_rules: usize,
    ) -> Result<usize, ChannelError> {
        let id = id.to_owned();
        self.db
            .run(move |c| delete_channel(c, &id, expected_version, expected_rules))
            .await
    }
}

fn delete_channel(
    conn: &mut Connection,
    id: &str,
    expected_version: Version,
    expected_rules: usize,
) -> Result<usize, ChannelError> {
    // Same locking as every other mutation: the write lock is taken before the
    // version check, so nothing can slip in between the check and the delete.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: Version = tx
        .prepare_cached("SELECT version FROM channels WHERE id = ?1")?
        .query_row([id], |r| r.get(0))
        .optional()?
        .ok_or_else(|| ChannelError::NotFound {
            kind: "channel",
            id: id.to_owned(),
        })?;
    if version != expected_version {
        return Err(ChannelError::Conflict {
            kind: "channel",
            id: id.to_owned(),
            expected: expected_version,
            actual: version,
        });
    }
    let rules: i64 = tx
        .prepare_cached("SELECT count(*) FROM rules WHERE channel_id = ?1")?
        .query_row([id], |r| r.get(0))?;
    if rules as usize != expected_rules {
        return Err(ChannelError::OrderMismatch { kind: "rule" });
    }
    tx.prepare_cached("DELETE FROM rules WHERE channel_id = ?1")?
        .execute([id])?;
    tx.prepare_cached("DELETE FROM channels WHERE id = ?1")?
        .execute([id])?;
    tx.commit()?;
    Ok(rules as usize)
}

#[cfg(test)]
#[path = "delete_tests.rs"]
mod tests;
