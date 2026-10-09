//! What the dialog shows for the offset and exceptions typed in it. The cases
//! the screen once checked itself (`mapping.test.ts`) are here with the same
//! inputs and expected results, then the cases of the whole dialog.

use super::*;

const TWELVE_THEN: [&str; 4] = ["13", "14", "15", "24"];

fn texts(episodes: &[&str]) -> Vec<String> {
    episodes.iter().map(|e| (*e).to_owned()).collect()
}

fn exception(episode: &str, target: Option<u32>) -> Exception {
    Exception {
        key: stored_key(episode),
        episode: episode.to_owned(),
        target,
    }
}

fn row(episode: &str, target: &str, skip: bool) -> Row {
    Row {
        episode: episode.to_owned(),
        target: target.to_owned(),
        skip,
    }
}

/// `(episode, where it goes)` of each move.
fn goes(moves: &[Move]) -> Vec<(String, String)> {
    moves.iter().map(|m| (m.episode.clone(), m.to())).collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

fn user(offset: Option<i64>, exceptions: Vec<Exception>) -> Mapping {
    Mapping {
        kind: MappingKind::User,
        offset,
        evidence: "사용자가 정했어요".to_owned(),
        decided_at: 0,
        version: 1,
        exceptions,
    }
}

fn user_line(
    offset: i64,
    exceptions: Vec<Exception>,
    episodes: &[&str],
    total: Option<u32>,
) -> String {
    user(Some(offset), exceptions).line(&texts(episodes), total)
}

#[test]
fn continuing_on_from_a_12_episode_earlier_season_is_an_offset_of_minus_12() {
    assert_eq!(offset_of(Choice::Continue, "", Some(12)), Some(-12));
    assert_eq!(choice_of(Some(-12), Some(12)), Some(Choice::Continue));
    assert_eq!(choice_of(Some(0), Some(12)), Some(Choice::Same));
    assert_eq!(choice_of(Some(-5), Some(12)), Some(Choice::Custom));
    assert_eq!(choice_of(None, Some(12)), None);
}

#[test]
fn continuing_on_is_not_offered_when_the_earlier_seasons_episode_count_is_unknown_and_says_why() {
    assert_eq!(offset_of(Choice::Continue, "", None), None);
    assert!(continue_reason(None).unwrap().contains("알 수 없어서"));
    assert!(continue_reason(Some(0))
        .unwrap()
        .contains("앞 시즌이 없어서"));
    assert_eq!(continue_reason(Some(12)), None);
    // An offset that happens to be the earlier seasons' sum is only a custom one without them.
    assert_eq!(choice_of(Some(-12), None), Some(Choice::Custom));
    assert_eq!(choice_of(Some(-12), Some(0)), Some(Choice::Custom));
}

#[test]
fn a_custom_offset_is_any_integer_within_the_range_and_nothing_else() {
    assert_eq!(offset_of(Choice::Custom, "-12", None), Some(-12));
    assert_eq!(offset_of(Choice::Custom, " 3 ", None), Some(3));
    assert_eq!(offset_of(Choice::Custom, "1.5", None), None);
    assert_eq!(offset_of(Choice::Custom, "", None), None);
    assert_eq!(offset_of(Choice::Custom, "10000", None), None);
    // The edges, a sign, and what is no integer of up to five digits.
    assert_eq!(offset_of(Choice::Custom, "9999", None), Some(9999));
    assert_eq!(offset_of(Choice::Custom, "-9999", None), Some(-9999));
    assert_eq!(offset_of(Choice::Custom, "+5", None), Some(5));
    assert_eq!(offset_of(Choice::Custom, "-0", None), Some(0));
    assert_eq!(offset_of(Choice::Custom, "123456", None), None);
    assert_eq!(offset_of(Choice::Custom, "1 2", None), None);
    assert_eq!(offset_of(Choice::Custom, "+", None), None);
    assert_eq!(offset_of(Choice::Custom, "１２", None), None);
}

#[test]
fn the_preview_shows_where_the_current_episodes_go_under_the_choice() {
    let moves = moves(&texts(&TWELVE_THEN), Some(-12), &[], Some(12));
    assert_eq!(
        goes(&moves),
        pairs(&[("13", "1화"), ("14", "2화"), ("15", "3화"), ("24", "12화")])
    );
    assert_eq!(
        preview_text(&moves, 4),
        "13화 → 1화 · 14화 → 2화 · 15화 → 3화 · 24화 → 12화"
    );
    assert_eq!(
        preview_text(&moves, 3),
        "13화 → 1화 · 14화 → 2화 … 24화 → 12화"
    );
}

#[test]
fn the_same_number_puts_an_episode_past_the_seasons_count_outside_the_season() {
    let moves = moves(&texts(&["12", "13"]), Some(0), &[], Some(12));
    assert_eq!(goes(&moves), pairs(&[("12", "12화"), ("13", "시즌 밖")]));
}

#[test]
fn a_decimal_episode_set_to_not_received_is_left_and_the_others_follow_the_default() {
    let moves = moves(
        &texts(&["12", "13", "13.5"]),
        Some(0),
        &[exception("13.5", None)],
        None,
    );
    assert_eq!(
        goes(&moves),
        pairs(&[("12", "12화"), ("13", "13화"), ("13.5", "받지 않음")])
    );
}

#[test]
fn an_exception_is_applied_before_the_offset() {
    let moves = moves(
        &texts(&["13", "14"]),
        Some(-12),
        &[exception("014", Some(5))],
        Some(12),
    );
    assert_eq!(
        moves.iter().map(Move::to).collect::<Vec<_>>(),
        ["1화", "5화"]
    );
}

#[test]
fn the_episodes_that_fit_nowhere_are_offered_as_exceptions_to_write() {
    assert_eq!(
        misfits(
            &texts(&["12", "13.5", "SP", "24"]),
            Some(-12),
            &[],
            Some(12)
        ),
        ["12", "13.5", "SP"]
    );
}

#[test]
fn an_episode_an_exception_covers_is_not_offered_again() {
    assert_eq!(
        misfits(
            &texts(&["13", "13.5", "SP"]),
            Some(-12),
            &[exception("13.5", None)],
            Some(12)
        ),
        ["SP"]
    );
}

#[test]
fn two_exceptions_of_one_number_are_refused_whatever_they_say_with_a_sentence() {
    let rows = [row("013", "", true), row("13", "2", false)];
    let message = exceptions_of(&rows).unwrap_err();
    assert!(message.contains("예외가 둘이에요"), "{message}");
    assert_eq!(
        message,
        "회차 ‘13’의 예외가 둘이에요. 같은 회차에는 예외를 하나만 둘 수 있어요."
    );
}

#[test]
fn an_exception_row_needs_a_season_episode_or_the_not_received_choice_and_an_empty_row_is_left_out()
{
    assert!(exceptions_of(&[row("13.5", "", false)]).is_err());
    assert!(exceptions_of(&[row("13.5", "0", false)]).is_err());
    assert_eq!(
        exceptions_of(&[row("", "2", false)]).unwrap_err(),
        "예외의 회차를 써 주세요."
    );
    assert_eq!(
        exceptions_of(&[row("13.5", "", false)]).unwrap_err(),
        "회차 ‘13.5’가 시즌의 몇 화인지 1 이상의 숫자로 써 주세요. 받지 않으려면 ‘받지 않음’을 골라 주세요."
    );
    let ok = exceptions_of(&[
        row("", "", false),
        row("13.5", "", true),
        row("14", "3", false),
    ])
    .unwrap();
    assert_eq!(ok, [exception("13.5", None), exception("14", Some(3))]);
}

#[test]
fn the_group_line_after_continuing_on_from_a_12_episode_season_is_a_user_line_with_one_example() {
    assert_eq!(
        user_line(-12, vec![], &TWELVE_THEN, None),
        "직접 정함 · 13화 → 1화"
    );
}

#[test]
fn the_group_line_says_the_same_number_for_an_offset_of_zero_and_gives_the_exceptions_compactly() {
    assert_eq!(
        user_line(0, vec![], &TWELVE_THEN, None),
        "직접 정함 · 같은 번호"
    );
    assert_eq!(
        user_line(0, vec![exception("13.5", None)], &TWELVE_THEN, None),
        "직접 정함 · 같은 번호 · 예외 13.5 받지 않음"
    );
    assert_eq!(
        user_line(
            0,
            vec![
                exception("13.5", None),
                exception("14", Some(3)),
                exception("SP", None),
                exception("OVA", None),
            ],
            &TWELVE_THEN,
            None
        ),
        "직접 정함 · 같은 번호 · 예외 13.5 받지 않음, 14 → 3화 외 2개"
    );
}

#[test]
fn the_group_line_of_an_app_decided_mapping_keeps_its_grounds_and_a_users_gives_the_exceptions() {
    let mapping = |kind, offset| Mapping {
        kind,
        offset,
        evidence: "앞 시즌 12화".to_owned(),
        decided_at: 0,
        version: 1,
        exceptions: Vec::new(),
    };
    let episodes = texts(&TWELVE_THEN);
    assert_eq!(
        mapping(MappingKind::Auto, Some(-12)).line(&episodes, None),
        "자동 · 앞 시즌 12화"
    );
    assert_eq!(
        mapping(MappingKind::Undecided, None).line(&episodes, None),
        "회차 대응 미정 · 앞 시즌 12화"
    );
    assert_eq!(
        mapping(MappingKind::User, Some(-12)).line(&episodes, None),
        "직접 정함 · 13화 → 1화"
    );
}

#[test]
fn an_exceptions_target_past_the_seasons_count_is_shown_as_received_with_the_count_named() {
    let exceptions = [exception("13.5", Some(32))];
    let moves = moves(&texts(&["12", "13.5"]), Some(0), &exceptions, Some(12));
    assert_eq!(
        moves
            .iter()
            .map(|m| (m.episode.as_str(), m.to(), m.lands()))
            .collect::<Vec<_>>(),
        [
            ("12", "12화".to_owned(), true),
            ("13.5", "32화 (시즌 회차 수 밖)".to_owned(), false)
        ]
    );
    // What an exception covers is not offered as a misfit again.
    assert_eq!(
        misfits(&texts(&["12", "13.5"]), Some(0), &exceptions, Some(12)),
        Vec::<String>::new()
    );
}

#[test]
fn the_group_line_names_the_first_episode_that_lands_inside_the_season_and_no_exception_covers_by_its_number(
) {
    // 012 lands outside (0), 013 is covered by an exception, 014 is the first that lands.
    assert_eq!(
        user_line(
            -12,
            vec![exception("13", Some(5))],
            &["012", "013", "014"],
            Some(12)
        ),
        "직접 정함 · 14화 → 2화 · 예외 13 → 5화"
    );
    // The episode text is written by its number, not as posted.
    assert_eq!(
        user_line(-12, vec![], &["013", "014"], Some(12)),
        "직접 정함 · 13화 → 1화"
    );
    // Past the count it does not land: the next one does.
    assert_eq!(
        user_line(-12, vec![], &["30", "24"], Some(12)),
        "직접 정함 · 24화 → 12화"
    );
}

#[test]
fn two_episodes_landing_on_one_season_episode_are_warned_about_not_refused() {
    let exceptions = [exception("13.5", Some(13))];
    assert_eq!(
        collisions(&moves(
            &texts(&["13", "13.5"]),
            Some(0),
            &exceptions,
            Some(24)
        )),
        ["13화에 두 회차가 들어와요 (13, 13.5)"]
    );
    assert_eq!(
        collisions(&moves(&texts(&["13", "14"]), Some(0), &[], Some(24))),
        Vec::<String>::new()
    );
    // A not-received episode and an episode outside the season collide with nothing.
    assert_eq!(
        collisions(&moves(
            &texts(&["13", "13.5", "30"]),
            Some(0),
            &[exception("13.5", None)],
            Some(24)
        )),
        Vec::<String>::new()
    );
}

#[test]
fn the_group_line_gives_the_offset_when_no_episode_shows_it() {
    assert_eq!(user_line(5, vec![], &[], None), "직접 정함 · +5화 차이");
}

// ---- the cases the screen did not check on their own

#[test]
fn the_episodes_of_a_source_are_each_listed_once_by_number_in_order_without_zero() {
    assert_eq!(
        episodes_of(&texts(&[
            "SP", "14", "013", "13.0", "0", "00", "13.5", "2", "OVA"
        ])),
        ["2", "013", "13.5", "14", "OVA", "SP"]
    );
}

#[test]
fn a_collision_names_the_count_in_words_up_to_three() {
    let exceptions = [
        exception("A", Some(5)),
        exception("B", Some(5)),
        exception("C", Some(5)),
        exception("D", Some(5)),
    ];
    let one = |episodes: &[&str]| collisions(&moves(&texts(episodes), Some(0), &exceptions, None));
    assert_eq!(
        one(&["A", "B", "C"]),
        ["5화에 세 회차가 들어와요 (A, B, C)"]
    );
    assert_eq!(
        one(&["A", "B", "C", "D"]),
        ["5화에 4개 회차가 들어와요 (A, B, C, D)"]
    );
}

#[test]
fn the_preview_of_nothing_is_empty_and_a_row_less_dialog_shows_the_choices_alone() {
    assert_eq!(preview_text(&[], 4), "");
    let ground = Ground {
        episodes: &[],
        previous: Some(12),
        total: Some(12),
    };
    let input = Input {
        choice: Some(Choice::Continue),
        custom: String::new(),
        rows: vec![],
    };
    let shown = shown(&input, &ground);
    assert_eq!(shown.offset, Some(-12));
    assert_eq!(
        (
            shown.same.as_str(),
            shown.continued.as_str(),
            shown.custom.as_str()
        ),
        ("", "", "")
    );
}

fn twelve_then_ground(episodes: &[String]) -> Ground<'_> {
    Ground {
        episodes,
        previous: Some(12),
        total: Some(12),
    }
}

#[test]
fn the_dialog_shows_a_text_under_each_choice_and_the_warnings_of_the_one_chosen() {
    let episodes = texts(&["13", "14", "15", "24", "13.5"]);
    let input = Input {
        choice: Some(Choice::Continue),
        custom: "-5".to_owned(),
        rows: vec![row("13.5", "", true)],
    };
    let shown = shown(&input, &twelve_then_ground(&episodes));
    assert_eq!(shown.continue_reason, None);
    assert_eq!(shown.offset, Some(-12));
    assert_eq!(
        shown.same,
        "13화 → 시즌 밖 · 13.5화 → 받지 않음 · 14화 → 시즌 밖 … 24화 → 시즌 밖"
    );
    assert_eq!(
        shown.continued,
        "13화 → 1화 · 13.5화 → 받지 않음 · 14화 → 2화 … 24화 → 12화"
    );
    assert_eq!(
        shown.custom,
        "13화 → 8화 · 13.5화 → 받지 않음 · 14화 → 9화 … 24화 → 시즌 밖"
    );
    assert_eq!(shown.exceptions, [exception("13.5", None)]);
    assert_eq!(shown.exceptions_problem, None);
    assert_eq!(shown.offset_problem, None);
    assert!(shown.warnings.is_empty());
    assert!(shown.to_add.is_empty());
}

#[test]
fn the_dialog_offers_the_misfits_no_row_names_and_tells_a_custom_offset_that_is_no_offset() {
    let episodes = texts(&["12", "13.5", "SP", "24"]);
    let input = Input {
        choice: Some(Choice::Custom),
        custom: "12.5".to_owned(),
        rows: vec![row(" SP ", "", true)],
    };
    let shown = shown(&input, &twelve_then_ground(&episodes));
    assert_eq!(shown.offset, None);
    assert_eq!(
        shown.offset_problem.as_deref(),
        Some("-9999에서 9999 사이의 정수를 써 주세요.")
    );
    assert_eq!(shown.custom, "");
    // With no offset every whole episode is unplaced; the row written for `SP` is not offered again.
    assert_eq!(shown.to_add, ["12", "13.5", "24"]);
    // Nothing is said of an empty box, or while another choice is made.
    let blank = Input {
        custom: "  ".to_owned(),
        ..input.clone()
    };
    assert_eq!(
        super::shown(&blank, &twelve_then_ground(&episodes)).offset_problem,
        None
    );
    let other = Input {
        choice: Some(Choice::Same),
        ..input
    };
    assert_eq!(
        super::shown(&other, &twelve_then_ground(&episodes)).offset_problem,
        None
    );
}

#[test]
fn a_row_that_cannot_be_saved_is_told_and_the_previews_go_on_without_the_exceptions() {
    let episodes = texts(&["13", "13.5"]);
    let input = Input {
        choice: Some(Choice::Same),
        custom: String::new(),
        rows: vec![row("13.5", "", true), row("", "7", false)],
    };
    let shown = shown(&input, &twelve_then_ground(&episodes));
    assert_eq!(
        shown.exceptions_problem.as_deref(),
        Some("예외의 회차를 써 주세요.")
    );
    assert!(shown.exceptions.is_empty());
    assert_eq!(shown.same, "13화 → 시즌 밖 · 13.5화 → 정해지지 않음");
    // `13.5` has a row, so only `13`, which the same number puts outside the season, is offered.
    assert_eq!(shown.to_add, ["13"]);
}

#[test]
fn nothing_is_chosen_nothing_is_saved_and_the_choices_still_show() {
    let episodes = texts(&["1", "2"]);
    let input = Input {
        choice: None,
        custom: String::new(),
        rows: vec![],
    };
    let ground = Ground {
        episodes: &episodes,
        previous: None,
        total: Some(12),
    };
    let shown = shown(&input, &ground);
    assert_eq!(shown.offset, None);
    assert_eq!(shown.same, "1화 → 1화 · 2화 → 2화");
    assert_eq!(shown.continue_reason, continue_reason(None));
    assert_eq!(shown.continued, "");
    // No choice means no offset: every whole episode is unplaced, and each is offered.
    assert_eq!(shown.to_add, ["1", "2"]);
}

#[test]
fn texts_are_trimmed_as_the_dialog_always_read_them() {
    assert_eq!(trimmed("\u{feff} 12\u{a0}\n"), "12");
    assert_eq!(trimmed("\u{85}12"), "\u{85}12");
    assert_eq!(
        exceptions_of(&[row(" 13.5 ", " 2 ", false)]).unwrap(),
        [exception("13.5", Some(2))]
    );
}
