//! The `rule_archive` command around the work folder's move ([`run`], ADR
//! 0015: a rule is tested in the crate that owns it): which rules keep a
//! folder in place, what the command ends with, when the rule is turned on,
//! and what a move that Transmission does not finish in time leaves for the
//! next look. The move itself is in `work_folder/tests.rs`, and the web's part
//! and the order of the commands in trss-worker's `tests/it/archive_move.rs`.
//! Every case runs through [`World::archive_of`] against the fake Transmission
//! and real temporary folders.

use std::{fs, path::Path, time::Duration};

use trss_core::commands::{CommandState, MAX_ATTEMPTS};

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
async fn a_folder_another_active_rule_saves_in_stays_until_the_last_rule_is_archived() {
    let s = world(vec![rule("Clevatess S2", "Clevatess/Season 02")]).await;
    // The other rule is in another channel: every channel counts.
    let other = s
        .channel_with("feed-b", vec![rule("Clevatess S3", "Clevatess/Season 03")])
        .await;
    let (season2, season3) = (s.rule_of(0).await, other.rules[0].clone());
    s.seeding_in(
        1,
        "Clevatess S02E01.mkv",
        &s.media.join("Clevatess/Season 02"),
    );

    let kept = s.archive_of(&season2, Direction::Archive).await;
    assert_eq!(kept.state, CommandState::Done);
    assert_eq!(kept.outcome.result, KEPT);
    assert_eq!(
        kept.outcome.reason.as_deref(),
        Some("‘Clevatess/Season 03’ 규칙이 아직 이 작품 폴더에 받고 있어서 옮기지 않았어요.")
    );
    assert_eq!(s.stored_rule(&season2).await.state, RuleState::Archived);
    assert_eq!(
        files(&s.media),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert!(files(&s.archive).is_empty());
    assert!(s.tr.calls_of("torrent-set-location").is_empty());

    // Restoring it now finds the folder in the collect folder already.
    s.advance(1);
    let restored = s.archive_of(&season2, Direction::Restore).await;
    assert_eq!(restored.state, CommandState::Done, "{restored:?}");
    assert_eq!(restored.outcome.result, MOVED);
    assert_eq!(
        restored.outcome.reason.as_deref(),
        Some("작품 폴더 `Clevatess`가 이미 수집 폴더에 있어요.")
    );
    assert_eq!(s.stored_rule(&season2).await.state, RuleState::Active);

    // Archived together, the folder moves with the last of them.
    s.advance(1);
    let first = s.archive_of(&season2, Direction::Archive).await;
    assert_eq!(first.outcome.result, KEPT);
    let last = s.archive_of(&season3, Direction::Archive).await;
    assert_eq!(last.outcome.result, MOVED);
    assert_eq!(
        files(&s.archive),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert!(!s.media.join("Clevatess").exists());
}

#[tokio::test]
async fn the_same_file_on_both_sides_moves_nothing_and_moving_again_works_once_cleared() {
    let s = world(vec![rule("Clevatess", "Clevatess/Season 02")]).await;
    let rule = s.rule_of(0).await;
    s.seeding_in(
        1,
        "Clevatess S02E01.mkv",
        &s.media.join("Clevatess/Season 02"),
    );
    write(
        &s.media.join("Clevatess/Season 02/Clevatess S02E02.mkv"),
        "new",
    );
    write(
        &s.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv"),
        "old",
    );
    let (collect_before, archive_before) = (files(&s.media), files(&s.archive));

    let failed = s.archive_of(&rule, Direction::Archive).await;

    assert_eq!(failed.state, CommandState::Failed, "{failed:?}");
    assert_eq!(failed.outcome.result, FAILED);
    let reason = failed.outcome.reason.unwrap();
    assert!(
        reason.contains("`Season 02/Clevatess S02E02.mkv`"),
        "{reason}"
    );
    assert!(reason.contains("아무것도 옮기지 않았어요"), "{reason}");
    // The two copies may differ: it asks to compare, never to clear a side.
    assert!(reason.contains("두 쪽을 견줘"), "{reason}");
    assert!(!reason.contains("정리"), "{reason}");
    // The rule is archived, and nothing moved, on disk or in Transmission.
    assert_eq!(s.stored_rule(&rule).await.state, RuleState::Archived);
    assert_eq!(files(&s.media), collect_before);
    assert_eq!(files(&s.archive), archive_before);
    assert!(s.tr.calls_of("torrent-set-location").is_empty());

    // `다시 옮기기` is a new archive of the archived rule.
    fs::remove_file(s.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv")).unwrap();
    s.advance(1);
    let moved = s.archive_of(&rule, Direction::Archive).await;
    assert_eq!(moved.state, CommandState::Done, "{moved:?}");
    assert_eq!(moved.outcome.result, MOVED);
    assert_eq!(
        files(&s.archive),
        [
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/Clevatess S02E02.mkv",
        ]
    );
    assert!(!s.media.join("Clevatess").exists());
}

#[tokio::test]
async fn restoring_moves_the_folder_back_before_the_rule_is_on() {
    let s = world(vec![rule("Clevatess", "Clevatess/Season 02")]).await;
    let rule = s.rule_of(0).await;
    s.seeding_in(
        1,
        "Clevatess S02E01.mkv",
        &s.media.join("Clevatess/Season 02"),
    );
    write(&s.media.join("Clevatess/.trss/subs/S02E01.ass"), "sub");
    let archived = s.archive_of(&rule, Direction::Archive).await;
    assert_eq!(archived.outcome.result, MOVED, "{archived:?}");
    assert!(s.archive.join("Clevatess").exists());

    // A restore that cannot move keeps the rule archived, with the reason.
    write(
        &s.media.join("Clevatess/.trss/subs/S02E01.ass"),
        "made meanwhile",
    );
    s.advance(1);
    let failed = s.archive_of(&rule, Direction::Restore).await;
    assert_eq!(failed.state, CommandState::Failed);
    assert!(failed
        .outcome
        .reason
        .unwrap()
        .contains("`.trss/subs/S02E01.ass`"));
    assert_eq!(s.stored_rule(&rule).await.state, RuleState::Archived);
    fs::remove_dir_all(s.media.join("Clevatess")).unwrap();

    // While the folder moves back, the rule is still off.
    s.advance(1);
    let command = s.start_archive(&rule, Direction::Restore).await;
    let gate = s.tr.hold("torrent-set-location");
    let watch = async {
        gate.wait_arrived().await;
        assert_eq!(s.stored_rule(&rule).await.state, RuleState::Archived);
        gate.release_all();
    };
    let (finished, ()) = tokio::join!(s.run_archive(&command), watch);

    let finished = finished.unwrap();
    assert_eq!(finished.outcome.result, MOVED, "{finished:?}");
    assert_eq!(s.stored_rule(&rule).await.state, RuleState::Active);
    assert!(!s.archive.join("Clevatess").exists());
    assert_eq!(
        files(&s.media.join("Clevatess")),
        [".trss/subs/S02E01.ass", "Season 02/Clevatess S02E01.mkv"]
    );
    assert_eq!(
        s.tr.torrent(&crate::test_world::show_hash(1)).download_dir,
        s.media.join("Clevatess/Season 02").to_str().unwrap()
    );
}

#[tokio::test]
async fn a_rule_whose_work_folder_cannot_be_told_is_only_archived() {
    struct Case {
        what: &'static str,
        directory: &'static str,
        archive_folder: bool,
        collect_folder: bool,
        reason: &'static str,
    }
    for case in [
        Case {
            what: "no archive folder",
            directory: "Clevatess/Season 02",
            archive_folder: false,
            collect_folder: true,
            reason: "보관 폴더를 정하지 않아서 폴더는 옮기지 않았어요.",
        },
        Case {
            what: "the rule saves into the collect folder itself",
            directory: "",
            archive_folder: true,
            collect_folder: true,
            reason: "저장 폴더가 수집 폴더 자체라서 옮길 작품 폴더가 없어요.",
        },
        Case {
            what: "the rule saves outside the collect folder",
            directory: "../elsewhere/Clevatess",
            archive_folder: true,
            collect_folder: true,
            reason: "저장 폴더가 수집 폴더 밖이라서 옮기지 않았어요.",
        },
        Case {
            what: "no collect folder",
            directory: "Clevatess/Season 02",
            archive_folder: true,
            collect_folder: false,
            reason: "수집 폴더를 정하지 않아서 폴더는 옮기지 않았어요.",
        },
    ] {
        let rules = vec![rule("Clevatess", case.directory)];
        let s = if case.archive_folder {
            world(rules).await
        } else {
            World::with_rules(rules).await
        };
        if !case.collect_folder {
            s.sql("DELETE FROM collection_settings");
        }
        let rule = s.rule_of(0).await;
        s.seeding_in(
            1,
            "Clevatess S02E01.mkv",
            &s.media.join("Clevatess/Season 02"),
        );

        let kept = s.archive_of(&rule, Direction::Archive).await;

        assert_eq!(kept.state, CommandState::Done, "{}", case.what);
        assert_eq!(kept.outcome.result, KEPT, "{}", case.what);
        assert_eq!(
            kept.outcome.reason.as_deref(),
            Some(case.reason),
            "{}",
            case.what
        );
        assert_eq!(
            s.stored_rule(&rule).await.state,
            RuleState::Archived,
            "{}",
            case.what
        );
        assert_eq!(
            files(&s.media),
            ["Clevatess/Season 02/Clevatess S02E01.mkv"],
            "{}",
            case.what
        );
        // Transmission is never asked.
        assert!(
            s.tr.calls_of("torrent-set-location").is_empty(),
            "{}",
            case.what
        );
        assert!(s.tr.calls_of("torrent-get").is_empty(), "{}", case.what);
    }
}

#[tokio::test]
async fn transmission_slower_than_the_wait_keeps_the_command_for_the_next_look_and_fails_it_on_the_last(
) {
    let mut s = world(vec![rule("Clevatess", "Clevatess/Season 02")]).await;
    s.ctx.moves = MovePolicy {
        poll: Duration::from_millis(20),
        timeout: Duration::from_millis(100),
    };
    let rule = s.rule_of(0).await;
    s.seeding_in(
        1,
        "Clevatess S02E01.mkv",
        &s.media.join("Clevatess/Season 02"),
    );
    write(&s.media.join("Clevatess/Season 02/notes.txt"), "x");
    s.tr.async_locations(true);
    s.tr.lag_locations(u32::MAX);

    // Every look but the last leaves the command running for the next one.
    let mut command = s.start_archive(&rule, Direction::Archive).await;
    for look in 1..MAX_ATTEMPTS {
        assert_eq!(command.attempts, look);
        let retry = s.run_archive(&command).await.unwrap_err();
        assert!(matches!(retry, Retry::Later(_)), "{retry:?}");
        assert_eq!(s.command(&command.id).await.state, CommandState::Running);
        command = s.claim().await;
    }

    // The last look ends it, with the reason.
    assert_eq!(command.attempts, MAX_ATTEMPTS);
    let failed = s.run_archive(&command).await.unwrap();
    assert_eq!(failed.state, CommandState::Failed, "{failed:?}");
    let reason = failed.outcome.reason.unwrap();
    assert!(reason.contains("Transmission"), "{reason}");
    assert!(reason.contains("다시 옮기면 남은 것만"), "{reason}");
    assert!(!reason.contains("정리"), "{reason}");
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
async fn archiving_a_work_keeps_its_id_under_the_archive_folder_and_merges_into_a_work_already_there(
) {
    let s = world(vec![
        rule("Clevatess", "Clevatess/Season 02"),
        rule("Solo", "Solo/Season 01"),
    ])
    .await;
    write(
        &s.media.join("Clevatess/Season 02/Clevatess S02E01.mkv"),
        "x",
    );
    write(
        &s.media.join("Clevatess/Season 02/Clevatess S02E02.mkv"),
        "x",
    );
    write(&s.media.join("Solo/Season 01/Solo S01E01.mkv"), "x");
    // The archive already has the earlier season of Clevatess.
    write(
        &s.archive.join("Clevatess/Season 01/Clevatess S01E01.mkv"),
        "x",
    );
    let (collect, archive) = watching(&s).await;
    let solo = work_of(&s, &collect, "Solo").await;
    assert_eq!(solo.episodes.len(), 1);
    let moved = work_of(&s, &collect, "Clevatess").await;
    let kept = work_of(&s, &archive, "Clevatess").await;
    assert_ne!(moved.id, kept.id);

    // A work only the collect folder has: the same ID, now under the archive
    // folder, with nothing left behind.
    let done = s.archive_of(&s.rule_of(1).await, Direction::Archive).await;
    assert_eq!(done.outcome.result, MOVED, "{done:?}");
    assert!(work_in(&s, &collect, "Solo").await.is_none());
    let now_archived = work_of(&s, &archive, "Solo").await;
    assert_eq!(now_archived.id, solo.id);
    assert_eq!(now_archived.episodes, solo.episodes);
    assert_eq!(now_archived.first_seen_at, solo.first_seen_at);

    // A work both folders have: it merges into the destination's ID.
    s.advance(1);
    let done = s.archive_of(&s.rule_of(0).await, Direction::Archive).await;
    assert_eq!(done.outcome.result, MOVED, "{done:?}");
    assert!(work_in(&s, &collect, "Clevatess").await.is_none());
    let merged = work_of(&s, &archive, "Clevatess").await;
    assert_eq!(merged.id, kept.id);
    assert_eq!(merged.seasons, [1, 2]);
    assert_eq!(merged.episodes.len(), 3);

    // The next reading of the folders finds the same works, one row each.
    read_again(&s, &[&collect, &archive]).await;
    let read = work_of(&s, &archive, "Solo").await;
    assert_eq!(read.id, solo.id);
    assert!(!read.missing);
    let read = work_of(&s, &archive, "Clevatess").await;
    assert_eq!(
        (read.id.as_str(), read.episodes.len()),
        (kept.id.as_str(), 3)
    );
    let rows = s.ctx.library.works(&archive.id).await.unwrap();
    assert_eq!(rows.iter().filter(|w| w.dir_name == "Clevatess").count(), 1);
    assert!(s
        .ctx
        .library
        .works(&collect.id)
        .await
        .unwrap()
        .iter()
        .all(|w| w.dir_name != "Solo" && w.dir_name != "Clevatess" || w.missing));
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
