//! What the worker does with the watch folders (ticket 0012): a cycle reads
//! every watch folder after the RSS work and an unreadable one stops neither,
//! and a `다시 확인` command the web accepted reaches the worker and runs once
//! per command ID. The real web API, the real worker, real temporary folders
//! and a fake Transmission.
//!
//! What is found in a folder, the records and the rows are tested where the
//! rule lives: `trss-library` (`discovery`, `watch`, `watch_rescan`, `store`),
//! `trss-web` (`watch_folders_api`, `commands_api`) and `trss-collect`
//! (`rule_archive`, which keeps a work's ID when it moves).
//!
//! The worker's clock is the harness's manual one, so what time a file gets is
//! exact.

use crate::common;

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use trss_collect::{
    commands::rule_archive::work_folder::MovePolicy, store::channels::ChannelInput,
};
use trss_core::settings::SettingsStore;
use trss_library::store::library::{LibraryStore, WatchFolder, WorkRecord};
use trss_worker::{CommandsOutcome, CycleReport, TickOutcome, Worker};

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "x").unwrap();
}

fn text(path: impl AsRef<Path>) -> String {
    path.as_ref().to_str().unwrap().to_owned()
}

struct Lib {
    h: Harness,
    /// The one worker of the test, as a process has one: what it remembers
    /// between scans (see `trss_library::watch`) lives as long as it does.
    worker: Worker,
    api: WebApi,
    library: LibraryStore,
    /// A folder under the harness's temporary directory that watch folders go in.
    media: PathBuf,
}

impl Lib {
    async fn new() -> Lib {
        let h = Harness::without_collect_folder().await;
        let media = h.dir.path().join("media");
        fs::create_dir_all(&media).unwrap();
        Lib {
            api: h.web_api(),
            library: LibraryStore::new(h.db.clone()),
            worker: h.worker().with_move_policy(MovePolicy {
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

    /// Adds a folder through the web API and returns it as stored.
    async fn register(&self, path: &Path) -> WatchFolder {
        let (status, text, body) = self
            .api
            .call(
                "POST",
                "/api/library/watch-folders",
                Some(json!({ "path": text(path) })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{text}");
        self.library
            .folder(body["folder"]["id"].as_str().unwrap())
            .await
            .unwrap()
            .unwrap()
    }

    async fn list(&self) -> Vec<Value> {
        let (status, text, body) = self
            .api
            .call("GET", "/api/library/watch-folders", None)
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        body["folders"].as_array().unwrap().clone()
    }

    async fn works(&self, folder: &WatchFolder) -> Vec<WorkRecord> {
        self.library.works(&folder.id).await.unwrap()
    }

    async fn work(&self, folder: &WatchFolder, name: &str) -> WorkRecord {
        self.works(folder)
            .await
            .into_iter()
            .find(|w| w.dir_name == name)
            .unwrap_or_else(|| panic!("no work {name}"))
    }

    /// One worker cycle (which rescans every watch folder after the RSS work).
    async fn tick(&self) -> CycleReport {
        match self.worker.tick(&CancellationToken::new()).await.unwrap() {
            TickOutcome::Ran(report) => report,
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    async fn send(&self, id: &str, kind: &str, payload: Value) -> (StatusCode, Value) {
        let (status, _, body) = self
            .api
            .call(
                "POST",
                "/api/commands",
                Some(json!({ "id": id, "kind": kind, "payload": payload })),
            )
            .await;
        (status, body)
    }

    async fn run_commands(&self) -> CommandsOutcome {
        self.worker
            .clone()
            .run_commands(&CancellationToken::new())
            .await
            .unwrap()
    }

    async fn command(&self, id: &str) -> Value {
        let (status, text, body) = self
            .api
            .call("GET", &format!("/api/commands/{id}"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        body
    }
}

fn lycoris(root: &Path) {
    for file in [
        "Lycoris Recoil/Season 01/Lycoris Recoil S01E01.mkv",
        "Lycoris Recoil/Season 01/Lycoris Recoil S01E01.smi",
        "Lycoris Recoil/Season 01/Lycoris Recoil S01E02.mkv",
        "Lycoris Recoil/Season 02/Lycoris Recoil S02E01.mkv",
    ] {
        touch(&root.join(file));
    }
}

// --- the cycle ------------------------------------------------------------------------

#[tokio::test]
async fn an_unreadable_watch_folder_shows_its_reason_and_stops_neither_the_others_nor_rss() {
    let lib = Lib::new().await;
    let (a, b) = (lib.folder("a"), lib.folder("b"));
    touch(&a.join("Show A/Season 01/Show A S01E01.mkv"));
    touch(&b.join("Show B/Season 01/Show B S01E01.mkv"));
    lib.register(&a).await;
    let fb = lib.register(&b).await;

    // An RSS channel that the cycle must still run.
    SettingsStore::new(lib.h.db.clone())
        .put_collection(0, text(lib.folder("collect")), None)
        .await
        .unwrap();
    lib.h
        .channels
        .create_channel_with_rules(ChannelInput::new(lib.h.feeds.url("feed-a")), feed_a_rules())
        .await
        .unwrap();

    fs::set_permissions(&a, fs::Permissions::from_mode(0o000)).unwrap();
    touch(&b.join("Show B/Season 01/Show B S01E02.mkv"));
    lib.h.advance(1000);
    let report = lib.tick().await;
    fs::set_permissions(&a, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(report.channels_read, 1);
    assert!(report.items_seen > 0, "{report:?}");

    let rows = lib.list().await;
    assert!(
        rows[0]["error"].as_str().unwrap().contains("권한"),
        "{rows:?}"
    );
    assert_eq!(rows[1]["error"], Value::Null);
    // The other folder was read.
    let b_work = lib.work(&fb, "Show B").await;
    assert!(b_work.files().contains_key("Season 01/Show B S01E02.mkv"));
}

// --- 다시 확인 ---------------------------------------------------------------------------

#[tokio::test]
async fn a_rescan_the_web_accepted_is_run_by_the_worker_once_per_command_id() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    lycoris(&root);
    let folder = lib.register(&root).await;

    touch(&root.join("Brand New/Season 01/Brand New S01E01.mkv"));
    lib.h.advance(5000);
    let payload = json!({ "folder_id": folder.id });
    let (status, body) = lib
        .send("rescan-0001", "watch_rescan", payload.clone())
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    // Accepted is not done: nothing was read yet.
    assert_eq!(lib.works(&folder).await.len(), 1);

    assert_eq!(lib.run_commands().await, CommandsOutcome::Ran(1));
    let command = lib.command("rescan-0001").await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "scanned");
    let row = &lib.list().await[0];
    assert_eq!(row["works"], 2);
    assert_eq!(row["checked_at"], lib.h.now());

    // The same ID sent again does not read the folder again.
    touch(&root.join("Another/Season 01/Another S01E01.mkv"));
    lib.h.advance(5000);
    let (status, body) = lib
        .send("rescan-0001", "watch_rescan", payload.clone())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "done");
    assert_eq!(lib.run_commands().await, CommandsOutcome::Idle);
    assert_eq!(lib.list().await[0]["works"], 2);

    // A new ID is a new action: the folder is read again.
    let (status, _) = lib.send("rescan-0005", "watch_rescan", payload).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(lib.run_commands().await, CommandsOutcome::Ran(1));
    assert_eq!(lib.list().await[0]["works"], 3);
}
