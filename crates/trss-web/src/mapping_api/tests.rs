//! The user's episode mapping and the `회차 확인 필요` to-do as the screens see
//! them (`trss_jobs::mapping` and `trss_jobs::follow` have the decisions).

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::AppState;
use trss_collect::store::channels::{ChannelInput, NewSubscription, Rule, RuleInput, SubtitleMode};
use trss_core::{Db, DbError};

const ANIME: i64 = 3424;
/// Episode 1 of the season airs here (Unix seconds); one episode a week.
const AIRED: i64 = 1_790_000_000;
const WEEK: i64 = 7 * 24 * 3600;
const MAPPING: &str = "/api/library/works/w1/seasons/1/anissia/sources/s1/mapping";
const CANDIDATES: &str = "/api/library/works/w1/seasons/1/anissia/candidates";

struct App {
    state: AppState,
    router: Router,
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
    /// Work `w1` season 1 of 12 episodes aired weekly, linked to anime
    /// [`ANIME`] by a collecting subscription that gets subtitles with no
    /// creator chosen yet.
    async fn new() -> App {
        let state = AppState::new(Db::open_blocking(":memory:").unwrap());
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
        let router = Router::new().nest("/api", crate::api::router().with_state(state.clone()));
        App {
            state,
            router,
            rule,
        }
    }

    /// Lines of the creator `에루샤` (source `s1`), each written an hour after
    /// its episode aired when the episode is a number.
    async fn observe(&self, episodes: &[&str]) {
        let mut batch = String::new();
        for episode in episodes {
            let at = episode.parse::<i64>().map_or("NULL".to_owned(), |n| {
                ((AIRED + (n - 1) * WEEK + 3600) * 1000).to_string()
            });
            batch.push_str(&format!(
                "INSERT OR IGNORE INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('s1', {ANIME}, '에루샤', 1);
                 INSERT INTO caption_observations (source_id, post_url, episode, updated,
                     updated_at, first_seen_at)
                     VALUES ('s1', 'https://fake.trss.invalid/ok/{episode}', '{episode}',
                             'x', {at}, 7);"
            ));
        }
        sql(&self.state, batch).await;
    }

    /// `에루샤` becomes the subscribed creator; the follower looks at once.
    async fn follow(&self) {
        let (status, rule) = self
            .call(
                Method::PUT,
                &format!("/api/rules/{}/creator", self.rule.id),
                Some(json!({ "version": self.rule.version, "creator": "에루샤" })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{rule}");
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

    /// The candidates answer's mapping of `s1`.
    async fn mapping(&self) -> Value {
        self.get(CANDIDATES).await["mappings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["source_id"] == "s1")
            .cloned()
            .unwrap_or(Value::Null)
    }

    async fn save(&self, version: i64, offset: i64, exceptions: Value) -> (StatusCode, Value) {
        self.call(
            Method::PUT,
            MAPPING,
            Some(json!({ "version": version, "offset": offset, "exceptions": exceptions })),
        )
        .await
    }

    /// The episodes of the jobs the follower made, sorted.
    async fn received(&self) -> Vec<String> {
        let groups = self.get("/api/subtitle-jobs").await;
        let mut episodes = Vec::new();
        for group in ["failed", "waiting", "running"] {
            for job in groups[group].as_array().unwrap() {
                episodes.push(job["episodes"][0].as_str().unwrap().to_owned());
            }
        }
        for job in groups["done"]["items"].as_array().unwrap() {
            episodes.push(job["episodes"][0].as_str().unwrap().to_owned());
        }
        episodes.sort();
        episodes
    }
}

#[tokio::test]
async fn an_undecided_subscribed_creator_is_a_to_do_that_goes_when_the_user_maps_it() {
    let app = App::new().await;
    // The creator numbers on from an earlier season of 12: 13 is the first episode.
    // Its time (a week past the last of the 12) says nothing, so the app cannot decide.
    app.observe(&["13"]).await;
    app.follow().await;
    let mapping = app.mapping().await;
    assert_eq!(mapping["kind"], "undecided");
    assert_eq!(mapping["exceptions"], json!([]));
    assert_eq!(app.get(CANDIDATES).await["season_episodes"], 12);
    assert!(app.received().await.is_empty());

    // The user continues on from the earlier season: 13 is the first.
    let (status, saved) = app
        .save(mapping["version"].as_i64().unwrap(), -12, json!([]))
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["kind"], "user");
    assert_eq!(saved["offset"], -12);
    assert!(saved["version"].as_i64().unwrap() > mapping["version"].as_i64().unwrap());
    // The follower looked at once: the episode is received as the season's first.
    assert_eq!(app.received().await, ["13"]);
    let shown = app.mapping().await;
    assert_eq!(
        (shown["kind"].clone(), shown["offset"].clone()),
        (json!("user"), json!(-12))
    );
}

#[tokio::test]
async fn a_saved_exception_is_echoed_as_sent_and_shown_with_the_candidates() {
    let app = App::new().await;
    // 1 and 2 on time, 13.5 fits no mapping.
    app.observe(&["1", "2", "13.5"]).await;
    app.follow().await;

    // `13.50` is `13.5`: the exception covers it, and the other episodes keep the default.
    let version = app.mapping().await["version"].as_i64().unwrap();
    let (status, saved) = app
        .save(version, 0, json!([{ "episode": "13.50", "target": null }]))
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(
        saved["exceptions"],
        json!([{ "episode": "13.50", "target": null }])
    );
    // The candidates answer says what a screen needs for the dialog.
    let shown = app.get(CANDIDATES).await;
    assert_eq!(shown["previous_episodes"], 0);
    assert_eq!(shown["mappings"][0], saved);
}

#[tokio::test]
async fn a_save_from_an_older_version_is_409_with_the_current_mapping_and_changes_nothing() {
    let app = App::new().await;
    app.observe(&["1", "2"]).await;
    app.follow().await;
    let read = app.mapping().await["version"].as_i64().unwrap();

    // Two screens read the same version; the first saves.
    let (status, first) = app.save(read, 2, json!([])).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let (status, late) = app
        .save(read, 5, json!([{ "episode": "1", "target": 3 }]))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{late}");
    assert_eq!(late["error"], "conflict");
    assert_eq!(late["current"]["source_id"], "s1");
    assert_eq!(late["current"]["mapping"], first);
    assert_eq!(app.mapping().await, first);
    // A revert from the older version is refused the same way.
    let (status, late) = app
        .call(
            Method::POST,
            &format!("{MAPPING}/revert"),
            Some(json!({ "version": read })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{late}");
    assert_eq!(late["current"]["mapping"], first);
    assert_eq!(app.mapping().await["kind"], "user");
}

#[tokio::test]
async fn two_exceptions_of_one_number_are_refused_with_a_sentence() {
    let app = App::new().await;
    app.observe(&["1", "2"]).await;
    app.follow().await;
    let version = app.mapping().await["version"].as_i64().unwrap();
    let (status, body) = app
        .save(
            version,
            0,
            json!([{ "episode": "013", "target": 1 }, { "episode": "13", "target": 2 }]),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid");
    assert!(body["message"].as_str().unwrap().contains("같은 회차"));
    // Nothing was saved.
    assert_eq!(app.mapping().await["kind"], "auto");

    // A body that lacks a field is no request.
    let (status, _) = app
        .call(Method::PUT, MAPPING, Some(json!({ "version": version })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_source_that_is_not_a_creator_of_the_seasons_anime_is_not_found() {
    let app = App::new().await;
    app.observe(&["1"]).await;
    sql(
        &app.state,
        "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
         VALUES ('other', 999, '다른', 1);"
            .to_owned(),
    )
    .await;
    for source in ["nope", "other"] {
        let (status, body) = app
            .call(
                Method::PUT,
                &format!("/api/library/works/w1/seasons/1/anissia/sources/{source}/mapping"),
                Some(json!({ "version": 0, "offset": 0, "exceptions": [] })),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{source}: {body}");
    }
    let (status, _) = app
        .call(
            Method::PUT,
            "/api/library/works/nope/seasons/1/anissia/sources/s1/mapping",
            Some(json!({ "version": 0, "offset": 0, "exceptions": [] })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // A creator that is not the subscribed one can be mapped too.
    let (status, body) = app.save(0, 0, json!([])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["kind"], "user");
}

#[tokio::test]
async fn reverting_gives_the_source_back_to_the_app_which_decides_again() {
    let app = App::new().await;
    app.observe(&["1", "2"]).await;
    app.follow().await;
    let auto = app.mapping().await;
    assert_eq!(auto["kind"], "auto");

    // The app's own mapping is not the user's to take back.
    let (status, body) = app
        .call(
            Method::POST,
            &format!("{MAPPING}/revert"),
            Some(json!({ "version": auto["version"] })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (_, saved) = app
        .save(
            auto["version"].as_i64().unwrap(),
            4,
            json!([{ "episode": "9", "target": 1 }]),
        )
        .await;
    assert_eq!(saved["offset"], 4);
    let (status, reverted) = app
        .call(
            Method::POST,
            &format!("{MAPPING}/revert"),
            Some(json!({ "version": saved["version"] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{reverted}");
    assert_eq!(reverted["source_id"], "s1");
    // The answer is the mapping the app decided again, a new version of it.
    let mapping = &reverted["mapping"];
    assert_ne!(mapping["version"], saved["version"]);
    assert_eq!(app.mapping().await, *mapping);

    // A screen that still holds the user's version cannot save over the app's.
    let (status, late) = app
        .save(saved["version"].as_i64().unwrap(), 7, json!([]))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{late}");
    assert_eq!(late["current"]["mapping"], *mapping);
}

#[tokio::test]
async fn a_creator_the_app_does_not_decide_goes_back_to_no_mapping_at_all() {
    let app = App::new().await;
    // Nobody is subscribed to this creator: the app decides nothing for it.
    app.observe(&["1"]).await;
    let (status, saved) = app.save(0, 1, json!([])).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (status, reverted) = app
        .call(
            Method::POST,
            &format!("{MAPPING}/revert"),
            Some(json!({ "version": saved["version"] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{reverted}");
    assert_eq!(reverted, json!({ "source_id": "s1", "mapping": null }));
    assert_eq!(app.mapping().await, Value::Null);
}

#[tokio::test]
async fn an_exception_decides_which_candidate_revises_a_subtitle_file_the_user_named() {
    let app = App::new().await;
    app.observe(&["5", "14"]).await;
    // The season's 5 and 12 have a subtitle file the user gave to 에루샤.
    sql(
        &app.state,
        "INSERT INTO episodes (work_id, season, episode) VALUES ('w1', 1, '05'), ('w1', 1, '12');
         INSERT INTO media_files (work_id, path, season, episode, kind, creator_source_id)
             VALUES ('w1', 'S01E05.ass', 1, '05', 'subtitle', 's1'),
                    ('w1', 'S01E12.ass', 1, '12', 'subtitle', 's1');"
            .to_owned(),
    )
    .await;
    let revision = |list: &Value, episode: &str| {
        list["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["episode"] == episode)
            .unwrap()["revision"]
            .clone()
    };
    let marked = json!({ "of": null, "same_post": null });

    // No mapping: compared by number as they are.
    let list = app.get(CANDIDATES).await;
    assert_eq!(revision(&list, "5"), marked);
    assert_eq!(revision(&list, "14"), Value::Null);

    // The default 0 changes nothing; the exceptions come first: 14 is the
    // season's 12, and 5 is not received, so it revises nothing.
    let (status, _) = app
        .save(
            0,
            0,
            json!([{ "episode": "014", "target": 12 }, { "episode": "5", "target": null }]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let list = app.get(CANDIDATES).await;
    assert_eq!(revision(&list, "5"), Value::Null);
    assert_eq!(revision(&list, "14"), marked);
}

#[tokio::test]
async fn reverting_a_source_with_no_mapping_is_refused_as_invalid_and_a_stale_version_as_a_conflict(
) {
    let app = App::new().await;
    app.observe(&["1"]).await;
    // No mapping row yet: version 0 is the stored one, and there is nothing of the user's to give back.
    let (status, body) = app
        .call(
            Method::POST,
            &format!("{MAPPING}/revert"),
            Some(json!({ "version": 0 })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    app.follow().await;
    let stored = app.mapping().await;
    // A version that is not the stored one is the conflict, with the stored mapping.
    let (status, body) = app
        .call(
            Method::POST,
            &format!("{MAPPING}/revert"),
            Some(json!({ "version": 0 })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["current"]["mapping"]["version"], stored["version"]);
}
