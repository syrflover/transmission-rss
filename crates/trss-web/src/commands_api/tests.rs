use axum::{
    http::{Method, StatusCode},
    Router,
};
use serde_json::{json, Value};

use super::*;
use crate::testing;
use trss_collect::commands::receive_once::NotRetryable;
use trss_collect::store::{
    channels::{Channel, ChannelInput, Rule, RuleInput, RuleState},
    history::{HistoryItem, HistoryQuery, HistoryResult, Observation},
};
use trss_core::Db;

const TOKEN: &str = "s3cr3t-tok-0123";
const ID: &str = "0b7d5a44-6c1e-4c62-9a6a-3f0c1d2e4b55";

struct App {
    state: AppState,
    router: Router,
}

impl App {
    fn new() -> App {
        let state = AppState::new(Db::open_blocking(":memory:").unwrap());
        let router = testing::api(&state);
        App { state, router }
    }

    async fn call(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, String, Value) {
        testing::call_text(&self.router, method, uri, body).await
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
            trss_core::commands::Outcome {
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
    assert_eq!(body["message"], BUSY);
}

#[tokio::test]
async fn a_request_for_a_rule_is_refused_when_the_rule_would_not_pick_the_item() {
    let app = App::new();
    let channel = app.channel().await;
    let liar = app.rule(&channel, RuleState::Active).await;
    let another = app
        .state
        .channels
        .create_rule(
            &channel.id,
            RuleInput {
                r#match: Some("Another Show".into()),
                directory: "Another Show/Season 01".into(),
                ..RuleInput::default()
            },
        )
        .await
        .unwrap();
    let item = app.item(&channel, "26", HistoryResult::NoMatch).await;

    // The rule would not pick the item (the reasons are trss-collect
    // `receive_once::adoption_plan`'s): a 400 with its sentence, and nothing
    // stored.
    let (status, text, body) = app
        .post_payload(ID, json!({ "item_id": item.id, "rule_id": another.id }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert_eq!(body["error"], "invalid");
    assert_eq!(body["message"], NotRetryable::NotMatching.message());
    assert!(app.state.commands.get(ID).await.unwrap().is_none());

    // The rule that would pick it is accepted, and the request is stored with
    // the rule.
    let (status, text, _) = app
        .post_payload(ID, json!({ "item_id": item.id, "rule_id": liar.id }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    let stored = app.state.commands.get(ID).await.unwrap().unwrap();
    assert!(stored.payload.contains(&liar.id), "{}", stored.payload);
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

/// Which reason an item cannot be retried for is trss-collect
/// `receive_once::retry_plan`'s, and the sentences are `NotRetryable::message`'s;
/// the web answers 400 with the sentence as it is.
#[tokio::test]
async fn an_item_that_cannot_be_retried_is_refused_with_its_reason_and_nothing_is_stored() {
    let app = App::new();
    let channel = app.channel().await;
    let active = app.rule(&channel, RuleState::Active).await;
    let deleted = app.rule(&channel, RuleState::Active).await;
    let rule_deleted = app.failed(&channel, "1", &deleted).await;
    app.state
        .channels
        .delete_rule(&deleted.id, deleted.version)
        .await
        .unwrap();
    let received = app
        .item_of(
            &channel,
            "6",
            HistoryResult::Received,
            Some(active.id.clone()),
        )
        .await;

    for (item, why) in [
        (&rule_deleted, NotRetryable::RuleDeleted),
        (&received, NotRetryable::Held),
    ] {
        let (status, text, body) = app.post(ID, item).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{}: {text}", item.title);
        assert_eq!(body["error"], "invalid");
        assert_eq!(body["message"], why.message(), "{}", item.title);
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

// --- `되돌리기` of an automatic episode offset --------------------------------------

fn undo_body(id: &str, rule_id: &str, episode: i64) -> Value {
    json!({
        "id": id,
        "kind": "episode_undo",
        "payload": { "rule_id": rule_id, "episode": episode },
    })
}

/// A rule whose offset the app set to `−48`, over the `1` it held.
async fn automatic_rule(app: &App, channel: &Channel) -> Rule {
    let rule = app.rule(channel, RuleState::Active).await;
    app.state
        .channels
        .set_auto_episode(
            &rule.id,
            rule.version,
            -48,
            "첫 화가 49화라서 −48로 정했어요.",
        )
        .await
        .unwrap()
        .expect("the rule was at the version read")
}

/// Whether the rule has an automatic value to undo (typed, changed since, an
/// unfinished undo) is trss-collect `episode_undo::standing`'s, which the
/// worker decides again; the web maps it to 404, 400 and 202.
#[tokio::test]
async fn an_undo_is_accepted_for_the_automatic_value_the_user_saw_and_refused_otherwise() {
    let app = App::new();
    let channel = app.channel().await;
    let rule = app.rule(&channel, RuleState::Active).await;

    // A rule that is gone.
    let (status, text, _) = app
        .call(
            Method::POST,
            "/api/commands",
            Some(undo_body(ID, "gone", -48)),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{text}");

    // A value the user typed is nothing to undo.
    let (status, text, _) = app
        .call(
            Method::POST,
            "/api/commands",
            Some(undo_body(ID, &rule.id, -48)),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert!(app.state.commands.get(ID).await.unwrap().is_none());

    // The automatic value the user saw is accepted.
    let rule = automatic_rule(&app, &channel).await;
    let (status, text, view) = app
        .call(
            Method::POST,
            "/api/commands",
            Some(undo_body(ID, &rule.id, -48)),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    assert_eq!(view["kind"], "episode_undo");
    let stored = app.state.commands.get(ID).await.unwrap().unwrap();
    assert_eq!(stored.subject.as_deref(), Some(rule.id.as_str()));
}

// --- watch_rescan ---------------------------------------------------------------------------

#[tokio::test]
async fn a_rescan_is_accepted_for_a_registered_folder_once_and_refused_otherwise() {
    let app = App::new();
    app.state
        .library
        .add_folder(
            "/anime".into(),
            trss_library::discovery::Scan { works: vec![] },
            100,
            &[],
        )
        .await
        .unwrap();
    let folder = app.state.library.folders().await.unwrap().remove(0);
    let send = |id: &str, payload: Value| {
        let body = json!({ "id": id, "kind": "watch_rescan", "payload": payload });
        let app = &app;
        async move { app.call(Method::POST, "/api/commands", Some(body)).await }
    };
    let payload = json!({ "folder_id": folder.id });

    // Accepted is not done: the worker reads the folder.
    let (status, text, body) = send("rescan-0001", payload.clone()).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    assert_eq!(body["state"], "pending");
    assert_eq!(body["kind"], "watch_rescan");

    // The same command again is the same command; a second one for the same
    // folder is refused while the first is open.
    let (status, _, again) = send("rescan-0001", payload.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["id"], "rescan-0001");
    let (status, _, body) = send("rescan-0002", payload.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("다시 확인하는 중"));

    // A folder that is not registered is refused, and a payload without one.
    let (status, _, _) = send("rescan-0003", json!({ "folder_id": "nope" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = send("rescan-0004", json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Nothing but the first was stored.
    let (status, _, _) = app
        .call(Method::GET, "/api/commands/rescan-0002", None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
