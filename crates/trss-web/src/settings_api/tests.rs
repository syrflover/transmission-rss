use std::path::Path;

use axum::{
    http::{Method, StatusCode},
    Router,
};
use serde_json::{json, Value};
use tempfile::TempDir;

use super::*;
use crate::testing;
use trss_core::Db;

struct App {
    router: Router,
}

impl App {
    fn new() -> App {
        let state = AppState::new(Db::open_blocking(":memory:").unwrap());
        App {
            router: testing::api(&state),
        }
    }

    async fn call(&self, method: Method, body: Option<Value>) -> (StatusCode, Value) {
        testing::call(&self.router, method, "/api/settings/collection", body).await
    }

    async fn get(&self) -> Value {
        let (status, json) = self.call(Method::GET, None).await;
        assert_eq!(status, StatusCode::OK);
        json
    }

    async fn put(&self, version: i64, folder: &str, archive: Option<&str>) -> (StatusCode, Value) {
        self.call(
            Method::PUT,
            Some(json!({ "version": version, "folder": folder, "archive_folder": archive })),
        )
        .await
    }
}

/// A media area with `current/` and `archive/` on one filesystem.
struct Media {
    dir: TempDir,
}

impl Media {
    fn new() -> Media {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("current")).unwrap();
        std::fs::create_dir(dir.path().join("archive")).unwrap();
        Media { dir }
    }

    fn path(&self, name: &str) -> String {
        self.dir.path().join(name).to_string_lossy().into_owned()
    }
}

fn message(json: &Value) -> &str {
    json["message"].as_str().unwrap()
}

#[tokio::test]
async fn nothing_is_set_on_a_fresh_database() {
    let app = App::new();
    assert_eq!(
        app.get().await,
        json!({ "folder": null, "archive_folder": null, "version": 0 })
    );
}

#[tokio::test]
async fn both_folders_are_saved_and_read_back() {
    let app = App::new();
    let media = Media::new();

    let (status, saved) = app
        .put(0, &media.path("current"), Some(&media.path("archive")))
        .await;

    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(
        saved,
        json!({
            "folder": media.path("current"),
            "archive_folder": media.path("archive"),
            "version": 1,
        })
    );
    assert_eq!(app.get().await, saved);
}

#[tokio::test]
async fn the_archive_folder_may_be_left_empty() {
    let app = App::new();
    let media = Media::new();

    for (version, archive) in [(0, None), (1, Some("")), (2, Some("   "))] {
        let (status, saved) = app.put(version, &media.path("current"), archive).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["archive_folder"], Value::Null);
        assert_eq!(saved["version"], version + 1);
    }
    assert_eq!(app.get().await["folder"], media.path("current"));
}

#[tokio::test]
async fn folders_are_stored_as_typed_without_trailing_slashes() {
    let app = App::new();
    let media = Media::new();

    let (status, saved) = app
        .put(
            0,
            &format!("  {}/ ", media.path("current")),
            Some(&format!("{}//", media.path("archive"))),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["folder"], media.path("current"));
    assert_eq!(saved["archive_folder"], media.path("archive"));
}

#[tokio::test]
async fn a_folder_that_is_missing_or_not_a_directory_is_refused_with_a_reason() {
    let app = App::new();
    let media = Media::new();
    std::fs::write(media.dir.path().join("file.txt"), "x").unwrap();

    let cases = [
        (media.path("nope"), None, "수집 폴더", "찾지 못했어요"),
        (
            media.path("current"),
            Some(media.path("nope")),
            "보관 폴더",
            "찾지 못했어요",
        ),
        (media.path("file.txt"), None, "수집 폴더", "폴더가 아니에요"),
        (
            media.path("current"),
            Some(media.path("file.txt")),
            "보관 폴더",
            "폴더가 아니에요",
        ),
        ("downloads/Shows".to_owned(), None, "수집 폴더", "전체 경로"),
        (
            media.path("current"),
            Some("archive".to_owned()),
            "보관 폴더",
            "전체 경로",
        ),
    ];
    for (folder, archive, field, reason) in cases {
        let (status, json) = app.put(0, &folder, archive.as_deref()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{folder} {archive:?}");
        assert_eq!(json["error"], "invalid");
        assert!(message(&json).contains(field), "{}", message(&json));
        assert!(message(&json).contains(reason), "{}", message(&json));
    }

    let (status, json) = app.put(0, "", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message(&json).contains("입력해 주세요"));
    assert_eq!(app.get().await["version"], 0, "a refusal saves nothing");
}

#[tokio::test]
async fn the_archive_folder_inside_the_collect_folder_is_refused_and_so_is_a_missing_one() {
    let app = App::new();
    let media = Media::new();
    std::fs::create_dir(media.dir.path().join("current/Shows")).unwrap();

    // An existing folder inside the collect folder.
    let (status, json) = app
        .put(
            0,
            &media.path("current"),
            Some(&media.path("current/Shows")),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        message(&json).contains("보관 폴더가 수집 폴더 안에"),
        "{}",
        message(&json)
    );

    // One that does not exist is refused too, for that.
    let (status, json) = app
        .put(
            0,
            &media.path("current"),
            Some(&media.path("current/Missing")),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message(&json).contains("찾지 못했어요"));

    assert_eq!(app.get().await["version"], 0);
}

#[tokio::test]
async fn the_same_folder_or_one_containing_the_other_is_refused_however_it_is_spelled() {
    let app = App::new();
    let media = Media::new();
    std::fs::create_dir(media.dir.path().join("current/Shows")).unwrap();

    let same = app
        .put(
            0,
            &media.path("current"),
            Some(&format!("{}/", media.path("current"))),
        )
        .await;
    assert_eq!(same.0, StatusCode::BAD_REQUEST);
    assert!(
        message(&same.1).contains("같은 폴더"),
        "{}",
        message(&same.1)
    );

    // A path that only reaches the same folder by going up and down again.
    let roundabout = app
        .put(
            0,
            &media.path("current"),
            Some(&media.path("current/Shows/../../current")),
        )
        .await;
    assert_eq!(roundabout.0, StatusCode::BAD_REQUEST);
    assert!(message(&roundabout.1).contains("같은 폴더"));

    // The collect folder inside the archive folder.
    let contained = app
        .put(
            0,
            &media.path("current/Shows"),
            Some(&media.path("current")),
        )
        .await;
    assert_eq!(contained.0, StatusCode::BAD_REQUEST);
    assert!(
        message(&contained.1).contains("수집 폴더가 보관 폴더 안에"),
        "{}",
        message(&contained.1)
    );
    assert_eq!(app.get().await["version"], 0);
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_cannot_hide_a_folder_inside_the_other() {
    let app = App::new();
    let media = Media::new();
    std::fs::create_dir(media.dir.path().join("current/Shows")).unwrap();
    std::os::unix::fs::symlink(
        media.dir.path().join("current/Shows"),
        media.dir.path().join("archive-link"),
    )
    .unwrap();

    let (status, json) = app
        .put(0, &media.path("current"), Some(&media.path("archive-link")))
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        message(&json).contains("보관 폴더가 수집 폴더 안에"),
        "{}",
        message(&json)
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn folders_on_different_filesystems_are_refused() {
    let app = App::new();
    let media = Media::new();
    // `/proc` is a directory on a filesystem of its own.
    assert!(Path::new("/proc").is_dir());

    let (status, json) = app.put(0, &media.path("current"), Some("/proc")).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        message(&json).contains("다른 파일시스템"),
        "{}",
        message(&json)
    );
    assert_eq!(app.get().await["version"], 0);

    // Without an archive folder there is nothing to compare.
    let (status, _) = app.put(0, "/proc", None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_save_from_a_stale_version_conflicts_and_shows_the_current_value() {
    let app = App::new();
    let media = Media::new();
    app.put(0, &media.path("current"), None).await;
    app.put(1, &media.path("current"), Some(&media.path("archive")))
        .await;

    let (status, json) = app.put(1, &media.path("archive"), None).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["error"], "conflict");
    assert_eq!(json["current"]["version"], 2);
    assert_eq!(json["current"]["folder"], media.path("current"));
    assert_eq!(json["current"]["archive_folder"], media.path("archive"));
    assert_eq!(app.get().await["folder"], media.path("current"));

    // A creation from the screen that saw nothing conflicts once a folder exists.
    let (status, _) = app.put(0, &media.path("archive"), None).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_body_that_is_not_the_expected_shape_is_a_plain_refusal() {
    let app = App::new();
    let media = Media::new();
    for body in [
        json!({ "folder": media.path("current") }),
        json!({ "version": 0 }),
        json!({ "version": 0, "folder": media.path("current"), "nope": 1 }),
        json!({ "version": "0", "folder": media.path("current") }),
    ] {
        let (status, json) = app.call(Method::PUT, Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "invalid");
    }
    assert_eq!(app.get().await["version"], 0);
}
