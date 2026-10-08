//! The videos a person is asked about (`checks.rs`).

use std::collections::BTreeSet;

use crate::{
    discovery::{ScannedWork, SeenFile, Unrecognized, WorkRead},
    store::library::*,
};

const ASKED: &str = "Season 01/[Group] Show - 03 (1080p).mkv";

fn store() -> LibraryStore {
    LibraryStore::new(Db::open_blocking(":memory:").unwrap())
}

fn identity(size: u64) -> SeenFile {
    SeenFile {
        size,
        mtime_ns: 1_700_000_000_123_456_789,
    }
}

/// Work `name` with an unrecognized video at [`ASKED`] as `check` says, a
/// subtitle the name gives no episode of, and a video outside a season.
fn work(name: &str, check: Option<Option<SeenFile>>) -> WorkRead {
    let mut unrecognized = vec![
        Unrecognized {
            path: "Season 01/extra.ass".into(),
            reason: Reason::NoEpisode,
            check: None,
        },
        Unrecognized {
            path: "Extras/PV.mkv".into(),
            reason: Reason::OutsideSeason,
            check: None,
        },
    ];
    if let Some(check) = check {
        unrecognized.push(Unrecognized {
            path: ASKED.into(),
            reason: Reason::NoEpisode,
            check,
        });
    }
    WorkRead::Read(ScannedWork {
        dir_name: name.into(),
        seasons: BTreeSet::from([1]),
        files: Vec::new(),
        unrecognized,
    })
}

fn scan(works: Vec<WorkRead>) -> Scan {
    Scan { works }
}

async fn asked(store: &LibraryStore) -> Vec<(String, String)> {
    store
        .video_checks()
        .await
        .unwrap()
        .into_iter()
        .map(|c| (c.work_name, c.path))
        .collect()
}

fn asked_of(name: &str) -> Vec<(String, String)> {
    vec![(name.to_owned(), ASKED.to_owned())]
}

#[tokio::test]
async fn a_video_is_asked_about_until_checked_and_again_once_another_file_is_there() {
    let store = store();
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![work("A", Some(Some(identity(5))))]),
            100,
            &[],
        )
        .await
        .unwrap();
    let rescan = |check| {
        let (store, folder) = (store.clone(), folder.id.clone());
        async move {
            store
                .record_scan(&folder, Ok(scan(vec![work("A", check)])), 200)
                .await
                .unwrap()
                .unwrap();
        }
    };
    let checks = store.video_checks().await.unwrap();
    assert_eq!(
        checks.len(),
        1,
        "only the video in a season folder: {checks:?}"
    );
    let work_id = checks[0].work_id.clone();
    assert_eq!(
        checks[0],
        VideoCheck {
            work_id: work_id.clone(),
            work_name: "A".into(),
            season: 1,
            path: ASKED.into(),
            reason: Reason::NoEpisode,
            identity: identity(5),
        }
    );

    // What the screen saw must be what is there, and only that video is checked.
    let check = |path: &'static str, seen| store.check_video(&work_id, path, seen, 300);
    assert!(matches!(
        check(ASKED, identity(6)).await,
        Err(CheckError::Changed)
    ));
    assert!(matches!(
        check("Season 01/extra.ass", identity(5)).await,
        Err(CheckError::NoFile)
    ));
    assert!(matches!(
        check("Season 01/gone.mkv", identity(5)).await,
        Err(CheckError::NoFile)
    ));
    assert_eq!(asked(&store).await, asked_of("A"));

    check(ASKED, identity(5)).await.unwrap();
    assert_eq!(asked(&store).await, []);
    let unrecognized = |works: Vec<WorkRecord>| -> Vec<(String, bool)> {
        works[0]
            .unrecognized
            .iter()
            .map(|u| (u.path.clone(), u.checked))
            .collect()
    };
    assert_eq!(
        unrecognized(store.works(&folder.id).await.unwrap()),
        [
            ("Extras/PV.mkv".to_owned(), false),
            (ASKED.to_owned(), true),
            ("Season 01/extra.ass".to_owned(), false),
        ]
    );
    let detail = store.work_detail(&work_id).await.unwrap().unwrap();
    assert!(detail
        .unrecognized
        .iter()
        .any(|u| u.path == ASKED && u.checked));

    // The same video found again stays checked, and so does one whose size and
    // time could not be read this time.
    rescan(Some(Some(identity(5)))).await;
    assert_eq!(asked(&store).await, []);
    rescan(Some(None)).await;
    assert_eq!(asked(&store).await, []);
    rescan(Some(Some(identity(5)))).await;
    assert_eq!(asked(&store).await, []);

    // Another video at the path is asked about, and the mark is gone with the
    // one it was for.
    rescan(Some(Some(identity(7)))).await;
    assert_eq!(asked(&store).await, asked_of("A"));
    rescan(Some(Some(identity(5)))).await;
    assert_eq!(asked(&store).await, asked_of("A"));

    // Checked, then renamed or moved away: no question, and the same video
    // put back is asked about again.
    check(ASKED, identity(5)).await.unwrap();
    rescan(None).await;
    assert_eq!(asked(&store).await, []);
    rescan(Some(Some(identity(5)))).await;
    assert_eq!(asked(&store).await, asked_of("A"));
}

#[tokio::test]
async fn a_work_whose_folder_is_gone_or_unregistered_is_not_asked_about() {
    let store = store();
    let (folder, _) = store
        .add_folder(
            "/w".into(),
            scan(vec![
                work("A", Some(Some(identity(5)))),
                work("B", Some(Some(identity(5)))),
            ]),
            100,
            &[],
        )
        .await
        .unwrap();
    assert_eq!(asked(&store).await.len(), 2);
    store
        .record_scan(
            &folder.id,
            Ok(scan(vec![work("B", Some(Some(identity(5))))])),
            200,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(asked(&store).await, asked_of("B"));
    store.remove_folder(&folder.id, 300).await.unwrap();
    assert_eq!(asked(&store).await, []);
}

#[tokio::test]
async fn a_merge_keeps_the_moved_works_marks() {
    let store = store();
    let (from, _) = store
        .add_folder(
            "/from".into(),
            scan(vec![work("A", Some(Some(identity(5))))]),
            100,
            &[],
        )
        .await
        .unwrap();
    let (to, _) = store
        .add_folder(
            "/to".into(),
            scan(vec![work("A", None)]),
            100,
            std::slice::from_ref(&from),
        )
        .await
        .unwrap();
    let moved = store.video_checks().await.unwrap().remove(0).work_id;
    store
        .check_video(&moved, ASKED, identity(5), 150)
        .await
        .unwrap();
    assert_eq!(
        store.follow_move(&from.id, &to.id, "A").await.unwrap(),
        Followed::Merged
    );
    assert_eq!(asked(&store).await, []);
    // The scan of the kept work finds the moved video as it was.
    store
        .record_scan(
            &to.id,
            Ok(scan(vec![work("A", Some(Some(identity(5))))])),
            200,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(asked(&store).await, []);
}
