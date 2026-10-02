use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use trss_core::Db;

use super::*;

struct App {
    db: Db,
    router: Router,
}

impl App {
    fn new() -> App {
        let db = Db::open_blocking(":memory:").unwrap();
        let state = AppState::new(db.clone());
        App {
            db,
            router: Router::new().nest("/api", crate::api::router().with_state(state)),
        }
    }

    async fn call(&self, method: Method, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri("/api/settings/policy");
        let body = match body {
            Some(json) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn get(&self) -> Value {
        let (status, json) = self.call(Method::GET, None).await;
        assert_eq!(status, StatusCode::OK);
        json
    }

    async fn put(&self, body: Value) -> (StatusCode, Value) {
        self.call(Method::PUT, Some(body)).await
    }
}

fn body(version: i64, order: &[&str], idle: i64, jobs: i64) -> Value {
    json!({
        "version": version,
        "format_order": order,
        "idle_timeout_seconds": idle,
        "max_concurrent_jobs": jobs,
    })
}

#[tokio::test]
async fn the_policy_starts_at_the_defaults_and_a_save_answers_it_as_saved() {
    let app = App::new();
    let first = app.get().await;
    assert_eq!(first["format_order"], json!(["ass", "srt", "smi"]));
    assert_eq!(
        (
            &first["idle_timeout_seconds"],
            &first["max_concurrent_jobs"]
        ),
        (&json!(300), &json!(1))
    );
    assert_eq!(
        (&first["version"], &first["saved_at"]),
        (&json!(0), &Value::Null)
    );
    assert_eq!(
        first["limits"],
        json!({ "idle_timeout_seconds": { "min": 60, "max": 3600 },
                "max_concurrent_jobs": { "min": 1, "max": 3 } })
    );
    assert_eq!(first["overrides"], json!([]));

    let (status, saved) = app.put(body(0, &["smi", "srt", "ass"], 900, 2)).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["format_order"], json!(["smi", "srt", "ass"]));
    assert_eq!(saved["version"], 1);
    assert!(saved["saved_at"].is_i64());
    assert_eq!(app.get().await, saved);
}

#[tokio::test]
async fn an_order_with_a_format_twice_or_a_number_out_of_range_is_refused_and_nothing_changes() {
    let app = App::new();
    for (bad, says) in [
        (
            body(0, &["smi", "smi", "ass"], 300, 1),
            "ASS·SRT·SMI가 한 번씩",
        ),
        (body(0, &["ass", "srt"], 300, 1), "ASS·SRT·SMI가 한 번씩"),
        (
            body(0, &["ass", "srt", "vtt"], 300, 1),
            "ASS·SRT·SMI가 한 번씩",
        ),
        (body(0, &["ass", "srt", "smi"], 59, 1), "1분부터 60분까지"),
        (body(0, &["ass", "srt", "smi"], 3601, 1), "1분부터 60분까지"),
        (body(0, &["ass", "srt", "smi"], 300, 0), "1개부터 3개까지"),
        (body(0, &["ass", "srt", "smi"], 300, 4), "1개부터 3개까지"),
        (body(0, &["ass", "srt", "smi"], -300, 1), "1분부터 60분까지"),
        (body(0, &["ass", "srt", "smi"], 300, -1), "1개부터 3개까지"),
        (
            body(0, &["ass", "srt", "smi"], 300, 4_294_967_297),
            "1개부터 3개까지",
        ),
    ] {
        let (status, answer) = app.put(bad.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        let message = answer["message"].as_str().unwrap();
        assert!(message.contains(says), "{message}");
    }
    let (status, _) = app
        .put(json!({ "version": 0, "format_order": ["ass"] }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(app.get().await["version"], 0);
}

#[tokio::test]
async fn a_save_from_a_version_another_screen_saved_over_is_refused_with_the_stored_policy() {
    let app = App::new();
    // Two screens opened version 0; the first saves.
    let (status, _) = app.put(body(0, &["srt", "ass", "smi"], 300, 1)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, refused) = app.put(body(0, &["smi", "ass", "srt"], 600, 3)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(refused["message"].as_str().is_some_and(|m| !m.is_empty()));
    assert_eq!(
        refused["current"]["format_order"],
        json!(["srt", "ass", "smi"])
    );
    assert_eq!(refused["current"]["version"], 1);
    // Saving again from the server's version is the explicit second save.
    let (status, saved) = app.put(body(1, &["smi", "ass", "srt"], 600, 3)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["version"], 2);
}

#[tokio::test]
async fn the_works_with_their_own_order_are_listed_with_their_names() {
    let app = App::new();
    app.db
        .run::<_, trss_core::DbError, _>(|c| {
            c.execute_batch(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
                 INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Lycoris Recoil');
                 INSERT INTO work_subtitle_policy (work_id, format_order, updated_at)
                     VALUES ('w1', 'srt,ass,smi', 10);",
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        app.get().await["overrides"],
        json!([{ "work_id": "w1", "name": "Lycoris Recoil", "format_order": ["srt", "ass", "smi"] }])
    );
}
