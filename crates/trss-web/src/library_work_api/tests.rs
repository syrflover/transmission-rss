use std::collections::BTreeSet;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use super::*;
use crate::api;
use trss_collect::store::channels::{ChannelInput, RuleInput, RuleState};
use trss_core::Db;
use trss_library::discovery::{
    EpisodeFile, FileKind, Reason, Scan, ScannedWork, Unrecognized, WorkRead,
};

fn file(season: u32, episode: &str, name: &str, kind: FileKind) -> EpisodeFile {
    EpisodeFile {
        path: format!("Season {season:02}/{name}"),
        kind,
        season,
        episode: episode.to_owned(),
    }
}

fn lycoris() -> ScannedWork {
    ScannedWork {
        dir_name: "Lycoris Recoil".into(),
        seasons: BTreeSet::from([1, 2]),
        files: vec![
            file(1, "02", "S01E02.mkv", FileKind::Video),
            file(1, "01", "S01E01.mkv", FileKind::Video),
            file(1, "01", "S01E01.ko.ass", FileKind::Subtitle),
            file(2, "01", "S02E01.mkv", FileKind::Video),
        ],
        unrecognized: vec![Unrecognized {
            path: "Season 01/extra.mkv".into(),
            reason: Reason::NoEpisode,
        }],
    }
}

fn rule(directory: &str, state: RuleState) -> RuleInput {
    RuleInput {
        r#match: Some("Lycoris".into()),
        directory: directory.into(),
        state,
        ..Default::default()
    }
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let response = api::router()
        .with_state(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn state() -> AppState {
    AppState::new(Db::open_blocking(":memory:").unwrap())
}

/// A state with watch folder `/c` holding `Lycoris Recoil`, and that work's ID.
async fn state_with_work() -> (AppState, String) {
    let state = state();
    state
        .library
        .add_folder(
            "/c".into(),
            Scan {
                works: vec![WorkRead::Read(lycoris())],
            },
            100,
            &[],
        )
        .await
        .unwrap();
    let id = state.library.overview().await.unwrap()[0].id.clone();
    (state, id)
}

#[tokio::test]
async fn answers_the_work_with_its_seasons_files_and_leftovers() {
    let (state, id) = state_with_work().await;
    let (status, body) = get(&state, &format!("/library/works/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "Lycoris Recoil");
    assert_eq!(body["missing"], false);
    assert_eq!(body["watch_folder"]["path"], "/c");
    assert_eq!(body["folder_path"], "/c/Lycoris Recoil");
    assert_eq!(body["added_at"], Value::Null);

    let seasons = body["seasons"].as_array().unwrap();
    assert_eq!(seasons.len(), 2);
    assert_eq!(seasons[0]["number"], 1);
    let episodes = seasons[0]["episodes"].as_array().unwrap();
    assert_eq!(episodes[0]["episode"], "01");
    assert_eq!(episodes[0]["sort"], 1.0);
    assert_eq!(episodes[1]["episode"], "02");
    assert_eq!(episodes[0]["video"][0]["path"], "Season 01/S01E01.mkv");
    assert_eq!(episodes[0]["video"][0]["added_at"], Value::Null);
    assert_eq!(
        episodes[0]["subtitle"][0]["path"],
        "Season 01/S01E01.ko.ass"
    );
    assert_eq!(episodes[1]["subtitle"].as_array().unwrap().len(), 0);

    let leftovers = body["unrecognized"].as_array().unwrap();
    assert_eq!(leftovers.len(), 1);
    assert_eq!(leftovers[0]["path"], "Season 01/extra.mkv");
    assert_eq!(leftovers[0]["reason"], "no_episode");
    assert_eq!(leftovers[0]["message"], Reason::NoEpisode.message());
    assert_eq!(body["rules"], serde_json::json!([]));
}

#[tokio::test]
async fn an_unknown_work_is_a_json_404() {
    let (state, _) = state_with_work().await;
    let (status, body) = get(&state, "/library/works/no-such-work").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not_found");
}

#[tokio::test]
async fn the_list_route_is_not_taken_by_the_detail_route() {
    let (state, id) = state_with_work().await;
    let (status, body) = get(&state, "/library/works").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"][0]["id"], id.as_str());
}

#[tokio::test]
async fn lists_the_rules_that_save_into_the_work_folder() {
    let (state, id) = state_with_work().await;
    state
        .settings
        .put_collection(0, "/c".into(), Some("/a".into()))
        .await
        .unwrap();
    let mut named = ChannelInput::new("https://feeds.example.org/rss?key=1");
    named.name = Some("SubsPlease".into());
    state
        .channels
        .create_channel_with_rules(
            named,
            vec![
                rule("Lycoris Recoil/Season 01", RuleState::Active),
                rule("Other Show/Season 01", RuleState::Active),
                // Saves into the collect folder itself: no work folder.
                rule("", RuleState::Active),
                // The name is a whole path component, not a prefix.
                rule("Lycoris Recoil 2/Season 01", RuleState::Active),
                // Leaves the folder text-wise.
                rule("Lycoris Recoil/../Elsewhere", RuleState::Active),
            ],
        )
        .await
        .unwrap();
    state
        .channels
        .create_channel_with_rules(
            ChannelInput::new("https://other.example.net/feed"),
            vec![rule("Lycoris Recoil", RuleState::Archived)],
        )
        .await
        .unwrap();

    let (_, body) = get(&state, &format!("/library/works/{id}")).await;
    let rules = body["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 2, "{body}");
    assert_eq!(rules[0]["directory"], "Lycoris Recoil/Season 01");
    assert_eq!(rules[0]["save_path"], "/c/Lycoris Recoil/Season 01");
    assert_eq!(rules[0]["state"], "active");
    assert_eq!(rules[0]["match"], "Lycoris");
    assert_eq!(rules[0]["channel"]["name"], "SubsPlease");
    assert_eq!(rules[0]["channel"]["host"], "feeds.example.org");
    assert_eq!(rules[1]["directory"], "Lycoris Recoil");
    assert_eq!(rules[1]["state"], "archived");
    assert_eq!(rules[1]["channel"]["name"], Value::Null);
    assert_eq!(rules[1]["channel"]["host"], "other.example.net");
}

#[tokio::test]
async fn a_work_in_the_archive_folder_has_its_rules_and_one_elsewhere_has_none() {
    let state = state();
    let scan = |name: &str| Scan {
        works: vec![WorkRead::Read(ScannedWork {
            dir_name: name.to_owned(),
            ..lycoris()
        })],
    };
    state
        .library
        .add_folder("/a".into(), scan("Lycoris Recoil"), 100, &[])
        .await
        .unwrap();
    state
        .library
        .add_folder(
            "/elsewhere".into(),
            scan("Other"),
            100,
            &state.library.folders().await.unwrap(),
        )
        .await
        .unwrap();
    state
        .settings
        .put_collection(0, "/c".into(), Some("/a/".into()))
        .await
        .unwrap();
    state
        .channels
        .create_channel_with_rules(
            ChannelInput::new("https://feeds.example.org/rss"),
            vec![
                rule("Lycoris Recoil", RuleState::Archived),
                rule("Other", RuleState::Active),
            ],
        )
        .await
        .unwrap();

    for work in state.library.overview().await.unwrap() {
        let (_, body) = get(&state, &format!("/library/works/{}", work.id)).await;
        let rules = body["rules"].as_array().unwrap();
        match work.dir_name.as_str() {
            "Lycoris Recoil" => {
                assert_eq!(rules.len(), 1);
                assert_eq!(rules[0]["state"], "archived");
            }
            _ => assert!(rules.is_empty(), "{body}"),
        }
    }
}

#[tokio::test]
async fn without_a_collect_folder_no_rule_belongs_to_a_work() {
    let (state, id) = state_with_work().await;
    state
        .channels
        .create_channel_with_rules(
            ChannelInput::new("https://feeds.example.org/rss"),
            vec![rule("Lycoris Recoil", RuleState::Active)],
        )
        .await
        .unwrap();
    let (_, body) = get(&state, &format!("/library/works/{id}")).await;
    assert_eq!(body["rules"], serde_json::json!([]));
}

async fn post(state: &AppState, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let response = api::router()
        .with_state(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A done job of the work that stored `Show - <n>.ass` for each episode `n`
/// of season 1 and applied none; the stored subtitles' IDs are `s<n>`.
async fn stored_only(state: &AppState, work: &str, episodes: &[i64]) {
    let work = work.to_owned();
    let episodes = episodes.to_vec();
    state
        .jobs
        .db()
        .run(move |c| {
            c.execute_batch(&format!(
                "INSERT INTO subtitle_jobs (id, command_id, request, origin, work_id, season, state,
                                            created_at, updated_at, state_at)
                     VALUES ('j1', 'c1', '{{}}', 'pick', '{work}', 1, 'done', 0, 0, 0);
                 INSERT INTO subtitle_job_items (id, job_id, position, episode, post_url, found_at,
                                                 state, updated_at)
                     VALUES (1, 'j1', 0, '2', 'https://example.org/p', 0, 'done', 0);
                 INSERT INTO subtitle_packages (id, work_id, job_id, source_kind, created_at)
                     VALUES ('p1', '{work}', 'j1', 'post', 0);"
            ))?;
            for (position, n) in episodes.iter().enumerate() {
                let sha = format!("{n:064}");
                c.execute_batch(&format!(
                    "INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state,
                                                     size, sha256, created_at, updated_at)
                         VALUES ('f{n}', 'j1', 1, 'k{n}', 'Show - {n:02}.ass', 'done', 1, '{sha}',
                                 0, 0);
                     INSERT INTO subtitle_assets (id, work_id, kind, base, relative_path,
                                                  byte_size, sha256, created_at)
                         VALUES ('a{n}', '{work}', 'subtitle', 'work',
                                 '.trss/subtitles/하느/Show - {n:02}.ass', 1, '{sha}', 0);
                     INSERT INTO subtitle_stored (id, work_id, season, package_id,
                                                  subtitle_asset_id, assignment, episode, format,
                                                  creator, stored_at)
                         VALUES ('s{n}', '{work}', 1, 'p1', 'a{n}', 'explicit', {n}, 'ass', '하느',
                                 {n});
                     INSERT INTO subtitle_job_plan (job_id, position, file_id, name, kind, format,
                                                    size, sha256, item_id, assignment, episode,
                                                    action, stored_id, outcome, note, updated_at)
                         VALUES ('j1', {position}, 'f{n}', 'Show - {n:02}.ass', 'subtitle', 'ass',
                                 1, '{sha}', 1, 'explicit', {n}, 'store', 's{n}', 'stored',
                                 '고르지 않은 회차라 보관만 해요', 0);"
                ))?;
            }
            Ok::<_, trss_jobs::JobError>(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn an_episode_lists_its_stored_only_subtitles_and_one_is_applied_on_request() {
    let (state, id) = state_with_work().await;
    // Episode 1 has a subtitle, 2 has a video only, 5 has no file.
    stored_only(&state, &id, &[1, 2, 5]).await;
    let (status, body) = get(&state, &format!("/library/works/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let episodes = body["seasons"][0]["episodes"].as_array().unwrap();
    let of = |e: &str| episodes.iter().find(|x| x["episode"] == e).unwrap();
    assert_eq!(of("02")["stored"][0]["name"], "Show - 02.ass");
    assert_eq!(of("02")["stored"][0]["creator"], "하느");
    assert_eq!(of("02")["stored"][0]["can_apply"], true);
    // An episode with stored subtitles only has a row with no files.
    assert_eq!(of("05")["video"], serde_json::json!([]));
    assert_eq!(of("05")["stored"][0]["id"], "s5");
    let numbers: Vec<f64> = episodes
        .iter()
        .map(|e| e["sort"].as_f64().unwrap())
        .collect();
    assert_eq!(numbers, [1.0, 2.0, 5.0]);

    let (status, body) = post(&state, &format!("/library/works/{id}/stored/s2/apply")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["job_id"], "j1");
    let plan = state.jobs.plan("j1").await.unwrap();
    let row = plan
        .iter()
        .find(|r| r.stored_id.as_deref() == Some("s2"))
        .unwrap();
    assert_eq!(row.action, trss_jobs::model::PlanAction::Apply);
    assert_eq!(row.outcome, None);
    let job = state.jobs.detail("j1").await.unwrap().unwrap();
    assert_eq!(job.row.state, trss_jobs::JobState::Pending);

    // An episode with a subtitle is a replacement's, and an unknown stored
    // subtitle is no one's.
    let (status, body) = post(&state, &format!("/library/works/{id}/stored/s1/apply")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["message"].as_str().unwrap().contains("자막이 있어요"));
    let (status, _) = post(&state, &format!("/library/works/{id}/stored/nope/apply")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = post(&state, "/library/works/other/stored/s5/apply").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_stored_subtitle_waiting_for_its_video_says_so() {
    let (state, id) = state_with_work().await;
    // Episode 5 has no file: its row waits for the video.
    stored_only(&state, &id, &[2, 5]).await;
    state
        .jobs
        .db()
        .run(|c| {
            c.execute_batch(
                "UPDATE subtitle_job_plan SET action = 'apply', outcome = 'no_video'
                  WHERE stored_id = 's5';
                 UPDATE subtitle_jobs SET state = 'waiting', wait = 'video' WHERE id = 'j1';",
            )?;
            Ok::<_, trss_jobs::JobError>(())
        })
        .await
        .unwrap();
    let (status, body) = get(&state, &format!("/library/works/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let episodes = body["seasons"][0]["episodes"].as_array().unwrap();
    let of = |e: &str| episodes.iter().find(|x| x["episode"] == e).unwrap();
    assert_eq!(of("05")["stored"][0]["awaiting_video"], true);
    assert_eq!(of("02")["stored"][0]["awaiting_video"], false);
}

#[tokio::test]
async fn a_stored_subtitle_waiting_for_a_replacement_names_its_job() {
    let (state, id) = state_with_work().await;
    // Episode 1 has a subtitle: the job's plan to replace it waits for the
    // user (`교체 승인`).
    stored_only(&state, &id, &[1, 2]).await;
    let work = id.clone();
    state
        .jobs
        .db()
        .run(move |c| {
            c.execute_batch(&format!(
                "UPDATE subtitle_job_plan SET action = 'apply', outcome = NULL
                  WHERE stored_id = 's1';
                 UPDATE subtitle_jobs SET state = 'waiting', wait = 'approval' WHERE id = 'j1';
                 INSERT INTO subtitle_replacements
                     (id, job_id, position, version, state, work_id, season, episode, assignment,
                      folder, video_path, video_object, video_size, video_mtime, stored_id,
                      asset_id, asset_path, asset_size, asset_sha256, target, created_at,
                      updated_at)
                     VALUES ('r1', 'j1', 0, 1, 'open', '{work}', 1, 1, 'explicit', '/media/Show',
                             'S01E01.mkv', '1:2:3', 5, 7, 's1', 'a1',
                             '.trss/subtitles/하느/Show - 01.ass', 1, printf('%064d', 1),
                             'S01E01.ass', 0, 0);"
            ))?;
            Ok::<_, trss_jobs::JobError>(())
        })
        .await
        .unwrap();
    let (status, body) = get(&state, &format!("/library/works/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let episodes = body["seasons"][0]["episodes"].as_array().unwrap();
    let of = |e: &str| episodes.iter().find(|x| x["episode"] == e).unwrap();
    assert_eq!(of("01")["stored"][0]["approval_job"], "j1");
    assert_eq!(of("02")["stored"][0]["approval_job"], Value::Null);
}
