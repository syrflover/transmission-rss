use super::*;
use crate::store::channels::{Channel, Subscription, SubtitleMode};

/// 2026-10-01 12:00 in Seoul, when the subscriptions below began.
const NOW: i64 = 1_790_780_400_000 + 12 * 60 * 60 * 1000;

fn channel_excluding(excludes: &[&str]) -> Channel {
    Channel {
        id: "c1".into(),
        position: 0,
        version: 1,
        url: "https://feed.test/rss".into(),
        excludes: excludes.iter().map(|e| e.to_string()).collect(),
        secret_query: Vec::new(),
        past_search: None,
        name: None,
    }
}

fn channel() -> Channel {
    channel_excluding(&["[Batch]"])
}

fn rule(id: &str, position: i64, m: Option<&str>) -> Rule {
    Rule {
        id: id.into(),
        channel_id: "c1".into(),
        position,
        version: 1,
        r#match: m.map(str::to_owned),
        regex: false,
        case_insensitive: false,
        directory: format!("{id}/Season 01"),
        episode: 0,
        episode_auto: false,
        state: RuleState::Active,
        subscription: None,
        resumed_at: None,
    }
}

fn channel_with(rules: Vec<Rule>) -> ChannelWithRules {
    ChannelWithRules {
        channel: channel(),
        rules,
    }
}

fn item(id: i64, title: &str, first_seen_at: i64, result: HistoryResult) -> HistoryItem {
    HistoryItem {
        id,
        channel_id: "c1".into(),
        channel_label: "https://feed.test/".into(),
        identity_key: title.into(),
        title: title.into(),
        link: "https://feed.test/x".into(),
        first_seen_at,
        last_seen_at: first_seen_at,
        result,
        result_at: first_seen_at,
        rule_id: None,
        reason: None,
        torrent_hash: None,
        first_read: false,
    }
}

fn input(m: &str, directory: &str) -> RuleInput {
    RuleInput {
        r#match: Some(m.into()),
        directory: directory.into(),
        episode: 0,
        ..RuleInput::default()
    }
}

fn run(cwr: &ChannelWithRules, edit: Edit<'_>, items: &[HistoryItem]) -> Preview {
    preview(Path::new("/media"), cwr, edit, items, 100).unwrap()
}

/// A subscription began at `subscribed_at`, with its phrase given at `titled_at`
/// when it had waited for one.
fn subscribed(mut rule: Rule, subscribed_at: i64, titled_at: Option<i64>) -> Rule {
    rule.subscription = Some(Subscription {
        anissia_anime_no: 3320,
        subtitles: SubtitleMode::Undecided,
        creator: None,
        season_id: None,
        season_blocked: None,
        subscribed_at,
        titled_at,
    });
    rule
}

fn edit_of<'a>(id: &'a str, input: &'a RuleInput) -> Edit<'a> {
    Edit {
        id: Some(id),
        input,
        position: None,
    }
}

fn new_rule<'a>(input: &'a RuleInput) -> Edit<'a> {
    Edit {
        id: None,
        input,
        position: None,
    }
}

/// The listed items as `(title, kind, past cause)`, by title.
fn rows(preview: &Preview) -> Vec<(String, Kind, Option<PastCause>)> {
    let mut rows: Vec<_> = preview
        .items
        .iter()
        .map(|i| (i.title.clone(), i.kind, i.past_cause))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

fn row(title: &str, kind: Kind, cause: Option<PastCause>) -> (String, Kind, Option<PastCause>) {
    (title.to_owned(), kind, cause)
}

fn recorded(titles: &[&str], first_seen_at: i64) -> Vec<HistoryItem> {
    titles
        .iter()
        .enumerate()
        .map(|(n, title)| item(n as i64 + 1, title, first_seen_at, HistoryResult::NoMatch))
        .collect()
}

#[test]
fn a_new_rule_goes_last_unless_a_position_says_otherwise() {
    let cwr = channel_with(vec![rule("first", 0, Some("Show"))]);
    let items = recorded(&["Show - 01"], 10);
    let edited = input("Show", "New/Season 01");

    // Last: the first rule takes the item.
    let last = run(&cwr, new_rule(&edited), &items);
    assert_eq!(rows(&last), [row("Show - 01", Kind::Earlier, None)]);
    assert_eq!(
        last.items[0].taken_by,
        Some(TakenBy {
            rule_id: "first".into(),
            r#match: Some("Show".into())
        })
    );
    assert_eq!(
        last.items[0].save_path.as_deref(),
        Some(Path::new("/media/first/Season 01"))
    );

    // First: the new rule takes it.
    let first = run(
        &cwr,
        Edit {
            position: Some(0),
            ..new_rule(&edited)
        },
        &items,
    );
    assert_eq!(rows(&first), [row("Show - 01", Kind::Mine, None)]);
    assert_eq!(
        first.items[0].save_path.as_deref(),
        Some(Path::new("/media/New/Season 01"))
    );
}

#[test]
fn an_unknown_rule_id_is_not_found() {
    let cwr = channel_with(vec![rule("first", 0, Some("Show"))]);
    let edited = input("Show", "x");
    let err = preview(
        Path::new("/media"),
        &cwr,
        edit_of("nope", &edited),
        &[],
        100,
    )
    .unwrap_err();
    assert_eq!(err, RuleNotFound("nope".into()));
}

#[test]
fn widening_a_phrase_shows_the_items_an_earlier_rule_takes() {
    let rules = |gamma: &str| {
        vec![
            rule("alpha", 0, Some("Alpha - ")),
            rule("beta", 1, Some("Beta - ")),
            rule("gamma", 2, Some(gamma)),
        ]
    };
    let items = recorded(
        &["Alpha - 01", "Beta - 01", "Gamma - 01", "Other - 01"],
        1_000,
    );
    let cwr = channel_with(rules("Gamma - "));
    let gamma = cwr.rules[2].to_input();

    // As it is, the rule takes only its own item.
    let before = run(&cwr, edit_of("gamma", &gamma), &items);
    assert_eq!(rows(&before), [row("Gamma - 01", Kind::Mine, None)]);
    assert_eq!(before.counts.unmatched, 3);

    // Widened to match everything with " - 01": the others go to earlier rules.
    let wide = RuleInput {
        r#match: Some(" - 01".into()),
        ..gamma
    };
    let preview = run(&cwr, edit_of("gamma", &wide), &items);
    assert_eq!(
        rows(&preview),
        [
            row("Alpha - 01", Kind::Earlier, None),
            row("Beta - 01", Kind::Earlier, None),
            row("Gamma - 01", Kind::Mine, None),
            row("Other - 01", Kind::Mine, None),
        ]
    );
    // The taking rule is named, with its phrase and where the item would go.
    let alpha = &preview.items[0];
    assert_eq!(
        alpha.taken_by,
        Some(TakenBy {
            rule_id: "alpha".into(),
            r#match: Some("Alpha - ".into())
        })
    );
    assert_eq!(
        alpha.save_path.as_deref(),
        Some(Path::new("/media/alpha/Season 01"))
    );
    assert_eq!((preview.counts.mine, preview.counts.earlier), (2, 2));

    // The overlap flag of a saved rule list: not before, and the widened rule after.
    let titles = || items.iter().map(|i| i.title.as_str());
    let plan = ChannelPlan::new(cwr, Path::new("/media"));
    assert!(overlapping_rules(&plan, titles()).is_empty());
    let saved = ChannelPlan::new(channel_with(rules(" - 01")), Path::new("/media"));
    assert_eq!(
        overlapping_rules(&saved, titles()),
        HashSet::from(["gamma".to_owned()])
    );
}

#[test]
fn an_invalid_regex_is_the_edited_rules_error_and_other_rules_still_preview() {
    let cwr = channel_with(vec![
        rule("alpha", 0, Some("Alpha")),
        rule("beta", 1, Some("Beta")),
    ]);
    let items = recorded(&["Alpha - 01", "Beta - 01"], 1_000);

    let broken = RuleInput {
        regex: true,
        ..input("Beta - (", "Beta")
    };
    let p = run(&cwr, edit_of("beta", &broken), &items);
    assert!(p.error.is_some());
    assert_eq!(rows(&p), []);

    // Another rule's preview is unaffected by this one's edit.
    let alpha = cwr.rules[0].to_input();
    let fine = run(&cwr, edit_of("alpha", &alpha), &items);
    assert!(fine.error.is_none());
    assert_eq!(rows(&fine), [row("Alpha - 01", Kind::Mine, None)]);

    // A broken regex of another stored rule is not the edited rule's error.
    let mut stored_broken = rule("beta", 1, Some("Beta - ("));
    stored_broken.regex = true;
    let cwr = channel_with(vec![rule("alpha", 0, Some("Alpha")), stored_broken]);
    let fine = run(&cwr, edit_of("alpha", &alpha), &items);
    assert!(fine.error.is_none());
    assert_eq!(rows(&fine), [row("Alpha - 01", Kind::Mine, None)]);
}

#[test]
fn the_channel_excludes_and_the_position_in_the_order_apply_to_the_preview() {
    let cwr = channel_with(vec![
        rule("first", 0, Some("Show")),
        rule("second", 1, Some("Show - 01")),
    ]);
    let items = recorded(&["Show - 01", "Show - 01 [Batch]"], 1_000);
    let second = cwr.rules[1].to_input();

    // In second place the earlier rule takes the item; the batch is excluded.
    let p = run(&cwr, edit_of("second", &second), &items);
    assert_eq!(
        rows(&p),
        [
            row("Show - 01", Kind::Earlier, None),
            row("Show - 01 [Batch]", Kind::Excluded, None),
        ]
    );
    let excluded = p.items.iter().find(|i| i.kind == Kind::Excluded).unwrap();
    assert_eq!(excluded.excluded_by.as_deref(), Some("[Batch]"));
    assert_eq!(excluded.save_path, None);

    // Moved to the front, the rule takes the plain item itself.
    let moved = run(
        &cwr,
        Edit {
            position: Some(0),
            ..edit_of("second", &second)
        },
        &items,
    );
    assert_eq!(
        rows(&moved),
        [
            row("Show - 01", Kind::Mine, None),
            row("Show - 01 [Batch]", Kind::Excluded, None),
        ]
    );
}

#[test]
fn a_rule_not_saved_yet_previews_as_the_last_rule_and_an_off_one_as_collecting() {
    let items = recorded(&["Show - 01", "New - 01"], 1_000);
    let cwr = channel_with(vec![rule("first", 0, Some("Show"))]);

    let new = run(&cwr, new_rule(&input("New", "New/Season 01")), &items);
    assert_eq!(rows(&new), [row("New - 01", Kind::Mine, None)]);
    assert_eq!(
        new.items[0].save_path.as_deref(),
        Some(Path::new("/media/New/Season 01"))
    );

    // A rule waiting for its title matches nothing.
    let waiting = RuleInput {
        r#match: None,
        ..input("", "x")
    };
    assert_eq!(rows(&run(&cwr, new_rule(&waiting), &items)), []);

    // Archived or paused, a rule is judged as if turned on after everything
    // recorded: what history holds that it has not taken came while it was
    // off, so it would leave that to the user.
    for state in [RuleState::Archived, RuleState::Paused] {
        let mut off = rule("first", 0, Some("Show"));
        off.state = state;
        let edited = off.to_input();
        let p = run(&channel_with(vec![off]), edit_of("first", &edited), &items);
        assert_eq!(
            rows(&p),
            [row("Show - 01", Kind::Past, Some(PastCause::Resumed))],
            "{state:?}"
        );
        assert_eq!(p.counts.past, 1);
    }
}

#[test]
fn a_paused_rule_matches_nothing_and_does_not_shadow_a_later_one() {
    let items = recorded(&["[SubsPlease] Work - 01", "[SubsPlease] Work - 02"], 1_000);
    let later = rule("later", 1, Some("Work"));
    let active = subscribed(rule("sub", 0, Some("Work")), NOW, None);
    let mut paused = active.clone();
    paused.state = RuleState::Paused;
    let edited_later = input("Work", "x");
    let edited_sub = input("Work", "Work/Season 01");

    // While the subscription collects it takes both items before the later rule.
    let cwr = channel_with(vec![active, later.clone()]);
    let view = run(&cwr, edit_of("later", &edited_later), &items);
    assert_eq!(view.counts.earlier, 2);

    let cwr = channel_with(vec![paused, later]);
    let view = run(&cwr, edit_of("later", &edited_later), &items);
    assert_eq!((view.counts.mine, view.counts.earlier), (2, 0));
    // The paused rule's own preview shows what it would do once on: both items
    // were recorded before the subscription, so they are past.
    let view = run(&cwr, edit_of("sub", &edited_sub), &items);
    assert_eq!(view.counts.past, 2);
}

#[test]
fn what_was_recorded_before_a_subscription_is_past_until_the_rule_received_it() {
    let work = |n: u8| format!("[SubsPlease] Work - {n:02} (1080p) [ABCD123{n}].mkv");
    let cwr = channel_with(vec![subscribed(rule("sub", 0, Some("Work")), NOW, None)]);
    let edited = input("Work", "Work/Season 01");
    let mut items = vec![
        item(1, &work(1), 1_000, HistoryResult::NoMatch),
        item(2, &work(2), 1_000, HistoryResult::NoMatch),
        item(3, &work(3), 1_000, HistoryResult::NoMatch),
        item(
            4,
            "[SubsPlease] Another Show - 05 (1080p) [ABCD1237].mkv",
            1_000,
            HistoryResult::NoMatch,
        ),
    ];

    // The items recorded before the subscription are past, with the folder that
    // `받기` would save them into and the cause.
    let view = run(&cwr, edit_of("sub", &edited), &items);
    assert_eq!(
        rows(&view),
        [
            row(&work(1), Kind::Past, Some(PastCause::Subscribed)),
            row(&work(2), Kind::Past, Some(PastCause::Subscribed)),
            row(&work(3), Kind::Past, Some(PastCause::Subscribed)),
        ]
    );
    assert!(view
        .items
        .iter()
        .all(|i| i.stored_result == HistoryResult::NoMatch
            && i.save_path.as_deref() == Some(Path::new("/media/Work/Season 01"))));
    assert_eq!((view.counts.past, view.counts.mine), (3, 0));

    // One recorded after the subscription is the rule's own.
    items.push(item(5, &work(4), NOW + 1_000, HistoryResult::NoMatch));
    let view = run(&cwr, edit_of("sub", &edited), &items);
    assert_eq!(
        rows(&view)
            .into_iter()
            .filter(|r| r.1 == Kind::Mine)
            .collect::<Vec<_>>(),
        [row(&work(4), Kind::Mine, None)]
    );

    // The rule received a past item (the worker's `받기`): it stops being past.
    items[0].result = HistoryResult::Received;
    items[0].rule_id = Some("sub".into());
    items[0].result_at = NOW + 2_000;
    let view = run(&cwr, edit_of("sub", &edited), &items);
    assert_eq!(view.counts.past, 2);
    assert_eq!(view.counts.mine, 2);
}

#[test]
fn a_subscription_that_waits_for_its_title_takes_nothing_and_is_given_one_by_a_save() {
    let edited = input("New Work", "New Work/Season 01");
    let new_1 = "[SubsPlease] New Work - 01 (1080p) [AAAA1111].mkv";
    let new_2 = "[SubsPlease] New Work - 02 (1080p) [AAAA1112].mkv";
    let new_3 = "[SubsPlease] New Work - 03 (1080p) [AAAA1113].mkv";
    let batch = "[Group] New Work - 01-12 [Batch]";
    let waiting = subscribed(rule("sub", 0, None), NOW, None);
    let cwr = channel_with(vec![waiting]);
    // `channel()` excludes `[Batch]`; this channel does not.
    let cwr = ChannelWithRules {
        channel: channel_excluding(&[]),
        ..cwr
    };

    // It matches nothing: whatever is recorded, the preview of the stored rule
    // takes nothing.
    let nothing = RuleInput {
        r#match: None,
        ..edited.clone()
    };
    let items = [item(1, new_1, NOW + 1_000, HistoryResult::NoMatch)];
    let view = run(&cwr, edit_of("sub", &nothing), &items);
    assert_eq!((view.counts.mine, view.counts.past), (0, 0));

    // Given its title, what was recorded is past, as the cycle would not take it
    // either.
    let items = [
        item(1, new_1, NOW + 1_000, HistoryResult::NoMatch),
        item(2, new_2, NOW + 1_000, HistoryResult::NoMatch),
        item(3, batch, NOW + 1_000, HistoryResult::NoMatch),
    ];
    let mut expected = vec![
        row(new_1, Kind::Past, Some(PastCause::Titled)),
        row(new_2, Kind::Past, Some(PastCause::Titled)),
        row(batch, Kind::Past, Some(PastCause::Titled)),
    ];
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(rows(&run(&cwr, edit_of("sub", &edited), &items)), expected);

    // Once stored with the title (given at NOW + 60 s, by naming it or by saving
    // the phrase), it previews the same, and what is recorded after is its own.
    let named = subscribed(
        Rule {
            r#match: Some("New Work".into()),
            directory: "New Work/Season 01".into(),
            ..rule("sub", 0, None)
        },
        NOW,
        Some(NOW + 60_000),
    );
    let cwr = ChannelWithRules {
        channel: channel_excluding(&[]),
        rules: vec![named],
    };
    let items = [
        item(1, new_1, NOW + 1_000, HistoryResult::NoMatch),
        item(2, new_2, NOW + 1_000, HistoryResult::NoMatch),
        item(3, batch, NOW + 1_000, HistoryResult::NoMatch),
        item(4, new_3, NOW + 70_000, HistoryResult::NoMatch),
    ];
    let mut expected = vec![
        row(new_1, Kind::Past, Some(PastCause::Titled)),
        row(new_2, Kind::Past, Some(PastCause::Titled)),
        row(new_3, Kind::Mine, None),
        row(batch, Kind::Past, Some(PastCause::Titled)),
    ];
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    let view = run(&cwr, edit_of("sub", &edited), &items);
    assert_eq!(rows(&view), expected);
    assert_eq!(view.counts.past, 3);
}

#[test]
fn the_items_the_rule_takes_say_the_episode_they_are_received_as() {
    let cwr = channel_with(vec![rule("other", 0, Some("Other"))]);
    let mut items = recorded(
        &[
            "[SubsPlease] Show - 24 (1080p)",
            "[SubsPlease] Show - 25 (1080p)",
            "[SubsPlease] Show (01-12) (1080p) [Batch]",
            "[SubsPlease] Other - 03 (1080p)",
        ],
        1_000,
    );
    // The batch is excluded by `channel()`; this channel has no excludes.
    let cwr = ChannelWithRules {
        channel: channel_excluding(&[]),
        ..cwr
    };
    let named = |items: &[HistoryItem], episode: i64| {
        let edited = RuleInput {
            episode,
            ..input("SubsPlease", "Show/Season 02")
        };
        let p = run(&cwr, new_rule(&edited), items);
        let mut named: Vec<_> = p
            .items
            .iter()
            .map(|i| (i.title.clone(), i.release, i.episode_name.clone()))
            .collect();
        named.sort();
        (named, p.earliest_receivable)
    };
    let row = |title: &str, release: Option<u32>, name: Option<&str>| {
        (title.to_owned(), release, name.map(str::to_owned))
    };

    let (plain, earliest) = named(&items, 1);
    // An item another rule takes, or a batch, names no episode of this rule.
    assert_eq!(
        plain,
        [
            row("[SubsPlease] Other - 03 (1080p)", None, None),
            row("[SubsPlease] Show (01-12) (1080p) [Batch]", None, None),
            row("[SubsPlease] Show - 24 (1080p)", Some(24), Some("S02E24")),
            row("[SubsPlease] Show - 25 (1080p)", Some(25), Some("S02E25")),
        ]
    );
    // Both are items no rule received, so the earliest one may be asked for.
    assert_eq!(earliest, Some(24));
    let (converted, _) = named(&items, -12);
    assert_eq!(
        converted[2..],
        [
            row("[SubsPlease] Show - 24 (1080p)", Some(24), Some("S02E12")),
            row("[SubsPlease] Show - 25 (1080p)", Some(25), Some("S02E13")),
        ]
    );

    // An item a rule received is not one to ask for again.
    items[0].result = HistoryResult::Received;
    assert_eq!(named(&items, 1).1, Some(25));
}

#[test]
fn titles_with_the_secret_mask_are_flagged() {
    let cwr = channel_with(vec![rule("first", 0, Some("Show"))]);
    let items = recorded(&["Show - 01", "Show *** search", "Other *** one"], 1_000);
    let edited = cwr.rules[0].to_input();
    let p = run(&cwr, edit_of("first", &edited), &items);
    // Counted over every judged item, listed or not.
    assert_eq!(p.masked_total, 2);
    for listed in &p.items {
        assert_eq!(
            listed.masked,
            listed.title.contains("***"),
            "{}",
            listed.title
        );
    }
    assert_eq!(p.items.len(), 2);
}

#[test]
fn a_long_preview_lists_the_newest_matches_up_to_the_limit_and_counts_all_of_them() {
    let cwr = channel_with(vec![rule("first", 0, Some("Show"))]);
    // Newest first, as the caller reads them.
    let titles: Vec<String> = (0..130).rev().map(|n| format!("Show - {n:03}")).collect();
    let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
    let items = recorded(&refs, 1_000);
    let edited = cwr.rules[0].to_input();
    let at = |limit| {
        preview(
            Path::new("/media"),
            &cwr,
            edit_of("first", &edited),
            &items,
            limit,
        )
        .unwrap()
    };

    let p = at(100);
    assert_eq!(p.counts.mine, 130);
    assert_eq!(p.items.len(), 100);
    assert_eq!(p.items[0].title, "Show - 129");
    assert!(p.truncated);
    // The limit is the caller's: with room for all of them nothing is cut off.
    let all = at(130);
    assert_eq!(all.items.len(), 130);
    assert!(!all.truncated);
    // Asking for none lists none, and still counts.
    let none = at(0);
    assert!(none.items.is_empty() && none.truncated);
    assert_eq!(none.counts.mine, 130);
}

#[test]
fn a_rule_saving_straight_under_the_collect_folder_keeps_its_trailing_slash_in_the_preview_the_plan_and_a_retry(
) {
    // A channel's base folder with nothing after it became the collect folder
    // with a directory ending in `/` (or an empty one): the text the cycle gave
    // Transmission stays what the preview shows and a retry uses.
    let items = recorded(&["Show - 01"], 10);
    for (directory, saved) in [("other/", "/media/other/"), ("", "/media/")] {
        let stored = Rule {
            directory: directory.into(),
            ..rule("r", 0, Some("Show"))
        };
        let cwr = channel_with(vec![stored.clone()]);

        let listed = run(&cwr, edit_of("r", &stored.to_input()), &items);
        assert_eq!(
            listed.items[0].save_path.as_deref().and_then(Path::to_str),
            Some(saved),
            "preview of {directory:?}"
        );
        let plan = ChannelPlan::new(cwr, Path::new("/media"));
        let Judgement::Selected { save_path, .. } = plan.judge("Show - 01") else {
            panic!("the rule takes the item");
        };
        assert_eq!(save_path.to_str(), Some(saved), "plan of {directory:?}");
        let (retry, _) = crate::plan::rule_destination(Path::new("/media"), &stored);
        assert_eq!(retry.to_str(), Some(saved), "retry of {directory:?}");
    }
}

#[test]
fn the_preview_agrees_with_the_plan_for_every_recorded_title() {
    // The preview and the worker judge through `ChannelPlan`. This pins that the
    // preview's per-item answer is that plan's answer for the same items and
    // settings, including archived rules, regexes and excludes.
    let rule_of = |id: &str, position: i64, input: RuleInput| Rule {
        r#match: input.r#match,
        regex: input.regex,
        case_insensitive: input.case_insensitive,
        directory: input.directory,
        episode: input.episode,
        state: input.state,
        ..rule(id, position, None)
    };
    let rules = vec![
        rule_of(
            "doll",
            0,
            RuleInput {
                r#match: Some("Sono Bisque Doll".into()),
                case_insensitive: true,
                directory: "Doll/Season 02".into(),
                episode: -12,
                ..RuleInput::default()
            },
        ),
        rule_of(
            "regex",
            1,
            RuleInput {
                r#match: Some(r"^\[SubsPlease\] (Slime|Lara) - \d+".into()),
                regex: true,
                directory: "Regex".into(),
                ..RuleInput::default()
            },
        ),
        rule_of(
            "archived",
            2,
            RuleInput {
                r#match: Some("Archived Show".into()),
                directory: "Archived".into(),
                state: RuleState::Archived,
                ..RuleInput::default()
            },
        ),
        rule_of(
            "waiting",
            3,
            RuleInput {
                r#match: None,
                directory: "Waiting".into(),
                ..RuleInput::default()
            },
        ),
        rule_of(
            "catch",
            4,
            RuleInput {
                r#match: Some("[SubsPlease]".into()),
                directory: "Catch all".into(),
                ..RuleInput::default()
            },
        ),
    ];
    let cwr = ChannelWithRules {
        channel: channel_excluding(&["[Batch]", "(720p)"]),
        rules,
    };
    let titles = [
        "[SubsPlease] Sono Bisque Doll - 13 (1080p)",
        "sono bisque doll - 14",
        "[SubsPlease] Slime - 62 (1080p)",
        "[SubsPlease] Lara - 03 (720p)",
        "[SubsPlease] Sono Bisque Doll - 01~12 [Batch]",
        "[SubsPlease] Archived Show - 01",
        "[Erai-raws] Unrelated - 05",
        "Slime - 62",
        "",
        // The sample feed of the cycle tests, as the worker's channel recorded it.
        "[SubsPlease] Sayonara Lara - 03 (1080p) [AAAA0001].mkv",
        "[SubsPlease] Sayonara Lara - 03 (720p) [AAAA0002].mkv",
        "[SubsPlease] Sono Bisque Doll - 13 (1080p) [AAAA0003].mkv",
        "[SubsPlease] Tensei Shitara Slime Datta Ken - 62 (1080p) [AAAA0006].mkv",
        "[SubsPlease] Sono Bisque Doll - 01~12 [Batch] (1080p).mkv",
        "[SubsPlease] Unrelated Show - 05 (1080p) [AAAA0009].mkv",
    ];
    let items = recorded(&titles, 1_000);

    // Edit the last rule (the catch-all) without changing anything, and compare
    // its preview with the plan's answer for each item.
    let last = &cwr.rules[4];
    let collect = Path::new("/media");
    let p = preview(
        collect,
        &cwr,
        edit_of("catch", &last.to_input()),
        &items,
        100,
    )
    .unwrap();
    let plan = ChannelPlan::new(cwr.clone(), collect);
    let mut expected_mine = 0;
    let mut expected_earlier = 0;
    for item in &items {
        let evaluation = plan.evaluate(&item.title);
        let listed = p.items.iter().find(|i| i.id == item.id);
        match (&evaluation.judgement, listed) {
            (
                Judgement::Selected {
                    rule_id, save_path, ..
                },
                Some(listed),
            ) if rule_id == "catch" => {
                assert_eq!(listed.kind, Kind::Mine, "{}", item.title);
                assert_eq!(listed.save_path.as_deref(), Some(save_path.as_path()));
                expected_mine += 1;
            }
            (
                Judgement::Selected {
                    rule_id, save_path, ..
                },
                Some(listed),
            ) => {
                assert_eq!(listed.kind, Kind::Earlier, "{}", item.title);
                assert_eq!(listed.taken_by.as_ref().unwrap().rule_id, *rule_id);
                assert_eq!(listed.save_path.as_deref(), Some(save_path.as_path()));
                assert!(evaluation.overlapping.contains(&"catch".to_owned()));
                expected_earlier += 1;
            }
            (Judgement::Selected { .. }, None) => {
                assert!(
                    !evaluation.overlapping.contains(&"catch".to_owned()),
                    "{}",
                    item.title
                );
            }
            (Judgement::Excluded, Some(listed)) => {
                assert_eq!(listed.kind, Kind::Excluded, "{}", item.title)
            }
            (Judgement::Excluded, None) | (Judgement::NoMatch, None) => {}
            (Judgement::NoMatch, Some(listed)) => {
                panic!(
                    "{} listed as {:?} but nothing matches",
                    item.title, listed.kind
                )
            }
        }
    }
    assert_eq!(p.counts.mine, expected_mine);
    assert_eq!(p.counts.earlier, expected_earlier);
    assert!(expected_mine > 0 && expected_earlier > 0);
}
