//! What a cycle decides about a selected revision ([`plan`], then
//! `cycle::leave_revisions` and `cycle::process_job`): replace the video,
//! skip it, hold it as `버전 미상` or receive it as an ordinary item
//! (`docs/specs/collection.md`, 영상 수정본의 대체; ticket 0025). The cases
//! run through [`World::cycle`] against the fake Transmission, which acts on
//! the media folder the way Transmission does.

use super::fixtures::*;
use super::*;
use crate::{
    store::history::HistoryResult,
    test_world::{crc, feed_xml, read, World},
};

/// A revision whose name carries no CRC32 could not be checked once it is
/// received, so it is not received: `버전 미상` in history, with the row
/// `다시 받기` needs.
#[tokio::test]
async fn a_revision_without_a_crc_in_its_name_is_not_received() {
    let s = World::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    s.feed(&[(NEW_HASH, &v2), (OLD_HASH, &v1())]);
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.added(NEW_HASH), 0);
    let item = s.item(&v2).await;
    assert_eq!(item.result, HistoryResult::VersionUnknown);
    assert!(item.reason.unwrap().contains("CRC32"));
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(s.state_of(&v2).await, RevisionState::Unknown);
}

/// The folder's video belongs to no torrent history knows, so its CRC32 is
/// read and compared with the names of the release's revisions: the same as
/// the revision's own name or a higher one is skipped, the same as an older
/// one is replaced, and any other is `버전 미상`.
#[tokio::test]
async fn a_video_of_unknown_revision_is_told_by_its_crc() {
    struct Case {
        what: &'static str,
        /// The episode name's file, of no torrent.
        bytes: &'static [u8],
        /// What the cycle does with `14v2`: replaces the video, or holds it.
        replaced: bool,
        result: HistoryResult,
        state: RevisionState,
    }
    let cases = [
        Case {
            what: "equal to the old release",
            bytes: OLD_BYTES,
            replaced: true,
            result: HistoryResult::Received,
            state: RevisionState::Done,
        },
        Case {
            what: "equal to the newest",
            bytes: NEW_BYTES,
            replaced: false,
            result: HistoryResult::Duplicate,
            state: RevisionState::Skipped,
        },
        Case {
            what: "equal to neither",
            bytes: b"some other cut",
            replaced: false,
            result: HistoryResult::VersionUnknown,
            state: RevisionState::Unknown,
        },
    ];
    for case in cases {
        let s = World::new().await;
        s.untracked_video(case.bytes).await;
        s.feed(&[(NEW_HASH, &v2())]);
        s.tr.content_on_add(NEW_HASH, NEW_BYTES);
        s.cycle().await;
        if case.replaced {
            assert_eq!(s.added(NEW_HASH), 1, "{}", case.what);
            s.complete(NEW_HASH);
        }
        s.cycle().await;

        assert_eq!(
            s.added(NEW_HASH),
            usize::from(case.replaced),
            "{}",
            case.what
        );
        assert_eq!(s.names(), vec![EPISODE_NAME], "{}", case.what);
        let expected = if case.replaced { NEW_BYTES } else { case.bytes };
        assert_eq!(read(&s.file(EPISODE_NAME)), expected, "{}", case.what);
        assert_eq!(s.item(&v2()).await.result, case.result, "{}", case.what);
        let row = s.row_of(&v2()).await;
        assert_eq!(row.state, case.state, "{}", case.what);
        if case.replaced {
            // The version line of the episode: from `v1`.
            assert_eq!(row.old_version, Some(1));
        }
    }
}

/// Another group's `14`, and a revision of another release of it (720p),
/// are received as they are: a duplicate of the episode, not a replacement,
/// and no rename ever takes a name that is taken.
#[tokio::test]
async fn another_release_of_the_episode_is_held_as_a_duplicate_not_a_replacement() {
    let s = World::with_match("Show - 14").await;
    s.received_v1().await;
    let erai = format!("[Erai-raws] Show - 14 [1080p][{}].mkv", crc(b"erai"));
    let other_v2 = format!("[SubsPlease] Show - 14v2 (720p) [{}].mkv", crc(b"720p"));
    s.feed(&[
        (OTHER_HASH, &erai),
        (NEW_HASH, &other_v2),
        (OLD_HASH, &v1()),
    ]);
    s.tr.content_on_add(OTHER_HASH, b"erai");
    s.tr.content_on_add(NEW_HASH, b"720p");
    s.cycle().await;
    s.complete(OTHER_HASH);
    s.complete(NEW_HASH);
    s.cycle().await;

    let mut expected = vec![EPISODE_NAME.to_owned(), erai.clone(), other_v2.clone()];
    expected.sort();
    assert_eq!(s.names(), expected);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(s.tr.calls_of("torrent-remove").is_empty());
    assert!(!s.renamed_onto_episode(OTHER_HASH));
    assert!(!s.renamed_onto_episode(NEW_HASH));
    assert!(s.failures().await.is_empty());
    for title in [&erai, &other_v2] {
        let item = s.item(title).await;
        assert_eq!(item.result, HistoryResult::Received);
        assert!(s.ctx.revisions.by_item(item.id).await.unwrap().is_none());
    }
}

/// `14v3` replaced `14`, and its torrent has since left Transmission. `14v2`
/// appearing now is lower than the folder's video: it is skipped, not left
/// to `다시 받기` as a video of unknown revision.
#[tokio::test]
async fn a_lower_revision_after_a_higher_one_is_skipped_without_its_torrent_too() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(V3_HASH, &v3()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(V3_HASH, V3_BYTES);
    s.cycle().await;
    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);

    s.tr.remove(V3_HASH);
    s.feed(&[(NEW_HASH, &v2())]);
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.added(NEW_HASH), 0);
    let item = s.item(&v2()).await;
    assert_eq!(item.result, HistoryResult::Duplicate);
    assert_eq!(item.reason.as_deref(), Some(NOT_HIGHER));
    assert!(s
        .ctx
        .revisions
        .by_item(item.id)
        .await
        .unwrap()
        .is_none_or(|row| row.state == RevisionState::Skipped));
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
}

/// The same release reaches the folder through two channels (one torrent).
/// Once `14v2` replaced it, neither channel's `14` brings it back.
#[tokio::test]
async fn the_old_release_in_another_channel_is_not_received_again() {
    let s = World::new().await;
    s.second_channel("show2").await;
    s.received_v1().await;
    s.feeds.set_xml("show2", &feed_xml(&[(OLD_HASH, &v1())]));
    s.cycle().await;
    let firsts = s.history_items().await;
    assert_eq!(firsts.iter().filter(|i| i.title == v1()).count(), 2);

    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert!(s.tr.torrents().iter().all(|t| t.hash != OLD_HASH));

    let adds = s.added(OLD_HASH);
    s.cycle().await;
    s.cycle().await;
    assert_eq!(
        s.added(OLD_HASH),
        adds,
        "the old release is not added again"
    );
    assert!(s.tr.torrents().iter().all(|t| t.hash != OLD_HASH));
    assert_eq!(s.names(), vec![EPISODE_NAME]);
}

/// `14v2` in two channels is one torrent: it replaces `14` once, and the
/// other channel's item is no failure.
#[tokio::test]
async fn the_same_revision_in_two_channels_replaces_once_without_a_failure() {
    let s = World::new().await;
    s.second_channel("show2").await;
    s.received_v1().await;
    let both = [(NEW_HASH, v2()), (OLD_HASH, v1())];
    let both: Vec<(&str, &str)> = both.iter().map(|(h, t)| (*h, t.as_str())).collect();
    s.feeds.set_xml("show", &feed_xml(&both));
    s.feeds.set_xml("show2", &feed_xml(&both));
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    s.complete(NEW_HASH);
    s.cycle().await;
    s.cycle().await;
    s.cycle().await;

    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    let failures = s.failures().await;
    assert!(failures.is_empty(), "{failures:?}");
}

/// A revision seen first (no earlier release of it in the folder) is received
/// as any release and named as its episode, not as `trname` reads its marker.
#[tokio::test]
async fn a_revision_seen_first_is_named_as_its_episode() {
    // (the rule's match, the group's revision, its hash, the episode's name)
    let cases = [
        ("[Erai-raws] Show - ", erai("v2"), ERAI_HASH, ERAI_EPISODE),
        ("[SubsPlease] Show - ", v2(), NEW_HASH, EPISODE_NAME),
    ];
    for (rule, title, hash, episode) in cases {
        let s = World::with_match(rule).await;
        s.feed(&[(hash, &title)]);
        s.tr.content_on_add(hash, NEW_BYTES);
        s.cycle().await;
        assert_eq!(s.names(), vec![episode], "{title}");
        assert_eq!(s.tr.torrent(hash).name, episode, "{title}");
        // History keeps the release's own title.
        assert_eq!(s.item(&title).await.result, HistoryResult::Received);
    }
}

/// A show named with `NvM` (`Show 3v3`) whose first release of an episode
/// carries no revision marker: that release is the episode's first revision
/// (not revision 3 of another release), received and named as any, and its
/// `06v2` replaces it.
#[tokio::test]
async fn a_revision_of_a_show_named_with_a_number_v_number_replaces_its_first_release() {
    let s = World::with_match("[SubsPlease] Show 3v3 - ").await;
    let first = format!(
        "[SubsPlease] Show 3v3 - 06 (1080p) [{}].mkv",
        crc(OLD_BYTES)
    );
    let second = format!(
        "[SubsPlease] Show 3v3 - 06v2 (1080p) [{}].mkv",
        crc(NEW_BYTES)
    );
    let episode = "Show S01E06.mkv";
    s.feed(&[(OLD_HASH, &first)]);
    s.tr.content_on_add(OLD_HASH, OLD_BYTES);
    s.cycle().await;
    s.complete(OLD_HASH);
    assert_eq!(s.names(), vec![episode]);
    assert_eq!(s.tr.torrent(OLD_HASH).name, episode);

    s.feed(&[(NEW_HASH, &second), (OLD_HASH, &first)]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    assert_eq!(s.added(NEW_HASH), 1);
    assert_eq!(s.state_of(&second).await, RevisionState::Receiving);
    let mut both = vec![episode.to_owned(), second.clone()];
    both.sort();
    assert_eq!(s.names(), both);

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&second).await, RevisionState::Done);
    assert_eq!(s.names(), vec![episode]);
    assert_eq!(read(&s.file(episode)), NEW_BYTES);
    assert!(s.removed(OLD_HASH));
}
