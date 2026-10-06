//! Changes in the watch folders taken from inotify alerts, end to end (ticket
//! 0016): a real worker, real temporary folders and the kernel's own alerts, the
//! real web API for registering folders, a fake Transmission for the archive
//! move, and no mock of the watching itself.
//!
//! The worker's clock is the harness's manual one, so what time a file gets is
//! exact; the waits are real, so they poll the database with a generous limit
//! instead of sleeping for a guessed time. Debounce and retry are shortened so
//! the tests are quick, and the safety net keeps its hour (the clock is
//! manual, so a test moves the hour itself).

use crate::common;

use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
    time::Duration,
};

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use trss_collect::{
    commands::rule_archive::work_folder::MovePolicy,
    store::channels::{ChannelInput, ChannelWithRules, RuleInput},
};
use trss_core::{heartbeat::HeartbeatStore, lock_path_for, CycleLock};
use trss_library::{
    discovery::Reason,
    live::{LiveConfig, Reading},
    store::library::{LibraryStore, WatchFolder, WorkRecord},
};
use trss_worker::{CommandsOutcome, CycleReport, TickOutcome, Worker};

const DEBOUNCE: Duration = Duration::from_millis(150);

fn config() -> LiveConfig {
    LiveConfig {
        debounce: DEBOUNCE,
        retry: Duration::from_millis(40),
        ..LiveConfig::default()
    }
}

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "x").unwrap();
}

fn text(path: impl AsRef<Path>) -> String {
    path.as_ref().to_str().unwrap().to_owned()
}

/// Waits until `check` holds, polling; a generous limit, not a pause.
async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    for _ in 0..600 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for: {what}");
}

struct Live {
    h: Harness,
    worker: Worker,
    api: WebApi,
    library: LibraryStore,
    media: PathBuf,
}

impl Live {
    async fn new(config: LiveConfig) -> Live {
        let h = Harness::without_collect_folder().await;
        let media = h.dir.path().join("media");
        fs::create_dir_all(&media).unwrap();
        Live {
            api: h.web_api(),
            library: LibraryStore::new(h.db.clone()),
            worker: h
                .worker()
                .with_live_config(config)
                .with_move_policy(MovePolicy {
                    poll: Duration::from_millis(10),
                    timeout: Duration::from_secs(5),
                }),
            h,
            media,
        }
    }

    fn folder(&self, name: &str) -> PathBuf {
        let path = self.media.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// Registers a folder through the web API (which reads it once).
    async fn register(&self, path: &Path) -> WatchFolder {
        let (status, _, body) = self
            .api
            .call(
                "POST",
                "/api/library/watch-folders",
                Some(json!({ "path": text(path) })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        self.library
            .folder(body["folder"]["id"].as_str().unwrap())
            .await
            .unwrap()
            .unwrap()
    }

    async fn unregister(&self, folder: &WatchFolder) {
        let (status, _, body) = self
            .api
            .call(
                "DELETE",
                &format!("/api/library/watch-folders/{}", folder.id),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    async fn save_collection(&self, collect: &Path, archive: &Path) {
        let (status, text, body) = self
            .api
            .call(
                "PUT",
                "/api/settings/collection",
                Some(json!({
                    "version": 0,
                    "folder": text(collect),
                    "archive_folder": text(archive),
                })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert!(body != Value::Null);
    }

    async fn folder_at(&self, path: &Path) -> WatchFolder {
        self.library
            .folders()
            .await
            .unwrap()
            .into_iter()
            .find(|f| f.path == text(path))
            .unwrap()
    }

    async fn works(&self, folder: &WatchFolder) -> Vec<WorkRecord> {
        self.library.works(&folder.id).await.unwrap()
    }

    async fn work(&self, folder: &WatchFolder, name: &str) -> Option<WorkRecord> {
        self.works(folder)
            .await
            .into_iter()
            .find(|w| w.dir_name == name)
    }

    /// The `added_at` of a file of a work, once the library has the file.
    async fn added_at(&self, folder: &WatchFolder, work: &str, file: &str) -> Option<Option<i64>> {
        let work = self.work(folder, work).await?;
        let added = work.files().get(file).map(|f| f.added_at);
        added
    }

    async fn has_file(&self, folder: &WatchFolder, work: &str, file: &str) -> bool {
        self.added_at(folder, work, file).await.is_some()
    }

    async fn has_unrecognized(&self, folder: &WatchFolder, work: &str, path: &str) -> bool {
        match self.work(folder, work).await {
            Some(work) => work.unrecognized.iter().any(|u| u.path == path),
            None => false,
        }
    }

    async fn tick(&self) -> CycleReport {
        match self.worker.tick(&CancellationToken::new()).await.unwrap() {
            TickOutcome::Ran(report) => report,
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    /// The readings made so far, to compare with later.
    fn readings(&self) -> Vec<Reading> {
        self.worker.live().readings()
    }

    fn readings_since(&self, before: usize) -> Vec<Reading> {
        self.readings().split_off(before)
    }

    /// A folder with two works, registered, watched, and read by the first cycle.
    async fn two_works(&self) -> (PathBuf, WatchFolder) {
        let root = self.folder("anime");
        touch(&root.join("A/Season 01/A S01E01.mkv"));
        touch(&root.join("B/Season 01/B S01E01.mkv"));
        let folder = self.register(&root).await;
        self.worker.start_watching().await;
        self.h.advance(1000);
        self.tick().await;
        (root, folder)
    }
}

#[tokio::test]
async fn a_file_added_while_the_worker_runs_is_recorded_without_a_cycle_and_only_its_work_is_read()
{
    let lib = Live::new(config()).await;
    let (root, folder) = lib.two_works().await;
    let status = lib.worker.live().status(&folder.id).unwrap();
    // The folder, A, A's season, B and B's season.
    assert_eq!(status.watches, 5);
    assert!(status.root_watched && status.unwatched_dirs == 0);
    let before = lib.readings().len();

    lib.h.advance(5000);
    let stamp = lib.h.now();
    touch(&root.join("A/Season 01/A S01E02.mkv"));

    eventually("the new episode is recorded", || async {
        lib.added_at(&folder, "A", "Season 01/A S01E02.mkv").await == Some(Some(stamp))
    })
    .await;
    // Nothing but A was read, and no whole folder: B was left alone.
    let read = lib.readings_since(before);
    assert!(!read.is_empty());
    assert!(
        read.iter()
            .all(|r| *r == Reading::Work(folder.id.clone(), "A".into())),
        "{read:?}"
    );
    // The work's other records are as they were.
    assert_eq!(lib.work(&folder, "B").await.unwrap().files().len(), 1);
}

#[tokio::test]
async fn a_reading_an_alert_asked_for_beats_under_the_lock_and_lets_go_of_the_hold() {
    let lib = Live::new(config()).await;
    let (root, _folder) = lib.two_works().await;
    let heartbeat = HeartbeatStore::new(lib.h.db.clone());
    let before = heartbeat.read().await.unwrap().expect("the cycle's beat");

    lib.h.advance(5000);
    let stamp = lib.h.now();
    touch(&root.join("A/Season 01/A S01E02.mkv"));

    // The reading beat at its own time, then let go: the board keeps a worker
    // that reads its folders for a long while from looking stopped.
    eventually("the reading's beat", || async {
        let beat = heartbeat.read().await.unwrap().unwrap();
        beat.beat_at == stamp && beat.held_since.is_none()
    })
    .await;
    assert!(before.beat_at < stamp);
}

#[tokio::test]
async fn a_part_file_that_keeps_growing_is_not_read_again_and_its_rename_is_recorded() {
    let lib = Live::new(config()).await;
    let (root, folder) = lib.two_works().await;
    let season = root.join("A/Season 01");

    lib.h.advance(1000);
    fs::write(season.join("A S01E02.mkv.part"), "x").unwrap();
    eventually("the download in progress is counted", || async {
        lib.work(&folder, "A")
            .await
            .unwrap()
            .unrecognized
            .iter()
            .any(|u| u.path.ends_with(".part") && u.reason == Reason::Partial)
    })
    .await;
    // Let the reading that its creation caused finish.
    tokio::time::sleep(DEBOUNCE * 2).await;
    let (_, reads) = lib.worker.live().read_counts();

    // Writing more (the download) tells the worker nothing.
    for _ in 0..5 {
        let mut part = fs::OpenOptions::new()
            .append(true)
            .open(season.join("A S01E02.mkv.part"))
            .unwrap();
        std::io::Write::write_all(&mut part, &[0u8; 4096]).unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
    tokio::time::sleep(DEBOUNCE * 3).await;
    assert_eq!(lib.worker.live().read_counts().1, reads);

    // Done: the rename to the final name is an alert, and the episode is recorded.
    lib.h.advance(1000);
    let stamp = lib.h.now();
    fs::rename(
        season.join("A S01E02.mkv.part"),
        season.join("A S01E02.mkv"),
    )
    .unwrap();
    eventually("the finished episode is recorded", || async {
        lib.added_at(&folder, "A", "Season 01/A S01E02.mkv").await == Some(Some(stamp))
    })
    .await;
    assert!(lib
        .work(&folder, "A")
        .await
        .unwrap()
        .unrecognized
        .is_empty());
}

#[tokio::test]
async fn a_new_work_with_a_season_and_a_file_made_together_is_recorded_and_watched() {
    let lib = Live::new(config()).await;
    let (root, folder) = lib.two_works().await;
    let watches = lib.worker.live().status(&folder.id).unwrap().watches;

    lib.h.advance(1000);
    let stamp = lib.h.now();
    touch(&root.join("New/Season 01/New S01E01.mkv"));
    eventually("the new work, season and episode are recorded", || async {
        lib.added_at(&folder, "New", "Season 01/New S01E01.mkv")
            .await
            == Some(Some(stamp))
    })
    .await;
    let new = lib.work(&folder, "New").await.unwrap();
    assert_eq!(new.seasons, [1]);
    assert_eq!(new.first_seen_at, Some(stamp));
    // The new work and its season are watched now.
    let status = lib.worker.live().status(&folder.id).unwrap();
    assert_eq!(status.watches, watches + 2);

    // So the next file is an alert too, without any cycle.
    lib.h.advance(1000);
    let later = lib.h.now();
    touch(&root.join("New/Season 01/New S01E02.mkv"));
    eventually("the next episode is recorded", || async {
        lib.added_at(&folder, "New", "Season 01/New S01E02.mkv")
            .await
            == Some(Some(later))
    })
    .await;
    // A new season of an existing work is watched the same way.
    lib.h.advance(1000);
    let season_two = lib.h.now();
    touch(&root.join("A/Season 02/A S02E01.mkv"));
    eventually("the new season's episode is recorded", || async {
        lib.added_at(&folder, "A", "Season 02/A S02E01.mkv").await == Some(Some(season_two))
    })
    .await;
    lib.h.advance(1000);
    let after = lib.h.now();
    touch(&root.join("A/Season 02/A S02E02.mkv"));
    eventually(
        "the second episode of the new season is recorded",
        || async {
            lib.added_at(&folder, "A", "Season 02/A S02E02.mkv").await == Some(Some(after))
        },
    )
    .await;
}

#[tokio::test]
async fn a_work_folder_that_is_removed_or_moved_out_is_missing_and_loses_its_watches() {
    let lib = Live::new(config()).await;
    let (root, folder) = lib.two_works().await;
    touch(&root.join("C/Season 01/C S01E01.mkv"));
    eventually("C is recorded", || async {
        lib.work(&folder, "C").await.is_some()
    })
    .await;
    assert_eq!(lib.worker.live().status(&folder.id).unwrap().watches, 7);

    fs::remove_dir_all(root.join("A")).unwrap();
    let outside = lib.folder("elsewhere");
    fs::rename(root.join("C"), outside.join("C")).unwrap();
    eventually("A and C are missing", || async {
        lib.work(&folder, "A").await.unwrap().missing
            && lib.work(&folder, "C").await.unwrap().missing
    })
    .await;
    assert!(!lib.work(&folder, "B").await.unwrap().missing);
    // Only the folder and B (with its season) are watched.
    let live = lib.worker.live();
    eventually("the watches are released", || async {
        live.status(&folder.id).unwrap().watches == 3
    })
    .await;

    // What happens to the moved folder is not heard under its old name...
    let before = lib.readings().len();
    touch(&outside.join("C/Season 01/C S01E02.mkv"));
    tokio::time::sleep(DEBOUNCE * 3).await;
    assert!(lib.readings_since(before).is_empty());
    // ...and a folder that comes back is a work again, with its ID.
    let id = lib.work(&folder, "A").await.unwrap().id;
    lib.h.advance(1000);
    touch(&root.join("A/Season 01/A S01E01.mkv"));
    eventually("A is back", || async {
        let work = lib.work(&folder, "A").await.unwrap();
        !work.missing && work.id == id
    })
    .await;
}

#[tokio::test]
async fn work_read_by_an_alert_waits_for_the_workers_lock() {
    let lib = Live::new(config()).await;
    let (root, folder) = lib.two_works().await;
    let held = CycleLock::try_acquire(&lock_path_for(&lib.h.db_path()))
        .unwrap()
        .expect("the lock is free between cycles");

    lib.h.advance(1000);
    touch(&root.join("A/Season 01/A S01E02.mkv"));
    // The alert comes and its debounce passes, but another cycle holds the lock.
    tokio::time::sleep(DEBOUNCE * 5).await;
    assert!(!lib.has_file(&folder, "A", "Season 01/A S01E02.mkv").await);

    drop(held);
    eventually("the episode is recorded once the lock is free", || async {
        lib.has_file(&folder, "A", "Season 01/A S01E02.mkv").await
    })
    .await;
}

// --- the archive move ------------------------------------------------------------------

struct Archive {
    lib: Live,
    collect: PathBuf,
    archive: PathBuf,
    collect_folder: WatchFolder,
    archive_folder: WatchFolder,
    channel: ChannelWithRules,
}

impl Archive {
    async fn new() -> Archive {
        let lib = Live::new(config()).await;
        let collect = lib.folder("Shows (current)");
        let archive = lib.folder("Shows");
        lib.h.feeds.set_xml(
            "feed-a",
            r#"<?xml version="1.0"?><rss version="2.0"><channel><title>x</title>
            <link>http://x/</link><description>d</description></channel></rss>"#,
        );
        let channel = lib
            .h
            .channels
            .create_channel_with_rules(
                ChannelInput::new(lib.h.feeds.url("feed-a")),
                vec![
                    RuleInput {
                        r#match: Some("Clevatess".into()),
                        directory: "Clevatess/Season 02".into(),
                        ..RuleInput::default()
                    },
                    RuleInput {
                        r#match: Some("Solo".into()),
                        directory: "Solo/Season 01".into(),
                        ..RuleInput::default()
                    },
                ],
            )
            .await
            .unwrap();
        touch(&collect.join("Clevatess/Season 02/Clevatess S02E01.mkv"));
        touch(&collect.join("Solo/Season 01/Solo S01E01.mkv"));
        touch(&archive.join("Clevatess/Season 01/Clevatess S01E01.mkv"));
        lib.save_collection(&collect, &archive).await;
        let collect_folder = lib.folder_at(&collect).await;
        let archive_folder = lib.folder_at(&archive).await;
        lib.worker.start_watching().await;
        lib.h.advance(1000);
        lib.tick().await;
        Archive {
            lib,
            collect,
            archive,
            collect_folder,
            archive_folder,
            channel,
        }
    }
}

#[tokio::test]
async fn an_archive_move_continues_the_work_under_the_archive_folder_and_moves_its_watches() {
    let a = Archive::new().await;
    let live = a.lib.worker.live();
    // Collect: the folder, Clevatess and Solo with a season each. Archive: the
    // folder and Clevatess with its season.
    assert_eq!(live.status(&a.collect_folder.id).unwrap().watches, 5);
    assert_eq!(live.status(&a.archive_folder.id).unwrap().watches, 3);
    let solo = a.lib.work(&a.collect_folder, "Solo").await.unwrap();

    let (status, _, body) = a
        .lib
        .api
        .call(
            "POST",
            "/api/commands",
            Some(json!({
                "id": "archive-solo",
                "kind": "rule_archive",
                "payload": { "rule_id": a.channel.rules[1].id, "direction": "archive" },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(
        a.lib
            .worker
            .run_commands(&CancellationToken::new())
            .await
            .unwrap(),
        CommandsOutcome::Ran(1)
    );
    assert!(a.archive.join("Solo/Season 01/Solo S01E01.mkv").exists());

    // The same ID now belongs to the archive folder, and the alerts of the move
    // that follow leave it as it is.
    let moved = a.lib.work(&a.archive_folder, "Solo").await.unwrap();
    assert_eq!(moved.id, solo.id);
    eventually("both folders' watches follow the work", || async {
        live.status(&a.collect_folder.id).unwrap().watches == 3
            && live.status(&a.archive_folder.id).unwrap().watches == 5
    })
    .await;
    tokio::time::sleep(DEBOUNCE * 4).await;
    let after = a.lib.work(&a.archive_folder, "Solo").await.unwrap();
    assert_eq!(after.id, solo.id);
    assert!(!after.missing);
    assert!(a.lib.work(&a.collect_folder, "Solo").await.is_none());

    // A file that arrives in the work's new place is an alert there.
    a.lib.h.advance(1000);
    let stamp = a.lib.h.now();
    touch(&a.archive.join("Solo/Season 01/Solo S01E02.mkv"));
    eventually(
        "the episode is recorded under the archive folder",
        || async {
            a.lib
                .added_at(&a.archive_folder, "Solo", "Season 01/Solo S01E02.mkv")
                .await
                == Some(Some(stamp))
        },
    )
    .await;
    // And the folder it left hears nothing of it.
    let before = a.lib.readings().len();
    touch(&a.collect.join("Clevatess/Season 02/Clevatess S02E02.mkv"));
    eventually("the collect folder's other work is read", || async {
        a.lib
            .has_file(
                &a.collect_folder,
                "Clevatess",
                "Season 02/Clevatess S02E02.mkv",
            )
            .await
    })
    .await;
    assert!(a
        .lib
        .readings_since(before)
        .iter()
        .all(|r| matches!(r, Reading::Work(_, name) if name == "Clevatess")));
    assert!(a.lib.work(&a.collect_folder, "Solo").await.is_none());
}

// --- what the alerts cannot tell -------------------------------------------------------

#[tokio::test]
async fn a_change_made_while_the_worker_was_off_is_found_by_the_first_cycle_after_the_start() {
    let lib = Live::new(config()).await;
    let root = lib.folder("anime");
    touch(&root.join("A/Season 01/A S01E01.mkv"));
    touch(&root.join("A/Season 01/A S01E02.mkv"));
    touch(&root.join("B/Season 01/B S01E01.mkv"));
    let folder = lib.register(&root).await;

    // Nothing is watching while these happen.
    lib.h.advance(1000);
    touch(&root.join("A/Season 01/A S01E03.mkv"));
    fs::remove_file(root.join("A/Season 01/A S01E01.mkv")).unwrap();
    fs::remove_dir_all(root.join("B")).unwrap();

    lib.worker.start_watching().await;
    lib.h.advance(1000);
    let stamp = lib.h.now();
    lib.tick().await;

    let a = lib.work(&folder, "A").await.unwrap();
    let files = a.files();
    assert!(!files.contains_key("Season 01/A S01E01.mkv"));
    assert_eq!(files["Season 01/A S01E03.mkv"].added_at, Some(stamp));
    assert!(lib.work(&folder, "B").await.unwrap().missing);
    // It was the whole folder, once.
    assert_eq!(lib.readings(), vec![Reading::Folder(folder.id.clone())]);
}

#[tokio::test]
async fn a_queue_overflow_reads_the_whole_folder_and_finds_what_no_alert_told() {
    let lib = Live::new(config()).await;
    let (root, folder) = lib.two_works().await;
    let season = root.join("A/Season 01");

    // A folder inside a season folder has no watch of its own.
    fs::create_dir_all(season.join("Batch")).unwrap();
    tokio::time::sleep(DEBOUNCE * 4).await;
    lib.h.advance(1000);
    touch(&season.join("Batch/ep 02.mkv"));
    tokio::time::sleep(DEBOUNCE * 4).await;
    assert!(
        !lib.has_unrecognized(&folder, "A", "Season 01/Batch/ep 02.mkv")
            .await
    );

    // The kernel says it dropped alerts: the whole folder is read.
    let before = lib.readings().len();
    lib.worker.live().simulate_overflow(&folder.id);
    eventually("the folder is read whole and the file is found", || async {
        lib.has_unrecognized(&folder, "A", "Season 01/Batch/ep 02.mkv")
            .await
    })
    .await;
    assert!(lib
        .readings_since(before)
        .contains(&Reading::Folder(folder.id.clone())));
}

#[tokio::test]
async fn directories_without_a_watch_are_read_by_every_cycle_and_the_row_says_why() {
    // Room for the folder and two works with a season each, not for the third.
    let lib = Live::new(LiveConfig {
        max_watches: Some(5),
        ..config()
    })
    .await;
    let root = lib.folder("anime");
    for work in ["A", "B", "C"] {
        touch(&root.join(format!("{work}/Season 01/{work} S01E01.mkv")));
    }
    let folder = lib.register(&root).await;
    lib.worker.start_watching().await;
    lib.h.advance(1000);
    lib.tick().await;

    let status = lib.worker.live().status(&folder.id).unwrap();
    assert_eq!(status.watches, 5);
    assert_eq!(status.unwatched_dirs, 1);
    assert_eq!(
        status.unwatched_works.iter().collect::<Vec<_>>(),
        ["C"],
        "the third work is the one left out"
    );
    // The row says how many and why, through the API the screen reads.
    eventually("the row carries the note", || async {
        lib.library
            .folder(&folder.id)
            .await
            .unwrap()
            .unwrap()
            .watch_note
            .is_some()
    })
    .await;
    let (status, _, body) = lib
        .api
        .call("GET", "/api/library/watch-folders", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let note = body["folders"][0]["watch_note"].as_str().unwrap();
    assert!(note.contains("폴더 1개"), "{note}");
    assert!(note.contains("fs.inotify.max_user_watches"), "{note}");

    // The watched works are heard...
    lib.h.advance(1000);
    let stamp = lib.h.now();
    touch(&root.join("A/Season 01/A S01E02.mkv"));
    eventually("A's episode is recorded by its alert", || async {
        lib.added_at(&folder, "A", "Season 01/A S01E02.mkv").await == Some(Some(stamp))
    })
    .await;

    // ...and the one without a watch is read by the next cycle, alone.
    lib.h.advance(1000);
    let cycle = lib.h.now();
    touch(&root.join("C/Season 01/C S01E02.mkv"));
    tokio::time::sleep(DEBOUNCE * 3).await;
    assert!(!lib.has_file(&folder, "C", "Season 01/C S01E02.mkv").await);
    let before = lib.readings().len();
    lib.tick().await;
    assert_eq!(
        lib.added_at(&folder, "C", "Season 01/C S01E02.mkv").await,
        Some(Some(cycle))
    );
    assert_eq!(
        lib.readings_since(before),
        vec![Reading::Work(folder.id.clone(), "C".into())]
    );
}

#[tokio::test]
async fn a_quiet_hour_reads_no_folder_but_the_safety_net() {
    let lib = Live::new(config()).await;
    let (_, folder) = lib.two_works().await;
    let (folders_read, works_read) = lib.worker.live().read_counts();
    assert_eq!(folders_read, 1, "the first cycle after the start");

    // Eleven cycles over 55 minutes: every directory is watched, nothing changes.
    for _ in 0..11 {
        lib.h.advance(5 * 60 * 1000);
        lib.tick().await;
    }
    assert_eq!(lib.worker.live().read_counts(), (1, works_read));

    // The twelfth reaches the hour since the last whole read: one read, once.
    lib.h.advance(5 * 60 * 1000);
    lib.tick().await;
    assert_eq!(lib.worker.live().read_counts(), (2, works_read));
    lib.h.advance(5 * 60 * 1000);
    lib.tick().await;
    assert_eq!(lib.worker.live().read_counts(), (2, works_read));
    assert_eq!(
        lib.readings(),
        vec![Reading::Folder(folder.id.clone()); 2],
        "the first cycle and the safety net, nothing else"
    );
}

#[tokio::test]
async fn without_watching_every_cycle_reads_every_folder_as_before() {
    let lib = Live::new(config()).await;
    let root = lib.folder("anime");
    touch(&root.join("A/Season 01/A S01E01.mkv"));
    let folder = lib.register(&root).await;
    for _ in 0..3 {
        lib.h.advance(1000);
        lib.tick().await;
    }
    assert_eq!(lib.worker.live().read_counts(), (3, 0));
    assert!(lib.worker.live().status(&folder.id).is_none());
}

#[tokio::test]
async fn the_running_worker_watches_folders_registered_later_and_catches_up_on_them() {
    let lib = Live::new(config()).await;
    let first = lib.folder("first");
    touch(&first.join("A/Season 01/A S01E01.mkv"));
    let first_folder = lib.register(&first).await;

    let worker = lib
        .worker
        .clone()
        .with_command_poll(Duration::from_millis(40));
    let cancel = CancellationToken::new();
    let running = tokio::spawn({
        let (worker, cancel) = (worker.clone(), cancel.clone());
        async move { worker.run(cancel).await }
    });
    let live = worker.live();
    eventually("the first folder is watched", || async {
        live.status(&first_folder.id).is_some()
    })
    .await;

    // Registered while running (the web reads it once); a file arrives right after.
    let second = lib.folder("second");
    touch(&second.join("S/Season 01/S S01E01.mkv"));
    let second_folder = lib.register(&second).await;
    lib.h.advance(1000);
    let stamp = lib.h.now();
    touch(&second.join("S/Season 01/S S01E02.mkv"));
    eventually("the second folder is watched", || async {
        live.status(&second_folder.id).is_some()
    })
    .await;
    eventually("what came after the web's reading is recorded", || async {
        lib.added_at(&second_folder, "S", "Season 01/S S01E02.mkv")
            .await
            == Some(Some(stamp))
    })
    .await;

    // Unregistered: the watches go.
    lib.unregister(&second_folder).await;
    eventually("the watches of the removed folder go", || async {
        live.status(&second_folder.id).is_none()
    })
    .await;
    assert!(live.status(&first_folder.id).is_some());

    cancel.cancel();
    running.await.unwrap();
    assert!(
        live.status(&first_folder.id).is_none(),
        "stopped with the worker"
    );
}

// --- the real process ----------------------------------------------------------------

#[tokio::test]
async fn the_worker_process_records_a_new_episode_within_seconds_without_waiting_for_a_cycle() {
    use std::process::{Command, Stdio};

    let lib = Live::new(config()).await;
    let root = lib.folder("anime");
    touch(&root.join("A/Season 01/A S01E01.mkv"));
    let folder = lib.register(&root).await;

    let out = lib.h.dir.path().join("worker.out");
    let err = lib.h.dir.path().join("worker.err");
    let mut child = Command::new(env!("CARGO_BIN_EXE_trss-worker"))
        .current_dir(lib.h.dir.path())
        .env("TRSS_DB_PATH", lib.h.db_path())
        .env("TRANSMISSION_URL", lib.h.tr.url())
        // One cycle at the start and none after it for the whole test.
        .env("TRSS_WORKER_INTERVAL_SECS", "3600")
        .env_remove("CHANNELS_CONFIG_URL")
        .stdout(fs::File::create(&out).unwrap())
        .stderr(fs::File::create(&err).unwrap())
        .spawn()
        .expect("spawn trss-worker");
    let output = || {
        format!(
            "{}{}",
            fs::read_to_string(&out).unwrap_or_default(),
            fs::read_to_string(&err).unwrap_or_default()
        )
    };

    let outcome = async {
        eventually(
            "the folder is watched and read by the first cycle",
            || async {
                let log = output();
                log.contains("directories watched") && log.contains(": 1 works,")
            },
        )
        .await;
        touch(&root.join("A/Season 01/A S01E02.mkv"));
        // The default debounce is three seconds; the cycle is an hour away.
        eventually("the episode is recorded by the running process", || async {
            lib.has_file(&folder, "A", "Season 01/A S01E02.mkv").await
        })
        .await;
        // The reading logs after it commits, so the line can trail the row.
        eventually("the targeted reading logs it", || async {
            output().contains("works read (A)")
        })
        .await;
        let log = output();
        assert!(!log.contains("Cycle failed"), "{log}");
    }
    .await;

    let _ = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .stdout(Stdio::null())
        .status();
    let _ = child.wait();
    outcome
}
