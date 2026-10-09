//! The app-wide collect folder in the worker's cycle (ticket 0010).
//!
//! While no collect folder is set the cycle adds nothing, records no failure,
//! tallies what waits in its report, and removes no torrent from Transmission;
//! once the folder is set the next cycle receives what the rules picked. What
//! the cycle judges and records meanwhile, and where the items go afterwards,
//! are `trss-collect`'s tests (`cycle/tests.rs`); the board's
//! `collect_folder_set` is `trss-web`'s.
//!
//! The channel URL's token is made up.

use crate::common;

use common::*;
use tokio_util::sync::CancellationToken;
use trss_core::settings::SettingsStore;
use trss_worker::{CycleReport, TickOutcome, Worker};

async fn run(worker: &Worker) -> CycleReport {
    match worker.tick(&CancellationToken::new()).await.unwrap() {
        TickOutcome::Ran(report) => report,
        other => panic!("expected a cycle, got {other:?}"),
    }
}

#[tokio::test]
async fn with_no_collect_folder_a_cycle_tallies_what_waits_adds_and_removes_nothing_and_the_folder_is_read_again(
) {
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

    let report = run(&h.worker()).await;

    // The four items no rule picks are judged as usual; the three a rule picks
    // wait, so nothing piles up as `add_failed`.
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

    // The worker reads the folder at every cycle: choosing it is all it takes.
    SettingsStore::new(h.db.clone())
        .put_collection(0, "/media".to_owned(), None)
        .await
        .unwrap();
    h.advance(300_000);
    let report = run(&h.worker()).await;
    assert_eq!((report.added, report.add_failed), (3, 0), "{report:?}");
    assert_eq!(report.items_new, 3, "{report:?}");
    assert_eq!(report.waiting_for_collect_folder, 0);
}
