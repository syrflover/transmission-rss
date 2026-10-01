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
    store.advance(row.id, 30, Step::Removing).await.unwrap();
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

    store.advance(row.id, 20, Step::Removing).await.unwrap();
    let marks = store.marks("c1".into(), keys.clone()).await.unwrap();
    assert_eq!(marks.get("14"), Some(&Mark::Superseded));

    store.advance(row.id, 30, Step::Done).await.unwrap();
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
    let a = store
        .create(
            10,
            new(item(&db, "14v2").await, None, RevisionState::Receiving),
        )
        .await
        .unwrap();
    let b = store
        .create(
            10,
            new(item(&db, "15v2").await, None, RevisionState::Receiving),
        )
        .await
        .unwrap();
    let c = store
        .create(
            10,
            new(item(&db, "16v2").await, None, RevisionState::Receiving),
        )
        .await
        .unwrap();
    store
        .advance(
            a.id,
            20,
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
            Step::Removed {
                reason: Some("busy".into()),
            },
        )
        .await
        .unwrap();
    store
        .advance(c.id, 40, Step::Removed { reason: None })
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

    store.advance(a.id, 50, Step::Cleared).await.unwrap();
    store.advance(c.id, 50, Step::Done).await.unwrap();
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
