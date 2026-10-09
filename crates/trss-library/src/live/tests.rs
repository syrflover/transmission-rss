//! What the inotify alerts make of the watch folders: real temporary folders
//! and the kernel's own alerts, a context of the library alone (the worker's
//! lock and heartbeat are the real ones, on a temporary database), and no mock
//! of the watching itself. `tree` tests the watches of one folder; these tests
//! start the whole [`LiveWatch`].
//!
//! The clock is the fixture's manual one, so what time a file gets is exact; the
//! waits are real, so they poll the database with a generous limit instead of
//! sleeping for a guessed time. Debounce and retry are shortened so the tests
//! are quick, and the safety net keeps its hour (the clock is manual, so a test
//! moves the hour itself).

use std::{fs, future::Future, path::PathBuf};

use super::*;
use crate::{
    discovery::Reason,
    store::library::WatchFolder,
    watch::fixture::{touch, Fixture},
};

const DEBOUNCE: Duration = Duration::from_millis(150);

fn config() -> LiveConfig {
    LiveConfig {
        debounce: DEBOUNCE,
        retry: Duration::from_millis(40),
        ..LiveConfig::default()
    }
}

/// Waits until `check` holds, polling; a generous limit, not a pause.
async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    for _ in 0..600 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for: {what}");
}

impl Fixture {
    async fn has_file(&self, folder: &WatchFolder, work: &str, file: &str) -> bool {
        self.added_at(folder, work, file).await.is_some()
    }

    async fn has_unrecognized(&self, folder: &WatchFolder, work: &str, path: &str) -> bool {
        match self.find(folder, work).await {
            Some(work) => work.unrecognized.iter().any(|u| u.path == path),
            None => false,
        }
    }

    /// The readings made since the first `before` of them.
    fn readings_since(&self, before: usize) -> Vec<Reading> {
        self.ctx.live.readings().split_off(before)
    }

    /// A folder with two works, registered, watched, and read by the first cycle.
    async fn two_works(&self) -> (PathBuf, WatchFolder) {
        let root = self.folder("anime");
        touch(&root.join("A/Season 01/A S01E01.mkv"));
        touch(&root.join("B/Season 01/B S01E01.mkv"));
        let folder = self.register(&root).await;
        self.start_watching().await;
        self.advance(1000);
        self.scan_all().await;
        (root, folder)
    }
}

#[tokio::test]
async fn a_file_added_while_the_worker_runs_is_recorded_without_a_cycle_and_only_its_work_is_read()
{
    let fx = Fixture::with_live(config()).await;
    let (root, folder) = fx.two_works().await;
    let status = fx.ctx.live.status(&folder.id).unwrap();
    // The folder, A, A's season, B and B's season.
    assert_eq!(status.watches, 5);
    assert!(status.root_watched && status.unwatched_dirs == 0);
    let before = fx.ctx.live.readings().len();

    fx.advance(5000);
    let stamp = fx.now();
    touch(&root.join("A/Season 01/A S01E02.mkv"));

    eventually("the new episode is recorded", || async {
        fx.added_at(&folder, "A", "Season 01/A S01E02.mkv").await == Some(Some(stamp))
    })
    .await;
    // Nothing but A was read, and no whole folder: B was left alone.
    let read = fx.readings_since(before);
    assert!(!read.is_empty());
    assert!(
        read.iter()
            .all(|r| *r == Reading::Work(folder.id.clone(), "A".into())),
        "{read:?}"
    );
    // The work's other records are as they were.
    assert_eq!(fx.work(&folder, "B").await.files().len(), 1);
}

#[tokio::test]
async fn a_part_file_that_keeps_growing_is_not_read_again_and_its_rename_is_recorded() {
    let fx = Fixture::with_live(config()).await;
    let (root, folder) = fx.two_works().await;
    let season = root.join("A/Season 01");

    fx.advance(1000);
    fs::write(season.join("A S01E02.mkv.part"), "x").unwrap();
    eventually("the download in progress is counted", || async {
        fx.work(&folder, "A")
            .await
            .unrecognized
            .iter()
            .any(|u| u.path.ends_with(".part") && u.reason == Reason::Partial)
    })
    .await;
    // Let the reading that its creation caused finish.
    tokio::time::sleep(DEBOUNCE * 2).await;
    let (_, reads) = fx.ctx.live.read_counts();

    // Writing more (the download) tells the worker nothing.
    for _ in 0..5 {
        let mut part = fs::OpenOptions::new()
            .append(true)
            .open(season.join("A S01E02.mkv.part"))
            .unwrap();
        std::io::Write::write_all(&mut part, &[0u8; 4096]).unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
    tokio::time::sleep(DEBOUNCE * 3).await;
    assert_eq!(fx.ctx.live.read_counts().1, reads);

    // Done: the rename to the final name is an alert, and the episode is recorded.
    fx.advance(1000);
    let stamp = fx.now();
    fs::rename(
        season.join("A S01E02.mkv.part"),
        season.join("A S01E02.mkv"),
    )
    .unwrap();
    eventually("the finished episode is recorded", || async {
        fx.added_at(&folder, "A", "Season 01/A S01E02.mkv").await == Some(Some(stamp))
    })
    .await;
    assert!(fx.work(&folder, "A").await.unrecognized.is_empty());
}

#[tokio::test]
async fn a_new_work_with_a_season_and_a_file_made_together_is_recorded_and_watched() {
    let fx = Fixture::with_live(config()).await;
    let (root, folder) = fx.two_works().await;
    let watches = fx.ctx.live.status(&folder.id).unwrap().watches;

    fx.advance(1000);
    let stamp = fx.now();
    touch(&root.join("New/Season 01/New S01E01.mkv"));
    eventually("the new work, season and episode are recorded", || async {
        fx.added_at(&folder, "New", "Season 01/New S01E01.mkv")
            .await
            == Some(Some(stamp))
    })
    .await;
    let new = fx.work(&folder, "New").await;
    assert_eq!(new.seasons, [1]);
    assert_eq!(new.first_seen_at, Some(stamp));
    // The new work and its season are watched now.
    let status = fx.ctx.live.status(&folder.id).unwrap();
    assert_eq!(status.watches, watches + 2);

    // So the next file is an alert too, without any cycle.
    fx.advance(1000);
    let later = fx.now();
    touch(&root.join("New/Season 01/New S01E02.mkv"));
    eventually("the next episode is recorded", || async {
        fx.added_at(&folder, "New", "Season 01/New S01E02.mkv")
            .await
            == Some(Some(later))
    })
    .await;
    // A new season of an existing work is watched the same way.
    fx.advance(1000);
    let season_two = fx.now();
    touch(&root.join("A/Season 02/A S02E01.mkv"));
    eventually("the new season's episode is recorded", || async {
        fx.added_at(&folder, "A", "Season 02/A S02E01.mkv").await == Some(Some(season_two))
    })
    .await;
    fx.advance(1000);
    let after = fx.now();
    touch(&root.join("A/Season 02/A S02E02.mkv"));
    eventually(
        "the second episode of the new season is recorded",
        || async { fx.added_at(&folder, "A", "Season 02/A S02E02.mkv").await == Some(Some(after)) },
    )
    .await;
}

#[tokio::test]
async fn a_work_folder_that_is_removed_or_moved_out_is_missing_and_loses_its_watches() {
    let fx = Fixture::with_live(config()).await;
    let (root, folder) = fx.two_works().await;
    touch(&root.join("C/Season 01/C S01E01.mkv"));
    eventually("C is recorded", || async {
        fx.find(&folder, "C").await.is_some()
    })
    .await;
    assert_eq!(fx.ctx.live.status(&folder.id).unwrap().watches, 7);

    fs::remove_dir_all(root.join("A")).unwrap();
    let outside = fx.folder("elsewhere");
    fs::rename(root.join("C"), outside.join("C")).unwrap();
    eventually("A and C are missing", || async {
        fx.work(&folder, "A").await.missing && fx.work(&folder, "C").await.missing
    })
    .await;
    assert!(!fx.work(&folder, "B").await.missing);
    // Only the folder and B (with its season) are watched.
    eventually("the watches are released", || async {
        fx.ctx.live.status(&folder.id).unwrap().watches == 3
    })
    .await;

    // What happens to the moved folder is not heard under its old name...
    let before = fx.ctx.live.readings().len();
    touch(&outside.join("C/Season 01/C S01E02.mkv"));
    tokio::time::sleep(DEBOUNCE * 3).await;
    assert!(fx.readings_since(before).is_empty());
    // ...and a folder that comes back is a work again, with its ID.
    let id = fx.work(&folder, "A").await.id;
    fx.advance(1000);
    touch(&root.join("A/Season 01/A S01E01.mkv"));
    eventually("A is back", || async {
        let work = fx.work(&folder, "A").await;
        !work.missing && work.id == id
    })
    .await;
}

#[tokio::test]
async fn a_change_made_while_the_worker_was_off_is_found_by_the_first_cycle_after_the_start() {
    let fx = Fixture::with_live(config()).await;
    let root = fx.folder("anime");
    touch(&root.join("A/Season 01/A S01E01.mkv"));
    touch(&root.join("A/Season 01/A S01E02.mkv"));
    touch(&root.join("B/Season 01/B S01E01.mkv"));
    let folder = fx.register(&root).await;

    // Nothing is watching while these happen.
    fx.advance(1000);
    touch(&root.join("A/Season 01/A S01E03.mkv"));
    fs::remove_file(root.join("A/Season 01/A S01E01.mkv")).unwrap();
    fs::remove_dir_all(root.join("B")).unwrap();

    fx.start_watching().await;
    fx.advance(1000);
    let stamp = fx.now();
    fx.scan_all().await;

    let a = fx.work(&folder, "A").await;
    let files = a.files();
    assert!(!files.contains_key("Season 01/A S01E01.mkv"));
    assert_eq!(files["Season 01/A S01E03.mkv"].added_at, Some(stamp));
    assert!(fx.work(&folder, "B").await.missing);
    // It was the whole folder, once.
    assert_eq!(
        fx.ctx.live.readings(),
        vec![Reading::Folder(folder.id.clone())]
    );
}

#[tokio::test]
async fn a_queue_overflow_reads_the_whole_folder_and_finds_what_no_alert_told() {
    let fx = Fixture::with_live(config()).await;
    let (root, folder) = fx.two_works().await;
    let season = root.join("A/Season 01");

    // A folder inside a season folder has no watch of its own.
    fs::create_dir_all(season.join("Batch")).unwrap();
    tokio::time::sleep(DEBOUNCE * 4).await;
    fx.advance(1000);
    touch(&season.join("Batch/ep 02.mkv"));
    tokio::time::sleep(DEBOUNCE * 4).await;
    assert!(
        !fx.has_unrecognized(&folder, "A", "Season 01/Batch/ep 02.mkv")
            .await
    );

    // The kernel says it dropped alerts: the whole folder is read.
    let before = fx.ctx.live.readings().len();
    fx.ctx.live.simulate_overflow(&folder.id);
    eventually("the folder is read whole and the file is found", || async {
        fx.has_unrecognized(&folder, "A", "Season 01/Batch/ep 02.mkv")
            .await
    })
    .await;
    assert!(fx
        .readings_since(before)
        .contains(&Reading::Folder(folder.id.clone())));
}

#[tokio::test]
async fn directories_without_a_watch_are_read_by_every_cycle_and_the_row_says_why() {
    // Room for the folder and two works with a season each, not for the third.
    let fx = Fixture::with_live(LiveConfig {
        max_watches: Some(5),
        ..config()
    })
    .await;
    let root = fx.folder("anime");
    for work in ["A", "B", "C"] {
        touch(&root.join(format!("{work}/Season 01/{work} S01E01.mkv")));
    }
    let folder = fx.register(&root).await;
    fx.start_watching().await;
    fx.advance(1000);
    fx.scan_all().await;

    let status = fx.ctx.live.status(&folder.id).unwrap();
    assert_eq!(status.watches, 5);
    assert_eq!(status.unwatched_dirs, 1);
    assert_eq!(
        status.unwatched_works.iter().collect::<Vec<_>>(),
        ["C"],
        "the third work is the one left out"
    );
    // The row says how many and why: the sentence is on the folder's row.
    let note = status.note.clone().expect("a note");
    assert!(note.contains("폴더 1개"), "{note}");
    assert!(note.contains("fs.inotify.max_user_watches"), "{note}");
    eventually("the row carries the note", || async {
        fx.stored(&folder).await.watch_note.as_deref() == Some(note.as_str())
    })
    .await;
    // The cycle reads what has no watch, and only that.
    assert_eq!(
        fx.ctx.live.poll_for(&folder.id, fx.now()),
        Poll::Works(vec!["C".to_owned()])
    );

    // The watched works are heard...
    fx.advance(1000);
    let stamp = fx.now();
    touch(&root.join("A/Season 01/A S01E02.mkv"));
    eventually("A's episode is recorded by its alert", || async {
        fx.added_at(&folder, "A", "Season 01/A S01E02.mkv").await == Some(Some(stamp))
    })
    .await;

    // ...and the one without a watch is read by the next cycle, alone.
    fx.advance(1000);
    let cycle = fx.now();
    touch(&root.join("C/Season 01/C S01E02.mkv"));
    tokio::time::sleep(DEBOUNCE * 3).await;
    assert!(!fx.has_file(&folder, "C", "Season 01/C S01E02.mkv").await);
    let before = fx.ctx.live.readings().len();
    fx.scan_all().await;
    assert_eq!(
        fx.added_at(&folder, "C", "Season 01/C S01E02.mkv").await,
        Some(Some(cycle))
    );
    assert_eq!(
        fx.readings_since(before),
        vec![Reading::Work(folder.id.clone(), "C".into())]
    );
}

#[tokio::test]
async fn a_quiet_hour_reads_no_folder_but_the_safety_net() {
    let fx = Fixture::with_live(config()).await;
    let (_, folder) = fx.two_works().await;
    let (folders_read, works_read) = fx.ctx.live.read_counts();
    assert_eq!(folders_read, 1, "the first cycle after the start");

    // Eleven cycles over 55 minutes: every directory is watched, nothing changes.
    for _ in 0..11 {
        fx.advance(5 * 60 * 1000);
        fx.scan_all().await;
    }
    assert_eq!(fx.ctx.live.read_counts(), (1, works_read));

    // The twelfth reaches the hour since the last whole read: one read, once.
    fx.advance(5 * 60 * 1000);
    fx.scan_all().await;
    assert_eq!(fx.ctx.live.read_counts(), (2, works_read));
    fx.advance(5 * 60 * 1000);
    fx.scan_all().await;
    assert_eq!(fx.ctx.live.read_counts(), (2, works_read));
    assert_eq!(
        fx.ctx.live.readings(),
        vec![Reading::Folder(folder.id.clone()); 2],
        "the first cycle and the safety net, nothing else"
    );
}

#[tokio::test]
async fn a_folder_registered_while_the_worker_runs_is_watched_and_caught_up_and_an_unregistered_one_loses_its_watches(
) {
    let fx = Fixture::with_live(config()).await;
    let first = fx.folder("first");
    touch(&first.join("A/Season 01/A S01E01.mkv"));
    let first_folder = fx.register(&first).await;
    fx.start_watching().await;
    assert!(fx.ctx.live.status(&first_folder.id).is_some());

    // Registered after the start (the web reads it once); a file arrives right after.
    let second = fx.folder("second");
    touch(&second.join("S/Season 01/S S01E01.mkv"));
    let second_folder = fx.register(&second).await;
    fx.advance(1000);
    let stamp = fx.now();
    touch(&second.join("S/Season 01/S S01E02.mkv"));
    fx.ctx.live.sync_folders().await;
    assert!(fx.ctx.live.status(&second_folder.id).is_some());
    eventually("what came after the web's reading is recorded", || async {
        fx.added_at(&second_folder, "S", "Season 01/S S01E02.mkv")
            .await
            == Some(Some(stamp))
    })
    .await;

    // Unregistered: the watches go.
    fx.ctx
        .library
        .remove_folder(&second_folder.id, fx.now())
        .await
        .unwrap();
    fx.ctx.live.sync_folders().await;
    assert!(fx.ctx.live.status(&second_folder.id).is_none());
    assert!(fx.ctx.live.status(&first_folder.id).is_some());

    // Stopped: no folder is watched, and every cycle reads every folder again.
    fx.ctx.live.stop();
    assert!(fx.ctx.live.status(&first_folder.id).is_none());
    assert_eq!(
        fx.ctx.live.poll_for(&first_folder.id, fx.now()),
        Poll::Folder
    );
}
