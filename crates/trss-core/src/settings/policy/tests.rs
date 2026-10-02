use super::*;
use crate::db::{Db, DbError};

async fn store() -> (Db, SettingsStore) {
    let db = Db::open(":memory:").await.unwrap();
    (db.clone(), SettingsStore::new(db))
}

fn order(codes: &[&str]) -> FormatOrder {
    FormatOrder::from_codes(codes).unwrap()
}

#[tokio::test]
async fn the_policy_is_the_defaults_until_saved_and_versioned_after() {
    let (_, store) = store().await;
    let first = store.policy().await.unwrap();
    assert_eq!(first, Policy::default());
    assert_eq!(first.format_order.to_string(), "ass,srt,smi");
    assert_eq!(
        (first.idle_timeout_seconds, first.max_concurrent_jobs),
        (300, 1)
    );
    assert_eq!((first.version, first.saved_at), (0, None));

    let saved = store
        .put_policy(0, order(&["smi", "ass", "srt"]), 600, 2, 1_000)
        .await
        .unwrap();
    assert_eq!(saved.format_order.to_string(), "smi,ass,srt");
    assert_eq!(
        (saved.idle_timeout_seconds, saved.max_concurrent_jobs),
        (600, 2)
    );
    assert_eq!((saved.version, saved.saved_at), (1, Some(1_000)));
    assert_eq!(store.policy().await.unwrap(), saved);

    let again = store
        .put_policy(1, FormatOrder::default(), 60, 3, 2_000)
        .await
        .unwrap();
    assert_eq!((again.version, again.saved_at), (2, Some(2_000)));
}

#[tokio::test]
async fn a_save_from_a_stale_version_or_out_of_range_changes_nothing() {
    let (_, store) = store().await;
    let saved = store
        .put_policy(0, FormatOrder::default(), 300, 1, 1_000)
        .await
        .unwrap();
    // Another screen saved version 1 → 2 first.
    store
        .put_policy(1, order(&["srt", "ass", "smi"]), 300, 1, 2_000)
        .await
        .unwrap();
    let stale = store
        .put_policy(saved.version, order(&["smi", "srt", "ass"]), 900, 2, 3_000)
        .await
        .unwrap_err();
    assert!(matches!(
        stale,
        SettingsError::Conflict {
            expected: 1,
            actual: 2
        }
    ));

    for (idle, jobs) in [(59, 1), (3601, 1), (0, 1), (300, 0), (300, 4)] {
        let refused = store
            .put_policy(2, FormatOrder::default(), idle, jobs, 4_000)
            .await
            .unwrap_err();
        assert!(
            matches!(refused, SettingsError::Invalid(_)),
            "{idle} {jobs}"
        );
    }
    let kept = store.policy().await.unwrap();
    assert_eq!(kept.format_order.to_string(), "srt,ass,smi");
    assert_eq!((kept.version, kept.saved_at), (2, Some(2_000)));
    // The ends of the ranges are accepted.
    store
        .put_policy(2, FormatOrder::default(), 3600, 3, 5_000)
        .await
        .unwrap();
}

#[test]
fn a_format_order_names_each_format_once() {
    assert!(FormatOrder::from_codes(&["ass", "srt", "smi"]).is_some());
    assert!(FormatOrder::from_codes(&["smi", "smi", "ass"]).is_none());
    assert!(FormatOrder::from_codes(&["ass", "srt"]).is_none());
    assert!(FormatOrder::from_codes(&["ass", "srt", "smi", "ass"]).is_none());
    assert!(FormatOrder::from_codes(&["ass", "srt", "vtt"]).is_none());
    assert!(FormatOrder::from_codes(&["ASS", "srt", "smi"]).is_none());
}

#[tokio::test]
async fn the_table_refuses_an_order_that_is_not_one_and_numbers_that_are_not_positive() {
    let (db, _) = store().await;
    let refused = db
        .run::<_, DbError, _>(|c| {
            let insert = |order: &str, idle: i64, jobs: i64| {
                c.execute(
                    "INSERT INTO policy_settings
                         (id, format_order, idle_timeout_seconds, max_concurrent_jobs, version, saved_at)
                     VALUES (1, ?1, ?2, ?3, 1, 1)",
                    params![order, idle, jobs],
                )
                .is_err()
            };
            Ok([
                insert("smi,smi,ass", 300, 1),
                insert("ass,srt", 300, 1),
                insert("ass,srt,smi", 0, 1),
                insert("ass,srt,smi", 300, 0),
            ])
        })
        .await
        .unwrap();
    assert_eq!(refused, [true; 4]);
}

#[tokio::test]
async fn the_works_with_their_own_order_are_listed_newest_first_and_go_with_their_work() {
    let (db, store) = store().await;
    assert!(store.work_format_orders().await.unwrap().is_empty());
    db.run::<_, DbError, _>(|c| {
        c.execute_batch(
            "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
             INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'A'), ('w2', 'f1', 'B');
             INSERT INTO work_subtitle_policy (work_id, format_order, updated_at)
                 VALUES ('w1', 'srt,ass,smi', 10), ('w2', 'smi,srt,ass', 20);",
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let listed = store.work_format_orders().await.unwrap();
    let seen: Vec<(&str, &str, String)> = listed
        .iter()
        .map(|w| {
            (
                w.work_id.as_str(),
                w.name.as_str(),
                w.format_order.to_string(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            ("w2", "B", "smi,srt,ass".to_owned()),
            ("w1", "A", "srt,ass,smi".to_owned())
        ]
    );
    db.run::<_, DbError, _>(|c| {
        c.execute("DELETE FROM works WHERE id = 'w2'", [])?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(store.work_format_orders().await.unwrap().len(), 1);
}
