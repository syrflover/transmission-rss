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
