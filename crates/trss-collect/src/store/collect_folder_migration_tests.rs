//! The migration that replaced the channels' base folders with the collect
//! folder (migration 7, in `trss-core`), checked against [`crate::rss::save_path`],
//! the join it must leave unchanged.

use std::path::Path;

use rusqlite::Connection;

use trss_core::{
    db::{database_at, Db, DbError},
    settings::SettingsStore,
};

use crate::rss::save_path;

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

/// Opens a database at version 6 holding `channels` (id, base folder), expects
/// the migration to fail, checks nothing changed, and returns the error message.
async fn failed_migration(channels: &[(&str, &str)]) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    {
        let conn = database_at(&path, BEFORE);
        for (i, (id, base)) in channels.iter().enumerate() {
            channel(&conn, id, i as i64, base);
        }
        rule(&conn, "r1", channels[0].0, 0, "Show", "active");
    }

    let message = match Db::open(&path).await {
        Err(DbError::Migration(message)) => message,
        other => panic!("expected a migration error, got {:?}", other.map(|_| ())),
    };

    // Still the previous schema, with its data, so a fixed configuration can retry.
    let conn = Connection::open(&path).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, BEFORE as i64);
    let base: String = conn
        .query_row(
            "SELECT base_dir FROM channels WHERE id = ?1",
            [channels[0].0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(base, channels[0].1);
    let directory: String = conn
        .query_row("SELECT directory FROM rules WHERE id = 'r1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(directory, "Show");
    let tables: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'collection_settings'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
    message
}

#[tokio::test]
async fn folders_with_no_common_parent_stop_the_migration_and_change_nothing() {
    let message = failed_migration(&[("a", "relative/a"), ("b", "/absolute/b")]).await;
    assert!(message.contains("share no parent folder"), "{message}");
    assert!(message.contains("`relative/a` (channel a)"), "{message}");
    assert!(message.contains("`/absolute/b` (channel b)"), "{message}");
}

#[tokio::test]
async fn a_base_that_cannot_be_folded_byte_for_byte_is_named_not_blamed_on_the_parent() {
    // Two trailing slashes cannot be put back: the folders do share a parent.
    let message = failed_migration(&[("a", "/d/a"), ("b", "/d/b//")]).await;
    assert!(!message.contains("share no parent folder"), "{message}");
    assert!(message.contains("`/d/b//` (channel b)"), "{message}");
    assert!(!message.contains("channel a"), "{message}");
    assert!(message.contains("single slashes"), "{message}");
}

#[tokio::test]
async fn an_empty_base_is_named_with_its_channel() {
    let message = failed_migration(&[("a", "/d/a"), ("b", ""), ("c", "/d/c")]).await;
    assert!(message.contains("empty base folder"), "{message}");
    assert!(message.contains("`` (channel b)"), "{message}");
    assert!(!message.contains("channel a"), "{message}");
    assert!(!message.contains("channel c"), "{message}");
}
