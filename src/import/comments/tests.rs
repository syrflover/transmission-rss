use super::*;

fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_owned).collect()
}

fn address(anime_no: i64, creator: Option<&str>) -> Reading {
    Reading::Address {
        anime_no,
        creator: creator.map(str::to_owned),
    }
}

fn unreadable(reason: &str) -> Reading {
    Reading::Unreadable {
        reason: reason.to_owned(),
    }
}

#[test]
fn an_address_and_a_creator_are_read_whatever_else_the_comment_says() {
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
                address(1001, Some("Team Alpha")),
                address(1002, None),
                unreadable(NO_ADDRESS),
                Reading::None,
                address(1001, Some("Team Alpha")),
            ],
            vec![address(1003, Some("Team Epsilon")), address(1004, None)],
        ]
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
