//! A job's replacements as its detail and the to-dos show them, and the
//! person's decision (`trss_jobs::place::replace` has the plans themselves).

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
use trss_core::{Db, DbError};
use trss_jobs::{area::ReceiveArea, Created, NewItem, NewJob, Runner};
use trss_subtitles::{
    fake::{self, FakeSource},
    Sources,
};

const VIDEO: &str = "Season 01/Show S01E02.mkv";
const TARGET: &str = "Season 01/Show S01E02.ass";

struct App {
    state: AppState,
    router: Router,
    dir: tempfile::TempDir,
    runner: Runner,
}

impl App {
    /// Work `w1` season 1 in a real folder, whose episode 2 has a video.
    async fn new() -> App {
        let state = AppState::new(Db::open_blocking(":memory:").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let shows = dir.path().join("shows");
        std::fs::create_dir_all(shows.join("Show/Season 01")).unwrap();
        std::fs::write(shows.join("Show").join(VIDEO), b"video").unwrap();
        let path = shows.to_string_lossy().into_owned();
        state
            .jobs
            .db()
            .run::<_, DbError, _>(move |c| {
                c.execute(
                    "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', ?1, 1)",
                    [path],
                )?;
                c.execute_batch(&format!(
                    "INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
                     INSERT INTO seasons (work_id, number) VALUES ('w1', 1);
                     INSERT INTO episodes (work_id, season, episode) VALUES ('w1', 1, '02');
                     INSERT INTO media_files (work_id, path, season, episode, kind)
                         VALUES ('w1', '{VIDEO}', 1, '02', 'video');"
                ))?;
                Ok(())
            })
            .await
            .unwrap();
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
            dir,
            runner,
        }
    }

    fn at(&self, path: &str) -> std::path::PathBuf {
        self.dir.path().join("shows/Show").join(path)
    }

    /// A pick's job of `creator` for the fake post `path`, run.
    async fn job(&self, command: &str, creator: &str, path: &str) -> String {
        let made = self
            .state
            .jobs
            .create(
                NewJob {
                    command_id: command.to_owned(),
                    request: "{}".to_owned(),
                    origin: "pick".to_owned(),
                    work_id: Some("w1".to_owned()),
                    season: Some(1),
                    anime_no: None,
                    source_id: None,
                    creator: Some(creator.to_owned()),
                    revision_of: None,
                    revises_attributed: false,
                    items: vec![NewItem {
                        observation_id: None,
                        episode: "2".to_owned(),
                        post_url: format!("https://{}{path}", fake::HOST),
                        found_at: 1,
                    }],
                },
                100,
            )
            .await
            .unwrap();
        let Created::Created(id) = made else { panic!() };
        self.run().await;
        id
    }

    async fn run(&self) {
        self.runner
            .run_ready(&CancellationToken::new())
            .await
            .unwrap();
    }

    async fn sql(&self, sql: String) {
        self.state
            .jobs
            .db()
            .run::<_, DbError, _>(move |c| Ok(c.execute_batch(&sql)?))
            .await
            .unwrap();
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

    async fn detail(&self, job: &str) -> Value {
        let (status, body) = self
            .call(Method::GET, &format!("/api/subtitle-jobs/{job}"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// The job's one replacement.
    async fn replacement(&self, job: &str) -> Value {
        let detail = self.detail(job).await;
        let all = detail["replacements"].as_array().unwrap();
        assert_eq!(all.len(), 1, "{all:?}");
        all[0].clone()
    }

    async fn decide(
        &self,
        job: &str,
        plan: &Value,
        version: Value,
        decision: &str,
    ) -> (StatusCode, Value) {
        let plan = plan["plan_id"].as_str().unwrap();
        self.call(
            Method::POST,
            &format!("/api/subtitle-jobs/{job}/replacements/{plan}"),
            Some(json!({ "version": version, "decision": decision })),
        )
        .await
    }
}

#[tokio::test]
async fn a_revision_of_the_same_post_shows_two_version_lines_and_nothing_else() {
    let app = App::new().await;
    let first = app.job("c1", "에루샤", "/ok/Show-02").await;
    let second = app.job("c2", "에루샤", "/ok/Show-02v2").await;
    // The same post's file changed: one page for both.
    app.sql(format!(
        "UPDATE subtitle_packages SET source_page = 'https://post.test/1'
          WHERE job_id IN ('{first}', '{second}')"
    ))
    .await;

    let detail = app.detail(&second).await;
    assert_eq!(
        (&detail["state"], &detail["wait"]),
        (&json!("waiting"), &json!("approval"))
    );
    let r = app.replacement(&second).await;
    assert_eq!(r["state"], "open");
    assert_eq!(r["version"], 1);
    assert_eq!(r["episode"], 2);
    assert_eq!(r["again"], Value::Null);
    assert_eq!(r["side_by_side"], false);
    assert_eq!(r["limits"], json!([]));
    let work = app
        .at("")
        .to_string_lossy()
        .trim_end_matches('/')
        .to_owned();
    assert_eq!(
        r["paths"],
        json!([{ "path": format!("{work}/{TARGET}"), "action": "replace", "managed": true,
                 "warning": null }])
    );
    let current = &r["current"];
    assert_eq!(current["managed"], true);
    assert_eq!(current["lines"], 24);
    assert_eq!(current["size"], fake::ass("Show-02").len());
    assert_eq!(current["creator"], "에루샤");
    assert_eq!(current["post"], "https://post.test/1");
    assert_eq!(current["changed_at"], Value::Null);
    assert!(current["received_at"].is_i64());
    assert_eq!(
        current["stored"],
        format!("{work}/.trss/subtitles/에루샤/Show-02.ass")
    );
    let new = &r["new"];
    assert_eq!(new["lines"], 24);
    assert_eq!(new["size"], fake::ass("Show-02v2").len());
    assert_eq!(new["format"], "ass");
    assert_eq!(
        new["path"],
        format!("{work}/.trss/subtitles/에루샤/Show-02v2.ass")
    );

    // The to-do of the work, counted in the badge, opens the job.
    let (_, todo) = app.call(Method::GET, "/api/todo", None).await;
    assert_eq!(todo["count"], 1);
    let card = &todo["needs"][0];
    assert_eq!(card["kind"], "replacement");
    assert_eq!(card["key"], "replacement:w1");
    assert_eq!(card["job_id"], second.as_str());
    assert_eq!(card["episodes"], json!([2]));
    assert_eq!(card["jobs"], 1);
    assert_eq!(card["creator"], "에루샤");
}

#[tokio::test]
async fn another_creators_subtitle_is_compared_side_by_side() {
    let app = App::new().await;
    app.job("c1", "에루샤", "/ok/Show-02").await;
    let second = app.job("c2", "코코렛", "/ok/Show-02v2").await;
    let r = app.replacement(&second).await;
    assert_eq!(r["side_by_side"], true);
    assert_eq!(r["current"]["creator"], "에루샤");
    assert_eq!(r["new"]["creator"], "코코렛");
    assert_eq!(r["limits"], json!([]));
}

#[tokio::test]
async fn another_creators_ass_beside_an_applied_srt_warns_of_its_removal() {
    let app = App::new().await;
    app.job("c1", "에루샤", "/pack/Show - 02.srt").await;
    let job = app.job("c2", "코코렛", "/ok/Show-02").await;
    let r = app.replacement(&job).await;
    let work = app
        .at("")
        .to_string_lossy()
        .trim_end_matches('/')
        .to_owned();
    assert_eq!(
        r["paths"],
        json!([
            { "path": format!("{work}/{TARGET}"), "action": "add", "managed": false,
              "warning": null },
            { "path": format!("{work}/Season 01/Show S01E02.srt"), "action": "remove",
              "managed": true, "warning": "remove_applied" },
        ])
    );
    // The applied `.srt` is the current subtitle, compared side by side.
    assert_eq!(r["side_by_side"], true);
    assert_eq!(r["current"]["format"], "srt");
    assert_eq!(r["current"]["creator"], "에루샤");
    assert_eq!(r["new"]["format"], "ass");
}

#[tokio::test]
async fn overwriting_a_file_the_app_did_not_manage_is_a_warning_and_a_limit() {
    let app = App::new().await;
    std::fs::write(app.at(TARGET), b"mine").unwrap();
    let job = app.job("c1", "에루샤", "/ok/Show-02").await;
    let r = app.replacement(&job).await;
    assert_eq!(r["side_by_side"], true);
    assert_eq!(r["paths"][0]["action"], "replace");
    assert_eq!(r["paths"][0]["warning"], "overwrite_unmanaged");
    assert_eq!(r["limits"], json!(["unknown_source", "lines_unknown"]));
    let current = &r["current"];
    assert_eq!(current["managed"], false);
    assert_eq!(current["received_at"], Value::Null);
    assert!(current["changed_at"].is_i64());
    assert_eq!(current["creator"], Value::Null);
    assert_eq!(current["stored"], Value::Null);
}

#[tokio::test]
async fn a_decision_names_the_version_it_saw() {
    let app = App::new().await;
    app.job("c1", "에루샤", "/ok/Show-02").await;
    let job = app.job("c2", "에루샤", "/ok/Show-02v2").await;
    let r = app.replacement(&job).await;

    let (status, _) = app.decide(&job, &r, json!(2), "replace").await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = app.decide(&job, &r, json!(1), "maybe").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = app
        .call(
            Method::POST,
            &format!("/api/subtitle-jobs/{job}/replacements/nothing"),
            Some(json!({ "version": 1, "decision": "keep" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = app.decide(&job, &r, json!(1), "replace").await;
    assert_eq!(
        (status, body),
        (StatusCode::OK, json!({ "state": "approved" }))
    );
    // Used once.
    let (status, _) = app.decide(&job, &r, json!(1), "keep").await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (_, todo) = app.call(Method::GET, "/api/todo", None).await;
    assert_eq!(todo["count"], 0);

    app.run().await;
    assert_eq!(
        std::fs::read(app.at(TARGET)).unwrap(),
        fake::ass("Show-02v2")
    );
    let detail = app.detail(&job).await;
    assert_eq!(detail["state"], "done");
    assert_eq!(detail["replacements"][0]["state"], "done");
}

#[tokio::test]
async fn a_plan_compared_again_says_why() {
    let app = App::new().await;
    app.job("c1", "에루샤", "/ok/Show-02").await;
    let job = app.job("c2", "에루샤", "/ok/Show-02v2").await;
    let r = app.replacement(&job).await;
    app.decide(&job, &r, json!(1), "replace").await;
    // The video is replaced before the worker carries it out.
    let new = app.at("Season 01/new.mkv");
    std::fs::write(&new, b"video 2").unwrap();
    std::fs::rename(&new, app.at(VIDEO)).unwrap();
    app.run().await;

    let next = app.replacement(&job).await;
    assert_eq!(
        (&next["state"], &next["version"]),
        (&json!("open"), &json!(2))
    );
    assert_eq!(
        next["again"],
        json!({ "reason": "영상이 바뀌었어요", "new_revision": false })
    );
    // The approval of the first version is not used again.
    let (status, _) = app.decide(&job, &r, json!(1), "replace").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(std::fs::read(app.at(TARGET)).unwrap(), fake::ass("Show-02"));
}

impl App {
    /// The first job applies a subtitle, the second's plan to replace it
    /// waits: the second job and its plan's ID.
    async fn waiting_revision(&self) -> (String, String) {
        self.job("c1", "에루샤", "/ok/Show-02").await;
        self.revision_of("c2", "/ok/Show-02v2").await
    }

    /// Another job's plan against the file beside the video.
    async fn revision_of(&self, command: &str, post: &str) -> (String, String) {
        let job = self.job(command, "에루샤", post).await;
        let plan = self.replacement(&job).await["plan_id"]
            .as_str()
            .unwrap()
            .to_owned();
        (job, plan)
    }

    fn lines_uri(job: &str, plan: &str) -> String {
        format!("/api/subtitle-jobs/{job}/replacements/{plan}/lines")
    }

    /// The plan's comparison as an earlier build left it (none) or as the
    /// worker could not make it (`why`).
    async fn recompared(&self, plan: &str, why: Option<&str>) {
        // The comparison does not change once made, so it is replaced whole.
        let insert = match why {
            Some(why) => format!(
                "INSERT INTO subtitle_replacement_diffs (plan_id, path, unreadable)
                 VALUES ('{plan}', '{TARGET}', '{why}');"
            ),
            None => String::new(),
        };
        self.sql(format!(
            "DELETE FROM subtitle_replacement_diffs WHERE plan_id = '{plan}'; {insert}"
        ))
        .await;
    }
}

#[tokio::test]
async fn the_detail_says_what_changed_without_the_lines() {
    let app = App::new().await;
    let (job, _) = app.waiting_revision().await;

    let r = app.replacement(&job).await;

    let side = json!({ "format": "ASS", "encoding": "UTF-8", "cues": 24 });
    assert_eq!(
        r["comparison"],
        json!({
            "state": "compared",
            "current": side,
            "new": side,
            "dialogue": { "added": 0, "changed": 24, "removed": 0 },
            "timing": { "count": 0 },
            "styles": { "added": [], "removed": [], "changed": [] },
            "fonts": { "added": [], "removed": [] },
            "not_compared": []
        })
    );
}

#[tokio::test]
async fn a_plan_with_no_comparison_or_an_unreadable_one_says_so_and_never_no_difference() {
    let app = App::new().await;
    let (job, plan) = app.waiting_revision().await;

    app.recompared(&plan, None).await;
    assert_eq!(app.replacement(&job).await["comparison"], Value::Null);

    app.recompared(&plan, Some("현재 자막: 인코딩을 알 수 없어요"))
        .await;
    assert_eq!(
        app.replacement(&job).await["comparison"],
        json!({ "state": "unreadable", "reason": "현재 자막: 인코딩을 알 수 없어요" })
    );
}

#[tokio::test]
async fn the_lines_of_a_compared_plan_come_from_their_own_route() {
    let app = App::new().await;
    let (job, plan) = app.waiting_revision().await;

    let (status, lines) = app
        .call(Method::GET, &App::lines_uri(&job, &plan), None)
        .await;

    assert_eq!(status, StatusCode::OK, "{lines}");
    assert_eq!(lines["timing"], json!([]));
    let dialogue = lines["dialogue"].as_array().unwrap();
    assert_eq!(dialogue.len(), 24);
    assert_eq!(
        dialogue[0],
        json!({
            "kind": "changed",
            "old": { "text": "가짜 자막 Show-02 1", "start": 0, "end": 1500 },
            "new": { "text": "가짜 자막 Show-02v2 1", "start": 0, "end": 1500 }
        })
    );
}

#[tokio::test]
async fn the_lines_of_a_plan_that_is_not_the_jobs_or_was_not_compared_are_not_found() {
    let app = App::new().await;
    let (job, plan) = app.waiting_revision().await;
    let (status, _) = app
        .call(Method::GET, &App::lines_uri(&job, &plan), None)
        .await;
    assert_eq!(status, StatusCode::OK);

    // Another job's plan, and no plan.
    for (job, plan) in [("another", plan.as_str()), (job.as_str(), "no-plan")] {
        let (status, body) = app
            .call(Method::GET, &App::lines_uri(job, plan), None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{job} {plan}: {body}");
    }
    // A plan made before contents were compared, and one that could not be.
    for why in [
        None,
        Some("새 자막: 이미지 자막이라 내용을 비교할 수 없어요"),
    ] {
        app.recompared(&plan, why).await;
        let (status, body) = app
            .call(Method::GET, &App::lines_uri(&job, &plan), None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{why:?}: {body}");
    }
}

#[tokio::test]
async fn the_replacement_to_do_sums_what_its_open_plans_change() {
    let app = App::new().await;
    app.job("c1", "에루샤", "/ok/Show-02").await;
    // Two plans compared (24 dialogue lines changed each), one that could not
    // be, and one made before contents were compared.
    app.revision_of("c2", "/ok/Show-02v2").await;
    app.revision_of("c3", "/ok/Show-02v3").await;
    let (_, unreadable) = app.revision_of("c4", "/ok/Show-02v4").await;
    let (_, earlier) = app.revision_of("c5", "/ok/Show-02v5").await;
    app.recompared(&unreadable, Some("현재 자막: 인코딩을 알 수 없어요"))
        .await;
    app.recompared(&earlier, None).await;

    let (_, todo) = app.call(Method::GET, "/api/todo", None).await;

    let card = &todo["needs"][0];
    assert_eq!(card["kind"], "replacement");
    assert_eq!(card["episodes"], json!([2]));
    assert_eq!(card["jobs"], 4);
    assert_eq!(
        card["changes"],
        json!({
            "added": 0, "changed": 48, "removed": 0, "timing": 0, "styles": 0, "fonts": 0,
            "uncompared": 2, "partial": 0, "plans": 4
        })
    );
}

#[tokio::test]
async fn the_replacement_to_do_sums_timing_styles_and_fonts_too() {
    let app = App::new().await;
    // A file the app did not manage: the new copy's font and one line's start
    // differ.
    let current = String::from_utf8(fake::ass("Show-02v2"))
        .unwrap()
        .replace("Style: Default,Arial,", "Style: Default,Noto Sans CJK KR,")
        .replacen("Dialogue: 0,0:00:00.00,", "Dialogue: 0,0:00:00.50,", 1);
    std::fs::write(app.at(TARGET), current).unwrap();
    app.job("c1", "에루샤", "/ok/Show-02v2").await;

    let (_, todo) = app.call(Method::GET, "/api/todo", None).await;

    assert_eq!(
        todo["needs"][0]["changes"],
        json!({
            "added": 0, "changed": 0, "removed": 0, "timing": 1, "styles": 1, "fonts": 2,
            "uncompared": 0, "partial": 0, "plans": 1
        })
    );
}

#[tokio::test]
async fn the_replacement_to_do_counts_a_plan_compared_only_in_part() {
    let app = App::new().await;
    // An SRT beside the video with the new ASS's very dialogue: nothing
    // differs in what was compared, but the ASS's styles and fonts were not.
    let mut srt = String::new();
    for i in 0..24 {
        srt += &format!(
            "{}\n00:00:{:02},000 --> 00:00:{:02},500\n가짜 자막 Show-02v2 {}\n\n",
            i + 1,
            i * 2,
            i * 2 + 1,
            i + 1
        );
    }
    std::fs::write(app.at("Season 01/Show S01E02.srt"), srt).unwrap();
    app.job("c1", "에루샤", "/ok/Show-02v2").await;

    let (_, todo) = app.call(Method::GET, "/api/todo", None).await;

    assert_eq!(
        todo["needs"][0]["changes"],
        json!({
            "added": 0, "changed": 0, "removed": 0, "timing": 0, "styles": 0, "fonts": 0,
            "uncompared": 0, "partial": 1, "plans": 1
        })
    );
}
