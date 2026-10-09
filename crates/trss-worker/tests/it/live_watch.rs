//! What the worker does around the watching of the watch folders (ticket 0016):
//! a reading an alert asked for holds the worker's lock and beats the heartbeat,
//! an archive move and the alerts it raises do not interleave, the running
//! worker watches the folders registered while it runs, and a real `trss-worker`
//! process records a new episode between cycles. A real worker, real temporary
//! folders and the kernel's own alerts, the real web API for registering
//! folders, a fake Transmission for the archive move.
//!
//! What an alert makes the library read, the poll decisions and the safety net
//! are tested in `trss-library` (`live::tests`), with the same real inotify.
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

    // The work belongs to the archive folder now (that it keeps its ID is
    // `trss-collect`'s rule), and the alerts of the move that follow leave it as
    // it is.
    assert!(a.lib.work(&a.archive_folder, "Solo").await.is_some());
    eventually("both folders' watches follow the work", || async {
        live.status(&a.collect_folder.id).unwrap().watches == 3
            && live.status(&a.archive_folder.id).unwrap().watches == 5
    })
    .await;
    tokio::time::sleep(DEBOUNCE * 4).await;
    let after = a.lib.work(&a.archive_folder, "Solo").await.unwrap();
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

// --- the running worker ----------------------------------------------------------------

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
