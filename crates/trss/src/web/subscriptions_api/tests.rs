use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

use axum::{
    http::{Method, StatusCode},
    Router,
};
use serde_json::{json, Value};

use trss_core::{Clock, Db};

use super::*;
use crate::web::testing;
use trss_anissia::{fake::Fake, Anissia};
use trss_collect::store::{
    channels::{Channel, ChannelInput},
    history::{HistoryResult, Observation},
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
        App::over(None).await
    }

    /// [`App::new`] whose artwork keeps its images in `covers`' folder and
    /// asks its AniList (subscription covers).
    async fn over(covers: Option<(&std::path::Path, trss_anilist::AnilistConfig)>) -> App {
        let db = Db::open_blocking(":memory:").unwrap();
        let fake = Fake::start().await;
        let now = Arc::new(AtomicI64::new(NOW));
        let clock: Clock = {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        };
        let anissia = Anissia::new(db.clone(), fake.config(), clock).with_spacing(Duration::ZERO);
        let mut state = AppState::new(db.clone()).with_anissia(anissia);
        if let Some((dir, anilist)) = covers {
            state = state.with_artwork(
                trss_library::artwork::Artwork::new(
                    db,
                    Some(trss_library::artwork::AppData::new(dir)),
                    anilist,
                )
                .with_spacing(Duration::ZERO),
            );
        }
        state
            .settings
            .put_collection(0, "/media".to_owned(), None)
            .await
            .unwrap();
        let router = testing::api(&state);
        App {
            state,
            router,
            fake,
            now,
        }
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        testing::call(&self.router, method, uri, body).await
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

    /// The full list `작품 찾기` searches, four to a page: `안녕` finds anime of
    /// Wednesday (3320, on air), Thursday (3005, `결방`) and `신작` (3008), one
    /// Anissia ended (2969) and one whose end date has passed (3002).
    fn catalogue(&self) {
        let entry =
            |week: u8, no: i64, time: &str, subject: &str, start: &str, end: &str, status: &str| {
                let mut e = self.fake.entry(week, no, time, subject, "");
                e["startDate"] = json!(start);
                e["endDate"] = json!(end);
                e["status"] = json!(status);
                e
            };
        self.fake.set_catalogue(vec![
            entry(3, 3320, "22:30", "안녕 수요일", "2026-07-01", "", "ON"),
            self.fake.finished(1, 2969, "안녕, 라라", "Sayonara Lara"),
            entry(
                2,
                3002,
                "23:00",
                "안녕 지난 분기",
                "2026-07-02",
                "2026-09-20",
                "ON",
            ),
            entry(4, 3005, "01:00", "안녕 결방", "2026-07-03", "", "OFF"),
            entry(8, 3008, "", "안녕 다음 분기", "2027-01", "", "ON"),
        ]);
        self.fake.state.lock().unwrap().page_size = 4;
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
    // The run state is trss-collect `schedule::state::run_state`'s: the fake
    // anime starts on 2026-10-07, after the test's today.
    assert_eq!(schedule["entries"][1]["run"], "upcoming");
    assert_eq!(schedule["cached"], false);

    // A second look comes from the short cache.
    let (_, again) = app.get("/api/anissia/schedule/3").await;
    assert_eq!(again["cached"], true);

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

    // Too many requests: told to wait (the block that holds the next ask back
    // is trss-anissia's).
    {
        let mut state = app.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = Some(30);
    }
    let (status, body) = app.get("/api/anissia/schedule/3").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body["message"].as_str().unwrap().contains("30초"), "{body}");
}

fn numbers(entries: &Value) -> Vec<i64> {
    entries
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["anime_no"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn a_search_lists_the_anime_of_every_week_that_have_not_ended_with_their_week() {
    let app = App::new().await;
    app.catalogue();

    let (status, first) = app.get("/api/anissia/search?q=%EC%95%88%EB%85%95").await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["q"], "안녕");
    // Anissia's order, without the anime that ended by Anissia's word or by their end date.
    assert_eq!(numbers(&first["entries"]), [3320, 3005]);
    let wednesday = &first["entries"][0];
    assert_eq!(wednesday["week"], 3);
    assert_eq!(wednesday["air_time"], "22:30");
    assert_eq!(wednesday["start_date"], "2026-07-01");
    assert_eq!(wednesday["run"], "on");
    assert_eq!(wednesday["subscribed_rules"], json!([]));
    assert_eq!(first["entries"][1]["week"], 4);
    assert_eq!(first["entries"][1]["run"], "off");
    assert_eq!(
        (first["page"].as_u64(), first["has_next"].as_bool()),
        (Some(1), Some(true))
    );
    assert_eq!(first["max_page"], 100);
    assert_eq!(first["cached"], false);

    let (_, next) = app
        .get("/api/anissia/search?q=%EC%95%88%EB%85%95&page=2")
        .await;
    assert_eq!(numbers(&next["entries"]), [3008]);
    assert_eq!(next["entries"][0]["week"], 8);
    assert_eq!(next["entries"][0]["run"], "upcoming");
    assert_eq!(
        (next["page"].as_u64(), next["has_next"].as_bool()),
        (Some(2), Some(false))
    );
    // The screen counts pages from 1 and Anissia from 0.
    assert_eq!(app.fake.count("/anime/list/0?q=안녕"), 1);
    assert_eq!(app.fake.count("/anime/list/1?q=안녕"), 1);

    // A page of ended anime only is empty.
    let (_, ended) = app.get("/api/anissia/search?q=%EB%9D%BC%EB%9D%BC").await;
    assert_eq!(numbers(&ended["entries"]), Vec::<i64>::new());
    assert_eq!(ended["has_next"], false);
}

#[tokio::test]
async fn a_search_with_the_ended_ones_keeps_the_anime_that_have_ended() {
    let app = App::new().await;
    app.catalogue();

    let (status, first) = app
        .get("/api/anissia/search?q=%EC%95%88%EB%85%95&ended=true")
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(numbers(&first["entries"]), [3320, 2969, 3002, 3005]);
    let runs: Vec<&str> = first["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["run"].as_str().unwrap())
        .collect();
    assert_eq!(runs, ["on", "end", "end", "off"]);
    assert_eq!(first["entries"][1]["week"], 1);
    assert_eq!(first["entries"][1]["end_date"], "2021-06-27");
    assert_eq!(first["has_next"], true);

    let (_, ended) = app
        .get("/api/anissia/search?q=%EB%9D%BC%EB%9D%BC&ended=true")
        .await;
    assert_eq!(numbers(&ended["entries"]), [2969]);
}

#[tokio::test]
async fn a_search_needs_its_text_and_a_page_in_range_and_its_failure_leaves_the_weeks() {
    let app = App::new().await;
    app.catalogue();
    app.schedule_of_wednesday();
    for uri in [
        "/api/anissia/search",
        "/api/anissia/search?q=%20%20",
        "/api/anissia/search?q=a&page=0",
        "/api/anissia/search?q=a&page=101",
        "/api/anissia/search?q=a&page=two",
    ] {
        let (status, body) = app.get(uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
    }
    assert_eq!(app.fake.count("/anime/"), 0);

    app.fake.state.lock().unwrap().failing = 1;
    let (status, body) = app.get("/api/anissia/search?q=a").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_eq!(body["error"], "unavailable");
    let message = body["message"].as_str().unwrap();
    assert!(message.contains("HTTP 500"), "{message}");
    assert!(
        message.ends_with("편성표에서는 고를 수 있어요."),
        "{message}"
    );
    let (status, _) = app.get("/api/anissia/schedule/3").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn subscribing_to_an_anime_found_by_a_search_checks_it_against_that_page() {
    let app = App::new().await;
    // Wednesday's schedule names 3320 `작품`; the search names it `안녕 수요일`.
    app.schedule_of_wednesday();
    app.catalogue();
    let channel = app.channel("feed.test").await;
    app.record(&channel, 1000, &[WORK_1]).await;
    let mut found = app.subscribe_body(&channel);
    found.as_object_mut().unwrap().remove("week");
    found["q"] = json!("안녕");
    found["page"] = json!(1);
    let with = |key: &str, value: Value| {
        let mut body = found.clone();
        body[key] = value;
        body
    };

    for (body, message) in [
        (
            with("anissia_anime_no", json!(3008)),
            "검색 결과에서 이 작품을 찾지 못했어요",
        ),
        (
            with("page", json!(2)),
            "검색 결과에서 이 작품을 찾지 못했어요",
        ),
        (with("week", json!(3)), "편성표나 검색 결과에서 골라"),
        (with("q", Value::Null), "편성표나 검색 결과에서 골라"),
        (with("q", json!(" ")), "검색어를 입력해"),
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
    assert_eq!(app.rules(&channel).await, 0);

    let (status, body) = app
        .call(Method::POST, "/api/subscriptions", Some(found))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    // The snapshot is the one the search gave.
    let anime = &body["rule"]["subscription"]["anime"];
    assert_eq!(anime["subject"], "안녕 수요일");
    assert_eq!(anime["week"], 3);
    assert_eq!(anime["start_date"], "2026-07-01");
    let (_, again) = app.get("/api/anissia/search?q=%EC%95%88%EB%85%95").await;
    assert_eq!(
        again["entries"][0]["subscribed_rules"][0]["channel_id"],
        channel.id
    );
}

#[tokio::test]
async fn the_creators_of_an_anime_come_from_its_captions_and_may_be_none() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    let (status, body) = app.get("/api/anissia/anime/3320/creators").await;
    assert_eq!(status, StatusCode::OK);
    let creators = body["creators"].as_array().unwrap();
    assert!(creators[0]["name"].is_string(), "{body}");
    assert!(creators[0]["captions"].is_u64(), "{body}");

    let (status, body) = app.get("/api/anissia/anime/3321/creators").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["creators"], json!([]));
}

#[tokio::test]
async fn only_the_works_the_channels_history_holds_are_offered() {
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
    assert_eq!(titles[0]["work"], "Work");
    assert_eq!(titles[0]["items"], 3);
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
}

#[tokio::test]
async fn a_subscription_is_made_with_the_offset_applied_before_receiving() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    let channel = app.channel("feed.test").await;
    app.record(&channel, 1000, &[WORK_1]).await;
    let mut body = app.subscribe_body(&channel);
    body["directory"] = json!("Work/Season 02");
    body["episode"] = json!(-12);

    let (status, body) = app
        .call(Method::POST, "/api/subscriptions", Some(body))
        .await;

    assert_eq!(status, StatusCode::CREATED, "{body}");
    // The person's own value, not one the app set.
    assert_eq!(body["rule"]["episode"], -12);
    assert_eq!(body["rule"]["episode_auto"], false);

    // Without one, the rule leaves numbers as they are.
    let other = app.channel("other.test").await;
    app.record(&other, 1000, &[WORK_1]).await;
    let (status, body) = app
        .call(
            Method::POST,
            "/api/subscriptions",
            Some(app.subscribe_body(&other)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["rule"]["episode"], 1);
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
    assert!(list["quarter"]["year"].is_u64(), "{list}");
    let item = &list["subscriptions"][0];
    assert_eq!(item["title"], "Work");
    assert_eq!(item["subscription"]["anime"]["air_time"], "22:30");
    assert_eq!(item["subscription"]["anime"]["week"], 3);
    assert!(item["quarter"]["number"].is_u64(), "{item}");
    assert_eq!(item["upcoming"], false);
    assert_eq!(item["run"], "upcoming");
    assert_eq!(item["channel_host"], "feed.test");

    // A paused rule is listed with its state; an archived one is deleted.
    let id = item["rule_id"].as_str().unwrap();
    let paused = app
        .state
        .channels
        .set_rule_state(id, RuleState::Paused, 0)
        .await
        .unwrap()
        .unwrap();
    let (_, list) = app.get("/api/subscriptions").await;
    assert_eq!(list["subscriptions"][0]["state"], "paused");
    app.state
        .channels
        .delete_rule(id, paused.version)
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
    assert_eq!(list["subscriptions"][0]["run"], "upcoming");
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
        // The collect folder itself (which paths are, is trss-core `folders`').
        (with("directory", json!(".")), "수집 폴더 자체"),
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
    assert_eq!(
        conflict["current"]["subscription"]["anissia_anime_no"],
        3320
    );
}

/// The rule detail (ticket 2-work-discovery-and-subscriptions/0019): the switches, the creator, `Anissia 연결` and
/// what the rule's view says of its season.
mod rule_detail {
    use std::collections::BTreeSet;

    use super::*;
    use trss_anilist::{Entry, FuzzyDate};
    use trss_collect::store::channels::Rule;
    use trss_library::{
        artwork::{image::samples, queue::Ran},
        discovery::{EpisodeFile, FileKind, Scan, ScannedWork, WorkRead},
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
            korean_titles: Vec::new(),
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

        let stored = app.fresh(&rule).await;
        let (_, on) = app
            .put(&stored, "switch", json!({ "subtitles": true }))
            .await;
        assert_eq!(on["subscription"]["subtitles"], "follow");
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
    async fn a_rule_is_linked_to_an_anime_found_by_a_search_checked_against_that_page() {
        let app = App::new().await;
        app.catalogue();
        let channel = app.channel("feed.test").await;
        let rule = app.plain(&channel, "Off Show").await;
        let post = |body: Value| {
            app.call(
                Method::POST,
                format!("/api/rules/{}/subscription", rule.id).leak(),
                Some(body),
            )
        };
        let link = |no: i64, page: u32| {
            json!({
                "version": rule.version, "anissia_anime_no": no, "q": "안녕", "page": page,
                "subtitles": "undecided",
            })
        };

        let (status, refused) = post(link(3005, 2)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
        assert!(refused["message"].as_str().unwrap().contains("검색 결과"));
        assert!(app.fresh(&rule).await.subscription.is_none());

        let (status, linked) = post(link(3005, 1)).await;
        assert_eq!(status, StatusCode::OK, "{linked}");
        assert_eq!(linked["subscription"]["anime"]["subject"], "안녕 결방");
        assert_eq!(linked["subscription"]["anime"]["week"], 4);
        assert_eq!(linked["match"], "Off Show");
        assert_eq!(linked["directory"], "Off Show/Season 02");
    }

    /// `Anissia 연결` picks from the whole list, so a rule can follow an anime
    /// no week of the schedule lists. It reads as ended, has no card this week
    /// and is suggested for the archive: at once when its end date has passed,
    /// and after the daily refresh found no week listing it when it has none.
    #[tokio::test]
    async fn a_rule_linked_to_an_anime_the_schedule_does_not_list_reads_as_ended() {
        let app = App::new().await;
        app.catalogue();
        let channel = app.channel("feed.test").await;
        let past = app.plain(&channel, "Past Show").await;
        let dateless = app.plain(&channel, "Off Show").await;
        for (rule, no) in [(&past, 3002), (&dateless, 3005)] {
            let (status, linked) = app
                .call(
                    Method::POST,
                    format!("/api/rules/{}/subscription", rule.id).leak(),
                    Some(json!({
                        "version": rule.version, "anissia_anime_no": no, "q": "안녕", "page": 1,
                        "subtitles": "none",
                    })),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{linked}");
        }
        let runs = || async {
            let (_, list) = app.get("/api/subscriptions").await;
            list["subscriptions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| (s["title"].as_str().unwrap().to_owned(), s["run"].clone()))
                .collect::<Vec<_>>()
        };
        let cards = || async {
            // The home screen's week, which it shows once the first run's checklist is done.
            let week = crate::web::schedule_api::week_at(&app.state, NOW)
                .await
                .unwrap();
            serde_json::to_value(week).unwrap()["days"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|d| d["cards"].as_array().unwrap().clone())
                .map(|c| c["anime_no"].as_i64().unwrap())
                .collect::<Vec<_>>()
        };
        let suggested = || async {
            let (_, list) = app.get("/api/archive-suggestions").await;
            list["suggestions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| {
                    let ground = &s["grounds"][0];
                    (
                        s["rule_id"].as_str().unwrap().to_owned(),
                        ground["kind"].as_str().unwrap().to_owned(),
                    )
                })
                .collect::<Vec<_>>()
        };

        // 3002 (Tuesday) ended on 2026-09-20; 3005 (Thursday, `결방`) has no end date.
        assert_eq!(
            runs().await,
            [
                ("Past Show".to_owned(), json!("end")),
                ("Off Show".to_owned(), json!("off")),
            ]
        );
        assert_eq!(cards().await, [3005]);
        assert_eq!(suggested().await, [(past.id.clone(), "ended".to_owned())]);

        // The daily refresh asked every week and none listed 3005.
        app.state
            .anissia_store
            .mark_unlisted(vec![3005], NOW, NOW + 24 * 60 * 60 * 1000, NOW + 1)
            .await
            .unwrap();
        assert_eq!(
            runs().await,
            [
                ("Past Show".to_owned(), json!("end")),
                ("Off Show".to_owned(), json!("end")),
            ]
        );
        assert_eq!(cards().await, Vec::<i64>::new());
        let mut both = suggested().await;
        both.sort();
        let mut expected = vec![
            (past.id.clone(), "ended".to_owned()),
            (dateless.id.clone(), "unlisted".to_owned()),
        ];
        expected.sort();
        assert_eq!(both, expected);
    }

    #[tokio::test]
    async fn a_rule_that_saves_into_the_collect_folder_itself_is_not_linked() {
        let app = App::new().await;
        app.schedule_of_wednesday();
        let channel = app.channel("feed.test").await;
        // Which paths are the collect folder itself is trss-core `folders`'.
        let rule = app
            .state
            .channels
            .create_rule(
                &channel.id,
                RuleInput {
                    r#match: Some("Work".into()),
                    directory: String::new(),
                    ..RuleInput::default()
                },
            )
            .await
            .unwrap();
        let (status, body) = app
            .call(
                Method::POST,
                format!("/api/rules/{}/subscription", rule.id).leak(),
                Some(json!({
                    "version": rule.version, "anissia_anime_no": 3320, "week": 3,
                    "subtitles": "undecided",
                })),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(app.fresh(&rule).await.subscription.is_none());
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
        // Season 1 is linked to the subscription's anime, whose title names the work.
        assert_eq!(view["season"]["work_name"], "작품");
        assert_eq!(view["season"]["number"], 1);
        assert_eq!(view["season"]["videos"], 9);
        assert_eq!(view["season"]["episodes"], Value::Null, "unknown count");
        assert_eq!(view["season"]["cover_url"], Value::Null);
        assert_eq!(view["season_blocked"], Value::Null);

        // With the AniList entries' episode count known, the progress has a total
        // (how entries combine into a season's count is trss-library `combine`'s).
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
    }

    // feedback-0.8.0/0008: 구독은 AniList를 기다리지 않고 바로 만들어지고, 시즌에 붙기 전의 편성 카드와
    // 규칙의 포스터는 구독이 찾은 표지를, 붙은 뒤에는 작품의 표지를 보여줘요.
    #[tokio::test]
    async fn a_subscription_shows_its_own_cover_until_its_season_has_the_work_s() {
        let dir = tempfile::tempdir().unwrap();
        let anilist = trss_anilist::fake::Fake::start().await;
        let app = App::over(Some((dir.path(), anilist.config()))).await;
        // AniList fails: subscribing neither asks it nor waits for it.
        anilist.state.lock().unwrap().failing = 100;
        let (_, rule) = app.subscribed().await;
        assert!(anilist.api_requests().is_empty());
        let detail = || async {
            let (status, view) = app.get(&format!("/api/rules/{}", rule.id)).await;
            assert_eq!(status, StatusCode::OK, "{view}");
            view
        };
        // A week in which the anime (from Wednesday 2026-10-07) airs.
        let card = || async {
            let week = crate::web::schedule_api::week_at(&app.state, NOW + 7 * 86_400_000)
                .await
                .unwrap();
            serde_json::to_value(week).unwrap()["days"][2]["cards"][0].clone()
        };
        assert_eq!(detail().await["subscription"]["cover_url"], Value::Null);
        assert_eq!(card().await["rule_id"], rule.id.as_str());
        assert_eq!(card().await["cover_url"], Value::Null);

        // The queue finds the anime's one entry once AniList answers.
        let mut entry = anilist.entry(77, "Work", &[]);
        entry["title"]["native"] = json!("Work Original");
        entry["startDate"] = json!({ "year": 2026, "month": 10, "day": 7 });
        anilist.add_search("Work Original", vec![entry], &samples::jpeg());
        anilist.state.lock().unwrap().failing = 0;
        assert_eq!(app.state.artwork.run_next().await, Some(Ran::Recorded));
        assert_eq!(app.state.artwork.run_next().await, Some(Ran::Recorded));
        let image = app
            .state
            .artwork
            .store
            .subscription_image(&rule.id)
            .await
            .unwrap()
            .unwrap();
        let own = format!("/api/rules/{}/cover?v={}", rule.id, image.id);
        assert_eq!(detail().await["subscription"]["cover_url"], own.as_str());
        assert_eq!(card().await["cover_url"], own.as_str());
        let (_, list) = app.get("/api/subscriptions").await;
        assert_eq!(
            list["subscriptions"][0]["subscription"]["cover_url"],
            own.as_str()
        );
        let (status, bytes) =
            testing::read(&app.router, testing::request(Method::GET, &own, None)).await;
        assert_eq!((status, bytes.to_vec()), (StatusCode::OK, samples::jpeg()));

        // Connected to a season of a work whose cover has nothing yet: the work
        // takes the image, and both show the work's cover.
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
        let work = app.state.library.works(&folder.id).await.unwrap()[0]
            .id
            .clone();
        app.state
            .channels
            .link_season(&rule.id, &format!("{work}:1"))
            .await
            .unwrap();
        let theirs = format!("/api/library/works/{work}/artwork/image?v={}", image.id);
        let view = detail().await;
        assert_eq!(view["subscription"]["cover_url"], Value::Null);
        assert_eq!(view["season"]["cover_url"], theirs.as_str());
        assert_eq!(card().await["cover_url"], theirs.as_str());
        let (status, _) =
            testing::read(&app.router, testing::request(Method::GET, &own, None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// Two subscriptions of different anime whose videos are in one season:
    /// the first holds it and the second is noted as blocked by it. The holder
    /// first, then the blocked one.
    async fn blocked_pair(app: &App) -> (Rule, Rule) {
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
        (holder, blocked)
    }

    #[tokio::test]
    async fn the_view_says_which_anime_holds_the_season_that_kept_a_rule_unconnected() {
        let app = App::new().await;
        let (_holder, blocked) = blocked_pair(&app).await;

        let (_, view) = app.get(&format!("/api/rules/{}", blocked.id)).await;
        assert_eq!(view["season"], Value::Null);
        // The holder's anime names the work through season 1.
        assert_eq!(view["season_blocked"]["work_name"], "작품");
        assert_eq!(view["season_blocked"]["number"], 1);
        assert_eq!(view["season_blocked"]["holder_anime_no"], 3320);
        assert_eq!(view["season_blocked"]["holder_subject"], "작품");
    }

    #[tokio::test]
    async fn the_view_does_not_explain_a_block_that_nothing_holds_any_more() {
        let app = App::new().await;
        let (holder, blocked) = blocked_pair(&app).await;
        let (_, view) = app.get(&format!("/api/rules/{}", blocked.id)).await;
        assert_eq!(view["season_blocked"]["held_by"], "subscription");
        let season = view["season_blocked"].clone();
        // The worker has not cleared the note yet, but the holder is gone. The
        // season keeps the holder's anime as its own link, which still holds it.
        let holder = app.fresh(&holder).await;
        app.state
            .channels
            .delete_rule(&holder.id, holder.version)
            .await
            .unwrap();
        let (_, view) = app.get(&format!("/api/rules/{}", blocked.id)).await;
        assert_eq!(view["season"], Value::Null);
        assert_eq!(view["season_blocked"]["holder_anime_no"], 3320);
        assert_eq!(view["season_blocked"]["held_by"], "link");

        // Once the link is cut, nothing holds the season.
        let uri = format!(
            "/api/library/works/{}/seasons/1/anissia/link",
            season["work_id"].as_str().unwrap()
        );
        let (status, cut) = app
            .call(
                Method::POST,
                &uri,
                Some(json!({ "version": 1, "anime_no": null })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{cut}");
        let (_, view) = app.get(&format!("/api/rules/{}", blocked.id)).await;
        assert_eq!(view["season"], Value::Null);
        assert_eq!(view["season_blocked"], Value::Null);
    }
}

#[path = "title_tests.rs"]
mod title_waiting;

#[tokio::test]
async fn subscribing_to_a_work_in_the_archive_folder_makes_the_rule_paused_with_its_start_open() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    let channel = app.channel("feed.test").await;
    app.record(&channel, 1000, &[WORK_1]).await;
    let tmp = tempfile::tempdir().unwrap();
    let (collect, archive) = (tmp.path().join("Shows (current)"), tmp.path().join("Shows"));
    std::fs::create_dir(&collect).unwrap();
    std::fs::create_dir_all(archive.join("Work/Season 01")).unwrap();
    app.state
        .settings
        .put_collection(
            1,
            collect.display().to_string(),
            Some(archive.display().to_string()),
        )
        .await
        .unwrap();

    let (status, body) = app
        .call(
            Method::POST,
            "/api/subscriptions",
            Some(app.subscribe_body(&channel)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let rule = &body["rule"];
    // Still a subscription, off until the worker has moved the work folder.
    assert_eq!(rule["state"], "paused");
    assert_eq!(rule["subscription"]["anissia_anime_no"], 3320);
    assert_eq!(rule["archive_move"]["direction"], "start");
    assert_eq!(rule["archive_move"]["command"]["state"], "pending");
}

#[tokio::test]
async fn a_subscription_that_waits_for_its_work_folder_carries_the_ticked_items_in_its_start() {
    let app = App::new().await;
    app.schedule_of_wednesday();
    let channel = app.channel("feed.test").await;
    app.record(&channel, 1000, &[WORK_1, WORK_2, WORK_3]).await;
    let ids: std::collections::HashMap<String, i64> =
        crate::web::rules_api::channel_items(&app.state.history, &channel.id)
            .await
            .unwrap()
            .into_iter()
            .map(|item| (item.title, item.id))
            .collect();
    let (two, three) = (ids[WORK_2], ids[WORK_3]);
    let tmp = tempfile::tempdir().unwrap();
    let (collect, archive) = (tmp.path().join("Shows (current)"), tmp.path().join("Shows"));
    std::fs::create_dir(&collect).unwrap();
    std::fs::create_dir_all(archive.join("Work/Season 01")).unwrap();
    app.state
        .settings
        .put_collection(
            1,
            collect.display().to_string(),
            Some(archive.display().to_string()),
        )
        .await
        .unwrap();
    let ticked = |receive: Value| {
        let mut body = app.subscribe_body(&channel);
        body["receive"] = receive;
        body
    };

    // An item the channel's history does not hold is refused, and nothing is made.
    let (status, body) = app
        .call(
            Method::POST,
            "/api/subscriptions",
            Some(ticked(json!([two, 999_999]))),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(app.rules(&channel).await, 0);

    let (status, body) = app
        .call(
            Method::POST,
            "/api/subscriptions",
            Some(ticked(json!([three, two]))),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let rule = &body["rule"];
    assert_eq!(rule["state"], "paused");
    // The start carries them in the order ticked, for the worker to receive
    // once the rule is on.
    let start = app
        .state
        .commands
        .get(rule["archive_move"]["command"]["id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    let payload: Value = serde_json::from_str(&start.payload).unwrap();
    assert_eq!(payload["rule_id"], rule["id"]);
    assert_eq!(payload["receive"], json!([three, two]));

    // A subscription that need not wait leaves them to the screen, which
    // receives them itself: nothing is stored for them.
    let other = app.channel("other.test").await;
    app.record(&other, 1000, &[WORK_1]).await;
    let mut body = app.subscribe_body(&other);
    body["directory"] = json!("Another/Season 01");
    body["receive"] = json!([ids[WORK_1]]);
    let (status, body) = app
        .call(Method::POST, "/api/subscriptions", Some(body))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["rule"]["state"], "active");
    assert_eq!(body["rule"]["archive_move"], Value::Null);
    let receives = app
        .state
        .commands
        .open_for_subjects(
            trss_collect::commands::receive_once::KIND,
            vec![ids[WORK_1].to_string()],
        )
        .await
        .unwrap();
    assert!(receives.is_empty());
}
