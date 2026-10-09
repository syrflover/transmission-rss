use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use trss_collect::{
    plan::Judgement,
    store::{
        channels::{ChannelInput, ChannelWithRules},
        history::Observation,
    },
};
use trss_core::Db;

struct App {
    state: AppState,
    router: Router,
    db: Db,
}

impl App {
    /// An app whose collect folder is `/media`.
    async fn new() -> App {
        let db = Db::open_blocking(":memory:").unwrap();
        let state = AppState::new(db.clone());
        state
            .settings
            .put_collection(0, "/media".to_owned(), None)
            .await
            .unwrap();
        let router = Router::new().nest("/api", crate::api::router().with_state(state.clone()));
        App { state, router, db }
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

    /// A channel at `https://<host>/rss?token=<token>` with rules `(phrase, directory)`.
    async fn channel(
        &self,
        host: &str,
        excludes: &[&str],
        rules: &[(&str, &str)],
    ) -> ChannelWithRules {
        let mut input = ChannelInput::new(format!("https://{host}/rss?token=SECRETVALUE99"));
        input.excludes = excludes.iter().map(|s| s.to_string()).collect();
        let rules = rules
            .iter()
            .map(|(phrase, directory)| RuleInput {
                r#match: Some(phrase.to_string()),
                directory: directory.to_string(),
                ..RuleInput::default()
            })
            .collect();
        self.state
            .channels
            .create_channel_with_rules(input, rules)
            .await
            .unwrap()
    }

    /// Records titles the way the worker would, at time `at`.
    async fn record(&self, channel: &Channel, at: i64, titles: &[&str]) {
        self.record_as(channel, at, titles, HistoryResult::NoMatch, None)
            .await
    }

    async fn record_as(
        &self,
        channel: &Channel,
        at: i64,
        titles: &[&str],
        result: HistoryResult,
        rule_id: Option<&str>,
    ) {
        let observations = titles
            .iter()
            .map(|title| Observation {
                channel_id: channel.id.clone(),
                channel_label: channel.masked_url(),
                identity_key: format!("title:{title}"),
                title: title.to_string(),
                link: "https://feed.test/item".into(),
                result,
                rule_id: rule_id.map(str::to_owned),
                torrent_hash: None,
                reason: None,
            })
            .collect();
        self.state.history.record(at, observations).await.unwrap();
    }

    async fn list(&self) -> Value {
        let (status, text, json) = self.call(Method::GET, "/api/rules", None).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        json
    }

    async fn preview(&self, body: Value) -> Value {
        let (status, text, json) = self
            .call(Method::POST, "/api/rules/preview", Some(body))
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        json
    }
}

fn ids(list: &Value) -> Vec<String> {
    list["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect()
}

fn rule_body(channel: &Channel, rule: &Value, patch: Value) -> Value {
    let mut body = json!({
        "version": rule["version"],
        "channel_id": channel.id,
        "match": rule["match"],
        "regex": rule["regex"],
        "case_insensitive": rule["case_insensitive"],
        "directory": rule["directory"],
        "episode": rule["episode"],
        "state": rule["state"],
    });
    for (key, value) in patch.as_object().unwrap() {
        body[key] = value.clone();
    }
    body
}

fn kinds(preview: &Value) -> Vec<(String, String)> {
    preview["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["title"].as_str().unwrap().to_owned(),
                i["kind"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

// --- the list -----------------------------------------------------------------

#[tokio::test]
async fn the_list_holds_the_rules_of_every_channel_with_their_check_order() {
    let app = App::new().await;
    let a = app
        .channel("a.test", &[], &[("Alpha", "Alpha"), ("Beta", "Beta")])
        .await;
    let b = app.channel("b.test", &[], &[("Gamma", "Gamma")]).await;

    let list = app.list().await;
    assert_eq!(
        ids(&list),
        vec![
            a.rules[0].id.clone(),
            a.rules[1].id.clone(),
            b.rules[0].id.clone()
        ]
    );
    let orders: Vec<u64> = list["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["order"].as_u64().unwrap())
        .collect();
    assert_eq!(orders, [1, 2, 1]);
    assert_eq!(list["channels"].as_array().unwrap().len(), 2);
    assert_eq!(list["channels"][0]["host"], "a.test");
    assert_eq!(list["channels"][0]["rule_count"], 2);
    assert_eq!(list["rules"][0]["overlap"], false);
    assert_eq!(list["rules"][0]["last_received_at"], Value::Null);
    // No secret value of the channel URL is in the response.
    let (_, text, _) = app.call(Method::GET, "/api/rules", None).await;
    assert!(!text.contains("SECRETVALUE99"), "{text}");
}

#[tokio::test]
async fn reading_the_list_changes_nothing_and_the_order_does_not_depend_on_it() {
    // The screen sorts the list by title in the browser; that is a display
    // matter and never a request. Reading twice leaves order and versions alone.
    let app = App::new().await;
    let a = app
        .channel(
            "a.test",
            &[],
            &[("Zeta", "z"), ("Alpha", "a"), ("Mid", "m")],
        )
        .await;
    let before = app.list().await;
    let again = app.list().await;
    assert_eq!(before["rules"], again["rules"]);
    let stored = app.state.channels.list_rules(&a.channel.id).await.unwrap();
    assert_eq!(stored, a.rules);
}

#[tokio::test]
async fn last_received_is_the_latest_time_the_rule_got_an_item_into_transmission() {
    let app = App::new().await;
    let a = app
        .channel("a.test", &[], &[("Alpha", "a"), ("Beta", "b")])
        .await;
    let alpha = a.rules[0].id.as_str();
    app.record_as(
        &a.channel,
        1_000,
        &["Alpha 01"],
        HistoryResult::Received,
        Some(alpha),
    )
    .await;
    app.record_as(
        &a.channel,
        2_000,
        &["Alpha 02"],
        HistoryResult::Received,
        Some(alpha),
    )
    .await;
    app.record_as(
        &a.channel,
        3_000,
        &["Alpha 03"],
        HistoryResult::AddFailed,
        Some(alpha),
    )
    .await;

    let list = app.list().await;
    assert_eq!(list["rules"][0]["last_received_at"], 2_000);
    assert_eq!(list["rules"][1]["last_received_at"], Value::Null);
}

// --- overlap ------------------------------------------------------------------

#[tokio::test]
async fn overlap_is_shown_only_on_the_rule_an_earlier_rule_shadows() {
    let app = App::new().await;
    let a = app
        .channel(
            "a.test",
            &[],
            &[
                ("Show", "first"),
                ("Show - 02", "second"),
                ("Other", "third"),
            ],
        )
        .await;
    app.record(&a.channel, 1_000, &["Show - 01", "Show - 02", "Other - 01"])
        .await;

    let list = app.list().await;
    let overlap: Vec<bool> = list["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["overlap"].as_bool().unwrap())
        .collect();
    // "Show - 02" is taken by the first rule although the second matches too.
    assert_eq!(overlap, [false, true, false]);
}

// --- saving -------------------------------------------------------------------

#[tokio::test]
async fn a_rule_is_created_at_the_end_saved_and_deleted_with_versions() {
    let app = App::new().await;
    let a = app.channel("a.test", &[], &[("Alpha", "a")]).await;

    let (status, text, created) = app
        .call(
            Method::POST,
            "/api/rules",
            Some(json!({
                "channel_id": a.channel.id,
                "match": "Beta",
                "case_insensitive": true,
                "directory": " Beta/Season 01 ",
                "episode": -12,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{text}");
    assert_eq!(created["order"], 2);
    assert_eq!(created["directory"], "Beta/Season 01");
    assert_eq!(created["episode"], -12);
    assert_eq!(created["state"], "active");
    assert_eq!(created["case_insensitive"], true);

    let id = created["id"].as_str().unwrap();
    let uri = format!("/api/rules/{id}");
    let (status, text, saved) = app
        .call(
            Method::PUT,
            &uri,
            Some(rule_body(
                &a.channel,
                &created,
                json!({ "match": "Beta - " }),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(saved["match"], "Beta - ", "the trailing space is kept");
    assert_eq!(saved["version"], created["version"].as_i64().unwrap() + 1);

    // A blank phrase is a rule waiting for its title.
    let (_, _, waiting) = app
        .call(
            Method::PUT,
            &uri,
            Some(rule_body(&a.channel, &saved, json!({ "match": "" }))),
        )
        .await;
    assert_eq!(waiting["match"], Value::Null);

    let (status, text, removed) = app
        .call(
            Method::DELETE,
            &format!("{uri}?version={}", waiting["version"]),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(removed["removed"], true);
    assert_eq!(app.list().await["rules"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn the_second_save_from_an_old_version_is_a_conflict_that_shows_the_saved_rule() {
    let app = App::new().await;
    let a = app.channel("a.test", &[], &[("Alpha", "a")]).await;
    let rule = app.list().await["rules"][0].clone();
    let uri = format!("/api/rules/{}", rule["id"].as_str().unwrap());

    // Two screens open the same rule; the first saves.
    let (status, _, first) = app
        .call(
            Method::PUT,
            &uri,
            Some(rule_body(&a.channel, &rule, json!({ "directory": "one" }))),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, conflict) = app
        .call(
            Method::PUT,
            &uri,
            Some(rule_body(&a.channel, &rule, json!({ "directory": "two" }))),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict["error"], "conflict");
    assert_eq!(conflict["current"]["directory"], "one");
    assert_eq!(conflict["current"]["version"], first["version"]);
    // Nothing of the second save reached the store.
    let stored = app
        .state
        .channels
        .get_rule(rule["id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.directory, "one");

    // Resaving against the version the conflict showed goes through.
    let (status, _, resaved) = app
        .call(
            Method::PUT,
            &uri,
            Some(rule_body(
                &a.channel,
                &conflict["current"],
                json!({ "directory": "two" }),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resaved["directory"], "two");
}

#[tokio::test]
async fn a_delete_from_an_old_version_is_a_conflict() {
    let app = App::new().await;
    let a = app.channel("a.test", &[], &[("Alpha", "a")]).await;
    let rule = app.list().await["rules"][0].clone();
    let uri = format!("/api/rules/{}", rule["id"].as_str().unwrap());
    app.call(
        Method::PUT,
        &uri,
        Some(rule_body(&a.channel, &rule, json!({ "directory": "one" }))),
    )
    .await;

    let (status, _, conflict) = app
        .call(
            Method::DELETE,
            &format!("{uri}?version={}", rule["version"]),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict["current"]["directory"], "one");
    assert_eq!(app.list().await["rules"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn an_invalid_regex_is_refused_with_a_sentence_and_nothing_is_saved() {
    let app = App::new().await;
    let a = app.channel("a.test", &[], &[("Alpha", "a")]).await;
    let rule = app.list().await["rules"][0].clone();
    let uri = format!("/api/rules/{}", rule["id"].as_str().unwrap());

    let (status, text, error) = app
        .call(
            Method::PUT,
            &uri,
            Some(rule_body(
                &a.channel,
                &rule,
                json!({ "regex": true, "match": "Show - (" }),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert_eq!(error["error"], "invalid");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("정규식이 올바르지 않아요"),
        "{text}"
    );
    let stored = app
        .state
        .channels
        .get_rule(rule["id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.r#match.as_deref(), Some("Alpha"));
    assert!(!stored.regex);

    // The same for a new rule.
    let (status, _, _) = app
        .call(
            Method::POST,
            "/api/rules",
            Some(json!({
                "channel_id": a.channel.id, "match": "(", "regex": true, "episode": 1
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(app.list().await["rules"].as_array().unwrap().len(), 1);

    // A phrase that is a valid regex, or a plain phrase ending in "(", saves.
    let (status, _, _) = app
        .call(
            Method::PUT,
            &uri,
            Some(rule_body(
                &a.channel,
                &rule,
                json!({ "regex": false, "match": "Show - (" }),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn an_absolute_save_folder_and_a_missing_rule_are_refused() {
    let app = App::new().await;
    let a = app.channel("a.test", &[], &[("Alpha", "a")]).await;
    let rule = app.list().await["rules"][0].clone();
    let uri = format!("/api/rules/{}", rule["id"].as_str().unwrap());
    let (status, _, error) = app
        .call(
            Method::PUT,
            &uri,
            Some(rule_body(&a.channel, &rule, json!({ "directory": "/etc" }))),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid");

    let (status, _, error) = app
        .call(
            Method::PUT,
            "/api/rules/nope",
            Some(rule_body(&a.channel, &rule, json!({}))),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["error"], "not_found");
}

#[tokio::test]
async fn a_rule_cannot_move_to_another_channel() {
    let app = App::new().await;
    let a = app.channel("a.test", &[], &[("Alpha", "a")]).await;
    let b = app.channel("b.test", &[], &[]).await;
    let rule = app.list().await["rules"][0].clone();
    let uri = format!("/api/rules/{}", rule["id"].as_str().unwrap());
    let (status, _, error) = app
        .call(
            Method::PUT,
            &uri,
            Some(rule_body(&b.channel, &rule, json!({}))),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid");
    assert_eq!(
        app.state
            .channels
            .list_rules(&a.channel.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn the_episode_offset_stays_automatic_only_until_the_user_changes_it() {
    let app = App::new().await;
    let a = app.channel("a.test", &[], &[]).await;
    app.state
        .channels
        .create_rule(
            &a.channel.id,
            RuleInput {
                r#match: Some("Alpha".into()),
                episode: -24,
                episode_auto: true,
                ..RuleInput::default()
            },
        )
        .await
        .unwrap();
    let rule = app.list().await["rules"][0].clone();
    assert_eq!(rule["episode_auto"], true);
    let uri = format!("/api/rules/{}", rule["id"].as_str().unwrap());

    let (_, _, same) = app
        .call(
            Method::PUT,
            &uri,
            Some(rule_body(&a.channel, &rule, json!({ "directory": "a" }))),
        )
        .await;
    assert_eq!(same["episode_auto"], true);
    let (_, _, typed) = app
        .call(
            Method::PUT,
            &uri,
            Some(rule_body(&a.channel, &same, json!({ "episode": -12 }))),
        )
        .await;
    assert_eq!(typed["episode_auto"], false);
}

// --- order --------------------------------------------------------------------

#[tokio::test]
async fn reordering_takes_the_versioned_list_and_changes_the_check_order() {
    let app = App::new().await;
    let a = app
        .channel("a.test", &[], &[("One", "1"), ("Two", "2"), ("Three", "3")])
        .await;
    let list = app.list().await;
    let rules = list["rules"].as_array().unwrap();
    let order: Vec<Value> = [2usize, 0, 1]
        .iter()
        .map(|&i| json!({ "id": rules[i]["id"], "version": rules[i]["version"] }))
        .collect();

    let (status, text, reordered) = app
        .call(
            Method::PUT,
            "/api/rules/order",
            Some(json!({ "channel_id": a.channel.id, "order": order })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let phrases: Vec<&str> = reordered["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["match"].as_str().unwrap())
        .collect();
    assert_eq!(phrases, ["Three", "One", "Two"]);
    assert_eq!(reordered["rules"][0]["order"], 1);

    // The list shows the new order, and the store holds it.
    let phrases: Vec<String> = app
        .state
        .channels
        .list_rules(&a.channel.id)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.r#match.unwrap())
        .collect();
    assert_eq!(phrases, ["Three", "One", "Two"]);
}

#[tokio::test]
async fn a_stale_order_is_a_conflict_that_carries_the_current_rules() {
    let app = App::new().await;
    let a = app
        .channel("a.test", &[], &[("One", "1"), ("Two", "2")])
        .await;
    let list = app.list().await;
    let stale: Vec<Value> = list["rules"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .map(|r| json!({ "id": r["id"], "version": r["version"] }))
        .collect();

    // Someone else edits the first rule meanwhile.
    let first = &list["rules"][0];
    app.call(
        Method::PUT,
        &format!("/api/rules/{}", first["id"].as_str().unwrap()),
        Some(rule_body(
            &a.channel,
            first,
            json!({ "directory": "moved" }),
        )),
    )
    .await;

    let (status, _, conflict) = app
        .call(
            Method::PUT,
            "/api/rules/order",
            Some(json!({ "channel_id": a.channel.id, "order": stale })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict["current"]["rules"][0]["directory"], "moved");
    let phrases: Vec<String> = app
        .state
        .channels
        .list_rules(&a.channel.id)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.r#match.unwrap())
        .collect();
    assert_eq!(phrases, ["One", "Two"], "the order is unchanged");
}

// --- the preview --------------------------------------------------------------

fn edit(rule: &Value, patch: Value) -> Value {
    let mut edited = json!({
        "match": rule["match"],
        "regex": rule["regex"],
        "case_insensitive": rule["case_insensitive"],
        "directory": rule["directory"],
        "episode": rule["episode"],
    });
    for (key, value) in patch.as_object().unwrap() {
        edited[key] = value.clone();
    }
    edited
}

#[tokio::test]
async fn widening_a_phrase_shows_the_items_an_earlier_rule_takes_and_saving_marks_the_overlap() {
    let app = App::new().await;
    let a = app
        .channel(
            "a.test",
            &[],
            &[
                ("Alpha - ", "Alpha"),
                ("Beta - ", "Beta"),
                ("Gamma - ", "Gamma"),
            ],
        )
        .await;
    app.record(
        &a.channel,
        1_000,
        &["Alpha - 01", "Beta - 01", "Gamma - 01", "Other - 01"],
    )
    .await;
    let gamma = app.list().await["rules"][2].clone();

    // As it is, the rule takes only its own item.
    let before = app
        .preview(json!({
            "channel_id": a.channel.id,
            "rule_id": gamma["id"],
            "rule": edit(&gamma, json!({})),
        }))
        .await;
    assert_eq!(kinds(&before), [("Gamma - 01".into(), "mine".into())]);
    assert_eq!(before["counts"]["unmatched"], 3);

    // Widened to match everything with " - 01": the others go to earlier rules.
    let wide = edit(&gamma, json!({ "match": " - 01" }));
    let preview = app
        .preview(json!({
            "channel_id": a.channel.id,
            "rule_id": gamma["id"],
            "rule": wide,
        }))
        .await;
    let mut got = kinds(&preview);
    got.sort();
    assert_eq!(
        got,
        [
            ("Alpha - 01".to_owned(), "earlier".to_owned()),
            ("Beta - 01".to_owned(), "earlier".to_owned()),
            ("Gamma - 01".to_owned(), "mine".to_owned()),
            ("Other - 01".to_owned(), "mine".to_owned()),
        ],
        "{preview}"
    );
    // The taking rule is named, with its phrase and where the item would go.
    let alpha = preview["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["title"] == "Alpha - 01")
        .unwrap();
    assert_eq!(alpha["taken_by"]["match"], "Alpha - ");
    assert_eq!(alpha["taken_by"]["rule_id"], a.rules[0].id);
    assert_eq!(alpha["save_path"], "/media/Alpha");
    assert_eq!(preview["counts"]["mine"], 2);
    assert_eq!(preview["counts"]["earlier"], 2);

    // Not saved yet: the list has no overlap.
    assert_eq!(app.list().await["rules"][2]["overlap"], false);

    // Saved: the rule's line in the list shows the overlap.
    let (status, text, _) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{}", gamma["id"].as_str().unwrap()),
            Some(rule_body(&a.channel, &gamma, json!({ "match": " - 01" }))),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let list = app.list().await;
    assert_eq!(list["rules"][2]["overlap"], true);
    assert_eq!(list["rules"][0]["overlap"], false);
}

#[tokio::test]
async fn an_invalid_regex_is_shown_in_the_preview_and_other_rules_still_preview() {
    let app = App::new().await;
    let a = app
        .channel("a.test", &[], &[("Alpha", "Alpha"), ("Beta", "Beta")])
        .await;
    app.record(&a.channel, 1_000, &["Alpha - 01", "Beta - 01"])
        .await;
    let rules = app.list().await["rules"].clone();
    let beta = &rules[1];

    let broken = app
        .preview(json!({
            "channel_id": a.channel.id,
            "rule_id": beta["id"],
            "rule": edit(beta, json!({ "regex": true, "match": "Beta - (" })),
        }))
        .await;
    assert!(
        broken["error"]["message"]
            .as_str()
            .unwrap()
            .contains("정규식이 올바르지 않아요"),
        "{broken}"
    );
    assert!(!broken["error"]["detail"].as_str().unwrap().is_empty());
    assert_eq!(kinds(&broken), []);

    // Another rule's preview is unaffected by this one's edit.
    let alpha = &rules[0];
    let fine = app
        .preview(json!({
            "channel_id": a.channel.id,
            "rule_id": alpha["id"],
            "rule": edit(alpha, json!({})),
        }))
        .await;
    assert_eq!(fine["error"], Value::Null);
    assert_eq!(kinds(&fine), [("Alpha - 01".to_owned(), "mine".to_owned())]);
}

#[tokio::test]
async fn the_channel_excludes_and_the_position_in_the_order_apply_to_the_preview() {
    let app = App::new().await;
    let a = app
        .channel(
            "a.test",
            &["[Batch]"],
            &[("Show", "first"), ("Show - 01", "second")],
        )
        .await;
    app.record(&a.channel, 1_000, &["Show - 01", "Show - 01 [Batch]"])
        .await;
    let second = app.list().await["rules"][1].clone();

    // In second place the earlier rule takes the item; the batch is excluded.
    let preview = app
        .preview(json!({
            "channel_id": a.channel.id,
            "rule_id": second["id"],
            "rule": edit(&second, json!({})),
        }))
        .await;
    let mut got = kinds(&preview);
    got.sort();
    assert_eq!(
        got,
        [
            ("Show - 01".to_owned(), "earlier".to_owned()),
            ("Show - 01 [Batch]".to_owned(), "excluded".to_owned()),
        ]
    );
    let excluded = preview["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "excluded")
        .unwrap();
    assert_eq!(excluded["excluded_by"], "[Batch]");
    assert_eq!(excluded["save_path"], Value::Null);

    // Moved to the front, the rule takes the plain item itself.
    let moved = app
        .preview(json!({
            "channel_id": a.channel.id,
            "rule_id": second["id"],
            "rule": edit(&second, json!({})),
            "position": 0,
        }))
        .await;
    let mut got = kinds(&moved);
    got.sort();
    assert_eq!(
        got,
        [
            ("Show - 01".to_owned(), "mine".to_owned()),
            ("Show - 01 [Batch]".to_owned(), "excluded".to_owned()),
        ]
    );
    // The preview changed nothing that is stored.
    let stored = app.state.channels.list_rules(&a.channel.id).await.unwrap();
    assert_eq!(stored, a.rules);
}

#[tokio::test]
async fn a_rule_not_saved_yet_previews_as_the_last_rule_and_an_archived_one_as_collecting() {
    let app = App::new().await;
    let a = app.channel("a.test", &[], &[("Show", "first")]).await;
    app.record(&a.channel, 1_000, &["Show - 01", "New - 01"])
        .await;

    let new = app
        .preview(json!({
            "channel_id": a.channel.id,
            "rule": { "match": "New", "directory": "New/Season 01", "episode": 1 },
        }))
        .await;
    assert_eq!(kinds(&new), [("New - 01".to_owned(), "mine".to_owned())]);
    assert_eq!(new["items"][0]["save_path"], "/media/New/Season 01");

    // A rule waiting for its title matches nothing.
    let waiting = app
        .preview(json!({
            "channel_id": a.channel.id,
            "rule": { "match": null, "episode": 1 },
        }))
        .await;
    assert_eq!(kinds(&waiting), []);

    // Archived, a rule is judged as if restored: what history holds that it has
    // not taken came while it was off, so it would leave that to the user.
    let rule = app.list().await["rules"][0].clone();
    app.state
        .channels
        .set_rule_state(rule["id"].as_str().unwrap(), RuleState::Archived, 0)
        .await
        .unwrap();
    let archived = app.list().await["rules"][0].clone();
    assert_eq!(archived["state"], "archived");
    let restored = app
        .preview(json!({
            "channel_id": a.channel.id,
            "rule_id": archived["id"],
            "rule": edit(&archived, json!({})),
        }))
        .await;
    assert_eq!(
        kinds(&restored),
        [("Show - 01".to_owned(), "past".to_owned())]
    );
    assert_eq!(restored["items"][0]["past_cause"], "resumed");

    let (status, _, _) = app
        .call(
            Method::POST,
            "/api/rules/preview",
            Some(json!({
                "channel_id": a.channel.id,
                "rule_id": "nope",
                "rule": { "match": "x", "episode": 1 },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_items_the_rule_takes_say_the_episode_they_are_received_as() {
    let app = App::new().await;
    let a = app
        .channel("a.test", &[], &[("Other", "Other/Season 01")])
        .await;
    app.record(
        &a.channel,
        1_000,
        &[
            "[SubsPlease] Show - 24 (1080p)",
            "[SubsPlease] Show - 25 (1080p)",
            "[SubsPlease] Show (01-12) (1080p) [Batch]",
            "[SubsPlease] Other - 03 (1080p)",
        ],
    )
    .await;
    let named = |preview: &Value| {
        let mut named: Vec<(String, Value, Value)> = preview["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                (
                    i["title"].as_str().unwrap().to_owned(),
                    i["release"].clone(),
                    i["episode_name"].clone(),
                )
            })
            .collect();
        named.sort_by(|a, b| a.0.cmp(&b.0));
        named
    };
    let row = |title: &str, release: Value, name: Value| (title.to_owned(), release, name);
    let preview = |episode: i64| {
        app.preview(json!({
            "channel_id": a.channel.id,
            "rule": { "match": "SubsPlease", "directory": "Show/Season 02", "episode": episode },
        }))
    };

    let plain = preview(1).await;
    let converted = preview(-12).await;

    // An item another rule takes, or a batch, names no episode of this rule.
    assert_eq!(
        named(&plain),
        [
            row("[SubsPlease] Other - 03 (1080p)", Value::Null, Value::Null),
            row(
                "[SubsPlease] Show (01-12) (1080p) [Batch]",
                Value::Null,
                Value::Null
            ),
            row("[SubsPlease] Show - 24 (1080p)", json!(24), json!("S02E24")),
            row("[SubsPlease] Show - 25 (1080p)", json!(25), json!("S02E25")),
        ]
    );
    assert_eq!(
        named(&converted)[2..],
        [
            row("[SubsPlease] Show - 24 (1080p)", json!(24), json!("S02E12")),
            row("[SubsPlease] Show - 25 (1080p)", json!(25), json!("S02E13")),
        ]
    );
    // Only a preview asked for a subscription about to be made carries an offer.
    assert!(plain.get("episode_suggestion").is_none());
}

#[tokio::test]
async fn titles_with_the_secret_mask_are_flagged() {
    let app = App::new().await;
    let a = app.channel("a.test", &[], &[("Show", "first")]).await;
    app.record(&a.channel, 1_000, &["Show - 01", "Show *** search"])
        .await;
    let rule = app.list().await["rules"][0].clone();
    let preview = app
        .preview(json!({
            "channel_id": a.channel.id,
            "rule_id": rule["id"],
            "rule": edit(&rule, json!({})),
        }))
        .await;
    assert_eq!(preview["masked_total"], 1);
    for item in preview["items"].as_array().unwrap() {
        assert_eq!(
            item["masked"],
            item["title"].as_str().unwrap().contains("***")
        );
    }
}

#[tokio::test]
async fn a_long_preview_lists_the_newest_matches_and_counts_all_of_them() {
    let app = App::new().await;
    let a = app.channel("a.test", &[], &[("Show", "first")]).await;
    let titles: Vec<String> = (0..130).map(|n| format!("Show - {n:03}")).collect();
    let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
    for (i, chunk) in refs.chunks(10).enumerate() {
        app.record(&a.channel, 1_000 + i as i64, chunk).await;
    }
    let rule = app.list().await["rules"][0].clone();
    let preview = app
        .preview(json!({
            "channel_id": a.channel.id,
            "rule_id": rule["id"],
            "rule": edit(&rule, json!({})),
        }))
        .await;
    assert_eq!(preview["counts"]["mine"], 130);
    assert_eq!(
        preview["items"].as_array().unwrap().len(),
        PREVIEW_LIST_LIMIT
    );
    assert_eq!(preview["truncated"], true);
}

#[tokio::test]
async fn the_preview_agrees_with_the_worker_mapping_for_every_recorded_title() {
    // The preview and the worker judge through `ChannelPlan`. This pins that the
    // preview's per-item answer is that plan's answer for the same items and
    // settings, including archived rules, regexes and excludes.
    let app = App::new().await;
    let mut input = ChannelInput::new("https://a.test/rss?token=SECRETVALUE99");
    input.excludes = vec!["[Batch]".into(), "(720p)".into()];
    let rules = vec![
        RuleInput {
            r#match: Some("Sono Bisque Doll".into()),
            case_insensitive: true,
            directory: "Doll/Season 02".into(),
            episode: -12,
            ..RuleInput::default()
        },
        RuleInput {
            r#match: Some(r"^\[SubsPlease\] (Slime|Lara) - \d+".into()),
            regex: true,
            directory: "Regex".into(),
            ..RuleInput::default()
        },
        RuleInput {
            r#match: Some("Archived Show".into()),
            directory: "Archived".into(),
            state: RuleState::Archived,
            ..RuleInput::default()
        },
        RuleInput {
            r#match: None,
            directory: "Waiting".into(),
            ..RuleInput::default()
        },
        RuleInput {
            r#match: Some("[SubsPlease]".into()),
            directory: "Catch all".into(),
            ..RuleInput::default()
        },
    ];
    let cwr = app
        .state
        .channels
        .create_channel_with_rules(input, rules)
        .await
        .unwrap();
    let titles = [
        "[SubsPlease] Sono Bisque Doll - 13 (1080p)",
        "sono bisque doll - 14",
        "[SubsPlease] Slime - 62 (1080p)",
        "[SubsPlease] Lara - 03 (720p)",
        "[SubsPlease] Sono Bisque Doll - 01~12 [Batch]",
        "[SubsPlease] Archived Show - 01",
        "[Erai-raws] Unrelated - 05",
        "Slime - 62",
        "",
    ];
    app.record(&cwr.channel, 1_000, &titles).await;

    // Edit the last rule (the catch-all) without changing anything, and compare
    // its preview with the plan's answer for each item.
    let last = &cwr.rules[4];
    let items = app
        .state
        .history
        .list(Default::default())
        .await
        .unwrap()
        .items;
    let collect = std::path::Path::new("/media");
    let preview = build_preview(
        collect,
        &cwr,
        Some(&last.id),
        &last.to_input(),
        None,
        &items,
    )
    .unwrap();
    let plan = ChannelPlan::new(cwr.clone(), collect);
    let mut expected_mine = 0;
    let mut expected_earlier = 0;
    for item in &items {
        let evaluation = plan.evaluate(&item.title);
        let listed = preview.items.iter().find(|p| p.id == item.id);
        match (&evaluation.judgement, listed) {
            (
                Judgement::Selected {
                    rule_id, save_path, ..
                },
                Some(p),
            ) if rule_id == &last.id => {
                assert_eq!(p.kind, Kind::Mine, "{}", item.title);
                assert_eq!(p.save_path.as_deref(), Some(save_path.to_str().unwrap()));
                expected_mine += 1;
            }
            (
                Judgement::Selected {
                    rule_id, save_path, ..
                },
                Some(p),
            ) => {
                assert_eq!(p.kind, Kind::Earlier, "{}", item.title);
                assert_eq!(
                    p.taken_by.as_ref().unwrap().rule_id.as_deref(),
                    Some(rule_id.as_str())
                );
                assert_eq!(p.save_path.as_deref(), Some(save_path.to_str().unwrap()));
                assert!(evaluation.overlapping.contains(&last.id));
                expected_earlier += 1;
            }
            (Judgement::Selected { .. }, None) => {
                assert!(!evaluation.overlapping.contains(&last.id), "{}", item.title);
            }
            (Judgement::Excluded, Some(p)) => assert_eq!(p.kind, Kind::Excluded, "{}", item.title),
            (Judgement::Excluded, None) | (Judgement::NoMatch, None) => {}
            (Judgement::NoMatch, Some(p)) => {
                panic!("{} listed as {:?} but nothing matches", item.title, p.kind)
            }
        }
    }
    assert_eq!(preview.counts.mine, expected_mine);
    assert_eq!(preview.counts.earlier, expected_earlier);
    assert!(expected_mine > 0 && expected_earlier > 0);
}

// --- archive and restore --------------------------------------------------------

#[tokio::test]
async fn the_state_is_not_saved_by_an_edit_and_the_last_archive_move_is_shown() {
    use trss_collect::commands::rule_archive::{Direction, RuleArchive, KIND};
    use trss_core::commands::{CommandState, NewCommand, Outcome};

    let app = App::new().await;
    let a = app
        .channel("a.test", &[], &[("Show", "Show/Season 01")])
        .await;
    let rule = app.list().await["rules"][0].clone();
    let id = rule["id"].as_str().unwrap().to_owned();
    assert_eq!(rule["archive_move"], Value::Null);

    // An edit cannot archive: only the worker does, after the command.
    let (status, text, body) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{id}"),
            Some(rule_body(&a.channel, &rule, json!({ "state": "archived" }))),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert!(
        body["message"].as_str().unwrap().contains("`보관`"),
        "{text}"
    );
    let unchanged = app.list().await["rules"][0].clone();
    assert_eq!(unchanged["state"], "active");
    assert_eq!(unchanged["version"], rule["version"]);

    // An edit that keeps the state still saves.
    let (status, text, _) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{id}"),
            Some(rule_body(&a.channel, &rule, json!({ "episode": 3 }))),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");

    // The rule shows its last archive command, open or ended.
    let payload = RuleArchive {
        rule_id: id.clone(),
        direction: Direction::Archive,
        receive: Vec::new(),
    };
    let new = |cid: &str| NewCommand {
        id: cid.to_owned(),
        kind: KIND.to_owned(),
        payload: payload.canonical(),
        subject: Some(id.clone()),
    };
    app.state
        .commands
        .accept(new("cmd-first-1"), 1)
        .await
        .unwrap();
    app.state
        .commands
        .finish(
            "cmd-first-1",
            CommandState::Failed,
            Outcome {
                result: "failed".into(),
                reason: Some("겹쳐요".into()),
            },
            2,
        )
        .await
        .unwrap();
    app.state
        .commands
        .accept(new("cmd-second"), 3)
        .await
        .unwrap();

    let (status, text, one) = app
        .call(Method::GET, &format!("/api/rules/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(one["archive_move"]["direction"], "archive");
    assert_eq!(one["archive_move"]["command"]["id"], "cmd-second");
    assert_eq!(one["archive_move"]["command"]["state"], "pending");
    assert_eq!(
        app.list().await["rules"][0]["archive_move"],
        one["archive_move"]
    );

    let (status, _, _) = app.call(Method::GET, "/api/rules/no-such-rule", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_save_folder_with_parent_components_is_refused_but_a_stored_one_still_saves() {
    let app = App::new().await;
    let a = app
        .channel("a.test", &[], &[("Old", "Old/../Other/Season 01")])
        .await;

    let (status, text, error) = app
        .call(
            Method::POST,
            "/api/rules",
            Some(json!({
                "channel_id": a.channel.id,
                "match": "New",
                "directory": "New/../../etc",
                "episode": 0,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert!(
        error["message"].as_str().unwrap().contains("`..`"),
        "{text}"
    );
    assert_eq!(app.list().await["rules"].as_array().unwrap().len(), 1);

    // The rule stored before keeps saving other changes with its folder...
    let rule = app.list().await["rules"][0].clone();
    let uri = format!("/api/rules/{}", rule["id"].as_str().unwrap());
    let (status, text, saved) = app
        .call(
            Method::PUT,
            &uri,
            Some(rule_body(&a.channel, &rule, json!({ "episode": 2 }))),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(saved["directory"], "Old/../Other/Season 01");

    // ...but its folder cannot be changed to another one with `..`.
    let (status, text, _) = app
        .call(
            Method::PUT,
            &uri,
            Some(rule_body(
                &a.channel,
                &saved,
                json!({ "directory": "Old/../Else" }),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
}

#[tokio::test]
async fn while_a_work_folder_moves_no_rule_changes_its_folder_into_or_out_of_it() {
    use trss_collect::commands::rule_archive::{Direction, RuleArchive, KIND};
    use trss_core::commands::{CommandState, NewCommand, Outcome};

    let app = App::new().await;
    let a = app
        .channel(
            "a.test",
            &[],
            &[
                ("Clevatess", "Clevatess/Season 02"),
                ("Other", "Other/Season 01"),
            ],
        )
        .await;
    let list = app.list().await;
    let moving = list["rules"][0].clone();
    let other = list["rules"][1].clone();
    let moving_id = moving["id"].as_str().unwrap().to_owned();
    let payload = RuleArchive {
        rule_id: moving_id.clone(),
        direction: Direction::Archive,
        receive: Vec::new(),
    };
    app.state
        .commands
        .accept(
            NewCommand {
                id: "cmd-moving-1".into(),
                kind: KIND.into(),
                payload: payload.canonical(),
                subject: Some(moving_id.clone()),
            },
            1,
        )
        .await
        .unwrap();

    // The moving rule keeps its folder; other fields still save.
    let (status, text, error) = put(
        &app,
        &a.channel,
        &moving,
        json!({ "directory": "Clevatess/Season 03" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert!(
        error["message"].as_str().unwrap().contains("옮기는 중"),
        "{text}"
    );
    let (status, text, moving) = put(&app, &a.channel, &moving, json!({ "episode": 4 })).await;
    assert_eq!(status, StatusCode::OK, "{text}");

    // No rule moves into the moving work folder, nor is one made there.
    let (status, text, _) = put(
        &app,
        &a.channel,
        &other,
        json!({ "directory": "Clevatess/Season 03" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    let (status, text, _) = app
        .call(
            Method::POST,
            "/api/rules",
            Some(json!({
                "channel_id": a.channel.id,
                "match": "Clevatess S3",
                "directory": "Clevatess/Season 03",
                "episode": 0,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    // Elsewhere is fine.
    let (status, text, _) = put(
        &app,
        &a.channel,
        &other,
        json!({ "directory": "Other/Season 02" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");

    // Once the move has ended, the folder can change again.
    app.state
        .commands
        .finish(
            "cmd-moving-1",
            CommandState::Done,
            Outcome {
                result: "moved".into(),
                reason: None,
            },
            2,
        )
        .await
        .unwrap();
    let (status, text, _) = put(
        &app,
        &a.channel,
        &moving,
        json!({ "directory": "Clevatess/Season 03" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
}

async fn put(
    app: &App,
    channel: &Channel,
    rule: &Value,
    patch: Value,
) -> (StatusCode, String, Value) {
    let uri = format!("/api/rules/{}", rule["id"].as_str().unwrap());
    app.call(Method::PUT, &uri, Some(rule_body(channel, rule, patch)))
        .await
}

#[tokio::test]
async fn an_edit_cannot_pause_a_rule_but_a_paused_rule_still_saves_its_fields() {
    let app = App::new().await;
    let a = app
        .channel("a.test", &[], &[("Show", "Show/Season 01")])
        .await;
    let rule = app.list().await["rules"][0].clone();
    let id = rule["id"].as_str().unwrap().to_owned();

    // The switch pauses; an edit that carries another state is refused.
    let (status, text, _) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{id}"),
            Some(rule_body(&a.channel, &rule, json!({ "state": "paused" }))),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");

    let (status, text, paused) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{id}/switch"),
            Some(json!({ "version": rule["version"], "video": false })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(paused["state"], "paused");

    // A paused rule saves its fields with the state it has.
    let (status, text, saved) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{id}"),
            Some(rule_body(
                &a.channel,
                &paused,
                json!({ "episode": 5, "state": "paused" }),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(saved["state"], "paused");
    assert_eq!(saved["episode"], 5);
}

// --- a rule that starts collecting while its work is in the archive folder (0123) ---------------

/// An app whose collect and archive folders are real temporary folders and
/// whose library records `works` in the archive folder.
async fn app_with_archive(works: &[&str]) -> (App, tempfile::TempDir) {
    use std::collections::BTreeSet;
    use trss_library::discovery::{EpisodeFile, FileKind, Scan, ScannedWork, WorkRead};

    let tmp = tempfile::tempdir().unwrap();
    let collect = tmp.path().join("Shows (current)");
    let archive = tmp.path().join("Shows");
    std::fs::create_dir(&collect).unwrap();
    for work in works {
        std::fs::create_dir_all(archive.join(work).join("Season 01")).unwrap();
    }
    let app = App::new().await;
    app.state
        .settings
        .put_collection(
            1,
            collect.display().to_string(),
            Some(archive.display().to_string()),
        )
        .await
        .unwrap();
    let scan = Scan {
        works: works
            .iter()
            .map(|name| {
                WorkRead::Read(ScannedWork {
                    dir_name: (*name).to_owned(),
                    seasons: BTreeSet::from([1]),
                    files: vec![EpisodeFile {
                        path: format!("Season 01/{name} S01E01.mkv"),
                        kind: FileKind::Video,
                        season: 1,
                        episode: "01".to_owned(),
                    }],
                    unrecognized: Vec::new(),
                })
            })
            .collect(),
    };
    app.state
        .library
        .add_folder(archive.display().to_string(), scan, 100, &[])
        .await
        .unwrap();
    (app, tmp)
}

#[tokio::test]
async fn a_rule_for_a_work_in_the_archive_folder_is_made_paused_with_its_start_open() {
    let (app, tmp) = app_with_archive(&["Clevatess"]).await;
    let a = app.channel("a.test", &[], &[]).await;
    let new_rule = |directory: &str, state: Option<&str>| {
        let mut body = json!({
            "channel_id": a.channel.id,
            "match": "Clevatess",
            "directory": directory,
            "episode": 1,
        });
        if let Some(state) = state {
            body["state"] = json!(state);
        }
        body
    };

    // The archive folder holds the work: the rule is made off, with the start open.
    let (status, text, rule) = app
        .call(
            Method::POST,
            "/api/rules",
            Some(new_rule("Clevatess/Season 03", None)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{text}");
    assert_eq!(rule["state"], "paused");
    assert_eq!(rule["archive_move"]["direction"], "start");
    assert_eq!(rule["archive_move"]["command"]["kind"], "rule_archive");
    assert_eq!(rule["archive_move"]["command"]["state"], "pending");
    assert_eq!(rule["archive_move"]["command"]["outcome"], Value::Null);

    // Another rule in the work folder being moved is refused until it is done.
    let (status, _, _) = app
        .call(
            Method::POST,
            "/api/rules",
            Some(new_rule("Clevatess/Season 04", None)),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A rule made paused on purpose, or for a work the archive folder lacks, is
    // stored as asked, with no move.
    let (status, _, paused) = app
        .call(
            Method::POST,
            "/api/rules",
            Some(new_rule("Elsewhere/Season 01", Some("paused"))),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(paused["state"], "paused");
    assert_eq!(paused["archive_move"], Value::Null);
    let (status, _, fresh) = app
        .call(
            Method::POST,
            "/api/rules",
            Some(new_rule("Elsewhere/Season 02", None)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(fresh["state"], "active");
    assert_eq!(fresh["archive_move"], Value::Null);
    drop(tmp);
}

#[tokio::test]
async fn turning_video_on_for_a_work_in_the_archive_folder_leaves_the_rule_paused_with_its_resume_open(
) {
    let (app, tmp) = app_with_archive(&["Clevatess"]).await;
    let a = app
        .channel(
            "a.test",
            &[],
            &[
                ("Clevatess", "Clevatess/Season 02"),
                ("Other", "Other/Season 01"),
            ],
        )
        .await;
    for rule in &a.rules {
        app.state
            .channels
            .set_rule_state(&rule.id, RuleState::Paused, 0)
            .await
            .unwrap();
    }
    let switch = |rule: &Value| {
        let (id, version) = (
            rule["id"].as_str().unwrap().to_owned(),
            rule["version"].clone(),
        );
        let app = &app;
        async move {
            app.call(
                Method::PUT,
                &format!("/api/rules/{id}/switch"),
                Some(json!({ "version": version, "video": true })),
            )
            .await
        }
    };
    let read = |id: &str| {
        let id = id.to_owned();
        let app = &app;
        async move {
            app.call(Method::GET, &format!("/api/rules/{id}"), None)
                .await
                .2
        }
    };

    let (status, text, held) = switch(&read(&a.rules[0].id).await).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(held["state"], "paused");
    assert_eq!(held["archive_move"]["direction"], "resume");
    assert_eq!(held["archive_move"]["command"]["state"], "pending");

    // Pressed again while the move is open: refused, nothing stored.
    let (status, text, refused) = switch(&read(&a.rules[0].id).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert_eq!(refused["error"], "invalid");
    assert!(
        refused["message"].as_str().unwrap().contains("옮기는 중"),
        "{text}"
    );

    // A work that is not in the archive folder turns on at once.
    let (status, text, on) = switch(&read(&a.rules[1].id).await).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(on["state"], "active");
    assert_eq!(on["archive_move"], Value::Null);
    drop(tmp);
}

#[tokio::test]
async fn the_form_is_told_which_work_in_the_archive_folder_it_would_bring_over() {
    let (app, tmp) = app_with_archive(&["Clevatess"]).await;
    let (status, text, told) = app
        .call(
            Method::GET,
            "/api/rules/archived-work?directory=Clevatess%2FSeason%2003",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let archived = &told["archived"];
    assert_eq!(archived["work"], "Clevatess");
    assert_eq!(archived["merges"], false);
    assert!(archived["archive_folder"]
        .as_str()
        .unwrap()
        .ends_with("/Shows"));
    assert!(archived["collect_folder"]
        .as_str()
        .unwrap()
        .ends_with("/Shows (current)"));

    for directory in ["Other%2FSeason%2001", ""] {
        let (status, text, told) = app
            .call(
                Method::GET,
                &format!("/api/rules/archived-work?directory={directory}"),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert_eq!(told, json!({ "archived": null }));
    }
    drop(tmp);
}

#[tokio::test]
async fn a_browser_cannot_post_a_start_or_a_resume() {
    let (app, tmp) = app_with_archive(&[]).await;
    let a = app
        .channel("a.test", &[], &[("Clevatess", "Clevatess/Season 02")])
        .await;
    app.state
        .channels
        .set_rule_state(&a.rules[0].id, RuleState::Paused, 0)
        .await
        .unwrap();
    // Even for a paused rule: only the web, after it saved the rule paused for
    // the move, makes them, so a posted one could not turn a rule on early.
    for (id, direction) in [("start-1", "start"), ("resume-1", "resume")] {
        let (status, text, body) = app
            .call(
                Method::POST,
                "/api/commands",
                Some(json!({
                    "id": id,
                    "kind": "rule_archive",
                    "payload": { "rule_id": a.rules[0].id, "direction": direction },
                })),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
        assert_eq!(body["error"], "invalid");
    }
    // Nothing was stored; archive and restore are the browser's.
    assert!(!app.state.commands.has_open().await.unwrap());
    let (status, text, _) = app
        .call(
            Method::POST,
            "/api/commands",
            Some(json!({
                "id": "archive-1",
                "kind": "rule_archive",
                "payload": { "rule_id": a.rules[0].id, "direction": "archive" },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    drop(tmp);
}

#[tokio::test]
async fn editing_a_collecting_rule_into_an_archived_work_folder_saves_it_paused_with_its_start_open(
) {
    let (app, tmp) = app_with_archive(&["Clevatess"]).await;
    let a = app
        .channel(
            "a.test",
            &[],
            &[
                ("Mushoku", "Mushoku/Season 01"),
                ("Other", "Other/Season 01"),
            ],
        )
        .await;
    // Collecting since 777: a `start` must not move that time.
    let id = a.rules[0].id.clone();
    app.state
        .channels
        .set_rule_state(&id, RuleState::Paused, 700)
        .await
        .unwrap();
    app.state
        .channels
        .set_rule_state(&id, RuleState::Active, 777)
        .await
        .unwrap();
    let rule = app.list().await["rules"][0].clone();
    assert_eq!(rule["state"], "active");

    // The same work folder: nothing moves, the rule keeps collecting.
    let (status, text, same) = put(
        &app,
        &a.channel,
        &rule,
        json!({ "directory": "Mushoku/Season 02" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(same["state"], "active");
    assert_eq!(same["archive_move"], Value::Null);

    // Into a work folder the archive folder holds: saved, but paused, with the
    // start open, so no cycle makes `Clevatess` in the collect folder first.
    let (status, text, held) = put(
        &app,
        &a.channel,
        &same,
        json!({ "directory": "Clevatess/Season 03" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(held["directory"], "Clevatess/Season 03");
    assert_eq!(held["state"], "paused");
    assert_eq!(held["archive_move"]["direction"], "start");
    assert_eq!(held["archive_move"]["command"]["state"], "pending");
    let stored = app.state.channels.get_rule(&id).await.unwrap().unwrap();
    assert_eq!(stored.state, RuleState::Paused);
    assert_eq!(stored.resumed_at, Some(777));

    // A rule that goes to a work the archive lacks is saved as before.
    let other = app.list().await["rules"][1].clone();
    let (status, text, moved) = put(
        &app,
        &a.channel,
        &other,
        json!({ "directory": "Elsewhere/Season 01" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(moved["state"], "active");
    assert_eq!(moved["archive_move"], Value::Null);
    drop(tmp);
}

#[tokio::test]
async fn a_failed_start_is_retried_as_a_start_and_a_move_open_refuses_off_and_two_switches() {
    use trss_core::commands::{CommandState, Outcome};

    let (app, tmp) = app_with_archive(&["Clevatess"]).await;
    let a = app.channel("a.test", &[], &[]).await;
    let (status, text, made) = app
        .call(
            Method::POST,
            "/api/rules",
            Some(json!({
                "channel_id": a.channel.id,
                "match": "Clevatess",
                "directory": "Clevatess/Season 03",
                "episode": 1,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{text}");
    let id = made["id"].as_str().unwrap().to_owned();
    let switch = |body: Value| {
        let app = &app;
        let id = id.clone();
        async move {
            let rule = app
                .call(Method::GET, &format!("/api/rules/{id}"), None)
                .await
                .2;
            let mut body = body;
            body["version"] = rule["version"].clone();
            app.call(Method::PUT, &format!("/api/rules/{id}/switch"), Some(body))
                .await
        }
    };

    // While the start is open, off changes nothing the worker would honor: the
    // worker turns the rule on after the move. Two switches at once are no
    // request, whatever the rule is doing.
    let (status, text, _) = switch(json!({ "video": false })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    let (status, text, bad) = switch(json!({ "video": true, "subtitles": true })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert!(
        bad["message"].as_str().unwrap().contains("요청 내용"),
        "{text}"
    );
    let (status, text, _) = switch(json!({ "video": true })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");

    // The start fails (an overlapping file, say): the rule stays off.
    let open = app
        .state
        .commands
        .claim_next(2_000)
        .await
        .unwrap()
        .expect("the start is waiting");
    app.state
        .commands
        .finish(
            &open.id,
            CommandState::Failed,
            Outcome {
                result: "failed".into(),
                reason: Some("겹치는 파일이 있어요".into()),
            },
            2_001,
        )
        .await
        .unwrap();
    // Off on a paused rule with nothing open is the no-op it always was.
    let (status, text, off) = switch(json!({ "video": false })).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(off["state"], "paused");

    // The retry is a start again: the rule never collected, so the time it
    // waited does not make what arrived meanwhile a past item.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let (status, text, retry) = switch(json!({ "video": true })).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(retry["state"], "paused");
    assert_eq!(retry["archive_move"]["direction"], "start");
    assert_eq!(retry["archive_move"]["command"]["state"], "pending");
    assert_ne!(retry["archive_move"]["command"]["id"], json!(open.id));
    drop(tmp);
}

#[tokio::test]
async fn the_edit_form_asks_whether_the_changed_folder_moves_a_work_over() {
    let (app, tmp) = app_with_archive(&["Clevatess"]).await;
    let ask = |query: &str| {
        let uri = format!("/api/rules/archived-work?{query}");
        let app = &app;
        async move { app.call(Method::GET, &uri, None).await }
    };
    // As typed: the server finds the work folder, whatever the spelling.
    for directory in [
        "Clevatess%2FSeason%2003",
        ".%2FClevatess%2FSeason%2003",
        "Clevatess%2F",
    ] {
        let (status, text, told) = ask(&format!("directory={directory}")).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert_eq!(told["archived"]["work"], "Clevatess", "{directory}: {text}");
    }
    // `.` is the collect folder itself.
    let (_, _, told) = ask("directory=.").await;
    assert_eq!(told, json!({ "archived": null }));
    // A stored rule that stays in the work folder moves nothing.
    let (_, _, told) = ask("directory=Clevatess%2FSeason%2004&from=Clevatess%2FSeason%2003").await;
    assert_eq!(told, json!({ "archived": null }));
    let (_, _, told) = ask("directory=Clevatess%2FSeason%2004&from=Other%2FSeason%2001").await;
    assert_eq!(told["archived"]["work"], "Clevatess");
    drop(tmp);
}

// --- the episode offset in the rule detail (ticket 0024) ------------------------
//
// What the cycle decides is tested in `trss-collect`'s `offsets`; here are the
// rule detail's grounds and suggestions read from the stores, and `적용`.

mod episode_offset {
    use std::collections::BTreeSet;

    use super::*;
    use trss_anilist::{Entry, FuzzyDate};
    use trss_anissia::Anime;
    use trss_collect::store::channels::{NewSubscription, Rule, SubtitleMode};
    use trss_library::discovery::{EpisodeFile, FileKind, Scan, ScannedWork, WorkRead};

    fn entry(id: i64, episodes: Option<u32>) -> Entry {
        Entry {
            id,
            romaji: None,
            english: None,
            native: None,
            format: None,
            status: None,
            episodes,
            start: FuzzyDate::default(),
            end: FuzzyDate::default(),
            studios: Vec::new(),
            genres: Vec::new(),
            description: None,
            airing: Vec::new(),
            korean_titles: Vec::new(),
            sequels: Vec::new(),
            fetched_at: 1,
        }
    }

    /// `Show` in the library of the collect folder `/media`, with these
    /// seasons and the videos named in each; AniList entries with these
    /// episode counts are linked to the seasons before the third.
    async fn library(app: &App, seasons: &[(u32, &[&str])], counts: &[Option<u32>]) {
        let files = seasons
            .iter()
            .flat_map(|(season, episodes)| {
                episodes.iter().map(move |episode| EpisodeFile {
                    path: format!("Season {season:02}/Show S{season:02}E{episode}.mkv"),
                    kind: FileKind::Video,
                    season: *season,
                    episode: (*episode).to_owned(),
                })
            })
            .collect();
        let scan = Scan {
            works: vec![WorkRead::Read(ScannedWork {
                dir_name: "Show".into(),
                seasons: seasons.iter().map(|(s, _)| *s).collect::<BTreeSet<_>>(),
                files,
                unrecognized: Vec::new(),
            })],
        };
        let (folder, _) = app
            .state
            .library
            .add_folder("/media".into(), scan, 100, &[])
            .await
            .unwrap();
        let work = app
            .state
            .library
            .works(&folder.id)
            .await
            .unwrap()
            .remove(0)
            .id;
        for (index, count) in counts.iter().enumerate() {
            let season = index as u32 + 1;
            let id = 100 + i64::from(season);
            app.state
                .seasons
                .store
                .put_entry(entry(id, *count))
                .await
                .unwrap();
            let link = app.state.seasons.store.link(&work, season).await.unwrap();
            app.state
                .seasons
                .store
                .set_links(&work, season, link.version, vec![id])
                .await
                .unwrap();
        }
    }

    /// A subscription of the channel to the anime `no` saving into
    /// `directory` with the offset `episode` in its field.
    async fn subscription(
        app: &App,
        channel: &Channel,
        no: i64,
        directory: &str,
        episode: i64,
    ) -> Rule {
        app.state
            .channels
            .create_subscription_rule(
                &channel.id,
                RuleInput {
                    r#match: Some("Show".to_owned()),
                    directory: directory.to_owned(),
                    episode,
                    ..RuleInput::default()
                },
                NewSubscription {
                    anime: Anime {
                        anime_no: no,
                        subject: "Show".to_owned(),
                        original_subject: None,
                        week: 4,
                        air_time: Some("23:00".to_owned()),
                        start_date: Some("2026-10-08".to_owned()),
                        end_date: None,
                        status: "ON".to_owned(),
                        fetched_at: 1,
                    },
                    subtitles: SubtitleMode::Undecided,
                    creator: None,
                    subscribed_at: 1,
                },
            )
            .await
            .unwrap()
    }

    async fn view(app: &App, rule: &Rule) -> Value {
        let (status, text, view) = app
            .call(Method::GET, &format!("/api/rules/{}", rule.id), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        view
    }

    /// The rule picked `title` as its first item.
    async fn picked(app: &App, channel: &Channel, rule: &Rule, title: &str) {
        app.record_as(
            channel,
            2_000,
            &[title],
            HistoryResult::Received,
            Some(&rule.id),
        )
        .await;
    }

    const BASIS: &str =
        "AniList 기준 이전 시즌이 24화까지이고 첫 화가 25화라서 회차 변환을 −24로 정했어요.";

    #[tokio::test]
    async fn the_view_of_an_automatic_offset_says_its_grounds_and_what_it_replaced() {
        let app = App::new().await;
        let a = app.channel("a.test", &[], &[]).await;
        let rule = subscription(&app, &a.channel, 7, "Show/Season 03", 1).await;
        app.state
            .channels
            .set_auto_episode(&rule.id, rule.version, -24, BASIS)
            .await
            .unwrap()
            .expect("the rule was at the version read");

        let view = view(&app, &rule).await;

        assert_eq!(view["episode"], -24);
        assert_eq!(view["episode_auto"], true);
        // A value that left numbers as they were is said so.
        let basis = view["episode_basis"].as_str().unwrap();
        assert!(basis.contains("24화") && basis.contains("25화"), "{basis}");
        assert!(basis.ends_with("(전에는 변환 없음)."), "{basis}");
        assert_eq!(view["episode_previous"], 1);
        assert_eq!(view["episode_suggestion"], Value::Null);

        // A value carried over from the previous season is said too.
        let carried = subscription(&app, &a.channel, 8, "Show/Season 03", -24).await;
        app.state
            .channels
            .set_auto_episode(&carried.id, carried.version, -48, BASIS)
            .await
            .unwrap()
            .expect("the rule was at the version read");
        let view = self::view(&app, &carried).await;
        assert_eq!(view["episode_previous"], -24);
        assert!(view["episode_basis"]
            .as_str()
            .unwrap()
            .ends_with("(전에는 −24)."));
    }

    #[tokio::test]
    async fn an_automatic_value_the_user_changes_loses_its_mark_and_its_grounds() {
        let app = App::new().await;
        let a = app.channel("a.test", &[], &[]).await;
        let rule = subscription(&app, &a.channel, 7, "Show/Season 03", 1).await;
        app.state
            .channels
            .set_auto_episode(&rule.id, rule.version, -24, BASIS)
            .await
            .unwrap()
            .expect("the rule was at the version read");
        let auto = view(&app, &rule).await;
        let uri = format!("/api/rules/{}", rule.id);

        // The rule detail saves the whole rule. Saving the value as it is
        // keeps the mark and the grounds ...
        let (status, _, kept) = app
            .call(
                Method::PUT,
                &uri,
                Some(rule_body(
                    &a.channel,
                    &auto,
                    json!({ "directory": "Show/Season 03" }),
                )),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{kept}");
        assert_eq!(kept["episode_auto"], true);
        assert_eq!(kept["episode_basis"], auto["episode_basis"]);

        // ... and a changed value is the user's.
        let (status, _, saved) = app
            .call(
                Method::PUT,
                &uri,
                Some(rule_body(&a.channel, &kept, json!({ "episode": -12 }))),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["episode"], -12);
        assert_eq!(saved["episode_auto"], false);
        assert_eq!(saved["episode_basis"], Value::Null);
        assert_eq!(saved["episode_previous"], Value::Null);
    }

    /// What the rule detail offers.
    enum Offer {
        Nothing,
        /// A suggestion with this value (`None`: the grounds say none and the
        /// user has to write it) and these pieces of its sentence.
        Suggests(Option<i64>, &'static [&'static str]),
    }

    /// A rule that picked its first item is offered what the earlier seasons
    /// give, or told why it is not (`{ value, basis }`), whatever its field
    /// holds once the value differs from it.
    #[tokio::test]
    async fn a_rule_that_picked_its_first_item_is_offered_what_the_earlier_seasons_give() {
        let both: &[(u32, &[&str])] = &[(1, &["01", "02"]), (2, &["01", "02"])];
        let with_third: &[(u32, &[&str])] =
            &[(1, &["01", "02"]), (2, &["01", "02"]), (3, &["01", "02"])];
        // (name, library, counts of seasons 1 and 2, field, first release, offer)
        let cases = [
            (
                "a_first_release_in_the_middle_of_a_season_is_suggested",
                both,
                [Some(12), Some(12)],
                1,
                27,
                Offer::Suggests(Some(-24), &["27화", "24화"]),
            ),
            (
                "earlier_seasons_without_a_known_count_are_never_guessed",
                both,
                [Some(12), None],
                1,
                25,
                Offer::Suggests(None, &["시즌 2의 AniList 회차 수"]),
            ),
            (
                "a_season_folder_that_has_videos_already_has_no_value_to_offer",
                with_third,
                [Some(12), Some(12)],
                1,
                25,
                Offer::Suggests(None, &["1–2화"]),
            ),
            (
                "numbers_run_on_from_the_season_before_are_offered",
                both,
                [Some(24), Some(24)],
                0,
                25,
                Offer::Suggests(
                    Some(-24),
                    &["2기부터 이어 센 번호로 보여요. 회차 변환을 −24로 할까요?"],
                ),
            ),
            // A value that differs from the field is offered whatever the
            // field holds (user direction, 2026-10-02) ...
            (
                "a_rule_that_started_with_a_carried_over_value_is_offered_the_sum",
                both,
                [Some(12), Some(12)],
                -12,
                25,
                Offer::Suggests(Some(-24), &["25화"]),
            ),
            // ... and a field that already holds it is offered nothing.
            (
                "a_rule_whose_field_already_holds_the_suggestion_is_offered_nothing",
                both,
                [Some(12), Some(12)],
                -24,
                25,
                Offer::Nothing,
            ),
        ];
        for (name, seasons, counts, field, first, offer) in cases {
            let app = App::new().await;
            library(&app, seasons, &counts).await;
            let a = app.channel("a.test", &[], &[]).await;
            let rule = subscription(&app, &a.channel, 7, "Show/Season 03", field).await;
            let title = format!("[SubsPlease] Show - {first:02} (1080p)");
            picked(&app, &a.channel, &rule, &title).await;

            let view = view(&app, &rule).await;

            assert_eq!(view["episode_basis"], Value::Null, "{name}");
            assert_eq!(view["episode"], field, "{name}");
            match offer {
                Offer::Nothing => assert_eq!(view["episode_suggestion"], Value::Null, "{name}"),
                Offer::Suggests(value, grounds) => {
                    assert_eq!(view["episode_suggestion"]["value"], json!(value), "{name}");
                    let basis = view["episode_suggestion"]["basis"].as_str().unwrap();
                    for piece in grounds {
                        assert!(basis.contains(piece), "{name}: {basis}");
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn a_suggestion_is_applied_as_the_users_own_value() {
        let app = App::new().await;
        library(
            &app,
            &[(1, &["01", "02"]), (2, &["01", "02"])],
            &[Some(12), Some(12)],
        )
        .await;
        let a = app.channel("a.test", &[], &[]).await;
        let rule = subscription(&app, &a.channel, 7, "Show/Season 03", 1).await;
        picked(&app, &a.channel, &rule, "[SubsPlease] Show - 27 (1080p)").await;
        let offered = view(&app, &rule).await;
        assert_eq!(offered["episode_suggestion"]["value"], -24);
        let uri = format!("/api/rules/{}/episode", rule.id);

        // `적용` saves the value as the user's own.
        let (status, text, applied) = app
            .call(
                Method::PUT,
                &uri,
                Some(json!({ "version": offered["version"], "episode": -24 })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert_eq!(applied["episode"], -24);
        assert_eq!(applied["episode_auto"], false);
        assert_eq!(applied["episode_suggestion"], Value::Null);

        // A stale version is a conflict that carries the current rule.
        let (status, text, conflict) = app
            .call(
                Method::PUT,
                &uri,
                Some(json!({ "version": offered["version"], "episode": -12 })),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{text}");
        assert_eq!(conflict["current"]["episode"], -24);
    }

    #[tokio::test]
    async fn a_suggestion_that_cannot_be_read_leaves_the_rule_list_answering() {
        let app = App::new().await;
        library(
            &app,
            &[(1, &["01", "02"]), (2, &["01", "02"])],
            &[Some(12), Some(12)],
        )
        .await;
        let a = app.channel("a.test", &[], &[]).await;
        let rule = subscription(&app, &a.channel, 7, "Show/Season 03", 1).await;
        picked(&app, &a.channel, &rule, "[SubsPlease] Show - 27 (1080p)").await;
        assert_eq!(view(&app, &rule).await["episode_suggestion"]["value"], -24);

        // The AniList counts the suggestion needs cannot be read any more.
        app.db
            .run(|c| {
                c.execute_batch("ALTER TABLE season_info RENAME TO season_info_gone")
                    .map_err(trss_core::db::DbError::from)
            })
            .await
            .unwrap();

        assert_eq!(view(&app, &rule).await["episode_suggestion"], Value::Null);
        let (status, text, _) = app
            .call(
                Method::GET,
                &format!("/api/channels/{}", a.channel.id),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
    }

    /// The names a preview gives the items its rule may receive, sorted.
    fn received_names(preview: &Value) -> Vec<String> {
        let mut names: Vec<String> = preview["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|i| i["episode_name"].as_str().map(str::to_owned))
            .collect();
        names.sort();
        names
    }

    /// A season that ended before it is subscribed brings no new release, so
    /// the earliest of its past items decides what is offered, before any is
    /// received and named.
    #[tokio::test]
    async fn a_season_that_ended_is_offered_its_offset_before_its_first_past_item_is_received() {
        let app = App::new().await;
        library(
            &app,
            &[(1, &["01", "02"]), (2, &["01", "02"])],
            &[Some(12), Some(12)],
        )
        .await;
        let a = app.channel("a.test", &[], &[]).await;
        app.record(
            &a.channel,
            1_000,
            &[
                "[SubsPlease] Show - 49 (1080p)",
                "[SubsPlease] Show - 48 (1080p)",
            ],
        )
        .await;
        let draft = |episode: i64| {
            json!({
                "channel_id": a.channel.id,
                "subscribing": true,
                "rule": { "match": "Show", "directory": "Show/Season 03", "episode": episode },
            })
        };

        // The subscription about to be made: each item says its name, and the
        // offset the earliest one gives is offered.
        let preview = app.preview(draft(1)).await;
        assert_eq!(received_names(&preview), ["S03E48", "S03E49"]);
        assert_eq!(preview["episode_suggestion"]["value"], -24);
        let basis = preview["episode_suggestion"]["basis"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            basis.contains("고를 수 있는 지난 회차 중 가장 앞선 릴리스가 48화")
                && basis.contains("시즌 24화"),
            "{basis}"
        );
        // Applied, the names follow and nothing is offered any more.
        let applied = app.preview(draft(-24)).await;
        assert_eq!(received_names(&applied), ["S03E24", "S03E25"]);
        assert!(applied.get("episode_suggestion").is_none(), "{applied}");

        // Subscribed without it, the rule detail offers the same before
        // anything is received, and its preview names the past items.
        let rule = subscription(&app, &a.channel, 7, "Show/Season 03", 1).await;
        let view = view(&app, &rule).await;
        assert_eq!(view["episode_suggestion"]["value"], -24);
        assert_eq!(view["episode_suggestion"]["basis"], basis);
        let preview = app
            .preview(json!({
                "channel_id": a.channel.id,
                "rule_id": rule.id,
                "rule": { "match": "Show", "directory": "Show/Season 03", "episode": 1 },
            }))
            .await;
        assert_eq!(received_names(&preview), ["S03E48", "S03E49"]);
    }

    /// The last undo of the rule comes with its view: the command, the value
    /// undone and put back, and each video with where it stands.
    #[tokio::test]
    async fn the_view_tells_the_last_undo_with_the_videos_it_renamed_or_left() {
        use trss_collect::store::channels::NewUndoFile;
        use trss_core::commands::{CommandState, NewCommand, Outcome};
        let app = App::new().await;
        let a = app.channel("a.test", &[], &[]).await;
        let rule = subscription(&app, &a.channel, 7, "Show/Season 03", 1).await;
        app.state
            .channels
            .set_auto_episode(&rule.id, rule.version, -24, BASIS)
            .await
            .unwrap()
            .expect("the rule was at the version read");
        assert_eq!(view(&app, &rule).await["episode_undo"], Value::Null);
        let titles = [
            "[SubsPlease] Show - 25 (1080p)",
            "[SubsPlease] Show - 26 (1080p)",
            "[SubsPlease] Show - 27 (1080p)",
        ];
        app.record_as(
            &a.channel,
            2_000,
            &titles,
            HistoryResult::Received,
            Some(&rule.id),
        )
        .await;
        let items = app
            .state
            .history
            .list(Default::default())
            .await
            .unwrap()
            .items;
        let item = |n: usize| items.iter().find(|i| i.title == titles[n]).unwrap().id;
        let planned = |n: usize, from: &str, to: &str| NewUndoFile {
            item_id: item(n),
            folder: "/media/Show/Season 03".into(),
            from_name: from.into(),
            to_name: to.into(),
            torrent_hash: None,
            identity: None,
            kept: None,
        };

        // The request is told as it ended before the undo began.
        let new = |id: &str| NewCommand {
            id: id.into(),
            kind: "episode_undo".into(),
            payload: format!(r#"{{"rule_id":"{}","episode":-24}}"#, rule.id),
            subject: Some(rule.id.clone()),
        };
        app.state
            .commands
            .accept(new("undo-1"), 1_000)
            .await
            .unwrap();
        app.state
            .commands
            .finish(
                "undo-1",
                CommandState::Failed,
                Outcome {
                    result: "failed".into(),
                    reason: Some("수정본으로 대체하는 중인 영상이 있어요.".into()),
                },
                1_500,
            )
            .await
            .unwrap();
        let told = view(&app, &rule).await["episode_undo"].clone();
        assert_eq!(told["command"]["state"], "failed");
        assert_eq!(
            (
                told["from"].clone(),
                told["to"].clone(),
                told["files"].clone()
            ),
            (Value::Null, Value::Null, json!([]))
        );

        // The undo of a second request began: one video is renamed, one waits
        // for its torrent, one is left as it is.
        app.state
            .commands
            .accept(new("undo-2"), 2_000)
            .await
            .unwrap();
        app.state
            .channels
            .begin_episode_undo(
                "undo-2",
                &rule.id,
                -24,
                1,
                vec![
                    planned(0, "Show S03E01.mkv", "Show S03E25.mkv"),
                    planned(1, "Show S03E02.mkv", "Show S03E26.mkv"),
                    NewUndoFile {
                        kept: Some("같은 이름의 파일이 이미 있어요.".into()),
                        ..planned(2, "Show S03E03.mkv", "Show S03E27.mkv")
                    },
                ],
                2_500,
            )
            .await
            .unwrap();
        app.state
            .channels
            .finish_undo_file("undo-2", item(0), None, 3_000)
            .await
            .unwrap();
        app.state
            .commands
            .finish(
                "undo-2",
                CommandState::Done,
                Outcome {
                    result: "paused".into(),
                    reason: None,
                },
                3_500,
            )
            .await
            .unwrap();

        let view = view(&app, &rule).await;
        let told = &view["episode_undo"];
        assert_eq!(told["command"]["id"], "undo-2");
        assert_eq!(told["command"]["state"], "done");
        assert_eq!(
            (told["from"].clone(), told["to"].clone()),
            (json!(-24), json!(1))
        );
        let files: Vec<(String, String, String, Value)> = told["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["from_name"].as_str().unwrap().to_owned(),
                    f["to_name"].as_str().unwrap().to_owned(),
                    f["state"].as_str().unwrap().to_owned(),
                    f["reason"].clone(),
                )
            })
            .collect();
        assert_eq!(
            files,
            [
                (
                    "Show S03E01.mkv".to_owned(),
                    "Show S03E25.mkv".to_owned(),
                    "renamed".to_owned(),
                    Value::Null
                ),
                (
                    "Show S03E02.mkv".to_owned(),
                    "Show S03E26.mkv".to_owned(),
                    "pending".to_owned(),
                    Value::Null
                ),
                (
                    "Show S03E03.mkv".to_owned(),
                    "Show S03E27.mkv".to_owned(),
                    "kept".to_owned(),
                    json!("같은 이름의 파일이 이미 있어요.")
                ),
            ]
        );
        // The value is back, as the user's own.
        assert_eq!(
            (view["episode"].clone(), view["episode_auto"].clone()),
            (json!(1), json!(false))
        );
        assert_eq!(view["episode_basis"], Value::Null);
        assert_eq!(view["episode_previous"], Value::Null);
    }
}
