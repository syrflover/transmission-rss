use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use crate::{
    anissia::{fake::Fake, Anissia},
    store::{
        channels::{Channel, ChannelInput},
        history::{HistoryResult, Observation},
        Db,
    },
    worker::Clock,
};

/// 2026-10-01 12:00 in Seoul: 4분기.
const NOW: Millis = 1_790_780_400_000 + 12 * 60 * 60 * 1000;

struct App {
    state: AppState,
    router: Router,
    fake: Fake,
    now: Arc<AtomicI64>,
}

impl App {
    async fn new() -> App {
        let db = Db::open_blocking(":memory:").unwrap();
        let fake = Fake::start().await;
        let now = Arc::new(AtomicI64::new(NOW));
        let clock: Clock = {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        };
        let anissia = Anissia::new(db.clone(), fake.config(), clock).with_spacing(Duration::ZERO);
        let state = AppState::new(db).with_anissia(anissia);
        state
            .settings
            .put_collection(0, "/media".to_owned(), None)
            .await
            .unwrap();
        let router =
            Router::new().nest("/api", crate::web::api::router().with_state(state.clone()));
        App {
            state,
            router,
            fake,
            now,
        }
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
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
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        self.call(Method::GET, uri, None).await
    }

    async fn channel(&self, host: &str) -> Channel {
        self.state
            .channels
            .create_channel(ChannelInput::new(format!(
                "https://{host}/rss?token=SECRETVALUE99"
            )))
            .await
            .unwrap()
    }

    async fn record(&self, channel: &Channel, at: Millis, titles: &[&str]) {
        let observations = titles
            .iter()
            .map(|title| Observation {
                channel_id: channel.id.clone(),
                channel_label: channel.masked_url(),
                identity_key: format!("title:{title}"),
                title: title.to_string(),
                link: "https://feed.test/item".into(),
                result: HistoryResult::NoMatch,
                rule_id: None,
                torrent_hash: None,
                reason: None,
            })
            .collect();
        self.state.history.record(at, observations).await.unwrap();
    }

    /// The Wednesday schedule has "Work" (3320) with two creators and "Bare" (3321) with none.
    fn schedule_of_wednesday(&self) {
        self.fake.set_week(
            3,
            vec![
                self.fake.entry(3, 3320, "22:30", "작품", "Work Original"),
                self.fake.entry(3, 3321, "01:00", "빈 작품", ""),
            ],
        );
        self.fake.set_captions(
            3320,
            vec![
                self.fake.caption("1", "2026-10-08T01:00:00", "에텔레로사"),
                self.fake.caption("2", "2026-10-15T01:00:00", "에텔레로사"),
                self.fake.caption("1", "2026-10-09T01:00:00", "다른 제작자"),
            ],
        );
    }

    fn subscribe_body(&self, channel: &Channel) -> Value {
        json!({
            "channel_id": channel.id, "anissia_anime_no": 3320, "week": 3,
            "work": "Work", "subtitles": "follow", "creator": "에텔레로사",
            "directory": "Work/Season 01",
        })
    }

    async fn rules(&self, channel: &Channel) -> usize {
        self.state
            .channels
            .list_rules(&channel.id)
            .await
            .unwrap()
            .len()
    }
}

const WORK_1: &str = "[SubsPlease] Work - 01 (1080p) [ABCD1234].mkv";
const WORK_2: &str = "[SubsPlease] Work - 02 (1080p) [ABCD1235].mkv";
const WORK_3: &str = "[SubsPlease] Work - 03 (1080p) [ABCD1236].mkv";
const OTHER: &str = "[SubsPlease] Another Show - 05 (1080p) [ABCD1237].mkv";

#[tokio::test]
async fn the_schedule_of_a_weekday_lists_its_anime_by_time_and_marks_the_followed_ones() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    let (status, schedule) = app.get("/api/anissia/schedule/3").await;
    assert_eq!(status, StatusCode::OK, "{schedule}");
    let titles: Vec<&str> = schedule["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["subject"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["빈 작품", "작품"], "by air time");
    assert_eq!(schedule["entries"][1]["air_time"], "22:30");
    assert_eq!(schedule["entries"][1]["subscribed_rules"], json!([]));
    assert_eq!(schedule["cached"], false);

    // A second look comes from the short cache.
    let (_, again) = app.get("/api/anissia/schedule/3").await;
    assert_eq!(again["cached"], true);
    assert_eq!(app.fake.count("/anime/schedule/3"), 1);

    // After subscribing, the entry says who follows it.
    let channel = app.channel("feed.test").await;
    app.record(&channel, 1000, &[WORK_1]).await;
    let (status, _) = app
        .call(
            Method::POST,
            "/api/subscriptions",
            Some(app.subscribe_body(&channel)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, schedule) = app.get("/api/anissia/schedule/3").await;
    let followed = &schedule["entries"][1]["subscribed_rules"];
    assert_eq!(followed.as_array().unwrap().len(), 1);
    assert_eq!(followed[0]["channel_id"], channel.id);

    assert_eq!(
        app.get("/api/anissia/schedule/9").await.0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn the_other_and_upcoming_groups_list_by_start_date() {
    let app = App::new().await;
    app.fake.set_week(
        8,
        vec![
            {
                let mut a = app.fake.entry(8, 10, "2027-01-08", "나중", "");
                a["startDate"] = json!("2027-01-08");
                a
            },
            {
                let mut a = app.fake.entry(8, 11, "2026-12-30", "먼저", "");
                a["startDate"] = json!("2026-12-30");
                a
            },
        ],
    );
    let (status, schedule) = app.get("/api/anissia/schedule/8").await;
    assert_eq!(status, StatusCode::OK);
    let order: Vec<i64> = schedule["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["anime_no"].as_i64().unwrap())
        .collect();
    assert_eq!(order, [11, 10]);
    // A date in the time column is not a time.
    assert_eq!(schedule["entries"][0]["air_time"], Value::Null);
}

#[tokio::test]
async fn when_anissia_does_not_answer_the_reason_is_told_and_nothing_is_kept() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    for (setup, expected) in [
        (
            Box::new(|f: &Fake| f.state.lock().unwrap().failing = 1) as Box<dyn Fn(&Fake)>,
            "HTTP 500",
        ),
        (
            Box::new(|f: &Fake| f.state.lock().unwrap().raw = Some("<html>".into())),
            "읽지 못했어요",
        ),
    ] {
        setup(&app.fake);
        let (status, body) = app.get("/api/anissia/schedule/3").await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
        assert_eq!(body["error"], "unavailable");
        assert!(
            body["message"].as_str().unwrap().contains(expected),
            "{body}"
        );
        app.fake.state.lock().unwrap().raw = None;
    }

    // Too many requests: told to wait, and the next ask is held back too.
    {
        let mut state = app.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = Some(30);
    }
    let (status, body) = app.get("/api/anissia/schedule/3").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body["message"].as_str().unwrap().contains("30초"), "{body}");
    let requests = app.fake.requests().len();
    let (status, _) = app.get("/api/anissia/anime/3320/creators").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        app.fake.requests().len(),
        requests,
        "the block holds the call back"
    );

    // Once it has passed, the screen's retry works.
    app.now.fetch_add(30_000, Ordering::SeqCst);
    let (status, _) = app.get("/api/anissia/schedule/3").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_creators_of_an_anime_come_from_its_captions_and_may_be_none() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    let (status, body) = app.get("/api/anissia/anime/3320/creators").await;
    assert_eq!(status, StatusCode::OK);
    let creators = body["creators"].as_array().unwrap();
    assert_eq!(creators.len(), 2);
    assert_eq!(creators[0]["name"], "에텔레로사");
    assert_eq!(creators[0]["captions"], 2);

    let (status, body) = app.get("/api/anissia/anime/3321/creators").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["creators"], json!([]));
}

#[tokio::test]
async fn only_the_works_the_channels_history_holds_are_offered_with_a_folder() {
    let app = App::new().await;
    let channel = app.channel("feed.test").await;
    let empty = app.channel("empty.test").await;
    app.record(&channel, 1000, &[WORK_1, OTHER]).await;
    app.record(&channel, 2000, &[WORK_2, WORK_3]).await;

    let (status, body) = app
        .get(&format!(
            "/api/subscriptions/titles?channel_id={}",
            channel.id
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["recorded_items"], 4);
    let titles = body["titles"].as_array().unwrap();
    assert_eq!(titles.len(), 2);
    assert_eq!(titles[0]["work"], "Work");
    assert_eq!(titles[0]["items"], 3);
    assert_eq!(titles[0]["folder"], "Work/Season 01");
    assert_eq!(titles[0]["latest_title"], WORK_3);

    // The filter looks at the work and the whole title, ignoring case.
    let (_, body) = app
        .get(&format!(
            "/api/subscriptions/titles?channel_id={}&q=another",
            channel.id
        ))
        .await;
    assert_eq!(body["titles"].as_array().unwrap().len(), 1);
    assert_eq!(body["titles"][0]["work"], "Another Show");
    let (_, body) = app
        .get(&format!(
            "/api/subscriptions/titles?channel_id={}&q=nothing",
            channel.id
        ))
        .await;
    assert_eq!(body["titles"], json!([]));
    assert_eq!(body["total"], 0);
    assert_eq!(
        body["recorded_items"], 4,
        "the channel has history, nothing fits"
    );

    // A channel with no history has no title to choose from.
    let (_, body) = app
        .get(&format!(
            "/api/subscriptions/titles?channel_id={}",
            empty.id
        ))
        .await;
    assert_eq!(body["recorded_items"], 0);
    assert_eq!(body["titles"], json!([]));

    let (status, _) = app.get("/api/subscriptions/titles?channel_id=nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn subscribing_creates_the_rule_with_the_chosen_work_and_receives_nothing() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    let channel = app.channel("feed.test").await;
    app.record(&channel, 1000, &[WORK_1, WORK_2, WORK_3, OTHER])
        .await;

    let (status, body) = app
        .call(
            Method::POST,
            "/api/subscriptions",
            Some(app.subscribe_body(&channel)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let rule = &body["rule"];
    assert_eq!(rule["match"], "Work");
    assert_eq!(rule["directory"], "Work/Season 01");
    assert_eq!(rule["channel_id"], channel.id);
    assert_eq!(rule["state"], "active");
    let subscription = &rule["subscription"];
    assert_eq!(subscription["anissia_anime_no"], 3320);
    assert_eq!(subscription["subtitles"], "follow");
    assert_eq!(subscription["creator"], "에텔레로사");
    assert_eq!(subscription["season_id"], Value::Null);
    assert_eq!(subscription["subscribed_at"], NOW);
    // The snapshot is stored with it.
    assert_eq!(subscription["anime"]["subject"], "작품");
    assert_eq!(subscription["anime"]["air_time"], "22:30");
    assert_eq!(subscription["anime"]["week"], 3);

    // Nothing was received or queued, and the recorded items stay as they were.
    let items = app
        .state
        .history
        .list(Default::default())
        .await
        .unwrap()
        .items;
    assert!(items.iter().all(|i| i.result == HistoryResult::NoMatch));
    assert!(app
        .state
        .commands
        .open_for_subjects("receive_once", vec![])
        .await
        .unwrap()
        .is_empty());

    // The rule reads back the same through the rules API, which is what the
    // detail shows, and without Anissia.
    app.fake.state.lock().unwrap().failing = 100;
    let id = rule["id"].as_str().unwrap();
    let (status, read) = app.get(&format!("/api/rules/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["subscription"]["anime"]["subject"], "작품");
    let (_, listed) = app.get("/api/rules").await;
    assert_eq!(listed["rules"][0]["subscription"]["anissia_anime_no"], 3320);

    // The past items the user may pick are the ones the new rule matches.
    let (status, preview) = app
        .call(
            Method::POST,
            "/api/rules/preview",
            Some(json!({
                "channel_id": channel.id, "rule_id": id,
                "rule": { "match": "Work", "directory": "Work/Season 01", "episode": 1 },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview["counts"]["mine"], 3);
    assert!(preview["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["kind"] == "mine" && i["stored_result"] == "no_match"));
}

#[tokio::test]
async fn the_subscription_lists_under_this_quarter_with_its_stored_schedule() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    let channel = app.channel("feed.test").await;
    app.record(&channel, 1000, &[WORK_1]).await;
    app.call(
        Method::POST,
        "/api/subscriptions",
        Some(app.subscribe_body(&channel)),
    )
    .await;

    // Anissia down: the list still has the stored weekday and time.
    app.fake.state.lock().unwrap().failing = 100;
    let (status, list) = app.get("/api/subscriptions").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["quarter"], json!({ "year": 2026, "number": 4 }));
    let item = &list["subscriptions"][0];
    assert_eq!(item["title"], "Work");
    assert_eq!(item["subscription"]["anime"]["air_time"], "22:30");
    assert_eq!(item["subscription"]["anime"]["week"], 3);
    assert_eq!(item["quarter"], json!({ "year": 2026, "number": 4 }));
    assert_eq!(item["upcoming"], false);
    assert_eq!(item["channel_host"], "feed.test");

    // An archived rule is not listed.
    let id = item["rule_id"].as_str().unwrap();
    let rule = app.state.channels.get_rule(id).await.unwrap().unwrap();
    app.state
        .channels
        .update_rule(
            id,
            rule.version,
            &rule.channel_id,
            RuleInput {
                state: RuleState::Archived,
                ..rule.to_input()
            },
        )
        .await
        .unwrap();
    let (_, list) = app.get("/api/subscriptions").await;
    assert_eq!(list["subscriptions"], json!([]));
}

#[tokio::test]
async fn an_anime_that_starts_next_quarter_is_marked_upcoming() {
    let app = App::new().await;
    let mut entry = app.fake.entry(8, 77, "2027-01-08", "새 작품", "");
    entry["startDate"] = json!("2027-01-08");
    app.fake.set_week(8, vec![entry]);
    let channel = app.channel("feed.test").await;
    app.record(&channel, 1000, &[WORK_1]).await;
    let (status, body) = app
        .call(
            Method::POST,
            "/api/subscriptions",
            Some(json!({
                "channel_id": channel.id, "anissia_anime_no": 77, "week": 8,
                "work": "Work", "subtitles": "none", "directory": "Work/Season 01",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["rule"]["subscription"]["subtitles"], "none");
    let (_, list) = app.get("/api/subscriptions").await;
    assert_eq!(list["subscriptions"][0]["upcoming"], true);
    assert_eq!(
        list["subscriptions"][0]["quarter"],
        json!({ "year": 2027, "number": 1 })
    );
}

#[tokio::test]
async fn an_anime_with_no_creator_is_subscribed_undecided_or_without_subtitles() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    let channel = app.channel("feed.test").await;
    let other = app.channel("other.test").await;
    app.record(&channel, 1000, &[WORK_1]).await;
    app.record(&other, 1000, &[WORK_1]).await;

    for (target, subtitles) in [(&channel, "undecided"), (&other, "none")] {
        let (status, body) = app
            .call(
                Method::POST,
                "/api/subscriptions",
                Some(json!({
                    "channel_id": target.id, "anissia_anime_no": 3321, "week": 3,
                    "work": "Work", "subtitles": subtitles, "directory": "Work/Season 01",
                })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["rule"]["subscription"]["subtitles"], subtitles);
        assert_eq!(body["rule"]["subscription"]["creator"], Value::Null);
    }
}

#[tokio::test]
async fn a_subscription_is_refused_when_what_it_names_is_not_there_and_creates_nothing() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    let channel = app.channel("feed.test").await;
    let empty = app.channel("empty.test").await;
    app.record(&channel, 1000, &[WORK_1]).await;
    let good = app.subscribe_body(&channel);
    let with = |key: &str, value: Value| {
        let mut body = good.clone();
        body[key] = value;
        body
    };

    for (body, message) in [
        // A work the channel's history does not hold; there is no free text.
        (with("work", json!("Typed By Hand")), "수집 기록에 없는"),
        (with("directory", json!("  ")), "저장 폴더"),
        (with("directory", json!("/abs/path")), "/로 시작"),
        (with("directory", json!("../escape")), ".."),
        (with("creator", json!("없는 제작자")), "자막 목록에 없는"),
        (with("creator", Value::Null), "제작자를 골라"),
        (with("subtitles", json!("undecided")), "따라 받을 때만"),
        (with("anissia_anime_no", json!(999)), "찾지 못했어요"),
        (with("week", json!(4)), "찾지 못했어요"),
    ] {
        let (status, error) = app
            .call(Method::POST, "/api/subscriptions", Some(body.clone()))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {error}");
        assert!(
            error["message"].as_str().unwrap().contains(message),
            "{body}: {error}"
        );
    }
    let mut elsewhere = good.clone();
    elsewhere["channel_id"] = json!(empty.id);
    let (status, error) = app
        .call(Method::POST, "/api/subscriptions", Some(elsewhere))
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "no history, no title: {error}"
    );
    let (status, _) = app
        .call(
            Method::POST,
            "/api/subscriptions",
            Some(with("channel_id", json!("nope"))),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app
        .call(
            Method::POST,
            "/api/subscriptions",
            Some(json!({ "channel_id": channel.id })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    assert_eq!(app.rules(&channel).await, 0);
    assert_eq!(app.rules(&empty).await, 0);

    // Anissia not answering at the moment of subscribing refuses it too.
    app.fake.state.lock().unwrap().failing = 100;
    app.now.fetch_add(10 * 60 * 1000, Ordering::SeqCst); // the cached schedule is old
    let (status, error) = app
        .call(Method::POST, "/api/subscriptions", Some(good))
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{error}");
    assert_eq!(app.rules(&channel).await, 0);
}

#[tokio::test]
async fn an_anime_is_followed_once_per_channel() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    let channel = app.channel("feed.test").await;
    app.record(&channel, 1000, &[WORK_1, OTHER]).await;
    let body = app.subscribe_body(&channel);
    assert_eq!(
        app.call(Method::POST, "/api/subscriptions", Some(body.clone()))
            .await
            .0,
        StatusCode::CREATED
    );
    let mut again = body;
    again["work"] = json!("Another Show");
    let (status, error) = app
        .call(Method::POST, "/api/subscriptions", Some(again))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert!(error["message"].as_str().unwrap().contains("이미 구독"));
    assert_eq!(app.rules(&channel).await, 1);
}

#[tokio::test]
async fn editing_a_subscription_rule_keeps_its_subscription_and_a_stale_version_conflicts() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    let channel = app.channel("feed.test").await;
    app.record(&channel, 1000, &[WORK_1]).await;
    let (_, created) = app
        .call(
            Method::POST,
            "/api/subscriptions",
            Some(app.subscribe_body(&channel)),
        )
        .await;
    let rule = &created["rule"];
    let id = rule["id"].as_str().unwrap();
    let edit = |version: i64, directory: &str| {
        json!({
            "version": version, "channel_id": channel.id, "match": "Work",
            "regex": false, "case_insensitive": false, "directory": directory, "episode": 1,
        })
    };

    let (status, saved) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{id}"),
            Some(edit(rule["version"].as_i64().unwrap(), "Work/Season 02")),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["directory"], "Work/Season 02");
    assert_eq!(saved["subscription"]["anissia_anime_no"], 3320);
    assert_eq!(saved["subscription"]["creator"], "에텔레로사");

    // Saving with the version seen before is a conflict carrying the current rule.
    let (status, conflict) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{id}"),
            Some(edit(rule["version"].as_i64().unwrap(), "Work/Season 03")),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict["current"]["directory"], "Work/Season 02");
    assert_eq!(
        conflict["current"]["subscription"]["anissia_anime_no"],
        3320
    );
}
