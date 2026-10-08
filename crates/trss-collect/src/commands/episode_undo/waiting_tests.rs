//! A video `되돌리기` cannot rename yet waits, still `pending` with why, and
//! the request for the same value again (`이어서 되돌리기`) renames it once its
//! torrent is complete, whatever device number the file system gives its
//! file after a restart.

use super::fixtures::*;
use super::*;
use crate::test_world::show_hash;
use trss_core::commands::CommandState;

/// A torrent still downloading is not renamed: its file waits, `pending`
/// with why, and `이어서 되돌리기` renames it once it is complete.
#[tokio::test]
async fn a_torrent_still_downloading_waits_and_is_renamed_when_carried_on() {
    let (s, rule) = third_season_received().await;
    s.tr.unfinish(&show_hash(50));

    let finished = s.undo(&rule, "undo-0103-a", -48).await;

    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(finished.outcome.result, PAUSED, "{finished:?}");
    assert_eq!(s.torrent_names(), ["Show S03E02.mkv", "Show S03E25.mkv"]);
    assert_eq!(
        s.undo_files("undo-0103-a").await,
        [
            file("Show S03E01.mkv", "Show S03E25.mkv", "renamed"),
            file("Show S03E02.mkv", "Show S03E26.mkv", "pending"),
        ]
    );
    let reason = s.undo_reason("undo-0103-a", 1).await;
    assert!(reason.contains("다 받은 뒤 이어서"), "{reason}");

    s.tr.finish(&show_hash(50));
    let finished = s.undo(&rule, "undo-0103-b", -48).await;
    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(finished.outcome.result, UNDONE, "{finished:?}");
    assert_eq!(s.torrent_names(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    assert_eq!(s.content("Show S03E26.mkv"), b"video 50");
}

/// A torrent whose file is not written yet when the undo is planned (still
/// a partial file) waits too, and is renamed once complete.
#[tokio::test]
async fn a_torrent_without_its_file_yet_waits_and_is_renamed_when_carried_on() {
    let (s, rule) = third_season_received().await;
    s.tr.unfinish(&show_hash(50));
    let (whole, part) = (
        s.season3().join("Show S03E02.mkv"),
        s.season3().join("Show S03E02.mkv.part"),
    );
    std::fs::rename(&whole, &part).unwrap();

    let finished = s.undo(&rule, "undo-0104-a", -48).await;

    assert_eq!(finished.outcome.result, PAUSED, "{finished:?}");
    assert_eq!(s.undo_files("undo-0104-a").await[1].2, "pending");

    // The download ends.
    std::fs::rename(&part, &whole).unwrap();
    s.tr.finish(&show_hash(50));
    let finished = s.undo(&rule, "undo-0104-b", -48).await;
    assert_eq!(finished.outcome.result, UNDONE, "{finished:?}");
    assert_eq!(s.torrent_names(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    assert_eq!(s.on_disk(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
}

/// A torrent that waited to finish is renamed by Transmission after a restart
/// that mounted the folder again: the file at its name is still the one
/// planned.
#[tokio::test]
async fn a_waiting_torrent_is_renamed_whatever_device_number_its_file_was_mounted_with() {
    let (s, rule) = third_season_received().await;
    s.tr.unfinish(&show_hash(50));
    let finished = s.undo(&rule, "undo-0306-a", -48).await;
    assert_eq!(finished.outcome.result, PAUSED, "{finished:?}");

    s.mounted_again();
    s.tr.finish(&show_hash(50));
    let finished = s.undo(&rule, "undo-0306-b", -48).await;

    assert_eq!(finished.outcome.result, UNDONE, "{finished:?}");
    assert_eq!(s.torrent_names(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    assert_eq!(s.content("Show S03E26.mkv"), b"video 50");
}
