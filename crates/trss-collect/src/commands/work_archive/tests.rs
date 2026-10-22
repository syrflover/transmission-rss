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
use trss_library::{
    discovery,
    store::library::{WatchFolder, WorkRecord},
};

use super::super::rule_archive::{work_folder::MovePolicy, FAILED, MOVED};
use super::*;
use crate::{
    store::channels::{Rule, RuleInput, RuleState},
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
    // The two copies may differ: it asks to compare, never to clear a side.
    assert!(reason.contains("두 쪽을 견줘"), "{reason}");
    assert!(!reason.contains("정리"), "{reason}");
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

/// The move `archive_rule` stored for a rule, run as the worker runs it.
async fn moved_for(s: &World, archived: Result<RuleArchived, NotArchived>) -> Finished {
    let Ok(RuleArchived::Moving(stored)) = archived else {
        panic!("no move was stored: {archived:?}");
    };
    let command = s.claim().await;
    assert_eq!(command.id, stored.id);
    s.run_work_archive(&command).await.expect("the move ended")
}

#[tokio::test]
async fn archiving_a_rule_deletes_it_and_the_folder_moves_with_the_last_rule_collecting_there() {
    let s = world(vec![rule(
        "Clevatess S2",
        "Clevatess/Season 02",
        RuleState::Active,
    )])
    .await;
    // The other rules are in another channel: every channel counts.
    let other = s
        .channel_with(
            "feed-b",
            vec![
                rule("Clevatess S3", "Clevatess/Season 03", RuleState::Active),
                rule("Clevatess S4", "Clevatess/Season 04", RuleState::Paused),
            ],
        )
        .await;
    let (season2, season3, season4) = (
        s.rule_of(0).await,
        other.rules[0].clone(),
        other.rules[1].clone(),
    );
    s.seeding_in(
        1,
        "Clevatess S02E01.mkv",
        &s.media.join("Clevatess/Season 02"),
    );

    // Season 03 still collects into the folder: the rule goes, the folder stays.
    let season2_id = format!("rule-{}-{}", season2.id, s.now());
    let kept = moved_for(&s, s.archive_rule(&season2).await).await;
    assert_eq!(kept.state, CommandState::Done);
    assert_eq!(kept.outcome.result, KEPT);
    assert_eq!(
        kept.outcome.reason.as_deref(),
        Some("‘Clevatess/Season 03’ 규칙이 아직 이 작품 폴더에 받고 있어서 옮기지 않았어요.")
    );
    assert!(s
        .ctx
        .channels
        .get_rule(&season2.id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        files(&s.media),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert!(s.tr.calls_of("torrent-set-location").is_empty());

    // Its ID used for Season 03, whose move would be the same, is another
    // request: the rule stays.
    let collect = s.media.display().to_string();
    let reused = archive_rule(
        &s.ctx.channels,
        &s.ctx.commands,
        Some(&collect),
        &season3,
        season2_id.clone(),
        s.now(),
    )
    .await
    .unwrap();
    assert!(
        matches!(&reused, Err(NotArchived::Mismatch(c)) if c.id == season2_id),
        "{reused:?}"
    );
    assert_eq!(s.stored_rule(&season3).await, season3);

    // The last collecting rule takes the folder with it; the paused one stays
    // and does not hold it.
    s.advance(1);
    let moved = moved_for(&s, s.archive_rule(&season3).await).await;
    assert_eq!(moved.outcome.result, MOVED, "{moved:?}");
    assert!(s
        .ctx
        .channels
        .get_rule(&season3.id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(s.stored_rule(&season4).await.state, RuleState::Paused);
    assert_eq!(
        files(&s.archive),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert!(!s.media.join("Clevatess").exists());

    // A rule read before it changed, and one whose folder is moving, are not
    // archived: nothing changes.
    s.advance(1);
    let stale = Rule {
        version: season4.version + 1,
        ..season4.clone()
    };
    let refused = s.archive_rule(&stale).await.unwrap_err();
    assert!(
        matches!(&refused, NotArchived::Changed(e) if e.is_conflict()),
        "{refused:?}"
    );
    let open = s.start_work_archive("Clevatess").await;
    let refused = s.archive_rule(&season4).await.unwrap_err();
    assert!(
        matches!(&refused, NotArchived::Busy(c) if c.id == open.id),
        "{refused:?}"
    );
    assert_eq!(s.stored_rule(&season4).await, season4);
}

#[tokio::test]
async fn a_rule_with_no_folder_to_move_is_only_deleted_and_without_an_archive_folder_the_move_says_so(
) {
    // No archive folder: the move is stored, and keeps the folder saying why.
    let s = World::with_rules(vec![rule(
        "Clevatess",
        "Clevatess/Season 02",
        RuleState::Active,
    )])
    .await;
    let rule_ = s.rule_of(0).await;
    s.seeding_in(
        1,
        "Clevatess S02E01.mkv",
        &s.media.join("Clevatess/Season 02"),
    );
    let kept = moved_for(&s, s.archive_rule(&rule_).await).await;
    assert_eq!(kept.outcome.result, KEPT);
    assert_eq!(
        kept.outcome.reason.as_deref(),
        Some("보관 폴더를 정하지 않아서 폴더는 옮기지 않았어요.")
    );
    assert!(s.ctx.channels.get_rule(&rule_.id).await.unwrap().is_none());
    assert_eq!(
        files(&s.media),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );

    // No work folder: the rule is deleted and no move is stored.
    for (what, directory, collect_folder, why) in [
        ("the collect folder itself", "", true, NoMove::CollectItself),
        ("outside", "../elsewhere/Clevatess", true, NoMove::Outside),
        (
            "`..` that lands inside",
            "Other/../Clevatess/Season 02",
            true,
            NoMove::Outside,
        ),
        (
            "no collect folder",
            "Clevatess/Season 02",
            false,
            NoMove::NoCollectFolder,
        ),
    ] {
        let s = world(vec![rule("Clevatess", directory, RuleState::Active)]).await;
        if !collect_folder {
            s.sql("DELETE FROM collection_settings");
        }
        let rule_ = s.rule_of(0).await;
        s.seeding_in(
            1,
            "Clevatess S02E01.mkv",
            &s.media.join("Clevatess/Season 02"),
        );

        let archived = s.archive_rule(&rule_).await;

        assert!(
            matches!(archived, Ok(RuleArchived::Deleted(no_move)) if no_move == why),
            "{what}: {archived:?}"
        );
        assert!(
            s.ctx.channels.get_rule(&rule_.id).await.unwrap().is_none(),
            "{what}"
        );
        assert!(
            s.ctx.commands.claim_next(s.now()).await.unwrap().is_none(),
            "{what}"
        );
        assert_eq!(
            files(&s.media),
            ["Clevatess/Season 02/Clevatess S02E01.mkv"],
            "{what}"
        );
        assert!(s.tr.calls_of("torrent-get").is_empty(), "{what}");
    }
}

#[test]
fn the_forecast_says_the_rule_goes_and_whether_the_folder_would_move_and_if_not_why() {
    let rule = |directory: &str, state: RuleState| Rule {
        id: directory.into(),
        channel_id: "c".into(),
        position: 0,
        version: 1,
        r#match: Some("x".into()),
        regex: false,
        case_insensitive: false,
        directory: directory.into(),
        episode: 1,
        episode_auto: false,
        state,
        subscription: None,
        resumed_at: None,
    };
    let both = Some(("/c".to_owned(), Some("/a".to_owned())));
    let alone = rule("Alone/Season 01", RuleState::Active);
    let shared = rule("Shared/Season 01", RuleState::Active);
    let sibling = rule("Shared/Season 02", RuleState::Active);
    let sleeping = rule("Shared/Season 03", RuleState::Paused);
    let dotted = rule("Other/../Shared/Season 04", RuleState::Active);

    assert_eq!(
        forecast(both.clone(), std::slice::from_ref(&alone), &alone),
        "규칙을 지우고 작품 폴더(`Alone`)를 보관 폴더로 옮겨요."
    );
    assert_eq!(
        forecast(both.clone(), &[shared.clone(), sibling.clone()], &shared),
        "규칙을 지워요. ‘Shared/Season 02’ 규칙이 아직 이 작품 폴더에 받고 있어서 폴더는 옮기지 않아요. 남은 규칙까지 보관할 때 옮겨요."
    );
    assert_eq!(
        forecast(
            both.clone(),
            &[shared.clone(), sibling.clone(), dotted.clone()],
            &shared
        ),
        "규칙을 지워요. ‘Shared/Season 02’ 규칙 외 1개가 아직 이 작품 폴더에 받고 있어서 폴더는 옮기지 않아요. 남은 규칙까지 보관할 때 옮겨요."
    );
    // A paused rule does not keep the folder.
    assert_eq!(
        forecast(both.clone(), &[shared.clone(), sleeping], &shared),
        "규칙을 지우고 작품 폴더(`Shared`)를 보관 폴더로 옮겨요."
    );
    assert_eq!(
        forecast(Some(("/c".into(), None)), &[], &alone),
        "규칙을 지워요. 보관 폴더를 정하지 않아서 폴더는 옮기지 않아요."
    );
    assert_eq!(
        forecast(None, &[], &alone),
        "규칙을 지워요. 수집 폴더를 정하지 않아서 폴더는 옮기지 않아요."
    );
    assert_eq!(
        forecast(both.clone(), &[], &rule("", RuleState::Active)),
        "규칙을 지워요. 저장 폴더가 수집 폴더 자체라서 옮길 작품 폴더가 없어요."
    );
    assert_eq!(
        forecast(both, &[], &rule("../Elsewhere", RuleState::Active)),
        "규칙을 지워요. 저장 폴더가 수집 폴더 밖이라서 옮기지 않아요."
    );
}

#[tokio::test]
async fn transmission_slower_than_the_wait_keeps_the_move_for_the_next_look_and_fails_it_on_the_last(
) {
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

    // Every look but the last leaves the command running for the next one.
    let mut command = s.start_work_archive("Clevatess").await;
    for look in 1..MAX_ATTEMPTS {
        assert_eq!(command.attempts, look);
        let retry = s.run_work_archive(&command).await.unwrap_err();
        assert!(matches!(retry, Retry::Later(_)), "{retry:?}");
        assert_eq!(s.command(&command.id).await.state, CommandState::Running);
        command = s.claim().await;
    }

    // The last look ends it, with the reason.
    assert_eq!(command.attempts, MAX_ATTEMPTS);
    let failed = s.run_work_archive(&command).await.unwrap();
    assert_eq!(failed.state, CommandState::Failed, "{failed:?}");
    let reason = failed.outcome.reason.unwrap();
    assert!(reason.contains("Transmission"), "{reason}");
    assert!(reason.contains("다시 옮기면 남은 것만"), "{reason}");
    assert!(!reason.contains("정리"), "{reason}");
}

/// The works the library holds in `folder` by name.
async fn work_in(s: &World, folder: &WatchFolder, name: &str) -> Option<WorkRecord> {
    s.ctx
        .library
        .works(&folder.id)
        .await
        .unwrap()
        .into_iter()
        .find(|w| w.dir_name == name)
}

#[tokio::test]
async fn a_work_only_the_collect_folder_has_keeps_its_id_and_one_both_have_merges_into_the_archives(
) {
    let s = world(Vec::new()).await;
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
    let collect = watching(&s).await;
    let archive = s
        .ctx
        .library
        .folders()
        .await
        .unwrap()
        .into_iter()
        .find(|f| Path::new(&f.path) == s.archive)
        .unwrap();
    let solo = work_in(&s, &collect, "Solo").await.unwrap();
    let moved = work_in(&s, &collect, "Clevatess").await.unwrap();
    let kept = work_in(&s, &archive, "Clevatess").await.unwrap();
    assert_ne!(moved.id, kept.id);

    // A work only the collect folder has: the same ID, now under the archive
    // folder, with nothing left behind.
    let done = s.work_archive_of("Solo").await;
    assert_eq!(done.outcome.result, MOVED, "{done:?}");
    assert!(work_in(&s, &collect, "Solo").await.is_none());
    let now_archived = work_in(&s, &archive, "Solo").await.unwrap();
    assert_eq!(now_archived.id, solo.id);
    assert_eq!(now_archived.episodes, solo.episodes);
    assert_eq!(now_archived.first_seen_at, solo.first_seen_at);

    // A work both folders have: it merges into the destination's ID.
    s.advance(1);
    let done = s.work_archive_of("Clevatess").await;
    assert_eq!(done.outcome.result, MOVED, "{done:?}");
    assert!(work_in(&s, &collect, "Clevatess").await.is_none());
    let merged = work_in(&s, &archive, "Clevatess").await.unwrap();
    assert_eq!(merged.id, kept.id);
    assert_eq!(merged.seasons, [1, 2]);
    assert_eq!(merged.episodes.len(), 3);

    // The next reading of the folders finds the same works, one row each.
    for folder in [&collect, &archive] {
        let scan = discovery::scan(Path::new(&folder.path)).unwrap();
        s.advance(1_000);
        s.ctx
            .library
            .record_scan(&folder.id, Ok(scan), s.now())
            .await
            .unwrap();
    }
    let read = work_in(&s, &archive, "Solo").await.unwrap();
    assert_eq!(read.id, solo.id);
    assert!(!read.missing);
    let read = work_in(&s, &archive, "Clevatess").await.unwrap();
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
