use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::{ApiError, AppState};
use trss_anissia::{fake::Fake, Anissia};
use trss_collect::store::channels::{ChannelInput, NewSubscription, RuleInput, SubtitleMode};
use trss_core::{Clock, Db};
use trss_library::discovery::{Scan, ScannedWork, WorkRead};

const NOW: i64 = 1_790_780_400_000;

struct App {
    db: Db,
    state: AppState,
    router: Router,
    fake: Fake,
    now: Arc<AtomicI64>,
    /// The work `Sayonara Lara` with seasons 1 and 2.
    work: String,
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
        let state = AppState::new(db.clone()).with_anissia(anissia);
        let scan = Scan {
            works: vec![WorkRead::Read(ScannedWork {
                dir_name: "Sayonara Lara".into(),
                seasons: BTreeSet::from([1, 2]),
                files: Vec::new(),
                unrecognized: Vec::new(),
            })],
        };
        let (folder, _) = state
            .library
            .add_folder("/c".into(), scan, 1, &[])
            .await
            .unwrap();
        let work = state.library.works(&folder.id).await.unwrap().remove(0).id;
        let router = Router::new().nest("/api", crate::api::router().with_state(state.clone()));
        App {
            db,
            state,
            router,
            fake,
            now,
            work,
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
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn path(&self, season: u32, tail: &str) -> String {
        format!(
            "/api/library/works/{}/seasons/{season}/anissia{tail}",
            self.work
        )
    }

    async fn get(&self, season: u32) -> Value {
        let (status, body) = self.call(Method::GET, &self.path(season, ""), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn search(&self, season: u32, body: Value) -> (StatusCode, Value) {
        self.call(Method::POST, &self.path(season, "/search"), Some(body))
            .await
    }

    async fn link(&self, season: u32, body: Value) -> (StatusCode, Value) {
        self.call(Method::POST, &self.path(season, "/link"), Some(body))
            .await
    }

    /// The `seasons[].anissia` of the work detail.
    async fn detail(&self, season: u32) -> Value {
        let (status, body) = self
            .call(
                Method::GET,
                &format!("/api/library/works/{}", self.work),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["seasons"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["number"] == season)
            .unwrap()["anissia"]
            .clone()
    }

    /// The full list: a finished show the schedule no longer has, and others.
    fn catalogue(&self) {
        self.fake.set_catalogue(vec![
            self.fake
                .entry(1, 3441, "21:30", "안녕하세요 마녀입니다", "Majo Desu"),
            self.fake.finished(1, 2969, "안녕, 라라", "Sayonara Lara"),
            self.fake
                .finished(0, 1900, "안녕, 나의 크라머", "Sayonara Cramer"),
            self.fake.finished(2, 77, "전혀 다른 작품", "Another"),
        ]);
    }

    fn requests_for(&self, part: &str) -> usize {
        self.fake.count(part)
    }
}

fn numbers(page: &Value) -> Vec<i64> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["anime_no"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn a_finished_show_is_found_by_the_folder_name_and_saved_as_the_seasons_link() {
    let app = App::new().await;
    app.catalogue();
    assert_eq!(app.get(1).await["version"], 0);
    assert_eq!(app.get(1).await["anime"], Value::Null);

    // No text given: the work's folder name is what is searched for.
    let (status, found) = app.search(1, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["q"], "Sayonara Lara");
    assert_eq!(numbers(&found), [2969]);
    assert_eq!(found["items"][0]["status"], "END");
    assert_eq!(found["items"][0]["subject"], "안녕, 라라");
    assert_eq!(found["has_next"], false);
    assert_eq!(found["page"], 1);
    assert_eq!(app.requests_for("/anime/schedule/"), 0);

    // The finished show is picked from that search and saved.
    let (status, linked) = app
        .link(
            1,
            json!({ "version": 0, "anime_no": 2969, "q": "Sayonara Lara", "page": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{linked}");
    assert_eq!(linked["version"], 1);
    assert_eq!(linked["anime"]["anime_no"], 2969);
    assert_eq!(linked["anime"]["subject"], "안녕, 라라");
    assert_eq!(linked["anime"]["status"], "END");
    assert_eq!(
        linked["anime"]["url"],
        "https://anissia.net/anime?animeNo=2969"
    );
    assert_eq!(linked["subscription"], Value::Null);
    // The search was not asked again: the page the pick came from was kept.
    assert_eq!(app.requests_for("/anime/list/"), 1);

    // The work detail shows the link on its season, and not on the other.
    let shown = app.detail(1).await;
    assert_eq!(shown, linked);
    assert_eq!(app.detail(2).await["version"], 0);
    assert_eq!(app.detail(2).await["anime"], Value::Null);
}

#[tokio::test]
async fn an_edited_query_and_the_next_page_are_searched_and_the_pick_is_checked_against_its_page() {
    let app = App::new().await;
    app.catalogue();
    app.fake.state.lock().unwrap().page_size = 2;

    let (_, first) = app.search(1, json!({ "q": "안녕" })).await;
    assert_eq!(numbers(&first), [3441, 2969]);
    assert_eq!(first["has_next"], true);
    let (_, next) = app.search(1, json!({ "q": "안녕", "page": 2 })).await;
    assert_eq!(numbers(&next), [1900]);
    assert_eq!(next["has_next"], false);
    assert_eq!(next["page"], 2);
    // An edited query is another search.
    let (_, edited) = app.search(1, json!({ "q": " 다른 " })).await;
    assert_eq!(edited["q"], "다른");
    assert_eq!(numbers(&edited), [77]);
    assert_eq!(app.requests_for("/anime/list/0?q=안녕"), 1);
    assert_eq!(app.requests_for("/anime/list/1?q=안녕"), 1);
    assert_eq!(app.requests_for("/anime/list/0?q=다른"), 1);

    // An anime of the second page is saved from the second page...
    let (status, linked) = app
        .link(
            1,
            json!({ "version": 0, "anime_no": 1900, "q": "안녕", "page": 2 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{linked}");
    assert_eq!(linked["anime"]["subject"], "안녕, 나의 크라머");
    // ...and one that page does not list is refused, the link as it was.
    let (status, refused) = app
        .link(
            1,
            json!({ "version": 1, "anime_no": 3441, "q": "안녕", "page": 2 }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(refused["message"].as_str().unwrap().contains("검색 결과"));
    assert_eq!(app.get(1).await["anime"]["anime_no"], 1900);
    assert_eq!(app.get(1).await["version"], 1);
}

#[tokio::test]
async fn the_schedule_tab_links_the_same_way_and_a_link_can_be_changed_and_cut() {
    let app = App::new().await;
    app.fake.set_week(
        3,
        vec![app
            .fake
            .entry(3, 3320, "22:30", "이번 분기 작품", "This Quarter")],
    );

    let (status, linked) = app
        .link(2, json!({ "version": 0, "anime_no": 3320, "week": 3 }))
        .await;
    assert_eq!(status, StatusCode::OK, "{linked}");
    assert_eq!(linked["season"], 2);
    assert_eq!(linked["anime"]["subject"], "이번 분기 작품");
    assert_eq!(linked["anime"]["status"], "ON");
    assert_eq!(app.detail(2).await, linked);
    // The schedule's anime is checked against the schedule: one it does not list is refused.
    let (status, refused) = app
        .link(2, json!({ "version": 1, "anime_no": 9999, "week": 3 }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(app.get(2).await["anime"]["anime_no"], 3320);

    // Changed to another, then cut.
    app.fake.set_week(
        4,
        vec![app.fake.entry(4, 3330, "23:00", "다른 작품", "Other")],
    );
    let (_, changed) = app
        .link(2, json!({ "version": 1, "anime_no": 3330, "week": 4 }))
        .await;
    assert_eq!(
        (
            changed["version"].clone(),
            changed["anime"]["anime_no"].clone()
        ),
        (json!(2), json!(3330))
    );
    let (status, cut) = app.link(2, json!({ "version": 2, "anime_no": null })).await;
    assert_eq!(status, StatusCode::OK, "{cut}");
    assert_eq!(
        (cut["version"].clone(), cut["anime"].clone()),
        (json!(3), Value::Null)
    );
}

#[tokio::test]
async fn a_search_that_fails_says_so_and_neither_the_schedule_nor_the_stored_link_is_affected() {
    let app = App::new().await;
    app.catalogue();
    app.fake.set_week(
        3,
        vec![app
            .fake
            .entry(3, 3320, "22:30", "이번 분기 작품", "This Quarter")],
    );
    let (_, saved) = app
        .link(1, json!({ "version": 0, "anime_no": 3320, "week": 3 }))
        .await;
    assert_eq!(saved["version"], 1);

    // The answer's format changed, in each way it can: the search is reported
    // as failed, and the schedule tab still works.
    for raw in [
        "<html>maintenance</html>",
        r#"{"code":"ok","data":[]}"#,
        r#"{"code":"ok","data":{"content":[{"animeNo":1,"subject":"x"}]}}"#,
        r#"{"code":"ok","data":{"last":true}}"#,
    ] {
        app.fake.state.lock().unwrap().raw = Some(raw.to_owned());
        let (status, failed) = app.search(1, json!({ "q": "안녕" })).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{raw}: {failed}");
        assert_eq!(failed["error"], "unavailable");
        let message = failed["message"].as_str().unwrap();
        assert!(
            message.contains("검색하지 못했어요") && message.contains("편성표"),
            "{message}"
        );
        assert_eq!(app.get(1).await["anime"]["anime_no"], 3320, "{raw}");
        assert_eq!(app.get(1).await["version"], 1);
    }
    app.fake.state.lock().unwrap().raw = None;
    let (status, schedule) = app.call(Method::GET, "/api/anissia/schedule/3", None).await;
    assert_eq!(status, StatusCode::OK, "{schedule}");
    assert_eq!(schedule["entries"][0]["anime_no"], 3320);
    let (status, linked) = app
        .link(2, json!({ "version": 0, "anime_no": 3320, "week": 3 }))
        .await;
    assert_eq!(status, StatusCode::OK, "{linked}");

    // A failure of Anissia's own.
    app.fake.state.lock().unwrap().failing = 1;
    let (status, failed) = app.search(1, json!({ "q": "안녕" })).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{failed}");
    assert!(failed["message"].as_str().unwrap().contains("HTTP 500"));
    assert_eq!(app.get(1).await["anime"]["anime_no"], 3320);
    // A search that works again finds the list, with nothing remembered of the failures.
    let (status, found) = app.search(1, json!({ "q": "안녕" })).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(numbers(&found), [3441, 2969, 1900]);
}

#[tokio::test]
async fn after_a_429_the_search_waits_and_asks_again_only_when_the_wait_is_over() {
    let app = App::new().await;
    app.catalogue();
    let (_, saved) = app
        .link(
            1,
            json!({ "version": 0, "anime_no": 2969, "q": "Sayonara Lara", "page": 1 }),
        )
        .await;
    assert_eq!(saved["version"], 1);
    {
        let mut state = app.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = Some(30);
    }
    let asked = app.requests_for("/anime/list/");

    let (status, busy) = app.search(1, json!({ "q": "라라" })).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{busy}");
    assert!(busy["message"].as_str().unwrap().contains("30초"));
    assert_eq!(app.requests_for("/anime/list/"), asked + 1);

    // Within the wait it is told to wait, and no request is sent.
    let (status, busy) = app.search(1, json!({ "q": "라라" })).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{busy}");
    assert_eq!(app.requests_for("/anime/list/"), asked + 1);
    // The schedule is held by the same wait: one pace for every Anissia request.
    let (status, waiting) = app.call(Method::GET, "/api/anissia/schedule/3", None).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(waiting["message"]
        .as_str()
        .unwrap()
        .contains("초쯤 뒤에 다시 시도해 주세요"));
    assert_eq!(app.requests_for("/anime/schedule/"), 0);
    assert_eq!(app.get(1).await["anime"]["anime_no"], 2969);

    app.now.fetch_add(30_000, Ordering::SeqCst);
    let (status, found) = app.search(1, json!({ "q": "라라" })).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(numbers(&found), [2969]);
    assert_eq!(app.requests_for("/anime/list/"), asked + 2);
    assert_eq!(app.get(1).await["version"], 1);
}

#[tokio::test]
async fn a_season_a_subscription_holds_cannot_be_changed_from_the_work_detail() {
    let app = App::new().await;
    app.catalogue();
    app.fake.set_week(
        3,
        vec![app.fake.entry(3, 3320, "22:30", "구독 작품", "Subscribed")],
    );
    let channel = app
        .state
        .channels
        .create_channel(ChannelInput::new("https://feed.test/rss"))
        .await
        .unwrap();
    let rule = app
        .state
        .channels
        .create_subscription_rule(
            &channel.id,
            RuleInput {
                r#match: Some("Sub".into()),
                directory: "Sayonara Lara/Season 02".into(),
                ..RuleInput::default()
            },
            NewSubscription {
                anime: app
                    .fake
                    .entry(3, 3320, "22:30", "구독 작품", "Subscribed")
                    .as_object()
                    .map(|_| trss_anissia::Anime {
                        anime_no: 3320,
                        subject: "구독 작품".into(),
                        original_subject: None,
                        week: 3,
                        air_time: Some("22:30".into()),
                        start_date: None,
                        end_date: None,
                        status: "ON".into(),
                        fetched_at: NOW,
                    })
                    .unwrap(),
                subtitles: SubtitleMode::None,
                creator: None,
                subscribed_at: NOW,
            },
        )
        .await
        .unwrap();
    app.state
        .channels
        .link_season(&rule.id, &format!("{}:2", app.work))
        .await
        .unwrap();

    // The work detail shows the season as the subscription's, with its anime.
    let held = app.detail(2).await;
    assert_eq!(held["version"], 1);
    assert_eq!(held["anime"]["anime_no"], 3320);
    assert_eq!(held["subscription"]["rule_id"], rule.id.as_str());
    assert_eq!(held["subscription"]["subject"], "구독 작품");

    // A change, a cut and the same anime again are all refused, with the way out.
    for body in [
        json!({ "version": 1, "anime_no": 2969, "q": "Sayonara Lara", "page": 1 }),
        json!({ "version": 1, "anime_no": 3320, "week": 3 }),
        json!({ "version": 1, "anime_no": null }),
        json!({ "version": 0, "anime_no": null }),
    ] {
        let (status, refused) = app.link(2, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {refused}");
        let message = refused["message"].as_str().unwrap();
        assert!(
            message.contains("구독 작품") && message.contains("구독을 먼저 삭제해 주세요"),
            "{message}"
        );
    }
    assert_eq!(app.detail(2).await, held);
    // Season 1 of the same work is free.
    let (status, _) = app
        .link(1, json!({ "version": 0, "anime_no": 3320, "week": 3 }))
        .await;
    assert_eq!(status, StatusCode::OK);

    // The way the message names: delete the subscription, and the season keeps
    // its link, which can then be changed here.
    let rule = app
        .state
        .channels
        .get_rule(&rule.id)
        .await
        .unwrap()
        .unwrap();
    app.state
        .channels
        .delete_rule(&rule.id, rule.version)
        .await
        .unwrap();
    let freed = app.detail(2).await;
    assert_eq!(freed["subscription"], Value::Null);
    assert_eq!(freed["anime"]["anime_no"], 3320);
    let (status, changed) = app
        .link(
            2,
            json!({ "version": 1, "anime_no": 2969, "q": "Sayonara Lara", "page": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert_eq!(changed["anime"]["anime_no"], 2969);
}

#[tokio::test]
async fn a_late_save_from_a_second_screen_is_a_version_conflict_that_carries_the_current_link() {
    let app = App::new().await;
    app.catalogue();
    // Two screens read the season as never linked.
    let first = app.get(1).await;
    let second = app.get(1).await;
    assert_eq!(
        (first["version"].clone(), second["version"].clone()),
        (json!(0), json!(0))
    );

    let (status, saved) = app
        .link(
            1,
            json!({ "version": 0, "anime_no": 2969, "q": "Sayonara Lara", "page": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");

    let (status, late) = app
        .link(
            1,
            json!({ "version": 0, "anime_no": 1900, "q": "안녕", "page": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{late}");
    assert_eq!(late["error"], "conflict");
    assert_eq!(late["current"]["version"], 1);
    assert_eq!(late["current"]["anime"]["anime_no"], 2969);
    // Nothing of the late save was kept, not even the anime it named.
    assert_eq!(app.get(1).await["anime"]["anime_no"], 2969);
    assert!(app.state.anissia_store.anime(1900).await.unwrap().is_none());
    // The late save also loses a cut, and wins from the version it carries now.
    let (status, _) = app.link(1, json!({ "version": 0, "anime_no": null })).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, cut) = app.link(1, json!({ "version": 1, "anime_no": null })).await;
    assert_eq!(status, StatusCode::OK, "{cut}");
    assert_eq!(cut["version"], 2);
}

#[tokio::test]
async fn a_request_that_cannot_be_applied_is_refused_with_a_sentence() {
    let app = App::new().await;
    app.catalogue();
    // A season the work does not have, and a work the library does not have.
    let (status, _) = app.call(Method::GET, &app.path(7, ""), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app.search(7, json!({ "q": "x" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app.link(7, json!({ "version": 0, "anime_no": null })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app
        .call(
            Method::GET,
            "/api/library/works/nobody/seasons/1/anissia",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    for (path, body) in [
        ("/search", json!({ "page": 0 })),
        ("/search", json!({ "page": 101 })),
        ("/search", json!({ "q": "가".repeat(201) })),
        ("/search", json!({ "page": "one" })),
        ("/link", json!({ "version": 0, "anime_no": 0, "week": 3 })),
        ("/link", json!({ "version": 0, "anime_no": 2969 })),
        (
            "/link",
            json!({ "version": 0, "anime_no": 2969, "week": 3, "q": "x" }),
        ),
        ("/link", json!({ "anime_no": 2969, "week": 3 })),
        (
            "/link",
            json!({ "version": 0, "anime_no": 2969, "q": "x", "page": 0 }),
        ),
    ] {
        let (status, refused) = app
            .call(Method::POST, &app.path(1, path), Some(body.clone()))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path} {body}: {refused}");
        assert!(refused["message"].as_str().is_some_and(|m| !m.is_empty()));
    }
    // Nothing above reached Anissia or changed the link.
    assert_eq!(app.requests_for("/anime/"), 0);
    assert_eq!(app.get(1).await["version"], 0);
}

#[tokio::test]
async fn a_rule_blocked_by_a_seasons_link_says_the_link_holds_the_season() {
    let app = App::new().await;
    app.catalogue();
    app.fake.set_week(
        3,
        vec![app.fake.entry(3, 3320, "22:30", "구독 작품", "Subscribed")],
    );
    // The user linked season 2 to a finished anime; a subscription to another
    // anime then finds its videos in that season.
    let (_, saved) = app
        .link(
            2,
            json!({ "version": 0, "anime_no": 2969, "q": "Sayonara Lara", "page": 1 }),
        )
        .await;
    assert_eq!(saved["version"], 1);
    let channel = app
        .state
        .channels
        .create_channel(ChannelInput::new("https://feed.test/rss"))
        .await
        .unwrap();
    let anime = trss_anissia::Anime {
        anime_no: 3320,
        subject: "구독 작품".into(),
        original_subject: None,
        week: 3,
        air_time: None,
        start_date: None,
        end_date: None,
        status: "ON".into(),
        fetched_at: NOW,
    };
    let rule = app
        .state
        .channels
        .create_subscription_rule(
            &channel.id,
            RuleInput {
                r#match: Some("Sub".into()),
                directory: "Sayonara Lara/Season 02".into(),
                ..RuleInput::default()
            },
            NewSubscription {
                anime,
                subtitles: SubtitleMode::None,
                creator: None,
                subscribed_at: NOW,
            },
        )
        .await
        .unwrap();
    app.state
        .channels
        .link_season(&rule.id, &format!("{}:2", app.work))
        .await
        .unwrap();

    let (status, view) = app
        .call(Method::GET, &format!("/api/rules/{}", rule.id), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    let blocked = &view["season_blocked"];
    assert_eq!(blocked["holder_anime_no"], 2969);
    assert_eq!(blocked["holder_subject"], "안녕, 라라");
    assert_eq!(blocked["held_by"], "link");
    // The link is the user's, and still can be changed from the work detail.
    assert_eq!(app.detail(2).await["subscription"], Value::Null);
}

/// An AniList entry with only the titles that matter here.
fn entry_with_native(id: i64, native: Option<&str>) -> trss_anilist::Entry {
    trss_anilist::Entry {
        id,
        romaji: Some("Sayonara Lara".into()),
        english: Some("Goodbye Lara".into()),
        native: native.map(str::to_owned),
        format: Some("TV".into()),
        status: Some("FINISHED".into()),
        episodes: Some(12),
        start: Default::default(),
        end: Default::default(),
        studios: Vec::new(),
        genres: Vec::new(),
        description: None,
        airing: Vec::new(),
        sequels: Vec::new(),
        fetched_at: NOW,
    }
}

#[tokio::test]
async fn the_first_query_is_the_native_title_of_the_seasons_anilist_entry_else_the_folder_name() {
    let app = App::new().await;
    // Anissia's titles are Japanese or Korean; the library folder is not.
    app.fake.set_catalogue(vec![
        app.fake.finished(1, 2969, "안녕, 라라", "さよならララ"),
        app.fake
            .finished(0, 1900, "안녕, 나의 크라머", "Sayonara Cramer"),
    ]);
    let store = &app.state.seasons.store;
    store
        .put_entry(entry_with_native(5001, Some("さよならララ")))
        .await
        .unwrap();
    store
        .put_entry(entry_with_native(5002, None))
        .await
        .unwrap();
    store
        .put_entry(entry_with_native(5003, Some("続編")))
        .await
        .unwrap();
    // Season 1 links an entry with a native title (and a second one that is
    // not the first); season 2 links an entry with none.
    let version = |season: u32| {
        let store = store.clone();
        let work = app.work.clone();
        async move { store.link(&work, season).await.unwrap().version }
    };
    store
        .set_links(&app.work, 1, version(1).await, vec![5001, 5003])
        .await
        .unwrap();
    store
        .set_links(&app.work, 2, version(2).await, vec![5002])
        .await
        .unwrap();

    // The linked season starts with the native title and finds the show by it.
    let (status, found) = app.search(1, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["q"], "さよならララ");
    assert_eq!(numbers(&found), [2969]);
    // Nothing was asked of AniList for it.
    // The query the user wrote stays theirs.
    let (_, written) = app.search(1, json!({ "q": "Sayonara" })).await;
    assert_eq!(written["q"], "Sayonara");
    assert_eq!(numbers(&written), [1900]);

    // An entry without a native title, and a season with no entry at all, are
    // searched by the work's folder name.
    let (_, no_native) = app.search(2, json!({})).await;
    assert_eq!(no_native["q"], "Sayonara Lara");
    let (_, unlinked) = app
        .call(Method::POST, &app.path(0, "/search"), Some(json!({})))
        .await;
    assert_eq!(unlinked["error"], "not_found");
}

#[tokio::test]
async fn an_unlinked_season_starts_with_the_folder_name() {
    let app = App::new().await;
    app.catalogue();
    let (status, found) = app.search(2, json!({ "q": "  " })).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["q"], "Sayonara Lara");
    assert_eq!(numbers(&found), [2969]);
}

#[tokio::test]
async fn the_holder_shown_is_the_first_subscription_by_rule_id_whatever_the_listing_order() {
    let app = App::new().await;
    let channel = app
        .state
        .channels
        .create_channel(ChannelInput::new("https://feed.test/rss"))
        .await
        .unwrap();
    let anime = |no: i64, subject: &str| trss_anissia::Anime {
        anime_no: no,
        subject: subject.into(),
        original_subject: None,
        week: 3,
        air_time: None,
        start_date: None,
        end_date: None,
        status: "ON".into(),
        fetched_at: NOW,
    };
    // Two rules on one season for one anime, the one that sorts first by ID
    // listed second (rules list by position).
    let mut rules = Vec::new();
    for n in 0..64 {
        let rule = app
            .state
            .channels
            .create_subscription_rule(
                &channel.id,
                RuleInput {
                    r#match: Some(format!("Sub {n}")),
                    directory: "Sayonara Lara/Season 02".into(),
                    ..RuleInput::default()
                },
                NewSubscription {
                    anime: anime(3320 + n, "구독 작품"),
                    subtitles: SubtitleMode::None,
                    creator: None,
                    subscribed_at: NOW,
                },
            )
            .await
            .unwrap();
        let lower = rules
            .first()
            .is_some_and(|first: &trss_collect::store::channels::Rule| rule.id < first.id);
        rules.push(rule);
        if lower {
            break;
        }
    }
    assert!(
        rules.len() >= 2,
        "no rule sorted before the first in 64 tries"
    );
    // The connection itself refuses a second anime on a season, so a database
    // that holds two (one the migration made) is built by hand.
    let season = format!("{}:2", app.work);
    let ids: Vec<String> = rules.iter().map(|r| r.id.clone()).collect();
    app.db
        .run::<_, trss_core::DbError, _>(move |c| {
            for id in &ids {
                c.execute(
                    "UPDATE rule_subscriptions SET season_id = ?2 WHERE rule_id = ?1",
                    rusqlite::params![id, season],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let first = rules.iter().min_by_key(|r| r.id.clone()).unwrap();
    assert_ne!(
        first.id, rules[0].id,
        "the listing order differs from the ID order"
    );

    let shown = app.detail(2).await;
    assert_eq!(shown["subscription"]["rule_id"], first.id.as_str());
    assert_eq!(
        shown["subscription"]["anime_no"],
        first.subscription.as_ref().unwrap().anissia_anime_no
    );
}

#[test]
fn every_kind_of_search_failure_says_the_schedule_is_another_place_to_pick_from() {
    use trss_anissia::AnissiaError;

    for error in [
        AnissiaError::Invalid("not a page".into()),
        AnissiaError::Status(500),
        AnissiaError::Unreachable("refused".into()),
    ] {
        let ApiError::Unavailable(message) = super::search_unavailable(error) else {
            panic!("not unavailable");
        };
        assert!(
            message.ends_with("편성표에서는 고를 수 있어요."),
            "{message}"
        );
    }
    // During a 429 wait only the weeks read already can be picked from.
    let ApiError::Unavailable(message) = super::search_unavailable(AnissiaError::Busy {
        retry_after: Duration::from_secs(30),
    }) else {
        panic!("not unavailable");
    };
    assert!(
        message.contains("30초쯤 뒤에 다시 시도해 주세요"),
        "{message}"
    );
    assert!(
        message.ends_with("이미 불러온 편성표에서는 고를 수 있어요."),
        "{message}"
    );
}
