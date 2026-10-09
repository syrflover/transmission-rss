use std::collections::BTreeSet;

use axum::{
    http::{Method, StatusCode},
    Router,
};
use serde_json::{json, Value};

use crate::{testing, AppState};
use trss_core::Db;
use trss_library::discovery::{Reason, Scan, ScannedWork, SeenFile, Unrecognized, WorkRead};

const ASKED: &str = "Season 01/[Group] Show - 03 (1080p).mkv";
const MTIME_NS: i64 = 1_700_000_000_123_456_789;

fn identity(size: u64) -> SeenFile {
    SeenFile {
        size,
        mtime_ns: MTIME_NS,
    }
}

/// Work `Show`: a video in season 1 the name gives no episode of, which is
/// `check`, and files the app does not ask about.
fn show(check: SeenFile) -> Scan {
    let unrecognized = |path: &str, check| Unrecognized {
        path: path.into(),
        reason: Reason::NoEpisode,
        check,
    };
    Scan {
        works: vec![WorkRead::Read(ScannedWork {
            dir_name: "Show".into(),
            seasons: BTreeSet::from([0, 1]),
            files: Vec::new(),
            unrecognized: vec![
                unrecognized(ASKED, Some(check)),
                unrecognized("Season 00/extra.mkv", None),
                unrecognized("Season 01/extra.ass", None),
            ],
        })],
    }
}

struct App {
    state: AppState,
    router: Router,
    folder: String,
}

impl App {
    async fn new() -> App {
        let state = AppState::new(Db::open_blocking(":memory:").unwrap());
        let (folder, _) = state
            .library
            .add_folder("/media".into(), show(identity(5)), 100, &[])
            .await
            .unwrap();
        let router = testing::api(&state);
        App {
            state,
            router,
            folder: folder.id,
        }
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        testing::call(&self.router, method, uri, body).await
    }

    async fn get(&self, uri: &str) -> Value {
        let (status, body) = self.call(Method::GET, uri, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        body
    }

    async fn check(&self, work: &str, path: &str, seen: &str) -> StatusCode {
        let uri = format!("/api/library/works/{work}/videos/check");
        let body = json!({ "path": path, "seen": seen });
        self.call(Method::POST, &uri, Some(body)).await.0
    }

    async fn badges(&self) -> Value {
        self.get("/api/library/works?sort=title").await["items"][0]["todos"].clone()
    }
}

#[tokio::test]
async fn a_video_whose_name_gives_no_episode_is_a_to_do_until_a_person_checks_it() {
    let app = App::new().await;
    let todos = app.get("/api/todo").await;
    let work = todos["needs"][0]["work"]["id"].as_str().unwrap().to_owned();
    let seen = format!("5:{MTIME_NS}");
    // The card's composition is `trss-jobs`'; the screen reads these of it.
    let card = &todos["needs"][0];
    assert_eq!(
        (&card["kind"], &card["path"], &card["seen"], &card["badge"]),
        (
            &json!("video_check"),
            &json!(ASKED),
            &json!(seen),
            &json!("episode_check")
        )
    );
    assert_eq!(todos["count"], 1);
    assert_eq!(todos["badges"], json!({ work.clone(): ["episode_check"] }));
    assert_eq!(app.badges().await, json!(["episode_check"]));

    // Only the video as the screen saw it is checked.
    assert_eq!(app.check(&work, ASKED, "5").await, StatusCode::BAD_REQUEST);
    assert_eq!(
        app.check(&work, ASKED, &format!("6:{MTIME_NS}")).await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.check(&work, "Season 01/extra.ass", &seen).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(app.check("nope", ASKED, &seen).await, StatusCode::NOT_FOUND);
    assert_eq!(app.get("/api/todo").await["count"], 1);

    assert_eq!(app.check(&work, ASKED, &seen).await, StatusCode::NO_CONTENT);
    assert_eq!(
        app.get("/api/todo").await,
        json!({ "needs": [], "count": 0, "badges": {} })
    );
    assert_eq!(app.get("/api/todo/count").await, json!({ "count": 0 }));
    assert_eq!(app.badges().await, json!([]));
    let detail = app.get(&format!("/api/library/works/{work}")).await;
    let checked: Vec<(&str, bool)> = detail["unrecognized"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| (u["path"].as_str().unwrap(), u["checked"].as_bool().unwrap()))
        .collect();
    assert_eq!(
        checked,
        [
            ("Season 00/extra.mkv", false),
            (ASKED, true),
            ("Season 01/extra.ass", false)
        ]
    );

    // Another video at the path is asked about again.
    app.state
        .library
        .record_scan(&app.folder, Ok(show(identity(7))), 200)
        .await
        .unwrap();
    let todos = app.get("/api/todo").await;
    assert_eq!(todos["count"], 1);
    assert_eq!(todos["needs"][0]["seen"], format!("7:{MTIME_NS}"));
}
