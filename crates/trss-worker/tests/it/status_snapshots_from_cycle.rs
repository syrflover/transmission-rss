//! The snapshots a worker cycle leaves for the status board (ticket 0006):
//! whether each channel's feed could be read and Transmission's torrent counts.
//! The web reads these instead of the feed or Transmission.

use crate::common;

use common::*;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use trss_collect::store::status::StatusStore;
use trss_core::heartbeat::HeartbeatStore;
use trss_worker::{CycleReport, TickOutcome, Worker};

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
    // How Transmission's statuses are told apart is `TransmissionLook`'s rule
    // (trss-collect); here the cycle writes what it made of the two preloaded
    // seeding torrents and its own adds.
    let counts = status.transmission().await.unwrap().unwrap();
    assert_eq!(counts.seeding, 2);
    assert!(counts.downloading >= 1, "{counts:?}");
    assert_eq!(counts.taken_at, h.now());
    let hashes = status.downloading_hashes().await.unwrap();
    assert_eq!(hashes.len() as u32, counts.downloading);
    let listing = status.torrent_listing().await.unwrap().unwrap();
    assert_eq!(listing.taken_at, h.now());
    assert_eq!(listing.hashes.len(), 3 + hashes.len());
}

#[tokio::test]
async fn a_feed_that_answers_500_is_recorded_as_a_failed_read() {
    let h = Harness::new().await;
    h.add_channel("feed-a", "/media/anime", &[], feed_a_rules())
        .await;
    let status = StatusStore::new(h.db.clone());
    h.feeds.set_status("feed-a", 500);
    run(&h.worker()).await;

    // That a failed read keeps the time of the last good one is the store's rule.
    let reads = status.channel_reads().await.unwrap();
    assert_eq!(reads.len(), 1);
    assert!(!reads[0].ok);
    assert_eq!(reads[0].read_at, h.now());
}

#[tokio::test]
async fn when_transmission_cannot_be_asked_the_old_counts_stay_and_the_cycle_still_finishes() {
    let mut h = Harness::new().await;
    h.add_channel("feed-a", "/media/anime", &[], feed_a_rules())
        .await;
    let status = StatusStore::new(h.db.clone());
    run(&h.worker()).await;
    let before = status.transmission().await.unwrap().unwrap();
    let listed = status.torrent_listing().await.unwrap().unwrap();

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
    // Nor an empty list, which would say every torrent was removed.
    assert_eq!(status.torrent_listing().await.unwrap().unwrap(), listed);
    // The feed was still read and recorded.
    let reads = status.channel_reads().await.unwrap();
    assert!(reads[0].ok);
    assert_eq!(reads[0].read_at, h.now());
}

// --- the worker's heartbeat (ticket 0021) ------------------------------------------------------

#[tokio::test]
async fn a_worker_beats_while_a_cycle_holds_the_lock_and_clears_the_hold_after_it() {
    let h = Harness::new().await;
    h.add_channel("feed-a", "/media/anime", &[], feed_a_rules())
        .await;
    let heartbeat = HeartbeatStore::new(h.db.clone());
    assert_eq!(heartbeat.read().await.unwrap(), None);
    let first = h.worker().with_heartbeat_every(Duration::from_millis(10));
    let second = h.worker();

    // The cycle waits for the feed, holding the lock.
    let gate = h.feeds.hold("feed-a");
    let started = h.now();
    let running = {
        let first = first.clone();
        tokio::spawn(async move { first.tick(&CancellationToken::new()).await })
    };
    gate.wait_arrived().await;
    let held = heartbeat.read().await.unwrap().expect("a beat");
    assert_eq!(held.held_since, Some(started));
    assert_eq!(held.beat_at, started);

    // Another worker finding the lock taken writes no beat of its own.
    h.advance(30_000);
    assert_eq!(
        second.tick(&CancellationToken::new()).await.unwrap(),
        TickOutcome::Busy
    );

    // Nor does it take over the hold: it is still dated from the first's start.
    assert_eq!(
        heartbeat.read().await.unwrap().unwrap().held_since,
        Some(started)
    );

    gate.release_all();
    assert!(matches!(
        running.await.unwrap().unwrap(),
        TickOutcome::Ran(_)
    ));
    assert_eq!(heartbeat.read().await.unwrap().unwrap().held_since, None);
}

#[tokio::test]
async fn a_cycle_that_does_not_start_leaves_no_hold() {
    let h = Harness::new().await;
    h.add_channel("feed-a", "/media/anime", &[], feed_a_rules())
        .await;
    let heartbeat = HeartbeatStore::new(h.db.clone());
    let worker = h.worker().with_min_gap(Duration::from_secs(3600));
    run(&worker).await;

    // A second try so soon after the start is refused under the lock.
    h.advance(1_000);
    assert_eq!(
        worker.tick(&CancellationToken::new()).await.unwrap(),
        TickOutcome::TooSoon
    );
    let beat = heartbeat.read().await.unwrap().unwrap();
    assert_eq!(beat.held_since, None);
    assert_eq!(beat.beat_at, h.now());
}
