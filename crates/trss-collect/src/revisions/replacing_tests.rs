//! A replacement carried through its states by [`advance`]: the new video
//! received and checked, the old one removed, the new one named as the
//! episode, and what each step that cannot go on leaves (`docs/specs/
//! collection.md`, 영상 수정본의 대체; tickets 0025, 0033). The cases run
//! through [`World::cycle`] against the fake Transmission, which acts on the
//! media folder the way Transmission does.

use super::fixtures::*;
use super::*;
use crate::{
    store::history::{HistoryResult, Observation},
    test_world::{crc, magnet, read, World},
};
use trss_core::files::testing;
use trss_transmission::fake::FakeTorrent;

/// Before ticket 0025 the cycle renamed `14v2` onto the episode name right
/// after adding it. With libtransmission's rename (the target exists, so the
/// file stays and the answer is `success`) that left both files, the old
/// video under the episode name and the new one under its release name, and
/// the new torrent naming the old video's file. The first part of this test,
/// run against the worker of `d4129fa` (asserting no rename after the add),
/// failed with both files in the folder
/// (`["Show S01E14.mkv", "[SubsPlease] Show - 14v2 (1080p) [8F2EFECC].mkv"]`)
/// and the new torrent named `Show S01E14.mkv`.
#[tokio::test]
async fn row_1_a_received_episode_is_replaced_by_its_revision_once_it_is_received_and_checked() {
    let s = World::new().await;
    s.received_v1().await;

    // `14v2` appears; it is still downloading after the cycle that adds it.
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.added(NEW_HASH), 1, "received automatically");
    assert!(
        !s.renamed_onto_episode(NEW_HASH),
        "not renamed after the add"
    );
    assert_eq!(s.names(), vec![EPISODE_NAME.to_owned(), v2()]);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert_eq!(s.tr.torrent(NEW_HASH).name, v2());
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    // Still downloading: nothing changes.
    s.cycle().await;
    assert_eq!(s.names(), vec![EPISODE_NAME.to_owned(), v2()]);

    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.tr.torrent(NEW_HASH).name, EPISODE_NAME);
    assert!(s.tr.torrents().iter().all(|t| t.hash != OLD_HASH));
    let removes = s.tr.calls_of("torrent-remove");
    assert_eq!(removes.len(), 1);
    assert_eq!(removes[0].args["ids"], serde_json::json!([OLD_HASH]));
    assert_eq!(removes[0].args["delete-local-data"], true);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.item(&v2()).await.result, HistoryResult::Received);

    // The version line, quietly; no failure.
    let row = s.row_of(&v2()).await;
    assert_eq!((row.old_version, row.new_version), (Some(1), 2));
    assert_eq!(row.replaced_at, Some(s.now()));
    assert!(s.failures().await.is_empty());

    // `14` is still in the feed: it is not received again.
    let adds = s.added(OLD_HASH);
    s.cycle().await;
    assert_eq!(s.added(OLD_HASH), adds);
    assert!(s.tr.torrents().iter().all(|t| t.hash != OLD_HASH));
    assert_eq!(s.names(), vec![EPISODE_NAME]);
}

#[tokio::test]
async fn row_2_a_revision_whose_crc_differs_from_its_name_leaves_the_old_video() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, b"not what the name says");
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME.to_owned(), v2()]);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(s.tr.torrents().iter().any(|t| t.hash == OLD_HASH));
    assert!(s.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);

    // `받기 실패` with why; both files stay, the new one under its received
    // name, and nothing was replaced.
    let failures = s.failures().await;
    let failure = one_failure(&failures);
    assert!(
        failure.reason.as_deref().unwrap().contains("CRC32"),
        "{failure:?}"
    );
    assert_eq!(failure.episode_name, EPISODE_NAME);
    assert_eq!(failure.received_name.as_deref(), Some(v2().as_str()));
    assert_eq!(failure.claimed_at, None);
    assert_eq!(failure.replaced_at, None);

    // The person deletes the new file: the failure goes away.
    std::fs::remove_file(s.file(&v2())).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Cleared);
    assert!(s.failures().await.is_empty());
}

#[tokio::test]
async fn row_2_a_revision_whose_download_stops_leaves_the_old_video() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    // The torrent is taken out of Transmission before it finished.
    s.tr.remove(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(s.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    // Neither file changed: the old video is kept and the new one was not
    // received.
    let failures = s.failures().await;
    let failure = one_failure(&failures);
    assert!(failure.not_received(), "{failure:?}");
    assert_eq!(failure.claimed_at, None);
    // While it stays in the feed the item is received again, as any item a
    // rule picked and Transmission does not hold, and the replacement goes on.
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    assert_eq!(s.added(NEW_HASH), 2);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    assert!(s.failures().await.is_empty());
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

#[tokio::test]
async fn row_4_an_old_torrent_that_cannot_be_removed_keeps_both_files() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.tr.reject_remove_of(OLD_HASH, Some("permission denied"));
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME.to_owned(), v2()]);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(!s.renamed_onto_episode(NEW_HASH));
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    let failures = s.failures().await;
    let failure = one_failure(&failures);
    assert!(failure
        .reason
        .as_deref()
        .unwrap()
        .contains("permission denied"));
    assert_eq!(failure.received_name.as_deref(), Some(v2().as_str()));
}

#[tokio::test]
async fn row_5_an_old_file_of_no_torrent_that_cannot_be_deleted_keeps_both_files() {
    let s = World::new().await;
    s.received_v1().await;
    // `14` left the feed and its torrent went (its data stays, as a cycle
    // removes departed torrents).
    s.tr.remove(OLD_HASH);
    s.feed(&[(NEW_HASH, &v2())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);

    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&s.season, std::fs::Permissions::from_mode(0o555)).unwrap();
    let writable = std::fs::write(s.season.join(".probe"), b"").is_ok();
    if !writable {
        s.cycle().await;
    }
    std::fs::set_permissions(&s.season, std::fs::Permissions::from_mode(0o755)).unwrap();
    if writable {
        // Root ignores the folder's mode, so the failure cannot be made here:
        // run as root this test checks nothing and passes.
        eprintln!("skipped: the folder stays writable for this user");
        return;
    }

    assert_eq!(s.names(), vec![EPISODE_NAME.to_owned(), v2()]);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(!s.renamed_onto_episode(NEW_HASH));
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    let failures = s.failures().await;
    let failure = one_failure(&failures);
    assert!(
        failure
            .reason
            .as_deref()
            .unwrap()
            .contains("지우지 못했어요"),
        "{failure:?}"
    );
}

#[tokio::test]
async fn row_6_an_old_video_in_a_batch_torrent_is_not_replaced() {
    let s = World::new().await;
    s.untracked_video(OLD_BYTES).await;
    std::fs::write(s.file("Show S01E13.mkv"), b"episode 13").unwrap();
    let batch = "4444000000000000000000000000000000000001";
    s.tr.preload(
        FakeTorrent::new(batch, "Show 01-14")
            .in_dir(&s.season)
            .files(&["Show S01E13.mkv", EPISODE_NAME])
            .status(6),
    );
    s.feed(&[(NEW_HASH, &v2())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(
        s.names(),
        vec!["Show S01E13.mkv".to_owned(), EPISODE_NAME.to_owned(), v2()]
    );
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(s.tr.torrents().iter().any(|t| t.hash == batch));
    assert!(s.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    let failures = s.failures().await;
    let failure = one_failure(&failures);
    assert!(
        failure
            .reason
            .as_deref()
            .unwrap()
            .contains("파일 여러 개를 담은 토렌트"),
        "{failure:?}"
    );
}

#[tokio::test]
async fn row_7_a_restart_after_the_old_video_was_removed_finishes_the_rename() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    // The rename does not go through, as for a worker stopped right after
    // the old video went.
    s.tr.reject_rename_of(NEW_HASH, Some("busy"));
    s.cycle().await;

    assert_eq!(s.names(), vec![v2()]);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);
    // The old video is removed and the new one waits under its received
    // name; the episode has no file under its name now, and its row says why.
    let failures = s.failures().await;
    let failure = one_failure(&failures);
    assert_eq!(failure.received_name.as_deref(), Some(v2().as_str()));
    assert!(failure.claimed_at.is_some(), "{failure:?}");
    assert!(failure.reason.is_some(), "{failure:?}");

    // The next cycle, as after a restart, carries on from the row.
    s.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert!(
        s.tr.torrents().iter().all(|t| t.hash != OLD_HASH),
        "the old video is not brought back"
    );
    assert!(s.failures().await.is_empty());
}

#[tokio::test]
async fn row_7_a_taken_episode_name_is_never_renamed_over() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.tr.reject_rename_of(NEW_HASH, Some("busy"));
    s.cycle().await;
    // Someone puts a file under the episode name before the rename.
    std::fs::write(s.file(EPISODE_NAME), b"someone else's").unwrap();
    s.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME.to_owned(), v2()]);
    assert_eq!(read(&s.file(EPISODE_NAME)), b"someone else's");
    assert_eq!(read(&s.file(&v2())), NEW_BYTES);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);
    let failures = s.failures().await;
    assert!(one_failure(&failures)
        .reason
        .as_deref()
        .unwrap()
        .contains("다른 파일"));
}

#[tokio::test]
async fn a_revision_whose_torrent_reports_a_local_error_goes_on_once_it_clears() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    s.tr.set_local_error(NEW_HASH, Some("No space left on device"));
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    let failures = s.failures().await;
    assert!(one_failure(&failures)
        .reason
        .as_deref()
        .unwrap()
        .contains("No space left"));

    s.tr.set_local_error(NEW_HASH, None);
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert!(s.failures().await.is_empty());
}

#[tokio::test]
async fn a_revision_received_outside_the_rule_folder_stays_a_failure() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    let elsewhere = s.season.parent().unwrap().join("Elsewhere");
    s.tr.relocate(NEW_HASH, &elsewhere);
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert_eq!(s.failures().await.len(), 1);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
}

/// A new video's torrent that holds several files is not one the
/// replacement can name as the episode: both videos stay.
#[tokio::test]
async fn a_revision_whose_torrent_holds_several_files_does_not_replace_the_old_video() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    s.tr.set_files(NEW_HASH, &[v2().as_str(), "Extras.mkv"]);
    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    let failures = s.failures().await;
    assert_eq!(
        one_failure(&failures).reason.as_deref(),
        Some(SEVERAL_FILES)
    );
}

/// A new video's torrent whose one file sits inside the torrent's folder is
/// not replaced either, and the reason names the folder rather than several
/// files.
#[tokio::test]
async fn a_revision_whose_one_file_is_inside_a_folder_does_not_replace_the_old_video() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    s.tr.set_files(NEW_HASH, &[format!("Show/{}", v2()).as_str()]);
    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    let failures = s.failures().await;
    assert_eq!(one_failure(&failures).reason.as_deref(), Some(IN_A_FOLDER));
}

/// Transmission names the old video's folder another way (through a
/// symbolic link): the torrent is still found to hold the file, and removed
/// with it, instead of the file being deleted under it.
#[tokio::test]
async fn an_old_torrent_whose_folder_is_spelled_another_way_is_removed_with_its_file() {
    let s = World::new().await;
    s.received_v1().await;
    let link = s.season.parent().unwrap().join("Season 01 link");
    std::os::unix::fs::symlink(&s.season, &link).unwrap();
    s.tr.set_download_dir(OLD_HASH, &link);
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert!(s.removed(OLD_HASH), "the old torrent is removed");
    assert!(s.tr.torrents().iter().all(|t| t.hash != OLD_HASH));
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// Two torrents name the old video's file: removing either could take the
/// other's data, so the old video is not replaced.
#[tokio::test]
async fn an_old_video_two_torrents_hold_is_not_removed() {
    let s = World::new().await;
    s.received_v1().await;
    s.tr.preload(
        FakeTorrent::new(OTHER_HASH, EPISODE_NAME)
            .in_dir(&s.season)
            .status(6),
    );
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert!(s.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(s.tr.torrents().iter().any(|t| t.hash == OLD_HASH));
}

/// A revision whose received file is empty does not replace the old video,
/// even though the name's CRC32 (`00000000`) matches.
#[tokio::test]
async fn an_empty_revision_does_not_replace_the_old_video() {
    let s = World::new().await;
    s.received_v1().await;
    let empty = release("v2", Some(&crc(b"")));
    s.feed(&[(NEW_HASH, &empty), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, b"");
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(s.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.state_of(&empty).await, RevisionState::Failed);
}

/// The rename went through but `done` was not written (the worker stopped, or
/// the write failed): the next look finds the new video under the episode
/// name and finishes, instead of failing on a name its own video holds.
#[tokio::test]
async fn a_rename_whose_done_was_not_written_finishes_on_the_next_look() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.sql(
        "CREATE TRIGGER no_done BEFORE UPDATE OF state ON video_revisions
         WHEN NEW.state = 'done' BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    s.cycle().await;
    s.sql("DROP TRIGGER no_done;");
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);

    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert!(s.failures().await.is_empty());
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.row_of(&v2()).await.new_version, 2);

    // A higher revision of the episode is not held up behind it.
    let v3 = v3();
    s.feed(&[(V3_HASH, &v3)]);
    s.tr.content_on_add(V3_HASH, V3_BYTES);
    s.cycle().await;
    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v3).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
}

/// The same when the new torrent has left Transmission since: the file
/// under the episode name is told by its CRC32.
#[tokio::test]
async fn a_rename_whose_done_was_not_written_finishes_without_its_torrent_too() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.sql(
        "CREATE TRIGGER no_done BEFORE UPDATE OF state ON video_revisions
         WHEN NEW.state = 'done' BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    s.cycle().await;
    s.sql("DROP TRIGGER no_done;");
    s.tr.remove(NEW_HASH);

    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert!(s.failures().await.is_empty());
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// Transmission renames the new video of a revision whose torrent is still
/// there; the app renames it itself when the torrent has gone, and then syncs
/// the folder so the episode name outlasts a power loss. A folder that cannot
/// be synced does not undo the rename.
#[tokio::test]
async fn a_new_video_renamed_on_disk_has_its_folder_synced_and_a_failing_sync_does_not_undo_it() {
    for fail in [false, true] {
        let s = World::new().await;
        s.received_v1().await;
        s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
        s.tr.content_on_add(NEW_HASH, NEW_BYTES);
        s.cycle().await;
        s.complete(NEW_HASH);
        // The rename does not go through, as for a worker stopped right after
        // the old video went, and the torrent has left by the next look.
        s.tr.reject_rename_of(NEW_HASH, Some("busy"));
        s.cycle().await;
        assert_eq!(s.names(), vec![v2()]);
        s.tr.remove(NEW_HASH);
        let folder = s.file(EPISODE_NAME).parent().unwrap().to_path_buf();
        let failing = fail.then(|| testing::fail_syncs_of(&folder));
        assert_eq!(testing::syncs_of(&folder), 0);

        s.cycle().await;

        assert_eq!(s.names(), vec![EPISODE_NAME], "failing syncs: {fail}");
        assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
        assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
        assert!(s.failures().await.is_empty());
        match failing {
            None => assert!(
                testing::syncs_of(&folder) >= 1,
                "the folder the video was renamed in was not synced"
            ),
            Some(_) => assert_eq!(testing::syncs_of(&folder), 0),
        }
    }
}

/// A file put over the old video (an atomic rename by another program) while
/// the worker reads the old video's CRC32 is not deleted: the CRC32 belongs
/// to the file that was read, not to the one at the name now. The episode
/// name is a pipe here so the test can swap the file while the read is open.
#[tokio::test]
async fn a_file_put_over_the_old_video_while_it_is_read_is_not_deleted() {
    let s = World::new().await;
    s.received_v1().await;
    // A file of no torrent, which is deleted by path once its CRC32 matches.
    s.tr.remove(OLD_HASH);
    s.feed(&[(NEW_HASH, &v2())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    let episode = s.file(EPISODE_NAME);
    std::fs::remove_file(&episode).unwrap();
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &episode,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o644),
        0,
    )
    .unwrap();
    let incoming = s.file(".incoming");
    std::fs::write(&incoming, b"someone else's").unwrap();
    s.complete(NEW_HASH);

    let swap = {
        let episode = episode.clone();
        tokio::task::spawn_blocking(move || {
            use std::{io::Write, os::unix::fs::OpenOptionsExt};
            let begun = std::time::Instant::now();
            // Waits for the worker to open the pipe for its read.
            let mut pipe = loop {
                match std::fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
                    .open(&episode)
                {
                    Ok(pipe) => break pipe,
                    Err(_) if begun.elapsed() < std::time::Duration::from_secs(30) => {
                        std::thread::sleep(std::time::Duration::from_millis(5))
                    }
                    Err(err) => panic!("the worker never read the old video: {err}"),
                }
            };
            std::fs::rename(&incoming, &episode).unwrap();
            pipe.write_all(OLD_BYTES).unwrap();
        })
    };
    let (_, swapped) = tokio::join!(s.cycle(), swap);
    swapped.unwrap();
    // A read the swap disturbed may be left to the next look.
    s.cycle().await;

    assert_eq!(read(&episode), b"someone else's", "the new file is kept");
    assert_eq!(read(&s.file(&v2())), NEW_BYTES);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    // The failure is listed, with both files kept.
    let failures = s.failures().await;
    assert!(one_failure(&failures).reason.is_some());
}

/// The old video is gone after the replacement was claimed (`removing` is
/// written) but before the name is looked at again: Transmission finished an
/// earlier removal, or the person deleted it. That is the old video removed:
/// the replacement goes on to name the new video and the old release stays
/// superseded, instead of failing as changed and being cleared.
#[tokio::test]
async fn an_old_video_gone_after_the_claim_still_ends_in_the_replacement() {
    let s = World::new().await;
    // A file of no torrent whose CRC32 is its name's: an empty file's, so a
    // pipe the worker reads to its end with nothing written gives it.
    let old = release("", Some("00000000"));
    s.untracked_video_of(b"", &old).await;
    s.feed(&[(NEW_HASH, &v2())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);

    let episode = s.file(EPISODE_NAME);
    std::fs::remove_file(&episode).unwrap();
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &episode,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o644),
        0,
    )
    .unwrap();
    s.complete(NEW_HASH);

    let db_path = s.db_path();
    let gone = {
        let episode = episode.clone();
        tokio::task::spawn_blocking(move || {
            use std::os::unix::fs::OpenOptionsExt;
            let begun = std::time::Instant::now();
            // Waits for the worker to open the pipe for its read.
            let pipe = loop {
                match std::fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
                    .open(&episode)
                {
                    Ok(pipe) => break pipe,
                    Err(_) if begun.elapsed() < std::time::Duration::from_secs(30) => {
                        std::thread::sleep(std::time::Duration::from_millis(5))
                    }
                    Err(err) => panic!("the worker never read the old video: {err}"),
                }
            };
            // The worker's writes of the cycle are done up to the claim of the
            // replacement, the next one; it waits for this lock.
            let db = rusqlite::Connection::open(&db_path).unwrap();
            db.busy_timeout(std::time::Duration::from_secs(10)).unwrap();
            db.execute_batch("BEGIN IMMEDIATE").unwrap();
            // The end of the pipe ends the worker's read.
            drop(pipe);
            std::thread::sleep(std::time::Duration::from_millis(500));
            std::fs::remove_file(&episode).unwrap();
            db.execute_batch("COMMIT").unwrap();
        })
    };
    let (_, gone) = tokio::join!(s.cycle(), gone);
    gone.unwrap();
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&episode), NEW_BYTES);
    assert!(s.failures().await.is_empty());
    let row = s.row_of(&v2()).await;
    assert_eq!(row.old_item_id, Some(s.item(&old).await.id));
}

/// Transmission removed the old torrent and its data, but its answer never
/// came (a timeout): the replacement goes on as removing, finds the old video
/// gone, and names the new one; the old release is not received again.
#[tokio::test]
async fn an_old_torrent_removed_without_an_answer_still_ends_in_the_replacement() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.tr.break_remove_answer_of(OLD_HASH);
    s.cycle().await;
    assert!(s.tr.torrents().iter().all(|t| t.hash != OLD_HASH));
    let adds = s.added(OLD_HASH);
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(
        s.added(OLD_HASH),
        adds,
        "the old release is not received again"
    );
    assert!(s.failures().await.is_empty());
    // Every item of the removed torrent stays superseded.
    let row = s.row_of(&v2()).await;
    assert_eq!(row.old_torrent_hash.as_deref(), Some(OLD_HASH));
}

/// Transmission took the old torrent out but its file is still there (it
/// deletes data after answering, or could not), and the old release's name
/// carries no CRC32 to check the file by. The replacement stays removing and
/// says so, the old release is not received again, and once the file is gone
/// the new video takes the name.
#[tokio::test]
async fn an_old_file_left_after_its_torrent_was_removed_waits_as_removing() {
    let s = World::new().await;
    let v1 = release("", None);
    s.feed(&[(OLD_HASH, &v1)]);
    s.tr.content_on_add(OLD_HASH, OLD_BYTES);
    s.cycle().await;
    s.complete(OLD_HASH);
    assert_eq!(s.names(), vec![EPISODE_NAME]);

    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1)]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.tr.keep_data_on_remove_of(OLD_HASH);
    s.cycle().await;
    let adds = s.added(OLD_HASH);
    s.cycle().await;
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.state_of(&v2()).await, RevisionState::Removing);
    // It waits with its old video's file still there: a failure that says so.
    let failures = s.failures().await;
    let failure = one_failure(&failures);
    assert!(failure.reason.is_some(), "{failure:?}");
    assert_eq!(s.added(OLD_HASH), adds);

    // Transmission (or the person) deletes it at last.
    std::fs::remove_file(s.file(EPISODE_NAME)).unwrap();
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(
        s.added(OLD_HASH),
        adds,
        "the old release is not received again"
    );
    assert!(s.failures().await.is_empty());
}

/// The old video is the single file of a torrent that keeps it in a folder
/// (`Season 01/Show S01E14.mkv` under the work folder): removing that torrent
/// with its data could take the folder with it, so it is not replaced.
#[tokio::test]
async fn an_old_video_inside_its_torrents_folder_is_not_removed() {
    let s = World::new().await;
    std::fs::write(s.file(EPISODE_NAME), OLD_BYTES).unwrap();
    let show = s.season.parent().unwrap();
    s.tr.preload(
        FakeTorrent::new(OLD_HASH, "Season 01")
            .in_dir(show)
            .files(&[&format!("Season 01/{EPISODE_NAME}")])
            .status(6),
    );
    s.ctx
        .history
        .record(
            1,
            vec![Observation {
                channel_id: s.channel_id.clone(),
                channel_label: "https://feeds.example.test/show".into(),
                identity_key: "guid:v1".into(),
                title: v1(),
                link: magnet(OLD_HASH, &v1()),
                result: HistoryResult::Received,
                rule_id: None,
                torrent_hash: Some(OLD_HASH.into()),
                reason: None,
            }],
        )
        .await
        .unwrap();
    s.feed(&[(NEW_HASH, &v2())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;

    assert!(s.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
}
