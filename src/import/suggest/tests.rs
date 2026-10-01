use super::*;

fn rule(directory: &str) -> RuleInput {
    RuleInput {
        r#match: Some("a".into()),
        directory: directory.into(),
        ..RuleInput::default()
    }
}

fn address(anime_no: i64, creator: Option<&str>) -> Reading {
    Reading::Address {
        anime_no,
        creator: creator.map(str::to_owned),
        airs: None,
    }
}

#[test]
fn the_four_cases_start_checked_only_when_a_creator_is_named() {
    let readings = [
        address(1, Some("Team")),
        address(2, None),
        Reading::Unreadable {
            reason: "Anissia 주소 형식이 달라서".into(),
        },
        Reading::None,
    ];
    let rules: Vec<_> = (0..4)
        .map(|i| rule(&format!("Show {i}/Season 01")))
        .collect();
    let out = suggest(&readings, &rules);

    let kinds: Vec<_> = out.iter().map(|s| s.kind()).collect();
    assert_eq!(
        kinds,
        [
            Kind::WithCreator,
            Kind::AddressOnly,
            Kind::Unreadable,
            Kind::None
        ]
    );
    let checked: Vec<_> = out.iter().map(|s| s.checked_at_first()).collect();
    assert_eq!(checked, [true, false, false, false]);
    assert_eq!(out[0].offer(), Some((1, Some("Team"))));
    assert_eq!(out[1].offer(), Some((2, None)));
    assert_eq!(out[2].offer(), None);
    assert_eq!(out[3].offer(), None);
    assert!(out.iter().all(|s| s.blocked.is_none()));
}

#[test]
fn a_rule_saving_into_the_collect_folder_itself_cannot_be_a_subscription() {
    let readings = vec![address(1, Some("Team")); 3];
    let rules = [rule(""), rule("."), rule("./")];
    for suggestion in suggest(&readings, &rules) {
        assert!(suggestion.blocked.is_some());
        assert!(!suggestion.checked_at_first());
        assert_eq!(suggestion.offer(), None);
        // What the comment said is still shown.
        assert_eq!(suggestion.kind(), Kind::WithCreator);
    }
    // A comment that was not read is never blocked: nothing is offered.
    let out = suggest(&[Reading::None], &[rule("")]);
    assert_eq!(out[0].blocked, None);
}

#[test]
fn a_channel_follows_an_anime_with_its_first_rule_only() {
    let readings = [
        address(1, Some("Team")),
        address(2, None),
        address(1, Some("Other")),
        address(2, None),
    ];
    let rules = [rule("A"), rule("B"), rule("A2"), rule("B2")];
    let out = suggest(&readings, &rules);
    assert_eq!(out[0].blocked, None);
    assert_eq!(out[1].blocked, None);
    assert!(out[2].blocked.as_deref().unwrap().contains("1번째 규칙"));
    assert!(out[3].blocked.as_deref().unwrap().contains("2번째 규칙"));
    // A rule blocked for its folder does not claim the anime.
    let out = suggest(
        &[address(1, None), address(1, None)],
        &[rule(""), rule("B")],
    );
    assert!(out[0].blocked.is_some());
    assert_eq!(out[1].blocked, None);
}
