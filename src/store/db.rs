//! Connection handling and versioned, embedded migrations.
//!
//! The database is a local SQLite file (WAL mode) that both binaries open.
//! It must not live on a network share (see ADR 0005).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, TransactionBehavior};

/// Environment variable naming the database file. Deployment configuration,
/// deliberately not part of the app settings.
pub const DB_PATH_ENV: &str = "TRSS_DB_PATH";

/// How long a connection waits for a lock held by the other process.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Embedded migrations. Migration `n` (1-based position in this list) moves
/// `PRAGMA user_version` from `n - 1` to `n`. Only append; never edit or
/// reorder an entry that has shipped.
const MIGRATIONS: &[&str] = &[
    // 1: channels and rules
    include_str!("channels/schema.sql"),
    // 2: collection history and the worker's cycle marker
    include_str!("history/schema.sql"),
    // 3: optional channel display name
    "ALTER TABLE channels ADD COLUMN name TEXT CHECK (name IS NULL OR name <> '');",
    // 4: commands the web accepts and the worker carries out
    include_str!("commands/schema.sql"),
];

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
    #[error("environment variable {DB_PATH_ENV} is not set")]
    PathNotConfigured,
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    SchemaTooNew { found: usize, supported: usize },
}

/// Handle to the app database. Cheap to clone; all clones share one
/// connection, so calls from one process are serialized. Another process
/// (the web or the worker) uses its own connection and waits on SQLite's lock.
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    /// Opens (creating if needed) the database at `path` and applies pending
    /// migrations. The parent directory must exist.
    pub async fn open(path: impl AsRef<Path>) -> Result<Db, DbError> {
        let path = path.as_ref().to_owned();
        tokio::task::spawn_blocking(move || Db::open_blocking(path)).await?
    }

    /// Same as [`Db::open`] with the path taken from [`DB_PATH_ENV`].
    pub async fn open_from_env() -> Result<Db, DbError> {
        let path = std::env::var_os(DB_PATH_ENV).ok_or(DbError::PathNotConfigured)?;
        Db::open(path).await
    }

    /// Blocking variant of [`Db::open`], for code outside a runtime.
    pub fn open_blocking(path: impl AsRef<Path>) -> Result<Db, DbError> {
        let mut conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        migrate(&mut conn)?;
        Ok(Db {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Runs `f` with the connection on a blocking thread, so async callers do
    /// not stall the runtime.
    pub async fn run<T, E, F>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<DbError> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            // A panic in an earlier call rolled its transaction back on unwind,
            // so the connection itself is still usable.
            let mut guard = conn.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut guard)
        })
        .await
        .map_err(|e| E::from(DbError::Task(e)))?
    }
}

/// Applies pending migrations inside one write transaction, so two processes
/// starting together apply each migration once.
fn migrate(conn: &mut Connection) -> Result<(), DbError> {
    let supported = MIGRATIONS.len();
    let current = user_version(conn)?;
    if current == supported {
        return Ok(());
    }
    check_supported(current, supported)?;

    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Another process may have migrated while this one waited for the lock.
    let current = user_version(&tx)?;
    check_supported(current, supported)?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
    }
    tx.commit()?;
    Ok(())
}

fn check_supported(found: usize, supported: usize) -> Result<(), DbError> {
    if found > supported {
        Err(DbError::SchemaTooNew { found, supported })
    } else {
        Ok(())
    }
}

fn user_version(conn: &Connection) -> Result<usize, rusqlite::Error> {
    conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map(|v| v as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSERT_CHANNEL: &str =
        "INSERT INTO channels (id, position, url, base_dir, excludes, secret_query, version)
         VALUES ('c1', 0, 'http://x/', '/d', '[]', '[]', 1)";

    async fn count_channels(db: &Db) -> i64 {
        db.run::<_, DbError, _>(|c| {
            Ok(c.query_row("SELECT count(*) FROM channels", [], |r| r.get(0))?)
        })
        .await
        .unwrap()
    }

    async fn version_of(db: &Db) -> usize {
        db.run::<_, DbError, _>(|c| Ok(user_version(c)?))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn empty_db_is_migrated_once_and_reopen_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");

        let db = Db::open(&path).await.unwrap();
        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let tables: Vec<String> = db
            .run::<_, DbError, _>(|c| {
                let mut stmt = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
                let names = stmt
                    .query_map([], |r| r.get(0))?
                    .collect::<Result<Vec<String>, _>>()?;
                Ok(names)
            })
            .await
            .unwrap();
        assert!(tables.contains(&"channels".to_owned()));
        assert!(tables.contains(&"rules".to_owned()));

        // Leave a row behind: a second open must not re-run the schema script,
        // which would fail on the existing tables.
        db.run::<_, DbError, _>(|c| Ok(c.execute(INSERT_CHANNEL, []).map(|_| ())?))
            .await
            .unwrap();
        drop(db);

        let db = Db::open(&path).await.unwrap();
        assert_eq!(count_channels(&db).await, 1);
        assert_eq!(version_of(&db).await, MIGRATIONS.len());
    }

    #[tokio::test]
    async fn database_from_before_the_channel_name_gets_an_unnamed_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with two migrations left it, one channel in.
            let conn = Connection::open(&path).unwrap();
            for sql in &MIGRATIONS[..2] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", 2_i64).unwrap();
            conn.execute("INSERT INTO channels (id, position, url, base_dir, excludes, secret_query, version) VALUES ('c1', 0, 'http://x/', '/d', '[]', '[]', 1)", [])
                .unwrap();
        }

        let db = Db::open(&path).await.unwrap();
        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let name: Option<String> = db
            .run::<_, DbError, _>(|c| {
                Ok(
                    c.query_row("SELECT name FROM channels WHERE id = 'c1'", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .await
            .unwrap();
        assert_eq!(name, None);
        // A blank name is not storable; the API and store turn it into NULL.
        let blank = db
            .run::<_, DbError, _>(|c| {
                Ok(c.execute("UPDATE channels SET name = ''", []).map(|_| ())?)
            })
            .await;
        assert!(blank.is_err());
    }

    #[tokio::test]
    async fn database_from_before_the_commands_gets_the_commands_table_and_keeps_its_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with three migrations left it, one channel in.
            let conn = Connection::open(&path).unwrap();
            for sql in &MIGRATIONS[..3] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", 3_i64).unwrap();
            conn.execute(INSERT_CHANNEL, []).unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        assert_eq!(count_channels(&db).await, 1);
        let commands: i64 = db
            .run::<_, DbError, _>(|c| {
                Ok(c.query_row("SELECT count(*) FROM commands", [], |r| r.get(0))?)
            })
            .await
            .unwrap();
        assert_eq!(commands, 0);
    }

    #[tokio::test]
    async fn newer_schema_than_the_build_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "user_version", (MIGRATIONS.len() + 1) as i64)
                .unwrap();
        }
        match Db::open(&path).await {
            Err(DbError::SchemaTooNew { .. }) => {}
            other => panic!("expected SchemaTooNew, got {:?}", other.map(|_| ())),
        }
    }

    #[tokio::test]
    async fn two_handles_share_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        let a = Db::open(&path).await.unwrap();
        let b = Db::open(&path).await.unwrap();
        a.run::<_, DbError, _>(|c| Ok(c.execute(INSERT_CHANNEL, []).map(|_| ())?))
            .await
            .unwrap();
        assert_eq!(count_channels(&b).await, 1);
    }
}
