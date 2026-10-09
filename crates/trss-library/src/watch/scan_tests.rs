//! The reading of the watch folders that the worker's cycle and the `다시 확인`
//! command make: which directories a scan lists again, how the collect and
//! archive folders of the settings become watch folders, what a cycle reads
//! when the worker is not watching, and a folder that cannot be read.
//! (`watch_rescan` tests its command; `live` tests what the alerts ask for.)

use std::{fs, os::unix::fs::PermissionsExt};

use super::fixture::*;
use super::*;
use crate::store::library::{EpisodeRange, SubtitleCoverage};

#[tokio::test]
async fn the_periodic_scan_skips_unchanged_directories_and_a_full_scan_reads_everything() {
    let fx = Fixture::new().await;
    let root = fx.folder("anime");
    lycoris(&root);
    let folder = fx.register(&root).await;
    age_dirs(&root);
    let season = root.join("Lycoris Recoil/Season 01");
    let work = || fx.work(&folder, "Lycoris Recoil");

    // The first scan after the worker starts reads everything and remembers it.
    fx.advance(1000);
    fx.scan_all().await;
    assert_eq!(work().await.files().len(), 4);

    // A change that leaves the season folder's modification time as it was (a
    // coarse clock) is not seen by the periodic scan...
    touch(&season.join("Lycoris Recoil S01E03.mkv"));
    age_dirs(&season);
    fx.advance(1000);
    fx.scan_all().await;
    assert!(!work()
        .await
        .files()
        .contains_key("Season 01/Lycoris Recoil S01E03.mkv"));

    // ...but a full scan (`다시 확인`) reads every directory, and the file gets
    // that time.
    fx.advance(1000);
    let scanned = scan_folder(&fx.ctx, &folder, fx.now(), ScanMode::Full)
        .await
        .unwrap();
    assert!(matches!(scanned, Scanned::Read(_)), "{scanned:?}");
    assert_eq!(
        work().await.files()["Season 01/Lycoris Recoil S01E03.mkv"].added_at,
        Some(fx.now())
    );

    // An ordinary change (the folder's time moves) is found by the next cycle,
    // while the folders around it are not listed again.
    touch(&season.join("Lycoris Recoil S01E04.mkv"));
    fx.advance(1000);
    fx.scan_all().await;
    assert_eq!(
        work().await.files()["Season 01/Lycoris Recoil S01E04.mkv"].added_at,
        Some(fx.now())
    );
    // The time of a file's modification is not an added time.
    age(&season.join("Lycoris Recoil S01E04.mkv"));
    fx.advance(1000);
    fx.scan_all().await;
    assert_eq!(
        work().await.files()["Season 01/Lycoris Recoil S01E04.mkv"].added_at,
        Some(fx.now() - 1000)
    );
}

#[tokio::test]
async fn a_cycle_registers_the_folders_of_settings_saved_before_they_were_watched() {
    let fx = Fixture::new().await;
    let (collect, archive) = (fx.folder("Shows (current)"), fx.folder("Shows"));
    touch(&collect.join("A/Season 01/A S01E01.mkv"));
    touch(&archive.join("B/Season 01/B S01E01.mkv"));
    // As a database from before the folders were watched has them: set, not watched.
    fx.ctx
        .settings
        .put_collection(0, text(&collect), Some(text(&archive)))
        .await
        .unwrap();
    // One of them was registered by hand.
    let by_hand = fx.register(&archive).await;
    let in_b = fx.work(&by_hand, "B").await;
    assert_eq!(fx.ctx.library.folders().await.unwrap().len(), 1);

    fx.advance(1000);
    fx.scan_all().await;

    let folders = fx.ctx.library.folders().await.unwrap();
    assert_eq!(folders.len(), 2);
    assert!(folders.iter().all(|f| f.automatic));
    let adopted = fx.folder_at(&archive).await.unwrap();
    assert_eq!(adopted.id, by_hand.id);
    assert_eq!(fx.work(&adopted, "B").await, in_b);
    // The new one was read in the same cycle, as its baseline.
    let fc = fx.folder_at(&collect).await.unwrap();
    assert!(fc.baselined);
    assert_eq!(fx.work(&fc, "A").await.first_seen_at, None);

    // The next cycle changes none of that.
    fx.advance(1000);
    fx.scan_all().await;
    assert_eq!(fx.ctx.library.folders().await.unwrap().len(), 2);
    assert_eq!(fx.folder_at(&collect).await.unwrap().id, fc.id);
}

#[tokio::test]
async fn the_settings_folders_are_registered_only_when_set_and_only_once() {
    let fx = Fixture::new().await;
    // No collection settings: nothing to register.
    let by_hand = fx.folder("by-hand");
    touch(&by_hand.join("A/Season 01/A S01E01.mkv"));
    fx.register(&by_hand).await;
    sync_automatic(&fx.ctx, fx.now()).await;
    let folders = fx.ctx.library.folders().await.unwrap();
    assert_eq!(folders.len(), 1);
    assert!(!folders[0].automatic);

    // Settings whose folders are registered already as they should be.
    let collect = fx.folder("collect");
    fx.ctx
        .settings
        .put_collection(0, text(&collect), None)
        .await
        .unwrap();
    sync_automatic(&fx.ctx, fx.now()).await;
    let after_first = fx.ctx.library.folders().await.unwrap();
    assert_eq!(after_first.len(), 2);
    sync_automatic(&fx.ctx, fx.now()).await;
    assert_eq!(fx.ctx.library.folders().await.unwrap(), after_first);
}

#[tokio::test]
async fn without_watching_every_cycle_reads_every_folder_as_before() {
    let fx = Fixture::new().await;
    let root = fx.folder("anime");
    touch(&root.join("A/Season 01/A S01E01.mkv"));
    let folder = fx.register(&root).await;
    for _ in 0..3 {
        fx.advance(1000);
        fx.scan_all().await;
    }
    assert_eq!(fx.ctx.live.read_counts(), (3, 0));
    assert!(fx.ctx.live.status(&folder.id).is_none());
}

#[tokio::test]
async fn an_unreadable_watch_folder_records_its_reason_and_the_others_are_still_read() {
    let fx = Fixture::new().await;
    let (a, b) = (fx.folder("a"), fx.folder("b"));
    touch(&a.join("Show A/Season 01/Show A S01E01.mkv"));
    touch(&b.join("Show B/Season 01/Show B S01E01.mkv"));
    let fa = fx.register(&a).await;
    let fb = fx.register(&b).await;
    let before = fx.works(&fa).await;

    fs::set_permissions(&a, fs::Permissions::from_mode(0o000)).unwrap();
    touch(&b.join("Show B/Season 01/Show B S01E02.mkv"));
    fx.advance(1000);
    fx.scan_all().await;
    fs::set_permissions(&a, fs::Permissions::from_mode(0o755)).unwrap();

    let unreadable = fx.stored(&fa).await;
    assert!(unreadable.error.unwrap().contains("권한"));
    assert_eq!(unreadable.checked_at, Some(fx.now()));
    // The earlier records of the folder that could not be read are as they were.
    assert_eq!(fx.works(&fa).await, before);
    // The other folder was read.
    assert_eq!(fx.stored(&fb).await.error, None);
    let b_work = fx.work(&fb, "Show B").await;
    assert!(b_work.files().contains_key("Season 01/Show B S01E02.mkv"));

    // Readable again: the error is gone.
    fx.advance(1000);
    fx.scan_all().await;
    assert_eq!(fx.stored(&fa).await.error, None);
}

/// About 520 works and 10 000 files: one full reading and recording, an unchanged
/// one, and one with a single new file. The times are printed (run with
/// `--nocapture`) and not asserted: what is checked is that the counts are right.
#[tokio::test]
async fn a_library_of_520_works_and_10_000_files_is_read_and_recorded() {
    const WORKS: usize = 520;
    let fx = Fixture::new().await;
    let root = fx.folder("big");
    let mut files = 0;
    let build = Instant::now();
    for w in 0..WORKS {
        let name = format!("Work {w:03}");
        for e in 1..=12 {
            touch(&root.join(format!("{name}/Season 01/{name} S01E{e:02}.mkv")));
            files += 1;
        }
        for e in 1..=6 {
            touch(&root.join(format!("{name}/Season 01/{name} S01E{e:02}.ko.ass")));
            files += 1;
        }
        touch(&root.join(format!("{name}/Season 01/folder.jpg")));
        touch(&root.join(format!("{name}/Season 01/{name} S01E13.mkv.part")));
        files += 2;
    }
    eprintln!(
        "built {WORKS} works, {files} files in {:?}",
        build.elapsed()
    );

    // As a library that has been there a while: the periodic scan does not
    // skip directories modified moments ago.
    age_dirs(&root);

    let read = Instant::now();
    let full = discovery::scan_incremental(&root, None);
    let read_time = read.elapsed();
    let dirs = full.stats.dirs_read;
    assert_eq!(full.result.unwrap().works.len(), WORKS);
    assert_eq!(dirs, 1 + 2 * WORKS);
    let again = Instant::now();
    let unchanged = discovery::scan_incremental(&root, Some(full.cache));
    let unchanged_time = again.elapsed();
    assert_eq!(unchanged.stats.dirs_read, 0);
    assert_eq!(unchanged.stats.dirs_reused, dirs);

    let first = Instant::now();
    let folder = fx.register(&root).await;
    let first_time = first.elapsed();
    assert_eq!(fx.works(&folder).await.len(), WORKS);

    // The first cycle after the worker starts reads everything; the next finds
    // nothing changed.
    fx.advance(1000);
    let cold = Instant::now();
    fx.scan_all().await;
    let cold_time = cold.elapsed();
    fx.advance(1000);
    let quiet = Instant::now();
    fx.scan_all().await;
    let quiet_time = quiet.elapsed();

    touch(&root.join("Work 007/Season 01/Work 007 S01E14.mkv"));
    fx.advance(1000);
    let second = Instant::now();
    fx.scan_all().await;
    let second_time = second.elapsed();
    let work = fx.work(&folder, "Work 007").await;
    assert_eq!(work.episodes.len(), 13);
    assert_eq!(
        work.files()["Season 01/Work 007 S01E14.mkv"].added_at,
        Some(fx.now())
    );

    // What the library list shows of it.
    let list = Instant::now();
    let overview = fx.ctx.library.overview().await.unwrap();
    let list_time = list.elapsed();
    assert_eq!(overview.len(), WORKS);
    let seven = overview.iter().find(|w| w.dir_name == "Work 007").unwrap();
    let range = |first: &str, last: &str| EpisodeRange {
        first: first.to_owned(),
        last: last.to_owned(),
    };
    assert_eq!(seven.video, [range("01", "12"), range("14", "14")]);
    assert_eq!(seven.subtitle, [range("01", "06")]);
    assert_eq!(seven.subtitle_coverage, Some(SubtitleCoverage::Some));

    eprintln!("library list ({} works): {list_time:?}", overview.len());
    eprintln!(
        "scan only ({dirs} directories): all listed {read_time:?}, none changed {unchanged_time:?}; \
         first registration (scan and record): {first_time:?}; cycles (scan and record): first \
         after start {cold_time:?}, nothing changed {quiet_time:?}, one new file {second_time:?}"
    );
}
