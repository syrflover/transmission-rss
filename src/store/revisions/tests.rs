use super::*;
use crate::store::history::{HistoryResult, HistoryStore, Observation};

async fn db() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    (dir, db)
}

/// Records an item of channel `c1` with `key` and returns its ID.
async fn item(db: &Db, key: &str) -> i64 {
    let history = HistoryStore::new(db.clone());
    history
        .record(
            1,
            vec![Observation {
                channel_id: "c1".into(),
                channel_label: "https://x/".into(),
                identity_key: key.into(),
                title: format!("[SubsPlease] Show - {key} (1080p).mkv"),
                link: "magnet:?".into(),
                result: HistoryResult::Received,
                rule_id: Some("r1".into()),
                torrent_hash: Some(format!("hash-{key}")),
                reason: None,
            }],
        )
        .await
        .unwrap();
    history
        .item_by_key("c1".into(), key.into())
        .await
        .unwrap()
        .unwrap()
        .id
}

fn new(item_id: i64, old_item_id: Option<i64>, state: RevisionState) -> NewRevision {
    NewRevision {
        item_id,
        old_item_id,
        rule_id: "r1".into(),
        folder: "/media/Show/Season 01".into(),
        episode_name: "Show S01E14.mkv".into(),
        old_version: Some(1),
        new_version: 2,
        old_crc: None,
        expected_crc: Some("1A2B3C4D".into()),
        torrent_hash: Some("hash-14v2".into()),
        state,
        reason: None,
    }
}

#[tokio::test]
async fn an_item_is_decided_once() {
    let (_dir, db) = db().await;
    let store = RevisionStore::new(db.clone());
    let v2 = item(&db, "14v2").await;
    let first = store
        .create(10, new(v2, None, RevisionState::Unknown))
        .await
        .unwrap();
    let again = store
        .create(20, new(v2, None, RevisionState::Receiving))
        .await
        .unwrap();
    assert_eq!(again, first);
    assert_eq!(first.state, RevisionState::Unknown);
    assert_eq!(first.created_at, 10);
}

#[tokio::test]
async fn a_confirmed_unknown_revision_is_received_without_a_crc_check() {
    let (_dir, db) = db().await;
    let store = RevisionStore::new(db.clone());
    let v2 = item(&db, "14v2").await;
    let mut unknown = new(v2, None, RevisionState::Unknown);
    unknown.torrent_hash = None;
    unknown.reason = Some("no crc".into());
    store.create(10, unknown).await.unwrap();

    let row = store
        .confirm(v2, 20, "h2".into(), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, RevisionState::Receiving);
    assert_eq!(row.torrent_hash.as_deref(), Some("h2"));
    assert_eq!((row.expected_crc, row.reason), (None, None));

    // A row under way is not confirmed again.
    store
        .advance(row.id, 30, RevisionState::Receiving, Step::Removing)
        .await
        .unwrap();
    let row = store
        .confirm(v2, 40, "h3".into(), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, RevisionState::Removing);
    assert_eq!(row.torrent_hash.as_deref(), Some("h2"));
}

#[tokio::test]
async fn marks_say_which_items_a_cycle_leaves_to_the_replacement() {
    let (_dir, db) = db().await;
    let store = RevisionStore::new(db.clone());
    let v1 = item(&db, "14").await;
    let v2 = item(&db, "14v2").await;
    let other = item(&db, "15").await;
    let row = store
        .create(10, new(v2, Some(v1), RevisionState::Receiving))
        .await
        .unwrap();
    let keys = vec!["14".to_owned(), "14v2".to_owned(), "15".to_owned()];

    let marks = store.marks("c1".into(), keys.clone()).await.unwrap();
    // The old item's torrent is still there while the new one downloads.
    assert_eq!(marks.get("14"), None);
    assert_eq!(
        marks.get("14v2"),
        Some(&Mark::Revision(RevisionState::Receiving))
    );
    assert_eq!(marks.get("15"), None);
    assert_eq!(store.held_hashes().await.unwrap(), vec!["hash-14v2"]);

    store
        .advance(row.id, 20, RevisionState::Receiving, Step::Removing)
        .await
        .unwrap();
    let marks = store.marks("c1".into(), keys.clone()).await.unwrap();
    assert_eq!(marks.get("14"), Some(&Mark::Superseded));

    store
        .advance(row.id, 30, RevisionState::Removing, Step::Done)
        .await
        .unwrap();
    let marks = store.marks("c1".into(), keys).await.unwrap();
    assert_eq!(marks.get("14"), Some(&Mark::Superseded));
    assert_eq!(
        marks.get("14v2"),
        Some(&Mark::Revision(RevisionState::Done))
    );
    assert!(store.held_hashes().await.unwrap().is_empty());
    assert!(store
        .marks("c2".into(), vec!["14".into()])
        .await
        .unwrap()
        .is_empty());
    let _ = other;
}

#[tokio::test]
async fn failures_are_failed_rows_and_renames_still_waiting() {
    let (_dir, db) = db().await;
    let store = RevisionStore::new(db.clone());
    // Three episodes, each with its own torrent.
    let mut rows = Vec::new();
    for episode in ["14", "15", "16"] {
        let mut row = new(
            item(&db, &format!("{episode}v2")).await,
            None,
            RevisionState::Receiving,
        );
        row.episode_name = format!("Show S01E{episode}.mkv");
        row.torrent_hash = Some(format!("hash-{episode}v2"));
        rows.push(store.create(10, row).await.unwrap());
    }
    let [a, b, c] = <[Revision; 3]>::try_from(rows).unwrap();
    store
        .advance(
            a.id,
            20,
            RevisionState::Receiving,
            Step::Failed {
                reason: "crc".into(),
                received_name: Some("v2.mkv".into()),
            },
        )
        .await
        .unwrap();
    store
        .advance(
            b.id,
            30,
            RevisionState::Receiving,
            Step::Removed {
                reason: Some("busy".into()),
            },
        )
        .await
        .unwrap();
    store
        .advance(
            c.id,
            40,
            RevisionState::Receiving,
            Step::Removed { reason: None },
        )
        .await
        .unwrap();

    let failures = store.failures().await.unwrap();
    assert_eq!(
        failures.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![b.id, a.id]
    );
    let shown = store.in_work_folder("/media/Show".into()).await.unwrap();
    assert_eq!(shown.len(), 2);
    assert!(store
        .in_work_folder("/media/Sho".into())
        .await
        .unwrap()
        .is_empty());

    store
        .advance(a.id, 50, RevisionState::Failed, Step::Cleared)
        .await
        .unwrap();
    store
        .advance(c.id, 50, RevisionState::Removed, Step::Done)
        .await
        .unwrap();
    let failures = store.failures().await.unwrap();
    assert_eq!(
        failures.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![b.id]
    );
    let done = store.by_item(c.item_id).await.unwrap().unwrap();
    assert_eq!(
        (done.state, done.replaced_at),
        (RevisionState::Done, Some(50))
    );
    assert_eq!(store.open().await.unwrap().len(), 1);
}

/// A row of `key` (`14v2`, `14v3`) for the episode `Show S01E14.mkv`, with
/// its own torrent `hash-<key>`.
fn of_episode(item_id: i64, key: &str, version: u32) -> NewRevision {
    let mut row = new(item_id, None, RevisionState::Receiving);
    row.new_version = version;
    row.torrent_hash = Some(format!("hash-{key}"));
    row
}

fn old() -> OldVideo {
    OldVideo {
        item_id: None,
        version: Some(1),
        torrent_hash: Some("hash-14".into()),
    }
}

async fn verified(store: &RevisionStore, id: i64) {
    let step = Step::Verified {
        received_name: format!("v{id}.mkv"),
        file_crc: "1A2B3C4D".into(),
    };
    store
        .advance(id, 15, RevisionState::Receiving, step)
        .await
        .unwrap();
}

#[tokio::test]
async fn one_replacement_of_an_episode_removes_the_old_video_at_a_time() {
    let (_dir, db) = db().await;
    let store = RevisionStore::new(db.clone());
    let v2 = store
        .create(10, of_episode(item(&db, "14v2").await, "14v2", 2))
        .await
        .unwrap();
    let v3 = store
        .create(10, of_episode(item(&db, "14v3").await, "14v3", 3))
        .await
        .unwrap();
    verified(&store, v2.id).await;
    verified(&store, v3.id).await;

    // A higher revision on its way overtakes the lower one.
    assert_eq!(store.verdict(v2.id).await.unwrap(), Claim::Overtaken);
    assert_eq!(
        store.claim(v2.id, 20, old()).await.unwrap(),
        Claim::Overtaken
    );
    assert_eq!(
        store.by_item(v2.item_id).await.unwrap().unwrap().state,
        RevisionState::Verified,
        "the caller writes the skip"
    );

    assert_eq!(store.claim(v3.id, 20, old()).await.unwrap(), Claim::Go);
    let row = store.by_item(v3.item_id).await.unwrap().unwrap();
    assert_eq!(row.state, RevisionState::Removing);
    assert_eq!(row.old_torrent_hash.as_deref(), Some("hash-14"));
    assert_eq!(row.old_version, Some(1));
    // Claimed again after a restart: still its own.
    assert_eq!(store.claim(v3.id, 21, old()).await.unwrap(), Claim::Go);

    // Another `14v3` (another torrent) waits while the first one removes.
    let other = store
        .create(10, of_episode(item(&db, "14v3b").await, "14v3b", 3))
        .await
        .unwrap();
    verified(&store, other.id).await;
    assert_eq!(store.claim(other.id, 22, old()).await.unwrap(), Claim::Wait);
    store
        .advance(
            v3.id,
            23,
            RevisionState::Removing,
            Step::Removed { reason: None },
        )
        .await
        .unwrap();
    assert_eq!(store.verdict(other.id).await.unwrap(), Claim::Wait);

    // Done: the lower and equal revisions still checked are skipped.
    store
        .advance(v3.id, 30, RevisionState::Removed, Step::Done)
        .await
        .unwrap();
    for id in [v2.item_id, other.item_id] {
        let row = store.by_item(id).await.unwrap().unwrap();
        assert_eq!(row.state, RevisionState::Skipped);
        assert_eq!(row.reason.as_deref(), Some(OVERTAKEN));
    }
}

/// A lower revision skipped for a higher one on its way keeps that row, and
/// goes back to its first step when that row fails; one skipped for another
/// reason, or for a replacement that is done, does not.
#[tokio::test]
async fn a_revision_skipped_for_a_higher_one_comes_back_when_that_one_fails() {
    let (_dir, db) = db().await;
    let store = RevisionStore::new(db.clone());
    let v2 = store
        .create(10, of_episode(item(&db, "14v2").await, "14v2", 2))
        .await
        .unwrap();
    let v3 = store
        .create(10, of_episode(item(&db, "14v3").await, "14v3", 3))
        .await
        .unwrap();
    let v4 = store
        .create(10, of_episode(item(&db, "14v4").await, "14v4", 4))
        .await
        .unwrap();
    let other = store
        .create(10, of_episode(item(&db, "14v2b").await, "14v2b", 2))
        .await
        .unwrap();
    verified(&store, v2.id).await;
    verified(&store, other.id).await;

    // Skipped for the highest revision on its way.
    assert!(store
        .advance(v2.id, 20, RevisionState::Verified, Step::Overtaken)
        .await
        .unwrap());
    let row = store.by_item(v2.item_id).await.unwrap().unwrap();
    assert_eq!(row.state, RevisionState::Skipped);
    assert_eq!(row.reason.as_deref(), Some(OVERTAKEN));
    assert_eq!(row.overtaken_by, Some(v4.id));
    let skip = Step::Skipped {
        reason: "the folder holds it".into(),
    };
    assert!(store
        .advance(other.id, 20, RevisionState::Verified, skip)
        .await
        .unwrap());
    // `14v3` is skipped for `14v4` too.
    assert!(store
        .advance(v3.id, 20, RevisionState::Receiving, Step::Overtaken)
        .await
        .unwrap());

    // `14v4` fails: the rows skipped for it start over, the other one stays.
    let failed = Step::Failed {
        reason: "stopped".into(),
        received_name: None,
    };
    assert!(store
        .advance(v4.id, 30, RevisionState::Receiving, failed.clone())
        .await
        .unwrap());
    for id in [v2.item_id, v3.item_id] {
        let row = store.by_item(id).await.unwrap().unwrap();
        assert_eq!(row.state, RevisionState::Receiving, "{row:?}");
        assert_eq!(
            (
                row.reason,
                row.received_name,
                row.file_crc,
                row.overtaken_by
            ),
            (None, None, None, None)
        );
        assert_eq!(row.updated_at, 30);
    }
    let row = store.by_item(other.item_id).await.unwrap().unwrap();
    assert_eq!(row.state, RevisionState::Skipped);

    // `14v2` is overtaken again, now by `14v3`; with nothing overtaking it
    // the skip is not written.
    assert!(store
        .advance(v2.id, 40, RevisionState::Receiving, Step::Overtaken)
        .await
        .unwrap());
    assert_eq!(
        store
            .by_item(v2.item_id)
            .await
            .unwrap()
            .unwrap()
            .overtaken_by,
        Some(v3.id)
    );
    assert!(store
        .advance(v3.id, 50, RevisionState::Receiving, failed)
        .await
        .unwrap());
    assert!(!store
        .advance(v2.id, 60, RevisionState::Receiving, Step::Overtaken)
        .await
        .unwrap());
    assert_eq!(
        store.by_item(v2.item_id).await.unwrap().unwrap().state,
        RevisionState::Receiving
    );
}

/// A revision skipped for a replacement that is done does not come back.
#[tokio::test]
async fn a_revision_skipped_for_a_done_one_is_not_bound_to_it() {
    let (_dir, db) = db().await;
    let store = RevisionStore::new(db.clone());
    let v2 = store
        .create(10, of_episode(item(&db, "14v2").await, "14v2", 2))
        .await
        .unwrap();
    let v3 = store
        .create(10, of_episode(item(&db, "14v3").await, "14v3", 3))
        .await
        .unwrap();
    for (from, step) in [
        (RevisionState::Receiving, Step::Removing),
        (RevisionState::Removing, Step::Removed { reason: None }),
        (RevisionState::Removed, Step::Done),
    ] {
        assert!(store.advance(v3.id, 20, from, step).await.unwrap());
    }
    let row = store.by_item(v2.item_id).await.unwrap().unwrap();
    assert_eq!(
        (row.state, row.overtaken_by),
        (RevisionState::Skipped, None)
    );
    // Written by `Overtaken` too, the skip is bound to no row.
    let v2b = store
        .create(10, of_episode(item(&db, "14v2b").await, "14v2b", 2))
        .await
        .unwrap();
    assert!(store
        .advance(v2b.id, 30, RevisionState::Receiving, Step::Overtaken)
        .await
        .unwrap());
    let row = store.by_item(v2b.item_id).await.unwrap().unwrap();
    assert_eq!(
        (row.state, row.overtaken_by),
        (RevisionState::Skipped, None)
    );
}

#[tokio::test]
async fn a_claim_after_a_restart_keeps_the_old_torrent_it_found_first() {
    let (_dir, db) = db().await;
    let store = RevisionStore::new(db.clone());
    let v2 = store
        .create(10, of_episode(item(&db, "14v2").await, "14v2", 2))
        .await
        .unwrap();
    verified(&store, v2.id).await;
    assert_eq!(store.claim(v2.id, 20, old()).await.unwrap(), Claim::Go);

    // The old torrent was removed before the restart: the second look finds
    // no torrent and must not forget the one the first look found.
    let gone = OldVideo {
        torrent_hash: None,
        ..old()
    };
    assert_eq!(store.claim(v2.id, 21, gone).await.unwrap(), Claim::Go);
    let row = store.by_item(v2.item_id).await.unwrap().unwrap();
    assert_eq!(row.old_torrent_hash.as_deref(), Some("hash-14"));
}

#[tokio::test]
async fn a_removing_row_holds_the_old_torrent_it_asked_to_remove() {
    let (_dir, db) = db().await;
    let store = RevisionStore::new(db.clone());
    let v2 = store
        .create(10, of_episode(item(&db, "14v2").await, "14v2", 2))
        .await
        .unwrap();
    verified(&store, v2.id).await;
    assert_eq!(store.held_hashes().await.unwrap(), vec!["hash-14v2"]);

    assert_eq!(store.claim(v2.id, 20, old()).await.unwrap(), Claim::Go);
    let mut held = store.held_hashes().await.unwrap();
    held.sort();
    assert_eq!(held, vec!["hash-14", "hash-14v2"]);

    // Once removed, the old torrent is the cycle's to take out like any other.
    assert!(store
        .advance(
            v2.id,
            30,
            RevisionState::Removing,
            Step::Removed { reason: None }
        )
        .await
        .unwrap());
    assert_eq!(store.held_hashes().await.unwrap(), vec!["hash-14v2"]);
}

#[tokio::test]
async fn a_step_from_a_state_the_row_has_left_is_not_written() {
    let (_dir, db) = db().await;
    let store = RevisionStore::new(db.clone());
    let v2 = store
        .create(10, of_episode(item(&db, "14v2").await, "14v2", 2))
        .await
        .unwrap();
    assert!(store
        .advance(v2.id, 20, RevisionState::Receiving, Step::Removing)
        .await
        .unwrap());
    // A step decided from what the row said before is not written over it.
    let stale = Step::Failed {
        reason: "stopped".into(),
        received_name: None,
    };
    assert!(!store
        .advance(v2.id, 30, RevisionState::Receiving, stale)
        .await
        .unwrap());
    let row = store.by_item(v2.item_id).await.unwrap().unwrap();
    assert_eq!((row.state, row.reason), (RevisionState::Removing, None));
}

#[tokio::test]
async fn a_row_whose_torrent_another_row_has_is_skipped() {
    let (_dir, db) = db().await;
    let store = RevisionStore::new(db.clone());
    let first = store
        .create(10, of_episode(item(&db, "14v2").await, "14v2", 2))
        .await
        .unwrap();
    assert_eq!(first.state, RevisionState::Receiving);
    // The same torrent through another item (another channel).
    let second = store
        .create(11, of_episode(item(&db, "14v2b").await, "14v2", 2))
        .await
        .unwrap();
    assert_eq!(second.state, RevisionState::Skipped);
    assert_eq!(second.reason.as_deref(), Some(SAME_TORRENT));

    // `다시 받기` of a `버전 미상` row with that torrent is skipped too.
    let mut unknown = of_episode(item(&db, "14v2c").await, "x", 2);
    unknown.state = RevisionState::Unknown;
    unknown.torrent_hash = None;
    store.create(12, unknown.clone()).await.unwrap();
    let row = store
        .confirm(unknown.item_id, 13, "hash-14v2".into(), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, RevisionState::Skipped);
    assert_eq!(store.held_hashes().await.unwrap(), vec!["hash-14v2"]);
}

#[tokio::test]
async fn a_failure_before_the_video_was_received_is_received_again() {
    let (_dir, db) = db().await;
    let store = RevisionStore::new(db.clone());
    let v2 = item(&db, "14v2").await;
    let row = store.create(10, of_episode(v2, "14v2", 2)).await.unwrap();
    let stopped = Step::Failed {
        reason: "stopped".into(),
        received_name: None,
    };
    store
        .advance(row.id, 20, RevisionState::Receiving, stopped)
        .await
        .unwrap();

    let marks = store.marks("c1".into(), vec!["14v2".into()]).await.unwrap();
    let Some(Mark::Retry(failed)) = marks.get("14v2") else {
        panic!("{marks:?}")
    };
    assert!(failed.not_received());

    let row = store
        .reopen(row.id, 30, "hash-again".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, RevisionState::Receiving);
    assert_eq!(
        (row.torrent_hash.as_deref(), row.reason),
        (Some("hash-again"), None)
    );

    // A failure with the new video received is not reopened.
    let crc = Step::Failed {
        reason: "crc".into(),
        received_name: Some("v2.mkv".into()),
    };
    store
        .advance(row.id, 40, RevisionState::Receiving, crc)
        .await
        .unwrap();
    let row = store
        .reopen(row.id, 50, "hash-3".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, RevisionState::Failed);
    let marks = store.marks("c1".into(), vec!["14v2".into()]).await.unwrap();
    assert_eq!(
        marks.get("14v2"),
        Some(&Mark::Revision(RevisionState::Failed))
    );
}

#[tokio::test]
async fn every_item_of_the_removed_torrent_is_superseded() {
    let (_dir, db) = db().await;
    let store = RevisionStore::new(db.clone());
    let v1 = item(&db, "14").await;
    // The same torrent in channel `c2`.
    HistoryStore::new(db.clone())
        .record(
            1,
            vec![Observation {
                channel_id: "c2".into(),
                channel_label: "https://y/".into(),
                identity_key: "other-14".into(),
                title: "[SubsPlease] Show - 14 (1080p).mkv".into(),
                link: "magnet:?".into(),
                result: HistoryResult::Duplicate,
                rule_id: Some("r2".into()),
                torrent_hash: Some("hash-14".into()),
                reason: None,
            }],
        )
        .await
        .unwrap();
    let row = store
        .create(10, of_episode(item(&db, "14v2").await, "14v2", 2))
        .await
        .unwrap();
    verified(&store, row.id).await;
    let old = OldVideo {
        item_id: Some(v1),
        version: Some(1),
        torrent_hash: Some("hash-14".into()),
    };
    assert_eq!(store.claim(row.id, 20, old).await.unwrap(), Claim::Go);

    let marks = store
        .marks("c2".into(), vec!["other-14".into()])
        .await
        .unwrap();
    assert_eq!(marks.get("other-14"), Some(&Mark::Superseded));
    let marks = store.marks("c1".into(), vec!["14".into()]).await.unwrap();
    assert_eq!(marks.get("14"), Some(&Mark::Superseded));

    let replacements = store.replacements().await.unwrap();
    assert_eq!(replacements.len(), 1);
    assert_eq!(
        replacements[0].title,
        "[SubsPlease] Show - 14v2 (1080p).mkv"
    );
    assert_eq!(replacements[0].new_version, 2);
}
