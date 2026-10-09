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

    let (status, found) = app.search(1, json!({ "q": "Sayonara Lara" })).await;
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
fn entry_titled(
    id: i64,
    native: Option<&str>,
    english: Option<&str>,
    romaji: Option<&str>,
    korean: &[&str],
) -> trss_anilist::Entry {
    trss_anilist::Entry {
        id,
        romaji: romaji.map(str::to_owned),
        english: english.map(str::to_owned),
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
        korean_titles: korean.iter().map(|k| (*k).to_owned()).collect(),
        sequels: Vec::new(),
        fetched_at: NOW,
    }
}

fn reference(view: &Value) -> Vec<(String, String)> {
    view["reference_titles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["kind"].as_str().unwrap().to_owned(),
                t["title"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn pair(kind: &str, title: &str) -> (String, String) {
    (kind.to_owned(), title.to_owned())
}

#[tokio::test]
async fn the_view_lists_the_titles_to_read_without_empty_or_repeated_ones() {
    let app = App::new().await;
    let store = &app.state.seasons.store;
    // Season 1 links two entries: the second repeats the first's romaji and
    // English titles (in another case), has an empty native title and a Korean
    // synonym that the first has too.
    store
        .put_entry(entry_titled(
            5001,
            Some("さよならララ"),
            Some("Goodbye Lara"),
            Some("Sayonara Lara"),
            &["안녕, 라라", "라라여 안녕"],
        ))
        .await
        .unwrap();
    store
        .put_entry(entry_titled(
            5002,
            Some("  "),
            Some("goodbye lara"),
            Some("Sayonara Lara 2"),
            &["안녕, 라라"],
        ))
        .await
        .unwrap();
    let version = store.link(&app.work, 1).await.unwrap().version;
    store
        .set_links(&app.work, 1, version, vec![5001, 5002])
        .await
        .unwrap();

    // The folder name repeats the first entry's romaji title, so it is not listed twice.
    let want = vec![
        pair("korean", "안녕, 라라"),
        pair("korean", "라라여 안녕"),
        pair("native", "さよならララ"),
        pair("english", "Goodbye Lara"),
        pair("romaji", "Sayonara Lara"),
        pair("romaji", "Sayonara Lara 2"),
    ];
    assert_eq!(reference(&app.get(1).await), want);
    // The work detail carries the same, so the dialog needs no request of its own.
    assert_eq!(reference(&app.detail(1).await), want);

    // A season with no entry has the folder name only.
    assert_eq!(
        reference(&app.get(2).await),
        [pair("folder", "Sayonara Lara")]
    );

    // A folder name no entry has is listed last.
    let store = &app.state.seasons.store;
    store
        .put_entry(entry_titled(5003, Some("続"), None, None, &[]))
        .await
        .unwrap();
    let version = store.link(&app.work, 2).await.unwrap().version;
    store
        .set_links(&app.work, 2, version, vec![5003])
        .await
        .unwrap();
    assert_eq!(
        reference(&app.get(2).await),
        [pair("native", "続"), pair("folder", "Sayonara Lara")]
    );
}

#[tokio::test]
async fn a_search_without_a_query_is_refused_and_asks_nothing_of_anissia() {
    let app = App::new().await;
    app.catalogue();
    for body in [
        json!({}),
        json!({ "q": "" }),
        json!({ "q": "   " }),
        json!({ "q": null }),
    ] {
        let (status, refused) = app.search(1, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {refused}");
        assert_eq!(refused["message"], "검색어를 입력해 주세요.");
    }
    // A link picked from a search must say which search it was.
    let (status, refused) = app
        .link(
            1,
            json!({ "version": 0, "anime_no": 2969, "q": " ", "page": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(app.requests_for("/anime/"), 0);
    assert_eq!(app.get(1).await["version"], 0);
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

// --- subtitle candidates (ticket 0035) ----------------------------------------

use trss_collect::anissia::captions::CaptionObserver;

impl App {
    /// The 30-minute reading's observer over this app's database and fake.
    fn observer(&self) -> CaptionObserver {
        CaptionObserver::new(self.state.anissia.clone(), self.state.anissia_store.clone())
    }

    async fn candidates(&self, season: u32) -> Value {
        let (status, body) = self
            .call(Method::GET, &self.path(season, "/candidates"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    fn schedule_3320(&self) {
        self.fake.set_week(
            3,
            vec![self
                .fake
                .entry(3, 3320, "22:30", "이번 분기 작품", "This Quarter")],
        );
    }

    async fn command(&self, id: &str, body: Value) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            "/api/commands",
            Some(json!({ "id": id, "kind": "anissia_captions", "payload": body })),
        )
        .await
    }
}

#[tokio::test]
async fn candidates_of_a_season_with_no_link_are_none() {
    let app = App::new().await;
    let shown = app.candidates(1).await;
    assert_eq!(shown["season"], 1);
    assert_eq!(shown["anime_no"], Value::Null);
    assert_eq!(shown["candidates"], json!([]));
    assert_eq!(shown["refresh"], Value::Null);
    // A season the work does not have is not found.
    let (status, _) = app
        .call(Method::GET, &app.path(9, "/candidates"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn each_creators_episodes_come_as_runs_with_zero_inside_them() {
    let app = App::new().await;
    app.schedule_3320();
    // A reading holds each creator's latest line: the episodes pile up over
    // the readings.
    let observer = app.observer();
    for (episode, other) in [("2", "05"), ("0", "05"), ("SP", "05"), ("1", "05")] {
        let line = |episode: &str, creator: &str| {
            let url = format!("https://blog.test/{creator}-{episode}");
            app.fake
                .recent_line(3320, episode, "2026-10-02T11:00:00", &url, creator)
        };
        app.fake
            .set_recent(vec![line(episode, "에루샤"), line(other, "코코렛")]);
        observer.run_due().await.unwrap();
        app.now.fetch_add(30 * 60_000, Ordering::SeqCst);
    }
    let (status, linked) = app
        .link(1, json!({ "version": 0, "anime_no": 3320, "week": 3 }))
        .await;
    assert_eq!(status, StatusCode::OK, "{linked}");

    let shown = app.candidates(1).await;
    let source_of = |creator: &str| {
        shown["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["creator"] == creator)
            .unwrap()["source_id"]
            .clone()
    };
    let by_source = |creator: &str| {
        let source = source_of(creator);
        shown["creator_episodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["source_id"] == source)
            .unwrap()["episode_segments"]
            .clone()
    };
    assert_eq!(
        by_source("에루샤"),
        json!([
            { "text": "0–2", "count": 3, "whole": true },
            { "text": "SP", "count": 1, "whole": false },
        ])
    );
    assert_eq!(
        by_source("코코렛"),
        json!([{ "text": "5", "count": 1, "whole": true }])
    );
    assert_eq!(shown["creator_episodes"].as_array().unwrap().len(), 2);
    // A season with no link has no creators.
    assert_eq!(app.candidates(2).await["creator_episodes"], json!([]));
}

#[tokio::test]
async fn a_candidate_tells_its_episode_as_written_and_shown() {
    let app = App::new().await;
    app.schedule_3320();
    app.fake.set_recent(vec![app.fake.recent_line(
        3320,
        "05",
        "2026-10-02T11:00:00",
        "https://blog.test/a",
        "에루샤",
    )]);
    app.observer().run_due().await.unwrap();
    let (status, linked) = app
        .link(1, json!({ "version": 0, "anime_no": 3320, "week": 3 }))
        .await;
    assert_eq!(status, StatusCode::OK, "{linked}");

    let shown = app.candidates(1).await;

    assert_eq!(shown["candidates"][0]["episode"], "05");
    assert_eq!(shown["candidates"][0]["episode_shown"], "5");
}

#[tokio::test]
async fn linking_after_observations_exist_shows_the_earlier_ones_at_once_and_asks_the_worker_to_read(
) {
    let app = App::new().await;
    app.schedule_3320();
    // Before any season is linked, the recent list is read twice: the creator
    // moves from episode 3 to 4, and a second creator posts.
    app.fake.set_recent(vec![app.fake.recent_line(
        3320,
        "3",
        "2026-10-02T11:00:00",
        "https://blog.test/a",
        "에루샤",
    )]);
    let observer = app.observer();
    observer.run_due().await.unwrap();
    app.now.fetch_add(30 * 60_000, Ordering::SeqCst);
    app.fake.set_recent(vec![
        app.fake.recent_line(
            3320,
            "4",
            "2026-10-02T11:40:00",
            "https://blog.test/b",
            "에루샤",
        ),
        app.fake
            .recent_line(3320, "0", "soon", "https://blog.test/c", "코코렛"),
        app.fake.recent_line(
            9999,
            "1",
            "2026-10-02T11:40:00",
            "https://blog.test/d",
            "다른 작품의 제작자",
        ),
    ]);
    observer.run_due().await.unwrap();

    // Linking answers without asking Anissia for the lines: the worker does.
    let (status, linked) = app
        .link(1, json!({ "version": 0, "anime_no": 3320, "week": 3 }))
        .await;
    assert_eq!(status, StatusCode::OK, "{linked}");
    assert_eq!(app.fake.count("/anime/caption/animeNo/"), 0);
    let command = app
        .state
        .commands
        .latest_for_subjects("anissia_captions", vec!["3320".into()])
        .await
        .unwrap()
        .remove("3320")
        .expect("a read is waiting for the worker");
    assert_eq!(command.state, trss_core::commands::CommandState::Pending);
    assert_eq!(command.payload, r#"{"anime_no":3320}"#);

    // The earlier observations are the season's candidates at once.
    let shown = app.candidates(1).await;
    assert_eq!(shown["anime_no"], 3320);
    assert_eq!(shown["refresh"]["state"], "pending");
    assert_eq!(shown["refresh"]["kind"], "anissia_captions");
    let rows = shown["candidates"].as_array().unwrap();
    let seen: Vec<(&str, &str)> = rows
        .iter()
        .map(|c| {
            (
                c["creator"].as_str().unwrap(),
                c["episode"].as_str().unwrap(),
            )
        })
        .collect();
    // Newest first: episode 4 (11:40 Seoul time), 3 (11:00), and the unreadable
    // date by when it was first seen: the second reading, which is the
    // earliest of the three moments (the clock of the test is a day before).
    assert_eq!(seen, [("에루샤", "4"), ("에루샤", "3"), ("코코렛", "0")]);
    assert_eq!(rows[2]["updated"], "soon");
    assert_eq!(rows[2]["updated_at"], Value::Null);
    assert_eq!(rows[2]["updated_parse_failed"], true);
    assert_eq!(rows[2]["sort_at"], rows[2]["first_seen_at"]);
    assert_eq!(rows[0]["updated_parse_failed"], false);
    assert_eq!(rows[0]["post_url"], "https://blog.test/b");
    assert_eq!(rows[0]["revision"], Value::Null);
    assert_eq!(rows[0]["source_id"], rows[1]["source_id"]);
    assert_ne!(rows[0]["source_id"], rows[2]["source_id"]);
    // Another anime's lines are not this season's.
    assert!(!rows.iter().any(|c| c["post_url"] == "https://blog.test/d"));
    // The other season has none.
    assert_eq!(app.candidates(2).await["candidates"], json!([]));
    assert!(shown["read_at"].is_i64());
}

#[tokio::test]
async fn a_fix_of_a_received_post_is_a_revision_and_each_candidate_says_how_its_job_stands() {
    let app = App::new().await;
    app.schedule_3320();
    let line = |updated: &str| {
        app.fake
            .recent_line(3320, "3", updated, "https://blog.test/a", "에루샤")
    };
    app.fake.set_recent(vec![line("2026-10-02T11:00:00")]);
    let observer = app.observer();
    observer.run_due().await.unwrap();
    app.now.fetch_add(30 * 60_000, Ordering::SeqCst);
    app.fake.set_recent(vec![line("2026-10-02T11:50:00")]);
    observer.run_due().await.unwrap();
    app.link(1, json!({ "version": 0, "anime_no": 3320, "week": 3 }))
        .await;

    let rows = app.candidates(1).await["candidates"].clone();
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["updated"], "2026-10-02T11:50:00");
    // Observed with the same episode before, but nothing received.
    assert_eq!(rows[0]["revision"], Value::Null);
    assert_eq!(rows[1]["job"], Value::Null);

    // The older post is picked: its job stands, nothing is received yet.
    let older = rows[1]["id"].as_i64().unwrap();
    let (status, made) = app
        .call(
            Method::POST,
            "/api/subtitle-jobs",
            Some(
                json!({ "id": "pick-1", "work_id": app.work, "season": 1, "candidates": [older] }),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    let job = made["id"].as_str().unwrap().to_owned();
    let rows = app.candidates(1).await["candidates"].clone();
    assert_eq!(
        rows[1]["job"],
        json!({ "id": job, "state": "pending", "wait": null })
    );
    assert_eq!(rows[0]["revision"], Value::Null);
    assert_eq!(rows[0]["job"], Value::Null);

    // Once it is received, the fix of the same post revises it.
    let item = app.state.jobs.items(&job).await.unwrap()[0].id;
    trss_jobs::JobRun::new(app.state.db().clone())
        .set_item(item, trss_jobs::ItemState::Done, None, None, NOW)
        .await
        .unwrap();
    let rows = app.candidates(1).await["candidates"].clone();
    assert_eq!(rows[1]["job"]["state"], "done");
    assert_eq!(
        rows[0]["revision"],
        json!({ "of": older, "same_post": true })
    );
    assert_eq!(rows[1]["revision"], Value::Null);
}

#[tokio::test]
async fn cutting_a_link_asks_for_nothing_and_a_repeated_link_waits_on_the_read_already_asked() {
    let app = App::new().await;
    app.schedule_3320();
    app.link(1, json!({ "version": 0, "anime_no": 3320, "week": 3 }))
        .await;
    // Another season of the work linked to the same anime while the first read
    // waits: one read stands for both.
    let (status, _) = app
        .link(2, json!({ "version": 0, "anime_no": 3320, "week": 3 }))
        .await;
    assert_eq!(status, StatusCode::OK);
    let open = app
        .state
        .commands
        .open_for_subjects("anissia_captions", vec!["3320".into()])
        .await
        .unwrap();
    assert_eq!(open.len(), 1);

    // Once the read has ended, cutting a link asks for nothing.
    let id = open.into_values().next().unwrap().id;
    app.state
        .commands
        .finish(
            &id,
            trss_core::commands::CommandState::Done,
            trss_core::commands::Outcome {
                result: "read".into(),
                reason: None,
            },
            NOW,
        )
        .await
        .unwrap();
    let (status, _) = app.link(1, json!({ "version": 1, "anime_no": null })).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!app.state.commands.has_open().await.unwrap());
    assert_eq!(app.candidates(1).await["anime_no"], Value::Null);
}

#[tokio::test]
async fn the_refresh_command_is_accepted_for_a_linked_anime_only_and_one_at_a_time() {
    let app = App::new().await;
    app.schedule_3320();
    // No season is linked to 3320 yet.
    let (status, body) = app
        .command("refresh-0001", json!({ "anime_no": 3320 }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(!app.state.commands.has_open().await.unwrap());

    // Linked: the link's own read is the open one, and a refresh meanwhile is
    // told to wait for it.
    app.link(1, json!({ "version": 0, "anime_no": 3320, "week": 3 }))
        .await;
    let (status, body) = app
        .command("refresh-0001", json!({ "anime_no": 3320 }))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // Once that read has ended, a refresh is accepted, and repeated by its ID.
    let open = app
        .state
        .commands
        .open_for_subjects("anissia_captions", vec!["3320".into()])
        .await
        .unwrap()
        .remove("3320")
        .unwrap();
    app.state
        .commands
        .finish(
            &open.id,
            trss_core::commands::CommandState::Done,
            trss_core::commands::Outcome {
                result: "read".into(),
                reason: None,
            },
            NOW,
        )
        .await
        .unwrap();
    let (status, body) = app
        .command("refresh-0001", json!({ "anime_no": 3320 }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["state"], "pending");
    assert_eq!(body["kind"], "anissia_captions");
    let (status, _) = app
        .command("refresh-0001", json!({ "anime_no": 3320 }))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app
        .command("refresh-0001", json!({ "anime_no": 3321 }))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // The payload is the anime and nothing else.
    let (status, _) = app
        .command("refresh-0002", json!({ "anime_no": 3320, "x": 1 }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
