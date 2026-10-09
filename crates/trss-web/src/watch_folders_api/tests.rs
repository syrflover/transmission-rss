//! The watch folder endpoints and the saving of the collect and archive
//! folders, which registers them: status codes, the row shape and the
//! sentences. What reading a folder finds, and how the records change, is tested
//! in `trss-library`; the worker's cycle and the `watch_rescan` command run in
//! `trss-worker` (`commands_api` tests the request).

use std::{collections::BTreeSet, fs, os::unix::fs::PermissionsExt, path::Path};

use axum::{http::Method, http::StatusCode, Router};
use serde_json::{json, Value};
use tempfile::TempDir;

use super::*;
use crate::testing;
use trss_core::Db;
use trss_library::discovery::{EpisodeFile, FileKind, Scan, ScannedWork, WorkRead};

struct App {
    state: AppState,
    router: Router,
    /// The folders a test makes, and the watch folders it registers.
    dir: TempDir,
}

impl App {
    fn new() -> App {
        let state = AppState::new(Db::open_blocking(":memory:").unwrap());
        let router = testing::api(&state);
        App {
            state,
            router,
            dir: tempfile::tempdir().unwrap(),
        }
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        testing::call(&self.router, method, uri, body).await
    }

    /// A folder named `name` under the test's directory.
    fn folder(&self, name: &str) -> std::path::PathBuf {
        let path = self.dir.path().join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }

    async fn add(&self, path: &str) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            "/api/library/watch-folders",
            Some(json!({ "path": path })),
        )
        .await
    }

    /// Adds a folder that must be accepted; its row.
    async fn register(&self, path: &Path) -> Value {
        let (status, body) = self.add(&text(path)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["folder"].clone()
    }

    async fn remove(&self, id: &str) -> (StatusCode, Value) {
        self.call(
            Method::DELETE,
            &format!("/api/library/watch-folders/{id}"),
            None,
        )
        .await
    }

    async fn list(&self) -> Vec<Value> {
        let (status, body) = self
            .call(Method::GET, "/api/library/watch-folders", None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["folders"].as_array().unwrap().clone()
    }

    async fn save_collection(
        &self,
        version: i64,
        collect: &Path,
        archive: Option<&Path>,
    ) -> (StatusCode, Value) {
        self.call(
            Method::PUT,
            "/api/settings/collection",
            Some(json!({
                "version": version,
                "folder": text(collect),
                "archive_folder": archive.map(text),
            })),
        )
        .await
    }

    async fn folder_at(&self, path: &Path) -> Option<WatchFolder> {
        self.state
            .library
            .folders()
            .await
            .unwrap()
            .into_iter()
            .find(|f| f.path == text(path))
    }
}

fn text(path: impl AsRef<Path>) -> String {
    path.as_ref().to_str().unwrap().to_owned()
}

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "x").unwrap();
}

/// Every path under `root`, for showing that nothing was touched.
fn tree(root: &Path) -> BTreeSet<String> {
    fn walk(dir: &Path, out: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            }
            out.insert(path.display().to_string());
        }
    }
    let mut out = BTreeSet::new();
    walk(root, &mut out);
    out
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

fn message(body: &Value) -> &str {
    body["message"].as_str().unwrap()
}

/// A work of one video, as a reading of a folder finds it.
fn work(name: &str) -> WorkRead {
    WorkRead::Read(ScannedWork {
        dir_name: name.to_owned(),
        seasons: BTreeSet::from([1]),
        files: vec![EpisodeFile {
            path: format!("Season 01/{name} S01E01.mkv"),
            kind: FileKind::Video,
            season: 1,
            episode: "01".to_owned(),
        }],
        unrecognized: vec![],
    })
}

// --- adding a folder ------------------------------------------------------------------

#[tokio::test]
async fn adding_a_folder_answers_its_row_and_how_many_works_it_found_and_the_list_shows_it() {
    let app = App::new();
    let root = app.folder("anime");
    lycoris(&root);

    let (status, body) = app.add(&format!("{}/", root.display())).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["works_found"], 1);
    let row = &body["folder"];
    // Stored as typed, without the trailing slash.
    assert_eq!(row["path"], text(&root));
    assert_eq!(row["automatic"], false);
    assert_eq!(row["works"], 1);
    assert_eq!(row["missing_works"], 0);
    assert_eq!(row["linked_works"], 0);
    assert_eq!(row["new_works"], 0);
    assert!(row["checked_at"].as_i64().unwrap() > 0);
    assert_eq!(row["error"], Value::Null);
    assert_eq!(row["watch_note"], Value::Null);

    assert_eq!(app.list().await, std::slice::from_ref(row));

    // The sentence the worker stores while it cannot watch every directory is
    // on the row.
    let id = row["id"].as_str().unwrap();
    app.state
        .library
        .set_watch_note(id, Some("폴더 1개를 감시하지 못해요.".into()))
        .await
        .unwrap();
    assert_eq!(
        app.list().await[0]["watch_note"],
        "폴더 1개를 감시하지 못해요."
    );
}

#[tokio::test]
async fn folders_that_overlap_a_registered_one_or_cannot_be_read_are_refused_with_a_reason() {
    let app = App::new();
    let root = app.folder("anime");
    touch(&root.join("Show/Season 01/Show S01E01.mkv"));
    app.register(&root).await;

    // The same folder, also through a link and with a trailing slash.
    let link = app.dir.path().join("anime-link");
    std::os::unix::fs::symlink(&root, &link).unwrap();
    for path in [text(&root), format!("{}/", root.display()), text(&link)] {
        let (status, body) = app.add(&path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
        assert!(message(&body).contains("이미 등록한"), "{body}");
    }

    // Inside a registered folder.
    let (status, body) = app.add(&text(root.join("Show"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message(&body).contains("안에 있는 폴더"), "{body}");

    // Around a registered folder.
    let (status, body) = app.add(&text(app.dir.path())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        message(&body).contains("안에 이미 등록한 감시 폴더"),
        "{body}"
    );

    // Missing, not a folder, not absolute, empty.
    let file = app.dir.path().join("file.txt");
    touch(&file);
    let (status, body) = app.add(&text(app.dir.path().join("nope"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message(&body).contains("찾지 못했어요"), "{body}");
    let (_, body) = app.add(&text(&file)).await;
    assert!(message(&body).contains("폴더가 아니에요"), "{body}");
    let (_, body) = app.add("relative/anime").await;
    assert!(message(&body).contains("전체 경로"), "{body}");
    let (_, body) = app.add("  ").await;
    assert!(message(&body).contains("입력해 주세요"), "{body}");

    // A folder the web cannot read.
    let locked = app.folder("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let (status, body) = app.add(&text(&locked)).await;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(message(&body).contains("권한"), "{body}");

    // Nothing but the first folder was registered.
    assert_eq!(app.list().await.len(), 1);
}

// --- the list -------------------------------------------------------------------------

#[tokio::test]
async fn the_row_counts_new_works_found_within_a_week_after_the_first_reading() {
    let app = App::new();
    let (now, day) = (now_millis(), DAY_MS);
    let library = &app.state.library;
    let (folder, _) = library
        .add_folder(
            "/anime".into(),
            Scan {
                works: vec![work("Old")],
            },
            now - 20 * day,
            &[],
        )
        .await
        .unwrap();
    let row = |rows: &[Value]| {
        (
            rows[0]["works"].as_u64().unwrap(),
            rows[0]["new_works"].as_u64().unwrap(),
        )
    };
    // The first reading dates nothing.
    assert_eq!(row(&app.list().await), (1, 0));

    // One found eight days ago is not new, and one found a minute ago is.
    for (found, works) in [
        (now - 8 * day, vec![work("Old"), work("Eight days")]),
        (
            now - 60_000,
            vec![work("Old"), work("Eight days"), work("New")],
        ),
    ] {
        library
            .record_scan(&folder.id, Ok(Scan { works }), found)
            .await
            .unwrap();
    }
    assert_eq!(row(&app.list().await), (3, 1));
}

// --- unregistering --------------------------------------------------------------------

#[tokio::test]
async fn unregistering_takes_the_works_out_and_the_same_path_brings_them_back_under_their_ids() {
    let app = App::new();
    let (a, b) = (app.folder("a"), app.folder("b"));
    lycoris(&a);
    touch(&b.join("Keep/Season 01/Keep S01E01.mkv"));
    let row_a = app.register(&a).await;
    app.register(&b).await;
    let folder_a = app.folder_at(&a).await.unwrap();
    let work = app
        .state
        .library
        .works(&folder_a.id)
        .await
        .unwrap()
        .remove(0);
    let on_disk = tree(&a);

    let (status, body) = app.remove(row_a["id"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["removed_works"], 1);

    // Out of the folder list, the work list and the work's screens.
    assert_eq!(app.list().await.len(), 1);
    let (_, works) = app.call(Method::GET, "/api/library/works", None).await;
    let names: Vec<&str> = works["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Keep"]);
    for uri in [
        format!("/api/library/works/{}", work.id),
        format!("/api/library/works/{}/artwork", work.id),
    ] {
        let (status, _) = app.call(Method::GET, &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
    assert_eq!(tree(&a), on_disk);

    // A second time there is nothing to unregister.
    let (status, body) = app.remove(row_a["id"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(message(&body).contains("등록이 해제됐을 수"), "{body}");

    // The same path again: the same folder and work.
    let again = app.register(&a).await;
    assert_eq!(again["id"], row_a["id"]);
    let works = app.state.library.works(&folder_a.id).await.unwrap();
    assert_eq!(works.len(), 1);
    assert_eq!(works[0].id, work.id);
    let (status, _) = app
        .call(
            Method::GET,
            &format!("/api/library/works/{}", work.id),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

// --- the collect and archive folders --------------------------------------------------

#[tokio::test]
async fn saving_the_collect_and_archive_folders_registers_and_reads_them_and_they_cannot_be_unregistered(
) {
    let app = App::new();
    let (collect, archive) = (app.folder("Shows (current)"), app.folder("Shows"));
    touch(&collect.join("A/Season 01/A S01E01.mkv"));
    touch(&archive.join("B/Season 01/B S01E01.mkv"));

    let (status, body) = app.save_collection(0, &collect, Some(&archive)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let listed = app.list().await;
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0]["path"], text(&collect));
    assert_eq!(listed[1]["path"], text(&archive));
    for folder in &listed {
        assert_eq!(folder["automatic"], true, "{folder}");
        assert_eq!(folder["works"], 1);
        assert!(folder["checked_at"].as_i64().unwrap() > 0);
        assert_eq!(folder["error"], Value::Null);
        // The first reading is the baseline: nothing is dated, nothing is new.
        assert_eq!(folder["new_works"], 0);
    }

    // Neither can be unregistered, and both stay.
    for path in [&collect, &archive] {
        let folder = app.folder_at(path).await.unwrap();
        let (status, body) = app.remove(&folder.id).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(message(&body).contains("수집 폴더나 보관 폴더"), "{body}");
    }
    assert_eq!(app.list().await.len(), 2);

    // A folder added by hand is not automatic, and can be unregistered.
    let by_hand = app.register(&app.folder("other")).await;
    assert_eq!(by_hand["automatic"], false);
    let (status, _) = app.remove(by_hand["id"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_collect_or_archive_folder_that_overlaps_a_registered_folder_is_not_saved() {
    let app = App::new();
    let downloads = app.folder("downloads");
    let current = app.folder("downloads/Shows (current)");
    let archive = app.folder("archive");
    app.register(&downloads).await;

    let (status, body) = app.save_collection(0, &current, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        message(&body).contains("수집 폴더") && message(&body).contains("안에 있어요"),
        "{body}"
    );
    // Nothing was saved or registered.
    let (_, settings) = app
        .call(Method::GET, "/api/settings/collection", None)
        .await;
    assert_eq!(settings["folder"], Value::Null);
    assert_eq!(settings["version"], 0);
    assert_eq!(app.list().await.len(), 1);

    // A folder around a registered one is refused as well.
    let inner = app.folder("inner/watched");
    let around = app.dir.path().join("inner");
    app.register(&inner).await;
    let (status, body) = app.save_collection(0, &archive, Some(&around)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        message(&body).contains("보관 폴더") && message(&body).contains("등록한 감시 폴더"),
        "{body}"
    );
    assert!(app.folder_at(&archive).await.is_none());
    assert_eq!(app.list().await.len(), 2);
}

#[tokio::test]
async fn a_collect_folder_that_cannot_be_read_is_still_saved_and_registered_with_no_reading() {
    let app = App::new();
    let collect = app.folder("locked");
    touch(&collect.join("A/Season 01/A S01E01.mkv"));
    fs::set_permissions(&collect, fs::Permissions::from_mode(0o000)).unwrap();
    let (status, body) = app.save_collection(0, &collect, None).await;
    fs::set_permissions(&collect, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");

    // Registered, with no first reading: the worker reads it when it can and
    // shows why on the row when it cannot.
    let folder = app.folder_at(&collect).await.unwrap();
    assert!(folder.automatic);
    assert!(!folder.baselined && folder.checked_at.is_none());
    assert!(app
        .state
        .library
        .works(&folder.id)
        .await
        .unwrap()
        .is_empty());
    let rows = app.list().await;
    assert_eq!(rows[0]["works"], 0);
    assert_eq!(rows[0]["checked_at"], Value::Null);
    assert_eq!(rows[0]["error"], Value::Null);
}
