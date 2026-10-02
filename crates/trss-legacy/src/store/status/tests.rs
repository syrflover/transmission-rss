use super::*;
use crate::store::history::{HistoryResult, HistoryStore, Observation};
use trss_core::db::schema_version;

async fn db() -> Db {
    Db::open(":memory:").await.unwrap()
}

fn read(id: &str, ok: bool) -> ChannelReadResult {
    ChannelReadResult {
        channel_id: id.into(),
        ok,
    }
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[tokio::test]
async fn a_failed_read_keeps_the_time_of_the_last_success() {
    let store = StatusStore::new(db().await);

    store
        .record_reads(
            100,
            vec![read("a", true), read("b", false)],
            ids(&["a", "b"]),
        )
        .await
        .unwrap();
    store
        .record_reads(
            200,
            vec![read("a", false), read("b", true)],
            ids(&["a", "b"]),
        )
        .await
        .unwrap();

    let reads = store.channel_reads().await.unwrap();
    assert_eq!(
        reads,
        vec![
            ChannelRead {
                channel_id: "a".into(),
                ok: false,
                read_at: 200,
                ok_at: Some(100),
            },
            ChannelRead {
                channel_id: "b".into(),
                ok: true,
                read_at: 200,
                ok_at: Some(200),
            },
        ]
    );
}

#[tokio::test]
async fn a_channel_that_never_read_has_no_success_time() {
    let store = StatusStore::new(db().await);
    store
        .record_reads(100, vec![read("a", false)], ids(&["a"]))
        .await
        .unwrap();
    let reads = store.channel_reads().await.unwrap();
    assert_eq!(reads[0].ok_at, None);
    assert!(!reads[0].ok);
}

#[tokio::test]
async fn rows_of_deleted_channels_go_but_an_unread_channel_keeps_its_row() {
    let store = StatusStore::new(db().await);
    store
        .record_reads(
            100,
            vec![read("a", true), read("b", true), read("gone", true)],
            ids(&["a", "b", "gone"]),
        )
        .await
        .unwrap();

    // "gone" was deleted; "b" exists but was not read in this cycle.
    store
        .record_reads(200, vec![read("a", true)], ids(&["a", "b"]))
        .await
        .unwrap();

    let reads = store.channel_reads().await.unwrap();
    let names: Vec<&str> = reads.iter().map(|r| r.channel_id.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(reads[1].read_at, 100);
}

#[tokio::test]
async fn transmission_counts_are_replaced() {
    let store = StatusStore::new(db().await);
    assert_eq!(store.transmission().await.unwrap(), None);

    store
        .record_transmission(
            TransmissionCounts {
                downloading: 2,
                seeding: 5,
                taken_at: 100,
            },
            Vec::new(),
        )
        .await
        .unwrap();
    store
        .record_transmission(
            TransmissionCounts {
                downloading: 0,
                seeding: 7,
                taken_at: 300,
            },
            Vec::new(),
        )
        .await
        .unwrap();

    assert_eq!(
        store.transmission().await.unwrap(),
        Some(TransmissionCounts {
            downloading: 0,
            seeding: 7,
            taken_at: 300,
        })
    );
}

#[tokio::test]
async fn the_downloading_hashes_are_replaced_with_the_counts() {
    let store = StatusStore::new(db().await);
    assert!(store.downloading_hashes().await.unwrap().is_empty());
    let counts = |downloading| TransmissionCounts {
        downloading,
        seeding: 0,
        taken_at: 100,
    };

    store
        .record_transmission(counts(2), ids(&["aaa", "bbb", "aaa", ""]))
        .await
        .unwrap();
    assert_eq!(
        store.downloading_hashes().await.unwrap(),
        ["aaa", "bbb"].iter().map(|h| h.to_string()).collect()
    );

    store
        .record_transmission(counts(1), ids(&["ccc"]))
        .await
        .unwrap();
    assert_eq!(
        store.downloading_hashes().await.unwrap(),
        ["ccc"].iter().map(|h| h.to_string()).collect()
    );
}

#[tokio::test]
async fn the_cycle_interval_is_whatever_the_worker_last_recorded() {
    let store = StatusStore::new(db().await);
    assert_eq!(store.cycle_interval().await.unwrap(), None);

    store.record_cycle_interval(300_000).await.unwrap();
    store.record_cycle_interval(60_000).await.unwrap();

    assert_eq!(store.cycle_interval().await.unwrap(), Some(60_000));
}

#[tokio::test]
async fn the_heartbeat_is_whatever_the_worker_last_wrote() {
    let store = StatusStore::new(db().await);
    assert_eq!(store.heartbeat().await.unwrap(), None);

    store.record_heartbeat(1_000, Some(900)).await.unwrap();
    store.record_heartbeat(16_000, Some(900)).await.unwrap();
    assert_eq!(
        store.heartbeat().await.unwrap(),
        Some(WorkerHeartbeat {
            beat_at: 16_000,
            held_since: Some(900)
        })
    );

    // Letting go of the lock keeps the time and clears the hold.
    store.record_heartbeat(20_000, None).await.unwrap();
    assert_eq!(
        store.heartbeat().await.unwrap(),
        Some(WorkerHeartbeat {
            beat_at: 20_000,
            held_since: None
        })
    );
}

fn observation(key: &str, result: HistoryResult) -> Observation {
    Observation {
        channel_id: "c1".into(),
        channel_label: "https://feed.test/".into(),
        identity_key: key.into(),
        title: key.into(),
        link: "https://feed.test/x".into(),
        result,
        rule_id: None,
        torrent_hash: None,
        reason: None,
    }
}

#[tokio::test]
async fn history_questions_count_by_result_time() {
    let db = db().await;
    let history = HistoryStore::new(db.clone());
    let store = StatusStore::new(db);

    history
        .record(
            1_000,
            vec![
                observation("old-received", HistoryResult::Received),
                observation("old-failed", HistoryResult::AddFailed),
            ],
        )
        .await
        .unwrap();
    history
        .record(
            5_000,
            vec![
                observation("new-received", HistoryResult::Received),
                observation("new-received-2", HistoryResult::Received),
                observation("new-duplicate", HistoryResult::Duplicate),
                observation("new-failed", HistoryResult::AddFailed),
                observation("new-nomatch", HistoryResult::NoMatch),
                observation("new-version-unknown", HistoryResult::VersionUnknown),
            ],
        )
        .await
        .unwrap();

    assert_eq!(
        store.received_since(0).await.unwrap(),
        vec![1_000, 5_000, 5_000]
    );
    assert_eq!(
        store.received_since(2_000).await.unwrap(),
        vec![5_000, 5_000]
    );
    assert_eq!(store.problems_since(2_000).await.unwrap(), 3);
    assert_eq!(store.problems_since(0).await.unwrap(), 4);
    assert_eq!(store.problems_since(9_000).await.unwrap(), 0);
}

// ---------------------------------------------------------------------------
// Read days
// ---------------------------------------------------------------------------

const DAY: Millis = 24 * 60 * 60 * 1000;

/// Reads of the channel `a` at noon of each of the days `from..=to`.
async fn read_days(store: &StatusStore, from: i64, to: i64, ok: bool) {
    for day in from..=to {
        store
            .record_reads(day * DAY + DAY / 2, vec![read("a", ok)], ids(&["a"]))
            .await
            .unwrap();
    }
}

/// The floor as of day 2000, long after the days the tests read on.
async fn floor_of_a(store: &StatusStore) -> Option<i64> {
    floor_of_a_on(store, 2000).await
}

async fn floor_of_a_on(store: &StatusStore, day: i64) -> Option<i64> {
    store
        .read_day_floors(ids(&["a", "b"]), day * DAY)
        .await
        .unwrap()
        .get("a")
        .copied()
}

async fn stored_days(store: &StatusStore, channel: &'static str) -> i64 {
    store
        .db
        .run::<_, StatusError, _>(move |c| {
            Ok(c.query_row(
                "SELECT count(*) FROM channel_read_days WHERE channel_id = ?1",
                [channel],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn the_28th_newest_read_day_is_the_floor_and_fewer_days_give_none() {
    let store = StatusStore::new(db().await);
    read_days(&store, 1000, 1026, true).await;
    assert_eq!(floor_of_a(&store).await, None, "27 days");

    read_days(&store, 1027, 1027, true).await;
    assert_eq!(floor_of_a(&store).await, Some(1000), "28 days");

    // The oldest day goes as a newer one comes: 28 are kept, no more.
    read_days(&store, 1028, 1030, true).await;
    assert_eq!(floor_of_a(&store).await, Some(1003));
    assert_eq!(stored_days(&store, "a").await, 28);
}

#[tokio::test]
async fn a_day_read_twice_counts_once_and_a_failed_read_or_a_gap_counts_for_nothing() {
    let store = StatusStore::new(db().await);
    read_days(&store, 1000, 1013, true).await;
    // Reads again on the same days, and failed reads on others.
    read_days(&store, 1010, 1013, true).await;
    read_days(&store, 1014, 1030, false).await;
    assert_eq!(floor_of_a(&store).await, None, "14 days read");

    // The days that failed are not made up later: a gap stays a gap.
    read_days(&store, 1031, 1044, true).await;
    assert_eq!(floor_of_a(&store).await, Some(1000), "14 + 14 days");
}

#[tokio::test]
async fn a_read_on_the_newest_day_again_adds_nothing() {
    let store = StatusStore::new(db().await);
    read_days(&store, 1000, 1027, true).await;
    read_days(&store, 1027, 1027, true).await;
    assert_eq!(floor_of_a(&store).await, Some(1000));
    assert_eq!(stored_days(&store, "a").await, 28);
}

#[tokio::test]
async fn a_clock_that_went_back_takes_the_days_ahead_of_it_with_it() {
    let store = StatusStore::new(db().await);
    read_days(&store, 1000, 1027, true).await;
    // The clock is now at day 1020: the days after it were stamped by a clock
    // that was ahead.
    read_days(&store, 1020, 1020, true).await;
    assert_eq!(stored_days(&store, "a").await, 21);
    assert_eq!(floor_of_a_on(&store, 1020).await, None);
}

#[tokio::test]
async fn days_written_by_a_clock_that_was_ahead_do_not_count_once_it_is_back() {
    let store = StatusStore::new(db().await);
    // 12 real days, then a clock a long way ahead for 16 days, so that 28 days
    // are stored in all.
    read_days(&store, 1000, 1011, true).await;
    read_days(&store, 5000, 5015, true).await;
    assert_eq!(stored_days(&store, "a").await, 28);

    // The clock is back at day 1012: only 12 real days are behind it. The
    // days ahead are not read days, whether or not another read has been
    // written yet.
    assert_eq!(floor_of_a_on(&store, 1012).await, None);
    read_days(&store, 1012, 1012, true).await;
    assert_eq!(floor_of_a_on(&store, 1012).await, None, "13 real days");
    // They did not crowd the real days out either.
    assert_eq!(stored_days(&store, "a").await, 13);

    // Real days make up the 28 as they come.
    read_days(&store, 1013, 1027, true).await;
    assert_eq!(floor_of_a_on(&store, 1027).await, Some(1000));
}

#[tokio::test]
async fn the_read_days_of_deleted_channels_go_with_them() {
    let store = StatusStore::new(db().await);
    read_days(&store, 1000, 1027, true).await;
    // A later write that no longer lists the channel `a`.
    store
        .record_reads(1028 * DAY, vec![read("b", true)], ids(&["b"]))
        .await
        .unwrap();
    assert_eq!(floor_of_a(&store).await, None);
    assert_eq!(stored_days(&store, "a").await, 0);
    assert_eq!(stored_days(&store, "b").await, 1);
}

#[test]
fn a_day_is_unix_ms_over_a_day() {
    assert_eq!(read_day(0), 0);
    assert_eq!(read_day(DAY - 1), 0);
    assert_eq!(read_day(DAY), 1);
    assert_eq!(read_day(-1), -1);
}

#[tokio::test]
async fn the_torrent_listing_is_none_until_the_worker_writes_one_and_is_replaced_as_a_whole() {
    let store = StatusStore::new(db().await);
    assert_eq!(store.torrent_listing().await.unwrap(), None);

    store
        .record_listing(100, ids(&["AAA", "bbb", "aaa", ""]))
        .await
        .unwrap();
    let listing = store.torrent_listing().await.unwrap().unwrap();
    assert_eq!(listing.taken_at, 100);
    // Hashes are kept lowercase and asked for in either case.
    assert_eq!(listing.hashes, ["aaa", "bbb"].map(String::from).into());
    assert!(listing.holds("BBB") && !listing.holds("ccc"));

    // A look at an empty Transmission is a list too, not "no list".
    store.record_listing(200, vec![]).await.unwrap();
    assert_eq!(
        store.torrent_listing().await.unwrap(),
        Some(TorrentListing {
            taken_at: 200,
            hashes: HashSet::new(),
        })
    );
}

// --- migrations, seen through the store ----------------------------------------------

/// The `user_version` of an opened database.
async fn schema_version_of(db: &Db) -> usize {
    db.run::<_, DbError, _>(|c| {
        Ok(c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize)
    })
    .await
    .unwrap()
}

/// The migration that kept the list of every torrent in Transmission is number 27.
const BEFORE_TORRENT_LISTING: usize = 26;

#[tokio::test]
async fn a_database_from_before_the_torrent_listing_has_none_until_the_worker_writes_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    {
        let conn = crate::store::db::database_at(&path, BEFORE_TORRENT_LISTING);
        conn.execute(
            "INSERT INTO transmission_snapshot (id, downloading, seeding, taken_at)
             VALUES (1, 2, 3, 400)",
            [],
        )
        .unwrap();
    }

    let db = Db::open(&path).await.unwrap();

    assert_eq!(schema_version_of(&db).await, schema_version());
    let status = StatusStore::new(db);
    // The counts the older worker left stay, and say nothing of which
    // torrents are gone.
    assert_eq!(status.transmission().await.unwrap().unwrap().seeding, 3);
    assert_eq!(status.torrent_listing().await.unwrap(), None);
    status.record_listing(500, vec!["aa".into()]).await.unwrap();
    assert!(status.torrent_listing().await.unwrap().unwrap().holds("aa"));
}

/// The migration that added the worker's heartbeat is number 29.
const BEFORE_HEARTBEAT: usize = 28;

#[tokio::test]
async fn a_database_from_before_the_heartbeat_has_none_until_the_worker_writes_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    {
        let conn = crate::store::db::database_at(&path, BEFORE_HEARTBEAT);
        conn.execute(
            "INSERT INTO worker_info (id, cycle_interval_ms) VALUES (1, 300000)",
            [],
        )
        .unwrap();
    }

    let db = Db::open(&path).await.unwrap();

    assert_eq!(schema_version_of(&db).await, schema_version());
    let status = StatusStore::new(db);
    assert_eq!(status.cycle_interval().await.unwrap(), Some(300_000));
    assert_eq!(status.heartbeat().await.unwrap(), None);
    status.record_heartbeat(500, Some(400)).await.unwrap();
    assert_eq!(status.heartbeat().await.unwrap().unwrap().beat_at, 500);
}
