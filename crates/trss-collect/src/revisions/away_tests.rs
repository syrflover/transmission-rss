//! A rule's folder that is away decides nothing: a mount that is not there,
//! a rule archived on purpose, and the week after which an away folder ends
//! a failure ([`advance`]; `docs/specs/collection.md`, 영상 수정본의 대체;
//! ticket 0033). The cases run through [`World::cycle`] against the fake
//! Transmission, which acts on the media folder the way Transmission does.

use super::fixtures::*;
use super::*;
use crate::test_world::{read, World};

const DAY: i64 = 24 * 60 * 60 * 1000;

/// `14v2`'s torrent completes while the rule's folder is away: the new video
/// is not "gone", the replacement waits, and goes on once the folder is back.
#[tokio::test]
async fn a_revision_completing_while_its_folder_is_away_waits_for_it() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    let elsewhere = s.folder_away();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    s.folder_back(&elsewhere);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// `14v2` waits, removing, for `14`'s file to go after its torrent was
/// removed, when the folder is away: that is not the old video gone, and
/// the replacement keeps waiting instead of going on to a rename that would
/// find `14` back under the name.
#[tokio::test]
async fn a_removal_waiting_while_its_folder_is_away_keeps_waiting() {
    let s = World::new().await;
    s.removal_waits().await;
    let elsewhere = s.folder_away();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removing);

    s.folder_back(&elsewhere);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removing);
    std::fs::remove_file(s.file(EPISODE_NAME)).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// A failure that keeps both files is not "resolved" by its folder being
/// away: it stays a failure.
#[tokio::test]
async fn a_failure_whose_folder_is_away_stays_a_failure() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, b"not what the name says");
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);

    let elsewhere = s.folder_away();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    s.folder_back(&elsewhere);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert_eq!(s.failures().await.len(), 1);
}

/// `14v2` is still downloading when the rule's folder goes away and does not
/// come back. A week after the first look found it away, the replacement,
/// which still holds its torrent, stays but is a `받기 실패` that says the
/// work's folder is not seen. Once the folder is back that goes, and the
/// replacement goes on.
#[tokio::test]
async fn a_replacement_whose_folder_is_away_for_a_week_says_so() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    let elsewhere = s.folder_away();
    s.cycle().await;
    s.advance(7 * DAY - 1);
    s.cycle().await;
    assert!(s.failures().await.is_empty());

    s.advance(1);
    s.cycle().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Receiving);
    let failures = s.failures().await;
    let failure = one_failure(&failures);
    assert!(
        failure
            .reason
            .as_deref()
            .unwrap()
            .contains("작품 폴더가 보이지 않아요"),
        "{failure:?}"
    );
    assert!(s.tr.torrents().iter().any(|t| t.hash == NEW_HASH));

    s.folder_back(&elsewhere);
    s.cycle().await;
    assert!(s.failures().await.is_empty());
    assert_eq!(s.row_of(&v2()).await.reason, None);
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// A failure that keeps both files, whose folder is away a week after the
/// first look found it away, is cleared: it leaves the list, and stays
/// cleared when the folder comes back. A folder that came back in between
/// starts the week over.
#[tokio::test]
async fn a_failure_whose_folder_is_away_for_a_week_is_cleared() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, b"not what the name says");
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);

    let elsewhere = s.folder_away();
    s.cycle().await;
    s.advance(4 * DAY);
    s.folder_back(&elsewhere);
    s.cycle().await;
    let elsewhere = s.folder_away();
    s.cycle().await;
    s.advance(4 * DAY);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert_eq!(s.failures().await.len(), 1);

    s.advance(3 * DAY);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Cleared);
    assert!(s.failures().await.is_empty());
    s.folder_back(&elsewhere);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Cleared);
    assert_eq!(s.names(), sorted(vec![EPISODE_NAME.to_owned(), v2()]));
}

/// A replacement that ended with no video under the episode name, whose
/// folder is away for a week, is no failure any more.
#[tokio::test]
async fn an_ended_replacement_whose_folder_is_away_for_a_week_is_cleared() {
    let s = World::new().await;
    s.v2_waits_for_its_name().await;
    std::fs::remove_file(s.file(&v2())).unwrap();
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.failures().await.len(), 1);

    let _elsewhere = s.folder_away();
    s.cycle().await;
    s.advance(7 * DAY);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert!(s.failures().await.is_empty());
}

/// The rule of a failure that keeps both files is archived for ten days:
/// its work folder is in the archive folder on purpose, not a mount that
/// went away, so the failure is not cleared, and it is there as before once
/// the rule is restored.
#[tokio::test]
async fn a_failure_of_a_rule_archived_for_ten_days_is_there_after_its_restore() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, b"not what the name says");
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);

    let archived = s.archive().await;
    s.cycle().await;
    s.advance(10 * DAY);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);

    s.restore(&archived).await;
    s.cycle().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Failed, "{row:?}");
    assert_eq!(row.folder_away_since, None);
    assert_eq!(s.failures().await.len(), 1);
    // The week starts with the first look that finds the folder away after
    // the restore.
    let elsewhere = s.folder_away();
    s.cycle().await;
    s.advance(7 * DAY - 1);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    s.folder_back(&elsewhere);
}
