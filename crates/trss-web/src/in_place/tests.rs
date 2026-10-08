//! `다시 받기` of a video revision is not offered when the web can tell the
//! episode's place holds the same or a higher revision already.

use std::path::PathBuf;

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::{commands_api::now_millis, AppState};
use trss_collect::store::{
    channels::{Channel, ChannelInput, Rule, RuleInput},
    history::{HistoryItem, HistoryResult, Observation},
    revisions::{NewRevision, OldVideo, RevisionState, Step},
};
use trss_core::Db;

const EPISODE_NAME: &str = "Show S01E14.mkv";
const STOPPED: &str =
    "새 영상의 토렌트가 Transmission에서 사라져 받기가 끝나지 않았어요. 이전 영상은 그대로 있어요.";
const IN_PLACE_V3: &str = "이미 같거나 더 높은 수정본(v3)이 있어서 다시 받지 않아요.";
const COMMAND_ID: &str = "0b7d5a44-6c1e-4c62-9a6a-3f0c1d2e4b55";
const MINUTE: i64 = 60_000;

fn title(episode: u32, version: u32) -> String {
    let v = if version > 1 {
        format!("v{version}")
    } else {
        String::new()
    };
    format!("[SubsPlease] Show - {episode}{v} (1080p) [8F2EFEC{version}].mkv")
}

struct World {
    /// The collect folder is `media` in it.
    _dir: tempfile::TempDir,
    /// The rule's folder, `Show/Season 01` under the collect folder.
    folder: PathBuf,
    state: AppState,
    router: Router,
    channel: Channel,
    rule: Rule,
    /// Hands out torrent hashes and identity keys.
    next: std::cell::Cell<u32>,
}

impl World {
    async fn new() -> World {
        let state = AppState::new(Db::open_blocking(":memory:").unwrap());
        let router = Router::new().nest("/api", crate::api::router().with_state(state.clone()));
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        let folder = media.join("Show").join("Season 01");
        std::fs::create_dir_all(&folder).unwrap();
        // The episode's video is in its place, unless a test takes it away.
        std::fs::write(folder.join(EPISODE_NAME), b"video").unwrap();
        state
            .settings
            .put_collection(0, media.to_str().unwrap().to_owned(), None)
            .await
            .unwrap();
        let channel = state
            .channels
            .create_channel(ChannelInput::new("https://feed.example/rss"))
            .await
            .unwrap();
        let rule = state
            .channels
            .create_rule(
                &channel.id,
                RuleInput {
                    r#match: Some("Show - ".into()),
                    directory: "Show/Season 01".into(),
                    ..RuleInput::default()
                },
            )
            .await
            .unwrap();
        World {
            _dir: dir,
            folder,
            state,
            router,
            channel,
            rule,
            next: std::cell::Cell::new(0),
        }
    }

    /// The video at the episode name is gone.
    fn remove_video(&self) {
        std::fs::remove_file(self.folder.join(EPISODE_NAME)).unwrap();
    }

    fn fresh(&self) -> (String, String) {
        self.next.set(self.next.get() + 1);
        let n = self.next.get();
        (format!("guid-{n}"), format!("{n:040x}"))
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(json) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// A history item of the rule, recorded ten minutes ago; its torrent hash.
    async fn item(&self, title: String, result: HistoryResult) -> (HistoryItem, String) {
        let (key, hash) = self.fresh();
        self.state
            .history
            .record(
                now_millis() - 10 * MINUTE,
                vec![Observation {
                    channel_id: self.channel.id.clone(),
                    channel_label: self.channel.masked_url(),
                    identity_key: key.clone(),
                    title,
                    link: format!("magnet:?xt=urn:btih:{hash}"),
                    result,
                    rule_id: Some(self.rule.id.clone()),
                    torrent_hash: Some(hash.clone()),
                    reason: None,
                }],
            )
            .await
            .unwrap();
        let item = self
            .state
            .history
            .item_by_key(self.channel.id.clone(), key)
            .await
            .unwrap()
            .unwrap();
        (item, hash)
    }

    fn row(&self, item: &HistoryItem, hash: &str, version: u32) -> NewRevision {
        NewRevision {
            item_id: item.id,
            old_item_id: None,
            rule_id: self.rule.id.clone(),
            folder: self.folder.to_str().unwrap().to_owned(),
            episode_name: EPISODE_NAME.into(),
            old_version: Some(1),
            new_version: version,
            old_crc: None,
            expected_crc: Some("8F2EFEC2".into()),
            torrent_hash: Some(hash.to_owned()),
            state: RevisionState::Receiving,
            reason: None,
        }
    }

    /// `14v2` whose download stopped before it was received.
    async fn stopped_v2(&self) -> HistoryItem {
        let (item, hash) = self.item(title(14, 2), HistoryResult::Received).await;
        let row = self
            .state
            .revisions
            .create(10, self.row(&item, &hash, 2))
            .await
            .unwrap();
        let step = Step::Failed {
            reason: STOPPED.into(),
            received_name: None,
        };
        self.state
            .revisions
            .advance(row.id, 20, RevisionState::Receiving, step)
            .await
            .unwrap();
        item
    }

    /// A `버전 미상` `14v2`: the worker did not receive it.
    async fn unknown_v2(&self) -> HistoryItem {
        self.unknown_titled(title(14, 2)).await
    }

    /// A `버전 미상` item of `title`.
    async fn unknown_titled(&self, title: String) -> HistoryItem {
        let (item, _) = self.item(title, HistoryResult::VersionUnknown).await;
        let mut row = self.row(&item, "", 2);
        row.state = RevisionState::Unknown;
        row.torrent_hash = None;
        row.reason = Some("no crc".into());
        self.state.revisions.create(10, row).await.unwrap();
        item
    }

    /// A replacement of the episode by the revision `version`, done.
    async fn done(&self, version: u32) {
        self.done_titled(title(14, version), version).await;
    }

    /// [`World::done`] of the release `title`.
    async fn done_titled(&self, title: String, version: u32) {
        let (item, hash) = self.item(title, HistoryResult::Received).await;
        let store = &self.state.revisions;
        let row = store
            .create(10, self.row(&item, &hash, version))
            .await
            .unwrap();
        let step = Step::Verified {
            received_name: "v.mkv".into(),
            file_crc: "1A2B3C4D".into(),
            file_identity: "1:2:3:4:5:6:7".into(),
        };
        store
            .advance(row.id, 11, RevisionState::Receiving, step)
            .await
            .unwrap();
        let old = OldVideo {
            item_id: None,
            version: Some(1),
            torrent_hash: None,
        };
        store.claim(row.id, 12, old).await.unwrap();
        store
            .advance(
                row.id,
                13,
                RevisionState::Removing,
                Step::Removed { reason: None },
            )
            .await
            .unwrap();
        store
            .advance(row.id, 14, RevisionState::Removed, Step::Done)
            .await
            .unwrap();
    }

    /// An ordinary item (no replacement row) of `title`, received; its hash.
    async fn placed(&self, title: String) -> String {
        self.item(title, HistoryResult::Received).await.1
    }

    /// The worker's list of Transmission's torrents, taken `taken_ago` ago.
    async fn listing(&self, taken_ago: i64, hashes: &[&str]) {
        self.state
            .status
            .record_listing(
                now_millis() - taken_ago,
                hashes.iter().map(|h| (*h).to_owned()).collect(),
            )
            .await
            .unwrap();
    }

    /// The `revision` entry of the to-do source.
    async fn failure(&self) -> Value {
        let (status, json) = self
            .call(Method::GET, "/api/todo/receive-failures", None)
            .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let items: Vec<&Value> = json["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["kind"] == "revision")
            .collect();
        assert_eq!(items.len(), 1, "{json}");
        items[0].clone()
    }

    async fn history_row(&self, item: &HistoryItem) -> Value {
        let (status, json) = self
            .call(Method::GET, &format!("/api/history/{}", item.id), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        json
    }

    async fn retry(&self, item: &HistoryItem) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            "/api/commands",
            Some(json!({
                "id": COMMAND_ID,
                "kind": "receive_once",
                "payload": { "item_id": item.id },
            })),
        )
        .await
    }
}

/// Whether the row offers `다시 받기`; an offered button has no reason and a
/// hidden one has.
fn offered(row: &Value) -> bool {
    let can_retry = row["can_retry"] == true;
    if can_retry {
        assert!(row["retry_blocked"].is_null(), "{row}");
    }
    can_retry
}

// --- a replacement of the episode done at the same or a higher revision ----

#[tokio::test]
async fn a_stopped_revision_below_a_done_revision_of_the_episode_has_no_button_and_says_why() {
    let w = World::new().await;
    w.stopped_v2().await;
    w.done(3).await;
    let row = w.failure().await;
    assert!(!offered(&row), "{row}");
    assert_eq!(row["retry_blocked"], IN_PLACE_V3);
}

#[tokio::test]
async fn a_version_unknown_item_below_a_done_revision_of_the_episode_has_no_button_and_says_why() {
    let w = World::new().await;
    let item = w.unknown_v2().await;
    w.done(3).await;
    let row = w.history_row(&item).await;
    assert!(!offered(&row), "{row}");
    assert_eq!(row["retry_blocked"], IN_PLACE_V3);
    let listed = w.call(Method::GET, "/api/history", None).await.1;
    let in_list = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == item.id)
        .unwrap();
    assert_eq!(in_list["retry_blocked"], IN_PLACE_V3);
}

#[tokio::test]
async fn a_done_revision_of_the_same_version_hides_the_button_too() {
    let w = World::new().await;
    let item = w.unknown_v2().await;
    w.done(2).await;
    let row = w.history_row(&item).await;
    assert!(!offered(&row), "{row}");
    assert_eq!(
        row["retry_blocked"],
        "이미 같거나 더 높은 수정본(v2)이 있어서 다시 받지 않아요."
    );
}

#[tokio::test]
async fn a_done_revision_of_a_lower_version_leaves_the_button() {
    let w = World::new().await;
    let item = w.unknown_v2().await;
    w.done(1).await;
    let row = w.history_row(&item).await;
    assert!(offered(&row), "{row}");
}

// --- an ordinary item of the release placed at the episode name -----------

#[tokio::test]
async fn a_higher_item_held_by_transmission_at_the_episode_name_hides_the_button() {
    let w = World::new().await;
    let item = w.unknown_v2().await;
    let hash = w.placed(title(14, 3)).await;
    w.listing(MINUTE, &[&hash]).await;
    let row = w.history_row(&item).await;
    assert!(!offered(&row), "{row}");
    assert_eq!(row["retry_blocked"], IN_PLACE_V3);
}

#[tokio::test]
async fn a_stopped_revision_below_a_held_higher_item_has_no_button() {
    let w = World::new().await;
    w.stopped_v2().await;
    let hash = w.placed(title(14, 3)).await;
    w.listing(MINUTE, &[&hash]).await;
    let row = w.failure().await;
    assert!(!offered(&row), "{row}");
    assert_eq!(row["retry_blocked"], IN_PLACE_V3);
}

/// Evidence that is one thing short leaves the button where it was: the
/// worker looks at the folder when the command runs.
#[tokio::test]
async fn a_listing_that_is_absent_stale_or_without_the_torrent_leaves_the_button() {
    // (what is short, the listing's age, whether it holds the torrent)
    let cases = [
        ("no listing", None, true),
        ("a stale listing", Some(3 * 24 * 60 * MINUTE), true),
        ("a listing without the torrent", Some(MINUTE), false),
    ];
    for (what, taken_ago, holds) in cases {
        let w = World::new().await;
        let item = w.unknown_v2().await;
        let hash = w.placed(title(14, 3)).await;
        if let Some(ago) = taken_ago {
            let other = "f".repeat(40);
            w.listing(ago, &[if holds { &hash } else { &other }]).await;
        }
        let row = w.history_row(&item).await;
        assert!(offered(&row), "{what}: {row}");
    }
}

#[tokio::test]
async fn a_held_item_of_another_episode_or_a_lower_revision_leaves_the_button() {
    for (episode, version) in [(15, 3), (14, 1)] {
        let w = World::new().await;
        let item = w.unknown_v2().await;
        let hash = w.placed(title(episode, version)).await;
        w.listing(MINUTE, &[&hash]).await;
        let row = w.history_row(&item).await;
        assert!(offered(&row), "episode {episode} v{version}: {row}");
    }
}

#[tokio::test]
async fn a_held_item_of_another_release_leaves_the_button() {
    let w = World::new().await;
    let item = w.unknown_v2().await;
    let hash = w
        .placed("[Other] Show - 14v3 (1080p) [8F2EFEC3].mkv".into())
        .await;
    w.listing(MINUTE, &[&hash]).await;
    let row = w.history_row(&item).await;
    assert!(offered(&row), "{row}");
}

// --- the command ----------------------------------------------------------

#[tokio::test]
async fn a_retry_of_a_revision_the_episode_holds_already_is_refused_with_the_reason() {
    let w = World::new().await;
    let item = w.stopped_v2().await;
    w.done(3).await;
    let (status, body) = w.retry(&item).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["message"], IN_PLACE_V3);
    let stored = w.state.commands.get(COMMAND_ID).await.unwrap();
    assert!(stored.is_none(), "nothing is stored");
}

#[tokio::test]
async fn a_retry_of_a_version_unknown_item_the_listing_places_is_refused() {
    let w = World::new().await;
    let item = w.unknown_v2().await;
    let hash = w.placed(title(14, 3)).await;
    w.listing(MINUTE, &[&hash]).await;
    let (status, body) = w.retry(&item).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["message"], IN_PLACE_V3);
    assert!(w.state.commands.get(COMMAND_ID).await.unwrap().is_none());
}

#[tokio::test]
async fn a_retry_with_no_evidence_is_accepted_and_the_worker_stays_the_authority() {
    let w = World::new().await;
    let item = w.unknown_v2().await;
    let (status, body) = w.retry(&item).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
}

// --- the file at the episode name -----------------------------------------

#[tokio::test]
async fn a_done_revision_without_a_file_at_the_episode_name_leaves_the_button() {
    let w = World::new().await;
    let item = w.unknown_v2().await;
    w.done(3).await;
    w.remove_video();
    let row = w.history_row(&item).await;
    assert!(offered(&row), "{row}");
    let (status, body) = w.retry(&item).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
}

#[tokio::test]
async fn a_held_item_without_a_file_at_the_episode_name_leaves_the_button() {
    let w = World::new().await;
    let item = w.unknown_v2().await;
    let hash = w.placed(title(14, 3)).await;
    w.listing(MINUTE, &[&hash]).await;
    w.remove_video();
    let row = w.history_row(&item).await;
    assert!(offered(&row), "{row}");
    let (status, body) = w.retry(&item).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
}

#[tokio::test]
async fn a_folder_that_cannot_be_looked_at_leaves_the_button() {
    let w = World::new().await;
    let item = w.unknown_v2().await;
    w.done(3).await;
    std::fs::remove_dir_all(&w.folder).unwrap();
    let row = w.history_row(&item).await;
    assert!(offered(&row), "{row}");
    let (status, body) = w.retry(&item).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
}

/// A stopped revision is also refused by the worker's own verdict on the rows
/// of the episode (`RevisionRetry::Overtaken`), whatever the folder holds, so
/// the web is no stricter than the worker in keeping it hidden.
#[tokio::test]
async fn a_stopped_revision_below_a_done_one_stays_refused_as_the_worker_refuses_it() {
    let w = World::new().await;
    let item = w.stopped_v2().await;
    w.done(3).await;
    w.remove_video();
    let row = w.failure().await;
    assert!(!offered(&row), "{row}");
    let (status, _) = w.retry(&item).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// --- Erai-raws' magnet titles: no extension, other language lists ----------

const MAGNET_V2: &str = "[Magnet] Show - 14 (V2) [1080p CR WEB-DL AVC AAC][us][br][pl][Airing]";
const MAGNET_V3: &str = "[Magnet] Show - 14 (V3) [1080p CR WEB-DL AVC AAC][us][br][Airing]";

/// The release is told without its language tags: a done replacement by
/// another list of the same release hides the button.
#[tokio::test]
async fn a_done_revision_listed_with_other_language_tags_is_the_same_release() {
    let w = World::new().await;
    let item = w.unknown_titled(MAGNET_V2.into()).await;
    w.done_titled(MAGNET_V3.into(), 3).await;
    let row = w.history_row(&item).await;
    assert!(!offered(&row), "{row}");
    assert_eq!(row["retry_blocked"], IN_PLACE_V3);

    // Another group's release is another release.
    let w = World::new().await;
    let item = w.unknown_titled(MAGNET_V2.into()).await;
    w.done_titled(MAGNET_V3.replacen("[Magnet]", "[Other]", 1), 3)
        .await;
    assert!(offered(&w.history_row(&item).await));
}

/// An item held at the episode name is found for a title without an
/// extension (the name `trname` gives it, with any video extension), and by
/// its release without the language tags.
#[tokio::test]
async fn a_held_magnet_item_with_other_language_tags_hides_the_button() {
    let w = World::new().await;
    let item = w.unknown_titled(MAGNET_V2.into()).await;
    let hash = w.placed(MAGNET_V3.into()).await;
    w.listing(MINUTE, &[&hash]).await;
    let row = w.history_row(&item).await;
    assert!(!offered(&row), "{row}");
    assert_eq!(row["retry_blocked"], IN_PLACE_V3);

    // One of another episode names another file.
    let w = World::new().await;
    let item = w.unknown_titled(MAGNET_V2.into()).await;
    let hash = w.placed(MAGNET_V3.replace("14", "15")).await;
    w.listing(MINUTE, &[&hash]).await;
    assert!(offered(&w.history_row(&item).await));
}
