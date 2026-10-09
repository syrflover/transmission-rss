//! Archiving and restoring a rule with its work folder (the `rule_archive`
//! command), end to end: the real commands and rules HTTP API, the real worker
//! and Transmission client, a fake Transmission that moves torrent data on the
//! real disk, and real temporary folders as the collect and archive folders.
//! Ticket 0011's completion rows.
//!
//! What stays here is what only a process shows (ADR 0015, ticket 0101): that a
//! command the web accepted reaches the worker and ends, the lock and the turns
//! of the folders, the order of the commands and the cycles beside a move, and
//! a move cut short by the worker stopping. The rules of the move (the checks,
//! which torrents move, the waits for Transmission, the ends of the command)
//! and of the cycle meeting an archived work folder are tested in trss-collect
//! (`rule_archive/work_folder/tests.rs`, `rule_archive/run_tests.rs`,
//! `cycle/tests.rs`).
//!
//! The worker in these tests runs as the test's own user, so every folder has
//! one owner: a test cannot make Transmission's user differ from the worker's
//! without root, and owners are not told apart here.

use crate::common;

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use trss_anissia::Anime;
use trss_collect::{
    commands::rule_archive::{self, work_folder::MovePolicy, Direction},
    store::channels::{
        ChannelInput, ChannelWithRules, NewSubscription, RuleInput, RuleState, SubtitleMode,
    },
};
use trss_core::{
    commands::{Accepted, CommandState, CommandStore},
    settings::SettingsStore,
};
use trss_worker::{CommandsOutcome, TickOutcome, Worker};

const SEEDING: u8 = 6;

fn hash(n: u32) -> String {
    format!("dddd{n:036}")
}

fn magnet(n: u32, name: &str) -> String {
    let dn: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
    format!("magnet:?xt=urn:btih:{}&dn={dn}", hash(n))
}

fn feed_xml(items: &[(u32, &str)]) -> String {
    let items: String = items
        .iter()
        .map(|(n, title)| {
            format!(
                "<item><title>{title}</title><link>{}</link><guid isPermaLink=\"false\">g{n}</guid></item>",
                magnet(*n, title).replace('&', "&amp;")
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>x</title>
        <link>http://x/</link><description>d</description>{items}</channel></rss>"#
    )
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Every file below `root`, relative, sorted; empty when `root` is missing.
fn files(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                walk(&path, root, out);
            } else {
                out.push(path.strip_prefix(root).unwrap().display().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn text(path: impl AsRef<Path>) -> String {
    path.as_ref().to_str().unwrap().to_owned()
}

struct Scene {
    h: Harness,
    api: WebApi,
    collect: PathBuf,
    archive: PathBuf,
    outside: PathBuf,
}

impl Scene {
    /// Real collect and archive folders (`Shows (current)` and `Shows`), the
    /// archive folder set only when `with_archive`.
    async fn new(with_archive: bool) -> Scene {
        let h = Harness::without_collect_folder().await;
        let media = h.dir.path().join("media");
        let collect = media.join("Shows (current)");
        let archive = media.join("Shows");
        let outside = h.dir.path().join("elsewhere");
        for dir in [&collect, &archive, &outside] {
            fs::create_dir_all(dir).unwrap();
        }
        SettingsStore::new(h.db.clone())
            .put_collection(0, text(&collect), with_archive.then(|| text(&archive)))
            .await
            .unwrap();
        Scene {
            api: h.web_api(),
            h,
            collect,
            archive,
            outside,
        }
    }

    /// A channel on the fake feed `feed` with rules `(phrase, directory)`.
    async fn channel(&self, feed: &str, rules: &[(&str, &str)]) -> ChannelWithRules {
        self.h.feeds.set_xml(feed, &feed_xml(&[]));
        let rules = rules
            .iter()
            .map(|(phrase, directory)| RuleInput {
                r#match: Some(phrase.to_string()),
                directory: directory.to_string(),
                ..RuleInput::default()
            })
            .collect();
        self.h
            .channels
            .create_channel_with_rules(ChannelInput::new(self.h.feeds.url(feed)), rules)
            .await
            .unwrap()
    }

    fn worker(&self) -> Worker {
        self.h.worker().with_move_policy(MovePolicy {
            poll: Duration::from_millis(10),
            timeout: Duration::from_secs(5),
        })
    }

    /// Sends `보관` (`archive`) or `복원` (`restore`) of the rule as the screen does.
    async fn send(&self, id: &str, rule_id: &str, direction: &str) -> (StatusCode, Value) {
        let (status, _, body) = self
            .api
            .call(
                "POST",
                "/api/commands",
                Some(json!({
                    "id": id,
                    "kind": "rule_archive",
                    "payload": { "rule_id": rule_id, "direction": direction },
                })),
            )
            .await;
        (status, body)
    }

    async fn run(&self) -> CommandsOutcome {
        self.worker()
            .run_commands(&CancellationToken::new())
            .await
            .unwrap()
    }

    /// The rule as the screen reads it.
    async fn rule(&self, id: &str) -> Value {
        let (status, text, body) = self
            .api
            .call("GET", &format!("/api/rules/{id}"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        body
    }

    async fn command(&self, id: &str) -> Value {
        let (status, text, body) = self
            .api
            .call("GET", &format!("/api/commands/{id}"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        body
    }

    /// A seeding torrent of a person (no bot label, so no cycle removes it)
    /// with its single file in `dir`.
    fn seeding(&self, n: u32, name: &str, dir: &Path) {
        write(&dir.join(name), "video");
        self.h
            .tr
            .preload(FakeTorrent::new(&hash(n), name).in_dir(dir).status(SEEDING));
    }
}

// --- 1. the simple move ----------------------------------------------------------------

/// The web accepts `보관`, the worker runs it and the rule shows it. What the
/// move does with the folder and the torrents is tested in trss-collect
/// (`work_folder/tests.rs`, `run_tests.rs`).
#[tokio::test]
async fn archiving_turns_the_rule_off_and_moves_the_work_folder_with_its_torrents() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    let rule = &c.rules[0];
    let season = s.collect.join("Clevatess/Season 02");
    s.seeding(1, "Clevatess S02E01.mkv", &season);

    let (status, body) = s.send("archive-simple-1", &rule.id, "archive").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["state"], "pending");
    // Accepted is not done: nothing changed yet.
    assert_eq!(s.rule(&rule.id).await["state"], "active");
    assert!(season.exists());

    assert_eq!(s.run().await, CommandsOutcome::Ran(1));

    let command = s.command("archive-simple-1").await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "moved");
    let view = s.rule(&rule.id).await;
    assert_eq!(view["state"], "archived");
    assert!(view["version"].as_i64().unwrap() > rule.version);
    assert_eq!(view["archive_move"]["direction"], "archive");
    assert_eq!(view["archive_move"]["command"]["id"], "archive-simple-1");
    // The worker's Transmission client moved the torrent, and the worker the
    // folder.
    assert!(!s.collect.join("Clevatess").exists());
    assert_eq!(
        files(&s.archive),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert_eq!(
        s.h.tr.torrent(&hash(1)).download_dir,
        text(s.archive.join("Clevatess/Season 02"))
    );
}

// --- 7. stopped midway -----------------------------------------------------------------------

#[tokio::test]
async fn a_worker_stopped_after_transmission_moved_finishes_the_rest_on_the_next_run() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    let rule = &c.rules[0];
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );
    write(&s.collect.join("Clevatess/Season 02/notes.txt"), "x");
    write(&s.collect.join("Clevatess/.trss/subs/S02E01.ass"), "sub");

    // Transmission moves the torrent's data, and the worker is stopped before
    // it hears back.
    let gate = s.h.tr.hold_answer("torrent-set-location");
    s.send("archive-stop-01", &rule.id, "archive").await;
    let worker = s.worker();
    let task = tokio::spawn(async move { worker.run_commands(&CancellationToken::new()).await });
    gate.wait_arrived().await;
    task.abort();
    let _ = task.await;
    gate.release_all();
    s.h.wait_lock_free().await;

    let stored = CommandStore::new(s.h.db.clone())
        .get("archive-stop-01")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.state, CommandState::Running);
    assert_eq!(s.rule(&rule.id).await["state"], "archived");
    // Split for now: the torrent's file moved, the rest did not.
    assert_eq!(
        files(&s.archive),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert_eq!(
        files(&s.collect),
        [
            "Clevatess/.trss/subs/S02E01.ass",
            "Clevatess/Season 02/notes.txt"
        ]
    );

    // A restarted worker runs a cycle first: the archived rule adds nothing to
    // the folder left behind.
    s.h.feeds
        .set_xml("feed-a", &feed_xml(&[(2, "Clevatess S02E02.mkv")]));
    let tick = s.worker().tick(&CancellationToken::new()).await.unwrap();
    assert!(matches!(tick, TickOutcome::Ran(_)), "{tick:?}");
    assert!(s.h.tr.calls_of("torrent-add").is_empty());

    // Then it picks the command up again and moves only what is left.
    assert_eq!(s.run().await, CommandsOutcome::Ran(1));
    let command = s.command("archive-stop-01").await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "moved");
    assert!(!s.collect.join("Clevatess").exists());
    assert_eq!(
        files(&s.archive),
        [
            "Clevatess/.trss/subs/S02E01.ass",
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/notes.txt",
        ]
    );
    assert_eq!(
        s.h.tr.calls_of("torrent-set-location").len(),
        1,
        "the torrent already at the archive is not moved again"
    );
}

// --- 9. the cycle cannot add while a folder moves --------------------------------------------

#[tokio::test]
async fn no_cycle_runs_while_a_folder_moves() {
    let s = Scene::new(true).await;
    let c = s
        .channel(
            "feed-a",
            &[
                ("Clevatess", "Clevatess/Season 02"),
                ("Other", "Other/Season 01"),
            ],
        )
        .await;
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );
    // An item for the rule being archived and one for another rule wait in the feed.
    s.h.feeds.set_xml(
        "feed-a",
        &feed_xml(&[(2, "Clevatess S02E02.mkv"), (3, "Other S01E01.mkv")]),
    );

    let gate = s.h.tr.hold("torrent-set-location");
    s.send("archive-cycle-1", &c.rules[0].id, "archive").await;
    let worker = s.worker();
    let task = tokio::spawn(async move { worker.run_commands(&CancellationToken::new()).await });
    gate.wait_arrived().await;

    // Mid-move: another worker's cycle and look for commands both find the
    // lock between processes taken.
    let tick = s.worker().tick(&CancellationToken::new()).await.unwrap();
    assert_eq!(tick, TickOutcome::Busy);
    assert_eq!(s.run().await, CommandsOutcome::Busy);
    assert!(s.h.tr.calls_of("torrent-add").is_empty());

    gate.release_all();
    assert_eq!(task.await.unwrap().unwrap(), CommandsOutcome::Ran(1));
    assert!(!s.collect.join("Clevatess").exists());

    // Afterwards the cycle runs, and the archived rule adds nothing.
    let tick = s.worker().tick(&CancellationToken::new()).await.unwrap();
    assert!(matches!(tick, TickOutcome::Ran(_)), "{tick:?}");
    let adds = s.h.tr.calls_of("torrent-add");
    assert_eq!(adds.len(), 1);
    assert_eq!(
        adds[0].args["download-dir"],
        text(s.collect.join("Other/Season 01")).as_str()
    );
    assert!(!s.collect.join("Clevatess").exists());
}

#[tokio::test]
async fn the_same_workers_cycle_adds_beside_a_move_and_skips_the_rule_archived_meanwhile() {
    let s = Scene::new(true).await;
    let c = s
        .channel(
            "feed-a",
            &[
                ("Clevatess", "Clevatess/Season 02"),
                ("Other", "Other/Season 01"),
            ],
        )
        .await;
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );
    s.h.feeds.set_xml(
        "feed-a",
        &feed_xml(&[(2, "Clevatess S02E02.mkv"), (3, "Other S01E01.mkv")]),
    );
    // One worker, as a process has one.
    let worker = s.worker();

    // Its cycle has read the rules, both active, and waits for the feed.
    let feed = s.h.feeds.hold("feed-a");
    s.h.advance(300_000);
    let cycle = tokio::spawn({
        let worker = worker.clone();
        async move { worker.tick(&CancellationToken::new()).await }
    });
    feed.wait_arrived().await;

    // Meanwhile `보관` turns the first rule off and starts moving its folder.
    let gate = s.h.tr.hold("torrent-set-location");
    s.send("archive-beside-1", &c.rules[0].id, "archive").await;
    let commands = tokio::spawn({
        let worker = worker.clone();
        async move { worker.run_commands(&CancellationToken::new()).await }
    });
    gate.wait_arrived().await;

    // Mid-move, the cycle goes on: the other work folder's item goes in, the
    // moving one's waits for the move.
    feed.release_all();
    let adds = || s.h.tr.calls_of("torrent-add");
    for _ in 0..1000 {
        if !adds().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(adds().len(), 1);
    assert_eq!(
        adds()[0].args["download-dir"],
        text(s.collect.join("Other/Season 01")).as_str()
    );
    assert!(!cycle.is_finished());

    // The move ends; the waiting item's rule is archived now, so it is not
    // added into the folder that left.
    gate.release_all();
    assert_eq!(commands.await.unwrap().unwrap(), CommandsOutcome::Ran(1));
    assert_eq!(
        s.command("archive-beside-1").await["outcome"]["result"],
        "moved"
    );
    let TickOutcome::Ran(report) = cycle.await.unwrap().unwrap() else {
        panic!("expected a cycle")
    };
    assert_eq!(report.rule_changed, 1);
    assert_eq!(report.added, 1);
    assert_eq!(adds().len(), 1);
    assert!(!s.collect.join("Clevatess").exists());
    assert_eq!(
        files(&s.archive),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
}

#[tokio::test]
async fn a_rescan_of_another_watch_folder_runs_while_a_folder_moves() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );
    // A watch folder outside the collect and archive folders.
    let (status, _, body) = s
        .api
        .call(
            "POST",
            "/api/library/watch-folders",
            Some(json!({ "path": text(&s.outside) })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let folder_id = body["folder"]["id"].as_str().unwrap().to_owned();
    write(
        &s.outside
            .join("Lycoris Recoil/Season 01/Lycoris Recoil S01E01.mkv"),
        "video",
    );
    let worker = s.worker();

    let gate = s.h.tr.hold("torrent-set-location");
    s.send("archive-rescan-1", &c.rules[0].id, "archive").await;
    let moving = tokio::spawn({
        let worker = worker.clone();
        async move { worker.run_commands(&CancellationToken::new()).await }
    });
    gate.wait_arrived().await;

    let (status, _, body) = s
        .api
        .call(
            "POST",
            "/api/commands",
            Some(json!({
                "id": "rescan-beside-1",
                "kind": "watch_rescan",
                "payload": { "folder_id": folder_id },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let ran = tokio::time::timeout(
        Duration::from_secs(10),
        worker.run_commands(&CancellationToken::new()),
    )
    .await
    .expect("the rescan waited for the move")
    .unwrap();

    assert_eq!(ran, CommandsOutcome::Ran(1));
    let rescan = s.command("rescan-beside-1").await;
    assert_eq!(rescan["state"], "done", "{rescan}");
    assert_eq!(rescan["outcome"]["result"], "scanned");
    assert_eq!(s.command("archive-rescan-1").await["state"], "running");
    assert!(s.collect.join("Clevatess").exists());

    gate.release_all();
    assert_eq!(moving.await.unwrap().unwrap(), CommandsOutcome::Ran(1));
    assert_eq!(
        s.command("archive-rescan-1").await["outcome"]["result"],
        "moved"
    );
}

#[tokio::test]
async fn a_move_accepted_after_a_retry_into_its_work_folder_waits_for_it_and_takes_its_torrent() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    let rule = &c.rules[0];
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );
    // Transmission refused the rule's next episode: `다시 받기` is offered.
    s.h.feeds
        .set_xml("feed-a", &feed_xml(&[(2, "Clevatess S02E02.mkv")]));
    s.h.tr.reject_adds(Some("not today"));
    assert!(matches!(
        s.worker().tick(&CancellationToken::new()).await.unwrap(),
        TickOutcome::Ran(_)
    ));
    s.h.tr.reject_adds(None);
    s.h.tr.clear_calls();
    let item = s.h.item("Clevatess S02E02").await;
    // The retried torrent's file is written where Transmission puts it.
    s.h.tr.on_disk(s.h.dir.path());
    s.h.tr.content_on_add(&hash(2), b"video");
    s.h.tr.seeding_on_add(&hash(2));

    // `다시 받기` first, then `보관` of the same rule: one work folder.
    let add = s.h.tr.hold("torrent-add");
    let (status, _, body) = s
        .api
        .call(
            "POST",
            "/api/commands",
            Some(json!({
                "id": "retry-turns-0001",
                "kind": "receive_once",
                "payload": { "item_id": item.id },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    s.send("archive-turns-01", &rule.id, "archive").await;
    let look = tokio::spawn({
        let worker = s.worker();
        async move { worker.run_commands(&CancellationToken::new()).await }
    });
    add.wait_arrived().await;

    // Both are claimed; the move waits for the retry's add.
    let commands = CommandStore::new(s.h.db.clone());
    for _ in 0..1000 {
        let archive = commands.get("archive-turns-01").await.unwrap().unwrap();
        if archive.state == CommandState::Running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(s.command("archive-turns-01").await["state"], "running");
    assert_eq!(s.rule(&rule.id).await["state"], "active");
    assert!(s.h.tr.calls_of("torrent-set-location").is_empty());

    add.release_all();
    assert_eq!(look.await.unwrap().unwrap(), CommandsOutcome::Ran(2));

    // In the order they were accepted: the retry's torrent went into the work
    // folder, and the move took it along with the rest.
    let retry = s.command("retry-turns-0001").await;
    assert_eq!(retry["outcome"]["result"], "received", "{retry}");
    let archive = s.command("archive-turns-01").await;
    assert_eq!(archive["outcome"]["result"], "moved", "{archive}");
    let season = text(s.archive.join("Clevatess/Season 02"));
    assert_eq!(s.h.tr.torrent(&hash(1)).download_dir, season);
    assert_eq!(s.h.tr.torrent(&hash(2)).download_dir, season);
    assert!(!s.collect.join("Clevatess").exists());
    assert_eq!(
        files(&s.archive),
        [
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/Clevatess S02E02.mkv"
        ]
    );
}

#[tokio::test]
async fn a_retry_accepted_while_a_move_waits_for_transmission_and_a_cycle_ends_still_runs() {
    let s = Scene::new(true).await;
    let c = s
        .channel(
            "feed-a",
            &[
                ("Clevatess", "Clevatess/Season 02"),
                ("Other", "Other/Season 01"),
            ],
        )
        .await;
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );
    // Transmission refused the other rule's item: `다시 받기` is offered.
    s.h.feeds
        .set_xml("feed-a", &feed_xml(&[(3, "Other S01E01.mkv")]));
    s.h.tr.reject_adds(Some("not today"));
    assert!(matches!(
        s.worker().tick(&CancellationToken::new()).await.unwrap(),
        TickOutcome::Ran(_)
    ));
    s.h.tr.reject_adds(None);
    s.h.feeds.set_xml("feed-a", &feed_xml(&[]));
    let item = s.h.item("Other S01E01").await;
    let worker = s.worker();

    // A move waits for Transmission, holding the torrent gate.
    let gate = s.h.tr.hold("torrent-set-location");
    s.send("archive-gate-001", &c.rules[0].id, "archive").await;
    let moving = tokio::spawn({
        let worker = worker.clone();
        async move { worker.run_commands(&CancellationToken::new()).await }
    });
    gate.wait_arrived().await;

    // The same worker's cycle comes to its removal and leaves it for later:
    // it goes on to count the torrents. (Its reading of the collect folder
    // then waits for the move.)
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
    for _ in 0..1000 {
        if counts() > counted {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(counts() > counted, "the cycle waited at its removal");

    // A retry in another work folder starts and ends all the same.
    let (status, _, body) = s
        .api
        .call(
            "POST",
            "/api/commands",
            Some(json!({
                "id": "retry-gate-00001",
                "kind": "receive_once",
                "payload": { "item_id": item.id },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let ran = tokio::time::timeout(
        Duration::from_secs(10),
        worker.run_commands(&CancellationToken::new()),
    )
    .await
    .expect("the retry waited behind the cycle's removal")
    .unwrap();
    assert_eq!(ran, CommandsOutcome::Ran(1));
    let retry = s.command("retry-gate-00001").await;
    assert_eq!(retry["outcome"]["result"], "received", "{retry}");

    gate.release_all();
    assert_eq!(moving.await.unwrap().unwrap(), CommandsOutcome::Ran(1));
    let TickOutcome::Ran(report) = cycle.await.unwrap().unwrap() else {
        panic!("expected a cycle")
    };
    assert!(report.commands_at_work);
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert_eq!(
        s.command("archive-gate-001").await["outcome"]["result"],
        "moved"
    );
}

#[tokio::test]
async fn a_retry_passes_a_rescan_of_the_collect_folder_queued_behind_a_move_of_another_rule() {
    let s = Scene::new(true).await;
    let c = s
        .channel(
            "feed-a",
            &[
                ("Clevatess", "Clevatess/Season 02"),
                ("Other", "Other/Season 01"),
            ],
        )
        .await;
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );
    // Transmission refused the other rule's item: `다시 받기` is offered. The
    // cycle has also made the collect folder a watch folder.
    s.h.feeds
        .set_xml("feed-a", &feed_xml(&[(3, "Other S01E01.mkv")]));
    s.h.tr.reject_adds(Some("not today"));
    assert!(matches!(
        s.worker().tick(&CancellationToken::new()).await.unwrap(),
        TickOutcome::Ran(_)
    ));
    s.h.tr.reject_adds(None);
    s.h.feeds.set_xml("feed-a", &feed_xml(&[]));
    let item = s.h.item("Other S01E01").await;
    let (status, text_, body) = s.api.call("GET", "/api/library/watch-folders", None).await;
    assert_eq!(status, StatusCode::OK, "{text_}");
    let collect_folder = body["folders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == text(&s.collect))
        .unwrap_or_else(|| panic!("the collect folder is not watched: {body}"))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let worker = s.worker();

    // A move of the first rule waits for Transmission.
    let gate = s.h.tr.hold("torrent-set-location");
    s.send("archive-pass-001", &c.rules[0].id, "archive").await;
    let moving = tokio::spawn({
        let worker = worker.clone();
        async move { worker.run_commands(&CancellationToken::new()).await }
    });
    gate.wait_arrived().await;

    // `다시 확인` of the whole collect folder, then `다시 받기` into the other
    // rule's work folder inside it.
    for (id, kind, payload) in [
        (
            "rescan-pass-0001",
            "watch_rescan",
            json!({ "folder_id": collect_folder }),
        ),
        (
            "retry-pass-00001",
            "receive_once",
            json!({ "item_id": item.id }),
        ),
    ] {
        let (status, _, body) = s
            .api
            .call(
                "POST",
                "/api/commands",
                Some(json!({ "id": id, "kind": kind, "payload": payload })),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    }
    let look = tokio::spawn({
        let worker = worker.clone();
        async move { worker.run_commands(&CancellationToken::new()).await }
    });

    // The rescan waits behind the move; the retry does not wait behind it.
    let mut retry = Value::Null;
    for _ in 0..1000 {
        retry = s.command("retry-pass-00001").await;
        if retry["state"] == "done" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(retry["state"], "done", "the retry waited: {retry}");
    assert_eq!(retry["outcome"]["result"], "received", "{retry}");
    assert_eq!(s.command("rescan-pass-0001").await["state"], "running");
    assert_eq!(s.command("archive-pass-001").await["state"], "running");

    gate.release_all();
    assert_eq!(moving.await.unwrap().unwrap(), CommandsOutcome::Ran(1));
    assert_eq!(look.await.unwrap().unwrap(), CommandsOutcome::Ran(2));
    assert_eq!(
        s.command("archive-pass-001").await["outcome"]["result"],
        "moved"
    );
    let rescan = s.command("rescan-pass-0001").await;
    assert_eq!(rescan["outcome"]["result"], "scanned", "{rescan}");
}

// --- 9. a rule starts collecting while its work folder is in the archive folder (ticket 0123) ---

impl Scene {
    /// `POST /api/rules` of a rule on `channel`, as the rule-add form sends it.
    async fn create_rule(
        &self,
        channel: &str,
        phrase: &str,
        directory: &str,
    ) -> (StatusCode, Value) {
        let (status, _, body) = self
            .api
            .call(
                "POST",
                "/api/rules",
                Some(json!({
                    "channel_id": channel,
                    "match": phrase,
                    "directory": directory,
                    "episode": 1,
                })),
            )
            .await;
        (status, body)
    }

    /// `PUT /api/rules/{id}/switch` of `영상 받기`.
    async fn switch_video(&self, rule: &Value, on: bool) -> (StatusCode, Value) {
        let (status, _, body) = self
            .api
            .call(
                "PUT",
                &format!("/api/rules/{}/switch", rule["id"].as_str().unwrap()),
                Some(json!({ "version": rule["version"], "video": on })),
            )
            .await;
        (status, body)
    }

    /// The `download-dir` of every torrent a cycle added.
    fn added_dirs(&self) -> Vec<String> {
        self.h
            .tr
            .calls_of("torrent-add")
            .iter()
            .map(|call| call.args["download-dir"].as_str().unwrap().to_owned())
            .collect()
    }

    /// One cycle, after the pause the cycles keep between them.
    async fn cycle(&self, worker: &Worker) -> trss_worker::cycle::CycleReport {
        self.h.advance(300_000);
        let TickOutcome::Ran(report) = worker.tick(&CancellationToken::new()).await.unwrap() else {
            panic!("expected a cycle");
        };
        report
    }
}

#[tokio::test]
async fn a_new_rule_for_an_archived_work_collects_only_after_its_folder_came_into_the_collect_folder(
) {
    let s = Scene::new(true).await;
    let c = s.channel("feed-a", &[]).await;
    let season = s.archive.join("Clevatess/Season 02");
    s.seeding(1, "Clevatess S02E01.mkv", &season);
    write(&s.archive.join("Clevatess/.trss/subs/S02E01.ass"), "sub");
    // The first episode of the new season is in the feed already.
    s.h.feeds
        .set_xml("feed-a", &feed_xml(&[(2, "Clevatess S03E01.mkv")]));

    // The rule exists, off, with its move waiting for the worker.
    let (status, rule) = s
        .create_rule(&c.channel.id, "Clevatess", "Clevatess/Season 03")
        .await;
    assert_eq!(status, StatusCode::CREATED, "{rule}");
    assert_eq!(rule["state"], "paused");
    assert_eq!(rule["archive_move"]["direction"], "start");
    assert_eq!(rule["archive_move"]["command"]["state"], "pending");
    let id = rule["id"].as_str().unwrap();
    let command_id = rule["archive_move"]["command"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // A check of the feed before the move receives nothing, and makes no work
    // folder in the collect folder for the move to run into.
    let worker = s.worker();
    let report = s.cycle(&worker).await;
    assert_eq!(report.added, 0);
    assert!(s.added_dirs().is_empty());
    assert!(!s.collect.join("Clevatess").exists());

    // Mid-move the rule is still off, and a check beside the move receives
    // nothing either.
    let gate = s.h.tr.hold("torrent-set-location");
    let moving = tokio::spawn({
        let worker = worker.clone();
        async move { worker.run_commands(&CancellationToken::new()).await }
    });
    gate.wait_arrived().await;
    assert_eq!(s.rule(id).await["state"], "paused");
    let report = s.cycle(&worker).await;
    assert_eq!(report.added, 0);
    assert!(s.added_dirs().is_empty());
    assert!(!s.collect.join("Clevatess").exists());

    gate.release_all();
    assert_eq!(moving.await.unwrap().unwrap(), CommandsOutcome::Ran(1));
    let command = s.command(&command_id).await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "moved");

    // The whole work folder is in the collect folder, with its torrent, and
    // the rule is on.
    assert!(!s.archive.join("Clevatess").exists());
    assert_eq!(
        files(&s.collect),
        [
            "Clevatess/.trss/subs/S02E01.ass",
            "Clevatess/Season 02/Clevatess S02E01.mkv"
        ]
    );
    let moved_to = text(s.collect.join("Clevatess/Season 02"));
    assert_eq!(s.h.tr.torrent(&hash(1)).download_dir, moved_to);
    let view = s.rule(id).await;
    assert_eq!(view["state"], "active");
    assert_eq!(view["archive_move"]["command"]["state"], "done");

    // Only now does the feed's item go in, into the work folder that came over.
    let report = s.cycle(&worker).await;
    assert_eq!(report.added, 1);
    assert_eq!(
        s.added_dirs(),
        [text(s.collect.join("Clevatess/Season 03"))]
    );
}

#[tokio::test]
async fn two_starts_for_one_work_folder_run_one_after_the_other_and_the_second_just_turns_its_rule_on(
) {
    let s = Scene::new(true).await;
    let c = s
        .channel(
            "feed-a",
            &[
                ("Clevatess S3", "Clevatess/Season 03"),
                ("Clevatess S4", "Clevatess/Season 04"),
            ],
        )
        .await;
    let season = s.archive.join("Clevatess/Season 02");
    s.seeding(1, "Clevatess S02E01.mkv", &season);
    for rule in &c.rules {
        s.h.channels
            .set_rule_state(&rule.id, RuleState::Paused, 0)
            .await
            .unwrap();
    }
    // Both rules are switched on while the work folder is in the archive folder
    // (the second is not refused: only a rule's own move is).
    let mut commands = Vec::new();
    for rule in &c.rules {
        let view = s.rule(&rule.id).await;
        let (status, view) = s.switch_video(&view, true).await;
        assert_eq!(status, StatusCode::OK, "{view}");
        commands.push(
            view["archive_move"]["command"]["id"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }

    // Both are claimed together; the work folder's turn lets one move it while
    // the other waits, and both rules stay off until their own command ends.
    let gate = s.h.tr.hold("torrent-set-location");
    let worker = s.worker();
    let running = tokio::spawn({
        let worker = worker.clone();
        async move { worker.run_commands(&CancellationToken::new()).await }
    });
    gate.wait_arrived().await;
    for rule in &c.rules {
        assert_eq!(s.rule(&rule.id).await["state"], "paused");
    }
    gate.release_all();
    assert_eq!(running.await.unwrap().unwrap(), CommandsOutcome::Ran(2));

    for (rule, command) in c.rules.iter().zip(&commands) {
        assert_eq!(s.rule(&rule.id).await["state"], "active");
        assert_eq!(s.command(command).await["state"], "done");
    }
    // The work folder moved once, with its torrent asked once.
    assert!(!s.archive.join("Clevatess").exists());
    assert_eq!(
        files(&s.collect),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert_eq!(s.h.tr.calls_of("torrent-set-location").len(), 1);
    assert_eq!(
        s.h.tr.torrent(&hash(1)).download_dir,
        text(s.collect.join("Clevatess/Season 02"))
    );
}

#[tokio::test]
async fn a_start_stopped_midway_leaves_the_rule_off_until_the_next_run_finishes_it() {
    let s = Scene::new(true).await;
    let c = s.channel("feed-a", &[]).await;
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.archive.join("Clevatess/Season 02"),
    );
    write(&s.archive.join("Clevatess/Season 02/notes.txt"), "x");
    s.h.feeds
        .set_xml("feed-a", &feed_xml(&[(2, "Clevatess S03E01.mkv")]));
    let (_, rule) = s
        .create_rule(&c.channel.id, "Clevatess", "Clevatess/Season 03")
        .await;
    let id = rule["id"].as_str().unwrap();
    let command_id = rule["archive_move"]["command"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Transmission moves the torrent's data, and the worker is stopped before
    // it hears back.
    let gate = s.h.tr.hold_answer("torrent-set-location");
    let worker = s.worker();
    let task = tokio::spawn(async move { worker.run_commands(&CancellationToken::new()).await });
    gate.wait_arrived().await;
    task.abort();
    let _ = task.await;
    gate.release_all();
    s.h.wait_lock_free().await;

    let stored = CommandStore::new(s.h.db.clone())
        .get(&command_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.state, CommandState::Running);
    assert_eq!(s.rule(id).await["state"], "paused");
    // The work is split for now: the torrent's file came over, the rest did not.
    assert_eq!(
        files(&s.collect),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert_eq!(files(&s.archive), ["Clevatess/Season 02/notes.txt"]);

    // A restarted worker runs a cycle first: the rule is still off, so nothing
    // is added into the half-moved folder.
    let report = s.cycle(&s.worker()).await;
    assert_eq!(report.added, 0);
    assert!(s.added_dirs().is_empty());

    // Then it picks the command up again, moves what is left and turns the
    // rule on.
    assert_eq!(s.run().await, CommandsOutcome::Ran(1));
    let command = s.command(&command_id).await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "moved");
    assert_eq!(s.rule(id).await["state"], "active");
    assert!(!s.archive.join("Clevatess").exists());
    assert_eq!(
        files(&s.collect),
        [
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/notes.txt"
        ]
    );
    assert_eq!(s.h.tr.calls_of("torrent-set-location").len(), 1);
}

// --- 10. the worker checks again right before receiving (ticket 0123, follow-up 2) ---

impl Scene {
    /// A rule saving to `directory`, made straight in the store and on, as a
    /// title given to a subscription that waited for one leaves it (no web gate
    /// moved its work folder first), with `resumed_at` 777.
    async fn rule_made_in_the_store(&self, feed: &str, phrase: &str, directory: &str) -> String {
        let c = self.channel(feed, &[(phrase, directory)]).await;
        let id = c.rules[0].id.clone();
        self.h
            .channels
            .set_rule_state(&id, RuleState::Paused, 700)
            .await
            .unwrap();
        self.h
            .channels
            .set_rule_state(&id, RuleState::Active, 777)
            .await
            .unwrap();
        id
    }
}

/// What the rules decide about such an item is tested in trss-collect
/// (`cycle/tests.rs`); the report's tallies, which a cycle's process keeps, are
/// checked here.
#[tokio::test]
async fn a_cycle_reports_the_item_waiting_for_its_work_folder_and_the_move_it_asked_for_runs() {
    let s = Scene::new(true).await;
    let id = s
        .rule_made_in_the_store("feed-a", "Clevatess", "Clevatess/Season 03")
        .await;
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.archive.join("Clevatess/Season 02"),
    );
    s.h.feeds
        .set_xml("feed-a", &feed_xml(&[(2, "Clevatess S03E01.mkv")]));

    // The rule is on, the work folder is in the archive folder: the item waits,
    // and the cycle stored the command that brings the folder in.
    let worker = s.worker();
    let report = s.cycle(&worker).await;
    assert_eq!(report.added, 0);
    assert_eq!(report.waiting_for_move, 1);
    assert_eq!(report.moves_asked, 1);
    assert!(s.added_dirs().is_empty());
    assert_eq!(s.rule(&id).await["state"], "paused");

    // The rule being off, another check beside the open move asks for nothing.
    let report = s.cycle(&worker).await;
    assert_eq!(report.moves_asked, 0);
    assert!(s.added_dirs().is_empty());

    // The command the cycle stored runs, and the next check receives the item
    // into the work folder that came over.
    assert_eq!(s.run().await, CommandsOutcome::Ran(1));
    assert_eq!(s.rule(&id).await["state"], "active");
    let report = s.cycle(&worker).await;
    assert_eq!(report.added, 1);
    assert_eq!(report.waiting_for_move, 0);
    assert_eq!(
        s.added_dirs(),
        [text(s.collect.join("Clevatess/Season 03"))]
    );
}

// --- 11. the past items ticked while subscribing (ticket 0125) -------------------------------------

#[tokio::test]
async fn a_subscriptions_start_receives_the_ticked_items_into_the_work_folder_that_came_over() {
    let s = Scene::new(true).await;
    // Two releases were posted before the subscription: a check records them
    // as items no rule took.
    let c = s.channel("feed-a", &[]).await;
    s.h.feeds.set_xml(
        "feed-a",
        &feed_xml(&[
            (2, "[Group] Clevatess - 02 (1080p).mkv"),
            (3, "[Group] Clevatess - 03 (1080p).mkv"),
        ]),
    );
    let worker = s.worker();
    assert_eq!(s.cycle(&worker).await.added, 0);
    let (two, three) = (
        s.h.item("Clevatess - 02").await.id,
        s.h.item("Clevatess - 03").await.id,
    );
    // The work folder is in the archive folder, with its first episode's torrent.
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.archive.join("Clevatess/Season 02"),
    );
    s.h.tr.on_disk(s.h.dir.path());
    for n in [2, 3] {
        s.h.tr
            .content_on_add(&hash(n), format!("video {n}").as_bytes());
        s.h.tr.seeding_on_add(&hash(n));
    }

    // The subscription as the web makes it: the rule off, and its start
    // carrying the ticked items in the order the confirm step sends them.
    let rule =
        s.h.channels
            .create_subscription_rule(
                &c.channel.id,
                RuleInput {
                    r#match: Some("Clevatess".to_owned()),
                    directory: "Clevatess/Season 02".to_owned(),
                    state: RuleState::Paused,
                    ..RuleInput::default()
                },
                NewSubscription {
                    anime: Anime {
                        anime_no: 7,
                        subject: "Clevatess".to_owned(),
                        original_subject: None,
                        week: 4,
                        air_time: Some("23:00".to_owned()),
                        start_date: Some("2026-10-08".to_owned()),
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
            .unwrap();
    let commands = CommandStore::new(s.h.db.clone());
    let accepted = rule_archive::ask_start_receiving(
        &commands,
        &rule.id,
        Direction::Start,
        vec![two, three],
        s.h.now(),
    )
    .await
    .unwrap();
    let Accepted::Created(start) = accepted else {
        panic!("expected a stored command: {accepted:?}");
    };

    // One look moves the work folder, turns the rule on, and runs the two
    // receives the start accepted after it, one at a time: a subscription that
    // has picked nothing decides its episode offset from the first item it
    // receives, so the second waits for the first even while Transmission is
    // slow to take it.
    let adding = s.h.tr.hold("torrent-add");
    let worker = s.worker();
    let look = tokio::spawn(async move {
        worker
            .run_commands(&CancellationToken::new())
            .await
            .unwrap()
    });
    adding.wait_arrived().await;
    let second = tokio::time::timeout(Duration::from_millis(300), adding.wait_arrived()).await;
    assert!(
        second.is_err(),
        "the second receive went to Transmission beside the first"
    );
    adding.release_all();
    assert_eq!(look.await.unwrap(), CommandsOutcome::Ran(3));
    let start = s.command(&start.id).await;
    assert_eq!(start["state"], "done", "{start}");
    assert_eq!(start["outcome"]["result"], "moved");
    let reason = start["outcome"]["reason"].as_str().unwrap();
    assert!(
        reason.ends_with("구독할 때 체크한 지난 항목 2개를 이어서 받아요."),
        "{reason}"
    );
    assert_eq!(s.rule(&rule.id).await["state"], "active");
    assert!(!s.archive.join("Clevatess").exists());

    // Both went into the folder that came over, in the order ticked, under the
    // names the rule gives them.
    let season = text(s.collect.join("Clevatess/Season 02"));
    let adds: Vec<(String, String)> =
        s.h.tr
            .calls_of("torrent-add")
            .iter()
            .map(|call| {
                let magnet = call.args["filename"].as_str().unwrap();
                let n = if magnet.contains(&hash(2)) { 2 } else { 3 };
                (
                    hash(n),
                    call.args["download-dir"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
    assert_eq!(adds, [(hash(2), season.clone()), (hash(3), season.clone())]);
    for item in [two, three] {
        let receive = s
            .command(&format!("{}-receive-{item}", start["id"].as_str().unwrap()))
            .await;
        assert_eq!(receive["state"], "done", "{receive}");
        assert_eq!(receive["outcome"]["result"], "received", "{receive}");
    }
    assert_eq!(
        files(&s.collect),
        [
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/Clevatess S02E02.mkv",
            "Clevatess/Season 02/Clevatess S02E03.mkv",
        ]
    );
}
