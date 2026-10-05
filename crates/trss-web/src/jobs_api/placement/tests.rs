use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use super::super::tests::{app, call, get, sql};

const SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// An upload of season 1 (three episodes; videos of the first two, a
/// subtitle on the second) waiting for its 배치 확인: `01` and `02` on their
/// numbers, `04` held outside the season, and a font kept as it is.
async fn waiting_upload(state: &crate::AppState) {
    sql(
        state,
        "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
         INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
         INSERT INTO seasons (work_id, number) VALUES ('w1', 1);
         INSERT OR IGNORE INTO season_info (work_id, season) VALUES ('w1', 1);
         INSERT INTO anilist_entries (id, format, episodes, fetched_at) VALUES (1, 'TV', 3, 1);
         INSERT INTO season_entries (work_id, season, position, anilist_id) VALUES ('w1', 1, 0, 1);
         INSERT INTO episodes (work_id, season, episode) VALUES ('w1', 1, '01'), ('w1', 1, '02');
         INSERT INTO media_files (work_id, path, season, episode, kind)
             VALUES ('w1', 'Season 01/Show S01E01.mkv', 1, '01', 'video'),
                    ('w1', 'Season 01/Show S01E02.mkv', 1, '02', 'video'),
                    ('w1', 'Season 01/Show S01E02.ass', 1, '02', 'subtitle');
         INSERT INTO subtitle_jobs (id, command_id, request, origin, state, wait, stage, note,
                                    work_id, season, created_at, updated_at, state_at)
             VALUES ('j1', 'c1', '{}', 'upload', 'waiting', 'placement', 'placement',
                     '자막 3개가 붙을 회차를 확인해 주세요 · 회차를 정할 파일 1개',
                     'w1', 1, 1, 1, 1);
         INSERT INTO subtitle_job_items (id, job_id, position, episode, post_url, found_at, state,
                                         updated_at)
             VALUES (1, 'j1', 0, '', 'upload:', 1, 'done', 1);
         INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state, size, path,
                                         created_at, updated_at, format, kind)
             VALUES ('a', 'j1', 1, 'a', 'Show - 01.ass', 'done', 9, 'j1/a', 1, 1, 'ass', 'subtitle'),
                    ('b', 'j1', 1, 'b', 'Show - 02.ass', 'done', 9, 'j1/b', 1, 1, 'ass', 'subtitle'),
                    ('c', 'j1', 1, 'c', 'Show - 04.ass', 'done', 9, 'j1/c', 1, 1, 'ass', 'subtitle'),
                    ('d', 'j1', 1, 'd', 'A.ttf', 'done', 9, 'j1/d', 1, 1, 'other', 'font');",
    )
    .await;
    let rows = [
        (
            0,
            "a",
            "Show - 01.ass",
            "'subtitle', 'ass'",
            "'01'",
            "'explicit'",
            "1",
            "'apply'",
            "NULL",
        ),
        (
            1,
            "b",
            "Show - 02.ass",
            "'subtitle', 'ass'",
            "'02'",
            "'explicit'",
            "2",
            "'apply'",
            "NULL",
        ),
        (
            2,
            "c",
            "Show - 04.ass",
            "'subtitle', 'ass'",
            "'04'",
            "NULL",
            "NULL",
            "'apply'",
            "'4화가 시즌의 1–3화 밖이에요'",
        ),
        (
            3,
            "d",
            "A.ttf",
            "'font', NULL",
            "NULL",
            "NULL",
            "NULL",
            "'store'",
            "NULL",
        ),
    ];
    let mut insert = String::new();
    for (position, file, name, kind, named, assignment, episode, action, question) in rows {
        insert.push_str(&format!(
            "INSERT INTO subtitle_job_plan (job_id, position, file_id, name, kind, format, size,
                                            sha256, item_id, attachment_episode, assignment,
                                            episode, action, question, updated_at)
             VALUES ('j1', {position}, '{file}', '{name}', {kind}, 9, '{SHA}', 1, {named},
                     {assignment}, {episode}, {action}, {question}, 1);"
        ));
    }
    let insert: &'static str = Box::leak(insert.into_boxed_str());
    sql(state, insert).await;
}

fn placing(rows: &[(i64, Option<i64>, bool)]) -> Value {
    json!({
        "rows": rows
            .iter()
            .map(|(position, episode, apply)| {
                json!({ "position": position, "episode": episode, "apply": apply })
            })
            .collect::<Vec<_>>()
    })
}

#[tokio::test]
async fn an_upload_waiting_for_its_placement_shows_its_table_and_one_to_do() {
    let (state, router) = app();
    waiting_upload(&state).await;

    let (status, detail) = get(&router, "/api/subtitle-jobs/j1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        detail["confirm"],
        json!({
            "scope": "whole",
            "positions": [0, 1, 2],
            "total": 3,
            "episodes": [
                { "episode": 1, "video": "Show S01E01.mkv", "videos": 1, "subtitle": false },
                { "episode": 2, "video": "Show S01E02.mkv", "videos": 1, "subtitle": true },
                { "episode": 3, "video": null, "videos": 0, "subtitle": false },
            ],
        })
    );
    let rows = detail["placements"].as_array().unwrap();
    let row = |name: &str| rows.iter().find(|r| r["name"] == name).unwrap();
    assert_eq!(row("Show - 01.ass")["named"], "01");
    assert_eq!(row("Show - 01.ass")["assignment"], "explicit");
    assert_eq!(row("Show - 04.ass")["episode"], Value::Null);
    assert_eq!(
        row("Show - 04.ass")["question"],
        "4화가 시즌의 1–3화 밖이에요"
    );
    assert_eq!(row("A.ttf")["kind"], "font");

    // One to-do for the job, saying what it waits for.
    let (_, todo) = get(&router, "/api/todo").await;
    let checks: Vec<&Value> = todo["needs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["kind"] == "placement_check")
        .collect();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0]["job_id"], "j1");
    // The files its table places, the held one among them.
    assert_eq!(
        checks[0]["files"],
        json!(["Show - 01.ass", "Show - 02.ass", "Show - 04.ass"])
    );
    assert_eq!(
        checks[0]["reason"],
        "자막 3개가 붙을 회차를 확인해 주세요 · 회차를 정할 파일 1개"
    );
}

#[tokio::test]
async fn the_persons_table_is_taken_once_and_one_that_cannot_be_kept_is_refused() {
    let (state, router) = app();
    waiting_upload(&state).await;
    let uri = "/api/subtitle-jobs/j1/placement";

    // An episode outside the season, and a file applied on none.
    let (status, answer) = call(
        &router,
        Method::POST,
        uri,
        Some(placing(&[
            (0, Some(1), true),
            (1, Some(2), true),
            (2, Some(4), true),
        ])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(answer["message"], "4화는 이 시즌의 1–3화 밖이에요.");
    let (status, answer) = call(
        &router,
        Method::POST,
        uri,
        Some(placing(&[
            (0, Some(1), true),
            (1, Some(2), true),
            (2, None, true),
        ])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    // Rows that are not the table's.
    let (status, _) = call(
        &router,
        Method::POST,
        uri,
        Some(placing(&[(0, Some(1), true), (1, Some(2), true)])),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // The table as the person set it: `04` on episode 3.
    let (status, answer) = call(
        &router,
        Method::POST,
        uri,
        Some(placing(&[
            (0, Some(1), true),
            (1, Some(2), true),
            (2, Some(3), true),
        ])),
    )
    .await;
    assert_eq!(
        (status, answer),
        (StatusCode::OK, json!({ "applied": 3, "stored": 0 }))
    );
    let (_, detail) = get(&router, "/api/subtitle-jobs/j1").await;
    assert_eq!(detail["state"], "pending");
    assert_eq!(detail["confirm"], Value::Null);
    let rows = detail["placements"].as_array().unwrap();
    let moved = rows.iter().find(|r| r["name"] == "Show - 04.ass").unwrap();
    assert_eq!(
        (&moved["episode"], &moved["assignment"]),
        (&json!(3), &json!("explicit"))
    );
    let (_, todo) = get(&router, "/api/todo").await;
    assert!(!todo["needs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["kind"] == "placement_check"));

    // Taken once; no such job is no table.
    let (status, _) = call(
        &router,
        Method::POST,
        uri,
        Some(placing(&[
            (0, Some(1), true),
            (1, Some(2), true),
            (2, Some(3), true),
        ])),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs/nope/placement",
        Some(placing(&[])),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_upload_of_fonts_alone_is_confirmed_with_no_row() {
    let (state, router) = app();
    waiting_upload(&state).await;
    sql(
        &state,
        "DELETE FROM subtitle_job_plan WHERE kind = 'subtitle';
         UPDATE subtitle_jobs SET note = '받은 폰트와 첨부를 보관하기 전에 확인해 주세요';",
    )
    .await;
    let (_, detail) = get(&router, "/api/subtitle-jobs/j1").await;
    assert_eq!(detail["confirm"]["scope"], "whole");
    assert_eq!(detail["confirm"]["positions"], json!([]));
    let (status, answer) = call(
        &router,
        Method::POST,
        "/api/subtitle-jobs/j1/placement",
        Some(placing(&[])),
    )
    .await;
    assert_eq!(
        (status, answer),
        (StatusCode::OK, json!({ "applied": 0, "stored": 0 }))
    );
}
