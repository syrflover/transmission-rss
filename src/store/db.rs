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

/// One step of the schema history.
enum Migration {
    /// A SQL script.
    Sql(&'static str),
    /// Code for a change SQL cannot express. It runs inside the migration's
    /// transaction, so an `Err` leaves the database at the previous version.
    Code(fn(&Connection) -> Result<(), DbError>),
}

impl Migration {
    fn apply(&self, conn: &Connection) -> Result<(), DbError> {
        match self {
            Migration::Sql(sql) => Ok(conn.execute_batch(sql)?),
            Migration::Code(run) => run(conn),
        }
    }
}

/// Embedded migrations. Migration `n` (1-based position in this list) moves
/// `PRAGMA user_version` from `n - 1` to `n`. Only append; never edit or
/// reorder an entry that has shipped.
const MIGRATIONS: &[Migration] = &[
    // 1: channels and rules
    Migration::Sql(include_str!("channels/schema.sql")),
    // 2: collection history and the worker's cycle marker
    Migration::Sql(include_str!("history/schema.sql")),
    // 3: optional channel display name
    Migration::Sql("ALTER TABLE channels ADD COLUMN name TEXT CHECK (name IS NULL OR name <> '');"),
    // 4: the worker's snapshots for the collection screen's status board
    Migration::Sql(include_str!("status/schema.sql")),
    // 5: commands the web accepts and the worker carries out
    Migration::Sql(include_str!("commands/schema.sql")),
    // 6: a command whose Transmission add got no answer
    Migration::Sql(include_str!("commands/add_unconfirmed.sql")),
    // 7: the app-wide collect folder replaces the channels' base folders
    Migration::Code(super::settings::fold_base_dirs),
    // 8: watch folders and the works, seasons, episodes and files found in them
    Migration::Sql(include_str!("library/schema.sql")),
    // 9: work folders seen but not readable yet, so a folder's baseline is per work
    Migration::Sql(include_str!("library/unread_works.sql")),
    // 10: watch folders the app registers for the collect and archive folders
    Migration::Sql(include_str!("library/automatic.sql")),
    // 11: work artwork: the selection, the image files the app made, AniList's pace
    Migration::Sql(include_str!("artwork/schema.sql")),
    // 12: what the worker's inotify watches could not cover in a watch folder
    Migration::Sql(include_str!("library/watch_note.sql")),
    // 13: season info: the AniList entries linked to each local season
    Migration::Sql(include_str!("seasons/schema.sql")),
    // 14: unregistered watch folders keep their works instead of deleting them
    Migration::Sql(include_str!("library/unregistered.sql")),
    // 15: Anissia: the schedule snapshot of subscribed anime, rule subscriptions, the request pace
    Migration::Sql(include_str!("anissia/schema.sql")),
    // 16: a rule can be paused (`영상 받기` off); a subscription notes a season that is taken
    Migration::Sql(include_str!("channels/paused.sql")),
    // 17: the history's items found by the rule that received them
    Migration::Sql(include_str!("history/by_rule.sql")),
    // 18: a number that changes when a video lookup in the library could answer differently
    Migration::Sql(include_str!("library/generation.sql")),
    // 19: when a rule was last turned back on, so what it missed while off is left to the user
    Migration::Sql(include_str!("channels/resumed.sql")),
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
    /// A migration that runs code could not preserve the data's meaning.
    #[error("cannot migrate the database: {0}")]
    Migration(String),
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

/// Creates a database at `path` left as a build with its first `n` migrations
/// would leave it, for tests of the migrations after them.
#[cfg(test)]
pub(crate) fn database_at(path: &Path, n: usize) -> Connection {
    let conn = Connection::open(path).unwrap();
    for migration in &MIGRATIONS[..n] {
        migration.apply(&conn).unwrap();
    }
    conn.pragma_update(None, "user_version", n as i64).unwrap();
    conn
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
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(current) {
        migration.apply(&tx)?;
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
        "INSERT INTO channels (id, position, url, excludes, secret_query, version)
         VALUES ('c1', 0, 'http://x/', '[]', '[]', 1)";

    /// A channel as the schema had it before the collect folder replaced the
    /// channels' base folders (migrations 1 to 6).
    const INSERT_CHANNEL_WITH_BASE: &str =
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
            let conn = database_at(&path, 2);
            conn.execute(INSERT_CHANNEL_WITH_BASE, []).unwrap();
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
            let conn = database_at(&path, 3);
            conn.execute(INSERT_CHANNEL_WITH_BASE, []).unwrap();
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
    async fn database_from_before_unconfirmed_adds_keeps_its_commands_as_confirmed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with five migrations left it, one command ended.
            let conn = database_at(&path, 5);
            conn.execute(
                "INSERT INTO commands (id, kind, payload, state, attempts, created_at,
                     updated_at, finished_at, outcome)
                 VALUES ('cmd-1', 'receive_once', '{}', 'failed', 1, 1, 2, 2,
                     '{\"result\":\"add_failed\",\"reason\":\"x\"}')",
                [],
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let row: (String, i64) = db
            .run::<_, DbError, _>(|c| {
                Ok(c.query_row(
                    "SELECT state, add_unconfirmed FROM commands WHERE id = 'cmd-1'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(row, ("failed".to_owned(), 0));
    }

    #[tokio::test]
    async fn a_database_from_before_the_library_keeps_every_row_through_the_library_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with seven migrations left it, with data
            // in every table the library migrations must leave alone.
            let conn = database_at(&path, 7);
            conn.execute_batch(
                "INSERT INTO channels (id, position, url, excludes, secret_query, version)
                     VALUES ('c1', 0, 'http://x/feed', '[\"[Batch]\"]', '[\"token\"]', 3);
                 INSERT INTO rules (id, channel_id, position, match_text, regex,
                         case_insensitive, directory, episode, episode_auto, state, version)
                     VALUES ('r1', 'c1', 0, 'Clevatess', 0, 1, 'Clevatess/Season 02', -24, 1,
                         'active', 2);
                 INSERT INTO history_items (id, channel_id, channel_label, identity_key, title,
                         link, first_seen_at, last_seen_at, result, result_at, rule_id,
                         torrent_hash)
                     VALUES (1, 'c1', 'feed', 'k1', 'Clevatess - 01', 'magnet:?xt=urn:btih:aa',
                         10, 20, 'received', 15, 'r1', 'aa');
                 INSERT INTO history_changes (item_id, changed_at, from_result, to_result)
                     VALUES (1, 15, 'new', 'received');
                 INSERT INTO collection_cycle (id, started_at, finished_at) VALUES (1, 100, 110);
                 INSERT INTO collection_settings (id, collect_folder, archive_folder, version)
                     VALUES (1, '/downloads/Shows (current)', '/downloads/Shows', 4);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let rows: Vec<String> = db
            .run::<_, DbError, _>(|c| {
                let one = |sql: &str| -> rusqlite::Result<String> {
                    c.query_row(sql, [], |r| r.get::<_, String>(0))
                };
                Ok(vec![
                    one(
                        "SELECT url || '|' || excludes || '|' || secret_query || '|' || version
                           FROM channels WHERE id = 'c1'",
                    )?,
                    one(
                        "SELECT match_text || '|' || directory || '|' || episode || '|' || state
                           || '|' || version FROM rules WHERE id = 'r1'",
                    )?,
                    one(
                        "SELECT title || '|' || result || '|' || rule_id || '|' || torrent_hash
                           FROM history_items WHERE id = 1",
                    )?,
                    one("SELECT from_result || '>' || to_result FROM history_changes")?,
                    one("SELECT started_at || '|' || finished_at FROM collection_cycle")?,
                    one(
                        "SELECT collect_folder || '|' || archive_folder || '|' || version
                           FROM collection_settings",
                    )?,
                ])
            })
            .await
            .unwrap();
        assert_eq!(
            rows,
            [
                "http://x/feed|[\"[Batch]\"]|[\"token\"]|3",
                "Clevatess|Clevatess/Season 02|-24|active|2",
                "Clevatess - 01|received|r1|aa",
                "new>received",
                "100|110",
                "/downloads/Shows (current)|/downloads/Shows|4",
            ]
        );

        // The library is there and empty: the collect and archive folders become
        // watch folders when the worker runs, not in the migration (the media may
        // not be mounted then).
        let (folders, works, pending): (i64, i64, i64) = db
            .run::<_, DbError, _>(|c| {
                Ok(c.query_row(
                    "SELECT (SELECT count(*) FROM watch_folders), (SELECT count(*) FROM works),
                            (SELECT count(*) FROM unread_works)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!((folders, works, pending), (0, 0, 0));
        // A folder registered afterwards is not automatic unless it says so.
        let automatic: i64 = db
            .run::<_, DbError, _>(|c| {
                c.execute(
                    "INSERT INTO watch_folders (id, path, created_at) VALUES ('w', '/w', 1)",
                    [],
                )?;
                Ok(c.query_row("SELECT automatic FROM watch_folders", [], |r| r.get(0))?)
            })
            .await
            .unwrap();
        assert_eq!(automatic, 0);
    }

    #[tokio::test]
    async fn a_database_from_before_resume_times_keeps_its_rules_with_no_resume_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with eighteen migrations left it: a paused
            // rule and an active one.
            let conn = database_at(&path, 18);
            conn.execute_batch(
                "INSERT INTO channels (id, position, url, excludes, secret_query, version)
                     VALUES ('c1', 0, 'http://x/feed', '[]', '[]', 1);
                 INSERT INTO rules (id, channel_id, position, match_text, regex,
                         case_insensitive, directory, episode, episode_auto, state, version)
                     VALUES ('r1', 'c1', 0, 'Clevatess', 0, 1, 'Clevatess/Season 02', 1, 0,
                         'active', 2),
                            ('r2', 'c1', 1, 'Old', 0, 0, 'Old', 0, 0, 'paused', 5);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (rules, stamped): (String, i64) = db
            .run::<_, DbError, _>(|c| {
                Ok(c.query_row(
                    "SELECT group_concat(id || ':' || state || ':' || version, ','),
                            count(resumed_at)
                       FROM rules",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!((rules.as_str(), stamped), ("r1:active:2,r2:paused:5", 0));
    }

    #[tokio::test]
    async fn a_database_from_before_subscriptions_keeps_its_rules_as_plain_rules() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with fourteen migrations left it, one rule in.
            let conn = database_at(&path, 14);
            conn.execute_batch(
                "INSERT INTO channels (id, position, url, excludes, secret_query, version)
                     VALUES ('c1', 0, 'http://x/feed', '[]', '[]', 1);
                 INSERT INTO rules (id, channel_id, position, match_text, regex,
                         case_insensitive, directory, episode, episode_auto, state, version)
                     VALUES ('r1', 'c1', 0, 'Clevatess', 0, 1, 'Clevatess/Season 02', 1, 0,
                         'active', 2);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (rule, subscriptions, snapshots): (String, i64, i64) = db
            .run::<_, DbError, _>(|c| {
                Ok(c.query_row(
                    "SELECT (SELECT match_text || '|' || version FROM rules WHERE id = 'r1'),
                            (SELECT count(*) FROM rule_subscriptions),
                            (SELECT count(*) FROM anissia_anime)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(
            (rule.as_str(), subscriptions, snapshots),
            ("Clevatess|2", 0, 0)
        );
    }

    #[tokio::test]
    async fn a_database_from_before_paused_rules_keeps_rules_and_subscriptions_and_accepts_paused()
    {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with fifteen migrations left it: an
            // active rule, an archived one and a subscription of the first.
            let conn = database_at(&path, 15);
            conn.execute_batch(
                "INSERT INTO channels (id, position, url, excludes, secret_query, version)
                     VALUES ('c1', 0, 'http://x/feed', '[]', '[]', 1);
                 INSERT INTO rules (id, channel_id, position, match_text, regex,
                         case_insensitive, directory, episode, episode_auto, state, version)
                     VALUES ('r1', 'c1', 0, 'Clevatess', 0, 1, 'Clevatess/Season 02', 1, 0,
                         'active', 2),
                            ('r2', 'c1', 1, 'Old', 0, 0, 'Old', 0, 0, 'archived', 5);
                 INSERT INTO anissia_anime (anime_no, subject, week, status, fetched_at)
                     VALUES (7, '클레바테스', 1, 'ON', 10);
                 INSERT INTO rule_subscriptions (rule_id, anissia_anime_no, subtitles, creator,
                         season_id, subscribed_at)
                     VALUES ('r1', 7, 'follow', 'SubKor', 'w1:2', 99);",
            )
            .unwrap();
        }

        // Foreign keys are on for a connection the app opens.
        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (rules, subscription, blocked, broken): (String, String, Option<String>, i64) = db
            .run::<_, DbError, _>(|c| {
                let rules = c.query_row(
                    "SELECT group_concat(id || ':' || state || ':' || version, ',')
                       FROM (SELECT * FROM rules ORDER BY position)",
                    [],
                    |r| r.get(0),
                )?;
                let (subscription, blocked) = c.query_row(
                    "SELECT anissia_anime_no || '|' || subtitles || '|' || creator || '|'
                            || season_id || '|' || subscribed_at, season_blocked
                       FROM rule_subscriptions WHERE rule_id = 'r1'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                let broken =
                    c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                        r.get(0)
                    })?;
                Ok((rules, subscription, blocked, broken))
            })
            .await
            .unwrap();
        assert_eq!(rules, "r1:active:2,r2:archived:5");
        assert_eq!(subscription, "7|follow|SubKor|w1:2|99");
        assert_eq!(blocked, None);
        assert_eq!(broken, 0);

        // A rule can now be paused, and the subscription still goes with its rule.
        let left: i64 = db
            .run::<_, DbError, _>(|c| {
                c.execute("UPDATE rules SET state = 'paused' WHERE id = 'r1'", [])?;
                assert!(c
                    .execute("UPDATE rules SET state = 'stopped' WHERE id = 'r1'", [])
                    .is_err());
                c.execute("DELETE FROM rules WHERE id = 'r1'", [])?;
                Ok(c.query_row("SELECT count(*) FROM rule_subscriptions", [], |r| r.get(0))?)
            })
            .await
            .unwrap();
        assert_eq!(left, 0);
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
