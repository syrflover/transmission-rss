//! The `rule_archive` command, which brings an archived work folder back for a
//! rule's start or resume ([`run`], ADR 0015: a rule is tested in the crate
//! that owns it): when the rule is turned on, what a refused move leaves, and
//! what the library keeps of the work. The move itself is in
//! `work_folder/tests.rs`; archiving a work is `work_archive`'s. Every case runs
//! through [`World::archive_of`] against the fake Transmission and real
//! temporary folders.

use std::{fs, path::Path, time::Duration};

use trss_core::commands::CommandState;

use trss_library::{
    discovery,
    store::library::{WatchFolder, WorkRecord},
};

use super::*;
use crate::{
    store::channels::RuleInput,
    test_world::{entry, files, write, World},
};

fn rule(phrase: &str, directory: &str) -> RuleInput {
    RuleInput {
        r#match: Some(phrase.to_owned()),
        directory: directory.to_owned(),
        ..Default::default()
    }
}

/// A world with the archive folder set and a move that waits for Transmission
/// for moments only.
async fn world(rules: Vec<RuleInput>) -> World {
    let mut s = World::with_rules(rules).await.with_archive_folder().await;
    s.ctx.moves = MovePolicy {
        poll: Duration::from_millis(10),
        timeout: Duration::from_secs(5),
    };
    s
}

#[tokio::test]
async fn a_start_that_failed_on_an_overlap_moves_the_work_folder_in_once_the_overlap_is_cleared() {
    let s = world(vec![RuleInput {
        state: RuleState::Paused,
        ..rule("Clevatess S03", "Clevatess/Season 03")
    }])
    .await;
    let rule = s.rule_of(0).await;
    // The same file on both sides: the move refuses before it moves anything.
    write(
        &s.media.join("Clevatess/Season 02/Clevatess S02E02.mkv"),
        "new",
    );
    write(
        &s.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv"),
        "old",
    );
    write(
        &s.archive.join("Clevatess/Season 01/Clevatess S01E01.mkv"),
        "old",
    );
    let (collect_before, archive_before) = (files(&s.media), files(&s.archive));

    let failed = s.archive_of(&rule, Direction::Start).await;

    assert_eq!(failed.state, CommandState::Failed, "{failed:?}");
    assert_eq!(s.stored_rule(&rule).await.state, RuleState::Paused);
    assert_eq!(files(&s.media), collect_before);
    assert_eq!(files(&s.archive), archive_before);
    assert!(s.tr.calls_of("torrent-set-location").is_empty());

    // Once the overlap is cleared, the next start moves the folder in and turns
    // the rule on. A rule that never collected starts again, so the time it
    // waited does not make the items that came meanwhile past ones.
    fs::remove_file(s.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv")).unwrap();
    s.advance(1);
    let moved = s.archive_of(&rule, Direction::Start).await;
    assert_eq!(moved.state, CommandState::Done, "{moved:?}");
    assert_eq!(moved.outcome.result, MOVED);
    let stored = s.stored_rule(&rule).await;
    assert_eq!(stored.state, RuleState::Active);
    assert_eq!(stored.resumed_at, None);
    assert!(!s.archive.join("Clevatess").exists());
    assert_eq!(
        files(&s.media),
        [
            "Clevatess/Season 01/Clevatess S01E01.mkv",
            "Clevatess/Season 02/Clevatess S02E02.mkv",
        ]
    );
}

#[tokio::test]
async fn a_resume_moves_an_archived_work_in_and_turns_the_rule_on_from_when_it_was_pressed() {
    let paused = |phrase: &str, directory: &str| RuleInput {
        state: RuleState::Paused,
        ..rule(phrase, directory)
    };
    let s = world(vec![
        paused("Solo", "Solo/Season 01"),
        paused("Clevatess", "Clevatess/Season 02"),
    ])
    .await;
    // Solo is in the archive folder only, with a torrent; Clevatess is in both
    // folders.
    s.seeding_in(1, "Solo S01E01.mkv", &s.archive.join("Solo/Season 01"));
    write(
        &s.archive.join("Clevatess/Season 01/Clevatess S01E01.mkv"),
        "old",
    );
    write(
        &s.media.join("Clevatess/Season 02/Clevatess S02E01.mkv"),
        "now",
    );

    for n in 0..2 {
        let rule = s.rule_of(n).await;
        s.advance(1_000);
        let command = s.start_archive(&rule, Direction::Resume).await;
        // The command ran some time after the switch was pressed.
        s.advance(500);
        let moved = s.run_archive(&command).await.unwrap();

        assert_eq!(moved.state, CommandState::Done, "{moved:?}");
        assert_eq!(moved.outcome.result, MOVED);
        let stored = s.stored_rule(&rule).await;
        assert_eq!(stored.state, RuleState::Active);
        assert_eq!(stored.resumed_at, Some(command.created_at));
    }

    assert!(!s.archive.join("Solo").exists());
    assert!(!s.archive.join("Clevatess").exists());
    assert_eq!(
        files(&s.media),
        [
            "Clevatess/Season 01/Clevatess S01E01.mkv",
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Solo/Season 01/Solo S01E01.mkv",
        ]
    );
    assert_eq!(
        s.tr.torrent(&crate::test_world::show_hash(1)).download_dir,
        s.media.join("Solo/Season 01").to_str().unwrap()
    );
}

// --- the library follows the move -------------------------------------------------

/// Both folders registered as watch folders, as saving the settings leaves
/// them, each read once.
async fn watching(s: &World) -> (WatchFolder, WatchFolder) {
    let mut folders = Vec::new();
    for path in [&s.media, &s.archive] {
        let registered = s.ctx.library.folders().await.unwrap();
        let scan = discovery::scan(path).unwrap();
        let (folder, _) = s
            .ctx
            .library
            .add_folder(
                path.to_str().unwrap().to_owned(),
                scan,
                s.now(),
                &registered,
            )
            .await
            .unwrap();
        folders.push(folder);
    }
    let archive = folders.pop().unwrap();
    (folders.pop().unwrap(), archive)
}

/// The worker's next reading of the watch folders.
async fn read_again(s: &World, folders: &[&WatchFolder]) {
    for folder in folders {
        let scan = discovery::scan(Path::new(&folder.path)).unwrap();
        s.advance(1_000);
        s.ctx
            .library
            .record_scan(&folder.id, Ok(scan), s.now())
            .await
            .unwrap();
    }
}

async fn work_in(s: &World, folder: &WatchFolder, name: &str) -> Option<WorkRecord> {
    s.ctx
        .library
        .works(&folder.id)
        .await
        .unwrap()
        .into_iter()
        .find(|w| w.dir_name == name)
}

async fn work_of(s: &World, folder: &WatchFolder, name: &str) -> WorkRecord {
    work_in(s, folder, name)
        .await
        .unwrap_or_else(|| panic!("no work {name}"))
}

#[tokio::test]
async fn starting_a_rule_for_an_archived_work_keeps_its_id_and_its_season_link_under_the_collect_folder(
) {
    let paused = |phrase: &str, directory: &str| RuleInput {
        state: RuleState::Paused,
        ..rule(phrase, directory)
    };
    let s = world(vec![
        paused("Mushoku", "Mushoku/Season 02"),
        paused("Clevatess S03", "Clevatess/Season 03"),
    ])
    .await;
    write(&s.archive.join("Mushoku/Season 01/Mushoku S01E01.mkv"), "x");
    write(&s.archive.join("Mushoku/Season 01/Mushoku S01E02.mkv"), "x");
    // Clevatess is in both folders.
    write(
        &s.archive.join("Clevatess/Season 01/Clevatess S01E01.mkv"),
        "x",
    );
    write(
        &s.media.join("Clevatess/Season 02/Clevatess S02E01.mkv"),
        "x",
    );
    let (collect, archive) = watching(&s).await;
    let mushoku = work_of(&s, &archive, "Mushoku").await;
    assert_eq!(mushoku.episodes.len(), 2);
    // What the user chose for the work before: season 1 linked.
    s.ctx
        .seasons
        .put_entry(entry(166873, Some(2)))
        .await
        .unwrap();
    let link = s.ctx.seasons.link(&mushoku.id, 1).await.unwrap();
    let linked = s
        .ctx
        .seasons
        .set_links(&mushoku.id, 1, link.version, vec![166873])
        .await
        .unwrap();

    // An archived work: the same ID, now under the collect folder, with what
    // was chosen for it, and nothing left behind.
    let done = s.archive_of(&s.rule_of(0).await, Direction::Start).await;
    assert_eq!(done.outcome.result, MOVED, "{done:?}");
    assert!(work_in(&s, &archive, "Mushoku").await.is_none());
    let moved = work_of(&s, &collect, "Mushoku").await;
    assert_eq!(moved.id, mushoku.id);
    assert_eq!(moved.episodes, mushoku.episodes);
    assert_eq!(moved.first_seen_at, mushoku.first_seen_at);
    assert_eq!(s.ctx.seasons.link(&mushoku.id, 1).await.unwrap(), linked);

    // A work in both folders: the archive's records merge into the collect
    // folder's ID.
    let kept = work_of(&s, &collect, "Clevatess").await;
    let from_archive = work_of(&s, &archive, "Clevatess").await;
    assert_ne!(kept.id, from_archive.id);
    s.advance(1);
    let done = s.archive_of(&s.rule_of(1).await, Direction::Start).await;
    assert_eq!(done.outcome.result, MOVED, "{done:?}");
    assert!(work_in(&s, &archive, "Clevatess").await.is_none());
    let merged = work_of(&s, &collect, "Clevatess").await;
    assert_eq!(merged.id, kept.id);
    assert_eq!(merged.seasons, [1, 2]);
    assert_eq!(merged.episodes.len(), 2);

    // The next reading finds the same works, one row each.
    read_again(&s, &[&collect, &archive]).await;
    let read = work_of(&s, &collect, "Mushoku").await;
    assert_eq!(read.id, mushoku.id);
    assert!(!read.missing);
    assert_eq!(s.ctx.seasons.link(&mushoku.id, 1).await.unwrap(), linked);
    let read = work_of(&s, &collect, "Clevatess").await;
    assert_eq!(
        (read.id.as_str(), read.episodes.len()),
        (kept.id.as_str(), 2)
    );
    let rows = s.ctx.library.works(&collect.id).await.unwrap();
    assert_eq!(rows.iter().filter(|w| w.dir_name == "Clevatess").count(), 1);
}

#[tokio::test]
async fn a_start_of_a_rule_saved_into_an_archived_work_folder_moves_the_work_and_keeps_when_the_rule_began_collecting(
) {
    let s = world(vec![rule("Mushoku", "Mushoku/Season 02")]).await;
    write(&s.archive.join("Mushoku/Season 01/Mushoku S01E02.mkv"), "x");
    // Collecting since 777 until the edit into the archived folder paused it.
    s.resumed(0, 777).await;
    let rule = s.rule_of(0).await;
    s.ctx
        .channels
        .set_rule_state(&rule.id, RuleState::Paused, 800)
        .await
        .unwrap();

    let done = s.archive_of(&s.rule_of(0).await, Direction::Start).await;
    assert_eq!(done.outcome.result, MOVED, "{done:?}");

    let stored = s.stored_rule(&rule).await;
    assert_eq!(stored.state, RuleState::Active);
    assert_eq!(stored.resumed_at, Some(777));
    assert!(!s.archive.join("Mushoku").exists());
    assert!(s
        .media
        .join("Mushoku/Season 01/Mushoku S01E02.mkv")
        .exists());
}
