//! What a subscription does at a channel's first read, through the cycle
//! ([`World::cycle`]), the history and the rule detail's preview. A channel
//! with no history (one the legacy YAML import made, for one) has no record to
//! tell the items its feed already held from the new ones, so a subscription
//! receives none of them; the user picks from the rule's `지난 회차`. A plain
//! rule judges as always. The parts are `plan.rs`'s and the history store's
//! (the flag of the first read, the times); these are how the cycle puts them
//! together.
//!
//! Release names and hashes are made up.

use trss_core::commands::CommandState;

use crate::{
    plan::{
        preview::{Kind, Preview},
        PastCause,
    },
    store::{
        channels::{Rule, RuleInput},
        history::{HistoryResult, Observation},
    },
    test_world::World,
};

const CMD: &str = "5a1f0e0e-0a70-4c1e-8f6b-7d0c2a9b3e11";
const NOVA_DIR: &str = "노바 퀘스트/Season 01";

/// `(hash, title)` of the n-th made-up release.
type Release = (String, String);

fn hash(n: u32) -> String {
    format!("eeee{n:036}")
}

fn release(n: u32, title: &str) -> Release {
    (hash(n), title.to_owned())
}

fn nova(episode: u32) -> Release {
    release(
        100 + episode,
        &format!("[SubsPlease] Nova Quest - {episode:02} (1080p) [ABCD{episode:04}].mkv"),
    )
}

fn plain(episode: u32) -> Release {
    release(
        200 + episode,
        &format!("[SubsPlease] Plain Show - {episode:02} (1080p) [EFGH{episode:04}].mkv"),
    )
}

fn plain_rule() -> RuleInput {
    RuleInput {
        r#match: Some("Plain Show".to_owned()),
        directory: "Plain Show/Season 01".to_owned(),
        ..Default::default()
    }
}

impl World {
    /// The channel of the rule `Plain Show` with no history yet, whose feed
    /// holds `releases`, and a subscription to `Nova Quest` made a second
    /// later.
    async fn with_a_subscription_to_nova(releases: &[&Release]) -> (World, Rule) {
        let s = World::with_rules(vec![plain_rule()]).await;
        s.serve(releases);
        s.advance(1_000);
        let subscription = s.subscribe("Nova Quest", NOVA_DIR, 4410, 0).await;
        (s, subscription)
    }

    /// The `show` feed holds `releases`.
    fn serve(&self, releases: &[&Release]) {
        let items: Vec<(&str, &str)> = releases
            .iter()
            .map(|(hash, title)| (hash.as_str(), title.as_str()))
            .collect();
        self.feed(&items);
    }

    /// The hashes of the torrents Transmission holds, sorted.
    fn hashes(&self) -> Vec<String> {
        let mut hashes: Vec<String> = self.tr.torrents().into_iter().map(|t| t.hash).collect();
        hashes.sort();
        hashes
    }

    /// The user picks the item with `part` for the rule, and the command runs.
    async fn pick(&self, part: &str, rule: &Rule) {
        let item = self.item_containing(part).await;
        let finished = self.receive_for(item.id, &rule.id, CMD).await.unwrap();
        assert_eq!(finished.state, CommandState::Done, "{part}");
    }
}

/// The preview's `(kind, past cause)` of the item whose title has `part`.
fn row_of(preview: &Preview, part: &str) -> (Kind, Option<PastCause>) {
    let item = preview
        .items
        .iter()
        .find(|i| i.title.contains(part))
        .unwrap_or_else(|| panic!("the preview lists no {part}: {:?}", preview.items));
    (item.kind, item.past_cause)
}

fn past_for_the_first_read() -> (Kind, Option<PastCause>) {
    (Kind::Past, Some(PastCause::FirstRead))
}

#[tokio::test]
async fn a_subscription_receives_nothing_of_what_the_feed_held_at_the_first_read() {
    let (p1, n1, n2, n3) = (plain(1), nova(1), nova(2), nova(3));
    let (s, subscription) = World::with_a_subscription_to_nova(&[&p1, &n1, &n2]).await;

    // The first read: the plain rule receives its item as ever; the
    // subscription's items are recorded, with no rule behind them.
    s.cycle_later().await;
    assert_eq!(s.hashes(), vec![hash(201)]);
    for part in ["Nova Quest - 01", "Nova Quest - 02"] {
        let item = s.item_containing(part).await;
        assert_eq!(item.result, HistoryResult::NoMatch, "{part}");
        assert_eq!(item.rule_id, None, "{part}");
    }

    // Nothing more comes of them, however often the cycle runs.
    s.cycle_later().await;
    assert_eq!(s.hashes(), vec![hash(201)]);

    // The rule detail shows them as past, for the first read.
    let preview = s.preview_of(&subscription).await;
    for part in ["Nova Quest - 01", "Nova Quest - 02"] {
        assert_eq!(row_of(&preview, part), past_for_the_first_read(), "{part}");
    }
    assert_eq!(preview.counts.past, 2);

    // The user picks episode 2; episode 1 stays as it is.
    s.pick("Nova Quest - 02", &subscription).await;
    assert_eq!(s.hashes(), vec![hash(102), hash(201)]);
    assert_eq!(
        s.item_containing("Nova Quest - 01").await.result,
        HistoryResult::NoMatch
    );

    // An episode that appears later is the subscription's own.
    s.serve(&[&p1, &n1, &n2, &n3]);
    s.cycle_later().await;
    assert_eq!(s.hashes(), vec![hash(102), hash(103), hash(201)]);
    let preview = s.preview_of(&subscription).await;
    assert_eq!(row_of(&preview, "Nova Quest - 03"), (Kind::Mine, None));
}

#[tokio::test]
async fn a_clock_that_goes_back_does_not_move_the_first_read_boundary() {
    let (p1, n1, n2, n3) = (plain(1), nova(1), nova(2), nova(3));
    let (s, subscription) = World::with_a_subscription_to_nova(&[&p1, &n1, &n2]).await;

    // The first read: the subscription's items are set aside.
    s.cycle_later().await;
    assert_eq!(s.hashes(), vec![hash(201)]);

    // The server's clock goes back an hour, and episode 3 comes: it is the
    // subscription's own, first seen before the channel's first read.
    s.advance(-3_600_000);
    s.serve(&[&p1, &n1, &n2, &n3]);
    s.cycle_later().await;
    assert_eq!(s.hashes(), vec![hash(103), hash(201)]);
    let first_read = s.item_containing("Nova Quest - 01").await.first_seen_at;
    assert!(s.item_containing("Nova Quest - 03").await.first_seen_at < first_read);

    // The first read stays the boundary: episodes 1 and 2 are still the ones
    // the feed held then, and no cycle takes them.
    s.cycle_later().await;
    s.cycle_later().await;
    assert_eq!(s.hashes(), vec![hash(103), hash(201)]);
    for part in ["Nova Quest - 01", "Nova Quest - 02"] {
        assert_eq!(
            s.item_containing(part).await.result,
            HistoryResult::NoMatch,
            "{part}"
        );
    }
    let preview = s.preview_of(&subscription).await;
    for part in ["Nova Quest - 01", "Nova Quest - 02"] {
        assert_eq!(row_of(&preview, part), past_for_the_first_read(), "{part}");
    }
}

/// A subscription that took nothing of the first read leaves the items the feed
/// held then to the user, whatever the clock did afterwards. An item that came
/// after the first read and that no rule wanted at the time is not one of them:
/// when the subscription's phrase is changed to match it, the cycle receives it.
/// The clock is moved by `before_first_read` before the first read and by
/// `after_first_read` after it, so the later item is first seen before the time
/// the first read was stamped with.
#[tokio::test]
async fn an_item_that_came_after_the_first_read_is_not_past_whatever_the_clock_did() {
    const DAY: i64 = 86_400_000;
    const HOUR: i64 = 3_600_000;
    // (what, before the first read, after it)
    let cases = [
        (
            "a first read stamped by a clock a day ahead",
            DAY,
            -(DAY - HOUR),
        ),
        (
            "a clock that went back an hour after the first read",
            2 * HOUR,
            -HOUR,
        ),
    ];
    for (what, before_first_read, after_first_read) in cases {
        let (n1, other) = (
            nova(1),
            release(301, "[SubsPlease] Other Show - 01 (1080p) [IJKL0001].mkv"),
        );
        let (s, subscription) = World::with_a_subscription_to_nova(&[&n1]).await;

        s.advance(before_first_read);
        s.cycle_later().await;
        assert!(s.hashes().is_empty(), "{what}");

        s.advance(after_first_read);
        s.serve(&[&n1, &other]);
        s.cycle_later().await;
        assert!(s.hashes().is_empty(), "{what}");
        let first_read = s.item_containing("Nova Quest - 01").await.first_seen_at;
        let later = s.item_containing("Other Show - 01").await;
        assert!(
            later.first_seen_at < first_read,
            "{what}: the clock went back"
        );
        assert_eq!(later.result, HistoryResult::NoMatch, "{what}");

        // The subscription now follows Other Show: its item came after the
        // subscription began and was not on the feed at the first read.
        s.ctx
            .channels
            .update_rule(
                &subscription.id,
                subscription.version,
                &s.channel_id,
                RuleInput {
                    r#match: Some("Other Show".to_owned()),
                    ..subscription.to_input()
                },
            )
            .await
            .unwrap();
        s.cycle_later().await;
        assert_eq!(s.hashes(), vec![hash(301)], "{what}");
    }
}

#[tokio::test]
async fn a_subscription_added_to_a_channel_that_has_history_keeps_its_own_boundary() {
    let (n1, n2) = (nova(1), nova(2));
    let s = World::with_rules(vec![plain_rule()]).await;
    s.serve(&[&n1]);

    // The first read has no subscription: the plain rule matches nothing here
    // and the episode is recorded as `no_match`.
    s.cycle_later().await;
    assert!(s.tr.torrents().is_empty());
    s.advance(1_000);
    let subscription = s.subscribe("Nova Quest", NOVA_DIR, 4410, 0).await;

    // The episode came before the subscription (not "for the first read": the
    // subscription did not exist then); the next one is received.
    s.serve(&[&n1, &n2]);
    s.cycle_later().await;
    assert_eq!(s.hashes(), vec![hash(102)]);
    let preview = s.preview_of(&subscription).await;
    assert_eq!(
        row_of(&preview, "Nova Quest - 01"),
        (Kind::Past, Some(PastCause::Subscribed))
    );
}

#[tokio::test]
async fn a_failed_first_read_leaves_the_first_read_to_the_next_cycle() {
    let (n1, n2) = (nova(1), nova(2));
    let (s, _) = World::with_a_subscription_to_nova(&[&n1]).await;

    // The feed cannot be read: nothing is recorded, so history still has no
    // record of the channel.
    s.feeds.set_xml("show", "not a feed");
    s.cycle_later().await;
    assert!(s.history_items().await.is_empty());

    // The first successful read is the first read.
    s.serve(&[&n1, &n2]);
    s.cycle_later().await;
    assert!(s.tr.torrents().is_empty());
    for part in ["Nova Quest - 01", "Nova Quest - 02"] {
        let item = s.item_containing(part).await;
        assert_eq!(item.result, HistoryResult::NoMatch, "{part}");
        assert!(item.first_read, "{part}");
    }
}

#[tokio::test]
async fn a_history_that_cannot_say_when_the_channel_was_first_read_holds_its_subscription_back() {
    let (p1, n1, n2, n3) = (plain(1), nova(1), nova(2), nova(3));
    let (s, _) = World::with_a_subscription_to_nova(&[&p1, &n1, &n2]).await;

    // The time the channel was first read, as stored, is one history cannot
    // read, so the question "when was this channel first read" fails, while the
    // feed's items are still looked up one by one.
    s.sql(&format!(
        "INSERT INTO history_first_reads (channel_id, first_read_at)
         VALUES ('{}', 'unreadable')",
        s.channel_id
    ));

    // Nothing says what the feed already held: the subscription does not take
    // it. The plain rule is unaffected.
    s.cycle_later().await;
    assert_eq!(s.hashes(), vec![hash(201)]);

    // Once history can be read again (the stored time mended, and the items of
    // that first cycle marked as the first read's, which a read that had worked
    // would have done), what the subscription sat out is past.
    let first_read = s.item_containing("Nova Quest - 01").await.first_seen_at;
    s.sql(&format!(
        "UPDATE history_first_reads SET first_read_at = {first_read};
         UPDATE history_items SET first_read = 1 WHERE first_seen_at = {first_read};"
    ));
    s.cycle_later().await;
    assert_eq!(s.hashes(), vec![hash(201)]);

    // An episode that appears later is the subscription's own.
    s.serve(&[&p1, &n1, &n2, &n3]);
    s.cycle_later().await;
    assert_eq!(s.hashes(), vec![hash(103), hash(201)]);
}

/// What the past search leaves in history when it runs before the channel's
/// feed has ever been read: the tracker's search feed is not the channel's
/// feed, so that item says nothing of what the channel held.
#[tokio::test]
async fn a_record_made_off_the_feed_is_not_the_channels_first_read() {
    let (p1, n1, n2) = (plain(1), nova(1), nova(2));
    let (s, subscription) = World::with_a_subscription_to_nova(&[&p1, &n1, &n2]).await;

    // A past search finds an earlier episode on the tracker before any cycle
    // has read the channel.
    let (off_hash, off_title) = nova(0);
    let off_link = crate::test_world::magnet(&off_hash, &off_title);
    s.ctx
        .history
        .record_elsewhere(
            s.now(),
            vec![Observation {
                channel_id: s.channel_id.clone(),
                channel_label: "x".to_owned(),
                identity_key: format!("guid-{off_hash}"),
                title: off_title,
                link: off_link,
                result: HistoryResult::NoMatch,
                rule_id: None,
                torrent_hash: None,
                reason: None,
            }],
        )
        .await
        .unwrap();
    assert!(!s.item_containing("Nova Quest - 00").await.first_read);

    // The first cycle that reads the feed still reads it for the first time:
    // the subscription takes none of what the feed holds.
    s.cycle_later().await;
    assert_eq!(s.hashes(), vec![hash(201)]);
    for part in ["Nova Quest - 01", "Nova Quest - 02"] {
        let item = s.item_containing(part).await;
        assert_eq!(item.result, HistoryResult::NoMatch, "{part}");
        assert!(item.first_read, "{part}");
    }

    // Later cycles leave them alone too, and they are past for the rule.
    s.cycle_later().await;
    assert_eq!(s.hashes(), vec![hash(201)]);
    let preview = s.preview_of(&subscription).await;
    for part in ["Nova Quest - 01", "Nova Quest - 02"] {
        assert_eq!(row_of(&preview, part), past_for_the_first_read(), "{part}");
    }
}
