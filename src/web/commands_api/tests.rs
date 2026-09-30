use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use crate::store::{
    channels::{Channel, ChannelInput},
    history::{HistoryItem, HistoryQuery, HistoryResult, Observation},
    Db,
};

const TOKEN: &str = "s3cr3t-tok-0123";
const ID: &str = "0b7d5a44-6c1e-4c62-9a6a-3f0c1d2e4b55";

struct App {
    state: AppState,
    router: Router,
}

impl App {
    fn new() -> App {
        let state = AppState::new(Db::open_blocking(":memory:").unwrap());
        let router =
            Router::new().nest("/api", crate::web::api::router().with_state(state.clone()));
        App { state, router }
    }

    async fn call(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, String, Value) {
        let mut request = Request::builder().method(method).uri(uri);
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
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let json = serde_json::from_str(&text).unwrap_or(Value::Null);
        (status, text, json)
    }

    async fn channel(&self, base_dir: &str) -> Channel {
        let url = format!("https://feed.example/rss?token={TOKEN}");
        self.state
            .channels
            .create_channel(ChannelInput::new(url, base_dir))
            .await
            .unwrap()
    }

    async fn item(&self, channel: &Channel, key: &str, result: HistoryResult) -> HistoryItem {
        self.state
            .history
            .record(
                1_000,
                vec![Observation {
                    channel_id: channel.id.clone(),
                    channel_label: channel.masked_url(),
                    identity_key: key.to_owned(),
                    title: format!("LIAR GAME - {key}"),
                    link: "magnet:?xt=urn:btih:abc&token=***".to_owned(),
                    result,
                    rule_id: None,
                    torrent_hash: None,
                    reason: None,
                }],
            )
            .await
            .unwrap();
        let page = self
            .state
            .history
            .list(HistoryQuery {
                channel_id: Some(channel.id.clone()),
                limit: 500,
                ..Default::default()
            })
            .await
            .unwrap();
        page.items
            .into_iter()
            .find(|i| i.identity_key == key)
            .unwrap()
    }

    async fn post(
        &self,
        id: &str,
        item: &HistoryItem,
        folder: &str,
    ) -> (StatusCode, String, Value) {
        self.call(
            Method::POST,
            "/api/commands",
            Some(json!({
                "id": id,
                "kind": "receive_once",
                "payload": { "item_id": item.id, "folder": folder },
            })),
        )
        .await
    }
}

fn assert_no_secret(text: &str) {
    assert!(!text.contains(TOKEN), "secret leaked in: {text}");
}

#[tokio::test]
async fn a_command_is_accepted_pending_and_can_be_read_back() {
    let app = App::new();
    let channel = app.channel("/media/anime").await;
    let item = app.item(&channel, "26", HistoryResult::NoMatch).await;

    let (status, text, view) = app.post(ID, &item, "LIAR GAME/Season 01").await;

    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    assert_eq!(view["id"], ID);
    assert_eq!(view["kind"], "receive_once");
    assert_eq!(view["state"], "pending", "accepted is not received");
    assert_eq!(view["outcome"], Value::Null);
    assert_no_secret(&text);

    let (status, _, read) = app
        .call(Method::GET, &format!("/api/commands/{ID}"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["state"], "pending");

    // The stored payload is the canonical form of the request.
    let stored = app.state.commands.get(ID).await.unwrap().unwrap();
    assert_eq!(
        stored.payload,
        format!(
            r#"{{"item_id":{},"folder":"LIAR GAME/Season 01"}}"#,
            item.id
        )
    );
    assert_eq!(
        stored.subject.as_deref(),
        Some(item.id.to_string().as_str())
    );
    // Accepting it did not touch the item.
    let same = app.state.history.get(item.id).await.unwrap().unwrap();
    assert_eq!(same.result, HistoryResult::NoMatch);
}

#[tokio::test]
async fn the_same_command_delivered_twice_is_stored_once_and_answered_alike() {
    let app = App::new();
    let channel = app.channel("/media/anime").await;
    let item = app.item(&channel, "26", HistoryResult::NoMatch).await;

    let (first, _, one) = app.post(ID, &item, "LIAR GAME/Season 01").await;
    // The repeat may differ in spacing only.
    let (second, _, two) = app.post(ID, &item, " LIAR GAME/Season 01 ").await;

    assert_eq!(first, StatusCode::ACCEPTED);
    assert_eq!(second, StatusCode::OK);
    assert_eq!(one, two);

    // After the worker ended it, a late repeat still gets the outcome.
    let claimed = app.state.commands.claim_next(2_000).await.unwrap().unwrap();
    app.state
        .commands
        .finish(
            &claimed.id,
            CommandState::Done,
            crate::store::commands::Outcome {
                result: "received".into(),
                reason: None,
            },
            3_000,
        )
        .await
        .unwrap();
    let (third, _, late) = app.post(ID, &item, "LIAR GAME/Season 01").await;
    assert_eq!(third, StatusCode::OK);
    assert_eq!(late["state"], "done");
    assert_eq!(late["outcome"]["result"], "received");
}

#[tokio::test]
async fn the_same_id_for_another_item_or_folder_is_refused() {
    let app = App::new();
    let channel = app.channel("/media/anime").await;
    let item = app.item(&channel, "26", HistoryResult::NoMatch).await;
    let other = app.item(&channel, "27", HistoryResult::NoMatch).await;
    app.post(ID, &item, "LIAR GAME/Season 01").await;

    let (status, text, body) = app.post(ID, &other, "LIAR GAME/Season 01").await;
    assert_eq!(status, StatusCode::CONFLICT, "{text}");
    assert_eq!(body["error"], "conflict");
    assert!(body["message"].as_str().unwrap().ends_with("요."));
    assert_eq!(body["current"]["id"], ID);

    let (status, _, _) = app.post(ID, &item, "Elsewhere/Season 01").await;
    assert_eq!(status, StatusCode::CONFLICT);

    // The stored command is still the first one.
    let stored = app.state.commands.get(ID).await.unwrap().unwrap();
    assert!(stored.payload.contains(&format!("\"item_id\":{}", item.id)));
    assert!(stored.payload.contains("LIAR GAME/Season 01"));
}

#[tokio::test]
async fn a_second_command_for_an_item_still_being_received_is_refused() {
    let app = App::new();
    let channel = app.channel("/media/anime").await;
    let item = app.item(&channel, "26", HistoryResult::NoMatch).await;
    app.post(ID, &item, "").await;

    let (status, _, body) = app.post("another-command-id", &item, "").await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["current"]["id"], ID, "the open command is handed back");
    assert!(app
        .state
        .commands
        .get("another-command-id")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_folder_that_leaves_the_base_folder_is_refused_and_nothing_is_stored() {
    let app = App::new();
    let channel = app.channel("/media/anime").await;
    let item = app.item(&channel, "26", HistoryResult::NoMatch).await;

    for folder in [
        "../../etc",
        "..",
        "a/../../b",
        "/etc",
        "/media/anime/x",
        "a\\b",
    ] {
        let (status, text, body) = app.post(ID, &item, folder).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{folder}: {text}");
        assert_eq!(body["error"], "invalid");
        let message = body["message"].as_str().unwrap();
        assert!(message.contains("저장 폴더"), "{message}");
        assert!(message.ends_with("요."), "{message}");
    }

    assert!(app.state.commands.get(ID).await.unwrap().is_none());
    let (status, _, _) = app
        .call(Method::GET, &format!("/api/commands/{ID}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[cfg(unix)]
#[tokio::test]
async fn a_folder_that_reaches_outside_through_a_link_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("anime");
    let outside = dir.path().join("elsewhere");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, base.join("escape")).unwrap();

    let app = App::new();
    let channel = app.channel(base.to_str().unwrap()).await;
    let item = app.item(&channel, "26", HistoryResult::NoMatch).await;

    let (status, _, body) = app.post(ID, &item, "escape/Season 01").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["message"].as_str().unwrap().contains("링크"));
    assert!(app.state.commands.get(ID).await.unwrap().is_none());
}

#[tokio::test]
async fn requests_that_do_not_make_sense_are_refused_with_a_sentence() {
    let app = App::new();
    let channel = app.channel("/media/anime").await;
    let item = app.item(&channel, "26", HistoryResult::NoMatch).await;
    let received = app.item(&channel, "25", HistoryResult::Received).await;
    let duplicate = app.item(&channel, "24", HistoryResult::Duplicate).await;

    let (status, _, body) = app.post(ID, &received, "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["message"].as_str().unwrap().contains("이미"));
    assert_eq!(
        app.post(ID, &duplicate, "").await.0,
        StatusCode::BAD_REQUEST
    );

    let mut missing = item.clone();
    missing.id = 9_999;
    assert_eq!(app.post(ID, &missing, "").await.0, StatusCode::NOT_FOUND);

    for (id, kind, payload) in [
        ("short", "receive_once", json!({ "item_id": item.id })),
        (
            "has spaces in it",
            "receive_once",
            json!({ "item_id": item.id }),
        ),
        (ID, "something_else", json!({ "item_id": item.id })),
        (ID, "receive_once", json!({ "item_id": "7" })),
        (
            ID,
            "receive_once",
            json!({ "item_id": item.id, "link": "magnet:?x" }),
        ),
        (ID, "receive_once", json!({})),
    ] {
        let (status, text, body) = app
            .call(
                Method::POST,
                "/api/commands",
                Some(json!({ "id": id, "kind": kind, "payload": payload })),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{id} {kind} {payload}: {text}"
        );
        assert_eq!(body["error"], "invalid");
    }
    let (status, _, _) = app
        .call(
            Method::POST,
            "/api/commands",
            Some(json!({ "nonsense": true })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A deleted channel cannot say where the item should go.
    let gone = app.channel("/media/gone").await;
    let orphan = app.item(&gone, "1", HistoryResult::NoMatch).await;
    app.state
        .channels
        .delete_channel(&gone.id, gone.version, 0)
        .await
        .unwrap();
    let (status, _, body) = app.post(ID, &orphan, "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["message"].as_str().unwrap().contains("채널"));

    assert!(app.state.commands.get(ID).await.unwrap().is_none());
}

#[tokio::test]
async fn an_unknown_command_id_is_not_found() {
    let app = App::new();

    let (status, _, body) = app
        .call(Method::GET, "/api/commands/never-sent-id", None)
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not_found");
    assert!(body["message"].as_str().unwrap().ends_with("요."));
}
