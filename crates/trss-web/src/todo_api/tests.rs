//! `자막 구독` and the subscribed creator's auto receipts as the screens see
//! them (`trss_jobs::follow` has the decisions themselves).

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::AppState;
use trss_collect::store::{
    channels::{ChannelInput, NewSubscription, Rule, RuleInput, SubtitleMode},
    history::{HistoryResult, Observation},
};
use trss_core::{Db, DbError};
use trss_jobs::{area::ReceiveArea, Runner};
use trss_subtitles::{fake::FakeSource, Sources};

const ANIME: i64 = 3424;
/// Episode 1 of the season airs here (Unix seconds); one episode a week.
const AIRED: i64 = 1_790_000_000;
const WEEK: i64 = 7 * 24 * 3600;

struct App {
    state: AppState,
    router: Router,
    _dir: tempfile::TempDir,
    runner: Runner,
    rule: Rule,
}

async fn sql(state: &AppState, sql: String) {
    state
        .jobs
        .db()
        .run::<_, DbError, _>(move |c| Ok(c.execute_batch(&sql)?))
        .await
        .unwrap();
}

impl App {
    /// Work `w1` season 1, linked to anime [`ANIME`] by a collecting
    /// subscription that gets subtitles with no creator chosen yet.
    async fn new() -> App {
        let state = AppState::new(Db::open_blocking(":memory:").unwrap());
        // The season's 12 episodes, aired weekly: what the creators' lines are
        // read against.
        let airing = (1..=12)
            .map(|k| format!(r#"{{"episode":{k},"at":{}}}"#, AIRED + (k - 1) * WEEK))
            .collect::<Vec<_>>()
            .join(",");
        sql(
            &state,
            format!(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
                 INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
                 INSERT INTO seasons (work_id, number) VALUES ('w1', 1);
                 INSERT INTO anilist_entries (id, format, episodes, airing, fetched_at)
                     VALUES (1, 'TV', 12, '[{airing}]', 1);
                 INSERT INTO season_entries (work_id, season, position, anilist_id)
                     VALUES ('w1', 1, 0, 1);"
            ),
        )
        .await;
        let channel = state
            .channels
            .create_channel(ChannelInput::new("https://feed.test/rss"))
            .await
            .unwrap();
        let rule = state
            .channels
            .create_subscription_rule(
                &channel.id,
                RuleInput {
                    r#match: Some("Show".into()),
                    directory: "Show".into(),
                    episode: 0,
                    ..RuleInput::default()
                },
                NewSubscription {
                    anime: trss_anissia::Anime {
                        anime_no: ANIME,
                        subject: "작품".into(),
                        original_subject: None,
                        week: 2,
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
        state.channels.link_season(&rule.id, "w1:1").await.unwrap();
        let rule = state.channels.get_rule(&rule.id).await.unwrap().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let runner = Runner::new(
            state.jobs.clone(),
            Sources::none().with_fake(FakeSource),
            ReceiveArea::in_app_data(dir.path()),
            Arc::new(|| 2_000),
        );
        let router = Router::new().nest("/api", crate::api::router().with_state(state.clone()));
        App {
            state,
            router,
            _dir: dir,
            runner,
            rule,
        }
    }

    /// Lines of the anime as the reading stored them: `(source, creator,
    /// episode, path, updated)`. Anissia wrote each an hour after its episode
    /// aired (`updated` is only the text).
    async fn observe(&self, lines: &[(&str, &str, &str, &str, &str)]) {
        let mut batch = String::new();
        for (source, creator, episode, path, updated) in lines {
            let at = episode.parse::<i64>().map_or("NULL".to_owned(), |n| {
                ((AIRED + (n - 1) * WEEK + 3600) * 1000).to_string()
            });
            batch.push_str(&format!(
                "INSERT OR IGNORE INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('{source}', {ANIME}, '{creator}', 1);
                 INSERT INTO caption_observations (source_id, post_url, episode, updated,
                     updated_at, first_seen_at)
                     VALUES ('{source}', 'https://fake.trss.invalid{path}', '{episode}',
                             '{updated}', {at}, 7);"
            ));
        }
        sql(&self.state, batch).await;
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let request = Request::builder().method(method).uri(uri);
        let request = match body {
            Some(body) => request
                .header("content-type", "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn get(&self, uri: &str) -> Value {
        let (status, body) = self.call(Method::GET, uri, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        body
    }

    async fn run(&self) {
        self.runner
            .run_ready(&CancellationToken::new())
            .await
            .unwrap();
    }

    /// Every job the screen lists, whatever its group.
    async fn jobs(&self) -> Vec<Value> {
        let groups = self.get("/api/subtitle-jobs").await;
        let mut all = Vec::new();
        for group in ["failed", "waiting", "running"] {
            all.extend(groups[group].as_array().unwrap().iter().cloned());
        }
        all.extend(groups["done"]["items"].as_array().unwrap().iter().cloned());
        all
    }
}

#[tokio::test]
async fn a_work_without_a_creator_is_one_suggestion_with_no_names_and_no_badge() {
    let app = App::new().await;
    app.observe(&[
        ("s1", "에루샤", "1", "/ok/a1", "x"),
        ("s1", "에루샤", "2", "/ok/a2", "x"),
        ("s2", "코코렛", "3", "/ok/b3", "x"),
        ("s2", "코코렛", "04", "/ok/b4", "x"),
        ("s1", "에루샤", "4", "/ok/a4", "x"),
    ])
    .await;

    let shown = app.get("/api/todo/subtitle-follow").await;
    assert_eq!(
        shown,
        json!({ "suggestions": [{
            "work": { "id": "w1", "name": "Show", "cover_url": null },
            "title": "작품", "season": 1, "rule_id": app.rule.id, "anime_no": ANIME,
            "episodes": ["1", "2", "3", "04"], "creators": 2, "since": 7 }] })
    );
    assert!(!shown.to_string().contains("에루샤"));
    // A suggestion is no to-do: the badge does not count it.
    assert_eq!(
        app.get("/api/todo").await,
        json!({ "needs": [], "count": 0 })
    );
    assert!(app.jobs().await.is_empty());
}

#[tokio::test]
async fn choosing_a_candidates_creator_receives_its_episodes_and_a_check_is_a_to_do() {
    let app = App::new().await;
    // 에루샤's post of episode 2 asks for a check. The creators are chosen from
    // the lines the app observed, without asking Anissia.
    app.observe(&[
        ("s1", "에루샤", "1", "/ok/a1", "x"),
        ("s1", "에루샤", "2", "/auth/a2", "x"),
        ("s2", "코코렛", "3", "/ok/b3", "x"),
    ])
    .await;

    let (status, rule) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{}/creator", app.rule.id),
            Some(json!({ "version": app.rule.version, "creator": "코코렛" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{rule}");
    let jobs = app.jobs().await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["origin"], "auto");
    assert_eq!(jobs[0]["episodes"], json!(["3"]));
    assert_eq!(jobs[0]["revision_of"], Value::Null);
    assert_eq!(jobs[0]["revises_job"], Value::Null);
    // The work has a creator now.
    assert_eq!(
        app.get("/api/todo/subtitle-follow").await,
        json!({ "suggestions": [] })
    );

    // Changed again: the new creator's episodes from now on, the received one stays.
    let version = app
        .state
        .channels
        .get_rule(&app.rule.id)
        .await
        .unwrap()
        .unwrap()
        .version;
    app.run().await;
    let (status, _) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{}/creator", app.rule.id),
            Some(json!({ "version": version, "creator": "에루샤" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    app.run().await;
    let jobs = app.jobs().await;
    let mut episodes: Vec<&str> = jobs
        .iter()
        .map(|j| j["episodes"][0].as_str().unwrap())
        .collect();
    episodes.sort();
    assert_eq!(episodes, ["1", "2", "3"]);
    assert!(jobs.iter().all(|j| j["origin"] == "auto"));

    // Episode 2 waits for a person: that is a to-do, counted in the badge.
    let todo = app.get("/api/todo").await;
    assert_eq!(todo["count"], 1);
    assert_eq!(todo["needs"][0]["kind"], "auth");
    assert_eq!(todo["needs"][0]["episodes"], json!(["2"]));

    // The candidates show the app's mapping of the creator's source.
    let shown = app
        .get("/api/library/works/w1/seasons/1/anissia/candidates")
        .await;
    let mappings = shown["mappings"].as_array().unwrap();
    let s1 = mappings.iter().find(|m| m["source_id"] == "s1").unwrap();
    assert_eq!(s1["kind"], "auto");
    assert_eq!(s1["offset"], 0);
    assert!(s1["evidence"].as_str().unwrap().contains("1화·2화"), "{s1}");
}

#[tokio::test]
async fn a_revision_job_names_the_receipt_it_revises() {
    let app = App::new().await;
    app.observe(&[("s1", "에루샤", "1", "/ok/a1", "x")]).await;
    let (status, _) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{}/creator", app.rule.id),
            Some(json!({ "version": app.rule.version, "creator": "에루샤" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    app.run().await;
    let first = app.jobs().await[0]["id"].as_str().unwrap().to_owned();

    // The post fixed: observation 2, made a job when the worker looks next.
    app.observe(&[("s1", "에루샤", "1", "/ok/a1", "y")]).await;
    let made = app.state.follow.evaluate(3_000).await.unwrap();
    assert_eq!(made.len(), 1);
    let (_, detail) = app
        .call(
            Method::GET,
            &format!("/api/subtitle-jobs/{}", made[0]),
            None,
        )
        .await;
    assert_eq!(detail["origin"], "auto");
    assert_eq!(detail["revision_of"], 1);
    assert_eq!(detail["revises_job"], first);
}

#[tokio::test]
async fn turning_subtitles_off_takes_the_work_out_of_the_suggestions() {
    let app = App::new().await;
    app.observe(&[("s1", "에루샤", "1", "/ok/a1", "x")]).await;
    app.state
        .channels
        .set_subtitle_receiving(&app.rule.id, app.rule.version, false)
        .await
        .unwrap();
    assert_eq!(
        app.get("/api/todo/subtitle-follow").await,
        json!({ "suggestions": [] })
    );

    // On again: suggested again, and still nothing received.
    let version = app
        .state
        .channels
        .get_rule(&app.rule.id)
        .await
        .unwrap()
        .unwrap()
        .version;
    let (status, body) = app
        .call(
            Method::PUT,
            &format!("/api/rules/{}/switch", app.rule.id),
            Some(json!({ "version": version, "subtitles": true })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        app.get("/api/todo/subtitle-follow").await["suggestions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(app.jobs().await.is_empty());
}

/// A to-do of `kind` about `work`, the rest of it left plain.
fn todo_of(kind: &str, work: Option<&str>) -> super::Todo {
    use super::{Changes, Todo};
    let work = work.map(|id| crate::jobs_api::WorkRefView {
        id: id.to_owned(),
        name: id.to_owned(),
        cover_url: None,
    });
    let (key, at, title) = (format!("{kind}:{work:?}"), 1, "작품".to_owned());
    match kind {
        "auth" => Todo::Auth {
            key,
            at,
            work,
            title,
            season: None,
            episodes: vec![],
            creator: None,
            reason: String::new(),
            job_id: "j".into(),
            jobs: 1,
        },
        "receive_failed" => Todo::ReceiveFailed {
            key,
            at,
            context: "revision",
            work,
            title,
            season: None,
            episodes: vec![],
            count: 1,
            reason: None,
            channel_id: None,
        },
        "episode_check" => Todo::EpisodeCheck {
            key,
            at,
            work,
            title,
            season: 1,
            creator: String::new(),
            source_id: String::new(),
            episodes: vec![],
            reason: None,
            sources: 1,
        },
        "placement_check" => Todo::PlacementCheck {
            key,
            at,
            work,
            title,
            season: None,
            creator: None,
            origin: "upload".into(),
            source: None,
            files: vec![],
            reason: None,
            job_id: "j".into(),
        },
        "video_check" => Todo::VideoCheck {
            key,
            at,
            work,
            title,
            season: 1,
            path: "Season 01/extra.mkv".into(),
            reason: String::new(),
            seen: "1:1".into(),
        },
        "replacement" => Todo::Replacement {
            key,
            at,
            work,
            title,
            season: None,
            episodes: vec![],
            creator: None,
            job_id: "j".into(),
            jobs: 1,
            changes: Changes::default(),
            current_received_at: None,
            current_changed_at: None,
            new_received_at: None,
        },
        _ => unreachable!("{kind}"),
    }
}

#[test]
fn a_works_badges_are_its_kinds_once_each_in_the_lists_order() {
    // As `todo_list` orders them: the red kinds first.
    let todos = [
        todo_of("auth", Some("w2")),
        todo_of("receive_failed", Some("w1")),
        todo_of("receive_failed", Some("w1")),
        todo_of("replacement", Some("w1")),
        todo_of("replacement", None),
        todo_of("episode_check", Some("w3")),
        todo_of("placement_check", Some("w3")),
        todo_of("placement_check", Some("w1")),
        todo_of("video_check", Some("w4")),
        todo_of("video_check", Some("w3")),
    ];

    let badges = super::badges_by_work(&todos);

    assert_eq!(
        badges,
        std::collections::HashMap::from([
            (
                "w1".to_owned(),
                vec!["receive_failed", "replacement", "episode_check"]
            ),
            ("w2".to_owned(), vec!["auth"]),
            // A job's 배치 확인 and a video's episode are the same badge as
            // a mapping's.
            ("w3".to_owned(), vec!["episode_check"]),
            ("w4".to_owned(), vec!["episode_check"]),
        ])
    );
}

/// A history item a rule picked and Transmission did not add is listed with
/// the receive failures (`GET /api/todo/receive-failures`), with Transmission's
/// reason.
#[tokio::test]
async fn an_add_failure_is_in_the_receive_failure_source_too() {
    let app = App::new().await;
    let title = "[SubsPlease] Show - 14 (1080p) [8F2EFECC].mkv";
    app.state
        .history
        .record(
            1,
            vec![Observation {
                channel_id: app.rule.channel_id.clone(),
                channel_label: "https://feed.test/rss".into(),
                identity_key: "guid:14".into(),
                title: title.into(),
                link: "magnet:?xt=urn:btih:1111000000000000000000000000000000000014".into(),
                result: HistoryResult::AddFailed,
                rule_id: Some(app.rule.id.clone()),
                torrent_hash: None,
                reason: Some("duplicate torrent? no: refused".into()),
            }],
        )
        .await
        .unwrap();

    let list = app.get("/api/todo/receive-failures").await;
    let items = list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{list}");
    assert_eq!(items[0]["kind"], "add_failed");
    assert_eq!(items[0]["title"], title);
    assert!(items[0]["reason"].as_str().unwrap().contains("refused"));
}
