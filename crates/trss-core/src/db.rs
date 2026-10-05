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
    /// A SQL script that makes anew a table other tables refer to (SQLite
    /// cannot change a column's CHECK in place). It runs in a transaction of
    /// its own with foreign keys off, so dropping the old table neither
    /// cascades to the rows that refer to it nor fails, and it commits only
    /// when it broke no reference (SQLite's "other kinds of table schema
    /// changes"); one broken before it is not its own and does not stop it.
    Remake(&'static str),
}

impl Migration {
    fn apply(&self, conn: &Connection) -> Result<(), DbError> {
        match self {
            Migration::Sql(sql) | Migration::Remake(sql) => Ok(conn.execute_batch(sql)?),
            Migration::Code(run) => run(conn),
        }
    }

    fn remakes(&self) -> bool {
        matches!(self, Migration::Remake(_))
    }
}

/// Embedded migrations. Migration `n` (1-based position in this list) moves
/// `PRAGMA user_version` from `n - 1` to `n`. Only append; never edit or
/// reorder an entry that has shipped.
const MIGRATIONS: &[Migration] = &[
    // 1: channels and rules
    Migration::Sql(include_str!("../migrations/channels/schema.sql")),
    // 2: collection history and the worker's cycle marker
    Migration::Sql(include_str!("../migrations/history/schema.sql")),
    // 3: optional channel display name
    Migration::Sql("ALTER TABLE channels ADD COLUMN name TEXT CHECK (name IS NULL OR name <> '');"),
    // 4: the worker's snapshots for the collection screen's status board
    Migration::Sql(include_str!("../migrations/status/schema.sql")),
    // 5: commands the web accepts and the worker carries out
    Migration::Sql(include_str!("../migrations/commands/schema.sql")),
    // 6: a command whose Transmission add got no answer
    Migration::Sql(include_str!("../migrations/commands/add_unconfirmed.sql")),
    // 7: the app-wide collect folder replaces the channels' base folders
    Migration::Code(crate::settings::fold_base_dirs),
    // 8: watch folders and the works, seasons, episodes and files found in them
    Migration::Sql(include_str!("../migrations/library/schema.sql")),
    // 9: work folders seen but not readable yet, so a folder's baseline is per work
    Migration::Sql(include_str!("../migrations/library/unread_works.sql")),
    // 10: watch folders the app registers for the collect and archive folders
    Migration::Sql(include_str!("../migrations/library/automatic.sql")),
    // 11: work artwork: the selection, the image files the app made, AniList's pace
    Migration::Sql(include_str!("../migrations/artwork/schema.sql")),
    // 12: what the worker's inotify watches could not cover in a watch folder
    Migration::Sql(include_str!("../migrations/library/watch_note.sql")),
    // 13: season info: the AniList entries linked to each local season
    Migration::Sql(include_str!("../migrations/seasons/schema.sql")),
    // 14: unregistered watch folders keep their works instead of deleting them
    Migration::Sql(include_str!("../migrations/library/unregistered.sql")),
    // 15: Anissia: the schedule snapshot of subscribed anime, rule subscriptions, the request pace
    Migration::Sql(include_str!("../migrations/anissia/schema.sql")),
    // 16: a rule can be paused (`영상 받기` off); a subscription notes a season that is taken
    Migration::Sql(include_str!("../migrations/channels/paused.sql")),
    // 17: the history's items found by the rule that received them
    Migration::Sql(include_str!("../migrations/history/by_rule.sql")),
    // 18: a number that changes when a video lookup in the library could answer differently
    Migration::Sql(include_str!("../migrations/library/generation.sql")),
    // 19: when a rule was last turned back on, so what it missed while off is left to the user
    Migration::Sql(include_str!("../migrations/channels/resumed.sql")),
    // 20: when a title-waiting subscription got its title; the title candidates the user rejected
    Migration::Sql(include_str!("../migrations/channels/title_waiting.sql")),
    // 21: the weekly schedule: the torrents being downloaded and the cycle interval the worker
    //     leaves for the web, and the first run's checklist
    Migration::Sql(concat!(
        include_str!("../migrations/status/week.sql"),
        include_str!("../migrations/setup/schema.sql")
    )),
    // 22: when Anissia was found not to list an anime any more; the first run's steps and end
    //     kept, instead of read from the data each time
    Migration::Sql(concat!(
        include_str!("../migrations/anissia/unlisted.sql"),
        include_str!("../migrations/setup/ended.sql")
    )),
    // 23: the grounds of a rule's automatic episode offset
    Migration::Sql(include_str!("../migrations/channels/episode_basis.sql")),
    // 24: when a rule started and which archive suggestion grounds the user chose to keep collecting
    Migration::Sql(include_str!(
        "../migrations/channels/archive_suggestion.sql"
    )),
    // 25: the replacement of video revisions and how far each has come
    Migration::Sql(include_str!("../migrations/revisions/schema.sql")),
    // 26: the pace of search requests to a feed host, shared by the web and the worker
    Migration::Sql(include_str!("../migrations/search_pace/schema.sql")),
    // 27: every torrent Transmission held when the worker last looked, for the web's past search
    Migration::Sql(include_str!("../migrations/status/listing.sql")),
    // 28: which items a channel's first read recorded, and when it was
    Migration::Sql(include_str!("../migrations/history/first_read.sql")),
    // 29: the worker's heartbeat while it holds the cycle lock, so the web tells a dead worker from a slow cycle
    Migration::Sql(include_str!("../migrations/status/heartbeat.sql")),
    // 30: the name a command's torrent had before the command renamed it
    Migration::Sql(include_str!("../migrations/commands/original_name.sql")),
    // 31: the Anissia anime each season is linked to; seasons with a subscription get its anime
    Migration::Sql(include_str!("../migrations/seasons/anissia_link.sql")),
    // 32: the Korean titles of an AniList entry
    Migration::Sql(include_str!("../migrations/seasons/korean_titles.sql")),
    // 33: the observations of Anissia's subtitle lines, their sources, and the reading's schedule
    Migration::Sql(include_str!("../migrations/anissia/captions.sql")),
    // 34: subtitle jobs, their items, steps, log and file receipts
    Migration::Sql(include_str!("../migrations/jobs/schema.sql")),
    // 35: the common policy (subtitle format order, server browser) and a work's own format order
    Migration::Sql(include_str!("../migrations/settings/policy.sql")),
    // 36: a file receipt's format, failure class, answer facts and snapshot; a failed item's class
    Migration::Sql(include_str!("../migrations/jobs/results.sql")),
    // 37: a job's revision of a received subtitle; a subtitle source's episode mapping to a season
    Migration::Sql(include_str!("../migrations/jobs/follow.sql")),
    // 38: the kind of a file a person uploaded, and the files an upload did not keep
    Migration::Sql(include_str!("../migrations/jobs/upload.sql")),
    // 39: the format of an uploaded archive
    Migration::Sql(include_str!("../migrations/jobs/upload_archive.sql")),
    // 40: the creator the user named for a subtitle file of the library
    Migration::Sql(include_str!("../migrations/library/subtitle_creator.sql")),
    // 41: the daily recheck of a received episode's files, and the revision that equals what was received
    Migration::Sql(include_str!("../migrations/jobs/recheck.sql")),
    // 42: the offset an auto mapping was taken back from; the episodes that do not fit a source's mapping
    Migration::Sql(include_str!("../migrations/jobs/airtime.sql")),
    // 43: the version of a source's episode mapping and the user's per-episode exceptions to it
    Migration::Sql(include_str!("../migrations/jobs/user_mapping.sql")),
    // 44: an item can fail for no subtitle in an image or for a key it needs; a received file's folders
    Migration::Sql(include_str!("../migrations/jobs/winpng.sql")),
    // 45: the remote screen of a job that waits for a person's check: the bound browser run and the web's requests
    Migration::Sql(include_str!("../migrations/jobs/remote_screen.sql")),
    // 46: a find job's request to finish receiving; its screen's first page and a request to close a popup
    Migration::Sql(include_str!("../migrations/jobs/find.sql")),
    // 47: the pages of a screen's run a person may see as tabs, and a request to show another one
    Migration::Sql(include_str!("../migrations/jobs/screen_controls.sql")),
    // 48: a person's request to start a screen's browser run anew
    Migration::Sql(include_str!("../migrations/jobs/screen_restart.sql")),
    // 49: storing and applying what a job received: packages, assets, stored and applied subtitles, the plan and its file effects
    Migration::Sql(include_str!("../migrations/jobs/store_apply.sql")),
    // 50: the asset a plan row keeps when it is a font, an attachment or a companion file
    Migration::Sql(include_str!("../migrations/jobs/package_assets.sql")),
    // 51: a received subtitle whose episode has no video waits for it; the jobs earlier builds finished so go back in line
    Migration::Sql(include_str!("../migrations/jobs/awaiting_video.sql")),
    // 52: replacing an episode's subtitle once a person approves it: the plans, their paths, the effects that take a file off its path or import it, packages imported from beside a video
    Migration::Remake(include_str!("../migrations/jobs/replacement.sql")),
    // 53: unpacking a received archive: the volumes of a split one, whether and why not it was unpacked, its members
    Migration::Sql(include_str!("../migrations/jobs/unpack.sql")),
    // 54: a person confirms an upload's or a find job's placement before any of it is kept; the ones earlier builds finished go back in line
    Migration::Sql(include_str!("../migrations/jobs/placement_confirm.sql")),
    // 55: a person's cleanup of a stored subtitle and the files that go with it; a removed asset frees its path
    Migration::Sql(include_str!("../migrations/jobs/cleanup.sql")),
    // 56: a Google Drive font not received again because its size and Last-Modified did not change: the font its receipt uses
    Migration::Sql(include_str!("../migrations/jobs/unchanged_fonts.sql")),
    // 57: what differs between a replacement plan's current subtitle and the new one, kept with the plan
    Migration::Sql(include_str!("../migrations/jobs/replacement_diffs.sql")),
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

/// How many migrations this build applies, which is the `user_version` of a
/// database it has opened.
#[cfg(any(test, feature = "test-support"))]
pub fn schema_version() -> usize {
    MIGRATIONS.len()
}

/// Creates a database at `path` left as a build with its first `n` migrations
/// would leave it, for tests of the migrations after them.
#[cfg(any(test, feature = "test-support"))]
pub fn database_at(path: &Path, n: usize) -> Connection {
    let conn = Connection::open(path).unwrap();
    for migration in &MIGRATIONS[..n] {
        migration.apply(&conn).unwrap();
    }
    conn.pragma_update(None, "user_version", n as i64).unwrap();
    conn
}

/// Applies pending migrations inside write transactions, so two processes
/// starting together apply each migration once: the ones in a row that are
/// no [`Migration::Remake`] in one, each remake in one of its own with
/// foreign keys off.
fn migrate(conn: &mut Connection) -> Result<(), DbError> {
    let supported = MIGRATIONS.len();
    loop {
        let current = user_version(conn)?;
        if current == supported {
            return Ok(());
        }
        check_supported(current, supported)?;
        let remake = MIGRATIONS[current].remakes();
        // A no-op inside a transaction, so it is set before one begins.
        if remake {
            conn.pragma_update(None, "foreign_keys", false)?;
        }
        let ran = migrate_run(conn, remake);
        if remake {
            conn.pragma_update(None, "foreign_keys", true)?;
        }
        ran?;
    }
}

/// One transaction of [`migrate`]: the pending migrations from the first
/// on that are remakes (`remake`, one of them) or are not.
fn migrate_run(conn: &mut Connection, remake: bool) -> Result<(), DbError> {
    let supported = MIGRATIONS.len();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Another process may have migrated while this one waited for the lock.
    let current = user_version(&tx)?;
    check_supported(current, supported)?;
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(current) {
        if migration.remakes() != remake {
            break;
        }
        let before = match remake {
            true => broken_references(&tx)?,
            false => Vec::new(),
        };
        migration.apply(&tx)?;
        if remake {
            let broken = newly_broken(before, broken_references(&tx)?);
            if !broken.is_empty() {
                return Err(DbError::Migration(format!(
                    "migration {} left broken references: {}",
                    index + 1,
                    broken.join(", ")
                )));
            }
        }
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
        if remake {
            break;
        }
    }
    tx.commit()?;
    Ok(())
}

/// The references whose row is missing, as `table row n -> parent`.
fn broken_references(conn: &Connection) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
    let rows = stmt.query_map([], |r| {
        Ok(format!(
            "{} row {} -> {}",
            r.get::<_, String>(0)?,
            r.get::<_, Option<i64>>(1)?.unwrap_or_default(),
            r.get::<_, String>(2)?
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The references of `after` that `before` does not account for. A child
/// table without rowids has every row as `row 0`, so references read alike:
/// each one before excuses one after.
fn newly_broken(mut before: Vec<String>, after: Vec<String>) -> Vec<String> {
    after
        .into_iter()
        .filter(|b| match before.iter().position(|x| x == b) {
            Some(i) => {
                before.swap_remove(i);
                false
            }
            None => true,
        })
        .collect()
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
    async fn database_from_before_original_names_keeps_its_commands_with_no_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 29 migrations left it, one command running.
            let conn = database_at(&path, 29);
            conn.execute(
                "INSERT INTO commands (id, kind, payload, state, attempts, created_at,
                     updated_at, add_unconfirmed)
                 VALUES ('cmd-1', 'receive_once', '{}', 'running', 1, 1, 2, 1)",
                [],
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let row: (String, i64, i64, Option<String>) = db
            .run::<_, DbError, _>(|c| {
                Ok(c.query_row(
                    "SELECT state, attempts, add_unconfirmed, original_name
                     FROM commands WHERE id = 'cmd-1'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(row, ("running".to_owned(), 1, 1, None));
    }

    #[tokio::test]
    async fn a_database_from_before_season_anissia_links_gives_subscribed_seasons_their_anime() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        let summary = "SELECT group_concat(rule_id || '|' || anissia_anime_no || '|' || subtitles
                              || '|' || ifnull(creator, '') || '|' || ifnull(season_id, '') || '|'
                              || subscribed_at, ';')
                         FROM (SELECT * FROM rule_subscriptions ORDER BY rule_id)";
        let before: String = {
            // A database as the build with 30 migrations left it: a work with
            // two seasons, one of them subscribed (by two channels' rules); a
            // second work with a season nobody subscribes to; a subscription
            // that has no season yet; one whose work is gone; a plain rule;
            // two subscriptions of different anime on one season; an archived
            // rule's subscription; a subscription whose season has no row.
            let conn = database_at(&path, 30);
            conn.execute_batch(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
                 INSERT INTO works (id, watch_folder_id, dir_name)
                     VALUES ('w1', 'f1', 'Clevatess'), ('w2', 'f1', 'Plain'),
                            ('w3', 'f1', 'Twin'), ('w4', 'f1', 'Old'), ('w5', 'f1', 'Bare');
                 INSERT INTO seasons (work_id, number)
                     VALUES ('w1', 1), ('w1', 2), ('w2', 1), ('w3', 1), ('w4', 1);
                 INSERT INTO channels (id, position, url, excludes, secret_query, version)
                     VALUES ('c1', 0, 'http://x/feed', '[]', '[]', 1),
                            ('c2', 1, 'http://y/feed', '[]', '[]', 1);
                 INSERT INTO rules (id, channel_id, position, match_text, regex,
                         case_insensitive, directory, episode, episode_auto, state, version)
                     VALUES ('r1', 'c1', 0, 'Clevatess', 0, 1, 'Clevatess/Season 02', 1, 0,
                                'active', 2),
                            ('r2', 'c1', 1, 'Waiting', 0, 1, 'Waiting', 1, 0, 'active', 1),
                            ('r3', 'c1', 2, 'Gone', 0, 1, 'Gone', 1, 0, 'active', 1),
                            ('r4', 'c1', 3, 'Plain', 0, 1, 'Plain', 1, 0, 'active', 1),
                            ('r5', 'c2', 0, 'Clevatess', 0, 1, 'Clevatess/Season 02', 1, 0,
                                'paused', 1),
                            ('r6', 'c2', 1, 'Twin', 0, 1, 'Twin/Season 01', 1, 0, 'active', 1),
                            ('r7', 'c2', 2, 'Twin B', 0, 1, 'Twin/Season 01', 1, 0, 'active', 1),
                            ('r8', 'c2', 3, 'Old', 0, 1, 'Old/Season 01', 1, 0, 'archived', 1),
                            ('r9', 'c2', 4, 'Bare', 0, 1, 'Bare/Season 03', 1, 0, 'active', 1);
                 INSERT INTO anissia_anime (anime_no, subject, week, status, fetched_at)
                     VALUES (7, '클레바테스', 1, 'ON', 10), (8, '기다림', 2, 'ON', 10),
                            (9, '사라진 작품', 3, 'OFF', 10);
                 INSERT INTO rule_subscriptions (rule_id, anissia_anime_no, subtitles, creator,
                         season_id, subscribed_at)
                     VALUES ('r1', 7, 'follow', 'SubKor', 'w1:2', 99),
                            ('r2', 8, 'undecided', NULL, NULL, 98),
                            ('r3', 9, 'none', NULL, 'removed-work:1', 97),
                            ('r5', 7, 'undecided', NULL, 'w1:2', 96),
                            ('r6', 8, 'none', NULL, 'w3:1', 95),
                            ('r7', 9, 'none', NULL, 'w3:1', 94),
                            ('r8', 7, 'none', NULL, 'w4:1', 93),
                            ('r9', 7, 'none', NULL, 'w5:3', 92);",
            )
            .unwrap();
            conn.query_row(summary, [], |r| r.get(0)).unwrap()
        };

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (links, after, broken): (String, String, i64) = db
            .run::<_, DbError, _>(move |c| {
                let links = c.query_row(
                    "SELECT ifnull(group_concat(work_id || ':' || season || '=' || anime_no
                                   || '@' || version, ','), '')
                       FROM (SELECT * FROM season_anissia ORDER BY work_id, season)",
                    [],
                    |r| r.get(0),
                )?;
                let after = c.query_row(summary, [], |r| r.get(0))?;
                let broken =
                    c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                        r.get(0)
                    })?;
                Ok((links, after, broken))
            })
            .await
            .unwrap();
        // Each subscribed season is linked to the subscription's anime, once: the
        // two subscriptions of one season give the anime of the first rule by ID
        // (8, not 9), an archived rule's subscription and one whose season has no
        // row of its own link too. The unsubscribed season, the waiting
        // subscription and the one whose work is gone add nothing. The
        // subscriptions are as they were.
        assert_eq!(links, "w1:2=7@1,w3:1=8@1,w4:1=7@1,w5:3=7@1");
        assert_eq!(after, before);
        assert_eq!(broken, 0);

        // The link goes with its work.
        let left: i64 = db
            .run::<_, DbError, _>(|c| {
                c.execute("DELETE FROM works WHERE id = 'w1'", [])?;
                Ok(c.query_row(
                    "SELECT count(*) FROM season_anissia WHERE work_id = 'w1'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn a_database_from_before_korean_titles_keeps_its_entries_with_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 31 migrations left it: an AniList entry
            // linked to a season.
            let conn = database_at(&path, 31);
            conn.execute_batch(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
                 INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
                 INSERT INTO seasons (work_id, number) VALUES ('w1', 1);
                 INSERT INTO anilist_entries (id, romaji, native, genres, fetched_at)
                     VALUES (5, 'Show', 'ショー', '[\"Action\"]', 77);
                 INSERT INTO season_entries (work_id, season, position, anilist_id)
                     VALUES ('w1', 1, 0, 5);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (row, linked, bad): (String, i64, bool) = db
            .run::<_, DbError, _>(|c| {
                let row = c.query_row(
                    "SELECT romaji || '|' || native || '|' || genres || '|' || fetched_at
                            || '|' || korean_titles FROM anilist_entries WHERE id = 5",
                    [],
                    |r| r.get(0),
                )?;
                let linked =
                    c.query_row("SELECT count(*) FROM season_entries", [], |r| r.get(0))?;
                // The column holds JSON: a stored list reads back, text that is not JSON is refused.
                c.execute(
                    "UPDATE anilist_entries SET korean_titles = '[\"봇치\"]' WHERE id = 5",
                    [],
                )?;
                let bad = c
                    .execute(
                        "UPDATE anilist_entries SET korean_titles = 'not json' WHERE id = 5",
                        [],
                    )
                    .is_err();
                Ok((row, linked, bad))
            })
            .await
            .unwrap();
        // The entry is as it was, with no Korean titles until it is received again.
        assert_eq!(row, "Show|ショー|[\"Action\"]|77|[]");
        assert_eq!(linked, 1);
        assert!(bad);
    }

    #[tokio::test]
    async fn a_database_from_before_caption_observations_keeps_its_rows_and_starts_with_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 32 migrations left it: a season linked
            // to an Anissia anime, and that anime's snapshot.
            let conn = database_at(&path, 32);
            conn.execute_batch(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
                 INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
                 INSERT INTO seasons (work_id, number) VALUES ('w1', 1);
                 INSERT INTO anissia_anime (anime_no, subject, week, status, fetched_at)
                     VALUES (3441, '작품', 2, 'ON', 77);
                 INSERT INTO season_anissia (work_id, season, anime_no, version)
                     VALUES ('w1', 1, 3441, 1);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (linked, sources, observations, polls, refused): (i64, i64, i64, i64, [bool; 4]) = db
            .run::<_, DbError, _>(|c| {
                let count = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0));
                let linked = count("SELECT anime_no FROM season_anissia WHERE work_id = 'w1'")?;
                let (sources, observations, polls) = (
                    count("SELECT count(*) FROM subtitle_sources")?,
                    count("SELECT count(*) FROM caption_observations")?,
                    count("SELECT count(*) FROM anissia_caption_poll")?,
                );
                c.execute(
                    "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('s1', 3441, '에루샤', 5)",
                    [],
                )?;
                c.execute(
                    "INSERT INTO caption_observations
                         (source_id, post_url, episode, updated, updated_at, first_seen_at)
                     VALUES ('s1', 'https://erulabo.com/837', '0', 'not a date', NULL, 6)",
                    [],
                )?;
                // A creator has one source per anime; an observation names a
                // source and a post; the schedule is one row.
                let refused = [
                    c.execute(
                        "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                         VALUES ('s2', 3441, '에루샤', 5)",
                        [],
                    ),
                    c.execute(
                        "INSERT INTO caption_observations
                             (source_id, post_url, episode, updated, first_seen_at)
                         VALUES ('nobody', 'https://a.test/1', '1', 'x', 6)",
                        [],
                    ),
                    c.execute(
                        "INSERT INTO caption_observations
                             (source_id, post_url, episode, updated, first_seen_at)
                         VALUES ('s1', '', '1', 'x', 6)",
                        [],
                    ),
                    c.execute(
                        "INSERT INTO anissia_caption_poll (id, next_at) VALUES (2, 1)",
                        [],
                    ),
                ]
                .map(|r| r.is_err());
                Ok((linked, sources, observations, polls, refused))
            })
            .await
            .unwrap();
        // The link is as it was, and nothing is observed until the worker reads.
        assert_eq!((linked, sources, observations, polls), (3441, 0, 0, 0));
        assert_eq!(refused, [true; 4]);
    }

    #[tokio::test]
    async fn a_database_from_before_the_common_policy_keeps_its_settings_and_has_no_policy_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 34 migrations left it: the collect folder set.
            let conn = database_at(&path, 34);
            conn.execute_batch(
                "INSERT INTO collection_settings (id, collect_folder, archive_folder, version)
                     VALUES (1, '/downloads/Shows', NULL, 3);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (version, policies, overrides): (i64, i64, i64) = db
            .run::<_, DbError, _>(|c| {
                let count = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0));
                Ok((
                    count("SELECT version FROM collection_settings")?,
                    count("SELECT count(*) FROM policy_settings")?,
                    count("SELECT count(*) FROM work_subtitle_policy")?,
                ))
            })
            .await
            .unwrap();
        assert_eq!((version, policies, overrides), (3, 0, 0));
    }

    #[tokio::test]
    async fn a_database_from_before_receive_results_keeps_its_receipts_and_checks_the_new_columns()
    {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 35 migrations left it: a job with a
            // received file and a failed one.
            let conn = database_at(&path, 35);
            conn.execute_batch(
                "INSERT INTO subtitle_jobs (id, command_id, request, origin, state,
                     created_at, updated_at, state_at)
                     VALUES ('j1', 'c1', '{}', 'pick', 'partial', 1, 1, 1);
                 INSERT INTO subtitle_job_items (job_id, position, episode, post_url, found_at,
                     state, reason, updated_at)
                     VALUES ('j1', 0, '1', 'https://a.tistory.com/1', 6, 'done', NULL, 1),
                            ('j1', 1, '2', 'https://a.tistory.com/2', 6, 'failed', '없어요', 1);
                 INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state,
                     size, sha256, path, created_at, updated_at)
                     VALUES ('a1', 'j1', 1, 'k1', 'x.ass', 'done', 10, 'ab', 'j1/x.ass', 1, 1),
                            ('a2', 'j1', 2, 'k2', 'y.ass', 'failed', NULL, NULL, NULL, 1, 1);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        type Kept = (i64, i64, Option<String>, Option<String>, Option<String>);
        let (kept, refused, accepted): (Kept, [bool; 6], i64) = db
            .run::<_, DbError, _>(|c| {
                let kept = c.query_row(
                    "SELECT (SELECT count(*) FROM subtitle_job_files),
                            (SELECT count(*) FROM subtitle_job_items),
                            (SELECT format FROM subtitle_job_files WHERE id = 'a1'),
                            (SELECT failure FROM subtitle_job_files WHERE id = 'a2'),
                            (SELECT failure FROM subtitle_job_items WHERE position = 1)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )?;
                let update = |sql: &str| c.execute(sql, []).is_err();
                let refused = [
                    update("UPDATE subtitle_job_files SET format = 'mp4' WHERE id = 'a1'"),
                    update("UPDATE subtitle_job_files SET failure = 'gone' WHERE id = 'a2'"),
                    update("UPDATE subtitle_job_files SET http_status = 99 WHERE id = 'a2'"),
                    update("UPDATE subtitle_job_files SET response_size = -1 WHERE id = 'a2'"),
                    update("UPDATE subtitle_job_files SET snapshot = 'not json' WHERE id = 'a1'"),
                    update("UPDATE subtitle_job_items SET failure = 'timeout' WHERE position = 1"),
                ];
                c.execute_batch(
                    "UPDATE subtitle_job_files SET format = 'zip',
                         snapshot = '[[\"article:modified_time\",\"2026-09-28T00:13:41+09:00\"]]'
                         WHERE id = 'a1';
                     UPDATE subtitle_job_files SET failure = 'expired', http_status = 404,
                         content_type = 'text/html', response_size = 150 WHERE id = 'a2';
                     UPDATE subtitle_job_items SET failure = 'expired' WHERE position = 1;",
                )?;
                let accepted = c.query_row(
                    "SELECT count(*) FROM subtitle_job_files WHERE format IS NOT NULL OR failure IS NOT NULL",
                    [],
                    |r| r.get(0),
                )?;
                Ok((kept, refused, accepted))
            })
            .await
            .unwrap();
        assert_eq!(kept, (2, 2, None, None, None));
        assert_eq!(refused, [true; 6]);
        assert_eq!(accepted, 2);
    }

    #[tokio::test]
    async fn a_database_from_before_auto_receipts_keeps_its_jobs_and_maps_no_source_yet() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 36 migrations left it: a work, a
            // source with two observations and a job that received the first.
            let conn = database_at(&path, 36);
            conn.execute_batch(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
                 INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
                 INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('s1', 3441, '에루샤', 5);
                 INSERT INTO caption_observations (source_id, post_url, episode, updated,
                     first_seen_at)
                     VALUES ('s1', 'https://erulabo.com/1', '3', 'x', 6),
                            ('s1', 'https://erulabo.com/1', '3', 'y', 7);
                 INSERT INTO subtitle_jobs (id, command_id, request, origin, work_id, season,
                     anime_no, source_id, creator, state, created_at, updated_at, state_at)
                     VALUES ('j1', 'c1', '{}', 'pick', 'w1', 1, 3441, 's1', '에루샤', 'done',
                             1, 1, 1);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        type Kept = (i64, Option<i64>, i64);
        let (kept, refused, accepted): (Kept, [bool; 5], i64) = db
            .run::<_, DbError, _>(|c| {
                let kept = c.query_row(
                    "SELECT (SELECT count(*) FROM subtitle_jobs),
                            (SELECT revision_of FROM subtitle_jobs WHERE id = 'j1'),
                            (SELECT count(*) FROM subtitle_episode_mappings)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
                let write = |sql: &str| c.execute(sql, []).is_err();
                let refused = [
                    // An observation that is not there.
                    write("UPDATE subtitle_jobs SET revision_of = 99 WHERE id = 'j1'"),
                    write(
                        "INSERT INTO subtitle_episode_mappings
                         (work_id, season, source_id, kind, episode_offset, evidence, decided_at) VALUES
                         ('w1', 1, 's1', 'guess', 0, '근거', 1)",
                    ),
                    // Undecided with an offset, decided without one.
                    write(
                        "INSERT INTO subtitle_episode_mappings
                         (work_id, season, source_id, kind, episode_offset, evidence, decided_at) VALUES
                         ('w1', 1, 's1', 'undecided', 0, '근거', 1)",
                    ),
                    write(
                        "INSERT INTO subtitle_episode_mappings
                         (work_id, season, source_id, kind, episode_offset, evidence, decided_at) VALUES
                         ('w1', 1, 's1', 'auto', NULL, '근거', 1)",
                    ),
                    write(
                        "INSERT INTO subtitle_episode_mappings
                         (work_id, season, source_id, kind, episode_offset, evidence, decided_at) VALUES
                         ('w1', 1, 'nope', 'auto', 0, '근거', 1)",
                    ),
                ];
                c.execute_batch(
                    "UPDATE subtitle_jobs SET revision_of = 1 WHERE id = 'j1';
                     INSERT INTO subtitle_episode_mappings
                         (work_id, season, source_id, kind, episode_offset, evidence, decided_at) VALUES
                         ('w1', 1, 's1', 'auto', -12, '근거', 1);
                     INSERT INTO subtitle_episode_mappings
                         (work_id, season, source_id, kind, episode_offset, evidence, decided_at) VALUES
                         ('w1', 2, 's1', 'undecided', NULL, '이유', 1);",
                )?;
                // The mappings go with their work.
                c.execute("DELETE FROM works WHERE id = 'w1'", [])?;
                let accepted =
                    c.query_row("SELECT count(*) FROM subtitle_episode_mappings", [], |r| {
                        r.get(0)
                    })?;
                Ok((kept, refused, accepted))
            })
            .await
            .unwrap();
        assert_eq!(kept, (1, None, 0));
        assert_eq!(refused, [true; 5]);
        assert_eq!(accepted, 0);
    }

    #[tokio::test]
    async fn a_database_from_before_uploads_keeps_its_jobs_and_checks_the_new_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 37 migrations left it: a job with a
            // received file.
            let conn = database_at(&path, 37);
            conn.execute_batch(
                "INSERT INTO subtitle_jobs (id, command_id, request, origin, state,
                     created_at, updated_at, state_at)
                     VALUES ('j1', 'c1', '{}', 'pick', 'done', 1, 1, 1);
                 INSERT INTO subtitle_job_items (job_id, position, episode, post_url, found_at,
                     state, updated_at)
                     VALUES ('j1', 0, '1', 'https://a.tistory.com/1', 6, 'done', 1);
                 INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state,
                     size, sha256, path, created_at, updated_at)
                     VALUES ('a1', 'j1', 1, 'k1', 'x.ass', 'done', 10, 'ab', 'j1/x.ass', 1, 1);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (kept, refused, cascaded): ((i64, Option<String>), [bool; 4], i64) = db
            .run::<_, DbError, _>(|c| {
                let kept = c.query_row(
                    "SELECT (SELECT count(*) FROM subtitle_job_files),
                            (SELECT kind FROM subtitle_job_files WHERE id = 'a1')",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                let write = |sql: &str| c.execute(sql, []).is_err();
                let refused = [
                    write("UPDATE subtitle_job_files SET kind = 'video' WHERE id = 'a1'"),
                    write("INSERT INTO subtitle_job_dropped VALUES ('nobody', 0, 'a.txt', '이유')"),
                    write("INSERT INTO subtitle_job_dropped VALUES ('j1', 0, '', '이유')"),
                    write("INSERT INTO subtitle_job_dropped VALUES ('j1', 0, 'a.txt', '')"),
                ];
                c.execute_batch(
                    "UPDATE subtitle_job_files SET kind = 'font' WHERE id = 'a1';
                     INSERT INTO subtitle_job_dropped VALUES ('j1', 0, 'a.txt', '이유');",
                )?;
                // The dropped names go with their job.
                c.execute("DELETE FROM subtitle_jobs WHERE id = 'j1'", [])?;
                let cascaded =
                    c.query_row("SELECT count(*) FROM subtitle_job_dropped", [], |r| {
                        r.get(0)
                    })?;
                Ok((kept, refused, cascaded))
            })
            .await
            .unwrap();
        assert_eq!(kept, (1, None));
        assert_eq!(refused, [true; 4]);
        assert_eq!(cascaded, 0);
    }

    #[tokio::test]
    async fn a_database_from_before_archive_types_keeps_its_uploads_and_checks_the_new_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 38 migrations left it: an upload
            // job with a ZIP.
            let conn = database_at(&path, 38);
            conn.execute_batch(
                "INSERT INTO subtitle_jobs (id, command_id, request, origin, state,
                     created_at, updated_at, state_at)
                     VALUES ('j1', 'c1', '{}', 'upload', 'done', 1, 1, 1);
                 INSERT INTO subtitle_job_items (job_id, position, episode, post_url, found_at,
                     state, updated_at)
                     VALUES ('j1', 0, '', 'upload:', 6, 'done', 1);
                 INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state,
                     size, sha256, path, created_at, updated_at, format, kind)
                     VALUES ('a1', 'j1', 1, 'k1', 'x.zip', 'done', 10, 'ab', 'j1/x.zip', 1, 1,
                             'zip', 'archive');",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (kept, refused, accepted): ((String, Option<String>), [bool; 2], bool) = db
            .run::<_, DbError, _>(|c| {
                let kept = c.query_row(
                    "SELECT kind, archive_type FROM subtitle_job_files WHERE id = 'a1'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                let write = |sql: &str| c.execute(sql, []).is_err();
                let refused = [
                    write("UPDATE subtitle_job_files SET archive_type = 'iso' WHERE id = 'a1'"),
                    write("UPDATE subtitle_job_files SET archive_type = 'ZIP' WHERE id = 'a1'"),
                ];
                let accepted = ["zip", "rar", "7z", "gz", "bz2", "xz", "tar"]
                    .iter()
                    .all(|t| {
                        c.execute(
                            "UPDATE subtitle_job_files SET archive_type = ?1 WHERE id = 'a1'",
                            [t],
                        )
                        .is_ok()
                    });
                Ok((kept, refused, accepted))
            })
            .await
            .unwrap();
        assert_eq!(kept, ("archive".to_owned(), None));
        assert_eq!(refused, [true; 2]);
        assert!(accepted);
    }

    #[tokio::test]
    async fn a_library_from_before_subtitle_creators_keeps_its_files_with_no_creator() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 39 migrations left it: a work with a
            // video and a subtitle, and a subtitle source.
            let conn = database_at(&path, 39);
            conn.execute_batch(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
                 INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
                 INSERT INTO seasons (work_id, number) VALUES ('w1', 1);
                 INSERT INTO episodes (work_id, season, episode) VALUES ('w1', 1, '01');
                 INSERT INTO media_files (work_id, path, season, episode, kind, added_at)
                     VALUES ('w1', 'Season 01/e01.mkv', 1, '01', 'video', 5),
                            ('w1', 'Season 01/e01.ass', 1, '01', 'subtitle', 6);
                 INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('s1', 3441, '하느', 5);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        type Kept = Vec<(String, Option<String>, i64)>;
        let (kept, refused, named): (Kept, [bool; 2], i64) = db
            .run::<_, DbError, _>(|c| {
                let mut stmt = c.prepare(
                    "SELECT path, creator_source_id, creator_version FROM media_files
                      ORDER BY path",
                )?;
                let kept = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<rusqlite::Result<Kept>>()?;
                let write = |sql: &str| c.execute(sql, []).is_err();
                let refused = [
                    // A source that is not there; a version below zero.
                    write(
                        "UPDATE media_files SET creator_source_id = 'nope' WHERE kind = 'subtitle'",
                    ),
                    write("UPDATE media_files SET creator_version = -1 WHERE kind = 'subtitle'"),
                ];
                c.execute(
                    "UPDATE media_files SET creator_source_id = 's1', creator_version = 1
                      WHERE kind = 'subtitle'",
                    [],
                )?;
                let named = c.query_row(
                    "SELECT count(*) FROM media_files WHERE creator_source_id = 's1'",
                    [],
                    |r| r.get(0),
                )?;
                Ok((kept, refused, named))
            })
            .await
            .unwrap();
        assert_eq!(
            kept,
            [
                ("Season 01/e01.ass".to_owned(), None, 0),
                ("Season 01/e01.mkv".to_owned(), None, 0)
            ]
        );
        assert_eq!(refused, [true; 2]);
        assert_eq!(named, 1);

        // The time a creator was named is none for the files that were there,
        // and a job of an earlier build is no revision of a named file.
        let (set_at, flag): (i64, (i64, String)) = db
            .run::<_, DbError, _>(|c| {
                Ok((
                    c.query_row(
                        "SELECT count(*) FROM media_files WHERE creator_set_at IS NOT NULL",
                        [],
                        |r| r.get(0),
                    )?,
                    c.query_row(
                        "SELECT \"notnull\", dflt_value FROM pragma_table_info('subtitle_jobs')
                          WHERE name = 'revises_attributed'",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(set_at, 0);
        assert_eq!(flag, (1, "0".to_owned()));
    }

    #[tokio::test]
    async fn a_database_from_before_the_recheck_keeps_its_items_and_starts_with_no_reading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 40 migrations left it: a job that
            // received one episode.
            let conn = database_at(&path, 40);
            conn.execute_batch(
                "INSERT INTO subtitle_jobs (id, command_id, request, origin, state,
                     created_at, updated_at, state_at)
                     VALUES ('j1', 'c1', '{}', 'auto', 'done', 1, 1, 1);
                 INSERT INTO subtitle_job_items (job_id, position, episode, post_url,
                     found_at, state, updated_at)
                     VALUES ('j1', 0, '1', 'https://erulabo.com/1', 6, 'done', 1);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (kept, refused, cascaded): ((i64, Option<String>, i64), [bool; 4], i64) = db
            .run::<_, DbError, _>(|c| {
                let kept = c.query_row(
                    "SELECT (SELECT count(*) FROM subtitle_job_items),
                            (SELECT unchanged_from FROM subtitle_job_items WHERE id = 1),
                            (SELECT count(*) FROM subtitle_item_rechecks)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
                let write = |sql: &str| c.execute(sql, []).is_err();
                let refused = [
                    // An item that is not there.
                    write("INSERT INTO subtitle_item_rechecks (item_id, checked_at) VALUES (9, 1)"),
                    // A result that is not one of the known ones.
                    write(
                        "INSERT INTO subtitle_item_rechecks (item_id, checked_at, result)
                         VALUES (1, 1, 'maybe')",
                    ),
                    // `observed` is JSON.
                    write(
                        "INSERT INTO subtitle_item_rechecks (item_id, checked_at, observed)
                         VALUES (1, 1, 'not json')",
                    ),
                    // A job that is not there.
                    write("UPDATE subtitle_job_items SET unchanged_from = 'nobody' WHERE id = 1"),
                ];
                c.execute_batch(
                    "INSERT INTO subtitle_item_rechecks
                         (item_id, checked_at, checks, result, observed, job_id, result_at)
                         VALUES (1, 5, 1, 'changed', '[{\"key\":\"k\",\"size\":3}]', 'j1', 5);
                     UPDATE subtitle_job_items SET unchanged_from = 'j1' WHERE id = 1;",
                )?;
                // A reading goes with its item.
                c.execute("DELETE FROM subtitle_job_items WHERE id = 1", [])?;
                let cascaded =
                    c.query_row("SELECT count(*) FROM subtitle_item_rechecks", [], |r| {
                        r.get(0)
                    })?;
                Ok((kept, refused, cascaded))
            })
            .await
            .unwrap();
        assert_eq!(kept, (1, None, 0));
        assert_eq!(refused, [true; 4]);
        assert_eq!(cascaded, 0);
    }

    #[tokio::test]
    async fn a_database_from_before_the_air_time_mapping_keeps_its_mappings_and_starts_with_no_conflict(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 41 migrations left it: a work whose
            // source has an automatic mapping.
            let conn = database_at(&path, 41);
            conn.execute_batch(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
                 INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
                 INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('s1', 3441, '에루샤', 5);
                 INSERT INTO subtitle_episode_mappings
                     (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
                     VALUES ('w1', 1, 's1', 'auto', 0, '근거', 7);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (kept, refused, cascaded): ((i64, Option<i64>, i64), [bool; 4], i64) = db
            .run::<_, DbError, _>(|c| {
                let kept = c.query_row(
                    "SELECT (SELECT count(*) FROM subtitle_episode_mappings),
                            (SELECT retired_offset FROM subtitle_episode_mappings),
                            (SELECT count(*) FROM subtitle_mapping_conflicts)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
                let write = |sql: &str| c.execute(sql, []).is_err();
                let refused = [
                    // A work that is not there.
                    write(
                        "INSERT INTO subtitle_mapping_conflicts
                             (work_id, season, source_id, episode, reason, found_at)
                             VALUES ('nope', 1, 's1', '13.5', '소수', 1)",
                    ),
                    // A source that is not there.
                    write(
                        "INSERT INTO subtitle_mapping_conflicts
                             (work_id, season, source_id, episode, reason, found_at)
                             VALUES ('w1', 1, 'nope', '13.5', '소수', 1)",
                    ),
                    // No reason.
                    write(
                        "INSERT INTO subtitle_mapping_conflicts
                             (work_id, season, source_id, episode, reason, found_at)
                             VALUES ('w1', 1, 's1', '13.5', '', 1)",
                    ),
                    // One row per episode of a source.
                    {
                        c.execute_batch(
                            "INSERT INTO subtitle_mapping_conflicts
                                 (work_id, season, source_id, episode, reason, found_at)
                                 VALUES ('w1', 1, 's1', '13.5', '소수', 1)",
                        )?;
                        write(
                            "INSERT INTO subtitle_mapping_conflicts
                                 (work_id, season, source_id, episode, reason, found_at)
                                 VALUES ('w1', 1, 's1', '13.5', '또', 2)",
                        )
                    },
                ];
                // The offset an auto mapping was taken back from is kept with the row.
                c.execute(
                    "UPDATE subtitle_episode_mappings
                        SET kind = 'undecided', episode_offset = NULL, retired_offset = 0",
                    [],
                )?;
                // The conflicts go with their work.
                c.execute("DELETE FROM works WHERE id = 'w1'", [])?;
                let cascaded =
                    c.query_row("SELECT count(*) FROM subtitle_mapping_conflicts", [], |r| {
                        r.get(0)
                    })?;
                Ok((kept, refused, cascaded))
            })
            .await
            .unwrap();
        assert_eq!(kept, (1, None, 0));
        assert_eq!(refused, [true; 4]);
        assert_eq!(cascaded, 0);
    }

    #[tokio::test]
    async fn a_database_from_before_the_users_mapping_keeps_its_mappings_at_version_one_and_starts_with_no_exception(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 42 migrations left it: a work whose
            // source has an automatic mapping.
            let conn = database_at(&path, 42);
            conn.execute_batch(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
                 INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
                 INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('s1', 3441, '에루샤', 5);
                 INSERT INTO subtitle_episode_mappings
                     (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
                     VALUES ('w1', 1, 's1', 'auto', 0, '근거', 7);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (kept, refused, after_mapping, after_work): (
            (i64, i64, i64, i64),
            [bool; 5],
            i64,
            i64,
        ) = db
            .run::<_, DbError, _>(|c| {
                let kept = c.query_row(
                    "SELECT (SELECT count(*) FROM subtitle_episode_mappings),
                            (SELECT version FROM subtitle_episode_mappings),
                            (SELECT version FROM subtitle_mapping_clock),
                            (SELECT count(*) FROM subtitle_episode_exceptions)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?;
                let write = |sql: &str| c.execute(sql, []).is_err();
                let exception = |key: &str, episode: &str, target: &str, source: &str| {
                    format!(
                        "INSERT INTO subtitle_episode_exceptions
                             (work_id, season, source_id, episode_key, episode, target)
                         VALUES ('w1', 1, '{source}', '{key}', '{episode}', {target})"
                    )
                };
                let refused = [
                    // A target below the first episode.
                    write(&exception("n:13", "13", "0", "s1")),
                    // No key, no text.
                    write(&exception("", "13", "1", "s1")),
                    write(&exception("n:13", "", "1", "s1")),
                    // A mapping that is not there.
                    write(&exception("n:13", "13", "1", "nope")),
                    // One exception per episode key of a source.
                    {
                        c.execute_batch(&exception("n:13", "013", "1", "s1"))?;
                        write(&exception("n:13", "13.0", "NULL", "s1"))
                    },
                ];
                // 받지 않음 has no target.
                c.execute_batch(&exception("n:13.5", "13.5", "NULL", "s1"))?;
                // The exceptions go with their mapping...
                c.execute("DELETE FROM subtitle_episode_mappings", [])?;
                let after_mapping = c.query_row(
                    "SELECT count(*) FROM subtitle_episode_exceptions",
                    [],
                    |r| r.get(0),
                )?;
                // ...and with their work.
                c.execute_batch(
                    "INSERT INTO subtitle_episode_mappings
                         (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
                         VALUES ('w1', 1, 's1', 'user', 0, '근거', 7);",
                )?;
                c.execute_batch(&exception("n:13", "13", "1", "s1"))?;
                c.execute("DELETE FROM works WHERE id = 'w1'", [])?;
                let after_work = c.query_row(
                    "SELECT count(*) FROM subtitle_episode_exceptions",
                    [],
                    |r| r.get(0),
                )?;
                Ok((kept, refused, after_mapping, after_work))
            })
            .await
            .unwrap();
        assert_eq!(kept, (1, 1, 1, 0));
        assert_eq!(refused, [true; 5]);
        assert_eq!(after_mapping, 0);
        assert_eq!(after_work, 0);
    }

    #[tokio::test]
    async fn a_database_from_before_subtitle_jobs_keeps_its_observations_and_starts_with_no_job() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 33 migrations left it: one source and
            // one of its observations.
            let conn = database_at(&path, 33);
            conn.execute_batch(
                "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('s1', 3441, '에루샤', 5);
                 INSERT INTO caption_observations
                     (source_id, post_url, episode, updated, first_seen_at)
                     VALUES ('s1', 'https://erulabo.com/837', '1', 'x', 6);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (observations, jobs, refused, cascaded): (i64, i64, [bool; 7], i64) = db
            .run::<_, DbError, _>(|c| {
                let count = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0));
                let observations = count("SELECT count(*) FROM caption_observations")?;
                let jobs = count("SELECT count(*) FROM subtitle_jobs")?;
                c.execute_batch(
                    "INSERT INTO subtitle_jobs (id, command_id, request, origin, source_id,
                         state, created_at, updated_at, state_at)
                         VALUES ('j1', 'c1', '{}', 'pick', 's1', 'pending', 1, 1, 1);
                     INSERT INTO subtitle_job_items (job_id, position, observation_id, episode,
                         post_url, found_at, state, updated_at)
                         VALUES ('j1', 0, 1, '1', 'https://erulabo.com/837', 6, 'pending', 1);
                     INSERT INTO subtitle_job_steps (job_id, step, state, at)
                         VALUES ('j1', 'found', 'done', 1);
                     INSERT INTO subtitle_job_events (job_id, at, message) VALUES ('j1', 1, '시작');
                     INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state,
                         created_at, updated_at)
                         VALUES ('a1', 'j1', 1, 'k', 'x.ass', 'intended', 1, 1);",
                )?;
                // One job per browser command, known states only, items and
                // files belong to a job.
                let refused = [
                    c.execute(
                        "INSERT INTO subtitle_jobs (id, command_id, request, origin, state,
                             created_at, updated_at, state_at)
                         VALUES ('j2', 'c1', '{}', 'pick', 'pending', 1, 1, 1)",
                        [],
                    ),
                    c.execute(
                        "INSERT INTO subtitle_jobs (id, command_id, request, origin, state,
                             created_at, updated_at, state_at)
                         VALUES ('j3', 'c3', '{}', 'pick', 'paused', 1, 1, 1)",
                        [],
                    ),
                    c.execute(
                        "INSERT INTO subtitle_job_items (job_id, position, episode, post_url,
                             found_at, state, updated_at)
                         VALUES ('j1', 0, '2', 'https://a.test/2', 6, 'pending', 1)",
                        [],
                    ),
                    c.execute(
                        "INSERT INTO subtitle_job_items (job_id, position, episode, post_url,
                             found_at, state, updated_at)
                         VALUES ('nobody', 0, '2', 'https://a.test/2', 6, 'pending', 1)",
                        [],
                    ),
                    c.execute(
                        "INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name,
                             state, created_at, updated_at)
                         VALUES ('a2', 'j1', 1, 'k', 'x.ass', 'partial', 1, 1)",
                        [],
                    ),
                    // A file of a job is received once.
                    c.execute(
                        "INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name,
                             state, path, created_at, updated_at)
                         VALUES ('a3', 'j1', 1, 'k', 'x.ass', 'done', 'j1/x.ass', 1, 1),
                                ('a4', 'j1', 1, 'k', 'x.ass', 'done', 'j1/x (2).ass', 1, 1)",
                        [],
                    ),
                    c.execute(
                        "UPDATE subtitle_jobs SET state = 'waiting', wait = 'forever' WHERE id = 'j1'",
                        [],
                    ),
                ]
                .map(|r| r.is_err());
                c.execute("DELETE FROM subtitle_jobs WHERE id = 'j1'", [])?;
                let cascaded = count(
                    "SELECT (SELECT count(*) FROM subtitle_job_items)
                          + (SELECT count(*) FROM subtitle_job_steps)
                          + (SELECT count(*) FROM subtitle_job_events)
                          + (SELECT count(*) FROM subtitle_job_files)",
                )?;
                Ok((observations, jobs, refused, cascaded))
            })
            .await
            .unwrap();
        assert_eq!((observations, jobs), (1, 0));
        assert_eq!(refused, [true; 7]);
        assert_eq!(cascaded, 0);
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
        type Row = (String, i64, bool, Option<String>, Option<i64>, bool, i64);
        let rows: Vec<Row> = db
            .run::<_, DbError, _>(|c| {
                c.execute(
                    "UPDATE rules SET episode_basis = '이전 시즌이 24화까지예요.' WHERE id = 'derived'",
                    [],
                )?;
                let mut stmt = c.prepare(
                    "SELECT id, episode, episode_auto, episode_basis, episode_previous,
                            episode_decided, version
                       FROM rules ORDER BY id",
                )?;
                let rows = stmt.query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                })?;
                Ok(rows.collect::<rusqlite::Result<_>>()?)
            })
            .await
            .unwrap();
        // An automatic value from before has been decided; what it replaced
        // is not known, so it cannot be undone.
        assert_eq!(
            rows,
            vec![
                (
                    "derived".to_owned(),
                    -24,
                    true,
                    Some("이전 시즌이 24화까지예요.".to_owned()),
                    None,
                    true,
                    2
                ),
                ("typed".to_owned(), -12, false, None, None, false, 4),
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

    /// How many migrations come before the one that stored the first read of
    /// each channel (found by what it creates, so it stays right when other
    /// migrations are numbered ahead of it).
    fn before_first_reads() -> usize {
        MIGRATIONS
            .iter()
            .position(|m| {
                matches!(m, Migration::Sql(sql) if sql.contains("CREATE TABLE history_first_reads"))
            })
            .expect("the migration of the first reads")
    }

    #[tokio::test]
    async fn a_history_from_before_first_reads_takes_its_first_recorded_items_as_the_first_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            let conn = database_at(&path, before_first_reads());
            conn.execute_batch(
                "INSERT INTO history_items (channel_id, channel_label, identity_key, title, link,
                         first_seen_at, last_seen_at, result, result_at) VALUES
                     ('c1', 'feed', 'guid:b', 'B', 'x', 100, 900, 'no_match', 100),
                     ('c1', 'feed', 'guid:a', 'A', 'x', 300, 900, 'no_match', 300),
                     ('c1', 'feed', 'guid:c', 'C', 'x', 200, 900, 'no_match', 200),
                     ('c1', 'feed', 'guid:e', 'E', 'x', 100, 900, 'no_match', 100),
                     ('c1', 'feed', 'guid:f', 'F', 'x', 40, 900, 'no_match', 40),
                     ('c2', 'feed', 'guid:d', 'D', 'x', 50, 50, 'no_match', 50);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let found = db
            .run::<_, DbError, _>(|c| {
                let mut stmt = c.prepare(
                    "SELECT channel_id || '=' || first_read_at FROM history_first_reads
                     ORDER BY channel_id",
                )?;
                let found = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let items: i64 =
                    c.query_row("SELECT count(*) FROM history_items", [], |r| r.get(0))?;
                let marked: String = c.query_row(
                    "SELECT group_concat(identity_key, ',') FROM
                         (SELECT identity_key FROM history_items WHERE first_read = 1
                          ORDER BY identity_key)",
                    [],
                    |r| r.get(0),
                )?;
                Ok((found, items, marked))
            })
            .await
            .unwrap();
        assert_eq!(found.0, ["c1=100", "c2=50"]);
        assert_eq!(found.1, 6, "the records are untouched");
        assert_eq!(
            found.2, "guid:b,guid:d,guid:e",
            "the items first seen at the time of the channel's first recorded item are its \
             first read's, not the ones a clock that went back stamped earlier"
        );
    }

    #[tokio::test]
    async fn a_database_from_before_winpng_keeps_its_items_and_checks_the_new_classes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 43 migrations left it: a job with
            // a done item and an item that failed as `changed`.
            let conn = database_at(&path, 43);
            conn.execute_batch(
                "INSERT INTO subtitle_jobs (id, command_id, request, origin, state,
                     created_at, updated_at, state_at)
                     VALUES ('j1', 'c1', '{}', 'pick', 'partial', 1, 1, 1);
                 INSERT INTO subtitle_job_items (job_id, position, episode, post_url, found_at,
                     state, reason, failure, updated_at)
                     VALUES ('j1', 0, '1', 'https://a.tistory.com/1', 6, 'done', NULL, NULL, 1),
                            ('j1', 1, '2', 'https://a.tistory.com/2', 6, 'failed', '없어요',
                             'changed', 1);
                 INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state,
                     size, sha256, path, created_at, updated_at)
                     VALUES ('a1', 'j1', 1, 'k1', 'x.ass', 'done', 10, 'ab', 'j1/x.ass', 1, 1);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        type Kept = (i64, Option<String>, Option<String>, Option<String>);
        let (kept, refused, accepted): (Kept, [bool; 4], String) = db
            .run::<_, DbError, _>(|c| {
                let kept = c.query_row(
                    "SELECT (SELECT count(*) FROM subtitle_job_items),
                            (SELECT failure FROM subtitle_job_items WHERE position = 0),
                            (SELECT failure FROM subtitle_job_items WHERE position = 1),
                            (SELECT folder FROM subtitle_job_files WHERE id = 'a1')",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?;
                let update = |sql: &str| c.execute(sql, []).is_err();
                let refused = [
                    update("UPDATE subtitle_job_items SET failure = 'gone' WHERE position = 1"),
                    update("UPDATE subtitle_job_files SET failure = 'needs_input' WHERE id = 'a1'"),
                    update("UPDATE subtitle_job_files SET folder = '' WHERE id = 'a1'"),
                    update("UPDATE subtitle_job_items SET failure = 'timeout' WHERE position = 0"),
                ];
                c.execute_batch(
                    "UPDATE subtitle_job_items SET failure = 'no_subtitle' WHERE position = 0;
                     UPDATE subtitle_job_items SET failure = 'needs_input' WHERE position = 1;
                     UPDATE subtitle_job_files SET folder = '회차/2화' WHERE id = 'a1';",
                )?;
                let accepted = c.query_row(
                    "SELECT group_concat(failure, ',') FROM
                         (SELECT failure FROM subtitle_job_items ORDER BY position)",
                    [],
                    |r| r.get(0),
                )?;
                Ok((kept, refused, accepted))
            })
            .await
            .unwrap();
        assert_eq!(kept, (2, None, Some("changed".to_owned()), None));
        assert_eq!(refused, [true; 4]);
        assert_eq!(accepted, "no_subtitle,needs_input");
    }

    #[tokio::test]
    async fn a_database_from_before_the_remote_screen_gets_a_table_that_keeps_no_half_binding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with 44 migrations left it: a job that
            // waits for a person's check.
            let conn = database_at(&path, 44);
            conn.execute_batch(
                "INSERT INTO subtitle_jobs (id, command_id, request, origin, state, wait,
                     created_at, updated_at, state_at)
                     VALUES ('j1', 'c1', '{}', 'pick', 'waiting', 'auth', 1, 1, 1);
                 INSERT INTO subtitle_job_items (job_id, position, episode, post_url, found_at,
                     state, wait, updated_at)
                     VALUES ('j1', 0, '1', 'https://fake.trss.invalid/check/1', 6, 'waiting',
                             'auth', 1);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        type Kept = (Option<String>, Option<i64>);
        let (empty, refused, kept, left): (i64, [bool; 4], Kept, i64) = db
            .run::<_, DbError, _>(|c| {
                let count = |c: &Connection| {
                    c.query_row("SELECT count(*) FROM subtitle_job_screens", [], |r| {
                        r.get::<_, i64>(0)
                    })
                };
                let empty = count(c)?;
                let insert = |sql: &str| c.execute(sql, []).is_err();
                let refused = [
                    // A run without its page, and a page without its run.
                    insert(
                        "INSERT INTO subtitle_job_screens (job_id, item_id, run_id, bound_at,
                             updated_at) VALUES ('j1', 1, 'r', 5, 5)",
                    ),
                    insert(
                        "INSERT INTO subtitle_job_screens (job_id, item_id, target_id,
                             updated_at) VALUES ('j1', 1, 'T', 5)",
                    ),
                    // A run with no time it was bound.
                    insert(
                        "INSERT INTO subtitle_job_screens (job_id, item_id, run_id, target_id,
                             updated_at) VALUES ('j1', 1, 'r', 'T', 5)",
                    ),
                    // An item that is not there.
                    insert(
                        "INSERT INTO subtitle_job_screens (job_id, item_id, updated_at)
                             VALUES ('j1', 99, 5)",
                    ),
                ];
                c.execute(
                    "INSERT INTO subtitle_job_screens (job_id, item_id, run_id, target_id,
                         bound_at, updated_at) VALUES ('j1', 1, 'j1-abc', 'T1', 5, 5)",
                    [],
                )?;
                let kept = c.query_row(
                    "SELECT run_id, bound_at FROM subtitle_job_screens WHERE job_id = 'j1'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                // The row goes with its job.
                c.execute("DELETE FROM subtitle_jobs WHERE id = 'j1'", [])?;
                Ok((empty, refused, kept, count(c)?))
            })
            .await
            .unwrap();
        assert_eq!(empty, 0);
        assert_eq!(refused, [true; 4]);
        assert_eq!(kept, (Some("j1-abc".to_owned()), Some(5)));
        assert_eq!(left, 0);
    }

    const BEFORE_AWAITING_VIDEO: usize = 50;

    #[tokio::test]
    async fn a_job_an_earlier_build_finished_with_an_episode_without_a_video_goes_back_in_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // A database as the build with fifty migrations left it: `j1`
            // stored its subtitle only because the episode had no video, `j2`
            // applied its own, `j3` is held with such a row.
            let conn = database_at(&path, BEFORE_AWAITING_VIDEO);
            conn.execute_batch(
                "INSERT INTO subtitle_jobs (id, command_id, request, origin, work_id, season,
                                            state, created_at, updated_at, state_at, finished_at)
                     VALUES ('j1', 'c1', '{}', 'pick', 'w', 1, 'done', 0, 0, 0, 0),
                            ('j2', 'c2', '{}', 'pick', 'w', 1, 'done', 0, 0, 0, 0),
                            ('j3', 'c3', '{}', 'pick', 'w', 1, 'held', 0, 0, 0, NULL);
                 INSERT INTO subtitle_job_items (id, job_id, position, episode, post_url,
                                                 found_at, state, updated_at)
                     VALUES (1, 'j1', 0, '1', 'https://example.org/1', 0, 'done', 0),
                            (2, 'j2', 0, '2', 'https://example.org/2', 0, 'done', 0),
                            (3, 'j3', 0, '3', 'https://example.org/3', 0, 'done', 0);
                 INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state, size,
                                                 sha256, created_at, updated_at)
                     VALUES ('f1', 'j1', 1, 'k1', 'a.ass', 'done', 1, printf('%064d', 1), 0, 0),
                            ('f2', 'j2', 2, 'k2', 'b.ass', 'done', 1, printf('%064d', 2), 0, 0),
                            ('f3', 'j3', 3, 'k3', 'c.ass', 'done', 1, printf('%064d', 3), 0, 0);
                 INSERT INTO subtitle_job_plan (job_id, position, file_id, name, kind, format,
                                                size, sha256, assignment, episode, action,
                                                outcome, note, updated_at)
                     VALUES ('j1', 0, 'f1', 'a.ass', 'subtitle', 'ass', 1, printf('%064d', 1),
                             'explicit', 1, 'apply', 'no_video', '영상이 없어 보관만 했어요', 0),
                            ('j2', 0, 'f2', 'b.ass', 'subtitle', 'ass', 1, printf('%064d', 2),
                             'explicit', 2, 'apply', 'applied', NULL, 0),
                            ('j3', 0, 'f3', 'c.ass', 'subtitle', 'ass', 1, printf('%064d', 3),
                             'explicit', 3, 'apply', 'no_video', '영상이 없어 보관만 했어요', 0);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        let (jobs, notes) = db
            .run::<_, DbError, _>(|c| {
                let mut stmt = c.prepare(
                    "SELECT id || ':' || state || ':' || (finished_at IS NULL) FROM subtitle_jobs
                      ORDER BY id",
                )?;
                let jobs = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut stmt =
                    c.prepare("SELECT coalesce(note, '') FROM subtitle_job_plan ORDER BY job_id")?;
                let notes = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok((jobs, notes))
            })
            .await
            .unwrap();
        // A held job stays for a person; its row waits for the video too.
        assert_eq!(jobs, ["j1:pending:1", "j2:done:0", "j3:held:1"]);
        let waiting = "영상이 아직 없어 영상이 들어오면 적용해요";
        assert_eq!(notes, [waiting, "", waiting]);
    }

    /// Migration 51's database: replacements and their records come after it.
    const BEFORE_REPLACEMENT: usize = 51;

    #[tokio::test]
    async fn the_remade_package_and_effect_tables_keep_their_rows_and_references() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            let conn = database_at(&path, BEFORE_REPLACEMENT);
            conn.execute_batch(
                "INSERT INTO subtitle_jobs (id, command_id, request, origin, work_id, season,
                                            state, created_at, updated_at, state_at)
                     VALUES ('j1', 'c1', '{}', 'pick', 'w', 1, 'done', 0, 0, 0);
                 INSERT INTO subtitle_job_items (id, job_id, position, episode, post_url,
                                                 found_at, state, updated_at)
                     VALUES (1, 'j1', 0, '1', 'https://example.org/1', 0, 'done', 0);
                 INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state, size,
                                                 sha256, created_at, updated_at)
                     VALUES ('f1', 'j1', 1, 'k1', 'a.ass', 'done', 1, printf('%064d', 1), 0, 0);
                 INSERT INTO subtitle_packages (id, work_id, job_id, source_kind, source_page,
                                                received_at, created_at)
                     VALUES ('p1', 'w', 'j1', 'post', 'https://example.org/1', 5, 6);
                 INSERT INTO subtitle_assets (id, work_id, kind, base, relative_path, byte_size,
                                              sha256, created_at)
                     VALUES ('a1', 'w', 'subtitle', 'work', '.trss/subtitles/x/a.ass', 1,
                             printf('%064d', 1), 0);
                 INSERT INTO subtitle_package_entries (package_id, position, asset_id,
                                                       original_name)
                     VALUES ('p1', 0, 'a1', 'a.ass');
                 INSERT INTO subtitle_stored (id, work_id, season, package_id, subtitle_asset_id,
                                              assignment, episode, format, stored_at)
                     VALUES ('s1', 'w', 1, 'p1', 'a1', 'explicit', 1, 'ass', 0);
                 INSERT INTO subtitle_job_plan (job_id, position, file_id, name, kind, format,
                                                size, sha256, assignment, episode, action,
                                                stored_id, updated_at)
                     VALUES ('j1', 0, 'f1', 'a.ass', 'subtitle', 'ass', 1, printf('%064d', 1),
                             'explicit', 1, 'apply', 's1', 0);
                 INSERT INTO subtitle_file_effects (id, job_id, position, kind, state, folder,
                                                    temp, target, video, size, sha256,
                                                    object, created_at, updated_at)
                     VALUES ('e1', 'j1', 0, 'apply', 'prepared', '/w', '.trss/tmp/e1', 'v.ass',
                             'v.mkv', 1, printf('%064d', 1), '1:2', 0, 0);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        db.run::<_, DbError, _>(|c| {
            // The rows and what refers to them are as they were.
            let package: (String, Option<String>, Option<i64>) = c.query_row(
                "SELECT source_kind, source_page, received_at FROM subtitle_packages
                  WHERE id = 'p1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            assert_eq!(
                package,
                (
                    "post".to_owned(),
                    Some("https://example.org/1".to_owned()),
                    Some(5)
                )
            );
            let entries: i64 = c.query_row(
                "SELECT count(*) FROM subtitle_package_entries WHERE package_id = 'p1'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(entries, 1, "the remake cascaded nothing");
            let effect: (String, String, Option<String>) = c.query_row(
                "SELECT state, object, plan_id FROM subtitle_file_effects WHERE id = 'e1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            assert_eq!(effect, ("prepared".to_owned(), "1:2".to_owned(), None));
            let foreign: bool = c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
            assert!(foreign, "foreign keys are on again");
            let broken = c
                .prepare("PRAGMA foreign_key_check")?
                .query([])?
                .next()?
                .is_some();
            assert!(!broken);

            // A package imported from beside a video, and a plan for the row.
            c.execute_batch(
                "INSERT INTO subtitle_packages (id, work_id, job_id, source_kind, created_at)
                     VALUES ('p2', 'w', 'j1', 'existing', 0);
                 INSERT INTO subtitle_replacements
                     (id, job_id, position, version, state, work_id, season, episode, assignment,
                      folder, video_path, video_object, video_size, video_mtime, stored_id,
                      asset_id, asset_path, asset_size, asset_sha256, target, created_at,
                      updated_at)
                     VALUES ('r1', 'j1', 0, 1, 'open', 'w', 1, 1, 'explicit', '/w', 'v.mkv',
                             '1:3', 10, 11, 's1', 'a1', '.trss/subtitles/x/a.ass', 1,
                             printf('%064d', 1), 'v.ass', 0, 0);
                 INSERT INTO subtitle_replacement_paths (plan_id, path, action, byte_size, sha256,
                                                         object)
                     VALUES ('r1', 'v.ass', 'replace', 2, printf('%064d', 2), '1:4');",
            )?;
            // Its evidence does not change, and it only goes forward.
            let refused = |sql: &str| c.execute(sql, []).is_err();
            assert!(refused(
                "UPDATE subtitle_replacements SET video_size = 12 WHERE id = 'r1'"
            ));
            assert!(refused(
                "UPDATE subtitle_replacement_paths SET byte_size = 3"
            ));
            assert!(refused(
                "UPDATE subtitle_replacements SET state = 'done' WHERE id = 'r1'"
            ));
            c.execute(
                "UPDATE subtitle_replacements SET state = 'approved' WHERE id = 'r1'",
                [],
            )?;
            assert!(refused(
                "UPDATE subtitle_replacements SET state = 'open' WHERE id = 'r1'"
            ));
            // A row has one plan to decide or carry out at a time.
            assert!(refused(
                "INSERT INTO subtitle_replacements
                     (id, job_id, position, version, state, work_id, season, episode, assignment,
                      folder, video_path, video_object, video_size, video_mtime, stored_id,
                      asset_id, asset_path, asset_size, asset_sha256, target, created_at,
                      updated_at)
                     VALUES ('r2', 'j1', 0, 2, 'open', 'w', 1, 1, 'explicit', '/w', 'v.mkv',
                             '1:3', 10, 11, 's1', 'a1', '.trss/subtitles/x/a.ass', 1,
                             printf('%064d', 1), 'v.ass', 0, 0)"
            ));
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_reference_broken_before_the_remake_does_not_stop_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            // An item of no job, written with foreign keys off.
            let conn = database_at(&path, BEFORE_REPLACEMENT);
            conn.pragma_update(None, "foreign_keys", false).unwrap();
            conn.execute_batch(
                "INSERT INTO subtitle_job_items (id, job_id, position, episode, post_url,
                                                 found_at, state, updated_at)
                     VALUES (1, 'gone', 0, '1', 'https://example.org/1', 0, 'done', 0);",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
    }

    /// Migration 54's database: cleanups come after it.
    const BEFORE_CLEANUP: usize = 54;

    #[tokio::test]
    async fn stored_rows_survive_the_cleanup_migration_and_a_removed_asset_frees_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            let conn = database_at(&path, BEFORE_CLEANUP);
            conn.execute_batch(
                "INSERT INTO subtitle_jobs (id, command_id, request, origin, work_id, season,
                                            state, created_at, updated_at, state_at)
                     VALUES ('j1', 'c1', '{}', 'pick', 'w', 1, 'done', 0, 0, 0);
                 INSERT INTO subtitle_packages (id, work_id, job_id, source_kind, created_at)
                     VALUES ('p1', 'w', 'j1', 'post', 0);
                 INSERT INTO subtitle_assets (id, work_id, kind, base, relative_path, byte_size,
                                              sha256, created_at)
                     VALUES ('a1', 'w', 'subtitle', 'work', '.trss/subtitles/x/a.ass', 1,
                             printf('%064d', 1), 0),
                            ('a2', 'w', 'font', 'work', '.trss/subtitles/x/f.ttf', 2,
                             printf('%064d', 2), 0);
                 INSERT INTO subtitle_package_entries (package_id, position, asset_id,
                                                       original_name)
                     VALUES ('p1', 0, 'a1', 'a.ass'), ('p1', 1, 'a2', 'f.ttf');
                 INSERT INTO subtitle_stored (id, work_id, season, package_id, subtitle_asset_id,
                                              assignment, episode, format, stored_at)
                     VALUES ('s1', 'w', 1, 'p1', 'a1', 'explicit', 1, 'ass', 0);
                 INSERT INTO subtitle_stored_assets (stored_id, asset_id, role)
                     VALUES ('s1', 'a2', 'font');",
            )
            .unwrap();
        }

        let db = Db::open(&path).await.unwrap();

        assert_eq!(version_of(&db).await, MIGRATIONS.len());
        db.run::<_, DbError, _>(|c| {
            let kept: (i64, i64, i64, i64) = c.query_row(
                "SELECT (SELECT count(*) FROM subtitle_assets WHERE removed_at IS NULL),
                        (SELECT count(*) FROM subtitle_stored WHERE cleaned_at IS NULL),
                        (SELECT count(*) FROM subtitle_stored_assets),
                        (SELECT count(*) FROM subtitle_package_entries)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            assert_eq!(kept, (2, 1, 1, 2));
            let insert = |id: &str| {
                c.execute(
                    "INSERT INTO subtitle_assets (id, work_id, kind, base, relative_path,
                                                  byte_size, sha256, created_at)
                     VALUES (?1, 'w', 'subtitle', 'work', '.trss/subtitles/X/A.ass', 3,
                             printf('%064d', 3), 0)",
                    [id],
                )
            };
            // One path holds one asset, whatever its case, until it is removed.
            assert!(insert("a3").is_err());
            c.execute(
                "UPDATE subtitle_assets SET removed_at = 5 WHERE id = 'a1'",
                [],
            )?;
            insert("a3")?;
            assert!(insert("a4").is_err());
            // A stored subtitle has one asked cleanup at a time.
            c.execute_batch(
                "INSERT INTO subtitle_cleanups (id, work_id, stored_id, state, asked_at,
                                                updated_at)
                     VALUES ('k1', 'w', 's1', 'asked', 0, 0);
                 INSERT INTO subtitle_asset_removals (cleanup_id, asset_id, state)
                     VALUES ('k1', 'a2', 'named');",
            )?;
            assert!(c
                .execute(
                    "INSERT INTO subtitle_cleanups (id, work_id, stored_id, state, asked_at,
                                                    updated_at)
                         VALUES ('k2', 'w', 's1', 'asked', 0, 0)",
                    [],
                )
                .is_err());
            Ok(())
        })
        .await
        .unwrap();
    }

    #[test]
    fn a_reference_broken_again_alike_is_still_new() {
        let r = |s: &str| s.to_owned();
        // Rows of a table without rowids all read `row 0`.
        let before = vec![r("entries row 0 -> assets")];
        let after = vec![r("entries row 0 -> assets"), r("entries row 0 -> assets")];
        assert_eq!(
            newly_broken(before.clone(), after),
            [r("entries row 0 -> assets")]
        );
        assert_eq!(newly_broken(before.clone(), before), Vec::<String>::new());
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
