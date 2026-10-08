//! Replacing a video with a higher revision of the same release (ticket 0025,
//! `docs/specs/collection.md` 영상 수정본의 대체), end to end: the worker's
//! cycles and `다시 받기` against the fake Transmission, which acts on a
//! temporary media folder the way Transmission does (see
//! [`FakeTransmission::on_disk`]), and the web API on the same database.
//!
//! One test per row of the spec's table, then the ticket's own rows. Release
//! names, hashes and contents are made up; a name's CRC32 is the CRC32 of the
//! bytes its torrent writes, unless a test says otherwise.

use crate::common;

use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use trss_collect::store::{
    channels::{ChannelInput, RuleInput, RuleState},
    history::{HistoryItem, HistoryResult},
    revisions::{Revision, RevisionState, RevisionStore},
};
use trss_core::settings::SettingsStore;
use trss_worker::{CommandsOutcome, CycleReport, TickOutcome};

const OLD_HASH: &str = "1111000000000000000000000000000000000014";
const NEW_HASH: &str = "2222000000000000000000000000000000000014";
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
        h.channels
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
        Setup { h, season }
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

// --- several revisions, several channels, and what a check cannot vouch for -------

const V3_HASH: &str = "5555000000000000000000000000000000000014";
const V3_BYTES: &[u8] = b"episode 14, third release";

fn v3() -> String {
    release("v3", Some(&crc(V3_BYTES)))
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
}

// --- The new video is told by its CRC32 when its identity changed -----------------

// --- An abandoned replacement holds nothing back ---------------------------------------

// --- An ended replacement that left the episode without a video -------------------

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

impl Setup {
    /// Archives the rule of `14` (`보관`): the rule is off and its work
    /// folder is in the archive folder. Returns where the folder went.
    async fn archive(&self) -> PathBuf {
        let rule_id = self.row_of(&v2()).await.rule_id;
        self.h
            .channels
            .set_rule_state(&rule_id, RuleState::Archived, self.h.now())
            .await
            .unwrap();
        let work = self.season.parent().unwrap();
        let archive = self.h.dir.path().join("archive");
        std::fs::create_dir_all(&archive).unwrap();
        let archived = archive.join("Show");
        std::fs::rename(work, &archived).unwrap();
        archived
    }

    /// Restores the rule archived with [`Setup::archive`] (`복원`).
    async fn restore(&self, archived: &Path) {
        std::fs::rename(archived, self.season.parent().unwrap()).unwrap();
        let rule_id = self.row_of(&v2()).await.rule_id;
        self.h
            .channels
            .set_rule_state(&rule_id, RuleState::Active, self.h.now())
            .await
            .unwrap();
    }
}

/// The rule of a replacement that ended with no video under the episode name
/// is archived for ten days: the failure and its `다시 받기` are there once
/// the rule is restored, and `다시 받기` receives it again.
#[tokio::test]
async fn an_ended_replacement_of_a_rule_archived_for_ten_days_is_retried_after_its_restore() {
    let s = Setup::new().await;
    s.v2_ended_with_no_video().await;

    let archived = s.archive().await;
    s.cycle().await;
    s.h.advance(10 * DAY);
    s.cycle().await;

    s.restore(&archived).await;
    s.cycle().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state.code(), "abandoned", "{row:?}");
    assert!(row.reason.is_some(), "{row:?}");
    let failures = s.failures().await;
    assert_eq!(revision_failure(&failures)["can_retry"], true);
    s.retry(
        s.item(&v2()).await.id,
        "00000000-0000-4000-8000-000000000d08",
    )
    .await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
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

/// `14v2`'s torrent had left Transmission when it was received again, and
/// Transmission says the new torrent is complete with no file under its
/// name. The replacement removed `14` itself, so the empty episode name is
/// no sign the failure was resolved: it stays a failure before the video
/// was received, with `다시 받기`, which has Transmission check the torrent
/// and download the file.
#[tokio::test]
async fn a_replacement_received_again_whose_file_is_not_there_stays_a_failure() {
    let s = Setup::new().await;
    s.v2_ended_with_no_video().await;
    s.h.tr.remove(NEW_HASH);
    s.retry(
        s.item(&v2()).await.id,
        "00000000-0000-4000-8000-000000000d06",
    )
    .await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    // Transmission says it is whole and seeds it; its file is gone again.
    s.complete(NEW_HASH);
    std::fs::remove_file(s.file(&v2())).unwrap();

    s.cycle().await;
    s.cycle().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Failed, "{row:?}");
    assert_eq!(row.received_name, None);
    let failures = s.failures().await;
    let failure = revision_failure(&failures);
    assert_eq!(failure["can_retry"], true);
    // The replacement removed the old video before it was received again.
    assert_eq!(
        failure["files"],
        json!([
            { "role": "old", "path": format!("Season 01/{EPISODE_NAME}"), "state": "removed" },
            { "role": "new", "path": null, "state": "not_received" },
        ])
    );

    s.retry(
        s.item(&v2()).await.id,
        "00000000-0000-4000-8000-000000000d07",
    )
    .await;
    assert_eq!(s.commands().await, CommandsOutcome::Ran(1));
    assert_eq!(s.verifies(), 1);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    std::fs::write(s.file(&v2()), NEW_BYTES).unwrap();
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}
