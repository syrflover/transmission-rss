//! `한 번 받기` (receive once) from the record tab, end to end: the real
//! commands and history HTTP API, the real worker code and Transmission client,
//! a fake Transmission and fake RSS feeds. Ticket 0008's completion rows.
//!
//! Tokens are made up (`common::SECRET`).

mod common;

use std::{
    fs::File,
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, Instant},
};

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use transmission_rss::worker::commands::receive_once::NAME_NOT_DERIVED;
use transmission_rss::{
    store::{
        channels::{ChannelWithRules, RuleInput},
        commands::{CommandState, CommandStore},
        history::{HistoryItem, HistoryResult},
        Db,
    },
    worker::{lock_path_for, CommandsOutcome, CycleLock, CycleReport, TickOutcome, Worker},
};

const BASE: &str = "/media/anime";
const CMD: &str = "0b7d5a44-6c1e-4c62-9a6a-3f0c1d2e4b55";
const FEED: &str = "feed-x";

// --- feeds ----------------------------------------------------------------------------

/// A hash for the n-th made-up release.
fn hash(n: u32) -> String {
    format!("cccc{n:036}")
}

fn magnet(n: u32, name: &str, extra: &str) -> String {
    let dn: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
    format!("magnet:?xt=urn:btih:{}&dn={dn}{extra}", hash(n))
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;")
}

/// `(guid, title, link)`; an empty guid leaves the `<guid>` out.
struct Release {
    guid: &'static str,
    title: String,
    link: String,
}

fn release(guid: &'static str, n: u32, title: &str, extra: &str) -> Release {
    Release {
        guid,
        title: title.to_owned(),
        link: magnet(n, title, extra),
    }
}

fn feed_xml(releases: &[&Release]) -> String {
    let items: String = releases
        .iter()
        .map(|r| {
            let guid = if r.guid.is_empty() {
                String::new()
            } else {
                format!("<guid isPermaLink=\"false\">{}</guid>", r.guid)
            };
            format!(
                "<item><title>{}</title><link>{}</link>{guid}</item>",
                xml_escape(&r.title),
                xml_escape(&r.link)
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>x</title>
        <link>http://x/</link><description>d</description>{items}</channel></rss>"#
    )
}

const LIAR: &str = "[SubsPlease] LIAR GAME - 26 (1080p) [ABCD1234].mkv";
const OTHER: &str = "[SubsPlease] Another Show - 03 (1080p) [ABCD1235].mkv";

/// A rule that matches neither release above, so both are `no_match`.
fn unrelated_rule() -> Vec<RuleInput> {
    vec![rule("Some Other Show", "Some Other Show/Season 01")]
}

// --- the scene --------------------------------------------------------------------------

struct Scene {
    h: Harness,
    api: WebApi,
    channel: ChannelWithRules,
    /// Every response body the API gave, to look for secrets in.
    bodies: std::sync::Mutex<Vec<String>>,
}

impl Scene {
    /// A channel on `FEED` with `rules`, the feed serving `releases`, and one
    /// cycle run so that history holds them.
    async fn new(releases: &[&Release], rules: Vec<RuleInput>) -> Scene {
        Scene::with(releases, BASE, &[], rules).await
    }

    async fn with(
        releases: &[&Release],
        base_dir: &str,
        excludes: &[&str],
        rules: Vec<RuleInput>,
    ) -> Scene {
        let h = Harness::new().await;
        h.feeds.set_xml(FEED, &feed_xml(releases));
        let channel = h.add_channel(FEED, base_dir, excludes, rules).await;
        let scene = Scene {
            api: h.web_api(),
            h,
            channel,
            bodies: Default::default(),
        };
        scene.cycle().await;
        scene
    }

    /// Runs one collection cycle, a period after the last.
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
        let (status, text, json) = self.api.call(method, uri, body).await;
        self.bodies.lock().unwrap().push(text);
        (status, json)
    }

    async fn item(&self, title_part: &str) -> HistoryItem {
        self.h.item(title_part).await
    }

    async fn post(&self, id: &str, item: &HistoryItem, folder: &str) -> (StatusCode, Value) {
        self.call(
            "POST",
            "/api/commands",
            Some(json!({
                "id": id,
                "kind": "receive_once",
                "payload": { "item_id": item.id, "folder": folder },
            })),
        )
        .await
    }

    async fn command(&self, id: &str) -> (StatusCode, Value) {
        self.call("GET", &format!("/api/commands/{id}"), None).await
    }

    async fn run_commands(&self) -> CommandsOutcome {
        self.run_commands_with(&self.h.worker()).await
    }

    async fn run_commands_with(&self, worker: &Worker) -> CommandsOutcome {
        worker
            .run_commands(&CancellationToken::new())
            .await
            .unwrap()
    }

    fn adds(&self) -> Vec<Value> {
        self.h
            .tr
            .calls_of("torrent-add")
            .into_iter()
            .map(|c| c.args)
            .collect()
    }

    /// The secret must be nowhere: not in an API response, a history row (with
    /// its trail), nor a stored command.
    async fn assert_secret_nowhere(&self) {
        for body in self.bodies.lock().unwrap().iter() {
            assert!(!body.contains(SECRET), "secret in a response: {body}");
        }
        for item in self.h.history_items().await {
            let text = format!("{item:?}");
            assert!(!text.contains(SECRET), "secret in history: {text}");
            for change in self.h.history.changes(item.id).await.unwrap() {
                let text = format!("{change:?}");
                assert!(!text.contains(SECRET), "secret in the trail: {text}");
            }
        }
        let commands = CommandStore::new(self.h.db.clone());
        if let Some(command) = commands.get(CMD).await.unwrap() {
            let text = format!("{command:?}");
            assert!(!text.contains(SECRET), "secret in a command: {text}");
        }
    }
}

// --- the completion rows ----------------------------------------------------------------

#[tokio::test]
async fn a_no_match_item_is_received_into_the_chosen_folder_by_the_worker() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let other = release("guid-other-3", 3, OTHER, "");
    let s = Scene::new(&[&liar, &other], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    assert_eq!(item.result, HistoryResult::NoMatch);
    assert!(s.h.tr.torrents().is_empty());

    let (status, accepted) = s.post(CMD, &item, "LIAR GAME/Season 01").await;

    // Accepted, not received: nothing has happened yet.
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(accepted["state"], "pending");
    assert!(s.h.tr.torrents().is_empty());
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::NoMatch
    );
    let (_, listed) = s.call("GET", "/api/history", None).await;
    let row = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == item.id)
        .unwrap()
        .clone();
    assert_eq!(row["result"], "no_match");
    assert_eq!(
        row["command"]["state"], "pending",
        "the row shows it in progress"
    );

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    // In the chosen folder, with the bot label and a trname name, no rule.
    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 1);
    assert_eq!(torrents[0].hash, hash(26));
    assert_eq!(torrents[0].download_dir, "/media/anime/LIAR GAME/Season 01");
    assert_eq!(torrents[0].labels, [BOT_LABEL]);
    assert_eq!(torrents[0].name, "LIAR GAME S01E26.mkv");
    let received = s.item("LIAR GAME - 26").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert_eq!(received.rule_id, None);
    assert_eq!(received.reason, None, "a renamed file needs no note");
    assert_eq!(received.torrent_hash.as_deref(), Some(hash(26).as_str()));
    // Its neighbour was not touched, and no rule was made.
    assert_eq!(s.item("Another Show").await.result, HistoryResult::NoMatch);
    let rules =
        s.h.channels
            .list_rules(&s.channel.channel.id)
            .await
            .unwrap();
    assert_eq!(rules.len(), 1);

    // The screen's poll sees the outcome on the command and on the row.
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");
    let (_, row) = s
        .call("GET", &format!("/api/history/{}", item.id), None)
        .await;
    assert_eq!(row["result"], "received");
    assert_eq!(row["by_hand"], true);
    assert_eq!(row["command"], Value::Null);
    s.assert_secret_nowhere().await;
}

#[tokio::test]
async fn without_a_chosen_folder_it_goes_to_the_channels_base_folder() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;

    assert_eq!(s.post(CMD, &item, "").await.0, StatusCode::ACCEPTED);
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    assert_eq!(s.adds().len(), 1);
    assert_eq!(s.adds()[0]["download-dir"], BASE);
    // The base folder has no title and season to name the file after: the torrent
    // stays as it is (the rule path would have removed it and its data).
    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 1, "{torrents:?}");
    assert_eq!(torrents[0].name, LIAR);
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    let received = s.item("LIAR GAME - 26").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert_eq!(received.reason.as_deref(), Some(NAME_NOT_DERIVED));
}

#[tokio::test]
async fn a_name_trname_cannot_derive_stays_in_transmission_with_its_data_and_is_noted() {
    // A release name with no episode in it, into a folder without a season:
    // the rule path would remove the torrent and its data here.
    let odd = release("guid-odd", 9, "Some Special Collection.mkv", "");
    let s = Scene::new(&[&odd], unrelated_rule()).await;
    let item = s.item("Some Special").await;

    s.post(CMD, &item, "Some Show").await;
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    // Still there, in the chosen folder, under its own name; nothing was removed.
    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 1, "{torrents:?}");
    assert_eq!(torrents[0].name, "Some Special Collection.mkv");
    assert_eq!(torrents[0].download_dir, "/media/anime/Some Show");
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert!(s.h.tr.calls_of("torrent-rename-path").is_empty());
    // Received, with a note that the name was not changed.
    let received = s.item("Some Special").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert_eq!(received.reason.as_deref(), Some(NAME_NOT_DERIVED));
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");
    let (_, row) = s
        .call("GET", &format!("/api/history/{}", item.id), None)
        .await;
    assert_eq!(row["result"], "received");
    assert_eq!(row["reason"], NAME_NOT_DERIVED);

    // The next cycle leaves it as well.
    let report = s.cycle().await;
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert_eq!(s.h.tr.torrents().len(), 1);
}

#[tokio::test]
async fn the_command_ends_only_after_its_rename_step_and_note() {
    let odd = release("guid-odd", 9, "Some Special Collection.mkv", "");
    let s = Scene::new(&[&odd], unrelated_rule()).await;
    let item = s.item("Some Special").await;
    s.post(CMD, &item, "Some Show").await;
    let gate = s.h.tr.hold("torrent-get");
    let running = {
        let worker = s.h.worker();
        tokio::spawn(async move { worker.run_commands(&CancellationToken::new()).await })
    };

    // The add looks the new torrent up; that goes through.
    gate.wait_arrived().await;
    gate.release_one();
    // The rename step looks it up again. The screen re-reads the item once the
    // command has ended, so it must not have ended before the note is written.
    gate.wait_arrived().await;
    assert_eq!(s.command(CMD).await.1["state"], "running");

    gate.release_all();
    assert_eq!(running.await.unwrap().unwrap(), CommandsOutcome::Ran(1));
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    let (_, row) = s
        .call("GET", &format!("/api/history/{}", item.id), None)
        .await;
    assert_eq!(row["reason"], NAME_NOT_DERIVED);
}

#[tokio::test]
async fn an_excluded_item_is_named_without_the_rules_episode_conversion() {
    let title = "[SubsPlease] Sono Bisque Doll - 13 (720p) [ABCD1236].mkv";
    let sono = release("guid-sono-13", 13, title, "");
    let rules = vec![RuleInput {
        episode: -12,
        ..rule("Sono Bisque Doll", "Sono Bisque Doll/Season 02")
    }];
    let s = Scene::with(&[&sono], BASE, &["(720p)"], rules).await;
    let item = s.item("Sono Bisque Doll - 13").await;
    assert_eq!(item.result, HistoryResult::Excluded);

    s.post(CMD, &item, "Sono Bisque Doll/Season 02").await;
    s.run_commands().await;

    // The rule would have made this episode 1.
    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 1);
    assert_eq!(torrents[0].name, "Sono Bisque Doll S02E13.mkv");
}

#[tokio::test]
async fn the_same_command_id_delivered_twice_adds_one_torrent_and_returns_the_result() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;

    let (first, one) = s.post(CMD, &item, "LIAR GAME/Season 01").await;
    let (second, two) = s.post(CMD, &item, "LIAR GAME/Season 01").await;
    assert_eq!((first, second), (StatusCode::ACCEPTED, StatusCode::OK));
    assert_eq!(one, two);

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
    // A late repeat, after the work is done, answers with the outcome.
    let (third, late) = s.post(CMD, &item, "LIAR GAME/Season 01").await;
    assert_eq!(third, StatusCode::OK);
    assert_eq!(late["state"], "done");
    assert_eq!(late["outcome"]["result"], "received");
    assert_eq!(s.run_commands().await, CommandsOutcome::Idle);

    assert_eq!(s.adds().len(), 1, "one torrent-add for three deliveries");
    assert_eq!(s.h.tr.torrents().len(), 1);
}

#[tokio::test]
async fn the_same_command_id_with_another_item_is_refused() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let other = release("guid-other-3", 3, OTHER, "");
    let s = Scene::new(&[&liar, &other], unrelated_rule()).await;
    let liar_item = s.item("LIAR GAME - 26").await;
    let other_item = s.item("Another Show").await;
    s.post(CMD, &liar_item, "LIAR GAME/Season 01").await;

    let (status, body) = s.post(CMD, &other_item, "LIAR GAME/Season 01").await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "conflict");
    s.run_commands().await;
    // Only the first request was carried out.
    assert_eq!(s.adds().len(), 1);
    assert_eq!(s.adds()[0]["filename"], liar.link);
    assert_eq!(s.item("Another Show").await.result, HistoryResult::NoMatch);
}

#[tokio::test]
async fn a_folder_that_leaves_the_base_folder_is_refused_and_nothing_is_accepted() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;

    for folder in ["../../etc", "/etc/cron.d", "LIAR GAME/../../x"] {
        let (status, body) = s.post(CMD, &item, folder).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{folder}");
        assert!(
            body["message"].as_str().unwrap().contains("저장 폴더"),
            "{body}"
        );
    }

    assert_eq!(s.command(CMD).await.0, StatusCode::NOT_FOUND);
    assert_eq!(s.run_commands().await, CommandsOutcome::Idle);
    assert!(s.adds().is_empty());
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::NoMatch
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_made_after_the_request_was_accepted_is_caught_when_the_worker_runs_it() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("anime");
    let outside = dir.path().join("elsewhere");
    std::fs::create_dir_all(base.join("LIAR GAME")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let s = Scene::with(&[&liar], base.to_str().unwrap(), &[], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    assert_eq!(
        s.post(CMD, &item, "LIAR GAME/Season 01").await.0,
        StatusCode::ACCEPTED
    );

    // The folder becomes a link to somewhere else before the worker gets to it.
    std::os::unix::fs::symlink(&outside, base.join("LIAR GAME/Season 01")).unwrap();
    s.run_commands().await;

    assert!(s.adds().is_empty(), "nothing goes out through the link");
    let failed = s.item("LIAR GAME - 26").await;
    assert_eq!(failed.result, HistoryResult::AddFailed);
    assert!(failed.reason.unwrap().contains("링크"));
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "failed");
}

#[tokio::test]
async fn after_a_lost_answer_the_command_is_looked_up_by_the_same_id() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;

    // The request reached the server; the answer never reached the browser.
    let _lost = s.post(CMD, &item, "LIAR GAME/Season 01").await;

    let (status, view) = s.command(CMD).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["state"], "pending");
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(s.adds().len(), 1);

    // A request that never reached the server is told apart from a failed one:
    // it is simply not there, and sending it again with the same ID is safe.
    let never_sent = "9f2c1d3e-0000-4000-8000-000000000001";
    let (status, body) = s.command(never_sent).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not_found");
}

#[tokio::test]
async fn a_stopped_transmission_leaves_add_failed_with_a_reason() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let mut s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item, "LIAR GAME/Season 01").await;
    s.h.tr.stop().await;

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    let failed = s.item("LIAR GAME - 26").await;
    assert_eq!(failed.result, HistoryResult::AddFailed);
    let reason = failed.reason.expect("a reason");
    assert!(reason.contains("Transmission"), "{reason}");
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "failed");
    assert_eq!(view["outcome"]["result"], "add_failed");
    assert_eq!(view["outcome"]["reason"], reason);
    s.assert_secret_nowhere().await;

    // It ended: Transmission coming back does not resurrect it.
    s.h.tr.restart().await;
    assert_eq!(s.run_commands().await, CommandsOutcome::Idle);
    assert!(s.adds().is_empty());
}

#[tokio::test]
async fn a_transmission_that_refuses_the_torrent_gives_its_reason() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.h.tr.reject_adds(Some("duplicate or corrupt torrent"));
    s.post(CMD, &item, "").await;

    s.run_commands().await;

    let failed = s.item("LIAR GAME - 26").await;
    assert_eq!(failed.result, HistoryResult::AddFailed);
    assert!(failed
        .reason
        .unwrap()
        .contains("duplicate or corrupt torrent"));
    assert!(s.h.tr.torrents().is_empty());
}

#[tokio::test]
async fn a_failed_item_can_be_received_again_with_a_new_command() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.h.tr.reject_adds(Some("nope"));
    s.post(CMD, &item, "").await;
    s.run_commands().await;
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::AddFailed
    );

    s.h.tr.reject_adds(None);
    let second = "1e2d3c4b-0000-4000-8000-000000000002";
    let (status, _) = s.post(second, &item, "LIAR GAME/Season 01").await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the earlier command has ended"
    );
    s.run_commands().await;

    let received = s.item("LIAR GAME - 26").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert_eq!(received.reason, None);
    assert_eq!(s.h.tr.torrents().len(), 1);
}

// --- getting the original link back -------------------------------------------------------

#[tokio::test]
async fn a_secret_in_a_query_of_the_channels_name_is_filled_back_from_the_channel() {
    let with_token = release("guid-liar-26", 26, LIAR, &format!("&token={SECRET}"));
    let s = Scene::new(&[&with_token], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    assert!(item.link.contains("token=***"), "{}", item.link);
    assert!(!item.link.contains(SECRET));
    let feed_reads = s.h.feeds.hits(FEED);

    s.post(CMD, &item, "LIAR GAME/Season 01").await;
    s.run_commands().await;

    // Transmission got the link with the channel's token in place.
    assert_eq!(s.adds().len(), 1);
    assert_eq!(s.adds()[0]["filename"], with_token.link);
    assert_eq!(s.h.feeds.hits(FEED), feed_reads, "the feed was not needed");
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Received
    );
    s.assert_secret_nowhere().await;
    // Still masked in history afterwards.
    assert!(s.item("LIAR GAME - 26").await.link.contains("token=***"));
}

#[tokio::test]
async fn a_link_whose_own_value_differs_from_the_channels_is_taken_from_the_feed_instead() {
    // No GUID, so the item's identity is its link; the channel's token fills the
    // mask into a link that hashes to something else, which is not trusted.
    let own = "OWNTOKEN9876543210";
    let with_own_token = release("", 26, LIAR, &format!("&token={own}"));
    let s = Scene::new(&[&with_own_token], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    assert!(item.identity_key.starts_with("link:"));
    let feed_reads = s.h.feeds.hits(FEED);

    s.post(CMD, &item, "").await;
    s.run_commands().await;

    assert_eq!(s.adds()[0]["filename"], with_own_token.link);
    assert_eq!(s.h.feeds.hits(FEED), feed_reads + 1, "the feed was read");
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Received
    );
    assert!(!format!("{:?}", s.h.history_items().await).contains(own));
}

#[tokio::test]
async fn a_secret_under_another_name_is_found_in_the_current_feed() {
    // The channel's token, but under a name the channel's URL does not have.
    let with_passkey = release("guid-liar-26", 26, LIAR, &format!("&passkey={SECRET}"));
    let s = Scene::new(&[&with_passkey], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    assert!(item.link.contains("passkey=***"), "{}", item.link);
    let feed_reads = s.h.feeds.hits(FEED);

    s.post(CMD, &item, "LIAR GAME/Season 01").await;
    s.run_commands().await;

    assert_eq!(s.h.feeds.hits(FEED), feed_reads + 1);
    assert_eq!(s.adds().len(), 1);
    assert_eq!(s.adds()[0]["filename"], with_passkey.link);
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Received
    );
    s.assert_secret_nowhere().await;
}

#[tokio::test]
async fn when_the_item_has_left_the_feed_the_link_cannot_be_recovered() {
    let with_passkey = release("guid-liar-26", 26, LIAR, &format!("&passkey={SECRET}"));
    let other = release("guid-other-3", 3, OTHER, "");
    let s = Scene::new(&[&with_passkey, &other], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    // The item drops out of the feed.
    s.h.feeds.set_xml(FEED, &feed_xml(&[&other]));

    s.post(CMD, &item, "LIAR GAME/Season 01").await;
    s.run_commands().await;

    let failed = s.item("LIAR GAME - 26").await;
    assert_eq!(failed.result, HistoryResult::AddFailed);
    let reason = failed.reason.expect("a reason");
    assert!(reason.contains("원래 링크를 되살리지 못했어요"), "{reason}");
    assert!(s.adds().is_empty(), "the masked link is never handed out");
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "failed");
    assert!(view["outcome"]["reason"]
        .as_str()
        .unwrap()
        .contains("원래 링크를 되살리지 못했어요"));
    s.assert_secret_nowhere().await;
}

#[tokio::test]
async fn when_the_feed_cannot_be_read_the_link_cannot_be_recovered_either() {
    let with_passkey = release("guid-liar-26", 26, LIAR, &format!("&passkey={SECRET}"));
    let s = Scene::new(&[&with_passkey], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.h.feeds.set_status(FEED, 500);

    s.post(CMD, &item, "").await;
    s.run_commands().await;

    let failed = s.item("LIAR GAME - 26").await;
    assert_eq!(failed.result, HistoryResult::AddFailed);
    assert!(failed
        .reason
        .unwrap()
        .contains("원래 링크를 되살리지 못했어요"));
    assert!(s.adds().is_empty());
    s.assert_secret_nowhere().await;
}

// --- the next cycle ------------------------------------------------------------------------

#[tokio::test]
async fn a_torrent_received_by_hand_survives_the_next_cycle_while_its_item_is_in_the_feed() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let other = release("guid-other-3", 3, OTHER, "");
    let s = Scene::new(&[&liar, &other], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item, "LIAR GAME/Season 01").await;
    s.run_commands().await;
    assert_eq!(s.h.tr.torrents().len(), 1);

    // No rule selects it, so this used to look like a torrent that had left the feed.
    let report = s.cycle().await;
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    let report = s.cycle().await;
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert_eq!(s.h.tr.torrents().len(), 1);
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Received
    );

    // Once the item leaves the feed, the ordinary cleanup applies to it again.
    s.h.feeds.set_xml(FEED, &feed_xml(&[&other]));
    let report = s.cycle().await;
    assert_eq!(report.removed.len(), 1);
    assert_eq!(report.removed[0].hash, hash(26));
    assert!(s.h.tr.torrents().is_empty());
}

#[tokio::test]
async fn a_torrent_transmission_already_had_is_kept_too() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    // Transmission holds it already (the bot added it some time ago).
    s.h.tr
        .preload(FakeTorrent::new(&hash(26), LIAR).bot().status(6));

    s.post(CMD, &item, "").await;
    s.run_commands().await;
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Duplicate
    );
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "duplicate");

    let report = s.cycle().await;
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert_eq!(s.h.tr.torrents().len(), 1);
}

#[tokio::test]
async fn a_rule_that_later_selects_a_hand_received_item_neither_removes_nor_renames_it() {
    // Received by hand into a folder where trname has no name for the file.
    let odd = release("guid-odd", 9, "Some Special Collection.mkv", "");
    let s = Scene::new(&[&odd], unrelated_rule()).await;
    let item = s.item("Some Special").await;
    s.post(CMD, &item, "Some Show").await;
    s.run_commands().await;
    assert_eq!(s.h.tr.torrents().len(), 1);

    // A rule made from the item (항목에서 새 규칙) selects it now, into a folder
    // trname cannot name the file for either: the legacy renaming would remove
    // the torrent and its data.
    s.h.channels
        .create_rule(
            &s.channel.channel.id,
            rule("Some Special", "Some Special/Season 01"),
        )
        .await
        .unwrap();
    s.h.tr.clear_calls();
    let report = s.cycle().await;

    assert_eq!(report.duplicates, 1);
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert!(s.h.tr.calls_of("torrent-rename-path").is_empty());
    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 1);
    assert_eq!(torrents[0].name, "Some Special Collection.mkv");
    assert_eq!(torrents[0].download_dir, "/media/anime/Some Show");
    let kept = s.item("Some Special").await;
    assert_eq!(kept.result, HistoryResult::Received);
    assert_eq!(kept.rule_id, None, "still received by hand");
    assert_eq!(kept.reason.as_deref(), Some(NAME_NOT_DERIVED));
}

#[tokio::test]
async fn a_rule_with_another_folder_does_not_rename_a_hand_received_file() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item, "LIAR GAME/Season 01").await;
    s.run_commands().await;
    assert_eq!(s.h.tr.torrents()[0].name, "LIAR GAME S01E26.mkv");

    s.h.channels
        .create_rule(
            &s.channel.channel.id,
            rule("LIAR GAME", "LIAR GAME/Season 02"),
        )
        .await
        .unwrap();
    s.h.tr.clear_calls();
    s.cycle().await;

    assert!(s.h.tr.calls_of("torrent-rename-path").is_empty());
    assert_eq!(s.h.tr.torrents()[0].name, "LIAR GAME S01E26.mkv");
}

#[tokio::test]
async fn a_command_that_meets_the_torrent_a_rule_received_after_intake_leaves_it_as_it_is() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item, "Other Title/Season 03").await;
    // Before the worker gets to the command, a new rule receives the item.
    s.h.channels
        .create_rule(
            &s.channel.channel.id,
            rule("LIAR GAME", "LIAR GAME/Season 01"),
        )
        .await
        .unwrap();
    s.cycle().await;
    assert_eq!(s.h.tr.torrents()[0].name, "LIAR GAME S01E26.mkv");
    s.h.tr.clear_calls();

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    // The command's add met the rule's torrent: no rename into its folder's name.
    assert!(s.h.tr.calls_of("torrent-rename-path").is_empty());
    assert_eq!(s.h.tr.torrents()[0].name, "LIAR GAME S01E26.mkv");
    // It ends with what the item says, without calling it a duplicate.
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");
    assert_eq!(view["outcome"]["reason"], Value::Null);
    let received = s.item("LIAR GAME - 26").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert!(received.rule_id.is_some(), "still the rule's");
    assert_eq!(received.reason, None);
}

#[tokio::test]
async fn a_command_into_the_base_folder_puts_no_note_on_an_item_a_rule_received() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item, "").await;
    s.h.channels
        .create_rule(
            &s.channel.channel.id,
            rule("LIAR GAME", "LIAR GAME/Season 01"),
        )
        .await
        .unwrap();
    s.cycle().await;

    s.run_commands().await;

    let received = s.item("LIAR GAME - 26").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert_eq!(received.reason, None, "the rule's file was named; no note");
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["outcome"]["result"], "received");
    assert_eq!(view["outcome"]["reason"], Value::Null);
}

#[tokio::test]
async fn a_failed_add_for_an_item_already_held_ends_with_the_items_result() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let other = release("guid-other-3", 3, OTHER, "");
    let s = Scene::new(&[&liar, &other], unrelated_rule()).await;
    let liar_item = s.item("LIAR GAME - 26").await;
    let other_item = s.item("Another Show").await;
    s.post(CMD, &liar_item, "LIAR GAME/Season 01").await;
    let second = "1e2d3c4b-0000-4000-8000-000000000003";
    s.post(second, &other_item, "").await;
    // Before the worker gets to the commands, rules take both items: one is
    // received, the other was in Transmission already.
    s.h.tr
        .preload(FakeTorrent::new(&hash(3), OTHER).bot().status(6));
    for (text, folder) in [
        ("LIAR GAME", "LIAR GAME/Season 01"),
        ("Another Show", "Another Show/Season 01"),
    ] {
        s.h.channels
            .create_rule(&s.channel.channel.id, rule(text, folder))
            .await
            .unwrap();
    }
    s.cycle().await;
    // Then Transmission refuses the commands' adds.
    s.h.tr.reject_adds(Some("nope"));

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(2));

    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");
    assert_eq!(view["outcome"]["reason"], Value::Null);
    let (_, view) = s.command(second).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "duplicate");
    assert!(view["outcome"]["reason"]
        .as_str()
        .unwrap()
        .contains("이미 같은 토렌트"));
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Received
    );
    assert_eq!(
        s.item("Another Show").await.result,
        HistoryResult::Duplicate
    );
}

// --- restarts, two workers and the lock --------------------------------------------------------

#[tokio::test]
async fn a_command_accepted_before_a_restart_runs_once_after_it() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item, "LIAR GAME/Season 01").await;

    // The worker starts up again: a new process with its own database handle.
    let reopened = Db::open(s.h.db_path()).await.unwrap();
    let worker = s.h.worker_with_db(reopened);
    assert_eq!(s.run_commands_with(&worker).await, CommandsOutcome::Ran(1));
    assert_eq!(s.run_commands_with(&worker).await, CommandsOutcome::Idle);

    assert_eq!(s.adds().len(), 1);
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Received
    );
}

#[tokio::test]
async fn a_command_a_worker_died_in_is_run_again_and_adds_one_torrent() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item, "LIAR GAME/Season 01").await;
    // A worker claimed it and died before doing anything.
    let store = CommandStore::new(s.h.db.clone());
    let claimed = store.claim_next(s.h.now()).await.unwrap().unwrap();
    assert_eq!(claimed.state, CommandState::Running);
    assert_eq!(s.command(CMD).await.1["state"], "running");

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    assert_eq!(s.adds().len(), 1);
    assert_eq!(s.h.tr.torrents().len(), 1);
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");
    assert_eq!(store.get(CMD).await.unwrap().unwrap().attempts, 2);
}

#[tokio::test]
async fn a_command_whose_torrent_went_in_before_the_worker_died_adds_no_second_torrent() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item, "LIAR GAME/Season 01").await;
    let store = CommandStore::new(s.h.db.clone());
    store.claim_next(s.h.now()).await.unwrap().unwrap();
    // Transmission took the torrent, then the worker died before writing the result.
    s.h.tr.preload(FakeTorrent::new(&hash(26), LIAR).bot());

    s.run_commands().await;

    assert_eq!(s.h.tr.torrents().len(), 1, "one torrent in Transmission");
    let item = s.item("LIAR GAME - 26").await;
    assert_eq!(item.result, HistoryResult::Duplicate);
    assert_eq!(s.command(CMD).await.1["state"], "done");
    // The rerun did not add the torrent, so it does not rename it either.
    assert!(s.h.tr.calls_of("torrent-rename-path").is_empty());
    assert_eq!(item.reason, None);
}

#[tokio::test]
async fn two_workers_never_run_one_command_twice() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item, "LIAR GAME/Season 01").await;

    // The first worker is held inside Transmission's `torrent-add`.
    let gate = s.h.tr.hold("torrent-add");
    let first = {
        let worker = s.h.worker();
        tokio::spawn(async move { worker.run_commands(&CancellationToken::new()).await })
    };
    gate.wait_arrived().await;

    // The second finds the lock taken and leaves the command alone.
    let other = Db::open(s.h.db_path()).await.unwrap();
    assert_eq!(
        s.run_commands_with(&s.h.worker_with_db(other)).await,
        CommandsOutcome::Busy
    );

    gate.release_all();
    assert_eq!(first.await.unwrap().unwrap(), CommandsOutcome::Ran(1));
    assert_eq!(s.adds().len(), 1);
    assert_eq!(s.run_commands().await, CommandsOutcome::Idle);
}

#[tokio::test]
async fn a_command_waits_while_a_cycle_holds_the_lock() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item, "").await;

    // A cycle in progress (held at its first request to Transmission).
    let gate = s.h.tr.hold("session-set");
    s.h.advance(300_000);
    let cycle = {
        let worker = s.h.worker();
        tokio::spawn(async move { worker.tick(&CancellationToken::new()).await })
    };
    gate.wait_arrived().await;

    assert_eq!(s.run_commands().await, CommandsOutcome::Busy);
    assert!(s.adds().is_empty(), "no command runs inside a cycle");
    assert_eq!(s.command(CMD).await.1["state"], "pending");

    gate.release_all();
    cycle.await.unwrap().unwrap();
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
    assert_eq!(s.adds().len(), 1);
}

#[tokio::test]
async fn a_busy_lock_is_not_an_error_and_the_next_look_takes_the_command() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item, "").await;

    let lock = CycleLock::try_acquire(&lock_path_for(&s.h.db_path()))
        .unwrap()
        .expect("free lock");
    assert_eq!(s.run_commands().await, CommandsOutcome::Busy);
    assert_eq!(s.run_commands().await, CommandsOutcome::Busy);
    assert_eq!(s.command(CMD).await.1["state"], "pending");

    drop(lock);
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
}

#[tokio::test]
async fn the_running_worker_picks_up_a_command_without_waiting_for_the_next_cycle() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;

    // A cycle every hour; commands are looked for every 20 ms.
    let env =
        s.h.worker_env_with(&[("TRSS_WORKER_INTERVAL_SECS", "3600")]);
    let worker =
        s.h.worker_with(s.h.db.clone(), &env)
            .with_command_poll(Duration::from_millis(20));
    let cancel = CancellationToken::new();
    let running = {
        let (worker, cancel) = (worker.clone(), cancel.clone());
        tokio::spawn(async move { worker.run(cancel).await })
    };
    // The worker's own first cycle reads the feed; the command must not need another.
    let before_start = s.h.feeds.hits(FEED);
    wait_for("the first cycle", || async {
        s.h.feeds.hits(FEED) > before_start
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let cycles_before = s.h.feeds.hits(FEED);

    let (status, _) = s.post(CMD, &item, "LIAR GAME/Season 01").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    wait_for("the result", || async {
        s.command(CMD).await.1["state"] == "done"
    })
    .await;

    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Received
    );
    assert_eq!(s.h.feeds.hits(FEED), cycles_before, "no cycle was needed");
    cancel.cancel();
    running.await.unwrap();
}

async fn wait_for<F, Fut>(what: &str, mut cond: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(30);
    while !cond().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// --- the real worker process ----------------------------------------------------------------------

struct Proc {
    child: Child,
    stdout: PathBuf,
    stderr: PathBuf,
}

impl Proc {
    fn spawn(h: &Harness) -> Proc {
        let stdout = h.dir.path().join("w.out");
        let stderr = h.dir.path().join("w.err");
        let child = Command::new(env!("CARGO_BIN_EXE_trss-worker"))
            .current_dir(h.dir.path())
            .env("TRSS_DB_PATH", h.db_path())
            .env("TRANSMISSION_URL", h.tr.url())
            .env("TRSS_WORKER_INTERVAL_SECS", "3600")
            .env_remove("CHANNELS_CONFIG_URL")
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .expect("spawn trss-worker");
        Proc {
            child,
            stdout,
            stderr,
        }
    }

    fn output(&self) -> String {
        format!(
            "{}{}",
            std::fs::read_to_string(&self.stdout).unwrap_or_default(),
            std::fs::read_to_string(&self.stderr).unwrap_or_default()
        )
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn a_started_worker_process_runs_an_accepted_command_and_prints_no_secret() {
    // The secret sits under another name, so the process reads the feed for it.
    let with_passkey = release("guid-liar-26", 26, LIAR, &format!("&passkey={SECRET}"));
    let s = Scene::new(&[&with_passkey], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    // Accepted while no worker runs.
    s.post(CMD, &item, "LIAR GAME/Season 01").await;
    assert!(s.adds().is_empty());

    let mut worker = Proc::spawn(&s.h);
    wait_for("the command to be done", || async {
        s.command(CMD).await.1["state"] == "done"
    })
    .await;

    assert_eq!(s.adds().len(), 1, "{}", worker.output());
    assert_eq!(s.adds()[0]["filename"], with_passkey.link);
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Received
    );
    let status = Command::new("kill")
        .args(["-TERM", &worker.child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let deadline = Instant::now() + Duration::from_secs(10);
    while worker.child.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "no exit; output:\n{}",
            worker.output()
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let output = worker.output();
    assert!(output.contains(CMD), "the command is logged: {output}");
    assert!(
        !output.contains(SECRET),
        "secret in the worker's output:\n{output}"
    );
    s.assert_secret_nowhere().await;
}
