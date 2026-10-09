//! A job's replacements as its detail shows them, and the person's decision
//! (`trss_jobs::place::replace` has the plans themselves, and the replacement
//! to-do card is tested in `trss-jobs` `tests/it/todo.rs`).

use std::sync::Arc;

use axum::{
    http::{Method, StatusCode},
    Router,
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::{testing, AppState};
use trss_core::{Db, DbError};
use trss_jobs::{area::ReceiveArea, Created, JobRun, NewItem, NewJob, Runner};
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
        for episode in 2..=4 {
            let video = format!("Season 01/Show S01E{episode:02}.mkv");
            std::fs::write(shows.join("Show").join(video), b"video").unwrap();
        }
        let path = shows.to_string_lossy().into_owned();
        state
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
                for episode in [3, 4] {
                    c.execute_batch(&format!(
                        "INSERT INTO episodes (work_id, season, episode) VALUES ('w1', 1, '{episode:02}');
                         INSERT INTO media_files (work_id, path, season, episode, kind)
                             VALUES ('w1', 'Season 01/Show S01E{episode:02}.mkv', 1, '{episode:02}', 'video');"
                    ))?;
                }
                Ok(())
            })
            .await
            .unwrap();
        let runner = Runner::new(
            JobRun::new(state.db().clone()),
            Sources::none().with_fake(FakeSource),
            ReceiveArea::in_app_data(dir.path()),
            Arc::new(|| 2_000),
        );
        let router = testing::api(&state);
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
        self.job_of(command, creator, &[("2", path)]).await
    }

    /// A pick's job of `creator` for the fake posts (episode, path), run.
    async fn job_of(&self, command: &str, creator: &str, posts: &[(&str, &str)]) -> String {
        let made = self
            .state
            .job_requests
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
                    items: posts
                        .iter()
                        .map(|(episode, path)| NewItem {
                            observation_id: None,
                            episode: (*episode).to_owned(),
                            post_url: format!("https://{}{path}", fake::HOST),
                            found_at: 1,
                        })
                        .collect(),
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
            .db()
            .run::<_, DbError, _>(move |c| Ok(c.execute_batch(&sql)?))
            .await
            .unwrap();
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        testing::call(&self.router, method, uri, body).await
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
    let paths: Vec<(&Value, &Value, &Value)> = r["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (&p["action"], &p["managed"], &p["warning"]))
        .collect();
    assert_eq!(
        paths,
        [
            (&json!("add"), &json!(false), &Value::Null),
            (&json!("remove"), &json!(true), &json!("remove_applied")),
        ]
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
async fn a_plan_says_what_it_changes_as_one_plans_total() {
    let app = App::new().await;
    let (job, _) = app.waiting_revision().await;

    assert_eq!(
        app.replacement(&job).await["changes"],
        json!({
            "added": 0, "changed": 24, "removed": 0, "timing": 0, "styles": 0,
            "fonts": 0, "uncompared": 0, "partial": 0, "plans": 1
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
    assert_eq!(
        lines["dialogue"][0],
        json!({
            "kind": "changed",
            "old": { "text": "가짜 자막 Show-02 1", "start": 0, "end": 1500 },
            "new": { "text": "가짜 자막 Show-02v2 1", "start": 0, "end": 1500 }
        })
    );
}

#[tokio::test]
async fn the_lines_of_a_plan_that_is_not_the_jobs_are_not_found() {
    let app = App::new().await;
    let (job, plan) = app.waiting_revision().await;
    let (status, _) = app
        .call(Method::GET, &App::lines_uri(&job, &plan), None)
        .await;
    assert_eq!(status, StatusCode::OK);

    // Another job's plan is not this job's to read.
    let (status, body) = app
        .call(Method::GET, &App::lines_uri("another", &plan), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

impl App {
    /// A first job applies episodes 2 to 4 (`/ok/Show-0N`) and a second
    /// job's plans for the revisions (`/ok/Show-0Nv2`) wait: the second job
    /// and its plans' IDs and versions, by episode.
    async fn three_waiting(&self) -> (String, Vec<(String, i64)>) {
        let first = [
            ("2", "/ok/Show-02"),
            ("3", "/ok/Show-03"),
            ("4", "/ok/Show-04"),
        ];
        self.job_of("c1", "에루샤", &first).await;
        let second = [
            ("2", "/ok/Show-02v2"),
            ("3", "/ok/Show-03v2"),
            ("4", "/ok/Show-04v2"),
        ];
        let job = self.job_of("c2", "에루샤", &second).await;
        let detail = self.detail(&job).await;
        let mut plans: Vec<(i64, String, i64)> = detail["replacements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["episode"].as_i64().unwrap(),
                    r["plan_id"].as_str().unwrap().to_owned(),
                    r["version"].as_i64().unwrap(),
                )
            })
            .collect();
        plans.sort();
        assert_eq!(plans.len(), 3, "{plans:?}");
        (job, plans.into_iter().map(|(_, id, v)| (id, v)).collect())
    }

    async fn decide_many(&self, job: &str, body: Value) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            &format!("/api/subtitle-jobs/{job}/replacements"),
            Some(body),
        )
        .await
    }
}

#[tokio::test]
async fn several_decisions_at_once_answer_each_plan_in_the_order_asked() {
    let app = App::new().await;
    let (job, plans) = app.three_waiting().await;
    let [(two, v2), (three, v3), (four, v4)] = &plans[..] else {
        panic!()
    };
    // Episode 2 is replaced, 3 is kept, and 4 names a version that is not
    // the plan to decide.
    let (status, body) = app
        .decide_many(
            &job,
            json!({ "decisions": [
                { "plan": four, "version": v4 + 1, "decision": "replace" },
                { "plan": two, "version": v2, "decision": "replace" },
                { "plan": three, "version": v3, "decision": "keep" },
            ] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({ "results": [
            { "plan": four, "state": "stale" },
            { "plan": two, "state": "approved" },
            { "plan": three, "state": "kept" },
        ] })
    );
    // The job is in line for the two decisions; the plan left stale is still
    // open.
    let detail = app.detail(&job).await;
    assert_eq!(detail["state"], "pending");
    let states: Vec<_> = detail["replacements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["episode"].as_i64().unwrap(), r["state"].clone()))
        .collect();
    assert!(states.contains(&(2, json!("approved"))), "{states:?}");
    assert!(states.contains(&(3, json!("kept"))), "{states:?}");
    assert!(states.contains(&(4, json!("open"))), "{states:?}");
}

#[tokio::test]
async fn a_list_of_decisions_that_cannot_be_read_is_refused_with_nothing_written() {
    let app = App::new().await;
    let (job, plans) = app.three_waiting().await;
    let (id, version) = &plans[0];
    let (other, _) = &plans[1];
    let one = |plan: &str, decision: &str| json!({ "plan": plan, "version": version, "decision": decision });
    let too_many: Vec<Value> = (0..1001).map(|i| one(&format!("p{i}"), "keep")).collect();
    let cases = [
        json!({ "decisions": [] }),
        json!({ "decisions": too_many }),
        json!({ "decisions": [one(id, "keep"), one(other, "keep"), one(id, "replace")] }),
        json!({ "decisions": [one(id, "keep"), one(other, "maybe")] }),
    ];
    for body in cases {
        let (status, error) = app.decide_many(&job, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:.200} {error}");
        assert_eq!(error["error"], "invalid");
    }
    // The limit itself is fine to ask: 1,000 plans that do not exist are
    // not found, not too many.
    let at_limit: Vec<Value> = (0..1000).map(|i| one(&format!("p{i}"), "keep")).collect();
    let (status, _) = app
        .decide_many(&job, json!({ "decisions": at_limit }))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Without a body the request is not read at all.
    let (status, _) = app.decide_many(&job, json!({})).await;
    assert!(status.is_client_error());

    let detail = app.detail(&job).await;
    assert_eq!(detail["wait"], "approval");
    assert!(detail["replacements"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["state"] == "open"));
}

#[tokio::test]
async fn a_plan_of_another_job_is_not_found_and_nothing_of_the_list_is_written() {
    let app = App::new().await;
    let (job, plans) = app.three_waiting().await;
    // Another job's plan for episode 2 waits too.
    let (other, foreign) = app.revision_of("c3", "/ok/Show-02v3").await;
    assert_ne!(job, other);
    let mut decisions: Vec<Value> = plans
        .iter()
        .map(|(id, v)| json!({ "plan": id, "version": v, "decision": "replace" }))
        .collect();
    decisions.push(json!({ "plan": foreign, "version": 1, "decision": "replace" }));
    let (status, body) = app
        .decide_many(&job, json!({ "decisions": decisions }))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "not_found");
}

#[tokio::test]
async fn the_worker_is_woken_only_when_a_decision_was_written() {
    use std::os::unix::net::UnixDatagram;
    let mut app = App::new().await;
    let socket_path = app.dir.path().join("worker.wake");
    let socket = UnixDatagram::bind(&socket_path).unwrap();
    socket.set_nonblocking(true).unwrap();
    app.state = app.state.clone().with_worker_wake(socket_path);
    app.router = testing::api(&app.state);
    let (job, plans) = app.three_waiting().await;
    let woke = || socket.recv(&mut [0u8; 8]).is_ok();
    while woke() {}

    let stale: Vec<Value> = plans
        .iter()
        .map(|(id, v)| json!({ "plan": id, "version": v + 1, "decision": "replace" }))
        .collect();
    let (status, body) = app.decide_many(&job, json!({ "decisions": stale })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!woke(), "nothing was written");

    let all: Vec<Value> = plans
        .iter()
        .map(|(id, v)| json!({ "plan": id, "version": v, "decision": "replace" }))
        .collect();
    let (status, body) = app.decide_many(&job, json!({ "decisions": all })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(woke(), "the worker was woken");
}
