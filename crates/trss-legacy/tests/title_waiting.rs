//! A subscription made before the first episode is out, and the title that
//! completes it, end to end: the real worker cycle, the real history, rules and
//! subscriptions HTTP API, a fake Transmission and a fake RSS feed. Ticket
//! 0020's completion rows about what the cycle, the preview and `받기` do.
//!
//! Tokens are made up (`common::SECRET`).

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use trss_legacy::{
    store::{
        anissia::Anime,
        channels::{ChannelWithRules, NewSubscription, Rule, RuleInput, SubtitleMode},
        history::{HistoryItem, HistoryResult},
    },
    worker::{CommandsOutcome, CycleReport, TickOutcome},
};

const BASE: &str = "/media/anime";
const FEED: &str = "feed-x";
const CMD: &str = "5a1f0e0e-0a70-4c1e-8f6b-7d0c2a9b3e11";

fn hash(n: u32) -> String {
    format!("dddd{n:036}")
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;")
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

fn feed_xml(releases: &[&Release]) -> String {
    let items: String = releases
        .iter()
        .map(|r| {
            format!(
                "<item><title>{}</title><link>{}</link><guid isPermaLink=\"false\">{}</guid></item>",
                xml_escape(&r.title),
                xml_escape(&r.link),
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
    /// A channel whose only rule matches nothing the feed serves, the feed
    /// serving `releases`, and one cycle run so that history holds them as
    /// `no_match`.
    async fn new(releases: &[&Release]) -> Scene {
        let h = Harness::new().await;
        h.feeds.set_xml(FEED, &feed_xml(releases));
        let channel = h
            .add_channel(
                FEED,
                BASE,
                &["Batch"],
                vec![rule("Some Other Show", "Some Other Show/Season 01")],
            )
            .await;
        let scene = Scene {
            api: h.web_api(),
            h,
            channel,
        };
        scene.cycle().await;
        scene.h.tr.clear_calls();
        scene
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

    async fn item(&self, title_part: &str) -> HistoryItem {
        self.h.item(title_part).await
    }

    /// A subscription with no title yet, subscribed now.
    async fn wait_for_a_title(&self, directory: &str) -> Rule {
        self.h
            .channels
            .create_subscription_rule(
                &self.channel.channel.id,
                RuleInput {
                    r#match: None,
                    directory: directory.to_owned(),
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

    async fn rule(&self, id: &str) -> Rule {
        self.h.channels.get_rule(id).await.unwrap().unwrap()
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
}

/// The preview's `(title part, kind, past cause)` of the item whose title has `part`.
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
async fn a_title_waiting_subscription_receives_nothing_and_offers_the_new_title_as_a_candidate() {
    let (n1, n2) = (nova(1), nova(2));
    let old = release(7, "[SubsPlease] Old Show - 03 (1080p) [ABCD0003].mkv");
    let s = Scene::new(&[&old]).await;
    s.h.advance(1_000);
    let waiting = s.wait_for_a_title("노바 퀘스트/Season 01").await;
    assert_eq!(waiting.r#match, None);

    // The cycle never matches a rule without a phrase: the new release is
    // recorded as `no_match`, nothing is added.
    s.feed(&[&old, &n1, &n2]);
    let report = s.cycle().await;
    assert_eq!(report.added, 0, "{report:?}");
    assert!(s.h.tr.torrents().is_empty());
    assert_eq!(
        s.item("Nova Quest - 02").await.result,
        HistoryResult::NoMatch
    );

    // The candidate is the work part of what was first seen while the
    // subscription waited; the old show, recorded before, is not one.
    let (status, body) = s.call("GET", "/api/subscriptions/candidates", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let candidates = body["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1, "{body}");
    assert_eq!(candidates[0]["work"], "Nova Quest", "{body}");
    assert_eq!(candidates[0]["items"], 2, "{body}");
    assert_eq!(candidates[0]["waiting"][0]["rule_id"], waiting.id, "{body}");

    // Rejecting hides it for good; the subscription keeps waiting.
    let (status, _) = s
        .call(
            "POST",
            "/api/subscriptions/candidates/reject",
            Some(json!({ "channel_id": s.channel.channel.id, "work": "Nova Quest" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    s.cycle().await;
    let (_, body) = s.call("GET", "/api/subscriptions/candidates", None).await;
    assert_eq!(body["candidates"].as_array().unwrap().len(), 0, "{body}");
    assert_eq!(s.rule(&waiting.id).await.r#match, None);
}

#[tokio::test]
async fn items_seen_before_the_title_are_past_and_what_comes_after_is_received_by_the_cycle() {
    let (n1, n2, n3, batch) = (
        nova(1),
        nova(2),
        nova(3),
        release(
            900,
            "[SubsPlease] Nova Quest Batch 01~02 (1080p) [ABCD0900].mkv",
        ),
    );
    let s = Scene::new(&[]).await;
    s.h.advance(1_000);
    let waiting = s.wait_for_a_title("anime/노바 퀘스트/Season 01").await;

    s.feed(&[&n1, &n2, &batch]);
    s.cycle().await;
    assert!(s.h.tr.torrents().is_empty());

    // The user names the title; the folder stays the one chosen at subscribe time.
    s.h.advance(1_000);
    let titled =
        s.h.channels
            .give_title(&waiting.id, waiting.version, "Nova Quest", None, s.h.now())
            .await
            .unwrap();
    assert_eq!(titled.r#match.as_deref(), Some("Nova Quest"));

    // The cycle does not reach back: the two episodes recorded before the
    // title stay `no_match`, however often it runs.
    s.cycle().await;
    s.cycle().await;
    assert!(s.h.tr.torrents().is_empty());
    for part in ["Nova Quest - 01", "Nova Quest - 02"] {
        let item = s.item(part).await;
        assert_eq!(item.result, HistoryResult::NoMatch, "{part}");
        assert_eq!(item.rule_id, None);
    }

    // The preview says the same: past, because of the title, and the excluded
    // batch is not offered.
    let preview = s.preview(&titled).await;
    assert_eq!(
        row_of(&preview, "Batch"),
        ("excluded".to_owned(), None),
        "{preview}"
    );
    assert_eq!(
        row_of(&preview, "Nova Quest - 02"),
        ("past".to_owned(), Some("titled".to_owned())),
        "{preview}"
    );
    assert_eq!(preview["counts"]["past"], 2, "{preview}");

    // The user ticks episode 2 only: that one is received into the rule's
    // folder, and the other stays as it is.
    let item = s.item("Nova Quest - 02").await;
    let (status, body) = s
        .call(
            "POST",
            "/api/commands",
            Some(json!({
                "id": CMD,
                "kind": "receive_once",
                "payload": { "item_id": item.id, "rule_id": titled.id },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(
        s.h.worker()
            .run_commands(&CancellationToken::new())
            .await
            .unwrap(),
        CommandsOutcome::Ran(1)
    );
    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 1);
    assert_eq!(torrents[0].hash, hash(102));
    assert_eq!(
        torrents[0].download_dir,
        "/media/anime/노바 퀘스트/Season 01"
    );
    assert_eq!(
        s.item("Nova Quest - 02").await.result,
        HistoryResult::Received
    );
    assert_eq!(
        s.item("Nova Quest - 01").await.result,
        HistoryResult::NoMatch
    );
    assert_eq!(
        s.item("Nova Quest Batch").await.result,
        HistoryResult::Excluded
    );

    // A release first seen after the title is the rule's own.
    s.feed(&[&n1, &n2, &batch, &n3]);
    s.cycle().await;
    let hashes: Vec<String> = s.h.tr.torrents().into_iter().map(|t| t.hash).collect();
    assert_eq!(hashes.len(), 2, "{hashes:?}");
    assert!(hashes.contains(&hash(103)));
    assert_eq!(
        s.item("Nova Quest - 03").await.result,
        HistoryResult::Received
    );
    let preview = s.preview(&s.rule(&titled.id).await).await;
    assert_eq!(
        row_of(&preview, "Nova Quest - 03"),
        ("mine".to_owned(), None),
        "{preview}"
    );
}

#[tokio::test]
async fn a_paused_waiting_subscription_offers_no_candidate() {
    let (n1, n2) = (nova(1), nova(2));
    let s = Scene::new(&[&n1]).await;
    s.h.advance(1_000);
    let waiting = s.wait_for_a_title("노바 퀘스트/Season 01").await;
    s.h.channels
        .set_rule_state(
            &waiting.id,
            trss_legacy::store::channels::RuleState::Archived,
            s.h.now(),
        )
        .await
        .unwrap();
    s.feed(&[&n1, &n2]);
    s.cycle().await;
    let (_, body) = s.call("GET", "/api/subscriptions/candidates", None).await;
    assert_eq!(body["candidates"].as_array().unwrap().len(), 0, "{body}");
}
