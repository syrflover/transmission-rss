//! `다시 받기` (a retry, stored as the `receive_once` command) from the record
//! tab, end to end: the real commands and history HTTP API, the real worker
//! code and Transmission client, a fake Transmission and fake RSS feeds.
//! Ticket 1-app-owned-collection/0008's completion rows.
//!
//! What a retry decides (where it adds, what it names the file, which torrent
//! is its own, which request ends at once, how the original link is got back)
//! is tested in trss-collect (`commands::receive_once`, ADR 0015). This file
//! keeps what only a process shows: that a command the web accepted reaches
//! the worker and ends, the order of cycles and commands, a worker
//! that restarts or dies, and a cycle that must not remove what a command
//! has put in.
//!
//! The items retried here were picked by a rule whose add Transmission refused
//! (`Scene::failing`), so they are `add_failed` with the rule recorded, as the
//! rule's own cycle leaves them.
//!
//! Tokens are made up (`common::SECRET`).

use crate::common;

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
use trss::{
    wake::Wakes,
    web::AppState,
    worker::{CommandsOutcome, CycleReport, TickOutcome, Worker},
};
use trss_collect::{
    commands::receive_once::NAME_NOT_DERIVED,
    store::{
        channels::{ChannelWithRules, RuleInput, RuleState},
        history::{HistoryItem, HistoryResult},
    },
};
use trss_core::{
    commands::{CommandState, CommandStore, MAX_ATTEMPTS},
    Db,
};
use trss_transmission::item_label;

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
    /// cycle run in which Transmission refuses every add: the items the rules
    /// pick are `add_failed` with their rule recorded, which is what `다시 받기`
    /// is for. Transmission takes adds again when this returns.
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

    /// Runs one collection cycle of `worker`, a period after the last.
    async fn cycle_with(&self, worker: &Worker) -> CycleReport {
        self.h.advance(300_000);
        match worker.tick(&CancellationToken::new()).await.unwrap() {
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

/// The path of a retry through the web and the worker: accepted pending, run by
/// the worker, ended `done`, read back by the screen. What the retry adds,
/// where and under which name is trss-collect's
/// (`commands::receive_once::retry_tests`).
#[tokio::test]
async fn a_failed_item_is_added_again_by_the_worker_and_the_screen_reads_the_end() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let other = release("guid-other-3", 3, OTHER, "");
    let s = Scene::failing(&[&liar, &other], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    assert_eq!(item.result, HistoryResult::AddFailed);
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

    assert_eq!(s.h.tr.torrents().len(), 1);
    assert_eq!(s.h.tr.torrents()[0].hash, hash(26));
    // Its neighbour was not touched.
    assert_eq!(
        s.item("Another Show").await.result,
        HistoryResult::AddFailed
    );

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

// --- items that cannot be retried ----------------------------------------------------------

// --- outcomes ------------------------------------------------------------------------------

// --- getting the original link back -------------------------------------------------------

// --- the next cycle ------------------------------------------------------------------------

// --- restarts and a look that fails -------------------------------------------------------------

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

    // The rerun met the torrent and took it as its own by the command's label
    // (trss-collect decides that: `commands::receive_once::unanswered_tests`).
    assert_eq!(s.h.tr.torrents().len(), 1, "one torrent in Transmission");
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Received
    );
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");
}

/// A look that cannot write the result (here the database refusing it) leaves
/// the command `running` and not ended, and the next look runs it again
/// (`Ran::NotNow`; trss-collect's `Retry::Store`).
#[tokio::test]
async fn a_command_whose_result_could_not_be_written_is_run_again_at_a_later_look() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    s.post(CMD, &item).await;
    let sql = |sql: &str| {
        rusqlite::Connection::open(s.h.db_path())
            .unwrap()
            .execute_batch(sql)
            .unwrap()
    };
    sql("CREATE TRIGGER no_result BEFORE UPDATE ON history_items
         WHEN NEW.result = 'received'
         BEGIN SELECT RAISE(ABORT, 'injected'); END;");

    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(0));

    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "running", "not ended: {view}");
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::AddFailed
    );
    sql("DROP TRIGGER no_result;");

    // The next look runs it again; the torrent its first start put in is its own.
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");
    assert_eq!(s.h.tr.torrents().len(), 1);
    assert_eq!(
        s.item("LIAR GAME - 26").await.result,
        HistoryResult::Received
    );
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

    // The next look adds it again, and the command ends (what it makes of
    // Transmission's answer is trss-collect's: `unanswered_tests`).
    late.release_all();
    assert_eq!(s.run_commands().await, CommandsOutcome::Ran(1));

    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");
    assert_eq!(s.h.tr.torrents().len(), 1);

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
async fn the_running_worker_picks_up_a_command_without_waiting_for_the_next_cycle() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    // The item has left the feed, so the worker's own cycles do not pick it.
    s.feed(&[]);

    // A cycle every hour; the web rings the worker's commands' loop.
    let env =
        s.h.worker_env_with(&[("TRSS_WORKER_INTERVAL_SECS", "3600")]);
    let wakes = Wakes::default();
    let web = WebApi::with_state(AppState::new(s.h.db.clone()).with_wakes(wakes.clone()));
    let worker = s.h.worker_with(s.h.db.clone(), &env).with_wakes(wakes);
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

    let (status, _, body) = web
        .call(
            "POST",
            "/api/commands",
            Some(json!({ "id": CMD, "kind": "receive_once", "payload": { "item_id": item.id } })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
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

#[tokio::test]
async fn a_command_left_for_later_starts_again_by_itself_once_its_wait_is_over() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    // The item has left the feed, so the worker's own cycles do not take it.
    s.feed(&[]);
    s.post(CMD, &s.item("LIAR GAME - 26").await).await;
    let store = CommandStore::new(s.h.db.clone());
    // Transmission takes the add but answers only after the worker gave up,
    // so the first start leaves the command for later.
    let late = s.h.tr.hold_answer("torrent-add");

    // A cycle an hour and no ring: what starts the command again is its wait.
    let env =
        s.h.worker_env_with(&[("TRSS_WORKER_INTERVAL_SECS", "3600")]);
    let worker =
        s.h.worker_with(s.h.db.clone(), &env)
            .with_transmission_timeout(Duration::from_millis(300))
            // Long enough for the gate to open before the second start, whose
            // add would otherwise be held too.
            .with_command_retry(Duration::from_secs(2));
    let cancel = CancellationToken::new();
    let running = {
        let (worker, cancel) = (worker.clone(), cancel.clone());
        tokio::spawn(async move { worker.run(cancel).await })
    };
    wait_for("the first start to be left for later", || async {
        store.get(CMD).await.unwrap().unwrap().add_unconfirmed
    })
    .await;
    late.release_all();

    wait_for("the command to be done", || async {
        state_of(&s, CMD).await == "done"
    })
    .await;
    let command = store.get(CMD).await.unwrap().unwrap();
    assert_eq!(command.attempts, 2);
    assert_eq!(command.outcome.unwrap().result, "received");
    assert_eq!(s.h.tr.torrents().len(), 1);
    cancel.cancel();
    running.await.unwrap();
}

#[tokio::test]
async fn a_look_that_fails_looks_again_by_itself() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    // The item has left the feed, so the worker's own cycles do not take it.
    s.feed(&[]);
    s.post(CMD, &s.item("LIAR GAME - 26").await).await;
    // The worker's looks fail while the table is away.
    let rename = |from: &'static str, to: &'static str| {
        s.h.db.run(move |c| {
            c.execute_batch(&format!("ALTER TABLE {from} RENAME TO {to}"))
                .map_err(trss_core::DbError::from)
        })
    };
    rename("commands", "commands_away").await.unwrap();

    // A cycle an hour and no ring: what finds the command is the look again.
    let env =
        s.h.worker_env_with(&[("TRSS_WORKER_INTERVAL_SECS", "3600")]);
    let worker =
        s.h.worker_with(s.h.db.clone(), &env)
            .with_command_retry(Duration::from_millis(200));
    let cancel = CancellationToken::new();
    let running = {
        let (worker, cancel) = (worker.clone(), cancel.clone());
        tokio::spawn(async move { worker.run(cancel).await })
    };
    tokio::time::sleep(Duration::from_millis(600)).await;
    rename("commands_away", "commands").await.unwrap();
    assert_eq!(state_of(&s, CMD).await, "pending");

    wait_for("the command to be done", || async {
        state_of(&s, CMD).await == "done"
    })
    .await;
    let command = CommandStore::new(s.h.db.clone())
        .get(CMD)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(command.attempts, 1);
    assert_eq!(s.adds().len(), 1);
    cancel.cancel();
    running.await.unwrap();
}

#[tokio::test]
async fn at_most_eight_commands_run_at_once_and_the_next_starts_when_one_ends() {
    let titles: Vec<String> = (1..=9)
        .map(|n| format!("[SubsPlease] Show {n} - 01 (1080p) [ABCD{n:04}].mkv"))
        .collect();
    let guids: Vec<&'static str> = (1..=9)
        .map(|n| &*Box::leak(format!("guid-show-{n}").into_boxed_str()))
        .collect();
    let releases: Vec<Release> = (0..9)
        .map(|i| release(guids[i], 100 + i as u32, &titles[i], ""))
        .collect();
    let rules = (1..=9)
        .map(|n| rule(&format!("Show {n}"), &format!("Show {n}/Season 01")))
        .collect();
    let s = Scene::failing(&releases.iter().collect::<Vec<_>>(), rules).await;
    // The items have left the feed, so the worker's own cycles do not take them.
    s.feed(&[]);
    let ids: Vec<String> = (1..=9)
        .map(|n| format!("5a0c2a71-0000-4000-8000-0000000001{n:02}"))
        .collect();
    for (n, id) in (1..=9).zip(&ids) {
        let item = s.item(&format!("Show {n} - 01")).await;
        let (status, view) = s.post(id, &item).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{view}");
    }
    // Every add waits until the test lets it through.
    let gate = s.h.tr.hold("torrent-add");

    let env =
        s.h.worker_env_with(&[("TRSS_WORKER_INTERVAL_SECS", "3600")]);
    let worker = s.h.worker_with(s.h.db.clone(), &env);
    let cancel = CancellationToken::new();
    let running = {
        let (worker, cancel) = (worker.clone(), cancel.clone());
        tokio::spawn(async move { worker.run(cancel).await })
    };
    for _ in 0..8 {
        gate.wait_arrived().await;
    }
    let ninth = tokio::time::timeout(Duration::from_millis(500), gate.wait_arrived()).await;
    assert!(ninth.is_err(), "a ninth command ran beside eight");
    assert_eq!(state_of(&s, &ids[8]).await, "pending");

    // One ends, and the ninth takes its place.
    gate.release_one();
    gate.wait_arrived().await;
    gate.release_all();
    wait_for("every command to end", || async {
        for id in &ids {
            if state_of(&s, id).await != "done" {
                return false;
            }
        }
        true
    })
    .await;
    assert_eq!(s.adds().len(), 9);
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
        let child = h
            .trss_process()
            .env("TRSS_WORKER_INTERVAL_SECS", "3600")
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .expect("spawn trss");
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

    /// The address the process's web listens on, once it has said so.
    async fn web_addr(&self) -> String {
        const SAID: &str = "trss listening on http://";
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = self.output();
            let said = output.lines().find_map(|line| {
                let rest = line.strip_prefix(SAID)?;
                Some(rest.split_whitespace().next()?.to_owned())
            });
            if let Some(addr) = said {
                return addr;
            }
            assert!(
                Instant::now() < deadline,
                "the web did not listen:\n{output}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Sends `command` to the process's web as the screen does.
    async fn post_command(&self, command: Value) -> u16 {
        let addr = self.web_addr().await;
        reqwest::Client::new()
            .post(format!("http://{addr}/api/commands"))
            .header("content-type", "application/json")
            .body(command.to_string())
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
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
async fn the_process_runs_a_command_to_its_end_while_its_cycle_is_held_in_transmission() {
    let liar = release("guid-liar-26", 26, LIAR, "");
    let s = Scene::failing(&[&liar], picked_rules()).await;
    let item = s.item("LIAR GAME - 26").await;
    // The process's first cycle is held at its first request to Transmission,
    // which no command makes.
    let gate = s.h.tr.hold("session-set");
    let worker = Proc::spawn(&s.h);
    gate.wait_arrived().await;

    let status = worker
        .post_command(
            json!({ "id": CMD, "kind": "receive_once", "payload": { "item_id": item.id } }),
        )
        .await;
    assert_eq!(status, 202, "{}", worker.output());
    wait_for("the command to be done", || async {
        s.command(CMD).await.1["state"] == "done"
    })
    .await;

    assert_eq!(s.command(CMD).await.1["outcome"]["result"], "received");
    let output = worker.output();
    assert!(
        !output.contains("Cycle finished"),
        "the cycle was not held:\n{output}"
    );
    gate.release_all();
    wait_for("the cycle to finish", || async {
        worker.output().contains("Cycle finished")
    })
    .await;
}

#[tokio::test]
async fn a_started_process_runs_again_the_commands_an_earlier_one_left_open() {
    const STARTED: &str = "5a0c2a71-0000-4000-8000-000000000041";
    const ACCEPTED: &str = "5a0c2a71-0000-4000-8000-000000000042";
    let liar = release("guid-liar-26", 26, LIAR, "");
    let other = release("guid-other-3", 3, OTHER, "");
    let s = Scene::failing(&[&liar, &other], picked_rules()).await;
    // The items have left the feed, so the process's cycle does not take them.
    s.feed(&[]);
    let store = CommandStore::new(s.h.db.clone());
    // An earlier process (an earlier release, say, stopped for an upgrade)
    // started one command and stopped before its end, and accepted another.
    s.post(STARTED, &s.item("LIAR GAME - 26").await).await;
    let started = store.claim_next(s.h.now()).await.unwrap().unwrap();
    assert_eq!((started.id.as_str(), started.attempts), (STARTED, 1));
    s.post(ACCEPTED, &s.item("Another Show").await).await;

    let worker = Proc::spawn(&s.h);
    wait_for("both commands to end", || async {
        state_of(&s, STARTED).await == "done" && state_of(&s, ACCEPTED).await == "done"
    })
    .await;

    for (id, attempts) in [(STARTED, 2), (ACCEPTED, 1)] {
        let command = store.get(id).await.unwrap().unwrap();
        assert_eq!(command.attempts, attempts, "{id}: {}", worker.output());
        assert_eq!(command.outcome.unwrap().result, "received");
    }
    assert_eq!(s.h.tr.torrents().len(), 2);
}

// --- receiving the past items of a subscription ------------------------------------------

// --- the rule detail's view of the past items ----------------------------------------------

// --- what a rule missed while it was off ------------------------------------------------------

// --- commands at once, beside the cycles (ticket 3-subtitle-candidates-and-receiving/0031) -----------------------------------------

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
    // The worker looks for commands by itself only at its start: only the
    // web's ring can explain a start within the second.
    let wakes = Wakes::default();
    let worker = s.h.worker().with_wakes(wakes.clone());
    let cancel = CancellationToken::new();
    let running = tokio::spawn({
        let (worker, cancel) = (worker.clone(), cancel.clone());
        async move { worker.run(cancel).await }
    });
    // Its first cycle and first look have found nothing to do.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Accepted by a web that does not ring the worker: nothing starts it.
    let (status, _) = s.post(UNHEARD, &s.item("Another Show").await).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(state_of(&s, UNHEARD).await, "pending");

    // Accepted by a web that rings it: under way within the second.
    let web = WebApi::with_state(AppState::new(s.h.db.clone()).with_wakes(wakes));
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
    // The look the ring started takes every open command.
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
async fn the_cycles_removal_leaves_the_torrents_alone_while_a_command_adds() {
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

    // The same worker's cycle comes to its removal meanwhile: it does not wait
    // for the command, and goes on past it (to count the torrents) having
    // removed nothing. Its reading of the collect folder then waits for the
    // command's turn at the work folder.
    let counts = || {
        s.h.tr
            .calls_of("torrent-get")
            .iter()
            .filter(|c| c.args["fields"] == json!(["status", "hashString"]))
            .count()
    };
    let counted = counts();
    s.h.advance(300_000);
    let cycle = tokio::spawn({
        let worker = worker.clone();
        async move { worker.tick(&CancellationToken::new()).await }
    });
    wait_for("the cycle to pass its removal", || async {
        counts() > counted
    })
    .await;
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.h.tr.torrents().len(), 1);

    late.release_all();
    assert_eq!(commands.await.unwrap().unwrap(), CommandsOutcome::Ran(1));
    let TickOutcome::Ran(report) = cycle.await.unwrap().unwrap() else {
        panic!("expected a cycle")
    };
    assert!(report.commands_at_work);
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.h.tr.torrents().len(), 1);

    // The command met its torrent and named it; history holds its hash.
    let (_, view) = s.command(CMD).await;
    assert_eq!(view["state"], "done");
    assert_eq!(view["outcome"]["result"], "received");
    let held = s.item("LIAR GAME - 26").await;
    assert_eq!(held.result, HistoryResult::Received);
    assert_eq!(held.torrent_hash.as_deref(), Some(hash(26).as_str()));
    assert_eq!(s.h.tr.torrent(&hash(26)).name, "LIAR GAME S01E26.mkv");

    // With no command at work, the cycles clean up as usual: the second one
    // that finds the torrent gone removes it.
    let report = s.cycle_with(&worker).await;
    assert!(!report.commands_at_work);
    let report = s.cycle_with(&worker).await;
    assert_eq!(report.removed.len(), 1);
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
