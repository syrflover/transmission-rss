//! The past episode search of the rule detail (ticket 0026,
//! `docs/specs/collection.md` 지난 회차 검색), end to end: the real web API
//! and search service, a fake nyaa search RSS, the worker's `receive_past`
//! command against the fake Transmission, and a temporary media folder.
//!
//! One test per row of the ticket's table (the phone-width row is the
//! screen's, checked in the browser), then the rows' edges. Release names,
//! hashes and contents are made up.

mod common;

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant},
};

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use trss_collect::{
    past_search::service::PastSearch,
    store::{
        channels::{ChannelInput, RuleInput, RuleState},
        history::{HistoryResult, Observation},
        revisions::{RevisionState, RevisionStore},
        search_pace::SearchPace,
        status::StatusStore,
    },
};
use trss_core::settings::SettingsStore;
use trss_web::AppState;
use trss_worker::{CommandsOutcome, TickOutcome};

const SPACING: Duration = Duration::from_millis(150);
static COMMANDS: AtomicU32 = AtomicU32::new(0);

fn crc(bytes: &[u8]) -> String {
    format!("{:08X}", crc32fast::hash(bytes))
}

/// A harness with a temporary media folder the fake Transmission acts on, a
/// fake nyaa, and one channel on it with one rule.
struct Setup {
    h: Harness,
    nyaa: FakeNyaa,
    api: WebApi,
    folder: PathBuf,
    rule_id: String,
    channel_id: String,
}

struct Options {
    phrase: &'static str,
    directory: &'static str,
    episode: i64,
    format: Option<&'static str>,
}

impl Options {
    fn show() -> Options {
        Options {
            phrase: "Show",
            directory: "Show/Season 01",
            episode: 0,
            format: Some("[SubsPlease] {match} 1080p"),
        }
    }
}

impl Setup {
    async fn new(options: Options) -> Setup {
        let h = Harness::without_collect_folder().await;
        let media = h.dir.path().join("media");
        let folder = media.join(options.directory);
        std::fs::create_dir_all(&folder).unwrap();
        SettingsStore::new(h.db.clone())
            .put_collection(0, media.to_str().unwrap().to_owned(), None)
            .await
            .unwrap();
        h.tr.on_disk(&media);
        let nyaa = FakeNyaa::start().await;
        let mut input = ChannelInput::new(nyaa.url(SECRET));
        input.past_search = options.format.map(str::to_owned);
        let channel = h
            .channels
            .create_channel_with_rules(
                input,
                vec![RuleInput {
                    r#match: Some(options.phrase.to_owned()),
                    directory: options.directory.to_owned(),
                    episode: options.episode,
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
            channel_id: channel.channel.id,
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

    /// `receive_past` of the result `key` of `poll`'s search; returns the
    /// command's ID.
    async fn receive(&self, poll: &Value, key: &str) -> String {
        let id = format!("past-cmd-{}", COMMANDS.fetch_add(1, Ordering::SeqCst));
        let (status, body) = self
            .call(
                "POST",
                "/api/commands",
                Some(json!({
                    "id": id,
                    "kind": "receive_past",
                    "payload": {
                        "rule_id": self.rule_id,
                        "search_id": poll["search_id"],
                        "key": key,
                    },
                })),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        id
    }

    async fn run_commands(&self) {
        let outcome = self
            .h
            .worker()
            .run_commands(&CancellationToken::new())
            .await
            .unwrap();
        assert!(matches!(outcome, CommandsOutcome::Ran(_)), "{outcome:?}");
    }

    async fn command(&self, id: &str) -> Value {
        let (status, body) = self.call("GET", &format!("/api/commands/{id}"), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn cycle(&self) {
        match self
            .h
            .worker()
            .tick(&CancellationToken::new())
            .await
            .unwrap()
        {
            TickOutcome::Ran(_) => {}
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.folder.join(name), bytes).unwrap();
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

fn item<'a>(poll: &'a Value, title_part: &str) -> &'a Value {
    let found: Vec<&Value> = poll["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["title"].as_str().unwrap().contains(title_part))
        .collect();
    assert_eq!(found.len(), 1, "{title_part}: {found:?}");
    found[0]
}

fn selected(poll: &Value) -> Vec<u32> {
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

fn episode(group: &str, show: &str, n: u32, extra: &str) -> String {
    format!(
        "[{group}] {show} - {n:02} (1080p) [{:08X}]{extra}.mkv",
        0xA000_0000u32 + n
    )
}

// --- the ticket's rows ----------------------------------------------------------------

/// Row 1: a search that returns everything at once.
#[tokio::test]
async fn row_1_a_search_that_returns_all_selects_the_missing_episodes_and_adds_only_the_confirmed()
{
    let s = Setup::new(Options {
        phrase: "Sayonara Lara",
        directory: "Sayonara Lara/Season 01",
        ..Options::show()
    })
    .await;
    let titles: Vec<String> = (1..=12)
        .rev()
        .map(|n| episode("SubsPlease", "Sayonara Lara", n, ""))
        .collect();
    s.nyaa.set_releases(&titles);
    for n in 1..=3 {
        s.write(&format!("Sayonara Lara S01E{n:02}.mkv"), b"x");
    }

    let (status, context) = s
        .call(
            "GET",
            &format!("/api/rules/{}/past-search", s.rule_id),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(context["query"], "[SubsPlease] Sayonara Lara 1080p");
    assert_eq!(context["from_format"], true);

    let poll = s.search("[SubsPlease] Sayonara Lara 1080p", 1, 12).await;
    assert_eq!(poll["state"], "done", "{poll}");
    assert_eq!(poll["result"]["items"].as_array().unwrap().len(), 12);
    assert_eq!(selected(&poll), (4..=12).collect::<Vec<u32>>());
    assert_eq!(
        poll["result"]["missing"],
        json!((4..=12).collect::<Vec<u32>>())
    );
    assert_eq!(poll["result"]["out_of_range"], 0);
    assert_eq!(s.nyaa.queries().len(), 1, "one search was enough");
    assert!(s.added().is_empty(), "a search adds nothing");

    // The person confirms two of the nine.
    let four = item(&poll, "- 04 ")["key"].as_str().unwrap().to_owned();
    let nine = item(&poll, "- 09 ")["key"].as_str().unwrap().to_owned();
    let (c4, c9) = (s.receive(&poll, &four).await, s.receive(&poll, &nine).await);
    s.run_commands().await;

    for id in [&c4, &c9] {
        let command = s.command(id).await;
        assert_eq!(command["state"], "done", "{command}");
        assert_eq!(command["outcome"]["result"], "received", "{command}");
    }
    let added = s.added();
    assert_eq!(added.len(), 2, "{added:?}");
    for n in [4, 9] {
        let hash = FakeNyaa::hash_for(&episode("SubsPlease", "Sayonara Lara", n, ""));
        assert_eq!(added.iter().filter(|a| a.contains(&hash)).count(), 1);
    }
    let torrents = s.h.tr.torrents();
    assert_eq!(torrents.len(), 2);
    for t in &torrents {
        assert!(
            t.download_dir.ends_with("Sayonara Lara/Season 01"),
            "{}",
            t.download_dir
        );
    }
}

/// Row 2: 1기, 2기, the revision `14v2`, official and unofficial batches.
#[tokio::test]
async fn row_2_other_seasons_are_folded_batches_are_not_selected_and_only_the_revision_is() {
    let s = Setup::new(Options {
        phrase: "Sono Bisque Doll",
        directory: "Sono Bisque Doll/Season 02",
        episode: -12,
        ..Options::show()
    })
    .await;
    let mut titles = vec![
        "[SubsPlease] Sono Bisque Doll (01-12) (1080p) [Batch]".to_owned(),
        "[SubsPlease] Sono Bisque Doll (01-24) (1080p) Unofficial Batch".to_owned(),
        "[SubsPlease] Sono Bisque Doll - 14v2 (1080p) [BBBB0014].mkv".to_owned(),
    ];
    titles.extend(
        (1..=24)
            .rev()
            .map(|n| episode("SubsPlease", "Sono Bisque Doll", n, "")),
    );
    s.nyaa.set_releases(&titles);

    let poll = s
        .search("[SubsPlease] Sono Bisque Doll 1080p", 13, 24)
        .await;
    assert_eq!(poll["state"], "done", "{poll}");
    let result = &poll["result"];
    // Season 1's twelve episodes and the batch of season 1.
    assert_eq!(result["out_of_range"], 13);
    // Twelve episodes of the range, the revision, and the batch that covers it.
    assert_eq!(result["items"].as_array().unwrap().len(), 14);

    assert_eq!(item(&poll, "Unofficial")["state"], "batch");
    assert_eq!(item(&poll, "Unofficial")["selected"], false);
    assert_eq!(item(&poll, "14v2")["selected"], true);
    assert_eq!(item(&poll, "- 14 (1080p)")["selected"], false);
    assert_eq!(item(&poll, "- 14 (1080p)")["state"], "superseded");
    assert_eq!(selected(&poll), (13..=24).collect::<Vec<u32>>());
    assert_eq!(item(&poll, "- 13 ")["folder"], "S02E01");
    assert_eq!(item(&poll, "14v2")["folder"], "S02E02");
    assert_eq!(result["missing"], json!((13..=24).collect::<Vec<u32>>()));
}

/// Row 3: an episode the folder has, and one received from another release.
#[tokio::test]
async fn row_3_episodes_in_the_folder_or_received_from_another_release_are_shown_and_not_selected()
{
    let s = Setup::new(Options::show()).await;
    let titles: Vec<String> = (5..=8)
        .rev()
        .map(|n| episode("SubsPlease", "Show", n, ""))
        .collect();
    s.nyaa.set_releases(&titles);
    s.write("Show S01E05.mkv", b"x");
    // Episode 6 was received from Erai-raws: Transmission holds that torrent
    // (history says so) and the file is still downloading.
    let erai = "[Erai-raws] Show - 06 [1080p][ABCD1234].mkv";
    s.h.history
        .record(
            1,
            vec![Observation {
                channel_id: s.channel_id.clone(),
                channel_label: "nyaa".into(),
                identity_key: "guid:erai06".into(),
                title: erai.into(),
                link: "magnet:?xt=urn:btih:eeee00000000000000000000000000000000000a".into(),
                result: HistoryResult::Received,
                rule_id: Some(s.rule_id.clone()),
                torrent_hash: Some("eeee00000000000000000000000000000000000a".into()),
                reason: None,
            }],
        )
        .await
        .unwrap();
    s.write("Show S01E06.mkv.part", b"x");

    let poll = s.search("[SubsPlease] Show 1080p", 5, 8).await;
    assert_eq!(poll["state"], "done", "{poll}");
    assert_eq!(item(&poll, "- 05 ")["state"], "have");
    assert_eq!(item(&poll, "- 06 ")["state"], "have");
    assert!(item(&poll, "- 06 ")["note"]
        .as_str()
        .unwrap()
        .contains("다른 릴리스"));
    assert_eq!(selected(&poll), vec![7, 8]);
    assert_eq!(poll["result"]["missing"], json!([7, 8]));

    // The judgement is by number and file: choosing episode 6 by hand adds the
    // SubsPlease torrent, whose hash Transmission does not have.
    let six = item(&poll, "- 06 ")["key"].as_str().unwrap().to_owned();
    let id = s.receive(&poll, &six).await;
    s.run_commands().await;
    assert_eq!(s.command(&id).await["outcome"]["result"], "received");
    assert_eq!(s.added().len(), 1);
}

/// Row 4: a video of unknown version whose CRC32 differs from the result's.
#[tokio::test]
async fn row_4_a_video_of_unknown_version_with_another_crc_makes_the_result_unknown_and_unselected()
{
    let s = Setup::new(Options::show()).await;
    let old = b"episode 14, the video in the folder";
    s.write("Show S01E14.mkv", old);
    s.nyaa.set_releases(&[
        format!(
            "[SubsPlease] Show - 14v2 (1080p) [{}].mkv",
            crc(b"episode 14, a revision of another file")
        ),
        "[SubsPlease] Show - 15 (1080p) [AAAA0015].mkv".to_owned(),
    ]);

    let poll = s.search("[SubsPlease] Show 1080p", 14, 15).await;
    let v2 = item(&poll, "14v2");
    assert_eq!(v2["state"], "version_unknown", "{poll}");
    assert_eq!(v2["selected"], false);
    assert!(v2["note"].as_str().unwrap().contains("CRC32"));
    assert_eq!(selected(&poll), vec![15]);

    // When the video equals the CRC32 of the lower revision in the results, the
    // revision replaces it; when it equals the result's own, the folder has it.
    s.nyaa.set_releases(&[
        format!("[SubsPlease] Show - 14v2 (1080p) [{}].mkv", crc(b"new")),
        format!("[SubsPlease] Show - 14 (1080p) [{}].mkv", crc(old)),
    ]);
    let poll = s.search("[SubsPlease] Show 1080p", 14, 14).await;
    assert_eq!(item(&poll, "14v2")["state"], "replace", "{poll}");
    assert_eq!(item(&poll, "14v2")["selected"], false);
    s.nyaa.set_releases(&[format!(
        "[SubsPlease] Show - 14v2 (1080p) [{}].mkv",
        crc(old)
    )]);
    let poll = s.search("[SubsPlease] Show 1080p", 14, 14).await;
    assert_eq!(item(&poll, "14v2")["state"], "have", "{poll}");
}

/// Row 5: a first result of 75 (a long series), then spaced searches of the
/// missing episodes, merged without loss or repeat.
#[tokio::test]
async fn row_5_a_full_first_page_is_followed_by_spaced_searches_merged_without_loss_or_repeat() {
    let s = Setup::new(Options {
        phrase: "One Piece",
        directory: "One Piece/Season 21",
        ..Options::show()
    })
    .await;
    // Episodes 1001 to 1100, newest first: the tracker returns the newest 75.
    let titles: Vec<String> = (1001..=1100)
        .rev()
        .map(|n| {
            format!(
                "[SubsPlease] One Piece - {n} (1080p) [{:08X}].mkv",
                0xB000_0000u32 + n
            )
        })
        .collect();
    s.nyaa.set_releases(&titles);
    // The folder has 1090; it is not searched for again.
    s.write("One Piece S21E1090.mkv", b"x");

    let poll = s.search("[SubsPlease] One Piece 1080p", 1001, 1100).await;
    assert_eq!(poll["state"], "done", "{poll}");
    let result = &poll["result"];
    assert_eq!(result["first_full"], true);
    // 1001 to 1025 are missing from the first 75 (1026 to 1100): 25 episodes
    // in groups of ten.
    assert_eq!(result["extra_needed"], 3);
    assert_eq!(result["extra_sent"], 3);
    let queries = s.nyaa.queries();
    assert_eq!(queries.len(), 4, "{queries:?}");
    assert_eq!(queries[0], "[SubsPlease] One Piece 1080p");
    assert_eq!(
        queries[1],
        "[SubsPlease] One Piece - (1001|1002|1003|1004|1005|1006|1007|1008|1009|1010) 1080p"
    );
    assert_eq!(
        queries[3],
        "[SubsPlease] One Piece - (1021|1022|1023|1024|1025) 1080p"
    );
    // One after the other, with the spacing between them. The gap is measured
    // where the request arrives, so scheduling noise moves it by a few
    // milliseconds either way; with no spacing the gaps would be near zero.
    for gap in s.nyaa.gaps() {
        assert!(gap >= SPACING - Duration::from_millis(60), "{gap:?}");
    }

    // All hundred, once each.
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 100);
    let mut numbers: Vec<u64> = items
        .iter()
        .map(|i| i["release"].as_u64().unwrap())
        .collect();
    numbers.sort_unstable();
    assert_eq!(numbers, (1001..=1100).collect::<Vec<u64>>());
    let mut keys: Vec<&str> = items.iter().map(|i| i["key"].as_str().unwrap()).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), 100);
    // 1090 has a file already.
    assert_eq!(item(&poll, "- 1090 ")["state"], "have");
    assert_eq!(selected(&poll).len(), 99);
    assert!(result["notes"].as_array().unwrap().is_empty());
}

/// A first page that is not full needs no more searches.
#[tokio::test]
async fn a_first_page_that_is_not_full_ends_the_search() {
    let s = Setup::new(Options::show()).await;
    s.nyaa.set_releases(&[
        episode("SubsPlease", "Show", 3, ""),
        episode("SubsPlease", "Show", 1, ""),
    ]);
    let poll = s.search("[SubsPlease] Show 1080p", 1, 12).await;
    assert_eq!(poll["result"]["first_full"], false);
    assert_eq!(s.nyaa.queries().len(), 1);
    assert_eq!(poll["result"]["not_found"].as_array().unwrap().len(), 10);
}

/// Row 6: a channel without a search format.
#[tokio::test]
async fn row_6_without_a_format_the_query_starts_from_the_match_phrase_and_is_edited_in_place() {
    let s = Setup::new(Options {
        phrase: "Sayonara Lara",
        directory: "Sayonara Lara/Season 01",
        format: None,
        ..Options::show()
    })
    .await;
    s.nyaa.set_releases(&[
        episode("SubsPlease", "Sayonara Lara", 2, ""),
        episode("Erai-raws", "Sayonara Lara", 1, ""),
        episode("SubsPlease", "Sayonara Lara", 1, ""),
    ]);
    let (_, context) = s
        .call(
            "GET",
            &format!("/api/rules/{}/past-search", s.rule_id),
            None,
        )
        .await;
    assert_eq!(context["query"], "Sayonara Lara");
    assert_eq!(context["from_format"], false);
    assert_eq!(context["format"], Value::Null);

    // Edited in the box before the one search: the group is added.
    let poll = s.search("[SubsPlease]   Sayonara Lara 1080p", 1, 2).await;
    assert_eq!(poll["state"], "done", "{poll}");
    assert_eq!(s.nyaa.queries(), vec!["[SubsPlease] Sayonara Lara 1080p"]);
    assert_eq!(poll["result"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(selected(&poll), vec![1, 2]);
}

/// Row 7: no channel is left, and history holds only what was received.
#[tokio::test]
async fn row_7_a_search_leaves_no_channel_and_history_holds_only_the_received_items() {
    let s = Setup::new(Options::show()).await;
    s.nyaa.set_releases(&[
        episode("SubsPlease", "Show", 2, ""),
        episode("SubsPlease", "Show", 1, ""),
    ]);
    let poll = s.search("[SubsPlease] Show 1080p", 1, 2).await;
    assert_eq!(poll["state"], "done");
    assert_eq!(
        s.h.channels.list_channels_with_rules().await.unwrap().len(),
        1
    );
    assert!(s.h.history_items().await.is_empty());

    let one = item(&poll, "- 01 ")["key"].as_str().unwrap().to_owned();
    let id = s.receive(&poll, &one).await;
    // Leaving the search (the screen's cancel) forgets its results.
    s.run_commands().await;
    assert_eq!(s.command(&id).await["outcome"]["result"], "received");

    let items = s.h.history_items().await;
    assert_eq!(items.len(), 1, "{items:?}");
    assert!(items[0].title.contains("- 01 "));
    assert_eq!(items[0].result, HistoryResult::Received);
    assert_eq!(items[0].rule_id.as_deref(), Some(s.rule_id.as_str()));
    assert!(items[0].torrent_hash.is_some());
    assert_eq!(
        s.h.channels.list_channels_with_rules().await.unwrap().len(),
        1
    );
    assert!(!items[0].link.contains(SECRET));

    let search = poll["search_id"].as_str().unwrap();
    let (status, _) = s
        .call("DELETE", &format!("/api/past-searches/{search}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = s
        .call("GET", &format!("/api/past-searches/{search}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The accepted request still answers as itself, without its search.
    let (status, again) = s
        .call(
            "POST",
            "/api/commands",
            Some(json!({
                "id": id,
                "kind": "receive_past",
                "payload": { "rule_id": s.rule_id, "search_id": search, "key": one },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    // A request with no search behind it stores nothing.
    let (status, _) = s
        .call(
            "POST",
            "/api/commands",
            Some(json!({
                "id": "past-cmd-gone",
                "kind": "receive_past",
                "payload": { "rule_id": s.rule_id, "search_id": search, "key": "guid:x" },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(s.h.history_items().await.len(), 1);
}

// --- the edges -------------------------------------------------------------------------

#[tokio::test]
async fn a_request_that_cannot_be_searched_is_refused_before_any_request_is_sent() {
    let s = Setup::new(Options::show()).await;
    let uri = format!("/api/rules/{}/past-search", s.rule_id);
    for body in [
        json!({ "query": "  ", "from": 1, "to": 2 }),
        json!({ "query": "Show", "from": 0, "to": 2 }),
        json!({ "query": "Show", "from": 5, "to": 2 }),
        json!({ "query": "Show", "from": 1, "to": 5000 }),
        json!({ "query": "Show", "from": 1, "to": 2, "extra": 1 }),
    ] {
        let (status, answer) = s.call("POST", &uri, Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {answer}");
    }
    s.h.channels
        .set_rule_state(&s.rule_id, RuleState::Paused, 1)
        .await
        .unwrap();
    let (status, _) = s
        .call(
            "POST",
            &uri,
            Some(json!({ "query": "Show", "from": 1, "to": 2 })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, context) = s.call("GET", &uri, None).await;
    assert!(context["blocked"].as_str().unwrap().contains("멈춰"));
    assert!(s.nyaa.queries().is_empty());
}

#[tokio::test]
async fn a_tracker_that_asks_to_wait_fails_the_search_and_says_for_how_long() {
    let s = Setup::new(Options::show()).await;
    s.nyaa.refuse(Some((429, Some(30))));
    let poll = s.search("[SubsPlease] Show 1080p", 1, 2).await;
    assert_eq!(poll["state"], "failed", "{poll}");
    assert!(poll["error"].as_str().unwrap().contains("30초"), "{poll}");
    assert_eq!(s.nyaa.queries().len(), 1);
}

#[tokio::test]
async fn a_new_search_of_a_rule_ends_the_one_before_it() {
    let s = Setup::new(Options::show()).await;
    s.nyaa.set_releases(&[episode("SubsPlease", "Show", 1, "")]);
    let first = s.search("[SubsPlease] Show 1080p", 1, 2).await;
    let second = s.search("[SubsPlease] Show 1080p", 1, 2).await;
    assert_ne!(first["search_id"], second["search_id"]);
    let (status, _) = s
        .call(
            "GET",
            &format!(
                "/api/past-searches/{}",
                first["search_id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // A result of the ended search cannot be received.
    let (status, _) = s
        .call(
            "POST",
            "/api/commands",
            Some(json!({
                "id": "past-cmd-old",
                "kind": "receive_past",
                "payload": {
                    "rule_id": s.rule_id,
                    "search_id": first["search_id"],
                    "key": "guid:x",
                },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A result no rule would pick is not received, even asked for by a stale tab.
#[tokio::test]
async fn a_result_the_rule_does_not_pick_is_refused_and_leaves_no_history() {
    let s = Setup::new(Options::show()).await;
    s.nyaa.set_releases(&[
        episode("SubsPlease", "Show", 1, ""),
        episode("SubsPlease", "Show Side Story", 1, ""),
    ]);
    let poll = s.search("[SubsPlease] Show 1080p", 1, 2).await;
    // "Show Side Story" contains the phrase, so the rule picks it as well.
    assert_eq!(poll["result"]["items"].as_array().unwrap().len(), 2);
    let id = format!("past-cmd-{}", COMMANDS.fetch_add(1, Ordering::SeqCst));
    let (status, _) = s
        .call(
            "POST",
            "/api/commands",
            Some(json!({
                "id": id,
                "kind": "receive_past",
                "payload": { "rule_id": s.rule_id, "search_id": poll["search_id"], "key": "guid:nope" },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(s.h.history_items().await.is_empty());
}

/// The revision of a video the folder holds, received from the search: the new
/// torrent keeps its name until checked, then replaces the old video.
#[tokio::test]
async fn a_revision_chosen_in_the_preview_replaces_the_video_after_it_is_received_and_checked() {
    let s = Setup::new(Options::show()).await;
    let old = b"episode 14, first release";
    let new = b"episode 14, second release";
    let v1 = format!("[SubsPlease] Show - 14 (1080p) [{}].mkv", crc(old));
    let v2 = format!("[SubsPlease] Show - 14v2 (1080p) [{}].mkv", crc(new));
    let (v1_hash, v2_hash) = (FakeNyaa::hash_for(&v1), FakeNyaa::hash_for(&v2));
    // v1 comes through the channel's feed and a cycle, as it does for a rule.
    s.nyaa.set_releases(std::slice::from_ref(&v1));
    s.h.tr.content_on_add(&v1_hash, old);
    s.cycle().await;
    s.h.tr.finish(&v1_hash);
    s.h.tr.set_status(&v1_hash, 6);
    assert!(s.folder.join("Show S01E14.mkv").exists());

    // v2 is found by search: it replaces, and is not chosen for the person.
    s.nyaa.set_releases(&[v2.clone(), v1.clone()]);
    s.h.tr.content_on_add(&v2_hash, new);
    s.h.tr.unfinished_on_add(&v2_hash);
    let poll = s.search("[SubsPlease] Show 1080p", 14, 14).await;
    let v2_item = item(&poll, "14v2");
    assert_eq!(v2_item["state"], "replace", "{poll}");
    assert_eq!(v2_item["selected"], false);

    let key = v2_item["key"].as_str().unwrap().to_owned();
    let id = s.receive(&poll, &key).await;
    s.run_commands().await;
    let command = s.command(&id).await;
    assert_eq!(command["outcome"]["result"], "received", "{command}");
    // Not renamed after the add; both videos are there until it is checked.
    let renamed =
        s.h.tr
            .calls_of("torrent-rename-path")
            .iter()
            .filter(|c| c.args["ids"][0] == v2_hash && c.args["name"] == "Show S01E14.mkv")
            .count();
    assert_eq!(renamed, 0);
    let item_row =
        s.h.history_items()
            .await
            .into_iter()
            .find(|i| i.title == v2)
            .expect("the revision is in history");
    let revision = RevisionStore::new(s.h.db.clone())
        .by_item(item_row.id)
        .await
        .unwrap()
        .expect("a revision row");
    assert_eq!(revision.state, RevisionState::Receiving);
    assert_eq!(
        std::fs::read(s.folder.join("Show S01E14.mkv")).unwrap(),
        old,
        "the old video stays until the new one is checked"
    );

    // Received and checked: the old video goes and the new takes its name.
    s.h.tr.finish(&v2_hash);
    s.h.tr.set_status(&v2_hash, 6);
    s.cycle().await;
    assert_eq!(
        std::fs::read(s.folder.join("Show S01E14.mkv")).unwrap(),
        new
    );
    let revision = RevisionStore::new(s.h.db.clone())
        .by_item(item_row.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(revision.state, RevisionState::Done);
}

// --- revisions received from a search and the feed together --------------------------

/// Release `14` of `[SubsPlease] Show` at `version` (1 for the first release)
/// with the CRC32 of `bytes`.
fn show_14(version: u32, bytes: &[u8]) -> String {
    let revision = if version > 1 {
        format!("v{version}")
    } else {
        String::new()
    };
    format!(
        "[SubsPlease] Show - 14{revision} (1080p) [{}].mkv",
        crc(bytes)
    )
}

const V1: &[u8] = b"episode 14, first release";
const V2: &[u8] = b"episode 14, second release";
const V3: &[u8] = b"episode 14, third release";

impl Setup {
    /// v1 of episode 14 received through the feed and seeding as
    /// `Show S01E14.mkv`.
    async fn first_release(&self) -> String {
        let v1 = show_14(1, V1);
        self.nyaa.set_releases(std::slice::from_ref(&v1));
        self.h.tr.content_on_add(&FakeNyaa::hash_for(&v1), V1);
        self.cycle().await;
        self.seed(&v1);
        assert_eq!(self.episode_14(), V1);
        v1
    }

    /// The torrent of `title` has finished and seeds.
    fn seed(&self, title: &str) {
        let hash = FakeNyaa::hash_for(title);
        self.h.tr.finish(&hash);
        self.h.tr.set_status(&hash, 6);
    }

    /// The next add of `title` gives `bytes`, left unfinished.
    fn on_add(&self, title: &str, bytes: &[u8]) {
        let hash = FakeNyaa::hash_for(title);
        self.h.tr.content_on_add(&hash, bytes);
        self.h.tr.unfinished_on_add(&hash);
    }

    /// `receive_past` of the result titled `title` of `poll`, run; returns the
    /// command.
    async fn receive_title(&self, poll: &Value, title: &str) -> Value {
        let key = item(poll, title)["key"].as_str().unwrap().to_owned();
        let id = self.receive(poll, &key).await;
        self.run_commands().await;
        self.command(&id).await
    }

    fn episode_14(&self) -> Vec<u8> {
        std::fs::read(self.folder.join("Show S01E14.mkv")).unwrap()
    }

    async fn revision_of(&self, title: &str) -> Option<RevisionState> {
        let item = self
            .h
            .history_items()
            .await
            .into_iter()
            .find(|i| i.title == title)?;
        RevisionStore::new(self.h.db.clone())
            .by_item(item.id)
            .await
            .unwrap()
            .map(|r| r.state)
    }
}

/// Two revisions of an episode chosen from a search: the lower one finishing
/// first does not take the episode name, and the higher one replaces the video.
#[tokio::test]
async fn two_searched_revisions_of_an_episode_leave_the_higher_one() {
    let s = Setup::new(Options::show()).await;
    let v1 = s.first_release().await;
    let (v2, v3) = (show_14(2, V2), show_14(3, V3));
    s.nyaa.set_releases(&[v3.clone(), v2.clone(), v1.clone()]);
    s.on_add(&v2, V2);
    s.on_add(&v3, V3);
    let poll = s.search("[SubsPlease] Show 1080p", 14, 14).await;
    for title in [&v3, &v2] {
        let command = s.receive_title(&poll, title).await;
        assert_eq!(command["outcome"]["result"], "received", "{command}");
    }

    s.seed(&v2);
    s.cycle().await;
    assert_eq!(
        s.episode_14(),
        V1,
        "the lower revision does not take the name"
    );
    assert_eq!(s.revision_of(&v2).await, Some(RevisionState::Skipped));

    s.seed(&v3);
    s.cycle().await;
    assert_eq!(s.episode_14(), V3);
    assert_eq!(s.revision_of(&v3).await, Some(RevisionState::Done));
    assert_eq!(s.revision_of(&v2).await, Some(RevisionState::Skipped));
    // v2 keeps its own name; only the episode name was replaced.
    assert_eq!(std::fs::read(s.folder.join(&v2)).unwrap(), V2);
}

/// A revision chosen from a search while the feed's lower revision is on its
/// way: the feed's one finishing first is skipped and the searched one
/// replaces the video.
#[tokio::test]
async fn a_searched_revision_higher_than_the_feeds_open_one_is_the_one_that_replaces() {
    let s = Setup::new(Options::show()).await;
    let v1 = s.first_release().await;
    let (v2, v3) = (show_14(2, V2), show_14(3, V3));
    s.nyaa.set_releases(&[v2.clone(), v1.clone()]);
    s.on_add(&v2, V2);
    s.cycle().await;
    assert_eq!(s.revision_of(&v2).await, Some(RevisionState::Receiving));

    s.nyaa.set_releases(&[v3.clone(), v2.clone(), v1.clone()]);
    s.on_add(&v3, V3);
    let poll = s.search("[SubsPlease] Show 1080p", 14, 14).await;
    let command = s.receive_title(&poll, &v3).await;
    assert_eq!(command["outcome"]["result"], "received", "{command}");
    assert_eq!(s.revision_of(&v3).await, Some(RevisionState::Receiving));

    s.seed(&v2);
    s.cycle().await;
    assert_eq!(s.episode_14(), V1);
    assert_eq!(s.revision_of(&v2).await, Some(RevisionState::Skipped));

    s.seed(&v3);
    s.cycle().await;
    assert_eq!(s.episode_14(), V3);
    assert_eq!(s.revision_of(&v3).await, Some(RevisionState::Done));
}

/// A lower revision chosen from a search after a higher one replaced the
/// video is not added.
#[tokio::test]
async fn a_searched_revision_lower_than_the_one_that_replaced_the_video_is_not_added() {
    let s = Setup::new(Options::show()).await;
    let v1 = s.first_release().await;
    let (v2, v3) = (show_14(2, V2), show_14(3, V3));
    s.nyaa.set_releases(&[v3.clone(), v1.clone()]);
    s.on_add(&v3, V3);
    let poll = s.search("[SubsPlease] Show 1080p", 14, 14).await;
    let command = s.receive_title(&poll, &v3).await;
    assert_eq!(command["outcome"]["result"], "received", "{command}");
    s.seed(&v3);
    s.cycle().await;
    assert_eq!(s.episode_14(), V3);

    s.nyaa.set_releases(&[v3.clone(), v2.clone(), v1.clone()]);
    let poll = s.search("[SubsPlease] Show 1080p", 14, 14).await;
    let command = s.receive_title(&poll, &v2).await;
    assert_eq!(command["outcome"]["result"], "duplicate", "{command}");
    let v2_hash = FakeNyaa::hash_for(&v2);
    assert!(!s.added().iter().any(|link| link.contains(&v2_hash)));
    assert_eq!(s.episode_14(), V3);
}

/// The replacement row of a searched revision is written with the item's
/// `received`: when it cannot be written the command runs again, instead of
/// ending with the revision received and nothing to replace the video.
#[tokio::test]
async fn a_searched_revision_whose_replacement_is_not_written_runs_again_and_replaces() {
    let s = Setup::new(Options::show()).await;
    let v1 = s.first_release().await;
    let v2 = show_14(2, V2);
    s.nyaa.set_releases(&[v2.clone(), v1.clone()]);
    s.on_add(&v2, V2);
    let poll = s.search("[SubsPlease] Show 1080p", 14, 14).await;
    // The feed no longer has it: only the search's `받기` receives it.
    s.nyaa.set_releases(std::slice::from_ref(&v1));
    let sql = |sql: &str| {
        rusqlite::Connection::open(s.h.db_path())
            .unwrap()
            .execute_batch(sql)
            .unwrap()
    };
    sql("CREATE TRIGGER no_row BEFORE INSERT ON video_revisions
         BEGIN SELECT RAISE(ABORT, 'injected'); END;");
    let key = item(&poll, &v2)["key"].as_str().unwrap().to_owned();
    let id = s.receive(&poll, &key).await;
    s.run_commands().await;
    sql("DROP TRIGGER no_row;");
    // Runs it again if it is still to run.
    s.h.worker()
        .run_commands(&CancellationToken::new())
        .await
        .unwrap();
    let command = s.command(&id).await;
    assert_eq!(command["outcome"]["result"], "received", "{command}");
    assert_eq!(s.revision_of(&v2).await, Some(RevisionState::Receiving));

    s.seed(&v2);
    s.cycle().await;
    assert_eq!(s.episode_14(), V2);
    assert_eq!(s.revision_of(&v2).await, Some(RevisionState::Done));
}

/// A revision chosen in a past episode search whose download stopped (its
/// torrent left Transmission) is offered `다시 받기`, though no feed has it,
/// and replaces the video once received again.
#[tokio::test]
async fn a_searched_revision_whose_download_stopped_is_received_again_with_retry() {
    let s = Setup::new(Options::show()).await;
    let v1 = s.first_release().await;
    let v2 = show_14(2, V2);
    s.nyaa.set_releases(&[v2.clone(), v1.clone()]);
    s.on_add(&v2, V2);
    let poll = s.search("[SubsPlease] Show 1080p", 14, 14).await;
    s.nyaa.set_releases(std::slice::from_ref(&v1));
    let command = s.receive_title(&poll, &v2).await;
    assert_eq!(command["outcome"]["result"], "received", "{command}");
    assert_eq!(s.revision_of(&v2).await, Some(RevisionState::Receiving));

    let hash = FakeNyaa::hash_for(&v2);
    s.h.tr.remove(&hash);
    s.cycle().await;
    assert_eq!(s.revision_of(&v2).await, Some(RevisionState::Failed));
    let (status, failures) = s.call("GET", "/api/todo/receive-failures", None).await;
    assert_eq!(status, StatusCode::OK, "{failures}");
    let failure = &failures["items"][0];
    assert_eq!(failure["kind"], "revision", "{failures}");
    assert_eq!(failure["can_retry"], true, "{failure}");

    s.on_add(&v2, V2);
    let (status, body) = s
        .call(
            "POST",
            "/api/commands",
            Some(json!({
                "id": "00000000-0000-4000-8000-0000000000b1",
                "kind": "receive_once",
                "payload": { "item_id": failure["history_item_id"] },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    s.run_commands().await;
    assert_eq!(s.revision_of(&v2).await, Some(RevisionState::Receiving));
    assert_eq!(s.episode_14(), V1);

    s.seed(&v2);
    s.cycle().await;
    assert_eq!(s.episode_14(), V2);
    assert_eq!(s.revision_of(&v2).await, Some(RevisionState::Done));
}

// --- a `받기` whose add got no answer -----------------------------------------------

/// A `받기` whose add got no answer though Transmission took the torrent, and
/// whose rule is paused before the next start: the command ends at once, the
/// torrent stays unaccounted for so the next cycle removes nothing, and the
/// command's label comes off it.
#[tokio::test]
async fn a_paused_rule_after_an_unanswered_add_ends_the_command_and_holds_the_next_cleanup() {
    let s = Setup::new(Options::show()).await;
    let title = episode("SubsPlease", "Show", 3, "");
    s.nyaa.set_releases(std::slice::from_ref(&title));
    let poll = s.search("[SubsPlease] Show 1080p", 3, 3).await;
    let key = item(&poll, &title)["key"].as_str().unwrap().to_owned();
    let id = s.receive(&poll, &key).await;

    let late = s.h.tr.hold_answer("torrent-add");
    let impatient =
        s.h.worker()
            .with_transmission_timeout(Duration::from_millis(300));
    let cancel = CancellationToken::new();
    assert_eq!(
        impatient.run_commands(&cancel).await.unwrap(),
        CommandsOutcome::Ran(0)
    );
    late.release_all();
    assert_eq!(s.command(&id).await["state"], "running");
    let label = trss_transmission::command_label(&id);
    assert!(s.h.tr.torrents()[0].labels.contains(&label));

    s.h.channels
        .set_rule_state(&s.rule_id, RuleState::Paused, 1)
        .await
        .unwrap();
    assert_eq!(
        impatient.run_commands(&cancel).await.unwrap(),
        CommandsOutcome::Ran(1)
    );
    let command = s.command(&id).await;
    assert_eq!(command["state"], "failed", "{command}");
    assert!(!s.h.tr.torrents()[0].labels.contains(&label));

    // The search's result is in no feed, and the torrent's hash was never
    // learned: only the unanswered add keeps it from the cleanup.
    s.nyaa.set_releases(&[]);
    let TickOutcome::Ran(report) = s.h.worker().tick(&cancel).await.unwrap() else {
        panic!("expected a cycle");
    };
    assert_eq!(report.commands_unconfirmed, 1);
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert_eq!(s.h.tr.torrents().len(), 1);
}

// --- an episode received before whose video and torrent are gone -------------------------

impl Setup {
    /// Searches episodes 5 to 8 and receives the fifth; returns its key and
    /// torrent hash.
    async fn receive_five(&self) -> (String, String) {
        let titles: Vec<String> = (5..=8)
            .rev()
            .map(|n| episode("SubsPlease", "Show", n, ""))
            .collect();
        self.nyaa.set_releases(&titles);
        let poll = self.search("[SubsPlease] Show 1080p", 5, 8).await;
        assert_eq!(poll["state"], "done", "{poll}");
        let key = item(&poll, "- 05 ")["key"].as_str().unwrap().to_owned();
        let id = self.receive(&poll, &key).await;
        self.run_commands().await;
        assert_eq!(self.command(&id).await["outcome"]["result"], "received");
        let hash = FakeNyaa::hash_for(&episode("SubsPlease", "Show", 5, ""));
        assert!(self.h.tr.torrents().iter().any(|t| t.hash == hash));
        (key, hash)
    }

    /// The worker's next look at Transmission, after the clock moved on: the
    /// list of the torrents it holds, as a cycle leaves it (a cycle itself
    /// would also add the feed's other episodes, which these tests do not
    /// want; `status_snapshots_from_cycle` covers the cycle).
    async fn look_at_transmission(&self) {
        self.h.advance(60_000);
        let hashes = self.h.tr.torrents().into_iter().map(|t| t.hash).collect();
        StatusStore::new(self.h.db.clone())
            .record_listing(self.h.now(), hashes)
            .await
            .unwrap();
    }

    /// Asks for `key` of `poll` to be received; the answer's status.
    async fn try_receive(&self, poll: &Value, key: &str) -> StatusCode {
        let (status, _) = self
            .call(
                "POST",
                "/api/commands",
                Some(json!({
                    "id": "past-cmd-refused",
                    "kind": "receive_past",
                    "payload": {
                        "rule_id": self.rule_id,
                        "search_id": poll["search_id"],
                        "key": key,
                    },
                })),
            )
            .await;
        status
    }
}

fn choice(item: &Value) -> (Value, Value, Value) {
    (
        item["state"].clone(),
        item["selected"].clone(),
        item["selectable"].clone(),
    )
}

#[tokio::test]
async fn an_episode_whose_video_and_torrent_are_gone_is_missing_and_received_again() {
    let s = Setup::new(Options::show()).await;
    let (key, hash) = s.receive_five().await;

    // The person deleted the video (it never reached the folder here) and
    // removed the torrent from Transmission.
    s.h.tr.remove(&hash);
    s.look_at_transmission().await;

    let poll = s.search("[SubsPlease] Show 1080p", 5, 8).await;
    assert_eq!(
        choice(item(&poll, "- 05 ")),
        (json!("missing"), json!(true), json!(true))
    );
    assert_eq!(selected(&poll), vec![5, 6, 7, 8]);
    assert_eq!(poll["result"]["missing"], json!([5, 6, 7, 8]));

    let id = s.receive(&poll, &key).await;
    s.run_commands().await;
    let command = s.command(&id).await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "received", "{command}");
    let added = s.added();
    assert_eq!(
        added.iter().filter(|a| a.contains(&hash)).count(),
        2,
        "{added:?}"
    );
    assert!(s.h.tr.torrents().iter().any(|t| t.hash == hash));
    // One history item, still received.
    let fives: Vec<_> =
        s.h.history_items()
            .await
            .into_iter()
            .filter(|i| i.title.contains("- 05 "))
            .collect();
    assert_eq!(fives.len(), 1);
    assert_eq!(fives[0].result, HistoryResult::Received);
}

#[tokio::test]
async fn an_episode_whose_video_is_in_the_folder_is_had_though_its_torrent_is_gone() {
    let s = Setup::new(Options::show()).await;
    let (key, hash) = s.receive_five().await;

    // Seeding is over: the torrent was removed, the video stays.
    s.h.tr.remove(&hash);
    s.write("Show S01E05.mkv", b"x");
    s.look_at_transmission().await;

    let poll = s.search("[SubsPlease] Show 1080p", 5, 8).await;
    assert_eq!(
        choice(item(&poll, "- 05 ")),
        (json!("have"), json!(false), json!(false))
    );
    assert_eq!(selected(&poll), vec![6, 7, 8]);
    // The screen does not offer it and the web does not take it.
    assert_eq!(s.try_receive(&poll, &key).await, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_episode_whose_torrent_is_in_transmission_cannot_be_chosen_before_its_video_is_placed() {
    let s = Setup::new(Options::show()).await;
    let (key, _) = s.receive_five().await;
    s.look_at_transmission().await;

    let poll = s.search("[SubsPlease] Show 1080p", 5, 8).await;
    assert_eq!(
        choice(item(&poll, "- 05 ")),
        (json!("have"), json!(false), json!(false))
    );
    assert_eq!(selected(&poll), vec![6, 7, 8]);
    assert_eq!(s.try_receive(&poll, &key).await, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_result_offered_from_an_older_look_is_not_added_when_its_torrent_is_back() {
    let s = Setup::new(Options::show()).await;
    let (key, hash) = s.receive_five().await;
    s.h.tr.remove(&hash);
    s.look_at_transmission().await;
    let poll = s.search("[SubsPlease] Show 1080p", 5, 8).await;
    assert_eq!(item(&poll, "- 05 ")["selectable"], true);

    // Before the person confirms, the torrent is added to Transmission by hand.
    let title = episode("SubsPlease", "Show", 5, "");
    s.h.tr
        .preload(FakeTorrent::new(&hash, &title).in_dir(&s.folder));
    s.h.tr.clear_calls();
    let id = s.receive(&poll, &key).await;
    s.run_commands().await;

    // Transmission has the torrent: the worker does not add it a second time.
    let command = s.command(&id).await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "received", "{command}");
    assert!(s.added().is_empty(), "{:?}", s.added());
}

#[tokio::test]
async fn a_result_offered_from_an_older_look_is_not_added_when_its_video_is_back() {
    let s = Setup::new(Options::show()).await;
    let (key, hash) = s.receive_five().await;
    s.h.tr.remove(&hash);
    s.look_at_transmission().await;
    let poll = s.search("[SubsPlease] Show 1080p", 5, 8).await;
    assert_eq!(item(&poll, "- 05 ")["selectable"], true);

    s.write("Show S01E05.mkv", b"x");
    s.h.tr.clear_calls();
    let id = s.receive(&poll, &key).await;
    s.run_commands().await;

    let command = s.command(&id).await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "received", "{command}");
    assert!(s.added().is_empty(), "{:?}", s.added());
}

// --- what is not taken for gone ----------------------------------------------------------

/// The result of episode 5 offered again (its torrent is removed, its video
/// was never placed), and the poll that offered it.
async fn offered_again(s: &Setup) -> (Value, String) {
    let (key, hash) = s.receive_five().await;
    s.h.tr.remove(&hash);
    s.look_at_transmission().await;
    let poll = s.search("[SubsPlease] Show 1080p", 5, 8).await;
    assert_eq!(item(&poll, "- 05 ")["selectable"], true, "{poll}");
    (poll, key)
}

#[tokio::test]
async fn a_work_folder_that_is_not_there_does_not_make_a_received_episode_gone() {
    let s = Setup::new(Options::show()).await;
    let (key, hash) = s.receive_five().await;
    s.h.tr.remove(&hash);
    s.look_at_transmission().await;

    // The media volume is not mounted: no video is seen, and none is gone.
    std::fs::remove_dir_all(&s.folder).unwrap();
    let poll = s.search("[SubsPlease] Show 1080p", 5, 8).await;
    assert_eq!(
        choice(item(&poll, "- 05 ")),
        (json!("have"), json!(false), json!(false))
    );
    assert_eq!(s.try_receive(&poll, &key).await, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_folder_that_vanishes_after_the_search_stops_the_worker_adding_the_result() {
    let s = Setup::new(Options::show()).await;
    let (poll, key) = offered_again(&s).await;

    std::fs::remove_dir_all(&s.folder).unwrap();
    s.h.tr.clear_calls();
    let id = s.receive(&poll, &key).await;
    s.run_commands().await;
    assert_eq!(s.command(&id).await["state"], "done");
    assert!(s.added().is_empty(), "{:?}", s.added());
}

#[tokio::test]
async fn a_folder_with_more_entries_than_are_looked_at_holds_a_received_episode() {
    let s = Setup::new(Options::show()).await;
    let (key, hash) = s.receive_five().await;
    s.h.tr.remove(&hash);
    s.look_at_transmission().await;
    for n in 0..5001 {
        s.write(&format!("{n}.txt"), b"");
    }
    let poll = s.search("[SubsPlease] Show 1080p", 5, 8).await;
    assert_eq!(
        choice(item(&poll, "- 05 ")),
        (json!("have"), json!(false), json!(false))
    );
    assert_eq!(s.try_receive(&poll, &key).await, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_batch_whose_torrent_is_removed_stays_held() {
    let s = Setup::new(Options::show()).await;
    let batch = "[SubsPlease] Show (05-08) (1080p) [Batch]".to_owned();
    let mut titles: Vec<String> = (5..=8)
        .map(|n| episode("SubsPlease", "Show", n, ""))
        .collect();
    titles.push(batch.clone());
    s.nyaa.set_releases(&titles);
    let poll = s.search("[SubsPlease] Show 1080p", 5, 8).await;
    let key = item(&poll, "Batch")["key"].as_str().unwrap().to_owned();
    s.h.history
        .record(
            s.h.now(),
            vec![Observation {
                channel_id: s.channel_id.clone(),
                channel_label: "nyaa".into(),
                identity_key: key.clone(),
                title: batch.clone(),
                link: "magnet:?xt=urn:btih:bbbb".into(),
                result: HistoryResult::Received,
                rule_id: Some(s.rule_id.clone()),
                torrent_hash: Some("bbbb".into()),
                reason: None,
            }],
        )
        .await
        .unwrap();
    // The batch's torrent is gone from Transmission, as they are once done.
    s.look_at_transmission().await;

    let poll = s.search("[SubsPlease] Show 1080p", 5, 8).await;
    assert_eq!(item(&poll, "Batch")["selectable"], false, "{poll}");
    assert_eq!(s.try_receive(&poll, &key).await, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_rule_that_is_paused_after_the_search_does_not_receive_a_gone_result() {
    let s = Setup::new(Options::show()).await;
    let (poll, key) = offered_again(&s).await;
    s.h.channels
        .set_rule_state(&s.rule_id, RuleState::Paused, 1)
        .await
        .unwrap();
    assert_eq!(s.try_receive(&poll, &key).await, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn another_torrent_of_the_rule_for_the_episode_stops_the_worker_adding_a_gone_result() {
    let s = Setup::new(Options::show()).await;
    let (poll, key) = offered_again(&s).await;

    // Meanwhile the same episode came from another release, which Transmission holds.
    let hash = "eeee00000000000000000000000000000000000a";
    let erai = "[Erai-raws] Show - 05 [1080p][ABCD1234].mkv";
    s.h.history
        .record(
            1,
            vec![Observation {
                channel_id: s.channel_id.clone(),
                channel_label: "nyaa".into(),
                identity_key: "guid:erai05".into(),
                title: erai.into(),
                link: format!("magnet:?xt=urn:btih:{hash}"),
                result: HistoryResult::Received,
                rule_id: Some(s.rule_id.clone()),
                torrent_hash: Some(hash.into()),
                reason: None,
            }],
        )
        .await
        .unwrap();
    s.h.tr
        .preload(FakeTorrent::new(hash, erai).in_dir(&s.folder));
    s.h.tr.clear_calls();
    let id = s.receive(&poll, &key).await;
    s.run_commands().await;
    assert_eq!(s.command(&id).await["state"], "done");
    assert!(s.added().is_empty(), "{:?}", s.added());
}

#[tokio::test]
async fn a_lower_revision_that_a_higher_one_replaced_is_not_received_again_when_the_folder_is_empty(
) {
    let s = Setup::new(Options::show()).await;
    let v1 = s.first_release().await;
    let v3 = show_14(3, V3);
    s.nyaa.set_releases(&[v3.clone(), v1.clone()]);
    s.on_add(&v3, V3);
    let poll = s.search("[SubsPlease] Show 1080p", 14, 14).await;
    let command = s.receive_title(&poll, &v3).await;
    assert_eq!(command["outcome"]["result"], "received", "{command}");
    s.seed(&v3);
    s.cycle().await;
    assert_eq!(s.episode_14(), V3);

    // The person deletes the video and removes the torrent: the folder has no
    // episode 14, and v1, which v3 replaced, is gone from the work too.
    s.h.tr.remove(&FakeNyaa::hash_for(&v3));
    std::fs::remove_file(s.folder.join("Show S01E14.mkv")).unwrap();
    s.look_at_transmission().await;
    let poll = s.search("[SubsPlease] Show 1080p", 14, 14).await;
    assert_eq!(item(&poll, &v1)["selectable"], true, "{poll}");

    s.h.tr.clear_calls();
    let command = s.receive_title(&poll, &v1).await;
    assert_eq!(command["outcome"]["result"], "duplicate", "{command}");
    assert!(s.added().is_empty(), "{:?}", s.added());
}
