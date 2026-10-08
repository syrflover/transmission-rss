//! Several revisions of one episode in flight: which one takes the name, and
//! what a lower one does once a higher one has failed, been abandoned or
//! replaced the video (`docs/specs/collection.md`, 영상 수정본의 대체;
//! tickets 0025, 0033). The cases run through [`World::cycle`] against the
//! fake Transmission.

use super::fixtures::*;
use super::*;
use crate::test_world::{read, World};

/// Two revisions of one release, both selected while `14` is in place: the
/// higher one finishes first and takes the episode name. The lower one,
/// finishing later, must not remove it (its row was decided against `14`).
#[tokio::test]
async fn a_lower_revision_finishing_after_a_higher_one_never_replaces_it() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.tr.content_on_add(V3_HASH, V3_BYTES);
    s.tr.unfinished_on_add(NEW_HASH);
    s.tr.unfinished_on_add(V3_HASH);
    s.cycle().await;
    assert_eq!((s.added(NEW_HASH), s.added(V3_HASH)), (1, 1));

    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);

    s.complete(NEW_HASH);
    s.cycle().await;
    s.cycle().await;

    assert_eq!(
        read(&s.file(EPISODE_NAME)),
        V3_BYTES,
        "the newest video stays"
    );
    assert!(s.tr.torrents().iter().any(|t| t.hash == V3_HASH));
    assert!(!s.removed(V3_HASH));
    assert_eq!(s.names(), sorted(vec![EPISODE_NAME.to_owned(), v2()]));
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);
    assert!(s.failures().await.is_empty());
}

/// Both revisions finish in the same cycle: whichever the worker looks at
/// first, the higher one ends under the episode name.
#[tokio::test]
async fn two_revisions_finishing_together_leave_the_higher_one() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.tr.content_on_add(V3_HASH, V3_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.complete(V3_HASH);
    s.cycle().await;
    s.cycle().await;

    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert!(s.tr.torrents().iter().any(|t| t.hash == V3_HASH));
    assert!(s.failures().await.is_empty());
}

/// `14v3` failed before it was received, then `14v2` removed the old video
/// and its rename is still to go through: the empty episode name is not the
/// person having resolved `14v3`'s failure.
#[tokio::test]
async fn a_higher_revision_not_received_is_not_cleared_while_a_lower_one_is_renamed() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.tr.unfinished_on_add(NEW_HASH);
    s.tr.unfinished_on_add(V3_HASH);
    s.cycle().await;
    // `14v3` stops: its torrent is taken out and it leaves the feed.
    s.tr.remove(V3_HASH);
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);

    s.complete(NEW_HASH);
    s.tr.reject_rename_of(NEW_HASH, Some("busy"));
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);

    s.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);
}

/// `14v3` replaces the video and skips `14v2` in the same pass; the pass then
/// reaches `14v2` from what it read before. Its stale look (a CRC32 that
/// does not match) must not turn the skip into a failure.
#[tokio::test]
async fn a_revision_skipped_earlier_in_the_same_pass_stays_skipped() {
    let s = World::new().await;
    s.received_v1().await;
    // `14v3`'s row comes first, so the pass looks at it first.
    s.feed(&[(V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(V3_HASH, V3_BYTES);
    s.tr.unfinished_on_add(V3_HASH);
    s.cycle().await;
    s.feed(&[(NEW_HASH, &v2()), (V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, b"not what the name says");
    s.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    s.complete(V3_HASH);
    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);
    assert!(s.failures().await.is_empty());
}

/// The same when the higher revision fails after it was received, on its
/// CRC32 check (a failure that is final).
#[tokio::test]
async fn a_lower_revision_skipped_for_a_higher_one_whose_check_fails_replaces_the_video() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.tr.content_on_add(V3_HASH, b"not what the name says");
    s.tr.unfinished_on_add(NEW_HASH);
    s.tr.unfinished_on_add(V3_HASH);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);

    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// `14v3` fails after `14v2`, skipped for it, left the feed and the cycle
/// took its torrent out of Transmission: `14v2` comes back but cannot be
/// checked, so it is a failure before it was received (with `다시 받기`),
/// written once; `14` stays and nothing is added or removed again.
#[tokio::test]
async fn a_lower_revision_whose_item_is_gone_when_the_higher_one_fails_stays_a_failure() {
    let s = World::new().await;
    s.v2_skipped_for_v3().await;
    // `14v2` leaves the feed; the cycle takes its finished torrent out.
    s.feed(&[(V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.cycle().await;
    assert!(!s.tr.torrents().iter().any(|t| t.hash == NEW_HASH));
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);

    s.tr.remove(V3_HASH);
    s.feed(&[(OLD_HASH, &v1())]);
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);
    s.cycle().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Failed);
    assert!(row.not_received(), "{row:?}");

    let adds = (s.added(NEW_HASH), s.added(V3_HASH));
    for _ in 0..3 {
        s.advance(60_000);
        s.cycle().await;
    }
    assert_eq!(s.row_of(&v2()).await, row, "written once");
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);
    assert_eq!((s.added(NEW_HASH), s.added(V3_HASH)), adds);
    assert!(!s.removed(OLD_HASH));
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    let failures = s.failures().await;
    assert!(
        failures.iter().any(|f| f.item_id == row.item_id),
        "14v2 is a failure"
    );
    assert!(s.can_retry(&v2()).await);
}

/// `14v2` was skipped because `14v3` was on its way. `14v3` removed `14`'s
/// torrent, whose file Transmission left, and then lost its own video: it
/// ended watching `14`'s file, and `14v2` started over. `14v2` removes that
/// file and waits a cycle for its name. The name is empty only on its way to
/// `14v2`, so `14v3` does not say the episode has no video.
#[tokio::test]
async fn a_lower_revision_taking_the_name_from_a_watched_old_file_is_no_failure_of_the_watcher() {
    let s = World::new().await;
    s.v2_skipped_for_v3().await;
    s.tr.keep_data_on_remove_of(OLD_HASH);
    s.complete(V3_HASH);
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Removing);
    std::fs::remove_file(s.file(&v3())).unwrap();
    s.cycle().await;
    s.cycle().await;
    let row = s.row_of(&v3()).await;
    assert_eq!(row.state.code(), "abandoned", "{row:?}");
    assert_eq!(row.reason.as_deref(), Some(OLD_FILE_WATCHED));
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    s.tr.reject_rename_of(NEW_HASH, Some("busy"));
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);
    assert_eq!(s.names(), vec![v2()]);
    let row = s.row_of(&v3()).await;
    assert_eq!(row.reason.as_deref(), Some(OLD_FILE_WATCHED), "{row:?}");

    s.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    s.cycle().await;
    assert!(s.failures().await.is_empty());
}

/// `14v2` was skipped because `14v3` was on its way. `14v3` removes `14` and
/// then loses its video before it takes the name, so it is abandoned: `14v2`
/// starts over as if `14v3` had failed, and puts its video under the episode
/// name.
#[tokio::test]
async fn a_lower_revision_skipped_for_an_abandoned_one_replaces_the_video() {
    let s = World::new().await;
    s.v2_skipped_for_v3().await;
    s.tr.reject_rename_of(V3_HASH, Some("busy"));
    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Removed);
    assert_eq!(s.names(), sorted(vec![v2(), v3()]));

    std::fs::remove_file(s.file(&v3())).unwrap();
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await.code(), "abandoned");
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}
