use super::*;

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
    fn a_whole_episode_the_offset_cannot_be_added_to_is_unmapped() {
        let m = user_mapping(1, &[]);
        assert_eq!(
            m.season_episode("9223372036854775806"),
            Mapped::Episode(i64::MAX)
        );
        assert_eq!(m.season_episode("9223372036854775807"), Mapped::Unmapped);
        let m = user_mapping(-9999, &[]);
        assert_eq!(
            m.season_episode("9223372036854775807"),
            Mapped::Episode(i64::MAX - 9999)
        );
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

mod whole_episodes {
    use super::*;

    #[test]
    fn only_a_positive_whole_number_is_a_whole_episode() {
        for (text, n) in [("13", Some(13)), ("013", Some(13)), ("13.0", Some(13))] {
            assert_eq!(whole(text), n, "{text}");
        }
        for text in [
            "0",
            "00",
            "13.5",
            "SP",
            "",
            "-1",
            " 13",
            "99999999999999999999",
        ] {
            assert_eq!(whole(text), None, "{text:?}");
        }
        assert_eq!(whole("9223372036854775807"), Some(i64::MAX));
    }
}

mod shifting {
    use super::*;

    #[test]
    fn a_whole_episode_is_moved_by_the_offset_and_nothing_else_is() {
        assert_eq!(shifted("14", -12), Some(2));
        assert_eq!(shifted("014", 0), Some(14));
        assert_eq!(shifted("3", -12), Some(-9));
        for text in ["0", "13.5", "SP", ""] {
            assert_eq!(shifted(text, 5), None, "{text:?}");
        }
        assert_eq!(shifted("9223372036854775807", 1), None);
        assert_eq!(shifted("9223372036854775807", 0), Some(i64::MAX));
        assert_eq!(shifted("1", i64::MIN), Some(i64::MIN + 1));
    }
}

mod reconciling {
    use super::*;

    fn decided(offset: Option<i64>) -> Decided {
        Decided {
            offset,
            evidence: "근거".into(),
        }
    }

    #[test]
    fn a_decision_with_nothing_taken_back_is_taken_as_it_is() {
        let r = reconcile(None, &decided(Some(-12)));
        assert_eq!(
            (r.kind, r.offset, r.evidence.as_str(), r.retired),
            (MappingKind::Auto, Some(-12), "근거", None)
        );
        let r = reconcile(None, &decided(None));
        assert_eq!(
            (r.kind, r.offset, r.retired),
            (MappingKind::Undecided, None, None)
        );
    }

    #[test]
    fn the_offset_that_was_taken_back_is_decided_again_and_no_longer_taken_back() {
        let r = reconcile(Some(1), &decided(Some(1)));
        assert_eq!(
            (r.kind, r.offset, r.retired),
            (MappingKind::Auto, Some(1), None)
        );
    }

    #[test]
    fn another_offset_than_the_one_taken_back_is_refused_and_says_both() {
        let r = reconcile(Some(0), &decided(Some(-12)));
        assert_eq!(
            (r.kind, r.offset, r.retired),
            (MappingKind::Undecided, None, Some(0))
        );
        assert_eq!(
            r.evidence,
            "자동으로 정한 차이(0)와 다른 차이(−12)를 가리키는 회차가 생겼어요"
        );
        let r = reconcile(Some(-12), &decided(Some(1)));
        assert_eq!(
            r.evidence,
            "자동으로 정한 차이(−12)와 다른 차이(+1)를 가리키는 회차가 생겼어요"
        );
    }

    #[test]
    fn a_mapping_decided_undecided_keeps_the_offset_it_had() {
        let r = reconcile(Some(3), &decided(None));
        assert_eq!(
            (r.kind, r.offset, r.retired, r.evidence.as_str()),
            (MappingKind::Undecided, None, Some(3), "근거")
        );
    }
}
