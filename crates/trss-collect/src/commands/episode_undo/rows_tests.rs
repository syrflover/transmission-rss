//! `되돌리기` and the video revision rows of the episodes it renames: a
//! replacement under way holds an undo up, a row that ended follows its
//! episode's file to the new name, and a row of the new name's own is never
//! merged with it.

use super::fixtures::*;
use super::*;
use crate::{store::revisions::OLD_FILE_WATCHED, test_world::show_hash};
use trss_core::commands::CommandState;

#[tokio::test]
async fn an_undo_waits_for_a_revision_replacement_under_way_and_moves_finished_ones() {
    let (s, rule) = third_season_received().await;
    // A replacement of `S03E02` by a higher revision is under way.
    let row = s
        .revision_row(&rule, "Show - 50", "Show S03E02.mkv", true)
        .await;

    // Refused before anything changes.
    let finished = s.undo(&rule, "undo-0004-a", -48).await;
    assert_eq!(finished.state, CommandState::Failed, "{finished:?}");
    let reason = finished.outcome.reason.unwrap();
    assert!(reason.contains("Show S03E02.mkv"), "{reason}");
    let stored = s.stored_rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (-48, true));
    assert_eq!(s.torrent_names(), ["Show S03E01.mkv", "Show S03E02.mkv"]);
    assert_eq!(s.mark_of(&rule).await.previous, Some(-24));

    // Once the replacement has ended, the undo goes through and the row
    // follows its episode's file to the new name.
    s.sql(&format!(
        "UPDATE video_revisions SET state = 'done' WHERE id = {row}"
    ));
    let finished = s.undo(&rule, "undo-0004-b", -48).await;
    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(s.torrent_names(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    assert_eq!(s.rows_of("Show S03E26.mkv").await, [row]);
    assert_eq!(s.rows_of("Show S03E02.mkv").await, Vec::<i64>::new());
}

/// A replacement that ended beside its episode's old file, which the worker
/// still watches (an abandoned row with a reason), acts on no file: it does
/// not hold up an undo, and its row follows the file to the new name, where
/// the worker keeps watching it, so no cycle takes the old name's emptiness
/// for the file gone.
#[tokio::test]
async fn an_undo_moves_an_ended_replacement_that_watches_its_file() {
    let (s, rule) = third_season_received().await;
    let item = s.item_containing("Show - 50").await;
    let row = s
        .revision_row(&rule, "Show - 50", "Show S03E02.mkv", true)
        .await;
    rusqlite::Connection::open(s.db_path())
        .unwrap()
        .execute(
            "UPDATE video_revisions SET state = 'abandoned', reason = ?2 WHERE id = ?1",
            rusqlite::params![row, OLD_FILE_WATCHED],
        )
        .unwrap();

    let finished = s.undo(&rule, "undo-0004-c", -48).await;
    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(s.on_disk(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    assert_eq!(s.rows_of("Show S03E26.mkv").await, [row]);

    s.cycle_later().await;
    let row = s.ctx.revisions.by_item(item.id).await.unwrap().unwrap();
    assert_eq!(row.reason.as_deref(), Some(OLD_FILE_WATCHED));
    assert!(s.failures().await.is_empty());
}

/// Revision rows the new name has already are never merged with the file's.
#[tokio::test]
async fn a_new_name_that_has_revision_rows_of_its_own_is_not_taken() {
    let (s, rule) = third_season_received().await;
    // An ended replacement left a row for `S03E26`, whose file is gone.
    let row = s
        .revision_row(&rule, "Show - 49", "Show S03E26.mkv", false)
        .await;

    let finished = s.undo(&rule, "undo-0202-a", -48).await;

    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(
        s.undo_files("undo-0202-a").await,
        [
            file("Show S03E01.mkv", "Show S03E25.mkv", "renamed"),
            file("Show S03E02.mkv", "Show S03E26.mkv", "kept"),
        ]
    );
    assert_eq!(s.torrent_names(), ["Show S03E02.mkv", "Show S03E25.mkv"]);
    assert_eq!(s.rows_of("Show S03E26.mkv").await, [row]);
}

/// A file that waited moves only the revision rows of its old name that are
/// its own or older than the undo: a row a cycle wrote since, under the
/// restored value, for another episode of that name stays.
#[tokio::test]
async fn a_waiting_file_moves_only_the_revision_rows_that_were_its_own() {
    let (s, rule) = third_season_received().await;
    // A row of `S03E02` older than the undo (of another item: its own rows
    // move whoever wrote them).
    let older = s
        .revision_row(&rule, "Other - 01", "Show S03E02.mkv", false)
        .await;
    s.tr.unfinish(&show_hash(50));
    let finished = s.undo(&rule, "undo-0401-a", -48).await;
    assert_eq!(finished.outcome.result, PAUSED, "{finished:?}");

    // While `S03E02` waits, a cycle writes a row of that name for another
    // episode.
    s.advance(1_000);
    let since = s
        .revision_row(&rule, "Show - 49", "Show S03E02.mkv", false)
        .await;
    s.tr.finish(&show_hash(50));
    let finished = s.undo(&rule, "undo-0401-b", -48).await;
    assert_eq!(finished.outcome.result, UNDONE, "{finished:?}");

    assert_eq!(s.rows_of("Show S03E26.mkv").await, [older]);
    assert_eq!(s.rows_of("Show S03E02.mkv").await, [since]);
}
