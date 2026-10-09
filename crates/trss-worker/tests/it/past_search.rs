//! The past episode search of the rule detail (ticket 0026,
//! `docs/specs/collection.md` 지난 회차 검색), end to end: the real web API
//! and search service, a fake nyaa search RSS, the worker's `receive_past`
//! command against the fake Transmission, and a temporary media folder.
//!
//! One test: what a search and a `받기` do through every layer. The rules
//! each layer owns are tested there (ADR 0015): the judgement and paging of a
//! search, `receive_past` and its revisions in trss-collect (`past_search`,
//! `commands/receive_past`, `revisions/past_tests.rs`), the request checks and
//! the shapes in trss-web (`past_search_api`, `commands_api/past_tests.rs`).
//! A command that ended with an unanswered add holds the next cleanup whatever
//! its kind (`receive_once.rs`). Release names, hashes and contents are made up.

use crate::common;

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use trss_collect::{
    past_search::service::PastSearch,
    store::{
        channels::{ChannelInput, RuleInput},
        history::HistoryResult,
        search_pace::SearchPace,
    },
};
use trss_core::settings::SettingsStore;
use trss_web::AppState;
use trss_worker::CommandsOutcome;

const SPACING: Duration = Duration::from_millis(150);
const COMMAND: &str = "past-cmd-0";

/// A harness with a temporary media folder the fake Transmission acts on, a
/// fake nyaa, and one channel on it with one rule.
struct Setup {
    h: Harness,
    nyaa: FakeNyaa,
    api: WebApi,
    folder: PathBuf,
    rule_id: String,
}

impl Setup {
    async fn new() -> Setup {
        let h = Harness::without_collect_folder().await;
        let media = h.dir.path().join("media");
        let folder = media.join("Show/Season 01");
        std::fs::create_dir_all(&folder).unwrap();
        SettingsStore::new(h.db.clone())
            .put_collection(0, media.to_str().unwrap().to_owned(), None)
            .await
            .unwrap();
        h.tr.on_disk(&media);
        let nyaa = FakeNyaa::start().await;
        let mut input = ChannelInput::new(nyaa.url(SECRET));
        input.past_search = Some("[SubsPlease] {match} 1080p".to_owned());
        let channel = h
            .channels
            .create_channel_with_rules(
                input,
                vec![RuleInput {
                    r#match: Some("Show".to_owned()),
                    directory: "Show/Season 01".to_owned(),
                    ..Default::default()
                }],
            )
            .await
            .unwrap();
        let state = AppState::new(h.db.clone())
            .with_past_search(PastSearch::new(SearchPace::new(h.db.clone())).with_spacing(SPACING));
        Setup {
            api: WebApi::with_state(state),
            rule_id: channel.rules[0].id.clone(),
            folder,
            nyaa,
            h,
        }
    }

    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let (status, text, json) = self.api.call(method, uri, body).await;
        assert!(
            !text.contains(SECRET),
            "a secret value is in an answer: {text}"
        );
        (status, json)
    }

    /// Starts a search and waits for it to end; returns the final poll.
    async fn search(&self, query: &str, from: u32, to: u32) -> Value {
        let (status, started) = self
            .call(
                "POST",
                &format!("/api/rules/{}/past-search", self.rule_id),
                Some(json!({ "query": query, "from": from, "to": to })),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{started}");
        let id = started["search_id"].as_str().unwrap().to_owned();
        let begun = Instant::now();
        loop {
            let (status, poll) = self
                .call("GET", &format!("/api/past-searches/{id}"), None)
                .await;
            assert_eq!(status, StatusCode::OK, "{poll}");
            if poll["state"] != "running" {
                let mut poll = poll;
                poll["search_id"] = json!(id);
                return poll;
            }
            assert!(begun.elapsed() < Duration::from_secs(30), "search hangs");
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }

    fn added(&self) -> Vec<String> {
        self.h
            .tr
            .calls_of("torrent-add")
            .iter()
            .map(|c| c.args["filename"].as_str().unwrap().to_owned())
            .collect()
    }
}

fn episode(n: u32) -> String {
    format!(
        "[SubsPlease] Show - {n:02} (1080p) [{:08X}].mkv",
        0xA000_0000u32 + n
    )
}

/// The ticket's first row: a search that returns every episode selects the
/// missing ones, adds nothing by itself, and the worker adds the one the
/// person confirms into the rule's folder.
#[tokio::test]
async fn a_result_chosen_in_a_search_is_added_by_the_worker_into_the_rules_folder() {
    let s = Setup::new().await;
    let titles: Vec<String> = (1..=3).rev().map(episode).collect();
    s.nyaa.set_releases(&titles);
    std::fs::write(s.folder.join("Show S01E01.mkv"), b"x").unwrap();

    let poll = s.search("[SubsPlease] Show 1080p", 1, 3).await;

    assert_eq!(poll["state"], "done", "{poll}");
    let items = poll["result"]["items"].as_array().unwrap();
    let selected: Vec<u64> = items
        .iter()
        .filter(|i| i["selected"] == true)
        .map(|i| i["release"].as_u64().unwrap())
        .collect();
    assert_eq!(selected.len(), 2, "{poll}");
    assert!(s.added().is_empty(), "a search adds nothing");

    // The person confirms the third.
    let three = items
        .iter()
        .find(|i| i["release"] == 3)
        .expect("episode 3 is listed")["key"]
        .clone();
    let (status, body) = s
        .call(
            "POST",
            "/api/commands",
            Some(json!({
                "id": COMMAND,
                "kind": "receive_past",
                "payload": {
                    "rule_id": s.rule_id,
                    "search_id": poll["search_id"],
                    "key": three,
                },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let outcome =
        s.h.worker()
            .run_commands(&CancellationToken::new())
            .await
            .unwrap();
    assert_eq!(outcome, CommandsOutcome::Ran(1));

    let (_, command) = s
        .call("GET", &format!("/api/commands/{COMMAND}"), None)
        .await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "received", "{command}");
    let hash = FakeNyaa::hash_for(&episode(3));
    let added = s.added();
    assert_eq!(added.len(), 1, "{added:?}");
    assert!(added[0].contains(&hash), "{added:?}");
    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 1);
    assert!(
        torrents[0].download_dir.ends_with("Show/Season 01"),
        "{}",
        torrents[0].download_dir
    );
    // Only the confirmed result is in history.
    let history = s.h.history_items().await;
    assert_eq!(history.len(), 1, "{history:?}");
    assert_eq!(history[0].title, episode(3));
    assert_eq!(history[0].result, HistoryResult::Received);
}
