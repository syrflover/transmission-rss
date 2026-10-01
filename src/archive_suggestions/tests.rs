use super::*;
use crate::store::status::read_day;
use crate::{
    schedule::calendar::date_text,
    store::channels::{Channel, Subscription, SubtitleMode},
};

/// 2026-10-01 12:00 in Seoul.
const NOW: Millis = 1_790_780_400_000 + 12 * 60 * 60 * 1000;
const WEEK: Millis = 7 * DAY_MS;

fn channel(id: &str) -> Channel {
    Channel {
        id: id.into(),
        position: 0,
        version: 1,
        url: "https://feed.test/rss".into(),
        excludes: vec!["[Batch]".into()],
        secret_query: Vec::new(),
        past_search: None,
        name: None,
    }
}

fn rule(id: &str, phrase: Option<&str>) -> Rule {
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
        subscription: None,
        resumed_at: None,
    }
}

fn subscribed(mut rule: Rule, anime_no: i64, subscribed_at: Millis) -> Rule {
    rule.subscription = Some(Subscription {
        anissia_anime_no: anime_no,
        subtitles: SubtitleMode::None,
        creator: None,
        season_id: None,
        season_blocked: None,
        subscribed_at,
        titled_at: None,
    });
    rule
}

fn anime(no: i64, end_date: Option<&str>) -> Anime {
    Anime {
        anime_no: no,
        subject: "작품".into(),
        original_subject: None,
        week: 3,
        air_time: Some("22:30".into()),
        start_date: Some("2026-07-01".into()),
        end_date: end_date.map(str::to_owned),
        status: "ON".into(),
        fetched_at: NOW,
    }
}

/// The inputs of a test, with the defaults of a rule that started long ago.
#[derive(Default)]
struct World {
    rules: Vec<Rule>,
    animes: HashMap<i64, Anime>,
    unlisted: HashSet<i64>,
    last_received: HashMap<String, Millis>,
    started: HashMap<String, Millis>,
    first_read: HashMap<String, Millis>,
    read_floors: HashMap<String, i64>,
    kept: HashSet<(String, String)>,
}

impl World {
    fn with(rules: Vec<Rule>) -> World {
        let mut world = World {
            rules,
            ..World::default()
        };
        // The channel was read long before the rules' quiet stretch.
        world.first_read.insert("c1".into(), NOW - 52 * WEEK);
        // And read every day since, the newest of the 28 days being today.
        world.read_floors.insert("c1".into(), read_day(NOW) - 27);
        world
    }

    fn received(mut self, rule: &str, at: Millis) -> World {
        self.last_received.insert(rule.into(), at);
        self
    }

    fn channels(&self) -> Vec<ChannelWithRules> {
        vec![ChannelWithRules {
            channel: channel("c1"),
            rules: self.rules.clone(),
        }]
    }

    fn facts<'a>(&'a self, channels: &'a [ChannelWithRules]) -> Facts<'a> {
        Facts {
            now: NOW,
            channels,
            animes: &self.animes,
            unlisted: &self.unlisted,
            last_received: &self.last_received,
            started: &self.started,
            first_read: &self.first_read,
            read_floors: &self.read_floors,
            kept: &self.kept,
        }
    }

    /// The suggestions when the channel's last 4 weeks hold `titles`.
    fn suggest(&self, titles: &[&str]) -> Vec<ArchiveSuggestion> {
        let channels = self.channels();
        let facts = self.facts(&channels);
        let titles: Vec<String> = titles.iter().map(|t| (*t).to_owned()).collect();
        let recent = HashMap::from([(
            "c1".to_owned(),
            recent_matches(&channels[0], &titles, false),
        )]);
        facts.suggestions(&recent)
    }

    fn quiet_rules(&self, titles: &[&str]) -> Vec<String> {
        self.suggest(titles)
            .into_iter()
            .filter(|s| s.grounds.iter().any(|g| matches!(g, Ground::Quiet { .. })))
            .map(|s| s.rule_id)
            .collect()
    }
}

const WORK_5: &str = "[SubsPlease] Work - 05 (1080p) [ABCD1234].mkv";
const OTHER_5: &str = "[SubsPlease] Another Show - 05 (1080p) [ABCD1235].mkv";

// ---------------------------------------------------------------------------
// The anime has ended
// ---------------------------------------------------------------------------

#[test]
fn a_subscription_whose_end_date_has_passed_is_suggested_as_ended() {
    let today = day_of(NOW);
    let yesterday = date_text(today - 1);
    let mut world = World::with(vec![subscribed(
        rule("r1", Some("Work")),
        7,
        NOW - 60 * DAY_MS,
    )]);
    world.animes.insert(7, anime(7, Some(&yesterday)));
    // Something just came, so it is not quiet: this ground alone.
    world.last_received.insert("r1".into(), NOW - DAY_MS);

    let found = world.suggest(&[]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].rule_id, "r1");
    assert_eq!(
        found[0].grounds,
        [Ground::Ended {
            anime_no: 7,
            over: Over::EndDate(yesterday.clone())
        }]
    );
    assert_eq!(found[0].grounds[0].key(), format!("ended:7:{yesterday}"));
}

#[test]
fn an_end_date_that_is_today_or_ahead_has_not_ended_the_anime() {
    let today = day_of(NOW);
    for end in [date_text(today), date_text(today + 10)] {
        let mut world = World::with(vec![subscribed(
            rule("r1", Some("Work")),
            7,
            NOW - 60 * DAY_MS,
        )]);
        world.animes.insert(7, anime(7, Some(&end)));
        world.last_received.insert("r1".into(), NOW - DAY_MS);
        assert_eq!(world.suggest(&[]), [], "ends {end}");
    }
}

#[test]
fn a_month_only_end_date_reaches_the_end_of_its_month() {
    let mut world = World::with(vec![subscribed(
        rule("r1", Some("Work")),
        7,
        NOW - 60 * DAY_MS,
    )]);
    world.last_received.insert("r1".into(), NOW - DAY_MS);
    // October is the month of NOW: not over. September is.
    world.animes.insert(7, anime(7, Some("2026-10")));
    assert_eq!(world.suggest(&[]), []);
    world.animes.insert(7, anime(7, Some("2026-09")));
    assert_eq!(world.suggest(&[]).len(), 1);
}

#[test]
fn an_anime_without_an_end_date_ends_when_anissia_no_longer_lists_it() {
    let mut world = World::with(vec![subscribed(
        rule("r1", Some("Work")),
        7,
        NOW - 60 * DAY_MS,
    )]);
    world.animes.insert(7, anime(7, None));
    world.last_received.insert("r1".into(), NOW - DAY_MS);
    // Listed, or Anissia could not be asked: nothing is found out.
    assert_eq!(world.suggest(&[]), []);

    world.unlisted.insert(7);
    let found = world.suggest(&[]);
    assert_eq!(
        found[0].grounds,
        [Ground::Ended {
            anime_no: 7,
            over: Over::Unlisted
        }]
    );
    assert_eq!(found[0].grounds[0].key(), "unlisted:7");
}

#[test]
fn an_end_date_ahead_wins_over_a_missing_listing() {
    let ahead = date_text(day_of(NOW) + 30);
    let mut world = World::with(vec![subscribed(
        rule("r1", Some("Work")),
        7,
        NOW - 60 * DAY_MS,
    )]);
    world.animes.insert(7, anime(7, Some(&ahead)));
    world.unlisted.insert(7);
    world.last_received.insert("r1".into(), NOW - DAY_MS);
    assert_eq!(world.suggest(&[]), []);
}

#[test]
fn a_rule_that_is_no_subscription_or_has_no_snapshot_has_no_ending() {
    let mut world = World::with(vec![
        rule("plain", Some("Plain")),
        subscribed(rule("lost", Some("Lost")), 8, NOW - 60 * DAY_MS),
    ]);
    world.animes.insert(7, anime(7, Some("2020-01-01")));
    world.unlisted.insert(7);
    world.last_received.insert("plain".into(), NOW - DAY_MS);
    world.last_received.insert("lost".into(), NOW - DAY_MS);
    assert_eq!(world.suggest(&[]), []);
}

// ---------------------------------------------------------------------------
// Nothing new for 4 weeks
// ---------------------------------------------------------------------------

#[test]
fn three_weeks_after_the_last_receive_is_not_quiet_and_four_weeks_is() {
    let world = World::with(vec![rule("r1", Some("Work"))]);

    let three = world.received_clone("r1", NOW - 3 * WEEK);
    assert_eq!(three.quiet_rules(&[]), Vec::<String>::new());

    let just_short = world.received_clone("r1", NOW - QUIET + 1);
    assert_eq!(just_short.quiet_rules(&[]), Vec::<String>::new());

    let exactly = world.received_clone("r1", NOW - QUIET);
    assert_eq!(exactly.quiet_rules(&[]), ["r1"]);

    let over = world.received_clone("r1", NOW - 5 * WEEK);
    assert_eq!(over.quiet_rules(&[]), ["r1"]);
}

impl World {
    fn received_clone(&self, rule: &str, at: Millis) -> World {
        World {
            rules: self.rules.clone(),
            animes: self.animes.clone(),
            unlisted: self.unlisted.clone(),
            last_received: HashMap::from([(rule.to_owned(), at)]),
            started: self.started.clone(),
            first_read: self.first_read.clone(),
            read_floors: self.read_floors.clone(),
            kept: self.kept.clone(),
        }
    }
}

#[test]
fn the_quiet_ground_says_where_the_weeks_count_from() {
    let world = World::with(vec![rule("r1", Some("Work"))]).received("r1", NOW - 6 * WEEK);
    let found = world.suggest(&[]);
    assert_eq!(
        found[0].grounds,
        [Ground::Quiet {
            since: NOW - 6 * WEEK,
            last_received: Some(NOW - 6 * WEEK)
        }]
    );
    assert_eq!(
        found[0].grounds[0].key(),
        format!("quiet:{}", NOW - 6 * WEEK)
    );
}

#[test]
fn a_recent_item_that_matches_the_rule_is_a_new_item_whatever_became_of_it() {
    let world = World::with(vec![rule("r1", Some("Work"))]).received("r1", NOW - 6 * WEEK);
    assert_eq!(world.quiet_rules(&[WORK_5]), Vec::<String>::new());
    // Another show's item is not this rule's.
    assert_eq!(world.quiet_rules(&[OTHER_5]), ["r1"]);
}

#[test]
fn an_item_the_channel_excludes_is_not_a_new_item_of_the_rule() {
    let world = World::with(vec![rule("r1", Some("Work"))]).received("r1", NOW - 6 * WEEK);
    assert_eq!(world.quiet_rules(&["[Group] Work - 01-12 [Batch]"]), ["r1"]);
}

#[test]
fn an_item_an_earlier_rule_took_still_counts_for_a_later_rule_it_matches() {
    let mut early = rule("early", Some("Work"));
    early.position = 0;
    let mut late = rule("late", Some("Work - 05"));
    late.position = 1;
    let world = World::with(vec![early, late])
        .received("early", NOW - 6 * WEEK)
        .received("late", NOW - 6 * WEEK);
    // Both match the title, only the first would take it.
    assert_eq!(world.quiet_rules(&[WORK_5]), Vec::<String>::new());
}

#[test]
fn an_item_matching_an_archived_rule_ahead_still_reaches_the_rules_after_it() {
    let mut archived = rule("old", Some("Work"));
    archived.state = RuleState::Archived;
    let world =
        World::with(vec![archived, rule("r1", Some("Work - 05"))]).received("r1", NOW - 6 * WEEK);
    assert_eq!(world.quiet_rules(&[WORK_5]), Vec::<String>::new());
}

#[test]
fn a_paused_rule_is_suggested_on_the_same_ground() {
    let mut paused = rule("r1", Some("Work"));
    paused.state = RuleState::Paused;
    let world = World::with(vec![paused]).received("r1", NOW - 6 * WEEK);
    assert_eq!(world.quiet_rules(&[]), ["r1"]);
    // What came while it was off is a new item too: the work is still coming.
    assert_eq!(world.quiet_rules(&[WORK_5]), Vec::<String>::new());
}

#[test]
fn an_archived_rule_and_a_subscription_waiting_for_its_title_are_never_suggested() {
    let mut archived = rule("archived", Some("Old"));
    archived.state = RuleState::Archived;
    let waiting = subscribed(rule("waiting", None), 9, NOW - 60 * DAY_MS);
    let mut world = World::with(vec![archived, waiting]);
    world.animes.insert(9, anime(9, Some("2020-01-01")));
    world.unlisted.insert(9);
    assert_eq!(world.suggest(&[]), []);
    assert!(world.facts(&world.channels()).channels_to_read().is_empty());
}

#[test]
fn a_rule_that_never_received_counts_from_when_it_started_collecting() {
    // Stamped by the database when the rule was made.
    let mut world = World::with(vec![rule("r1", Some("Work"))]);
    world.started.insert("r1".into(), NOW - 3 * WEEK);
    assert_eq!(world.quiet_rules(&[]), Vec::<String>::new());
    world.started.insert("r1".into(), NOW - 5 * WEEK);
    let found = world.suggest(&[]);
    assert_eq!(
        found[0].grounds,
        [Ground::Quiet {
            since: NOW - 5 * WEEK,
            last_received: None
        }]
    );
}

#[test]
fn a_subscription_counts_from_when_it_was_subscribed_or_given_its_title() {
    let mut late = subscribed(rule("r1", Some("Work")), 7, NOW - 10 * WEEK);
    late.subscription.as_mut().unwrap().titled_at = Some(NOW - 2 * WEEK);
    let world = World::with(vec![late]);
    assert_eq!(world.quiet_rules(&[]), Vec::<String>::new());

    let world = World::with(vec![subscribed(
        rule("r1", Some("Work")),
        7,
        NOW - 10 * WEEK,
    )]);
    assert_eq!(world.quiet_rules(&[]), ["r1"]);
}

#[test]
fn turning_a_rule_back_on_starts_the_weeks_over() {
    let mut resumed = rule("r1", Some("Work"));
    resumed.resumed_at = Some(NOW - 2 * WEEK);
    let world = World::with(vec![resumed]).received("r1", NOW - 20 * WEEK);
    assert_eq!(world.quiet_rules(&[]), Vec::<String>::new());
}

#[test]
fn a_rule_from_before_the_stamp_counts_from_when_its_channel_was_first_read() {
    let mut world = World::with(vec![rule("r1", Some("Work"))]);
    world.first_read.insert("c1".into(), NOW - 3 * WEEK);
    assert_eq!(world.quiet_rules(&[]), Vec::<String>::new());
    world.first_read.insert("c1".into(), NOW - 9 * WEEK);
    assert_eq!(world.quiet_rules(&[]), ["r1"]);
    // Nothing says when it started: no ground.
    world.first_read.clear();
    assert_eq!(world.quiet_rules(&[]), Vec::<String>::new());
}

#[test]
fn a_first_read_stamped_in_the_future_does_not_start_the_weeks() {
    let mut world = World::with(vec![rule("r1", Some("Work"))]);
    world.started.insert("r1".into(), NOW - 9 * WEEK);
    // The channel's first read has a time ahead of now (a clock that was ahead
    // then): the rule counts from when it started instead.
    world.first_read.insert("c1".into(), NOW + 20 * WEEK);
    assert_eq!(world.quiet_rules(&[]), ["r1"]);
}

#[test]
fn a_window_that_was_cut_short_or_a_broken_regex_gives_no_quiet_ground() {
    let mut broken = rule("broken", Some("(unclosed"));
    broken.regex = true;
    let world = World::with(vec![broken, rule("fine", Some("Work"))])
        .received("broken", NOW - 6 * WEEK)
        .received("fine", NOW - 6 * WEEK);
    assert_eq!(world.quiet_rules(&[]), ["fine"]);

    let channels = world.channels();
    let facts = world.facts(&channels);
    let cut = HashMap::from([("c1".to_owned(), recent_matches(&channels[0], &[], true))]);
    assert_eq!(facts.suggestions(&cut), []);
}

#[test]
fn a_channel_with_no_recent_reading_gives_no_quiet_ground() {
    let world = World::with(vec![rule("r1", Some("Work"))]).received("r1", NOW - 6 * WEEK);
    let channels = world.channels();
    let facts = world.facts(&channels);
    assert_eq!(facts.suggestions(&HashMap::new()), []);
    assert_eq!(facts.channels_to_read(), ["c1"]);
}

#[test]
fn only_the_channels_with_a_rule_quiet_by_the_clock_are_read() {
    let world = World::with(vec![rule("r1", Some("Work"))]).received("r1", NOW - 3 * WEEK);
    let channels = world.channels();
    assert!(world.facts(&channels).channels_to_read().is_empty());
}

#[test]
fn both_grounds_are_listed_with_the_end_of_the_anime_first() {
    let mut world = World::with(vec![subscribed(
        rule("r1", Some("Work")),
        7,
        NOW - 60 * DAY_MS,
    )])
    .received("r1", NOW - 6 * WEEK);
    world.animes.insert(7, anime(7, Some("2026-09-01")));
    let found = world.suggest(&[]);
    assert_eq!(found.len(), 1);
    assert!(matches!(found[0].grounds[0], Ground::Ended { .. }));
    assert!(matches!(found[0].grounds[1], Ground::Quiet { .. }));
}

// ---------------------------------------------------------------------------
// 수집 유지
// ---------------------------------------------------------------------------

#[test]
fn a_kept_ground_does_not_suggest_again_and_a_new_one_does() {
    let mut world = World::with(vec![subscribed(
        rule("r1", Some("Work")),
        7,
        NOW - 60 * DAY_MS,
    )])
    .received("r1", NOW - 6 * WEEK);
    world.animes.insert(7, anime(7, None));
    assert_eq!(world.suggest(&[]).len(), 1, "quiet");

    // Kept on the quiet stretch: gone, and nothing is read for it any more.
    world
        .kept
        .insert(("r1".into(), format!("quiet:{}", NOW - 6 * WEEK)));
    assert_eq!(world.suggest(&[]), []);
    assert!(world.facts(&world.channels()).channels_to_read().is_empty());

    // The anime ends: a new ground, so the rule is suggested again, and only on it.
    world.unlisted.insert(7);
    let found = world.suggest(&[]);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].grounds,
        [Ground::Ended {
            anime_no: 7,
            over: Over::Unlisted
        }]
    );

    // Kept on that too: gone for good, however long it stays so.
    world.kept.insert(("r1".into(), "unlisted:7".into()));
    assert_eq!(world.suggest(&[]), []);
}

#[test]
fn a_rule_that_receives_and_falls_quiet_again_is_a_new_ground() {
    let mut world = World::with(vec![rule("r1", Some("Work"))]).received("r1", NOW - 6 * WEEK);
    world
        .kept
        .insert(("r1".into(), format!("quiet:{}", NOW - 10 * WEEK)));
    // Kept on an earlier stretch; this one counts from a later receive.
    assert_eq!(world.quiet_rules(&[]), ["r1"]);
}

#[test]
fn the_keep_of_another_rule_changes_nothing() {
    let mut world = World::with(vec![rule("r1", Some("Work"))]).received("r1", NOW - 6 * WEEK);
    world
        .kept
        .insert(("other".into(), format!("quiet:{}", NOW - 6 * WEEK)));
    assert_eq!(world.quiet_rules(&[]), ["r1"]);
}

// ---------------------------------------------------------------------------
// Order
// ---------------------------------------------------------------------------

#[test]
fn suggestions_follow_the_channels_and_their_rules() {
    let mut a = rule("a", Some("A"));
    a.position = 0;
    let mut b = rule("b", Some("B"));
    b.position = 1;
    let world = World::with(vec![a, b])
        .received("a", NOW - 6 * WEEK)
        .received("b", NOW - 7 * WEEK);
    let ids: Vec<String> = world.suggest(&[]).into_iter().map(|s| s.rule_id).collect();
    assert_eq!(ids, ["a", "b"]);
}

#[test]
fn the_window_starts_four_weeks_back() {
    let world = World::with(vec![]);
    let channels = world.channels();
    assert_eq!(world.facts(&channels).window_start("c1"), NOW - 28 * DAY_MS);
}

// ---------------------------------------------------------------------------
// Only the weeks the channel was read count
// ---------------------------------------------------------------------------

#[test]
fn weeks_the_channel_could_not_be_read_are_not_quiet_weeks() {
    // The last item came 5 weeks ago; the feed could not be read for the last 2,
    // so the newest of its 28 read days is 2 weeks old and only 3 weeks of
    // reading follow the item.
    let mut world = World::with(vec![rule("r1", Some("Work"))]).received("r1", NOW - 5 * WEEK);
    world
        .read_floors
        .insert("c1".into(), read_day(NOW - 2 * WEEK) - 27);
    assert_eq!(world.quiet_rules(&[]), Vec::<String>::new());
    let channels = world.channels();
    assert!(
        world.facts(&channels).channels_to_read().is_empty(),
        "there is nothing to read for it either"
    );
}

#[test]
fn the_ground_appears_when_reading_has_made_up_the_four_weeks() {
    // The same, one week after the feed could be read again: the item is 6
    // weeks old, 3 weeks were read before the gap (to 3 weeks ago) and the 8
    // days since 1 week ago make the 28 read days.
    let mut world = World::with(vec![rule("r1", Some("Work"))]).received("r1", NOW - 6 * WEEK);
    world
        .read_floors
        .insert("c1".into(), read_day(NOW - 3 * WEEK) - 19);
    assert_eq!(world.quiet_rules(&[]), ["r1"]);
}

#[test]
fn a_channel_that_has_not_been_read_for_28_days_gives_no_quiet_ground() {
    let mut world = World::with(vec![rule("r1", Some("Work"))]).received("r1", NOW - 20 * WEEK);
    world.read_floors.clear();
    assert_eq!(world.quiet_rules(&[]), Vec::<String>::new());
    let channels = world.channels();
    assert!(world.facts(&channels).channels_to_read().is_empty());
}

#[test]
fn the_day_of_the_moment_itself_is_not_one_of_the_28() {
    let since = NOW - 10 * WEEK;
    let mut world = World::with(vec![rule("r1", Some("Work"))]).received("r1", since);
    // The newest 28 read days start on the very day of the moment: that day is
    // not after it, so only 27 days are.
    world.read_floors.insert("c1".into(), read_day(since));
    assert_eq!(world.quiet_rules(&[]), Vec::<String>::new());
    world.read_floors.insert("c1".into(), read_day(since) + 1);
    assert_eq!(world.quiet_rules(&[]), ["r1"]);
}

#[test]
fn the_clock_still_has_to_reach_four_weeks_when_the_days_are_there() {
    // 28 read days after the moment's own day, but an hour short of 4 weeks.
    let since = NOW - QUIET + 60 * 60 * 1000;
    let mut world = World::with(vec![rule("r1", Some("Work"))]).received("r1", since);
    world.read_floors.insert("c1".into(), read_day(since) + 1);
    assert_eq!(world.quiet_rules(&[]), Vec::<String>::new());
}

#[test]
fn the_window_reaches_back_to_the_first_of_the_read_days() {
    // The feed was read until 2 weeks ago: the window starts at the first of
    // its last 28 read days, 6 weeks back, not 4 weeks from now.
    let mut world = World::with(vec![rule("r1", Some("Work"))]);
    let floor = read_day(NOW - 2 * WEEK) - 27;
    world.read_floors.insert("c1".into(), floor);
    let channels = world.channels();
    let facts = world.facts(&channels);
    assert_eq!(facts.window_start("c1"), floor * DAY_MS - 1);
    assert!(facts.window_start("c1") < NOW - QUIET);
    // A channel with fewer read days has only the clock to go by.
    assert_eq!(facts.window_start("other"), NOW - QUIET);
}
