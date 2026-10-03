use super::*;
use trss_core::{Db, DbError};

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;
/// The first episode airs here (Unix ms).
const BASE: i64 = 1_790_000_000_000;

/// A season of `episodes` episodes airing weekly from [`BASE`].
fn weekly(episodes: u32) -> BTreeMap<u32, i64> {
    (1..=episodes)
        .map(|k| (k, BASE + i64::from(k - 1) * 7 * DAY))
        .collect()
}

fn air(k: u32) -> i64 {
    BASE + i64::from(k - 1) * 7 * DAY
}

fn at(episode: u32, at: i64) -> Posted {
    Posted {
        episode,
        at: Some(at),
    }
}

fn season(number: u32, schedule: &BTreeMap<u32, i64>, previous: Option<u32>) -> Season<'_> {
    Season {
        number,
        schedule,
        count: Some(12),
        previous,
    }
}

fn auto(decided: &Decided, offset: i64) -> &str {
    assert_eq!(decided.offset, Some(offset), "{}", decided.evidence);
    &decided.evidence
}

fn undecided<'a>(decided: &'a Decided, why: &str) -> &'a str {
    assert_eq!(decided.offset, None, "{}", decided.evidence);
    assert!(decided.evidence.contains(why), "{}", decided.evidence);
    &decided.evidence
}

#[test]
fn only_a_decided_mapping_has_an_offset_whatever_the_stored_number_is() {
    let mapping = |kind, offset| Mapping {
        kind,
        offset,
        evidence: String::new(),
        decided_at: 0,
        version: 1,
        exceptions: Vec::new(),
    };
    assert_eq!(
        mapping(MappingKind::Auto, Some(-12)).decided_offset(),
        Some(-12)
    );
    assert_eq!(
        mapping(MappingKind::User, Some(0)).decided_offset(),
        Some(0)
    );
    assert_eq!(
        mapping(MappingKind::Undecided, Some(3)).decided_offset(),
        None
    );
    assert_eq!(mapping(MappingKind::Undecided, None).decided_offset(), None);
    assert_eq!(mapping(MappingKind::Auto, None).decided_offset(), None);
}

#[test]
fn two_episodes_posted_after_they_aired_map_as_they_are_and_both_are_the_evidence() {
    let schedule = weekly(12);
    // 1 an hour and a half after it aired, 2 two days after.
    let posted = [at(1, air(1) + 90 * 60_000), at(2, air(2) + 2 * DAY)];
    let d = decide(&posted, &season(1, &schedule, Some(0)));
    assert_eq!(auto(&d, 0), "1화·2화가 방영 뒤에 올라왔어요");
}

#[test]
fn many_episodes_are_written_as_a_range() {
    let schedule = weekly(12);
    let posted: Vec<Posted> = (1..=8).map(|n| at(n, air(n) + HOUR)).collect();
    let d = decide(&posted, &season(1, &schedule, Some(0)));
    assert_eq!(auto(&d, 0), "1–8화가 방영 뒤에 올라왔어요");
}

#[test]
fn a_cumulative_number_posted_after_the_seasons_first_episode_is_the_previous_seasons_sum_back() {
    // Season 2 of 12 after a season of 12: the creator numbers on, so `13`
    // is posted after episode 1 aired.
    let schedule = weekly(12);
    let d = decide(
        &[at(13, air(1) + 2 * HOUR)],
        &season(2, &schedule, Some(12)),
    );
    assert_eq!(
        auto(&d, -12),
        "13화가 1화 방영 뒤에 올라왔고 앞 시즌 회차 수(12)만큼 이어 셌어요"
    );
    // The same season, another creator who counts from 1 in it.
    let d = decide(&[at(1, air(1) + 2 * HOUR)], &season(2, &schedule, Some(12)));
    assert_eq!(
        auto(&d, 0),
        "1화가 방영 뒤에 올라왔고 시즌 회차 수(12) 안이에요"
    );
}

#[test]
fn a_single_episode_that_does_not_fit_the_structure_is_undecided() {
    let schedule = weekly(12);
    // 1 posted after episode 2 aired: +1, which no structure explains.
    let d = decide(&[at(1, air(2) + HOUR)], &season(1, &schedule, Some(0)));
    undecided(&d, "1화 하나만 차이(+1)를 가리키고 시즌 구조와 맞지 않아요");
    // An offset that is the previous seasons' sum needs a previous season, and
    // a sum that is not known explains nothing.
    let d = decide(&[at(13, air(1) + HOUR)], &season(2, &schedule, None));
    undecided(&d, "13화 하나만 차이(−12)");
    let d = decide(&[at(13, air(1) + HOUR)], &season(1, &schedule, Some(0)));
    undecided(&d, "13화 하나만 차이(−12)");
    // The right offset for the wrong sum.
    let d = decide(&[at(13, air(1) + HOUR)], &season(2, &schedule, Some(24)));
    undecided(&d, "13화 하나만 차이(−12)");
    // Offset 0, but past the season's episodes.
    let d = decide(&[at(13, air(12) + HOUR)], &season(1, &schedule, Some(0)));
    undecided(&d, "차이(−1)");
    let d = decide(&[at(12, air(12) + HOUR)], &season(1, &schedule, Some(0)));
    auto(&d, 0);
}

#[test]
fn the_count_is_the_highest_scheduled_episode_when_anilist_has_none() {
    let schedule = weekly(12);
    let unknown = Season {
        count: None,
        ..season(1, &schedule, Some(0))
    };
    assert_eq!(unknown.total(), Some(12));
    let d = decide(&[at(1, air(1) + HOUR)], &unknown);
    assert_eq!(
        auto(&d, 0),
        "1화가 방영 뒤에 올라왔고 시즌 회차 수(12) 안이에요"
    );
    // With neither, the single episode has no bound to be within.
    let empty = BTreeMap::new();
    let none = Season {
        count: None,
        ..season(1, &empty, Some(0))
    };
    assert_eq!(none.total(), None);
}

#[test]
fn an_all_at_once_release_and_a_missing_schedule_leave_the_mapping_undecided() {
    // An ONA: every episode at one time, so no time says which episode.
    let once: BTreeMap<u32, i64> = (1..=6).map(|k| (k, BASE)).collect();
    let d = decide(
        &[at(1, BASE + HOUR), at(2, BASE + 2 * HOUR)],
        &season(1, &once, Some(0)),
    );
    undecided(&d, "방영 시각으로 가리키는 회차가 아직 없어요");
    // A film or an entry with no schedule.
    let none = BTreeMap::new();
    let d = decide(&[at(1, BASE + HOUR)], &season(1, &none, Some(0)));
    undecided(&d, "방영 일정이 없어요");
    // Specials.
    let schedule = weekly(12);
    let d = decide(&[at(1, air(1) + HOUR)], &season(0, &schedule, Some(0)));
    undecided(&d, "시즌 0");
    // No whole episode seen.
    let d = decide(&[], &season(1, &schedule, Some(0)));
    undecided(&d, "숫자 회차를 아직 보지 못했어요");
}

#[test]
fn a_post_belongs_to_the_episode_whose_window_it_is_in() {
    let schedule = weekly(12);
    let twelve = season(1, &schedule, Some(0));
    let k = |t: i64| aired(&twelve, t);
    assert_eq!(k(air(1) - 1), None, "before the first airing");
    assert_eq!(k(air(1)), Some(1), "at an airing it is that episode's");
    assert_eq!(k(air(2) - 1), Some(1));
    assert_eq!(k(air(2)), Some(2), "exactly at the next airing is the next");
    // The last episode's window is 7 days.
    assert_eq!(k(air(12) + 7 * DAY - 1), Some(12));
    assert_eq!(k(air(12) + 7 * DAY), None);
    // A scheduled next episode ends the window early, whatever its gap.
    let gap: BTreeMap<u32, i64> = [(1, BASE), (2, BASE + 30 * DAY)].into();
    let two = Season {
        count: Some(2),
        ..season(1, &gap, Some(0))
    };
    assert_eq!(aired(&two, BASE + 29 * DAY), Some(1));
    assert_eq!(aired(&two, BASE + 30 * DAY), Some(2));
    // An episode the schedule skips leaves the end of the one before unknown.
    let skipped: BTreeMap<u32, i64> = [(1, BASE), (3, BASE + 14 * DAY)].into();
    let three = Season {
        count: Some(3),
        ..season(1, &skipped, Some(0))
    };
    assert_eq!(aired(&three, BASE + 6 * DAY), None);
    assert_eq!(aired(&three, BASE + 14 * DAY), Some(3));
    // So does a schedule cut short of the count: the 7 days are the last
    // episode's only.
    let cut = weekly(25);
    let fifty = Season {
        count: Some(50),
        ..season(1, &cut, Some(0))
    };
    assert_eq!(aired(&fifty, air(25) + DAY), None);
    assert_eq!(aired(&fifty, air(24) + DAY), Some(24));
}

#[test]
fn a_time_that_did_not_read_is_no_evidence() {
    let schedule = weekly(12);
    let posted = [
        Posted {
            episode: 1,
            at: None,
        },
        Posted {
            episode: 2,
            at: None,
        },
    ];
    let d = decide(&posted, &season(1, &schedule, Some(0)));
    undecided(&d, "방영 시각으로 가리키는 회차가 아직 없어요");
    // One that did read is the only evidence.
    let posted = [
        Posted {
            episode: 1,
            at: None,
        },
        at(2, air(2) + HOUR),
    ];
    let d = decide(&posted, &season(1, &schedule, Some(0)));
    assert_eq!(
        auto(&d, 0),
        "2화가 방영 뒤에 올라왔고 시즌 회차 수(12) 안이에요"
    );
}

#[test]
fn episodes_that_point_at_different_offsets_are_undecided() {
    let schedule = weekly(12);
    // Two singles that disagree.
    let d = decide(
        &[at(1, air(1) + HOUR), at(2, air(3) + HOUR)],
        &season(1, &schedule, Some(0)),
    );
    undecided(&d, "회차마다 가리키는 차이가 달라요(0, +1)");
    // Two offsets that two episodes each point at.
    let posted = [
        at(1, air(1) + HOUR),
        at(2, air(2) + HOUR),
        at(3, air(4) + HOUR),
        at(4, air(5) + HOUR),
    ];
    let d = decide(&posted, &season(1, &schedule, Some(0)));
    undecided(&d, "회차마다 가리키는 차이가 달라요(0, +1)");
}

#[test]
fn an_offset_two_episodes_agree_on_wins_over_a_stray_one() {
    let schedule = weekly(12);
    let posted = [
        at(1, air(1) + HOUR),
        at(2, air(2) + HOUR),
        at(3, air(5) + HOUR),
    ];
    let d = decide(&posted, &season(1, &schedule, Some(0)));
    assert_eq!(auto(&d, 0), "1화·2화가 방영 뒤에 올라왔어요");
    // Two episodes posted late agree on their own offset all the same.
    let posted = [at(3, air(4) + HOUR), at(4, air(5) + 2 * HOUR)];
    let d = decide(&posted, &season(1, &schedule, Some(0)));
    assert_eq!(auto(&d, 1), "3화·4화가 방영 뒤에 올라왔어요");
}

#[test]
fn a_later_part_continues_after_the_earlier_parts_episodes_in_the_schedule() {
    // Part 2 of 12 episodes after a part of 12, as the season's schedule has it.
    let schedule: BTreeMap<u32, i64> = (13..=24).map(|k| (k, air(k - 12) + 100 * DAY)).collect();
    let season = Season {
        count: Some(24),
        ..season(1, &schedule, Some(0))
    };
    let d = decide(
        &[
            at(13, air(1) + 100 * DAY + HOUR),
            at(14, air(2) + 100 * DAY + HOUR),
        ],
        &season,
    );
    assert_eq!(auto(&d, 0), "13화·14화가 방영 뒤에 올라왔어요");
}

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
            let saved = done(set_user_in(c, "w1", 1, "s1", version, &user(3, &[]), 30).unwrap());
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
            let first = done(set_user_in(c, "w1", 1, "s1", 0, &user(0, &[]), 10).unwrap());
            let late = set_user_in(c, "w1", 1, "s1", 0, &user(5, &[("2", None)]), 20).unwrap();
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
            let late = set_user_in(c, "w1", 1, "s1", first.version, &user(0, &[]), 40).unwrap();
            assert_eq!(late, Saved::Stale(Some(undecided.clone())));
            // ...and from the version read after it goes through.
            assert!(matches!(
                set_user_in(c, "w1", 1, "s1", undecided.version, &user(0, &[]), 50).unwrap(),
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
                set_user_in(c, "w1", 1, "s1", version, &user(-12, &[("13.5", None)]), 20).unwrap(),
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
            let saved =
                done(set_user_in(c, "w1", 1, "s1", 0, &user(0, &[("2", Some(1))]), 10).unwrap());
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
                set_user_in(c, "w1", 1, "s1", saved.version, &user(0, &[]), 30).unwrap(),
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

mod mapped {
    use super::*;

    fn user_mapping(offset: i64, exceptions: &[(&str, Option<i64>)]) -> Mapping {
        let user = UserMapping::new(
            offset,
            exceptions
                .iter()
                .map(|(episode, target)| ((*episode).to_owned(), *target))
                .collect(),
        )
        .unwrap();
        Mapping {
            kind: MappingKind::User,
            offset: Some(user.offset),
            evidence: USER_EVIDENCE.into(),
            decided_at: 1,
            version: 5,
            exceptions: user.exceptions,
        }
    }

    #[test]
    fn an_exception_is_applied_before_the_offset_and_the_others_follow_the_offset() {
        let m = user_mapping(0, &[("13.5", None), ("14", Some(3)), ("SP", Some(20))]);
        assert_eq!(m.season_episode("13.5"), Mapped::NotReceived);
        assert_eq!(m.season_episode("13.50"), Mapped::NotReceived);
        // The key is the number: the exception for `14` covers `014` and `14.0`.
        for text in ["14", "014", "14.0"] {
            assert_eq!(m.season_episode(text), Mapped::Episode(3), "{text}");
        }
        assert_eq!(m.season_episode("SP"), Mapped::Episode(20));
        // Every other whole episode follows the default; text with no exception has no place.
        assert_eq!(m.season_episode("13"), Mapped::Episode(13));
        assert_eq!(m.season_episode("1"), Mapped::Episode(1));
        assert_eq!(m.season_episode("0"), Mapped::Unmapped);
        assert_eq!(m.season_episode("OVA"), Mapped::Unmapped);
        assert_eq!(m.season_episode("12.5"), Mapped::Unmapped);
    }

    #[test]
    fn an_exception_of_a_number_the_offset_would_put_outside_still_stands() {
        let m = user_mapping(-12, &[("1", Some(1)), ("13", None)]);
        assert_eq!(m.season_episode("1"), Mapped::Episode(1));
        assert_eq!(m.season_episode("13"), Mapped::NotReceived);
        assert_eq!(m.season_episode("14"), Mapped::Episode(2));
        // Without it `2` would map below 1.
        assert_eq!(m.season_episode("2"), Mapped::Episode(-10));
    }

    #[test]
    fn an_undecided_mapping_places_nothing() {
        let m = Mapping {
            kind: MappingKind::Undecided,
            offset: None,
            evidence: "근거 없음".into(),
            decided_at: 1,
            version: 1,
            exceptions: Vec::new(),
        };
        assert_eq!(m.season_episode("1"), Mapped::Unmapped);
    }

    #[test]
    fn an_episode_an_exception_covers_is_no_conflict_but_the_others_still_are() {
        let schedule = weekly(12);
        let season = season(1, &schedule, Some(0));
        let m = user_mapping(0, &[("13.5", None), ("13", Some(12)), ("SP", Some(3))]);
        let texts: Vec<String> = ["12", "13", "13.5", "14", "14.5", "SP", "OVA"]
            .iter()
            .map(|t| (*t).to_owned())
            .collect();
        let found = conflicts(&m, &texts, &[], &season);
        let names: Vec<&str> = found.iter().map(|c| c.episode.as_str()).collect();
        // `13` is past the 12 by the offset but the exception puts it at 12.
        assert_eq!(names, ["14", "14.5", "OVA"]);
    }

    #[test]
    fn two_exceptions_of_one_number_are_refused_whatever_they_say() {
        for pair in [
            [("013", Some(1)), ("13", Some(2))],
            [("13", None), ("13.0", Some(2))],
            [("13.50", None), ("13.5", None)],
            [("13", Some(1)), ("13", Some(1))],
        ] {
            let err =
                UserMapping::new(0, pair.iter().map(|(e, t)| ((*e).to_owned(), *t)).collect())
                    .unwrap_err();
            assert!(err.to_string().contains("같은 회차"), "{pair:?}: {err}");
        }
        // Different numbers and different texts are not repeats.
        assert!(UserMapping::new(
            0,
            vec![
                ("13".into(), None),
                ("13.5".into(), None),
                ("SP".into(), None),
                ("SP2".into(), Some(2))
            ]
        )
        .is_ok());
    }

    #[test]
    fn a_mapping_the_user_sends_must_be_one_that_can_be_stored() {
        let refused = |offset: i64, exceptions: Vec<(&str, Option<i64>)>| {
            UserMapping::new(
                offset,
                exceptions
                    .into_iter()
                    .map(|(e, t)| (e.to_owned(), t))
                    .collect(),
            )
            .is_err()
        };
        assert!(refused(10_000, vec![]));
        assert!(refused(-10_000, vec![]));
        assert!(!refused(9_999, vec![]));
        assert!(refused(0, vec![("", None)]));
        assert!(refused(0, vec![("  ", Some(1))]));
        assert!(refused(0, vec![("13", Some(0))]));
        assert!(refused(0, vec![("13", Some(-1))]));
        assert!(refused(0, vec![("13", Some(10_000))]));
        assert!(refused(0, vec![(&"1".repeat(33), None)]));
        let many: Vec<(String, Option<i64>)> = (1..=MAX_EXCEPTIONS as i64 + 1)
            .map(|n| (n.to_string(), None))
            .collect();
        assert!(UserMapping::new(0, many).is_err());
    }
}

mod conflicting {
    use super::*;

    fn mapping(kind: MappingKind, offset: Option<i64>) -> Mapping {
        Mapping {
            kind,
            offset,
            evidence: "근거".into(),
            decided_at: 1,
            version: 1,
            exceptions: Vec::new(),
        }
    }

    fn texts(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|t| (*t).to_owned()).collect()
    }

    fn episodes(found: &[Conflict]) -> Vec<&str> {
        found.iter().map(|c| c.episode.as_str()).collect()
    }

    #[test]
    fn a_decimal_or_text_episode_conflicts_and_zero_never_does() {
        let schedule = weekly(12);
        let season = season(1, &schedule, Some(0));
        for kind in [MappingKind::Auto, MappingKind::User] {
            let found = conflicts(
                &mapping(kind, Some(0)),
                &texts(&["0", "00", "1", "13.5", "SP", "0.5", "2"]),
                &[],
                &season,
            );
            assert_eq!(episodes(&found), ["13.5", "SP", "0.5"], "{kind:?}");
            assert!(found[0].reason.contains("소수 회차(13.5)"));
            assert!(found[1].reason.contains("숫자가 아닌 회차(SP)"));
        }
    }

    #[test]
    fn a_whole_episode_outside_the_season_once_mapped_conflicts() {
        let schedule = weekly(12);
        let season = season(1, &schedule, Some(0));
        // Offset 0: 13 is past the 12.
        let found = conflicts(
            &mapping(MappingKind::Auto, Some(0)),
            &texts(&["12", "13"]),
            &[],
            &season,
        );
        assert_eq!(episodes(&found), ["13"]);
        assert!(found[0].reason.contains("13화"), "{}", found[0].reason);
        // Offset −12: 1 maps below 1, 13 and 24 inside.
        let found = conflicts(
            &mapping(MappingKind::User, Some(-12)),
            &texts(&["1", "13", "24", "25"]),
            &[],
            &season,
        );
        assert_eq!(episodes(&found), ["1", "25"]);
        // An unknown count has no end to be past, but below 1 is outside all the same.
        let unknown = Season {
            count: None,
            schedule: &BTreeMap::new(),
            ..season
        };
        let found = conflicts(
            &mapping(MappingKind::User, Some(-12)),
            &texts(&["1", "13", "99"]),
            &[],
            &unknown,
        );
        assert_eq!(episodes(&found), ["1"]);
    }

    #[test]
    fn an_auto_mappings_episode_that_points_at_another_offset_conflicts_but_a_users_does_not() {
        let schedule = weekly(12);
        let season = season(1, &schedule, Some(0));
        // 1 and 2 agree on 0; 3 is posted after episode 5 aired.
        let posted = [
            at(1, air(1) + HOUR),
            at(2, air(2) + HOUR),
            at(3, air(5) + HOUR),
        ];
        let names = texts(&["1", "2", "3"]);
        let found = conflicts(
            &mapping(MappingKind::Auto, Some(0)),
            &names,
            &posted,
            &season,
        );
        assert_eq!(episodes(&found), ["3"]);
        assert!(found[0].reason.contains("차이(+2)"), "{}", found[0].reason);
        let found = conflicts(
            &mapping(MappingKind::User, Some(0)),
            &names,
            &posted,
            &season,
        );
        assert!(found.is_empty());
    }

    #[test]
    fn an_undecided_mapping_has_no_conflicts() {
        let schedule = weekly(12);
        let found = conflicts(
            &mapping(MappingKind::Undecided, None),
            &texts(&["13.5", "99"]),
            &[],
            &season(1, &schedule, Some(0)),
        );
        assert!(found.is_empty());
    }
}
