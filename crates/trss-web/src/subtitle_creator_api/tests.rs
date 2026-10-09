use std::{
    collections::BTreeSet,
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

use crate::{testing, AppState};
use trss_anissia::{fake::Fake, Anissia};
use trss_collect::anissia::captions::CaptionObserver;
use trss_core::{Clock, Db, DbError};
use trss_library::discovery::{EpisodeFile, FileKind, Scan, ScannedWork, WorkRead};

const NOW: i64 = 1_790_780_400_000;
const ANIME: i64 = 3320;
/// The version a subtitle file starts at: the time of the scan that found it (the folder is added at 1).
const FIRST: i64 = 1;

struct App {
    db: Db,
    state: AppState,
    router: Router,
    fake: Fake,
    now: Arc<AtomicI64>,
    /// The work `Sayonara Lara`: season 1 has a video and a subtitle for each
    /// of the episodes 1 to 12, season 2 has none.
    work: String,
}

fn episode_file(kind: FileKind, season: u32, episode: u32) -> EpisodeFile {
    let ext = if kind == FileKind::Video {
        "mkv"
    } else {
        "ass"
    };
    EpisodeFile {
        path: format!("Season {season:02}/S{season:02}E{episode:02}.{ext}"),
        kind,
        season,
        episode: format!("{episode:02}"),
    }
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
        let mut files = Vec::new();
        for episode in 1..=12 {
            files.push(episode_file(FileKind::Video, 1, episode));
            files.push(episode_file(FileKind::Subtitle, 1, episode));
        }
        let scan = Scan {
            works: vec![WorkRead::Read(ScannedWork {
                dir_name: "Sayonara Lara".into(),
                seasons: BTreeSet::from([1, 2]),
                files,
                unrecognized: Vec::new(),
            })],
        };
        let (folder, _) = state
            .library
            .add_folder("/c".into(), scan, 1, &[])
            .await
            .unwrap();
        let work = state.library.works(&folder.id).await.unwrap().remove(0).id;
        let router = testing::api(&state);
        let app = App {
            db,
            state,
            router,
            fake,
            now,
            work,
        };
        // Anissia lists two creators of the anime, 하느 and 카이란; none of
        // their lines is observed yet.
        app.fake.set_week(
            3,
            vec![app
                .fake
                .entry(3, ANIME, "22:30", "이번 분기 작품", "This Quarter")],
        );
        app.fake.set_captions(
            ANIME,
            vec![
                app.fake.caption("12", "2026-10-02T11:00:00", "하느"),
                app.fake.caption("12", "2026-10-02T11:00:00", "카이란"),
            ],
        );
        app
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        testing::call(&self.router, method, uri, body).await
    }

    fn season_path(&self, season: u32, tail: &str) -> String {
        format!("/api/library/works/{}/seasons/{season}{tail}", self.work)
    }

    /// Links season 1 to the anime.
    async fn link(&self) {
        let (status, body) = self
            .call(
                Method::POST,
                &self.season_path(1, "/anissia/link"),
                Some(json!({ "version": 0, "anime_no": ANIME, "week": 3 })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    async fn name_all(&self, season: u32, creator: &str) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            &self.season_path(season, "/subtitle-creators"),
            Some(json!({ "creator": creator })),
        )
        .await
    }

    async fn name_file(
        &self,
        path: &str,
        version: i64,
        creator: Option<&str>,
    ) -> (StatusCode, Value) {
        self.call(
            Method::PUT,
            &self.season_path(1, "/subtitle-creators/file"),
            Some(json!({ "path": path, "version": version, "creator": creator })),
        )
        .await
    }

    async fn detail(&self) -> Value {
        let (status, body) = self
            .call(
                Method::GET,
                &format!("/api/library/works/{}", self.work),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// The subtitle files of season 1 with the creator's name and version.
    async fn subtitles(&self) -> Vec<(String, Option<String>, i64)> {
        let detail = self.detail().await;
        let season = detail["seasons"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["number"] == 1)
            .unwrap();
        let mut out = Vec::new();
        for episode in season["episodes"].as_array().unwrap() {
            assert!(
                episode["video"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|v| v.get("creator").is_none()),
                "a video has no creator"
            );
            for file in episode["subtitle"].as_array().unwrap() {
                out.push((
                    file["path"].as_str().unwrap().to_owned(),
                    file["creator"]["name"].as_str().map(str::to_owned),
                    file["creator_version"].as_i64().unwrap(),
                ));
            }
        }
        out
    }

    fn observer(&self) -> CaptionObserver {
        CaptionObserver::new(self.state.anissia.clone(), self.state.anissia_store.clone())
    }

    /// The creators' lines of episode 5 appear in the recent list and are
    /// observed.
    async fn observe_episode_5(&self) {
        self.fake.set_recent(vec![
            self.fake.recent_line(
                ANIME,
                "5",
                "2026-10-02T11:40:00",
                "https://blog.test/hanu-5",
                "하느",
            ),
            self.fake.recent_line(
                ANIME,
                "5",
                "2026-10-02T11:41:00",
                "https://blog.test/kairan-5",
                "카이란",
            ),
        ]);
        let observer = self.observer();
        observer.run_due().await.unwrap();
        // Half an hour later 하느 moves on to episode 13 (a line holds one
        // episode), which the season has no file for.
        self.now.fetch_add(30 * 60_000, Ordering::SeqCst);
        self.fake.set_recent(vec![self.fake.recent_line(
            ANIME,
            "13",
            "2026-10-02T11:42:00",
            "https://blog.test/hanu-13",
            "하느",
        )]);
        observer.run_due().await.unwrap();
    }

    async fn candidates(&self) -> Vec<Value> {
        let (status, body) = self
            .call(
                Method::GET,
                &self.season_path(1, "/anissia/candidates"),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["candidates"].as_array().unwrap().clone()
    }

    async fn count(&self, table: &'static str) -> i64 {
        self.db
            .run::<_, DbError, _>(move |c| {
                Ok(c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?)
            })
            .await
            .unwrap()
    }
}

fn revision_of(candidates: &[Value], creator: &str, episode: &str) -> Value {
    candidates
        .iter()
        .find(|c| c["creator"] == creator && c["episode"] == episode)
        .unwrap_or_else(|| panic!("no candidate {creator} {episode}"))["revision"]
        .clone()
}

#[tokio::test]
async fn naming_a_creator_marks_all_twelve_unknown_files_and_makes_its_source_once() {
    let app = App::new().await;
    app.link().await;

    // 하느 is listed by Anissia but none of its lines was observed yet: the
    // source is made for the naming.
    assert_eq!(app.count("subtitle_sources").await, 0);
    let (status, named) = app.name_all(1, "하느").await;
    assert_eq!(status, StatusCode::OK, "{named}");
    assert_eq!(named["attached"], 12);
    assert_eq!(named["season"], 1);
    assert_eq!(named["creator"]["name"], "하느");
    assert_eq!(named["creator"]["anime_no"], ANIME);
    assert_eq!(app.count("subtitle_sources").await, 1);

    // Naming again finds nothing unknown, and the same creator's source stays.
    let (status, again) = app.name_all(1, "하느").await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["attached"], 0);
    assert_eq!(app.count("subtitle_sources").await, 1);
}

#[tokio::test]
async fn the_creators_later_candidate_of_an_episode_with_such_a_file_is_a_revision_candidate() {
    let app = App::new().await;
    app.link().await;
    app.name_all(1, "하느").await;

    // 하느 and 카이란 both post episode 5, and 하느 episode 13 (which the
    // season has no file for).
    app.observe_episode_5().await;
    let candidates = app.candidates().await;
    assert_eq!(candidates.len(), 3);
    assert_eq!(
        revision_of(&candidates, "하느", "5"),
        json!({ "of": null, "same_post": null })
    );

    // Naming 카이란 for episode 5 only marks 카이란's candidate too.
    let (status, _) = app
        .name_file("Season 01/S01E05.ass", FIRST + 1, Some("카이란"))
        .await;
    assert_eq!(status, StatusCode::OK);
    let candidates = app.candidates().await;
    assert_eq!(
        revision_of(&candidates, "카이란", "5"),
        json!({ "of": null, "same_post": null })
    );
    // 하느's episode 5 has no file of 하느's any more.
    assert_eq!(revision_of(&candidates, "하느", "5"), Value::Null);
}

#[tokio::test]
async fn the_sources_mapping_decides_which_episode_of_the_season_a_candidate_is() {
    let app = App::new().await;
    app.link().await;
    app.name_all(1, "하느").await;
    app.observe_episode_5().await;
    let source = app.candidates().await[0]["source_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let work = app.work.clone();
    let map = |kind: &'static str, offset: Option<i64>| {
        let (work, source) = (work.clone(), source.clone());
        let db = app.db.clone();
        async move {
            db.run::<_, DbError, _>(move |c| {
                c.execute(
                    "INSERT OR REPLACE INTO subtitle_episode_mappings
                         (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
                     VALUES (?1, 1, ?2, ?3, ?4, '근거', 1)",
                    rusqlite::params![work, source, kind, offset],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        }
    };

    // 하느's cumulative 5 is the season's episode 1 under -4, and the season's
    // 5 is then the creator's 9: the candidate of 5 revises the file for 1.
    map("auto", Some(-4)).await;
    let candidates = app.candidates().await;
    assert_eq!(
        revision_of(&candidates, "하느", "5"),
        json!({ "of": null, "same_post": null })
    );
    // 13 is the season's 9, which has a file too.
    assert_eq!(
        revision_of(&candidates, "하느", "13"),
        json!({ "of": null, "same_post": null })
    );
}

#[tokio::test]
async fn one_files_creator_changes_and_goes_back_to_unknown() {
    let app = App::new().await;
    app.link().await;
    app.name_all(1, "하느").await;

    let path = "Season 01/S01E07.ass";
    let (status, changed) = app.name_file(path, FIRST + 1, Some("카이란")).await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert_eq!(changed["path"], path);
    assert_eq!(changed["version"], FIRST + 2);
    assert_eq!(changed["creator"]["name"], "카이란");

    // The work's detail shows the new creator and version of the file.
    assert!(app.subtitles().await.contains(&(
        path.to_owned(),
        Some("카이란".to_owned()),
        FIRST + 2
    )));

    // Back to unknown.
    let (status, unknown) = app.name_file(path, FIRST + 2, None).await;
    assert_eq!(status, StatusCode::OK, "{unknown}");
    assert_eq!(unknown["creator"], Value::Null);
    assert_eq!(unknown["version"], FIRST + 3);
    assert!(app
        .subtitles()
        .await
        .contains(&(path.to_owned(), None, FIRST + 3)));
}

#[tokio::test]
async fn a_season_without_an_anissia_link_cannot_name_a_creator() {
    let app = App::new().await;
    let (status, refused) = app.name_all(1, "하느").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(refused["message"].as_str().unwrap().contains("연결"));
    let (status, refused) = app.name_file("Season 01/S01E01.ass", 0, Some("하느")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(app.subtitles().await.iter().all(|(_, c, _)| c.is_none()));
    // Another season of the work that has no link is the same; one the work
    // does not have is not found.
    let (status, _) = app.name_all(2, "하느").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = app.name_all(9, "하느").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_creator_the_anime_does_not_have_is_refused() {
    let app = App::new().await;
    app.link().await;
    let (status, refused) = app.name_all(1, "다른 작품의 제작자").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (status, refused) = app.name_all(1, "  ").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (status, _) = app
        .name_file("Season 01/S01E01.ass", 0, Some("다른 작품의 제작자"))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Unknown fields and a missing creator are not a request.
    let (status, _) = app
        .call(
            Method::POST,
            &app.season_path(1, "/subtitle-creators"),
            Some(json!({ "creator": "하느", "all": true })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(app.subtitles().await.iter().all(|(_, c, _)| c.is_none()));
    assert_eq!(app.count("subtitle_sources").await, 0);
}

#[tokio::test]
async fn naming_a_creator_makes_no_automatic_receipt_for_a_work_with_no_subscription() {
    let app = App::new().await;
    app.link().await;
    app.observe_episode_5().await;
    let jobs = app.count("subtitle_jobs").await;
    let commands = app.count("commands").await;

    app.name_all(1, "하느").await;
    app.name_file("Season 01/S01E01.ass", 1, Some("카이란"))
        .await;

    assert_eq!(app.count("subtitle_jobs").await, jobs);
    assert_eq!(app.count("commands").await, commands);
    // The worker's look at the subscriptions finds nothing to receive either.
    assert!(app.state.follow.evaluate(NOW).await.unwrap().is_empty());
    assert!(app
        .state
        .jobs
        .picks_of_anime(ANIME)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(app.count("subtitle_jobs").await, jobs);
}

#[tokio::test]
async fn the_later_of_two_screens_changing_the_same_file_is_refused_with_the_file_as_it_is() {
    let app = App::new().await;
    app.link().await;
    app.name_all(1, "하느").await;
    let path = "Season 01/S01E03.ass";

    // Both screens read the file at version FIRST + 1. The first changes it.
    let (status, first) = app.name_file(path, FIRST + 1, Some("카이란")).await;
    assert_eq!(status, StatusCode::OK, "{first}");

    // The second still holds version FIRST + 1: refused, with the first one's choice.
    let (status, refused) = app.name_file(path, FIRST + 1, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"], "conflict");
    assert_eq!(refused["current"]["path"], path);
    assert_eq!(refused["current"]["version"], FIRST + 2);
    assert_eq!(refused["current"]["creator"]["name"], "카이란");

    // A file the work does not have, and a video, are not found.
    for missing in ["Season 01/nope.ass", "Season 01/S01E03.mkv"] {
        let (status, _) = app.name_file(missing, 0, Some("하느")).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    }
}
