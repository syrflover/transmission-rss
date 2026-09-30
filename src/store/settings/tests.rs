use std::path::Path;

use rusqlite::Connection;

use super::*;
use crate::rss::save_path;
use crate::store::db::{database_at, Db, DbError};

/// The migration under test is the seventh; a database at version 6 is the
/// last one that still has `channels.base_dir`.
const BEFORE: usize = 6;

fn channel(conn: &Connection, id: &str, position: i64, base_dir: &str) {
    conn.execute(
        "INSERT INTO channels (id, position, url, base_dir, excludes, secret_query, version)
         VALUES (?1, ?2, 'http://x/', ?3, '[]', '[]', 1)",
        rusqlite::params![id, position, base_dir],
    )
    .unwrap();
}

fn rule(
    conn: &Connection,
    id: &str,
    channel_id: &str,
    position: i64,
    directory: &str,
    state: &str,
) {
    conn.execute(
        "INSERT INTO rules (id, channel_id, position, match_text, regex, case_insensitive,
                            directory, episode, episode_auto, state, version)
         VALUES (?1, ?2, ?3, 'm', 0, 0, ?4, 1, 0, ?5, 4)",
        rusqlite::params![id, channel_id, position, directory, state],
    )
    .unwrap();
}

/// What each rule saves to before the migration: the channel's base folder
/// with the rule's directory joined on, as the cycle computed it.
fn save_paths_before(conn: &Connection) -> Vec<(String, std::ffi::OsString)> {
    let mut stmt = conn
        .prepare(
            "SELECT rules.id, channels.base_dir, rules.directory
             FROM rules JOIN channels ON channels.id = rules.channel_id ORDER BY rules.id",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .map(|row| {
            let (id, base, directory) = row.unwrap();
            (
                id,
                save_path(Path::new(&base), Path::new(&directory)).into_os_string(),
            )
        })
        .collect();
    rows
}

/// The same, after the migration, from the collect folder and each rule.
async fn save_paths_after(db: &Db, collect: &str) -> Vec<(String, std::ffi::OsString)> {
    let collect = collect.to_owned();
    db.run::<_, DbError, _>(move |c| {
        let mut stmt = c.prepare("SELECT id, directory FROM rules ORDER BY id")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .map(|row| {
                let (id, directory) = row.unwrap();
                (
                    id,
                    save_path(Path::new(&collect), Path::new(&directory)).into_os_string(),
                )
            })
            .collect();
        Ok(rows)
    })
    .await
    .unwrap()
}

async fn directories(db: &Db) -> Vec<(String, String, i64)> {
    db.run::<_, DbError, _>(|c| {
        let mut stmt = c.prepare("SELECT id, directory, version FROM rules ORDER BY id")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    })
    .await
    .unwrap()
}

async fn has_base_dir_column(db: &Db) -> bool {
    db.run::<_, DbError, _>(|c| {
        let mut stmt = c.prepare("SELECT name FROM pragma_table_info('channels')")?;
        let names: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(names.iter().any(|n| n == "base_dir"))
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn channels_sharing_one_folder_make_it_the_collect_folder_and_rules_are_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let before = {
        let conn = database_at(&path, BEFORE);
        channel(&conn, "a", 0, "/downloads/Shows (current)");
        channel(&conn, "b", 1, "/downloads/Shows (current)");
        rule(&conn, "r1", "a", 0, "Clevatess/Season 02", "active");
        rule(&conn, "r2", "a", 1, "", "active");
        rule(&conn, "r3", "b", 0, "Slime/Season 04", "archived");
        save_paths_before(&conn)
    };

    let db = Db::open(&path).await.unwrap();

    let settings = SettingsStore::new(db.clone())
        .collection()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(settings.folder, "/downloads/Shows (current)");
    assert_eq!(settings.archive_folder, None);
    assert_eq!(settings.version, 1);
    assert!(!has_base_dir_column(&db).await);
    // Not a byte of a rule changed, not even its version.
    assert_eq!(
        directories(&db).await,
        [
            ("r1".to_owned(), "Clevatess/Season 02".to_owned(), 4),
            ("r2".to_owned(), String::new(), 4),
            ("r3".to_owned(), "Slime/Season 04".to_owned(), 4),
        ]
    );
    assert_eq!(save_paths_after(&db, &settings.folder).await, before);
}

#[tokio::test]
async fn channels_in_different_folders_share_their_common_parent_with_the_rest_in_front_of_the_rules(
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let before = {
        let conn = database_at(&path, BEFORE);
        channel(&conn, "a", 0, "/downloads/Shows (current)");
        channel(&conn, "b", 1, "/downloads/Movies");
        channel(&conn, "c", 2, "/downloads/Shows (current)/");
        rule(&conn, "a1", "a", 0, "Clevatess/Season 02", "active");
        // A rule that saves into the channel folder itself: its path ends in `/`.
        rule(&conn, "a2", "a", 1, "", "active");
        rule(&conn, "b1", "b", 0, "Dune", "active");
        rule(&conn, "b2", "b", 1, "", "archived");
        rule(&conn, "c1", "c", 0, "Other", "active");
        rule(&conn, "c2", "c", 1, "", "active");
        save_paths_before(&conn)
    };

    let db = Db::open(&path).await.unwrap();

    let settings = SettingsStore::new(db.clone())
        .collection()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(settings.folder, "/downloads");
    assert_eq!(
        directories(&db).await,
        [
            (
                "a1".to_owned(),
                "Shows (current)/Clevatess/Season 02".to_owned(),
                5
            ),
            ("a2".to_owned(), "Shows (current)/".to_owned(), 5),
            ("b1".to_owned(), "Movies/Dune".to_owned(), 5),
            ("b2".to_owned(), "Movies/".to_owned(), 5),
            ("c1".to_owned(), "Shows (current)/Other".to_owned(), 5),
            ("c2".to_owned(), "Shows (current)/".to_owned(), 5),
        ]
    );
    // Every rule, archived ones and the empty directory included, saves to the
    // same text as before.
    assert_eq!(save_paths_after(&db, &settings.folder).await, before);
    assert_eq!(before.len(), 6);
    assert!(!has_base_dir_column(&db).await);
}

#[tokio::test]
async fn no_channels_leave_the_collect_folder_unset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    drop(database_at(&path, BEFORE));

    let db = Db::open(&path).await.unwrap();

    assert_eq!(
        SettingsStore::new(db.clone()).collection().await.unwrap(),
        None
    );
    assert!(!has_base_dir_column(&db).await);
}

#[tokio::test]
async fn folders_with_no_common_parent_stop_the_migration_and_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    {
        let conn = database_at(&path, BEFORE);
        channel(&conn, "a", 0, "relative/a");
        channel(&conn, "b", 1, "/absolute/b");
        rule(&conn, "r1", "a", 0, "Show", "active");
    }

    match Db::open(&path).await {
        Err(DbError::Migration(_)) => {}
        other => panic!("expected a migration error, got {:?}", other.map(|_| ())),
    }

    // Still the previous schema, with its data, so a fixed configuration can retry.
    let conn = Connection::open(&path).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, BEFORE as i64);
    let base: String = conn
        .query_row("SELECT base_dir FROM channels WHERE id = 'a'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(base, "relative/a");
    let tables: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'collection_settings'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
}

async fn store() -> SettingsStore {
    SettingsStore::new(Db::open(":memory:").await.unwrap())
}

#[tokio::test]
async fn the_collection_settings_are_unset_until_written_and_versioned_after() {
    let store = store().await;
    assert_eq!(store.collection().await.unwrap(), None);

    let first = store
        .put_collection(
            0,
            "/downloads/Shows (current)".into(),
            Some("/downloads/Shows".into()),
        )
        .await
        .unwrap();
    assert_eq!(first.version, 1);
    assert_eq!(first.archive_folder.as_deref(), Some("/downloads/Shows"));
    assert_eq!(store.collection().await.unwrap(), Some(first));

    let second = store
        .put_collection(1, "/downloads/Shows (current)".into(), None)
        .await
        .unwrap();
    assert_eq!((second.version, second.archive_folder), (2, None));
}

#[tokio::test]
async fn a_write_from_a_stale_version_changes_nothing() {
    let store = store().await;
    // Nothing exists yet, so a version other than 0 is stale.
    let err = store
        .put_collection(3, "/a".into(), None)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        SettingsError::Conflict {
            expected: 3,
            actual: 0
        }
    ));
    assert_eq!(store.collection().await.unwrap(), None);

    store.put_collection(0, "/a".into(), None).await.unwrap();
    // The first writer's version no longer matches after a second write.
    store.put_collection(1, "/b".into(), None).await.unwrap();
    let err = store
        .put_collection(1, "/c".into(), None)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        SettingsError::Conflict {
            expected: 1,
            actual: 2
        }
    ));
    assert_eq!(store.collection().await.unwrap().unwrap().folder, "/b");
    // A creation over an existing row is stale too.
    let err = store
        .put_collection(0, "/d".into(), None)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        SettingsError::Conflict {
            expected: 0,
            actual: 2
        }
    ));
}

#[tokio::test]
async fn an_empty_folder_is_refused() {
    let store = store().await;
    assert!(matches!(
        store.put_collection(0, String::new(), None).await,
        Err(SettingsError::Invalid(_))
    ));
    assert!(matches!(
        store
            .put_collection(0, "/a".into(), Some(String::new()))
            .await,
        Err(SettingsError::Invalid(_))
    ));
    assert_eq!(store.collection().await.unwrap(), None);
}
