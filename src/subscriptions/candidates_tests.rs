use super::*;
use crate::store::channels::{Channel, Subscription, SubtitleMode};

const SINCE: Millis = 1_000;

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

fn rule(id: &str, phrase: Option<&str>, subscribed_at: Option<Millis>) -> Rule {
    Rule {
        id: id.into(),
        channel_id: "c1".into(),
        position: 0,
        version: 1,
        r#match: phrase.map(str::to_owned),
        regex: false,
        case_insensitive: false,
        directory: format!("{id}/Season 01"),
        episode: 1,
        episode_auto: false,
        state: RuleState::Active,
        subscription: subscribed_at.map(|at| Subscription {
            anissia_anime_no: 7,
            subtitles: SubtitleMode::None,
            creator: None,
            season_id: None,
            season_blocked: None,
            subscribed_at: at,
            titled_at: None,
        }),
        resumed_at: None,
    }
}

/// A subscription that has waited for its title since `SINCE`.
fn waiting() -> Rule {
    rule("waiting", None, Some(SINCE))
}

fn item(id: i64, title: &str, first_seen_at: Millis, result: HistoryResult) -> HistoryItem {
    HistoryItem {
        first_read: false,
        id,
        channel_id: "c1".into(),
        channel_label: "feed".into(),
        identity_key: format!("title:{id}"),
        title: title.into(),
        link: String::new(),
        first_seen_at,
        last_seen_at: first_seen_at,
        result,
        result_at: first_seen_at,
        rule_id: None,
        reason: None,
        torrent_hash: None,
    }
}

fn unmatched(id: i64, title: &str, at: Millis) -> HistoryItem {
    item(id, title, at, HistoryResult::NoMatch)
}

fn found(rules: Vec<Rule>, items: &[HistoryItem], rejected: &[&str]) -> Vec<TitleCandidate> {
    found_in(rules, items, false, rejected)
}

fn found_in(
    rules: Vec<Rule>,
    items: &[HistoryItem],
    truncated: bool,
    rejected: &[&str],
) -> Vec<TitleCandidate> {
    let channel = ChannelWithRules {
        channel: channel(),
        rules,
    };
    let rejected = rejected.iter().map(|k| (*k).to_owned()).collect();
    title_candidates(&channel, items, truncated, &rejected)
}

const NEW_1: &str = "[SubsPlease] New Work - 01 (1080p) [AAAA1111].mkv";
const NEW_2: &str = "[SubsPlease] New Work - 02 (1080p) [AAAA1112].mkv";

#[test]
fn a_new_work_is_one_candidate_however_many_episodes_appear() {
    let items = [unmatched(2, NEW_2, 3_000), unmatched(1, NEW_1, 2_000)];
    let candidates = found(vec![waiting()], &items, &[]);

    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!(candidate.work, "New Work");
    assert_eq!(candidate.key, "new work");
    assert_eq!(candidate.latest_title, NEW_2);
    assert_eq!(candidate.items, 2);
    assert_eq!(
        (candidate.first_seen_at, candidate.latest_seen_at),
        (2_000, 3_000)
    );
    assert_eq!(candidate.waiting, ["waiting"]);
}

#[test]
fn the_spellings_of_one_work_are_one_candidate() {
    let items = [
        unmatched(1, "[Erai-raws] new  work - 01 [1080p].mkv", 2_000),
        unmatched(2, NEW_2, 3_000),
    ];
    let candidates = found(vec![waiting()], &items, &[]);
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    // The work is written the way the newest item writes it.
    assert_eq!(candidates[0].work, "New Work");
}

#[test]
fn nothing_is_a_candidate_while_no_subscription_waits_for_a_title() {
    let items = [unmatched(1, NEW_1, 2_000)];
    // No subscription at all; one with its title; a plain rule without a phrase.
    assert!(found(vec![], &items, &[]).is_empty());
    assert!(found(vec![rule("s", Some("Other"), Some(SINCE))], &items, &[]).is_empty());
    assert!(found(vec![rule("plain", None, None)], &items, &[]).is_empty());
}

#[test]
fn a_work_seen_before_the_subscription_waited_is_not_new() {
    let items = [unmatched(1, NEW_1, SINCE - 1), unmatched(2, NEW_2, 3_000)];
    assert!(found(vec![waiting()], &items, &[]).is_empty());

    // The moment itself counts as while waiting.
    let items = [unmatched(1, NEW_1, SINCE)];
    assert_eq!(found(vec![waiting()], &items, &[]).len(), 1);

    // An excluded release seen earlier makes the work old too.
    let items = [
        item(
            1,
            "[SubsPlease] New Work - 01-12 [Batch]",
            SINCE - 1,
            HistoryResult::Excluded,
        ),
        unmatched(2, NEW_1, 3_000),
    ];
    assert!(found(vec![waiting()], &items, &[]).is_empty());
}

#[test]
fn a_work_only_an_exclude_caught_is_not_a_candidate() {
    let items = [item(
        1,
        "[SubsPlease] New Work - 01-12 [Batch]",
        3_000,
        HistoryResult::Excluded,
    )];
    assert!(found(vec![waiting()], &items, &[]).is_empty());
}

#[test]
fn a_work_another_rule_handles_is_not_a_candidate() {
    let items = [unmatched(1, NEW_1, 3_000)];
    // A rule matches it now (the item is recorded as unmatched from before).
    let matching = rule("other", Some("New Work"), None);
    assert!(found(vec![waiting(), matching], &items, &[]).is_empty());

    // The matching rule is paused or archived: it is still that rule's work.
    for state in [RuleState::Paused, RuleState::Archived] {
        let mut off = rule("other", Some("New Work"), None);
        off.state = state;
        assert!(
            found(vec![waiting(), off], &items, &[]).is_empty(),
            "{state:?}"
        );
    }

    // A rule received an item of the work, and the rule is gone since.
    for result in [
        HistoryResult::Received,
        HistoryResult::Duplicate,
        HistoryResult::AddFailed,
    ] {
        let items = [unmatched(1, NEW_1, 3_000), item(2, NEW_2, 3_100, result)];
        assert!(found(vec![waiting()], &items, &[]).is_empty(), "{result:?}");
    }

    // A rule for another work leaves it alone.
    let unrelated = rule("other", Some("Another Work"), None);
    assert_eq!(found(vec![waiting(), unrelated], &items, &[]).len(), 1);
}

#[test]
fn a_title_naming_the_waiting_subscription_itself_ends_the_candidate() {
    let items = [unmatched(1, NEW_1, 3_000)];
    let mut named = waiting();
    named.r#match = Some("New Work".into());
    assert!(found(vec![named], &items, &[]).is_empty());
}

#[test]
fn a_paused_or_archived_subscription_offers_nothing_and_a_resume_keeps_the_boundary() {
    let items = [unmatched(1, NEW_1, 3_000)];
    for state in [RuleState::Paused, RuleState::Archived] {
        let mut off = waiting();
        off.state = state;
        assert!(found(vec![off], &items, &[]).is_empty(), "{state:?}");
    }

    // Pausing and turning the subscription back on does not move the boundary: a
    // work first seen while it waited stays a candidate across the pause.
    for resumed_at in [2_000, 4_000] {
        let mut resumed = waiting();
        resumed.resumed_at = Some(resumed_at);
        assert_eq!(
            found(vec![resumed], &items, &[]).len(),
            1,
            "resumed at {resumed_at}"
        );
    }
    // A work first seen before the subscription began to wait is not new, however
    // late it was turned back on.
    let before = [unmatched(1, NEW_1, SINCE - 1)];
    let mut resumed = waiting();
    resumed.resumed_at = Some(4_000);
    assert!(found(vec![resumed], &before, &[]).is_empty());

    // Another subscription that waits since later does not make the work new
    // for it; one that waits since before the work does.
    let mut off = waiting();
    off.state = RuleState::Paused;
    let later = rule("other", None, Some(3_500));
    assert!(found(vec![off, later], &items, &[]).is_empty());
    let other = rule("other", None, Some(2_000));
    let candidates = found(vec![waiting(), other], &items, &[]);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].waiting, ["waiting", "other"]);
}

#[test]
fn a_rejected_work_is_not_offered_again() {
    let items = [
        unmatched(1, NEW_1, 3_000),
        unmatched(2, "[X] Second Work - 01", 3_100),
    ];
    let candidates = found(vec![waiting()], &items, &["new work"]);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].work, "Second Work");
    // A later episode of the rejected work stays rejected.
    let items = [unmatched(1, NEW_1, 3_000), unmatched(3, NEW_2, 9_000)];
    assert!(found(vec![waiting()], &items, &["new work"]).is_empty());
}

#[test]
fn candidates_come_newest_work_first_and_a_title_without_a_work_is_left_out() {
    let items = [
        unmatched(1, "[G] Older - 01", 2_000),
        unmatched(2, "[G] Newer - 01", 5_000),
        unmatched(3, "[G]", 6_000),
    ];
    let works: Vec<String> = found(vec![waiting()], &items, &[])
        .into_iter()
        .map(|c| c.work)
        .collect();
    assert_eq!(works, ["Newer", "Older"]);
}

#[test]
fn a_truncated_window_that_does_not_reach_the_boundary_offers_nothing() {
    // The newest items only: every work looks new, though a long-running one
    // may have been recorded long before the window starts.
    let items = [unmatched(2, NEW_2, 3_000), unmatched(1, NEW_1, SINCE)];
    assert_eq!(found_in(vec![waiting()], &items, false, &[]).len(), 1);
    assert!(found_in(vec![waiting()], &items, true, &[]).is_empty());

    // A window that reaches back past the boundary shows what was there before.
    let items = [
        unmatched(3, NEW_2, 3_000),
        unmatched(2, NEW_1, 2_000),
        unmatched(1, "[SubsPlease] Old Work - 05", SINCE - 1),
    ];
    let candidates = found_in(vec![waiting()], &items, true, &[]);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].work, "New Work");

    // A truncated window with no items has nothing to be wrong about.
    assert!(found_in(vec![waiting()], &[], true, &[]).is_empty());
}

#[test]
fn an_unknowable_first_sighting_is_logged_once_per_channel() {
    assert!(note_unknowable_once("log-once-a"));
    assert!(!note_unknowable_once("log-once-a"));
    assert!(note_unknowable_once("log-once-b"));
}
