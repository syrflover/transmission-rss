use super::*;
use crate::suggest::suggest;

fn rule(matching: &str, directory: &str) -> RuleInput {
    RuleInput {
        r#match: Some(matching.into()),
        directory: directory.into(),
        ..RuleInput::default()
    }
}

fn airs(week: u8, time: &str) -> Option<Airs> {
    Some(Airs {
        week,
        time: time.into(),
    })
}

fn address(anime_no: i64, creator: Option<&str>, airs: Option<Airs>) -> Reading {
    Reading::Address {
        anime_no,
        creator: creator.map(str::to_owned),
        airs,
    }
}

fn pick_of(channel: usize, rule: usize) -> Pick {
    Pick { channel, rule }
}

/// The file's channel 0: five rules in the order of the sample file the import
/// tests read (`legacy_commented.yml`): Alpha with a creator and Wednesday,
/// Beta without a creator, a comment that cannot be read, no comment, and Alpha
/// again.
fn channel() -> (Vec<Suggestion>, Vec<RuleInput>) {
    let readings = [
        address(1001, Some("Team Alpha"), airs(3, "22:30")),
        address(1002, None, airs(4, "23:00")),
        Reading::Unreadable {
            reason: "Anissia 주소가 없어서".into(),
        },
        Reading::None,
        address(1001, Some("Team Alpha"), airs(6, "01:00")),
    ];
    let rules = vec![
        rule("Alpha", "Alpha/Season 01"),
        rule("Beta", "Beta/Season 01"),
        rule("Gamma", "Gamma/Season 01"),
        rule("Delta", "Delta/Season 01"),
        rule("Alpha Again", "Alpha/Season 02"),
    ];
    (suggest(&readings, &rules), rules)
}

/// Channel 0 imported by the plan's action 0, as the preview numbered it.
fn decide(picks: &[Pick]) -> Result<Picked, StalePick> {
    let (suggestions, rules) = channel();
    pick(
        picks,
        &HashMap::from([(0, suggestions)]),
        &HashMap::from([(0, rules)]),
        &HashMap::from([(0, 0)]),
    )
}

/// What Anissia lists: Alpha, with its own subject.
fn listed() -> Resolved {
    Resolved {
        found: HashMap::from([(
            1001,
            ImportSubscription::stand_in(1001, "알파", Some((3, "22:30"))),
        )]),
        unavailable: None,
    }
}

fn asked_in_vain() -> Resolved {
    Resolved {
        found: HashMap::new(),
        unavailable: Some("Anissia에 연결하지 못했어요.".into()),
    }
}

#[test]
fn a_checked_address_follows_its_creator_and_one_without_leaves_the_creator_undecided() {
    let picked = decide(&[pick_of(0, 0), pick_of(0, 1)]).unwrap();
    assert_eq!(picked.anime_nos(), [1001, 1002]);

    let resolved = Resolved {
        found: HashMap::from([
            (
                1001,
                ImportSubscription::stand_in(1001, "알파", Some((3, "22:30"))),
            ),
            (
                1002,
                ImportSubscription::stand_in(1002, "베타", Some((4, "23:00"))),
            ),
        ]),
        unavailable: None,
    };
    let made = subscriptions(&picked.wanted, &resolved);
    assert_eq!(made.len(), 2);
    let (alpha, beta) = (&made[0], &made[1]);
    assert_eq!((alpha.action, alpha.rule), (0, 0));
    assert_eq!(alpha.subscription.subtitles, SubtitleMode::Follow);
    assert_eq!(alpha.subscription.creator.as_deref(), Some("Team Alpha"));
    assert_eq!((beta.action, beta.rule), (0, 1));
    assert_eq!(beta.subscription.subtitles, SubtitleMode::Undecided);
    assert_eq!(beta.subscription.creator, None);
    // What Anissia listed is the snapshot; no stand-in is made, and the store
    // stamps the time itself.
    for (made, subject) in [(alpha, "알파"), (beta, "베타")] {
        assert_eq!(made.subscription.anime.subject, subject);
        assert!(!made.placeholder);
        assert_eq!(made.subscription.subscribed_at, 0);
    }
}

#[test]
fn a_pick_is_asked_for_once_and_a_channel_that_is_skipped_or_not_imported_drops_its_picks() {
    // Twice the same pick is one pick.
    let picked = decide(&[pick_of(0, 0), pick_of(0, 0)]).unwrap();
    assert_eq!(picked.wanted.len(), 1);

    let (suggestions, rules) = channel();
    let by_file = |channels: &[usize]| {
        (
            channels
                .iter()
                .map(|&c| (c, suggestions.clone()))
                .collect::<HashMap<_, _>>(),
            channels
                .iter()
                .map(|&c| (c, rules.clone()))
                .collect::<HashMap<_, _>>(),
        )
    };
    // Channels 0 and 1 are imported; the plan's action 0 imports channel 1
    // because the user skipped channel 0.
    let (suggested, ruled) = by_file(&[0, 1]);
    let picked = pick(
        &[pick_of(0, 0), pick_of(1, 0)],
        &suggested,
        &ruled,
        &HashMap::from([(1, 0)]),
    )
    .unwrap();
    assert_eq!(picked.wanted.len(), 1);
    assert_eq!((picked.wanted[0].channel, picked.wanted[0].action), (1, 0));

    // A channel that is not imported has no suggestions, so even a pick that
    // would be stale in it is left out, not refused.
    let (suggested, ruled) = by_file(&[1]);
    let picked = pick(
        &[pick_of(0, 2), pick_of(0, 99)],
        &suggested,
        &ruled,
        &HashMap::from([(1, 0)]),
    )
    .unwrap();
    assert!(picked.wanted.is_empty());
    assert!(picked.not_created.is_empty());
}

#[test]
fn a_pick_the_file_does_not_back_is_stale() {
    // Rule 2's comment cannot be read, rule 3 has none, rule 9 is not there;
    // each makes the whole review stale, even next to a good pick.
    for rule in [2, 3, 9] {
        assert!(
            decide(&[pick_of(0, 0), pick_of(0, rule)]).is_err(),
            "rule {rule}"
        );
    }
    assert!(decide(&[pick_of(0, 0), pick_of(0, 1)]).is_ok());
}

#[test]
fn a_pick_that_cannot_be_a_subscription_is_reported_and_leaves_the_others_alone() {
    // Rule 4 offers Alpha again after rule 0 does.
    let picked = decide(&[pick_of(0, 0), pick_of(0, 4)]).unwrap();
    assert_eq!(picked.anime_nos(), [1001]);
    assert_eq!(picked.not_created.len(), 1);
    let blocked = &picked.not_created[0];
    assert_eq!((blocked.channel, blocked.rule), (0, 4));
    assert!(blocked.reason.contains("같은 작품"), "{}", blocked.reason);

    // A rule saving into the collect folder itself or through `..` is reported
    // with the sentence the preview shows for it.
    let readings = [
        address(1, Some("Team"), None),
        address(2, Some("Team"), None),
    ];
    let rules = vec![rule("Direct", ""), rule("Up", "../Elsewhere")];
    let suggestions = suggest(&readings, &rules);
    let reasons: Vec<_> = suggestions.iter().map(|s| s.blocked.clone()).collect();
    let picked = pick(
        &[pick_of(0, 0), pick_of(0, 1)],
        &HashMap::from([(0, suggestions)]),
        &HashMap::from([(0, rules)]),
        &HashMap::from([(0, 0)]),
    )
    .unwrap();
    assert!(picked.wanted.is_empty());
    let reported: Vec<_> = picked
        .not_created
        .iter()
        .map(|n| (n.channel, n.rule, Some(n.reason.clone())))
        .collect();
    assert_eq!(
        reported,
        [(0, 0, reasons[0].clone()), (0, 1, reasons[1].clone())]
    );
    assert!(reasons[0].as_deref().unwrap().contains("수집 폴더"));
    assert!(reasons[1].as_deref().unwrap().contains(".."));
}

#[test]
fn an_anime_anissia_answered_without_is_left_out_and_one_it_was_not_asked_about_stays() {
    // Zeta (9) is on no list; Alpha is listed.
    let readings = [
        address(1001, Some("Team"), None),
        address(9, Some("Team"), None),
    ];
    let rules = vec![rule("Alpha", "Alpha"), rule("Zeta", "Zeta")];
    let make = || {
        pick(
            &[pick_of(0, 0), pick_of(0, 1)],
            &HashMap::from([(0, suggest(&readings, &rules))]),
            &HashMap::from([(0, rules.clone())]),
            &HashMap::from([(0, 0)]),
        )
        .unwrap()
    };

    let settled = settle(make(), &listed());
    assert_eq!(settled.anime_nos(), [1001]);
    assert_eq!(settled.not_created.len(), 1);
    let left_out = &settled.not_created[0];
    assert_eq!((left_out.channel, left_out.rule), (0, 1));
    assert!(
        left_out.reason.contains("편성표에 없는"),
        "{}",
        left_out.reason
    );

    // When Anissia could not be asked at all, nothing is known to be unlisted:
    // both stay, to be made with stand-ins.
    let settled = settle(make(), &asked_in_vain());
    assert_eq!(settled.anime_nos(), [1001, 9]);
    assert!(settled.not_created.is_empty());
}

#[test]
fn a_stand_in_sits_on_the_comments_weekday_and_time_or_in_the_other_tab() {
    // Beta's comment gives Thursday 23:00 and a rule saves under "Beta"; Gamma's
    // comment gives no weekday.
    let readings = [
        address(1002, None, airs(4, "23:00")),
        address(1003, Some("Team"), None),
    ];
    let rules = vec![rule("[S] Beta - ", "Beta"), rule("[S] Gamma - ", "Gamma")];
    let picked = pick(
        &[pick_of(0, 0), pick_of(0, 1)],
        &HashMap::from([(0, suggest(&readings, &rules))]),
        &HashMap::from([(0, rules.clone())]),
        &HashMap::from([(0, 0)]),
    )
    .unwrap();

    let resolved = asked_in_vain();
    let made = subscriptions(&picked.wanted, &resolved);
    let (beta, gamma) = (&made[0], &made[1]);
    assert!(beta.placeholder && gamma.placeholder);
    // The stand-in is titled by the rule's match phrase.
    assert_eq!(beta.subscription.anime.subject, "[S] Beta - ");
    assert_eq!(beta.subscription.anime.week, 4);
    assert_eq!(beta.subscription.anime.air_time.as_deref(), Some("23:00"));
    assert_eq!(gamma.subscription.anime.week, trss_anissia::WEEK_OTHER);
    assert_eq!(gamma.subscription.anime.air_time, None);

    let outcomes = [SubscriptionOutcome::Created, SubscriptionOutcome::Created];
    let told = result(picked, &resolved, &outcomes);
    assert_eq!(told.created_count(), 2);
    assert_eq!(
        told.unavailable.as_deref(),
        Some("Anissia에 연결하지 못했어요.")
    );
    let (beta, gamma) = (&told.created[0], &told.created[1]);
    for created in [beta, gamma] {
        assert!(!created.schedule_known);
        assert_eq!(created.subject, None);
    }
    assert!(beta.schedule_from_comment);
    assert!(!gamma.schedule_from_comment);
    assert_eq!(gamma.creator.as_deref(), Some("Team"));
}

#[test]
fn what_anissia_listed_is_used_whatever_the_comment_said() {
    // Rule 0's comment says Wednesday 22:30 for Alpha, and Anissia lists the
    // anime under another subject: Anissia's word stands, and no stand-in is
    // made.
    let picked = decide(&[pick_of(0, 0)]).unwrap();
    let resolved = listed();
    let made = subscriptions(&picked.wanted, &resolved);
    assert!(!made[0].placeholder);
    assert_eq!(made[0].subscription.anime.week, 3);

    let told = result(picked, &resolved, &[SubscriptionOutcome::Created]);
    let created = &told.created[0];
    assert!(created.schedule_known);
    assert_eq!(created.subject.as_deref(), Some("알파"));
    // The comment gave a weekday and time, but they were not used.
    assert!(!created.schedule_from_comment);
    assert_eq!(told.unavailable, None);
}

#[test]
fn a_pick_the_store_did_not_make_is_reported_with_its_reason() {
    let picked = decide(&[pick_of(0, 0), pick_of(0, 1), pick_of(0, 4)]).unwrap();
    // Rule 4 was blocked before the store was asked.
    assert_eq!(picked.wanted.len(), 2);
    let outcomes = [
        SubscriptionOutcome::RuleAlreadySubscribed,
        SubscriptionOutcome::AnimeTakenInChannel {
            rule_id: "other".into(),
        },
    ];
    let told = result(picked, &listed(), &outcomes);
    assert_eq!(told.created_count(), 0);
    let reasons: Vec<_> = told
        .not_created
        .iter()
        .map(|n| (n.channel, n.rule, n.reason.as_str()))
        .collect();
    assert_eq!(reasons.len(), 3);
    // The blocked pick comes first, then the store's refusals in pick order.
    assert_eq!((reasons[0].0, reasons[0].1), (0, 4));
    assert_eq!(
        reasons[1],
        (0, 0, "이 규칙은 이미 구독이라서 그대로 두었어요.")
    );
    assert_eq!(
        reasons[2],
        (0, 1, "이 채널에는 같은 작품을 구독하는 규칙이 이미 있어요.")
    );
}
