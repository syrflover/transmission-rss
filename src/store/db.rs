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
    // 20: when a title-waiting subscription got its title; the title candidates the user rejected
    Migration::Sql(include_str!("channels/title_waiting.sql")),
    // 21: the weekly schedule: the torrents being downloaded and the cycle interval the worker
    //     leaves for the web, and the first run's checklist
    Migration::Sql(concat!(
        include_str!("status/week.sql"),
        include_str!("setup/schema.sql")
    )),
    // 22: when Anissia was found not to list an anime any more; the first run's steps and end
    //     kept, instead of read from the data each time
    Migration::Sql(concat!(
        include_str!("anissia/unlisted.sql"),
        include_str!("setup/ended.sql")
    )),
    // 23: the grounds of a rule's automatic episode offset
    Migration::Sql(include_str!("channels/episode_basis.sql")),
    // 24: when a rule started and which archive suggestion grounds the user chose to keep collecting
    Migration::Sql(include_str!("channels/archive_suggestion.sql")),
    // 25: the replacement of video revisions and how far each has come
    Migration::Sql(include_str!("revisions/schema.sql")),
    // 26: the pace of search requests to a feed host, shared by the web and the worker
    Migration::Sql(include_str!("search_pace/schema.sql")),
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
    use rusqlite::OptionalExtension;

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
    async fn a_database_from_before_title_waiting_keeps_its_subscriptions_with_no_title_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with nineteen migrations left it: a
            // subscription that waits for its title and one that has it.
            let conn = database_at(&path, 19);
            conn.execute_batch(
                "INSERT INTO channels (id, position, url, excludes, secret_query, version)
                     VALUES ('c1', 0, 'http://x/feed', '[]', '[]', 1);
                 INSERT INTO rules (id, channel_id, position, match_text, regex,
                         case_insensitive, directory, episode, episode_auto, state, version)
                     VALUES ('r1', 'c1', 0, NULL, 0, 0, 'Wait', 1, 0, 'active', 2),
                            ('r2', 'c1', 1, 'Clevatess', 0, 0, 'Clevatess', 1, 0, 'active', 3);
                 INSERT INTO anissia_anime (anime_no, subject, week, status, fetched_at)
                     VALUES (7, '기다림', 1, 'ON', 10), (8, '클레바테스', 1, 'ON', 10);
                 INSERT INTO rule_subscriptions (rule_id, anissia_anime_no, subtitles, creator,
                         season_id, subscribed_at)
                     VALUES ('r1', 7, 'undecided', NULL, NULL, 99),
                            ('r2', 8, 'undecided', NULL, NULL, 98);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (subscriptions, titled, rejected): (String, i64, i64) = db
            .run::<_, DbError, _>(|c| {
                // A rejection goes with its channel.
                c.execute(
                    "INSERT INTO rejected_titles (channel_id, title_key, work, rejected_at)
                     VALUES ('c1', 'new work', 'New Work', 1)",
                    [],
                )?;
                let rejected_before: i64 =
                    c.query_row("SELECT count(*) FROM rejected_titles", [], |r| r.get(0))?;
                let read = c.query_row(
                    "SELECT group_concat(rule_id || ':' || subscribed_at, ','), count(titled_at)
                       FROM rule_subscriptions",
                    [],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                )?;
                c.execute("DELETE FROM rules", [])?;
                c.execute("DELETE FROM channels", [])?;
                let rejected_after: i64 =
                    c.query_row("SELECT count(*) FROM rejected_titles", [], |r| r.get(0))?;
                assert_eq!(rejected_before, 1);
                Ok((read.0, read.1, rejected_after))
            })
            .await
            .unwrap();
        assert_eq!((subscriptions.as_str(), titled), ("r1:99,r2:98", 0));
        assert_eq!(rejected, 0);
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

    /// The migration that added `first_run` is number 21, the one after these.
    const BEFORE_FIRST_RUN: usize = 20;

    async fn first_run_rows(db: &Db) -> i64 {
        db.run::<_, DbError, _>(|c| {
            Ok(c.query_row("SELECT count(*) FROM first_run", [], |r| r.get(0))?)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_database_with_channels_or_folders_from_before_the_first_run_is_not_a_first_run() {
        for (name, data) in [
            ("channel", INSERT_CHANNEL),
            (
                "folder",
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('w', '/w', 1)",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("app.db");
            {
                let conn = database_at(&path, BEFORE_FIRST_RUN);
                conn.execute(data, []).unwrap();
            }

            let db = Db::open(&path).await.unwrap();

            assert_eq!(version_of(&db).await, MIGRATIONS.len(), "{name}");
            assert_eq!(first_run_rows(&db).await, 0, "{name}");
            let kept: i64 = db
                .run::<_, DbError, _>(|c| {
                    Ok(c.query_row(
                        "SELECT (SELECT count(*) FROM channels)
                                + (SELECT count(*) FROM watch_folders)",
                        [],
                        |r| r.get(0),
                    )?)
                })
                .await
                .unwrap();
            assert_eq!(kept, 1, "{name}");
        }
    }

    #[tokio::test]
    async fn an_empty_database_from_before_the_first_run_begins_one_and_gets_the_week_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            database_at(&path, BEFORE_FIRST_RUN);
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        assert_eq!(first_run_rows(&db).await, 1);
        // The tables the weekly schedule reads can be written and read.
        let (hashes, interval): (i64, i64) = db
            .run::<_, DbError, _>(|c| {
                c.execute(
                    "INSERT INTO transmission_downloading (hash) VALUES ('aa')",
                    [],
                )?;
                c.execute(
                    "INSERT INTO worker_info (id, cycle_interval_ms) VALUES (1, 300000)",
                    [],
                )?;
                Ok(c.query_row(
                    "SELECT (SELECT count(*) FROM transmission_downloading),
                            (SELECT cycle_interval_ms FROM worker_info)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!((hashes, interval), (1, 300_000));
    }

    /// The migration that kept the first run's steps and end is number 22.
    const BEFORE_ENDED: usize = 21;

    /// `(folder added, import applied, ended)` of the first run's row after the
    /// database from before the end was kept got `setup` done and was migrated.
    async fn first_run_after(setup: &[&str]) -> Option<(bool, bool, bool)> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            let conn = database_at(&path, BEFORE_ENDED);
            for sql in setup {
                conn.execute(sql, []).unwrap();
            }
        }
        let db = Db::open(&path).await.unwrap();
        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        db.run::<_, DbError, _>(|c| {
            Ok(c.query_row(
                "SELECT folder_added_at IS NOT NULL, import_applied_at IS NOT NULL,
                        ended_at IS NOT NULL
                   FROM first_run",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?)
        })
        .await
        .unwrap()
    }

    const INSERT_FOLDER: &str =
        "INSERT INTO watch_folders (id, path, created_at) VALUES ('w', '/w', 1)";
    const SKIP_IMPORT: &str = "UPDATE first_run SET import_skipped_at = 1";

    #[tokio::test]
    async fn a_first_run_that_had_not_started_is_untouched_by_the_kept_end() {
        assert_eq!(first_run_after(&[]).await, Some((false, false, false)));
    }

    #[tokio::test]
    async fn a_first_run_past_its_checklist_by_the_old_reading_stays_past_it() {
        // A folder and a channel were what ended the checklist: both steps are
        // done and the checklist is ended.
        assert_eq!(
            first_run_after(&[INSERT_FOLDER, INSERT_CHANNEL]).await,
            Some((true, true, true))
        );
        // A folder and a skipped import ended it too.
        assert_eq!(
            first_run_after(&[INSERT_FOLDER, SKIP_IMPORT]).await,
            Some((true, false, true))
        );
        // Both skipped.
        assert_eq!(
            first_run_after(&["UPDATE first_run SET import_skipped_at = 1, folder_skipped_at = 1"])
                .await,
            Some((false, false, true))
        );
    }

    #[tokio::test]
    async fn a_first_run_still_in_its_checklist_stays_in_it_with_the_steps_it_has() {
        assert_eq!(
            first_run_after(&[INSERT_FOLDER]).await,
            Some((true, false, false))
        );
        assert_eq!(
            first_run_after(&[INSERT_CHANNEL]).await,
            Some((false, true, false))
        );
        assert_eq!(
            first_run_after(&[SKIP_IMPORT]).await,
            Some((false, false, false))
        );
    }

    #[tokio::test]
    async fn a_snapshot_from_before_unlisted_is_listed_and_can_be_marked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            let conn = database_at(&path, BEFORE_ENDED);
            conn.execute(
                "INSERT INTO anissia_anime (anime_no, subject, week, status, fetched_at)
                 VALUES (7, '작품', 3, 'ON', 100)",
                [],
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (before, after): (Option<i64>, Option<i64>) = db
            .run::<_, DbError, _>(|c| {
                let read = |c: &Connection| {
                    c.query_row(
                        "SELECT unlisted_at FROM anissia_anime WHERE anime_no = 7",
                        [],
                        |r| r.get(0),
                    )
                };
                let before = read(c)?;
                c.execute("UPDATE anissia_anime SET unlisted_at = 5", [])?;
                Ok((before, read(c)?))
            })
            .await
            .unwrap();
        assert_eq!((before, after), (None, Some(5)));
    }

    /// The migration that added archive suggestions' tables is number 24.
    const BEFORE_ARCHIVE_SUGGESTIONS: usize = 23;

    #[tokio::test]
    async fn a_database_from_before_archive_suggestions_keeps_its_rows_and_stamps_new_rules() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with twenty-three migrations left it: a
            // rule that received an item.
            let conn = database_at(&path, BEFORE_ARCHIVE_SUGGESTIONS);
            conn.execute_batch(
                "INSERT INTO channels (id, position, url, excludes, secret_query, version)
                     VALUES ('c1', 0, 'http://x/feed', '[]', '[]', 1);
                 INSERT INTO rules (id, channel_id, position, match_text, regex,
                         case_insensitive, directory, episode, episode_auto, state, version)
                     VALUES ('r1', 'c1', 0, 'Clevatess', 0, 0, 'Clevatess', 1, 0, 'active', 3);
                 INSERT INTO history_items (channel_id, channel_label, identity_key, title, link,
                         first_seen_at, last_seen_at, result, result_at, rule_id)
                     VALUES ('c1', 'feed', 'title:a', 'Clevatess - 01', 'x', 10, 10, 'received',
                         20, 'r1');",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (rule, received, stamps, kept) = db
            .run::<_, DbError, _>(|c| {
                // The old rule is stamped by the upgrade, a rule made after it
                // by the trigger.
                c.execute(
                    "INSERT INTO rules (id, channel_id, position, match_text, regex,
                            case_insensitive, directory, episode, episode_auto, state, version)
                     VALUES ('r2', 'c1', 1, 'Other', 0, 0, 'Other', 1, 0, 'active', 1)",
                    [],
                )?;
                c.execute(
                    "INSERT INTO archive_suggestion_kept (rule_id, ground, kept_at)
                     VALUES ('r1', 'quiet:1', 5)",
                    [],
                )?;
                let rule: String = c.query_row(
                    "SELECT match_text || '|' || version FROM rules WHERE id = 'r1'",
                    [],
                    |r| r.get(0),
                )?;
                let received: String = c.query_row(
                    "SELECT rule_id || '|' || result_at FROM history_items",
                    [],
                    |r| r.get(0),
                )?;
                let stamps: String = c.query_row(
                    "SELECT group_concat(rule_id, ',') FROM rule_started",
                    [],
                    |r| r.get(0),
                )?;
                // The keep and the stamp go with the rule.
                c.execute("DELETE FROM rules", [])?;
                let kept: (i64, i64) = c.query_row(
                    "SELECT (SELECT count(*) FROM archive_suggestion_kept),
                            (SELECT count(*) FROM rule_started)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                Ok((rule, received, stamps, kept))
            })
            .await
            .unwrap();
        assert_eq!(rule, "Clevatess|3");
        assert_eq!(received, "r1|20");
        assert_eq!(stamps, "r1,r2");
        assert_eq!(kept, (0, 0));
    }

    #[tokio::test]
    async fn a_database_from_before_read_days_takes_the_28_days_up_to_each_last_success() {
        const DAY: i64 = 86_400_000;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with twenty-three migrations left it:
            // one channel read well, one whose last read failed after a
            // success long ago, and one that never worked.
            let conn = database_at(&path, BEFORE_ARCHIVE_SUGGESTIONS);
            conn.execute_batch(&format!(
                "INSERT INTO channel_read_status (channel_id, ok, read_at, ok_at) VALUES
                     ('fine', 1, {now}, {now}),
                     ('dead', 0, {now}, {then}),
                     ('never', 0, {now}, NULL);",
                now = 20_000 * DAY + 5,
                then = 19_000 * DAY + 5,
            ))
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let days: Vec<(String, i64, i64, i64)> = db
            .run::<_, DbError, _>(|c| {
                let mut stmt = c.prepare(
                    "SELECT channel_id, count(*), min(day), max(day)
                     FROM channel_read_days GROUP BY channel_id ORDER BY channel_id",
                )?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
            .unwrap();
        assert_eq!(
            days,
            [
                ("dead".to_owned(), 28, 19_000 - 27, 19_000),
                ("fine".to_owned(), 28, 20_000 - 27, 20_000),
            ],
            "a channel that never read successfully has none"
        );
    }

    /// The triggers that keep `rule_started` go with a rebuild of `rules`; a
    /// later migration that rebuilds it has to make them again.
    #[tokio::test]
    async fn the_rule_start_triggers_survive_every_migration() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        let triggers: String = db
            .run::<_, DbError, _>(|c| {
                Ok(c.query_row(
                    "SELECT group_concat(name, ',') FROM
                         (SELECT name FROM sqlite_master
                          WHERE type = 'trigger' AND tbl_name = 'rules'
                            AND name LIKE 'rule_started_%' ORDER BY name)",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(triggers, "rule_started_on_delete,rule_started_on_insert");
    }

    #[tokio::test]
    async fn an_install_that_was_not_a_first_run_still_is_not_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            let conn = database_at(&path, BEFORE_ENDED);
            conn.execute("DELETE FROM first_run", []).unwrap();
        }
        let db = Db::open(&path).await.unwrap();
        // Registering a folder later leaves no row behind.
        db.run::<_, DbError, _>(|c| Ok(c.execute(INSERT_FOLDER, []).map(|_| ())?))
            .await
            .unwrap();
        assert_eq!(first_run_rows(&db).await, 0);
    }

    /// The migration that added the grounds of an automatic episode offset
    /// follows the first run's end (22).
    const BEFORE_EPISODE_BASIS: usize = 22;

    #[tokio::test]
    async fn rules_from_before_episode_grounds_keep_their_offsets_and_take_grounds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            let conn = database_at(&path, BEFORE_EPISODE_BASIS);
            conn.execute_batch(
                "INSERT INTO channels (id, position, url, excludes, secret_query, version)
                 VALUES ('c1', 0, 'https://a.example/rss', '[]', '[]', 1);
                 INSERT INTO rules (id, channel_id, position, match_text, regex, case_insensitive,
                                    directory, episode, episode_auto, state, version)
                 VALUES ('typed', 'c1', 0, 'A', 0, 0, 'A/Season 01', -12, 0, 'active', 4),
                        ('derived', 'c1', 1, 'B', 0, 0, 'B/Season 01', -24, 1, 'active', 2);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let rows: Vec<(String, i64, bool, Option<String>, i64)> = db
            .run::<_, DbError, _>(|c| {
                c.execute(
                    "UPDATE rules SET episode_basis = '이전 시즌이 24화까지예요.' WHERE id = 'derived'",
                    [],
                )?;
                let mut stmt = c.prepare(
                    "SELECT id, episode, episode_auto, episode_basis, version FROM rules ORDER BY id",
                )?;
                let rows = stmt.query_map([], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?;
                Ok(rows.collect::<rusqlite::Result<_>>()?)
            })
            .await
            .unwrap();
        assert_eq!(
            rows,
            vec![
                (
                    "derived".to_owned(),
                    -24,
                    true,
                    Some("이전 시즌이 24화까지예요.".to_owned()),
                    2
                ),
                ("typed".to_owned(), -12, false, None, 4),
            ]
        );
        // A blank sentence is not a ground.
        let blank = db
            .run::<_, DbError, _>(|c| {
                Ok(
                    c.execute("UPDATE rules SET episode_basis = '' WHERE id = 'typed'", [])
                        .is_err(),
                )
            })
            .await
            .unwrap();
        assert!(blank);
    }

    /// The last version before the replacement of video revisions.
    const BEFORE_REVISIONS: usize = 24;

    #[tokio::test]
    async fn a_history_from_before_revisions_keeps_its_items_and_takes_revisions_of_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            let conn = database_at(&path, BEFORE_REVISIONS);
            conn.execute(
                "INSERT INTO history_items (id, channel_id, channel_label, identity_key, title,
                     link, first_seen_at, last_seen_at, result, result_at, rule_id, torrent_hash)
                 VALUES (7, 'c1', 'https://x/', 'guid:k', '[SubsPlease] Show - 14 (1080p).mkv',
                     'magnet:?', 1, 1, 'received', 1, 'r1', 'h1')",
                [],
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (title, rows): (String, i64) = db
            .run::<_, DbError, _>(|c| {
                c.execute(
                    "INSERT INTO video_revisions (item_id, old_item_id, rule_id, folder,
                         episode_name, new_version, state, created_at, updated_at)
                     VALUES (7, NULL, 'r1', '/m/Show/Season 01', 'Show S01E14.mkv', 2,
                         'receiving', 1, 1)",
                    [],
                )?;
                Ok(c.query_row(
                    "SELECT (SELECT title || '|' || result FROM history_items WHERE id = 7),
                            (SELECT count(*) FROM video_revisions)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(title, "[SubsPlease] Show - 14 (1080p).mkv|received");
        assert_eq!(rows, 1);
        // A state the table does not know is refused.
        let refused = db
            .run::<_, DbError, _>(|c| {
                Ok(c.execute(
                    "UPDATE video_revisions SET state = 'gone' WHERE item_id = 7",
                    [],
                )?)
            })
            .await;
        assert!(refused.is_err());
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
