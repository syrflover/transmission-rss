//! The `work_archive` command ([`run`], ADR 0015: a rule is tested in the crate
//! that owns it): the rules deleted with its acceptance ([`accept_deleting`]),
//! which rules keep a work folder in place, what the command ends with, and
//! what a move that Transmission does not finish in time leaves for the next
//! look. The move itself is `rule_archive`'s, tested in
//! `rule_archive/work_folder/tests.rs`; what the library keeps of a work that
//! moves (its cover, its creators, its season links) is tested with
//! `follow_move` in trss-library; the web's part and a worker stopped midway are
//! in trss's `tests/it/archive_move.rs`. Every case runs against the fake
//! Transmission and real temporary folders.

use std::{fs, path::Path, time::Duration};

use trss_core::commands::{Accepted, CommandState, MAX_ATTEMPTS};
use trss_library::{discovery, store::library::WatchFolder};

use super::super::rule_archive::{work_folder::MovePolicy, FAILED, MOVED};
use super::*;
use crate::{
    store::channels::{RuleInput, RuleState},
    test_world::{entry, files, show_hash, write, World},
};

fn rule(phrase: &str, directory: &str, state: RuleState) -> RuleInput {
    RuleInput {
        r#match: Some(phrase.to_owned()),
        directory: directory.to_owned(),
        state,
        ..Default::default()
    }
}

/// A world with the archive folder set, `rules` in its channel, and a move that
/// waits for Transmission for moments only.
async fn world(rules: Vec<RuleInput>) -> World {
    let mut s = World::with_rules(rules).await.with_archive_folder().await;
    s.ctx.moves = MovePolicy {
        poll: Duration::from_millis(10),
        timeout: Duration::from_secs(5),
    };
    s
}

/// The collect and archive folders registered as watch folders, as saving the
/// settings leaves them, each read once: the collect folder's.
async fn watching(s: &World) -> WatchFolder {
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
    folders.remove(0)
}

#[tokio::test]
async fn a_work_folder_moves_with_its_torrents_and_the_work_keeps_its_id_and_season_link() {
    let s = world(Vec::new()).await;
    s.seeding_in(
        1,
        "Clevatess S02E01.mkv",
        &s.media.join("Clevatess/Season 02"),
    );
    write(&s.media.join("Clevatess/.trss/subs/S02E01.ass"), "sub");
    let collect = watching(&s).await;
    let work = s.ctx.library.works(&collect.id).await.unwrap().remove(0);
    s.ctx
        .seasons
        .put_entry(entry(166873, Some(12)))
        .await
        .unwrap();
    let link = s.ctx.seasons.link(&work.id, 2).await.unwrap();
    let linked = s
        .ctx
        .seasons
        .set_links(&work.id, 2, link.version, vec![166873])
        .await
        .unwrap();

    let moved = s.work_archive_of("Clevatess").await;

    assert_eq!(moved.state, CommandState::Done, "{moved:?}");
    assert_eq!(moved.outcome.result, MOVED);
    assert_eq!(moved.outcome.reason, None);
    assert!(!s.media.join("Clevatess").exists());
    assert_eq!(
        files(&s.archive),
        [
            "Clevatess/.trss/subs/S02E01.ass",
            "Clevatess/Season 02/Clevatess S02E01.mkv",
        ]
    );
    assert_eq!(
        s.tr.torrent(&show_hash(1)).download_dir,
        s.archive.join("Clevatess/Season 02").to_str().unwrap()
    );
    // The library knows the work under the archive folder by the same ID.
    let after = s.ctx.library.work_detail(&work.id).await.unwrap().unwrap();
    assert_eq!(after.watch_folder_path, s.archive.to_str().unwrap());
    assert_eq!(s.ctx.seasons.link(&work.id, 2).await.unwrap(), linked);
}

#[tokio::test]
async fn an_active_rule_saving_into_the_folder_keeps_it_and_a_paused_one_does_not() {
    let s = world(vec![rule(
        "Clevatess S2",
        "Clevatess/Season 02",
        RuleState::Paused,
    )])
    .await;
    // The active rule is in another channel: every channel counts.
    let other = s
        .channel_with(
            "feed-b",
            vec![rule(
                "Clevatess S3",
                "Clevatess/Season 03",
                RuleState::Active,
            )],
        )
        .await;
    s.seeding_in(
        1,
        "Clevatess S02E01.mkv",
        &s.media.join("Clevatess/Season 02"),
    );

    let kept = s.work_archive_of("Clevatess").await;

    assert_eq!(kept.state, CommandState::Done);
    assert_eq!(kept.outcome.result, KEPT);
    assert_eq!(
        kept.outcome.reason.as_deref(),
        Some("‘Clevatess/Season 03’ 규칙이 아직 이 작품 폴더에 받고 있어서 옮기지 않았어요.")
    );
    assert_eq!(
        files(&s.media),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert!(files(&s.archive).is_empty());
    assert!(s.tr.calls_of("torrent-set-location").is_empty());

    // Paused too, it no longer keeps the folder; the paused rule never did.
    s.ctx
        .channels
        .set_rule_state(&other.rules[0].id, RuleState::Paused, s.now())
        .await
        .unwrap();
    s.advance(1);
    let moved = s.work_archive_of("Clevatess").await;
    assert_eq!(moved.outcome.result, MOVED, "{moved:?}");
    assert_eq!(
        files(&s.archive),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
}

#[tokio::test]
async fn without_a_collect_or_archive_folder_the_folder_stays_with_the_reason() {
    for (what, archive_folder, collect_folder, reason) in [
        (
            "no archive folder",
            false,
            true,
            "보관 폴더를 정하지 않아서 폴더는 옮기지 않았어요.",
        ),
        (
            "no collect folder",
            true,
            false,
            "수집 폴더를 정하지 않아서 폴더는 옮기지 않았어요.",
        ),
    ] {
        let s = if archive_folder {
            world(Vec::new()).await
        } else {
            World::with_rules(Vec::new()).await
        };
        if !collect_folder {
            s.sql("DELETE FROM collection_settings");
        }
        s.seeding_in(
            1,
            "Clevatess S02E01.mkv",
            &s.media.join("Clevatess/Season 02"),
        );

        let kept = s.work_archive_of("Clevatess").await;

        assert_eq!(kept.state, CommandState::Done, "{what}");
        assert_eq!(kept.outcome.result, KEPT, "{what}");
        assert_eq!(kept.outcome.reason.as_deref(), Some(reason), "{what}");
        assert_eq!(
            files(&s.media),
            ["Clevatess/Season 02/Clevatess S02E01.mkv"],
            "{what}"
        );
        assert!(s.tr.calls_of("torrent-get").is_empty(), "{what}");
    }
}

#[tokio::test]
async fn the_same_file_on_both_sides_moves_nothing_and_moving_again_works_once_cleared() {
    let s = world(Vec::new()).await;
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

    let failed = s.work_archive_of("Clevatess").await;

    assert_eq!(failed.state, CommandState::Failed, "{failed:?}");
    assert_eq!(failed.outcome.result, FAILED);
    let reason = failed.outcome.reason.unwrap();
    assert!(
        reason.contains("`Season 02/Clevatess S02E02.mkv`"),
        "{reason}"
    );
    assert!(reason.contains("아무것도 옮기지 않았어요"), "{reason}");
    assert_eq!(files(&s.media), collect_before);
    assert_eq!(files(&s.archive), archive_before);
    assert!(s.tr.calls_of("torrent-set-location").is_empty());

    // `다시 옮기기` is a new command of the same folder.
    fs::remove_file(s.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv")).unwrap();
    s.advance(1);
    let moved = s.work_archive_of("Clevatess").await;
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
async fn a_move_transmission_has_not_finished_is_carried_on_by_the_next_look() {
    let mut s = world(Vec::new()).await;
    s.ctx.moves = MovePolicy {
        poll: Duration::from_millis(20),
        timeout: Duration::from_millis(100),
    };
    s.seeding_in(
        1,
        "Clevatess S02E01.mkv",
        &s.media.join("Clevatess/Season 02"),
    );
    write(&s.media.join("Clevatess/Season 02/notes.txt"), "x");
    s.tr.async_locations(true);
    s.tr.lag_locations(u32::MAX);

    let command = s.start_work_archive("Clevatess").await;
    let retry = s.run_work_archive(&command).await.unwrap_err();
    assert!(matches!(retry, Retry::Later(_)), "{retry:?}");
    assert_eq!(s.command(&command.id).await.state, CommandState::Running);
    assert!(command.attempts < MAX_ATTEMPTS);

    // Transmission has finished by the next look, which moves the rest.
    s.tr.lag_locations(0);
    s.tr.settle_locations();
    let command = s.claim().await;
    let moved = s.run_work_archive(&command).await.unwrap();
    assert_eq!(moved.state, CommandState::Done, "{moved:?}");
    assert_eq!(moved.outcome.result, MOVED);
    assert!(!s.media.join("Clevatess").exists());
    assert_eq!(
        files(&s.archive),
        [
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/notes.txt",
        ]
    );
}

#[tokio::test]
async fn the_rules_are_deleted_with_the_stored_move_or_not_at_all() {
    let s = world(vec![
        rule("Clevatess S2", "Clevatess/Season 02", RuleState::Active),
        rule("Clevatess S3", "Clevatess/Season 03", RuleState::Paused),
    ])
    .await;
    let rules = || async {
        let channels = s.ctx.channels.list_channels_with_rules().await.unwrap();
        channels
            .into_iter()
            .flat_map(|cwr| cwr.rules)
            .collect::<Vec<_>>()
    };
    let before = rules().await;
    let both: Vec<(String, i64)> = before.iter().map(|r| (r.id.clone(), r.version)).collect();
    let move_ = |id: &str| {
        WorkArchive {
            work: "Clevatess".into(),
        }
        .command(id.into())
    };

    // A rule at another version: nothing is deleted or stored.
    let mut stale = both.clone();
    stale[1].1 += 1;
    let refused = accept_deleting(&s.ctx.commands, move_("move-1"), stale, s.now())
        .await
        .unwrap();
    assert!(refused.unwrap_err().is_conflict());
    assert_eq!(rules().await, before);
    assert!(s.ctx.commands.get("move-1").await.unwrap().is_none());

    let stored = accept_deleting(&s.ctx.commands, move_("move-1"), both.clone(), s.now())
        .await
        .unwrap();
    assert!(matches!(stored, Ok(Accepted::Created(_))), "{stored:?}");
    assert!(rules().await.is_empty());

    // The same request again is answered as stored, though its rules are gone.
    let again = accept_deleting(&s.ctx.commands, move_("move-1"), both, s.now())
        .await
        .unwrap();
    assert!(matches!(again, Ok(Accepted::Existing(_))), "{again:?}");

    // Another move of the folder while it is open deletes nothing.
    let late = s
        .channel_with(
            "feed-b",
            vec![rule(
                "Clevatess S4",
                "Clevatess/Season 04",
                RuleState::Active,
            )],
        )
        .await;
    let doomed = vec![(late.rules[0].id.clone(), late.rules[0].version)];
    let busy = accept_deleting(&s.ctx.commands, move_("move-2"), doomed, s.now())
        .await
        .unwrap();
    assert!(matches!(busy, Ok(Accepted::Busy(_))), "{busy:?}");
    assert_eq!(rules().await, late.rules);
}

#[tokio::test]
async fn a_payload_that_names_no_single_folder_fails_and_touches_nothing() {
    let s = world(Vec::new()).await;
    write(&s.media.join("Clevatess/Season 02/a.mkv"), "x");
    for work in ["..", "Clevatess/Season 02", "", "."] {
        s.advance(1);
        let failed = s.work_archive_of(work).await;
        assert_eq!(failed.state, CommandState::Failed, "{work:?}");
        assert_eq!(
            failed.outcome.reason.as_deref(),
            Some("요청 내용을 읽지 못했어요."),
            "{work:?}"
        );
    }
    assert_eq!(files(&s.media), ["Clevatess/Season 02/a.mkv"]);
    assert!(files(&s.archive).is_empty());
    assert!(Path::new(&s.archive).exists());
}
