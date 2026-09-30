//! The snapshots a worker cycle leaves for the status board (ticket 0006):
//! whether each channel's feed could be read and Transmission's torrent counts.
//! The web reads these instead of the feed or Transmission.

mod common;

use common::*;
use tokio_util::sync::CancellationToken;
use transmission_rss::{
    store::status::StatusStore,
    worker::{CycleReport, TickOutcome, Worker},
};

async fn run(worker: &Worker) -> CycleReport {
    match worker.tick(&CancellationToken::new()).await.unwrap() {
        TickOutcome::Ran(report) => report,
        other => panic!("expected a cycle, got {other:?}"),
    }
}

#[tokio::test]
async fn a_cycle_records_that_the_feed_was_read_and_how_many_torrents_transmission_has() {
    let h = Harness::new().await;
    let channel = h
        .add_channel("feed-a", "/media/anime", &[], feed_a_rules())
        .await;
    h.tr.preload(FakeTorrent::new(&"b".repeat(40), "Seeding one").status(6));
    h.tr.preload(FakeTorrent::new(&"c".repeat(40), "Seeding two").status(6));
    h.tr.preload(FakeTorrent::new(&"d".repeat(40), "Stopped one").status(0));
    let status = StatusStore::new(h.db.clone());
    assert!(status.channel_reads().await.unwrap().is_empty());
    assert!(status.transmission().await.unwrap().is_none());

    run(&h.worker()).await;

    let reads = status.channel_reads().await.unwrap();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].channel_id, channel.channel.id);
    assert!(reads[0].ok);
    assert_eq!(reads[0].read_at, h.now());
    assert_eq!(reads[0].ok_at, Some(h.now()));
    let counts = status.transmission().await.unwrap().unwrap();
    // Two preloaded seeding torrents, the cycle's own adds are downloading.
    assert_eq!(counts.seeding, 2);
    assert!(counts.downloading >= 1, "{counts:?}");
    assert_eq!(counts.taken_at, h.now());
}

#[tokio::test]
async fn a_failed_read_keeps_when_the_last_good_one_was() {
    let h = Harness::new().await;
    h.add_channel("feed-a", "/media/anime", &[], feed_a_rules())
        .await;
    let status = StatusStore::new(h.db.clone());
    run(&h.worker()).await;
    let first_at = h.now();

    h.advance(600_000);
    h.feeds.set_status("feed-a", 500);
    run(&h.worker()).await;

    let reads = status.channel_reads().await.unwrap();
    assert_eq!(reads.len(), 1);
    assert!(!reads[0].ok);
    assert_eq!(reads[0].read_at, h.now());
    assert_eq!(reads[0].ok_at, Some(first_at));
}

#[tokio::test]
async fn when_transmission_cannot_be_asked_the_old_counts_stay_and_the_cycle_still_finishes() {
    let mut h = Harness::new().await;
    h.add_channel("feed-a", "/media/anime", &[], feed_a_rules())
        .await;
    let status = StatusStore::new(h.db.clone());
    run(&h.worker()).await;
    let before = status.transmission().await.unwrap().unwrap();

    h.tr.stop().await;
    h.advance(600_000);
    let worker = h.worker();
    let outcome = worker.tick(&CancellationToken::new()).await;
    assert!(
        matches!(outcome, Ok(TickOutcome::Ran(_))),
        "the cycle must finish: {outcome:?}"
    );

    let after = status.transmission().await.unwrap().unwrap();
    assert_eq!(
        after, before,
        "no zeros were written for an unreachable Transmission"
    );
    // The feed was still read and recorded.
    let reads = status.channel_reads().await.unwrap();
    assert!(reads[0].ok);
    assert_eq!(reads[0].read_at, h.now());
}
