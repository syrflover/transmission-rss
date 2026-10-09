use axum::{
    http::{header, HeaderValue, Method, StatusCode},
    Router,
};
use serde_json::Value;

use super::*;
use crate::testing;
use trss_collect::store::{
    channels::{ChannelInput, RuleInput, RuleState},
    history::Observation,
};
use trss_core::{
    commands::{CommandState, NewCommand, Outcome},
    Db,
};

const TOKEN: &str = "s3cr3t-tok-0123";

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

    async fn get(&self, uri: &str) -> (StatusCode, String, Value) {
        let mut request = testing::request(Method::GET, uri, None);
        request
            .headers_mut()
            .insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        testing::send_text(&self.router, request).await
    }

    async fn channel(&self, host: &str, name: Option<&str>) -> Channel {
        let mut input = ChannelInput::new(format!("https://{host}/rss?token={TOKEN}"));
        input.name = name.map(str::to_owned);
        self.state.channels.create_channel(input).await.unwrap()
    }

    async fn record(&self, channel: &Channel, n: i64, result: HistoryResult, rule: Option<&str>) {
        self.state
            .history
            .record(
                1_000_000 + n * 60_000,
                vec![Observation {
                    channel_id: channel.id.clone(),
                    channel_label: channel.masked_url(),
                    identity_key: format!("guid:{n}"),
                    title: format!("[Group] Show - {n:04} (1080p).mkv"),
                    link: format!("magnet:?xt=urn:btih:{n:040}&token=***"),
                    result,
                    rule_id: rule.map(str::to_owned),
                    torrent_hash: None,
                    reason: (result == HistoryResult::AddFailed)
                        .then(|| "Transmission이 토렌트를 받지 않았어요".to_owned()),
                }],
            )
            .await
            .unwrap();
    }
}

fn ids(page: &Value) -> Vec<i64> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn items_come_newest_first_with_what_the_screen_needs() {
    let app = App::new();
    let named = app.channel("nyaa.example", Some("Nyaa")).await;
    let plain = app.channel("feed.example", None).await;
    let rule = app
        .state
        .channels
        .create_rule(
            &named.id,
            RuleInput {
                r#match: Some("Show".to_owned()),
                directory: "Show/Season 01".to_owned(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    app.record(&named, 1, HistoryResult::Received, Some(&rule.id))
        .await;
    app.record(&plain, 2, HistoryResult::NoMatch, None).await;
    app.record(&named, 3, HistoryResult::AddFailed, None).await;

    let (status, text, page) = app.get("/api/history").await;

    assert_eq!(status, StatusCode::OK, "{text}");
    let items = page["items"].as_array().unwrap();
    let titles: Vec<&str> = items.iter().map(|i| i["title"].as_str().unwrap()).collect();
    assert_eq!(
        titles,
        [
            "[Group] Show - 0003 (1080p).mkv",
            "[Group] Show - 0002 (1080p).mkv",
            "[Group] Show - 0001 (1080p).mkv"
        ]
    );
    assert_eq!(items[0]["result"], "add_failed");
    assert_eq!(items[0]["result_label"], "추가 실패");
    assert_eq!(items[0]["reason"], "Transmission이 토렌트를 받지 않았어요");
    assert_eq!(items[0]["channel_name"], "Nyaa");
    assert_eq!(
        items[1]["channel_name"], "feed.example",
        "an unnamed channel shows its host"
    );
    assert_eq!(items[1]["can_retry"], false, "no rule picked it");
    assert_eq!(items[1]["retry_blocked"], Value::Null);
    assert_eq!(items[2]["rule_label"], "Show");
    assert_eq!(items[2]["can_retry"], false);
    assert_eq!(items[2]["by_hand"], false);
    assert_eq!(items[0]["first_seen_at"], 1_000_000 + 3 * 60_000);
    assert_eq!(page["next"], Value::Null);
    assert_eq!(page["counts"]["total"], 3);
    assert_eq!(page["counts"]["received"], 1);
    assert_eq!(page["counts"]["no_match"], 1);
    assert_eq!(page["counts"]["add_failed"], 1);
    assert_eq!(page["counts"]["excluded"], 0);

    // The masked copy of a link is not sent, and no secret is anywhere.
    assert!(!text.contains("magnet:"), "{text}");
    assert!(!text.contains(TOKEN));
    assert!(!text.contains("\"link\""));

    let (status, _, one) = app.get(&format!("/api/history/{}", items[0]["id"])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(one["title"], items[0]["title"]);
    assert_eq!(
        app.get("/api/history/999999").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(app.get("/api/history/abc").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_result_filter_takes_several_codes_and_keeps_the_others_out() {
    let app = App::new();
    let channel = app.channel("nyaa.example", None).await;
    for (n, result) in [
        HistoryResult::Received,
        HistoryResult::NoMatch,
        HistoryResult::Excluded,
        HistoryResult::Duplicate,
        HistoryResult::AddFailed,
        HistoryResult::AddFailed,
    ]
    .into_iter()
    .enumerate()
    {
        app.record(&channel, n as i64, result, None).await;
    }
    let results = |page: &Value| -> Vec<String> {
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["result"].as_str().unwrap().to_owned())
            .collect()
    };

    let (_, _, both) = app.get("/api/history?result=add_failed,duplicate").await;
    assert_eq!(results(&both), ["add_failed", "add_failed", "duplicate"]);

    let (_, _, spaced) = app
        .get("/api/history?result=%20add_failed%20,,duplicate,add_failed")
        .await;
    assert_eq!(results(&spaced), results(&both));

    let (_, _, one) = app.get("/api/history?result=no_match").await;
    assert_eq!(results(&one), ["no_match"]);

    let (_, _, all) = app.get("/api/history?result=").await;
    assert_eq!(all["items"].as_array().unwrap().len(), 6);

    // The chips' counts are about the channel, not the result filter.
    assert_eq!(both["counts"]["total"], 6);

    let (status, _, body) = app.get("/api/history?result=add_failed,bogus").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid");
    assert!(body["message"].as_str().unwrap().ends_with("요."));
}

#[tokio::test]
async fn the_channel_filter_limits_the_list_and_the_counts() {
    let app = App::new();
    let a = app.channel("a.example", None).await;
    let b = app.channel("b.example", None).await;
    app.record(&a, 1, HistoryResult::NoMatch, None).await;
    app.record(&b, 2, HistoryResult::NoMatch, None).await;
    app.record(&b, 3, HistoryResult::Excluded, None).await;

    let (_, _, only_b) = app.get(&format!("/api/history?channel={}", b.id)).await;

    assert_eq!(only_b["items"].as_array().unwrap().len(), 2);
    assert_eq!(only_b["counts"]["total"], 2);
    assert_eq!(only_b["counts"]["no_match"], 1);
    let (_, _, none) = app.get("/api/history?channel=nope").await;
    assert!(none["items"].as_array().unwrap().is_empty());
    assert_eq!(none["counts"]["total"], 0);
}

#[tokio::test]
async fn a_bad_cursor_or_query_is_refused_and_the_page_size_is_capped() {
    let app = App::new();
    let channel = app.channel("nyaa.example", None).await;
    for n in 0..3 {
        app.record(&channel, n, HistoryResult::NoMatch, None).await;
    }

    let (status, _, body) = app.get("/api/history?after=garbage").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid");
    assert_eq!(
        app.get("/api/history?limit=abc").await.0,
        StatusCode::BAD_REQUEST
    );

    let (_, _, capped) = app.get("/api/history?limit=100000").await;
    assert_eq!(capped["items"].as_array().unwrap().len(), 3);
    let (_, _, one) = app.get("/api/history?limit=0").await;
    assert_eq!(
        one["items"].as_array().unwrap().len(),
        1,
        "at least one item per page"
    );
    // The cursor a page hands out is the `after` the next request takes. (The
    // paging rule is tested in trss-collect.)
    let cursor = one["next"].as_str().unwrap();
    let (status, _, second) = app
        .get(&format!("/api/history?limit=1&after={cursor}"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&second).len(), 1);
    assert_ne!(ids(&second), ids(&one));
}

#[tokio::test]
async fn the_row_says_whether_a_failed_item_can_be_retried_and_why_not() {
    let app = App::new();
    let channel = app.channel("nyaa.example", None).await;
    let rule = |state, directory: &'static str| RuleInput {
        r#match: Some("Show".to_owned()),
        directory: directory.to_owned(),
        state,
        ..Default::default()
    };
    let rules = &app.state.channels;
    let active = rules
        .create_rule(&channel.id, rule(RuleState::Active, "Show/Season 01"))
        .await
        .unwrap();
    let archived = rules
        .create_rule(&channel.id, rule(RuleState::Archived, "Show/Season 02"))
        .await
        .unwrap();
    let deleted = rules
        .create_rule(&channel.id, rule(RuleState::Active, "Show/Season 03"))
        .await
        .unwrap();
    rules
        .delete_rule(&deleted.id, deleted.version)
        .await
        .unwrap();

    app.record(&channel, 1, HistoryResult::AddFailed, Some(&active.id))
        .await;
    app.record(&channel, 2, HistoryResult::AddFailed, Some(&archived.id))
        .await;
    app.record(&channel, 3, HistoryResult::AddFailed, Some(&deleted.id))
        .await;
    app.record(&channel, 4, HistoryResult::AddFailed, None)
        .await;
    app.record(&channel, 5, HistoryResult::NoMatch, None).await;
    app.record(&channel, 6, HistoryResult::Received, Some(&active.id))
        .await;

    let (_, _, page) = app.get("/api/history").await;

    // Newest first: items 6 down to 1.
    let items = page["items"].as_array().unwrap();
    let row = |n: usize| &items[6 - n];
    let summary = |n: usize| (row(n)["can_retry"].clone(), row(n)["retry_blocked"].clone());
    assert_eq!(summary(1), (Value::Bool(true), Value::Null));
    assert_eq!(row(1)["rule_label"], "Show");
    assert_eq!(summary(2).0, false);
    assert_eq!(summary(2).1, "규칙이 보관돼 있어요. 복원한 뒤 다시 받아요.");
    assert_eq!(summary(3).0, false);
    assert_eq!(
        summary(3).1,
        "이 항목을 고른 규칙이 지워져서 다시 받을 수 없어요."
    );
    assert_eq!(summary(4).0, false);
    assert!(summary(4).1.as_str().unwrap().contains("규칙 없이"));
    // Nothing to say for an item no rule picked or one already added.
    assert_eq!(summary(5), (Value::Bool(false), Value::Null));
    assert_eq!(summary(6), (Value::Bool(false), Value::Null));
    assert_eq!(row(6)["result_label"], "추가함");
}

#[tokio::test]
async fn an_item_shows_its_open_receive_once_command_until_it_ends() {
    let app = App::new();
    let channel = app.channel("nyaa.example", None).await;
    app.record(&channel, 1, HistoryResult::NoMatch, None).await;
    app.record(&channel, 2, HistoryResult::NoMatch, None).await;
    let (_, _, page) = app.get("/api/history").await;
    let second = ids(&page)[0];
    let first = ids(&page)[1];
    app.state
        .commands
        .accept(
            NewCommand {
                id: "0b7d5a44-6c1e".into(),
                kind: "receive_once".into(),
                payload: format!(r#"{{"item_id":{first}}}"#),
                subject: Some(first.to_string()),
            },
            5_000,
        )
        .await
        .unwrap();

    let (_, _, page) = app.get("/api/history").await;
    let items = page["items"].as_array().unwrap();
    assert_eq!(items[0]["id"], second);
    assert_eq!(items[0]["command"], Value::Null);
    assert_eq!(items[1]["command"]["id"], "0b7d5a44-6c1e");
    assert_eq!(items[1]["command"]["state"], "pending");
    let (_, _, one) = app.get(&format!("/api/history/{first}")).await;
    assert_eq!(one["command"]["state"], "pending");

    app.state.commands.claim_next(6_000).await.unwrap().unwrap();
    let (_, _, running) = app.get(&format!("/api/history/{first}")).await;
    assert_eq!(running["command"]["state"], "running");

    app.state
        .commands
        .finish(
            "0b7d5a44-6c1e",
            CommandState::Done,
            Outcome {
                result: "received".into(),
                reason: None,
            },
            7_000,
        )
        .await
        .unwrap();
    let (_, _, ended) = app.get(&format!("/api/history/{first}")).await;
    assert_eq!(
        ended["command"],
        Value::Null,
        "an ended command is read by its ID"
    );
}

#[tokio::test]
async fn an_item_of_a_deleted_channel_still_lists_with_its_old_host() {
    let app = App::new();
    let channel = app.channel("gone.example", Some("Gone")).await;
    app.record(&channel, 1, HistoryResult::NoMatch, None).await;
    app.state
        .channels
        .delete_channel(&channel.id, channel.version, 0)
        .await
        .unwrap();

    let (_, text, page) = app.get("/api/history").await;

    let item = &page["items"][0];
    assert_eq!(item["channel_deleted"], true);
    assert_eq!(item["channel_name"], "gone.example");
    assert!(!text.contains(TOKEN));
}
