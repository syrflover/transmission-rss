//! App-wide settings stored in the app database.
//!
//! Today that is the collection folders ([`CollectionSettings`]): the collect
//! folder that every rule saves under and the optional archive folder. Writes
//! take the version the caller last saw and fail with
//! [`SettingsError::Conflict`], changing nothing, if someone saved first.
//!
//! The store keeps only the shape (a non-empty text). What makes a folder usable
//! (it exists, the two do not contain each other, they share a filesystem) is
//! checked by the web, which can see the disk; the worker finds out whether it
//! can write when it writes.

mod migrate;

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

pub(crate) use migrate::fold_base_dirs;

use crate::db::{Db, DbError};

/// Schema of the settings tables, run by the migration that adds them.
pub(crate) const SCHEMA: &str = include_str!("../../migrations/settings/schema.sql");

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error(transparent)]
    Db(#[from] DbError),
    /// The caller's version is not the stored one; nothing was changed.
    #[error("the collection settings were changed by someone else (expected version {expected}, found {actual})")]
    Conflict { expected: i64, actual: i64 },
    #[error("{0}")]
    Invalid(&'static str),
}

impl From<rusqlite::Error> for SettingsError {
    fn from(e: rusqlite::Error) -> Self {
        SettingsError::Db(DbError::Sqlite(e))
    }
}

/// The two folders of the collection. Absent (`None` from the store) until the
/// collect folder is chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionSettings {
    /// Where rules save what they select; a rule's directory is relative to it.
    pub folder: String,
    /// Where archived works move to; `None` when archiving leaves folders alone.
    pub archive_folder: Option<String>,
    pub version: i64,
}

/// Async access to the settings. Cheap to clone.
#[derive(Clone)]
pub struct SettingsStore {
    db: Db,
}

impl SettingsStore {
    pub fn new(db: Db) -> Self {
        SettingsStore { db }
    }

    /// The collection settings, or `None` while the collect folder is unset.
    pub async fn collection(&self) -> Result<Option<CollectionSettings>, SettingsError> {
        self.db.run(|c| Ok(read_collection(c)?)).await
    }

    /// Stores both folders if the settings are still at `expected_version` (0
    /// when none exist yet) and returns them with their new version.
    pub async fn put_collection(
        &self,
        expected_version: i64,
        folder: String,
        archive_folder: Option<String>,
    ) -> Result<CollectionSettings, SettingsError> {
        self.db
            .run(move |c| {
                write_collection(
                    c,
                    expected_version,
                    &folder,
                    archive_folder.as_deref(),
                    |_| Ok(()),
                )
                .map(|(stored, ())| stored)
            })
            .await
    }

    /// [`SettingsStore::put_collection`] that also runs `then` inside the same
    /// transaction after the settings are written, for a change that must
    /// happen with the save or not at all (the watch folders of the collect and
    /// archive folders). If `then` fails nothing is saved.
    pub async fn put_collection_with<R, E, F>(
        &self,
        expected_version: i64,
        folder: String,
        archive_folder: Option<String>,
        then: F,
    ) -> Result<(CollectionSettings, R), E>
    where
        F: FnOnce(&Transaction<'_>) -> Result<R, E> + Send + 'static,
        R: Send + 'static,
        E: From<SettingsError> + From<DbError> + Send + 'static,
    {
        self.db
            .run(move |c| {
                write_collection(
                    c,
                    expected_version,
                    &folder,
                    archive_folder.as_deref(),
                    then,
                )
            })
            .await
    }
}

fn read_collection(conn: &Connection) -> rusqlite::Result<Option<CollectionSettings>> {
    conn.query_row(
        "SELECT collect_folder, archive_folder, version FROM collection_settings WHERE id = 1",
        [],
        |row| {
            Ok(CollectionSettings {
                folder: row.get(0)?,
                archive_folder: row.get(1)?,
                version: row.get(2)?,
            })
        },
    )
    .optional()
}

fn write_collection<R, E>(
    conn: &mut Connection,
    expected_version: i64,
    folder: &str,
    archive_folder: Option<&str>,
    then: impl FnOnce(&Transaction<'_>) -> Result<R, E>,
) -> Result<(CollectionSettings, R), E>
where
    E: From<SettingsError>,
{
    if folder.is_empty() || archive_folder == Some("") {
        return Err(SettingsError::Invalid("a folder cannot be empty").into());
    }
    let sql = |e: rusqlite::Error| E::from(SettingsError::from(e));
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let actual = read_collection(&tx)
        .map_err(sql)?
        .map_or(0, |settings| settings.version);
    if actual != expected_version {
        return Err(SettingsError::Conflict {
            expected: expected_version,
            actual,
        }
        .into());
    }
    tx.execute(
        "INSERT INTO collection_settings (id, collect_folder, archive_folder, version)
         VALUES (1, ?1, ?2, 1)
         ON CONFLICT (id) DO UPDATE SET
             collect_folder = excluded.collect_folder,
             archive_folder = excluded.archive_folder,
             version = version + 1",
        params![folder, archive_folder],
    )
    .map_err(sql)?;
    let stored = read_collection(&tx)
        .map_err(sql)?
        .expect("the row was just written");
    let result = then(&tx)?;
    tx.commit().map_err(sql)?;
    Ok((stored, result))
}

/// Sets the collect folder inside `tx` only while none is set (the archive
/// folder stays empty). Used by the legacy import, which adopts the file's
/// folder in the same transaction that imports the channels. A folder that is
/// set already is a [`SettingsError::Conflict`] (expected version 0).
pub fn set_collect_folder_if_unset(
    tx: &Transaction<'_>,
    folder: &str,
) -> Result<(), SettingsError> {
    if let Some(found) = read_collection(tx)? {
        return Err(SettingsError::Conflict {
            expected: 0,
            actual: found.version,
        });
    }
    tx.execute(
        "INSERT INTO collection_settings (id, collect_folder, archive_folder, version)
         VALUES (1, ?1, NULL, 1)",
        [folder],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
