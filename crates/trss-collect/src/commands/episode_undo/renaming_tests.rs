//! What `되돌리기` renames (`docs/specs/collection.md`, 영상 회차 변환): the
//! videos a rule received under the app's offset take the names the value
//! put back gives them, through Transmission when their torrent is there and
//! on disk when it is gone, and never over a name that is taken. The cases
//! run [`World::undo`] against the fake Transmission, which renames files on
//! the media folder as Transmission does.

use super::fixtures::*;
use super::*;
use crate::test_world::{show_hash, World};
use trss_core::{commands::CommandState, files::testing};
use trss_transmission::fake::FakeTorrent;

#[tokio::test]
async fn undoing_puts_the_previous_value_back_and_renames_what_it_named() {
    let (s, rule) = third_season_received().await;
    assert_eq!(s.mark_of(&rule).await.previous, Some(-24));

    let finished = s.undo(&rule, "undo-0001-a", -48).await;

    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(finished.outcome.result, UNDONE);
    let stored = s.stored_rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (-24, false));
    let mark = s.mark_of(&rule).await;
    assert_eq!((mark.basis, mark.previous), (None, None));
    let undo = s
        .ctx
        .channels
        .episode_undo("undo-0001-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!((undo.from, undo.to), (-48, -24));
    assert_eq!(
        s.undo_files("undo-0001-a").await,
        [
            file("Show S03E01.mkv", "Show S03E25.mkv", "renamed"),
            file("Show S03E02.mkv", "Show S03E26.mkv", "renamed"),
        ]
    );
    // Renamed through Transmission: both torrents are still there, seeding
    // the files under their new names.
    assert_eq!(s.torrent_names(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    assert_eq!(s.on_disk(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    assert_eq!(s.content("Show S03E25.mkv"), b"video 49");

    // The value is the user's now: the next release is named with it, and
    // the app does not decide the rule again.
    s.feed_shows(&[49, 50, 51]);
    s.cycle_later().await;
    assert_eq!(
        s.torrent_names(),
        ["Show S03E25.mkv", "Show S03E26.mkv", "Show S03E27.mkv"]
    );
    let stored = s.stored_rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (-24, false));
}

#[tokio::test]
async fn a_name_that_is_taken_is_never_renamed_onto_and_the_others_go_on() {
    let (s, rule) = third_season_received().await;
    // Something of the person's has the name `S03E02` would go back to.
    std::fs::write(s.season3().join("Show S03E26.mkv"), "the person's").unwrap();

    let finished = s.undo(&rule, "undo-0002-a", -48).await;

    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(s.torrent_names(), ["Show S03E02.mkv", "Show S03E25.mkv"]);
    assert_eq!(
        s.on_disk(),
        ["Show S03E02.mkv", "Show S03E25.mkv", "Show S03E26.mkv"]
    );
    assert_eq!(s.content("Show S03E26.mkv"), b"the person's");
    assert_eq!(s.content("Show S03E02.mkv"), b"video 50");
    assert_eq!(
        s.undo_files("undo-0002-a").await,
        [
            file("Show S03E01.mkv", "Show S03E25.mkv", "renamed"),
            file("Show S03E02.mkv", "Show S03E26.mkv", "kept"),
        ]
    );
    let reason = s.undo_reason("undo-0002-a", 1).await;
    assert!(reason.contains("이미 있어요"), "{reason}");
    assert_eq!(s.stored_rule(&rule).await.episode, -24);
}

#[tokio::test]
async fn a_video_whose_torrent_is_gone_is_renamed_on_disk_without_replacing() {
    let (s, rule) = third_season_received().await;
    // The person removed the torrent of `- 50` and kept its file.
    s.tr.remove(&show_hash(50));

    let finished = s.undo(&rule, "undo-0003-a", -48).await;

    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(s.torrent_names(), ["Show S03E25.mkv"]);
    assert_eq!(s.on_disk(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    assert_eq!(s.content("Show S03E26.mkv"), b"video 50");
}

/// A rename on disk is the app's own, so its folder is synced for the new name
/// to outlast a power loss (the folder is the source's and the target's both).
/// A folder that cannot be synced does not undo the rename.
#[tokio::test]
async fn a_video_renamed_on_disk_has_its_folder_synced_and_a_failing_sync_does_not_undo_it() {
    let (s, rule) = third_season_received().await;
    s.tr.remove(&show_hash(50));
    s.tr.remove(&show_hash(49));
    let folder = s.season3();
    assert_eq!(testing::syncs_of(&folder), 0);

    let finished = s.undo(&rule, "undo-0006-a", -48).await;

    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(s.on_disk(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    assert!(
        testing::syncs_of(&folder) >= 1,
        "the folder the videos were renamed in was not synced"
    );

    let (s, rule) = third_season_received().await;
    s.tr.remove(&show_hash(50));
    s.tr.remove(&show_hash(49));
    let _failing = testing::fail_syncs_of(&s.season3());
    let finished = s.undo(&rule, "undo-0007-a", -48).await;
    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(s.on_disk(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    assert_eq!(
        s.undo_files("undo-0007-a").await,
        [
            file("Show S03E01.mkv", "Show S03E25.mkv", "renamed"),
            file("Show S03E02.mkv", "Show S03E26.mkv", "renamed"),
        ]
    );
}

#[tokio::test]
async fn an_undo_before_any_item_keeps_the_app_from_deciding_again() {
    let s = World::bare().await;
    let place = s
        .library_of(&[(1, &["01", "02"]), (2, &["01", "02"])])
        .await;
    place.link(1, &[Some(24)]).await;
    place.link(2, &[Some(24)]).await;
    s.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, -24).await;
    // The app decided, and nothing was received yet.
    s.ctx
        .channels
        .set_auto_episode(
            &rule.id,
            rule.version,
            -48,
            "첫 화가 49화라서 −48로 정했어요.",
        )
        .await
        .unwrap()
        .expect("the rule was at the version read");
    assert_eq!(s.mark_of(&rule).await.previous, Some(-24));

    let finished = s.undo(&rule, "undo-0005-a", -48).await;
    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(s.undo_files("undo-0005-a").await, []);
    assert_eq!(s.stored_rule(&rule).await.episode, -24);

    s.feed_shows(&[]);
    s.cycle_later().await;
    s.feed_shows(&[49]);
    s.cycle_later().await;
    assert_eq!(s.torrent_names(), ["Show S03E25.mkv"]);
    let stored = s.stored_rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (-24, false));
}

#[tokio::test]
async fn names_an_undo_frees_for_itself_are_taken_in_turn_and_nothing_is_kept() {
    let (s, rule) = overlapping_names().await;

    let finished = s.undo(&rule, "undo-0101-a", -36).await;

    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    let mut files = s.undo_files("undo-0101-a").await;
    files.sort();
    assert_eq!(
        files,
        [
            file("Show S03E01.mkv", "Show S03E13.mkv", "renamed"),
            file("Show S03E13.mkv", "Show S03E25.mkv", "renamed"),
        ]
    );
    assert_eq!(s.torrent_names(), ["Show S03E13.mkv", "Show S03E25.mkv"]);
    assert_eq!(s.content("Show S03E13.mkv"), b"video 37");
    assert_eq!(s.content("Show S03E25.mkv"), b"video 49");
}

#[tokio::test]
async fn a_torrent_whose_file_is_not_at_its_name_is_never_renamed_onto_another_file() {
    let (s, rule) = overlapping_names().await;
    // `- 49`'s file is not there (the person deleted it; Transmission still
    // names its torrent `S03E13`).
    std::fs::remove_file(s.season3().join("Show S03E13.mkv")).unwrap();

    let finished = s.undo(&rule, "undo-0102-a", -36).await;

    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    // Neither moves: `- 49`'s torrent has no file to rename, and its torrent
    // still claims the name `- 37` would take.
    assert_eq!(s.torrent_names(), ["Show S03E01.mkv", "Show S03E13.mkv"]);
    assert_eq!(s.on_disk(), ["Show S03E01.mkv"]);
    assert_eq!(s.content("Show S03E01.mkv"), b"video 37");
    let states: Vec<String> = s
        .undo_files("undo-0102-a")
        .await
        .into_iter()
        .map(|f| f.2)
        .collect();
    assert_eq!(states, ["kept", "kept"]);
}

#[tokio::test]
async fn a_gone_torrent_of_a_title_without_an_extension_is_found_by_its_name() {
    let (s, rule) = third_season_received().await;
    s.tr.remove(&show_hash(50));
    s.title_without_extension(50);

    let finished = s.undo(&rule, "undo-0801-a", -48).await;

    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(s.on_disk(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    assert_eq!(s.content("Show S03E26.mkv"), b"video 50");
    assert_eq!(
        s.undo_files("undo-0801-a").await,
        [
            file("Show S03E01.mkv", "Show S03E25.mkv", "renamed"),
            file("Show S03E02.mkv", "Show S03E26.mkv", "renamed"),
        ]
    );
}

#[tokio::test]
async fn a_gone_torrent_of_a_title_without_an_extension_and_two_videos_is_kept_with_why() {
    let (s, rule) = third_season_received().await;
    s.tr.remove(&show_hash(50));
    s.title_without_extension(50);
    std::fs::write(s.season3().join("Show S03E02.mp4"), b"another").unwrap();

    let finished = s.undo(&rule, "undo-0802-a", -48).await;

    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(
        s.on_disk(),
        ["Show S03E02.mkv", "Show S03E02.mp4", "Show S03E25.mkv"]
    );
    assert_eq!(
        s.undo_files("undo-0802-a").await,
        [
            file("Show S03E01.mkv", "Show S03E25.mkv", "renamed"),
            file("Show S03E02.mkv", "Show S03E26.mkv", "kept"),
        ]
    );
    let reason = s.undo_reason("undo-0802-a", 1).await;
    assert!(reason.contains("확장자만 다른 영상이 여러 개"), "{reason}");
}

/// A file another torrent lists too is not renamed through its torrent:
/// Transmission would move it from under the other one.
#[tokio::test]
async fn a_file_another_torrent_shares_keeps_its_name() {
    let (s, rule) = third_season_received().await;
    s.tr.preload(
        FakeTorrent::new(&format!("{:040}", 7), "Pack")
            .in_dir(s.season3())
            .files(&["Show S03E02.mkv"])
            .status(6),
    );

    let finished = s.undo(&rule, "undo-0402-a", -48).await;

    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(s.on_disk(), ["Show S03E02.mkv", "Show S03E25.mkv"]);
    assert_eq!(
        s.undo_files("undo-0402-a").await,
        [
            file("Show S03E01.mkv", "Show S03E25.mkv", "renamed"),
            file("Show S03E02.mkv", "Show S03E26.mkv", "kept"),
        ]
    );
    let reason = s.undo_reason("undo-0402-a", 1).await;
    assert!(reason.contains("다른 토렌트"), "{reason}");
}
