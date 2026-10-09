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

use super::failure_of;
use crate::AppState;
use trss_collect::store::{
    channels::{ChannelInput, NewSubscription, Rule, RuleInput, SubtitleMode},
    history::{HistoryResult, Observation},
    revisions::{NewRevision, Revision, RevisionState, Step},
};
use trss_core::{Db, DbError};
use trss_jobs::{area::ReceiveArea, JobRun, Runner};
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
            JobRun::new(state.db().clone()),
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
        json!({ "needs": [], "count": 0, "badges": {} })
    );
    assert!(app.jobs().await.is_empty());
}

#[tokio::test]
async fn choosing_a_candidates_creator_receives_its_episodes_and_the_candidates_show_its_mapping() {
    let app = App::new().await;
    // The creators are chosen from the lines the app observed, without asking
    // Anissia.
    app.observe(&[
        ("s1", "에루샤", "1", "/ok/a1", "x"),
        ("s1", "에루샤", "2", "/ok/a2", "x"),
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

// --- a failed replacement of a video revision ----------------------------------------

const EPISODE_NAME: &str = "Show S01E14.mkv";
const NEW_NAME: &str = "[SubsPlease] Show - 14v2 (1080p) [8F2EFECC].mkv";
const SEASON_FOLDER: &str = "/media/Show/Season 01";

/// A replacement row as the store keeps it, in `state` with the given
/// `reason`, received name and claim; the fields `failure_of` does not read
/// are made up.
fn replacement(
    state: RevisionState,
    reason: Option<&str>,
    received_name: Option<&str>,
    claimed_at: Option<i64>,
) -> Revision {
    Revision {
        id: 1,
        item_id: 7,
        old_item_id: None,
        rule_id: "r1".into(),
        folder: SEASON_FOLDER.into(),
        episode_name: EPISODE_NAME.into(),
        old_version: Some(1),
        new_version: 2,
        old_crc: None,
        old_torrent_hash: None,
        expected_crc: Some("8F2EFECC".into()),
        torrent_hash: None,
        received_name: received_name.map(str::to_owned),
        file_crc: None,
        file_identity: None,
        new_missing_at: None,
        folder_away_since: None,
        claimed_at,
        superseded_hash: None,
        state,
        reason: reason.map(str::to_owned),
        created_at: 10,
        updated_at: 20,
        replaced_at: None,
        overtaken_by: None,
    }
}

/// What a failed replacement says of its two files comes from the row alone:
/// the old video is `removed` once the replacement went ahead to remove it
/// (or was received again after that) and `kept` before, the new one is
/// under its received name, `missing` after the replacement ended, or not
/// received yet.
#[test]
fn a_failed_replacements_files_are_told_by_its_row() {
    use RevisionState::*;
    // (what, the row, old file, new file)
    let cases = [
        (
            "a check or a removal failed",
            replacement(Failed, Some("CRC32"), Some(NEW_NAME), None),
            "kept",
            Some((
                "Season 01/[SubsPlease] Show - 14v2 (1080p) [8F2EFECC].mkv",
                "received_name",
            )),
        ),
        (
            "the download stopped",
            replacement(Failed, Some("stopped"), None, None),
            "kept",
            None,
        ),
        (
            "received again after the old video was removed",
            replacement(Failed, Some("stopped"), None, Some(15)),
            "removed",
            None,
        ),
        (
            "the rename has not gone through",
            replacement(Removed, Some("busy"), Some(NEW_NAME), Some(15)),
            "removed",
            Some((
                "Season 01/[SubsPlease] Show - 14v2 (1080p) [8F2EFECC].mkv",
                "received_name",
            )),
        ),
        (
            "the replacement ended with no video left",
            replacement(Abandoned, Some("no video"), Some(NEW_NAME), Some(15)),
            "removed",
            Some((
                "Season 01/[SubsPlease] Show - 14v2 (1080p) [8F2EFECC].mkv",
                "missing",
            )),
        ),
        (
            "the old file is waited for",
            replacement(Removing, Some("waits"), Some(NEW_NAME), Some(15)),
            "kept",
            Some((
                "Season 01/[SubsPlease] Show - 14v2 (1080p) [8F2EFECC].mkv",
                "received_name",
            )),
        ),
        (
            "the new file was missing on one look",
            replacement(Verified, Some("missing once"), Some(NEW_NAME), None),
            "kept",
            Some((
                "Season 01/[SubsPlease] Show - 14v2 (1080p) [8F2EFECC].mkv",
                "received_name",
            )),
        ),
    ];
    for (what, row, old, new) in cases {
        let shown = failure_of(&row, Some(std::path::Path::new("/media/Show")));
        let new = match new {
            Some((path, state)) => json!({ "role": "new", "path": path, "state": state }),
            None => json!({ "role": "new", "path": null, "state": "not_received" }),
        };
        assert_eq!(
            serde_json::to_value(&shown.files).unwrap(),
            json!([
                { "role": "old", "path": "Season 01/Show S01E14.mkv", "state": old },
                new,
            ]),
            "{what}"
        );
        assert_eq!(shown.reason, row.reason.clone().unwrap(), "{what}");
    }
}

/// A path is shown under the work's folder when it is inside it, and whole
/// otherwise.
#[test]
fn a_failed_replacements_paths_are_shown_under_the_works_folder_when_inside_it() {
    let row = replacement(RevisionState::Failed, Some("x"), Some(NEW_NAME), None);
    let old_path = |base: Option<&str>| {
        let shown = failure_of(&row, base.map(std::path::Path::new));
        serde_json::to_value(&shown.files).unwrap()[0]["path"].clone()
    };
    assert_eq!(old_path(Some("/media/Show")), "Season 01/Show S01E14.mkv");
    assert_eq!(
        old_path(Some("/elsewhere")),
        "/media/Show/Season 01/Show S01E14.mkv"
    );
    assert_eq!(old_path(None), "/media/Show/Season 01/Show S01E14.mkv");
}

/// A failed replacement is listed with the receive failures with its work, its
/// episode, why, and both files (`GET /api/todo/receive-failures`).
#[tokio::test]
async fn a_failed_replacement_is_in_the_receive_failures_with_its_work_and_both_files() {
    let app = App::new().await;
    app.state
        .history
        .record(
            1,
            vec![Observation {
                channel_id: app.rule.channel_id.clone(),
                channel_label: "https://feed.test/rss".into(),
                identity_key: "guid:14v2".into(),
                title: NEW_NAME.into(),
                link: "magnet:?xt=urn:btih:2222000000000000000000000000000000000014".into(),
                result: HistoryResult::Received,
                rule_id: Some(app.rule.id.clone()),
                torrent_hash: Some("2222000000000000000000000000000000000014".into()),
                reason: None,
            }],
        )
        .await
        .unwrap();
    let item = app
        .state
        .history
        .item_by_key(app.rule.channel_id.clone(), "guid:14v2".into())
        .await
        .unwrap()
        .unwrap();
    let row = app
        .state
        .revisions
        .create(
            10,
            NewRevision {
                item_id: item.id,
                old_item_id: None,
                rule_id: app.rule.id.clone(),
                folder: SEASON_FOLDER.into(),
                episode_name: EPISODE_NAME.into(),
                old_version: Some(1),
                new_version: 2,
                old_crc: None,
                expected_crc: Some("8F2EFECC".into()),
                torrent_hash: None,
                state: RevisionState::Receiving,
                reason: None,
            },
        )
        .await
        .unwrap();
    app.state
        .revisions
        .advance(
            row.id,
            20,
            RevisionState::Receiving,
            Step::Failed {
                reason: "받은 파일의 CRC32가 이름과 달라요.".into(),
                received_name: Some(NEW_NAME.into()),
            },
        )
        .await
        .unwrap();

    let list = app.get("/api/todo/receive-failures").await;
    let items = list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{list}");
    let entry = &items[0];
    assert_eq!(entry["kind"], "revision");
    assert_eq!(entry["history_item_id"], item.id);
    assert_eq!(entry["title"], NEW_NAME);
    assert_eq!(entry["work"], json!({ "id": "w1", "name": "Show" }));
    assert_eq!(
        (entry["season"].clone(), entry["episode"].clone()),
        (json!(1), json!("14"))
    );
    assert!(
        entry["reason"].as_str().unwrap().contains("CRC32"),
        "{entry}"
    );
    // The files are told by `failure_of`, which the tests above check by its
    // rows; here the entry carries the old file and the new one.
    let roles: Vec<_> = entry["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["old", "new"]);
}
