//! The migration that replaces `channels.base_dir` with the collect folder.
//!
//! It runs inside the migration transaction (see `store::db`), once, on a
//! database whose channels still carry a base folder each:
//!
//! 1. the settings table is created;
//! 2. the channels' base folders are folded into one collect folder by
//!    [`crate::folders::fold_bases`]: the shared folder when all are the same,
//!    otherwise their longest common ancestor, with each channel's remaining
//!    part put in front of its rules' directories. No channels leave the
//!    collect folder unset;
//! 3. the column is dropped.
//!
//! What a rule saves to is `base.join(directory)` before and
//! `collect.join(directory')` after, and `fold_bases` guarantees the two are
//! the same text. This migration re-checks that for every rule before it
//! writes, and fails (changing nothing: the whole migration is one transaction)
//! rather than move a rule's save path.

use std::path::Path;

use rusqlite::{params, Connection};

use super::SCHEMA;
use crate::folders::{fold_bases, prefixed};
use crate::store::db::DbError;

struct ChannelBase {
    id: String,
    base_dir: String,
}

struct RuleDirectory {
    id: String,
    channel_id: String,
    directory: String,
}

pub(crate) fn fold_base_dirs(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(SCHEMA)?;

    let channels: Vec<ChannelBase> = {
        let mut stmt = conn.prepare("SELECT id, base_dir FROM channels ORDER BY position, id")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ChannelBase {
                    id: row.get(0)?,
                    base_dir: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };

    let bases: Vec<&str> = channels.iter().map(|c| c.base_dir.as_str()).collect();
    let folding = fold_bases(&bases).map_err(|_| {
        DbError::Migration(
            "the channels' base folders share no parent folder, so they cannot become one \
             collect folder without changing where rules save; make the channels' folders \
             absolute paths and start again"
                .to_owned(),
        )
    })?;

    if let Some(folding) = folding {
        let rules: Vec<RuleDirectory> = {
            let mut stmt = conn.prepare("SELECT id, channel_id, directory FROM rules")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(RuleDirectory {
                        id: row.get(0)?,
                        channel_id: row.get(1)?,
                        directory: row.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };

        for rule in &rules {
            let index = channels
                .iter()
                .position(|c| c.id == rule.channel_id)
                .ok_or_else(|| {
                    DbError::Migration("a rule belongs to a channel that does not exist".to_owned())
                })?;
            let prefix = &folding.prefixes[index];
            if prefix.is_empty() {
                continue;
            }
            let directory = prefixed(prefix, &rule.directory);
            let before = Path::new(&channels[index].base_dir).join(&rule.directory);
            let after = Path::new(&folding.collect).join(&directory);
            if before.as_os_str() != after.as_os_str() {
                return Err(DbError::Migration(
                    "a rule's save path would change".to_owned(),
                ));
            }
            conn.execute(
                "UPDATE rules SET directory = ?2, version = version + 1 WHERE id = ?1",
                params![rule.id, directory],
            )?;
        }

        conn.execute(
            "INSERT INTO collection_settings (id, collect_folder, archive_folder, version)
             VALUES (1, ?1, NULL, 1)",
            [&folding.collect],
        )?;
    }

    conn.execute_batch("ALTER TABLE channels DROP COLUMN base_dir;")?;
    Ok(())
}
