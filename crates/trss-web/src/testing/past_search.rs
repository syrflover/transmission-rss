//! A tracker, a channel on it and a rule, for the tests of the past episode
//! search and of the `받기` of its results: the real API and search service
//! against the fake nyaa search RSS ([`trss_collect::fake::FakeNyaa`]), with a
//! temporary media folder. Release names are made up.

use std::{path::PathBuf, time::Duration};

use axum::{
    http::{Method, StatusCode},
    Router,
};
use serde_json::{json, Value};
use tempfile::TempDir;
use trss_collect::{
    fake::FakeNyaa,
    past_search::service::PastSearch,
    store::{
        channels::{ChannelInput, RuleInput},
        search_pace::SearchPace,
    },
};
use trss_core::Db;

use crate::{testing, AppState};

/// A secret value of the channel's address, which no answer may hold.
pub(crate) const SECRET: &str = "SECRETTOKEN0123456789";

/// The time between two requests of the searches of a test.
const SPACING: Duration = Duration::from_millis(5);

/// A channel on a fake tracker with one rule saving into a work folder of a
/// temporary collect folder.
pub(crate) struct Tracker {
    pub state: AppState,
    pub router: Router,
    pub db: Db,
    pub nyaa: FakeNyaa,
    pub _dir: TempDir,
    /// The rule's work folder.
    pub folder: PathBuf,
    pub rule_id: String,
    pub channel_id: String,
}

impl Tracker {
    /// The rule of `Show` saving into `Show/Season 01`, on a channel whose
    /// search format is `[SubsPlease] {match} 1080p`.
    pub async fn new() -> Tracker {
        Tracker::with(
            Some("[SubsPlease] {match} 1080p"),
            "Show",
            "Show/Season 01",
            0,
        )
        .await
    }

    pub async fn with(
        format: Option<&str>,
        phrase: &str,
        directory: &str,
        episode: i64,
    ) -> Tracker {
        let db = Db::open_blocking(":memory:").unwrap();
        let state = AppState::new(db.clone())
            .with_past_search(PastSearch::new(SearchPace::new(db.clone())).with_spacing(SPACING));
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        let folder = media.join(directory);
        std::fs::create_dir_all(&folder).unwrap();
        state
            .settings
            .put_collection(0, media.to_str().unwrap().to_owned(), None)
            .await
            .unwrap();
        let nyaa = FakeNyaa::start().await;
        let mut input = ChannelInput::new(nyaa.url(SECRET));
        input.past_search = format.map(str::to_owned);
        let channel = state
            .channels
            .create_channel_with_rules(
                input,
                vec![RuleInput {
                    r#match: Some(phrase.to_owned()),
                    directory: directory.to_owned(),
                    episode,
                    ..Default::default()
                }],
            )
            .await
            .unwrap();
        Tracker {
            router: testing::api(&state),
            rule_id: channel.rules[0].id.clone(),
            channel_id: channel.channel.id,
            state,
            db,
            nyaa,
            _dir: dir,
            folder,
        }
    }

    /// A request; no answer holds the channel's secret.
    pub async fn call(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let (status, text, json) = testing::call_text(&self.router, method, uri, body).await;
        assert!(
            !text.contains(SECRET),
            "a secret value is in an answer: {text}"
        );
        (status, json)
    }

    pub fn search_uri(&self) -> String {
        format!("/api/rules/{}/past-search", self.rule_id)
    }

    /// Starts a search; the answer's status and body.
    pub async fn start(&self, query: &str, from: u32, to: u32) -> (StatusCode, Value) {
        let body = json!({ "query": query, "from": from, "to": to });
        self.call(Method::POST, &self.search_uri(), Some(body))
            .await
    }

    /// Polls the search `id` until it is not running; the final poll, with
    /// `search_id` added.
    pub async fn finished(&self, id: &str) -> Value {
        for _ in 0..2000 {
            let (status, mut poll) = self
                .call(Method::GET, &format!("/api/past-searches/{id}"), None)
                .await;
            assert_eq!(status, StatusCode::OK, "{poll}");
            if poll["state"] != "running" {
                poll["search_id"] = json!(id);
                return poll;
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        panic!("search hangs");
    }

    /// Starts a search and waits for it to end; returns the final poll.
    pub async fn search(&self, query: &str, from: u32, to: u32) -> Value {
        let (status, started) = self.start(query, from, to).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{started}");
        self.finished(started["search_id"].as_str().unwrap()).await
    }

    /// Asks for `key` of `poll`'s search to be received, as the command `id`.
    pub async fn receive(&self, id: &str, poll: &Value, key: &str) -> (StatusCode, Value) {
        self.receive_of(id, &self.rule_id, &poll["search_id"], key)
            .await
    }

    /// [`Tracker::receive`] with the rule and the search named as given.
    pub async fn receive_of(
        &self,
        id: &str,
        rule_id: &str,
        search_id: &Value,
        key: &str,
    ) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            "/api/commands",
            Some(json!({
                "id": id,
                "kind": "receive_past",
                "payload": { "rule_id": rule_id, "search_id": search_id, "key": key },
            })),
        )
        .await
    }

    pub fn write(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.folder.join(name), bytes).unwrap();
    }
}

/// The one item of `poll`'s result whose title holds `title_part`.
pub(crate) fn item<'a>(poll: &'a Value, title_part: &str) -> &'a Value {
    let found: Vec<&Value> = poll["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["title"].as_str().unwrap().contains(title_part))
        .collect();
    assert_eq!(found.len(), 1, "{title_part}: {found:?}");
    found[0]
}

/// The release numbers of the items `poll` selects, ascending.
pub(crate) fn selected(poll: &Value) -> Vec<u32> {
    let mut numbers: Vec<u32> = poll["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["selected"] == true)
        .map(|i| i["release"].as_u64().unwrap() as u32)
        .collect();
    numbers.sort_unstable();
    numbers
}

/// `[group] show - nn (1080p) [crc]extra.mkv`.
pub(crate) fn episode(group: &str, show: &str, n: u32, extra: &str) -> String {
    format!(
        "[{group}] {show} - {n:02} (1080p) [{:08X}]{extra}.mkv",
        0xA000_0000u32 + n
    )
}
