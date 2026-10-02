//! What a subscription does at a channel's first read: the real worker cycle,
//! the real history, rules HTTP API and `받기`, a fake Transmission and a fake
//! RSS feed. A channel with no history (one the legacy YAML import made, for
//! one) has no record to tell the items its feed already held from the new
//! ones, so a subscription receives none of them; the user picks from the
//! rule's `지난 회차`. A plain rule judges as always.
//!
//! Tokens are made up (`common::SECRET`).

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use trss_legacy::store::{
    anissia::Anime,
    channels::{ChannelWithRules, NewSubscription, Rule, RuleInput, SubtitleMode},
    history::{HistoryResult, Observation},
};
use trss_worker::{CommandsOutcome, CycleReport, TickOutcome};

const BASE: &str = "/media/anime";
const FEED: &str = "feed-x";
const CMD: &str = "5a1f0e0e-0a70-4c1e-8f6b-7d0c2a9b3e11";

fn hash(n: u32) -> String {
    format!("eeee{n:036}")
}

/// `(guid, title, link)`.
struct Release {
    guid: String,
    title: String,
    link: String,
}

fn release(n: u32, title: &str) -> Release {
    let dn: String = url::form_urlencoded::byte_serialize(title.as_bytes()).collect();
    Release {
        guid: format!("guid-{n}"),
        title: title.to_owned(),
        link: format!("magnet:?xt=urn:btih:{}&dn={dn}", hash(n)),
    }
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

fn feed_xml(releases: &[&Release]) -> String {
    let items: String = releases
        .iter()
        .map(|r| {
            format!(
                "<item><title>{}</title><link>{}</link><guid isPermaLink=\"false\">{}</guid></item>",
                r.title.replace('&', "&amp;"),
                r.link.replace('&', "&amp;"),
                r.guid
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>x</title>
        <link>http://x/</link><description>d</description>{items}</channel></rss>"#
    )
}

struct Scene {
    h: Harness,
    api: WebApi,
    channel: ChannelWithRules,
}

impl Scene {
    /// A channel with the rule `Plain Show`, no history, and the feed serving
    /// `releases`.
    async fn new(releases: &[&Release]) -> Scene {
        let h = Harness::new().await;
        h.feeds.set_xml(FEED, &feed_xml(releases));
        let channel = h
            .add_channel(
                FEED,
                BASE,
                &[],
                vec![rule("Plain Show", "Plain Show/Season 01")],
            )
            .await;
        Scene {
            api: h.web_api(),
            h,
            channel,
        }
    }

    fn feed(&self, releases: &[&Release]) {
        self.h.feeds.set_xml(FEED, &feed_xml(releases));
    }

    async fn cycle(&self) -> CycleReport {
        self.h.advance(300_000);
        match self
            .h
            .worker()
            .tick(&CancellationToken::new())
            .await
            .unwrap()
        {
            TickOutcome::Ran(report) => report,
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let (status, _, json) = self.api.call(method, uri, body).await;
        (status, json)
    }

    /// A subscription to `Nova Quest`, subscribed now.
    async fn subscribe(&self) -> Rule {
        self.h
            .channels
            .create_subscription_rule(
                &self.channel.channel.id,
                RuleInput {
                    r#match: Some("Nova Quest".to_owned()),
                    directory: "노바 퀘스트/Season 01".to_owned(),
                    ..Default::default()
                },
                NewSubscription {
                    anime: Anime {
                        anime_no: 4410,
                        subject: "노바 퀘스트".to_owned(),
                        original_subject: None,
                        week: 4,
                        air_time: Some("23:00".to_owned()),
                        start_date: Some("2026-10-08".to_owned()),
                        end_date: None,
                        status: "ON".to_owned(),
                        fetched_at: self.h.now(),
                    },
                    subtitles: SubtitleMode::Undecided,
                    creator: None,
                    subscribed_at: self.h.now(),
                },
            )
            .await
            .unwrap()
    }

    /// What the rule detail sends to preview a stored rule as it is.
    async fn preview(&self, rule: &Rule) -> Value {
        let (status, preview) = self
            .call(
                "POST",
                "/api/rules/preview",
                Some(json!({
                    "channel_id": self.channel.channel.id,
                    "rule_id": rule.id,
                    "rule": {
                        "match": rule.r#match,
                        "regex": rule.regex,
                        "case_insensitive": rule.case_insensitive,
                        "directory": rule.directory,
                        "episode": rule.episode,
                    },
                })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{preview}");
        preview
    }

    /// The user picks the item with `part` for the rule, and the worker runs it.
    async fn receive(&self, part: &str, rule: &Rule) {
        let item = self.h.item(part).await;
        let (status, body) = self
            .call(
                "POST",
                "/api/commands",
                Some(json!({
                    "id": CMD,
                    "kind": "receive_once",
                    "payload": { "item_id": item.id, "rule_id": rule.id },
                })),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        assert_eq!(
            self.h
                .worker()
                .run_commands(&CancellationToken::new())
                .await
                .unwrap(),
            CommandsOutcome::Ran(1)
        );
    }

    fn hashes(&self) -> Vec<String> {
        let mut hashes: Vec<String> = self.h.tr.torrents().into_iter().map(|t| t.hash).collect();
        hashes.sort();
        hashes
    }
}

/// The preview's `(kind, past cause)` of the item whose title has `part`.
fn row_of(preview: &Value, part: &str) -> (String, Option<String>) {
    let item = preview["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["title"].as_str().unwrap().contains(part))
        .unwrap_or_else(|| panic!("the preview lists no {part}: {preview}"));
    (
        item["kind"].as_str().unwrap().to_owned(),
        item["past_cause"].as_str().map(str::to_owned),
    )
}

#[tokio::test]
async fn a_subscription_receives_nothing_of_what_the_feed_held_at_the_first_read() {
    let (p1, n1, n2, n3) = (plain(1), nova(1), nova(2), nova(3));
    let s = Scene::new(&[&p1, &n1, &n2]).await;
    s.h.advance(1_000);
    let subscription = s.subscribe().await;

    // The first read: the plain rule receives its item as ever; the
    // subscription's items are recorded, with no rule behind them.
    let report = s.cycle().await;
    assert_eq!(report.added, 1, "{report:?}");
    assert_eq!(report.no_match, 2, "{report:?}");
    assert_eq!(s.hashes(), vec![hash(201)]);
    for part in ["Nova Quest - 01", "Nova Quest - 02"] {
        let item = s.h.item(part).await;
        assert_eq!(item.result, HistoryResult::NoMatch, "{part}");
        assert_eq!(item.rule_id, None, "{part}");
    }

    // Nothing more comes of them, however often the cycle runs.
    s.cycle().await;
    assert_eq!(s.hashes(), vec![hash(201)]);

    // The rule detail shows them as past, for the first read.
    let preview = s.preview(&subscription).await;
    for part in ["Nova Quest - 01", "Nova Quest - 02"] {
        assert_eq!(
            row_of(&preview, part),
            ("past".to_owned(), Some("first_read".to_owned())),
            "{preview}"
        );
    }
    assert_eq!(preview["counts"]["past"], 2, "{preview}");

    // The user picks episode 2; episode 1 stays as it is.
    s.receive("Nova Quest - 02", &subscription).await;
    assert_eq!(s.hashes(), vec![hash(102), hash(201)]);
    assert_eq!(
        s.h.item("Nova Quest - 01").await.result,
        HistoryResult::NoMatch
    );

    // An episode that appears later is the subscription's own.
    s.feed(&[&p1, &n1, &n2, &n3]);
    s.cycle().await;
    assert_eq!(s.hashes(), vec![hash(102), hash(103), hash(201)]);
    let preview = s.preview(&subscription).await;
    assert_eq!(
        row_of(&preview, "Nova Quest - 03"),
        ("mine".to_owned(), None),
        "{preview}"
    );
}

#[tokio::test]
async fn a_clock_that_goes_back_does_not_move_the_first_read_boundary() {
    let (p1, n1, n2, n3) = (plain(1), nova(1), nova(2), nova(3));
    let s = Scene::new(&[&p1, &n1, &n2]).await;
    s.h.advance(1_000);
    let subscription = s.subscribe().await;

    // The first read: the subscription's items are set aside.
    s.cycle().await;
    assert_eq!(s.hashes(), vec![hash(201)]);

    // The server's clock goes back an hour, and episode 3 comes: it is the
    // subscription's own, first seen before the channel's first read.
    s.h.advance(-3_600_000);
    s.feed(&[&p1, &n1, &n2, &n3]);
    s.cycle().await;
    assert_eq!(s.hashes(), vec![hash(103), hash(201)]);
    let first_read = s.h.item("Nova Quest - 01").await.first_seen_at;
    assert!(s.h.item("Nova Quest - 03").await.first_seen_at < first_read);

    // The first read stays the boundary: episodes 1 and 2 are still the ones
    // the feed held then, and no cycle takes them.
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.hashes(), vec![hash(103), hash(201)]);
    for part in ["Nova Quest - 01", "Nova Quest - 02"] {
        assert_eq!(
            s.h.item(part).await.result,
            HistoryResult::NoMatch,
            "{part}"
        );
    }
    let preview = s.preview(&subscription).await;
    for part in ["Nova Quest - 01", "Nova Quest - 02"] {
        assert_eq!(
            row_of(&preview, part),
            ("past".to_owned(), Some("first_read".to_owned())),
            "{preview}"
        );
    }
}

/// A subscription that took nothing of the first read leaves the items the feed
/// held then to the user, whatever the clock did afterwards. An item that came
/// after the first read and that no rule wanted at the time is not one of them:
/// when the subscription's phrase is changed to match it, the cycle receives it.
/// The clock is moved by `before_first_read` before the first read and by
/// `after_first_read` after it, so the later item is first seen before the time
/// the first read was stamped with.
async fn a_later_item_is_not_the_first_reads_whatever_the_clock_did(
    before_first_read: i64,
    after_first_read: i64,
) {
    let (n1, other) = (
        nova(1),
        release(301, "[SubsPlease] Other Show - 01 (1080p) [IJKL0001].mkv"),
    );
    let s = Scene::new(&[&n1]).await;
    s.h.advance(1_000);
    let subscription = s.subscribe().await;

    s.h.advance(before_first_read);
    s.cycle().await;
    assert!(s.hashes().is_empty());

    s.h.advance(after_first_read);
    s.feed(&[&n1, &other]);
    s.cycle().await;
    assert!(s.hashes().is_empty());
    let first_read = s.h.item("Nova Quest - 01").await.first_seen_at;
    let later = s.h.item("Other Show - 01").await;
    assert!(later.first_seen_at < first_read, "the clock went back");
    assert_eq!(later.result, HistoryResult::NoMatch);

    // The subscription now follows Other Show: its item came after the
    // subscription began and was not on the feed at the first read.
    let mut input = subscription.to_input();
    input.r#match = Some("Other Show".to_owned());
    s.h.channels
        .update_rule(
            &subscription.id,
            subscription.version,
            &s.channel.channel.id,
            input,
        )
        .await
        .unwrap();
    let report = s.cycle().await;
    assert_eq!(report.added, 1, "{report:?}");
    assert_eq!(s.hashes(), vec![hash(301)]);
}

#[tokio::test]
async fn a_first_read_stamped_by_a_clock_that_was_ahead_is_not_a_boundary_for_later_items() {
    const DAY: i64 = 86_400_000;
    a_later_item_is_not_the_first_reads_whatever_the_clock_did(DAY, -(DAY - 3_600_000)).await;
}

#[tokio::test]
async fn an_item_first_seen_before_the_first_reads_time_but_after_it_in_truth_is_not_past() {
    a_later_item_is_not_the_first_reads_whatever_the_clock_did(2 * 3_600_000, -3_600_000).await;
}

#[tokio::test]
async fn a_subscription_added_to_a_channel_that_has_history_keeps_its_own_boundary() {
    let (n1, n2) = (nova(1), nova(2));
    let s = Scene::new(&[&n1]).await;

    // The first read has no subscription: the plain rule matches nothing here
    // and the episode is recorded as `no_match`.
    s.cycle().await;
    assert!(s.h.tr.torrents().is_empty());
    s.h.advance(1_000);
    let subscription = s.subscribe().await;

    // The episode came before the subscription (not "for the first read": the
    // subscription did not exist then); the next one is received.
    s.feed(&[&n1, &n2]);
    let report = s.cycle().await;
    assert_eq!(report.added, 1, "{report:?}");
    assert_eq!(s.hashes(), vec![hash(102)]);
    let preview = s.preview(&subscription).await;
    assert_eq!(
        row_of(&preview, "Nova Quest - 01"),
        ("past".to_owned(), Some("subscribed".to_owned())),
        "{preview}"
    );
}

#[tokio::test]
async fn a_failed_first_read_leaves_the_first_read_to_the_next_cycle() {
    let (n1, n2) = (nova(1), nova(2));
    let s = Scene::new(&[&n1]).await;
    s.h.advance(1_000);
    s.subscribe().await;

    // The feed cannot be read: nothing is recorded, so history still has no
    // record of the channel.
    s.h.feeds.set_xml(FEED, "not a feed");
    let report = s.cycle().await;
    assert_eq!(report.channels_failed, 1, "{report:?}");
    assert!(s.h.history_items().await.is_empty());

    // The first successful read is the first read.
    s.feed(&[&n1, &n2]);
    let report = s.cycle().await;
    assert_eq!(report.added, 0, "{report:?}");
    assert_eq!(report.no_match, 2, "{report:?}");
    assert!(s.h.tr.torrents().is_empty());
}

#[tokio::test]
async fn a_history_that_cannot_say_when_the_channel_was_first_read_holds_its_subscription_back() {
    let (p1, n1, n2, n3) = (plain(1), nova(1), nova(2), nova(3));
    let s = Scene::new(&[&p1, &n1, &n2]).await;
    s.h.advance(1_000);
    s.subscribe().await;

    // The time the channel was first read, as stored, is one history cannot
    // read, so the question "when was this channel first read" fails, while the
    // feed's items are still looked up one by one.
    let channel_id = s.channel.channel.id.clone();
    s.h.db
        .run::<_, trss_legacy::store::DbError, _>(move |c| {
            c.execute(
                "INSERT INTO history_first_reads (channel_id, first_read_at)
                 VALUES (?1, 'unreadable')",
                [&channel_id],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    // Nothing says what the feed already held: the subscription does not take
    // it. The plain rule is unaffected.
    let report = s.cycle().await;
    assert_eq!(report.added, 1, "{report:?}");
    assert_eq!(s.hashes(), vec![hash(201)]);

    // Once history can be read again (the stored time mended, and the items of
    // that first cycle marked as the first read's, which a read that had worked
    // would have done), what the subscription sat out is past.
    let first_read = s.h.item("Nova Quest - 01").await.first_seen_at;
    s.h.db
        .run::<_, trss_legacy::store::DbError, _>(move |c| {
            c.execute(
                "UPDATE history_first_reads SET first_read_at = ?1",
                [first_read],
            )?;
            c.execute(
                "UPDATE history_items SET first_read = 1 WHERE first_seen_at = ?1",
                [first_read],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    s.cycle().await;
    assert_eq!(s.hashes(), vec![hash(201)]);

    // An episode that appears later is the subscription's own.
    s.feed(&[&p1, &n1, &n2, &n3]);
    s.cycle().await;
    assert_eq!(s.hashes(), vec![hash(103), hash(201)]);
}

/// What the past search leaves in history when it runs before the channel's
/// feed has ever been read: the tracker's search feed is not the channel's
/// feed, so that item says nothing of what the channel held.
#[tokio::test]
async fn a_record_made_off_the_feed_is_not_the_channels_first_read() {
    let (p1, n1, n2) = (plain(1), nova(1), nova(2));
    let s = Scene::new(&[&p1, &n1, &n2]).await;
    s.h.advance(1_000);
    let subscription = s.subscribe().await;

    // A past search finds an earlier episode on the tracker before any cycle
    // has read the channel.
    let off_feed = nova(0);
    let at = s.h.now();
    s.h.history
        .record_elsewhere(
            at,
            vec![Observation {
                channel_id: s.channel.channel.id.clone(),
                channel_label: "x".to_owned(),
                identity_key: off_feed.guid.clone(),
                title: off_feed.title.clone(),
                link: off_feed.link.clone(),
                result: HistoryResult::NoMatch,
                rule_id: None,
                torrent_hash: None,
                reason: None,
            }],
        )
        .await
        .unwrap();
    assert!(!s.h.item("Nova Quest - 00").await.first_read);

    // The first cycle that reads the feed still reads it for the first time:
    // the subscription takes none of what the feed holds.
    let report = s.cycle().await;
    assert_eq!(report.added, 1, "{report:?}");
    assert_eq!(s.hashes(), vec![hash(201)]);
    for part in ["Nova Quest - 01", "Nova Quest - 02"] {
        let item = s.h.item(part).await;
        assert_eq!(item.result, HistoryResult::NoMatch, "{part}");
        assert!(item.first_read, "{part}");
    }

    // Later cycles leave them alone too, and they are past for the rule.
    s.cycle().await;
    assert_eq!(s.hashes(), vec![hash(201)]);
    let preview = s.preview(&subscription).await;
    for part in ["Nova Quest - 01", "Nova Quest - 02"] {
        assert_eq!(
            row_of(&preview, part),
            ("past".to_owned(), Some("first_read".to_owned())),
            "{preview}"
        );
    }
}
