//! The subscription suggestions of the legacy import, end to end through the
//! API with a stand-in for Anissia. No test reaches the real Anissia.

use crate::testing;
use std::{
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::{
    http::{Method, StatusCode},
    Router,
};
use serde_json::{json, Value};
use tempfile::TempDir;

use trss_core::{Clock, Db};

use super::*;
use trss_anissia::{fake::Fake, Anissia};
use trss_collect::store::{
    channels::{ChannelInput, NewSubscription, RuleInput, SubtitleMode},
    history::HistoryQuery,
};

/// 2026-10-01 12:00 in Seoul.
const NOW: i64 = 1_790_780_400_000 + 12 * 60 * 60 * 1000;

const COMMENTED: &str = include_str!("../../../trss-import/tests/fixtures/legacy_commented.yml");

struct App {
    dir: TempDir,
    state: AppState,
    router: Router,
    fake: Fake,
}

impl App {
    async fn new() -> App {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("app.db")).await.unwrap();
        let fake = Fake::start().await;
        let now = Arc::new(AtomicI64::new(NOW));
        let clock: Clock = Arc::new(move || now.load(Ordering::SeqCst));
        let anissia = Anissia::new(db.clone(), fake.config(), clock).with_spacing(Duration::ZERO);
        let state = AppState::new(db).with_anissia(anissia);
        let router = testing::api(&state);
        let app = App {
            dir,
            state,
            router,
            fake,
        };
        // Alpha (1001) on Wednesdays, Beta (1002) on Thursdays, Epsilon (1003)
        // on Fridays; Zeta (1004) is on no weekday's list.
        app.fake.set_week(
            3,
            vec![app.fake.entry(3, 1001, "22:30", "알파", "Alpha Show")],
        );
        app.fake.set_week(
            4,
            vec![app.fake.entry(4, 1002, "23:00", "베타", "Beta Show")],
        );
        app.fake.set_week(
            5,
            vec![app.fake.entry(5, 1003, "21:00", "엡실론", "Epsilon Show")],
        );
        app
    }

    fn root(&self) -> String {
        self.dir.path().to_str().unwrap().to_owned()
    }

    /// `content` with `/media` moved under the scratch folder, created there.
    fn real(&self, content: &str) -> String {
        let root = self.root();
        std::fs::create_dir_all(format!("{root}/media/anime")).unwrap();
        content.replace("/media", &format!("{root}/media"))
    }

    async fn post(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        testing::call(&self.router, Method::POST, uri, Some(body)).await
    }

    async fn preview(&self, content: &str) -> Value {
        let (status, body) = self
            .post("/api/import/legacy/preview", json!({ "content": content }))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn apply(
        &self,
        content: &str,
        choices: Value,
        subscriptions: Value,
    ) -> (StatusCode, Value) {
        let reviewed = self
            .state
            .settings
            .collection()
            .await
            .unwrap()
            .map(|c| c.folder);
        self.post(
            "/api/import/legacy/apply",
            json!({
                "content": content,
                "choices": choices,
                "reviewed_collect_folder": reviewed,
                "subscriptions": subscriptions,
            }),
        )
        .await
    }

    async fn channels(&self) -> Vec<ChannelWithRules> {
        self.state
            .channels
            .list_channels_with_rules()
            .await
            .unwrap()
    }

    /// The anime a rule follows and the creator, by rule match phrase.
    async fn followed(&self) -> Vec<(String, i64, Option<String>)> {
        let mut out = Vec::new();
        for channel in self.channels().await {
            for rule in channel.rules {
                if let Some(s) = rule.subscription {
                    out.push((rule.r#match.unwrap(), s.anissia_anime_no, s.creator));
                }
            }
        }
        out
    }
}

fn pick(channel: usize, rule: usize) -> Value {
    json!({ "channel": channel, "rule": rule })
}

fn suggestion(preview: &Value, channel: usize, rule: usize) -> &Value {
    &preview["channels"][channel]["rules"][rule]["suggestion"]
}

#[tokio::test]
async fn the_preview_offers_the_four_cases_checked_as_the_spec_says_without_asking_anissia() {
    let app = App::new().await;
    let preview = app.preview(&app.real(COMMENTED)).await;

    // Address and creator: checked at first.
    let with_creator = suggestion(&preview, 0, 0);
    assert_eq!(with_creator["kind"], "with_creator");
    assert_eq!(with_creator["anime_no"], 1001);
    assert_eq!(with_creator["creator"], "Team Alpha");
    // What the comment says of the weekday and time rides along as a fallback.
    assert_eq!(
        with_creator["comment_airs"],
        json!({ "week": 3, "time": "22:30" })
    );
    assert_eq!(with_creator["checked"], true);
    assert_eq!(with_creator["blocked"], Value::Null);

    // Address only: `제작자 미정`, unchecked.
    let address_only = suggestion(&preview, 0, 1);
    assert_eq!(address_only["kind"], "address_only");
    assert_eq!(address_only["anime_no"], 1002);
    assert_eq!(address_only["creator"], Value::Null);
    assert_eq!(address_only["comment_airs"]["week"], 4);
    assert_eq!(address_only["checked"], false);

    // A comment that cannot be read says why, and offers nothing.
    let unreadable = suggestion(&preview, 0, 2);
    assert_eq!(unreadable["kind"], "unreadable");
    assert_eq!(unreadable["reason"], "Anissia 주소가 없어서");
    assert_eq!(unreadable["checked"], false);
    assert_eq!(unreadable["anime_no"], Value::Null);

    // No comment.
    let none = suggestion(&preview, 0, 3);
    assert_eq!(none["kind"], "none");
    assert_eq!(none["checked"], false);

    // The same anime again in the same channel cannot be a second subscription.
    let again = suggestion(&preview, 0, 4);
    assert_eq!(again["kind"], "with_creator");
    assert_eq!(again["checked"], false);
    assert!(again["blocked"].is_string());

    // Another channel follows the same kind of comments on its own.
    assert_eq!(suggestion(&preview, 1, 0)["creator"], "Team Epsilon");
    assert_eq!(suggestion(&preview, 1, 1)["kind"], "address_only");

    // The preview is instant: it never asks Anissia.
    assert!(app.fake.requests().is_empty());
}

#[tokio::test]
async fn a_first_run_import_adds_everything_unasked_and_the_checked_suggestions_become_subscriptions(
) {
    let app = App::new().await;
    let content = app.real(COMMENTED);

    // First run: nothing exists, so nothing is asked, and suggestions show.
    let preview = app.preview(&content).await;
    assert_eq!(preview["conflict_count"], 0);
    assert_eq!(suggestion(&preview, 0, 0)["checked"], true);

    // The user keeps the two suggestions that start checked: Alpha and Epsilon.
    let (status, done) = app
        .apply(&content, json!([]), json!([pick(0, 0), pick(1, 0)]))
        .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["counts"]["channels_added"], 2);
    assert_eq!(done["counts"]["rules_added"], 7);
    assert_eq!(done["counts"]["subscriptions_created"], 2);
    assert_eq!(done["subscriptions"]["unavailable"], Value::Null);
    let created = done["subscriptions"]["created"].as_array().unwrap();
    assert_eq!(created.len(), 2);
    assert_eq!(created[0]["subject"], "알파");
    assert_eq!(created[0]["schedule_known"], true);
    assert_eq!(created[0]["creator"], "Team Alpha");

    assert_eq!(
        app.followed().await,
        [
            (
                "[SubsPlease] Alpha Show - ".to_owned(),
                1001,
                Some("Team Alpha".to_owned())
            ),
            (
                "Epsilon Show".to_owned(),
                1003,
                Some("Team Epsilon".to_owned())
            ),
        ]
    );
    // The weekday and time were read from Anissia, and the import time is now.
    let channels = app.channels().await;
    let alpha = channels[0].rules[0].subscription.as_ref().unwrap();
    assert_eq!(alpha.subscribed_at, NOW);
    let snapshot = app.state.anissia_store.anime(1001).await.unwrap().unwrap();
    assert_eq!(
        (snapshot.week, snapshot.air_time.as_deref()),
        (3, Some("22:30"))
    );
    assert_eq!(snapshot.subject, "알파");
    // The rules the user did not check are plain rules.
    for rule in channels[0].rules.iter().skip(1) {
        assert!(rule.subscription.is_none(), "{:?}", rule.r#match);
    }
    // The creators were not checked against the caption list for the import.
    assert_eq!(app.fake.count("/caption/"), 0);
}

#[tokio::test]
async fn unchecked_suggestions_import_the_rules_alone_and_ask_nothing_of_anissia() {
    let app = App::new().await;
    let content = app.real(COMMENTED);
    // An apply that names no suggestion at all, as the screen sends after
    // `모두 해제` (and as a request that leaves the field out).
    let (status, done) = app
        .post(
            "/api/import/legacy/apply",
            json!({ "content": content, "choices": [], "reviewed_collect_folder": null }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{done}");

    assert_eq!(done["counts"]["rules_added"], 7);
    assert_eq!(done["counts"]["subscriptions_created"], 0);
    assert_eq!(done["subscriptions"]["created"], json!([]));
    assert!(app.followed().await.is_empty());
    assert!(app.fake.requests().is_empty());
    // No snapshot either: nothing follows an anime.
    assert!(app.state.anissia_store.anime(1001).await.unwrap().is_none());
}

#[tokio::test]
async fn an_import_receives_nothing_and_leaves_history_and_commands_alone() {
    let app = App::new().await;
    let content = app.real(COMMENTED);
    let before = app
        .state
        .history
        .list(HistoryQuery::default())
        .await
        .unwrap()
        .items
        .len();
    let (status, _) = app
        .apply(
            &content,
            json!([]),
            json!([pick(0, 0), pick(0, 1), pick(1, 0)]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    assert!(!app.state.commands.has_open().await.unwrap());
    let after = app
        .state
        .history
        .list(HistoryQuery::default())
        .await
        .unwrap()
        .items
        .len();
    assert_eq!(before, after);
    // The Anissia requests were reads of the schedule, nothing else.
    assert!(app
        .fake
        .requests()
        .iter()
        .all(|(_, path)| path.starts_with("/anime/schedule/")));
}

#[tokio::test]
async fn when_anissia_cannot_be_reached_the_subscription_is_kept_with_an_unknown_schedule() {
    let app = App::new().await;
    let content = app.real(COMMENTED);
    app.fake.state.lock().unwrap().failing = 100;

    let (status, done) = app.apply(&content, json!([]), json!([pick(0, 0)])).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["counts"]["subscriptions_created"], 1);
    assert!(done["subscriptions"]["unavailable"]
        .as_str()
        .unwrap()
        .contains("Anissia"));
    let created = &done["subscriptions"]["created"][0];
    assert_eq!(created["schedule_known"], false);
    assert_eq!(created["subject"], Value::Null);
    assert_eq!(created["creator"], "Team Alpha");
    assert_eq!(created["schedule_from_comment"], true);

    assert_eq!(app.followed().await.len(), 1);
    // A stand-in snapshot, which the worker's daily refresh replaces.
    let snapshot = app.state.anissia_store.anime(1001).await.unwrap().unwrap();
    assert_eq!(snapshot.fetched_at, 0);
}

#[tokio::test]
async fn the_comments_weekday_is_not_used_when_anissia_answered() {
    let app = App::new().await;
    let content = app.real(COMMENTED);
    // Anissia lists Alpha on its Wednesday list; the comment of rule (0, 4)
    // says Saturday 01:00 for the same anime. Anissia's word stands, and no
    // stand-in is written.
    let (status, done) = app.apply(&content, json!([]), json!([pick(0, 0)])).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    let created = &done["subscriptions"]["created"][0];
    assert_eq!(created["schedule_known"], true);
    assert_eq!(created["schedule_from_comment"], false);
    let snapshot = app.state.anissia_store.anime(1001).await.unwrap().unwrap();
    assert!(snapshot.fetched_at > 0);
    assert_eq!(
        (snapshot.week, snapshot.air_time.as_deref()),
        (3, Some("22:30"))
    );
}

#[tokio::test]
async fn an_anime_the_schedule_does_not_list_is_refused_and_its_rule_imported_plain() {
    let app = App::new().await;
    let content = app.real(COMMENTED);
    // Zeta (1004) is on no list: Anissia answered, so there is no stand-in. It is
    // picked next to Alpha (1001), which is listed.
    let (status, done) = app
        .apply(&content, json!([]), json!([pick(0, 0), pick(1, 1)]))
        .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["subscriptions"]["unavailable"], Value::Null);
    assert_eq!(done["counts"]["subscriptions_created"], 1);
    let created = done["subscriptions"]["created"].as_array().unwrap();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0]["anime_no"], 1001);
    assert_eq!(created[0]["schedule_known"], true);
    let not_created = done["subscriptions"]["not_created"].as_array().unwrap();
    assert_eq!(not_created.len(), 1);
    assert_eq!(
        (
            not_created[0]["channel"].as_u64(),
            not_created[0]["rule"].as_u64()
        ),
        (Some(1), Some(1))
    );
    assert!(not_created[0]["reason"].is_string());
    // The rule came in without a subscription, and no snapshot stands in for it.
    assert_eq!(
        app.followed().await,
        [(
            "[SubsPlease] Alpha Show - ".to_owned(),
            1001,
            Some("Team Alpha".to_owned())
        )]
    );
    assert_eq!(done["counts"]["rules_added"], 7);
    assert!(app.state.anissia_store.anime(1004).await.unwrap().is_none());
    // Every week was looked at before deciding.
    assert_eq!(app.fake.count("/anime/schedule/"), 9);
}

#[tokio::test]
async fn a_pick_the_file_does_not_back_makes_the_review_stale_and_changes_nothing() {
    let app = App::new().await;
    let content = app.real(COMMENTED);
    // Rule 2 has a comment that cannot be read. (What else makes a pick stale
    // is decided in `trss_import::picks`.)
    let (status, body) = app
        .apply(&content, json!([]), json!([pick(0, 0), pick(0, 2)]))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(app.channels().await.is_empty());
}

#[tokio::test]
async fn a_replaced_rule_that_follows_an_anime_already_keeps_it_and_the_preview_says_so() {
    let app = App::new().await;
    let content = app.real(COMMENTED);
    app.state
        .settings
        .put_collection(0, format!("{}/media", app.root()), None)
        .await
        .unwrap();
    let existing = app
        .state
        .channels
        .create_channel_with_rules(
            ChannelInput::new("https://feeds.example.test/subsplease?filter=1080p&token=old"),
            vec![RuleInput {
                r#match: Some("[SubsPlease] Alpha Show - ".into()),
                directory: "anime/Alpha".into(),
                ..RuleInput::default()
            }],
        )
        .await
        .unwrap();
    let anime = app.fake.entry(2, 9001, "20:00", "이미", "Already");
    let snapshot = trss_anissia::parse::schedule(&[anime], 2)
        .unwrap()
        .remove(0);
    app.state
        .channels
        .subscribe_rule(
            &existing.rules[0].id,
            existing.rules[0].version,
            NewSubscription {
                anime: snapshot.snapshot(NOW),
                subtitles: SubtitleMode::Undecided,
                creator: None,
                subscribed_at: NOW,
            },
        )
        .await
        .unwrap();
    let existing = app.channels().await.remove(0);

    let preview = app.preview(&content).await;
    let s = suggestion(&preview, 0, 0);
    assert_eq!(s["keeps_subscription"], true);
    // It still starts checked; the screen leaves it out while the channel is
    // replaced, and a copy (`추가`) would make a new rule for it.
    assert_eq!(s["checked"], true);

    let choices = json!([{
        "index": 0,
        "existing_id": existing.channel.id,
        "existing_version": existing.channel.version,
        "decision": "replace",
    }]);
    let (status, done) = app
        .apply(&content, choices, json!([pick(0, 0), pick(0, 1)]))
        .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    // Alpha's rule keeps the anime it followed; Beta's becomes a subscription.
    let followed = app.followed().await;
    assert!(followed.contains(&("[SubsPlease] Alpha Show - ".to_owned(), 9001, None)));
    assert!(followed.contains(&("[SubsPlease] Beta Show - ".to_owned(), 1002, None)));
    assert_eq!(done["counts"]["subscriptions_created"], 1);
    assert_eq!(
        done["subscriptions"]["not_created"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
