//! The subscription suggestions of the legacy import, end to end through the
//! API with a stand-in for Anissia. No test reaches the real Anissia.

use std::{
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
use tempfile::TempDir;
use tower::ServiceExt;

use super::*;
use crate::{
    anissia::{fake::Fake, Anissia},
    store::{
        channels::{ChannelInput, NewSubscription, RuleInput, SubtitleMode},
        history::{HistoryQuery, HistoryResult, KnownItem, Observation},
        Db,
    },
    worker::{plan::ChannelPlan, Clock},
};

/// 2026-10-01 12:00 in Seoul.
const NOW: i64 = 1_790_780_400_000 + 12 * 60 * 60 * 1000;

const COMMENTED: &str = include_str!("../../../tests/fixtures/legacy_commented.yml");

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
        let router =
            Router::new().nest("/api", crate::web::api::router().with_state(state.clone()));
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
        let request = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
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
    assert!(again["blocked"].as_str().unwrap().contains("1번째 규칙"));

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
    assert_eq!(alpha.subtitles, SubtitleMode::Follow);
    assert_eq!(alpha.subscribed_at, NOW);
    let snapshot = app.state.anissia.store.anime(1001).await.unwrap().unwrap();
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
async fn an_address_only_suggestion_the_user_checks_is_a_subscription_with_the_creator_undecided() {
    let app = App::new().await;
    let content = app.real(COMMENTED);
    let (status, done) = app.apply(&content, json!([]), json!([pick(0, 1)])).await;
    assert_eq!(status, StatusCode::OK, "{done}");

    assert_eq!(
        app.followed().await,
        [("[SubsPlease] Beta Show - ".to_owned(), 1002, None)]
    );
    let channels = app.channels().await;
    let beta = channels[0].rules[1].subscription.as_ref().unwrap();
    assert_eq!(beta.subtitles, SubtitleMode::Undecided);
    assert_eq!(beta.creator, None);
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
    assert!(app.state.anissia.store.anime(1001).await.unwrap().is_none());
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
async fn skipping_a_channel_drops_its_suggestions_with_it() {
    let app = App::new().await;
    let content = app.real(COMMENTED);
    // The second channel exists already; the user skips it.
    app.state
        .settings
        .put_collection(0, format!("{}/media", app.root()), None)
        .await
        .unwrap();
    let existing = app
        .state
        .channels
        .create_channel_with_rules(
            ChannelInput::new("https://feeds.example.test/erai-raws?token=old"),
            vec![RuleInput {
                r#match: Some("Mine".into()),
                ..RuleInput::default()
            }],
        )
        .await
        .unwrap();
    let preview = app.preview(&content).await;
    assert_eq!(preview["conflict_count"], 1);
    let choices = json!([{
        "index": 1,
        "existing_id": existing.channel.id,
        "existing_version": existing.channel.version,
        "decision": "skip",
    }]);

    // The screen would not send the skipped channel's picks; a request that
    // does is answered by leaving them out.
    let (status, done) = app
        .apply(&content, choices, json!([pick(0, 0), pick(1, 0)]))
        .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["counts"]["channels_skipped"], 1);
    assert_eq!(done["counts"]["subscriptions_created"], 1);
    assert_eq!(
        app.followed().await,
        [(
            "[SubsPlease] Alpha Show - ".to_owned(),
            1001,
            Some("Team Alpha".to_owned())
        )]
    );
    // The skipped channel is exactly as it was.
    let channels = app.channels().await;
    let kept = channels
        .iter()
        .find(|c| c.channel.id == existing.channel.id)
        .unwrap();
    assert_eq!(kept.rules.len(), 1);
    // Epsilon's anime was never looked for.
    assert!(app.state.anissia.store.anime(1003).await.unwrap().is_none());
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

    assert_eq!(app.followed().await.len(), 1);
    // A stand-in snapshot the worker's daily refresh finds due at once.
    let snapshot = app.state.anissia.store.anime(1001).await.unwrap().unwrap();
    assert_eq!(snapshot.fetched_at, 0);
    assert_eq!(snapshot.air_time, None);
    let due = app.state.anissia.store.due(NOW).await.unwrap();
    assert_eq!(due.iter().map(|d| d.anime_no).collect::<Vec<_>>(), [1001]);
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
    assert!(not_created[0]["reason"]
        .as_str()
        .unwrap()
        .contains("편성표에 없는"));
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
    assert!(app.state.anissia.store.anime(1004).await.unwrap().is_none());
    // Every week was looked at before deciding.
    assert_eq!(app.fake.count("/anime/schedule/"), 9);
}

#[tokio::test]
async fn items_history_recorded_before_the_import_are_past_for_the_new_subscription() {
    let app = App::new().await;
    let content = app.real(COMMENTED);
    let folder = format!("{}/media", app.root());
    app.state
        .settings
        .put_collection(0, folder.clone(), None)
        .await
        .unwrap();
    let existing = app
        .state
        .channels
        .create_channel_with_rules(
            ChannelInput::new("https://feeds.example.test/subsplease?filter=1080p&token=old"),
            vec![RuleInput {
                r#match: Some("[SubsPlease] Alpha Show - ".into()),
                ..RuleInput::default()
            }],
        )
        .await
        .unwrap();
    // History holds an episode nothing picked, from a day before the import.
    let title = "[SubsPlease] Alpha Show - 05 (1080p) [ABCD1234].mkv";
    let seen = NOW - 24 * 60 * 60 * 1000;
    app.state
        .history
        .record(
            seen,
            vec![Observation {
                channel_id: existing.channel.id.clone(),
                channel_label: existing.channel.masked_url(),
                identity_key: format!("title:{title}"),
                title: title.into(),
                link: "https://feed.test/item".into(),
                result: HistoryResult::NoMatch,
                rule_id: None,
                torrent_hash: None,
                reason: None,
            }],
        )
        .await
        .unwrap();

    // Replacing keeps the rule's ID, and the user keeps its suggestion checked.
    let choices = json!([{
        "index": 0,
        "existing_id": existing.channel.id,
        "existing_version": existing.channel.version,
        "decision": "replace",
    }]);
    let (status, done) = app.apply(&content, choices, json!([pick(0, 0)])).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["counts"]["subscriptions_created"], 1);

    let channel = app
        .channels()
        .await
        .into_iter()
        .find(|c| c.channel.id == existing.channel.id)
        .unwrap();
    assert_eq!(
        channel.rules[0].id, existing.rules[0].id,
        "the rule kept its ID"
    );
    let subscribed_at = channel.rules[0]
        .subscription
        .as_ref()
        .unwrap()
        .subscribed_at;
    assert_eq!(subscribed_at, NOW);

    // The plan the worker uses calls the old item past for the subscription,
    // so the cycle leaves it for the user; a plain rule would have taken it.
    let known = app
        .state
        .history
        .known_items(existing.channel.id.clone(), vec![format!("title:{title}")])
        .await
        .unwrap();
    let known = known.get(&format!("title:{title}")).copied();
    assert_eq!(
        known,
        Some(KnownItem {
            first_seen_at: seen,
            result: HistoryResult::NoMatch,
            // The first record of a channel the import made.
            first_read: true
        })
    );
    let rule_id = channel.rules[0].id.clone();
    let plan = ChannelPlan::new(channel, std::path::Path::new(&folder));
    assert!(plan.is_past(&rule_id, known));
    // An item first seen after the import is not past.
    assert!(!plan.is_past(
        &rule_id,
        Some(KnownItem {
            first_seen_at: NOW + 1,
            result: HistoryResult::NoMatch,
            first_read: false
        })
    ));
}

#[tokio::test]
async fn a_pick_the_file_does_not_back_makes_the_review_stale_and_changes_nothing() {
    let app = App::new().await;
    let content = app.real(COMMENTED);
    // Rule 2 has a comment that cannot be read, rule 3 has none, rule 9 is not there.
    for (channel, rule) in [(0, 2), (0, 3), (0, 9)] {
        let (status, body) = app
            .apply(
                &content,
                json!([]),
                json!([pick(0, 0), pick(channel, rule)]),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{channel}:{rule} {body}");
    }
    assert!(app.channels().await.is_empty());
}

#[tokio::test]
async fn a_pick_that_cannot_be_a_subscription_is_reported_and_the_rule_is_imported_plain() {
    let app = App::new().await;
    let content = app.real(COMMENTED);
    // Rule 4 offers Alpha again after rule 0 does.
    let (status, done) = app
        .apply(&content, json!([]), json!([pick(0, 0), pick(0, 4)]))
        .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["counts"]["subscriptions_created"], 1);
    assert_eq!(done["counts"]["rules_added"], 7);
    let not_created = done["subscriptions"]["not_created"].as_array().unwrap();
    assert_eq!(not_created.len(), 1);
    assert_eq!(
        (
            not_created[0]["channel"].as_u64(),
            not_created[0]["rule"].as_u64()
        ),
        (Some(0), Some(4))
    );
    assert!(not_created[0]["reason"]
        .as_str()
        .unwrap()
        .contains("같은 작품"));
    assert_eq!(app.followed().await.len(), 1);
}

#[tokio::test]
async fn a_rule_saving_into_the_collect_folder_itself_is_offered_but_blocked() {
    let app = App::new().await;
    let file = "\
- url: https://feeds.example.test/x?token=t
  directory: /media
  rules:
    # Wed. 22:30. Team
    # https://anissia.net/anime?animeNo=1001
    - match: Direct
      directory: ''
";
    let content = app.real(file);
    app.state
        .settings
        .put_collection(0, format!("{}/media", app.root()), None)
        .await
        .unwrap();
    let preview = app.preview(&content).await;
    let s = suggestion(&preview, 0, 0);
    assert_eq!(s["kind"], "with_creator");
    assert_eq!(s["checked"], false);
    assert!(s["blocked"].as_str().unwrap().contains("수집 폴더"));

    let (status, done) = app.apply(&content, json!([]), json!([pick(0, 0)])).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["counts"]["subscriptions_created"], 0);
    assert_eq!(
        done["subscriptions"]["not_created"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(app.followed().await.is_empty());
}

#[tokio::test]
async fn a_rule_saving_through_a_parent_folder_is_blocked_in_the_preview_and_refused_on_apply() {
    let app = App::new().await;
    let file = "\
- url: https://feeds.example.test/x?token=t
  directory: /media
  rules:
    # Wed. 22:30. Team
    # https://anissia.net/anime?animeNo=1001
    - match: Up
      directory: ../Elsewhere
";
    let content = app.real(file);
    app.state
        .settings
        .put_collection(0, format!("{}/media", app.root()), None)
        .await
        .unwrap();
    let preview = app.preview(&content).await;
    let s = suggestion(&preview, 0, 0);
    assert_eq!(s["checked"], false);
    assert!(s["blocked"].as_str().unwrap().contains(".."), "{s}");

    // A client that checks it anyway gets the rule without a subscription.
    let (status, done) = app.apply(&content, json!([]), json!([pick(0, 0)])).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["counts"]["subscriptions_created"], 0);
    assert_eq!(
        done["subscriptions"]["not_created"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(app.followed().await.is_empty());
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
    let snapshot = crate::anissia::parse::schedule(&[anime], 2)
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
