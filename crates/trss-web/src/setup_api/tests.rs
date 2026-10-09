use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use trss_core::Db;
use trss_library::discovery::Scan;

use super::*;
use crate::testing;

struct App {
    state: AppState,
    dir: tempfile::TempDir,
}

impl App {
    /// A new install: nothing registered, nothing imported.
    async fn new() -> App {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        App {
            state: AppState::new(db),
            dir,
        }
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        testing::call(&testing::bare_api(&self.state), method, uri, body).await
    }

    async fn home(&self) -> Value {
        let (status, body) = self.call(Method::GET, "/schedule/week", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn skip(&self, step: &str, skipped: bool) -> (StatusCode, Value) {
        self.call(
            Method::PUT,
            &format!("/first-run/{step}"),
            Some(json!({ "skipped": skipped })),
        )
        .await
    }

    async fn add_folder(&self) {
        self.state
            .library
            .add_folder("/c".into(), Scan { works: vec![] }, 100, &[])
            .await
            .unwrap();
    }
}

fn steps(view: &Value) -> Vec<(String, bool, bool)> {
    view["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["step"].as_str().unwrap().to_owned(),
                s["done"].as_bool().unwrap(),
                s["skipped"].as_bool().unwrap(),
            )
        })
        .collect()
}

#[tokio::test]
async fn a_new_install_shows_only_the_checklist() {
    let app = App::new().await;
    let home = app.home().await;
    assert_eq!(
        home["week"],
        Value::Null,
        "no schedule, status or next quarter"
    );
    assert_eq!(home["first_run"]["active"], true);
    assert_eq!(
        steps(&home["first_run"]),
        [
            ("folder".to_owned(), false, false),
            ("import".to_owned(), false, false)
        ]
    );
}

#[tokio::test]
async fn a_folder_and_a_skipped_import_end_the_checklist_for_every_device() {
    let app = App::new().await;
    app.add_folder().await;

    let (status, view) = app.skip("import", true).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["active"], false);
    assert_eq!(
        steps(&view),
        [
            ("folder".to_owned(), true, false),
            ("import".to_owned(), false, true)
        ]
    );

    // Another device asks: the schedule, not the checklist. (The server holds
    // the skip, so nothing about the device is involved.)
    let home = app.home().await;
    assert_eq!(home["first_run"], Value::Null);
    assert!(home["week"]["days"].as_array().unwrap().len() == 7);

    // The skip is taken back with `skipped: false`, and the checklist returns.
    let (status, view) = app.skip("import", false).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["active"], true);
    let home = app.home().await;
    assert_eq!(home["first_run"]["active"], true);
    assert_eq!(home["week"], Value::Null);
}

#[tokio::test]
async fn an_unknown_step_or_body_is_refused() {
    let app = App::new().await;
    assert_eq!(app.skip("other", true).await.0, StatusCode::NOT_FOUND);
    let (status, _) = app
        .call(
            Method::PUT,
            "/first-run/import",
            Some(json!({ "skipped": "yes" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Nothing was skipped by those.
    assert_eq!(app.home().await["first_run"]["active"], true);
}

#[tokio::test]
async fn an_install_that_was_not_empty_has_no_first_run() {
    // The database migration decides which installs begin a first run
    // (trss-core `db`); here the row is simply absent.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let db = Db::open(&path).await.unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM first_run", [])
        .unwrap();
    let app = App {
        state: AppState::new(db),
        dir,
    };
    let home = app.home().await;
    assert_eq!(home["first_run"], Value::Null);
    assert!(home["week"].is_object());
    assert_eq!(app.skip("folder", true).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn importing_the_legacy_file_into_a_new_install_adds_every_channel_without_asking() {
    let app = App::new().await;
    let media = app.dir.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    let media = media.to_str().unwrap();
    let content = format!(
        "- url: https://feeds.example.test/a?token=secret-a\n  directory: {media}\n  rules:\n    - match: One\n      directory: One\n\
         - url: https://feeds.example.test/b?token=secret-b\n  directory: {media}\n  rules:\n    - match: Two\n      directory: Two\n"
    );

    let (status, result) = app
        .call(
            Method::POST,
            "/import/legacy/apply",
            Some(json!({
                "content": content,
                "choices": [],
                "reviewed_collect_folder": null,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");

    // The import was applied (and set the collect folder, a watch folder):
    // both steps are done and the schedule is up.
    let run = app.state.setup.first_run().await.unwrap().unwrap();
    assert!(run.done(Step::Import) && run.done(Step::Folder));
    let home = app.home().await;
    assert_eq!(home["first_run"], Value::Null);
    assert!(home["week"].is_object());
}
