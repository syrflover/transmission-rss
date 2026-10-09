//! The sentences as the screens show them: the web tests and the jobs tests
//! read most of them literally.

use super::*;
use crate::mapping::{Exception, MappingKind};

fn mapping(kind: MappingKind, offset: Option<i64>, exceptions: &[(&str, Option<u32>)]) -> Mapping {
    Mapping {
        kind,
        offset,
        evidence: "근거".into(),
        decided_at: 1,
        version: 1,
        exceptions: exceptions
            .iter()
            .map(|(episode, target)| Exception {
                key: trss_core::episode::stored_key(episode),
                episode: (*episode).to_owned(),
                target: *target,
            })
            .collect(),
    }
}

#[test]
fn the_season_range_is_one_to_the_count_or_one_onwards() {
    assert_eq!(season_range(Some(12)), "1–12화");
    assert_eq!(season_range(Some(1)), "1–1화");
    assert_eq!(season_range(None), "1화부터");
}

#[test]
fn an_episode_is_in_the_season_from_one_to_its_count() {
    for (episode, total, in_it) in [
        (0, Some(12), false),
        (-3, None, false),
        (1, Some(12), true),
        (12, Some(12), true),
        (13, Some(12), false),
        (13, None, true),
        (i64::MAX, None, true),
    ] {
        assert_eq!(in_season(episode, total), in_it, "{episode} of {total:?}");
    }
}

#[test]
fn a_mapping_puts_an_episode_on_a_season_episode_even_outside_the_season() {
    let m = mapping(MappingKind::User, Some(-12), &[("13.5", Some(2))]);
    for wording in [Wording::Candidate, Wording::File, Wording::Stored] {
        assert_eq!(place(&m, "14", wording), Ok(2));
        assert_eq!(place(&m, "3", wording), Ok(-9));
        assert_eq!(place(&m, "13.5", wording), Ok(2));
    }
}

#[test]
fn an_episode_the_mapping_does_not_receive_is_said_so_in_each_wording() {
    let m = mapping(MappingKind::User, Some(0), &[("5", None)]);
    assert_eq!(
        place(&m, "5", Wording::Candidate),
        Err("회차 대응이 후보의 5화를 받지 않는 회차로 정해 두었어요".to_owned())
    );
    assert_eq!(
        place(&m, "5", Wording::File),
        Err("회차 대응이 5화를 받지 않는 회차로 정해 두었어요".to_owned())
    );
    assert_eq!(
        place(&m, "5", Wording::Stored),
        Err("회차 대응이 5화를 받지 않는 회차로 정했어요".to_owned())
    );
}

#[test]
fn an_episode_the_decided_mapping_cannot_move_is_said_so_in_each_wording() {
    let m = mapping(MappingKind::Auto, Some(0), &[]);
    assert_eq!(
        place(&m, "5.5", Wording::Candidate),
        Err("후보의 회차 5.5화는 회차 대응으로 옮길 수 없는 회차예요".to_owned())
    );
    for wording in [Wording::File, Wording::Stored] {
        assert_eq!(
            place(&m, "5.5", wording),
            Err("5.5화는 회차 대응으로 옮길 수 없는 회차예요".to_owned())
        );
    }
    // Text that is no number is written as it is.
    assert_eq!(
        place(&m, "SP", Wording::File),
        Err("SP는 회차 대응으로 옮길 수 없는 회차예요".to_owned())
    );
}

#[test]
fn an_undecided_mapping_places_nothing_and_says_so_in_every_wording() {
    let m = mapping(MappingKind::Undecided, None, &[]);
    for wording in [Wording::Candidate, Wording::File, Wording::Stored] {
        assert_eq!(
            place(&m, "5", wording),
            Err("이 제작자의 회차 대응이 아직 미정이에요".to_owned())
        );
    }
    assert_eq!(UNDECIDED, "이 제작자의 회차 대응이 아직 미정이에요");
}

#[test]
fn an_episode_that_is_no_whole_number_is_said_so_in_each_wording() {
    assert_eq!(
        not_whole("5.5", Wording::Candidate),
        "후보의 회차 5.5화는 정수 회차가 아니라 시즌의 회차로 옮기지 못했어요"
    );
    for wording in [Wording::File, Wording::Stored] {
        assert_eq!(
            not_whole("5.5", wording),
            "5.5화는 정수 회차가 아니라 시즌의 회차로 정하지 못했어요"
        );
    }
    assert_eq!(
        not_whole("SP", Wording::File),
        "SP는 정수 회차가 아니라 시즌의 회차로 정하지 못했어요"
    );
}

#[test]
fn an_episode_outside_the_season_is_said_so_in_each_wording() {
    assert_eq!(
        outside_season("14", 14, Some(12), Wording::Candidate),
        "후보의 14화를 옮긴 시즌 14화가 시즌의 1–12화 밖이에요"
    );
    assert_eq!(
        outside_season("14", 2, None, Wording::Candidate),
        "후보의 14화를 옮긴 시즌 2화가 시즌의 1화부터 밖이에요"
    );
    assert_eq!(
        outside_season("13", 1, Some(12), Wording::File),
        "13화가 시즌의 1–12화 밖이에요"
    );
    assert_eq!(
        outside_season("3", -9, Some(12), Wording::Stored),
        "3화를 옮긴 시즌 -9화가 시즌의 1–12화 밖이에요"
    );
}

#[test]
fn the_other_sentences_about_the_season_read_as_they_are_shown() {
    assert_eq!(
        another_seasons_file("13", Some(12)),
        "13화가 시즌의 1–12화 밖이라 다른 시즌의 파일로 보여요"
    );
    assert_eq!(
        another_seasons_file("013", None),
        "013화가 시즌의 1화부터 밖이라 다른 시즌의 파일로 보여요"
    );
    assert_eq!(
        numbering_unclear("13"),
        "파일 이름의 13화가 Anissia의 회차인지 시즌의 회차인지 정하지 못했어요"
    );
    assert_eq!(
        placed_outside_season(4, Some(3)),
        "4화는 이 시즌의 1–3화 밖이에요."
    );
    assert_eq!(placed_outside_season(0, None), "0화는 회차가 될 수 없어요.");
}
