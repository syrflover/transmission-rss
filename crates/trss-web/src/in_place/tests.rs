//! `다시 받기` of a video revision is not offered when the web can tell the
//! episode's place holds the same or a higher revision already.
//!
//! The rule (what [`Evidence::held`] tells, from which rows, listing and
//! files) is tested directly on `Evidence`; the router tests that stay check
//! that the screens and the command take its verdict (ADR 0015, ticket 0101).

use std::path::PathBuf;

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::Evidence;
use crate::{commands_api::now_millis, AppState};
use trss_collect::store::{
    channels::{Channel, ChannelInput, Rule, RuleInput, RuleState},
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

    /// The revision [`Evidence::held`] tells for the replacement row of `item`.
    async fn held(&self, item: &HistoryItem) -> Option<u32> {
        let row = self
            .state
            .revisions
            .by_item(item.id)
            .await
            .unwrap()
            .expect("a replacement row");
        let mut evidence = Evidence::load(&self.state).await.unwrap();
        evidence
            .held(item, &row)
            .await
            .unwrap()
            .map(|place| place.version)
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

// --- what `Evidence::held` tells ---------------------------------------------

/// The two kinds of row that wait for `다시 받기`: a revision whose download
/// stopped, and a `버전 미상` item.
#[derive(Clone, Copy, Debug)]
enum Waiting {
    Stopped,
    Unknown,
}

impl World {
    async fn waiting(&self, kind: Waiting) -> HistoryItem {
        match kind {
            Waiting::Stopped => self.stopped_v2().await,
            Waiting::Unknown => self.unknown_v2().await,
        }
    }
}

const BOTH: [Waiting; 2] = [Waiting::Stopped, Waiting::Unknown];

#[tokio::test]
async fn a_done_revision_of_the_same_or_a_higher_version_holds_the_episode_a_lower_one_does_not() {
    for kind in BOTH {
        for (done, expected) in [(3, Some(3)), (2, Some(2)), (1, None)] {
            let w = World::new().await;
            let item = w.waiting(kind).await;
            w.done(done).await;
            assert_eq!(w.held(&item).await, expected, "{kind:?}, done v{done}");
        }
    }
}

#[tokio::test]
async fn a_higher_item_held_by_transmission_at_the_episode_name_holds_the_episode() {
    for kind in BOTH {
        let w = World::new().await;
        let item = w.waiting(kind).await;
        let hash = w.placed(title(14, 3)).await;
        w.listing(MINUTE, &[&hash]).await;
        assert_eq!(w.held(&item).await, Some(3), "{kind:?}");
    }
}

/// Evidence that is one thing short holds nothing: the worker looks at the
/// folder when the command runs.
#[tokio::test]
async fn a_listing_that_is_absent_stale_or_without_the_torrent_holds_nothing() {
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
        assert_eq!(w.held(&item).await, None, "{what}");
    }
}

#[tokio::test]
async fn a_held_item_of_another_episode_release_or_a_lower_revision_holds_nothing() {
    for (what, title) in [
        ("another episode", self::title(15, 3)),
        ("a lower revision", self::title(14, 1)),
        (
            "another release",
            "[Other] Show - 14v3 (1080p) [8F2EFEC3].mkv".to_owned(),
        ),
    ] {
        let w = World::new().await;
        let item = w.unknown_v2().await;
        let hash = w.placed(title).await;
        w.listing(MINUTE, &[&hash]).await;
        assert_eq!(w.held(&item).await, None, "{what}");
    }
}

/// A place with no file at the episode name, or one that cannot be looked at,
/// holds nothing, whatever the rows and the listing say.
#[tokio::test]
async fn a_place_without_a_file_at_the_episode_name_holds_nothing() {
    // (what is missing, whether the evidence is a done row or a held item)
    for (what, done_row, remove_folder) in [
        ("the file, with a done revision", true, false),
        ("the file, with a held item", false, false),
        ("the folder", true, true),
    ] {
        let w = World::new().await;
        let item = w.unknown_v2().await;
        if done_row {
            w.done(3).await;
        } else {
            let hash = w.placed(title(14, 3)).await;
            w.listing(MINUTE, &[&hash]).await;
        }
        assert_eq!(w.held(&item).await, Some(3), "{what}: with the file");
        if remove_folder {
            std::fs::remove_dir_all(&w.folder).unwrap();
        } else {
            w.remove_video();
        }
        assert_eq!(w.held(&item).await, None, "{what}");
    }
}

// Erai-raws' magnet titles have no extension and other language lists.
const MAGNET_V2: &str = "[Magnet] Show - 14 (V2) [1080p CR WEB-DL AVC AAC][us][br][pl][Airing]";
const MAGNET_V3: &str = "[Magnet] Show - 14 (V3) [1080p CR WEB-DL AVC AAC][us][br][Airing]";

/// The release is told without its language tags: a done replacement by
/// another list of the same release holds the episode, another group's does
/// not.
#[tokio::test]
async fn a_done_revision_listed_with_other_language_tags_is_the_same_release() {
    let w = World::new().await;
    let item = w.unknown_titled(MAGNET_V2.into()).await;
    w.done_titled(MAGNET_V3.into(), 3).await;
    assert_eq!(w.held(&item).await, Some(3));

    let w = World::new().await;
    let item = w.unknown_titled(MAGNET_V2.into()).await;
    w.done_titled(MAGNET_V3.replacen("[Magnet]", "[Other]", 1), 3)
        .await;
    assert_eq!(w.held(&item).await, None);
}

/// An item held at the episode name is found for a title without an
/// extension (the name `trname` gives it, with any video extension), and by
/// its release without the language tags.
#[tokio::test]
async fn a_held_magnet_item_with_other_language_tags_holds_the_episode() {
    let w = World::new().await;
    let item = w.unknown_titled(MAGNET_V2.into()).await;
    let hash = w.placed(MAGNET_V3.into()).await;
    w.listing(MINUTE, &[&hash]).await;
    assert_eq!(w.held(&item).await, Some(3));

    // One of another episode names another file.
    let w = World::new().await;
    let item = w.unknown_titled(MAGNET_V2.into()).await;
    let hash = w.placed(MAGNET_V3.replace("14", "15")).await;
    w.listing(MINUTE, &[&hash]).await;
    assert_eq!(w.held(&item).await, None);
}

// --- the command ----------------------------------------------------------

/// The command takes the verdict for either row kind and either evidence: a
/// stopped revision below a done one, and a `버전 미상` item the listing places.
#[tokio::test]
async fn a_retry_of_a_revision_the_episode_holds_already_is_refused_with_the_reason() {
    for placed in [false, true] {
        let w = World::new().await;
        let item = if placed {
            let item = w.unknown_v2().await;
            let hash = w.placed(title(14, 3)).await;
            w.listing(MINUTE, &[&hash]).await;
            item
        } else {
            let item = w.stopped_v2().await;
            w.done(3).await;
            item
        };
        let (status, body) = w.retry(&item).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "placed {placed}: {body}");
        assert_eq!(body["message"], IN_PLACE_V3);
        let stored = w.state.commands.get(COMMAND_ID).await.unwrap();
        assert!(stored.is_none(), "nothing is stored");
    }
}

#[tokio::test]
async fn a_retry_with_no_evidence_is_accepted_and_the_worker_stays_the_authority() {
    let w = World::new().await;
    let item = w.unknown_v2().await;
    let (status, body) = w.retry(&item).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
}

// --- the offer of a stopped revision and its refusals ---------------------

/// A stopped revision is offered `다시 받기`; a request shows on the failure
/// as pending until the worker runs it.
#[tokio::test]
async fn a_stopped_revision_is_offered_and_an_accepted_request_shows_as_pending() {
    let w = World::new().await;
    let item = w.stopped_v2().await;
    let row = w.failure().await;
    assert!(offered(&row), "{row}");
    assert_eq!(row["history_item_id"], item.id);
    assert_eq!(row["command"], Value::Null);

    let (status, body) = w.retry(&item).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(w.failure().await["command"]["state"], "pending");
}

/// A revision received into another folder (it failed after it was received)
/// would end the same way, so it is not offered again and the request is
/// refused; the failure's own reason says why it failed, so the refusal adds
/// none to the row.
#[tokio::test]
async fn a_revision_received_elsewhere_is_not_offered_and_its_request_is_refused() {
    let w = World::new().await;
    let (item, hash) = w.item(title(14, 2), HistoryResult::Received).await;
    let row = w
        .state
        .revisions
        .create(10, w.row(&item, &hash, 2))
        .await
        .unwrap();
    let step = Step::Failed {
        reason: "받은 위치가 달라요.".into(),
        received_name: Some("v2.mkv".into()),
    };
    w.state
        .revisions
        .advance(row.id, 20, RevisionState::Receiving, step)
        .await
        .unwrap();

    let row = w.failure().await;
    assert!(!offered(&row), "{row}");
    assert_eq!(row["retry_blocked"], Value::Null);
    let (status, body) = w.retry(&item).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(w.state.commands.get(COMMAND_ID).await.unwrap().is_none());
}

/// A stopped revision whose rule is paused says why `다시 받기` is missing,
/// and the request is refused.
#[tokio::test]
async fn a_stopped_revision_of_a_paused_rule_says_why_and_its_request_is_refused() {
    let w = World::new().await;
    let item = w.stopped_v2().await;
    w.state
        .channels
        .set_rule_state(&w.rule.id, RuleState::Paused, now_millis())
        .await
        .unwrap();

    let row = w.failure().await;
    assert!(!offered(&row), "{row}");
    assert!(row["retry_blocked"].as_str().unwrap().contains("멈춰"));
    let (status, body) = w.retry(&item).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(w.state.commands.get(COMMAND_ID).await.unwrap().is_none());
}

/// The rule's folder changed after the replacement was decided: a torrent
/// added now would be received away from the video it replaces.
#[tokio::test]
async fn a_stopped_revision_of_a_rule_whose_folder_changed_says_why_and_its_request_is_refused() {
    let w = World::new().await;
    let item = w.stopped_v2().await;
    let rule_id = w.rule.id.clone();
    w.state
        .jobs
        .db()
        .run::<_, trss_core::DbError, _>(move |c| {
            c.execute(
                "UPDATE rules SET directory = 'Show/Season 02' WHERE id = ?1",
                [rule_id],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    let row = w.failure().await;
    assert!(!offered(&row), "{row}");
    assert!(row["retry_blocked"].as_str().unwrap().contains("폴더"));
    let (status, body) = w.retry(&item).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(w.state.commands.get(COMMAND_ID).await.unwrap().is_none());
}
