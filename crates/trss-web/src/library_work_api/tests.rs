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
            check: None,
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
    assert_eq!(leftovers[0]["checked"], false);
    assert_eq!(body["rules"], serde_json::json!([]));
}

/// The rule is `trss_core::episode`'s; this is where the answer carries it: a
/// season's runs, and the episode texts the screen shows.
#[tokio::test]
async fn a_season_tells_its_episodes_with_a_video_and_with_a_subtitle_as_runs() {
    let state = state();
    let video = |e: &str| file(1, e, &format!("S01E{e}.mkv"), FileKind::Video);
    let subtitle = |e: &str| file(1, e, &format!("S01E{e}.ass"), FileKind::Subtitle);
    let work = ScannedWork {
        dir_name: "Show".into(),
        seasons: BTreeSet::from([1]),
        files: vec![
            video("01"),
            video("02"),
            video("03"),
            video("05"),
            video("SP"),
            subtitle("01"),
            subtitle("02"),
        ],
        unrecognized: vec![],
    };
    state
        .library
        .add_folder(
            "/c".into(),
            Scan {
                works: vec![WorkRead::Read(work)],
            },
            100,
            &[],
        )
        .await
        .unwrap();
    let id = state.library.overview().await.unwrap()[0].id.clone();

    let (status, body) = get(&state, &format!("/library/works/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let season = &body["seasons"][0];
    assert_eq!(
        season["video_ranges"],
        serde_json::json!(["1–3", "5", "SP"])
    );
    assert_eq!(season["subtitle_ranges"], serde_json::json!(["1–2"]));
    let shown: Vec<(&str, &str)> = season["episodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["episode"].as_str().unwrap(),
                e["episode_shown"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("01", "1"),
            ("02", "2"),
            ("03", "3"),
            ("05", "5"),
            ("SP", "SP")
        ]
    );
}

/// A season without a file of a kind has no runs of it, and the rows the
/// answer adds for a stored subtitle and its copy are written as shown too.
#[tokio::test]
async fn a_season_with_no_subtitle_has_no_runs_of_it_and_stored_rows_and_copies_are_shown() {
    let (state, id) = state_with_work().await;
    stored_only(&state, &id, &[5]).await;
    let (status, body) = get(&state, &format!("/library/works/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["seasons"][1]["video_ranges"], serde_json::json!(["1"]));
    assert_eq!(body["seasons"][1]["subtitle_ranges"], serde_json::json!([]));
    let episodes = body["seasons"][0]["episodes"].as_array().unwrap();
    let stored_row = episodes.iter().find(|e| e["episode"] == "05").unwrap();
    assert_eq!(stored_row["episode_shown"], "5");
    // A row with no files adds nothing to the runs.
    assert_eq!(
        body["seasons"][0]["video_ranges"],
        serde_json::json!(["1–2"])
    );
    let copy = copy(&body, "s5");
    assert_eq!(
        (copy["episode"].as_str(), copy["episode_shown"].as_str()),
        (Some("05"), Some("5"))
    );
}

/// The wire shape of a subtitle file's `applied`: `null` for a file a person
/// put there, `{ "creator": … }` for a copy the app applied (the rule that makes
/// it so is the library's).
#[tokio::test]
async fn a_subtitle_file_tells_whether_it_is_an_applied_copy_and_of_whom() {
    let (state, id) = state_with_work().await;
    let uri = format!("/library/works/{id}");
    let (_, body) = get(&state, &uri).await;
    let file = &body["seasons"][0]["episodes"][0]["subtitle"][0];
    assert_eq!(file["creator"], Value::Null);
    assert_eq!(file["applied"], Value::Null);

    stored_only(&state, &id, &[1]).await;
    let work = id.clone();
    state
        .db()
        .run(move |c| {
            c.execute(
                "INSERT INTO subtitle_applied (id, work_id, stored_id, season, episode, video_path,
                                               path, byte_size, sha256, object, job_id, applied_at)
                     VALUES ('ap1', ?1, 's1', 1, 1, 'Season 01/S01E01.mkv',
                             'Season 01/S01E01.ko.ass', 1, printf('%064d', 1), '1:1', 'j1', 400)",
                [work],
            )?;
            Ok::<_, trss_jobs::JobError>(())
        })
        .await
        .unwrap();
    let (_, body) = get(&state, &uri).await;
    let file = &body["seasons"][0]["episodes"][0]["subtitle"][0];
    assert_eq!(file["creator"], Value::Null);
    assert_eq!(file["applied"], serde_json::json!({ "creator": "하느" }));
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
    // Applying it is a comparison where the episode has a subtitle.
    assert_eq!(of("01")["stored"][0]["compare"], true);
    assert_eq!(of("02")["stored"][0]["compare"], false);
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
    assert_eq!(
        (body["job_id"].as_str(), body["compare"].as_bool()),
        (Some("j1"), Some(false))
    );

    // An episode with a subtitle is a replacement the person compares, and an
    // unknown stored subtitle is no one's.
    let (status, body) = post(&state, &format!("/library/works/{id}/stored/s1/apply")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(
        (body["job_id"].as_str(), body["compare"].as_bool()),
        (Some("j1"), Some(true))
    );
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

async fn post_json(state: &AppState, uri: &str, body: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
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

/// [`state_with_work`] with the watch folder, the work folder and its
/// `.trss/subtitles` on disk, as a cleanup needs them.
async fn state_with_work_on_disk() -> (AppState, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("Lycoris Recoil/.trss/subtitles")).unwrap();
    let state = state();
    state
        .library
        .add_folder(
            dir.path().to_string_lossy().into_owned(),
            Scan {
                works: vec![WorkRead::Read(lycoris())],
            },
            100,
            &[],
        )
        .await
        .unwrap();
    let id = state.library.overview().await.unwrap()[0].id.clone();
    (state, id, dir)
}

#[tokio::test]
async fn a_work_shows_its_stored_files_and_one_is_cleaned_on_request() {
    let (state, id, _dir) = state_with_work_on_disk().await;
    stored_only(&state, &id, &[2, 5]).await;
    let (status, body) = get(&state, &format!("/library/works/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["storage"]["total"], 2);
    assert_eq!(body["storage"]["cleaning"], serde_json::json!([]));
    assert_eq!(
        body["storage"]["cleanable"][0],
        serde_json::json!({
            "id": "s2", "name": "Show - 02.ass", "season": 1, "episode": 2,
            "creator": "하느", "format": "ass", "size": 1, "stored_at": 2,
            "kind": "stored", "blocked": null,
            "with": [{ "id": "a2", "name": "Show - 02.ass", "kind": "subtitle", "size": 1 }],
            "kept": []
        })
    );
    assert_eq!(body["storage"]["cleanable"][1]["id"], "s5");

    // A body that is not the dialog's is refused before anything happens.
    let clean = format!("/library/works/{id}/stored/s2/clean");
    let (status, _) = post_json(&state, &clean, "{\"files\": []}").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Files that are not the ones that would go now: the entry as it is.
    let (status, body) = post_json(&state, &clean, "{\"assets\": []}").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["message"], FILES_CHANGED);
    assert_eq!(body["current"]["id"], "s2");
    assert_eq!(body["current"]["with"][0]["id"], "a2");

    let (status, body) = post_json(&state, &clean, "{\"assets\": [\"a2\"]}").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let cleanup = body["cleanup_id"].as_str().unwrap().to_owned();

    // It is no stored copy from then: the episode and the list lose it,
    // and the cleanup waits for the worker.
    let (_, body) = get(&state, &format!("/library/works/{id}")).await;
    let cleanable = body["storage"]["cleanable"].as_array().unwrap();
    assert_eq!(cleanable.len(), 1);
    assert_eq!(cleanable[0]["id"], "s5");
    assert_eq!(
        body["storage"]["cleaning"],
        serde_json::json!([{
            "id": cleanup, "name": "Show - 02.ass", "state": "asked", "reason": null
        }])
    );
    let episodes = body["seasons"][0]["episodes"].as_array().unwrap();
    let two = episodes.iter().find(|e| e["episode"] == "02").unwrap();
    assert_eq!(two["stored"], serde_json::json!([]));

    // From another work: no such stored copy.
    let (status, _) = post_json(
        &state,
        "/library/works/other/stored/s5/clean",
        "{\"assets\": [\"a5\"]}",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_stored_subtitle_a_held_job_uses_says_why_it_cannot_be_cleaned() {
    let (state, id, _dir) = state_with_work_on_disk().await;
    stored_only(&state, &id, &[2]).await;
    state
        .db()
        .run(|c| {
            c.execute_batch("UPDATE subtitle_jobs SET state = 'held' WHERE id = 'j1';")?;
            Ok::<_, trss_jobs::JobError>(())
        })
        .await
        .unwrap();
    let (_, body) = get(&state, &format!("/library/works/{id}")).await;
    let entry = &body["storage"]["cleanable"][0];
    assert_eq!(entry["blocked"], trss_jobs::place::cleanup::HELD_JOB);

    let (status, body) = post_json(
        &state,
        &format!("/library/works/{id}/stored/s2/clean"),
        "{\"assets\": [\"a2\"]}",
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["message"], trss_jobs::place::cleanup::HELD_JOB);
    assert_eq!(body["current"], Value::Null);
}

// 완료 기준: 설정의 파일 용량과 정리는 작품별 종류·표지 용량과 정리할 수
// 있는 수를 보여줘요.
#[tokio::test]
async fn the_storage_list_shows_each_works_kinds_cover_and_cleanable_count() {
    use trss_anilist::AnilistConfig;
    use trss_library::artwork::{AppData, Artwork};

    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("trss.db")).await.unwrap();
    let state = AppState::new(db.clone()).with_artwork(Artwork::new(
        db,
        Some(AppData::new(dir.path())),
        AnilistConfig::default(),
    ));
    let other = ScannedWork {
        dir_name: "Other".into(),
        seasons: BTreeSet::from([1]),
        files: Vec::new(),
        unrecognized: Vec::new(),
    };
    let empty = ScannedWork {
        dir_name: "Empty".into(),
        ..other.clone()
    };
    let shows = dir.path().join("shows");
    for name in ["Lycoris Recoil", "Other", "Empty"] {
        std::fs::create_dir_all(shows.join(name).join(".trss/subtitles")).unwrap();
    }
    state
        .library
        .add_folder(
            shows.to_string_lossy().into_owned(),
            Scan {
                works: vec![
                    WorkRead::Read(lycoris()),
                    WorkRead::Read(other),
                    WorkRead::Read(empty),
                ],
            },
            100,
            &[],
        )
        .await
        .unwrap();
    let works = state.library.overview().await.unwrap();
    let id_of = |name: &str| {
        works
            .iter()
            .find(|w| w.dir_name == name)
            .unwrap()
            .id
            .clone()
    };
    let (lycoris, other) = (id_of("Lycoris Recoil"), id_of("Other"));
    stored_only(&state, &lycoris, &[2, 5]).await;
    // Episode 5's copy waits for its video, which a cleanup may settle.
    let covers = [
        (lycoris.clone(), "covers/l.jpg"),
        (other.clone(), "covers/o.jpg"),
    ];
    state
        .db()
        .run(move |c| {
            c.execute_batch(
                "UPDATE subtitle_job_plan SET action = 'apply', outcome = 'no_video'
                  WHERE stored_id = 's5';
                 UPDATE subtitle_jobs SET state = 'waiting', wait = 'video' WHERE id = 'j1';",
            )?;
            for (work, path) in &covers {
                c.execute(
                    "UPDATE work_artwork
                        SET mode = 'manual', source = 'upload', image_id = ?2,
                            image_origin = 'upload', image_path = ?3, image_size = 10,
                            image_sha256 = printf('%064d', 0), image_format = 'jpeg',
                            job = NULL, job_requested_at = NULL
                      WHERE work_id = ?1",
                    rusqlite::params![work, format!("i-{work}"), path],
                )?;
            }
            Ok::<_, trss_jobs::JobError>(())
        })
        .await
        .unwrap();
    // Lycoris Recoil's cover is there; Other's is not.
    std::fs::create_dir_all(dir.path().join("covers")).unwrap();
    std::fs::write(dir.path().join("covers/l.jpg"), [0u8; 10]).unwrap();

    let (status, body) = get(&state, "/library/storage").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        serde_json::json!({ "works": [
            {
                "id": lycoris, "name": "Lycoris Recoil", "total": 12,
                "kinds": [
                    { "kind": "subtitle", "count": 2, "size": 2 },
                    { "kind": "cover", "count": 1, "size": 10 }
                ],
                "cleanable": 2
            },
            {
                "id": other, "name": "Other", "total": 0,
                "kinds": [{ "kind": "cover", "count": 1, "size": 0 }],
                "cleanable": 0
            }
        ]})
    );
}

async fn send(
    state: &AppState,
    method: Method,
    uri: &str,
    body: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(uri);
    if body.is_some() {
        request = request.header("content-type", "application/json");
    }
    let request = request
        .body(Body::from(body.unwrap_or_default().to_owned()))
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

/// What a person chose of the stored subtitle's row.
async fn chosen_of(state: &AppState, stored: &str) -> Option<trss_jobs::model::Chosen> {
    state
        .place
        .plan("j1")
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.stored_id.as_deref() == Some(stored))
        .unwrap()
        .chosen
}

#[tokio::test]
async fn a_stored_subtitle_is_chosen_with_a_mode_and_a_bad_one_changes_nothing() {
    use trss_jobs::model::Chosen;
    let (state, id) = state_with_work().await;
    // Episode 1 has a subtitle, 2 has a video only, 5 has no file.
    stored_only(&state, &id, &[1, 2, 5]).await;
    let apply = |stored: &str| format!("/library/works/{id}/stored/{stored}/apply");

    // A mode that is none, or a body that is not one, is refused.
    for body in [
        r#"{"mode":"replace"}"#,
        r#"{"mode":3}"#,
        "mode=add",
        "[]",
        r#"{"mode":""}"#,
    ] {
        let (status, refused) = send(&state, Method::POST, &apply("s1"), Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {refused}");
        assert!(refused["message"].as_str().is_some_and(|m| !m.is_empty()));
    }
    assert_eq!(chosen_of(&state, "s1").await, None);
    assert_eq!(
        state.jobs.detail("j1").await.unwrap().unwrap().row.state,
        trss_jobs::JobState::Done
    );

    // `apply` by name, by `{}` and by no body at all.
    let (status, body) = send(
        &state,
        Method::POST,
        &apply("s1"),
        Some(r#"{"mode":"apply"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["compare"], true);
    assert_eq!(chosen_of(&state, "s1").await, Some(Chosen::Apply));
    let (status, body) = send(&state, Method::POST, &apply("s2"), Some("{}")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["compare"], false);
    let (status, body) = send(&state, Method::POST, &apply("s5"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["compare"], false);
    assert_eq!(chosen_of(&state, "s5").await, Some(Chosen::Apply));

    // Nothing of those episodes is applied, so there is nothing to add to.
    let (status, body) = send(
        &state,
        Method::POST,
        &apply("s2"),
        Some(r#"{"mode":"add"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["message"],
        "추가로 적용할 수 있는 것은 이 회차에 적용한 제작자의 다른 형식이에요."
    );
    let (status, _) = send(
        &state,
        Method::POST,
        &apply("nope"),
        Some(r#"{"mode":"add"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// More stored subtitles on [`stored_only`]'s episode 2, beside an applied
/// copy of `s2` (`Season 01/S01E02.ass`): `s2s` (the same creator's SRT), `x2`
/// (another creator's ASS), `n2` (an ASS of no creator) and `o2` (a format the
/// app does not apply), each received by job `j1`.
async fn more_on_episode_two(state: &AppState, work: &str) {
    let work = work.to_owned();
    state
        .db()
        .run(move |c| {
            c.execute_batch(&format!(
                "INSERT INTO subtitle_applied (id, work_id, stored_id, season, episode, video_path,
                                               path, byte_size, sha256, object, job_id, applied_at)
                     VALUES ('ap2', '{work}', 's2', 1, 2, 'Season 01/S01E02.mkv',
                             'Season 01/S01E02.ass', 1, printf('%064d', 2), '1:2', 'j1', 400);"
            ))?;
            let more = [
                ("s2s", "srt", "하느", "Show - 02.srt", 7),
                ("x2", "ass", "가나", "Show - 02 [x].ass", 9),
                ("n2", "ass", "", "Show - 02 [n].ass", 8),
                ("o2", "ssa", "하느", "Show - 02.ssa", 6),
            ];
            for (position, (id, format, creator, name, at)) in more.iter().enumerate() {
                let (position, sha) = (position + 10, format!("{:064}", position + 10));
                let creator = match creator.is_empty() {
                    true => "NULL".to_owned(),
                    false => format!("'{creator}'"),
                };
                let format = if *format == "ssa" { "other" } else { format };
                c.execute_batch(&format!(
                    "INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state,
                                                     size, sha256, created_at, updated_at)
                         VALUES ('f{id}', 'j1', 1, 'k{id}', '{name}', 'done', 1, '{sha}', 0, 0);
                     INSERT INTO subtitle_assets (id, work_id, kind, base, relative_path,
                                                  byte_size, sha256, created_at)
                         VALUES ('a{id}', '{work}', 'subtitle', 'work',
                                 '.trss/subtitles/x/{name}', 1, '{sha}', 0);
                     INSERT INTO subtitle_stored (id, work_id, season, package_id,
                                                  subtitle_asset_id, assignment, episode, format,
                                                  creator, stored_at)
                         VALUES ('{id}', '{work}', 1, 'p1', 'a{id}', 'explicit', 2, '{format}',
                                 {creator}, {at});
                     INSERT INTO subtitle_job_plan (job_id, position, file_id, name, kind, format,
                                                    size, sha256, item_id, assignment, episode,
                                                    action, stored_id, outcome, note, updated_at)
                         VALUES ('j1', {position}, 'f{id}', '{name}', 'subtitle', '{format}',
                                 1, '{sha}', 1, 'explicit', 2, 'store', '{id}', 'stored',
                                 '고르지 않은 회차라 보관만 해요', 0);"
                ))?;
            }
            Ok::<_, trss_jobs::JobError>(())
        })
        .await
        .unwrap();
}

/// A copy of the card's creators by its `id`.
fn copy<'a>(body: &'a Value, id: &str) -> &'a Value {
    body["subtitles"]["creators"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["copies"].as_array().unwrap())
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no copy {id}"))
}

#[tokio::test]
async fn the_subtitles_card_groups_stored_copies_by_creator_with_what_choosing_each_does() {
    let (state, id) = state_with_work().await;
    // Episode 1 has a subtitle file, 2 an applied copy, 5 nothing.
    stored_only(&state, &id, &[1, 2, 5]).await;
    more_on_episode_two(&state, &id).await;
    let uri = format!("/library/works/{id}");
    let (status, body) = get(&state, &uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let card = &body["subtitles"];
    assert_eq!(
        card["format_order"],
        serde_json::json!({ "order": ["ass", "srt", "smi"], "own": false })
    );

    // Each creator's copies are in a list of its own, the creators by name
    // with the one of no name last (tested in trss-jobs).
    let creators = card["creators"].as_array().unwrap();
    assert_eq!(creators.len(), 3);
    assert!(creators.iter().all(|c| c["copies"].is_array()));

    // The applied copy: where it is, and nothing to choose.
    let applied = copy(&body, "s2");
    assert_eq!(applied["season"], 1);
    assert_eq!(applied["episode"], "02");
    assert_eq!(applied["name"], "Show - 02.ass");
    assert_eq!(applied["format"], "ass");
    assert_eq!(applied["stored_at"], 2);
    assert_eq!(applied["stored_path"], ".trss/subtitles/하느/Show - 02.ass");
    assert_eq!(
        applied["applied"],
        serde_json::json!([{ "path": "Season 01/S01E02.ass", "applied_at": 400 }])
    );
    assert_eq!(
        (&applied["choice"], &applied["can_add"], &applied["blocked"]),
        (&Value::Null, &Value::Bool(false), &Value::Null)
    );
    // The creator's SRT on the episode with an applied copy: compared, and
    // added beside it.
    let srt = copy(&body, "s2s");
    assert_eq!(srt["applied"], serde_json::json!([]));
    assert_eq!(
        (&srt["choice"], &srt["can_add"], &srt["blocked"]),
        (&Value::from("compare"), &Value::Bool(true), &Value::Null)
    );
    // An episode with a subtitle file, and one with none.
    assert_eq!(copy(&body, "s1")["choice"], "compare");
    assert_eq!(copy(&body, "s5")["choice"], "apply");
    assert_eq!(copy(&body, "s5")["can_add"], false);
    // A format the app does not apply says why.
    let other = copy(&body, "o2");
    assert_eq!(other["format"], "other");
    assert_eq!(other["choice"], Value::Null);
    assert_eq!(other["can_add"], false);
    assert_eq!(
        other["blocked"],
        "자동으로 적용하지 않는 형식이라 적용할 수 없어요."
    );
}

/// A state with watch folder `/c` holding two works: `Lycoris Recoil` and
/// `Show`.
async fn state_with_two_works() -> (AppState, String, String) {
    let state = state();
    let show = ScannedWork {
        dir_name: "Show".into(),
        seasons: BTreeSet::from([1]),
        files: vec![file(1, "01", "S01E01.mkv", FileKind::Video)],
        unrecognized: vec![],
    };
    state
        .library
        .add_folder(
            "/c".into(),
            Scan {
                works: vec![WorkRead::Read(lycoris()), WorkRead::Read(show)],
            },
            100,
            &[],
        )
        .await
        .unwrap();
    let works = state.library.overview().await.unwrap();
    let id = |name: &str| {
        works
            .iter()
            .find(|w| w.dir_name == name)
            .unwrap()
            .id
            .clone()
    };
    (state.clone(), id("Lycoris Recoil"), id("Show"))
}

#[tokio::test]
async fn a_works_own_format_order_is_set_listed_in_the_policy_and_taken_away() {
    let (state, lycoris, show) = state_with_two_works().await;
    let order_uri = |work: &str| format!("/library/works/{work}/subtitle-order");
    let orders = |body: &Value| -> Vec<(String, Value)> {
        body["overrides"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| {
                (
                    o["name"].as_str().unwrap().to_owned(),
                    o["format_order"].clone(),
                )
            })
            .collect()
    };
    let (_, policy) = get(&state, "/settings/policy").await;
    assert!(orders(&policy).is_empty());

    // A work's own order, which the screens and the settings list show.
    let (status, body) = send(
        &state,
        Method::PUT,
        &order_uri(&lycoris),
        Some(r#"{"format_order":["srt","ass","smi"]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        serde_json::json!({ "order": ["srt", "ass", "smi"], "own": true })
    );
    let (_, policy) = get(&state, "/settings/policy").await;
    assert_eq!(
        orders(&policy),
        [(
            "Lycoris Recoil".to_owned(),
            serde_json::json!(["srt", "ass", "smi"])
        )]
    );
    let (_, detail) = get(&state, &format!("/library/works/{lycoris}")).await;
    assert_eq!(
        detail["subtitles"]["format_order"],
        serde_json::json!({ "order": ["srt", "ass", "smi"], "own": true })
    );
    // Another work keeps the common order.
    let (_, other) = get(&state, &format!("/library/works/{show}")).await;
    assert_eq!(
        other["subtitles"]["format_order"],
        serde_json::json!({ "order": ["ass", "srt", "smi"], "own": false })
    );

    // An order that is none, and a body that is none, are refused with a
    // sentence, and the order stays.
    for bad in [
        r#"{"format_order":["srt","srt","smi"]}"#,
        r#"{"format_order":"srt,ass,smi"}"#,
        r#"{}"#,
    ] {
        let (status, refused) = send(&state, Method::PUT, &order_uri(&lycoris), Some(bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {refused}");
        assert!(
            refused["message"].as_str().is_some_and(|m| !m.is_empty()),
            "{bad}"
        );
    }
    let (_, policy) = get(&state, "/settings/policy").await;
    assert_eq!(
        orders(&policy),
        [(
            "Lycoris Recoil".to_owned(),
            serde_json::json!(["srt", "ass", "smi"])
        )]
    );

    // Going back: the answer is the common order as it is now, and the work
    // leaves the settings' list.
    state
        .settings
        .put_policy(
            0,
            trss_core::settings::policy::FormatOrder::from_codes(&["srt", "smi", "ass"]).unwrap(),
            300,
            1,
            50,
        )
        .await
        .unwrap();
    let (status, body) = send(&state, Method::DELETE, &order_uri(&lycoris), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        serde_json::json!({ "order": ["srt", "smi", "ass"], "own": false })
    );
    let (_, policy) = get(&state, "/settings/policy").await;
    assert!(orders(&policy).is_empty());
    let (_, detail) = get(&state, &format!("/library/works/{lycoris}")).await;
    assert_eq!(
        detail["subtitles"]["format_order"],
        serde_json::json!({ "order": ["srt", "smi", "ass"], "own": false })
    );
    // Again is no change.
    let (status, _) = send(&state, Method::DELETE, &order_uri(&lycoris), None).await;
    assert_eq!(status, StatusCode::OK);

    // No such work.
    let (status, _) = send(
        &state,
        Method::PUT,
        &order_uri("nope"),
        Some(r#"{"format_order":["srt","ass","smi"]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(&state, Method::DELETE, &order_uri("nope"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// --- the replacements of a work's episodes -------------------------------------------

mod replacements {
    use super::*;
    use trss_collect::store::{
        history::{HistoryResult, Observation},
        revisions::{NewRevision, OldVideo, RevisionState, Step},
    };

    const SEASON_FOLDER: &str = "/c/Lycoris Recoil/Season 01";

    /// A replacement row of the episode file `episode_name` in `folder`, in
    /// the state a store test would leave it in: a rule of the work, a history
    /// item received for it, and the row (receiving).
    async fn row(state: &AppState, folder: &str, episode_name: &str, n: u32) -> i64 {
        let channel = state
            .channels
            .create_channel_with_rules(
                ChannelInput::new(format!("https://feeds.example.org/rss?n={n}")),
                vec![rule("Lycoris Recoil/Season 01", RuleState::Active)],
            )
            .await
            .unwrap();
        let rule_id = channel.rules[0].id.clone();
        let hash = format!("{n:040x}");
        state
            .history
            .record(
                1,
                vec![Observation {
                    channel_id: channel.channel.id.clone(),
                    channel_label: "https://feeds.example.org/rss".into(),
                    identity_key: format!("guid:{n}"),
                    title: format!("[SubsPlease] Lycoris Recoil - 0{n}v2 (1080p).mkv"),
                    link: format!("magnet:?xt=urn:btih:{hash}"),
                    result: HistoryResult::Received,
                    rule_id: Some(rule_id.clone()),
                    torrent_hash: Some(hash.clone()),
                    reason: None,
                }],
            )
            .await
            .unwrap();
        let item = state
            .history
            .item_by_key(channel.channel.id, format!("guid:{n}"))
            .await
            .unwrap()
            .unwrap();
        state
            .revisions
            .create(
                10,
                NewRevision {
                    item_id: item.id,
                    old_item_id: None,
                    rule_id,
                    folder: folder.into(),
                    episode_name: episode_name.into(),
                    old_version: Some(1),
                    new_version: 2,
                    old_crc: None,
                    expected_crc: Some("8F2EFECC".into()),
                    torrent_hash: Some(hash),
                    state: RevisionState::Receiving,
                    reason: None,
                },
            )
            .await
            .unwrap()
            .id
    }

    /// The replacement is done at `at`.
    async fn done(state: &AppState, id: i64, at: i64) {
        let store = &state.revisions;
        let step = Step::Verified {
            received_name: "v.mkv".into(),
            file_crc: "8F2EFECC".into(),
            file_identity: "1:2:3:4:5:6:7".into(),
        };
        store
            .advance(id, 11, RevisionState::Receiving, step)
            .await
            .unwrap();
        let old = OldVideo {
            item_id: None,
            version: Some(1),
            torrent_hash: None,
        };
        store.claim(id, 12, old).await.unwrap();
        store
            .advance(
                id,
                13,
                RevisionState::Removing,
                Step::Removed { reason: None },
            )
            .await
            .unwrap();
        store
            .advance(id, at, RevisionState::Removed, Step::Done)
            .await
            .unwrap();
    }

    /// The replacement failed (the new file is in the folder under its
    /// received name).
    async fn failed(state: &AppState, id: i64) {
        let step = Step::Failed {
            reason: "받은 파일의 CRC32가 이름과 달라요.".into(),
            received_name: Some("[SubsPlease] Lycoris Recoil - 02v2 (1080p).mkv".into()),
        };
        state
            .revisions
            .advance(id, 20, RevisionState::Receiving, step)
            .await
            .unwrap();
    }

    fn episode<'a>(body: &'a Value, episode: &str) -> &'a Value {
        body["seasons"][0]["episodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["episode"] == episode)
            .unwrap_or_else(|| panic!("no episode {episode}: {body}"))
    }

    /// An episode whose video a revision replaced shows the version line, quietly:
    /// no failure.
    #[tokio::test]
    async fn an_episode_whose_video_was_replaced_shows_the_version_line() {
        let (state, id) = state_with_work().await;
        let revision = row(&state, SEASON_FOLDER, "S01E02.mkv", 2).await;
        done(&state, revision, 777).await;

        let (status, body) = get(&state, &format!("/library/works/{id}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let shown = episode(&body, "02");
        assert_eq!(
            shown["revision"],
            serde_json::json!({ "from": "v1", "to": "v2", "replaced_at": 777 })
        );
        assert_eq!(shown["failure"], Value::Null);
        assert_eq!(shown["video"][0]["path"], "Season 01/S01E02.mkv");
        assert_eq!(episode(&body, "01")["revision"], Value::Null);
    }

    /// A failed replacement is on its episode's row, with both files and why;
    /// one whose episode has no row (its old video is gone and the new one
    /// does not have the episode name yet) gets a row of its own with no
    /// files; one outside the work's season folders is left out.
    #[tokio::test]
    async fn a_failed_replacement_is_on_its_episode_row_or_on_one_of_its_own() {
        let (state, id) = state_with_work().await;
        let on_row = row(&state, SEASON_FOLDER, "S01E02.mkv", 2).await;
        failed(&state, on_row).await;
        let own = row(&state, SEASON_FOLDER, "S01E03.mkv", 3).await;
        failed(&state, own).await;
        let elsewhere = row(&state, "/c/Another Show/Season 01", "S01E01.mkv", 4).await;
        failed(&state, elsewhere).await;

        let (status, body) = get(&state, &format!("/library/works/{id}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let shown = episode(&body, "02");
        assert_eq!(shown["revision"], Value::Null);
        assert!(
            shown["failure"]["reason"]
                .as_str()
                .unwrap()
                .contains("CRC32"),
            "{shown}"
        );
        assert_eq!(
            shown["failure"]["files"],
            serde_json::json!([
                { "role": "old", "path": "Season 01/S01E02.mkv", "state": "kept" },
                {
                    "role": "new",
                    "path": "Season 01/[SubsPlease] Lycoris Recoil - 02v2 (1080p).mkv",
                    "state": "received_name",
                },
            ])
        );
        assert_eq!(shown["video"][0]["path"], "Season 01/S01E02.mkv");

        let alone = episode(&body, "03");
        assert_eq!(alone["video"], serde_json::json!([]));
        assert_eq!(episode(&body, "01")["failure"], Value::Null);
        let episodes = body["seasons"][0]["episodes"].as_array().unwrap();
        assert_eq!(episodes.len(), 3, "{body}");
    }

    /// A replacement whose download stopped offers `다시 받기` on its episode
    /// row too; one that failed after it was received does not.
    #[tokio::test]
    async fn a_stopped_replacements_retry_offer_is_on_its_episode_row() {
        let (state, id) = state_with_work().await;
        let stopped = row(&state, SEASON_FOLDER, "S01E02.mkv", 2).await;
        let step = Step::Failed {
            reason: "새 영상의 토렌트가 Transmission에서 사라져 받기가 끝나지 않았어요. 이전 영상은 그대로 있어요."
                .into(),
            received_name: None,
        };
        state
            .revisions
            .advance(stopped, 20, RevisionState::Receiving, step)
            .await
            .unwrap();
        let received = row(&state, SEASON_FOLDER, "S01E03.mkv", 3).await;
        failed(&state, received).await;

        let (status, body) = get(&state, &format!("/library/works/{id}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let offered = &episode(&body, "02")["failure"];
        assert_eq!(offered["can_retry"], true, "{offered}");
        assert_eq!(offered["retry_blocked"], Value::Null);
        assert_eq!(offered["command"], Value::Null);
        assert_eq!(offered["files"][1]["state"], "not_received");
        let not_offered = &episode(&body, "03")["failure"];
        assert_eq!(not_offered["can_retry"], false, "{not_offered}");
    }
}
