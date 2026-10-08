//! `다시 받기` (a retry, stored as the `receive_once` command) from the record
//! tab, end to end: the real commands and history HTTP API, the real worker
//! code and Transmission client, a fake Transmission and fake RSS feeds.
//! Ticket 0008's completion rows.
//!
//! What a retry decides (where it adds, what it names the file, which torrent
//! is its own, which request ends at once, how the original link is got back)
//! is tested in trss-collect (`commands::receive_once`, ADR 0015). This file
//! keeps what only a process shows: that a command the web accepted reaches
//! the worker and ends, the lock and the order of cycles and commands, a worker
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
use trss_collect::{
    commands::receive_once::NAME_NOT_DERIVED,
    store::{
        channels::{ChannelWithRules, RuleInput, RuleState},
        history::{HistoryItem, HistoryResult},
    },
};
use trss_core::{
    commands::{CommandState, CommandStore, MAX_ATTEMPTS},
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
    // trname cannot name the file for either: the legacy renaming removed such
    // a torrent with its data.
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

// --- receiving the past items of a subscription ------------------------------------------

// --- the rule detail's view of the past items ----------------------------------------------

// --- what a rule missed while it was off ------------------------------------------------------

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

    // With no command at work, the next cycle cleans up as usual.
    let report = s.cycle_with(&worker).await;
    assert!(!report.commands_at_work);
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
