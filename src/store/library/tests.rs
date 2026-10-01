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
            &[],
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
            &[],
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
            &[],
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

fn unreadable(name: &str) -> WorkRead {
    WorkRead::Unreadable {
        dir_name: name.to_owned(),
        reason: "읽을 권한이 없어요.".to_owned(),
    }
}

fn find<'a>(works: &'a [WorkRecord], name: &str) -> &'a WorkRecord {
    works
        .iter()
        .find(|w| w.dir_name == name)
        .unwrap_or_else(|| panic!("no work {name}"))
}

#[tokio::test]
async fn one_unreadable_work_does_not_hold_back_the_baseline_of_the_others() {
    let store = store();
    let (folder, report) = store
        .add_folder(
            "/w".into(),
            scan(vec![
                work("A", vec![video(1, "01", "A S01E01.mkv")]),
                unreadable("Locked"),
            ]),
            100,
            &[],
        )
        .await
        .unwrap();
    assert!(report.baseline);
    assert_eq!(report.works_unreadable, 1);
    let stored = store.folder(&folder.id).await.unwrap().unwrap();
    assert!(stored.baselined, "the first reading baselines the folder");
    assert!(stored.error.unwrap().contains("Locked"));
    // The unreadable work is not recorded; A is, of unknown age.
    let works = store.works(&folder.id).await.unwrap();
    assert_eq!(works.len(), 1);
    assert_eq!(find(&works, "A").first_seen_at, None);

    // A later scan: a new file of A and a new work B are stamped even though
    // Locked is still unreadable; Locked stays unrecorded.
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
                unreadable("Locked"),
            ])),
            200,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(!report.baseline);
    assert!(report.error.unwrap().contains("Locked"));
    let works = store.works(&folder.id).await.unwrap();
    assert_eq!(works.len(), 2);
    let a = find(&works, "A");
    assert_eq!(a.files()["Season 01/A S01E01.mkv"].added_at, None);
    assert_eq!(a.files()["Season 01/A S01E02.mkv"].added_at, Some(200));
    let b = find(&works, "B");
    assert_eq!(b.first_seen_at, Some(200));
    assert_eq!(b.files()["Season 01/B S01E01.mkv"].added_at, Some(200));

    // Locked becomes readable: it was there when the folder was first read, so
    // its files are of unknown age and it is not newly found.
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![
                work("A", vec![video(1, "01", "A S01E01.mkv")]),
                work("Locked", vec![video(1, "01", "L S01E01.mkv")]),
            ])),
            300,
        )
        .await
        .unwrap();
    let works = store.works(&folder.id).await.unwrap();
    let locked = find(&works, "Locked");
    assert_eq!(locked.first_seen_at, None);
    assert_eq!(locked.files()["Season 01/L S01E01.mkv"].added_at, None);
    // From then on it is an ordinary work: what it gets next is stamped.
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![work(
                "Locked",
                vec![
                    video(1, "01", "L S01E01.mkv"),
                    video(1, "02", "L S01E02.mkv"),
                ],
            )])),
            400,
        )
        .await
        .unwrap();
    let works = store.works(&folder.id).await.unwrap();
    let locked = find(&works, "Locked");
    assert_eq!(locked.files()["Season 01/L S01E01.mkv"].added_at, None);
    assert_eq!(locked.files()["Season 01/L S01E02.mkv"].added_at, Some(400));
}

#[tokio::test]
async fn a_work_folder_first_seen_unreadable_in_a_later_scan_is_dated_by_that_scan_with_unknown_files(
) {
    let store = store();
    let (folder, _) = store
        .add_folder("/w".into(), scan(vec![work("A", vec![])]), 100, &[])
        .await
        .unwrap();
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![work("A", vec![]), unreadable("New")])),
            200,
        )
        .await
        .unwrap();
    assert_eq!(store.works(&folder.id).await.unwrap().len(), 1);

    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![
                work("A", vec![]),
                work("New", vec![video(1, "01", "N S01E01.mkv")]),
            ])),
            300,
        )
        .await
        .unwrap();
    let works = store.works(&folder.id).await.unwrap();
    let new = find(&works, "New");
    // First seen at 200, though unreadable; the files may have been there then.
    assert_eq!(new.first_seen_at, Some(200));
    assert_eq!(new.files()["Season 01/N S01E01.mkv"].added_at, None);
}

#[tokio::test]
async fn an_unreadable_folder_that_is_gone_is_new_when_it_comes_back() {
    let store = store();
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![work("A", vec![]), unreadable("Locked")]),
            100,
            &[],
        )
        .await
        .unwrap();
    // It goes away without ever having been read.
    store
        .record_scan(&folder.id, Ok(scan(vec![work("A", vec![])])), 200)
        .await
        .unwrap();
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![
                work("A", vec![]),
                work("Locked", vec![video(1, "01", "L S01E01.mkv")]),
            ])),
            300,
        )
        .await
        .unwrap();
    let works = store.works(&folder.id).await.unwrap();
    let locked = find(&works, "Locked");
    assert_eq!(locked.first_seen_at, Some(300));
    assert_eq!(locked.files()["Season 01/L S01E01.mkv"].added_at, Some(300));
}

#[tokio::test]
async fn a_recorded_work_that_becomes_unreadable_keeps_its_records_and_times() {
    let store = store();
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![work("A", vec![video(1, "01", "A S01E01.mkv")])]),
            100,
            &[],
        )
        .await
        .unwrap();
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![work(
                "A",
                vec![
                    video(1, "01", "A S01E01.mkv"),
                    video(1, "02", "A S01E02.mkv"),
                ],
            )])),
            200,
        )
        .await
        .unwrap();
    let before = store.works(&folder.id).await.unwrap();

    store
        .record_scan(&folder.id, Ok(scan(vec![unreadable("A")])), 300)
        .await
        .unwrap();
    assert_eq!(store.works(&folder.id).await.unwrap(), before);

    // Readable again with a file more: only that one is stamped.
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![work(
                "A",
                vec![
                    video(1, "01", "A S01E01.mkv"),
                    video(1, "02", "A S01E02.mkv"),
                    video(1, "03", "A S01E03.mkv"),
                ],
            )])),
            400,
        )
        .await
        .unwrap();
    let works = store.works(&folder.id).await.unwrap();
    let a = find(&works, "A");
    assert_eq!(a.files()["Season 01/A S01E01.mkv"].added_at, None);
    assert_eq!(a.files()["Season 01/A S01E02.mkv"].added_at, Some(200));
    assert_eq!(a.files()["Season 01/A S01E03.mkv"].added_at, Some(400));
}

#[tokio::test]
async fn adding_the_same_path_twice_is_refused_and_removing_takes_the_works_along() {
    let store = store();
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![work("A", vec![video(1, "01", "A S01E01.mkv")])]),
            100,
            &[],
        )
        .await
        .unwrap();
    assert!(matches!(
        store.add_folder("/w".into(), scan(vec![]), 100, &[]).await,
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
            &[],
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
            &[],
        )
        .await
        .unwrap();
    let (to, _) = store
        .add_folder(
            "/to".into(),
            scan(vec![work("Both", vec![video(1, "01", "B S01E01.mkv")])]),
            100,
            std::slice::from_ref(&from),
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
            &[],
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

// --- the library list ---------------------------------------------------------------

fn subtitle(season: u32, episode: &str, name: &str) -> EpisodeFile {
    EpisodeFile {
        kind: FileKind::Subtitle,
        ..video(season, episode, name)
    }
}

fn range(first: &str, last: &str) -> EpisodeRange {
    EpisodeRange {
        first: first.into(),
        last: last.into(),
    }
}

#[tokio::test]
async fn the_list_summarizes_the_latest_season_with_ranges_split_at_gaps() {
    let store = store();
    let mut files = vec![
        video(1, "01", "A S01E01.mkv"),
        subtitle(1, "01", "A S01E01.ass"),
    ];
    for e in 1..=12 {
        files.push(video(2, &format!("{e:02}"), &format!("A S02E{e:02}.mkv")));
        if e != 4 {
            files.push(subtitle(
                2,
                &format!("{e:02}"),
                &format!("A S02E{e:02}.ass"),
            ));
        }
    }
    store
        .add_folder("/w".into(), scan(vec![work("A", files)]), 100, &[])
        .await
        .unwrap();

    let list = store.overview().await.unwrap();
    assert_eq!(list.len(), 1);
    let a = &list[0];
    assert_eq!(a.dir_name, "A");
    assert_eq!(a.watch_folder_path, "/w");
    assert_eq!(a.latest_season, Some(2));
    assert_eq!(a.video, [range("01", "12")]);
    assert_eq!(a.subtitle, [range("01", "03"), range("05", "12")]);
    assert_eq!(a.subtitle_coverage, Some(SubtitleCoverage::Some));
    assert!(!a.subtitle_check_needed);
}

#[tokio::test]
async fn an_unrecognized_subtitle_asks_for_a_check_and_a_download_in_progress_does_not() {
    let store = store();
    let unrecognized = |path: &str, reason: Reason| Unrecognized {
        path: path.into(),
        reason,
    };
    let with = |name: &str, extra: Vec<Unrecognized>| {
        WorkRead::Read(ScannedWork {
            dir_name: name.into(),
            seasons: BTreeSet::from([1]),
            files: vec![video(1, "01", &format!("{name} S01E01.mkv"))],
            unrecognized: extra,
        })
    };
    store
        .add_folder(
            "/w".into(),
            scan(vec![
                with("Plain", vec![]),
                with(
                    "Sub",
                    vec![unrecognized("Season 01/x.KO.srt", Reason::NoEpisode)],
                ),
                with(
                    "Part",
                    vec![unrecognized("Season 01/x.mkv.part", Reason::Partial)],
                ),
                // A video that could not be placed is not a subtitle problem.
                with(
                    "Vid",
                    vec![unrecognized("Season 01/y.mkv", Reason::NoEpisode)],
                ),
            ]),
            100,
            &[],
        )
        .await
        .unwrap();
    let list = store.overview().await.unwrap();
    let needed: Vec<(&str, bool)> = list
        .iter()
        .map(|w| (w.dir_name.as_str(), w.subtitle_check_needed))
        .collect();
    assert_eq!(
        needed,
        [
            ("Part", false),
            ("Plain", false),
            ("Sub", true),
            ("Vid", false)
        ]
    );
    // No subtitle is confirmed for any of them.
    assert!(list.iter().all(|w| w.subtitle.is_empty()));
    assert!(list
        .iter()
        .all(|w| w.subtitle_coverage == Some(SubtitleCoverage::None)));
}

#[tokio::test]
async fn a_work_whose_folder_is_gone_stays_listed_without_holdings_and_keeps_its_times() {
    let store = store();
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![work("Keep", vec![video(1, "01", "K S01E01.mkv")])]),
            100,
            &[],
        )
        .await
        .unwrap();
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![
                work("Keep", vec![video(1, "01", "K S01E01.mkv")]),
                work(
                    "Gone",
                    vec![
                        video(1, "01", "G S01E01.mkv"),
                        subtitle(1, "01", "G S01E01.ass"),
                    ],
                ),
            ])),
            200,
        )
        .await
        .unwrap();
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![work(
                "Keep",
                vec![video(1, "01", "K S01E01.mkv")],
            )])),
            300,
        )
        .await
        .unwrap();

    let list = store.overview().await.unwrap();
    let gone = list.iter().find(|w| w.dir_name == "Gone").unwrap();
    assert!(gone.missing);
    assert!(gone.video.is_empty() && gone.subtitle.is_empty());
    assert_eq!(gone.subtitle_coverage, None);
    assert!(!gone.subtitle_check_needed);
    // Its recorded times are still there for the orders.
    assert_eq!(gone.first_seen_at, Some(200));
    assert_eq!(gone.video_added_at, Some(200));
    assert_eq!(gone.subtitle_added_at, Some(200));
    let keep = list.iter().find(|w| w.dir_name == "Keep").unwrap();
    assert!(!keep.missing);
    assert_eq!(keep.video, [range("01", "01")]);
}

#[tokio::test]
async fn the_list_takes_the_latest_known_time_over_seasons_and_none_when_all_are_unknown() {
    let store = store();
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![
                work("Old", vec![video(1, "01", "O S01E01.mkv")]),
                work(
                    "Mixed",
                    vec![
                        video(1, "01", "M S01E01.mkv"),
                        subtitle(1, "01", "M S01E01.ass"),
                    ],
                ),
            ]),
            100,
            &[],
        )
        .await
        .unwrap();
    // A later scan: a new episode of the second season and a new subtitle.
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![
                work("Old", vec![video(1, "01", "O S01E01.mkv")]),
                work(
                    "Mixed",
                    vec![
                        video(1, "01", "M S01E01.mkv"),
                        subtitle(1, "01", "M S01E01.ass"),
                        video(2, "01", "M S02E01.mkv"),
                    ],
                ),
            ])),
            200,
        )
        .await
        .unwrap();
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![
                work("Old", vec![video(1, "01", "O S01E01.mkv")]),
                work(
                    "Mixed",
                    vec![
                        video(1, "01", "M S01E01.mkv"),
                        subtitle(1, "01", "M S01E01.ass"),
                        video(2, "01", "M S02E01.mkv"),
                        video(1, "02", "M S01E02.mkv"),
                    ],
                ),
            ])),
            300,
        )
        .await
        .unwrap();

    let list = store.overview().await.unwrap();
    let old = list.iter().find(|w| w.dir_name == "Mixed").unwrap();
    // The newest video is in season 1, the latest season is 2: times span all seasons.
    assert_eq!(old.latest_season, Some(2));
    assert_eq!(old.video_added_at, Some(300));
    // The only subtitle is of unknown age.
    assert_eq!(old.subtitle_added_at, None);
    assert_eq!(old.first_seen_at, None);
    // Holdings are the latest season's alone: a video and no subtitle.
    assert_eq!(old.video, [range("01", "01")]);
    assert!(old.subtitle.is_empty());
    assert_eq!(old.subtitle_coverage, Some(SubtitleCoverage::None));

    let plain = list.iter().find(|w| w.dir_name == "Old").unwrap();
    assert_eq!(
        (plain.video_added_at, plain.subtitle_added_at),
        (None, None)
    );
}

#[tokio::test]
async fn a_work_without_a_season_folder_has_no_latest_season_and_no_holdings() {
    let store = store();
    store
        .add_folder(
            "/w".into(),
            scan(vec![WorkRead::Read(ScannedWork {
                dir_name: "Bare".into(),
                ..ScannedWork::default()
            })]),
            100,
            &[],
        )
        .await
        .unwrap();
    let list = store.overview().await.unwrap();
    assert_eq!(list[0].latest_season, None);
    assert!(list[0].video.is_empty());
    assert_eq!(list[0].subtitle_coverage, Some(SubtitleCoverage::None));
}

/// A plan over `store`'s folders as they are.
async fn plan_over(store: &LibraryStore) -> AutomaticPlan {
    AutomaticPlan::over(&store.folders().await.unwrap())
}

#[tokio::test]
async fn adding_a_folder_checked_against_folders_that_have_changed_is_refused() {
    let store = store();
    let (a, _) = store
        .add_folder("/a".into(), scan(vec![]), 100, &[])
        .await
        .unwrap();
    // The caller checked against no folders, but /a is registered now.
    assert!(matches!(
        store.add_folder("/b".into(), scan(vec![]), 100, &[]).await,
        Err(LibraryError::Changed)
    ));
    assert_eq!(store.folders().await.unwrap().len(), 1);
    store
        .add_folder("/b".into(), scan(vec![]), 100, &[a])
        .await
        .unwrap();
    assert_eq!(store.folders().await.unwrap().len(), 2);
}

#[tokio::test]
async fn an_automatic_folder_is_registered_with_its_first_reading_or_without_one() {
    let store = store();
    let mut plan = plan_over(&store).await;
    plan.add.push(NewAutomatic {
        path: "/collect".into(),
        scan: Some(scan(vec![work("A", vec![video(1, "01", "A S01E01.mkv")])])),
    });
    plan.add.push(NewAutomatic {
        path: "/archive".into(),
        scan: None,
    });
    let applied = store.sync_automatic(plan, 0, 100).await.unwrap();
    assert_eq!(applied.added, 2);

    let folders = store.folders().await.unwrap();
    assert!(folders.iter().all(|f| f.automatic));
    let collect = &folders[0];
    assert_eq!(collect.path, "/collect");
    // Read once: baselined, its work of unknown age.
    assert!(collect.baselined);
    let works = store.works(&collect.id).await.unwrap();
    assert_eq!(works.len(), 1);
    assert_eq!(works[0].first_seen_at, None);
    // Not read yet: the worker's first reading is its baseline.
    let archive = &folders[1];
    assert!(!archive.baselined && archive.checked_at.is_none());
}

#[tokio::test]
async fn a_folder_registered_by_hand_becomes_automatic_and_keeps_its_records() {
    let store = store();
    let (manual, _) = store
        .add_folder(
            "/downloads/Shows".into(),
            scan(vec![work("A", vec![video(1, "01", "A S01E01.mkv")])]),
            100,
            &[],
        )
        .await
        .unwrap();
    let before = store.works(&manual.id).await.unwrap();
    assert!(!manual.automatic);

    let mut plan = plan_over(&store).await;
    plan.keep
        .push((manual.id.clone(), "/downloads/Shows".into()));
    let applied = store.sync_automatic(plan, 0, 200).await.unwrap();
    assert_eq!((applied.converted, applied.added), (1, 0));

    let folders = store.folders().await.unwrap();
    assert_eq!(folders.len(), 1);
    assert!(folders[0].automatic);
    assert_eq!(folders[0].id, manual.id);
    assert_eq!(store.works(&manual.id).await.unwrap(), before);
}

#[tokio::test]
async fn an_automatic_folder_that_is_not_called_for_any_more_goes_with_its_works() {
    let store = store();
    let mut plan = plan_over(&store).await;
    plan.add.push(NewAutomatic {
        path: "/a".into(),
        scan: Some(scan(vec![work("A", vec![video(1, "01", "A S01E01.mkv")])])),
    });
    store.sync_automatic(plan, 0, 100).await.unwrap();
    let a = store.folders().await.unwrap().remove(0);

    let mut plan = plan_over(&store).await;
    plan.remove.push(a.id.clone());
    plan.add.push(NewAutomatic {
        path: "/c".into(),
        scan: None,
    });
    let applied = store.sync_automatic(plan, 0, 200).await.unwrap();
    assert_eq!(
        (applied.removed, applied.removed_works, applied.added),
        (1, 1, 1)
    );
    let folders = store.folders().await.unwrap();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].path, "/c");
    assert!(store.works(&a.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn an_automatic_folder_cannot_be_removed_by_hand_but_a_manual_one_can() {
    let store = store();
    let (manual, _) = store
        .add_folder("/m".into(), scan(vec![]), 100, &[])
        .await
        .unwrap();
    let mut plan = plan_over(&store).await;
    plan.add.push(NewAutomatic {
        path: "/auto".into(),
        scan: None,
    });
    store.sync_automatic(plan, 0, 100).await.unwrap();
    let auto = store.folders().await.unwrap().remove(1);

    assert!(matches!(
        store.remove_folder(&auto.id).await,
        Err(LibraryError::Automatic)
    ));
    assert_eq!(store.folders().await.unwrap().len(), 2);
    assert_eq!(store.remove_folder(&manual.id).await.unwrap(), Some(0));
}

#[tokio::test]
async fn a_plan_made_from_other_folders_or_other_settings_is_not_applied() {
    let store = store();
    let stale = plan_over(&store).await;
    store
        .add_folder("/m".into(), scan(vec![]), 100, &[])
        .await
        .unwrap();
    let mut plan = stale;
    plan.add.push(NewAutomatic {
        path: "/auto".into(),
        scan: None,
    });
    assert!(matches!(
        store.sync_automatic(plan, 0, 100).await,
        Err(LibraryError::Changed)
    ));
    assert_eq!(store.folders().await.unwrap().len(), 1);

    // The settings are at version 0 (none stored): a plan for version 3 is stale.
    let mut plan = plan_over(&store).await;
    plan.add.push(NewAutomatic {
        path: "/auto".into(),
        scan: None,
    });
    assert!(matches!(
        store.sync_automatic(plan, 3, 100).await,
        Err(LibraryError::Changed)
    ));
    assert_eq!(store.folders().await.unwrap().len(), 1);
}

#[tokio::test]
async fn recording_some_works_changes_only_them_and_leaves_the_folders_error_alone() {
    let store = store();
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![
                work("A", vec![video(1, "01", "A S01E01.mkv")]),
                work("B", vec![video(1, "01", "B S01E01.mkv")]),
                work("C", vec![video(1, "01", "C S01E01.mkv")]),
            ]),
            100,
            &[],
        )
        .await
        .unwrap();
    let before = store.works(&folder.id).await.unwrap();

    // A gets a file and B's folder is gone; C is not part of the reading and
    // is absent from it without being marked missing.
    let report = store
        .record_works(
            &folder.id,
            vec!["A".into(), "B".into(), "D".into()],
            scan(vec![work(
                "A",
                vec![
                    video(1, "01", "A S01E01.mkv"),
                    video(1, "02", "A S01E02.mkv"),
                ],
            )]),
            200,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(!report.baseline);
    assert_eq!(report.files_added, 1);
    let works = store.works(&folder.id).await.unwrap();
    let by_name = |n: &str| works.iter().find(|w| w.dir_name == n).unwrap();
    assert_eq!(
        by_name("A").files()["Season 01/A S01E02.mkv"].added_at,
        Some(200)
    );
    assert!(by_name("B").missing);
    assert!(!by_name("C").missing);
    assert_eq!(
        by_name("C"),
        before.iter().find(|w| w.dir_name == "C").unwrap()
    );
    // A work no one knew and that is not there stays unknown.
    assert!(works.iter().all(|w| w.dir_name != "D"));
    assert_eq!(
        store.folder(&folder.id).await.unwrap().unwrap().checked_at,
        Some(200)
    );

    // A new work is dated by the reading that found it, B returns with its ID.
    let b_id = by_name("B").id.clone();
    store
        .record_works(
            &folder.id,
            vec!["B".into(), "E".into()],
            scan(vec![
                work("B", vec![video(1, "01", "B S01E01.mkv")]),
                work("E", vec![video(1, "01", "E S01E01.mkv")]),
            ]),
            300,
        )
        .await
        .unwrap();
    let works = store.works(&folder.id).await.unwrap();
    let by_name = |n: &str| works.iter().find(|w| w.dir_name == n).unwrap();
    assert_eq!(by_name("B").id, b_id);
    assert!(!by_name("B").missing);
    assert_eq!(by_name("E").first_seen_at, Some(300));
}

#[tokio::test]
async fn reading_some_works_adds_to_the_folders_error_and_keeps_it_until_a_whole_read() {
    let store = store();
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![work("A", vec![video(1, "01", "A S01E01.mkv")])]),
            100,
            &[],
        )
        .await
        .unwrap();

    let report = store
        .record_works(
            &folder.id,
            vec!["A".into()],
            scan(vec![WorkRead::Unreadable {
                dir_name: "A".into(),
                reason: "읽을 권한이 없어요.".into(),
            }]),
            200,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(report.works_unreadable, 1);
    let error = store.folder(&folder.id).await.unwrap().unwrap().error;
    assert!(error.as_deref().unwrap().contains("`A`"), "{error:?}");

    // A clean reading of the same work does not clear what a whole read said.
    store
        .record_works(
            &folder.id,
            vec!["A".into()],
            scan(vec![work("A", vec![video(1, "01", "A S01E01.mkv")])]),
            300,
        )
        .await
        .unwrap();
    assert_eq!(
        store.folder(&folder.id).await.unwrap().unwrap().error,
        error
    );
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![work("A", vec![video(1, "01", "A S01E01.mkv")])])),
            400,
        )
        .await
        .unwrap();
    assert_eq!(store.folder(&folder.id).await.unwrap().unwrap().error, None);
}

#[tokio::test]
async fn the_watch_note_is_kept_until_it_is_cleared() {
    let store = store();
    let (folder, _) = store
        .add_folder("/w".into(), scan(vec![]), 100, &[])
        .await
        .unwrap();
    assert_eq!(folder.watch_note, None);
    store
        .set_watch_note(&folder.id, Some("폴더 3개를 지켜보지 못해요.".into()))
        .await
        .unwrap();
    store
        .record_scan(&folder.id, Ok(scan(vec![])), 200)
        .await
        .unwrap();
    let note = store.folder(&folder.id).await.unwrap().unwrap().watch_note;
    assert_eq!(note.as_deref(), Some("폴더 3개를 지켜보지 못해요."));
    store.set_watch_note(&folder.id, None).await.unwrap();
    assert_eq!(
        store.folder(&folder.id).await.unwrap().unwrap().watch_note,
        None
    );
}
