use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use trss_legacy::store::{
    channels::{Channel, ChannelInput, Rule, RuleInput, RuleState},
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
            Router::new().nest("/api", crate::api::router().with_state(state.clone()));
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

    async fn channel(&self) -> Channel {
        let url = format!("https://feed.example/rss?token={TOKEN}");
        self.state
            .channels
            .create_channel(ChannelInput::new(url))
            .await
            .unwrap()
    }

    async fn rule(&self, channel: &Channel, state: RuleState) -> Rule {
        self.state
            .channels
            .create_rule(
                &channel.id,
                RuleInput {
                    r#match: Some("LIAR GAME".into()),
                    directory: "LIAR GAME/Season 01".into(),
                    state,
                    ..RuleInput::default()
                },
            )
            .await
            .unwrap()
    }

    /// An item with the result, not picked by any rule.
    async fn item(&self, channel: &Channel, key: &str, result: HistoryResult) -> HistoryItem {
        self.item_of(channel, key, result, None).await
    }

    /// An item a rule picked and failed to add: what `다시 받기` is for.
    async fn failed(&self, channel: &Channel, key: &str, rule: &Rule) -> HistoryItem {
        self.item_of(
            channel,
            key,
            HistoryResult::AddFailed,
            Some(rule.id.clone()),
        )
        .await
    }

    async fn item_of(
        &self,
        channel: &Channel,
        key: &str,
        result: HistoryResult,
        rule_id: Option<String>,
    ) -> HistoryItem {
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
                    rule_id,
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

    async fn post(&self, id: &str, item: &HistoryItem) -> (StatusCode, String, Value) {
        self.post_payload(id, json!({ "item_id": item.id })).await
    }

    async fn post_payload(&self, id: &str, payload: Value) -> (StatusCode, String, Value) {
        self.call(
            Method::POST,
            "/api/commands",
            Some(json!({ "id": id, "kind": "receive_once", "payload": payload })),
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
    let channel = app.channel().await;
    let rule = app.rule(&channel, RuleState::Active).await;
    let item = app.failed(&channel, "26", &rule).await;

    let (status, text, view) = app.post(ID, &item).await;

    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    assert_eq!(view["id"], ID);
    assert_eq!(view["kind"], "receive_once");
    assert_eq!(view["state"], "pending", "accepted is not added");
    assert_eq!(view["outcome"], Value::Null);
    assert_no_secret(&text);

    let (status, _, read) = app
        .call(Method::GET, &format!("/api/commands/{ID}"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["state"], "pending");

    // The stored payload is the canonical form of the request: the item alone.
    let stored = app.state.commands.get(ID).await.unwrap().unwrap();
    assert_eq!(stored.payload, format!(r#"{{"item_id":{}}}"#, item.id));
    assert_eq!(
        stored.subject.as_deref(),
        Some(item.id.to_string().as_str())
    );
    // Accepting it did not touch the item.
    let same = app.state.history.get(item.id).await.unwrap().unwrap();
    assert_eq!(same.result, HistoryResult::AddFailed);
    assert_eq!(same.rule_id.as_deref(), Some(rule.id.as_str()));
}

#[tokio::test]
async fn the_same_command_delivered_twice_is_stored_once_and_answered_alike() {
    let app = App::new();
    let channel = app.channel().await;
    let rule = app.rule(&channel, RuleState::Active).await;
    let item = app.failed(&channel, "26", &rule).await;

    let (first, _, one) = app.post(ID, &item).await;
    // A client from before the folder went away repeats it with an empty folder.
    let (second, _, two) = app
        .post_payload(ID, json!({ "item_id": item.id, "folder": " " }))
        .await;

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
            trss_legacy::store::commands::Outcome {
                result: "received".into(),
                reason: None,
            },
            3_000,
        )
        .await
        .unwrap();
    let (third, _, late) = app.post(ID, &item).await;
    assert_eq!(third, StatusCode::OK);
    assert_eq!(late["state"], "done");
    assert_eq!(late["outcome"]["result"], "received");
}

#[tokio::test]
async fn the_same_id_for_another_item_is_refused() {
    let app = App::new();
    let channel = app.channel().await;
    let rule = app.rule(&channel, RuleState::Active).await;
    let item = app.failed(&channel, "26", &rule).await;
    let other = app.failed(&channel, "27", &rule).await;
    app.post(ID, &item).await;

    let (status, text, body) = app.post(ID, &other).await;
    assert_eq!(status, StatusCode::CONFLICT, "{text}");
    assert_eq!(body["error"], "conflict");
    assert!(body["message"].as_str().unwrap().ends_with("요."));
    assert_eq!(body["current"]["id"], ID);

    // The stored command is still the first one.
    let stored = app.state.commands.get(ID).await.unwrap().unwrap();
    assert!(stored.payload.contains(&format!("\"item_id\":{}", item.id)));
}

#[tokio::test]
async fn a_second_command_for_an_item_still_being_added_is_refused() {
    let app = App::new();
    let channel = app.channel().await;
    let rule = app.rule(&channel, RuleState::Active).await;
    let item = app.failed(&channel, "26", &rule).await;
    app.post(ID, &item).await;

    let (status, _, body) = app.post("another-command-id", &item).await;

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
async fn a_request_that_names_a_folder_is_refused_and_nothing_is_stored() {
    let app = App::new();
    let channel = app.channel().await;
    let rule = app.rule(&channel, RuleState::Active).await;
    let item = app.failed(&channel, "26", &rule).await;

    for folder in ["LIAR GAME/Season 01", "../../etc", "/etc", "x"] {
        let (status, text, body) = app
            .post_payload(ID, json!({ "item_id": item.id, "folder": folder }))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{folder}: {text}");
        assert_eq!(body["error"], "invalid");
        let message = body["message"].as_str().unwrap();
        assert!(message.contains("폴더를 고를 수 없어요"), "{message}");
        assert!(message.ends_with("요."), "{message}");
    }

    assert!(app.state.commands.get(ID).await.unwrap().is_none());
    let (status, _, _) = app
        .call(Method::GET, &format!("/api/commands/{ID}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_item_that_cannot_be_retried_is_refused_with_its_reason_and_nothing_is_stored() {
    let app = App::new();
    let channel = app.channel().await;
    let active = app.rule(&channel, RuleState::Active).await;
    let archived = app.rule(&channel, RuleState::Archived).await;
    let deleted = app.rule(&channel, RuleState::Active).await;
    let rule_deleted = app.failed(&channel, "1", &deleted).await;
    app.state
        .channels
        .delete_rule(&deleted.id, deleted.version)
        .await
        .unwrap();
    let rule_archived = app.failed(&channel, "2", &archived).await;
    let no_rule = app.item(&channel, "3", HistoryResult::AddFailed).await;
    let no_match = app.item(&channel, "4", HistoryResult::NoMatch).await;
    let excluded = app.item(&channel, "5", HistoryResult::Excluded).await;
    let received = app
        .item_of(
            &channel,
            "6",
            HistoryResult::Received,
            Some(active.id.clone()),
        )
        .await;
    let duplicate = app
        .item_of(
            &channel,
            "7",
            HistoryResult::Duplicate,
            Some(active.id.clone()),
        )
        .await;

    for (item, says) in [
        (&rule_deleted, "지워져서"),
        (&rule_archived, "복원한 뒤"),
        (&no_rule, "규칙 없이"),
        (&no_match, "규칙이 고르지 않아서"),
        (&excluded, "규칙이 고르지 않아서"),
        (&received, "이미"),
        (&duplicate, "이미"),
    ] {
        let (status, text, body) = app.post(ID, item).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{}: {text}", item.title);
        assert_eq!(body["error"], "invalid");
        let message = body["message"].as_str().unwrap();
        assert!(message.contains(says), "{}: {message}", item.title);
        assert!(message.ends_with("요."), "{message}");
    }
    assert!(app.state.commands.get(ID).await.unwrap().is_none());
}

#[tokio::test]
async fn requests_that_do_not_make_sense_are_refused_with_a_sentence() {
    let app = App::new();
    let channel = app.channel().await;
    let rule = app.rule(&channel, RuleState::Active).await;
    let item = app.failed(&channel, "26", &rule).await;

    let mut missing = item.clone();
    missing.id = 9_999;
    assert_eq!(app.post(ID, &missing).await.0, StatusCode::NOT_FOUND);

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

    // A deleted channel takes its rules along, so nothing says where to go.
    let gone = app.channel().await;
    let gone_rule = app.rule(&gone, RuleState::Active).await;
    let orphan = app.failed(&gone, "1", &gone_rule).await;
    app.state
        .channels
        .delete_channel(&gone.id, gone.version, 1)
        .await
        .unwrap();
    let (status, _, body) = app.post(ID, &orphan).await;
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

/// Every command accepted before `다시 받기` stored `{"item_id":N,"folder":"…"}`.
async fn store_legacy(app: &App, id: &str, item: &HistoryItem, folder: &str) -> Command {
    let payload = format!(r#"{{"item_id":{},"folder":{}}}"#, item.id, json!(folder));
    match app
        .state
        .commands
        .accept(
            NewCommand {
                id: id.to_owned(),
                kind: "receive_once".to_owned(),
                payload,
                subject: Some(item.id.to_string()),
            },
            1_500,
        )
        .await
        .unwrap()
    {
        Accepted::Created(command) => command,
        other => panic!("not stored: {other:?}"),
    }
}

#[tokio::test]
async fn a_legacy_command_sent_again_is_answered_with_the_stored_command() {
    let app = App::new();
    let channel = app.channel().await;
    let rule = app.rule(&channel, RuleState::Active).await;
    // The item is not retryable now (it was received since): a stored command
    // is answered before the item is looked at.
    let item = app
        .item_of(
            &channel,
            "26",
            HistoryResult::Received,
            Some(rule.id.clone()),
        )
        .await;
    let item_named = app
        .item_of(
            &channel,
            "28",
            HistoryResult::Received,
            Some(rule.id.clone()),
        )
        .await;
    let other = app.failed(&channel, "27", &rule).await;

    let empty_id = "legacy-empty-folder";
    let named_id = "legacy-named-folder";
    let stored_empty = store_legacy(&app, empty_id, &item, "").await;
    let stored_named = store_legacy(&app, named_id, &item_named, "LIAR GAME/Season 01").await;

    // The request of the current client, and the old tab's own.
    for (id, stored, payload) in [
        (empty_id, &stored_empty, json!({ "item_id": item.id })),
        (
            empty_id,
            &stored_empty,
            json!({ "item_id": item.id, "folder": "" }),
        ),
        (
            named_id,
            &stored_named,
            json!({ "item_id": item_named.id, "folder": "LIAR GAME/Season 01" }),
        ),
    ] {
        let (status, text, body) = app.post_payload(id, payload.clone()).await;
        assert_eq!(status, StatusCode::OK, "{payload}: {text}");
        assert_eq!(body["id"], id);
        assert_eq!(body["state"], "pending");
        assert_eq!(
            body,
            serde_json::to_value(CommandView::from(stored)).unwrap(),
            "{payload}"
        );
    }

    // Another item under the same ID is still a different request.
    let (status, _, body) = app
        .post_payload(empty_id, json!({ "item_id": other.id }))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "conflict");
    assert_eq!(body["current"]["id"], empty_id);

    // A new ID that names a folder is still refused.
    let (status, _, _) = app
        .post_payload(
            "a-new-command-id",
            json!({ "item_id": other.id, "folder": "x" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// --- rule_archive (보관·복원) -------------------------------------------------------

fn archive_body(id: &str, rule: &Rule, direction: &str) -> Value {
    json!({
        "id": id,
        "kind": "rule_archive",
        "payload": { "rule_id": rule.id, "direction": direction },
    })
}

#[tokio::test]
async fn an_archive_is_accepted_for_its_rule_once_and_the_rule_is_not_changed_by_the_web() {
    let app = App::new();
    let channel = app.channel().await;
    let rule = app.rule(&channel, RuleState::Active).await;

    let (status, text, body) = app
        .call(
            Method::POST,
            "/api/commands",
            Some(archive_body(ID, &rule, "archive")),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    assert_eq!(body["kind"], "rule_archive");
    assert_eq!(body["state"], "pending");
    let stored = app.state.commands.get(ID).await.unwrap().unwrap();
    assert_eq!(stored.subject.as_deref(), Some(rule.id.as_str()));
    // Accepted is not done: the worker turns the rule off.
    let now = app
        .state
        .channels
        .get_rule(&rule.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(now.state, RuleState::Active);
    assert_eq!(now.version, rule.version);

    // The same request again is the same command.
    let (status, _, again) = app
        .call(
            Method::POST,
            "/api/commands",
            Some(archive_body(ID, &rule, "archive")),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["id"], ID);

    // The same ID for the other direction is another request.
    let (status, _, _) = app
        .call(
            Method::POST,
            "/api/commands",
            Some(archive_body(ID, &rule, "restore")),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // A second command for the rule while one is open, in either direction.
    let (status, text, body) = app
        .call(
            Method::POST,
            "/api/commands",
            Some(archive_body("second-command-1", &rule, "archive")),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{text}");
    assert_eq!(body["current"]["id"], ID);
    assert!(
        body["message"].as_str().unwrap().contains("이 규칙은"),
        "{text}"
    );
    assert!(app
        .state
        .commands
        .get("second-command-1")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_restore_needs_an_archived_rule_and_a_missing_rule_is_not_found() {
    let app = App::new();
    let channel = app.channel().await;
    let active = app.rule(&channel, RuleState::Active).await;
    let archived = app.rule(&channel, RuleState::Archived).await;

    let (status, text, _) = app
        .call(
            Method::POST,
            "/api/commands",
            Some(archive_body(ID, &active, "restore")),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert!(app.state.commands.get(ID).await.unwrap().is_none());

    // A paused rule is switched on, not restored: its folder never moved.
    let paused = app.rule(&channel, RuleState::Paused).await;
    let (status, text, _) = app
        .call(
            Method::POST,
            "/api/commands",
            Some(archive_body(ID, &paused, "restore")),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert!(app.state.commands.get(ID).await.unwrap().is_none());

    let (status, text, _) = app
        .call(
            Method::POST,
            "/api/commands",
            Some(archive_body(ID, &archived, "restore")),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");

    // Archiving an archived rule is `다시 옮기기`.
    let (status, text, _) = app
        .call(
            Method::POST,
            "/api/commands",
            Some(archive_body("again-command-1", &active, "archive")),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");

    let mut gone = active.clone();
    gone.id = "no-such-rule".into();
    let (status, _, _) = app
        .call(
            Method::POST,
            "/api/commands",
            Some(archive_body("gone-command-1", &gone, "archive")),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    for payload in [
        json!({ "rule_id": active.id, "direction": "away" }),
        json!({ "rule_id": active.id }),
        json!({ "rule_id": active.id, "direction": "archive", "folder": "/x" }),
    ] {
        let (status, text, _) = app
            .call(
                Method::POST,
                "/api/commands",
                Some(json!({ "id": "bad-command-1", "kind": "rule_archive", "payload": payload })),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    }
}
