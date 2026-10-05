//! The app-wide collect folder, end to end (ticket 0010).
//!
//! - While no collect folder is set the worker adds nothing and records no
//!   failure, and the status board says why; once it is set the items a rule
//!   picked are received by the next cycle.
//! - A database migrated from per-channel base folders gives every rule the
//!   folder it had before, whether the rule is run by the collection cycle,
//!   previewed on the rules tab or received again with `다시 받기`.
//!
//! The channel URL's token is made up.

use crate::common;

use std::collections::HashMap;
use std::path::Path;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use trss_collect::{
    rss::save_path,
    store::{
        channels::ChannelStore,
        history::{HistoryQuery, HistoryResult, HistoryStore, MAX_PAGE_SIZE},
    },
};
use trss_core::{settings::SettingsStore, Db};
use trss_worker::{CommandsOutcome, CycleReport, TickOutcome, Worker};

async fn run(worker: &Worker) -> CycleReport {
    match worker.tick(&CancellationToken::new()).await.unwrap() {
        TickOutcome::Ran(report) => report,
        other => panic!("expected a cycle, got {other:?}"),
    }
}

fn add_dirs(h: &Harness) -> Vec<String> {
    let mut dirs: Vec<String> =
        h.tr.calls_of("torrent-add")
            .iter()
            .map(|c| c.args["download-dir"].as_str().unwrap().to_owned())
            .collect();
    dirs.sort();
    dirs
}

async fn board(api: &WebApi) -> Value {
    let (status, text, board) = api.call("GET", "/api/collect/status", None).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    board
}

// --- no collect folder -------------------------------------------------------------

#[tokio::test]
async fn with_no_collect_folder_nothing_is_added_nothing_fails_and_the_board_says_why() {
    let h = Harness::without_collect_folder().await;
    h.add_channel(
        "feed-a",
        "/media/anime",
        &["[Batch]", "(720p)"],
        feed_a_rules(),
    )
    .await;
    // A finished bot torrent whose item left every feed would be removed by a
    // cycle that could judge the feeds against what it added.
    h.tr.preload(
        FakeTorrent::new(
            "gone0000000000000000000000000000000000aa",
            "Old Show - 12.mkv",
        )
        .bot(),
    );
    let api = h.web_api();
    assert_eq!(board(&api).await["collect_folder_set"], false);

    let report = run(&h.worker()).await;

    // The four items no rule picks are judged and recorded as usual; the three a
    // rule picks wait, unrecorded, so nothing piles up as `add_failed`.
    assert_eq!(report.channels_read, 1, "{report:?}");
    assert_eq!(report.items_seen, 7, "{report:?}");
    assert_eq!(report.waiting_for_collect_folder, 3, "{report:?}");
    assert_eq!(
        (report.added, report.duplicates, report.add_failed),
        (0, 0, 0)
    );
    assert_eq!((report.excluded, report.no_match), (2, 2), "{report:?}");
    assert!(h.tr.calls_of("torrent-add").is_empty());
    assert!(
        h.tr.calls_of("torrent-remove").is_empty(),
        "no torrent is removed either"
    );
    let items = h.history_items().await;
    assert_eq!(items.len(), 4, "{items:?}");
    assert!(items
        .iter()
        .all(|i| matches!(i.result, HistoryResult::Excluded | HistoryResult::NoMatch)));
    let status = board(&api).await;
    assert_eq!(status["collect_folder_set"], false);
    assert_eq!(status["problems"], 0);

    // The cycle again, still unset: the same, and still nothing recorded for them.
    h.advance(300_000);
    let again = run(&h.worker()).await;
    assert_eq!(again.waiting_for_collect_folder, 3, "{again:?}");
    assert_eq!(h.history_items().await.len(), 4);

    // Choosing the folder is all it takes: the next cycle judges those items as
    // new and receives them where their rules say.
    SettingsStore::new(h.db.clone())
        .put_collection(0, "/media".to_owned(), None)
        .await
        .unwrap();
    assert_eq!(board(&api).await["collect_folder_set"], true);
    h.advance(300_000);
    let report = run(&h.worker()).await;
    assert_eq!((report.added, report.add_failed), (3, 0), "{report:?}");
    assert_eq!(report.items_new, 3, "{report:?}");
    assert_eq!(report.waiting_for_collect_folder, 0);
    assert_eq!(
        add_dirs(&h),
        [
            "/media/anime/Sayonara Lara/Season 01",
            "/media/anime/Slime/Season 04",
            "/media/anime/Sono Bisque Doll/Season 02",
        ]
    );
    let items = h.history_items().await;
    assert_eq!(
        items
            .iter()
            .filter(|i| i.result == HistoryResult::Received)
            .count(),
        3
    );
    assert!(items.iter().all(|i| i.result != HistoryResult::AddFailed));
}

// --- a migrated database -------------------------------------------------------------

/// The database as the build before the collect folder left it: migrations 1
/// to 6, with the channels' base folders.
fn database_before_the_collect_folder(path: &Path) -> rusqlite::Connection {
    trss_core::db::database_at(path, 6)
}

fn old_channel(conn: &rusqlite::Connection, id: &str, position: i64, url: &str, base: &str) {
    conn.execute(
        "INSERT INTO channels (id, position, url, base_dir, excludes, secret_query, version)
         VALUES (?1, ?2, ?3, ?4, '[\"[Batch]\",\"(720p)\"]', '[\"filter\",\"token\"]', 1)",
        rusqlite::params![id, position, url, base],
    )
    .unwrap();
}

fn old_rule(
    conn: &rusqlite::Connection,
    id: &str,
    channel: &str,
    position: i64,
    phrase: &str,
    directory: &str,
) {
    conn.execute(
        "INSERT INTO rules (id, channel_id, position, match_text, regex, case_insensitive,
                            directory, episode, episode_auto, state, version)
         VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, 1, 0, 'active', 1)",
        rusqlite::params![id, channel, position, phrase, directory],
    )
    .unwrap();
}

/// The title a magnet link carries as its `dn`.
fn title_of(magnet: &str) -> String {
    url::Url::parse(magnet)
        .unwrap()
        .query_pairs()
        .find(|(name, _)| name == "dn")
        .map(|(_, value)| value.into_owned())
        .unwrap()
}

/// Ticket 0010: after the migration the rule cycle, the rule preview and
/// `다시 받기` give the same item the same folder, and it is the folder the
/// rule's channel had before.
#[tokio::test]
async fn after_the_migration_cycle_preview_and_retry_put_an_item_in_the_same_folder() {
    let h = Harness::without_collect_folder().await;
    let old_path = h.dir.path().join("old.db");
    let url_a = format!("{}?filter=1080p&token={SECRET}", h.feeds.url("feed-a"));
    let url_b = format!("{}?filter=1080p&token={SECRET}", h.feeds.url("feed-b"));

    // Two channels with different base folders, and the rules of the sample.
    let mut before: HashMap<String, std::ffi::OsString> = HashMap::new();
    {
        let conn = database_before_the_collect_folder(&old_path);
        old_channel(&conn, "ca", 0, &url_a, "/media/anime");
        old_channel(&conn, "cb", 1, &url_b, "/media/other/");
        let rules = [
            (
                "ca",
                "[SubsPlease] Sayonara Lara - ",
                "Sayonara Lara/Season 01",
            ),
            ("ca", "sono bisque doll", "Sono Bisque Doll/Season 02"),
            (
                "ca",
                "[SubsPlease] Tensei Shitara Slime Datta Ken",
                "Slime/Season 04",
            ),
            ("cb", "Sayonara Lara", "Elsewhere/Sayonara Lara/Season 01"),
            ("cb", "Never Aired", ""),
        ];
        for (position, (channel, phrase, directory)) in rules.iter().enumerate() {
            let id = format!("r{position}");
            old_rule(&conn, &id, channel, position as i64, phrase, directory);
            let base = if *channel == "ca" {
                "/media/anime"
            } else {
                "/media/other/"
            };
            // What the cycle passed to Transmission before: base and directory joined.
            before.insert(
                id,
                save_path(Path::new(base), Path::new(directory)).into_os_string(),
            );
        }
    }

    let db = Db::open(&old_path).await.unwrap();
    let collect = SettingsStore::new(db.clone())
        .collection()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(collect.folder, "/media");
    let channels = ChannelStore::new(db.clone())
        .list_channels_with_rules()
        .await
        .unwrap();
    let api = WebApi::new(db.clone());
    let worker = h.worker_with_db(db.clone());

    // 1. The cycle, with Transmission refusing the adds: the requests tell where
    // each item was sent, and the items are left `add_failed` for a retry.
    h.tr.reject_adds(Some("refused for the test"));
    let report = run(&worker).await;
    assert!(report.add_failed >= 3, "{report:?}");
    let by_cycle: HashMap<String, String> =
        h.tr.calls_of("torrent-add")
            .iter()
            .map(|c| {
                (
                    title_of(c.args["filename"].as_str().unwrap()),
                    c.args["download-dir"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
    assert!(by_cycle.len() >= 3, "{by_cycle:?}");

    // 2. The preview of every rule, from the same recorded items.
    let (status, text, list) = api.call("GET", "/api/rules", None).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(list["collect_folder"], "/media");
    let mut by_preview: HashMap<String, String> = HashMap::new();
    for rule in list["rules"].as_array().unwrap() {
        let body = json!({
            "channel_id": rule["channel_id"],
            "rule_id": rule["id"],
            "rule": {
                "match": rule["match"], "regex": rule["regex"],
                "case_insensitive": rule["case_insensitive"],
                "directory": rule["directory"], "episode": rule["episode"],
            },
        });
        let (status, text, preview) = api.call("POST", "/api/rules/preview", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        for item in preview["items"].as_array().unwrap() {
            if item["kind"] == "mine" {
                by_preview.insert(
                    item["title"].as_str().unwrap().to_owned(),
                    item["save_path"].as_str().unwrap().to_owned(),
                );
            }
        }
    }

    // 3. `다시 받기` of every item the rules picked, now that Transmission takes adds.
    h.tr.reject_adds(None);
    h.tr.clear_calls();
    let failed: Vec<_> = HistoryStore::new(db.clone())
        .list(HistoryQuery {
            limit: MAX_PAGE_SIZE,
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .into_iter()
        .filter(|i| i.result == HistoryResult::AddFailed)
        .collect();
    assert_eq!(failed.len(), by_cycle.len());
    for (n, item) in failed.iter().enumerate() {
        let (status, text, _) = api
            .call(
                "POST",
                "/api/commands",
                Some(json!({
                    "id": format!("retry-{n:04}-0000"),
                    "kind": "receive_once",
                    "payload": { "item_id": item.id },
                })),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    }
    assert_eq!(
        worker
            .run_commands(&CancellationToken::new())
            .await
            .unwrap(),
        CommandsOutcome::Ran(failed.len())
    );
    let by_retry: HashMap<String, String> =
        h.tr.calls_of("torrent-add")
            .iter()
            .map(|c| {
                (
                    title_of(c.args["filename"].as_str().unwrap()),
                    c.args["download-dir"].as_str().unwrap().to_owned(),
                )
            })
            .collect();

    // Every item the rules picked went to one folder on all three paths, and it
    // is the very text the old base folder and rule directory made.
    for item in &failed {
        let expected = before[item.rule_id.as_deref().unwrap()]
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            by_cycle.get(&item.title),
            Some(&expected),
            "cycle: {}",
            item.title
        );
        assert_eq!(
            by_preview.get(&item.title),
            Some(&expected),
            "preview: {}",
            item.title
        );
        assert_eq!(
            by_retry.get(&item.title),
            Some(&expected),
            "retry: {}",
            item.title
        );
    }
    assert_eq!(by_cycle.len(), by_preview.len());
    assert_eq!(by_cycle.len(), by_retry.len());
    // The folder with an empty rule directory keeps its trailing slash.
    assert!(
        by_cycle.values().any(|d| d == "/media/other/"),
        "{by_cycle:?}"
    );
    assert_eq!(channels.len(), 2);
}

// --- `다시 받기` without a collect folder -----------------------------------------

/// A retry has nowhere to put the torrent while no collect folder is set: the
/// command ends with that reason, Transmission is not asked, and the item is
/// left as it was (`add_failed` from before, not a new failure).
#[tokio::test]
async fn a_retry_without_a_collect_folder_ends_with_the_reason_and_leaves_the_item() {
    let h = Harness::new().await;
    h.add_channel("feed-a", "/media/anime", &["[Batch]"], feed_a_rules())
        .await;
    h.tr.reject_adds(Some("refused for the test"));
    let report = run(&h.worker()).await;
    assert!(report.add_failed >= 3, "{report:?}");
    h.tr.reject_adds(None);
    h.tr.clear_calls();

    // The folder goes away (no screen removes it; this is the state a database
    // that never had one would be in, with items that were refused before).
    let conn = rusqlite::Connection::open(h.dir.path().join("app.db")).unwrap();
    conn.execute("DELETE FROM collection_settings", []).unwrap();

    let item = h
        .history_items()
        .await
        .into_iter()
        .find(|i| i.result == HistoryResult::AddFailed)
        .unwrap();
    let api = h.web_api();
    let (status, text, _) = api
        .call(
            "POST",
            "/api/commands",
            Some(json!({
                "id": "retry-no-folder-0001",
                "kind": "receive_once",
                "payload": { "item_id": item.id },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    h.worker()
        .run_commands(&CancellationToken::new())
        .await
        .unwrap();

    let (status, text, command) = api
        .call("GET", "/api/commands/retry-no-folder-0001", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(command["state"], "failed", "{command}");
    assert!(
        command["outcome"]["reason"]
            .as_str()
            .unwrap()
            .contains("수집 폴더"),
        "{command}"
    );
    assert!(h.tr.calls_of("torrent-add").is_empty());
    let after = h
        .history_items()
        .await
        .into_iter()
        .find(|i| i.id == item.id)
        .unwrap();
    assert_eq!(after.result, HistoryResult::AddFailed);
    assert_eq!(
        after.reason, item.reason,
        "the item keeps its earlier reason"
    );
}
