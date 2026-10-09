use super::*;
use trss_core::{Db, DbError};

mod stored {
    use super::*;

    async fn db() -> Db {
        let db = Db::open(":memory:").await.unwrap();
        db.run::<_, DbError, _>(|c| {
            c.execute_batch(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
                 INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
                 INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('s1', 3441, '에루샤', 1);",
            )?;
            Ok(())
        })
        .await
        .unwrap();
        db
    }

    fn zero() -> Decided {
        Decided {
            offset: Some(0),
            evidence: "0 근거".into(),
        }
    }

    fn plus_one() -> Decided {
        Decided {
            offset: Some(1),
            evidence: "+1 근거".into(),
        }
    }

    fn nothing() -> Decided {
        Decided {
            offset: None,
            evidence: "근거 없음".into(),
        }
    }

    fn put(c: &Connection, decided: &Decided, now: Millis) -> Mapping {
        store_in(c, "w1", 1, "s1", decided, now).unwrap()
    }

    fn row(c: &Connection) -> (String, Option<i64>, Option<i64>) {
        c.query_row(
            "SELECT kind, episode_offset, retired_offset FROM subtitle_episode_mappings",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn an_auto_mapping_is_never_changed_to_another_offset_by_itself() {
        let db = db().await;
        db.run::<_, DbError, _>(|c| {
            assert_eq!(put(c, &zero(), 10).offset, Some(0));
            // The same again is kept as it is, with its time.
            assert_eq!(put(c, &zero(), 20).decided_at, 10);
            // Another offset takes it back and says both.
            let m = put(c, &plus_one(), 30);
            assert_eq!((m.kind, m.offset), (MappingKind::Undecided, None));
            assert_eq!(
                m.evidence,
                "자동으로 정한 차이(0)와 다른 차이(+1)를 가리키는 회차가 생겼어요"
            );
            assert_eq!(row(c), ("undecided".into(), None, Some(0)));
            // Decided again to +1 on a later look: still refused.
            let m = put(c, &plus_one(), 40);
            assert_eq!(m.kind, MappingKind::Undecided);
            assert_eq!(row(c).2, Some(0));
            // Grounds that decide nothing do not free it either.
            put(c, &nothing(), 50);
            assert_eq!(row(c), ("undecided".into(), None, Some(0)));
            put(c, &plus_one(), 60);
            assert_eq!(row(c).0, "undecided");
            // The offset it had is decided again: it is auto once more.
            let m = put(c, &zero(), 70);
            assert_eq!((m.kind, m.offset), (MappingKind::Auto, Some(0)));
            assert_eq!(row(c), ("auto".into(), Some(0), None));
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn an_auto_mapping_with_no_grounds_left_keeps_the_offset_it_had_to_decide_again() {
        let db = db().await;
        db.run::<_, DbError, _>(|c| {
            put(c, &zero(), 10);
            // The grounds come to nothing: undecided, with the new reason.
            let m = put(c, &nothing(), 20);
            assert_eq!(
                (m.kind, m.evidence.as_str()),
                (MappingKind::Undecided, "근거 없음")
            );
            assert_eq!(row(c), ("undecided".into(), None, Some(0)));
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_mapping_that_was_never_auto_is_decided_to_whatever_the_grounds_say() {
        let db = db().await;
        db.run::<_, DbError, _>(|c| {
            put(c, &nothing(), 10);
            assert_eq!(row(c), ("undecided".into(), None, None));
            let m = put(c, &plus_one(), 20);
            assert_eq!((m.kind, m.offset), (MappingKind::Auto, Some(1)));
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn the_users_mapping_is_not_touched() {
        let db = db().await;
        db.run::<_, DbError, _>(|c| {
            c.execute(
                "INSERT INTO subtitle_episode_mappings
                     (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
                 VALUES ('w1', 1, 's1', 'user', 2, '사용자가 정했어요', 5)",
                [],
            )?;
            for decided in [zero(), plus_one(), nothing()] {
                let m = put(c, &decided, 99);
                assert_eq!(
                    (m.kind, m.offset, m.decided_at),
                    (MappingKind::User, Some(2), 5)
                );
            }
            assert_eq!(row(c), ("user".into(), Some(2), None));
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn the_conflicts_are_rewritten_to_the_current_set_and_keep_when_they_were_found() {
        let db = db().await;
        db.run::<_, DbError, _>(|c| {
            let conflict = |episode: &str, reason: &str| Conflict {
                episode: episode.into(),
                reason: reason.into(),
            };
            let all = |c: &Connection| -> Vec<(String, String, Millis)> {
                let mut stmt = c
                    .prepare(
                        "SELECT episode, reason, found_at FROM subtitle_mapping_conflicts
                          ORDER BY episode",
                    )
                    .unwrap();
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .unwrap();
                rows.collect::<rusqlite::Result<_>>().unwrap()
            };
            let row = |episode: &str, reason: &str, at| (episode.to_owned(), reason.to_owned(), at);
            store_conflicts(
                c,
                "w1",
                1,
                "s1",
                &[conflict("13.5", "a"), conflict("SP", "b")],
                10,
            )
            .unwrap();
            assert_eq!(all(c), [row("13.5", "a", 10), row("SP", "b", 10)]);
            // `SP` stays (as it was found), `13.5` has another reason, `14` is new
            // and `13.5`'s reason is the latest.
            store_conflicts(
                c,
                "w1",
                1,
                "s1",
                &[
                    conflict("13.5", "c"),
                    conflict("SP", "b"),
                    conflict("14", "d"),
                ],
                20,
            )
            .unwrap();
            assert_eq!(
                all(c),
                [row("13.5", "c", 10), row("14", "d", 20), row("SP", "b", 10)]
            );
            // An episode that no longer conflicts goes.
            store_conflicts(c, "w1", 1, "s1", &[conflict("SP", "b")], 30).unwrap();
            assert_eq!(all(c), [row("SP", "b", 10)]);
            assert_eq!(
                conflicts_in(c, "w1", 1, "s1").unwrap(),
                [conflict("SP", "b")]
            );
            store_conflicts(c, "w1", 1, "s1", &[], 40).unwrap();
            assert!(all(c).is_empty());
            Ok(())
        })
        .await
        .unwrap();
    }

    // --- what the user sets -------------------------------------------------------------

    fn user(offset: i64, exceptions: &[(&str, Option<i64>)]) -> UserMapping {
        UserMapping::new(
            offset,
            exceptions
                .iter()
                .map(|(episode, target)| ((*episode).to_owned(), *target))
                .collect(),
        )
        .unwrap()
    }

    fn done(saved: Saved) -> Mapping {
        match saved {
            Saved::Done(Some(m)) => m,
            other => panic!("{other:?}"),
        }
    }

    fn exceptions_of(c: &Connection) -> Vec<(String, String, Option<u32>)> {
        let mut stmt = c
            .prepare(
                "SELECT episode_key, episode, target FROM subtitle_episode_exceptions
                  ORDER BY episode_key",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        rows.collect::<rusqlite::Result<_>>().unwrap()
    }

    #[tokio::test]
    async fn a_save_stores_the_offset_and_its_exceptions_as_the_users_mapping() {
        let db = db().await;
        db.run::<_, DbError, _>(|c| {
            let saved = done(
                set_user_in(
                    c,
                    "w1",
                    1,
                    "s1",
                    0,
                    &user(-12, &[("14", Some(3)), ("013.0", None)]),
                    None,
                    10,
                )
                .unwrap(),
            );
            assert_eq!(
                (saved.kind, saved.offset, saved.decided_at),
                (MappingKind::User, Some(-12), 10)
            );
            assert_eq!(saved.evidence, USER_EVIDENCE);
            assert!(saved.version > 1);
            assert_eq!(
                saved.exceptions,
                [
                    Exception {
                        key: "n:13".into(),
                        episode: "013.0".into(),
                        target: None
                    },
                    Exception {
                        key: "n:14".into(),
                        episode: "14".into(),
                        target: Some(3)
                    }
                ]
            );
            // What is read back is what was saved.
            let read = read_in(c, "w1", 1).unwrap().remove("s1").unwrap();
            assert_eq!(read, saved);
            // A second save replaces the exceptions as a whole.
            let again = done(
                set_user_in(
                    c,
                    "w1",
                    1,
                    "s1",
                    saved.version,
                    &user(0, &[("15", Some(15))]),
                    None,
                    20,
                )
                .unwrap(),
            );
            assert_eq!(again.offset, Some(0));
            assert!(again.version > saved.version);
            assert_eq!(exceptions_of(c), [("n:15".into(), "15".into(), Some(15))]);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_save_clears_the_offset_the_app_retired_so_it_may_decide_another_after_a_revert() {
        let db = db().await;
        db.run::<_, DbError, _>(|c| {
            put(c, &zero(), 10);
            put(c, &nothing(), 20);
            assert_eq!(row(c), ("undecided".into(), None, Some(0)));
            let version = read_in(c, "w1", 1).unwrap()["s1"].version;
            let saved =
                done(set_user_in(c, "w1", 1, "s1", version, &user(3, &[]), None, 30).unwrap());
            assert_eq!(row(c), ("user".into(), Some(3), None));
            // The app does not touch it.
            assert_eq!(put(c, &plus_one(), 40), saved);
            // A revert deletes the row, and the app decides whatever its grounds say.
            assert_eq!(
                revert_in(c, "w1", 1, "s1", saved.version).unwrap(),
                Saved::Done(None)
            );
            assert!(read_in(c, "w1", 1).unwrap().is_empty());
            let m = put(c, &plus_one(), 50);
            assert_eq!((m.kind, m.offset), (MappingKind::Auto, Some(1)));
            assert_eq!(row(c), ("auto".into(), Some(1), None));
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_save_from_an_older_version_is_refused_and_changes_nothing() {
        let db = db().await;
        db.run::<_, DbError, _>(|c| {
            // The source has no row: version 0. Two screens read that.
            let first = done(set_user_in(c, "w1", 1, "s1", 0, &user(0, &[]), None, 10).unwrap());
            let late =
                set_user_in(c, "w1", 1, "s1", 0, &user(5, &[("2", None)]), None, 20).unwrap();
            assert_eq!(late, Saved::Stale(Some(first.clone())));
            assert_eq!(row(c), ("user".into(), Some(0), None));
            assert!(exceptions_of(c).is_empty());
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn the_app_changing_a_mapping_gives_it_a_new_version_and_the_same_again_does_not() {
        let db = db().await;
        db.run::<_, DbError, _>(|c| {
            let first = put(c, &zero(), 10);
            assert_eq!(put(c, &zero(), 20).version, first.version);
            let undecided = put(c, &plus_one(), 30);
            assert!(undecided.version > first.version);
            // A user's save from the version read before the app's write is refused.
            let late =
                set_user_in(c, "w1", 1, "s1", first.version, &user(0, &[]), None, 40).unwrap();
            assert_eq!(late, Saved::Stale(Some(undecided.clone())));
            // ...and from the version read after it goes through.
            assert!(matches!(
                set_user_in(c, "w1", 1, "s1", undecided.version, &user(0, &[]), None, 50).unwrap(),
                Saved::Done(Some(_))
            ));
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn the_user_saving_while_the_app_is_about_to_store_its_decision_keeps_the_users_mapping()
    {
        let db = db().await;
        db.run::<_, DbError, _>(|c| {
            put(c, &zero(), 10);
            // The app decided +1 from its grounds; before it stores that, the
            // user maps the source with an exception.
            let decided = plus_one();
            let version = read_in(c, "w1", 1).unwrap()["s1"].version;
            let saved = done(
                set_user_in(
                    c,
                    "w1",
                    1,
                    "s1",
                    version,
                    &user(-12, &[("13.5", None)]),
                    None,
                    20,
                )
                .unwrap(),
            );
            let stored = put(c, &decided, 30);
            assert_eq!(stored, saved);
            assert_eq!(row(c), ("user".into(), Some(-12), None));
            assert_eq!(exceptions_of(c), [("n:13.5".into(), "13.5".into(), None)]);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_revert_takes_the_mapping_its_exceptions_and_conflicts_and_is_refused_from_an_older_version(
    ) {
        let db = db().await;
        db.run::<_, DbError, _>(|c| {
            let saved = done(
                set_user_in(c, "w1", 1, "s1", 0, &user(0, &[("2", Some(1))]), None, 10).unwrap(),
            );
            store_conflicts(
                c,
                "w1",
                1,
                "s1",
                &[Conflict {
                    episode: "13.5".into(),
                    reason: "소수".into(),
                }],
                11,
            )
            .unwrap();
            // From a version that is not the stored one: nothing happens.
            assert_eq!(
                revert_in(c, "w1", 1, "s1", saved.version - 1).unwrap(),
                Saved::Stale(Some(saved.clone()))
            );
            assert_eq!(row(c).0, "user");
            assert_eq!(
                revert_in(c, "w1", 1, "s1", saved.version).unwrap(),
                Saved::Done(None)
            );
            assert!(read_in(c, "w1", 1).unwrap().is_empty());
            assert!(exceptions_of(c).is_empty());
            assert!(conflicts_in(c, "w1", 1, "s1").unwrap().is_empty());
            // The source is version 0 again, and the app's new row is another
            // version than the one the screen held.
            let again = put(c, &zero(), 20);
            assert_ne!(again.version, saved.version);
            assert_eq!(
                set_user_in(c, "w1", 1, "s1", saved.version, &user(0, &[]), None, 30).unwrap(),
                Saved::Stale(Some(again))
            );
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn only_the_users_mapping_is_taken_back() {
        let db = db().await;
        db.run::<_, DbError, _>(|c| {
            // No row.
            assert_eq!(
                revert_in(c, "w1", 1, "s1", 0).unwrap(),
                Saved::NotTheUsers(None)
            );
            // The app's.
            let auto = put(c, &zero(), 10);
            assert_eq!(
                revert_in(c, "w1", 1, "s1", auto.version).unwrap(),
                Saved::NotTheUsers(Some(auto))
            );
            assert_eq!(row(c).0, "auto");
            Ok(())
        })
        .await
        .unwrap();
    }
}
