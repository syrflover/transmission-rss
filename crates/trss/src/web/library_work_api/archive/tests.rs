//! The wire of `보관 폴더로 옮기기`: the work detail's `archive` and
//! `POST /library/works/{id}/archive`. The move the command makes is tested in
//! trss-collect (`commands/work_archive/tests.rs`); the worker running it, in
//! trss's `tests/it/archive_move.rs`.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use crate::web::{testing, AppState};
use trss_collect::{
    commands::rule_archive::{FAILED, MOVED},
    store::channels::{ChannelInput, Rule, RuleInput, RuleState},
};
use trss_core::{
    commands::{CommandState, Outcome},
    Db,
};
use trss_library::discovery::{EpisodeFile, FileKind, Scan, ScannedWork, WorkRead};

const WORK: &str = "Lycoris Recoil";

fn scan(name: &str) -> Scan {
    Scan {
        works: vec![WorkRead::Read(ScannedWork {
            dir_name: name.to_owned(),
            seasons: BTreeSet::from([1]),
            files: vec![EpisodeFile {
                path: "Season 01/S01E01.mkv".into(),
                kind: FileKind::Video,
                season: 1,
                episode: "01".into(),
            }],
            unrecognized: vec![],
        })],
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

/// The collect folder `/c` holding the work, the archive folder `/a` set when
/// `archive`; the state and the work's ID.
async fn scene(archive: bool) -> (AppState, String) {
    let state = AppState::new(Db::open_blocking(":memory:").unwrap());
    state
        .library
        .add_folder("/c".into(), scan(WORK), 100, &[])
        .await
        .unwrap();
    state
        .settings
        .put_collection(0, "/c".into(), archive.then(|| "/a".into()))
        .await
        .unwrap();
    let id = state.library.overview().await.unwrap()[0].id.clone();
    (state, id)
}

async fn detail(state: &AppState, id: &str) -> Value {
    let (status, body) = testing::call(
        &testing::bare_api(state),
        Method::GET,
        &format!("/library/works/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn send(state: &AppState, id: &str, body: Value) -> (StatusCode, Value) {
    testing::call(
        &testing::bare_api(state),
        Method::POST,
        &format!("/library/works/{id}/archive"),
        Some(body),
    )
    .await
}

fn shown(rules: &[&Rule]) -> Value {
    Value::Array(
        rules
            .iter()
            .map(|r| json!({ "id": r.id, "version": r.version }))
            .collect(),
    )
}

async fn rule_ids(state: &AppState) -> Vec<String> {
    state
        .channels
        .list_channels_with_rules()
        .await
        .unwrap()
        .into_iter()
        .flat_map(|cwr| cwr.rules)
        .map(|r| r.id)
        .collect()
}

/// Ends the open command `id` the way the worker would.
async fn end(state: &AppState, id: &str, ended: CommandState, result: &str, reason: &str) {
    let claimed = state.commands.claim_next(200).await.unwrap().unwrap();
    assert_eq!(claimed.id, id);
    state
        .commands
        .finish(
            id,
            ended,
            Outcome {
                result: result.into(),
                reason: Some(reason.into()),
            },
            300,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn the_move_is_offered_only_for_a_work_in_the_collect_folder_with_an_archive_folder() {
    let (state, id) = scene(true).await;
    assert_eq!(
        detail(&state, &id).await["archive"],
        json!({ "available": true, "command": null })
    );

    let (state, id) = scene(false).await;
    assert_eq!(detail(&state, &id).await["archive"]["available"], false);

    // A work in the archive folder, and one in another watch folder.
    let (state, _) = scene(true).await;
    for path in ["/a", "/elsewhere"] {
        state
            .library
            .add_folder(
                path.into(),
                scan(&format!("{WORK} in {path}")),
                100,
                &state.library.folders().await.unwrap(),
            )
            .await
            .unwrap();
    }
    for work in state.library.overview().await.unwrap() {
        let offered = detail(&state, &work.id).await["archive"]["available"].clone();
        assert_eq!(offered, work.dir_name == WORK, "{}", work.dir_name);
    }
}

#[tokio::test]
async fn the_move_deletes_the_rules_it_showed_and_is_accepted_once() {
    let (state, id) = scene(true).await;
    let one = state
        .channels
        .create_channel_with_rules(
            ChannelInput::new("https://feeds.example.org/rss"),
            vec![
                rule("Lycoris Recoil/Season 01", RuleState::Active),
                rule("Other Show/Season 01", RuleState::Active),
            ],
        )
        .await
        .unwrap();
    // A paused rule of another channel is shown and deleted too, and so is
    // one whose save folder lands there by its text, which keeps the folder in
    // place as the move reads it.
    let two = state
        .channels
        .create_channel_with_rules(
            ChannelInput::new("https://other.example.net/feed"),
            vec![
                rule("Lycoris Recoil/Season 02", RuleState::Paused),
                rule("Other/../Lycoris Recoil/Season 03", RuleState::Active),
            ],
        )
        .await
        .unwrap();
    let (active, other) = (&one.rules[0], &one.rules[1]);
    let (paused, dotted) = (&two.rules[0], &two.rules[1]);
    let rules = detail(&state, &id).await["rules"].clone();
    assert_eq!(rules[0]["version"], active.version);
    assert_eq!(rules[1]["state"], "paused");
    assert_eq!(rules[2]["id"], json!(dotted.id));

    let request = json!({ "id": "work-archive-01", "rules": shown(&[paused, dotted, active]) });
    let (status, body) = send(&state, &id, request.clone()).await;

    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["id"], "work-archive-01");
    assert_eq!(body["kind"], "work_archive");
    assert_eq!(body["state"], "pending");
    assert_eq!(rule_ids(&state).await, [other.id.as_str()]);
    let stored = state
        .commands
        .get("work-archive-01")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.payload, json!({ "work": WORK }).to_string());
    assert_eq!(stored.subject.as_deref(), Some(WORK));
    let view = detail(&state, &id).await;
    assert_eq!(view["rules"], json!([]));
    assert_eq!(view["archive"]["command"]["id"], "work-archive-01");

    // The same request again (a lost answer) is answered as stored.
    let (status, body) = send(&state, &id, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], "work-archive-01");

    // Another action while it is open is refused with the open one.
    let (status, body) = send(&state, &id, json!({ "id": "work-archive-02", "rules": [] })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["current"]["command"]["id"], "work-archive-01");
    assert!(state
        .commands
        .get("work-archive-02")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn rules_other_than_the_ones_saving_there_now_delete_nothing() {
    let (state, id) = scene(true).await;
    let c = state
        .channels
        .create_channel_with_rules(
            ChannelInput::new("https://feeds.example.org/rss"),
            vec![
                rule("Lycoris Recoil/Season 01", RuleState::Active),
                rule("Lycoris Recoil/Season 02", RuleState::Paused),
            ],
        )
        .await
        .unwrap();
    let (first, second) = (&c.rules[0], &c.rules[1]);
    let stale = Rule {
        version: first.version - 1,
        ..first.clone()
    };

    for (what, rules) in [
        ("one of them left out", shown(&[first])),
        ("an old version", shown(&[&stale, second])),
        ("one of them twice", shown(&[first, first, second])),
        ("none at all", json!([])),
    ] {
        let (status, body) = send(
            &state,
            &id,
            json!({ "id": "work-archive-01", "rules": rules }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{what}: {body}");
        assert_eq!(body["current"]["command"], Value::Null, "{what}");
        let now: Vec<&Value> = body["current"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| &r["id"])
            .collect();
        assert_eq!(now, [&json!(first.id), &json!(second.id)], "{what}");
        assert_eq!(rule_ids(&state).await.len(), 2, "{what}");
        assert!(
            state
                .commands
                .get("work-archive-01")
                .await
                .unwrap()
                .is_none(),
            "{what}"
        );
    }
}

#[tokio::test]
async fn a_work_that_cannot_move_is_refused_with_why() {
    let (state, id) = scene(false).await;
    let (status, body) = send(&state, &id, json!({ "id": "work-archive-01", "rules": [] })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["message"],
        "보관 폴더를 정하지 않아서 옮길 수 없어요. 설정에서 보관 폴더를 정해 주세요."
    );
    assert_eq!(body.get("current"), None);

    let (state, _) = scene(true).await;
    state
        .library
        .add_folder(
            "/a".into(),
            scan("Archived"),
            100,
            &state.library.folders().await.unwrap(),
        )
        .await
        .unwrap();
    let archived = state
        .library
        .overview()
        .await
        .unwrap()
        .into_iter()
        .find(|w| w.dir_name == "Archived")
        .unwrap();
    let (status, body) = send(
        &state,
        &archived.id,
        json!({ "id": "work-archive-01", "rules": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["message"],
        "이 작품은 수집 폴더에 있지 않아서 보관 폴더로 옮길 수 없어요."
    );

    let (status, _) = send(
        &state,
        "nope",
        json!({ "id": "work-archive-01", "rules": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for bad in [
        json!({ "id": "x", "rules": [] }),
        json!({ "id": "work-archive-01" }),
        json!({ "id": "work-archive-01", "rules": [], "more": 1 }),
    ] {
        let (status, body) = send(&state, &archived.id, bad.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
    assert!(state
        .commands
        .get("work-archive-01")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_move_that_did_not_go_through_is_shown_until_a_rule_saves_there_again() {
    let (state, id) = scene(true).await;
    send(&state, &id, json!({ "id": "work-archive-01", "rules": [] })).await;
    end(
        &state,
        "work-archive-01",
        CommandState::Failed,
        FAILED,
        "겹치는 파일이 있어요.",
    )
    .await;

    let archive = detail(&state, &id).await["archive"].clone();
    assert_eq!(archive["available"], true);
    assert_eq!(archive["command"]["state"], "failed");
    assert_eq!(
        archive["command"]["outcome"]["reason"],
        "겹치는 파일이 있어요."
    );

    // A rule saving into the folder again makes it a work in use.
    state
        .channels
        .create_channel_with_rules(
            ChannelInput::new("https://feeds.example.org/rss"),
            vec![rule("Lycoris Recoil/Season 02", RuleState::Paused)],
        )
        .await
        .unwrap();
    assert_eq!(detail(&state, &id).await["archive"]["command"], Value::Null);

    // A move that went through is not shown (the work left the collect folder).
    let (state, id) = scene(true).await;
    send(&state, &id, json!({ "id": "work-archive-01", "rules": [] })).await;
    end(&state, "work-archive-01", CommandState::Done, MOVED, "").await;
    assert_eq!(detail(&state, &id).await["archive"]["command"], Value::Null);
}
