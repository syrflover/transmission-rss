//! Replacing a video with a higher revision of the same release (ticket 0025,
//! `docs/specs/collection.md` 영상 수정본의 대체), end to end: the worker's
//! cycles and `다시 받기` against the fake Transmission, which acts on a
//! temporary media folder the way Transmission does (see
//! [`FakeTransmission::on_disk`]), and the web API on the same database.
//!
//! One test per row of the spec's table, then the ticket's own rows. Release
//! names, hashes and contents are made up; a name's CRC32 is the CRC32 of the
//! bytes its torrent writes, unless a test says otherwise.

mod common;

use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use transmission_rss::{
    revision::FileIdentity,
    store::{
        channels::{ChannelInput, RuleInput},
        history::{HistoryItem, HistoryResult, Observation},
        revisions::{Revision, RevisionState, RevisionStore},
        settings::SettingsStore,
    },
    worker::{revisions, CommandsOutcome, CycleReport, TickOutcome},
};

const OLD_HASH: &str = "1111000000000000000000000000000000000014";
const NEW_HASH: &str = "2222000000000000000000000000000000000014";
const OTHER_HASH: &str = "3333000000000000000000000000000000000014";
const OLD_BYTES: &[u8] = b"episode 14, first release";
const NEW_BYTES: &[u8] = b"episode 14, second release";
const EPISODE_NAME: &str = "Show S01E14.mkv";

/// The CRC32 of `bytes` as a release name writes it.
fn crc(bytes: &[u8]) -> String {
    format!("{:08X}", crc32fast::hash(bytes))
}

/// `[SubsPlease] Show - 14<version> (1080p) [<crc>].mkv`.
fn release(version: &str, crc: Option<&str>) -> String {
    match crc {
        Some(crc) => format!("[SubsPlease] Show - 14{version} (1080p) [{crc}].mkv"),
        None => format!("[SubsPlease] Show - 14{version} (1080p).mkv"),
    }
}

fn v1() -> String {
    release("", Some(&crc(OLD_BYTES)))
}

fn v2() -> String {
    release("v2", Some(&crc(NEW_BYTES)))
}

fn magnet(hash: &str, name: &str) -> String {
    let dn: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
    format!("magnet:?xt=urn:btih:{hash}&dn={dn}")
}

/// A feed of `(hash, title)` items.
fn feed(items: &[(&str, &str)]) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>Show</title><link>https://feeds.example.test/show</link><description>made up</description>"#,
    );
    for (hash, title) in items {
        let link = magnet(hash, title).replace('&', "&amp;");
        xml.push_str(&format!(
            "<item><title>{title}</title><link>{link}</link><guid isPermaLink=\"false\">guid-{hash}</guid></item>"
        ));
    }
    xml.push_str("</channel></rss>");
    xml
}

/// A harness whose collect folder is a temporary `media` folder the fake
/// Transmission acts on, with one channel (`show` feed) and its rule saving to
/// `Show/Season 01`.
struct Setup {
    h: Harness,
    season: PathBuf,
    channel_id: String,
}

impl Setup {
    async fn new() -> Setup {
        Setup::with_match("[SubsPlease] Show - ").await
    }

    async fn with_match(text: &str) -> Setup {
        let h = Harness::without_collect_folder().await;
        let media = h.dir.path().join("media");
        let season = media.join("Show").join("Season 01");
        std::fs::create_dir_all(&season).unwrap();
        SettingsStore::new(h.db.clone())
            .put_collection(0, media.to_str().unwrap().to_owned(), None)
            .await
            .unwrap();
        h.tr.on_disk(&media);
        let url = format!("{}?token={SECRET}", h.feeds.url("show"));
        let channel = h
            .channels
            .create_channel_with_rules(
                ChannelInput::new(url),
                vec![RuleInput {
                    r#match: Some(text.to_owned()),
                    directory: "Show/Season 01".to_owned(),
                    ..Default::default()
                }],
            )
            .await
            .unwrap();
        Setup {
            h,
            season,
            channel_id: channel.channel.id,
        }
    }

    fn feed(&self, items: &[(&str, &str)]) {
        self.h.feeds.set_xml("show", &feed(items));
    }

    fn file(&self, name: &str) -> PathBuf {
        self.season.join(name)
    }

    /// The names in the season folder, sorted.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.season)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    async fn cycle(&self) -> CycleReport {
        match self
            .h
            .worker()
            .tick(&CancellationToken::new())
            .await
            .unwrap()
        {
            TickOutcome::Ran(report) => report,
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    /// Transmission has all of the torrent `hash`.
    fn complete(&self, hash: &str) {
        self.h.tr.finish(hash);
        self.h.tr.set_status(hash, 6);
    }

    /// `14` received by a cycle and renamed to the episode name; its torrent
    /// stays in Transmission.
    async fn received_v1(&self) {
        self.feed(&[(OLD_HASH, &v1())]);
        self.h.tr.content_on_add(OLD_HASH, OLD_BYTES);
        self.cycle().await;
        self.complete(OLD_HASH);
        assert_eq!(self.names(), vec![EPISODE_NAME]);
        assert_eq!(self.h.tr.torrent(OLD_HASH).name, EPISODE_NAME);
    }

    /// The episode name holds `bytes`, a file of no torrent, and history knows
    /// the release `14` of the channel (seen in its feed earlier).
    async fn untracked_video(&self, bytes: &[u8]) {
        self.untracked_video_of(bytes, &v1()).await
    }

    /// [`Setup::untracked_video`] for the release named `title`.
    async fn untracked_video_of(&self, bytes: &[u8], title: &str) {
        std::fs::write(self.file(EPISODE_NAME), bytes).unwrap();
        self.h
            .history
            .record(
                1,
                vec![Observation {
                    channel_id: self.channel_id.clone(),
                    channel_label: "https://feeds.example.test/show".into(),
                    identity_key: "guid:v1".into(),
                    title: title.to_owned(),
                    link: magnet(OLD_HASH, title),
                    result: HistoryResult::NoMatch,
                    rule_id: None,
                    torrent_hash: None,
                    reason: None,
                }],
            )
            .await
            .unwrap();
    }

    async fn item(&self, title: &str) -> HistoryItem {
        self.h
            .history_items()
            .await
            .into_iter()
            .find(|i| i.title == title)
            .unwrap_or_else(|| panic!("no history item {title}"))
    }

    async fn state_of(&self, title: &str) -> RevisionState {
        let item = self.item(title).await;
        RevisionStore::new(self.h.db.clone())
            .by_item(item.id)
            .await
            .unwrap()
            .expect("a revision row")
            .state
    }

    async fn get(&self, uri: &str) -> Value {
        let (status, text, json) = self.h.web_api().call("GET", uri, None).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        json
    }

    /// The work detail's row of episode 14 of season 1.
    async fn episode_row(&self) -> Value {
        let work = RevisionStore::new(self.h.db.clone())
            .work_at(self.season.to_str().unwrap().to_owned())
            .await
            .unwrap()
            .expect("the work is in the library");
        let detail = self.get(&format!("/api/library/works/{}", work.id)).await;
        let season = detail["seasons"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["number"] == 1)
            .unwrap()
            .clone();
        season["episodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["episode"] == "14")
            .cloned()
            .expect("episode 14 has a row")
    }

    async fn failures(&self) -> Vec<Value> {
        self.get("/api/todo/receive-failures").await["items"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn added(&self, hash: &str) -> usize {
        self.h
            .tr
            .calls_of("torrent-add")
            .iter()
            .filter(|c| c.args["filename"].as_str().unwrap().contains(hash))
            .count()
    }

    fn renamed_onto_episode(&self, hash: &str) -> bool {
        self.h
            .tr
            .calls_of("torrent-rename-path")
            .iter()
            .any(|c| c.args["ids"][0] == hash && c.args["name"] == EPISODE_NAME)
    }
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

/// The revision failure in the to-do source, with its two files.
fn revision_failure(items: &[Value]) -> &Value {
    let found: Vec<&Value> = items.iter().filter(|i| i["kind"] == "revision").collect();
    assert_eq!(found.len(), 1, "{items:?}");
    found[0]
}

// --- the spec's table ----------------------------------------------------------------

/// Before ticket 0025 the cycle renamed `14v2` onto the episode name right
/// after adding it. With libtransmission's rename (the target exists, so the
/// file stays and the answer is `success`) that left both files, the old
/// video under the episode name and the new one under its release name, and
/// the new torrent naming the old video's file. The first part of this test,
/// run against the worker of `d4129fa` (asserting no rename after the add),
/// failed with both files in the folder
/// (`["Show S01E14.mkv", "[SubsPlease] Show - 14v2 (1080p) [8F2EFECC].mkv"]`)
/// and the new torrent named `Show S01E14.mkv`.
#[tokio::test]
async fn row_1_a_received_episode_is_replaced_by_its_revision_once_it_is_received_and_checked() {
    let s = Setup::new().await;
    s.received_v1().await;

    // `14v2` appears; it is still downloading after the cycle that adds it.
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.h.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.added(NEW_HASH), 1, "received automatically");
    assert!(
        !s.renamed_onto_episode(NEW_HASH),
        "not renamed after the add"
    );
    assert_eq!(s.names(), vec![EPISODE_NAME.to_owned(), v2()]);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert_eq!(s.h.tr.torrent(NEW_HASH).name, v2());
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    // Still downloading: nothing changes.
    s.cycle().await;
    assert_eq!(s.names(), vec![EPISODE_NAME.to_owned(), v2()]);

    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.h.tr.torrent(NEW_HASH).name, EPISODE_NAME);
    assert!(s.h.tr.torrents().iter().all(|t| t.hash != OLD_HASH));
    let removes = s.h.tr.calls_of("torrent-remove");
    assert_eq!(removes.len(), 1);
    assert_eq!(removes[0].args["ids"], json!([OLD_HASH]));
    assert_eq!(removes[0].args["delete-local-data"], true);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.item(&v2()).await.result, HistoryResult::Received);

    // The version line, quietly; no failure.
    let row = s.episode_row().await;
    assert_eq!(row["revision"]["from"], "v1");
    assert_eq!(row["revision"]["to"], "v2");
    assert_eq!(row["revision"]["replaced_at"], s.h.now());
    assert_eq!(row["failure"], Value::Null);
    assert_eq!(row["video"][0]["path"], format!("Season 01/{EPISODE_NAME}"));
    assert!(s.failures().await.is_empty());

    // `14` is still in the feed: it is not received again.
    let adds = s.added(OLD_HASH);
    s.cycle().await;
    assert_eq!(s.added(OLD_HASH), adds);
    assert!(s.h.tr.torrents().iter().all(|t| t.hash != OLD_HASH));
    assert_eq!(s.names(), vec![EPISODE_NAME]);
}

#[tokio::test]
async fn row_2_a_revision_whose_crc_differs_from_its_name_leaves_the_old_video() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, b"not what the name says");
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME.to_owned(), v2()]);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(s.h.tr.torrents().iter().any(|t| t.hash == OLD_HASH));
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);

    // `받기 실패` with both files and why, in the to-do source and on the row.
    let failures = s.failures().await;
    let failure = revision_failure(&failures);
    assert!(
        failure["reason"].as_str().unwrap().contains("CRC32"),
        "{failure}"
    );
    assert_eq!(failure["work"]["name"], "Show");
    assert_eq!(
        (failure["season"].clone(), failure["episode"].clone()),
        (json!(1), json!("14"))
    );
    assert_eq!(
        failure["files"],
        json!([
            { "role": "old", "path": format!("Season 01/{EPISODE_NAME}"), "state": "kept" },
            { "role": "new", "path": format!("Season 01/{}", v2()), "state": "received_name" },
        ])
    );
    let row = s.episode_row().await;
    assert_eq!(row["failure"]["files"], failure["files"]);
    assert_eq!(row["failure"]["reason"], failure["reason"]);
    assert_eq!(row["revision"], Value::Null);

    // The person deletes the new file: the failure goes away.
    std::fs::remove_file(s.file(&v2())).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Cleared);
    assert!(s.failures().await.is_empty());
}

#[tokio::test]
async fn row_2_a_revision_whose_download_stops_leaves_the_old_video() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    // The torrent is taken out of Transmission before it finished.
    s.h.tr.remove(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    let failures = s.failures().await;
    let failure = revision_failure(&failures);
    assert_eq!(failure["files"][0]["state"], "kept");
    assert_eq!(
        failure["files"][1],
        json!({ "role": "new", "path": null, "state": "not_received" })
    );
    // While it stays in the feed the item is received again, as any item a
    // rule picked and Transmission does not hold, and the replacement goes on.
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    assert_eq!(s.added(NEW_HASH), 2);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    assert!(s.failures().await.is_empty());
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

#[tokio::test]
async fn row_3_a_revision_without_a_crc_in_its_name_is_not_received() {
    let s = Setup::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    s.feed(&[(NEW_HASH, &v2), (OLD_HASH, &v1())]);
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.added(NEW_HASH), 0);
    let item = s.item(&v2).await;
    assert_eq!(item.result, HistoryResult::VersionUnknown);
    assert!(item.reason.unwrap().contains("CRC32"));
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    // `다시 받기` is offered on it.
    let view = s.get(&format!("/api/history/{}", item.id)).await;
    assert_eq!(view["result"], "version_unknown");
    assert_eq!(view["result_label"], "버전 미상");
    assert_eq!(view["can_retry"], true);
}

#[tokio::test]
async fn row_4_an_old_torrent_that_cannot_be_removed_keeps_both_files() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.h.tr.reject_remove_of(OLD_HASH, Some("permission denied"));
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME.to_owned(), v2()]);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(!s.renamed_onto_episode(NEW_HASH));
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    let failures = s.failures().await;
    let failure = revision_failure(&failures);
    assert!(failure["reason"]
        .as_str()
        .unwrap()
        .contains("permission denied"));
    assert_eq!(failure["files"][0]["state"], "kept");
    assert_eq!(failure["files"][1]["state"], "received_name");
    assert_eq!(s.episode_row().await["failure"]["files"], failure["files"]);
}

#[tokio::test]
async fn row_5_an_old_file_of_no_torrent_that_cannot_be_deleted_keeps_both_files() {
    let s = Setup::new().await;
    s.received_v1().await;
    // `14` left the feed and its torrent went (its data stays, as a cycle
    // removes departed torrents).
    s.h.tr.remove(OLD_HASH);
    s.feed(&[(NEW_HASH, &v2())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);

    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&s.season, std::fs::Permissions::from_mode(0o555)).unwrap();
    let writable = std::fs::write(s.season.join(".probe"), b"").is_ok();
    if !writable {
        s.cycle().await;
    }
    std::fs::set_permissions(&s.season, std::fs::Permissions::from_mode(0o755)).unwrap();
    if writable {
        // Root ignores the folder's mode, so the failure cannot be made here.
        eprintln!("skipped: the folder stays writable for this user");
        return;
    }

    assert_eq!(s.names(), vec![EPISODE_NAME.to_owned(), v2()]);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(!s.renamed_onto_episode(NEW_HASH));
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    let failures = s.failures().await;
    let failure = revision_failure(&failures);
    assert!(
        failure["reason"]
            .as_str()
            .unwrap()
            .contains("지우지 못했어요"),
        "{failure}"
    );
    assert_eq!(failure["files"][0]["state"], "kept");
}

#[tokio::test]
async fn row_6_an_old_video_in_a_batch_torrent_is_not_replaced() {
    let s = Setup::new().await;
    s.untracked_video(OLD_BYTES).await;
    std::fs::write(s.file("Show S01E13.mkv"), b"episode 13").unwrap();
    let batch = "4444000000000000000000000000000000000001";
    s.h.tr.preload(
        FakeTorrent::new(batch, "Show 01-14")
            .in_dir(&s.season)
            .files(&["Show S01E13.mkv", EPISODE_NAME])
            .status(6),
    );
    s.feed(&[(NEW_HASH, &v2())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(
        s.names(),
        vec!["Show S01E13.mkv".to_owned(), EPISODE_NAME.to_owned(), v2()]
    );
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(s.h.tr.torrents().iter().any(|t| t.hash == batch));
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    let failures = s.failures().await;
    let failure = revision_failure(&failures);
    assert!(
        failure["reason"]
            .as_str()
            .unwrap()
            .contains("파일 여러 개를 담은 토렌트"),
        "{failure}"
    );
}

#[tokio::test]
async fn row_7_a_restart_after_the_old_video_was_removed_finishes_the_rename() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    // The rename does not go through, as for a worker stopped right after
    // the old video went.
    s.h.tr.reject_rename_of(NEW_HASH, Some("busy"));
    s.cycle().await;

    assert_eq!(s.names(), vec![v2()]);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);
    let failures = s.failures().await;
    let failure = revision_failure(&failures);
    assert_eq!(failure["files"][0]["state"], "removed");
    assert_eq!(failure["files"][1]["state"], "received_name");
    // The episode has no file under its name now; its row says why.
    let row = s.episode_row().await;
    assert_eq!(row["video"], json!([]));
    assert_eq!(row["failure"]["files"][0]["state"], "removed");

    // A new worker on the same database (a restart).
    s.h.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert!(
        s.h.tr.torrents().iter().all(|t| t.hash != OLD_HASH),
        "the old video is not brought back"
    );
    assert!(s.failures().await.is_empty());
}

#[tokio::test]
async fn row_7_a_taken_episode_name_is_never_renamed_over() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.h.tr.reject_rename_of(NEW_HASH, Some("busy"));
    s.cycle().await;
    // Someone puts a file under the episode name before the rename.
    std::fs::write(s.file(EPISODE_NAME), b"someone else's").unwrap();
    s.h.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME.to_owned(), v2()]);
    assert_eq!(read(&s.file(EPISODE_NAME)), b"someone else's");
    assert_eq!(read(&s.file(&v2())), NEW_BYTES);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);
    let failures = s.failures().await;
    assert!(revision_failure(&failures)["reason"]
        .as_str()
        .unwrap()
        .contains("다른 파일"));
}

#[tokio::test]
async fn row_8_a_video_of_unknown_revision_equal_to_the_old_release_is_replaced() {
    let s = Setup::new().await;
    s.untracked_video(OLD_BYTES).await;
    s.feed(&[(NEW_HASH, &v2())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    assert_eq!(s.added(NEW_HASH), 1);
    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.episode_row().await["revision"]["from"], "v1");
}

#[tokio::test]
async fn row_8_a_video_of_unknown_revision_equal_to_the_newest_is_skipped() {
    let s = Setup::new().await;
    s.untracked_video(NEW_BYTES).await;
    s.feed(&[(NEW_HASH, &v2())]);
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.added(NEW_HASH), 0);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.item(&v2()).await.result, HistoryResult::Duplicate);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);
}

#[tokio::test]
async fn row_8_a_video_of_unknown_revision_equal_to_neither_is_version_unknown() {
    let s = Setup::new().await;
    s.untracked_video(b"some other cut").await;
    s.feed(&[(NEW_HASH, &v2())]);
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.added(NEW_HASH), 0);
    assert_eq!(read(&s.file(EPISODE_NAME)), b"some other cut");
    assert_eq!(s.item(&v2()).await.result, HistoryResult::VersionUnknown);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Unknown);
}

#[tokio::test]
async fn row_9_another_release_of_the_episode_is_held_as_a_duplicate_not_a_replacement() {
    let s = Setup::with_match("Show - 14").await;
    s.received_v1().await;
    // Another group's `14`, and a revision of another release of it (720p).
    let erai = format!("[Erai-raws] Show - 14 [1080p][{}].mkv", crc(b"erai"));
    let other_v2 = format!("[SubsPlease] Show - 14v2 (720p) [{}].mkv", crc(b"720p"));
    s.feed(&[
        (OTHER_HASH, &erai),
        (NEW_HASH, &other_v2),
        (OLD_HASH, &v1()),
    ]);
    s.h.tr.content_on_add(OTHER_HASH, b"erai");
    s.h.tr.content_on_add(NEW_HASH, b"720p");
    s.cycle().await;
    s.complete(OTHER_HASH);
    s.complete(NEW_HASH);
    s.cycle().await;

    let mut expected = vec![EPISODE_NAME.to_owned(), erai.clone(), other_v2.clone()];
    expected.sort();
    assert_eq!(s.names(), expected);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert!(!s.renamed_onto_episode(OTHER_HASH));
    assert!(!s.renamed_onto_episode(NEW_HASH));
    assert!(s.failures().await.is_empty());
    let revisions = RevisionStore::new(s.h.db.clone());
    for title in [&erai, &other_v2] {
        let item = s.item(title).await;
        assert_eq!(item.result, HistoryResult::Received);
        assert!(revisions.by_item(item.id).await.unwrap().is_none());
    }
}

// --- the ticket's own rows -----------------------------------------------------------

#[tokio::test]
async fn a_revision_without_a_crc_received_with_retry_replaces_without_the_check() {
    let s = Setup::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    s.feed(&[(NEW_HASH, &v2), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    let item = s.item(&v2).await;
    assert_eq!(item.result, HistoryResult::VersionUnknown);

    let api = s.h.web_api();
    let (status, text, _) = api
        .call(
            "POST",
            "/api/commands",
            Some(json!({
                "id": "00000000-0000-4000-8000-000000000025",
                "kind": "receive_once",
                "payload": { "item_id": item.id },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    let outcome =
        s.h.worker()
            .run_commands(&CancellationToken::new())
            .await
            .unwrap();
    assert_eq!(outcome, CommandsOutcome::Ran(1));

    assert_eq!(s.item(&v2).await.result, HistoryResult::Received);
    assert!(!s.renamed_onto_episode(NEW_HASH));
    assert_eq!(s.names(), vec![EPISODE_NAME.to_owned(), v2.clone()]);
    assert_eq!(s.state_of(&v2).await, RevisionState::Receiving);

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.state_of(&v2).await, RevisionState::Done);
    assert_eq!(s.episode_row().await["revision"]["to"], "v2");
}

#[tokio::test]
async fn an_add_failure_is_in_the_receive_failure_source_too() {
    let s = Setup::new().await;
    s.feed(&[(OLD_HASH, &v1())]);
    s.h.tr.reject_adds(Some("duplicate torrent? no: refused"));
    s.cycle().await;
    let items = s.failures().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "add_failed");
    assert_eq!(items[0]["title"], v1());
    assert!(items[0]["reason"].as_str().unwrap().contains("refused"));
}

// --- several revisions, several channels, and what a check cannot vouch for -------

const V3_HASH: &str = "5555000000000000000000000000000000000014";
const V3_BYTES: &[u8] = b"episode 14, third release";

fn v3() -> String {
    release("v3", Some(&crc(V3_BYTES)))
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

/// A feed of owned `(hash, title)` items.
fn feed_of(items: &[(String, String)]) -> String {
    feed(
        &items
            .iter()
            .map(|(h, t)| (h.as_str(), t.as_str()))
            .collect::<Vec<_>>(),
    )
}

impl Setup {
    /// A second channel whose rule saves the same release to the same folder,
    /// reading the feed `path` (empty until set).
    async fn second_channel(&self, path: &str) -> String {
        self.h.feeds.set_xml(path, &feed(&[]));
        let url = format!("{}?token={SECRET}", self.h.feeds.url(path));
        self.h
            .channels
            .create_channel_with_rules(
                ChannelInput::new(url),
                vec![RuleInput {
                    r#match: Some("[SubsPlease] Show - ".to_owned()),
                    directory: "Show/Season 01".to_owned(),
                    ..Default::default()
                }],
            )
            .await
            .unwrap()
            .channel
            .id
    }

    fn removed(&self, hash: &str) -> bool {
        self.h
            .tr
            .calls_of("torrent-remove")
            .iter()
            .any(|c| c.args["ids"] == json!([hash]))
    }

    /// Runs the pending commands.
    async fn commands(&self) -> CommandsOutcome {
        self.h
            .worker()
            .run_commands(&CancellationToken::new())
            .await
            .unwrap()
    }

    /// Accepts `다시 받기` of `item_id` as the command `id`.
    async fn retry(&self, item_id: i64, id: &str) {
        let (status, text, _) = self
            .h
            .web_api()
            .call(
                "POST",
                "/api/commands",
                Some(json!({
                    "id": id,
                    "kind": "receive_once",
                    "payload": { "item_id": item_id },
                })),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    }

    /// Runs `sql` on the database from another connection.
    fn sql(&self, sql: &str) {
        rusqlite::Connection::open(self.h.db_path())
            .unwrap()
            .execute_batch(sql)
            .unwrap();
    }
}

/// Two revisions of one release, both selected while `14` is in place: the
/// higher one finishes first and takes the episode name. The lower one,
/// finishing later, must not remove it (its row was decided against `14`).
#[tokio::test]
async fn a_lower_revision_finishing_after_a_higher_one_never_replaces_it() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.h.tr.content_on_add(V3_HASH, V3_BYTES);
    s.h.tr.unfinished_on_add(NEW_HASH);
    s.h.tr.unfinished_on_add(V3_HASH);
    s.cycle().await;
    assert_eq!((s.added(NEW_HASH), s.added(V3_HASH)), (1, 1));

    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);

    s.complete(NEW_HASH);
    s.cycle().await;
    s.cycle().await;

    assert_eq!(
        read(&s.file(EPISODE_NAME)),
        V3_BYTES,
        "the newest video stays"
    );
    assert!(s.h.tr.torrents().iter().any(|t| t.hash == V3_HASH));
    assert!(!s.removed(V3_HASH));
    assert_eq!(s.names(), sorted(vec![EPISODE_NAME.to_owned(), v2()]));
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);
    assert!(s.failures().await.is_empty());
}

/// Both revisions finish in the same cycle: whichever the worker looks at
/// first, the higher one ends under the episode name.
#[tokio::test]
async fn two_revisions_finishing_together_leave_the_higher_one() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.h.tr.content_on_add(V3_HASH, V3_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.complete(V3_HASH);
    s.cycle().await;
    s.cycle().await;

    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert!(s.h.tr.torrents().iter().any(|t| t.hash == V3_HASH));
    assert!(s.failures().await.is_empty());
}

/// `14v3` replaced `14`, and its torrent has since left Transmission. `14v2`
/// appearing now is lower than the folder's video: it is skipped, not left
/// to `다시 받기` as a video of unknown revision.
#[tokio::test]
async fn a_lower_revision_after_a_higher_one_is_skipped_without_its_torrent_too() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(V3_HASH, V3_BYTES);
    s.cycle().await;
    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);

    s.h.tr.remove(V3_HASH);
    s.feed(&[(NEW_HASH, &v2())]);
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.added(NEW_HASH), 0);
    let item = s.item(&v2()).await;
    assert_eq!(item.result, HistoryResult::Duplicate);
    assert_eq!(item.reason.as_deref(), Some(revisions::NOT_HIGHER));
    assert!(RevisionStore::new(s.h.db.clone())
        .by_item(item.id)
        .await
        .unwrap()
        .is_none_or(|row| row.state == RevisionState::Skipped));
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
}

/// The same release reaches the folder through two channels (one torrent).
/// Once `14v2` replaced it, neither channel's `14` brings it back.
#[tokio::test]
async fn the_old_release_in_another_channel_is_not_received_again() {
    let s = Setup::new().await;
    s.second_channel("show2").await;
    s.received_v1().await;
    s.h.feeds.set_xml("show2", &feed(&[(OLD_HASH, &v1())]));
    s.cycle().await;
    let firsts = s.h.history_items().await;
    assert_eq!(firsts.iter().filter(|i| i.title == v1()).count(), 2);

    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert!(s.h.tr.torrents().iter().all(|t| t.hash != OLD_HASH));

    let adds = s.added(OLD_HASH);
    s.cycle().await;
    s.cycle().await;
    assert_eq!(
        s.added(OLD_HASH),
        adds,
        "the old release is not added again"
    );
    assert!(s.h.tr.torrents().iter().all(|t| t.hash != OLD_HASH));
    assert_eq!(s.names(), vec![EPISODE_NAME]);
}

/// `14v2` in two channels is one torrent: it replaces `14` once, and the
/// other channel's item is no failure.
#[tokio::test]
async fn the_same_revision_in_two_channels_replaces_once_without_a_failure() {
    let s = Setup::new().await;
    s.second_channel("show2").await;
    s.received_v1().await;
    let both = vec![(NEW_HASH.to_owned(), v2()), (OLD_HASH.to_owned(), v1())];
    s.h.feeds.set_xml("show", &feed_of(&both));
    s.h.feeds.set_xml("show2", &feed_of(&both));
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    let failures = s.failures().await;
    assert!(failures.is_empty(), "{failures:?}");
}

/// `다시 받기` of a `버전 미상` revision whose confirmation cannot be written:
/// the command is not ended as if it had been, and its next run starts the
/// replacement.
#[tokio::test]
async fn a_retry_whose_confirmation_is_not_written_runs_again_and_replaces() {
    let s = Setup::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    s.feed(&[(NEW_HASH, &v2), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    let item = s.item(&v2).await;
    assert_eq!(item.result, HistoryResult::VersionUnknown);

    s.sql(
        "CREATE TRIGGER no_confirm BEFORE UPDATE ON video_revisions
         BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    s.retry(item.id, "00000000-0000-4000-8000-000000000251")
        .await;
    s.commands().await;
    s.sql("DROP TRIGGER no_confirm;");
    s.commands().await;
    assert!(!s.renamed_onto_episode(NEW_HASH));

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.item(&v2).await.result, HistoryResult::Received);
}

/// The same, with the history write failing after the confirmation.
#[tokio::test]
async fn a_retry_whose_result_is_not_written_runs_again_and_replaces() {
    let s = Setup::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    s.feed(&[(NEW_HASH, &v2), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    let item = s.item(&v2).await;

    s.sql(
        "CREATE TRIGGER no_result BEFORE UPDATE OF result ON history_items
         BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    s.retry(item.id, "00000000-0000-4000-8000-000000000252")
        .await;
    s.commands().await;
    s.sql("DROP TRIGGER no_result;");
    s.commands().await;
    assert!(!s.renamed_onto_episode(NEW_HASH));

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.item(&v2).await.result, HistoryResult::Received);
}

/// `[SubsPlease] Show - <episode><version> (1080p) [<crc>].mkv` and its hash.
fn episode_release(episode: u32, version: &str, bytes: &[u8]) -> String {
    format!(
        "[SubsPlease] Show - {episode}{version} (1080p) [{}].mkv",
        crc(bytes)
    )
}

/// Transmission's whole file list grows with everything it holds; a cycle
/// asks for it at most once, and not at all for revisions whose own torrent
/// holds the episode name.
#[tokio::test]
async fn a_cycle_reads_transmissions_whole_file_list_at_most_once() {
    let episodes = [11_u32, 12, 13];
    let release_of = |prefix: &str, episode: u32, version: &str| {
        let bytes = format!("episode {episode}, {version}");
        (
            format!("{prefix}{episode:0>36}"),
            episode_release(episode, version, bytes.as_bytes()),
            bytes,
        )
    };
    let firsts: Vec<_> = episodes
        .iter()
        .map(|&e| release_of("1111", e, ""))
        .collect();
    let seconds: Vec<_> = episodes
        .iter()
        .map(|&e| release_of("2222", e, "v2"))
        .collect();
    let items = |of: &[&Vec<(String, String, String)>]| -> Vec<(String, String)> {
        of.iter()
            .flat_map(|v| v.iter().map(|(h, t, _)| (h.clone(), t.clone())))
            .collect()
    };

    let s = Setup::new().await;
    for (hash, _, bytes) in firsts.iter().chain(&seconds) {
        s.h.tr.content_on_add(hash, bytes.as_bytes());
    }
    for (hash, _, _) in &seconds {
        s.h.tr.unfinished_on_add(hash);
    }
    s.h.feeds.set_xml("show", &feed_of(&items(&[&firsts])));
    s.cycle().await;
    for (hash, _, _) in &firsts {
        s.complete(hash);
    }
    assert_eq!(s.names().len(), 3);

    // Three revisions to decide in one cycle.
    s.h.feeds
        .set_xml("show", &feed_of(&items(&[&seconds, &firsts])));
    s.h.tr.clear_calls();
    s.cycle().await;
    for (hash, _, _) in &seconds {
        assert_eq!(s.added(hash), 1);
    }
    let listings = s.h.tr.full_file_listings();
    assert!(listings <= 1, "{listings} full listings in one cycle");

    // Steady state: revisions received while their episode had no video hold
    // the name with their own torrent.
    let s = Setup::new().await;
    for (hash, _, bytes) in &seconds {
        s.h.tr.content_on_add(hash, bytes.as_bytes());
    }
    s.h.feeds.set_xml("show", &feed_of(&items(&[&seconds])));
    s.cycle().await;
    for (hash, _, _) in &seconds {
        s.complete(hash);
    }
    assert_eq!(s.names().len(), 3);
    s.h.tr.clear_calls();
    s.cycle().await;
    assert_eq!(s.h.tr.full_file_listings(), 0);
}

#[tokio::test]
async fn a_revision_whose_torrent_reports_a_local_error_goes_on_once_it_clears() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.h.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    s.h.tr
        .set_local_error(NEW_HASH, Some("No space left on device"));
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    let failures = s.failures().await;
    assert!(revision_failure(&failures)["reason"]
        .as_str()
        .unwrap()
        .contains("No space left"));

    s.h.tr.set_local_error(NEW_HASH, None);
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert!(s.failures().await.is_empty());
}

#[tokio::test]
async fn a_revision_received_outside_the_rule_folder_stays_a_failure() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.h.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    let elsewhere = s.season.parent().unwrap().join("Elsewhere");
    s.h.tr.relocate(NEW_HASH, &elsewhere);
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert_eq!(s.failures().await.len(), 1);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
}

/// Transmission names the old video's folder another way (through a
/// symbolic link): the torrent is still found to hold the file, and removed
/// with it, instead of the file being deleted under it.
#[tokio::test]
async fn an_old_torrent_whose_folder_is_spelled_another_way_is_removed_with_its_file() {
    let s = Setup::new().await;
    s.received_v1().await;
    let link = s.season.parent().unwrap().join("Season 01 link");
    std::os::unix::fs::symlink(&s.season, &link).unwrap();
    s.h.tr.set_download_dir(OLD_HASH, &link);
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert!(s.removed(OLD_HASH), "the old torrent is removed");
    assert!(s.h.tr.torrents().iter().all(|t| t.hash != OLD_HASH));
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// Two torrents name the old video's file: removing either could take the
/// other's data, so the old video is not replaced.
#[tokio::test]
async fn an_old_video_two_torrents_hold_is_not_removed() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.h.tr.preload(
        FakeTorrent::new(OTHER_HASH, EPISODE_NAME)
            .in_dir(&s.season)
            .status(6),
    );
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(s.h.tr.torrents().iter().any(|t| t.hash == OLD_HASH));
}

/// A revision whose received file is empty does not replace the old video,
/// even though the name's CRC32 (`00000000`) matches.
#[tokio::test]
async fn an_empty_revision_does_not_replace_the_old_video() {
    let s = Setup::new().await;
    s.received_v1().await;
    let empty = release("v2", Some(&crc(b"")));
    s.feed(&[(NEW_HASH, &empty), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, b"");
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.state_of(&empty).await, RevisionState::Failed);
}

/// A confirmed revision whose torrent names the episode file itself (the old
/// video, as a rename before ticket 0025 left such torrents) is not a new
/// video: nothing is removed.
#[tokio::test]
async fn a_revision_whose_torrent_names_the_episode_file_removes_nothing() {
    let s = Setup::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    let link = magnet(NEW_HASH, EPISODE_NAME).replace('&', "&amp;");
    let item = format!(
        r#"<item><title>{v2}</title><link>{link}</link><guid isPermaLink="false">guid-{NEW_HASH}</guid></item>"#
    );
    // `14` stays in the feed, so its torrent is not removed as departed.
    let xml = feed(&[(OLD_HASH, &v1())]).replace("</channel>", &format!("{item}</channel>"));
    s.h.feeds.set_xml("show", &xml);
    s.cycle().await;
    let item = s.item(&v2).await;
    assert_eq!(item.result, HistoryResult::VersionUnknown);
    s.retry(item.id, "00000000-0000-4000-8000-000000000253")
        .await;
    s.commands().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(s.h.tr.torrents().iter().any(|t| t.hash == OLD_HASH));
}

// --- what a step finds when an earlier one went further than it wrote --------------

/// The rename went through but `done` was not written (the worker stopped, or
/// the write failed): the next look finds the new video under the episode
/// name and finishes, instead of failing on a name its own video holds.
#[tokio::test]
async fn a_rename_whose_done_was_not_written_finishes_on_the_next_look() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.sql(
        "CREATE TRIGGER no_done BEFORE UPDATE OF state ON video_revisions
         WHEN NEW.state = 'done' BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    s.cycle().await;
    s.sql("DROP TRIGGER no_done;");
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);

    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert!(s.failures().await.is_empty());
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.episode_row().await["revision"]["to"], "v2");

    // A higher revision of the episode is not held up behind it.
    let v3 = v3();
    s.feed(&[(V3_HASH, &v3)]);
    s.h.tr.content_on_add(V3_HASH, V3_BYTES);
    s.cycle().await;
    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v3).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
}

/// The same when the new torrent has left Transmission since: the file
/// under the episode name is told by its CRC32.
#[tokio::test]
async fn a_rename_whose_done_was_not_written_finishes_without_its_torrent_too() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.sql(
        "CREATE TRIGGER no_done BEFORE UPDATE OF state ON video_revisions
         WHEN NEW.state = 'done' BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    s.cycle().await;
    s.sql("DROP TRIGGER no_done;");
    s.h.tr.remove(NEW_HASH);

    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert!(s.failures().await.is_empty());
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// A file put over the old video (an atomic rename by another program) while
/// the worker reads the old video's CRC32 is not deleted: the CRC32 belongs
/// to the file that was read, not to the one at the name now. The episode
/// name is a pipe here so the test can swap the file while the read is open.
#[tokio::test]
async fn a_file_put_over_the_old_video_while_it_is_read_is_not_deleted() {
    let s = Setup::new().await;
    s.received_v1().await;
    // A file of no torrent, which is deleted by path once its CRC32 matches.
    s.h.tr.remove(OLD_HASH);
    s.feed(&[(NEW_HASH, &v2())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    let episode = s.file(EPISODE_NAME);
    std::fs::remove_file(&episode).unwrap();
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &episode,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o644),
        0,
    )
    .unwrap();
    let incoming = s.file(".incoming");
    std::fs::write(&incoming, b"someone else's").unwrap();
    s.complete(NEW_HASH);

    let swap = {
        let episode = episode.clone();
        tokio::task::spawn_blocking(move || {
            use std::{io::Write, os::unix::fs::OpenOptionsExt};
            let begun = std::time::Instant::now();
            // Waits for the worker to open the pipe for its read.
            let mut pipe = loop {
                match std::fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
                    .open(&episode)
                {
                    Ok(pipe) => break pipe,
                    Err(_) if begun.elapsed() < std::time::Duration::from_secs(30) => {
                        std::thread::sleep(std::time::Duration::from_millis(5))
                    }
                    Err(err) => panic!("the worker never read the old video: {err}"),
                }
            };
            std::fs::rename(&incoming, &episode).unwrap();
            pipe.write_all(OLD_BYTES).unwrap();
        })
    };
    let (_, swapped) = tokio::join!(s.cycle(), swap);
    swapped.unwrap();
    // A read the swap disturbed may be left to the next look.
    s.cycle().await;

    assert_eq!(read(&episode), b"someone else's", "the new file is kept");
    assert_eq!(read(&s.file(&v2())), NEW_BYTES);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    let failures = s.failures().await;
    assert_eq!(revision_failure(&failures)["files"][0]["state"], "kept");
}

/// The old video is gone after the replacement was claimed (`removing` is
/// written) but before the name is looked at again: Transmission finished an
/// earlier removal, or the person deleted it. That is the old video removed:
/// the replacement goes on to name the new video and the old release stays
/// superseded, instead of failing as changed and being cleared.
#[tokio::test]
async fn an_old_video_gone_after_the_claim_still_ends_in_the_replacement() {
    let s = Setup::new().await;
    // A file of no torrent whose CRC32 is its name's: an empty file's, so a
    // pipe the worker reads to its end with nothing written gives it.
    let old = release("", Some("00000000"));
    s.untracked_video_of(b"", &old).await;
    s.feed(&[(NEW_HASH, &v2())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    let episode = s.file(EPISODE_NAME);
    std::fs::remove_file(&episode).unwrap();
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &episode,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o644),
        0,
    )
    .unwrap();
    s.complete(NEW_HASH);

    let db_path = s.h.db_path();
    let gone = {
        let episode = episode.clone();
        tokio::task::spawn_blocking(move || {
            use std::os::unix::fs::OpenOptionsExt;
            let begun = std::time::Instant::now();
            // Waits for the worker to open the pipe for its read.
            let pipe = loop {
                match std::fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
                    .open(&episode)
                {
                    Ok(pipe) => break pipe,
                    Err(_) if begun.elapsed() < std::time::Duration::from_secs(30) => {
                        std::thread::sleep(std::time::Duration::from_millis(5))
                    }
                    Err(err) => panic!("the worker never read the old video: {err}"),
                }
            };
            // The worker's writes of the cycle are done up to the claim of the
            // replacement, the next one; it waits for this lock.
            let db = rusqlite::Connection::open(&db_path).unwrap();
            db.busy_timeout(std::time::Duration::from_secs(10)).unwrap();
            db.execute_batch("BEGIN IMMEDIATE").unwrap();
            // The end of the pipe ends the worker's read.
            drop(pipe);
            std::thread::sleep(std::time::Duration::from_millis(500));
            std::fs::remove_file(&episode).unwrap();
            db.execute_batch("COMMIT").unwrap();
        })
    };
    let (_, gone) = tokio::join!(s.cycle(), gone);
    gone.unwrap();
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&episode), NEW_BYTES);
    assert!(s.failures().await.is_empty());
    let row = RevisionStore::new(s.h.db.clone())
        .by_item(s.item(&v2()).await.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.old_item_id, Some(s.item(&old).await.id));
}

/// Transmission removed the old torrent and its data, but its answer never
/// came (a timeout): the replacement goes on as removing, finds the old video
/// gone, and names the new one; the old release is not received again.
#[tokio::test]
async fn an_old_torrent_removed_without_an_answer_still_ends_in_the_replacement() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.h.tr.break_remove_answer_of(OLD_HASH);
    s.cycle().await;
    assert!(s.h.tr.torrents().iter().all(|t| t.hash != OLD_HASH));
    let adds = s.added(OLD_HASH);
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(
        s.added(OLD_HASH),
        adds,
        "the old release is not received again"
    );
    assert!(s.failures().await.is_empty());
    // Every item of the removed torrent stays superseded.
    let row = RevisionStore::new(s.h.db.clone())
        .by_item(s.item(&v2()).await.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.old_torrent_hash.as_deref(), Some(OLD_HASH));
}

/// Transmission took the old torrent out but its file is still there (it
/// deletes data after answering, or could not), and the old release's name
/// carries no CRC32 to check the file by. The replacement stays removing and
/// says so, the old release is not received again, and once the file is gone
/// the new video takes the name.
#[tokio::test]
async fn an_old_file_left_after_its_torrent_was_removed_waits_as_removing() {
    let s = Setup::new().await;
    let v1 = release("", None);
    s.feed(&[(OLD_HASH, &v1)]);
    s.h.tr.content_on_add(OLD_HASH, OLD_BYTES);
    s.cycle().await;
    s.complete(OLD_HASH);
    assert_eq!(s.names(), vec![EPISODE_NAME]);

    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1)]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.h.tr.keep_data_on_remove_of(OLD_HASH);
    s.cycle().await;
    let adds = s.added(OLD_HASH);
    s.cycle().await;
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.state_of(&v2()).await, RevisionState::Removing);
    let failures = s.failures().await;
    let failure = revision_failure(&failures);
    assert_eq!(failure["files"][0]["state"], "kept");
    assert_eq!(s.added(OLD_HASH), adds);

    // Transmission (or the person) deletes it at last.
    std::fs::remove_file(s.file(EPISODE_NAME)).unwrap();
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(
        s.added(OLD_HASH),
        adds,
        "the old release is not received again"
    );
    assert!(s.failures().await.is_empty());
}

/// `14v3` failed before it was received, then `14v2` removed the old video
/// and its rename is still to go through: the empty episode name is not the
/// person having resolved `14v3`'s failure.
#[tokio::test]
async fn a_higher_revision_not_received_is_not_cleared_while_a_lower_one_is_renamed() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.h.tr.unfinished_on_add(NEW_HASH);
    s.h.tr.unfinished_on_add(V3_HASH);
    s.cycle().await;
    // `14v3` stops: its torrent is taken out and it leaves the feed.
    s.h.tr.remove(V3_HASH);
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);

    s.complete(NEW_HASH);
    s.h.tr.reject_rename_of(NEW_HASH, Some("busy"));
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);

    s.h.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);
}

/// The old video is the single file of a torrent that keeps it in a folder
/// (`Season 01/Show S01E14.mkv` under the work folder): removing that torrent
/// with its data could take the folder with it, so it is not replaced.
#[tokio::test]
async fn an_old_video_inside_its_torrents_folder_is_not_removed() {
    let s = Setup::new().await;
    std::fs::write(s.file(EPISODE_NAME), OLD_BYTES).unwrap();
    let show = s.season.parent().unwrap();
    s.h.tr.preload(
        FakeTorrent::new(OLD_HASH, "Season 01")
            .in_dir(show)
            .files(&[&format!("Season 01/{EPISODE_NAME}")])
            .status(6),
    );
    s.h.history
        .record(
            1,
            vec![Observation {
                channel_id: s.channel_id.clone(),
                channel_label: "https://feeds.example.test/show".into(),
                identity_key: "guid:v1".into(),
                title: v1(),
                link: magnet(OLD_HASH, &v1()),
                result: HistoryResult::Received,
                rule_id: None,
                torrent_hash: Some(OLD_HASH.into()),
                reason: None,
            }],
        )
        .await
        .unwrap();
    s.feed(&[(NEW_HASH, &v2())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert!(s.h.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
}

/// `14v3` replaces the video and skips `14v2` in the same pass; the pass then
/// reaches `14v2` from what it read before. Its stale look (a CRC32 that
/// does not match) must not turn the skip into a failure.
#[tokio::test]
async fn a_revision_skipped_earlier_in_the_same_pass_stays_skipped() {
    let s = Setup::new().await;
    s.received_v1().await;
    // `14v3`'s row comes first, so the pass looks at it first.
    s.feed(&[(V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(V3_HASH, V3_BYTES);
    s.h.tr.unfinished_on_add(V3_HASH);
    s.cycle().await;
    s.feed(&[(NEW_HASH, &v2()), (V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, b"not what the name says");
    s.h.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    s.complete(V3_HASH);
    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);
    assert!(s.failures().await.is_empty());
}

/// A `버전 미상` decision is written with its history record: an item that
/// history holds as `버전 미상` always has the decision `다시 받기` needs.
#[tokio::test]
async fn a_version_unknown_item_is_recorded_with_its_decision() {
    let s = Setup::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    s.feed(&[(NEW_HASH, &v2), (OLD_HASH, &v1())]);
    s.sql(
        "CREATE TRIGGER no_row BEFORE INSERT ON video_revisions
         BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    s.cycle().await;
    s.sql("DROP TRIGGER no_row;");
    let revisions = RevisionStore::new(s.h.db.clone());
    let unknown =
        s.h.history_items()
            .await
            .into_iter()
            .find(|i| i.title == v2 && i.result == HistoryResult::VersionUnknown);
    if let Some(item) = unknown {
        assert!(
            revisions.by_item(item.id).await.unwrap().is_some(),
            "버전 미상 without its decision"
        );
    }

    s.cycle().await;
    assert_eq!(s.item(&v2).await.result, HistoryResult::VersionUnknown);
    assert_eq!(s.state_of(&v2).await, RevisionState::Unknown);
}

// --- a revision's name on the normal path ----------------------------------------

const ERAI_HASH: &str = "6666000000000000000000000000000000000006";
const ERAI_EPISODE: &str = "Show S01E06.mkv";

/// `[Erai-raws] Show - 06<version> [1080p CR WEBRip HEVC AAC][MultiSub][<crc>].mkv`:
/// `trname` alone reads `06v2` of this name as another episode, taken from
/// the CRC32 bracket.
fn erai(version: &str) -> String {
    format!(
        "[Erai-raws] Show - 06{version} [1080p CR WEBRip HEVC AAC][MultiSub][{}].mkv",
        crc(NEW_BYTES)
    )
}

/// A revision seen first (no earlier release of it in the folder) is received
/// as any release and named as its episode, not as `trname` reads its marker.
#[tokio::test]
async fn a_revision_seen_first_is_named_as_its_episode() {
    let s = Setup::with_match("[Erai-raws] Show - ").await;
    let title = erai("v2");
    s.feed(&[(ERAI_HASH, &title)]);
    s.h.tr.content_on_add(ERAI_HASH, NEW_BYTES);
    s.cycle().await;
    assert_eq!(s.names(), vec![ERAI_EPISODE]);
    assert_eq!(s.h.tr.torrent(ERAI_HASH).name, ERAI_EPISODE);
    // History keeps the release's own title.
    assert_eq!(s.item(&title).await.result, HistoryResult::Received);
}

#[tokio::test]
async fn a_revision_received_with_retry_is_named_as_its_episode() {
    let s = Setup::with_match("[Erai-raws] Show - ").await;
    let title = erai("v2");
    s.feed(&[(ERAI_HASH, &title)]);
    s.h.tr.reject_adds(Some("refused"));
    s.cycle().await;
    assert_eq!(s.item(&title).await.result, HistoryResult::AddFailed);
    s.h.tr.reject_adds(None);
    s.feed(&[]);

    s.h.tr.content_on_add(ERAI_HASH, NEW_BYTES);
    let item = s.item(&title).await;
    s.retry(item.id, "00000000-0000-4000-8000-000000000611")
        .await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    assert_eq!(s.item(&title).await.result, HistoryResult::Received);
    assert_eq!(s.names(), vec![ERAI_EPISODE]);
}

// --- `다시 받기` of a revision whose download stopped --------------------------------

/// `14v2` received while `14` is in place, its torrent taken out of
/// Transmission before it finished, and the release gone from the feed: no
/// cycle receives it again.
async fn stopped_after_leaving_the_feed(s: &Setup) -> HistoryItem {
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    s.h.tr.remove(NEW_HASH);
    s.feed(&[(OLD_HASH, &v1())]);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    s.cycle().await;
    assert_eq!(s.added(NEW_HASH), 1, "no cycle receives it again");
    s.item(&v2()).await
}

#[tokio::test]
async fn a_stopped_revision_that_left_the_feed_is_received_again_with_retry() {
    let s = Setup::new().await;
    let item = stopped_after_leaving_the_feed(&s).await;
    assert_eq!(item.result, HistoryResult::Received);

    // Both places that show the failure offer `다시 받기` on the item.
    let failures = s.failures().await;
    let failure = revision_failure(&failures);
    assert_eq!(failure["history_item_id"], item.id);
    assert_eq!(failure["can_retry"], true);
    assert_eq!(failure["command"], Value::Null);
    let row = s.episode_row().await;
    assert_eq!(row["failure"]["can_retry"], true);
    assert_eq!(row["failure"]["history_item_id"], item.id);

    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.h.tr.unfinished_on_add(NEW_HASH);
    s.retry(item.id, "00000000-0000-4000-8000-000000000a01")
        .await;
    // Accepted and not yet run: the failure says so.
    assert_eq!(
        revision_failure(&s.failures().await)["command"]["state"],
        "pending"
    );
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));

    // Received under its own name; the old video stays until it is checked.
    assert_eq!(s.added(NEW_HASH), 2);
    assert!(!s.renamed_onto_episode(NEW_HASH));
    assert_eq!(s.item(&v2()).await.result, HistoryResult::Received);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    assert!(s.failures().await.is_empty());
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
}

#[tokio::test]
async fn a_revision_whose_torrent_reports_an_error_is_started_again_with_retry() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.h.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    s.h.tr
        .set_local_error(NEW_HASH, Some("No space left on device"));
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert_eq!(revision_failure(&s.failures().await)["can_retry"], true);

    let item = s.item(&v2()).await;
    s.retry(item.id, "00000000-0000-4000-8000-000000000a02")
        .await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    // Transmission still had it: started again, not added a second time.
    assert_eq!(
        s.h.tr
            .calls_of("torrent-start")
            .iter()
            .filter(|c| c.args["ids"] == json!([NEW_HASH]))
            .count(),
        1
    );
    assert_eq!(s.item(&v2()).await.result, HistoryResult::Received);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    assert!(!s.renamed_onto_episode(NEW_HASH));

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// A revision whose retry cannot be added keeps its item's result and its
/// failure; only the command says why.
#[tokio::test]
async fn a_stopped_revision_whose_retry_is_refused_stays_a_failure() {
    let s = Setup::new().await;
    let item = stopped_after_leaving_the_feed(&s).await;
    s.h.tr.reject_adds(Some("refused"));
    s.retry(item.id, "00000000-0000-4000-8000-000000000a03")
        .await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    let (_, _, command) =
        s.h.web_api()
            .call(
                "GET",
                "/api/commands/00000000-0000-4000-8000-000000000a03",
                None,
            )
            .await;
    assert_eq!(command["state"], "failed", "{command}");
    assert!(command["outcome"]["reason"]
        .as_str()
        .unwrap()
        .contains("refused"));
    assert_eq!(s.item(&v2()).await.result, HistoryResult::Received);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert_eq!(revision_failure(&s.failures().await)["can_retry"], true);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
}

/// Only a download that stopped is offered again: a revision received into
/// another folder would end the same way.
#[tokio::test]
async fn a_revision_received_elsewhere_is_not_offered_again() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.h.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    let elsewhere = s.season.parent().unwrap().join("Elsewhere");
    s.h.tr.relocate(NEW_HASH, &elsewhere);
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    let failure = revision_failure(&s.failures().await).clone();
    assert_eq!(failure["can_retry"], false);
    assert_eq!(failure["retry_blocked"], Value::Null);

    let item = s.item(&v2()).await;
    let (status, text, _) =
        s.h.web_api()
            .call(
                "POST",
                "/api/commands",
                Some(json!({
                    "id": "00000000-0000-4000-8000-000000000a04",
                    "kind": "receive_once",
                    "payload": { "item_id": item.id },
                })),
            )
            .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
}

/// A stopped revision whose rule is paused says why `다시 받기` is missing,
/// and the request is refused.
#[tokio::test]
async fn a_stopped_revision_of_a_paused_rule_says_why_it_is_not_offered() {
    let s = Setup::new().await;
    let item = stopped_after_leaving_the_feed(&s).await;
    let rule_id = item.rule_id.clone().unwrap();
    s.sql(&format!(
        "UPDATE rules SET state = 'paused' WHERE id = '{rule_id}';"
    ));
    let failure = revision_failure(&s.failures().await).clone();
    assert_eq!(failure["can_retry"], false);
    assert!(failure["retry_blocked"].as_str().unwrap().contains("멈춰"));
}

/// The rule's folder changed after the revision's replacement was decided: a
/// torrent added now would be received into the new folder, away from the
/// video it replaces, and the replacement would fail again with nothing more
/// to try. `다시 받기` is not offered, and a request accepted before the change
/// ends without adding anything.
#[tokio::test]
async fn a_stopped_revision_of_a_rule_whose_folder_changed_is_not_received_again() {
    let s = Setup::new().await;
    let item = stopped_after_leaving_the_feed(&s).await;
    let rule_id = item.rule_id.clone().unwrap();
    // Accepted while the folder is the same.
    s.retry(item.id, "00000000-0000-4000-8000-000000000a05")
        .await;
    s.sql(&format!(
        "UPDATE rules SET directory = 'Show/Season 02' WHERE id = '{rule_id}';"
    ));

    let failure = revision_failure(&s.failures().await).clone();
    assert_eq!(failure["can_retry"], false);
    assert!(failure["retry_blocked"].as_str().unwrap().contains("폴더"));
    let (status, text, _) =
        s.h.web_api()
            .call(
                "POST",
                "/api/commands",
                Some(json!({
                    "id": "00000000-0000-4000-8000-000000000a06",
                    "kind": "receive_once",
                    "payload": { "item_id": item.id },
                })),
            )
            .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");

    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    let (_, _, command) =
        s.h.web_api()
            .call(
                "GET",
                "/api/commands/00000000-0000-4000-8000-000000000a05",
                None,
            )
            .await;
    assert_eq!(command["state"], "failed", "{command}");
    assert!(command["outcome"]["reason"]
        .as_str()
        .unwrap()
        .contains("폴더"));
    assert_eq!(s.added(NEW_HASH), 1, "nothing was added");
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
}

/// SubsPlease's `14v2` seen first is named as episode 14.
#[tokio::test]
async fn a_subsplease_revision_seen_first_is_named_as_its_episode() {
    let s = Setup::new().await;
    s.feed(&[(NEW_HASH, &v2())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    assert_eq!(s.names(), vec![EPISODE_NAME]);
}

/// A show named with `NvM` (`Show 3v3`) whose first release of an episode
/// carries no revision marker: that release is the episode's first revision
/// (not revision 3 of another release), received and named as any, and its
/// `06v2` replaces it.
#[tokio::test]
async fn a_revision_of_a_show_named_with_a_number_v_number_replaces_its_first_release() {
    let s = Setup::with_match("[SubsPlease] Show 3v3 - ").await;
    let first = format!(
        "[SubsPlease] Show 3v3 - 06 (1080p) [{}].mkv",
        crc(OLD_BYTES)
    );
    let second = format!(
        "[SubsPlease] Show 3v3 - 06v2 (1080p) [{}].mkv",
        crc(NEW_BYTES)
    );
    let episode = "Show S01E06.mkv";
    s.feed(&[(OLD_HASH, &first)]);
    s.h.tr.content_on_add(OLD_HASH, OLD_BYTES);
    s.cycle().await;
    s.complete(OLD_HASH);
    assert_eq!(s.names(), vec![episode]);
    assert_eq!(s.h.tr.torrent(OLD_HASH).name, episode);

    s.feed(&[(NEW_HASH, &second), (OLD_HASH, &first)]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    assert_eq!(s.added(NEW_HASH), 1);
    assert_eq!(s.state_of(&second).await, RevisionState::Receiving);
    assert_eq!(s.names(), sorted(vec![episode.to_owned(), second.clone()]));

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&second).await, RevisionState::Done);
    assert_eq!(s.names(), vec![episode]);
    assert_eq!(read(&s.file(episode)), NEW_BYTES);
    assert!(s.removed(OLD_HASH));
}

// --- a lower revision when the higher one it was skipped for fails ---------------

impl Setup {
    /// `14` in place; `14v2` and `14v3` both received, `14v2` complete and
    /// skipped because `14v3` is still on its way.
    async fn v2_skipped_for_v3(&self) {
        self.received_v1().await;
        self.feed(&[(NEW_HASH, &v2()), (V3_HASH, &v3()), (OLD_HASH, &v1())]);
        self.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
        self.h.tr.content_on_add(V3_HASH, V3_BYTES);
        self.h.tr.unfinished_on_add(NEW_HASH);
        self.h.tr.unfinished_on_add(V3_HASH);
        self.cycle().await;
        self.complete(NEW_HASH);
        self.cycle().await;
        assert_eq!(self.state_of(&v2()).await, RevisionState::Skipped);
        assert_eq!(self.state_of(&v3()).await, RevisionState::Receiving);
        assert_eq!(read(&self.file(EPISODE_NAME)), OLD_BYTES);
    }

    async fn row_of(&self, title: &str) -> Revision {
        let item = self.item(title).await;
        RevisionStore::new(self.h.db.clone())
            .by_item(item.id)
            .await
            .unwrap()
            .expect("a revision row")
    }
}

/// `14v2` was skipped because `14v3` was on its way; `14v3` then stops (its
/// torrent is gone and it left the feed). `14v2` replaces `14` on a
/// following cycle, `14v3` stays `받기 실패` with `다시 받기`, and `14v3`
/// received with it later replaces `14v2` as any higher revision does.
#[tokio::test]
async fn a_lower_revision_skipped_for_a_higher_one_that_fails_replaces_the_video() {
    let s = Setup::new().await;
    s.v2_skipped_for_v3().await;

    s.h.tr.remove(V3_HASH);
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);

    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert!(s.removed(OLD_HASH));
    assert!(!s.h.tr.torrents().iter().any(|t| t.hash == OLD_HASH));
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);
    let failures = s.failures().await;
    let failure = revision_failure(&failures);
    assert_eq!(failure["history_item_id"], s.item(&v3()).await.id);
    assert_eq!(failure["can_retry"], true);
    assert_eq!(s.episode_row().await["revision"]["to"], "v2");

    // `다시 받기` of `14v3`: it replaces `14v2` by the usual steps.
    s.h.tr.content_on_add(V3_HASH, V3_BYTES);
    s.h.tr.unfinished_on_add(V3_HASH);
    s.retry(
        s.item(&v3()).await.id,
        "00000000-0000-4000-8000-000000000b01",
    )
    .await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    assert_eq!(s.state_of(&v3()).await, RevisionState::Receiving);
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
    let removes = s.h.tr.calls_of("torrent-remove");
    let v2_removal = removes
        .iter()
        .find(|c| c.args["ids"] == json!([NEW_HASH]))
        .expect("14v2's torrent is removed");
    assert_eq!(v2_removal.args["delete-local-data"], true);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert!(s.failures().await.is_empty());
}

/// The same when the higher revision fails after it was received, on its
/// CRC32 check (a failure that is final).
#[tokio::test]
async fn a_lower_revision_skipped_for_a_higher_one_whose_check_fails_replaces_the_video() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.h.tr.content_on_add(V3_HASH, b"not what the name says");
    s.h.tr.unfinished_on_add(NEW_HASH);
    s.h.tr.unfinished_on_add(V3_HASH);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);

    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// `14v3` fails after `14v2`, skipped for it, left the feed and the cycle
/// took its torrent out of Transmission: `14v2` comes back but cannot be
/// checked, so it is a failure before it was received (with `다시 받기`),
/// written once; `14` stays and nothing is added or removed again.
#[tokio::test]
async fn a_lower_revision_whose_item_is_gone_when_the_higher_one_fails_stays_a_failure() {
    let s = Setup::new().await;
    s.v2_skipped_for_v3().await;
    // `14v2` leaves the feed; the cycle takes its finished torrent out.
    s.feed(&[(V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.cycle().await;
    assert!(!s.h.tr.torrents().iter().any(|t| t.hash == NEW_HASH));
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);

    s.h.tr.remove(V3_HASH);
    s.feed(&[(OLD_HASH, &v1())]);
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);
    s.cycle().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Failed);
    assert!(row.not_received(), "{row:?}");

    let adds = (s.added(NEW_HASH), s.added(V3_HASH));
    for _ in 0..3 {
        s.h.advance(60_000);
        s.cycle().await;
    }
    assert_eq!(s.row_of(&v2()).await, row, "written once");
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);
    assert_eq!((s.added(NEW_HASH), s.added(V3_HASH)), adds);
    assert!(!s.removed(OLD_HASH));
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    let failures = s.failures().await;
    let v2_id = s.item(&v2()).await.id;
    let v2_failure = failures
        .iter()
        .find(|f| f["history_item_id"] == v2_id)
        .expect("14v2 is a failure");
    assert_eq!(v2_failure["can_retry"], true);
}

/// A lower revision skipped for another reason (the folder's video was this
/// revision or a higher one already) does not come back when a higher
/// revision of the episode fails.
#[tokio::test]
async fn a_lower_revision_skipped_for_another_reason_stays_skipped() {
    let s = Setup::new().await;
    s.v2_skipped_for_v3().await;
    // Made by hand into a skip of another reason than `14v3`.
    s.sql(&format!(
        "UPDATE video_revisions SET overtaken_by = NULL, reason = 'other'
          WHERE item_id = {}",
        s.item(&v2()).await.id
    ));
    s.h.tr.remove(V3_HASH);
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
}

// --- a replacement whose new video is gone after the old torrent was removed -----

impl Setup {
    /// `14` in place; `14v2` removed `14`'s torrent but Transmission left its
    /// file, so `14v2` waits as removing.
    async fn removal_waits(&self) {
        self.received_v1().await;
        self.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
        self.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
        self.cycle().await;
        self.complete(NEW_HASH);
        self.h.tr.keep_data_on_remove_of(OLD_HASH);
        self.cycle().await;
        self.cycle().await;
        assert_eq!(self.state_of(&v2()).await, RevisionState::Removing);
        assert!(!self.h.tr.torrents().iter().any(|t| t.hash == OLD_HASH));
        assert_eq!(read(&self.file(EPISODE_NAME)), OLD_BYTES);
    }

    /// How many torrent removals asked for the data to go too.
    fn removals_with_data(&self) -> usize {
        self.h
            .tr
            .calls_of("torrent-remove")
            .iter()
            .filter(|c| c.args["delete-local-data"] == true)
            .count()
    }

    /// `14v3` appears and is received and checked.
    async fn v3_received(&self) {
        self.feed(&[(V3_HASH, &v3()), (OLD_HASH, &v1())]);
        self.h.tr.content_on_add(V3_HASH, V3_BYTES);
        self.h.tr.unfinished_on_add(V3_HASH);
        self.cycle().await;
        assert_eq!(self.state_of(&v3()).await, RevisionState::Receiving);
        self.complete(V3_HASH);
    }
}

/// While `14v2` waits for the old file to go, the person deletes the new
/// video instead. Seen gone on two looks, the replacement ends: nothing is
/// deleted, the old file stays (and its release is not received again), it
/// is no failure, and a later revision of the episode is not held up.
#[tokio::test]
async fn a_new_video_deleted_while_the_old_file_is_left_ends_the_replacement() {
    let s = Setup::new().await;
    s.removal_waits().await;
    let removes = s.removals_with_data();
    let adds = s.added(OLD_HASH);

    std::fs::remove_file(s.file(&v2())).unwrap();
    s.cycle().await;
    assert_eq!(
        s.state_of(&v2()).await,
        RevisionState::Removing,
        "seen gone once"
    );
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert!(s.failures().await.is_empty());
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert_eq!(s.removals_with_data(), removes);
    assert_eq!(
        s.added(OLD_HASH),
        adds,
        "the old release is not received again"
    );

    // `14v3` replaces the old file.
    s.v3_received().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
}

/// The old video is gone and `14v2`'s rename has not gone through when its
/// file goes missing for one look (a mount that was away): it keeps waiting,
/// and takes the episode name once the file is back.
#[tokio::test]
async fn a_new_video_missing_on_one_look_still_takes_the_name() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.h.tr.reject_rename_of(NEW_HASH, Some("busy"));
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);
    assert_eq!(s.names(), vec![v2()]);

    let away = s.season.parent().unwrap().join("away.mkv");
    std::fs::rename(s.file(&v2()), &away).unwrap();
    s.cycle().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Removed);
    assert!(
        row.reason.as_deref().unwrap().contains("찾지 못해"),
        "{row:?}"
    );

    std::fs::rename(&away, s.file(&v2())).unwrap();
    s.h.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// `14v2` removed the old video (its file had waited after the torrent was
/// removed) and its rename has not gone through when its own file goes
/// missing. Missing on two looks in a row, the replacement ends without
/// anything deleted, and `14v3`, decided while the old file was there, is
/// no longer held up by it.
#[tokio::test]
async fn a_new_video_missing_on_two_looks_ends_the_replacement() {
    let s = Setup::new().await;
    s.removal_waits().await;
    s.v3_received().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Verified);

    // The old file goes; `14v2`'s rename is refused for now.
    s.h.tr.reject_rename_of(NEW_HASH, Some("busy"));
    std::fs::remove_file(s.file(EPISODE_NAME)).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Verified);

    // Its file goes missing.
    std::fs::remove_file(s.file(&v2())).unwrap();
    let removes = s.removals_with_data();
    let adds = s.added(OLD_HASH);
    s.cycle().await;
    assert_eq!(
        s.state_of(&v2()).await,
        RevisionState::Removed,
        "seen missing once"
    );
    assert_eq!(s.state_of(&v3()).await, RevisionState::Verified);

    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
    // The cycle may take `14v2`'s torrent out once its item left the feed,
    // as any; never with data.
    assert_eq!(s.removals_with_data(), removes);
    assert_eq!(
        s.added(OLD_HASH),
        adds,
        "the old release is not received again"
    );
    // Ended with no video under the name, `14v2` is a failure until the next
    // look finds `14v3` there.
    assert_eq!(s.failures().await.len(), 1);
    s.cycle().await;
    assert!(s.failures().await.is_empty());
}

// --- `다시 받기` looks at the video in the episode's place ------------------------

impl Setup {
    /// The episode's video and its torrent are taken away, and `14v3` is the
    /// only item of the feed: it finds the episode name free, so it is
    /// received as an ordinary item (no replacement row) and takes the name.
    async fn v3_placed_without_a_row(&self) {
        self.h.tr.remove(OLD_HASH);
        std::fs::remove_file(self.file(EPISODE_NAME)).unwrap();
        self.feed(&[(V3_HASH, &v3())]);
        self.h.tr.content_on_add(V3_HASH, V3_BYTES);
        self.cycle().await;
        self.complete(V3_HASH);
        assert_eq!(read(&self.file(EPISODE_NAME)), V3_BYTES);
        assert_eq!(self.h.tr.torrent(V3_HASH).name, EPISODE_NAME);
        assert!(RevisionStore::new(self.h.db.clone())
            .by_item(self.item(&v3()).await.id)
            .await
            .unwrap()
            .is_none());
    }

    /// The command `id`, as the web shows it.
    async fn command(&self, id: &str) -> Value {
        self.get(&format!("/api/commands/{id}")).await
    }
}

/// `14v2` stopped before it was received; then `14v3` took the episode name
/// as an ordinary item. `다시 받기` of `14v2` is refused with the reason, adds
/// nothing, and the stopped replacement, which could replace nothing now,
/// ends as skipped.
#[tokio::test]
async fn a_retry_of_a_stopped_revision_lower_than_the_placed_video_is_refused() {
    let s = Setup::new().await;
    let item = stopped_after_leaving_the_feed(&s).await;
    s.v3_placed_without_a_row().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);

    let id = "00000000-0000-4000-8000-000000000c01";
    s.retry(item.id, id).await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    let command = s.command(id).await;
    assert_eq!(command["state"], "failed", "{command}");
    assert!(
        command["outcome"]["reason"]
            .as_str()
            .unwrap()
            .contains("같거나 더 높은 수정본"),
        "{command}"
    );
    assert_eq!(s.added(NEW_HASH), 1, "not added again");
    assert_eq!(s.item(&v2()).await.result, HistoryResult::Received);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);
    assert!(s.failures().await.is_empty());
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
    assert_eq!(s.added(NEW_HASH), 1);
}

/// A `버전 미상` `14v2` (no CRC32 in its name), and `14v3` placed since as an
/// ordinary item whose torrent has left Transmission: its CRC32 tells it is
/// `14v3`, and `다시 받기` of `14v2` is refused with the reason. The item stays
/// `버전 미상`, so a later request looks at the folder again.
#[tokio::test]
async fn a_retry_of_a_version_unknown_revision_lower_than_the_placed_video_is_refused() {
    let s = Setup::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    s.feed(&[(NEW_HASH, &v2), (OLD_HASH, &v1())]);
    s.cycle().await;
    let item = s.item(&v2).await;
    assert_eq!(item.result, HistoryResult::VersionUnknown);
    s.v3_placed_without_a_row().await;
    s.h.tr.remove(V3_HASH);

    let id = "00000000-0000-4000-8000-000000000c02";
    s.retry(item.id, id).await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    let command = s.command(id).await;
    assert_eq!(command["state"], "failed", "{command}");
    assert!(
        command["outcome"]["reason"]
            .as_str()
            .unwrap()
            .contains("같거나 더 높은 수정본"),
        "{command}"
    );
    assert_eq!(s.added(NEW_HASH), 0);
    assert_eq!(s.item(&v2).await.result, HistoryResult::VersionUnknown);
    assert_eq!(s.state_of(&v2).await, RevisionState::Unknown);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
}

// --- The new video is looked at again right before the old one goes -----------

impl Setup {
    /// `14v3` is received and checked while `14v2` waits for `14`'s file to
    /// go, so it waits as verified: one replacement of the episode at a time.
    /// Then `14`'s file goes and `14v2`'s rename is refused for now. Once
    /// `14v2` takes the name, `14v3` replaces it.
    async fn v3_verified_behind_v2(&self) {
        self.removal_waits().await;
        self.feed(&[(NEW_HASH, &v2()), (V3_HASH, &v3()), (OLD_HASH, &v1())]);
        self.h.tr.content_on_add(V3_HASH, V3_BYTES);
        self.h.tr.unfinished_on_add(V3_HASH);
        self.cycle().await;
        assert_eq!(self.state_of(&v3()).await, RevisionState::Receiving);
        self.complete(V3_HASH);
        self.cycle().await;
        assert_eq!(self.state_of(&v3()).await, RevisionState::Verified);

        self.h.tr.reject_rename_of(NEW_HASH, Some("busy"));
        std::fs::remove_file(self.file(EPISODE_NAME)).unwrap();
        self.cycle().await;
        assert_eq!(self.state_of(&v2()).await, RevisionState::Removed);
        assert_eq!(self.state_of(&v3()).await, RevisionState::Verified);
    }
}

/// `14v3`'s file is deleted after its CRC32 was checked and before it removes
/// the old video (`14v2`, which takes the episode name meanwhile). The old
/// video is not removed: the replacement waits on the first look and ends on
/// the second, with `14v2` and its torrent in place.
#[tokio::test]
async fn a_new_video_deleted_after_its_check_removes_no_old_video() {
    let s = Setup::new().await;
    s.v3_verified_behind_v2().await;
    std::fs::remove_file(s.file(&v3())).unwrap();
    let removes = s.removals_with_data();

    s.h.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(
        read(&s.file(EPISODE_NAME)),
        NEW_BYTES,
        "the old video stays"
    );
    let row = s.row_of(&v3()).await;
    assert_eq!(row.state, RevisionState::Verified, "seen missing once");
    assert!(row.reason.is_some(), "{row:?}");
    assert!(row.new_missing_at.is_some(), "{row:?}");
    assert_eq!(
        revision_failure(&s.failures().await)["files"][0]["state"],
        "kept"
    );

    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await.code(), "abandoned");
    s.cycle().await;
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert!(s.h.tr.torrents().iter().any(|t| t.hash == NEW_HASH));
    assert_eq!(s.removals_with_data(), removes);
    assert!(s.failures().await.is_empty());
}

/// `14v3`'s file is replaced by another file under its name after its CRC32
/// was checked: that is not the video checked, and the old video is not
/// removed for it.
#[tokio::test]
async fn a_new_video_replaced_after_its_check_removes_no_old_video() {
    let s = Setup::new().await;
    s.v3_verified_behind_v2().await;
    std::fs::remove_file(s.file(&v3())).unwrap();
    std::fs::write(s.file(&v3()), b"episode 14, something else").unwrap();
    let removes = s.removals_with_data();

    s.h.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(
        read(&s.file(EPISODE_NAME)),
        NEW_BYTES,
        "the old video stays"
    );
    assert_eq!(s.state_of(&v3()).await, RevisionState::Verified);
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await.code(), "abandoned");
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert!(s.h.tr.torrents().iter().any(|t| t.hash == NEW_HASH));
    assert_eq!(s.removals_with_data(), removes);
}

/// A row checked without the new file's identity kept (made so by hand)
/// removes the old video while the file under its received name has the
/// CRC32 that was checked, and not otherwise.
#[tokio::test]
async fn a_new_video_without_its_identity_kept_is_told_by_its_crc() {
    let s = Setup::new().await;
    s.v3_verified_behind_v2().await;
    let forget = format!(
        "UPDATE video_revisions SET file_identity = NULL WHERE item_id = {}",
        s.item(&v3()).await.id
    );
    s.sql(&forget);
    std::fs::write(s.file(&v3()), b"episode 14, third release and more").unwrap();
    s.h.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Verified);
    assert_eq!(
        read(&s.file(EPISODE_NAME)),
        NEW_BYTES,
        "the old video stays"
    );

    std::fs::write(s.file(&v3()), V3_BYTES).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
}

// --- Two looks in a row ------------------------------------------------------------

impl Setup {
    /// `14v2` removed `14` and its rename is refused for now.
    async fn v2_waits_for_its_name(&self) {
        self.received_v1().await;
        self.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
        self.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
        self.cycle().await;
        self.complete(NEW_HASH);
        self.h.tr.reject_rename_of(NEW_HASH, Some("busy"));
        self.cycle().await;
        assert_eq!(self.state_of(&v2()).await, RevisionState::Removed);
        assert_eq!(self.names(), vec![v2()]);
    }

    /// Where a test puts `14v2`'s file while it is "missing".
    fn away(&self) -> PathBuf {
        self.season.parent().unwrap().join("away.mkv")
    }
}

/// `14v2`'s file is missing on one look, there on the next (which goes no
/// further: Transmission does not answer), and missing again on the one
/// after. No two looks in a row found it missing, so the replacement goes on,
/// and the file takes the name once it is back.
#[tokio::test]
async fn a_new_video_found_between_two_misses_keeps_its_replacement() {
    let mut s = Setup::new().await;
    s.v2_waits_for_its_name().await;

    std::fs::rename(s.file(&v2()), s.away()).unwrap();
    s.cycle().await;
    std::fs::rename(s.away(), s.file(&v2())).unwrap();
    s.h.tr.stop().await;
    s.cycle().await;
    s.h.tr.restart().await;
    std::fs::rename(s.file(&v2()), s.away()).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);

    std::fs::rename(s.away(), s.file(&v2())).unwrap();
    s.h.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// `14v2`'s file is missing on one look and there on the next, which goes no
/// further (Transmission does not answer): the miss and its reason go, so
/// the replacement is no `받기 실패` any more.
#[tokio::test]
async fn a_new_video_found_by_a_look_that_goes_no_further_is_no_failure() {
    let mut s = Setup::new().await;
    s.v2_waits_for_its_name().await;

    std::fs::rename(s.file(&v2()), s.away()).unwrap();
    s.cycle().await;
    assert_eq!(s.failures().await.len(), 1);
    std::fs::rename(s.away(), s.file(&v2())).unwrap();
    s.h.tr.stop().await;
    s.cycle().await;
    s.h.tr.restart().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Removed);
    assert_eq!(row.new_missing_at, None);
    assert_eq!(row.reason, None, "{row:?}");
    assert!(s.failures().await.is_empty());
}

/// `14v2`'s file is missing on one look; on the next it is back, but another
/// file holds the episode name, so the rename waits. That look found the
/// file: the file missing once more after the name is free again is a first
/// miss, not the second in a row.
#[tokio::test]
async fn a_new_video_found_while_its_name_is_taken_breaks_the_run() {
    let s = Setup::new().await;
    s.v2_waits_for_its_name().await;

    std::fs::rename(s.file(&v2()), s.away()).unwrap();
    s.cycle().await;
    std::fs::rename(s.away(), s.file(&v2())).unwrap();
    std::fs::write(s.file(EPISODE_NAME), b"someone else's file").unwrap();
    s.cycle().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Removed);
    assert!(
        row.reason
            .as_deref()
            .unwrap()
            .contains("다른 파일이 있어서"),
        "{row:?}"
    );

    std::fs::remove_file(s.file(EPISODE_NAME)).unwrap();
    std::fs::rename(s.file(&v2()), s.away()).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);

    std::fs::rename(s.away(), s.file(&v2())).unwrap();
    s.h.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// `14v2`'s file is missing on one look, and on the next its whole folder is
/// away (a mount): that look decides nothing and breaks the run, so the file
/// missing once more after the folder is back is a first miss again.
#[tokio::test]
async fn a_folder_away_between_two_misses_keeps_the_replacement() {
    let s = Setup::new().await;
    s.v2_waits_for_its_name().await;

    std::fs::rename(s.file(&v2()), s.away()).unwrap();
    s.cycle().await;
    let elsewhere = s.season.with_file_name("Season 01 away");
    std::fs::rename(&s.season, &elsewhere).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);
    std::fs::rename(&elsewhere, &s.season).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);

    std::fs::rename(s.away(), s.file(&v2())).unwrap();
    s.h.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

// --- The new video is told by its CRC32 when its identity changed -----------------

/// `14v3`'s file is copied over itself after its check (as a remount or a
/// copy back from elsewhere would leave it): another inode, the same video.
/// Its CRC32, read again, is the checked one, so it replaces `14v2`.
#[tokio::test]
async fn a_new_video_whose_identity_changed_is_told_by_its_crc() {
    let s = Setup::new().await;
    s.v3_verified_behind_v2().await;
    let copy = s.season.parent().unwrap().join("copy.mkv");
    std::fs::copy(s.file(&v3()), &copy).unwrap();
    std::fs::rename(&copy, s.file(&v3())).unwrap();

    s.h.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
}

/// Told by its CRC32 after its identity changed, `14v3`'s file keeps the
/// identity it has now: a later look compares that instead of reading the
/// whole file again on every cycle.
#[tokio::test]
async fn a_new_video_told_by_its_crc_keeps_its_identity_now() {
    let s = Setup::new().await;
    s.v3_verified_behind_v2().await;
    let copy = s.season.parent().unwrap().join("copy.mkv");
    std::fs::copy(s.file(&v3()), &copy).unwrap();
    std::fs::rename(&copy, s.file(&v3())).unwrap();
    let now = FileIdentity::at(&s.file(&v3())).unwrap().to_text();
    assert_ne!(s.row_of(&v3()).await.file_identity, Some(now.clone()));

    s.h.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    s.cycle().await;
    let row = s.row_of(&v3()).await;
    assert_eq!(row.state, RevisionState::Done);
    assert_eq!(row.file_identity, Some(now));
}

/// `14v3`'s file is gone when it is to remove `14v2`, a file no torrent holds
/// that it would read whole to tell. The new video is looked at first: the
/// old one is never read for a replacement that cannot go on.
#[tokio::test]
async fn a_missing_new_video_is_seen_before_the_old_video_is_read() {
    let s = Setup::new().await;
    s.v3_verified_behind_v2().await;
    // `14v2` takes the name on disk once its torrent is gone.
    s.h.tr.remove(NEW_HASH);
    std::fs::remove_file(s.file(&v3())).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_ne!(s.state_of(&v3()).await.code(), "abandoned");

    // The episode name becomes a pipe, which tells whether anyone opens it.
    let episode = s.file(EPISODE_NAME);
    std::fs::remove_file(&episode).unwrap();
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &episode,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o644),
        0,
    )
    .unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let probe = {
        let (episode, stop) = (episode.clone(), stop.clone());
        tokio::task::spawn_blocking(move || {
            use std::{io::Write, os::unix::fs::OpenOptionsExt, sync::atomic::Ordering};
            while !stop.load(Ordering::SeqCst) {
                // Opens only while someone has it open for reading.
                if let Ok(mut pipe) = std::fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
                    .open(&episode)
                {
                    let _ = pipe.write_all(NEW_BYTES);
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            false
        })
    };
    s.cycle().await;
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(!probe.await.unwrap(), "the old video was read");
    assert_eq!(s.state_of(&v3()).await.code(), "abandoned");
}

// --- An abandoned replacement holds nothing back ---------------------------------------

/// `14v2` was skipped because `14v3` was on its way. `14v3` removes `14` and
/// then loses its video before it takes the name, so it is abandoned: `14v2`
/// starts over as if `14v3` had failed, and puts its video under the episode
/// name.
#[tokio::test]
async fn a_lower_revision_skipped_for_an_abandoned_one_replaces_the_video() {
    let s = Setup::new().await;
    s.v2_skipped_for_v3().await;
    s.h.tr.reject_rename_of(V3_HASH, Some("busy"));
    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Removed);
    assert_eq!(s.names(), sorted(vec![v2(), v3()]));

    std::fs::remove_file(s.file(&v3())).unwrap();
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await.code(), "abandoned");
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

// --- An ended replacement that left the episode without a video -------------------

/// `14v2` removed `14` and then lost its video before it took the name. The
/// episode has no video under its name, so the ended replacement is a
/// `받기 실패` that says so, until a video is under the name again.
#[tokio::test]
async fn a_replacement_ended_with_no_video_left_is_a_failure_until_the_name_holds_one() {
    let s = Setup::new().await;
    s.v2_waits_for_its_name().await;
    std::fs::remove_file(s.file(&v2())).unwrap();
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    let failures = s.failures().await;
    let failure = revision_failure(&failures);
    assert!(
        failure["reason"]
            .as_str()
            .unwrap()
            .contains("회차 이름에 영상이 없어요"),
        "{failure}"
    );
    assert_eq!(failure["files"][0]["state"], "removed");
    assert_eq!(failure["files"][1]["state"], "missing");
    assert_eq!(s.episode_row().await["failure"]["files"], failure["files"]);

    std::fs::write(s.file(EPISODE_NAME), b"put back by hand").unwrap();
    s.cycle().await;
    assert!(s.failures().await.is_empty());
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert_eq!(read(&s.file(EPISODE_NAME)), b"put back by hand");
}

/// `14v2` removed `14`'s torrent, whose file Transmission left, and then lost
/// its own video: the replacement ends with `14`'s file still under the name,
/// which is no failure. The ended replacement keeps watching that file: once
/// it goes too (Transmission deleting it late, or the person), the episode
/// has no video, and that is a `받기 실패` until a video is there again.
#[tokio::test]
async fn a_replacement_ended_beside_a_left_old_file_is_a_failure_once_that_file_goes() {
    let s = Setup::new().await;
    s.removal_waits().await;
    std::fs::remove_file(s.file(&v2())).unwrap();
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    s.cycle().await;
    assert!(s.failures().await.is_empty());
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);

    std::fs::remove_file(s.file(EPISODE_NAME)).unwrap();
    s.cycle().await;
    let failures = s.failures().await;
    let failure = revision_failure(&failures);
    assert!(
        failure["reason"]
            .as_str()
            .unwrap()
            .contains("회차 이름에 영상이 없어요"),
        "{failure}"
    );
    assert_eq!(failure["files"][0]["state"], "removed");
    assert_eq!(failure["files"][1]["state"], "missing");

    std::fs::write(s.file(EPISODE_NAME), b"put back by hand").unwrap();
    s.cycle().await;
    assert!(s.failures().await.is_empty());
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert_eq!(read(&s.file(EPISODE_NAME)), b"put back by hand");
}

/// `14v2` removed `14` and then lost its video, while its torrent is still
/// in Transmission: the ended replacement is a `받기 실패` with `다시 받기`.
/// Receiving it again has Transmission check the torrent's data (it finds
/// the file gone) and start it, and the replacement starts over from its
/// first step: the downloaded video is checked and takes the episode name.
#[tokio::test]
async fn a_replacement_ended_with_no_video_left_is_received_again_with_retry() {
    let s = Setup::new().await;
    s.v2_waits_for_its_name().await;
    std::fs::remove_file(s.file(&v2())).unwrap();
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert!(s.h.tr.torrents().iter().any(|t| t.hash == NEW_HASH));
    let old_adds = s.added(OLD_HASH);

    let item = s.item(&v2()).await;
    let failures = s.failures().await;
    let failure = revision_failure(&failures);
    assert_eq!(failure["history_item_id"], item.id);
    assert_eq!(failure["can_retry"], true, "{failure}");
    assert_eq!(s.episode_row().await["failure"]["can_retry"], true);

    s.h.tr.reject_rename_of(NEW_HASH, None);
    s.retry(item.id, "00000000-0000-4000-8000-000000000d01")
        .await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    assert_eq!(s.added(NEW_HASH), 2, "asked for again");
    assert_eq!(
        s.h.tr
            .mutations()
            .iter()
            .filter(|m| m.starts_with("torrent-verify") || m.starts_with("torrent-start"))
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            format!("torrent-start ids=[\"{NEW_HASH}\"]"),
            format!("torrent-verify ids=[\"{NEW_HASH}\"]"),
        ]
    );
    assert_eq!(s.item(&v2()).await.result, HistoryResult::Received);
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Receiving);
    assert_eq!(row.received_name, None);
    assert_eq!(row.new_missing_at, None);
    assert!(s.failures().await.is_empty());

    // Transmission downloads it again.
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    std::fs::write(s.file(&v2()), NEW_BYTES).unwrap();
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(
        s.added(OLD_HASH),
        old_adds,
        "the old release is not received again"
    );
    assert!(s.failures().await.is_empty());
}

// --- A rule folder that is away decides nothing -------------------------------------

impl Setup {
    /// Takes the season folder away (a mount that is not there) and returns
    /// where it went.
    fn folder_away(&self) -> PathBuf {
        let elsewhere = self.season.with_file_name("Season 01 away");
        std::fs::rename(&self.season, &elsewhere).unwrap();
        elsewhere
    }

    fn folder_back(&self, elsewhere: &Path) {
        std::fs::rename(elsewhere, &self.season).unwrap();
    }
}

/// `14v2`'s torrent completes while the rule's folder is away: the new video
/// is not "gone", the replacement waits, and goes on once the folder is back.
#[tokio::test]
async fn a_revision_completing_while_its_folder_is_away_waits_for_it() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    let elsewhere = s.folder_away();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    s.folder_back(&elsewhere);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// `14v2` waits, removing, for `14`'s file to go after its torrent was
/// removed, when the folder is away: that is not the old video gone, and
/// the replacement keeps waiting instead of going on to a rename that would
/// find `14` back under the name.
#[tokio::test]
async fn a_removal_waiting_while_its_folder_is_away_keeps_waiting() {
    let s = Setup::new().await;
    s.removal_waits().await;
    let elsewhere = s.folder_away();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removing);

    s.folder_back(&elsewhere);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removing);
    std::fs::remove_file(s.file(EPISODE_NAME)).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// A failure that keeps both files is not "resolved" by its folder being
/// away: it stays a failure.
#[tokio::test]
async fn a_failure_whose_folder_is_away_stays_a_failure() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, b"not what the name says");
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);

    let elsewhere = s.folder_away();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    s.folder_back(&elsewhere);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert_eq!(s.failures().await.len(), 1);
}

/// `다시 받기` of a `버전 미상` revision while Transmission does not answer:
/// the episode's video cannot be told, so nothing is added, and the command
/// says the person can ask again later (nothing tries again by itself).
#[tokio::test]
async fn a_retry_that_cannot_look_at_the_episode_says_to_ask_again() {
    let mut s = Setup::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    s.feed(&[(NEW_HASH, &v2), (OLD_HASH, &v1())]);
    s.cycle().await;
    let item = s.item(&v2).await;
    assert_eq!(item.result, HistoryResult::VersionUnknown);

    s.h.tr.stop().await;
    let id = "00000000-0000-4000-8000-000000000c03";
    s.retry(item.id, id).await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    s.h.tr.restart().await;
    let command = s.command(id).await;
    assert_eq!(command["state"], "failed", "{command}");
    let reason = command["outcome"]["reason"].as_str().unwrap();
    assert!(reason.contains("확인하지 못해서"), "{command}");
    assert!(
        reason.contains("다시 받기를 다시 누를 수 있어요"),
        "{command}"
    );
    assert_eq!(s.added(NEW_HASH), 0);
    assert_eq!(s.state_of(&v2).await, RevisionState::Unknown);
}

// --- A rule folder away for a long time -----------------------------------------------

const DAY: i64 = 24 * 60 * 60 * 1000;

/// `14v2` is still downloading when the rule's folder goes away and does not
/// come back. A week after the first look found it away, the replacement,
/// which still holds its torrent, stays but is a `받기 실패` that says the
/// work's folder is not seen. Once the folder is back that goes, and the
/// replacement goes on.
#[tokio::test]
async fn a_replacement_whose_folder_is_away_for_a_week_says_so() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.h.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    let elsewhere = s.folder_away();
    s.cycle().await;
    s.h.advance(7 * DAY - 1);
    s.cycle().await;
    assert!(s.failures().await.is_empty());

    s.h.advance(1);
    s.cycle().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Receiving);
    let failures = s.failures().await;
    let failure = revision_failure(&failures);
    assert!(
        failure["reason"]
            .as_str()
            .unwrap()
            .contains("작품 폴더가 보이지 않아요"),
        "{failure}"
    );
    assert!(s.h.tr.torrents().iter().any(|t| t.hash == NEW_HASH));

    s.folder_back(&elsewhere);
    s.cycle().await;
    assert!(s.failures().await.is_empty());
    assert_eq!(s.row_of(&v2()).await.reason, None);
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// A failure that keeps both files, whose folder is away a week after the
/// first look found it away, is cleared: it leaves the list, and stays
/// cleared when the folder comes back. A folder that came back in between
/// starts the week over.
#[tokio::test]
async fn a_failure_whose_folder_is_away_for_a_week_is_cleared() {
    let s = Setup::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.h.tr.content_on_add(NEW_HASH, b"not what the name says");
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);

    let elsewhere = s.folder_away();
    s.cycle().await;
    s.h.advance(4 * DAY);
    s.folder_back(&elsewhere);
    s.cycle().await;
    let elsewhere = s.folder_away();
    s.cycle().await;
    s.h.advance(4 * DAY);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert_eq!(s.failures().await.len(), 1);

    s.h.advance(3 * DAY);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Cleared);
    assert!(s.failures().await.is_empty());
    s.folder_back(&elsewhere);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Cleared);
    assert_eq!(s.names(), sorted(vec![EPISODE_NAME.to_owned(), v2()]));
}

/// A replacement that ended with no video under the episode name, whose
/// folder is away for a week, is no failure any more.
#[tokio::test]
async fn an_ended_replacement_whose_folder_is_away_for_a_week_is_cleared() {
    let s = Setup::new().await;
    s.v2_waits_for_its_name().await;
    std::fs::remove_file(s.file(&v2())).unwrap();
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.failures().await.len(), 1);

    let _elsewhere = s.folder_away();
    s.cycle().await;
    s.h.advance(7 * DAY);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert!(s.failures().await.is_empty());
}

// --- `다시 받기` of a replacement that ended with no video left ---------------------

impl Setup {
    /// `14v2` removed `14`, then lost its video before it took the name; its
    /// torrent is still in Transmission, its rename no longer refused.
    async fn v2_ended_with_no_video(&self) {
        self.v2_waits_for_its_name().await;
        std::fs::remove_file(self.file(&v2())).unwrap();
        self.cycle().await;
        self.cycle().await;
        assert_eq!(self.state_of(&v2()).await.code(), "abandoned");
        assert_eq!(revision_failure(&self.failures().await)["can_retry"], true);
        self.h.tr.reject_rename_of(NEW_HASH, None);
    }

    fn verifies(&self) -> usize {
        self.h.tr.calls_of("torrent-verify").len()
    }

    fn starts(&self) -> usize {
        self.h.tr.calls_of("torrent-start").len()
    }
}

/// Another file has taken `14v2`'s received name since its replacement
/// ended. Checking the torrent would have Transmission take that file for
/// its own and write over it: `다시 받기` is refused with the reason, nothing
/// is asked of Transmission, the file stays, and the failure keeps
/// `다시 받기`.
#[tokio::test]
async fn a_retry_whose_received_name_holds_another_file_is_refused() {
    let s = Setup::new().await;
    s.v2_ended_with_no_video().await;
    std::fs::write(s.file(&v2()), b"someone else's file").unwrap();

    let id = "00000000-0000-4000-8000-000000000d02";
    s.retry(s.item(&v2()).await.id, id).await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    let command = s.command(id).await;
    assert_eq!(command["state"], "failed", "{command}");
    assert!(
        command["outcome"]["reason"]
            .as_str()
            .unwrap()
            .contains("다른 파일이 있어서"),
        "{command}"
    );
    assert_eq!((s.verifies(), s.starts()), (0, 0));
    assert_eq!(read(&s.file(&v2())), b"someone else's file");
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert_eq!(revision_failure(&s.failures().await)["can_retry"], true);
}

/// `14v2`'s own video is back under its received name (the CRC32 checked
/// before): `다시 받기` goes ahead, and the replacement takes the name.
#[tokio::test]
async fn a_retry_whose_received_name_holds_the_checked_video_goes_ahead() {
    let s = Setup::new().await;
    s.v2_ended_with_no_video().await;
    let copy = s.season.parent().unwrap().join("copy.mkv");
    std::fs::write(&copy, NEW_BYTES).unwrap();
    std::fs::rename(&copy, s.file(&v2())).unwrap();

    s.retry(
        s.item(&v2()).await.id,
        "00000000-0000-4000-8000-000000000d03",
    )
    .await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    // Transmission finds the data whole and seeds it.
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// Transmission refuses to check `14v2`'s torrent: it is not started (it
/// would seed a file that is not there), the command fails, and the
/// failure keeps `다시 받기`, which goes through once the check does.
#[tokio::test]
async fn a_retry_whose_torrent_check_fails_starts_nothing_and_can_be_asked_again() {
    let s = Setup::new().await;
    s.v2_ended_with_no_video().await;
    s.h.tr.reject_verify_of(NEW_HASH, Some("busy"));

    let id = "00000000-0000-4000-8000-000000000d04";
    s.retry(s.item(&v2()).await.id, id).await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    assert_eq!(s.command(id).await["state"], "failed");
    assert_eq!(s.starts(), 0);
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert_eq!(revision_failure(&s.failures().await)["can_retry"], true);

    s.h.tr.reject_verify_of(NEW_HASH, None);
    s.retry(
        s.item(&v2()).await.id,
        "00000000-0000-4000-8000-000000000d05",
    )
    .await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    assert_eq!(s.starts(), 1);
}
