//! `다시 받기` (a retry, stored as the `receive_once` command) from the record
//! tab, end to end: the real commands and history HTTP API, the real worker
//! code and Transmission client, a fake Transmission and fake RSS feeds.
//! Ticket 0008's completion rows.
//!
//! The items retried here were picked by a rule whose add Transmission refused
//! (`Scene::failing`), so they are `add_failed` with the rule recorded, as the
//! rule's own cycle leaves them.
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
use trss_collect::{
    commands::receive_once::{NAME_NOT_DERIVED, SEVERAL_FILES},
    store::{
        channels::{ChannelWithRules, RuleInput, RuleState},
        history::{HistoryItem, HistoryResult},
    },
};
use trss_core::{
    commands::{CommandState, CommandStore, NewCommand, MAX_ATTEMPTS},
    lock_path_for, CycleLock, Db,
};
use trss_transmission::item_label;
use trss_worker::{CommandsOutcome, CycleReport, TickOutcome, Worker};

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

/// A `.torrent` download link on `host`, which the fake Transmission reads
/// like a magnet link.
fn download_link(host: &str, n: u32, name: &str, extra: &str) -> String {
    let dn: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
    format!("http://{host}/dl?xt=urn:btih:{}&dn={dn}{extra}", hash(n))
}

/// The host of the fake feeds, hence of every channel URL here.
const FEED_HOST: &str = "127.0.0.1";

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

/// A rule for each release above, with the folders a person would give them.
fn picked_rules() -> Vec<RuleInput> {
    vec![
        rule("LIAR GAME", "LIAR GAME/Season 01"),
        rule("Another Show", "Another Show/Season 01"),
    ]
}

/// A rule that matches neither release above, so both are `no_match`.
fn unrelated_rule() -> Vec<RuleInput> {
    vec![rule("Some Other Show", "Some Other Show/Season 01")]
}

/// What the fake Transmission says while it refuses the adds of a cycle.
const REFUSAL: &str = "Transmission is having a bad day";

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
        Scene::with(releases, BASE, &[], rules, false).await
    }

    /// Like [`Scene::new`], but Transmission refuses every add of that first
    /// cycle: the items the rules pick are `add_failed` with their rule
    /// recorded, which is what `다시 받기` is for. Transmission takes adds again
    /// when this returns.
    async fn failing(releases: &[&Release], rules: Vec<RuleInput>) -> Scene {
        Scene::with(releases, BASE, &[], rules, true).await
    }

    async fn with(
        releases: &[&Release],
        base_dir: &str,
        excludes: &[&str],
        rules: Vec<RuleInput>,
        refuse_adds: bool,
    ) -> Scene {
        let h = Harness::new().await;
        h.feeds.set_xml(FEED, &feed_xml(releases));
        if refuse_adds {
            h.tr.reject_adds(Some(REFUSAL));
        }
        let channel = h.add_channel(FEED, base_dir, excludes, rules).await;
        let scene = Scene {
            api: h.web_api(),
            h,
            channel,
            bodies: Default::default(),
        };
        scene.cycle().await;
        scene.h.tr.reject_adds(None);
        // What the set-up cycle asked of Transmission is not what tests look at.
        scene.h.tr.clear_calls();
        scene
    }

    /// What the feed serves from now on.
    fn feed(&self, releases: &[&Release]) {
        self.h.feeds.set_xml(FEED, &feed_xml(releases));
    }

    /// The `n`-th rule the channel was made with.
    fn rule_of(&self, n: usize) -> &trss_collect::store::channels::Rule {
        &self.channel.rules[n]
    }

    /// Archives the `n`-th rule, so that cycles no longer pick what it matched.
    async fn archive_rule(&self, n: usize) {
        let rule = self
            .h
            .channels
            .get_rule(&self.rule_of(n).id)
            .await
            .unwrap()
            .unwrap();
        self.h
            .channels
            .update_rule(
                &rule.id,
                rule.version,
                &rule.channel_id,
                RuleInput {
                    state: RuleState::Archived,
                    ..rule.to_input()
                },
            )
            .await
            .unwrap();
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

    async fn post(&self, id: &str, item: &HistoryItem) -> (StatusCode, Value) {
        self.post_payload(id, json!({ "item_id": item.id })).await
    }

    async fn post_payload(&self, id: &str, payload: Value) -> (StatusCode, Value) {
        self.call(
            "POST",
            "/api/commands",
            Some(json!({ "id": id, "kind": "receive_once", "payload": payload })),
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
async fn a_failed_item_is_added_again_into_its_rules_folder_by_the_worker() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let other = release("guid-other-3", 3, OTHER, "");
    let s = Scene::failing(&[&liar, &other], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    assert_eq!(item.result, HistoryResult::AddFailed);
    assert_eq!(item.rule_id.as_deref(), Some(s.rule_of(0).id.as_str()));
    assert!(s.h.tr.torrents().is_empty());

    let (status, accepted) = s.post(CMD, &item).await;

    // Accepted, not added: nothing has happened yet.
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(accepted["state"], "pending");
    assert!(s.h.tr.torrents().is_empty());
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::AddFailed
    );
    let (_, listed) = s.call("GET", "/api/history", None).await;
    let row = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == item.id)
        .unwrap()
        .clone();
    assert_eq!(row["result"], "add_failed");
    assert_eq!(row["can_retry"], true);
    assert_eq!(
        row["command"]["state"], "pending",
        "the row shows it in progress"
    );

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    // In the rule's folder, with the bot label and a trname name.
    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 1);
    assert_eq!(torrents[0].hash, hash(26));
    assert_eq!(torrents[0].download_dir, "/media/anime/LIAR GAME/Season 01");
    let item_label = format!("trss-item:{}:{}", item.channel_id, item.identity_key);
    // The add carried the command's label too; the command took it off once it
    // had recorded the torrent.
    let add = &s.h.tr.calls_of("torrent-add")[0];
    assert_eq!(
        add.args["labels"],
        json!([BOT_LABEL, item_label, format!("trss-cmd:{CMD}")])
    );
    assert_eq!(torrents[0].labels, [BOT_LABEL, item_label.as_str()]);
    assert_eq!(torrents[0].name, "LIAR GAME S01E26.mkv");
    // The item is the rule's now: it shows `규칙 ‘…’`, not a receive by hand.
    let received = s.item("LIAR GAME - 26").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert_eq!(received.rule_id.as_deref(), Some(s.rule_of(0).id.as_str()));
    assert_eq!(received.reason, None, "a renamed file needs no note");
    assert_eq!(received.torrent_hash.as_deref(), Some(hash(26).as_str()));
    // Its neighbour was not touched, and no rule was made.
    assert_eq!(
        s.item("Another Show").await.result,
        HistoryResult::AddFailed
    );
    let rules =
        s.h.channels
            .list_rules(&s.channel.channel.id)
            .await
            .unwrap();
    assert_eq!(rules.len(), 2);

    // The screen's poll sees the outcome on the command and on the row.
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");
    let (_, row) = s
        .call("GET", &format!("/api/history/{}", item.id), None)
        .await;
    assert_eq!(row["result"], "received");
    assert_eq!(row["result_label"], "추가함");
    assert_eq!(row["by_hand"], false);
    assert_eq!(row["rule_label"], "LIAR GAME");
    assert_eq!(row["can_retry"], false);
    assert_eq!(row["command"], Value::Null);
    s.assert_secret_nowhere().await;
}

#[tokio::test]
async fn the_rules_episode_conversion_names_the_file() {
    let title = "[SubsPlease] Sono Bisque Doll - 13 (1080p) [ABCD1236].mkv";
    let sono = release("guid-sono-13", 13, title, "");
    let rules = vec![RuleInput {
        episode: -12,
        ..rule("Sono Bisque Doll", "Sono Bisque Doll/Season 02")
    }];
    let s = Scene::failing(&[&sono], rules).await;
    let item = s.item("Sono Bisque Doll - 13").await;
    assert_eq!(item.result, HistoryResult::AddFailed);

    s.post(CMD, &item).await;
    s.run_commands().await;

    // Release 13 is episode 1 of season 2, as the rule's own cycle would name it.
    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 1);
    assert_eq!(
        torrents[0].download_dir,
        "/media/anime/Sono Bisque Doll/Season 02"
    );
    assert_eq!(torrents[0].name, "Sono Bisque Doll S02E01.mkv");
    assert_eq!(s.command(CMD).await.1["state"], "done");
}

#[tokio::test]
async fn a_retry_goes_where_the_rules_own_cycle_puts_the_same_release() {
    // The same release, added by a cycle in one scene and by a retry in another:
    // the folder and the file name agree.
    let title = "[SubsPlease] Sono Bisque Doll - 13 (1080p) [ABCD1236].mkv";
    let sono = release("guid-sono-13", 13, title, "");
    let rules = || {
        vec![RuleInput {
            episode: -12,
            ..rule("Sono Bisque Doll", "Sono Bisque Doll/Season 02")
        }]
    };
    let cycled = Scene::new(&[&sono], rules()).await;
    let retried = Scene::failing(&[&sono], rules()).await;
    retried
        .post(CMD, &retried.item("Sono Bisque Doll - 13").await)
        .await;
    retried.run_commands().await;

    let by_cycle = &cycled.h.tr.torrents()[0];
    let by_retry = &retried.h.tr.torrents()[0];
    assert_eq!(by_retry.download_dir, by_cycle.download_dir);
    assert_eq!(by_retry.name, by_cycle.name);
}

#[tokio::test]
async fn a_rule_without_a_folder_retries_into_the_collect_folder() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::with(
        &[&liar],
        COLLECT_FOLDER,
        &[],
        vec![rule("LIAR GAME", "")],
        true,
    )
    .await;
    let item = s.item("LIAR GAME - 26").await;

    assert_eq!(s.post(CMD, &item).await.0, StatusCode::ACCEPTED);
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    assert_eq!(s.adds().len(), 1);
    // The rule's own cycle joins an empty folder on the same way.
    assert_eq!(s.adds()[0]["download-dir"], "/media/");
    // The collect folder has no title and season to name the file after: the torrent
    // stays as it is (the rule's cycle would have removed it and its data).
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
    // the rule's cycle would remove the torrent and its data here.
    let odd = release("guid-odd", 9, "Some Special Collection.mkv", "");
    let s = Scene::failing(&[&odd], vec![rule("Some Special", "Some Show")]).await;
    let item = s.item("Some Special").await;

    s.post(CMD, &item).await;
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    // Still there, in the rule's folder, under its own name; nothing was removed.
    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 1, "{torrents:?}");
    assert_eq!(torrents[0].name, "Some Special Collection.mkv");
    assert_eq!(torrents[0].download_dir, "/media/anime/Some Show");
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert!(s.h.tr.calls_of("torrent-rename-path").is_empty());
    // Added, with a note that the name was not changed.
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
    assert_eq!(s.h.tr.torrents()[0].name, "Some Special Collection.mkv");
}

#[tokio::test]
async fn the_command_ends_only_after_its_rename_step_and_note() {
    let odd = release("guid-odd", 9, "Some Special Collection.mkv", "");
    let s = Scene::failing(&[&odd], vec![rule("Some Special", "Some Show")]).await;
    let item = s.item("Some Special").await;
    s.post(CMD, &item).await;
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
async fn a_torrent_with_several_files_is_left_as_it_is_without_retrying() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.h.tr.files_on_add(&hash(26), 12);
    s.post(CMD, &item).await;
    s.h.tr.clear_calls();

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    // One lookup by the add, one by the rename step, which stops there, and
    // one to take the command's label off.
    assert_eq!(s.h.tr.calls_of("torrent-get").len(), 3);
    assert!(s.h.tr.calls_of("torrent-rename-path").is_empty());
    assert_eq!(s.h.tr.torrents()[0].name, LIAR);
    let received = s.item("LIAR GAME - 26").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert_eq!(received.reason.as_deref(), Some(SEVERAL_FILES));
    assert_eq!(s.command(CMD).await.1["state"], "done");
}

#[tokio::test]
async fn the_same_command_id_delivered_twice_adds_one_torrent_and_returns_the_result() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.h.tr.clear_calls();

    let (first, one) = s.post(CMD, &item).await;
    // A page from before the folder went away repeats it with an empty one.
    let (second, two) = s
        .post_payload(CMD, json!({ "item_id": item.id, "folder": "" }))
        .await;
    assert_eq!((first, second), (StatusCode::ACCEPTED, StatusCode::OK));
    assert_eq!(one, two);

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
    // A late repeat, after the work is done, answers with the outcome.
    let (third, late) = s.post(CMD, &item).await;
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
    let s = Scene::failing(&[&liar, &other], picked_rules()).await;
    let liar_item = s.item("LIAR GAME - 26").await;
    let other_item = s.item("Another Show").await;
    s.h.tr.clear_calls();
    s.post(CMD, &liar_item).await;

    let (status, body) = s.post(CMD, &other_item).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "conflict");
    s.run_commands().await;
    // Only the first request was carried out.
    assert_eq!(s.adds().len(), 1);
    assert_eq!(s.adds()[0]["filename"], liar.link);
    assert_eq!(
        s.item("Another Show").await.result,
        HistoryResult::AddFailed
    );
}

#[tokio::test]
async fn a_request_naming_a_folder_is_refused_and_nothing_is_accepted() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.h.tr.clear_calls();

    for folder in ["LIAR GAME/Season 01", "../../etc", "/etc/cron.d"] {
        let (status, body) = s
            .post_payload(CMD, json!({ "item_id": item.id, "folder": folder }))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{folder}");
        assert_eq!(body["error"], "invalid");
    }

    assert_eq!(s.command(CMD).await.0, StatusCode::NOT_FOUND);
    assert_eq!(s.run_commands().await, CommandsOutcome::Idle);
    assert!(s.adds().is_empty());
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::AddFailed
    );
}

/// Stores a command the way a version that let a person type the folder did.
async fn store_legacy(s: &Scene, item: &HistoryItem, folder: &str) {
    CommandStore::new(s.h.db.clone())
        .accept(
            NewCommand {
                id: CMD.to_owned(),
                kind: "receive_once".to_owned(),
                payload: format!(r#"{{"item_id":{},"folder":{}}}"#, item.id, json!(folder)),
                subject: Some(item.id.to_string()),
            },
            s.h.now(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_command_stored_with_an_empty_folder_before_the_change_runs_as_a_retry() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    store_legacy(&s, &item, "").await;

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 1);
    assert_eq!(torrents[0].download_dir, "/media/anime/LIAR GAME/Season 01");
    assert_eq!(torrents[0].name, "LIAR GAME S01E26.mkv");
    assert_eq!(s.command(CMD).await.1["outcome"]["result"], "received");
}

#[tokio::test]
async fn a_command_stored_with_a_folder_before_the_change_is_not_run_into_the_rules_folder() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    store_legacy(&s, &item, "Somewhere/Else").await;
    // An earlier start of it put a torrent in under its label, and the worker
    // died before it wrote the result.
    CommandStore::new(s.h.db.clone())
        .claim_next(s.h.now())
        .await
        .unwrap()
        .unwrap();
    s.h.tr.preload(taken_by_the_commands_add(&item));

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    let (_, command) = s.command(CMD).await;
    assert_eq!(command["state"], "failed", "{command}");
    assert_eq!(command["outcome"]["result"], "add_failed");
    assert_eq!(
        command["outcome"]["reason"],
        "폴더를 고르던 예전 요청이라 실행하지 않았어요. 필요하면 다시 받기로 받아요."
    );
    assert!(s.adds().is_empty(), "nothing went to Transmission");
    assert_eq!(
        s.item("LIAR GAME - 26").await,
        item,
        "the item is as it was"
    );
    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 1, "the torrent is not removed");
    assert!(
        !torrents[0]
            .labels
            .iter()
            .any(|l| l.starts_with("trss-cmd:")),
        "{:?}",
        torrents[0].labels
    );
}

// --- items that cannot be retried ----------------------------------------------------------

/// What a retry that was accepted and then found ineligible leaves: the command
/// ended `add_failed` with the reason, nothing went to Transmission, and the
/// item is as it was.
async fn assert_ended_without_adding(s: &Scene, says: &str, item_before: &HistoryItem) {
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "failed", "{view}");
    assert_eq!(view["outcome"]["result"], "add_failed");
    let reason = view["outcome"]["reason"].as_str().unwrap();
    assert!(reason.contains(says), "{reason}");
    assert!(s.adds().is_empty(), "nothing went to Transmission");
    assert_eq!(&s.item(&item_before.title).await, item_before);
}

#[tokio::test]
async fn a_retry_with_no_collect_folder_ends_at_once_and_leaves_the_item_as_it_was() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    // The app cannot unset the folder once it is set; this is a database that
    // has none, as a fresh one does.
    s.h.db
        .run::<_, trss_core::DbError, _>(|c| {
            Ok(c.execute("DELETE FROM collection_settings", [])
                .map(|_| ())?)
        })
        .await
        .unwrap();

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    // The rule's folder is relative to the collect folder, so there is nowhere
    // to put the torrent. The item stays `add_failed` as it was, not failed anew.
    assert_ended_without_adding(&s, "수집 폴더", &item).await;
}

#[tokio::test]
async fn a_rule_deleted_after_the_request_ends_the_command_at_once() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    s.h.tr.clear_calls();
    let rule = s.rule_of(0);
    s.h.channels
        .delete_rule(&rule.id, rule.version)
        .await
        .unwrap();

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    assert_ended_without_adding(&s, "지워져서", &item).await;
    // The same item is now refused when it is asked for, with the same reason.
    let (status, body) = s.post("1e2d3c4b-0000-4000-8000-000000000004", &item).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["message"].as_str().unwrap().contains("지워져서"));
}

#[tokio::test]
async fn a_rule_archived_after_the_request_ends_the_command_at_once() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    s.h.tr.clear_calls();
    s.archive_rule(0).await;

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    assert_ended_without_adding(&s, "복원한 뒤", &item).await;
    let (status, body) = s.post("1e2d3c4b-0000-4000-8000-000000000004", &item).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["message"].as_str().unwrap().contains("복원한 뒤"));
}

#[tokio::test]
async fn an_item_that_a_later_cycle_found_no_rule_for_ends_the_command_at_once() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    // The rule is edited so that it no longer picks the item, and a cycle sees that.
    let rule = s.rule_of(0);
    s.h.channels
        .update_rule(
            &rule.id,
            rule.version,
            &rule.channel_id,
            RuleInput {
                r#match: Some("Something Else".into()),
                ..rule.to_input()
            },
        )
        .await
        .unwrap();
    s.cycle().await;
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::NoMatch
    );
    let item = s.item("LIAR GAME - 26").await;
    s.h.tr.clear_calls();

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    assert_ended_without_adding(&s, "규칙이 고르지 않아서", &item).await;
}

#[tokio::test]
async fn a_failure_with_no_rule_recorded_is_not_offered_and_ends_the_command_at_once() {
    // What the receive-once that took any item into a typed folder left behind:
    // a failed item with no rule.
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    assert_eq!(item.result, HistoryResult::NoMatch);
    s.h.history
        .record_outcome(
            item.id,
            s.h.now(),
            HistoryResult::AddFailed,
            None,
            Some("Transmission이 응답하지 않았어요".into()),
            None,
        )
        .await
        .unwrap();
    let item = s.item("LIAR GAME - 26").await;
    assert_eq!(item.rule_id, None);

    let (status, body) = s.post(CMD, &item).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["message"].as_str().unwrap().contains("규칙 없이"));
    let (_, row) = s
        .call("GET", &format!("/api/history/{}", item.id), None)
        .await;
    assert_eq!(row["can_retry"], false);
    assert!(row["retry_blocked"].as_str().unwrap().contains("규칙 없이"));

    // A command accepted before, run now, ends the same way.
    CommandStore::new(s.h.db.clone())
        .accept(
            NewCommand {
                id: CMD.to_owned(),
                kind: "receive_once".to_owned(),
                payload: format!(r#"{{"item_id":{},"folder":""}}"#, item.id),
                subject: Some(item.id.to_string()),
            },
            s.h.now(),
        )
        .await
        .unwrap();
    s.h.tr.clear_calls();
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
    assert_ended_without_adding(&s, "규칙 없이", &item).await;
}

#[tokio::test]
async fn items_no_rule_picked_and_items_transmission_holds_are_not_retried() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let other = release("guid-other-3", 3, OTHER, "");
    // `Another Show` has no rule, so it is `no_match`; LIAR GAME is added by its rule.
    let s = Scene::new(
        &[&liar, &other],
        vec![rule("LIAR GAME", "LIAR GAME/Season 01")],
    )
    .await;
    let no_match = s.item("Another Show").await;
    let added = s.item("LIAR GAME - 26").await;
    assert_eq!(no_match.result, HistoryResult::NoMatch);
    assert_eq!(added.result, HistoryResult::Received);

    for (item, says) in [(&no_match, "규칙이 고르지 않아서"), (&added, "이미")] {
        let (status, body) = s.post(CMD, item).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{}", item.title);
        assert!(body["message"].as_str().unwrap().contains(says), "{body}");
    }
    assert_eq!(s.command(CMD).await.0, StatusCode::NOT_FOUND);
}

// --- outcomes ------------------------------------------------------------------------------

#[tokio::test]
async fn after_a_lost_answer_the_command_is_looked_up_by_the_same_id() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.h.tr.clear_calls();

    // The request reached the server; the answer never reached the browser.
    let _lost = s.post(CMD, &item).await;

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
async fn a_stopped_transmission_leaves_add_failed_with_a_reason_and_the_rule() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let mut s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.h.tr.clear_calls();
    s.post(CMD, &item).await;
    s.h.tr.stop().await;

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    let failed = s.item("LIAR GAME - 26").await;
    assert_eq!(failed.result, HistoryResult::AddFailed);
    assert_eq!(
        failed.rule_id, item.rule_id,
        "still the rule's, so it can be tried again"
    );
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
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.h.tr.reject_adds(Some("duplicate or corrupt torrent"));
    s.post(CMD, &item).await;

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
async fn a_retry_that_failed_can_be_tried_again_with_a_new_command() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.h.tr.reject_adds(Some("nope"));
    s.post(CMD, &item).await;
    s.run_commands().await;
    let failed = s.item("LIAR GAME - 26").await;
    assert_eq!(failed.result, HistoryResult::AddFailed);
    assert_eq!(failed.rule_id, item.rule_id);

    s.h.tr.reject_adds(None);
    let second = "1e2d3c4b-0000-4000-8000-000000000002";
    let (status, _) = s.post(second, &item).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the earlier command has ended"
    );
    s.run_commands().await;

    let received = s.item("LIAR GAME - 26").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert_eq!(received.rule_id, item.rule_id);
    assert_eq!(received.reason, None);
    assert_eq!(s.h.tr.torrents().len(), 1);
}

// --- getting the original link back -------------------------------------------------------

#[tokio::test]
async fn a_secret_in_a_query_of_the_channels_name_is_filled_back_from_the_channel() {
    // A download link on the channel's own host.
    let with_token = Release {
        guid: "guid-liar-26",
        title: LIAR.to_owned(),
        link: download_link(FEED_HOST, 26, LIAR, &format!("&token={SECRET}")),
    };
    let s = Scene::failing(&[&with_token], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    assert!(item.link.contains("token=***"), "{}", item.link);
    assert!(!item.link.contains(SECRET));
    let feed_reads = s.h.feeds.hits(FEED);

    s.post(CMD, &item).await;
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
    let with_own_token = Release {
        guid: "",
        title: LIAR.to_owned(),
        link: download_link(FEED_HOST, 26, LIAR, &format!("&token={own}")),
    };
    let s = Scene::failing(&[&with_own_token], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    assert!(item.identity_key.starts_with("link:"));
    let feed_reads = s.h.feeds.hits(FEED);

    s.post(CMD, &item).await;
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
async fn a_link_on_another_host_is_not_filled_with_the_channels_secret() {
    // A GUID identity says nothing about the link, so a filled link could not be
    // checked: the channel's token would go to another host.
    let own = "OWNTOKEN9876543210";
    let elsewhere = Release {
        guid: "guid-liar-26",
        title: LIAR.to_owned(),
        link: download_link("elsewhere.test", 26, LIAR, &format!("&token={own}")),
    };
    let s = Scene::failing(&[&elsewhere], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    assert!(item.link.contains("token=***"), "{}", item.link);
    let feed_reads = s.h.feeds.hits(FEED);

    s.post(CMD, &item).await;
    s.run_commands().await;

    // The link came from the current feed, with its own value.
    assert_eq!(s.h.feeds.hits(FEED), feed_reads + 1, "the feed was read");
    assert_eq!(s.adds().len(), 1);
    assert_eq!(s.adds()[0]["filename"], elsewhere.link);
    for call in s.h.tr.calls() {
        assert!(!call.args.to_string().contains(SECRET), "{call:?}");
    }
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Received
    );
    s.assert_secret_nowhere().await;
}

#[tokio::test]
async fn a_link_on_another_host_that_left_the_feed_is_not_received() {
    let own = "OWNTOKEN9876543210";
    let elsewhere = Release {
        guid: "guid-liar-26",
        title: LIAR.to_owned(),
        link: download_link("elsewhere.test", 26, LIAR, &format!("&token={own}")),
    };
    let other = release("guid-other-3", 3, OTHER, "");
    let s = Scene::failing(&[&elsewhere, &other], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.h.feeds.set_xml(FEED, &feed_xml(&[&other]));

    s.post(CMD, &item).await;
    s.run_commands().await;

    assert!(s.adds().is_empty(), "nothing goes to Transmission");
    let failed = s.item("LIAR GAME - 26").await;
    assert_eq!(failed.result, HistoryResult::AddFailed);
    assert!(failed
        .reason
        .unwrap()
        .contains("원래 링크를 되살리지 못했어요"));
    assert_eq!(s.command(CMD).await.1["state"], "failed");
    s.assert_secret_nowhere().await;
}

#[tokio::test]
async fn a_secret_under_another_name_is_found_in_the_current_feed() {
    // The channel's token, but under a name the channel's URL does not have.
    let with_passkey = release("guid-liar-26", 26, LIAR, &format!("&passkey={SECRET}"));
    let s = Scene::failing(&[&with_passkey], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    assert!(item.link.contains("passkey=***"), "{}", item.link);
    let feed_reads = s.h.feeds.hits(FEED);

    s.post(CMD, &item).await;
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
    let s = Scene::failing(&[&with_passkey, &other], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    // The item drops out of the feed.
    s.h.feeds.set_xml(FEED, &feed_xml(&[&other]));

    s.post(CMD, &item).await;
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
    let s = Scene::failing(&[&with_passkey], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.h.feeds.set_status(FEED, 500);

    s.post(CMD, &item).await;
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
async fn a_torrent_a_retry_added_is_kept_and_left_alone_by_the_next_cycles() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let other = release("guid-other-3", 3, OTHER, "");
    let s = Scene::failing(
        &[&liar, &other],
        vec![rule("LIAR GAME", "LIAR GAME/Season 01")],
    )
    .await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    s.run_commands().await;
    assert_eq!(s.h.tr.torrents().len(), 1);
    assert_eq!(s.h.tr.torrents()[0].name, "LIAR GAME S01E26.mkv");

    // The rule picks the item again each cycle and meets its own torrent: nothing
    // is removed, and the file, named already, is not named again.
    s.h.tr.clear_calls();
    for _ in 0..2 {
        let report = s.cycle().await;
        assert_eq!(report.duplicates, 1);
        assert!(report.removed.is_empty(), "{:?}", report.removed);
    }
    assert_eq!(s.h.tr.torrents().len(), 1);
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert!(s.h.tr.calls_of("torrent-rename-path").is_empty());
    let kept = s.item("LIAR GAME - 26").await;
    assert_eq!(kept.result, HistoryResult::Received);
    assert_eq!(kept.rule_id, item.rule_id);

    // Once the item leaves the feed, the ordinary cleanup applies to it again.
    s.feed(&[&other]);
    let report = s.cycle().await;
    assert_eq!(report.removed.len(), 1);
    assert_eq!(report.removed[0].hash, hash(26));
    assert!(s.h.tr.torrents().is_empty());
}

#[tokio::test]
async fn a_torrent_transmission_already_had_is_kept_too() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    // Transmission holds it already (the bot added it some time ago).
    s.h.tr
        .preload(FakeTorrent::new(&hash(26), LIAR).bot().status(6));

    s.post(CMD, &item).await;
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

/// Sets `item` up as the `한 번 받기` that `다시 받기` replaced left it: received
/// with no rule, its torrent in the folder and under the name a person chose.
/// Rows like it are still in the database, and the rule's cycle leaves their
/// torrents alone.
async fn received_by_hand(s: &Scene, item: &HistoryItem, torrent: FakeTorrent, note: Option<&str>) {
    let labelled = FakeTorrent {
        labels: vec![
            BOT_LABEL.to_owned(),
            item_label(&item.channel_id, &item.identity_key),
        ],
        ..torrent
    };
    let hash = labelled.hash.clone();
    s.h.tr.preload(labelled);
    s.h.history
        .record_outcome(
            item.id,
            s.h.now(),
            HistoryResult::Received,
            None,
            None,
            Some(hash),
        )
        .await
        .unwrap();
    if let Some(note) = note {
        s.h.history.note_received(item.id, note).await.unwrap();
    }
}

#[tokio::test]
async fn a_rule_that_later_selects_an_item_received_by_hand_neither_removes_nor_renames_it() {
    // Received by hand into a folder where trname has no name for the file.
    let odd = release("guid-odd", 9, "Some Special Collection.mkv", "");
    let s = Scene::new(&[&odd], unrelated_rule()).await;
    let item = s.item("Some Special").await;
    received_by_hand(
        &s,
        &item,
        FakeTorrent {
            download_dir: "/media/anime/Some Show".to_owned(),
            ..FakeTorrent::new(&hash(9), "Some Special Collection.mkv")
        },
        Some(NAME_NOT_DERIVED),
    )
    .await;

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
async fn a_rule_with_another_folder_does_not_rename_a_file_received_by_hand() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::new(&[&liar], unrelated_rule()).await;
    let item = s.item("LIAR GAME - 26").await;
    received_by_hand(
        &s,
        &item,
        FakeTorrent {
            download_dir: "/media/anime/LIAR GAME/Season 01".to_owned(),
            ..FakeTorrent::new(&hash(26), "LIAR GAME S01E26.mkv")
        },
        None,
    )
    .await;

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
async fn an_item_a_rule_added_after_the_request_ends_with_the_items_result_without_adding_again() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    // Before the worker gets to the command, the rule's own cycle adds the item.
    s.cycle().await;
    assert_eq!(s.h.tr.torrents()[0].name, "LIAR GAME S01E26.mkv");
    s.h.tr.clear_calls();

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    // Nothing is added or named a second time.
    assert!(s.adds().is_empty());
    assert!(s.h.tr.calls_of("torrent-rename-path").is_empty());
    assert_eq!(s.h.tr.torrents().len(), 1);
    // It ends with what the item says, without calling it a duplicate.
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");
    assert_eq!(view["outcome"]["reason"], Value::Null);
    let received = s.item("LIAR GAME - 26").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert_eq!(received.rule_id, item.rule_id);
    assert_eq!(received.reason, None);
}

#[tokio::test]
async fn items_the_rules_took_after_the_requests_end_with_the_items_results() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let other = release("guid-other-3", 3, OTHER, "");
    let s = Scene::failing(&[&liar, &other], picked_rules()).await;
    let liar_item = s.item("LIAR GAME - 26").await;
    let other_item = s.item("Another Show").await;
    s.post(CMD, &liar_item).await;
    let second = "1e2d3c4b-0000-4000-8000-000000000003";
    s.post(second, &other_item).await;
    // Before the worker gets to the commands, the rules take both items: one is
    // received, the other was in Transmission already.
    s.h.tr
        .preload(FakeTorrent::new(&hash(3), OTHER).bot().status(6));
    s.cycle().await;
    // Then Transmission refuses adds, which the commands must not even try.
    s.h.tr.reject_adds(Some("nope"));
    s.h.tr.clear_calls();

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(2));

    assert!(s.adds().is_empty());
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
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;

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
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
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
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    let store = CommandStore::new(s.h.db.clone());
    store.claim_next(s.h.now()).await.unwrap().unwrap();
    // Transmission took the torrent with the add's labels, then the worker died
    // before writing the result.
    s.h.tr.preload(taken_by_the_commands_add(&item));

    s.run_commands().await;

    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 1, "one torrent in Transmission");
    let held = s.item("LIAR GAME - 26").await;
    assert_eq!(held.result, HistoryResult::Received);
    assert_eq!(held.torrent_hash.as_deref(), Some(hash(26).as_str()));
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");
    // The rerun knows the torrent as its own add's by the command's label, so it
    // renames it and takes the label off.
    assert_eq!(torrents[0].name, "LIAR GAME S01E26.mkv");
    assert_eq!(
        torrents[0].labels,
        [BOT_LABEL, &item_label(&item.channel_id, &item.identity_key)]
    );
}

#[tokio::test]
async fn a_bot_torrent_without_the_commands_label_is_not_taken_as_its_own() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    // Put in the rule's folder before the command ran, by an add that was not
    // this command's (the cron before the switch, say).
    s.h.tr.preload(FakeTorrent {
        download_dir: "/media/anime/LIAR GAME/Season 01".to_owned(),
        ..FakeTorrent::new(&hash(26), LIAR).bot()
    });

    s.run_commands().await;

    let item = s.item("LIAR GAME - 26").await;
    assert_eq!(item.result, HistoryResult::Duplicate);
    assert_eq!(s.command(CMD).await.1["outcome"]["result"], "duplicate");
    assert!(s.h.tr.calls_of("torrent-rename-path").is_empty());
    assert_eq!(item.reason, None);
}

/// The torrent Transmission holds after taking this command's add for `item`
/// into `LIAR GAME/Season 01`.
fn taken_by_the_commands_add(item: &HistoryItem) -> FakeTorrent {
    FakeTorrent {
        download_dir: "/media/anime/LIAR GAME/Season 01".to_owned(),
        labels: vec![
            BOT_LABEL.to_owned(),
            item_label(&item.channel_id, &item.identity_key),
            format!("trss-cmd:{CMD}"),
        ],
        ..FakeTorrent::new(&hash(26), LIAR)
    }
}

#[tokio::test]
async fn a_cycle_run_while_a_command_is_left_running_removes_nothing() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    let store = CommandStore::new(s.h.db.clone());
    store.claim_next(s.h.now()).await.unwrap().unwrap();
    // Transmission took the torrent, then the worker died before writing the
    // result: history knows no hash for it. The item has left the feed, so
    // nothing else says the torrent belongs.
    s.h.tr.preload(taken_by_the_commands_add(&item));
    s.feed(&[]);

    // A restarted worker runs its cycle before it looks for commands.
    let report = s.cycle().await;

    assert_eq!(report.commands_running, 1);
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.h.tr.torrents().len(), 1);

    // The rerun meets the torrent and records its hash, so later cycles keep it
    // (here the item is back in the feed).
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
    assert_eq!(s.command(CMD).await.1["outcome"]["result"], "received");
    s.feed(&[&liar]);
    let report = s.cycle().await;
    assert_eq!(report.commands_running, 0);
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert_eq!(s.h.tr.torrents().len(), 1);
}

#[tokio::test]
async fn a_command_add_that_timed_out_after_transmission_took_it_is_received_on_the_next_look() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    // The item has left the feed, so that cycles in between do not pick it
    // again themselves.
    s.feed(&[]);
    // Transmission takes the torrent but answers only after the worker gave up.
    let late = s.h.tr.hold_answer("torrent-add");
    let impatient =
        s.h.worker()
            .with_transmission_timeout(Duration::from_millis(300));

    assert_eq!(
        s.run_commands_with(&impatient).await,
        CommandsOutcome::Ran(0)
    );

    // Not ended: the command waits for the next look, and a cycle meanwhile
    // does not call the torrent departed.
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "running");
    assert_eq!(
        s.h.tr.torrents().len(),
        1,
        "Transmission has it all the same"
    );
    assert_eq!(s.item("LIAR GAME - 26").await.torrent_hash, None);
    let report = s.cycle().await;
    assert!(report.removed.is_empty(), "{:?}", report.removed);

    // The next look adds it again; Transmission answers that it has it, with
    // its hash, and the command takes that torrent as its own.
    late.release_all();
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");
    let held = s.item("LIAR GAME - 26").await;
    assert_eq!(held.result, HistoryResult::Received);
    let torrent = s.h.tr.torrents().into_iter().next().unwrap();
    assert_eq!(held.torrent_hash.as_deref(), Some(torrent.hash.as_str()));
    assert_eq!(torrent.name, "LIAR GAME S01E26.mkv");

    // Kept by the cycles after that, while the item is in the feed.
    s.feed(&[&liar]);
    s.cycle().await;
    s.cycle().await;
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.h.tr.torrents().len(), 1);
}

/// A command whose first add got no answer though Transmission took the
/// torrent, left for the next look.
async fn after_an_unanswered_add(s: &Scene) -> Worker {
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    let late = s.h.tr.hold_answer("torrent-add");
    let impatient =
        s.h.worker()
            .with_transmission_timeout(Duration::from_millis(300));
    assert_eq!(
        s.run_commands_with(&impatient).await,
        CommandsOutcome::Ran(0)
    );
    late.release_all();
    impatient
}

#[tokio::test]
async fn a_refused_add_after_an_unanswered_one_leaves_the_command_for_the_next_look() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let worker = after_an_unanswered_add(&s).await;
    // Fetching a `.torrent` URL again can fail (a one-time link, a rate
    // limit) whatever Transmission holds.
    s.h.tr
        .reject_adds(Some("gotMetadataFromURL: http error 429"));

    assert_eq!(s.run_commands_with(&worker).await, CommandsOutcome::Ran(0));
    assert_eq!(s.command(CMD).await.1["state"], "running");
    let report = s.cycle().await;
    assert!(report.removed.is_empty(), "{:?}", report.removed);

    s.h.tr.reject_adds(None);
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
    assert_eq!(s.command(CMD).await.1["outcome"]["result"], "received");
    s.cycle().await;
    assert_eq!(s.h.tr.torrents().len(), 1);
}

#[tokio::test]
async fn a_refused_connection_after_an_unanswered_add_leaves_the_command_for_the_next_look() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let mut s = Scene::failing(&[&liar], picked_rules()).await;
    let worker = after_an_unanswered_add(&s).await;
    // Transmission restarting: the next start cannot connect.
    s.h.tr.stop().await;

    assert_eq!(s.run_commands_with(&worker).await, CommandsOutcome::Ran(0));
    assert_eq!(s.command(CMD).await.1["state"], "running");

    s.h.tr.restart().await;
    // The item has left the feed while the cycles run, so they do not pick it
    // themselves.
    s.feed(&[]);
    let first = s.cycle().await;
    let second = s.cycle().await;
    assert!(first.removed.is_empty(), "{:?}", first.removed);
    assert!(second.removed.is_empty(), "{:?}", second.removed);
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
    assert_eq!(s.command(CMD).await.1["outcome"]["result"], "received");
    s.feed(&[&liar]);
    s.cycle().await;
    assert_eq!(s.h.tr.torrents().len(), 1);
}

#[tokio::test]
async fn a_last_start_refused_after_an_unanswered_add_still_holds_the_next_cleanup() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let worker = after_an_unanswered_add(&s).await;
    s.h.tr
        .reject_adds(Some("gotMetadataFromURL: http error 429"));

    for _ in 2..MAX_ATTEMPTS {
        assert_eq!(s.run_commands_with(&worker).await, CommandsOutcome::Ran(0));
    }
    assert_eq!(s.run_commands_with(&worker).await, CommandsOutcome::Ran(1));
    assert_eq!(s.command(CMD).await.1["state"], "failed");

    // The first add's torrent is still unaccounted for.
    s.h.tr.reject_adds(None);
    let report = s.cycle().await;
    assert_eq!(report.commands_unconfirmed, 1);
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert_eq!(s.h.tr.torrents().len(), 1);
}

#[tokio::test]
async fn a_torrent_a_rule_met_between_the_starts_is_named_by_the_rule_and_the_command_ends_with_it()
{
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let worker = after_an_unanswered_add(&s).await;
    // The rule's own cycle runs before the next look and meets the command's
    // torrent as one Transmission already has.
    let report = s.cycle().await;
    assert_eq!(report.duplicates, 1);
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Duplicate
    );

    // The next look finds the item taken care of and adds nothing.
    s.h.tr.clear_calls();
    assert_eq!(s.run_commands_with(&worker).await, CommandsOutcome::Ran(1));
    assert!(s.adds().is_empty());
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done", "{view}");
    assert_eq!(view["outcome"]["result"], "duplicate", "{view}");
    let held = s.item("LIAR GAME - 26").await;
    assert_eq!(held.rule_id.as_deref(), Some(s.rule_of(0).id.as_str()));
    // The rule named the file in its own folder, and the command's label, which
    // the unanswered add put on the torrent, is off it now.
    let torrent = s.h.tr.torrents().into_iter().next().unwrap();
    assert_eq!(held.torrent_hash.as_deref(), Some(torrent.hash.as_str()));
    assert_eq!(torrent.name, "LIAR GAME S01E26.mkv");
    assert_eq!(
        torrent.labels,
        [BOT_LABEL, &item_label(&held.channel_id, &held.identity_key)]
    );
    s.cycle().await;
    assert_eq!(s.h.tr.torrents().len(), 1);
}

#[tokio::test]
async fn a_deleted_channel_after_an_unanswered_add_ends_the_command_at_once() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let worker = after_an_unanswered_add(&s).await;
    // Trying again cannot learn the hash any more: the save folder and the
    // original link came from the channel.
    let channel = &s.channel.channel;
    s.h.channels
        .delete_channel(&channel.id, channel.version, s.channel.rules.len())
        .await
        .unwrap();

    assert_eq!(s.run_commands_with(&worker).await, CommandsOutcome::Ran(1));
    let (_, command) = s.command(CMD).await;
    assert_eq!(command["state"], "failed");
    assert!(
        command["outcome"]["reason"]
            .as_str()
            .unwrap()
            .contains("채널이 삭제"),
        "{command}"
    );
    // The first add's torrent is still unaccounted for.
    let report = s.cycle().await;
    assert_eq!(report.commands_unconfirmed, 1);
    assert!(report.removed.is_empty(), "{:?}", report.removed);
}

#[tokio::test]
async fn a_torrent_whose_hash_was_never_learned_stays_while_its_item_is_in_the_feed() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let other = release("guid-other-3", 3, OTHER, "");
    let s = Scene::failing(
        &[&liar, &other],
        vec![rule("LIAR GAME", "LIAR GAME/Season 01")],
    )
    .await;
    let worker = after_an_unanswered_add(&s).await;
    // Every later start is refused, so history never learns the hash.
    s.h.tr
        .reject_adds(Some("gotMetadataFromURL: http error 429"));
    for _ in 2..=MAX_ATTEMPTS {
        s.run_commands_with(&worker).await;
    }
    assert_eq!(s.command(CMD).await.1["state"], "failed");
    assert_eq!(s.item("LIAR GAME - 26").await.torrent_hash, None);
    s.h.tr.reject_adds(None);
    // The rule is archived, so the cycles do not meet the torrent again and
    // learn its hash that way.
    s.archive_rule(0).await;

    // The torrent says which item it is for, and that item is still in the feed.
    let torrent = s.h.tr.torrents().into_iter().next().unwrap();
    let item = s.item("LIAR GAME - 26").await;
    assert!(torrent.labels.contains(&format!(
        "trss-item:{}:{}",
        item.channel_id, item.identity_key
    )));
    for _ in 0..3 {
        let report = s.cycle().await;
        assert!(report.removed.is_empty(), "{:?}", report.removed);
    }
    assert_eq!(s.h.tr.torrents().len(), 1);

    // Once the item leaves the feed, the ordinary cleanup applies to it again.
    s.feed(&[&other]);
    let report = s.cycle().await;
    assert_eq!(report.removed.len(), 1);
}

#[tokio::test]
async fn a_torrent_the_bot_did_not_add_is_not_taken_as_the_commands_own_after_an_unanswered_add() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    // Added by hand in Transmission, into the folder the rule gives the command.
    s.h.tr.preload(FakeTorrent {
        download_dir: "/media/anime/LIAR GAME/Season 01".to_owned(),
        ..FakeTorrent::new(&hash(26), LIAR)
    });
    let worker = after_an_unanswered_add(&s).await;

    assert_eq!(s.run_commands_with(&worker).await, CommandsOutcome::Ran(1));

    assert_eq!(s.command(CMD).await.1["outcome"]["result"], "duplicate");
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Duplicate
    );
    assert_eq!(s.h.tr.torrents()[0].name, LIAR, "its name is left alone");
}

#[tokio::test]
async fn another_items_bot_torrent_in_the_rules_folder_is_not_taken_as_the_commands_own() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    // The same torrent, put in the folder the rule gives the command by another
    // channel's rule for its own item.
    s.h.tr.preload(FakeTorrent {
        download_dir: "/media/anime/LIAR GAME/Season 01".to_owned(),
        labels: vec![
            BOT_LABEL.to_owned(),
            item_label("another-channel", "guid:another-item"),
        ],
        ..FakeTorrent::new(&hash(26), LIAR)
    });
    let worker = after_an_unanswered_add(&s).await;

    assert_eq!(s.run_commands_with(&worker).await, CommandsOutcome::Ran(1));

    assert_eq!(s.command(CMD).await.1["outcome"]["result"], "duplicate");
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Duplicate
    );
    assert_eq!(s.h.tr.torrents()[0].name, LIAR, "its name is left alone");
}

#[tokio::test]
async fn a_command_whose_adds_never_get_an_answer_ends_add_failed_and_holds_the_next_cleanup() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    let late = s.h.tr.hold_answer("torrent-add");
    let impatient =
        s.h.worker()
            .with_transmission_timeout(Duration::from_millis(300));

    // Every start but the last leaves the command for the next look.
    for _ in 1..MAX_ATTEMPTS {
        assert_eq!(
            s.run_commands_with(&impatient).await,
            CommandsOutcome::Ran(0)
        );
    }
    assert_eq!(
        s.run_commands_with(&impatient).await,
        CommandsOutcome::Ran(1)
    );

    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "failed");
    assert_eq!(view["outcome"]["result"], "add_failed");
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::AddFailed
    );
    assert_eq!(s.h.tr.torrents().len(), 1);

    // The next cycle does not call it departed. (The rule is archived so that
    // the cycle does not ask Transmission, whose answers are held, to add the
    // item again.)
    s.archive_rule(0).await;
    let report = s.cycle().await;
    assert_eq!(report.commands_unconfirmed, 1);
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert_eq!(s.h.tr.torrents().len(), 1);
    late.release_all();
}

#[tokio::test]
async fn two_workers_never_run_one_command_twice() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;

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
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    // The item has left the feed, so the cycle does not pick it itself.
    s.feed(&[]);

    // Another worker's cycle in progress (held at its first request to
    // Transmission): the lock between processes keeps them apart.
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
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;

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
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    // The item has left the feed, so the worker's own cycles do not pick it.
    s.feed(&[]);

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

    let (status, _) = s.post(CMD, &item).await;
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
    // The link carries the channel's own token, which the command fills back
    // in from the channel. The feed is down, so the process's first cycle
    // cannot pick the item itself.
    let with_token = Release {
        guid: "guid-liar-26",
        title: LIAR.to_owned(),
        link: download_link(FEED_HOST, 26, LIAR, &format!("&token={SECRET}")),
    };
    let s = Scene::failing(&[&with_token], picked_rules()).await;
    s.h.feeds.set_status(FEED, 500);
    let item = s.item("LIAR GAME - 26").await;
    // Accepted while no worker runs.
    s.post(CMD, &item).await;
    assert!(s.adds().is_empty());

    let mut worker = Proc::spawn(&s.h);
    wait_for("the command to be done", || async {
        s.command(CMD).await.1["state"] == "done"
    })
    .await;

    assert_eq!(s.adds().len(), 1, "{}", worker.output());
    assert_eq!(s.adds()[0]["filename"], with_token.link);
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

#[tokio::test]
async fn an_item_no_rule_picked_is_left_as_it_is_when_its_channel_is_gone_and_the_label_comes_off()
{
    let liar = release("guid-liar-26", 26, LIAR, "");
    let other = release("guid-other-3", 3, OTHER, "");
    let s = Scene::failing(
        &[&liar, &other],
        vec![rule("LIAR GAME", "LIAR GAME/Season 01")],
    )
    .await;
    let picked = s.item("LIAR GAME - 26").await;
    let unpicked = s.item("Another Show - 03").await;
    assert_eq!(unpicked.result, HistoryResult::NoMatch);
    let channel = &s.channel.channel;
    s.h.channels
        .delete_channel(&channel.id, channel.version, s.channel.rules.len())
        .await
        .unwrap();
    // Accepted before the channel went; an earlier start left a torrent with
    // its label.
    CommandStore::new(s.h.db.clone())
        .accept(
            NewCommand {
                id: CMD.to_owned(),
                kind: "receive_once".to_owned(),
                payload: format!(r#"{{"item_id":{}}}"#, unpicked.id),
                subject: Some(unpicked.id.to_string()),
            },
            s.h.now(),
        )
        .await
        .unwrap();
    CommandStore::new(s.h.db.clone())
        .claim_next(s.h.now())
        .await
        .unwrap()
        .unwrap();
    s.h.tr.preload(taken_by_the_commands_add(&picked));

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    let (_, command) = s.command(CMD).await;
    assert_eq!(command["state"], "failed", "{command}");
    assert!(command["outcome"]["reason"]
        .as_str()
        .unwrap()
        .contains("규칙이 고르지"));
    assert!(s.adds().is_empty());
    assert_eq!(s.item("Another Show - 03").await, unpicked);
    assert!(
        !s.h.tr.torrents()[0]
            .labels
            .iter()
            .any(|l| l.starts_with("trss-cmd:")),
        "{:?}",
        s.h.tr.torrents()[0].labels
    );
}

#[tokio::test]
async fn a_held_item_whose_channel_is_gone_ends_with_its_result_and_the_label_comes_off() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    store_legacy(&s, &item, "").await;
    CommandStore::new(s.h.db.clone())
        .claim_next(s.h.now())
        .await
        .unwrap()
        .unwrap();
    // An earlier start's add put the torrent in; the rule's cycle then met it
    // and recorded the item, as it does when it finds one already there.
    s.h.tr.preload(taken_by_the_commands_add(&item));
    s.h.tr.reject_adds(None);
    s.cycle().await;
    let held = s.item("LIAR GAME - 26").await;
    assert_eq!(held.result, HistoryResult::Duplicate);
    let channel = &s.channel.channel;
    s.h.channels
        .delete_channel(&channel.id, channel.version, s.channel.rules.len())
        .await
        .unwrap();

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    let (_, command) = s.command(CMD).await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "duplicate");
    assert_eq!(s.item("LIAR GAME - 26").await, held);
    assert!(s
        .h
        .tr
        .torrents()
        .iter()
        .all(|t| !t.labels.iter().any(|l| l.starts_with("trss-cmd:"))));
}

// --- receiving the past items of a subscription ------------------------------------------

/// Makes `phrase` a subscription rule of the scene's channel, as of the harness's clock.
async fn subscribe(
    s: &Scene,
    phrase: &str,
    directory: &str,
) -> trss_collect::store::channels::Rule {
    use trss_anissia::Anime;
    use trss_collect::store::channels::{NewSubscription, SubtitleMode};
    s.h.channels
        .create_subscription_rule(
            &s.channel.channel.id,
            rule(phrase, directory),
            NewSubscription {
                anime: Anime {
                    anime_no: 3320,
                    subject: phrase.to_owned(),
                    original_subject: None,
                    week: 3,
                    air_time: Some("22:00".to_owned()),
                    start_date: Some("2026-10-07".to_owned()),
                    end_date: None,
                    status: "ON".to_owned(),
                    fetched_at: s.h.now(),
                },
                subtitles: SubtitleMode::Undecided,
                creator: None,
                subscribed_at: s.h.now(),
            },
        )
        .await
        .unwrap()
}

fn liar(episode: u32) -> Release {
    let title = format!("[SubsPlease] LIAR GAME - {episode} (1080p) [ABCD12{episode}].mkv");
    let guid: &'static str = Box::leak(format!("guid-liar-{episode}").into_boxed_str());
    release(guid, episode, &title, "")
}

#[tokio::test]
async fn a_subscription_receives_nothing_that_was_recorded_before_it_until_the_user_picks() {
    let (liar25, liar26) = (liar(25), liar(26));
    let s = Scene::new(&[&liar25, &liar26], unrelated_rule()).await;
    assert_eq!(
        s.item("LIAR GAME - 25").await.result,
        HistoryResult::NoMatch
    );

    s.h.advance(1_000);
    let sub = subscribe(&s, "LIAR GAME", "anime/LIAR GAME/Season 01").await;

    // The next cycle sees both in the feed and the new rule matches them, but
    // they are past: creating the rule received nothing.
    s.cycle().await;
    assert!(s.adds().is_empty());
    assert!(s.h.tr.torrents().is_empty());
    for part in ["LIAR GAME - 25", "LIAR GAME - 26"] {
        let item = s.item(part).await;
        assert_eq!(item.result, HistoryResult::NoMatch, "{part}");
        assert_eq!(item.rule_id, None);
    }

    // The user ticked the 25th only.
    let item = s.item("LIAR GAME - 25").await;
    let (status, _) = s
        .post_payload(CMD, json!({ "item_id": item.id, "rule_id": sub.id }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 1);
    assert_eq!(torrents[0].hash, hash(25));
    assert_eq!(torrents[0].download_dir, "/media/anime/LIAR GAME/Season 01");
    assert_eq!(torrents[0].name, "LIAR GAME S01E25.mkv");
    let received = s.item("LIAR GAME - 25").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert_eq!(received.rule_id.as_deref(), Some(sub.id.as_str()));
    assert_eq!(s.command(CMD).await.1["outcome"]["result"], "received");

    // The 26th stays unreceived, cycle after cycle.
    s.cycle().await;
    assert_eq!(s.h.tr.torrents().len(), 1);
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::NoMatch
    );

    // A release that appears after the subscription is collected as usual.
    let liar27 = liar(27);
    s.feed(&[&liar25, &liar26, &liar27]);
    s.cycle().await;
    let hashes: Vec<String> = s.h.tr.torrents().into_iter().map(|t| t.hash).collect();
    assert_eq!(hashes.len(), 2, "{hashes:?}");
    assert!(hashes.contains(&hash(27)));
    assert_eq!(
        s.item("LIAR GAME - 27").await.result,
        HistoryResult::Received
    );
    s.assert_secret_nowhere().await;
}

#[tokio::test]
async fn a_plain_rule_still_takes_the_items_in_the_feed_the_cycle_has_recorded_without_a_rule() {
    // Only subscriptions hold back the past: a rule made by hand keeps taking
    // what the feed still holds, as before.
    let liar26 = liar(26);
    let s = Scene::new(&[&liar26], unrelated_rule()).await;
    s.h.advance(1_000);
    s.h.channels
        .create_rule(
            &s.channel.channel.id,
            rule("LIAR GAME", "LIAR GAME/Season 01"),
        )
        .await
        .unwrap();
    s.cycle().await;
    assert_eq!(s.h.tr.torrents().len(), 1);
}

#[tokio::test]
async fn a_request_for_a_rule_is_refused_when_the_rule_would_not_pick_the_item() {
    let (liar26, other) = (liar(26), release("guid-other-3", 3, OTHER, ""));
    let s = Scene::new(&[&liar26, &other], unrelated_rule()).await;
    s.h.advance(1_000);
    let sub = subscribe(&s, "LIAR GAME", "anime/LIAR GAME/Season 01").await;
    let item = s.item("Another Show").await;

    // The title does not match the rule.
    let (status, body) = s
        .post_payload(CMD, json!({ "item_id": item.id, "rule_id": sub.id }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["message"].as_str().unwrap().contains("고르지 않는"));
    // A rule that does not exist.
    let liar_item = s.item("LIAR GAME - 26").await;
    let (status, _) = s
        .post_payload(
            CMD,
            json!({ "item_id": liar_item.id, "rule_id": "no-such-rule" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Without a rule, an item no rule picked is still not retried.
    let (status, _) = s.post(CMD, &liar_item).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    assert_eq!(s.run_commands().await, CommandsOutcome::Idle);
    assert!(s.h.tr.torrents().is_empty());
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::NoMatch
    );
}

#[tokio::test]
async fn a_repeat_of_a_request_is_not_stored_twice_and_an_archived_rule_receives_nothing() {
    let liar26 = liar(26);
    let s = Scene::new(&[&liar26], unrelated_rule()).await;
    s.h.advance(1_000);
    let sub = subscribe(&s, "LIAR GAME", "anime/LIAR GAME/Season 01").await;
    let item = s.item("LIAR GAME - 26").await;
    let payload = json!({ "item_id": item.id, "rule_id": sub.id });

    assert_eq!(
        s.post_payload(CMD, payload.clone()).await.0,
        StatusCode::ACCEPTED
    );
    // The same request again is the stored command.
    assert_eq!(s.post_payload(CMD, payload.clone()).await.0, StatusCode::OK);

    let rule = s.h.channels.get_rule(&sub.id).await.unwrap().unwrap();
    s.h.channels
        .update_rule(
            &rule.id,
            rule.version,
            &rule.channel_id,
            RuleInput {
                state: RuleState::Archived,
                ..rule.to_input()
            },
        )
        .await
        .unwrap();
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
    assert!(s.h.tr.torrents().is_empty());
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "failed");
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::NoMatch
    );
}

// --- the rule detail's view of the past items ----------------------------------------------

/// What the rule detail sends to preview the stored rule as it is.
fn preview_of(channel_id: &str, rule: &trss_collect::store::channels::Rule) -> Value {
    json!({
        "channel_id": channel_id,
        "rule_id": rule.id,
        "rule": {
            "match": rule.r#match,
            "regex": rule.regex,
            "case_insensitive": rule.case_insensitive,
            "directory": rule.directory,
            "episode": rule.episode,
        },
    })
}

fn kind_of<'a>(preview: &'a Value, title_part: &str) -> &'a str {
    preview["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["title"].as_str().unwrap().contains(title_part))
        .unwrap_or_else(|| panic!("the preview lists no {title_part}: {preview}"))["kind"]
        .as_str()
        .unwrap()
}

async fn preview_rule(s: &Scene, rule: &trss_collect::store::channels::Rule) -> Value {
    let (status, preview) = s
        .call(
            "POST",
            "/api/rules/preview",
            Some(preview_of(&s.channel.channel.id, rule)),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    preview
}

#[tokio::test]
async fn the_preview_calls_the_items_the_cycle_leaves_alone_past() {
    let (liar25, liar26) = (liar(25), liar(26));
    let s = Scene::new(&[&liar25, &liar26], unrelated_rule()).await;
    s.h.advance(1_000);
    let sub = subscribe(&s, "LIAR GAME", "anime/LIAR GAME/Season 01").await;

    // The cycle leaves both alone ...
    s.cycle().await;
    assert!(s.adds().is_empty());

    // ... and the rule detail says the same of them, with the folder `받기`
    // would use.
    let preview = preview_rule(&s, &sub).await;
    assert_eq!(kind_of(&preview, "LIAR GAME - 25"), "past", "{preview}");
    assert_eq!(kind_of(&preview, "LIAR GAME - 26"), "past", "{preview}");
    assert_eq!(preview["counts"]["past"], 2, "{preview}");
    assert_eq!(preview["counts"]["mine"], 0, "{preview}");
    assert_eq!(
        preview["items"][0]["save_path"],
        "/media/anime/LIAR GAME/Season 01"
    );

    // A release first seen after the subscription is the rule's own.
    let liar27 = liar(27);
    s.feed(&[&liar25, &liar26, &liar27]);
    s.cycle().await;
    assert_eq!(s.h.tr.torrents().len(), 1);
    let preview = preview_rule(&s, &sub).await;
    assert_eq!(kind_of(&preview, "LIAR GAME - 27"), "mine", "{preview}");
    assert_eq!(kind_of(&preview, "LIAR GAME - 26"), "past", "{preview}");
}

#[tokio::test]
async fn a_past_item_of_the_rule_detail_is_received_by_that_rule_and_then_reads_as_received() {
    let (liar25, liar26) = (liar(25), liar(26));
    let s = Scene::new(&[&liar25, &liar26], unrelated_rule()).await;
    s.h.advance(1_000);
    let sub = subscribe(&s, "LIAR GAME", "anime/LIAR GAME/Season 01").await;
    s.cycle().await;

    let preview = preview_rule(&s, &sub).await;
    let id = preview["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "past" && i["title"].as_str().unwrap().contains("LIAR GAME - 25"))
        .expect("a past item to receive")["id"]
        .as_i64()
        .unwrap();

    // `받기` of that row, long after the subscribe flow ended.
    let (status, _) = s
        .post_payload(CMD, json!({ "item_id": id, "rule_id": sub.id }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
    assert_eq!(s.command(CMD).await.1["outcome"]["result"], "received");
    assert_eq!(s.h.tr.torrents().len(), 1);
    assert_eq!(
        s.h.tr.torrents()[0].download_dir,
        "/media/anime/LIAR GAME/Season 01"
    );

    // The row stops being past: the rule received it. The other still is.
    let after = preview_rule(&s, &sub).await;
    assert_eq!(kind_of(&after, "LIAR GAME - 25"), "mine", "{after}");
    assert_eq!(kind_of(&after, "LIAR GAME - 26"), "past", "{after}");
    assert_eq!(after["counts"]["past"], 1);
}

#[tokio::test]
async fn an_item_that_failed_without_any_rule_is_not_said_to_belong_to_another_rule() {
    let liar26 = liar(26);
    let s = Scene::new(&[&liar26], unrelated_rule()).await;
    s.h.advance(1_000);
    let sub = subscribe(&s, "LIAR GAME", "anime/LIAR GAME/Season 01").await;
    let item = s.item("LIAR GAME - 26").await;
    s.h.history
        .record_outcome(
            item.id,
            s.h.now(),
            HistoryResult::AddFailed,
            None,
            Some("Transmission이 응답하지 않았어요".into()),
            None,
        )
        .await
        .unwrap();

    let (status, body) = s
        .post_payload(CMD, json!({ "item_id": item.id, "rule_id": sub.id }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let message = body["message"].as_str().unwrap();
    assert!(message.contains("규칙 없이"), "{message}");
    assert!(!message.contains("다른 규칙"), "{message}");
}

// --- what a rule missed while it was off ------------------------------------------------------

/// The `n`-th rule of the scene as stored now.
async fn stored_rule(s: &Scene, n: usize) -> trss_collect::store::channels::Rule {
    s.h.channels
        .get_rule(&s.rule_of(n).id)
        .await
        .unwrap()
        .unwrap()
}

/// `영상 받기` turned off or on, as of the harness's clock.
async fn switch_video(s: &Scene, n: usize, on: bool) {
    let rule = stored_rule(s, n).await;
    s.h.channels
        .set_video_receiving(&rule.id, rule.version, on, s.h.now())
        .await
        .unwrap();
}

#[tokio::test]
async fn an_item_first_seen_while_a_rule_was_paused_is_left_to_the_user_when_it_resumes() {
    let (liar25, liar26) = (liar(25), liar(26));
    let s = Scene::new(&[], vec![rule("LIAR GAME", "LIAR GAME/Season 01")]).await;

    switch_video(&s, 0, false).await;
    s.feed(&[&liar25]);
    s.cycle().await;
    assert!(s.h.tr.torrents().is_empty());
    assert_eq!(
        s.item("LIAR GAME - 25").await.result,
        HistoryResult::NoMatch
    );

    s.h.advance(1_000);
    switch_video(&s, 0, true).await;

    // Turned on, the rule does not take what appeared while it was off ...
    s.cycle().await;
    assert!(s.adds().is_empty());
    assert!(s.h.tr.torrents().is_empty());
    assert_eq!(
        s.item("LIAR GAME - 25").await.result,
        HistoryResult::NoMatch
    );

    // ... and its detail lists the item as past, as the cycle treats it.
    let rule = stored_rule(&s, 0).await;
    let preview = preview_rule(&s, &rule).await;
    assert_eq!(kind_of(&preview, "LIAR GAME - 25"), "past", "{preview}");
    assert_eq!(preview["items"][0]["past_cause"], "resumed");
    assert_eq!(preview["counts"]["past"], 1);

    // `받기` receives it with the rule.
    let item = s.item("LIAR GAME - 25").await;
    let (status, _) = s
        .post_payload(CMD, json!({ "item_id": item.id, "rule_id": rule.id }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
    let received = s.item("LIAR GAME - 25").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert_eq!(received.rule_id.as_deref(), Some(rule.id.as_str()));
    assert_eq!(
        s.h.tr.torrents()[0].download_dir,
        "/media/anime/LIAR GAME/Season 01"
    );

    // What appears after it resumed is received on its own.
    s.feed(&[&liar25, &liar26]);
    s.cycle().await;
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Received
    );
    assert_eq!(s.h.tr.torrents().len(), 2);
    let after = preview_rule(&s, &rule).await;
    assert_eq!(kind_of(&after, "LIAR GAME - 26"), "mine", "{after}");
}

#[tokio::test]
async fn a_rule_that_was_never_paused_takes_the_recorded_items_as_before() {
    // The rule is made after the cycle recorded the item without a rule.
    let liar26 = liar(26);
    let s = Scene::new(&[&liar26], unrelated_rule()).await;
    s.h.advance(1_000);
    s.h.channels
        .create_rule(
            &s.channel.channel.id,
            rule("LIAR GAME", "LIAR GAME/Season 01"),
        )
        .await
        .unwrap();
    s.cycle().await;
    assert_eq!(s.h.tr.torrents().len(), 1);

    // Pausing alone holds nothing back: the rule has never been turned back on.
    let created =
        s.h.channels
            .list_channels_with_rules()
            .await
            .unwrap()
            .remove(0)
            .rules
            .into_iter()
            .find(|r| r.r#match.as_deref() == Some("LIAR GAME"))
            .unwrap();
    assert_eq!(created.resumed_at, None);
    let preview = preview_rule(&s, &created).await;
    assert_eq!(kind_of(&preview, "LIAR GAME - 26"), "mine", "{preview}");
    assert_eq!(preview["counts"]["past"], 0);
}

#[tokio::test]
async fn an_item_first_seen_while_a_rule_was_archived_is_left_to_the_user_after_the_restore() {
    let (liar25, liar26) = (liar(25), liar(26));
    let s = Scene::new(&[], vec![rule("LIAR GAME", "LIAR GAME/Season 01")]).await;
    let id = s.rule_of(0).id.clone();
    let command = |name: &str, direction: &str| {
        json!({ "id": name, "kind": "rule_archive",
                "payload": { "rule_id": id, "direction": direction } })
    };

    let (status, body) = s
        .call(
            "POST",
            "/api/commands",
            Some(command("0b7d5a44-6c1e-4c62-9a6a-3f0c1d2e4b01", "archive")),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
    assert_eq!(stored_rule(&s, 0).await.state, RuleState::Archived);

    s.feed(&[&liar25]);
    s.cycle().await;
    assert!(s.h.tr.torrents().is_empty());

    s.h.advance(1_000);
    let (status, body) = s
        .call(
            "POST",
            "/api/commands",
            Some(command("0b7d5a44-6c1e-4c62-9a6a-3f0c1d2e4b02", "restore")),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
    let restored = stored_rule(&s, 0).await;
    assert_eq!(restored.state, RuleState::Active);
    assert!(restored.resumed_at.is_some());

    // Restored, it leaves the item that came while it was archived, and the
    // detail offers it as past.
    s.cycle().await;
    assert!(s.h.tr.torrents().is_empty());
    let preview = preview_rule(&s, &restored).await;
    assert_eq!(kind_of(&preview, "LIAR GAME - 25"), "past", "{preview}");

    // A later release is received as usual.
    s.feed(&[&liar25, &liar26]);
    s.cycle().await;
    assert_eq!(s.h.tr.torrents().len(), 1);
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Received
    );
}

// --- the worker's heartbeat while a command holds the lock (ticket 0021) -----------------------

#[tokio::test]
async fn a_long_command_beats_and_the_board_shows_no_stall_meanwhile() {
    use trss_collect::store::status::StatusStore;
    use trss_core::heartbeat::HeartbeatStore;
    use trss_web::{status_api::board, AppState};

    let odd = release("guid-odd", 9, "Some Special Collection.mkv", "");
    let s = Scene::failing(&[&odd], vec![rule("Some Special", "Some Show")]).await;
    let item = s.item("Some Special").await;
    s.post(CMD, &item).await;
    let status = StatusStore::new(s.h.db.clone());
    let heartbeat = HeartbeatStore::new(s.h.db.clone());
    // The set-up cycle ended; its worker takes five minutes between cycles.
    status.record_cycle_interval(300_000).await.unwrap();
    let state = AppState::new(s.h.db.clone());
    let stalled = |now| {
        let state = state.clone();
        async move { board(&state, now, 0).await.unwrap().cycle.unwrap().stalled }
    };

    let gate = s.h.tr.hold("torrent-get");
    let worker = s.h.worker().with_heartbeat_every(Duration::from_millis(10));
    let running = tokio::spawn(async move { worker.run_commands(&CancellationToken::new()).await });
    gate.wait_arrived().await;
    let started = s.h.now();
    let held = heartbeat.read().await.unwrap().expect("a beat");
    assert_eq!(held.held_since, Some(started));

    // The command goes on for a quarter of an hour (three intervals since the
    // cycle began): the worker is busy, and the board does not say it stopped.
    s.h.advance(15 * 60_000);
    let now = s.h.now();
    for _ in 0..200 {
        if heartbeat.read().await.unwrap().unwrap().beat_at == now {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let beat = heartbeat.read().await.unwrap().unwrap();
    assert_eq!((beat.beat_at, beat.held_since), (now, Some(started)));
    assert!(!stalled(now).await);

    gate.release_all();
    assert_eq!(running.await.unwrap().unwrap(), CommandsOutcome::Ran(1));
    let done = heartbeat.read().await.unwrap().unwrap();
    assert_eq!(done.held_since, None);
}

// --- commands at once, beside the cycles (ticket 0031) -----------------------------------------

/// The state of command `id` as the screen reads it.
async fn state_of(s: &Scene, id: &str) -> String {
    s.command(id).await.1["state"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn a_command_the_web_accepts_starts_within_a_second_without_waiting_for_a_look() {
    const WOKEN: &str = "5a0c2a71-0000-4000-8000-000000000031";
    const UNHEARD: &str = "5a0c2a71-0000-4000-8000-000000000032";
    let liar = release("guid-liar-26", 26, LIAR, "");
    let other = release("guid-other-3", 3, OTHER, "");
    let s = Scene::failing(&[&liar, &other], picked_rules()).await;
    // The items have left the feed, so the worker's own cycles do not take them.
    s.feed(&[]);
    let wake = trss_core::wake::wake_path_for(&s.h.db_path());
    // The worker would look by itself only once an hour: only the web's wake
    // can explain a start within the second.
    let worker =
        s.h.worker()
            .with_command_poll(Duration::from_secs(3600))
            .with_wake_socket(wake.clone());
    let cancel = CancellationToken::new();
    let running = tokio::spawn({
        let (worker, cancel) = (worker.clone(), cancel.clone());
        async move { worker.run(cancel).await }
    });
    wait_for("the worker's wake socket", || {
        let wake = wake.clone();
        async move { wake.exists() }
    })
    .await;
    // Its first cycle and first look have found nothing to do.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Accepted by a web that does not wake the worker: nothing starts it.
    let (status, _) = s.post(UNHEARD, &s.item("Another Show").await).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(state_of(&s, UNHEARD).await, "pending");

    // Accepted by a web that wakes it: under way within the second.
    let web = WebApi::with_state(trss_web::AppState::new(s.h.db.clone()).with_worker_wake(wake));
    let item = s.item("LIAR GAME - 26").await;
    let accepted = Instant::now();
    let (status, _, body) = web
        .call(
            "POST",
            "/api/commands",
            Some(json!({ "id": WOKEN, "kind": "receive_once", "payload": { "item_id": item.id } })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    while state_of(&s, WOKEN).await == "pending" {
        assert!(
            accepted.elapsed() < Duration::from_secs(1),
            "the command did not start within a second"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // The look the wake started takes every open command.
    wait_for("both commands to end", || async {
        state_of(&s, WOKEN).await == "done" && state_of(&s, UNHEARD).await == "done"
    })
    .await;
    assert_eq!(s.command(WOKEN).await.1["outcome"]["result"], "received");
    assert_eq!(s.command(UNHEARD).await.1["outcome"]["result"], "received");
    assert_eq!(s.h.tr.torrents().len(), 2);

    cancel.cancel();
    running.await.unwrap();
}

#[tokio::test]
async fn a_command_runs_to_its_end_while_the_same_workers_cycle_is_held_in_transmission() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    // One worker, as a process has one: its cycle and its commands side by side.
    let worker = s.h.worker();
    // The cycle is held at its first request to Transmission, which no
    // command makes.
    let gate = s.h.tr.hold("session-set");
    s.h.advance(300_000);
    let cycle = tokio::spawn({
        let worker = worker.clone();
        async move { worker.tick(&CancellationToken::new()).await }
    });
    gate.wait_arrived().await;

    s.post(CMD, &item).await;
    let ran = tokio::time::timeout(Duration::from_secs(10), s.run_commands_with(&worker))
        .await
        .expect("the command waited for the cycle");

    assert_eq!(ran, CommandsOutcome::Ran(1));
    assert!(!cycle.is_finished());
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");

    // The cycle then goes on and keeps the command's torrent.
    gate.release_all();
    let TickOutcome::Ran(report) = cycle.await.unwrap().unwrap() else {
        panic!("expected a cycle")
    };
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.h.tr.torrents().len(), 1);
    assert_eq!(s.h.tr.torrents()[0].name, "LIAR GAME S01E26.mkv");
}

#[tokio::test]
async fn the_cycles_removal_waits_until_a_commands_add_is_recorded() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    // The item has left the feed: nothing but history will say the command's
    // torrent belongs, so a removal between its add and its record would take
    // it away.
    s.feed(&[]);
    let worker = s.h.worker();
    // Transmission takes the torrent and is slow to say so.
    let late = s.h.tr.hold_answer("torrent-add");
    let commands = tokio::spawn({
        let worker = worker.clone();
        async move { worker.run_commands(&CancellationToken::new()).await }
    });
    late.wait_arrived().await;
    assert_eq!(s.h.tr.torrents().len(), 1, "Transmission holds it");

    // The same worker's cycle reads the feed and comes to its removal.
    let hits = s.h.feeds.hits(FEED);
    s.h.advance(300_000);
    let cycle = tokio::spawn({
        let worker = worker.clone();
        async move { worker.tick(&CancellationToken::new()).await }
    });
    wait_for("the cycle to read the feed", || async {
        s.h.feeds.hits(FEED) > hits
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // It waits for the command, and removes nothing meanwhile.
    assert!(!cycle.is_finished());
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.h.tr.torrents().len(), 1);

    late.release_all();
    assert_eq!(commands.await.unwrap().unwrap(), CommandsOutcome::Ran(1));
    let TickOutcome::Ran(report) = cycle.await.unwrap().unwrap() else {
        panic!("expected a cycle")
    };

    // The command met its torrent and named it; history holds its hash.
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");
    let held = s.item("LIAR GAME - 26").await;
    assert_eq!(held.result, HistoryResult::Received);
    assert_eq!(held.torrent_hash.as_deref(), Some(hash(26).as_str()));
    // Only then did the removal look: what it did with the departed item's
    // torrent is the ordinary cleanup, after everything the command did.
    let calls: Vec<String> = s.h.tr.calls().into_iter().map(|c| c.method).collect();
    let renamed = calls
        .iter()
        .position(|m| m == "torrent-rename-path")
        .expect("the command named the file");
    if let Some(removed) = calls.iter().position(|m| m == "torrent-remove") {
        assert!(removed > renamed, "{calls:?}");
        let last_of_the_command = calls
            .iter()
            .rposition(|m| m == "torrent-set")
            .unwrap_or(renamed);
        assert!(removed > last_of_the_command, "{calls:?}");
        assert_eq!(report.removed.len(), 1);
    }
    assert_eq!(report.commands_running, 0);
    assert_eq!(report.commands_unconfirmed, 0);
}

const SONO: &str = "[SubsPlease] Sono Bisque Doll - 13 (1080p) [ABCD1236].mkv";

/// A rule with an episode conversion of 12 (release 13 is named episode 24).
fn counting_on_rules() -> Vec<RuleInput> {
    vec![RuleInput {
        episode: 12,
        ..rule("Sono Bisque Doll", "Sono Bisque Doll/Season 01")
    }]
}

#[tokio::test]
async fn a_rerun_leaves_the_name_its_earlier_start_gave_and_converts_no_episode_twice() {
    let sono = release("guid-sono-13", 13, SONO, "");
    let s = Scene::failing(&[&sono], counting_on_rules()).await;
    let item = s.item("Sono Bisque Doll - 13").await;
    s.post(CMD, &item).await;
    let store = CommandStore::new(s.h.db.clone());
    store.claim_next(s.h.now()).await.unwrap().unwrap();
    // Transmission took the torrent and the earlier start renamed it, then the
    // worker died before writing the result.
    s.h.tr.preload(FakeTorrent {
        download_dir: "/media/anime/Sono Bisque Doll/Season 01".to_owned(),
        labels: vec![
            BOT_LABEL.to_owned(),
            item_label(&item.channel_id, &item.identity_key),
            format!("trss-cmd:{CMD}"),
        ],
        ..FakeTorrent::new(&hash(13), "Sono Bisque Doll S01E24.mkv")
    });

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    assert_eq!(s.command(CMD).await.1["outcome"]["result"], "received");
    assert!(s.h.tr.calls_of("torrent-rename-path").is_empty());
    assert_eq!(
        s.h.tr.torrent(&hash(13)).name,
        "Sono Bisque Doll S01E24.mkv"
    );
}

#[tokio::test]
async fn a_retry_and_the_cycles_add_of_the_same_item_go_in_turn() {
    let sono = release("guid-sono-13", 13, SONO, "");
    // The item stays in the feed, so the cycle adds it again itself.
    let s = Scene::failing(&[&sono], counting_on_rules()).await;
    let item = s.item("Sono Bisque Doll - 13").await;
    s.post(CMD, &item).await;
    let worker = s.h.worker();

    // The retry has added the torrent and is about to rename it.
    let rename = s.h.tr.hold("torrent-rename-path");
    let commands = tokio::spawn({
        let worker = worker.clone();
        async move { worker.run_commands(&CancellationToken::new()).await }
    });
    rename.wait_arrived().await;
    assert_eq!(s.adds().len(), 1);

    // The same worker's cycle picks the item again: its add waits for the
    // retry's turn at the work folder.
    let hits = s.h.feeds.hits(FEED);
    s.h.advance(300_000);
    let cycle = tokio::spawn({
        let worker = worker.clone();
        async move { worker.tick(&CancellationToken::new()).await }
    });
    wait_for("the cycle to read the feed", || async {
        s.h.feeds.hits(FEED) > hits
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(s.adds().len(), 1, "the cycle's add waits");
    assert!(!cycle.is_finished());

    rename.release_all();
    assert_eq!(commands.await.unwrap().unwrap(), CommandsOutcome::Ran(1));
    let TickOutcome::Ran(_) = cycle.await.unwrap().unwrap() else {
        panic!("expected a cycle")
    };

    // One conversion of the episode, whoever renamed.
    assert_eq!(s.command(CMD).await.1["outcome"]["result"], "received");
    assert_eq!(s.h.tr.torrents().len(), 1);
    assert_eq!(
        s.h.tr.torrent(&hash(13)).name,
        "Sono Bisque Doll S01E24.mkv"
    );
    assert_eq!(s.h.tr.calls_of("torrent-rename-path").len(), 1);
}
