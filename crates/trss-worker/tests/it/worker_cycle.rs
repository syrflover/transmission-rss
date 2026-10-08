//! The worker's collection cycle against a fake Transmission and fake RSS
//! feeds, with the real Transmission client and a real (temporary) app database.
//!
//! What a cycle decides about an item (which rule takes it, what its torrent
//! is named, what stays in Transmission and why, how secrets are masked) is
//! tested in trss-collect (`cycle/tests.rs`, ADR 0015). This file keeps what
//! only the worker shows: the report, the cycle marker and the lock, the
//! order of cycles, edits made while a cycle runs, and what a cycle does when
//! Transmission hangs, stops or the worker shuts down.

use crate::common;

use common::*;
use tokio_util::sync::CancellationToken;
use trss_collect::store::{
    channels::RuleInput,
    history::{HistoryQuery, HistoryResult, MAX_PAGE_SIZE},
};
use trss_worker::{CycleReport, TickOutcome, Worker};

async fn run(worker: &Worker) -> CycleReport {
    match worker.tick(&CancellationToken::new()).await.unwrap() {
        TickOutcome::Ran(report) => report,
        other => panic!("expected a cycle, got {other:?}"),
    }
}

fn hash_a(n: u32) -> String {
    format!("aaaa{n:036}")
}

async fn channel_a(h: &Harness) -> trss_collect::store::channels::ChannelWithRules {
    h.add_channel(
        "feed-a",
        "/media/anime",
        &["[Batch]", "(720p)"],
        feed_a_rules(),
    )
    .await
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

// --- what a cycle does -----------------------------------------------------------------

#[tokio::test]
async fn a_cycle_adds_the_selected_items_and_records_every_item() {
    let h = Harness::new().await;
    let channel = channel_a(&h).await;
    let report = run(&h.worker()).await;

    assert_eq!(report.channels_read, 1);
    assert_eq!(report.items_seen, 7);
    assert_eq!(report.items_new, 7);
    assert_eq!(
        (
            report.added,
            report.excluded,
            report.no_match,
            report.add_failed
        ),
        (3, 2, 2, 0)
    );

    // What each item was added as, named and recorded under which rule is the
    // collect crate's (`cycle/tests.rs`); the worker's cycle ran it once
    // and recorded every item of the feed with the channel and its first sight.
    assert_eq!(h.tr.calls_of("torrent-add").len(), 3);
    assert_eq!(h.tr.torrents().len(), 3);
    let items = h.history_items().await;
    assert_eq!(items.len(), 7);
    let count = |result: HistoryResult| items.iter().filter(|i| i.result == result).count();
    assert_eq!(count(HistoryResult::Received), 3);
    assert_eq!(count(HistoryResult::Excluded), 2);
    assert_eq!(count(HistoryResult::NoMatch), 2);
    for item in &items {
        assert_eq!(item.channel_id, channel.channel.id);
        assert_eq!(item.first_seen_at, 1_000_000);
    }
}

#[tokio::test]
async fn the_cycle_marker_records_start_and_finish() {
    let h = Harness::new().await;
    channel_a(&h).await;
    let worker = h.worker();

    run(&worker).await;
    let state = h.history.last_cycle().await.unwrap().unwrap();
    assert_eq!(state.started_at, 1_000_000);
    assert_eq!(state.finished_at, Some(1_000_000));
}

#[tokio::test]
async fn processing_the_same_feed_twice_adds_no_records_and_no_torrents() {
    let h = Harness::new().await;
    channel_a(&h).await;
    let worker = h.worker();

    run(&worker).await;
    let first = h.history_items().await;
    let torrents_after_first = h.tr.torrents();
    h.tr.clear_calls();

    h.advance(300_000);
    let report = run(&worker).await;

    // Transmission is asked again about each selected item (that is how
    // finished torrents get stopped), but it already has them.
    assert_eq!(report.added, 0);
    assert_eq!(report.duplicates, 3);
    assert_eq!(report.items_new, 0);
    assert_eq!(h.tr.torrents().len(), 3, "no torrent was added twice");
    assert_eq!(
        h.tr.torrents().iter().map(|t| &t.hash).collect::<Vec<_>>(),
        torrents_after_first
            .iter()
            .map(|t| &t.hash)
            .collect::<Vec<_>>()
    );
    assert!(
        h.tr.calls_of("torrent-remove").is_empty(),
        "torrents still in the feed are kept"
    );

    let second = h.history_items().await;
    assert_eq!(second.len(), first.len(), "no new records");
    for (a, b) in first.iter().zip(&second) {
        assert_eq!(a.id, b.id);
        assert_eq!(
            b.first_seen_at, 1_000_000,
            "first-seen time is the first cycle's"
        );
        assert_eq!(b.last_seen_at, 1_300_000);
        assert_eq!(
            a.result, b.result,
            "the duplicate answer does not undo `received`"
        );
        assert!(h.history.changes(b.id).await.unwrap().is_empty());
    }
}

// The cycle's own adds whose name stays keep their torrent, and the item notes
// why the name stayed, as `다시 받기` does.

#[tokio::test]
async fn every_torrent_the_cycle_holds_for_an_item_says_which_item_it_is() {
    let h = Harness::new().await;
    channel_a(&h).await;
    // A bot torrent from before the item labels (the legacy cron's), and one a
    // person added: only the bot's gets the item's label.
    h.tr.preload(FakeTorrent::new(&hash_a(1), "Sayonara Lara S01E03.mkv").bot());
    h.tr.preload(FakeTorrent::new(&hash_a(3), "Sono Bisque Doll S02E01.mkv"));

    run(&h.worker()).await;

    let items = h.history_items().await;
    let label_of = |n: u32| {
        let item = items
            .iter()
            .find(|i| i.torrent_hash.as_deref() == Some(hash_a(n).as_str()))
            .unwrap();
        format!("trss-item:{}:{}", item.channel_id, item.identity_key)
    };
    let labels_of = |n: u32| {
        h.tr.torrents()
            .into_iter()
            .find(|t| t.hash == hash_a(n))
            .unwrap()
            .labels
    };
    assert!(labels_of(1).contains(&label_of(1)), "{:?}", labels_of(1));
    assert!(labels_of(1).contains(&BOT_LABEL.to_owned()));
    assert!(labels_of(4).contains(&label_of(4)), "{:?}", labels_of(4));
    assert_eq!(
        labels_of(3),
        Vec::<String>::new(),
        "a person's torrent is left as it is"
    );
}

#[tokio::test]
async fn a_finished_bot_torrent_is_stopped_when_it_is_met_again() {
    let h = Harness::new().await;
    channel_a(&h).await;
    let worker = h.worker();

    run(&worker).await;
    h.tr.set_status(&hash_a(1), 6); // seeding
    h.tr.clear_calls();

    run(&worker).await;
    let stops = h.tr.calls_of("torrent-stop");
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0].args["ids"], serde_json::json!([hash_a(1)]));
    let stopped =
        h.tr.torrents()
            .into_iter()
            .find(|t| t.hash == hash_a(1))
            .unwrap();
    assert_eq!(stopped.status, 0);
}

#[tokio::test]
async fn torrents_that_left_the_feed_are_removed_but_only_bot_labelled_ones() {
    let h = Harness::new().await;
    channel_a(&h).await;
    h.tr.preload(FakeTorrent::new("gone0000000000000000000000000000000000aa", "Old Show").bot());
    h.tr.preload(FakeTorrent::new(
        "mine0000000000000000000000000000000000bb",
        "Manual Download",
    ));

    run(&h.worker()).await;

    let removes = h.tr.calls_of("torrent-remove");
    assert_eq!(removes.len(), 1);
    assert_eq!(
        removes[0].args["ids"],
        serde_json::json!(["gone0000000000000000000000000000000000aa"])
    );
    assert_eq!(removes[0].args["delete-local-data"], false, "data is kept");
    let hashes: Vec<String> = h.tr.torrents().into_iter().map(|t| t.hash).collect();
    assert!(hashes.contains(&"mine0000000000000000000000000000000000bb".to_owned()));
    assert!(!hashes.contains(&"gone0000000000000000000000000000000000aa".to_owned()));
}

#[tokio::test]
async fn a_bot_torrent_that_left_the_feed_unfinished_is_removed_once_it_has_finished() {
    let h = Harness::new().await;
    channel_a(&h).await;
    let hash = "slow0000000000000000000000000000000000aa";
    h.tr.preload(FakeTorrent::new(hash, "Stalled.mkv").bot().unfinished());
    let worker = h.worker();

    let report = run(&worker).await;
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert!(h.tr.calls_of("torrent-remove").is_empty());

    h.tr.finish(hash);
    h.advance(300_000);
    let report = run(&worker).await;
    let removed: Vec<_> = report.removed.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(removed, ["Stalled.mkv"]);
}

#[tokio::test]
async fn nothing_is_removed_when_no_feed_could_be_read() {
    let h = Harness::new().await;
    h.tr.preload(FakeTorrent::new("gone0000000000000000000000000000000000aa", "Old Show").bot());

    // No channels at all.
    let report = run(&h.worker()).await;
    assert_eq!(report.channels, 0);
    assert!(h.tr.calls_of("torrent-remove").is_empty());

    // Channels, but the feed is down.
    channel_a(&h).await;
    h.feeds.set_status("feed-a", 503);
    let report = run(&h.worker()).await;
    assert_eq!((report.channels_read, report.channels_failed), (0, 1));
    assert!(h.tr.calls_of("torrent-remove").is_empty());
    assert!(
        h.history_items().await.is_empty(),
        "an unread feed records nothing"
    );
}

// --- a failing item task ---------------------------------------------------------------

const STALE_HASH: &str = "gone0000000000000000000000000000000000aa";

#[tokio::test]
async fn a_panic_after_torrent_add_does_not_remove_the_new_torrent_or_any_other() {
    let h = Harness::new().await;
    channel_a(&h).await;
    h.tr.preload(FakeTorrent::new(STALE_HASH, "Old Show").bot());
    // Each item task panics right after its successful add is recorded,
    // before the renaming.
    let panicking = trss_collect::cycle::fault::panic_after_record(&h.tr.url());
    let worker = h.worker();

    let report = run(&worker).await;

    assert_eq!(report.job_panics, 3);
    // The torrents Transmission accepted are recorded as received and stay.
    assert_eq!(h.tr.torrents().len(), 4);
    assert_eq!(
        h.history_items()
            .await
            .iter()
            .filter(|i| i.result == HistoryResult::Received)
            .count(),
        3
    );
    // Nothing is removed in a cycle whose item tasks failed, not even the old one.
    assert!(report.removed.is_empty());
    assert!(h.tr.calls_of("torrent-remove").is_empty());

    // A healthy cycle carries on: the torrents are renamed and kept, the
    // departed one goes.
    drop(panicking);
    h.advance(300_000);
    let report = run(&worker).await;
    assert_eq!(report.job_panics, 0);
    assert_eq!(
        report
            .removed
            .iter()
            .map(|t| t.hash.as_str())
            .collect::<Vec<_>>(),
        [STALE_HASH]
    );
    let mut names: Vec<String> = h.tr.torrents().into_iter().map(|t| t.name).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "Sayonara Lara S01E03.mkv",
            "Slime S04E38.mkv",
            "Sono Bisque Doll S02E01.mkv",
        ]
    );
}

#[tokio::test]
async fn item_tasks_stop_when_the_cycle_is_dropped() {
    let h = Harness::new().await;
    channel_a(&h).await;
    let worker = h.worker();
    let add_gate = h.tr.hold("torrent-add");

    let running = {
        let worker = worker.clone();
        tokio::spawn(async move { worker.tick(&CancellationToken::new()).await })
    };
    add_gate.wait_arrived().await;

    // What a panic of the cycle does to its future: it is dropped mid-way.
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());

    // The item tasks went with it: they do not go on to record what
    // Transmission answers after the worker gave up its lock.
    add_gate.release_all();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let items = h.history_items().await;
    assert_eq!(items.len(), 4, "only the items that needed no Transmission");
    assert!(items.iter().all(|i| i.torrent_hash.is_none()));
}

// --- cleanup of departed torrents and unread channels -----------------------------------

/// A feed of the given items, each a release name and the digit its torrent
/// hash is made of.
fn feed_of(items: &[(&str, char)]) -> String {
    let items: String = items
        .iter()
        .map(|(name, digit)| {
            format!(
                "<item><title>{name}.mkv</title>\
                 <link>magnet:?xt=urn:btih:{}&amp;dn={name}.mkv</link>\
                 <guid>{name}-{digit}</guid></item>",
                digit.to_string().repeat(40)
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
        <link>http://x/</link><description>d</description>{items}</channel></rss>"#
    )
}

fn hash_of(digit: char) -> String {
    digit.to_string().repeat(40)
}

fn torrent_hashes(h: &Harness) -> Vec<String> {
    let mut hashes: Vec<String> = h.tr.torrents().into_iter().map(|t| t.hash).collect();
    hashes.sort();
    hashes
}

/// Channel X (`feed-x`: two shows) and channel Y (`feed-y`: two shows), one
/// cycle run so that Transmission holds all four and history knows where each
/// came from.
async fn two_channels_with_their_torrents(h: &Harness) -> Worker {
    h.feeds.set_xml(
        "feed-x",
        &feed_of(&[("[G] Show X1 - 01", '1'), ("[G] Show X2 - 01", '2')]),
    );
    h.feeds.set_xml(
        "feed-y",
        &feed_of(&[("[G] Show Y1 - 01", '3'), ("[G] Show Y2 - 01", '4')]),
    );
    h.add_channel(
        "feed-x",
        "/media/x",
        &[],
        vec![rule("Show X", "X/Season 01")],
    )
    .await;
    h.add_channel(
        "feed-y",
        "/media/y",
        &[],
        vec![rule("Show Y", "Y/Season 01")],
    )
    .await;
    let worker = h.worker();
    let report = run(&worker).await;
    assert_eq!((report.channels_read, report.added), (2, 4));
    assert_eq!(
        torrent_hashes(h),
        [hash_of('1'), hash_of('2'), hash_of('3'), hash_of('4')]
    );
    worker
}

#[tokio::test]
async fn torrents_of_a_channel_whose_feed_failed_stay_while_the_stale_ones_of_a_read_channel_go() {
    let h = Harness::new().await;
    let worker = two_channels_with_their_torrents(&h).await;
    h.tr.clear_calls();

    // X's feed is down. Y is read, and its second show has left the feed.
    h.feeds.set_status("feed-x", 503);
    h.feeds
        .set_xml("feed-y", &feed_of(&[("[G] Show Y1 - 01", '3')]));
    h.advance(300_000);
    let report = run(&worker).await;

    assert_eq!((report.channels_read, report.channels_failed), (1, 1));
    // X's torrents stay whatever its feed would say; Y's departed one goes.
    assert_eq!(
        report
            .removed
            .iter()
            .map(|t| t.hash.as_str())
            .collect::<Vec<_>>(),
        [hash_of('4')]
    );
    assert_eq!(
        torrent_hashes(&h),
        [hash_of('1'), hash_of('2'), hash_of('3')]
    );

    // Once X's feed is back and no longer lists its second show, that one goes too.
    h.feeds
        .set_xml("feed-x", &feed_of(&[("[G] Show X1 - 01", '1')]));
    h.advance(300_000);
    let report = run(&worker).await;
    assert_eq!(report.channels_failed, 0);
    assert_eq!(torrent_hashes(&h), [hash_of('1'), hash_of('3')]);
}

// --- rules ----------------------------------------------------------------------------

#[tokio::test]
async fn edits_made_during_a_cycle_apply_from_the_next_cycle() {
    let h = Harness::new().await;
    let channel = channel_a(&h).await;
    let worker = h.worker();

    // The first feed request is held; the rule is edited while the cycle waits.
    let gate = h.feeds.hold("feed-a");
    let running = {
        let worker = worker.clone();
        tokio::spawn(async move { worker.tick(&CancellationToken::new()).await })
    };
    gate.wait_arrived().await;

    let rule0 = &channel.rules[0];
    h.channels
        .update_rule(
            &rule0.id,
            rule0.version,
            &channel.channel.id,
            RuleInput {
                directory: "anime/Sayonara Lara/Season 09".into(),
                ..rule0.to_input()
            },
        )
        .await
        .unwrap();
    gate.release_all();

    let TickOutcome::Ran(report) = running.await.unwrap().unwrap() else {
        panic!("cycle did not run");
    };
    assert_eq!(report.added, 3);
    assert!(
        add_dirs(&h).contains(&"/media/anime/Sayonara Lara/Season 01".to_owned()),
        "this cycle ends with the settings it started with: {:?}",
        add_dirs(&h)
    );
    assert!(!add_dirs(&h).iter().any(|d| d.contains("Season 09")));

    h.tr.clear_calls();
    h.advance(300_000);
    run(&worker).await;
    assert!(
        add_dirs(&h).contains(&"/media/anime/Sayonara Lara/Season 09".to_owned()),
        "the next cycle uses the edited rule: {:?}",
        add_dirs(&h)
    );
}

// --- Transmission trouble ----------------------------------------------------------------

#[tokio::test]
async fn a_stopped_transmission_records_add_failed_and_the_worker_carries_on() {
    let mut h = Harness::new().await;
    channel_a(&h).await;
    let worker = h.worker();
    h.tr.stop().await;

    let report = run(&worker).await;
    assert_eq!(report.add_failed, 3);
    assert_eq!(report.added, 0);
    assert_eq!(
        h.history_items().await.len(),
        7,
        "everything is still recorded"
    );

    // Why each failed and under which rule is the collect crate's
    // (`cycle/tests.rs`); here every failure is on record with no torrent.
    let failed = h
        .history
        .list(HistoryQuery {
            result: Some(HistoryResult::AddFailed),
            limit: MAX_PAGE_SIZE,
            ..Default::default()
        })
        .await
        .unwrap()
        .items;
    assert_eq!(failed.len(), 3);
    assert!(failed.iter().all(|i| i.torrent_hash.is_none()));

    // The next cycle, with Transmission back, adds them.
    h.tr.restart().await;
    h.advance(300_000);
    let report = run(&worker).await;
    assert_eq!(report.added, 3);
    assert_eq!(h.tr.torrents().len(), 3);

    let recovered = h.item("Sayonara Lara - 03 (1080p)").await;
    assert_eq!(recovered.result, HistoryResult::Received);
    assert_eq!(recovered.first_seen_at, 1_000_000);

    // Transmission failing later does not turn a received item back into a failure.
    h.tr.stop().await;
    h.advance(300_000);
    let report = run(&worker).await;
    assert_eq!(report.add_failed, 3);
    assert_eq!(
        h.item("Sayonara Lara - 03 (1080p)").await.result,
        HistoryResult::Received
    );
}

// --- Transmission not answering ----------------------------------------------------------

#[tokio::test]
async fn a_hung_torrent_add_times_out_and_the_lock_is_released() {
    let h = Harness::new().await;
    channel_a(&h).await;
    let timeout = std::time::Duration::from_millis(300);
    let worker = h.worker().with_transmission_timeout(timeout);
    // Transmission takes the connection and never answers `torrent-add`.
    let _never_released = h.tr.hold("torrent-add");

    let started = std::time::Instant::now();
    let report = tokio::time::timeout(std::time::Duration::from_secs(20), run(&worker))
        .await
        .expect("the cycle ended although Transmission never answered");
    assert!(started.elapsed() < std::time::Duration::from_secs(10));

    assert_eq!(report.add_failed, 3);
    let failed = h
        .history
        .list(HistoryQuery {
            result: Some(HistoryResult::AddFailed),
            limit: MAX_PAGE_SIZE,
            ..Default::default()
        })
        .await
        .unwrap()
        .items;
    assert_eq!(failed.len(), 3);
    assert!(failed
        .iter()
        .all(|i| i.reason.as_deref().is_some_and(|r| !r.contains(SECRET))));

    // The lock was let go: the next tick is not refused as busy.
    h.advance(300_000);
    assert!(matches!(
        worker.tick(&CancellationToken::new()).await.unwrap(),
        TickOutcome::Ran(_)
    ));
}

#[tokio::test]
async fn an_add_that_times_out_after_transmission_took_it_is_not_removed() {
    let h = Harness::new().await;
    channel_a(&h).await;
    // A bot torrent that has left the feed: the plain rule would remove it.
    h.tr.preload(FakeTorrent::new("stale0000", "Gone - 01.mkv").bot());
    let worker = h
        .worker()
        .with_transmission_timeout(std::time::Duration::from_millis(300));
    // Transmission adds the torrents but answers only after the client gave up.
    let _late = h.tr.hold_answer("torrent-add");

    let report = tokio::time::timeout(std::time::Duration::from_secs(20), run(&worker))
        .await
        .expect("the cycle ended although Transmission never answered");

    assert_eq!(report.add_failed, 3);
    assert_eq!(report.adds_unconfirmed, 3);
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert!(h.tr.calls_of("torrent-remove").is_empty());
    // What Transmission took is still there, and so is the older torrent.
    assert_eq!(h.tr.torrents().len(), 4);
}

#[tokio::test]
async fn a_refused_add_still_lets_departed_torrents_go() {
    let h = Harness::new().await;
    channel_a(&h).await;
    h.tr.preload(FakeTorrent::new("stale0000", "Gone - 01.mkv").bot());
    h.tr.reject_adds(Some("invalid or corrupt torrent file"));

    let report = run(&h.worker()).await;

    assert_eq!(report.add_failed, 3);
    assert_eq!(report.adds_unconfirmed, 0);
    assert_eq!(report.removed.len(), 1);
}

/// Unlike `다시 받기`, which fails an add that could not connect at once, the
/// cycle counts it as unconfirmed: it has nowhere to remember an earlier add
/// of the item that got no answer, which only this add's `duplicate` answer
/// would have accounted for.
#[tokio::test]
async fn an_add_that_cannot_connect_is_unconfirmed_and_removes_nothing() {
    let mut h = Harness::new().await;
    channel_a(&h).await;
    h.tr.preload(FakeTorrent::new("stale0000", "Gone - 01.mkv").bot());
    let worker = h.worker();
    h.tr.stop().await;

    let report = run(&worker).await;

    assert_eq!(report.add_failed, 3);
    assert_eq!(report.adds_unconfirmed, 3);
    assert!(report.removed.is_empty(), "{:?}", report.removed);
}

#[tokio::test]
async fn a_hung_session_set_delays_the_cycle_by_the_timeout_only() {
    let h = Harness::new().await;
    channel_a(&h).await;
    let worker = h
        .worker()
        .with_transmission_timeout(std::time::Duration::from_millis(300));
    let _never_released = h.tr.hold("session-set");

    let report = tokio::time::timeout(std::time::Duration::from_secs(20), run(&worker))
        .await
        .expect("the cycle ended although session-set never answered");
    assert_eq!(report.added, 3);
}

// --- secrets -------------------------------------------------------------------------------

/// Every text value in the history tables, as one string.
async fn history_dump(h: &Harness) -> String {
    use trss_core::DbError;
    h.db.run::<_, DbError, _>(|conn| {
        let mut out = String::new();
        for table in ["history_items", "history_changes", "collection_cycle"] {
            let mut stmt = conn.prepare(&format!("SELECT * FROM {table}"))?;
            let cols = stmt.column_count();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                for i in 0..cols {
                    let value: rusqlite::types::Value = row.get(i)?;
                    out.push_str(&format!("{value:?}|"));
                }
                out.push('\n');
            }
        }
        Ok(out)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn secret_query_values_never_reach_history() {
    let mut h = Harness::new().await;
    channel_a(&h).await;
    // A second channel whose feed fails, and a third whose server is gone: the
    // HTTP errors of both quote the request URL.
    h.add_channel("broken", "/media/x", &[], vec![rule("x", "x")])
        .await;
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let dead_url = format!("http://{dead}/feed?token={SECRET}");
    h.channels
        .create_channel(trss_collect::store::channels::ChannelInput::new(dead_url))
        .await
        .unwrap();
    h.feeds.set_status("broken", 500);
    let worker = h.worker();

    run(&worker).await; // adds succeed
    h.tr.stop().await; // adds fail, with connection errors
    h.advance(300_000);
    run(&worker).await;

    let dump = history_dump(&h).await;
    assert!(!dump.contains(SECRET), "secret in history:\n{dump}");
    assert!(
        dump.contains("token=***"),
        "channel labels are masked:\n{dump}"
    );
}

// --- exclusivity -----------------------------------------------------------------------------

#[tokio::test]
async fn a_second_worker_skips_while_the_first_is_running() {
    let h = Harness::new().await;
    channel_a(&h).await;
    let first = h.worker();
    let second = h.worker();

    let gate = h.feeds.hold("feed-a");
    let running = {
        let first = first.clone();
        tokio::spawn(async move { first.tick(&CancellationToken::new()).await })
    };
    gate.wait_arrived().await;

    // The clock moves far past any plausible lease while the first still runs.
    h.advance(3_600_000);
    assert_eq!(
        second.tick(&CancellationToken::new()).await.unwrap(),
        TickOutcome::Busy
    );

    gate.release_all();
    assert!(matches!(
        running.await.unwrap().unwrap(),
        TickOutcome::Ran(_)
    ));
    assert_eq!(h.feeds.hits("feed-a"), 1, "the feed was read once");
    assert_eq!(h.tr.torrents().len(), 3);
}

#[tokio::test]
async fn workers_started_together_run_each_period_once() {
    let h = Harness::new().await;
    channel_a(&h).await;
    let gap = std::time::Duration::from_secs(150);
    let a = h.worker().with_min_gap(gap);
    let b = h.worker().with_min_gap(gap);
    let token = CancellationToken::new();

    let ran = |r: &Result<TickOutcome, _>| matches!(r, Ok(TickOutcome::Ran(_)));
    let (ra, rb) = tokio::join!(a.tick(&token), b.tick(&token));
    assert_eq!(
        [ran(&ra), ran(&rb)].iter().filter(|r| **r).count(),
        1,
        "{ra:?} {rb:?}"
    );

    // Whichever of them ticks again shortly after is refused, not just the loser.
    h.advance(10_000);
    let (ra, rb) = tokio::join!(a.tick(&token), b.tick(&token));
    for r in [&ra, &rb] {
        assert!(
            matches!(r, Ok(TickOutcome::TooSoon) | Ok(TickOutcome::Busy)),
            "{r:?}"
        );
    }
    assert_eq!(h.feeds.hits("feed-a"), 1);
    assert_eq!(h.tr.calls_of("torrent-add").len(), 3);

    // A full period later exactly one runs again.
    h.advance(300_000);
    let (ra, rb) = tokio::join!(a.tick(&token), b.tick(&token));
    assert_eq!([ran(&ra), ran(&rb)].iter().filter(|r| **r).count(), 1);
    assert_eq!(h.feeds.hits("feed-a"), 2);
}

// --- shutdown --------------------------------------------------------------------------------

#[tokio::test]
async fn shutdown_during_adding_records_what_was_handed_to_transmission() {
    let h = Harness::new().await;
    channel_a(&h).await;
    let worker = h.worker();
    let cancel = CancellationToken::new();
    let add_gate = h.tr.hold("torrent-add");

    let running = {
        let (worker, cancel) = (worker.clone(), cancel.clone());
        tokio::spawn(async move { worker.tick(&cancel).await })
    };
    add_gate.wait_arrived().await;
    cancel.cancel();
    add_gate.release_all();

    let TickOutcome::Ran(report) = running.await.unwrap().unwrap() else {
        panic!("cycle did not run");
    };
    assert!(report.interrupted);

    // The three items Transmission took are recorded as received, not left half-done.
    let received: Vec<_> = h
        .history_items()
        .await
        .into_iter()
        .filter(|i| i.result == HistoryResult::Received)
        .collect();
    assert_eq!(received.len(), 3);
    assert!(received.iter().all(|i| i.torrent_hash.is_some()));
    assert_eq!(h.tr.torrents().len(), 3);
    // Renaming was left for the next cycle, and the removal of departed
    // torrents (which needs the complete picture) was skipped.
    assert!(h.tr.calls_of("torrent-rename-path").is_empty());
    assert!(h.tr.calls_of("torrent-remove").is_empty());
    // The cycle did not claim to have finished.
    assert_eq!(
        h.history.last_cycle().await.unwrap().unwrap().finished_at,
        None
    );

    // The next cycle completes the work: renames the torrents.
    h.advance(300_000);
    let report = run(&worker).await;
    assert!(!report.interrupted);
    assert_eq!(h.tr.calls_of("torrent-rename-path").len(), 3);
    assert_eq!(
        h.item("Sayonara Lara - 03 (1080p)").await.result,
        HistoryResult::Received
    );
}

#[tokio::test]
async fn shutdown_while_reading_feeds_stops_without_waiting_or_recording() {
    let h = Harness::new().await;
    channel_a(&h).await;
    let worker = h.worker();
    let cancel = CancellationToken::new();
    let gate = h.feeds.hold("feed-a");

    let running = {
        let (worker, cancel) = (worker.clone(), cancel.clone());
        tokio::spawn(async move { worker.tick(&cancel).await })
    };
    gate.wait_arrived().await;
    cancel.cancel();

    // The held feed request is never released.
    let TickOutcome::Ran(report) = tokio::time::timeout(std::time::Duration::from_secs(5), running)
        .await
        .expect("the cycle stopped")
        .unwrap()
        .unwrap()
    else {
        panic!("cycle did not run");
    };
    assert!(report.interrupted);
    assert!(h.history_items().await.is_empty());
    assert!(h.tr.calls_of("torrent-add").is_empty());
    gate.release_all();
}
