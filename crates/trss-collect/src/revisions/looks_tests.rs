//! A replacement whose new video is looked at again: gone after the old
//! torrent was removed, missing on one look or two, replaced or changed
//! after its check, or the episode left with no video
//! ([`advance`]; `docs/specs/collection.md`, 영상 수정본의 대체; tickets 0025,
//! 0033). The cases run through [`World::cycle`] against the fake
//! Transmission, which acts on the media folder the way Transmission does.

use super::fixtures::*;
use super::*;
use crate::{
    revision::FileIdentity,
    test_world::{read, World},
};

/// While `14v2` waits for the old file to go, the person deletes the new
/// video instead. Seen gone on two looks, the replacement ends: nothing is
/// deleted, the old file stays (and its release is not received again), it
/// is no failure, and a later revision of the episode is not held up.
#[tokio::test]
async fn a_new_video_deleted_while_the_old_file_is_left_ends_the_replacement() {
    let s = World::new().await;
    s.removal_waits().await;
    let removes = s.removals_with_data();
    let adds = s.added(OLD_HASH);

    std::fs::remove_file(s.file(&v2())).unwrap();
    s.cycle().await;
    assert_eq!(
        s.state_of(&v2()).await,
        RevisionState::Removing,
        "seen gone once"
    );
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert!(s.failures().await.is_empty());
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert_eq!(s.removals_with_data(), removes);
    assert_eq!(
        s.added(OLD_HASH),
        adds,
        "the old release is not received again"
    );

    // `14v3` replaces the old file.
    s.v3_received().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
}

/// The old video is gone and `14v2`'s rename has not gone through when its
/// file goes missing for one look (a mount that was away): it keeps waiting,
/// and takes the episode name once the file is back.
#[tokio::test]
async fn a_new_video_missing_on_one_look_still_takes_the_name() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.tr.reject_rename_of(NEW_HASH, Some("busy"));
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);
    assert_eq!(s.names(), vec![v2()]);

    let away = s.season.parent().unwrap().join("away.mkv");
    std::fs::rename(s.file(&v2()), &away).unwrap();
    s.cycle().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Removed);
    assert!(
        row.reason.as_deref().unwrap().contains("찾지 못해"),
        "{row:?}"
    );

    std::fs::rename(&away, s.file(&v2())).unwrap();
    s.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// `14v2` removed the old video (its file had waited after the torrent was
/// removed) and its rename has not gone through when its own file goes
/// missing. Missing on two looks in a row, the replacement ends without
/// anything deleted, and `14v3`, decided while the old file was there, is
/// no longer held up by it.
#[tokio::test]
async fn a_new_video_missing_on_two_looks_ends_the_replacement() {
    let s = World::new().await;
    s.removal_waits().await;
    s.v3_received().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Verified);

    // The old file goes; `14v2`'s rename is refused for now.
    s.tr.reject_rename_of(NEW_HASH, Some("busy"));
    std::fs::remove_file(s.file(EPISODE_NAME)).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Verified);

    // Its file goes missing.
    std::fs::remove_file(s.file(&v2())).unwrap();
    let removes = s.removals_with_data();
    let adds = s.added(OLD_HASH);
    s.cycle().await;
    assert_eq!(
        s.state_of(&v2()).await,
        RevisionState::Removed,
        "seen missing once"
    );
    assert_eq!(s.state_of(&v3()).await, RevisionState::Verified);

    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
    // The cycle may take `14v2`'s torrent out once its item left the feed,
    // as any; never with data.
    assert_eq!(s.removals_with_data(), removes);
    assert_eq!(
        s.added(OLD_HASH),
        adds,
        "the old release is not received again"
    );
    // Ended with no video under the name, `14v2` is a failure until the next
    // look finds `14v3` there.
    assert_eq!(s.failures().await.len(), 1);
    s.cycle().await;
    assert!(s.failures().await.is_empty());
}

/// `14v3`'s file is deleted after its CRC32 was checked and before it removes
/// the old video (`14v2`, which takes the episode name meanwhile). The old
/// video is not removed: the replacement waits on the first look and ends on
/// the second, with `14v2` and its torrent in place.
#[tokio::test]
async fn a_new_video_deleted_after_its_check_removes_no_old_video() {
    let s = World::new().await;
    s.v3_verified_behind_v2().await;
    std::fs::remove_file(s.file(&v3())).unwrap();
    let removes = s.removals_with_data();

    s.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(
        read(&s.file(EPISODE_NAME)),
        NEW_BYTES,
        "the old video stays"
    );
    let row = s.row_of(&v3()).await;
    assert_eq!(row.state, RevisionState::Verified, "seen missing once");
    assert!(row.reason.is_some(), "{row:?}");
    assert!(row.new_missing_at.is_some(), "{row:?}");
    // A failure that says so; the old video is kept (the row is not
    // removed).
    let failures = s.failures().await;
    assert_eq!(one_failure(&failures).item_id, row.item_id);

    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await.code(), "abandoned");
    s.cycle().await;
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert!(s.tr.torrents().iter().any(|t| t.hash == NEW_HASH));
    assert_eq!(s.removals_with_data(), removes);
    assert!(s.failures().await.is_empty());
}

/// `14v3`'s file is replaced by another file under its name after its CRC32
/// was checked: that is not the video checked, and the old video is not
/// removed for it.
#[tokio::test]
async fn a_new_video_replaced_after_its_check_removes_no_old_video() {
    let s = World::new().await;
    s.v3_verified_behind_v2().await;
    std::fs::remove_file(s.file(&v3())).unwrap();
    std::fs::write(s.file(&v3()), b"episode 14, something else").unwrap();
    let removes = s.removals_with_data();

    s.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(
        read(&s.file(EPISODE_NAME)),
        NEW_BYTES,
        "the old video stays"
    );
    assert_eq!(s.state_of(&v3()).await, RevisionState::Verified);
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await.code(), "abandoned");
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert!(s.tr.torrents().iter().any(|t| t.hash == NEW_HASH));
    assert_eq!(s.removals_with_data(), removes);
}

/// A row checked without the new file's identity kept (made so by hand)
/// removes the old video while the file under its received name has the
/// CRC32 that was checked, and not otherwise.
#[tokio::test]
async fn a_new_video_without_its_identity_kept_is_told_by_its_crc() {
    let s = World::new().await;
    s.v3_verified_behind_v2().await;
    let forget = format!(
        "UPDATE video_revisions SET file_identity = NULL WHERE item_id = {}",
        s.item(&v3()).await.id
    );
    s.sql(&forget);
    std::fs::write(s.file(&v3()), b"episode 14, third release and more").unwrap();
    s.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Verified);
    assert_eq!(
        read(&s.file(EPISODE_NAME)),
        NEW_BYTES,
        "the old video stays"
    );

    std::fs::write(s.file(&v3()), V3_BYTES).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
}

/// `14v2`'s file is missing on one look, there on the next (which goes no
/// further: Transmission does not answer), and missing again on the one
/// after. No two looks in a row found it missing, so the replacement goes on,
/// and the file takes the name once it is back.
#[tokio::test]
async fn a_new_video_found_between_two_misses_keeps_its_replacement() {
    let mut s = World::new().await;
    s.v2_waits_for_its_name().await;

    std::fs::rename(s.file(&v2()), s.away()).unwrap();
    s.cycle().await;
    std::fs::rename(s.away(), s.file(&v2())).unwrap();
    s.tr.stop().await;
    s.cycle().await;
    s.tr.restart().await;
    std::fs::rename(s.file(&v2()), s.away()).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);

    std::fs::rename(s.away(), s.file(&v2())).unwrap();
    s.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// `14v2`'s file is missing on one look and there on the next, which goes no
/// further (Transmission does not answer): the miss and its reason go, so
/// the replacement is no `받기 실패` any more.
#[tokio::test]
async fn a_new_video_found_by_a_look_that_goes_no_further_is_no_failure() {
    let mut s = World::new().await;
    s.v2_waits_for_its_name().await;

    std::fs::rename(s.file(&v2()), s.away()).unwrap();
    s.cycle().await;
    assert_eq!(s.failures().await.len(), 1);
    std::fs::rename(s.away(), s.file(&v2())).unwrap();
    s.tr.stop().await;
    s.cycle().await;
    s.tr.restart().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Removed);
    assert_eq!(row.new_missing_at, None);
    assert_eq!(row.reason, None, "{row:?}");
    assert!(s.failures().await.is_empty());
}

/// `14v2`'s file is missing on one look; on the next it is back, but another
/// file holds the episode name, so the rename waits. That look found the
/// file: the file missing once more after the name is free again is a first
/// miss, not the second in a row.
#[tokio::test]
async fn a_new_video_found_while_its_name_is_taken_breaks_the_run() {
    let s = World::new().await;
    s.v2_waits_for_its_name().await;

    std::fs::rename(s.file(&v2()), s.away()).unwrap();
    s.cycle().await;
    std::fs::rename(s.away(), s.file(&v2())).unwrap();
    std::fs::write(s.file(EPISODE_NAME), b"someone else's file").unwrap();
    s.cycle().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Removed);
    assert!(
        row.reason
            .as_deref()
            .unwrap()
            .contains("다른 파일이 있어서"),
        "{row:?}"
    );

    std::fs::remove_file(s.file(EPISODE_NAME)).unwrap();
    std::fs::rename(s.file(&v2()), s.away()).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);

    std::fs::rename(s.away(), s.file(&v2())).unwrap();
    s.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// `14v2`'s file is missing on one look, and on the next its whole folder is
/// away (a mount): that look decides nothing and breaks the run, so the file
/// missing once more after the folder is back is a first miss again.
#[tokio::test]
async fn a_folder_away_between_two_misses_keeps_the_replacement() {
    let s = World::new().await;
    s.v2_waits_for_its_name().await;

    std::fs::rename(s.file(&v2()), s.away()).unwrap();
    s.cycle().await;
    let elsewhere = s.season.with_file_name("Season 01 away");
    std::fs::rename(&s.season, &elsewhere).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);
    std::fs::rename(&elsewhere, &s.season).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Removed);

    std::fs::rename(s.away(), s.file(&v2())).unwrap();
    s.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// `14v3`'s file is copied over itself after its check (as a remount or a
/// copy back from elsewhere would leave it): another inode, the same video.
/// Its CRC32, read again, is the checked one, so it replaces `14v2`.
#[tokio::test]
async fn a_new_video_whose_identity_changed_is_told_by_its_crc() {
    let s = World::new().await;
    s.v3_verified_behind_v2().await;
    let copy = s.season.parent().unwrap().join("copy.mkv");
    std::fs::copy(s.file(&v3()), &copy).unwrap();
    std::fs::rename(&copy, s.file(&v3())).unwrap();

    s.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
}

/// Told by its CRC32 after its identity changed, `14v3`'s file keeps the
/// identity it has now: a later look compares that instead of reading the
/// whole file again on every cycle.
#[tokio::test]
async fn a_new_video_told_by_its_crc_keeps_its_identity_now() {
    let s = World::new().await;
    s.v3_verified_behind_v2().await;
    let copy = s.season.parent().unwrap().join("copy.mkv");
    std::fs::copy(s.file(&v3()), &copy).unwrap();
    std::fs::rename(&copy, s.file(&v3())).unwrap();
    let now = FileIdentity::at(&s.file(&v3())).unwrap().to_text();
    assert_ne!(s.row_of(&v3()).await.file_identity, Some(now.clone()));

    s.tr.reject_rename_of(NEW_HASH, None);
    s.cycle().await;
    s.cycle().await;
    let row = s.row_of(&v3()).await;
    assert_eq!(row.state, RevisionState::Done);
    assert_eq!(row.file_identity, Some(now));
}

/// `14v3`'s file is gone when it is to remove `14v2`, a file no torrent holds
/// that it would read whole to tell. The new video is looked at first: the
/// old one is never read for a replacement that cannot go on.
#[tokio::test]
async fn a_missing_new_video_is_seen_before_the_old_video_is_read() {
    let s = World::new().await;
    s.v3_verified_behind_v2().await;
    // `14v2` takes the name on disk once its torrent is gone.
    s.tr.remove(NEW_HASH);
    std::fs::remove_file(s.file(&v3())).unwrap();
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_ne!(s.state_of(&v3()).await.code(), "abandoned");

    // The episode name becomes a pipe, which tells whether anyone opens it.
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
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let probe = {
        let (episode, stop) = (episode.clone(), stop.clone());
        tokio::task::spawn_blocking(move || {
            use std::{io::Write, os::unix::fs::OpenOptionsExt, sync::atomic::Ordering};
            while !stop.load(Ordering::SeqCst) {
                // Opens only while someone has it open for reading.
                if let Ok(mut pipe) = std::fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
                    .open(&episode)
                {
                    let _ = pipe.write_all(NEW_BYTES);
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            false
        })
    };
    s.cycle().await;
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(!probe.await.unwrap(), "the old video was read");
    assert_eq!(s.state_of(&v3()).await.code(), "abandoned");
}

/// `14v2` removed `14` and then lost its video before it took the name. The
/// episode has no video under its name, so the ended replacement is a
/// `받기 실패` that says so, until a video is under the name again.
#[tokio::test]
async fn a_replacement_ended_with_no_video_left_is_a_failure_until_the_name_holds_one() {
    let s = World::new().await;
    s.v2_waits_for_its_name().await;
    std::fs::remove_file(s.file(&v2())).unwrap();
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    // The old video is removed and the new one is missing.
    let failures = s.failures().await;
    let failure = one_failure(&failures);
    assert!(
        failure
            .reason
            .as_deref()
            .unwrap()
            .contains("회차 이름에 영상이 없어요"),
        "{failure:?}"
    );
    assert_eq!(failure.state, RevisionState::Abandoned);
    assert!(failure.received_name.is_some(), "{failure:?}");

    std::fs::write(s.file(EPISODE_NAME), b"put back by hand").unwrap();
    s.cycle().await;
    assert!(s.failures().await.is_empty());
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert_eq!(read(&s.file(EPISODE_NAME)), b"put back by hand");
}

/// `14v2` removed `14`'s torrent, whose file Transmission left, and then lost
/// its own video: the replacement ends with `14`'s file still under the name,
/// which is no failure. The ended replacement keeps watching that file: once
/// it goes too (Transmission deleting it late, or the person), the episode
/// has no video, and that is a `받기 실패` until a video is there again.
#[tokio::test]
async fn a_replacement_ended_beside_a_left_old_file_is_a_failure_once_that_file_goes() {
    let s = World::new().await;
    s.removal_waits().await;
    std::fs::remove_file(s.file(&v2())).unwrap();
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    s.cycle().await;
    assert!(s.failures().await.is_empty());
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);

    std::fs::remove_file(s.file(EPISODE_NAME)).unwrap();
    s.cycle().await;
    // The old video is removed and the new one is missing.
    let failures = s.failures().await;
    let failure = one_failure(&failures);
    assert!(
        failure
            .reason
            .as_deref()
            .unwrap()
            .contains("회차 이름에 영상이 없어요"),
        "{failure:?}"
    );
    assert_eq!(failure.state, RevisionState::Abandoned);
    assert!(failure.received_name.is_some(), "{failure:?}");

    std::fs::write(s.file(EPISODE_NAME), b"put back by hand").unwrap();
    s.cycle().await;
    assert!(s.failures().await.is_empty());
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert_eq!(read(&s.file(EPISODE_NAME)), b"put back by hand");
}
