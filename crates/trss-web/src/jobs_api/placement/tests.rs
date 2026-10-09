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
async fn an_upload_waiting_for_its_placement_shows_its_table() {
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
}

#[tokio::test]
async fn the_persons_table_is_taken_once_and_one_that_cannot_be_kept_is_refused() {
    let (state, router) = app();
    waiting_upload(&state).await;
    let uri = "/api/subtitle-jobs/j1/placement";

    // An episode outside the season (the season's total is the web's to
    // pass on), refused with a sentence.
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
    assert!(answer["message"].as_str().is_some_and(|m| !m.is_empty()));
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
async fn an_upload_of_fonts_alone_shows_a_table_with_no_row() {
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
}

/// The source `src`'s subtitle of Anissia's episode 14, applied on episode 2
/// under the mapping `−12`, and the mapping changed to `−11`: its relocation
/// waits for a person. Returns the relocation job.
async fn waiting_relocation(state: &crate::AppState) -> String {
    sql(
        state,
        "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
         INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
         INSERT INTO seasons (work_id, number) VALUES ('w1', 1);
         INSERT INTO episodes (work_id, season, episode) VALUES ('w1', 1, '02'), ('w1', 1, '03');
         INSERT INTO media_files (work_id, path, season, episode, kind)
             VALUES ('w1', 'Season 01/Show S01E02.mkv', 1, '02', 'video'),
                    ('w1', 'Season 01/Show S01E02.ass', 1, '02', 'subtitle'),
                    ('w1', 'Season 01/Show S01E03.mkv', 1, '03', 'video');
         INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
             VALUES ('src', 7, '제작자', 0);
         INSERT INTO subtitle_jobs (id, command_id, request, origin, work_id, season, source_id,
                                    state, created_at, updated_at, state_at)
             VALUES ('j0', 'c0', '{}', 'pick', 'w1', 1, 'src', 'done', 0, 0, 0);
         INSERT INTO subtitle_job_items (id, job_id, position, episode, post_url, found_at, state,
                                         updated_at)
             VALUES (1, 'j0', 0, '14', 'https://example.org/p', 0, 'done', 0);
         INSERT INTO subtitle_packages (id, work_id, job_id, source_kind, created_at)
             VALUES ('p1', 'w1', 'j0', 'post', 0);
         INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state, size, sha256,
                                         created_at, updated_at)
             VALUES ('f14', 'j0', 1, 'k14', 'Show - 14.ass', 'done', 1, printf('%064d', 14), 0, 0);
         INSERT INTO subtitle_assets (id, work_id, kind, base, relative_path, byte_size, sha256,
                                      created_at)
             VALUES ('a14', 'w1', 'subtitle', 'work', '.trss/subtitles/제작자/Show - 14.ass', 1,
                     printf('%064d', 14), 0);
         INSERT INTO subtitle_stored (id, work_id, season, package_id, subtitle_asset_id,
                                      source_id, anissia_episode, assignment, basis, episode,
                                      format, creator, stored_at)
             VALUES ('s14', 'w1', 1, 'p1', 'a14', 'src', '14', 'mapped', 'anissia', 2, 'ass',
                     '제작자', 0);
         INSERT INTO subtitle_job_plan (job_id, position, file_id, name, kind, format, size,
                                        sha256, item_id, anissia_episode, assignment, basis,
                                        episode, action, stored_id, outcome, updated_at)
             VALUES ('j0', 0, 'f14', 'Show - 14.ass', 'subtitle', 'ass', 1, printf('%064d', 14), 1,
                     '14', 'mapped', 'anissia', 2, 'apply', 's14', 'applied', 0);
         INSERT INTO subtitle_applied (id, work_id, stored_id, season, episode, video_path, path,
                                       byte_size, sha256, object, job_id, applied_at)
             VALUES ('ap14', 'w1', 's14', 1, 2, 'Season 01/Show S01E02.mkv',
                     'Season 01/Show S01E02.ass', 1, printf('%064d', 14), '1:2', 'j0', 0);
         INSERT INTO subtitle_episode_mappings
             (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
             VALUES ('w1', 1, 'src', 'user', -11, '시험', 0);",
    )
    .await;
    let remapped = state
        .db()
        .run(|c| {
            Ok::<_, trss_core::DbError>(trss_jobs::place::relocate::reevaluate_in(
                c, "w1", 1, "src", None, 10,
            )?)
        })
        .await
        .unwrap();
    assert_eq!(remapped.stored, 1);
    remapped.relocation.expect("a relocation waits")
}

#[tokio::test]
async fn a_relocation_shows_the_copies_it_takes_off_beside_the_rows_it_applies() {
    let (state, router) = app();
    let job = waiting_relocation(&state).await;

    let (status, detail) = get(&router, &format!("/api/subtitle-jobs/{job}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["origin"], "relocate");
    assert_eq!(detail["confirm"]["scope"], "relocate");
    // Nothing to find, open or receive: the steps it reached.
    assert_eq!(
        detail["steps"],
        json!([{ "step": "placement", "state": "waiting", "at": 10,
                 "note": "회차 대응이 바뀌어 적용본 1개를 옮길 계획을 확인해 주세요" }])
    );
    let position = detail["confirm"]["positions"][0].clone();
    let row = detail["placements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["position"] == position)
        .unwrap();
    assert_eq!(
        (row["name"].as_str(), row["episode"].as_i64()),
        (Some("Show - 14.ass"), Some(3))
    );
    let relocations = detail["relocations"].as_array().unwrap();
    assert_eq!(relocations.len(), 1);
    assert_eq!(
        (
            &relocations[0]["episode"],
            &relocations[0]["path"],
            &relocations[0]["position"],
            &relocations[0]["state"],
            &relocations[0]["reason"],
        ),
        (
            &json!(2),
            &json!("Season 01/Show S01E02.ass"),
            &position,
            &json!("planned"),
            &Value::Null,
        )
    );
}

#[tokio::test]
async fn a_relocation_is_confirmed_as_shown_with_its_removals() {
    let (state, router) = app();
    let job = waiting_relocation(&state).await;
    let (_, detail) = get(&router, &format!("/api/subtitle-jobs/{job}")).await;
    let position = detail["confirm"]["positions"][0].as_i64().unwrap();
    let removal = detail["relocations"][0]["id"].clone();
    let uri = format!("/api/subtitle-jobs/{job}/placement");

    let (status, answer) = call(
        &router,
        Method::POST,
        &uri,
        Some(json!({
            "rows": [{ "position": position, "episode": 3, "apply": true }],
            "removals": [removal],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer, json!({ "applied": 1, "stored": 0 }));
    let (_, detail) = get(&router, &format!("/api/subtitle-jobs/{job}")).await;
    assert_eq!(detail["state"], "pending");
    assert_eq!(detail["confirm"], Value::Null);
    assert_eq!(detail["relocations"][0]["state"], "planned");
}
