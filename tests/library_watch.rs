//! Watch folders and the discovery of works, end to end (ticket 0012): the real
//! web API (adding, listing and unregistering a folder, the `다시 확인`
//! command), the real worker (its cycle rescans every folder; commands run
//! under its lock; the archive move keeps works), real temporary folders, a
//! fake Transmission, and no mocks of the discovery itself.
//!
//! The worker's clock is the harness's manual one, so what time a file gets is
//! exact. File system times are set to values that must never be read.

mod common;

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use transmission_rss::{
    discovery,
    store::{
        channels::{ChannelInput, ChannelWithRules, RuleInput},
        library::{LibraryStore, WatchFolder, WorkRecord},
        settings::SettingsStore,
    },
    worker::{CommandsOutcome, CycleReport, MovePolicy, TickOutcome, Worker},
};

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "x").unwrap();
}

fn text(path: impl AsRef<Path>) -> String {
    path.as_ref().to_str().unwrap().to_owned()
}

/// Sets a file's modification time to the year 2001, which no record may show.
fn age(path: &Path) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000))
        .unwrap();
}

struct Lib {
    h: Harness,
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
            h,
            media,
        }
    }

    fn folder(&self, name: &str) -> PathBuf {
        let path = self.media.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }

    async fn add(&self, path: &str) -> (StatusCode, Value) {
        let (status, text, body) = self
            .api
            .call(
                "POST",
                "/api/library/watch-folders",
                Some(json!({ "path": path })),
            )
            .await;
        assert!(body != Value::Null, "{text}");
        (status, body)
    }

    /// Adds a folder that must be accepted and returns it as stored.
    async fn register(&self, path: &Path) -> WatchFolder {
        let (status, body) = self.add(&text(path)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
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

    fn worker(&self) -> Worker {
        self.h.worker().with_move_policy(MovePolicy {
            poll: Duration::from_millis(10),
            timeout: Duration::from_secs(5),
        })
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
        self.worker()
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

// --- registering a folder ------------------------------------------------------------

#[tokio::test]
async fn registering_a_folder_records_the_work_its_seasons_and_episodes_and_says_how_many_works() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    lycoris(&root);

    let (status, body) = lib.add(&format!("{}/", root.display())).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["works_found"], 1);
    assert_eq!(body["folder"]["path"], text(&root));
    assert_eq!(body["folder"]["works"], 1);
    assert_eq!(body["folder"]["linked_works"], 0);
    assert_eq!(body["folder"]["new_works"], 0);
    assert!(body["folder"]["checked_at"].as_i64().unwrap() > 0);
    assert_eq!(body["folder"]["error"], Value::Null);

    let folder = lib.library.folders().await.unwrap().remove(0);
    let work = lib.work(&folder, "Lycoris Recoil").await;
    assert_eq!(work.seasons, [1, 2]);
    let shape: Vec<(u32, &str, Vec<&str>)> = work
        .episodes
        .iter()
        .map(|e| {
            (
                e.season,
                e.episode.as_str(),
                e.files.iter().map(|f| f.path.as_str()).collect(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        [
            (
                1,
                "01",
                vec![
                    "Season 01/Lycoris Recoil S01E01.mkv",
                    "Season 01/Lycoris Recoil S01E01.smi"
                ]
            ),
            (1, "02", vec!["Season 01/Lycoris Recoil S01E02.mkv"]),
            (2, "01", vec!["Season 02/Lycoris Recoil S02E01.mkv"]),
        ]
    );
    assert!(work.unrecognized.is_empty());
    // Everything there at the first check is of unknown age.
    assert_eq!(work.first_seen_at, None);
    assert!(work.files().values().all(|f| f.added_at.is_none()));

    let listed = lib.list().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["works"], 1);
    // Reading a folder writes nothing into it.
    assert!(!root.join("Lycoris Recoil/.trss").exists());
}

#[tokio::test]
async fn folders_that_overlap_a_registered_one_or_cannot_be_read_are_refused_with_a_reason() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    touch(&root.join("Show/Season 01/Show S01E01.mkv"));
    lib.register(&root).await;

    let refused = |body: &Value| body["message"].as_str().unwrap().to_owned();

    // The same folder, also through a link and with a trailing slash.
    let link = lib.media.join("anime-link");
    std::os::unix::fs::symlink(&root, &link).unwrap();
    for path in [text(&root), format!("{}/", root.display()), text(&link)] {
        let (status, body) = lib.add(&path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
        assert!(refused(&body).contains("이미 등록한"), "{body}");
    }

    // Inside a registered folder.
    let (status, body) = lib.add(&text(root.join("Show"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(refused(&body).contains("안에 있는 폴더"), "{body}");

    // Around a registered folder.
    let (status, body) = lib.add(&text(&lib.media)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        refused(&body).contains("안에 이미 등록한 감시 폴더"),
        "{body}"
    );

    // Missing, not a folder, not absolute, empty.
    let file = lib.media.join("file.txt");
    touch(&file);
    let (status, body) = lib.add(&text(lib.media.join("nope"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(refused(&body).contains("찾지 못했어요"), "{body}");
    let (_, body) = lib.add(&text(&file)).await;
    assert!(refused(&body).contains("폴더가 아니에요"), "{body}");
    let (_, body) = lib.add("relative/anime").await;
    assert!(refused(&body).contains("전체 경로"), "{body}");
    let (_, body) = lib.add("  ").await;
    assert!(refused(&body).contains("입력해 주세요"), "{body}");

    // A folder the web cannot read.
    let locked = lib.folder("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let (status, body) = lib.add(&text(&locked)).await;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(refused(&body).contains("권한"), "{body}");

    // Nothing but the first folder was registered.
    assert_eq!(lib.library.folders().await.unwrap().len(), 1);
}

#[tokio::test]
async fn files_that_do_not_fit_are_counted_on_the_work_and_hidden_ones_are_not_discovered() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    for file in [
        "Show/Season 01/Show S01E01.mkv",
        // The season in the name is not the folder's.
        "Show/Season 01/Show S03E01.mkv",
        // Outside a season folder.
        "Show/Show S01E05.mkv",
        // In a folder inside a season folder.
        "Show/Season 01/[Batch] Show/Show 02.mkv",
        // Still downloading.
        "Show/Season 01/Show S01E04.mkv.part",
        // Hidden: never counted.
        "Show/.trss/subs/Show S01E01.ass",
        "Show/Season 01/.trss/cache.mkv",
        ".hidden/Season 01/H S01E01.mkv",
    ] {
        touch(&root.join(file));
    }
    lib.register(&root).await;
    let folder = lib.library.folders().await.unwrap().remove(0);
    assert_eq!(lib.works(&folder).await.len(), 1);

    let show = lib.work(&folder, "Show").await;
    assert_eq!(show.episodes.len(), 1);
    let mut reasons: Vec<(&str, &str)> = show
        .unrecognized
        .iter()
        .map(|u| (u.path.as_str(), u.reason.code()))
        .collect();
    reasons.sort();
    assert_eq!(
        reasons,
        [
            ("Season 01/Show S01E04.mkv.part", "partial"),
            ("Season 01/Show S03E01.mkv", "season_mismatch"),
            ("Season 01/[Batch] Show/Show 02.mkv", "in_subfolder"),
            ("Show S01E05.mkv", "outside_season"),
        ]
    );
}

// --- what a later check makes of a change --------------------------------------------

#[tokio::test]
async fn only_what_appears_after_the_first_check_gets_a_time_and_file_times_change_nothing() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    lycoris(&root);
    let folder = lib.register(&root).await;
    let first = lib.work(&folder, "Lycoris Recoil").await;

    // File times, in any direction, are not read.
    age(&root.join("Lycoris Recoil/Season 01/Lycoris Recoil S01E01.mkv"));
    lib.h.advance(60_000);
    let t1 = lib.h.now();
    touch(&root.join("Lycoris Recoil/Season 01/Lycoris Recoil S01E03.mkv"));
    touch(&root.join("Brand New/Season 01/Brand New S01E01.mkv"));
    age(&root.join("Brand New/Season 01/Brand New S01E01.mkv"));
    lib.tick().await;

    let work = lib.work(&folder, "Lycoris Recoil").await;
    assert_eq!(work.id, first.id);
    let files = work.files();
    assert_eq!(
        files["Season 01/Lycoris Recoil S01E03.mkv"].added_at,
        Some(t1)
    );
    for old in [
        "Season 01/Lycoris Recoil S01E01.mkv",
        "Season 01/Lycoris Recoil S01E01.smi",
        "Season 01/Lycoris Recoil S01E02.mkv",
        "Season 02/Lycoris Recoil S02E01.mkv",
    ] {
        assert_eq!(files[old].added_at, None, "{old}");
    }
    assert_eq!(work.first_seen_at, None);
    let fresh = lib.work(&folder, "Brand New").await;
    assert_eq!(fresh.first_seen_at, Some(t1));
    assert_eq!(fresh.files().values().next().unwrap().added_at, Some(t1));

    // A third check years later changes no recorded time.
    lib.h.advance(3_600_000);
    touch(&root.join("Lycoris Recoil/Season 01/Lycoris Recoil S01E04.mkv"));
    lib.tick().await;
    let again = lib.work(&folder, "Lycoris Recoil").await;
    let files = again.files();
    assert_eq!(
        files["Season 01/Lycoris Recoil S01E03.mkv"].added_at,
        Some(t1)
    );
    assert_eq!(
        files["Season 01/Lycoris Recoil S01E04.mkv"].added_at,
        Some(lib.h.now())
    );
    assert_eq!(lib.work(&folder, "Brand New").await.first_seen_at, Some(t1));
}

#[tokio::test]
async fn the_list_counts_new_works_found_within_a_week_after_the_first_check() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    lycoris(&root);
    // The worker's clock is the real time here, as the web's is.
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    lib.h.clock.store(now, std::sync::atomic::Ordering::SeqCst);
    lib.register(&root).await;
    assert_eq!(lib.list().await[0]["new_works"], 0);

    touch(&root.join("Brand New/Season 01/Brand New S01E01.mkv"));
    lib.tick().await;
    let row = &lib.list().await[0];
    assert_eq!(
        (row["works"].as_u64(), row["new_works"].as_u64()),
        (Some(2), Some(1))
    );
}

#[tokio::test]
async fn a_deleted_episode_file_drops_out_and_a_deleted_work_folder_keeps_its_id() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    lycoris(&root);
    touch(&root.join("Other/Season 01/Other S01E01.mkv"));
    let folder = lib.register(&root).await;
    let before = lib.work(&folder, "Lycoris Recoil").await;
    let other = lib.work(&folder, "Other").await;

    fs::remove_file(root.join("Lycoris Recoil/Season 01/Lycoris Recoil S01E02.mkv")).unwrap();
    fs::remove_dir_all(root.join("Other")).unwrap();
    lib.h.advance(1000);
    lib.tick().await;

    let after = lib.work(&folder, "Lycoris Recoil").await;
    assert_eq!(after.id, before.id);
    assert!(!after.missing);
    assert!(after.episodes.iter().all(|e| e.episode != "02"));
    assert_eq!(after.episodes.len(), before.episodes.len() - 1);

    let gone = lib.work(&folder, "Other").await;
    assert_eq!(gone.id, other.id);
    assert!(gone.missing);
    let row = &lib.list().await[0];
    assert_eq!(
        (row["works"].as_u64(), row["missing_works"].as_u64()),
        (Some(2), Some(1))
    );

    // It comes back: the same work, no longer missing.
    touch(&root.join("Other/Season 01/Other S01E01.mkv"));
    lib.h.advance(1000);
    lib.tick().await;
    let back = lib.work(&folder, "Other").await;
    assert_eq!(back.id, other.id);
    assert!(!back.missing);
}

// --- a folder that cannot be read ----------------------------------------------------

#[tokio::test]
async fn an_unreadable_watch_folder_shows_its_reason_and_stops_neither_the_others_nor_rss() {
    let lib = Lib::new().await;
    let (a, b) = (lib.folder("a"), lib.folder("b"));
    touch(&a.join("Show A/Season 01/Show A S01E01.mkv"));
    touch(&b.join("Show B/Season 01/Show B S01E01.mkv"));
    let fa = lib.register(&a).await;
    let fb = lib.register(&b).await;
    let before = lib.works(&fa).await;

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
    assert_eq!(rows[0]["works"], 1);
    assert_eq!(rows[1]["error"], Value::Null);
    // The earlier records of the folder that could not be read are as they were.
    assert_eq!(lib.works(&fa).await, before);
    // The other folder was read.
    let b_work = lib.work(&fb, "Show B").await;
    assert!(b_work.files().contains_key("Season 01/Show B S01E02.mkv"));

    // Readable again: the error is gone.
    lib.h.advance(1000);
    lib.tick().await;
    assert_eq!(lib.list().await[0]["error"], Value::Null);
}

// --- unregistering -------------------------------------------------------------------

#[tokio::test]
async fn unregistering_removes_the_folders_works_and_leaves_the_files_alone() {
    let lib = Lib::new().await;
    let (a, b) = (lib.folder("a"), lib.folder("b"));
    lycoris(&a);
    touch(&b.join("Keep/Season 01/Keep S01E01.mkv"));
    let fa = lib.register(&a).await;
    let fb = lib.register(&b).await;
    let on_disk = |root: &Path| {
        let mut out = Vec::new();
        fn walk(dir: &Path, out: &mut Vec<String>) {
            for e in fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, out);
                }
                out.push(p.display().to_string());
            }
        }
        walk(root, &mut out);
        out.sort();
        out
    };
    let files_before = on_disk(&a);

    let (status, _, body) = lib
        .api
        .call(
            "DELETE",
            &format!("/api/library/watch-folders/{}", fa.id),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["removed_works"], 1);

    assert_eq!(lib.list().await.len(), 1);
    assert!(lib.library.works(&fa.id).await.unwrap().is_empty());
    assert_eq!(lib.works(&fb).await.len(), 1);
    assert_eq!(on_disk(&a), files_before);

    let (status, _, body) = lib
        .api
        .call(
            "DELETE",
            &format!("/api/library/watch-folders/{}", fa.id),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // It can be registered again, and is read from scratch.
    let again = lib.register(&a).await;
    assert_ne!(again.id, fa.id);
    assert_eq!(lib.works(&again).await.len(), 1);
}

// --- 다시 확인 ---------------------------------------------------------------------------

#[tokio::test]
async fn rescan_is_accepted_as_a_command_the_worker_runs_once_per_command_id() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    lycoris(&root);
    let folder = lib.register(&root).await;
    let checked = lib
        .library
        .folder(&folder.id)
        .await
        .unwrap()
        .unwrap()
        .checked_at;

    touch(&root.join("Brand New/Season 01/Brand New S01E01.mkv"));
    lib.h.advance(5000);
    let payload = json!({ "folder_id": folder.id });
    let (status, body) = lib
        .send("rescan-0001", "watch_rescan", payload.clone())
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["state"], "pending");
    // Accepted is not done: nothing was read yet.
    assert_eq!(lib.works(&folder).await.len(), 1);
    // A second open rescan of the same folder is refused while this one is open.
    let (status, body) = lib
        .send("rescan-0002", "watch_rescan", payload.clone())
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    // The same command again is the same command.
    let (status, _) = lib
        .send("rescan-0001", "watch_rescan", payload.clone())
        .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(lib.run_commands().await, CommandsOutcome::Ran(1));
    let command = lib.command("rescan-0001").await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "scanned");
    let row = &lib.list().await[0];
    assert_eq!(row["works"], 2);
    assert_eq!(row["checked_at"], lib.h.now());
    assert_ne!(Some(lib.h.now()), checked);

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

    // The same ID with another folder is a mismatch.
    let (status, _) = lib
        .send(
            "rescan-0001",
            "watch_rescan",
            json!({ "folder_id": "other" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // A folder that is not registered is refused, and a bad payload too.
    let (status, _) = lib
        .send(
            "rescan-0003",
            "watch_rescan",
            json!({ "folder_id": "nope" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = lib.send("rescan-0004", "watch_rescan", json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A new ID is a new action: the folder is read again.
    let (status, _) = lib.send("rescan-0005", "watch_rescan", payload).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(lib.run_commands().await, CommandsOutcome::Ran(1));
    assert_eq!(lib.list().await[0]["works"], 3);
}

#[tokio::test]
async fn a_rescan_of_an_unreadable_folder_ends_failed_with_the_reason_and_keeps_the_records() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    lycoris(&root);
    let folder = lib.register(&root).await;
    let before = lib.works(&folder).await;

    fs::set_permissions(&root, fs::Permissions::from_mode(0o000)).unwrap();
    lib.send(
        "rescan-0001",
        "watch_rescan",
        json!({ "folder_id": folder.id }),
    )
    .await;
    lib.run_commands().await;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();

    let command = lib.command("rescan-0001").await;
    assert_eq!(command["state"], "failed", "{command}");
    assert!(command["outcome"]["reason"]
        .as_str()
        .unwrap()
        .contains("권한"));
    assert_eq!(lib.works(&folder).await, before);
    assert!(lib.list().await[0]["error"]
        .as_str()
        .unwrap()
        .contains("권한"));
}

// --- the archive move keeps the work ---------------------------------------------------

struct Archive {
    lib: Lib,
    collect: PathBuf,
    archive: PathBuf,
    collect_folder: WatchFolder,
    archive_folder: WatchFolder,
    channel: ChannelWithRules,
}

impl Archive {
    async fn new() -> Archive {
        let lib = Lib::new().await;
        let collect = lib.folder("Shows (current)");
        let archive = lib.folder("Shows");
        SettingsStore::new(lib.h.db.clone())
            .put_collection(0, text(&collect), Some(text(&archive)))
            .await
            .unwrap();
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
        touch(&collect.join("Clevatess/Season 02/Clevatess S02E02.mkv"));
        touch(&collect.join("Solo/Season 01/Solo S01E01.mkv"));
        // The archive already has the earlier season of Clevatess.
        touch(&archive.join("Clevatess/Season 01/Clevatess S01E01.mkv"));
        let collect_folder = lib.register(&collect).await;
        let archive_folder = lib.register(&archive).await;
        Archive {
            lib,
            collect,
            archive,
            collect_folder,
            archive_folder,
            channel,
        }
    }

    async fn archive_rule(&self, id: &str, rule: usize) -> Value {
        let (status, body) = self
            .lib
            .send(
                id,
                "rule_archive",
                json!({ "rule_id": self.channel.rules[rule].id, "direction": "archive" }),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        assert_eq!(self.lib.run_commands().await, CommandsOutcome::Ran(1));
        let command = self.lib.command(id).await;
        assert_eq!(command["state"], "done", "{command}");
        command
    }
}

#[tokio::test]
async fn archiving_a_work_keeps_its_id_under_the_archive_folder() {
    let a = Archive::new().await;
    let solo = a.lib.work(&a.collect_folder, "Solo").await;
    assert_eq!(solo.episodes.len(), 1);

    a.archive_rule("archive-solo-1", 1).await;
    assert!(!a.collect.join("Solo").exists());
    assert!(a.archive.join("Solo/Season 01/Solo S01E01.mkv").exists());

    // Same ID, now under the archive folder, with nothing left behind.
    assert!(a
        .lib
        .works(&a.collect_folder)
        .await
        .iter()
        .all(|w| w.dir_name != "Solo"));
    let moved = a.lib.work(&a.archive_folder, "Solo").await;
    assert_eq!(moved.id, solo.id);
    assert_eq!(moved.episodes, solo.episodes);
    assert_eq!(moved.first_seen_at, solo.first_seen_at);

    // The next cycle reads the folders as they are, and the work is the same one.
    a.lib.h.advance(1000);
    a.lib.tick().await;
    let read = a.lib.work(&a.archive_folder, "Solo").await;
    assert_eq!(read.id, solo.id);
    assert!(!read.missing);
    assert!(a
        .lib
        .works(&a.collect_folder)
        .await
        .iter()
        .all(|w| w.dir_name != "Solo"));
}

#[tokio::test]
async fn archiving_into_an_existing_work_folder_merges_into_the_destinations_id() {
    let a = Archive::new().await;
    let moved = a.lib.work(&a.collect_folder, "Clevatess").await;
    let kept = a.lib.work(&a.archive_folder, "Clevatess").await;
    assert_ne!(moved.id, kept.id);

    a.archive_rule("archive-clev-1", 0).await;

    assert!(a
        .lib
        .works(&a.collect_folder)
        .await
        .iter()
        .all(|w| w.dir_name != "Clevatess"));
    let merged = a.lib.work(&a.archive_folder, "Clevatess").await;
    assert_eq!(merged.id, kept.id);
    assert_eq!(merged.seasons, [1, 2]);
    assert_eq!(merged.episodes.len(), 3);

    // One row, not two, and the same after the next reading.
    a.lib.h.advance(1000);
    a.lib.tick().await;
    let read = a.lib.work(&a.archive_folder, "Clevatess").await;
    assert_eq!(read.id, kept.id);
    assert_eq!(read.episodes.len(), 3);
    let rows: Vec<_> = a
        .lib
        .works(&a.archive_folder)
        .await
        .into_iter()
        .filter(|w| w.dir_name == "Clevatess")
        .collect();
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn a_folder_moved_by_hand_is_a_new_work_and_the_old_one_is_missing() {
    let a = Archive::new().await;
    let solo = a.lib.work(&a.collect_folder, "Solo").await;

    fs::rename(a.collect.join("Solo"), a.archive.join("Solo")).unwrap();
    a.lib.h.advance(1000);
    a.lib.tick().await;

    let old = a.lib.work(&a.collect_folder, "Solo").await;
    assert_eq!(old.id, solo.id);
    assert!(old.missing);
    let new = a.lib.work(&a.archive_folder, "Solo").await;
    assert_ne!(new.id, solo.id);
    assert_eq!(new.first_seen_at, Some(a.lib.h.now()));
}

// --- a large library -----------------------------------------------------------------

// --- the library list ----------------------------------------------------------------

impl Lib {
    async fn library_list(&self) -> Vec<Value> {
        let (status, text, body) = self.api.call("GET", "/api/library/works", None).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        body["works"].as_array().unwrap().clone()
    }
}

#[tokio::test]
async fn the_library_list_answers_ranges_flags_and_times_of_every_work() {
    let lib = Lib::new().await;
    let root = lib.folder("anime");
    // Season 2 is the latest: videos 1-5, subtitles 1-2 and 4-5.
    for e in 1..=5 {
        touch(&root.join(format!("Alpha/Season 02/Alpha S02E{e:02}.mkv")));
        if e != 3 {
            touch(&root.join(format!("Alpha/Season 02/Alpha S02E{e:02}.ko.ass")));
        }
    }
    touch(&root.join("Alpha/Season 01/Alpha S01E01.mkv"));
    // A subtitle whose episode cannot be read.
    touch(&root.join("Beta/Season 01/Beta S01E01.mkv"));
    touch(&root.join("Beta/Season 01/Beta-extras.srt"));
    touch(&root.join("Gamma/Season 01/Gamma S01E13.mkv"));
    touch(&root.join("Gamma/Season 01/Gamma S01E13.ass"));
    let folder = lib.register(&root).await;

    // After the first check a new subtitle and a new work get the worker's time.
    touch(&root.join("Alpha/Season 02/Alpha S02E03.ko.ass"));
    touch(&root.join("Delta/Season 01/Delta S01E01.mkv"));
    lib.h.advance(5_000);
    lib.tick().await;
    let later = lib.h.now();

    // A work folder that goes away stays in the list.
    fs::remove_dir_all(root.join("Gamma")).unwrap();
    lib.h.advance(5_000);
    lib.tick().await;

    let list = lib.library_list().await;
    let names: Vec<&str> = list.iter().map(|w| w["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Alpha", "Beta", "Delta", "Gamma"]);
    let by = |name: &str| list.iter().find(|w| w["name"] == name).unwrap();

    let alpha = by("Alpha");
    assert_eq!(alpha["latest_season"], 2);
    assert_eq!(alpha["video"], json!([{ "first": "01", "last": "05" }]));
    // Episode 3's subtitle arrived after the first check.
    assert_eq!(alpha["subtitle"], json!([{ "first": "01", "last": "05" }]));
    assert_eq!(alpha["subtitle_coverage"], "all");
    assert_eq!(alpha["subtitle_check_needed"], false);
    assert_eq!(alpha["added_at"], Value::Null);
    assert_eq!(alpha["video_added_at"], Value::Null);
    assert_eq!(alpha["subtitle_added_at"], later);
    assert_eq!(alpha["watch_folder"]["id"], folder.id);
    assert_eq!(alpha["watch_folder"]["path"], text(&root));
    assert_eq!(alpha["missing"], false);

    let beta = by("Beta");
    assert_eq!(beta["subtitle"], json!([]));
    assert_eq!(beta["subtitle_coverage"], "none");
    assert_eq!(beta["subtitle_check_needed"], true);

    let delta = by("Delta");
    assert_eq!(delta["added_at"], later);
    assert_eq!(delta["video_added_at"], later);
    assert_eq!(delta["subtitle_added_at"], Value::Null);

    let gamma = by("Gamma");
    assert_eq!(gamma["missing"], true);
    assert_eq!(gamma["video"], json!([]));
    assert_eq!(gamma["subtitle_coverage"], Value::Null);
}

#[tokio::test]
async fn an_empty_library_lists_no_works() {
    let lib = Lib::new().await;
    assert!(lib.library_list().await.is_empty());
}

/// About 520 works and 10 000 files: one full reading and recording, an unchanged
/// one, and one with a single new file. The times are printed (run with
/// `--nocapture`) and recorded in the ticket, and not asserted: what is checked
/// is that the counts are right.
#[tokio::test]
async fn a_library_of_520_works_and_10_000_files_is_read_and_recorded() {
    const WORKS: usize = 520;
    let lib = Lib::new().await;
    let root = lib.folder("big");
    let mut files = 0;
    let build = std::time::Instant::now();
    for w in 0..WORKS {
        let name = format!("Work {w:03}");
        for e in 1..=12 {
            touch(&root.join(format!("{name}/Season 01/{name} S01E{e:02}.mkv")));
            files += 1;
        }
        for e in 1..=6 {
            touch(&root.join(format!("{name}/Season 01/{name} S01E{e:02}.ko.ass")));
            files += 1;
        }
        touch(&root.join(format!("{name}/Season 01/folder.jpg")));
        touch(&root.join(format!("{name}/Season 01/{name} S01E13.mkv.part")));
        files += 2;
    }
    eprintln!(
        "built {WORKS} works, {files} files in {:?}",
        build.elapsed()
    );

    let read = std::time::Instant::now();
    let scan = discovery::scan(&root).unwrap();
    let read_time = read.elapsed();
    assert_eq!(scan.works.len(), WORKS);

    let first = std::time::Instant::now();
    let folder = lib.register(&root).await;
    let first_time = first.elapsed();
    assert_eq!(lib.works(&folder).await.len(), WORKS);

    touch(&root.join("Work 007/Season 01/Work 007 S01E14.mkv"));
    lib.h.advance(1000);
    let second = std::time::Instant::now();
    lib.tick().await;
    let second_time = second.elapsed();
    let work = lib.work(&folder, "Work 007").await;
    assert_eq!(work.episodes.len(), 13);
    assert_eq!(
        work.files()["Season 01/Work 007 S01E14.mkv"].added_at,
        Some(lib.h.now())
    );

    let list = std::time::Instant::now();
    let works = lib.library_list().await;
    let list_time = list.elapsed();
    assert_eq!(works.len(), WORKS);
    let seven = works.iter().find(|w| w["name"] == "Work 007").unwrap();
    assert_eq!(
        seven["video"],
        json!([{ "first": "01", "last": "12" }, { "first": "14", "last": "14" }])
    );
    assert_eq!(seven["subtitle"], json!([{ "first": "01", "last": "06" }]));
    assert_eq!(seven["subtitle_coverage"], "some");

    eprintln!(
        "library list (GET /api/library/works, {} works): {list_time:?}",
        works.len()
    );
    eprintln!(
        "scan only: {read_time:?}; first registration (scan, web checks and record): {first_time:?}; \
         worker cycle with one new file (scan and record): {second_time:?}"
    );
}
