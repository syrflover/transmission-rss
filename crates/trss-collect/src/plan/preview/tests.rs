use super::*;
use crate::store::channels::{Channel, Subscription, SubtitleMode};

fn channel() -> Channel {
    Channel {
        id: "c1".into(),
        position: 0,
        version: 1,
        url: "https://feed.test/rss".into(),
        excludes: vec!["[Batch]".into()],
        secret_query: Vec::new(),
        past_search: None,
        name: None,
    }
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

fn kinds(preview: &Preview) -> Vec<(i64, Kind)> {
    preview.items.iter().map(|i| (i.id, i.kind)).collect()
}

#[test]
fn a_new_rule_goes_last_unless_a_position_says_otherwise() {
    let cwr = channel_with(vec![rule("first", 0, Some("Show"))]);
    let items = [item(1, "Show - 01", 10, HistoryResult::NoMatch)];
    let edited = input("Show", "New/Season 01");

    // Last: the first rule takes the item.
    let last = run(
        &cwr,
        Edit {
            id: None,
            input: &edited,
            position: None,
        },
        &items,
    );
    assert_eq!(kinds(&last), [(1, Kind::Earlier)]);
    assert_eq!(
        last.items[0].taken_by,
        Some(TakenBy {
            rule_id: "first".into(),
            r#match: Some("Show".into())
        })
    );

    // First: the new rule takes it.
    let first = run(
        &cwr,
        Edit {
            id: None,
            input: &edited,
            position: Some(0),
        },
        &items,
    );
    assert_eq!(kinds(&first), [(1, Kind::Mine)]);
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
        Edit {
            id: Some("nope"),
            input: &edited,
            position: None,
        },
        &[],
        100,
    )
    .unwrap_err();
    assert_eq!(err, RuleNotFound("nope".into()));
}

#[test]
fn a_new_rule_is_never_past_but_a_paused_one_is_previewed_as_resumed_after_everything_recorded() {
    let items = [item(1, "Show - 01", 10, HistoryResult::NoMatch)];
    let edited = input("Show", "x");

    let new = run(
        &channel_with(Vec::new()),
        Edit {
            id: None,
            input: &edited,
            position: None,
        },
        &items,
    );
    assert_eq!(kinds(&new), [(1, Kind::Mine)]);

    let mut paused = rule("paused", 0, Some("Show"));
    paused.state = RuleState::Paused;
    let cwr = channel_with(vec![paused]);
    let resumed = run(
        &cwr,
        Edit {
            id: Some("paused"),
            input: &edited,
            position: None,
        },
        &items,
    );
    assert_eq!(kinds(&resumed), [(1, Kind::Past)]);
    assert_eq!(resumed.items[0].past_cause, Some(PastCause::Resumed));
    assert_eq!(resumed.counts.past, 1);
}

#[test]
fn a_title_given_to_a_waiting_subscription_leaves_what_was_recorded_to_the_user() {
    let mut waiting = rule("sub", 0, None);
    waiting.subscription = Some(Subscription {
        anissia_anime_no: 7,
        subtitles: SubtitleMode::Undecided,
        creator: None,
        season_id: None,
        season_blocked: None,
        subscribed_at: 5,
        titled_at: None,
    });
    let cwr = channel_with(vec![waiting]);
    let items = [item(1, "Show - 01", 10, HistoryResult::NoMatch)];
    let edited = input("Show", "x");

    let given = run(
        &cwr,
        Edit {
            id: Some("sub"),
            input: &edited,
            position: None,
        },
        &items,
    );
    assert_eq!(kinds(&given), [(1, Kind::Past)]);
    assert_eq!(given.items[0].past_cause, Some(PastCause::Titled));
}

#[test]
fn an_excluded_item_is_listed_when_the_edited_rule_would_have_taken_it_without_the_exclude() {
    let cwr = channel_with(vec![rule("first", 0, Some("Show"))]);
    let items = [
        item(1, "Show - 01~12 [Batch]", 10, HistoryResult::Excluded),
        item(2, "Other - 01 [Batch]", 11, HistoryResult::Excluded),
    ];
    let edited = input("Show", "x");
    let p = run(
        &cwr,
        Edit {
            id: Some("first"),
            input: &edited,
            position: None,
        },
        &items,
    );
    assert_eq!(kinds(&p), [(1, Kind::Excluded)]);
    assert_eq!(p.items[0].excluded_by.as_deref(), Some("[Batch]"));
    assert_eq!(p.items[0].save_path, None);
    assert_eq!((p.counts.excluded, p.counts.unmatched), (1, 1));
}

#[test]
fn rules_that_also_match_a_taken_title_overlap() {
    let cwr = channel_with(vec![
        rule("first", 0, Some("Show")),
        rule("second", 1, Some("Show - 0")),
        rule("third", 2, Some("Other")),
    ]);
    let plan = ChannelPlan::new(cwr, Path::new("/media"));
    let overlap = overlapping_rules(&plan, ["Show - 01", "Other - 02", "Nothing"]);
    assert_eq!(overlap, HashSet::from(["second".to_owned()]));
}
