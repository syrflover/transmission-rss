//! A revision chosen in a past episode search (`receive_past::run`; tickets
//! 0026, 0033): it is decided as a cycle decides a revision of a video the
//! folder holds ([`crate::commands::receive_past`] `decide_revision`). The
//! new torrent keeps its name until it is checked, the folder's own video
//! stays until then, and a revision that a higher one replaced is not added.
//! The cases run through [`World::past`] and [`World::cycle`] against the fake
//! Transmission.

use trss_core::commands::CommandState;

use super::fixtures::*;
use super::*;
use crate::{
    commands::receive_past::ReceivePast,
    test_world::{read, World},
};

const CMD: &str = "0b7d5a44-6c1e-4c62-9a6a-3f0c1d2e4b55";
const SECOND_CMD: &str = "1e2d3c4b-0000-4000-8000-000000000002";

impl World {
    /// The result of a search the person chooses: the release `title`, whose
    /// torrent has `hash` and gives `bytes` once added (left unfinished).
    async fn chosen(&self, title: &str, hash: &str, bytes: &[u8]) -> ReceivePast {
        self.tr.content_on_add(hash, bytes);
        self.tr.unfinished_on_add(hash);
        self.past_result(&self.rule_of(0).await, hash, title)
    }
}

/// The revision of a video the folder holds, received from the search: the new
/// torrent keeps its name until checked, then replaces the old video.
#[tokio::test]
async fn a_revision_chosen_in_the_preview_replaces_the_video_after_it_is_received_and_checked() {
    let s = World::new().await;
    s.received_v1().await;
    let result = s.chosen(&v2(), NEW_HASH, NEW_BYTES).await;

    let finished = s.past(&result, CMD).await.unwrap();

    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(finished.outcome.result, "received", "{finished:?}");
    // Not renamed after the add; both videos are there until it is checked.
    assert!(!s.renamed_onto_episode(NEW_HASH));
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert_eq!(s.item(&v2()).await.result, HistoryResult::Received);

    // Received and checked: the old video goes and the new takes its name.
    s.complete(NEW_HASH);
    s.cycle().await;
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert!(s.failures().await.is_empty());
}

/// Two revisions of an episode chosen from a search: the lower one finishing
/// first does not take the episode name, and the higher one replaces the video.
#[tokio::test]
async fn two_searched_revisions_of_an_episode_leave_the_higher_one() {
    let s = World::new().await;
    s.received_v1().await;
    let v3_result = s.chosen(&v3(), V3_HASH, V3_BYTES).await;
    let v2_result = s.chosen(&v2(), NEW_HASH, NEW_BYTES).await;
    for (result, id) in [(&v3_result, CMD), (&v2_result, SECOND_CMD)] {
        let finished = s.past(result, id).await.unwrap();
        assert_eq!(finished.outcome.result, "received", "{finished:?}");
    }

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(
        read(&s.file(EPISODE_NAME)),
        OLD_BYTES,
        "the lower revision does not take the name"
    );
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);

    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);
    // v2 keeps its own name; only the episode name was replaced.
    assert_eq!(read(&s.file(&v2())), NEW_BYTES);
}

/// A revision chosen from a search while the feed's lower revision is on its
/// way: the feed's one finishing first is skipped and the searched one
/// replaces the video.
#[tokio::test]
async fn a_searched_revision_higher_than_the_feeds_open_one_is_the_one_that_replaces() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    let result = s.chosen(&v3(), V3_HASH, V3_BYTES).await;
    let finished = s.past(&result, CMD).await.unwrap();
    assert_eq!(finished.outcome.result, "received", "{finished:?}");
    assert_eq!(s.state_of(&v3()).await, RevisionState::Receiving);

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);

    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
}

/// A lower revision chosen from a search after a higher one replaced the
/// video is not added.
#[tokio::test]
async fn a_searched_revision_lower_than_the_one_that_replaced_the_video_is_not_added() {
    let s = World::new().await;
    s.received_v1().await;
    let v3_result = s.chosen(&v3(), V3_HASH, V3_BYTES).await;
    let finished = s.past(&v3_result, CMD).await.unwrap();
    assert_eq!(finished.outcome.result, "received", "{finished:?}");
    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);

    let v2_result = s.chosen(&v2(), NEW_HASH, NEW_BYTES).await;
    let finished = s.past(&v2_result, SECOND_CMD).await.unwrap();

    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(finished.outcome.result, "duplicate", "{finished:?}");
    assert_eq!(s.added(NEW_HASH), 0);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
}

/// The replacement row of a searched revision is written with the item's
/// `received`: when it cannot be written the command runs again, instead of
/// ending with the revision received and nothing to replace the video.
#[tokio::test]
async fn a_searched_revision_whose_replacement_is_not_written_runs_again_and_replaces() {
    let s = World::new().await;
    s.received_v1().await;
    let result = s.chosen(&v2(), NEW_HASH, NEW_BYTES).await;
    let command = s.start_past(&result, CMD).await;
    s.sql(
        "CREATE TRIGGER no_row BEFORE INSERT ON video_revisions
           BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );

    let ran = s.run_command(&command).await;

    assert!(ran.is_err(), "{ran:?}");
    assert_eq!(s.command(CMD).await.state, CommandState::Running);
    s.sql("DROP TRIGGER no_row;");
    // Runs it again, as the worker starts a command it left running.
    let again = s.claim().await;
    let finished = s.run_command(&again).await.unwrap();
    assert_eq!(finished.outcome.result, "received", "{finished:?}");
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
}

/// A lower revision that a higher one replaced is offered again once the
/// person removed the video and the torrent, and is still not received: the
/// folder is empty, but the release was replaced.
#[tokio::test]
async fn a_lower_revision_that_a_higher_one_replaced_is_not_received_again_when_the_folder_is_empty(
) {
    let s = World::new().await;
    s.received_v1().await;
    let v3_result = s.chosen(&v3(), V3_HASH, V3_BYTES).await;
    let finished = s.past(&v3_result, CMD).await.unwrap();
    assert_eq!(finished.outcome.result, "received", "{finished:?}");
    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);

    // The person deletes the video and removes the torrent: the folder has no
    // episode 14, and v1, which v3 replaced, is gone from the work too.
    s.tr.remove(V3_HASH);
    std::fs::remove_file(s.file(EPISODE_NAME)).unwrap();
    s.tr.clear_calls();
    let v1_result = s.past_result(&s.rule_of(0).await, OLD_HASH, &v1());
    let finished = s.past(&v1_result, SECOND_CMD).await.unwrap();

    assert_eq!(finished.outcome.result, "duplicate", "{finished:?}");
    assert!(s.tr.calls_of("torrent-add").is_empty());
}
