use std::collections::BTreeSet;

use super::*;
use crate::discovery::{EpisodeFile, ScannedWork, Unrecognized, WorkRead};

fn store() -> LibraryStore {
    LibraryStore::new(Db::open_blocking(":memory:").unwrap())
}

fn video(season: u32, episode: &str, name: &str) -> EpisodeFile {
    EpisodeFile {
        path: format!("Season {season:02}/{name}"),
        kind: FileKind::Video,
        season,
        episode: episode.to_owned(),
    }
}

fn work(name: &str, files: Vec<EpisodeFile>) -> WorkRead {
    WorkRead::Read(ScannedWork {
        dir_name: name.to_owned(),
        seasons: files.iter().map(|f| f.season).collect::<BTreeSet<_>>(),
        files,
        unrecognized: Vec::new(),
    })
}

fn scan(works: Vec<WorkRead>) -> Scan {
    Scan { works }
}

#[tokio::test]
async fn the_first_scan_leaves_times_unknown_and_a_later_one_stamps_only_what_is_new() {
    let store = store();
    let (folder, report) = store
        .add_folder(
            "/w".into(),
            scan(vec![work("A", vec![video(1, "01", "A S01E01.mkv")])]),
            100,
        )
        .await
        .unwrap();
    assert!(report.baseline);
    assert_eq!((report.works_found, report.works_added), (1, 1));

    let report = store
        .record_scan(
            &folder.id,
            Ok(scan(vec![
                work(
                    "A",
                    vec![
                        video(1, "01", "A S01E01.mkv"),
                        video(1, "02", "A S01E02.mkv"),
                    ],
                ),
                work("B", vec![video(1, "01", "B S01E01.mkv")]),
            ])),
            200,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(!report.baseline);
    assert_eq!((report.works_added, report.files_added), (1, 2));

    let works = store.works(&folder.id).await.unwrap();
    let a = &works[0];
    assert_eq!(a.first_seen_at, None);
    let files = a.files();
    assert_eq!(files["Season 01/A S01E01.mkv"].added_at, None);
    assert_eq!(files["Season 01/A S01E02.mkv"].added_at, Some(200));
    assert_eq!(works[1].first_seen_at, Some(200));

    // A third scan changes no time.
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![
                work(
                    "A",
                    vec![
                        video(1, "01", "A S01E01.mkv"),
                        video(1, "02", "A S01E02.mkv"),
                    ],
                ),
                work("B", vec![video(1, "01", "B S01E01.mkv")]),
            ])),
            300,
        )
        .await
        .unwrap();
    let again = store.works(&folder.id).await.unwrap();
    assert_eq!(again, works);
}

#[tokio::test]
async fn gone_files_and_episodes_drop_and_a_gone_work_folder_keeps_its_id() {
    let store = store();
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![
                work(
                    "A",
                    vec![
                        video(1, "01", "A S01E01.mkv"),
                        video(1, "02", "A S01E02.mkv"),
                    ],
                ),
                work("B", vec![video(1, "01", "B S01E01.mkv")]),
            ]),
            100,
        )
        .await
        .unwrap();
    let before = store.works(&folder.id).await.unwrap();

    let report = store
        .record_scan(
            &folder.id,
            Ok(scan(vec![work("A", vec![video(1, "01", "A S01E01.mkv")])])),
            200,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!((report.files_removed, report.works_missing), (1, 1));

    let after = store.works(&folder.id).await.unwrap();
    assert_eq!(after[0].id, before[0].id);
    assert_eq!(after[0].episodes.len(), 1);
    assert!(!after[0].missing);
    assert_eq!(after[1].id, before[1].id);
    assert!(after[1].missing);
    // Its records are kept.
    assert_eq!(after[1].episodes, before[1].episodes);

    // The folder comes back: same work, not missing.
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![
                work("A", vec![video(1, "01", "A S01E01.mkv")]),
                work("B", vec![video(1, "01", "B S01E01.mkv")]),
            ])),
            300,
        )
        .await
        .unwrap();
    let back = store.works(&folder.id).await.unwrap();
    assert_eq!(back[1].id, before[1].id);
    assert!(!back[1].missing);
}

#[tokio::test]
async fn a_failed_scan_records_the_error_and_keeps_what_was_known() {
    let store = store();
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![work("A", vec![video(1, "01", "A S01E01.mkv")])]),
            100,
        )
        .await
        .unwrap();
    let before = store.works(&folder.id).await.unwrap();

    store
        .record_scan(
            &folder.id,
            Err(ScanError {
                message: "폴더를 읽을 권한이 없어요.".into(),
                detail: "x".into(),
            }),
            200,
        )
        .await
        .unwrap();
    let stored = store.folder(&folder.id).await.unwrap().unwrap();
    assert_eq!(stored.error.as_deref(), Some("폴더를 읽을 권한이 없어요."));
    assert_eq!(stored.checked_at, Some(200));
    assert_eq!(store.works(&folder.id).await.unwrap(), before);

    // A good scan clears it.
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![work("A", vec![video(1, "01", "A S01E01.mkv")])])),
            300,
        )
        .await
        .unwrap();
    assert_eq!(store.folder(&folder.id).await.unwrap().unwrap().error, None);
}

#[tokio::test]
async fn an_unreadable_work_keeps_its_records_and_the_folder_is_not_baselined_yet() {
    let store = store();
    let unreadable = |name: &str| WorkRead::Unreadable {
        dir_name: name.to_owned(),
        reason: "읽을 권한이 없어요.".to_owned(),
    };
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![
                work("A", vec![video(1, "01", "A S01E01.mkv")]),
                unreadable("Locked"),
            ]),
            100,
        )
        .await
        .unwrap();
    let stored = store.folder(&folder.id).await.unwrap().unwrap();
    assert!(!stored.baselined);
    assert!(stored.error.unwrap().contains("Locked"));
    // The unreadable work was not recorded; A was.
    assert_eq!(store.works(&folder.id).await.unwrap().len(), 1);

    // Still not baselined: what turns up now is still of unknown age.
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![
                work(
                    "A",
                    vec![
                        video(1, "01", "A S01E01.mkv"),
                        video(1, "02", "A S01E02.mkv"),
                    ],
                ),
                work("Locked", vec![video(1, "01", "L S01E01.mkv")]),
            ])),
            200,
        )
        .await
        .unwrap();
    let works = store.works(&folder.id).await.unwrap();
    assert!(works.iter().all(|w| w.first_seen_at.is_none()));
    assert!(works[0].files().values().all(|f| f.added_at.is_none()));
    assert!(store.folder(&folder.id).await.unwrap().unwrap().baselined);
}

#[tokio::test]
async fn adding_the_same_path_twice_is_refused_and_removing_takes_the_works_along() {
    let store = store();
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![work("A", vec![video(1, "01", "A S01E01.mkv")])]),
            100,
        )
        .await
        .unwrap();
    assert!(matches!(
        store.add_folder("/w".into(), scan(vec![]), 100).await,
        Err(LibraryError::Duplicate)
    ));
    assert_eq!(store.folders().await.unwrap().len(), 1);

    assert_eq!(store.remove_folder(&folder.id).await.unwrap(), Some(1));
    assert_eq!(store.remove_folder(&folder.id).await.unwrap(), None);
    assert!(store.folders().await.unwrap().is_empty());
    let left: i64 = store
        .db
        .run::<_, DbError, _>(|c| {
            Ok(c.query_row(
                "SELECT (SELECT count(*) FROM works) + (SELECT count(*) FROM seasons)
                      + (SELECT count(*) FROM episodes) + (SELECT count(*) FROM media_files)
                      + (SELECT count(*) FROM unrecognized_files)",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn the_summary_counts_works_missing_ones_and_new_ones_within_the_window() {
    let store = store();
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![work("Old", vec![video(1, "01", "O S01E01.mkv")])]),
            100,
        )
        .await
        .unwrap();
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![
                work("Old", vec![video(1, "01", "O S01E01.mkv")]),
                work("Fresh", vec![video(1, "01", "F S01E01.mkv")]),
            ])),
            1_000,
        )
        .await
        .unwrap();
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![
                work("Fresh", vec![video(1, "01", "F S01E01.mkv")]),
                work("Newer", vec![video(1, "01", "N S01E01.mkv")]),
            ])),
            5_000,
        )
        .await
        .unwrap();
    let summary = &store.summaries(2_000).await.unwrap()[0];
    // Old is missing; Fresh is older than the window; only Newer is new.
    assert_eq!(
        (summary.works, summary.missing_works, summary.new_works),
        (3, 1, 1)
    );
    let summary = &store.summaries(1_000).await.unwrap()[0];
    assert_eq!(summary.new_works, 2);
}

#[tokio::test]
async fn following_a_move_keeps_the_id_and_a_merge_keeps_the_destinations() {
    let store = store();
    let (from, _) = store
        .add_folder(
            "/from".into(),
            scan(vec![
                work("Moved", vec![video(1, "01", "M S01E01.mkv")]),
                work("Both", vec![video(2, "01", "B S02E01.mkv")]),
            ]),
            100,
        )
        .await
        .unwrap();
    let (to, _) = store
        .add_folder(
            "/to".into(),
            scan(vec![work("Both", vec![video(1, "01", "B S01E01.mkv")])]),
            100,
        )
        .await
        .unwrap();
    let from_works = store.works(&from.id).await.unwrap();
    let to_works = store.works(&to.id).await.unwrap();
    let moved_id = from_works
        .iter()
        .find(|w| w.dir_name == "Moved")
        .unwrap()
        .id
        .clone();
    let both_from = from_works
        .iter()
        .find(|w| w.dir_name == "Both")
        .unwrap()
        .id
        .clone();
    let both_to = to_works[0].id.clone();

    assert_eq!(
        store.follow_move(&from.id, &to.id, "Moved").await.unwrap(),
        Followed::Moved
    );
    assert_eq!(
        store.follow_move(&from.id, &to.id, "Both").await.unwrap(),
        Followed::Merged
    );
    assert_eq!(
        store.follow_move(&from.id, &to.id, "Nope").await.unwrap(),
        Followed::NotTracked
    );

    assert!(store.works(&from.id).await.unwrap().is_empty());
    let after = store.works(&to.id).await.unwrap();
    let ids: Vec<_> = after
        .iter()
        .map(|w| (w.dir_name.as_str(), w.id.as_str()))
        .collect();
    assert_eq!(
        ids,
        [("Both", both_to.as_str()), ("Moved", moved_id.as_str())]
    );
    assert_ne!(both_from, both_to);
    // Both seasons now belong to the kept work.
    assert_eq!(after[0].seasons, [1, 2]);
    assert_eq!(after[0].episodes.len(), 2);
}

#[tokio::test]
async fn unrecognized_files_are_replaced_by_each_scan() {
    let store = store();
    let mut scanned = ScannedWork {
        dir_name: "A".into(),
        ..ScannedWork::default()
    };
    scanned.unrecognized = vec![
        Unrecognized {
            path: "x.mkv".into(),
            reason: Reason::OutsideSeason,
        },
        Unrecognized {
            path: "y.part".into(),
            reason: Reason::Partial,
        },
    ];
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![WorkRead::Read(scanned.clone())]),
            100,
        )
        .await
        .unwrap();
    assert_eq!(
        store.works(&folder.id).await.unwrap()[0].unrecognized.len(),
        2
    );
    scanned.unrecognized.pop();
    store
        .record_scan(&folder.id, Ok(scan(vec![WorkRead::Read(scanned)])), 200)
        .await
        .unwrap();
    let works = store.works(&folder.id).await.unwrap();
    assert_eq!(works[0].unrecognized.len(), 1);
    assert_eq!(works[0].unrecognized[0].reason, Reason::OutsideSeason);
}
