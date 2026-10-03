use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use trss_collect::store::history::{HistoryResult, Observation};
use trss_core::{Db, DbError};
use trss_jobs::{ItemState, JobStore};

const ANIME: i64 = 3424;

fn app() -> (AppState, Router) {
    let state = AppState::new(Db::open_blocking(":memory:").unwrap());
    let router = Router::new().nest("/api", crate::api::router().with_state(state.clone()));
    (state, router)
}

async fn call(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn get(router: &Router, uri: &str) -> (StatusCode, Value) {
    call(router, Method::GET, uri, None).await
}

async fn sql(state: &AppState, sql: &'static str) {
    state
        .jobs
        .db()
        .run::<_, DbError, _>(move |c| Ok(c.execute_batch(sql)?))
        .await
        .unwrap();
}

/// Work `w1` season 1 linked to anime [`ANIME`], two creators' candidates:
/// observations 1–3 by `s1` (에루샤), 4 by `s2`, and anime 9 with its own.
async fn linked_season(state: &AppState) {
    sql(
        state,
        "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
         INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
         INSERT INTO seasons (work_id, number) VALUES ('w1', 1);
         INSERT INTO seasons (work_id, number) VALUES ('w1', 2);
         INSERT INTO anissia_anime (anime_no, subject, week, status, fetched_at)
             VALUES (3424, '작품', 2, 'ON', 77);
         INSERT INTO season_anissia (work_id, season, anime_no, version) VALUES ('w1', 1, 3424, 1);
         INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
             VALUES ('s1', 3424, '에루샤', 5), ('s2', 3424, '다른', 5), ('s9', 9, '남', 5);
         INSERT INTO caption_observations (source_id, post_url, episode, updated, first_seen_at)
             VALUES ('s1', 'https://fake.trss.invalid/ok/1', '1', 'x', 6),
                    ('s1', 'https://fake.trss.invalid/ok/2', '2', 'x', 6),
                    ('s1', 'https://fake.trss.invalid/ok/3', '3', 'x', 6),
                    ('s2', 'https://fake.trss.invalid/ok/4', '4', 'x', 6),
                    ('s9', 'https://fake.trss.invalid/ok/9', '9', 'x', 6);",
    )
    .await;
}

fn pick(id: &str, candidates: &[i64]) -> Value {
    json!({ "id": id, "work_id": "w1", "season": 1, "candidates": candidates })
}

#[tokio::test]
async fn a_pick_makes_one_job_per_browser_id_of_one_creators_candidates() {
    let (state, router) = app();
    linked_season(&state).await;

    let (status, made) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs",
        Some(pick("b1", &[1, 2])),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let id = made["id"].as_str().unwrap().to_owned();
    let (status, again) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs",
        Some(pick("b1", &[1, 2])),
    )
    .await;
    assert_eq!(
        (status, again["id"].as_str()),
        (StatusCode::OK, Some(id.as_str()))
    );
    let (status, other) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs",
        Some(pick("b1", &[1])),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(other["current"]["id"].as_str(), Some(id.as_str()));
    // The IDs of the app's own receipts are not a browser's to take.
    let (status, refused) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs",
        Some(pick("auto:3", &[3])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(refused["message"], "이 요청 ID는 쓸 수 없어요.");
    let (status, refused) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs",
        Some(pick("recheck:3:ab12", &[3])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(refused["message"], "이 요청 ID는 쓸 수 없어요.");

    // Accepted is pending, not done: the detail says what it is about.
    let (status, detail) = get(&router, &format!("/api/subtitle-jobs/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["state"], "pending");
    assert_eq!(detail["title"], "작품");
    assert_eq!(detail["creator"], "에루샤");
    assert_eq!(detail["episodes"], json!(["1", "2"]));
    assert_eq!(detail["work"]["name"], "Show");
    assert_eq!(detail["source"], "fake.trss.invalid");
    let steps: Vec<(&str, &str)> = detail["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["step"].as_str().unwrap(), s["state"].as_str().unwrap()))
        .collect();
    assert_eq!(
        steps,
        [
            ("found", "done"),
            ("open", "upcoming"),
            ("receive", "upcoming")
        ]
    );
    assert_eq!(detail["receive_dir"], format!("receive/{id}"));

    let refused = [
        (pick("b2", &[1, 4]), StatusCode::BAD_REQUEST),
        (pick("b3", &[9]), StatusCode::BAD_REQUEST),
        (pick("b4", &[]), StatusCode::BAD_REQUEST),
        (pick("b5", &[1, 1]), StatusCode::BAD_REQUEST),
        (
            json!({ "id": "b6", "work_id": "w1", "season": 2, "candidates": [1] }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({ "id": "b7", "work_id": "w1", "season": 3, "candidates": [1] }),
            StatusCode::NOT_FOUND,
        ),
    ];
    for (body, expected) in refused {
        let (status, _) = call(
            &router,
            Method::POST,
            "/api/subtitle-jobs",
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, expected, "{body}");
    }
    assert_eq!(state.jobs.open_jobs().await.unwrap().len(), 1);
    assert_eq!(
        get(&router, "/api/subtitle-jobs/nope").await.0,
        StatusCode::NOT_FOUND
    );
}

/// Makes a job of `items` (post paths) and leaves it as `state` with each
/// item as given.
async fn job_in(
    jobs: &JobStore,
    n: usize,
    state: JobState,
    wait: Option<Wait>,
    items: &[(ItemState, Option<&str>)],
    at: i64,
) -> String {
    let made = jobs
        .create(
            NewJob {
                command_id: format!("c{n}"),
                request: "{}".into(),
                origin: "pick".into(),
                work_id: Some("w1".into()),
                season: Some(1),
                anime_no: Some(ANIME),
                source_id: Some("s1".into()),
                creator: Some("에루샤".into()),
                revision_of: None,
                revises_attributed: false,
                items: items
                    .iter()
                    .enumerate()
                    .map(|(i, _)| NewItem {
                        observation_id: None,
                        episode: (i + 1).to_string(),
                        post_url: format!("https://fake.trss.invalid/ok/{n}-{i}"),
                        found_at: 1,
                    })
                    .collect(),
            },
            at,
        )
        .await
        .unwrap();
    let Created::Created(id) = made else { panic!() };
    for (item, (item_state, reason)) in jobs.items(&id).await.unwrap().iter().zip(items) {
        let item_wait = (*item_state == ItemState::Waiting)
            .then_some(wait)
            .flatten();
        jobs.set_item(
            item.id,
            *item_state,
            item_wait,
            reason.map(str::to_owned),
            at,
        )
        .await
        .unwrap();
    }
    if state != JobState::Pending {
        jobs.settle(&id, state, wait, None, at).await.unwrap();
    }
    id
}

#[tokio::test]
async fn the_groups_put_failures_first_and_page_the_done_jobs_five_then_more() {
    let (state, router) = app();
    linked_season(&state).await;
    let jobs = &state.jobs;
    let ok = [(ItemState::Done, None)];
    let failed = [(ItemState::Failed, Some("게시물이 없어요 (404)"))];
    let mut n = 0;
    let mut next = || {
        n += 1;
        n
    };
    for i in 0..115 {
        job_in(jobs, next(), JobState::Done, None, &ok, 10_000 + i).await;
    }
    let old_failure = job_in(jobs, next(), JobState::Failed, None, &failed, 100).await;
    let new_failure = job_in(jobs, next(), JobState::Partial, None, &ok, 200).await;
    let pending = job_in(
        jobs,
        next(),
        JobState::Pending,
        None,
        &[(ItemState::Pending, None)],
        50,
    )
    .await;
    let held = job_in(
        jobs,
        next(),
        JobState::Held,
        None,
        &[(ItemState::Held, Some("x"))],
        50,
    )
    .await;
    let subtitle = job_in(
        jobs,
        next(),
        JobState::Waiting,
        Some(Wait::Subtitle),
        &[(ItemState::Waiting, Some("…"))],
        50,
    )
    .await;
    let mut auth = Vec::new();
    for _ in 0..2 {
        auth.push(
            job_in(
                jobs,
                next(),
                JobState::Waiting,
                Some(Wait::Auth),
                &[(ItemState::Waiting, Some("CAPTCHA"))],
                60,
            )
            .await,
        );
    }
    let running = job_in(
        jobs,
        next(),
        JobState::Running,
        None,
        &[(ItemState::Running, None)],
        70,
    )
    .await;

    let (status, groups) = get(&router, "/api/subtitle-jobs").await;
    assert_eq!(status, StatusCode::OK);
    let ids = |key: &str| -> Vec<String> {
        groups[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|j| j["id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(ids("failed"), [new_failure, old_failure]);
    assert_eq!(
        ids("waiting"),
        [auth[0].clone(), auth[1].clone(), subtitle, held, pending]
    );
    assert_eq!(ids("running"), [running]);
    let done = &groups["done"];
    assert_eq!(done["items"].as_array().unwrap().len(), 5);
    assert_eq!(done["total"], 115);

    // The rest comes a page at a time, newest first, each job once.
    let mut seen: Vec<i64> = done["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["state_at"].as_i64().unwrap())
        .collect();
    let mut after = done["next"].as_str().map(str::to_owned);
    while let Some(cursor) = after {
        let (status, page) = get(&router, &format!("/api/subtitle-jobs/done?after={cursor}")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(page["items"].as_array().unwrap().len() <= 20);
        seen.extend(
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|j| j["state_at"].as_i64().unwrap()),
        );
        after = page["next"].as_str().map(str::to_owned);
    }
    assert_eq!(seen.len(), 115);
    assert!(seen.windows(2).all(|w| w[0] > w[1]));

    // Failed jobs are no to-do; the checks a person has to pass are, one per work.
    let (_, todo) = get(&router, "/api/todo").await;
    assert_eq!(todo["count"], 1);
    assert_eq!(todo["needs"][0]["kind"], "auth");
    assert_eq!(todo["needs"][0]["jobs"], 2);
    assert_eq!(todo["needs"][0]["job_id"], auth[0].as_str());
    assert_eq!(todo["needs"][0]["reason"], "CAPTCHA");
}

#[tokio::test]
async fn a_job_with_one_of_three_failed_shows_each_episode_with_its_reason() {
    let (state, router) = app();
    linked_season(&state).await;
    let id = job_in(
        &state.jobs,
        1,
        JobState::Partial,
        None,
        &[
            (ItemState::Done, None),
            (ItemState::Failed, Some("게시물이 없어요 (404)")),
            (ItemState::Done, None),
        ],
        100,
    )
    .await;
    let (_, detail) = get(&router, &format!("/api/subtitle-jobs/{id}")).await;
    assert_eq!(detail["state"], "partial");
    assert_eq!(
        detail["progress"],
        json!({ "done": 2, "failed": 1, "total": 3 })
    );
    let items = detail["items"].as_array().unwrap();
    assert_eq!(items[1]["state"], "failed");
    assert_eq!(items[1]["reason"], "게시물이 없어요 (404)");
    assert_eq!(items[0]["state"], "done");
}

#[tokio::test]
async fn an_unfinished_file_is_receiving_only_while_its_episode_runs() {
    let (state, router) = app();
    linked_season(&state).await;
    let id = job_in(
        &state.jobs,
        1,
        JobState::Held,
        None,
        &[
            (ItemState::Running, None),
            (ItemState::Held, Some("확인하지 못했어요")),
        ],
        100,
    )
    .await;
    for (n, item) in state.jobs.items(&id).await.unwrap().iter().enumerate() {
        state
            .jobs
            .file_intend(trss_jobs::store::FileRow {
                id: format!("a{n}"),
                item_id: item.id,
                file_key: format!("k{n}"),
                name: format!("{n}.ass"),
                state: trss_jobs::FileState::Intended,
                same_as: None,
                temp_dir: Some(format!(".tmp/a{n}")),
                expected_size: None,
                size: None,
                sha256: None,
                object: None,
                path: None,
                reason: None,
                created_at: 100,
                format: None,
                failure: None,
                http_status: None,
                content_type: None,
                response_size: None,
                snapshot: None,
                kind: None,
                archive: None,
                folder: None,
            })
            .await
            .unwrap();
    }
    let (_, detail) = get(&router, &format!("/api/subtitle-jobs/{id}")).await;
    let items = detail["items"].as_array().unwrap();
    assert_eq!(items[0]["files"][0]["state"], "receiving");
    assert_eq!(items[1]["files"][0]["state"], "held");
}

#[tokio::test]
async fn a_site_check_comes_before_a_receive_failure_and_the_badge_counts_both() {
    let (state, router) = app();
    linked_season(&state).await;
    job_in(
        &state.jobs,
        1,
        JobState::Waiting,
        Some(Wait::Auth),
        &[
            (ItemState::Waiting, Some("CAPTCHA")),
            (ItemState::Done, None),
        ],
        100,
    )
    .await;
    // A newer receive failure still comes after the check.
    state
        .history
        .record(
            500,
            vec![Observation {
                channel_id: "c1".into(),
                channel_label: "https://feed.test/".into(),
                identity_key: "k".into(),
                title: "[Group] Show - 03".into(),
                link: "https://feed.test/x".into(),
                result: HistoryResult::AddFailed,
                rule_id: Some("r1".into()),
                torrent_hash: None,
                reason: Some("Transmission에 연결하지 못했어요".into()),
            }],
        )
        .await
        .unwrap();

    let (_, todo) = get(&router, "/api/todo").await;
    let kinds: Vec<&str> = todo["needs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["auth", "receive_failed"]);
    assert_eq!(todo["count"], 2);
    assert_eq!(todo["needs"][0]["episodes"], json!(["1"]));
    assert_eq!(todo["needs"][0]["title"], "작품");
    assert_eq!(todo["needs"][1]["context"], "add_failed");
    assert_eq!(todo["needs"][1]["channel_id"], "c1");
    assert_eq!(todo["needs"][1]["count"], 1);
    assert_eq!(
        get(&router, "/api/todo/count").await.1,
        json!({ "count": 2 })
    );
}

#[tokio::test]
async fn a_tistory_receipt_shows_its_format_and_failures_by_class_and_no_signed_address() {
    use trss_subtitles::testing::{spec, FileAnswer, PostAnswer, SourceServer};

    let (state, router) = app();
    linked_season(&state).await;
    let server = SourceServer::start().await;
    server.post(
        "sumomomo",
        492,
        vec![PostAnswer::Files(vec![
            spec("z", "Seihantai - 24.zip", "0.01MB"),
            spec("p", "page.zip", "1KB"),
            spec("x", "x.zip", "1KB"),
        ])],
    );
    let zip =
        trss_subtitles::verify::zip_of(&[("a.srt", b"1\n00:00:01,000 --> 00:00:02,000\nx\n")]);
    server.file("z", vec![FileAnswer::Refused, FileAnswer::Bytes(zip)]);
    server.file("p", vec![FileAnswer::Page]);
    server.file("x", vec![FileAnswer::Refused]);
    let made = state
        .jobs
        .create(
            NewJob {
                command_id: "t1".into(),
                request: "{}".into(),
                origin: "pick".into(),
                work_id: Some("w1".into()),
                season: Some(1),
                anime_no: Some(ANIME),
                source_id: Some("s1".into()),
                creator: Some("에루샤".into()),
                revision_of: None,
                revises_attributed: false,
                items: vec![NewItem {
                    observation_id: None,
                    episode: "24".into(),
                    post_url: server.post_url("sumomomo", 492),
                    found_at: 1,
                }],
            },
            100,
        )
        .await
        .unwrap();
    let Created::Created(id) = made else { panic!() };
    let dir = tempfile::tempdir().unwrap();
    trss_jobs::Runner::new(
        state.jobs.clone(),
        trss_subtitles::Sources::none().with_tistory(server.source()),
        trss_jobs::ReceiveArea::new(dir.path()),
        std::sync::Arc::new(|| 1_000),
    )
    .run_ready(&tokio_util::sync::CancellationToken::new())
    .await
    .unwrap();

    let (status, detail) = get(&router, &format!("/api/subtitle-jobs/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["state"], "failed");
    assert_eq!(detail["failure"], "not_a_file");
    let item = &detail["items"][0];
    assert_eq!(item["failure"], "not_a_file");
    let files = item["files"].as_array().unwrap();
    let by_name = |name: &str| files.iter().find(|f| f["name"] == name).unwrap();
    let received = by_name("Seihantai - 24.zip");
    assert_eq!(received["state"], "done");
    assert_eq!(received["format"], "zip");
    assert_eq!(received["failure"], Value::Null);
    let page = by_name("page.zip");
    assert_eq!(
        (
            &page["failure"],
            &page["http_status"],
            &page["content_type"]
        ),
        (&json!("not_a_file"), &json!(200), &json!("text/html"))
    );
    let expired = by_name("x.zip");
    assert_eq!(
        (
            &expired["failure"],
            &expired["http_status"],
            &expired["response_size"]
        ),
        (&json!("expired"), &json!(404), &json!(150))
    );
    let (_, groups) = get(&router, "/api/subtitle-jobs").await;
    assert_eq!(groups["failed"][0]["failure"], "not_a_file");

    // Neither the answers nor the log carry a signed address.
    let (_, todo) = get(&router, "/api/todo").await;
    for text in [detail.to_string(), groups.to_string(), todo.to_string()] {
        assert!(!text.contains("signature=") && !text.contains("credential="));
    }
    assert!(detail["log"].as_array().unwrap().len() > 3);
}

#[tokio::test]
async fn an_automatic_revision_of_a_named_subtitle_says_so_with_no_earlier_job() {
    let (state, router) = app();
    linked_season(&state).await;
    let made = state
        .jobs
        .create(
            NewJob {
                command_id: "auto:1".into(),
                request: "{}".into(),
                origin: "auto".into(),
                work_id: Some("w1".into()),
                season: Some(1),
                anime_no: Some(ANIME),
                source_id: Some("s1".into()),
                creator: Some("에루샤".into()),
                revision_of: None,
                revises_attributed: true,
                items: vec![NewItem {
                    observation_id: None,
                    episode: "5".into(),
                    post_url: "https://fake.trss.invalid/ok/5".into(),
                    found_at: 1,
                }],
            },
            100,
        )
        .await
        .unwrap();
    let Created::Created(id) = made else { panic!() };
    let plain = job_in(
        &state.jobs,
        2,
        JobState::Pending,
        None,
        &[(ItemState::Pending, None)],
        100,
    )
    .await;

    let (_, detail) = get(&router, &format!("/api/subtitle-jobs/{id}")).await;
    assert_eq!(detail["revises_attributed"], true);
    assert_eq!(detail["revision_of"], Value::Null);
    assert_eq!(detail["revises_job"], Value::Null);
    assert_eq!(
        detail["log"].as_array().unwrap().last().unwrap()["message"],
        "구독 제작자의 수정본이 제작자를 붙인 자막에 맞아 자동으로 작업을 만들었어요"
    );
    let (_, other) = get(&router, &format!("/api/subtitle-jobs/{plain}")).await;
    assert_eq!(other["revises_attributed"], false);
}

fn find(id: &str, season: u32, creator: &str) -> Value {
    json!({ "id": id, "work_id": "w1", "season": season, "creator": creator })
}

#[tokio::test]
async fn a_find_makes_one_job_per_browser_id_that_opens_the_creators_newest_post() {
    let (state, router) = app();
    linked_season(&state).await;

    let (status, made) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs/find",
        Some(find("f1", 1, "s1")),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let id = made["id"].as_str().unwrap().to_owned();
    let (status, again) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs/find",
        Some(find("f1", 1, "s1")),
    )
    .await;
    assert_eq!(
        (status, again["id"].as_str()),
        (StatusCode::OK, Some(id.as_str()))
    );
    let (status, other) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs/find",
        Some(find("f1", 1, "s2")),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(other["current"]["id"].as_str(), Some(id.as_str()));
    // A pick under the same ID is another request too.
    let (status, _) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs",
        Some(pick("f1", &[1])),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, detail) = get(&router, &format!("/api/subtitle-jobs/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["origin"], "find");
    assert_eq!(detail["state"], "pending");
    assert_eq!(detail["creator"], "에루샤");
    assert_eq!(detail["episodes"], json!([]));
    assert_eq!(detail["finishing"], false);
    assert_eq!(detail["upload"]["subtitles"], 0);
    // The browser starts at the creator's most recently observed post.
    assert_eq!(
        detail["items"][0]["post_url"],
        "https://fake.trss.invalid/ok/3"
    );
    assert_eq!(detail["items"][0]["episode"], "");
    // Only the steps it reached.
    assert_eq!(detail["steps"], json!([]));

    let refused = [
        // A creator of another anime, or none of the season's.
        (find("f2", 1, "s9"), StatusCode::BAD_REQUEST),
        (find("f3", 1, "nope"), StatusCode::BAD_REQUEST),
        // A season with no Anissia anime has no creators to find.
        (find("f4", 2, "s1"), StatusCode::BAD_REQUEST),
        (find("f5", 3, "s1"), StatusCode::NOT_FOUND),
        (find("auto:9", 1, "s1"), StatusCode::BAD_REQUEST),
        (find(" ", 1, "s1"), StatusCode::BAD_REQUEST),
        (find(&"x".repeat(129), 1, "s1"), StatusCode::BAD_REQUEST),
    ];
    for (body, expected) in refused {
        let (status, answer) = call(
            &router,
            Method::POST,
            "/api/subtitle-jobs/find",
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, expected, "{body}: {answer}");
    }
    let (_, unknown) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs/find",
        Some(find("f3", 1, "nope")),
    )
    .await;
    assert_eq!(
        unknown["message"],
        "고른 제작자가 이 시즌의 제작자가 아니에요. 화면을 새로고침해 주세요."
    );
    let (_, unlinked) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs/find",
        Some(find("f4", 2, "s1")),
    )
    .await;
    assert_eq!(
        unlinked["message"],
        "이 시즌은 Anissia 작품에 연결돼 있지 않아서 직접 찾을 제작자가 없어요."
    );
    assert_eq!(state.jobs.open_jobs().await.unwrap().len(), 1);
}

#[tokio::test]
async fn finishing_a_find_job_asks_the_worker_and_never_ends_it_here() {
    let (state, router) = app();
    linked_season(&state).await;
    let (_, made) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs/find",
        Some(find("f1", 1, "s1")),
    )
    .await;
    let id = made["id"].as_str().unwrap().to_owned();
    let finish = format!("/api/subtitle-jobs/{id}/finish");

    // In line for the worker: it is the worker's to end.
    let (status, answer) = call(&router, Method::POST, &finish, None).await;
    assert_eq!(
        (status, &answer),
        (StatusCode::OK, &json!({ "state": "finishing" }))
    );
    let (_, detail) = get(&router, &format!("/api/subtitle-jobs/{id}")).await;
    assert_eq!(detail["finishing"], true);
    assert_eq!(detail["state"], "pending");

    // Waiting with no browser run bound: still the worker's to end, since a
    // download a run left in the job's folder is seen by the worker alone.
    sql(
        &state,
        "UPDATE subtitle_jobs SET state = 'waiting', wait = 'auth';
         UPDATE subtitle_job_items SET state = 'waiting', wait = 'auth';",
    )
    .await;
    let (status, answer) = call(&router, Method::POST, &finish, None).await;
    assert_eq!(
        (status, &answer),
        (StatusCode::OK, &json!({ "state": "finishing" }))
    );
    let (_, detail) = get(&router, &format!("/api/subtitle-jobs/{id}")).await;
    assert_eq!(detail["state"], "waiting");
    assert_eq!(detail["finishing"], true);

    // Once the worker ended it, the answer says so.
    state.jobs.end_find(&id, None, 20_000).await.unwrap();
    let (status, answer) = call(&router, Method::POST, &finish, None).await;
    assert_eq!(
        (status, &answer),
        (StatusCode::OK, &json!({ "state": "done" }))
    );
    let (_, detail) = get(&router, &format!("/api/subtitle-jobs/{id}")).await;
    assert_eq!(detail["note"], "받은 파일 없음");
    assert_eq!(detail["finishing"], false);

    // Only a find job is finished so.
    let (_, picked) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs",
        Some(pick("p1", &[1])),
    )
    .await;
    let (status, _) = call(
        &router,
        Method::POST,
        &format!(
            "/api/subtitle-jobs/{}/finish",
            picked["id"].as_str().unwrap()
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs/nope/finish",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_find_job_waits_among_the_ordinary_waits_not_with_the_checks() {
    let (state, router) = app();
    linked_season(&state).await;
    let (_, made) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs/find",
        Some(find("f1", 1, "s1")),
    )
    .await;
    let found = made["id"].as_str().unwrap().to_owned();
    // A person browses its screen: it waits as a check's job does.
    sql(
        &state,
        "UPDATE subtitle_jobs SET state = 'waiting', wait = 'auth' WHERE origin = 'find';
         UPDATE subtitle_job_items SET state = 'waiting', wait = 'auth';",
    )
    .await;
    let subtitle = job_in(
        &state.jobs,
        1,
        JobState::Waiting,
        Some(Wait::Subtitle),
        &[(ItemState::Waiting, Some("…"))],
        50,
    )
    .await;
    let check = job_in(
        &state.jobs,
        2,
        JobState::Waiting,
        Some(Wait::Auth),
        &[(ItemState::Waiting, Some("CAPTCHA"))],
        60,
    )
    .await;
    let (_, groups) = get(&router, "/api/subtitle-jobs").await;
    let ids: Vec<&str> = groups["waiting"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["id"].as_str().unwrap())
        .collect();
    // The check made last comes first; the find job then sits by its order
    // among the other waits.
    assert_eq!(ids, [check.as_str(), found.as_str(), subtitle.as_str()]);
}
