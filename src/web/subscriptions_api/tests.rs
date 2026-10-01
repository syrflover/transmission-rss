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

    // The past items the user may pick are the ones the rule matches: the
    // stored rule is a subscription now, so they read as past, not as taken.
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
    assert_eq!(preview["counts"]["past"], 3);
    assert_eq!(preview["counts"]["mine"], 0);
    assert!(preview["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["kind"] == "past" && i["stored_result"] == "no_match"));
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
        // Folders that are the collect folder itself.
        (with("directory", json!(".")), "수집 폴더 자체"),
        (with("directory", json!("./")), "수집 폴더 자체"),
        (with("directory", json!(" ./. ")), "수집 폴더 자체"),
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
    let (status, created) = app
        .call(Method::POST, "/api/subscriptions", Some(body.clone()))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let mut again = body;
    again["work"] = json!("Another Show");
    let (status, error) = app
        .call(Method::POST, "/api/subscriptions", Some(again))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert!(error["message"].as_str().unwrap().contains("이미 구독"));
    // The answer names the rule that follows it, for the screen to open.
    assert_eq!(
        error["current"]["rule_id"], created["rule"]["id"],
        "{error}"
    );
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

/// The rule detail (ticket 0019): the switches, the creator, `편성표와 연결` and
/// what the rule's view says of its season.
mod rule_detail {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{
        discovery::{EpisodeFile, FileKind, Scan, ScannedWork, WorkRead},
        store::{
            channels::Rule,
            seasons::{Entry, FuzzyDate},
        },
    };

    impl App {
        /// A subscription to 3320 on a fresh channel, with the schedule up.
        async fn subscribed(&self) -> (Channel, Rule) {
            self.schedule_of_wednesday();
            let channel = self.channel("feed.test").await;
            self.record(&channel, 1000, &[WORK_1]).await;
            let (status, body) = self
                .call(
                    Method::POST,
                    "/api/subscriptions",
                    Some(self.subscribe_body(&channel)),
                )
                .await;
            assert_eq!(status, StatusCode::CREATED, "{body}");
            let id = body["rule"]["id"].as_str().unwrap().to_owned();
            let rule = self.state.channels.get_rule(&id).await.unwrap().unwrap();
            (channel, rule)
        }

        async fn put(&self, rule: &Rule, path: &str, mut body: Value) -> (StatusCode, Value) {
            body["version"] = json!(rule.version);
            self.call(
                Method::PUT,
                &format!("/api/rules/{}/{path}", rule.id),
                Some(body),
            )
            .await
        }

        async fn fresh(&self, rule: &Rule) -> Rule {
            self.state
                .channels
                .get_rule(&rule.id)
                .await
                .unwrap()
                .unwrap()
        }

        /// A plain rule (no subscription) in `channel`.
        async fn plain(&self, channel: &Channel, phrase: &str) -> Rule {
            self.state
                .channels
                .create_rule(
                    &channel.id,
                    RuleInput {
                        r#match: Some(phrase.into()),
                        directory: format!("{phrase}/Season 02"),
                        episode: -12,
                        ..RuleInput::default()
                    },
                )
                .await
                .unwrap()
        }
    }

    fn work_with_videos(count: usize) -> ScannedWork {
        ScannedWork {
            dir_name: "Work".into(),
            seasons: BTreeSet::from([1]),
            files: (1..=count)
                .map(|n| EpisodeFile {
                    path: format!("Season 01/Work S01E{n:02}.mkv"),
                    kind: FileKind::Video,
                    season: 1,
                    episode: format!("{n:02}"),
                })
                .collect(),
            unrecognized: Vec::new(),
        }
    }

    fn entry(id: i64, episodes: Option<u32>) -> Entry {
        Entry {
            id,
            romaji: Some("Work".into()),
            english: None,
            native: Some("ワーク".into()),
            format: Some("TV".into()),
            status: Some("RELEASING".into()),
            episodes,
            start: FuzzyDate {
                year: Some(2026),
                month: Some(10),
                day: None,
            },
            end: FuzzyDate::default(),
            studios: Vec::new(),
            genres: Vec::new(),
            description: None,
            airing: Vec::new(),
            sequels: Vec::new(),
            fetched_at: 1,
        }
    }

    #[tokio::test]
    async fn video_receiving_pauses_the_rule_and_leaves_its_folder_and_subscription() {
        let app = App::new().await;
        let (_, rule) = app.subscribed().await;

        let (status, off) = app.put(&rule, "switch", json!({ "video": false })).await;
        assert_eq!(status, StatusCode::OK, "{off}");
        assert_eq!(off["state"], "paused");
        assert_eq!(off["directory"], rule.directory);
        assert_eq!(off["subscription"]["anissia_anime_no"], 3320);
        assert_eq!(off["subscription"]["subtitles"], "follow");
        // A paused subscription still lists, marked as paused.
        let (_, list) = app.get("/api/subscriptions").await;
        assert_eq!(list["subscriptions"][0]["state"], "paused");

        // Pausing notes no resume; turning it back on notes when.
        let stored = app.fresh(&rule).await;
        assert_eq!(stored.resumed_at, None);
        app.now.fetch_add(60_000, Ordering::SeqCst);
        let (status, on) = app.put(&stored, "switch", json!({ "video": true })).await;
        assert_eq!(status, StatusCode::OK, "{on}");
        assert_eq!(on["state"], "active");
        assert_eq!(app.fresh(&rule).await.resumed_at, Some(NOW + 60_000));
        let (_, list) = app.get("/api/subscriptions").await;
        assert_eq!(list["subscriptions"][0]["state"], "active");
    }

    #[tokio::test]
    async fn subtitle_receiving_is_off_none_and_on_follows_the_kept_creator() {
        let app = App::new().await;
        let (_, rule) = app.subscribed().await;

        let (status, off) = app
            .put(&rule, "switch", json!({ "subtitles": false }))
            .await;
        assert_eq!(status, StatusCode::OK, "{off}");
        assert_eq!(off["subscription"]["subtitles"], "none");
        assert_eq!(off["subscription"]["creator"], "에텔레로사");
        assert_eq!(off["state"], "active");

        let stored = app.fresh(&rule).await;
        let (_, on) = app
            .put(&stored, "switch", json!({ "subtitles": true }))
            .await;
        assert_eq!(on["subscription"]["subtitles"], "follow");
        assert_eq!(on["subscription"]["creator"], "에텔레로사");
    }

    #[tokio::test]
    async fn a_switch_with_an_old_version_is_a_conflict_with_the_current_rule() {
        let app = App::new().await;
        let (_, rule) = app.subscribed().await;
        let (_, first) = app.put(&rule, "switch", json!({ "video": false })).await;

        // The same, old version again: nothing changes and the answer is the rule now.
        let (status, stale) = app.put(&rule, "switch", json!({ "video": true })).await;
        assert_eq!(status, StatusCode::CONFLICT, "{stale}");
        assert_eq!(stale["current"]["version"], first["version"]);
        assert_eq!(stale["current"]["state"], "paused");
        assert_eq!(app.fresh(&rule).await.state, RuleState::Paused);

        let (status, stale) = app
            .put(&rule, "switch", json!({ "subtitles": false }))
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    }

    #[tokio::test]
    async fn the_switches_refuse_what_the_detail_disables() {
        let app = App::new().await;
        let (channel, rule) = app.subscribed().await;

        // Both at once, or none, is no request.
        let (status, _) = app
            .put(&rule, "switch", json!({ "video": true, "subtitles": true }))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = app.put(&rule, "switch", json!({})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Subtitles wait for the video switch.
        let (_, paused) = app.put(&rule, "switch", json!({ "video": false })).await;
        let stored = app.fresh(&rule).await;
        assert_eq!(stored.version, paused["version"]);
        let (status, body) = app
            .put(&stored, "switch", json!({ "subtitles": false }))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["message"], "영상 받기를 켜야 자막을 받을 수 있어요.");

        // A plain rule has the video switch only.
        let plain = app.plain(&channel, "Plain").await;
        let (status, body) = app
            .put(&plain, "switch", json!({ "subtitles": false }))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let (status, body) = app.put(&plain, "switch", json!({ "video": false })).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["state"], "paused");
        assert_eq!(body["subscription"], Value::Null);

        // An archived rule is restored, not switched.
        let archived = app
            .state
            .channels
            .set_rule_state(&rule.id, RuleState::Archived, 0)
            .await
            .unwrap()
            .unwrap();
        let (status, body) = app.put(&archived, "switch", json!({ "video": true })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let (status, _) = app
            .put(&archived, "switch", json!({ "subtitles": true }))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(app.fresh(&rule).await.state, RuleState::Archived);

        // A rule that is not there.
        let mut gone = rule.clone();
        gone.id = "no-such-rule".into();
        let (status, _) = app.put(&gone, "switch", json!({ "video": true })).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_creator_is_changed_to_one_of_the_anime_or_to_undecided() {
        let app = App::new().await;
        let (_, rule) = app.subscribed().await;

        let (status, body) = app
            .put(&rule, "creator", json!({ "creator": "다른 제작자" }))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["subscription"]["creator"], "다른 제작자");
        assert_eq!(body["subscription"]["subtitles"], "follow");

        // A stale version conflicts and carries the rule as it is.
        let (status, stale) = app
            .put(&rule, "creator", json!({ "creator": "에텔레로사" }))
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{stale}");
        assert_eq!(stale["current"]["subscription"]["creator"], "다른 제작자");

        // Only a creator the anime's captions name.
        let stored = app.fresh(&rule).await;
        let (status, _) = app
            .put(&stored, "creator", json!({ "creator": "아무개" }))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = app.put(&stored, "creator", json!({ "creator": "" })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, undecided) = app
            .put(&stored, "creator", json!({ "creator": null }))
            .await;
        assert_eq!(status, StatusCode::OK, "{undecided}");
        assert_eq!(undecided["subscription"]["creator"], Value::Null);
        assert_eq!(undecided["subscription"]["subtitles"], "undecided");
    }

    #[tokio::test]
    async fn the_creator_is_not_changed_without_subtitles_or_a_subscription() {
        let app = App::new().await;
        let (channel, rule) = app.subscribed().await;
        let plain = app.plain(&channel, "Plain").await;
        let (status, _) = app
            .put(&plain, "creator", json!({ "creator": "에텔레로사" }))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (_, off) = app
            .put(&rule, "switch", json!({ "subtitles": false }))
            .await;
        let stored = app.fresh(&rule).await;
        assert_eq!(stored.version, off["version"]);
        let (status, _) = app
            .put(&stored, "creator", json!({ "creator": "다른 제작자" }))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn an_existing_rule_is_linked_to_the_schedule_keeping_phrase_folder_and_order() {
        let app = App::new().await;
        app.schedule_of_wednesday();
        let channel = app.channel("feed.test").await;
        let first = app.plain(&channel, "First").await;
        let rule = app.plain(&channel, "Work 1080").await;
        let link = |version: i64, creator: &str| {
            json!({
                "version": version, "anissia_anime_no": 3320, "week": 3,
                "subtitles": "follow", "creator": creator,
            })
        };
        let post = |rule: &Rule, body: Value| {
            app.call(
                Method::POST,
                format!("/api/rules/{}/subscription", rule.id).leak(),
                Some(body),
            )
        };

        // The creator must be one of the anime's.
        let (status, _) = post(&rule, link(rule.version, "아무개")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(app.fresh(&rule).await.subscription.is_none());
        // The anime must be in the week's schedule.
        let mut elsewhere = link(rule.version, "에텔레로사");
        elsewhere["anissia_anime_no"] = json!(9999);
        let (status, _) = post(&rule, elsewhere).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, linked) = post(&rule, link(rule.version, "에텔레로사")).await;
        assert_eq!(status, StatusCode::OK, "{linked}");
        assert_eq!(linked["match"], "Work 1080");
        assert_eq!(linked["directory"], "Work 1080/Season 02");
        assert_eq!(linked["episode"], -12);
        assert_eq!(linked["order"], 2);
        assert_eq!(linked["state"], "active");
        assert_eq!(linked["subscription"]["anissia_anime_no"], 3320);
        assert_eq!(linked["subscription"]["creator"], "에텔레로사");
        assert_eq!(linked["subscription"]["quarter"]["number"], 4);
        assert_eq!(app.fresh(&first).await.version, first.version);

        // It is a subscription now, and the schedule says who follows the anime.
        let (_, schedule) = app.get("/api/anissia/schedule/3").await;
        assert_eq!(
            schedule["entries"][1]["subscribed_rules"][0]["rule_id"],
            rule.id
        );
        let stored = app.fresh(&rule).await;
        let (status, _) = post(&stored, link(stored.version, "에텔레로사")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "linked once");

        // The channel keeps one rule per anime; an old version conflicts.
        let (status, _) = post(&first, link(first.version, "에텔레로사")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, stale) = post(&first, link(first.version + 5, "에텔레로사")).await;
        assert_eq!(status, StatusCode::CONFLICT, "{stale}");
        assert_eq!(stale["current"]["id"], first.id);

        // Undecided needs no creator.
        let (status, undecided) = post(
            &first,
            json!({ "version": first.version, "anissia_anime_no": 3321, "week": 3,
                    "subtitles": "undecided" }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{undecided}");
        assert_eq!(undecided["subscription"]["subtitles"], "undecided");
    }

    #[tokio::test]
    async fn a_paused_rule_matches_nothing_in_the_preview_and_does_not_shadow_a_later_one() {
        let app = App::new().await;
        let (channel, rule) = app.subscribed().await;
        let later = app.plain(&channel, "Work").await;
        app.record(&channel, 2000, &[WORK_2]).await;
        let preview = |edited: &Rule| {
            app.call(
                Method::POST,
                "/api/rules/preview",
                Some(json!({
                    "channel_id": channel.id, "rule_id": edited.id,
                    "rule": { "match": "Work", "directory": "x", "episode": 0 },
                })),
            )
        };

        // While the subscription collects it takes both items before the later rule.
        let (_, view) = preview(&later).await;
        assert_eq!(view["counts"]["earlier"], 2, "{view}");

        let (_, paused) = app.put(&rule, "switch", json!({ "video": false })).await;
        assert_eq!(paused["state"], "paused");
        let (_, view) = preview(&later).await;
        assert_eq!(view["counts"]["mine"], 2, "{view}");
        assert_eq!(view["counts"]["earlier"], 0, "{view}");
        // The paused rule's own preview shows what it would do once on: both
        // items were recorded before the subscription, so they are past.
        let (_, view) = preview(&rule).await;
        assert_eq!(view["counts"]["past"], 2, "{view}");
    }

    #[tokio::test]
    async fn the_view_of_a_connected_subscription_has_its_season_and_progress() {
        let app = App::new().await;
        let (_, rule) = app.subscribed().await;
        let (folder, _) = app
            .state
            .library
            .add_folder(
                "/c".into(),
                Scan {
                    works: vec![WorkRead::Read(work_with_videos(9))],
                },
                100,
                &[],
            )
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
        app.state
            .channels
            .link_season(&rule.id, &format!("{work}:1"))
            .await
            .unwrap();
        let (_, view) = app.get(&format!("/api/rules/{}", rule.id)).await;
        assert_eq!(view["season"]["work_id"], work.as_str());
        assert_eq!(view["season"]["work_name"], "Work");
        assert_eq!(view["season"]["number"], 1);
        assert_eq!(view["season"]["videos"], 9);
        assert_eq!(view["season"]["episodes"], Value::Null, "unknown count");
        assert_eq!(view["season"]["cover_url"], Value::Null);
        assert_eq!(view["season_blocked"], Value::Null);

        // With the AniList entries' episode count known, the progress has a total.
        app.state
            .seasons
            .store
            .put_entry(entry(1, Some(12)))
            .await
            .unwrap();
        let link = app.state.seasons.store.link(&work, 1).await.unwrap();
        app.state
            .seasons
            .set_links(&work, 1, link.version, vec![1])
            .await
            .unwrap();
        let (_, view) = app.get(&format!("/api/rules/{}", rule.id)).await;
        assert_eq!(view["season"]["episodes"], 12);
        assert_eq!(view["season"]["videos"], 9);

        // An entry without a count makes the whole season's count unknown.
        app.state
            .seasons
            .store
            .put_entry(entry(2, None))
            .await
            .unwrap();
        let link = app.state.seasons.store.link(&work, 1).await.unwrap();
        app.state
            .seasons
            .set_links(&work, 1, link.version, vec![1, 2])
            .await
            .unwrap();
        let (_, view) = app.get(&format!("/api/rules/{}", rule.id)).await;
        assert_eq!(view["season"]["episodes"], Value::Null);

        // The work's head: the Anissia title and the subscription of the season.
        let (status, detail) = app.get(&format!("/api/library/works/{work}")).await;
        assert_eq!(status, StatusCode::OK, "{detail}");
        assert_eq!(detail["korean_title"], "작품");
        assert_eq!(detail["subscriptions"][0]["season"], 1);
        assert_eq!(detail["subscriptions"][0]["rule_id"], rule.id.as_str());
        assert_eq!(detail["subscriptions"][0]["creator"], "에텔레로사");
        assert_eq!(
            detail["subscriptions"][0]["rule_version"],
            app.fresh(&rule).await.version
        );

        // Changing the creator in the rule detail shows in the work's head.
        let current = app.fresh(&rule).await;
        let (status, _) = app
            .put(&current, "creator", json!({ "creator": "다른 제작자" }))
            .await;
        assert_eq!(status, StatusCode::OK);
        let (_, detail) = app.get(&format!("/api/library/works/{work}")).await;
        assert_eq!(detail["subscriptions"][0]["creator"], "다른 제작자");
    }

    #[tokio::test]
    async fn a_work_without_a_connected_subscription_has_no_korean_title() {
        let app = App::new().await;
        let (_, _rule) = app.subscribed().await;
        let (folder, _) = app
            .state
            .library
            .add_folder(
                "/c".into(),
                Scan {
                    works: vec![WorkRead::Read(work_with_videos(1))],
                },
                100,
                &[],
            )
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
        let (_, detail) = app.get(&format!("/api/library/works/{work}")).await;
        assert_eq!(detail["korean_title"], Value::Null);
        assert_eq!(detail["subscriptions"], json!([]));
    }

    #[tokio::test]
    async fn the_view_says_which_anime_holds_the_season_that_kept_a_rule_unconnected() {
        let app = App::new().await;
        let (channel, holder) = app.subscribed().await;
        let blocked = app
            .state
            .channels
            .create_subscription_rule(
                &channel.id,
                RuleInput {
                    r#match: Some("Bare".into()),
                    directory: "Bare/Season 01".into(),
                    ..RuleInput::default()
                },
                NewSubscription {
                    anime: Anime {
                        anime_no: 3321,
                        subject: "빈 작품".into(),
                        original_subject: None,
                        week: 3,
                        air_time: None,
                        start_date: None,
                        end_date: None,
                        status: "ON".into(),
                        fetched_at: 1,
                    },
                    subtitles: SubtitleMode::Undecided,
                    creator: None,
                    subscribed_at: 1,
                },
            )
            .await
            .unwrap();
        let (folder, _) = app
            .state
            .library
            .add_folder(
                "/c".into(),
                Scan {
                    works: vec![WorkRead::Read(work_with_videos(2))],
                },
                100,
                &[],
            )
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
        let season = format!("{work}:1");
        app.state
            .channels
            .link_season(&holder.id, &season)
            .await
            .unwrap();
        app.state
            .channels
            .link_season(&blocked.id, &season)
            .await
            .unwrap();

        let (_, view) = app.get(&format!("/api/rules/{}", blocked.id)).await;
        assert_eq!(view["season"], Value::Null);
        assert_eq!(view["season_blocked"]["work_name"], "Work");
        assert_eq!(view["season_blocked"]["number"], 1);
        assert_eq!(view["season_blocked"]["holder_anime_no"], 3320);
        assert_eq!(view["season_blocked"]["holder_subject"], "작품");
    }
}
