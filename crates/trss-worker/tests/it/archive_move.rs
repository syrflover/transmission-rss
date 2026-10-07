//! Archiving and restoring a rule with its work folder (the `rule_archive`
//! command), end to end: the real commands and rules HTTP API, the real worker
//! and Transmission client, a fake Transmission that moves torrent data on the
//! real disk, and real temporary folders as the collect and archive folders.
//! Ticket 0011's completion rows.
//!
//! The worker in these tests runs as the test's own user, so every folder has
//! one owner: a test cannot make Transmission's user differ from the worker's
//! without root, and owners are not told apart here. What is checked is that
//! what the worker moves is renamed (the same inode, hence the same owner),
//! never copied or created by it.

use crate::common;

use std::{
    fs,
    os::unix::fs::{symlink, MetadataExt},
    path::{Path, PathBuf},
    time::Duration,
};

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use trss_collect::{
    commands::rule_archive::{self, work_folder::MovePolicy, Direction},
    store::channels::{ChannelInput, ChannelWithRules, RuleInput, RuleState},
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

#[tokio::test]
async fn archiving_turns_the_rule_off_and_moves_the_work_folder_with_its_torrents() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    let rule = &c.rules[0];
    let season = s.collect.join("Clevatess/Season 02");
    s.seeding(1, "Clevatess S02E01.mkv", &season);
    // A bot torrent too: every torrent in the folder moves, whoever added it.
    write(&season.join("Clevatess S02E02.mkv"), "video");
    s.h.tr.preload(
        FakeTorrent::new(&hash(2), "Clevatess S02E02.mkv")
            .in_dir(&season)
            .bot()
            .status(SEEDING),
    );
    write(&s.collect.join("Clevatess/.trss/subs/S02E01.ass"), "sub");
    write(
        &s.collect.join("Clevatess/Season 02/notes.txt"),
        "not a torrent",
    );
    // Another work that stays.
    s.seeding(3, "Other S01E01.mkv", &s.collect.join("Other/Season 01"));

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

    assert!(!s.collect.join("Clevatess").exists());
    assert_eq!(
        files(&s.archive.join("Clevatess")),
        [
            ".trss/subs/S02E01.ass",
            "Season 02/Clevatess S02E01.mkv",
            "Season 02/Clevatess S02E02.mkv",
            "Season 02/notes.txt",
        ]
    );
    assert_eq!(files(&s.collect), ["Other/Season 01/Other S01E01.mkv"]);

    // Transmission moved its torrents first, with their files, and keeps them.
    let new_dir = text(s.archive.join("Clevatess/Season 02"));
    for n in [1, 2] {
        let torrent = s.h.tr.torrent(&hash(n));
        assert_eq!(torrent.download_dir, new_dir);
        assert_eq!(torrent.status, SEEDING);
    }
    assert_eq!(
        s.h.tr.torrent(&hash(3)).download_dir,
        text(s.collect.join("Other/Season 01"))
    );
    let moves = s.h.tr.calls_of("torrent-set-location");
    assert_eq!(moves.len(), 2);
    for call in &moves {
        assert_eq!(call.args["move"], true);
        assert_eq!(call.args["location"], new_dir.as_str());
    }
    assert!(s.h.tr.calls_of("torrent-add").is_empty());
}

// --- 2. another rule still saves there ---------------------------------------------------

#[tokio::test]
async fn a_folder_another_active_rule_saves_in_stays_until_the_last_rule_is_archived() {
    let s = Scene::new(true).await;
    let a = s
        .channel("feed-a", &[("Clevatess S2", "Clevatess/Season 02")])
        .await;
    // The other rule is in another channel: every channel counts.
    let b = s
        .channel("feed-b", &[("Clevatess S3", "Clevatess/Season 03")])
        .await;
    let (season2, season3) = (&a.rules[0], &b.rules[0]);
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );

    s.send("archive-held-01", &season2.id, "archive").await;
    s.run().await;

    let command = s.command("archive-held-01").await;
    assert_eq!(command["state"], "done");
    assert_eq!(command["outcome"]["result"], "kept");
    assert_eq!(
        command["outcome"]["reason"],
        "‘Clevatess/Season 03’ 규칙이 아직 이 작품 폴더에 받고 있어서 옮기지 않았어요."
    );
    let view = s.rule(&season2.id).await;
    assert_eq!(view["state"], "archived");
    assert_eq!(view["archive_move"]["command"]["outcome"]["result"], "kept");
    assert_eq!(
        files(&s.collect),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert!(files(&s.archive).is_empty());
    assert!(s.h.tr.calls_of("torrent-set-location").is_empty());

    // Restoring it now finds the folder in the collect folder already.
    s.send("restore-held-1", &season2.id, "restore").await;
    s.run().await;
    let command = s.command("restore-held-1").await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "moved");
    assert_eq!(
        command["outcome"]["reason"],
        "작품 폴더 `Clevatess`가 이미 수집 폴더에 있어요."
    );
    assert_eq!(s.rule(&season2.id).await["state"], "active");

    // Archived together, the folder moves with the last of them.
    s.send("archive-held-02", &season2.id, "archive").await;
    s.send("archive-held-03", &season3.id, "archive").await;
    assert_eq!(s.run().await, CommandsOutcome::Ran(2));
    assert_eq!(
        s.command("archive-held-02").await["outcome"]["result"],
        "kept"
    );
    assert_eq!(
        s.command("archive-held-03").await["outcome"]["result"],
        "moved"
    );
    assert_eq!(
        files(&s.archive),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert!(!s.collect.join("Clevatess").exists());
}

// --- 3. merging into an archived work folder -------------------------------------------

#[tokio::test]
async fn a_season_merges_beside_the_archived_one_and_new_folders_take_the_parents_owner() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    write(
        &s.archive.join("Clevatess/Season 01/Clevatess S01E01.mkv"),
        "old",
    );
    write(
        &s.archive.join("Clevatess/.trss/subs/S01E01.ass"),
        "old sub",
    );
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );
    write(&s.collect.join("Clevatess/Season 02/extra.txt"), "x");
    write(&s.collect.join("Clevatess/.trss/subs/S02E01.ass"), "sub");
    write(&s.collect.join("Clevatess/.trss/fonts/a.ttf"), "font");
    let fonts_inode = fs::metadata(s.collect.join("Clevatess/.trss/fonts"))
        .unwrap()
        .ino();

    s.send("archive-merge-1", &c.rules[0].id, "archive").await;
    s.run().await;

    assert_eq!(
        s.command("archive-merge-1").await["outcome"]["result"],
        "moved"
    );
    assert!(
        !s.collect.join("Clevatess").exists(),
        "the emptied source is removed"
    );
    assert_eq!(
        files(&s.archive.join("Clevatess")),
        [
            ".trss/fonts/a.ttf",
            ".trss/subs/S01E01.ass",
            ".trss/subs/S02E01.ass",
            "Season 01/Clevatess S01E01.mkv",
            "Season 02/Clevatess S02E01.mkv",
            "Season 02/extra.txt",
        ]
    );

    // `Season 02` is new in the archive: Transmission made it for its torrent,
    // as its own user; the worker makes no folder. Every folder here belongs to
    // the user running the tests, so this cannot tell owners apart: it only
    // holds the expected result in place.
    let parent = fs::metadata(s.archive.join("Clevatess")).unwrap();
    let season = fs::metadata(s.archive.join("Clevatess/Season 02")).unwrap();
    assert_eq!((season.uid(), season.gid()), (parent.uid(), parent.gid()));
    // What the worker moved is renamed, not copied: it keeps its inode, and so
    // its owner.
    assert_eq!(
        fs::metadata(s.archive.join("Clevatess/.trss/fonts"))
            .unwrap()
            .ino(),
        fonts_inode
    );
    assert_eq!(
        s.h.tr.torrent(&hash(1)).download_dir,
        text(s.archive.join("Clevatess/Season 02"))
    );
}

// --- 4. a conflict ---------------------------------------------------------------------------

#[tokio::test]
async fn the_same_file_on_both_sides_moves_nothing_and_moving_again_works_once_cleared() {
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
    write(
        &s.collect.join("Clevatess/Season 02/Clevatess S02E02.mkv"),
        "new",
    );
    write(
        &s.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv"),
        "old",
    );
    let (collect_before, archive_before) = (files(&s.collect), files(&s.archive));

    s.send("archive-clash-1", &rule.id, "archive").await;
    s.run().await;

    let command = s.command("archive-clash-1").await;
    assert_eq!(command["state"], "failed", "{command}");
    let reason = command["outcome"]["reason"].as_str().unwrap();
    assert!(
        reason.contains("`Season 02/Clevatess S02E02.mkv`"),
        "{reason}"
    );
    assert!(reason.contains("아무것도 옮기지 않았어요"), "{reason}");
    // The two copies may differ: it asks to compare, never to clear a side.
    assert!(reason.contains("두 쪽을 견줘"), "{reason}");
    assert!(!reason.contains("정리"), "{reason}");
    // The rule is archived, and nothing moved, on disk or in Transmission.
    let view = s.rule(&rule.id).await;
    assert_eq!(view["state"], "archived");
    assert_eq!(view["archive_move"]["command"]["state"], "failed");
    assert_eq!(files(&s.collect), collect_before);
    assert_eq!(files(&s.archive), archive_before);
    assert!(s.h.tr.calls_of("torrent-set-location").is_empty());

    // `다시 옮기기` is a new archive of the archived rule.
    fs::remove_file(s.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv")).unwrap();
    let (status, body) = s.send("archive-clash-2", &rule.id, "archive").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    s.run().await;
    assert_eq!(
        s.command("archive-clash-2").await["outcome"]["result"],
        "moved"
    );
    assert_eq!(
        files(&s.archive),
        [
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/Clevatess S02E02.mkv",
        ]
    );
    assert!(!s.collect.join("Clevatess").exists());
}

// --- 5. restoring ----------------------------------------------------------------------------

#[tokio::test]
async fn restoring_moves_the_folder_back_before_the_rule_is_on_and_the_next_episode_lands_there() {
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
    write(&s.collect.join("Clevatess/.trss/subs/S02E01.ass"), "sub");
    s.send("archive-first-1", &rule.id, "archive").await;
    s.run().await;
    assert!(s.archive.join("Clevatess").exists());

    // A restore that cannot move keeps the rule archived, with the reason.
    write(
        &s.collect.join("Clevatess/.trss/subs/S02E01.ass"),
        "made meanwhile",
    );
    s.send("restore-clash-1", &rule.id, "restore").await;
    s.run().await;
    let command = s.command("restore-clash-1").await;
    assert_eq!(command["state"], "failed");
    assert!(command["outcome"]["reason"]
        .as_str()
        .unwrap()
        .contains("`.trss/subs/S02E01.ass`"));
    assert_eq!(s.rule(&rule.id).await["state"], "archived");
    fs::remove_dir_all(s.collect.join("Clevatess")).unwrap();

    // While the folder moves back, the rule is still off.
    let gate = s.h.tr.hold("torrent-set-location");
    s.send("restore-back-1", &rule.id, "restore").await;
    let worker = s.worker();
    let task = tokio::spawn(async move { worker.run_commands(&CancellationToken::new()).await });
    gate.wait_arrived().await;
    assert_eq!(s.rule(&rule.id).await["state"], "archived");
    gate.release_all();
    assert_eq!(task.await.unwrap().unwrap(), CommandsOutcome::Ran(1));

    let command = s.command("restore-back-1").await;
    assert_eq!(command["outcome"]["result"], "moved", "{command}");
    assert_eq!(s.rule(&rule.id).await["state"], "active");
    assert!(!s.archive.join("Clevatess").exists());
    assert_eq!(
        files(&s.collect.join("Clevatess")),
        [".trss/subs/S02E01.ass", "Season 02/Clevatess S02E01.mkv"]
    );
    let season = text(s.collect.join("Clevatess/Season 02"));
    assert_eq!(s.h.tr.torrent(&hash(1)).download_dir, season);

    // The next episode goes into the folder that came back.
    s.h.feeds
        .set_xml("feed-a", &feed_xml(&[(2, "Clevatess S02E02.mkv")]));
    let tick = s.worker().tick(&CancellationToken::new()).await.unwrap();
    assert!(matches!(tick, TickOutcome::Ran(_)), "{tick:?}");
    let adds = s.h.tr.calls_of("torrent-add");
    assert_eq!(adds.len(), 1);
    assert_eq!(adds[0].args["download-dir"], season.as_str());
}

// --- 6. no archive folder --------------------------------------------------------------------

#[tokio::test]
async fn without_an_archive_folder_the_rule_is_only_archived() {
    let s = Scene::new(false).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );

    s.send("archive-nofold1", &c.rules[0].id, "archive").await;
    s.run().await;

    let command = s.command("archive-nofold1").await;
    assert_eq!(command["state"], "done");
    assert_eq!(command["outcome"]["result"], "kept");
    assert_eq!(
        command["outcome"]["reason"],
        "보관 폴더를 정하지 않아서 폴더는 옮기지 않았어요."
    );
    assert_eq!(s.rule(&c.rules[0].id).await["state"], "archived");
    assert_eq!(
        files(&s.collect),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert!(s.h.tr.calls_of("torrent-set-location").is_empty());
    assert!(s.h.tr.calls_of("torrent-get").is_empty());

    // A rule saving into the collect folder itself has no work folder either.
    let s = Scene::new(true).await;
    let c = s.channel("feed-a", &[("Loose", "")]).await;
    s.send("archive-loose-1", &c.rules[0].id, "archive").await;
    s.run().await;
    let command = s.command("archive-loose-1").await;
    assert_eq!(command["outcome"]["result"], "kept");
    assert_eq!(
        command["outcome"]["reason"],
        "저장 폴더가 수집 폴더 자체라서 옮길 작품 폴더가 없어요."
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

#[tokio::test]
async fn a_worker_stopped_during_the_renames_finishes_the_rest_on_the_next_run() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    let rule = &c.rules[0];
    // The state a start that was stopped midway through the renames leaves:
    // the command running, the rule archived, the torrent at the archive, and
    // the files split between the two folders.
    s.send("archive-stop-02", &rule.id, "archive").await;
    let commands = CommandStore::new(s.h.db.clone());
    let claimed = commands.claim_next(s.h.now()).await.unwrap().unwrap();
    assert_eq!(claimed.id, "archive-stop-02");
    s.h.channels
        .set_rule_state(&rule.id, RuleState::Archived, 0)
        .await
        .unwrap();
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.archive.join("Clevatess/Season 02"),
    );
    write(
        &s.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv"),
        "moved",
    );
    write(
        &s.collect.join("Clevatess/Season 02/Clevatess S02E03.mkv"),
        "left",
    );
    write(&s.collect.join("Clevatess/.trss/subs/S02E01.ass"), "left");

    assert_eq!(s.run().await, CommandsOutcome::Ran(1));

    let command = s.command("archive-stop-02").await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "moved");
    assert!(!s.collect.join("Clevatess").exists());
    assert_eq!(
        files(&s.archive),
        [
            "Clevatess/.trss/subs/S02E01.ass",
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/Clevatess S02E02.mkv",
            "Clevatess/Season 02/Clevatess S02E03.mkv",
        ]
    );
    assert!(s.h.tr.calls_of("torrent-set-location").is_empty());
}

// --- 8. links out ----------------------------------------------------------------------------

#[tokio::test]
async fn a_work_folder_that_is_a_link_out_of_the_collect_folder_is_not_moved() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    write(
        &s.outside.join("Clevatess/Season 02/Clevatess S02E01.mkv"),
        "x",
    );
    symlink(s.outside.join("Clevatess"), s.collect.join("Clevatess")).unwrap();

    s.send("archive-link-01", &c.rules[0].id, "archive").await;
    s.run().await;

    let command = s.command("archive-link-01").await;
    assert_eq!(command["state"], "failed");
    let reason = command["outcome"]["reason"].as_str().unwrap();
    assert!(reason.contains("링크"), "{reason}");
    assert!(fs::symlink_metadata(s.collect.join("Clevatess"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        files(&s.outside),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert!(files(&s.archive).is_empty());
    assert!(s.h.tr.calls_of("torrent-set-location").is_empty());
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

// --- 10. waiting for Transmission ------------------------------------------------------------

#[tokio::test]
async fn the_renames_wait_until_transmission_reports_the_new_folder() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );
    write(&s.collect.join("Clevatess/Season 02/notes.txt"), "x");
    // Transmission 4 answers first and moves the data in the background.
    s.h.tr.async_locations(true);
    s.h.tr.lag_locations(3);

    s.send("archive-wait-01", &c.rules[0].id, "archive").await;
    s.run().await;

    assert_eq!(
        s.command("archive-wait-01").await["outcome"]["result"],
        "moved"
    );
    // One look before the move and the ones until the new folder showed.
    assert!(s.h.tr.calls_of("torrent-get").len() >= 5);
    assert_eq!(
        s.h.tr.torrent(&hash(1)).download_dir,
        text(s.archive.join("Clevatess/Season 02"))
    );
    assert!(!s.collect.join("Clevatess").exists());
}

#[tokio::test]
async fn a_transmission_that_refuses_the_move_leaves_everything_in_place() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );
    s.h.tr.reject_locations(Some("permission denied"));

    s.send("archive-refus-1", &c.rules[0].id, "archive").await;
    s.run().await;

    let command = s.command("archive-refus-1").await;
    assert_eq!(command["state"], "failed");
    assert!(command["outcome"]["reason"]
        .as_str()
        .unwrap()
        .contains("permission denied"));
    assert_eq!(
        files(&s.collect),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert!(files(&s.archive).is_empty());
}

// --- review: torrents Transmission would still write ------------------------------------------

#[tokio::test]
async fn a_torrent_still_downloading_or_verifying_in_the_work_folder_stops_the_move() {
    for (n, torrent) in [
        (
            1,
            FakeTorrent::new(&hash(1), "Clevatess S02E02.mkv").unfinished(),
        ),
        (
            2,
            FakeTorrent::new(&hash(2), "Clevatess S02E02.mkv").status(2),
        ),
    ] {
        let s = Scene::new(true).await;
        let c = s
            .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
            .await;
        let season = s.collect.join("Clevatess/Season 02");
        s.seeding(9, "Clevatess S02E01.mkv", &season);
        s.h.tr.preload(torrent.in_dir(&season));
        let before = files(&s.collect);

        let id = format!("archive-busy-{n}");
        s.send(&id, &c.rules[0].id, "archive").await;
        s.run().await;

        let command = s.command(&id).await;
        assert_eq!(command["state"], "failed", "{command}");
        let reason = command["outcome"]["reason"].as_str().unwrap();
        assert!(reason.contains("`Clevatess S02E02.mkv`"), "{reason}");
        assert!(reason.contains("다 받거나"), "{reason}");
        assert!(reason.contains("지운 뒤 `다시 옮기기`"), "{reason}");
        assert_eq!(s.rule(&c.rules[0].id).await["state"], "archived");
        assert!(s.h.tr.calls_of("torrent-set-location").is_empty());
        assert_eq!(files(&s.collect), before);
        assert!(files(&s.archive).is_empty());
    }
}

#[tokio::test]
async fn a_downloading_torrent_counts_even_when_the_work_folder_is_gone() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    // Its data is in Transmission's incomplete folder: nothing of the work
    // folder is on disk yet, and the archive has a file of that name.
    s.h.tr.preload(
        FakeTorrent::new(&hash(1), "Clevatess S02E02.mkv")
            .in_dir(s.collect.join("Clevatess/Season 02"))
            .unfinished(),
    );
    write(
        &s.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv"),
        "archived",
    );

    s.send("archive-gone-01", &c.rules[0].id, "archive").await;
    s.run().await;

    let command = s.command("archive-gone-01").await;
    assert_eq!(command["state"], "failed", "{command}");
    assert!(s.h.tr.calls_of("torrent-set-location").is_empty());
    assert_eq!(
        s.h.tr.torrent(&hash(1)).download_dir,
        text(s.collect.join("Clevatess/Season 02"))
    );
    assert_eq!(
        fs::read_to_string(s.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv")).unwrap(),
        "archived"
    );
}

// --- review: which torrents are in the work folder, by where their folder really is ------------

#[tokio::test]
async fn a_torrent_found_through_a_link_into_the_work_folder_stops_the_move() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    write(
        &s.collect.join("Clevatess/Season 02/Clevatess S02E01.mkv"),
        "video",
    );
    // Transmission knows the folder by another name: a link to the work folder.
    symlink(s.collect.join("Clevatess"), s.collect.join("Alias")).unwrap();
    s.h.tr.preload(
        FakeTorrent::new(&hash(1), "Clevatess S02E01.mkv")
            .in_dir(s.collect.join("Alias/Season 02"))
            .status(SEEDING),
    );
    let before = files(&s.collect);

    s.send("archive-alias-1", &c.rules[0].id, "archive").await;
    s.run().await;

    // Its text says elsewhere and the disk says here: nothing moves.
    let command = s.command("archive-alias-1").await;
    assert_eq!(command["state"], "failed", "{command}");
    let reason = command["outcome"]["reason"].as_str().unwrap();
    assert!(reason.contains("링크"), "{reason}");
    assert!(s.h.tr.calls_of("torrent-set-location").is_empty());
    assert_eq!(files(&s.collect), before);
    assert!(files(&s.archive).is_empty());
}

#[tokio::test]
async fn a_torrent_folder_with_dot_dot_is_judged_by_where_it_lands() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );
    // Another work's torrent, written through the work folder's name.
    write(&s.collect.join("Other/Season 01/Other S01E01.mkv"), "other");
    s.h.tr.preload(
        FakeTorrent::new(&hash(2), "Other S01E01.mkv")
            .in_dir(s.collect.join("Clevatess/../Other/Season 01"))
            .status(SEEDING),
    );
    // And one of this work, written through another's name.
    write(
        &s.collect.join("Clevatess/Season 02/Clevatess S02E02.mkv"),
        "video",
    );
    s.h.tr.preload(
        FakeTorrent::new(&hash(3), "Clevatess S02E02.mkv")
            .in_dir(s.collect.join("Other/../Clevatess/Season 02"))
            .status(SEEDING),
    );
    let before = files(&s.collect);

    s.send("archive-dots-01", &c.rules[0].id, "archive").await;
    s.run().await;

    // The one that lands in the work folder stops the move: nothing moves.
    let command = s.command("archive-dots-01").await;
    assert_eq!(command["state"], "failed", "{command}");
    let reason = command["outcome"]["reason"].as_str().unwrap();
    assert!(reason.contains("`..`"), "{reason}");
    assert!(s.h.tr.calls_of("torrent-set-location").is_empty());
    assert_eq!(files(&s.collect), before);

    // Without it, the work moves and the other work's torrent stays.
    s.h.tr.remove(&hash(3));
    fs::remove_file(s.collect.join("Clevatess/Season 02/Clevatess S02E02.mkv")).unwrap();
    s.send("archive-dots-02", &c.rules[0].id, "archive").await;
    s.run().await;
    let command = s.command("archive-dots-02").await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(
        s.h.tr.torrent(&hash(2)).download_dir,
        text(s.collect.join("Clevatess/../Other/Season 01"))
    );
    assert_eq!(files(&s.collect), ["Other/Season 01/Other S01E01.mkv"]);
    let moved: Vec<Value> =
        s.h.tr
            .calls_of("torrent-set-location")
            .into_iter()
            .map(|c| c.args)
            .collect();
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert_eq!(moved[0]["ids"], json!([hash(1)]));
}

// --- review: Transmission 4 moves in the background --------------------------------------------

#[tokio::test]
async fn a_move_error_transmission_reports_fails_the_move_with_its_reason_without_waiting() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );
    write(&s.collect.join("Clevatess/Season 02/notes.txt"), "x");
    s.h.tr.async_locations(true);
    s.h.tr.fail_location_of(&hash(1), Some("Permission denied"));

    s.send("archive-err-01", &c.rules[0].id, "archive").await;
    let started = std::time::Instant::now();
    s.h.worker()
        .with_move_policy(MovePolicy {
            poll: Duration::from_millis(20),
            timeout: Duration::from_secs(60),
        })
        .run_commands(&CancellationToken::new())
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(20));

    let command = s.command("archive-err-01").await;
    assert_eq!(command["state"], "failed", "{command}");
    let reason = command["outcome"]["reason"].as_str().unwrap();
    assert!(reason.contains("Permission denied"), "{reason}");
    assert!(!reason.contains("정리"), "{reason}");
    // The worker renamed nothing of its own.
    assert_eq!(
        files(&s.collect),
        [
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/notes.txt"
        ]
    );
    assert!(files(&s.archive).is_empty());
}

#[tokio::test]
async fn transmission_slower_than_the_wait_keeps_the_command_for_the_next_look() {
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
    s.h.tr.async_locations(true);
    s.h.tr.lag_locations(u32::MAX);
    let slow = || {
        s.h.worker().with_move_policy(MovePolicy {
            poll: Duration::from_millis(20),
            timeout: Duration::from_millis(300),
        })
    };

    s.send("archive-slow-01", &rule.id, "archive").await;
    let outcome = slow()
        .run_commands(&CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(outcome, CommandsOutcome::Ran(0));
    let command = s.command("archive-slow-01").await;
    assert_eq!(command["state"], "running", "{command}");
    // The worker renamed nothing of its own meanwhile.
    assert_eq!(
        files(&s.collect),
        [
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/notes.txt"
        ]
    );

    // Transmission catches up; the next look finishes the same command.
    s.h.tr.settle_locations();
    s.h.tr.lag_locations(0);
    assert_eq!(s.run().await, CommandsOutcome::Ran(1));
    let command = s.command("archive-slow-01").await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "moved");
    assert!(!s.collect.join("Clevatess").exists());
    assert_eq!(
        files(&s.archive),
        [
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/notes.txt",
        ]
    );
}

#[tokio::test]
async fn transmission_that_never_reports_the_new_folder_fails_on_the_last_look_with_the_reason() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.collect.join("Clevatess/Season 02"),
    );
    s.h.tr.async_locations(true);
    s.h.tr.lag_locations(u32::MAX);

    s.send("archive-never-1", &c.rules[0].id, "archive").await;
    for _ in 0..trss_core::commands::MAX_ATTEMPTS {
        s.h.worker()
            .with_move_policy(MovePolicy {
                poll: Duration::from_millis(20),
                timeout: Duration::from_millis(100),
            })
            .run_commands(&CancellationToken::new())
            .await
            .unwrap();
    }
    let command = s.command("archive-never-1").await;
    assert_eq!(command["state"], "failed", "{command}");
    let reason = command["outcome"]["reason"].as_str().unwrap();
    assert!(reason.contains("Transmission"), "{reason}");
    assert!(reason.contains("다시 옮기면 남은 것만"), "{reason}");
    assert!(!reason.contains("정리"), "{reason}");
}

#[tokio::test]
async fn a_refusal_after_some_torrents_moved_says_so_and_moving_again_finishes() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    let season = s.collect.join("Clevatess/Season 02");
    s.seeding(1, "Clevatess S02E01.mkv", &season);
    s.seeding(2, "Clevatess S02E02.mkv", &season);
    s.h.tr.reject_location_of(&hash(2), Some("torrent is busy"));

    s.send("archive-part-01", &c.rules[0].id, "archive").await;
    s.run().await;

    let command = s.command("archive-part-01").await;
    assert_eq!(command["state"], "failed", "{command}");
    let reason = command["outcome"]["reason"].as_str().unwrap();
    assert!(reason.contains("torrent is busy"), "{reason}");
    assert!(reason.contains("1개"), "{reason}");
    assert!(reason.contains("다시 옮기면 남은 것만"), "{reason}");
    assert!(!reason.contains("정리"), "{reason}");

    s.h.tr.reject_location_of(&hash(2), None);
    s.send("archive-part-02", &c.rules[0].id, "archive").await;
    s.run().await;
    assert_eq!(s.command("archive-part-02").await["state"], "done");
    assert!(!s.collect.join("Clevatess").exists());
    for n in [1, 2] {
        assert_eq!(
            s.h.tr.torrent(&hash(n)).download_dir,
            text(s.archive.join("Clevatess/Season 02"))
        );
    }
}

// --- second review: the torrent's own data, metadata, links inside ------------------------------

#[tokio::test]
async fn another_file_of_the_same_name_only_at_the_destination_stops_the_move() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    let season = s.collect.join("Clevatess/Season 02");
    // A stopped, finished torrent of two files; the person removed `e05.mkv`
    // at the source, and the archive has a different `e05.mkv`.
    write(&season.join("Pack/e04.mkv"), "episode 4");
    s.h.tr.preload(
        FakeTorrent::new(&hash(1), "Pack")
            .in_dir(&season)
            .files(&["Pack/e04.mkv", "Pack/e05.mkv"])
            .file_length(9)
            .status(0),
    );
    write(
        &s.archive.join("Clevatess/Season 02/Pack/e05.mkv"),
        "someone else's file",
    );
    let before = (files(&s.collect), files(&s.archive));

    s.send("archive-own-01", &c.rules[0].id, "archive").await;
    s.run().await;

    let command = s.command("archive-own-01").await;
    assert_eq!(command["state"], "failed", "{command}");
    let reason = command["outcome"]["reason"].as_str().unwrap();
    assert!(reason.contains("`Season 02/Pack/e05.mkv`"), "{reason}");
    assert!(s.h.tr.calls_of("torrent-set-location").is_empty());
    assert_eq!((files(&s.collect), files(&s.archive)), before);
}

#[tokio::test]
async fn a_torrent_whose_files_were_all_moved_by_hand_is_pointed_at_them() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    // The person moved the work folder already; Transmission still names the
    // old folder, and the files at the archive are the torrent's, size for size.
    write(
        &s.archive.join("Clevatess/Season 02/Pack/e04.mkv"),
        "episode 4",
    );
    write(
        &s.archive.join("Clevatess/Season 02/Pack/e05.mkv"),
        "episode 5",
    );
    s.h.tr.preload(
        FakeTorrent::new(&hash(1), "Pack")
            .in_dir(s.collect.join("Clevatess/Season 02"))
            .files(&["Pack/e04.mkv", "Pack/e05.mkv"])
            .file_length(9)
            .status(0),
    );

    s.send("archive-hand-01", &c.rules[0].id, "archive").await;
    s.run().await;

    let command = s.command("archive-hand-01").await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(
        s.h.tr.torrent(&hash(1)).download_dir,
        text(s.archive.join("Clevatess/Season 02"))
    );
    assert_eq!(
        fs::read_to_string(s.archive.join("Clevatess/Season 02/Pack/e05.mkv")).unwrap(),
        "episode 5"
    );
}

#[tokio::test]
async fn a_torrent_split_between_the_two_folders_stops_the_move_without_asking_to_clear_a_side() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    // Transmission was stopped halfway through moving the torrent's data:
    // one file is at the archive, the other still at the collect folder,
    // each the only copy.
    write(
        &s.collect.join("Clevatess/Season 02/Pack/e04.mkv"),
        "episode 4",
    );
    write(
        &s.archive.join("Clevatess/Season 02/Pack/e05.mkv"),
        "episode 5",
    );
    s.h.tr.preload(
        FakeTorrent::new(&hash(1), "Pack")
            .in_dir(s.collect.join("Clevatess/Season 02"))
            .files(&["Pack/e04.mkv", "Pack/e05.mkv"])
            .file_length(9)
            .status(0),
    );
    let (collect_before, archive_before) = (files(&s.collect), files(&s.archive));

    s.send("archive-split-01", &c.rules[0].id, "archive").await;
    s.run().await;

    let command = s.command("archive-split-01").await;
    assert_eq!(command["state"], "failed", "{command}");
    let reason = command["outcome"]["reason"].as_str().unwrap();
    assert!(reason.contains("아무것도 옮기지 않았어요"), "{reason}");
    assert!(reason.contains("`Pack`"), "{reason}");
    assert!(
        reason.contains("수집 폴더와 보관 폴더에 나뉘어"),
        "{reason}"
    );
    assert!(reason.contains("어느 쪽도 지우지 말고"), "{reason}");
    assert!(reason.contains("남은 파일을 한쪽으로 모으거나"), "{reason}");
    assert!(
        reason.contains("Transmission에서 그 토렌트의 위치를"),
        "{reason}"
    );
    assert!(reason.contains("`다시 옮기기`를 눌러"), "{reason}");
    assert!(!reason.contains("정리"), "{reason}");
    assert_eq!(files(&s.collect), collect_before);
    assert_eq!(files(&s.archive), archive_before);
    assert!(s.h.tr.calls_of("torrent-set-location").is_empty());
}

#[tokio::test]
async fn a_magnet_still_fetching_its_metadata_or_a_downloading_torrent_stops_the_move() {
    for (n, torrent) in [
        (
            1,
            FakeTorrent::new(&hash(1), "Clevatess S02E03")
                .without_metadata()
                .status(0),
        ),
        (
            2,
            FakeTorrent::new(&hash(2), "Clevatess S02E03")
                .without_metadata()
                .status(4),
        ),
        // Downloading, although it reports nothing left.
        (
            3,
            FakeTorrent::new(&hash(3), "Clevatess S02E03.mkv").status(4),
        ),
    ] {
        let s = Scene::new(true).await;
        let c = s
            .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
            .await;
        let season = s.collect.join("Clevatess/Season 02");
        s.seeding(9, "Clevatess S02E01.mkv", &season);
        s.h.tr.preload(torrent.in_dir(&season));

        let id = format!("archive-meta-{n}");
        s.send(&id, &c.rules[0].id, "archive").await;
        s.run().await;

        let command = s.command(&id).await;
        assert_eq!(command["state"], "failed", "{n}: {command}");
        let reason = command["outcome"]["reason"].as_str().unwrap();
        assert!(reason.contains("`Clevatess S02E03"), "{reason}");
        assert!(reason.contains("지운 뒤"), "{reason}");
        assert!(reason.contains("`다시 옮기기`"), "{reason}");
        assert!(s.h.tr.calls_of("torrent-set-location").is_empty());
        assert!(files(&s.archive).is_empty());
    }
}

#[tokio::test]
async fn a_torrent_reached_through_a_link_inside_the_work_folder_stops_the_move() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 01")])
        .await;
    write(&s.collect.join("Clevatess/Season 01/e01.mkv"), "a");
    // `Season 02` is a link to another work's folder, and Transmission saves
    // through it.
    write(&s.collect.join("Other/Season 02/o01.mkv"), "o");
    symlink(
        s.collect.join("Other/Season 02"),
        s.collect.join("Clevatess/Season 02"),
    )
    .unwrap();
    s.h.tr.preload(
        FakeTorrent::new(&hash(1), "o01.mkv")
            .in_dir(s.collect.join("Clevatess/Season 02"))
            .status(SEEDING),
    );
    let before = files(&s.collect);

    s.send("archive-inlink-1", &c.rules[0].id, "archive").await;
    s.run().await;

    let command = s.command("archive-inlink-1").await;
    assert_eq!(command["state"], "failed", "{command}");
    let reason = command["outcome"]["reason"].as_str().unwrap();
    assert!(reason.contains("`o01.mkv`"), "{reason}");
    assert!(reason.contains("링크"), "{reason}");
    assert!(s.h.tr.calls_of("torrent-set-location").is_empty());
    assert_eq!(files(&s.collect), before);
    assert!(files(&s.archive).is_empty());
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
async fn a_new_rule_whose_work_folder_cannot_be_moved_stays_off_and_shows_the_reason() {
    let s = Scene::new(true).await;
    let c = s
        .channel("feed-a", &[("Clevatess", "Clevatess/Season 02")])
        .await;
    // The same file on both sides: the move refuses before it moves anything.
    write(
        &s.collect.join("Clevatess/Season 02/Clevatess S02E02.mkv"),
        "new",
    );
    write(
        &s.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv"),
        "old",
    );
    write(
        &s.archive.join("Clevatess/Season 01/Clevatess S01E01.mkv"),
        "old",
    );
    s.h.feeds
        .set_xml("feed-a", &feed_xml(&[(2, "Clevatess S03E01.mkv")]));
    let (collect_before, archive_before) = (files(&s.collect), files(&s.archive));

    // The Season 02 rule would take the item too: pause it, so only the new
    // rule could.
    s.h.channels
        .set_rule_state(&c.rules[0].id, RuleState::Paused, 0)
        .await
        .unwrap();
    let (status, rule) = s
        .create_rule(&c.channel.id, "Clevatess S03", "Clevatess/Season 03")
        .await;
    assert_eq!(status, StatusCode::CREATED, "{rule}");
    let id = rule["id"].as_str().unwrap();
    assert_eq!(s.run().await, CommandsOutcome::Ran(1));

    let view = s.rule(id).await;
    assert_eq!(view["state"], "paused");
    let moved = &view["archive_move"];
    assert_eq!(moved["direction"], "start");
    assert_eq!(moved["command"]["state"], "failed", "{moved}");
    let reason = moved["command"]["outcome"]["reason"].as_str().unwrap();
    assert!(
        reason.contains("`Season 02/Clevatess S02E02.mkv`"),
        "{reason}"
    );
    assert!(reason.contains("아무것도 옮기지 않았어요"), "{reason}");
    assert_eq!(files(&s.collect), collect_before);
    assert_eq!(files(&s.archive), archive_before);
    assert!(s.h.tr.calls_of("torrent-set-location").is_empty());

    // The rule receives nothing while it is off.
    s.h.feeds
        .set_xml("feed-a", &feed_xml(&[(2, "Clevatess S03 E01.mkv")]));
    let report = s.cycle(&s.worker()).await;
    assert_eq!(report.added, 0);
    assert!(s.added_dirs().is_empty());

    // Once the overlap is cleared, `영상 받기` on again starts it again.
    fs::remove_file(s.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv")).unwrap();
    let (status, view) = s.switch_video(&view, true).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["state"], "paused");
    // A rule that never collected starts again, so the time it waited does not
    // make the items that came meanwhile past ones.
    assert_eq!(view["archive_move"]["direction"], "start", "{view}");
    assert_eq!(view["archive_move"]["command"]["state"], "pending");
    assert_eq!(s.run().await, CommandsOutcome::Ran(1));
    let view = s.rule(id).await;
    assert_eq!(view["state"], "active");
    assert_eq!(
        view["archive_move"]["command"]["outcome"]["result"],
        "moved"
    );
    let stored = s.h.channels.get_rule(id).await.unwrap().unwrap();
    assert_eq!(stored.resumed_at, None);
    assert!(!s.archive.join("Clevatess").exists());
    assert_eq!(
        files(&s.collect),
        [
            "Clevatess/Season 01/Clevatess S01E01.mkv",
            "Clevatess/Season 02/Clevatess S02E02.mkv",
        ]
    );
}

#[tokio::test]
async fn turning_video_on_moves_an_archived_work_first_and_a_work_not_archived_turns_on_at_once() {
    let s = Scene::new(true).await;
    let c = s
        .channel(
            "feed-a",
            &[
                ("Solo", "Solo/Season 01"),
                ("Clevatess", "Clevatess/Season 02"),
                ("Fresh", "Fresh/Season 01"),
            ],
        )
        .await;
    for rule in &c.rules {
        s.h.channels
            .set_rule_state(&rule.id, RuleState::Paused, 0)
            .await
            .unwrap();
    }
    // Solo is in the archive folder only; Clevatess is in both folders; Fresh
    // is in neither.
    s.seeding(1, "Solo S01E01.mkv", &s.archive.join("Solo/Season 01"));
    write(
        &s.archive.join("Clevatess/Season 01/Clevatess S01E01.mkv"),
        "old",
    );
    write(
        &s.collect.join("Clevatess/Season 02/Clevatess S02E01.mkv"),
        "now",
    );
    let ids: Vec<String> = c.rules.iter().map(|r| r.id.clone()).collect();

    // A work that is not in the archive folder turns on at once, as before.
    let (status, fresh) = s.switch_video(&s.rule(&ids[2]).await, true).await;
    assert_eq!(status, StatusCode::OK, "{fresh}");
    assert_eq!(fresh["state"], "active");
    assert!(fresh["archive_move"].is_null());

    // An archived work waits: the rule stays off with the move open, and a
    // second press while it is open is refused.
    let (status, solo) = s.switch_video(&s.rule(&ids[0]).await, true).await;
    assert_eq!(status, StatusCode::OK, "{solo}");
    assert_eq!(solo["state"], "paused");
    assert_eq!(solo["archive_move"]["direction"], "resume");
    assert_eq!(solo["archive_move"]["command"]["state"], "pending");
    let (status, refused) = s.switch_video(&s.rule(&ids[0]).await, true).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(refused["message"].as_str().unwrap().contains("옮기는 중"));
    // A work in both folders merges the archive's into the collect folder's.
    let (status, _) = s.switch_video(&s.rule(&ids[1]).await, true).await;
    assert_eq!(status, StatusCode::OK);

    // The two starts queue behind each other and both run.
    assert_eq!(s.run().await, CommandsOutcome::Ran(2));

    assert_eq!(s.rule(&ids[0]).await["state"], "active");
    assert_eq!(s.rule(&ids[1]).await["state"], "active");
    assert!(!s.archive.join("Solo").exists());
    assert!(!s.archive.join("Clevatess").exists());
    assert_eq!(
        files(&s.collect),
        [
            "Clevatess/Season 01/Clevatess S01E01.mkv",
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Solo/Season 01/Solo S01E01.mkv",
        ]
    );
    assert_eq!(
        s.h.tr.torrent(&hash(1)).download_dir,
        text(s.collect.join("Solo/Season 01"))
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

#[tokio::test]
async fn a_rule_for_a_work_that_is_not_archived_is_made_on_and_collects_at_once() {
    let s = Scene::new(true).await;
    let c = s.channel("feed-a", &[]).await;
    write(
        &s.collect.join("Clevatess/Season 02/Clevatess S02E01.mkv"),
        "now",
    );
    // The archive folder has another work, and the rule's work is only in the
    // collect folder.
    write(&s.archive.join("Other/Season 01/Other S01E01.mkv"), "old");
    s.h.feeds
        .set_xml("feed-a", &feed_xml(&[(2, "Clevatess S03E01.mkv")]));

    let (status, rule) = s
        .create_rule(&c.channel.id, "Clevatess", "Clevatess/Season 03")
        .await;
    assert_eq!(status, StatusCode::CREATED, "{rule}");
    assert_eq!(rule["state"], "active");
    assert!(rule["archive_move"].is_null());

    let report = s.cycle(&s.worker()).await;
    assert_eq!(report.added, 1);
    assert_eq!(
        s.added_dirs(),
        [text(s.collect.join("Clevatess/Season 03"))]
    );
    assert_eq!(files(&s.archive), ["Other/Season 01/Other S01E01.mkv"]);
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

    /// `receive_once` of `item_id`, stored as the retry button does.
    async fn retry(&self, command_id: &str, item_id: i64) {
        let (status, _, body) = self
            .api
            .call(
                "POST",
                "/api/commands",
                Some(json!({
                    "id": command_id,
                    "kind": "receive_once",
                    "payload": { "item_id": item_id },
                })),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    }

    async fn resumed_at(&self, rule_id: &str) -> Option<i64> {
        self.h
            .channels
            .get_rule(rule_id)
            .await
            .unwrap()
            .unwrap()
            .resumed_at
    }
}

#[tokio::test]
async fn a_cycle_leaves_an_item_of_a_rule_whose_work_folder_is_archived_and_brings_the_folder_in_first(
) {
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

    // The rule is on, the work folder is in the archive folder: the item is
    // not received, no folder is made in the collect folder, and nothing is
    // recorded for it.
    let worker = s.worker();
    let report = s.cycle(&worker).await;
    assert_eq!(report.added, 0);
    assert_eq!(report.waiting_for_move, 1);
    assert_eq!(report.moves_asked, 1);
    assert!(s.added_dirs().is_empty());
    assert!(!s.collect.join("Clevatess").exists());
    assert!(s.h.history_items().await.is_empty());

    // The rule is off with its work folder's move open, which the screen tells
    // as a move, and it keeps the time it was turned on.
    let view = s.rule(&id).await;
    assert_eq!(view["state"], "paused", "{view}");
    assert_eq!(view["archive_move"]["direction"], "start", "{view}");
    assert_eq!(view["archive_move"]["command"]["state"], "pending");
    let command_id = view["archive_move"]["command"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(s.resumed_at(&id).await, Some(777));

    // Another check beside the open move stores nothing more.
    let report = s.cycle(&worker).await;
    assert_eq!(report.moves_asked, 0);
    assert!(s.added_dirs().is_empty());

    // The command brings the folder in and turns the rule on, still with its
    // old `resumed_at`.
    assert_eq!(s.run().await, CommandsOutcome::Ran(1));
    let command = s.command(&command_id).await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "moved");
    assert!(!s.archive.join("Clevatess").exists());
    assert_eq!(
        files(&s.collect),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert_eq!(s.rule(&id).await["state"], "active");
    assert_eq!(s.resumed_at(&id).await, Some(777));

    // The next check receives the item into the work folder that came over.
    let report = s.cycle(&worker).await;
    assert_eq!(report.added, 1);
    assert_eq!(report.waiting_for_move, 0);
    assert_eq!(
        s.added_dirs(),
        [text(s.collect.join("Clevatess/Season 03"))]
    );
}

#[tokio::test]
async fn a_cycle_also_heals_a_work_split_across_both_folders_by_merging_the_archive_into_the_collect_one(
) {
    let s = Scene::new(true).await;
    let id = s
        .rule_made_in_the_store("feed-a", "Clevatess", "Clevatess/Season 03")
        .await;
    write(
        &s.collect.join("Clevatess/Season 01/Clevatess S01E01.mkv"),
        "a",
    );
    write(
        &s.archive.join("Clevatess/Season 02/Clevatess S02E01.mkv"),
        "b",
    );
    s.h.feeds
        .set_xml("feed-a", &feed_xml(&[(2, "Clevatess S03E01.mkv")]));

    let worker = s.worker();
    let report = s.cycle(&worker).await;
    assert_eq!(report.added, 0);
    assert_eq!(report.moves_asked, 1);
    assert_eq!(s.rule(&id).await["state"], "paused");

    assert_eq!(s.run().await, CommandsOutcome::Ran(1));
    assert!(!s.archive.join("Clevatess").exists());
    assert_eq!(
        files(&s.collect),
        [
            "Clevatess/Season 01/Clevatess S01E01.mkv",
            "Clevatess/Season 02/Clevatess S02E01.mkv"
        ]
    );
    assert_eq!(s.rule(&id).await["state"], "active");
    assert_eq!(s.cycle(&worker).await.added, 1);
}

#[tokio::test]
async fn a_cycle_with_a_move_already_open_for_the_rule_leaves_the_item_and_stores_nothing_more() {
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
    let commands = CommandStore::new(s.h.db.clone());
    let accepted = rule_archive::ask_start(&commands, &id, Direction::Start, s.h.now())
        .await
        .unwrap();
    let Accepted::Created(open) = accepted else {
        panic!("expected a stored command: {accepted:?}");
    };

    let worker = s.worker();
    let report = s.cycle(&worker).await;
    assert_eq!(report.added, 0);
    assert_eq!(report.waiting_for_move, 1);
    assert_eq!(report.moves_asked, 0);
    assert!(s.added_dirs().is_empty());
    assert!(!s.collect.join("Clevatess").exists());
    // The open command is the only one, and the rule was left as it was.
    let view = s.rule(&id).await;
    assert_eq!(view["archive_move"]["command"]["id"], open.id);
    assert_eq!(view["state"], "active", "{view}");

    // It runs, and the next check receives the item.
    assert_eq!(s.run().await, CommandsOutcome::Ran(1));
    assert_eq!(s.command(&open.id).await["state"], "done");
    assert_eq!(s.cycle(&worker).await.added, 1);
}

#[tokio::test]
async fn a_retry_into_an_archived_work_folder_ends_saying_so_pauses_the_rule_and_the_move_runs() {
    let s = Scene::new(true).await;
    let id = s
        .rule_made_in_the_store("feed-a", "Clevatess", "Clevatess/Season 02")
        .await;
    // Transmission refused the rule's next episode: `다시 받기` is offered.
    s.h.feeds
        .set_xml("feed-a", &feed_xml(&[(2, "Clevatess S02E02.mkv")]));
    s.h.tr.reject_adds(Some("not today"));
    s.cycle(&s.worker()).await;
    s.h.tr.reject_adds(None);
    s.h.tr.clear_calls();
    let item = s.h.item("Clevatess S02E02").await;
    // The work folder went to the archive folder meanwhile.
    s.seeding(
        1,
        "Clevatess S02E01.mkv",
        &s.archive.join("Clevatess/Season 02"),
    );

    // The retry ends first, and the move it asked for starts in the same look:
    // held at Transmission, the rule is still off.
    let gate = s.h.tr.hold("torrent-set-location");
    s.retry("retry-guard-0001", item.id).await;
    let look = tokio::spawn({
        let worker = s.worker();
        async move { worker.run_commands(&CancellationToken::new()).await }
    });
    gate.wait_arrived().await;
    let retry = s.command("retry-guard-0001").await;
    assert_eq!(retry["state"], "failed", "{retry}");
    let reason = retry["outcome"]["reason"].as_str().unwrap();
    assert!(reason.contains("먼저 수집 폴더로 옮기고"), "{reason}");
    assert!(s.h.tr.calls_of("torrent-add").is_empty());
    assert!(!s.collect.join("Clevatess").exists());
    // The item is as it was, and the rule is off with its move open.
    assert_eq!(s.h.item("Clevatess S02E02").await.result, item.result);
    let view = s.rule(&id).await;
    assert_eq!(view["state"], "paused", "{view}");
    assert_eq!(view["archive_move"]["direction"], "start", "{view}");
    assert_eq!(s.resumed_at(&id).await, Some(777));

    // The move ends; the retry then goes into the folder that came over.
    gate.release_all();
    assert_eq!(look.await.unwrap().unwrap(), CommandsOutcome::Ran(2));
    assert_eq!(s.rule(&id).await["state"], "active");
    assert_eq!(s.resumed_at(&id).await, Some(777));
    assert!(!s.archive.join("Clevatess").exists());
    s.h.tr.on_disk(s.h.dir.path());
    s.h.tr.content_on_add(&hash(2), b"video");
    s.h.tr.seeding_on_add(&hash(2));
    s.retry("retry-guard-0002", item.id).await;
    assert_eq!(s.run().await, CommandsOutcome::Ran(1));
    let retry = s.command("retry-guard-0002").await;
    assert_eq!(retry["outcome"]["result"], "received", "{retry}");
    assert_eq!(
        s.h.tr.torrent(&hash(2)).download_dir,
        text(s.collect.join("Clevatess/Season 02"))
    );
}
