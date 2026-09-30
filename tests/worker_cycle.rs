//! The worker's collection cycle against a fake Transmission and fake RSS
//! feeds, with the real Transmission client and a real (temporary) app database.

mod common;

use common::*;
use tokio_util::sync::CancellationToken;
use transmission_rss::{
    store::{
        channels::{RuleInput, RuleState},
        history::{HistoryQuery, HistoryResult, MAX_PAGE_SIZE},
    },
    worker::{CycleReport, TickOutcome, Worker},
};

async fn run(worker: &Worker) -> CycleReport {
    match worker.tick(&CancellationToken::new()).await.unwrap() {
        TickOutcome::Ran(report) => report,
        other => panic!("expected a cycle, got {other:?}"),
    }
}

fn hash_a(n: u32) -> String {
    format!("aaaa{n:036}")
}

async fn channel_a(h: &Harness) -> transmission_rss::store::channels::ChannelWithRules {
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
    let rules = &channel.rules;

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

    // The selected items went to Transmission with their rule's save path, the
    // bot label and the label of their item.
    assert_eq!(
        add_dirs(&h),
        [
            "/media/anime/Sayonara Lara/Season 01",
            "/media/anime/Slime/Season 04",
            "/media/anime/Sono Bisque Doll/Season 02",
        ]
    );
    let items = h.history_items().await;
    for call in h.tr.calls_of("torrent-add") {
        let hash = call.args["filename"]
            .as_str()
            .unwrap()
            .split("btih:")
            .nth(1)
            .unwrap()[..40]
            .to_lowercase();
        let item = items
            .iter()
            .find(|i| i.torrent_hash.as_deref() == Some(hash.as_str()))
            .unwrap();
        assert_eq!(
            call.args["labels"],
            serde_json::json!([
                BOT_LABEL,
                format!("trss-item:{}:{}", item.channel_id, item.identity_key)
            ])
        );
    }

    // Each was renamed by trname (episode offsets applied).
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

    // History has all seven, with results, applied rules and hashes.
    let items = h.history_items().await;
    assert_eq!(items.len(), 7);

    let sayonara = h.item("Sayonara Lara - 03 (1080p)").await;
    assert_eq!(sayonara.result, HistoryResult::Received);
    assert_eq!(sayonara.rule_id.as_deref(), Some(rules[0].id.as_str()));
    assert_eq!(sayonara.torrent_hash.as_deref(), Some(hash_a(1).as_str()));

    // Both "sono bisque doll" rules match; the first one wins.
    let sono = h.item("Sono Bisque Doll - 13").await;
    assert_eq!(sono.result, HistoryResult::Received);
    assert_eq!(sono.rule_id.as_deref(), Some(rules[1].id.as_str()));

    let slime = h.item("Slime Datta Ken - 62").await;
    assert_eq!(slime.rule_id.as_deref(), Some(rules[3].id.as_str()));

    for (part, result) in [
        ("(720p)", HistoryResult::Excluded),
        ("[Batch]", HistoryResult::Excluded),
        ("Unrelated Show", HistoryResult::NoMatch),
    ] {
        let item = h.item(part).await;
        assert_eq!(item.result, result, "{part}");
        assert_eq!(item.rule_id, None);
        assert_eq!(item.torrent_hash, None);
    }

    // The untitled item is judged like any other and recorded.
    let untitled = items.iter().find(|i| i.title.is_empty()).unwrap();
    assert_eq!(untitled.result, HistoryResult::NoMatch);

    for item in &items {
        assert_eq!(item.channel_id, channel.channel.id);
        assert_eq!(item.first_seen_at, 1_000_000);
    }
    // The identity key is a hash of the GUID, not the GUID.
    assert!(items[0].identity_key.starts_with("guid:"));
    assert!(!items[0].identity_key.contains("guid-aaaa"));
}

fn magnet(hash_digit: char, name: &str) -> String {
    format!(
        "magnet:?xt=urn:btih:{}&amp;dn={name}.mkv",
        hash_digit.to_string().repeat(40)
    )
}

#[tokio::test]
async fn items_differing_only_in_a_secret_named_query_value_are_all_added_and_recorded() {
    let h = Harness::new().await;
    // Every query name of the channel URL is secret, `id` included, and the
    // items' GUIDs differ only in their `id` value.
    let xml = format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
        <link>http://x/</link><description>d</description>
        <item><title>Show - 01</title><link>{}</link>
          <guid>https://t.test/details.php?id=101</guid></item>
        <item><title>Show - 02</title><link>{}</link>
          <guid>https://t.test/details.php?id=102</guid></item>
        <item><title>Show - 03</title><link>{}</link>
          <guid>https://t.test/details.php?id=103</guid></item>
        </channel></rss>"#,
        magnet('1', "Show%20-%2001"),
        magnet('2', "Show%20-%2002"),
        magnet('3', "Show%20-%2003"),
    );
    h.feeds.set_xml("ids", &xml);
    let input = transmission_rss::store::channels::ChannelInput::new(
        format!("{}?id=0&token={SECRET}", h.feeds.url("ids")),
        "/media/p",
    );
    assert!(input.secret_query.contains(&"id".to_owned()));
    h.channels
        .create_channel_with_rules(input, vec![rule("Show", "Show/Season 01")])
        .await
        .unwrap();
    let worker = h.worker();

    let report = run(&worker).await;
    assert_eq!(
        (report.items_seen, report.items_new, report.added),
        (3, 3, 3)
    );
    assert_eq!(h.tr.torrents().len(), 3);
    let items = h.history_items().await;
    assert_eq!(items.len(), 3);
    assert!(items.iter().all(|i| i.result == HistoryResult::Received));
    let keys: std::collections::HashSet<_> = items.iter().map(|i| &i.identity_key).collect();
    assert_eq!(keys.len(), 3);

    // The same sighting again is still three known items, not three new ones.
    h.advance(300_000);
    let report = run(&worker).await;
    assert_eq!((report.items_seen, report.items_new), (3, 0));
    assert_eq!(h.history_items().await.len(), 3);
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

#[tokio::test]
async fn a_torrent_transmission_already_had_is_never_removed_by_the_renaming() {
    let h = Harness::new().await;
    channel_a(&h).await;
    // Transmission holds the torrent of a selected item already, under a name
    // trname cannot derive a new one from. The legacy renaming removed such a
    // torrent together with its data; it was not added by this cycle.
    h.tr.preload(FakeTorrent::new(&hash_a(1), "Some Special Collection.mkv").bot());

    let report = run(&h.worker()).await;

    assert_eq!(report.duplicates, 1);
    assert!(h.tr.calls_of("torrent-remove").is_empty());
    assert!(h
        .tr
        .torrents()
        .iter()
        .any(|t| t.hash == hash_a(1) && t.name == "Some Special Collection.mkv"));
}

#[tokio::test]
async fn a_torrent_a_rule_received_and_named_is_not_renamed_again() {
    let h = Harness::new().await;
    channel_a(&h).await;
    let worker = h.worker();
    run(&worker).await;
    let slime = |h: &Harness| {
        h.tr.torrents()
            .into_iter()
            .find(|t| t.hash == hash_a(4))
            .unwrap()
            .name
    };
    assert_eq!(slime(&h), "Slime S04E38.mkv");
    h.tr.clear_calls();

    // Met again as a duplicate. Its rule's episode offset (-24) must not be
    // applied a second time to the name it already has.
    h.advance(300_000);
    run(&worker).await;

    assert_eq!(slime(&h), "Slime S04E38.mkv");
    assert!(h.tr.calls_of("torrent-rename-path").is_empty());
}

/// A torrent Transmission already holds for an item of `channel_a`, saved in
/// `dir` under `name`.
fn held(n: u32, name: &str, dir: &str) -> FakeTorrent {
    FakeTorrent {
        download_dir: dir.to_owned(),
        ..FakeTorrent::new(&hash_a(n), name).bot()
    }
}

fn renames_of(h: &Harness, n: u32) -> usize {
    h.tr.calls_of("torrent-rename-path")
        .iter()
        .filter(|c| c.args["ids"] == serde_json::json!([hash_a(n)]))
        .count()
}

fn name_of(h: &Harness, n: u32) -> String {
    h.tr.torrents()
        .into_iter()
        .find(|t| t.hash == hash_a(n))
        .unwrap()
        .name
}

#[tokio::test]
async fn a_named_torrent_under_a_folder_whose_case_changed_is_not_renamed() {
    let h = Harness::new().await;
    channel_a(&h).await;
    // Named under the rule's folder before its title was written in another
    // case. trname compares the title case-sensitively, so it would read the
    // name as a release and apply the rule's offset (-12) again.
    h.tr.preload(held(
        3,
        "SONO BISQUE DOLL S02E01.mkv",
        "/media/anime/SONO BISQUE DOLL/Season 02",
    ));

    run(&h.worker()).await;

    assert_eq!(name_of(&h, 3), "SONO BISQUE DOLL S02E01.mkv");
    assert_eq!(renames_of(&h, 3), 0);
}

#[tokio::test]
async fn a_torrent_in_another_rules_folder_is_not_renamed_after_this_rule() {
    let h = Harness::new().await;
    channel_a(&h).await;
    // Another rule or channel received the same torrent into its own folder,
    // with a name that rule's title gives. This rule's title does not apply.
    h.tr.preload(held(
        1,
        "[SubsPlease] Sayonara Lara - 03 (1080p) [AAAA0001].mkv",
        "/media/other/Lara/Season 01",
    ));

    run(&h.worker()).await;

    assert_eq!(
        name_of(&h, 1),
        "[SubsPlease] Sayonara Lara - 03 (1080p) [AAAA0001].mkv"
    );
    assert_eq!(renames_of(&h, 1), 0);
}

#[tokio::test]
async fn a_named_torrent_with_a_three_digit_episode_is_not_renamed() {
    let h = Harness::new().await;
    channel_a(&h).await;
    // trname only reads two-digit episodes as its own form, and would take
    // `05` out of `E105`.
    h.tr.preload(held(4, "Slime S04E105.mkv", "/media/anime/Slime/Season 04"));

    run(&h.worker()).await;

    assert_eq!(name_of(&h, 4), "Slime S04E105.mkv");
    assert_eq!(renames_of(&h, 4), 0);
}

#[tokio::test]
async fn a_release_named_like_sxxeyy_under_another_title_is_still_renamed_in_the_rules_folder() {
    let h = Harness::new().await;
    channel_a(&h).await;
    // A release whose own name ends in `SxxEyy`, left unrenamed by an earlier
    // run: it is not a name this rule gave, so its episode offset (-24) applies.
    h.tr.preload(held(
        4,
        "Tensura S04E62.mkv",
        "/media/anime/Slime/Season 04",
    ));

    run(&h.worker()).await;

    assert_eq!(name_of(&h, 4), "Slime S04E38.mkv");
}

#[tokio::test]
async fn a_rename_cut_short_in_the_rules_folder_is_finished_when_the_torrent_is_met_again() {
    let h = Harness::new().await;
    channel_a(&h).await;
    // An earlier run added the torrent into the rule's folder but did not get
    // to rename it.
    h.tr.preload(held(
        4,
        "[SubsPlease] Tensei Shitara Slime Datta Ken - 62 (1080p) [AAAA0006].mkv",
        "/media/anime/Slime/Season 04",
    ));

    let report = run(&h.worker()).await;

    assert_eq!(report.duplicates, 1);
    assert_eq!(name_of(&h, 4), "Slime S04E38.mkv");
}

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
async fn a_labelled_torrent_stays_while_its_item_is_in_a_feed_and_goes_after() {
    let h = Harness::new().await;
    let channel = channel_a(&h).await;
    run(&h.worker()).await;
    // A bot torrent history knows nothing of (its add was never answered,
    // say), labelled for an item of the feed, and one for an item not in it.
    let in_feed = h
        .history_items()
        .await
        .into_iter()
        .find(|i| i.torrent_hash.is_none())
        .unwrap();
    let label = |key: &str| format!("trss-item:{}:{key}", channel.channel.id);
    h.tr.preload(FakeTorrent {
        labels: vec![BOT_LABEL.to_owned(), label(&in_feed.identity_key)],
        ..FakeTorrent::new("feed0000000000000000000000000000000000aa", "Kept.mkv")
    });
    h.tr.preload(FakeTorrent {
        labels: vec![BOT_LABEL.to_owned(), label("guid:gone")],
        ..FakeTorrent::new("gone0000000000000000000000000000000000aa", "Gone.mkv")
    });
    h.advance(300_000);

    let report = run(&h.worker()).await;

    let removed: Vec<_> = report.removed.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(removed, ["Gone.mkv"]);
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
    // Every `torrent-get` leaves out `file-count`, so the renaming that follows
    // each successful add panics inside the item's task.
    h.tr.omit_file_count(true);
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
    h.tr.omit_file_count(false);
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

#[tokio::test]
async fn a_torrent_of_unknown_origin_goes_when_any_feed_was_read() {
    let h = Harness::new().await;
    let worker = two_channels_with_their_torrents(&h).await;
    // Added by something else that labels its torrents (the legacy cron before
    // the switch): history has no record of it.
    h.tr.preload(FakeTorrent::new(STALE_HASH, "Old Show").bot());
    h.tr.preload(FakeTorrent::new(
        "mine0000000000000000000000000000000000bb",
        "Manual",
    ));

    h.feeds.set_status("feed-x", 503);
    h.advance(300_000);
    let report = run(&worker).await;

    assert_eq!((report.channels_read, report.channels_failed), (1, 1));
    assert_eq!(
        report
            .removed
            .iter()
            .map(|t| t.hash.as_str())
            .collect::<Vec<_>>(),
        [STALE_HASH],
        "only the unlabelled-by-history one; X's and Y's torrents and the manual one stay"
    );
    assert_eq!(torrent_hashes(&h).len(), 5);
}

#[tokio::test]
async fn nothing_is_removed_when_every_feed_failed_even_for_unknown_origin() {
    let h = Harness::new().await;
    let worker = two_channels_with_their_torrents(&h).await;
    h.tr.preload(FakeTorrent::new(STALE_HASH, "Old Show").bot());

    h.feeds.set_status("feed-x", 503);
    h.feeds.set_status("feed-y", 503);
    h.advance(300_000);
    let report = run(&worker).await;

    assert_eq!(report.channels_read, 0);
    assert!(h.tr.calls_of("torrent-remove").is_empty());
}

// --- rules ----------------------------------------------------------------------------

#[tokio::test]
async fn archived_and_title_waiting_rules_receive_nothing_and_ids_map_back() {
    let h = Harness::new().await;
    let channel = h
        .add_channel(
            "feed-a",
            "/media/anime",
            &[],
            vec![
                // Matches item 1 but is archived.
                RuleInput {
                    state: RuleState::Archived,
                    ..rule("Sayonara Lara", "Archived")
                },
                // Waiting for its title: matches nothing, not even everything.
                RuleInput {
                    r#match: None,
                    directory: "Waiting".into(),
                    ..Default::default()
                },
                rule("Sono Bisque Doll - 13", "Sono Bisque Doll/Season 02"),
            ],
        )
        .await;

    let report = run(&h.worker()).await;

    assert_eq!(report.added, 1);
    assert_eq!(add_dirs(&h), ["/media/anime/Sono Bisque Doll/Season 02"]);
    let received = h.item("Sono Bisque Doll - 13").await;
    assert_eq!(received.result, HistoryResult::Received);
    // Third stored rule, second among the active ones: history holds the stored ID.
    assert_eq!(
        received.rule_id.as_deref(),
        Some(channel.rules[2].id.as_str())
    );
    assert_eq!(
        h.item("Sayonara Lara - 03 (1080p)").await.result,
        HistoryResult::NoMatch
    );
}

#[tokio::test]
async fn a_new_matching_rule_turns_no_match_into_received_and_keeps_the_first_seen_time() {
    let h = Harness::new().await;
    let channel = channel_a(&h).await;
    let worker = h.worker();

    run(&worker).await;
    let before = h.item("Unrelated Show").await;
    assert_eq!(before.result, HistoryResult::NoMatch);

    let new_rule = h
        .channels
        .create_rule(
            &channel.channel.id,
            rule("Unrelated Show", "Unrelated Show/Season 01"),
        )
        .await
        .unwrap();
    h.advance(300_000);
    run(&worker).await;

    let after = h.item("Unrelated Show").await;
    assert_eq!(after.id, before.id, "still one record");
    assert_eq!(after.result, HistoryResult::Received);
    assert_eq!(after.first_seen_at, 1_000_000);
    assert_eq!(after.result_at, 1_300_000);
    assert_eq!(after.rule_id.as_deref(), Some(new_rule.id.as_str()));

    let changes = h.history.changes(after.id).await.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        (changes[0].from, changes[0].to),
        (HistoryResult::NoMatch, HistoryResult::Received)
    );
    assert_eq!(h.history_items().await.len(), 7);
}

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
                directory: "Sayonara Lara/Season 09".into(),
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
    let channel = channel_a(&h).await;
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
    for item in &failed {
        assert!(
            item.reason.as_deref().is_some_and(|r| !r.is_empty()),
            "{item:?}"
        );
        assert!(item.rule_id.is_some());
        assert_eq!(item.torrent_hash, None);
    }
    assert!(failed
        .iter()
        .any(|i| i.rule_id.as_deref() == Some(channel.rules[0].id.as_str())));

    // The next cycle, with Transmission back, adds them.
    h.tr.restart().await;
    h.advance(300_000);
    let report = run(&worker).await;
    assert_eq!(report.added, 3);
    assert_eq!(h.tr.torrents().len(), 3);

    let recovered = h.item("Sayonara Lara - 03 (1080p)").await;
    assert_eq!(recovered.result, HistoryResult::Received);
    assert_eq!(recovered.reason, None);
    assert_eq!(recovered.first_seen_at, 1_000_000);
    let changes = h.history.changes(recovered.id).await.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].from, HistoryResult::AddFailed);

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

#[tokio::test]
async fn a_refused_torrent_is_add_failed_with_transmissions_reason() {
    let h = Harness::new().await;
    channel_a(&h).await;
    h.tr.reject_adds(Some("download directory path is not absolute"));

    let report = run(&h.worker()).await;
    assert_eq!(report.add_failed, 3);
    let item = h.item("Sayonara Lara - 03 (1080p)").await;
    assert_eq!(item.result, HistoryResult::AddFailed);
    assert!(item
        .reason
        .unwrap()
        .contains("download directory path is not absolute"));
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
    use transmission_rss::store::DbError;
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
        .create_channel(transmission_rss::store::channels::ChannelInput::new(
            dead_url, "/media/y",
        ))
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

#[tokio::test]
async fn secrets_in_item_links_and_guids_are_masked_in_history_but_used_for_adding() {
    let h = Harness::new().await;
    let xml = format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
        <link>http://x/</link><description>d</description>
        <item><title>Show - 01</title>
          <link>magnet:?xt=urn:btih:{}&amp;dn=Show%20-%2001.mkv&amp;passkey=PASSKEY-VALUE-123</link>
          <guid>https://tracker.test/t/1?passkey=PASSKEY-VALUE-123</guid></item>
        </channel></rss>"#,
        "cccc".repeat(10)
    );
    h.feeds.set_xml("private", &xml);
    let mut input = transmission_rss::store::channels::ChannelInput::new(
        format!("{}?passkey=PASSKEY-VALUE-123", h.feeds.url("private")),
        "/media/p",
    );
    input.secret_query = vec!["passkey".into()];
    h.channels
        .create_channel_with_rules(input, vec![rule("Show", "Show/Season 01")])
        .await
        .unwrap();

    run(&h.worker()).await;

    // Transmission got the real link.
    let add = &h.tr.calls_of("torrent-add")[0];
    assert!(add.args["filename"]
        .as_str()
        .unwrap()
        .contains("PASSKEY-VALUE-123"));
    // History did not.
    let dump = history_dump(&h).await;
    assert!(!dump.contains("PASSKEY-VALUE-123"), "{dump}");
    assert_eq!(h.history_items().await[0].result, HistoryResult::Received);
}

#[tokio::test]
async fn secret_values_are_masked_in_history_under_other_names_in_paths_and_encoded() {
    let h = Harness::new().await;
    // The channel URL spells the secret percent-encoded; feeds may quote it
    // decoded, in a path, or encoded again inside another URL.
    const IN_URL: &str = "Tk%2Fen%2BSECRETVALUE99";
    const DECODED: &str = "Tk/en+SECRETVALUE99";
    const ENCODED_TWICE: &str = "Tk%252Fen%252BSECRETVALUE99";
    let xml = format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
        <link>http://x/</link><description>d</description>
        <item><title>Show - 01 [{DECODED}]</title>
          <link>magnet:?xt=urn:btih:{}&amp;dn=Show%20-%2001&amp;tr=https%3A%2F%2Ftr.test%2Fa%3Ftorrent_pass%3D{ENCODED_TWICE}</link>
          <guid>https://t.test/details/{IN_URL}/1</guid></item>
        <item><title>Show - 02</title>
          <link>https://t.test/{IN_URL}/dl/2.torrent?torrent_pass={IN_URL}&amp;id=2</link>
          <guid>https://t.test/details/{DECODED}/2</guid></item>
        </channel></rss>"#,
        "dddd".repeat(10)
    );
    h.feeds.set_xml("other-names", &xml);
    let input = transmission_rss::store::channels::ChannelInput::new(
        format!("{}?passkey={IN_URL}&r=1080", h.feeds.url("other-names")),
        "/media/p",
    );
    h.channels
        .create_channel_with_rules(input, vec![rule("Show", "Show/Season 01")])
        .await
        .unwrap();
    // A refusal that quotes the secret ends up in `reason`.
    h.tr.reject_adds(Some(&format!("cannot use {DECODED} or {IN_URL}")));

    run(&h.worker()).await;

    // Transmission was asked with the real link.
    let asked = h.tr.calls_of("torrent-add");
    assert!(asked
        .iter()
        .any(|c| c.args["filename"].as_str().unwrap().contains(ENCODED_TWICE)));

    let items = h.history_items().await;
    assert_eq!(items.len(), 2);
    let dump = history_dump(&h).await;
    for form in ["SECRETVALUE99", IN_URL, DECODED, ENCODED_TWICE] {
        assert!(!dump.contains(form), "{form} in history:\n{dump}");
    }
    let first = h.item("Show - 01").await;
    assert_eq!(first.title, "Show - 01 [***]");
    assert!(first.link.contains("torrent_pass%3D***"), "{}", first.link);
    assert_eq!(first.result, HistoryResult::AddFailed);
    assert_eq!(
        first.reason.as_deref(),
        Some("Transmission refused the torrent: cannot use *** or ***")
    );
    let second = h.item("Show - 02").await;
    assert_eq!(
        second.link,
        "https://t.test/***/dl/2.torrent?torrent_pass=***&id=2"
    );
}

#[tokio::test]
async fn short_secret_looking_values_do_not_garble_stored_titles() {
    // `filter=1080p` is secret like every query name, but too short to be
    // replaced in text: the titles keep their "1080p".
    let h = Harness::new().await;
    channel_a(&h).await;
    run(&h.worker()).await;
    let sayonara = h.item("Sayonara Lara - 03 (1080p)").await;
    assert_eq!(
        sayonara.title,
        "[SubsPlease] Sayonara Lara - 03 (1080p) [AAAA0001].mkv"
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
