use super::*;

fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_owned).collect()
}

fn address(anime_no: i64, creator: Option<&str>) -> Reading {
    Reading::Address {
        anime_no,
        creator: creator.map(str::to_owned),
        airs: None,
    }
}

/// An address read together with the weekday and time of the comment's line 1.
fn address_airs(anime_no: i64, creator: Option<&str>, week: u8, time: &str) -> Reading {
    Reading::Address {
        anime_no,
        creator: creator.map(str::to_owned),
        airs: Some(Airs {
            week,
            time: time.to_owned(),
        }),
    }
}

fn unreadable(reason: &str) -> Reading {
    Reading::Unreadable {
        reason: reason.to_owned(),
    }
}

#[test]
fn an_address_and_a_creator_are_read_whatever_else_the_comment_says() {
    // The labelled forms and the paths without `animeNo` are also accepted.
    let comment = lines(" 수 22:30\n 자막: Team Alpha\n https://anissia.net/anime/1001");
    assert_eq!(read(&comment), address(1001, Some("Team Alpha")));

    // The creator may share a line with other parts, split at `|`.
    let one_line = lines(" 수 22:30 | 자막 제작자 : Team Beta | https://anissia.net/anime/1002");
    assert_eq!(read(&one_line), address(1002, Some("Team Beta")));

    // Full-width colon and the other labels.
    for label in ["제작자", "자막팀", "자막 제작자", "자막"] {
        let c = lines(&format!(
            " {label}： Team Gamma\n https://anissia.net/anime/7"
        ));
        assert_eq!(read(&c), address(7, Some("Team Gamma")), "{label}");
    }
}

#[test]
fn an_address_without_a_creator_reads_as_no_creator() {
    assert_eq!(
        read(&lines(" 목 23:00 https://anissia.net/anime/1002")),
        address(1002, None)
    );
    // A creator label with nothing useful after it names nobody.
    for blank in ["", "미정", "없음", "-", "?"] {
        let c = lines(&format!(" 자막: {blank}\n https://anissia.net/anime/9"));
        assert_eq!(read(&c), address(9, None), "{blank:?}");
    }
    // An address is not the creator, even after the label.
    let c = lines(" 자막: https://anissia.net/anime/9");
    assert_eq!(read(&c), address(9, None));
}

#[test]
fn the_anime_number_comes_from_the_query_or_the_last_numeric_segment() {
    for (url, no) in [
        ("https://anissia.net/anime/1001", 1001),
        ("https://anissia.net/anime/caption/1001/", 1001),
        ("https://www.anissia.net/anime?animeNo=55", 55),
        ("http://anissia.net/a?anime_no=12&x=1", 12),
    ] {
        assert_eq!(read(&lines(&format!(" {url}"))), address(no, None), "{url}");
    }
    // A fragment is not read.
    assert_eq!(
        read(&lines(" https://anissia.net/#/x?no=3")),
        unreadable(ODD_ADDRESS)
    );
    // Written inside text, in brackets or with trailing punctuation.
    assert_eq!(
        read(&lines(" 보기 (https://anissia.net/anime/42), 끝")),
        address(42, None)
    );
}

#[test]
fn a_comment_that_names_no_readable_address_says_why() {
    // No Anissia address at all.
    assert_eq!(
        read(&lines(
            " 금 21:00 자막 오래전에 받던 곳 https://nyaa.example.test/?q=x"
        )),
        unreadable(NO_ADDRESS)
    );
    assert_eq!(read(&lines(" Q. 2026/3")), unreadable(NO_ADDRESS));
    // An Anissia address that holds no anime number.
    assert_eq!(
        read(&lines(" https://anissia.net/")),
        unreadable(ODD_ADDRESS)
    );
    assert_eq!(
        read(&lines(" https://anissia.net/anime/schedule")),
        unreadable(ODD_ADDRESS)
    );
    assert_eq!(
        read(&lines(" https://anissia.net/anime/0")),
        unreadable(ODD_ADDRESS)
    );
    // Another host that only looks like Anissia.
    assert_eq!(
        read(&lines(" https://notanissia.net/anime/5")),
        unreadable(NO_ADDRESS)
    );
    assert_eq!(
        read(&lines(" https://anissia.net.example.test/anime/5")),
        unreadable(NO_ADDRESS)
    );
    // Two different anime.
    assert_eq!(
        read(&lines(
            " https://anissia.net/anime/5\n https://anissia.net/anime/6"
        )),
        unreadable(SEVERAL_ANIME)
    );
    // The same anime twice is one anime, and a home page next to it is ignored.
    assert_eq!(
        read(&lines(
            " https://anissia.net/anime/5\n https://anissia.net/anime/5\n https://anissia.net/"
        )),
        address(5, None)
    );
}

#[test]
fn no_comment_text_is_no_comment() {
    assert_eq!(read(&[]), Reading::None);
    assert_eq!(read(&lines("\n  \n")), Reading::None);
}

#[test]
fn each_rule_gets_the_comment_directly_above_it() {
    let content = include_str!("../../../tests/fixtures/legacy_commented.yml");
    let readings = rule_comments(content, &[5, 2]);
    assert_eq!(
        readings,
        vec![
            vec![
                // The section heading is separated by a blank line.
                address_airs(1001, Some("Team Alpha"), 3, "22:30"),
                // A line 1 with no creator is address only.
                address_airs(1002, None, 4, "23:00"),
                unreadable(NO_ADDRESS),
                Reading::None,
                address_airs(1001, Some("Team Alpha"), 6, "01:00"),
            ],
            vec![
                address_airs(1003, Some("Team Epsilon"), 0, "12:00"),
                // A time that is not `HH:MM`: the address alone.
                address(1004, None),
            ],
        ]
    );
}

#[test]
fn the_two_line_convention_gives_the_weekday_time_creator_and_anime() {
    let comment = lines(" Mon. 23:30. Team Alpha\n https://anissia.net/anime?animeNo=1001");
    assert_eq!(
        read(&comment),
        address_airs(1001, Some("Team Alpha"), 1, "23:30")
    );
    // The address line is the one directly above the rule: with line 1 after
    // it, the comment is not the convention and attributes nothing.
    let reversed = lines("  https://anissia.net/anime?animeNo=1001 \n   Mon. 23:30. Team Alpha  ");
    assert_eq!(read(&reversed), unreadable(ADDRESS_NOT_LAST));
}

#[test]
fn the_creator_is_everything_after_the_second_separator() {
    let comment =
        lines(" Tue. 00:05. Team A. B & Co. 2nd Unit \n https://anissia.net/anime?animeNo=12");
    assert_eq!(
        read(&comment),
        address_airs(12, Some("Team A. B & Co. 2nd Unit"), 2, "00:05")
    );
}

#[test]
fn each_weekday_abbreviation_is_read() {
    for (week, day) in ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        .iter()
        .enumerate()
    {
        let comment = lines(&format!(
            " {day}. 21:00. Team\n https://anissia.net/anime?animeNo=5"
        ));
        assert_eq!(
            read(&comment),
            address_airs(5, Some("Team"), week as u8, "21:00"),
            "{day}"
        );
    }
}

#[test]
fn a_line_one_that_does_not_fit_leaves_the_address_only() {
    let url = "https://anissia.net/anime?animeNo=7";
    for line in [
        // A time that is not `HH:MM`.
        "Mon. 9pm. Team",
        "Mon. 2330. Team",
        "Mon. 23:60. Team",
        "Mon. 24:00. Team",
        "Mon. 7:30. Team",
        // A weekday that is not the three-letter abbreviation.
        "Monday. 23:30. Team",
        "Mo. 23:30. Team",
        "Xyz. 23:30. Team",
        // Another separator.
        "Mon 23:30 Team",
        "Mon, 23:30, Team",
        // Nothing but the line's text.
        "Team Alpha",
    ] {
        let comment = lines(&format!(" {line}\n {url}"));
        assert_eq!(read(&comment), address(7, None), "{line}");
    }
}

#[test]
fn an_empty_creator_field_is_address_only_with_the_time_kept() {
    for line in [
        "Fri. 22:00.",
        "Fri. 22:00. ",
        "Fri. 22:00. 미정",
        "Fri. 22:00. -",
    ] {
        let comment = lines(&format!(" {line}\n https://anissia.net/anime?animeNo=7"));
        assert_eq!(read(&comment), address_airs(7, None, 5, "22:00"), "{line}");
    }
}

#[test]
fn a_missing_line_one_is_address_only() {
    assert_eq!(
        read(&lines(" https://anissia.net/anime?animeNo=7")),
        address(7, None)
    );
}

#[test]
fn a_blank_line_between_the_comment_and_the_rule_leaves_no_comment() {
    let content = "\
- url: https://x.test/rss
  directory: /m
  rules:
    # Mon. 23:30. Team Alpha
    # https://anissia.net/anime?animeNo=1001

    - match: a
      directory: A
    # Mon. 23:30. Team Alpha
    # https://anissia.net/anime?animeNo=1002
    - match: b
      directory: B
";
    assert_eq!(
        rule_comments(content, &[2]),
        vec![vec![
            Reading::None,
            address_airs(1002, Some("Team Alpha"), 1, "23:30")
        ]]
    );
}

#[test]
fn a_comment_above_something_else_is_not_a_rules_comment() {
    let content = "\
# above the file
- url: https://x.test/rss
  # above a key
  directory: /m
  # above the rules key
  rules:
    - match: a
      # inside the rule
      directory: A
- url: https://y.test/rss
  directory: /m
  rules:
    # https://anissia.net/anime/5

    - match: b
      directory: B
";
    assert_eq!(
        rule_comments(content, &[1, 1]),
        vec![vec![Reading::None], vec![Reading::None]]
    );
}

#[test]
fn the_layouts_yaml_allows_for_the_rules_list_are_followed() {
    // The sequence at the key's own indentation, CRLF line ends, a BOM, a
    // document marker, and rules that end where the next key starts.
    let content = "\u{feff}---\r\n- url: https://x.test/rss\r\n  directory: /m\r\n  rules:\r\n  # https://anissia.net/anime/11\r\n  - match: a\r\n    directory: A\r\n  # https://anissia.net/anime/12\r\n  - match: b\r\n    directory: B\r\n  excludes:\r\n  - skip\r\n- url: https://y.test/rss\r\n  directory: /m\r\n  rules: []\r\n";
    assert_eq!(
        rule_comments(content, &[2, 0]),
        vec![vec![address(11, None), address(12, None)], vec![]]
    );
}

#[test]
fn a_file_the_scan_cannot_place_comments_in_reads_none_without_comments_and_unreadable_with() {
    // Rules written inline are not scanned.
    let inline =
        "- url: https://x.test/rss\n  directory: /m\n  rules: [{match: a, directory: A}]\n";
    assert_eq!(rule_comments(inline, &[1]), vec![vec![Reading::None]]);
    let inline_commented = format!("# a comment\n{inline}");
    assert_eq!(
        rule_comments(&inline_commented, &[1]),
        vec![vec![unreadable(NO_PLACE)]]
    );
    // The parser found more rules than the scan did.
    let content = "- url: https://x.test/rss\n  directory: /m\n  rules:\n    # https://anissia.net/anime/5\n    - match: a\n      directory: A\n";
    assert_eq!(
        rule_comments(content, &[2]),
        vec![vec![unreadable(NO_PLACE); 2]]
    );
}

const HEAD: &str = "    # Mon. 23:30. Team Alpha\n    # https://anissia.net/anime?animeNo=1001\n";

/// A channel with one rule below `above`.
fn yaml_with(above: &str) -> String {
    format!(
        "- url: https://x.test/rss\n  directory: /m\n  rules:\n{above}    - match: a\n      directory: A\n"
    )
}

#[test]
fn only_the_two_lines_directly_above_the_rule_are_its_comment() {
    // A commented-out rule above the real comment, and a note above it, are not
    // part of it.
    let above =
        format!("    # - match: old\n    #   directory: Old\n    # a note about this one\n{HEAD}");
    assert_eq!(
        rule_comments(&yaml_with(&above), &[1]),
        vec![vec![address_airs(1001, Some("Team Alpha"), 1, "23:30")]]
    );
}

#[test]
fn a_commented_out_rule_above_a_rule_lends_it_no_address() {
    // The commented-out rule kept its own comment, address included, and
    // nothing real sits between it and the rule below.
    let above = format!("{HEAD}    # - match: old\n    #   directory: Old\n");
    assert_eq!(
        rule_comments(&yaml_with(&above), &[1]),
        vec![vec![unreadable(NO_ADDRESS)]]
    );
    // With the address on the line just above the commented-out line.
    let above = format!("{HEAD}    # - match: old\n");
    assert_eq!(
        rule_comments(&yaml_with(&above), &[1]),
        vec![vec![unreadable(ADDRESS_NOT_LAST)]]
    );
}

#[test]
fn a_note_between_the_address_and_the_rule_leaves_the_comment_unattributed() {
    let above = format!("{HEAD}    # watch out for batches\n");
    assert_eq!(
        rule_comments(&yaml_with(&above), &[1]),
        vec![vec![unreadable(ADDRESS_NOT_LAST)]]
    );
}
