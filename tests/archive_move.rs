//! Archiving and restoring a rule with its work folder (the `rule_archive`
//! command), end to end: the real commands and rules HTTP API, the real worker
//! and Transmission client, a fake Transmission that moves torrent data on the
//! real disk, and real temporary folders as the collect and archive folders.
//! Ticket 0011's completion rows.
//!
//! The worker in these tests runs as the test's own user, so every folder has
//! one owner: a test cannot make Transmission's user differ from the worker's
//! without root. What is checked is that the folders Transmission creates take
//! their parent's owner and group, and that what the worker moves is renamed
//! (the same inode, hence the same owner), never copied or created by it.

mod common;

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
use transmission_rss::{
    store::{
        channels::{ChannelInput, ChannelWithRules, RuleInput, RuleState},
        commands::{CommandState, CommandStore},
        settings::SettingsStore,
    },
    worker::{CommandsOutcome, MovePolicy, TickOutcome, Worker},
};

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
        .set_rule_state(&rule.id, RuleState::Archived)
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

    // Mid-move: a cycle and a second look for commands both find the lock taken.
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
async fn transmission_that_never_reports_the_new_folder_fails_the_move_and_moving_again_finishes() {
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
    s.h.tr.lag_locations(u32::MAX);

    s.send("archive-slow-01", &rule.id, "archive").await;
    s.h.worker()
        .with_move_policy(MovePolicy {
            poll: Duration::from_millis(20),
            timeout: Duration::from_secs(1),
        })
        .run_commands(&CancellationToken::new())
        .await
        .unwrap();
    let command = s.command("archive-slow-01").await;
    assert_eq!(command["state"], "failed", "{command}");
    assert!(command["outcome"]["reason"]
        .as_str()
        .unwrap()
        .contains("옮기지 못했어요"));
    // The worker renamed nothing of its own.
    assert_eq!(files(&s.collect), ["Clevatess/Season 02/notes.txt"]);

    // Transmission catches up; `다시 옮기기` moves the rest.
    s.h.tr.lag_locations(0);
    s.h.tr.settle_locations();
    s.send("archive-slow-02", &rule.id, "archive").await;
    s.run().await;
    assert_eq!(
        s.command("archive-slow-02").await["outcome"]["result"],
        "moved"
    );
    assert!(!s.collect.join("Clevatess").exists());
    assert_eq!(
        files(&s.archive),
        [
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/notes.txt",
        ]
    );
    assert_eq!(s.h.tr.calls_of("torrent-set-location").len(), 1);
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
