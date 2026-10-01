//! Subscribing before the first episode is in the channel, and the title
//! candidates that complete such a subscription (ticket 0020).

use super::*;
use crate::store::history::HistoryQuery;

const NEW_1: &str = "[SubsPlease] New Work - 01 (1080p) [AAAA1111].mkv";
const NEW_2: &str = "[SubsPlease] New Work - 02 (1080p) [AAAA1112].mkv";
const NEW_3: &str = "[SubsPlease] New Work - 03 (1080p) [AAAA1113].mkv";
const NEW_BATCH: &str = "[Group] New Work - 01-12 [Batch]";
const OLD: &str = "[SubsPlease] Old Show - 07 (1080p) [AAAA0007].mkv";

impl App {
    /// The body of a subscription that waits for its title.
    fn waiting_body(&self, channel: &Channel) -> Value {
        json!({
            "channel_id": channel.id, "anissia_anime_no": 3320, "week": 3,
            "subtitles": "undecided", "directory": "작품",
        })
    }

    /// A channel whose history holds `OLD` from before `NOW`, with a
    /// subscription that has waited for its title since `NOW`.
    async fn waiting_in_a_known_channel(&self) -> (Channel, Value) {
        self.schedule_of_wednesday();
        let channel = self.channel("feed.test").await;
        self.record(&channel, NOW - 5_000, &[OLD]).await;
        let (status, body) = self
            .call(
                Method::POST,
                "/api/subscriptions",
                Some(self.waiting_body(&channel)),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        (channel, body["rule"].clone())
    }

    async fn candidates(&self) -> Vec<Value> {
        let (status, body) = self.get("/api/subscriptions/candidates").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["candidates"].as_array().unwrap().clone()
    }

    async fn preview_fields(&self, channel: &Channel, rule_id: &str, fields: Value) -> Value {
        let (status, preview) = self
            .call(
                Method::POST,
                "/api/rules/preview",
                Some(json!({ "channel_id": channel.id, "rule_id": rule_id, "rule": fields })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{preview}");
        preview
    }
}

type Row = (String, String, Option<String>);

/// The listed items as `(title, kind, past cause)`, by title.
fn kinds(preview: &Value) -> Vec<Row> {
    let mut rows: Vec<Row> = preview["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["title"].as_str().unwrap().to_owned(),
                i["kind"].as_str().unwrap().to_owned(),
                i["past_cause"].as_str().map(str::to_owned),
            )
        })
        .collect();
    rows.sort();
    rows
}

fn row(title: &str, kind: &str, cause: Option<&str>) -> Row {
    (title.to_owned(), kind.to_owned(), cause.map(str::to_owned))
}

#[tokio::test]
async fn a_subscription_without_a_work_waits_for_its_title_and_receives_nothing() {
    let app = App::new().await;
    let (channel, rule) = app.waiting_in_a_known_channel().await;

    assert_eq!(rule["match"], Value::Null);
    assert_eq!(rule["state"], "active");
    assert_eq!(rule["directory"], "작품");
    assert_eq!(rule["subscription"]["anissia_anime_no"], 3320);
    assert_eq!(rule["subscription"]["subscribed_at"], NOW);
    assert_eq!(rule["subscription"]["anime"]["subject"], "작품");

    // The subscription list has it, with no title yet; this quarter.
    let (_, list) = app.get("/api/subscriptions").await;
    let item = &list["subscriptions"][0];
    assert_eq!(item["rule_id"], rule["id"]);
    assert_eq!(item["title"], Value::Null);
    assert_eq!(item["upcoming"], false);

    // It matches nothing: whatever is recorded, the preview of the stored
    // rule takes nothing.
    app.record(&channel, NOW + 1_000, &[WORK_1]).await;
    let id = rule["id"].as_str().unwrap();
    let preview = app
        .preview_fields(
            &channel,
            id,
            json!({ "match": null, "directory": "작품", "episode": 1 }),
        )
        .await;
    assert_eq!(preview["counts"]["mine"], 0);
    assert_eq!(preview["counts"]["past"], 0);
    assert!(app
        .state
        .commands
        .open_for_subjects("receive_once", vec![])
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_channel_with_no_recorded_history_cannot_have_a_subscription_waiting() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    let channel = app.channel("feed.test").await;

    let (status, body) = app
        .call(
            Method::POST,
            "/api/subscriptions",
            Some(app.waiting_body(&channel)),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("수집 기록이 아직 없어요"));
    assert_eq!(app.rules(&channel).await, 0);

    // The folder is required, as for any subscription.
    app.record(&channel, NOW - 1_000, &[OLD]).await;
    for directory in ["", " ", ".", "/abs"] {
        let mut body = app.waiting_body(&channel);
        body["directory"] = json!(directory);
        let (status, answer) = app
            .call(Method::POST, "/api/subscriptions", Some(body))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{directory:?}: {answer}");
    }
    assert_eq!(app.rules(&channel).await, 0);
}

#[tokio::test]
async fn a_new_title_becomes_one_candidate_and_a_second_episode_makes_no_second() {
    let app = App::new().await;
    let (channel, rule) = app.waiting_in_a_known_channel().await;
    assert!(app.candidates().await.is_empty(), "OLD was there before");

    app.record(&channel, NOW + 1_000, &[NEW_1]).await;
    let candidates = app.candidates().await;
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!(candidate["work"], "New Work");
    assert_eq!(candidate["channel_id"], channel.id);
    assert_eq!(candidate["folder"], "New Work/Season 01");
    assert_eq!(candidate["items"], 1);
    assert_eq!(candidate["waiting"][0]["rule_id"], rule["id"]);
    assert_eq!(candidate["waiting"][0]["rule_version"], rule["version"]);
    assert_eq!(candidate["waiting"][0]["anime"]["subject"], "작품");
    assert_eq!(candidate["waiting"][0]["directory"], "작품");

    app.record(&channel, NOW + 2_000, &[NEW_2]).await;
    let candidates = app.candidates().await;
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0]["items"], 2);
    assert_eq!(candidates[0]["latest_title"], NEW_2);
}

#[tokio::test]
async fn titles_seen_while_nothing_waited_make_no_candidate() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    let channel = app.channel("feed.test").await;
    // Appeared before any subscription waited.
    app.record(&channel, NOW - 2_000, &[OLD]).await;
    assert!(app.candidates().await.is_empty());

    let (status, body) = app
        .call(
            Method::POST,
            "/api/subscriptions",
            Some(app.waiting_body(&channel)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert!(app.candidates().await.is_empty());

    // A work that was in the history before is old, however many later
    // episodes appear; one that was not there is new.
    app.record(&channel, NOW + 1_000, &[NEW_1]).await;
    let next_old = OLD.replace("07", "08");
    app.record(&channel, NOW + 1_500, &[next_old.as_str()])
        .await;
    let works: Vec<_> = app
        .candidates()
        .await
        .into_iter()
        .map(|c| c["work"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(works, ["New Work"], "Old Show is old");
}

#[tokio::test]
async fn a_channel_without_a_waiting_subscription_has_no_candidates() {
    let app = App::new().await;
    let (_, rule) = app.waiting_in_a_known_channel().await;
    let other = app.channel("other.test").await;
    app.record(&other, NOW - 1, &[OLD]).await;
    app.record(&other, NOW + 1_000, &[NEW_1]).await;
    assert!(
        app.candidates().await.is_empty(),
        "the other channel waits for nothing"
    );

    // Nor once the only waiting subscription stopped collecting.
    let channel_id = rule["channel_id"].as_str().unwrap();
    let channel = app
        .state
        .channels
        .get_channel(channel_id)
        .await
        .unwrap()
        .unwrap();
    app.record(&channel, NOW + 2_000, &[NEW_1]).await;
    assert_eq!(app.candidates().await.len(), 1);
    let id = rule["id"].as_str().unwrap();
    let (status, paused) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{id}/switch"),
            Some(json!({ "version": rule["version"], "video": false })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{paused}");
    assert!(app.candidates().await.is_empty(), "paused offers nothing");

    // Turned back on, the candidate first seen while it waited is offered again,
    // and so is what appears afterwards.
    app.now.fetch_add(10_000, Ordering::SeqCst);
    let (_, resumed) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{id}/switch"),
            Some(json!({ "version": paused["version"], "video": true })),
        )
        .await;
    assert_eq!(resumed["state"], "active");
    let kept = app.candidates().await;
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0]["work"], "New Work");
    app.record(
        &channel,
        NOW + 20_000,
        &["[SubsPlease] Later Work - 01 (1080p)"],
    )
    .await;
    let candidates = app.candidates().await;
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0]["work"], "Later Work");
    assert_eq!(candidates[1]["work"], "New Work");
}

#[tokio::test]
async fn a_title_another_rule_already_handles_is_no_candidate() {
    let app = App::new().await;
    let (channel, _) = app.waiting_in_a_known_channel().await;
    app.record(&channel, NOW + 1_000, &[NEW_1]).await;
    assert_eq!(app.candidates().await.len(), 1);

    // A rule that matches the work makes it that rule's.
    app.state
        .channels
        .create_rule(
            &channel.id,
            RuleInput {
                r#match: Some("New Work".into()),
                directory: "New Work".into(),
                ..RuleInput::default()
            },
        )
        .await
        .unwrap();
    assert!(app.candidates().await.is_empty());
}

#[tokio::test]
async fn a_rejected_candidate_is_gone_for_good_and_the_subscription_keeps_waiting() {
    let app = App::new().await;
    let (channel, rule) = app.waiting_in_a_known_channel().await;
    app.record(&channel, NOW + 1_000, &[NEW_1]).await;
    app.record(&channel, NOW + 1_100, &["[X] Second Work - 01"])
        .await;
    assert_eq!(app.candidates().await.len(), 2);

    let (status, body) = app
        .call(
            Method::POST,
            "/api/subscriptions/candidates/reject",
            Some(json!({ "channel_id": channel.id, "work": "new work" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let works: Vec<_> = app
        .candidates()
        .await
        .into_iter()
        .map(|c| c["work"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(works, ["Second Work"]);

    // Later episodes do not bring it back, and rejecting twice is harmless.
    app.record(&channel, NOW + 2_000, &[NEW_2, NEW_3]).await;
    assert_eq!(app.candidates().await.len(), 1);
    let (status, _) = app
        .call(
            Method::POST,
            "/api/subscriptions/candidates/reject",
            Some(json!({ "channel_id": channel.id, "work": "New Work" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // The subscription is as it was.
    let id = rule["id"].as_str().unwrap();
    let (_, read) = app.get(&format!("/api/rules/{id}")).await;
    assert_eq!(read["match"], Value::Null);
    assert_eq!(read["state"], "active");
    assert_eq!(read["version"], rule["version"]);

    // A work the channel never recorded, and a channel that is not there.
    for body in [
        json!({ "channel_id": channel.id, "work": "Nothing Like It" }),
        json!({ "channel_id": "no-channel", "work": "New Work" }),
    ] {
        let (status, _) = app
            .call(
                Method::POST,
                "/api/subscriptions/candidates/reject",
                Some(body),
            )
            .await;
        assert!(status.is_client_error(), "{status}");
    }
}

#[tokio::test]
async fn naming_a_title_completes_the_rule_and_leaves_what_was_recorded_to_the_user() {
    let app = App::new().await;
    let (channel, rule) = app.waiting_in_a_known_channel().await;
    app.record(&channel, NOW + 1_000, &[NEW_1, NEW_2, NEW_BATCH])
        .await;
    let id = rule["id"].as_str().unwrap().to_owned();

    // Before saving anything, the preview of the rule with the title shows
    // what recording holds as past: the cycle would not take it either.
    let preview = app
        .preview_fields(
            &channel,
            &id,
            json!({ "match": "New Work", "directory": "New Work/Season 01", "episode": 1 }),
        )
        .await;
    let mut expected = vec![
        row(NEW_1, "past", Some("titled")),
        row(NEW_2, "past", Some("titled")),
        row(NEW_BATCH, "past", Some("titled")),
    ];
    expected.sort();
    assert_eq!(kinds(&preview), expected);

    app.now.fetch_add(60_000, Ordering::SeqCst);
    let (status, named) = app
        .call(
            Method::POST,
            &format!("/api/rules/{id}/title"),
            Some(json!({
                "version": rule["version"], "work": "NEW WORK",
                "directory": "New Work/Season 01",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{named}");
    // The phrase is the work as the channel's history writes it.
    assert_eq!(named["match"], "New Work");
    assert_eq!(named["directory"], "New Work/Season 01");
    assert_eq!(named["version"], rule["version"].as_i64().unwrap() + 1);
    assert_eq!(named["state"], "active");
    assert_eq!(named["subscription"]["subscribed_at"], NOW);

    // Nothing was received or queued.
    assert!(app
        .state
        .commands
        .open_for_subjects("receive_once", vec![])
        .await
        .unwrap()
        .is_empty());
    assert!(app.candidates().await.is_empty(), "the work has a rule now");

    // The stored rule previews the same: what was recorded is past, with the
    // cause, and what is recorded after it is the rule's own.
    app.record(&channel, NOW + 70_000, &[NEW_3]).await;
    let fields = json!({ "match": "New Work", "directory": "New Work/Season 01", "episode": 1 });
    let preview = app.preview_fields(&channel, &id, fields).await;
    let mut expected = vec![
        row(NEW_1, "past", Some("titled")),
        row(NEW_2, "past", Some("titled")),
        row(NEW_3, "mine", None),
        row(NEW_BATCH, "past", Some("titled")),
    ];
    expected.sort();
    assert_eq!(kinds(&preview), expected);
    assert_eq!(preview["counts"]["past"], 3);
}

#[tokio::test]
async fn a_title_cannot_be_named_when_it_is_stale_unrecorded_or_not_waiting() {
    let app = App::new().await;
    let (channel, rule) = app.waiting_in_a_known_channel().await;
    app.record(&channel, NOW + 1_000, &[NEW_1]).await;
    let id = rule["id"].as_str().unwrap().to_owned();
    let uri = format!("/api/rules/{id}/title");
    let version = rule["version"].clone();

    // A stale version conflicts and shows the rule now.
    let (status, body) = app
        .call(
            Method::POST,
            &uri,
            Some(json!({ "version": 99, "work": "New Work" })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["current"]["id"], rule["id"]);

    // A work the history does not hold, and folders that are no work folder.
    for body in [
        json!({ "version": version, "work": "Nothing Like It" }),
        json!({ "version": version, "work": "New Work", "directory": "" }),
        json!({ "version": version, "work": "New Work", "directory": "." }),
        json!({ "version": version, "work": "New Work", "directory": "/abs" }),
        json!({ "version": version, "work": "New Work", "directory": "../up" }),
        json!({ "version": version, "work": "New Work", "unknown": true }),
    ] {
        let (status, answer) = app.call(Method::POST, &uri, Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {answer}");
    }
    let (_, read) = app.get(&format!("/api/rules/{id}")).await;
    assert_eq!(read["match"], Value::Null, "nothing was written");

    // A paused subscription is not given a title.
    let (_, paused) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{id}/switch"),
            Some(json!({ "version": version, "video": false })),
        )
        .await;
    let (status, answer) = app
        .call(
            Method::POST,
            &uri,
            Some(json!({ "version": paused["version"], "work": "New Work" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");

    // Once it has its title, another cannot be named, and a rule that is no
    // subscription waits for none.
    let (_, on) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{id}/switch"),
            Some(json!({ "version": paused["version"], "video": true })),
        )
        .await;
    let (status, named) = app
        .call(
            Method::POST,
            &uri,
            Some(json!({ "version": on["version"], "work": "New Work" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{named}");
    let (status, _) = app
        .call(
            Method::POST,
            &uri,
            Some(json!({ "version": named["version"], "work": "Old Show" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let plain = app
        .state
        .channels
        .create_rule(&channel.id, RuleInput::default())
        .await
        .unwrap();
    let (status, _) = app
        .call(
            Method::POST,
            &format!("/api/rules/{}/title", plain.id),
            Some(json!({ "version": plain.version, "work": "Old Show" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn saving_the_phrase_in_the_rule_editor_leaves_the_recorded_items_to_the_user_too() {
    let app = App::new().await;
    let (channel, rule) = app.waiting_in_a_known_channel().await;
    app.record(&channel, NOW + 1_000, &[NEW_1]).await;
    let id = rule["id"].as_str().unwrap().to_owned();

    app.now.fetch_add(60_000, Ordering::SeqCst);
    let (status, saved) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{id}"),
            Some(json!({
                "version": rule["version"], "channel_id": channel.id,
                "match": "New Work", "directory": "작품", "episode": 1,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["match"], "New Work");

    app.record(&channel, NOW + 70_000, &[NEW_2]).await;
    let preview = app
        .preview_fields(
            &channel,
            &id,
            json!({ "match": "New Work", "directory": "작품", "episode": 1 }),
        )
        .await;
    let mut expected = vec![row(NEW_1, "past", Some("titled")), row(NEW_2, "mine", None)];
    expected.sort();
    assert_eq!(kinds(&preview), expected);
}

#[tokio::test]
async fn a_history_row_that_matched_no_rule_offers_naming_only_where_a_subscription_waits() {
    let app = App::new().await;
    let (channel, rule) = app.waiting_in_a_known_channel().await;
    let other = app.channel("other.test").await;
    app.record(&channel, NOW + 1_000, &[NEW_1]).await;
    app.record(&other, NOW + 1_000, &[NEW_1]).await;

    let (status, history) = app
        .get(&format!("/api/history?channel={}&limit=50", channel.id))
        .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    let row = history["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["title"] == NEW_1)
        .unwrap();
    assert_eq!(row["name_title"]["work"], "New Work");
    assert_eq!(row["name_title"]["folder"], "New Work/Season 01");
    assert_eq!(row["name_title"]["waiting"][0]["rule_id"], rule["id"]);
    assert_eq!(row["name_title"]["waiting"][0]["anime"]["subject"], "작품");

    // Another channel has no subscription waiting: no link.
    let (_, history) = app
        .get(&format!("/api/history?channel={}&limit=50", other.id))
        .await;
    assert!(history["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["name_title"].is_null()));

    // A received item has none, even in the channel that waits.
    let item = app
        .state
        .history
        .list(HistoryQuery {
            channel_id: Some(channel.id.clone()),
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|i| i.title == NEW_1)
        .unwrap();
    app.state
        .history
        .record_outcome(
            item.id,
            NOW + 2_000,
            HistoryResult::Received,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let (_, row) = app.get(&format!("/api/history/{}", item.id)).await;
    assert_eq!(row["result"], "received");
    assert!(row["name_title"].is_null());
}

#[tokio::test]
async fn the_item_window_says_when_it_left_older_items_unread() {
    let app = App::new().await;
    let channel = app.channel("feed.test").await;
    let titles: Vec<String> = (0..=crate::store::history::MAX_PAGE_SIZE)
        .map(|n| format!("[G] Work {n} - 01"))
        .collect();
    let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
    app.record(&channel, NOW, &refs).await;

    let (read, cut) = rules_api::channel_items_up_to(&app.state.history, &channel.id, 1)
        .await
        .unwrap();
    assert_eq!(read.len(), crate::store::history::MAX_PAGE_SIZE);
    assert!(cut, "a page was read and the history goes on");

    let (read, cut) = rules_api::channel_items_up_to(&app.state.history, &channel.id, 10_000)
        .await
        .unwrap();
    assert_eq!(read.len(), titles.len());
    assert!(!cut);
}
