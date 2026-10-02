//! The archive suggestions API (ticket 0023): the grounds, the 4 weeks, `수집
//! 유지`, and what archiving would do with the folder.

use std::sync::{
    atomic::{AtomicBool, AtomicI64, Ordering},
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
use trss_legacy::{
    anissia::{Anissia, AnissiaConfig},
    store::{
        anissia::Anime,
        channels::{Channel, ChannelInput, NewSubscription, RuleInput, RuleState, SubtitleMode},
        history::{HistoryResult, Observation},
        status::{read_day, ChannelReadResult},
    },
};

/// 2026-10-01 12:00 in Seoul.
const NOW: Millis = 1_790_780_400_000 + 12 * 60 * 60 * 1000;
const DAY: Millis = 24 * 60 * 60 * 1000;
const WEEK: Millis = 7 * DAY;

const WORK_5: &str = "[SubsPlease] Work - 05 (1080p) [ABCD1234].mkv";
const OTHER_5: &str = "[SubsPlease] Another Show - 05 (1080p) [ABCD1235].mkv";

struct App {
    db: Db,
    state: AppState,
    router: Router,
    now: Arc<AtomicI64>,
    channel: Channel,
    /// The last day the worker read the channel's feed, as far as the test has
    /// let it (see [`App::read_to_now`]).
    read_through: AtomicI64,
    /// Whether the feed can be read: while it cannot, days pass unread.
    readable: AtomicBool,
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
            readable: AtomicBool::new(true),
        }
    }

    /// The worker reads the feed once a day up to the test's clock while the
    /// feed can be read, and leaves the days unread while it cannot (the
    /// address is dead, or the worker is off).
    async fn read_to_now(&self) {
        self.read_up_to(self.now.load(Ordering::SeqCst)).await;
    }

    async fn read_up_to(&self, until: Millis) {
        let through = self.read_through.load(Ordering::SeqCst);
        let last = read_day(until);
        if self.readable.load(Ordering::SeqCst) {
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
        }
        self.read_through.fetch_max(last, Ordering::SeqCst);
    }

    /// The feed cannot be read from now on, or can be again, from today.
    fn feed_readable(&self, readable: bool) {
        self.readable.store(readable, Ordering::SeqCst);
        if readable {
            self.read_through.store(
                read_day(self.now.load(Ordering::SeqCst)) - 1,
                Ordering::SeqCst,
            );
        }
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

#[tokio::test]
async fn an_end_date_that_has_not_passed_gives_no_suggestion() {
    let app = App::new().await;
    let rule = app.subscription(Some("Work"), 7, Some("2026-10-01")).await;
    app.received_weeks_ago(&rule, 1).await;
    assert!(app.ids().await.is_empty());
}

#[tokio::test]
async fn an_anime_without_an_end_date_is_suggested_once_the_refresh_found_it_unlisted() {
    let app = App::new().await;
    let rule = app.subscription(Some("Work"), 7, None).await;
    app.received_weeks_ago(&rule, 1).await;
    assert!(
        app.ids().await.is_empty(),
        "Anissia lists it, or was not reachable"
    );

    app.state
        .anissia
        .store
        .mark_unlisted(vec![7], NOW - DAY, NOW + DAY, NOW)
        .await
        .unwrap();
    let found = app.suggestions().await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["grounds"][0]["kind"], "unlisted");
    assert_eq!(found[0]["grounds"][0]["key"], "unlisted:7");
}

#[tokio::test]
async fn three_weeks_without_a_new_item_is_not_enough_and_four_weeks_is() {
    let app = App::new().await;
    let rule = app.rule("Work", "Work/Season 01").await;
    app.received_weeks_ago(&rule, 3).await;
    assert!(app.ids().await.is_empty());

    // Not a moment too early, and at the very moment (the receive itself is
    // not a new item) it is there.
    app.at(NOW + WEEK - 1);
    assert!(app.ids().await.is_empty());
    app.at(NOW + WEEK);
    assert_eq!(app.ids().await.len(), 1);
    app.at(NOW + WEEK + DAY);
    let found = app.suggestions().await;
    assert_eq!(found.len(), 1);
    let ground = &found[0]["grounds"][0];
    assert_eq!(ground["kind"], "quiet");
    assert_eq!(ground["since"], NOW - 3 * WEEK);
    assert_eq!(ground["key"], format!("quiet:{}", NOW - 3 * WEEK));
}

#[tokio::test]
async fn weeks_the_channel_could_not_be_read_are_not_quiet_weeks() {
    let app = App::new().await;
    let rule = app.rule("Work", "Work/Season 01").await;
    // The rule's last item came 5 weeks ago, but the feed could not be read for
    // the last 2 of them: 3 weeks of reading, not enough yet.
    app.received_weeks_ago(&rule, 5).await;
    app.read_up_to(NOW - 2 * WEEK).await;
    app.feed_readable(false);
    assert!(app.ids().await.is_empty());

    // A channel that keeps failing gives its rules no new quiet ground, however
    // long it goes on.
    app.at(NOW + 10 * WEEK);
    assert!(app.ids().await.is_empty());
    app.at(NOW);

    // Reading again from today: 6 days make 27 days of reading since the item,
    // and the 7th the 28th.
    app.feed_readable(true);
    app.at(NOW + 5 * DAY);
    assert!(app.ids().await.is_empty(), "6 days after reads succeed");
    app.at(NOW + 6 * DAY);
    let found = app.suggestions().await;
    assert_eq!(found.len(), 1, "a week after reads succeed again");
    assert_eq!(found[0]["grounds"][0]["kind"], "quiet");
    assert_eq!(found[0]["grounds"][0]["since"], NOW - 5 * WEEK);
}

#[tokio::test]
async fn a_worker_that_was_off_for_four_weeks_suggests_nothing_when_it_starts_again() {
    let app = App::new().await;
    let rule = app.rule("Work", "Work/Season 01").await;
    app.received_weeks_ago(&rule, 6).await;
    // The worker read the feed up to 4 weeks ago, was off since, and starts now.
    app.read_up_to(NOW - 4 * WEEK).await;
    app.feed_readable(false);
    app.at(NOW - 1);
    assert!(app.ids().await.is_empty());
    app.feed_readable(true);
    app.at(NOW);
    assert!(app.ids().await.is_empty(), "right after the restart");

    // Two weeks of reading on, 2 + 2 weeks of the 6 have been read.
    app.at(NOW + 12 * DAY);
    assert!(app.ids().await.is_empty());
    app.at(NOW + 13 * DAY);
    assert_eq!(app.ids().await, std::slice::from_ref(&rule.id));
}

#[tokio::test]
async fn a_channel_never_read_for_28_days_gives_no_quiet_ground_at_all() {
    let app = App::new().await;
    let rule = app.rule("Work", "Work/Season 01").await;
    app.received_weeks_ago(&rule, 20).await;
    // 27 days of reading and no more.
    app.read_through.store(read_day(NOW) - 27, Ordering::SeqCst);
    assert!(app.ids().await.is_empty(), "27 read days");
    app.at(NOW + DAY);
    assert_eq!(app.ids().await.len(), 1, "28 read days");
}

#[tokio::test]
async fn a_new_item_that_matches_the_rule_ends_the_quiet_whatever_became_of_it() {
    let app = App::new().await;
    let rule = app.rule("Work", "Work/Season 01").await;
    app.received_weeks_ago(&rule, 6).await;
    assert_eq!(app.ids().await, std::slice::from_ref(&rule.id));

    // Another show's item changes nothing.
    app.record(NOW - 2 * DAY, OTHER_5, HistoryResult::NoMatch, None)
        .await;
    assert_eq!(app.ids().await, std::slice::from_ref(&rule.id));

    // The rule's own item, recorded without anyone taking it, is a new item.
    app.record(NOW - 2 * DAY, WORK_5, HistoryResult::NoMatch, None)
        .await;
    assert!(app.ids().await.is_empty());
}

#[tokio::test]
async fn a_paused_rule_is_suggested_but_an_archived_one_and_a_waiting_subscription_are_not() {
    let app = App::new().await;
    let paused = app.rule("Paused", "Paused/Season 01").await;
    let archived = app.rule("Archived", "Archived/Season 01").await;
    let waiting = app.subscription(None, 9, Some("2026-01-01")).await;
    app.record(
        NOW - 52 * WEEK,
        "[Other] Old - 01",
        HistoryResult::NoMatch,
        None,
    )
    .await;
    for (rule, state) in [
        (&paused, RuleState::Paused),
        (&archived, RuleState::Archived),
    ] {
        app.state
            .channels
            .set_rule_state(&rule.id, state, NOW - 20 * WEEK)
            .await
            .unwrap();
    }

    let found = app.suggestions().await;
    let ids: Vec<&str> = found
        .iter()
        .map(|s| s["rule_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [paused.id.as_str()], "{waiting:?}");
    assert_eq!(found[0]["state"], "paused");
}

#[tokio::test]
async fn a_rule_that_never_received_counts_from_when_it_started() {
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
    app.restamp(&rule, NOW - 3 * WEEK).await;
    assert!(app.ids().await.is_empty(), "three weeks old, nothing yet");

    app.restamp(&rule, NOW - 5 * WEEK).await;
    let found = app.suggestions().await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["last_received_at"], Value::Null);
    assert_eq!(found[0]["grounds"][0]["since"], NOW - 5 * WEEK);
}

#[tokio::test]
async fn keeping_collecting_hides_the_same_ground_but_not_a_new_one() {
    let app = App::new().await;
    let rule = app.subscription(Some("Work"), 7, None).await;
    app.received_weeks_ago(&rule, 6).await;
    let found = app.suggestions().await;
    let quiet = found[0]["grounds"][0]["key"].as_str().unwrap().to_owned();

    let (status, body) = app.keep(&rule, &[&quiet]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["kept"], 1);
    assert!(app.ids().await.is_empty());

    // Nothing changes while it stays quiet, however long.
    app.at(NOW + 10 * WEEK);
    assert!(app.ids().await.is_empty());

    // The anime ends: a new ground, and only that one is listed.
    app.state
        .anissia
        .store
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

#[tokio::test]
async fn suggestions_are_listed_in_the_order_of_the_rules() {
    let app = App::new().await;
    let a = app.rule("A", "A/Season 01").await;
    let b = app.rule("B", "B/Season 01").await;
    app.record(
        NOW - 52 * WEEK,
        "[Other] Old - 01",
        HistoryResult::NoMatch,
        None,
    )
    .await;
    for rule in [&a, &b] {
        app.state
            .history
            .record(
                NOW - 6 * WEEK,
                vec![Observation {
                    channel_id: app.channel.id.clone(),
                    channel_label: app.channel.masked_url(),
                    identity_key: format!("title:{}", rule.id),
                    title: "[X] Y - 01".into(),
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
    assert_eq!(app.ids().await, [a.id, b.id]);
}
