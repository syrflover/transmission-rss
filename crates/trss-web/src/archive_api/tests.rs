//! The archive suggestions API (ticket 0023): the shape of the list, the
//! facts the web hands to the rule, `수집 유지`, and what archiving would do
//! with the folder. The rules themselves (the grounds, the 4 weeks of reading,
//! which rules are listed and in what order, the forecast sentences) are tested
//! in trss-collect (`archive_suggestions/tests.rs`, `rule_archive.rs`, ADR 0015).

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

use trss_core::{Clock, Db};

use super::*;
use trss_anissia::{Anime, Anissia, AnissiaConfig};
use trss_collect::store::{
    channels::{Channel, ChannelInput, NewSubscription, RuleInput, SubtitleMode},
    history::{HistoryResult, Observation},
    status::{read_day, ChannelReadResult},
};

/// 2026-10-01 12:00 in Seoul.
const NOW: Millis = 1_790_780_400_000 + 12 * 60 * 60 * 1000;
const DAY: Millis = 24 * 60 * 60 * 1000;
const WEEK: Millis = 7 * DAY;

const WORK_5: &str = "[SubsPlease] Work - 05 (1080p) [ABCD1234].mkv";

struct App {
    db: Db,
    state: AppState,
    router: Router,
    now: Arc<AtomicI64>,
    channel: Channel,
    /// The last day the worker read the channel's feed, as far as the test has
    /// let it (see [`App::read_to_now`]).
    read_through: AtomicI64,
}

fn anime(no: i64, end_date: Option<&str>) -> Anime {
    Anime {
        anime_no: no,
        subject: format!("작품 {no}"),
        original_subject: None,
        week: 3,
        air_time: Some("22:30".into()),
        start_date: Some("2026-07-01".into()),
        end_date: end_date.map(str::to_owned),
        status: "ON".into(),
        fetched_at: 1,
    }
}

impl App {
    async fn new() -> App {
        let db = Db::open_blocking(":memory:").unwrap();
        let now = Arc::new(AtomicI64::new(NOW));
        let clock: Clock = {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        };
        let anissia = Anissia::new(db.clone(), AnissiaConfig::default(), clock);
        let state = AppState::new(db.clone()).with_anissia(anissia);
        state
            .settings
            .put_collection(0, "/media".to_owned(), None)
            .await
            .unwrap();
        let channel = state
            .channels
            .create_channel(ChannelInput::new("https://feed.test/rss"))
            .await
            .unwrap();
        let router = Router::new().nest("/api", crate::api::router().with_state(state.clone()));
        App {
            db,
            state,
            router,
            now,
            channel,
            // Read every day for two months up to now, unless a test says
            // otherwise.
            read_through: AtomicI64::new(read_day(NOW) - 60),
        }
    }

    /// The worker reads the feed once a day up to the test's clock. Which days
    /// count as read is the collect crate's rule (`archive_suggestions` and
    /// `StatusStore::read_day_floors` tests), so no test here leaves days unread.
    async fn read_to_now(&self) {
        let through = self.read_through.load(Ordering::SeqCst);
        let last = read_day(self.now.load(Ordering::SeqCst));
        for day in through + 1..=last {
            self.state
                .status
                .record_reads(
                    day * DAY,
                    vec![ChannelReadResult {
                        channel_id: self.channel.id.clone(),
                        ok: true,
                    }],
                    vec![self.channel.id.clone()],
                )
                .await
                .unwrap();
        }
        self.read_through.fetch_max(last, Ordering::SeqCst);
    }

    /// The database stamps a rule with the wall clock when it is made, which a
    /// test with its own clock replaces.
    async fn restamp(&self, rule: &Rule, at: Millis) {
        let id = rule.id.clone();
        self.db
            .run::<_, trss_core::DbError, _>(move |c| {
                c.execute(
                    "UPDATE rule_started SET started_at = ?2 WHERE rule_id = ?1",
                    rusqlite::params![id, at],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    fn at(&self, now: Millis) {
        self.now.store(now, Ordering::SeqCst);
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        self.read_to_now().await;
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

    async fn suggestions(&self) -> Vec<Value> {
        let (status, body) = self
            .call(Method::GET, "/api/archive-suggestions", None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["suggestions"].as_array().unwrap().clone()
    }

    async fn ids(&self) -> Vec<String> {
        self.suggestions()
            .await
            .iter()
            .map(|s| s["rule_id"].as_str().unwrap().to_owned())
            .collect()
    }

    /// A plain rule for `phrase` saving to `directory`.
    async fn rule(&self, phrase: &str, directory: &str) -> Rule {
        let rule = self
            .state
            .channels
            .create_rule(
                &self.channel.id,
                RuleInput {
                    r#match: Some(phrase.to_owned()),
                    directory: directory.to_owned(),
                    ..RuleInput::default()
                },
            )
            .await
            .unwrap();
        self.restamp(&rule, NOW - 52 * WEEK).await;
        rule
    }

    /// A subscription to anime `no` whose snapshot ends on `end_date`.
    async fn subscription(&self, phrase: Option<&str>, no: i64, end_date: Option<&str>) -> Rule {
        let rule = self
            .state
            .channels
            .create_subscription_rule(
                &self.channel.id,
                RuleInput {
                    r#match: phrase.map(str::to_owned),
                    directory: format!("작품 {no}/Season 01"),
                    ..RuleInput::default()
                },
                NewSubscription {
                    anime: anime(no, end_date),
                    subtitles: SubtitleMode::None,
                    creator: None,
                    subscribed_at: NOW - 100 * DAY,
                },
            )
            .await
            .unwrap();
        self.restamp(&rule, NOW - 100 * DAY).await;
        rule
    }

    async fn record(&self, at: Millis, title: &str, result: HistoryResult, rule: Option<&Rule>) {
        self.state
            .history
            .record(
                at,
                vec![Observation {
                    channel_id: self.channel.id.clone(),
                    channel_label: self.channel.masked_url(),
                    identity_key: format!("title:{title}:{at}"),
                    title: title.to_owned(),
                    link: "https://feed.test/item".into(),
                    result,
                    rule_id: rule.map(|r| r.id.clone()),
                    torrent_hash: None,
                    reason: None,
                }],
            )
            .await
            .unwrap();
    }

    /// The channel was read long ago, and `rule` received `weeks` weeks before now.
    async fn received_weeks_ago(&self, rule: &Rule, weeks: i64) {
        self.record(
            NOW - 52 * WEEK,
            "[Other] Old - 01",
            HistoryResult::NoMatch,
            None,
        )
        .await;
        self.record(
            NOW - weeks * WEEK,
            WORK_5,
            HistoryResult::Received,
            Some(rule),
        )
        .await;
    }

    async fn keep(&self, rule: &Rule, grounds: &[&str]) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            "/api/archive-suggestions/keep",
            Some(json!({ "rule_id": rule.id, "grounds": grounds })),
        )
        .await
    }
}

#[tokio::test]
async fn with_no_rules_there_is_nothing_to_suggest() {
    let app = App::new().await;
    assert!(app.suggestions().await.is_empty());
}

#[tokio::test]
async fn a_subscription_whose_end_date_has_passed_is_suggested_and_says_so() {
    let app = App::new().await;
    let rule = app.subscription(Some("Work"), 7, Some("2026-09-30")).await;
    app.received_weeks_ago(&rule, 1).await;

    let found = app.suggestions().await;
    assert_eq!(found.len(), 1);
    let s = &found[0];
    assert_eq!(s["rule_id"], rule.id);
    assert_eq!(s["title"], "Work");
    assert_eq!(s["state"], "active");
    assert_eq!(s["anime"]["subject"], "작품 7");
    assert_eq!(s["channel_host"], "feed.test");
    assert_eq!(s["last_received_at"], NOW - WEEK);
    assert_eq!(s["grounds"].as_array().unwrap().len(), 1);
    assert_eq!(s["grounds"][0]["kind"], "ended");
    assert_eq!(s["grounds"][0]["end_date"], "2026-09-30");
    assert_eq!(s["grounds"][0]["key"], "ended:7:2026-09-30");
    assert!(s["after"]
        .as_str()
        .unwrap()
        .contains("보관 폴더를 정하지 않아서"));
}

/// The rule (4 weeks from the start when nothing was received) is tested in
/// trss-collect; this checks that the start the store stamped reaches the
/// ground, and that a rule that never received has no `last_received_at`.
#[tokio::test]
async fn a_rule_that_never_received_says_when_it_started_in_its_quiet_ground() {
    let app = App::new().await;
    // The channel was read long ago, but the rule is more recent.
    app.record(
        NOW - 52 * WEEK,
        "[Other] Old - 01",
        HistoryResult::NoMatch,
        None,
    )
    .await;
    let rule = app.rule("Work", "Work/Season 01").await;
    app.restamp(&rule, NOW - 5 * WEEK).await;
    let found = app.suggestions().await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["last_received_at"], Value::Null);
    assert_eq!(found[0]["grounds"][0]["kind"], "quiet");
    assert_eq!(found[0]["grounds"][0]["since"], NOW - 5 * WEEK);
}

#[tokio::test]
async fn keeping_collecting_hides_the_same_ground_but_not_a_new_one() {
    let app = App::new().await;
    let rule = app.subscription(Some("Work"), 7, None).await;
    app.received_weeks_ago(&rule, 6).await;
    let found = app.suggestions().await;
    // The quiet ground as the screen reads it: where the weeks count from.
    assert_eq!(found[0]["grounds"][0]["kind"], "quiet");
    assert_eq!(found[0]["grounds"][0]["since"], NOW - 6 * WEEK);
    let quiet = found[0]["grounds"][0]["key"].as_str().unwrap().to_owned();
    assert_eq!(quiet, format!("quiet:{}", NOW - 6 * WEEK));

    let (status, body) = app.keep(&rule, &[&quiet]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["kept"], 1);
    assert!(app.ids().await.is_empty());

    // Nothing changes while it stays quiet, however long.
    app.at(NOW + 10 * WEEK);
    assert!(app.ids().await.is_empty());

    // The anime ends: a new ground, and only that one is listed.
    app.state
        .anissia_store
        .mark_unlisted(vec![7], NOW, NOW + DAY, NOW + 1)
        .await
        .unwrap();
    let found = app.suggestions().await;
    assert_eq!(found.len(), 1);
    let kinds: Vec<&str> = found[0]["grounds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["unlisted"]);

    let (status, _) = app.keep(&rule, &["unlisted:7"]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(app.ids().await.is_empty());
}

#[tokio::test]
async fn keeping_collecting_is_refused_for_a_gone_rule_and_for_grounds_it_does_not_know() {
    let app = App::new().await;
    let rule = app.rule("Work", "Work/Season 01").await;

    let (status, body) = app
        .call(
            Method::POST,
            "/api/archive-suggestions/keep",
            Some(json!({ "rule_id": "no-such-rule", "grounds": ["quiet:1"] })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    for bad in [
        json!([]),
        json!(["whatever"]),
        json!([""]),
        json!(vec!["quiet:1"; 9]),
        json!(["quiet:soon"]),
        json!(["quiet:1:2"]),
        json!(["ended:7"]),
        json!(["ended:7:"]),
        json!(["unlisted:x"]),
    ] {
        let (status, body) = app
            .call(
                Method::POST,
                "/api/archive-suggestions/keep",
                Some(json!({ "rule_id": rule.id, "grounds": bad })),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
    let (status, _) = app
        .call(
            Method::POST,
            "/api/archive-suggestions/keep",
            Some(json!({ "rule_id": rule.id, "grounds": ["quiet:1"], "extra": true })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The sentences are tested in trss-collect (`forecast_archive`); this checks
/// that the view is given the collection folders of the settings and the rules
/// of the channel to forecast from.
#[tokio::test]
async fn the_view_says_what_archiving_would_do_with_the_work_folder() {
    let app = App::new().await;
    app.state
        .settings
        .put_collection(
            1,
            "/media/current".to_owned(),
            Some("/media/archive".to_owned()),
        )
        .await
        .unwrap();
    // `Work/Season 01` and `Work/Season 02` share the work folder `Work`.
    let one = app.rule("Work S1", "Work/Season 01").await;
    let two = app.rule("Work S2", "Work/Season 02").await;
    let alone = app.rule("Alone", "Alone/Season 01").await;
    app.record(
        NOW - 52 * WEEK,
        "[Other] Old - 01",
        HistoryResult::NoMatch,
        None,
    )
    .await;
    for rule in [&one, &two, &alone] {
        app.state
            .history
            .record(
                NOW - 6 * WEEK,
                vec![Observation {
                    channel_id: app.channel.id.clone(),
                    channel_label: app.channel.masked_url(),
                    identity_key: format!("title:{}", rule.id),
                    title: format!("[X] {} - 01", rule.id),
                    link: "https://feed.test/item".into(),
                    result: HistoryResult::Received,
                    rule_id: Some(rule.id.clone()),
                    torrent_hash: None,
                    reason: None,
                }],
            )
            .await
            .unwrap();
    }

    let found = app.suggestions().await;
    assert_eq!(found.len(), 3);
    let after = |id: &str| {
        found.iter().find(|s| s["rule_id"] == id).unwrap()["after"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(after(&alone.id), "작품 폴더(`Alone`)를 보관 폴더로 옮겨요.");
    let held = after(&one.id);
    assert!(
        held.contains(
            "‘Work/Season 02’ 규칙이 아직 이 작품 폴더에 받고 있어서 폴더는 옮기지 않아요"
        ),
        "{held}"
    );
}
