use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use crate::store::channels::{ChannelInput, ChannelWithRules};
use crate::store::history::Observation;
use crate::store::Db;

struct App {
    state: AppState,
    router: Router,
}

impl App {
    /// An app whose collect folder is `/media`.
    async fn new() -> App {
        let state = AppState::new(Db::open_blocking(":memory:").unwrap());
        state
            .settings
            .put_collection(0, "/media".to_owned(), None)
            .await
            .unwrap();
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

#[tokio::test]
async fn an_excluded_item_and_an_archived_rule_make_no_overlap() {
    let app = App::new().await;
    let a = app
        .channel("a.test", &["[Batch]"], &[("Show", "one"), ("Show", "two")])
        .await;
    app.record(&a.channel, 1_000, &["Show [Batch]"]).await;
    assert_eq!(app.list().await["rules"][1]["overlap"], false);

    // With an item that both match, archiving the shadowed rule removes the overlap.
    app.record(&a.channel, 2_000, &["Show - 01"]).await;
    assert_eq!(app.list().await["rules"][1]["overlap"], true);
    let second = &app.list().await["rules"][1];
    // The worker archives (the `rule_archive` command); the store call is its.
    app.state
        .channels
        .set_rule_state(second["id"].as_str().unwrap(), RuleState::Archived)
        .await
        .unwrap();
    assert_eq!(app.list().await["rules"][1]["overlap"], false);
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
async fn the_history_survives_the_deletion_of_its_rule() {
    let app = App::new().await;
    let a = app.channel("a.test", &[], &[("Alpha", "a")]).await;
    let rule = &a.rules[0];
    app.record_as(
        &a.channel,
        1_000,
        &["Alpha 01"],
        HistoryResult::Received,
        Some(&rule.id),
    )
    .await;

    let (status, _, _) = app
        .call(
            Method::DELETE,
            &format!("/api/rules/{}?version={}", rule.id, rule.version),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let page = app.state.history.list(Default::default()).await.unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].rule_id.as_deref(), Some(rule.id.as_str()));
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

    // Archived, a rule is judged as if restored.
    let rule = app.list().await["rules"][0].clone();
    app.state
        .channels
        .set_rule_state(rule["id"].as_str().unwrap(), RuleState::Archived)
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
        [("Show - 01".to_owned(), "mine".to_owned())]
    );

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
    use crate::store::commands::{CommandState, NewCommand, Outcome};
    use crate::worker::commands::rule_archive::{Direction, RuleArchive, KIND};

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
    use crate::store::commands::{CommandState, NewCommand, Outcome};
    use crate::worker::commands::rule_archive::{Direction, RuleArchive, KIND};

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
