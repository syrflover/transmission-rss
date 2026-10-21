use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use super::*;

#[tokio::test]
async fn an_announced_change_reaches_every_subscriber() {
    let changes = Changes::default();
    let mut first = changes.subscribe();
    let mut second = changes.clone().subscribe();

    changes.announce(ChangeKind::Command, "c1");

    let expected = Change {
        kind: ChangeKind::Command,
        id: Some("c1".to_owned()),
    };
    assert_eq!(first.recv().await.unwrap(), expected);
    assert_eq!(second.recv().await.unwrap(), expected);
}

#[test]
fn announcing_with_no_listener_is_no_failure_and_a_late_listener_hears_only_what_follows() {
    let changes = Changes::default();
    changes.announce(ChangeKind::PastSearch, "before");

    let mut late = changes.subscribe();
    assert!(matches!(late.try_recv(), Err(TryRecvError::Empty)));

    changes.announce(ChangeKind::PastSearch, "after");
    assert_eq!(late.try_recv().unwrap().id.as_deref(), Some("after"));
}

#[tokio::test]
async fn a_listener_that_falls_behind_is_told_how_many_it_lost() {
    let changes = Changes::with_capacity(2);
    let mut listener = changes.subscribe();

    for n in 0..5 {
        changes.announce(ChangeKind::Command, format!("c{n}"));
    }

    assert!(matches!(listener.recv().await, Err(RecvError::Lagged(3))));
    // What the channel kept is still heard, in order.
    assert_eq!(listener.recv().await.unwrap().id.as_deref(), Some("c3"));
    assert_eq!(listener.recv().await.unwrap().id.as_deref(), Some("c4"));
}

#[test]
fn a_change_is_sent_as_its_kind_in_snake_case_and_its_id() {
    let change = Change {
        kind: ChangeKind::PastSearch,
        id: Some("s1".to_owned()),
    };
    assert_eq!(
        serde_json::to_string(&change).unwrap(),
        r#"{"kind":"past_search","id":"s1"}"#
    );
}

#[test]
fn a_change_without_a_target_is_sent_with_a_null_id() {
    let change = Change {
        kind: ChangeKind::Library,
        id: None,
    };
    assert_eq!(
        serde_json::to_string(&change).unwrap(),
        r#"{"kind":"library","id":null}"#
    );
}

#[test]
fn a_kind_is_named_as_it_is_sent() {
    for (_, how) in tables::TABLES {
        if let tables::Announced::As { kind, .. } = how {
            let sent = serde_json::to_string(kind).unwrap();
            assert_eq!(sent, format!("\"{}\"", kind.name()));
        }
    }
}

// ---------------------------------------------------------------------------
// What a database connection announces
// ---------------------------------------------------------------------------

mod from_the_connection {
    use std::collections::BTreeSet;

    use rusqlite::Connection;
    use tokio::sync::broadcast::Receiver;

    use super::super::tables::{Announced, TABLES};
    use super::super::*;
    use crate::db::{Db, DbError};

    /// A database whose writes are announced, and what its listener heard.
    struct World {
        _dir: tempfile::TempDir,
        path: std::path::PathBuf,
        db: Db,
        listener: Receiver<Change>,
    }

    impl World {
        fn new() -> World {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("app.db");
            let changes = Changes::default();
            let listener = changes.subscribe();
            let db = Db::open_blocking_announcing(&path, changes).unwrap();
            World {
                _dir: dir,
                path,
                db,
                listener,
            }
        }

        /// Runs `sql` (several statements) as one call.
        async fn exec(&self, sql: &'static str) {
            self.db
                .run::<_, DbError, _>(move |c| Ok(c.execute_batch(sql)?))
                .await
                .unwrap();
        }

        /// What was announced since the last call, as (kind, ID), in order of
        /// kind and ID, `None` for "no ID".
        fn heard(&mut self) -> Vec<(&'static str, Option<String>)> {
            let mut heard = Vec::new();
            while let Ok(change) = self.listener.try_recv() {
                heard.push((change.kind.name(), change.id));
            }
            heard.sort();
            heard
        }
    }

    fn one(kind: &'static str, id: &str) -> (&'static str, Option<String>) {
        (kind, Some(id.to_owned()))
    }

    fn any(kind: &'static str) -> (&'static str, Option<String>) {
        (kind, None)
    }

    const HISTORY_ITEM: &str = "INSERT INTO history_items
        (id, channel_id, channel_label, identity_key, title, link, first_seen_at,
         last_seen_at, result, result_at)
        VALUES (7, 'c', 'C', 'k', 't', 'l', 1, 1, 'added', 1)";

    const SOURCE: &str = "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
        VALUES ('s1', 12, 'someone', 1)";

    const FOLDER_AND_WORK: &str = "
        INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
        INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');";

    #[tokio::test]
    async fn every_table_of_the_migrated_schema_is_listed_with_a_kind_and_a_column_it_has() {
        let world = World::new();
        let (tables, columns) = world
            .db
            .run::<_, DbError, _>(|c| {
                let tables = c
                    .prepare(
                        "SELECT name FROM sqlite_master
                          WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
                    )?
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<Result<BTreeSet<_>, _>>()?;
                let mut columns = BTreeSet::new();
                for table in &tables {
                    for column in c
                        .prepare("SELECT name FROM pragma_table_info(?1)")?
                        .query_map([table], |r| r.get::<_, String>(0))?
                    {
                        columns.insert((table.clone(), column?));
                    }
                }
                Ok((tables, columns))
            })
            .await
            .unwrap();

        let listed: Vec<&str> = TABLES.iter().map(|(name, _)| *name).collect();
        let mut sorted = listed.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(listed, sorted, "TABLES is sorted by name, one entry each");

        let missing: Vec<_> = tables
            .iter()
            .filter(|t| !listed.contains(&t.as_str()))
            .collect();
        assert!(missing.is_empty(), "tables with no entry: {missing:?}");
        let gone: Vec<_> = listed.iter().filter(|t| !tables.contains(**t)).collect();
        assert!(
            gone.is_empty(),
            "entries of tables that do not exist: {gone:?}"
        );
        for (table, how) in TABLES {
            match how {
                Announced::As {
                    target: Some(column),
                    ..
                } => assert!(
                    columns.contains(&((*table).to_owned(), (*column).to_owned())),
                    "{table} has no column {column}"
                ),
                Announced::As { .. } => {}
                Announced::Not(reason) => assert!(!reason.is_empty(), "{table} needs a reason"),
            }
        }
    }

    #[tokio::test]
    async fn a_row_of_a_rowid_table_is_announced_when_it_is_inserted_changed_and_deleted() {
        let mut world = World::new();

        world.exec(HISTORY_ITEM).await;
        assert_eq!(world.heard(), [one("history", "7")]);

        world
            .exec("UPDATE history_items SET result = 'duplicate' WHERE id = 7")
            .await;
        assert_eq!(world.heard(), [one("history", "7")]);

        world.exec("DELETE FROM history_items WHERE id = 7").await;
        assert_eq!(world.heard(), [one("history", "7")]);
    }

    #[tokio::test]
    async fn a_row_of_a_without_rowid_table_is_announced_when_it_is_inserted_changed_and_deleted() {
        let mut world = World::new();

        world.exec(SOURCE).await;
        assert_eq!(world.heard(), [one("anissia", "12")]);

        world
            .exec("UPDATE subtitle_sources SET creator_name = 'other' WHERE id = 's1'")
            .await;
        assert_eq!(world.heard(), [one("anissia", "12")]);

        world
            .exec("DELETE FROM subtitle_sources WHERE id = 's1'")
            .await;
        assert_eq!(world.heard(), [one("anissia", "12")]);
    }

    #[tokio::test]
    async fn a_row_a_schema_trigger_made_is_announced() {
        let mut world = World::new();
        world
            .exec("INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1)")
            .await;
        world.heard();

        // The work's artwork row is made by a trigger of the schema, and the
        // first season's season info by another.
        world
            .exec("INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show')")
            .await;
        assert_eq!(world.heard(), [one("artwork", "w1"), one("library", "w1")]);

        world
            .exec("INSERT INTO seasons (work_id, number) VALUES ('w1', 1)")
            .await;
        assert_eq!(
            world.heard(),
            [one("library", "w1"), one("season_info", "w1")]
        );
    }

    #[tokio::test]
    async fn a_row_a_foreign_key_cascade_deleted_is_announced() {
        let mut world = World::new();
        world.exec(FOLDER_AND_WORK).await;
        world
            .exec(
                "INSERT INTO seasons (work_id, number) VALUES ('w1', 1);
                 INSERT INTO episodes (work_id, season, episode) VALUES ('w1', 1, '01');",
            )
            .await;
        world.heard();

        // The folder's delete takes the work with its artwork, seasons and
        // season info.
        world
            .exec("DELETE FROM watch_folders WHERE id = 'f1'")
            .await;

        assert_eq!(
            world.heard(),
            [
                one("artwork", "w1"),
                any("library"),
                one("library", "w1"),
                one("season_info", "w1"),
            ]
        );
    }

    #[tokio::test]
    async fn a_delete_without_where_announces_every_row_it_removed() {
        let mut world = World::new();
        world
            .exec(
                "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                 VALUES ('a', 1, 'x', 1), ('b', 2, 'x', 1), ('c', 2, 'y', 1)",
            )
            .await;
        world.heard();

        world.exec("DELETE FROM subtitle_sources").await;

        assert_eq!(world.heard(), [one("anissia", "1"), one("anissia", "2")]);
    }

    #[tokio::test]
    async fn an_update_of_the_target_column_announces_the_old_and_the_new_id() {
        let mut world = World::new();
        world.exec(SOURCE).await;
        world.heard();

        world
            .exec("UPDATE subtitle_sources SET anime_no = 40 WHERE id = 's1'")
            .await;

        assert_eq!(world.heard(), [one("anissia", "12"), one("anissia", "40")]);
    }

    #[tokio::test]
    async fn a_row_with_no_target_is_announced_with_none() {
        let mut world = World::new();

        world
            .exec("INSERT INTO collection_cycle (id, started_at) VALUES (1, 5)")
            .await;

        assert_eq!(world.heard(), [any("collect_status")]);
    }

    #[tokio::test]
    async fn a_rolled_back_transaction_announces_nothing() {
        let mut world = World::new();

        let rolled_back = world
            .db
            .run::<_, DbError, _>(|c| {
                let tx = c.transaction()?;
                tx.execute_batch(HISTORY_ITEM)?;
                tx.execute_batch(SOURCE)?;
                tx.rollback()?;
                Ok(())
            })
            .await;
        rolled_back.unwrap();
        assert_eq!(world.heard(), []);

        // Nothing stays noted for the next write to announce.
        world
            .exec("INSERT INTO collection_cycle (id, started_at) VALUES (1, 5)")
            .await;
        assert_eq!(world.heard(), [any("collect_status")]);
    }

    #[tokio::test]
    async fn a_write_that_changes_no_row_announces_nothing() {
        let mut world = World::new();
        world.exec(SOURCE).await;
        world.heard();

        world
            .exec(
                "DELETE FROM history_items WHERE id = 99;
                 UPDATE subtitle_sources SET creator_name = 'z' WHERE id = 'none';
                 INSERT OR IGNORE INTO subtitle_sources (id, anime_no, creator_name, created_at)
                 VALUES ('s1', 12, 'someone', 1);",
            )
            .await;

        assert_eq!(world.heard(), []);
    }

    #[tokio::test]
    async fn a_call_that_only_reads_announces_nothing() {
        let mut world = World::new();
        world.exec(SOURCE).await;
        world.heard();

        let count = world
            .db
            .run::<_, DbError, _>(|c| {
                Ok(
                    c.query_row("SELECT count(*) FROM subtitle_sources", [], |r| {
                        r.get::<_, i64>(0)
                    })?,
                )
            })
            .await
            .unwrap();

        assert_eq!(count, 1);
        assert_eq!(world.heard(), []);
    }

    #[tokio::test]
    async fn the_announcement_comes_after_the_commit() {
        let mut world = World::new();
        let during = world.listener.resubscribe();
        let path = world.path.clone();

        let (before_commit, from_another_connection) = world
            .db
            .run::<_, DbError, _>(move |c| {
                let mut during = during;
                let tx = c.transaction()?;
                tx.execute_batch(SOURCE)?;
                // The write is made, not committed: nothing is told, and
                // another connection does not see it.
                let before_commit = during.try_recv().is_err();
                let other = Connection::open(&path)?;
                let seen_before: i64 =
                    other.query_row("SELECT count(*) FROM subtitle_sources", [], |r| r.get(0))?;
                tx.commit()?;
                Ok((before_commit, seen_before))
            })
            .await
            .unwrap();
        assert!(before_commit);
        assert_eq!(from_another_connection, 0);

        // On hearing it, another connection reads the new row.
        let change = world.listener.recv().await.unwrap();
        assert_eq!(change.id.as_deref(), Some("12"));
        let other = Connection::open(&world.path).unwrap();
        let seen: i64 = other
            .query_row(
                "SELECT count(*) FROM subtitle_sources WHERE anime_no = 12",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(seen, 1);
    }

    #[tokio::test]
    async fn a_transaction_announces_each_kind_and_id_once() {
        let mut world = World::new();
        world
            .exec(
                "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                 VALUES ('a', 1, 'x', 1), ('b', 1, 'y', 1), ('c', 2, 'x', 1);
                 UPDATE subtitle_sources SET created_at = 2;
                 INSERT INTO channels (id, position, url, excludes, secret_query, version)
                 VALUES ('c1', 0, 'http://x/', '[]', '[]', 1),
                        ('c2', 1, 'http://y/', '[]', '[]', 1);",
            )
            .await;

        assert_eq!(
            world.heard(),
            [one("anissia", "1"), one("anissia", "2"), any("rules")]
        );
    }

    #[tokio::test]
    async fn a_statement_with_its_own_conflict_handling_announces_without_failing() {
        let mut world = World::new();
        world
            .exec("INSERT INTO collection_cycle (id, started_at) VALUES (1, 5)")
            .await;
        world.heard();

        // The statements fire the trigger twice for one target (an upsert's
        // update, and a replacing insert's insert), and name conflict handling
        // that a trigger's own statement would take over.
        world
            .exec(
                "INSERT INTO collection_cycle (id, started_at) VALUES (1, 6)
                   ON CONFLICT (id) DO UPDATE SET started_at = 6;
                 INSERT OR REPLACE INTO collection_cycle (id, started_at) VALUES (1, 7);
                 UPDATE OR FAIL collection_cycle SET finished_at = 8;
                 UPDATE OR ROLLBACK collection_cycle SET finished_at = 9;
                 UPDATE OR ABORT collection_cycle SET finished_at = 10;
                 UPDATE OR REPLACE collection_cycle SET finished_at = 11;",
            )
            .await;

        assert_eq!(world.heard(), [any("collect_status")]);
    }

    #[tokio::test]
    async fn a_transaction_that_a_later_call_ends_announces_when_it_ends() {
        let mut world = World::new();

        world
            .exec("BEGIN; INSERT INTO collection_cycle (id, started_at) VALUES (1, 5);")
            .await;
        assert_eq!(world.heard(), []);
        world.exec("COMMIT").await;

        assert_eq!(world.heard(), [any("collect_status")]);
    }

    #[tokio::test]
    async fn a_call_that_panics_announces_what_it_committed_and_not_what_it_rolled_back() {
        let mut world = World::new();

        let panicked = world
            .db
            .run::<(), DbError, _>(|c| {
                c.execute_batch(SOURCE).unwrap();
                let tx = c.transaction().unwrap();
                tx.execute_batch(HISTORY_ITEM).unwrap();
                panic!("a bug after a committed write");
            })
            .await;

        assert!(matches!(panicked, Err(DbError::Task(e)) if e.is_panic()));
        // Heard without another call: a later call might only read, which
        // drains nothing.
        assert_eq!(world.heard(), [one("anissia", "12")]);
        world.exec("SELECT 1").await;
        assert_eq!(world.heard(), []);
    }

    #[tokio::test]
    async fn a_command_is_announced_once_by_its_store_and_not_by_the_connection() {
        let dir = tempfile::tempdir().unwrap();
        let changes = Changes::default();
        let mut heard = changes.subscribe();
        let db = Db::open_blocking_announcing(dir.path().join("app.db"), changes.clone()).unwrap();
        let commands = crate::commands::CommandStore::new(db).announcing(changes);

        commands
            .accept(
                crate::commands::NewCommand {
                    id: "cmd-1".to_owned(),
                    kind: "receive_once".to_owned(),
                    payload: "{}".to_owned(),
                    subject: None,
                },
                1_000,
            )
            .await
            .unwrap();

        let first = heard.try_recv().unwrap();
        assert_eq!(
            (first.kind, first.id.as_deref()),
            (ChangeKind::Command, Some("cmd-1"))
        );
        assert!(heard.try_recv().is_err(), "one announcement for one accept");
    }

    /// Run by `dev/measure/db-announce-cost.sh`, not by a test run: how long
    /// the connection's noting of changes takes per written row.
    #[tokio::test]
    #[ignore = "a measurement: dev/measure/db-announce-cost.sh runs it"]
    async fn measure_what_noting_changes_costs_per_row() {
        use std::time::Instant;
        const N: usize = 10_000;

        async fn rows_in_one_transaction(db: &Db) -> (f64, f64) {
            db.run::<_, DbError, _>(|c| {
                let started = Instant::now();
                let tx = c.transaction()?;
                {
                    let mut insert = tx.prepare(
                        "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                         VALUES (?1, ?2, ?1, 1)",
                    )?;
                    for n in 0..N {
                        insert.execute(rusqlite::params![format!("s{n}"), 1 + (n % 50) as i64])?;
                    }
                }
                tx.commit()?;
                let inserted = started.elapsed().as_secs_f64();
                let started = Instant::now();
                let tx = c.transaction()?;
                {
                    let mut update =
                        tx.prepare("UPDATE subtitle_sources SET created_at = ?1 WHERE id = ?2")?;
                    for n in 0..N {
                        update.execute(rusqlite::params![2 + n as i64, format!("s{n}")])?;
                    }
                }
                tx.commit()?;
                Ok((inserted, started.elapsed().as_secs_f64()))
            })
            .await
            .unwrap()
        }

        async fn rows_one_by_one(db: &Db) -> (f64, f64, f64) {
            let started = Instant::now();
            for n in 0..N {
                db.run::<_, DbError, _>(move |c| {
                    Ok(c.execute(
                        "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                         VALUES (?1, ?2, ?1, 1)",
                        rusqlite::params![format!("t{n}"), 1 + (n % 50) as i64],
                    )
                    .map(|_| ())?)
                })
                .await
                .unwrap();
            }
            let inserted = started.elapsed().as_secs_f64();
            let started = Instant::now();
            for n in 0..N {
                db.run::<_, DbError, _>(move |c| {
                    Ok(c.execute(
                        "UPDATE subtitle_sources SET created_at = ?1 WHERE id = ?2",
                        rusqlite::params![2 + n as i64, format!("t{n}")],
                    )
                    .map(|_| ())?)
                })
                .await
                .unwrap();
            }
            let updated = started.elapsed().as_secs_f64();
            let started = Instant::now();
            for _ in 0..N {
                db.run::<_, DbError, _>(|c| {
                    Ok(
                        c.query_row("SELECT count(*) FROM subtitle_sources", [], |r| {
                            r.get::<_, i64>(0)
                        })
                        .map(|_| ())?,
                    )
                })
                .await
                .unwrap();
            }
            (inserted, updated, started.elapsed().as_secs_f64())
        }

        let profile = if cfg!(debug_assertions) {
            "dev"
        } else {
            "release"
        };
        for announcing in [false, true, false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("app.db");
            let opened = Instant::now();
            let db = if announcing {
                Db::open_blocking_announcing(&path, Changes::default()).unwrap()
            } else {
                Db::open_blocking(&path).unwrap()
            };
            let open = opened.elapsed().as_secs_f64();
            let (one_insert, one_update) = rows_in_one_transaction(&db).await;
            let (insert, update, read) = rows_one_by_one(&db).await;
            let per_us = |seconds: f64| seconds / N as f64 * 1e6;
            println!(
                "{profile} announcing={announcing} open={open:.4}s \
                 one transaction: insert {:.2} us/row, update {:.2} us/row; \
                 one row per call: insert {:.2} us, update {:.2} us, read-only call {:.2} us",
                per_us(one_insert),
                per_us(one_update),
                per_us(insert),
                per_us(update),
                per_us(read),
            );
        }
    }
}
