//! `다시 받기` of a video revision (`receive_once::run` on a command the web
//! accepted; `docs/specs/collection.md`, 영상 수정본의 대체; tickets 0025,
//! 0033): a revision with no CRC32 in its name confirmed, a download that
//! stopped received again, an ended replacement checked and started again,
//! and the requests that are refused with a reason. The cases run through
//! [`World::retry`] and [`World::cycle`] against the fake Transmission,
//! which acts on the media folder the way Transmission does.

use super::fixtures::*;
use super::*;
use crate::{
    commands::receive_once::Retry,
    store::history::HistoryResult,
    test_world::{feed_xml, magnet, read, World},
};
use trss_core::commands::CommandState;

// --- a revision with no CRC32 in its name ---------------------------------------------

#[tokio::test]
async fn a_revision_without_a_crc_received_with_retry_replaces_without_the_check() {
    let s = World::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    s.feed(&[(NEW_HASH, &v2), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    let item = s.item(&v2).await;
    assert_eq!(item.result, HistoryResult::VersionUnknown);

    let finished = s
        .retry(item.id, "00000000-0000-4000-8000-000000000025")
        .await
        .unwrap();
    assert_eq!(finished.state, CommandState::Done, "{finished:?}");

    assert_eq!(s.item(&v2).await.result, HistoryResult::Received);
    assert!(!s.renamed_onto_episode(NEW_HASH));
    assert_eq!(s.names(), vec![EPISODE_NAME.to_owned(), v2.clone()]);
    assert_eq!(s.state_of(&v2).await, RevisionState::Receiving);

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.state_of(&v2).await, RevisionState::Done);
    assert_eq!(s.row_of(&v2).await.new_version, 2);
}

/// `다시 받기` of a `버전 미상` revision whose confirmation cannot be written:
/// the command is not ended as if it had been, and its next run starts the
/// replacement.
#[tokio::test]
async fn a_retry_whose_confirmation_is_not_written_runs_again_and_replaces() {
    let s = World::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    s.feed(&[(NEW_HASH, &v2), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    let item = s.item(&v2).await;
    assert_eq!(item.result, HistoryResult::VersionUnknown);

    let command = s
        .start_retry(item.id, "00000000-0000-4000-8000-000000000251")
        .await;
    s.sql(
        "CREATE TRIGGER no_confirm BEFORE UPDATE ON video_revisions
         BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    let first = s.run_command(&command).await;
    assert!(matches!(first, Err(Retry::Store(_))), "{first:?}");
    s.sql("DROP TRIGGER no_confirm;");
    s.run_command(&command).await.unwrap();
    assert!(!s.renamed_onto_episode(NEW_HASH));

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.item(&v2).await.result, HistoryResult::Received);
}

/// The same, with the history write failing after the confirmation.
#[tokio::test]
async fn a_retry_whose_result_is_not_written_runs_again_and_replaces() {
    let s = World::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    s.feed(&[(NEW_HASH, &v2), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.cycle().await;
    let item = s.item(&v2).await;

    let command = s
        .start_retry(item.id, "00000000-0000-4000-8000-000000000252")
        .await;
    s.sql(
        "CREATE TRIGGER no_result BEFORE UPDATE OF result ON history_items
         BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    let first = s.run_command(&command).await;
    assert!(matches!(first, Err(Retry::Store(_))), "{first:?}");
    s.sql("DROP TRIGGER no_result;");
    s.run_command(&command).await.unwrap();
    assert!(!s.renamed_onto_episode(NEW_HASH));

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.item(&v2).await.result, HistoryResult::Received);
}

/// A confirmed revision whose torrent names the episode file itself (the old
/// video, as a rename before ticket 0025 left such torrents) is not a new
/// video: nothing is removed.
#[tokio::test]
async fn a_revision_whose_torrent_names_the_episode_file_removes_nothing() {
    let s = World::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    let link = magnet(NEW_HASH, EPISODE_NAME).replace('&', "&amp;");
    let item = format!(
        r#"<item><title>{v2}</title><link>{link}</link><guid isPermaLink="false">guid-{NEW_HASH}</guid></item>"#
    );
    // `14` stays in the feed, so its torrent is not removed as departed.
    let xml = feed_xml(&[(OLD_HASH, &v1())]).replace("</channel>", &format!("{item}</channel>"));
    s.feeds.set_xml("show", &xml);
    s.cycle().await;
    let item = s.item(&v2).await;
    assert_eq!(item.result, HistoryResult::VersionUnknown);
    s.retry(item.id, "00000000-0000-4000-8000-000000000253")
        .await
        .unwrap();
    s.complete(NEW_HASH);
    s.cycle().await;

    assert!(s.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    assert!(s.tr.torrents().iter().any(|t| t.hash == OLD_HASH));
}

/// A revision that was not received (its add was refused) is received with
/// `다시 받기` and named as its episode.
#[tokio::test]
async fn a_revision_received_with_retry_is_named_as_its_episode() {
    let s = World::with_match("[Erai-raws] Show - ").await;
    let title = erai("v2");
    s.feed(&[(ERAI_HASH, &title)]);
    s.tr.reject_adds(Some("refused"));
    s.cycle().await;
    assert_eq!(s.item(&title).await.result, HistoryResult::AddFailed);
    s.tr.reject_adds(None);
    s.feed(&[]);

    s.tr.content_on_add(ERAI_HASH, NEW_BYTES);
    let item = s.item(&title).await;
    let finished = s
        .retry(item.id, "00000000-0000-4000-8000-000000000611")
        .await
        .unwrap();
    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(s.item(&title).await.result, HistoryResult::Received);
    assert_eq!(s.names(), vec![ERAI_EPISODE]);
}

// --- a revision whose download stopped --------------------------------------------------

#[tokio::test]
async fn a_stopped_revision_that_left_the_feed_is_received_again_with_retry() {
    let s = World::new().await;
    let item = s.stopped_after_leaving_the_feed().await;
    assert_eq!(item.result, HistoryResult::Received);

    // The failure is on the item, with `다시 받기` offered.
    let failures = s.failures().await;
    assert_eq!(one_failure(&failures).item_id, item.id);
    assert!(s.can_retry(&v2()).await);

    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.tr.unfinished_on_add(NEW_HASH);
    s.retry(item.id, "00000000-0000-4000-8000-000000000a01")
        .await
        .unwrap();

    // Received under its own name; the old video stays until it is checked.
    assert_eq!(s.added(NEW_HASH), 2);
    assert!(!s.renamed_onto_episode(NEW_HASH));
    assert_eq!(s.item(&v2()).await.result, HistoryResult::Received);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    assert!(s.failures().await.is_empty());
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
}

#[tokio::test]
async fn a_revision_whose_torrent_reports_an_error_is_started_again_with_retry() {
    let s = World::new().await;
    s.received_v1().await;
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.tr.content_on_add(NEW_HASH, NEW_BYTES);
    s.tr.unfinished_on_add(NEW_HASH);
    s.cycle().await;
    s.tr.set_local_error(NEW_HASH, Some("No space left on device"));
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert!(s.can_retry(&v2()).await);

    let item = s.item(&v2()).await;
    s.retry(item.id, "00000000-0000-4000-8000-000000000a02")
        .await
        .unwrap();
    // Transmission still had it: started again, not added a second time.
    assert_eq!(
        s.tr.calls_of("torrent-start")
            .iter()
            .filter(|c| c.args["ids"] == serde_json::json!([NEW_HASH]))
            .count(),
        1
    );
    assert_eq!(s.item(&v2()).await.result, HistoryResult::Received);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    assert!(!s.renamed_onto_episode(NEW_HASH));

    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// A revision whose retry cannot be added keeps its item's result and its
/// failure; only the command says why.
#[tokio::test]
async fn a_stopped_revision_whose_retry_is_refused_stays_a_failure() {
    let s = World::new().await;
    let item = s.stopped_after_leaving_the_feed().await;
    s.tr.reject_adds(Some("refused"));
    let done = s
        .retry(item.id, "00000000-0000-4000-8000-000000000a03")
        .await;
    assert_refused(done, "refused");
    assert_eq!(s.item(&v2()).await.result, HistoryResult::Received);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert!(s.can_retry(&v2()).await);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
}

/// Only a download that stopped is offered again: a revision received into
/// another folder would end the same way. The refusal says nothing of why
/// (the failure's own reason does).
#[tokio::test]
async fn a_revision_received_elsewhere_is_not_offered_again() {
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

    let refusal = s.retry_refusal(&v2()).await.expect("not offered");
    assert!(!refusal.explains_missing_button(), "{refusal:?}");
}

/// A stopped revision whose rule is paused says why `다시 받기` is missing.
#[tokio::test]
async fn a_stopped_revision_of_a_paused_rule_says_why_it_is_not_offered() {
    let s = World::new().await;
    let item = s.stopped_after_leaving_the_feed().await;
    let rule_id = item.rule_id.clone().unwrap();
    s.sql(&format!(
        "UPDATE rules SET state = 'paused' WHERE id = '{rule_id}';"
    ));
    let refusal = s.retry_refusal(&v2()).await.expect("not offered");
    assert!(refusal.explains_missing_button(), "{refusal:?}");
    assert!(refusal.message().contains("멈춰"), "{refusal:?}");
}

/// The rule's folder changed after the revision's replacement was decided: a
/// torrent added now would be received into the new folder, away from the
/// video it replaces, and the replacement would fail again with nothing more
/// to try. `다시 받기` is not offered, and a request accepted before the change
/// ends without adding anything.
#[tokio::test]
async fn a_stopped_revision_of_a_rule_whose_folder_changed_is_not_received_again() {
    let s = World::new().await;
    let item = s.stopped_after_leaving_the_feed().await;
    let rule_id = item.rule_id.clone().unwrap();
    // Accepted while the folder is the same.
    let command = s
        .start_retry(item.id, "00000000-0000-4000-8000-000000000a05")
        .await;
    s.sql(&format!(
        "UPDATE rules SET directory = 'Show/Season 02' WHERE id = '{rule_id}';"
    ));

    let refusal = s.retry_refusal(&v2()).await.expect("not offered");
    assert!(refusal.message().contains("폴더"), "{refusal:?}");

    let done = s.run_command(&command).await;
    assert_refused(done, "폴더");
    assert_eq!(s.added(NEW_HASH), 1, "nothing was added");
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);
}

/// `14v2` was skipped because `14v3` was on its way; `14v3` then stops (its
/// torrent is gone and it left the feed). `14v2` replaces `14` on a
/// following cycle, `14v3` stays `받기 실패` with `다시 받기`, and `14v3`
/// received with it later replaces `14v2` as any higher revision does.
#[tokio::test]
async fn a_lower_revision_skipped_for_a_higher_one_that_fails_replaces_the_video() {
    let s = World::new().await;
    s.v2_skipped_for_v3().await;

    s.tr.remove(V3_HASH);
    s.feed(&[(NEW_HASH, &v2()), (OLD_HASH, &v1())]);
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);
    assert_eq!(read(&s.file(EPISODE_NAME)), OLD_BYTES);

    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert!(s.removed(OLD_HASH));
    assert!(!s.tr.torrents().iter().any(|t| t.hash == OLD_HASH));
    assert_eq!(s.state_of(&v3()).await, RevisionState::Failed);
    let failures = s.failures().await;
    assert_eq!(one_failure(&failures).item_id, s.item(&v3()).await.id);
    assert!(s.can_retry(&v3()).await);
    assert_eq!(s.row_of(&v2()).await.new_version, 2);

    // `다시 받기` of `14v3`: it replaces `14v2` by the usual steps.
    s.tr.content_on_add(V3_HASH, V3_BYTES);
    s.tr.unfinished_on_add(V3_HASH);
    s.retry(
        s.item(&v3()).await.id,
        "00000000-0000-4000-8000-000000000b01",
    )
    .await
    .unwrap();
    assert_eq!(s.state_of(&v3()).await, RevisionState::Receiving);
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    s.complete(V3_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v3()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
    let removes = s.tr.calls_of("torrent-remove");
    let v2_removal = removes
        .iter()
        .find(|c| c.args["ids"] == serde_json::json!([NEW_HASH]))
        .expect("14v2's torrent is removed");
    assert_eq!(v2_removal.args["delete-local-data"], true);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert!(s.failures().await.is_empty());
}

// --- `다시 받기` looks at the video in the episode's place ------------------------------

/// `14v2` stopped before it was received; then `14v3` took the episode name
/// as an ordinary item. `다시 받기` of `14v2` is refused with the reason, adds
/// nothing, and the stopped replacement, which could replace nothing now,
/// ends as skipped.
#[tokio::test]
async fn a_retry_of_a_stopped_revision_lower_than_the_placed_video_is_refused() {
    let s = World::new().await;
    let item = s.stopped_after_leaving_the_feed().await;
    s.v3_placed_without_a_row().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Failed);

    let done = s
        .retry(item.id, "00000000-0000-4000-8000-000000000c01")
        .await;
    assert_refused(done, "같거나 더 높은 수정본");
    assert_eq!(s.added(NEW_HASH), 1, "not added again");
    assert_eq!(s.item(&v2()).await.result, HistoryResult::Received);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Skipped);
    assert!(s.failures().await.is_empty());
    s.cycle().await;
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
    assert_eq!(s.added(NEW_HASH), 1);
}

/// A `버전 미상` `14v2` (no CRC32 in its name), and `14v3` placed since as an
/// ordinary item whose torrent has left Transmission: its CRC32 tells it is
/// `14v3`, and `다시 받기` of `14v2` is refused with the reason. The item stays
/// `버전 미상`, so a later request looks at the folder again.
#[tokio::test]
async fn a_retry_of_a_version_unknown_revision_lower_than_the_placed_video_is_refused() {
    let s = World::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    s.feed(&[(NEW_HASH, &v2), (OLD_HASH, &v1())]);
    s.cycle().await;
    let item = s.item(&v2).await;
    assert_eq!(item.result, HistoryResult::VersionUnknown);
    s.v3_placed_without_a_row().await;
    s.tr.remove(V3_HASH);

    let done = s
        .retry(item.id, "00000000-0000-4000-8000-000000000c02")
        .await;
    assert_refused(done, "같거나 더 높은 수정본");
    assert_eq!(s.added(NEW_HASH), 0);
    assert_eq!(s.item(&v2).await.result, HistoryResult::VersionUnknown);
    assert_eq!(s.state_of(&v2).await, RevisionState::Unknown);
    assert_eq!(read(&s.file(EPISODE_NAME)), V3_BYTES);
}

/// `다시 받기` of a `버전 미상` revision while Transmission does not answer:
/// the episode's video cannot be told, so nothing is added, and the command
/// says the person can ask again later (nothing tries again by itself).
#[tokio::test]
async fn a_retry_that_cannot_look_at_the_episode_says_to_ask_again() {
    let mut s = World::new().await;
    s.received_v1().await;
    let v2 = release("v2", None);
    s.feed(&[(NEW_HASH, &v2), (OLD_HASH, &v1())]);
    s.cycle().await;
    let item = s.item(&v2).await;
    assert_eq!(item.result, HistoryResult::VersionUnknown);

    s.tr.stop().await;
    let done = s
        .retry(item.id, "00000000-0000-4000-8000-000000000c03")
        .await;
    s.tr.restart().await;
    let finished = done.expect("the command ended");
    assert_eq!(finished.state, CommandState::Failed, "{finished:?}");
    let reason = finished.outcome.reason.as_deref().unwrap();
    assert!(reason.contains("확인하지 못해서"), "{finished:?}");
    assert!(
        reason.contains("다시 받기를 다시 누를 수 있어요"),
        "{finished:?}"
    );
    assert_eq!(s.added(NEW_HASH), 0);
    assert_eq!(s.state_of(&v2).await, RevisionState::Unknown);
}

// --- a replacement that ended with no video left ------------------------------------------

/// `14v2` removed `14` and then lost its video, while its torrent is still
/// in Transmission: the ended replacement is a `받기 실패` with `다시 받기`.
/// Receiving it again has Transmission check the torrent's data (it finds
/// the file gone) and start it, and the replacement starts over from its
/// first step: the downloaded video is checked and takes the episode name.
#[tokio::test]
async fn a_replacement_ended_with_no_video_left_is_received_again_with_retry() {
    let s = World::new().await;
    s.v2_waits_for_its_name().await;
    std::fs::remove_file(s.file(&v2())).unwrap();
    s.cycle().await;
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert!(s.tr.torrents().iter().any(|t| t.hash == NEW_HASH));
    let old_adds = s.added(OLD_HASH);

    let item = s.item(&v2()).await;
    let failures = s.failures().await;
    assert_eq!(one_failure(&failures).item_id, item.id);
    assert!(s.can_retry(&v2()).await);

    s.tr.reject_rename_of(NEW_HASH, None);
    s.retry(item.id, "00000000-0000-4000-8000-000000000d01")
        .await
        .unwrap();
    assert_eq!(s.added(NEW_HASH), 2, "asked for again");
    assert_eq!(
        s.tr.mutations()
            .iter()
            .filter(|m| m.starts_with("torrent-verify") || m.starts_with("torrent-start"))
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            format!("torrent-start ids=[\"{NEW_HASH}\"]"),
            format!("torrent-verify ids=[\"{NEW_HASH}\"]"),
        ]
    );
    assert_eq!(s.item(&v2()).await.result, HistoryResult::Received);
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Receiving);
    assert_eq!(row.received_name, None);
    assert_eq!(row.new_missing_at, None);
    assert!(s.failures().await.is_empty());

    // Transmission downloads it again.
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    std::fs::write(s.file(&v2()), NEW_BYTES).unwrap();
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(s.names(), vec![EPISODE_NAME]);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
    assert_eq!(
        s.added(OLD_HASH),
        old_adds,
        "the old release is not received again"
    );
    assert!(s.failures().await.is_empty());
}

/// The rule of a replacement that ended with no video under the episode name
/// is archived for ten days: the failure and its `다시 받기` are there once
/// the rule is restored, and `다시 받기` receives it again.
#[tokio::test]
async fn an_ended_replacement_of_a_rule_archived_for_ten_days_is_retried_after_its_restore() {
    let s = World::new().await;
    s.v2_ended_with_no_video().await;

    let archived = s.archive().await;
    s.cycle().await;
    s.advance(10 * 24 * 60 * 60 * 1000);
    s.cycle().await;

    s.restore(&archived).await;
    s.cycle().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state.code(), "abandoned", "{row:?}");
    assert!(row.reason.is_some(), "{row:?}");
    assert!(s.can_retry(&v2()).await);
    s.retry(
        s.item(&v2()).await.id,
        "00000000-0000-4000-8000-000000000d08",
    )
    .await
    .unwrap();
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
}

/// Another file has taken `14v2`'s received name since its replacement
/// ended. Checking the torrent would have Transmission take that file for
/// its own and write over it: `다시 받기` is refused with the reason, nothing
/// is asked of Transmission, the file stays, and the failure keeps
/// `다시 받기`.
#[tokio::test]
async fn a_retry_whose_received_name_holds_another_file_is_refused() {
    let s = World::new().await;
    s.v2_ended_with_no_video().await;
    std::fs::write(s.file(&v2()), b"someone else's file").unwrap();

    let done = s
        .retry(
            s.item(&v2()).await.id,
            "00000000-0000-4000-8000-000000000d02",
        )
        .await;
    assert_refused(done, "다른 파일이 있어서");
    assert_eq!((s.verifies(), s.starts()), (0, 0));
    assert_eq!(read(&s.file(&v2())), b"someone else's file");
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert!(s.can_retry(&v2()).await);
}

/// `14v2`'s own video is back under its received name (the CRC32 checked
/// before): `다시 받기` goes ahead, and the replacement takes the name.
#[tokio::test]
async fn a_retry_whose_received_name_holds_the_checked_video_goes_ahead() {
    let s = World::new().await;
    s.v2_ended_with_no_video().await;
    let copy = s.season.parent().unwrap().join("copy.mkv");
    std::fs::write(&copy, NEW_BYTES).unwrap();
    std::fs::rename(&copy, s.file(&v2())).unwrap();

    s.retry(
        s.item(&v2()).await.id,
        "00000000-0000-4000-8000-000000000d03",
    )
    .await
    .unwrap();
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    // Transmission finds the data whole and seeds it.
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}

/// Transmission refuses to check `14v2`'s torrent: it is not started (it
/// would seed a file that is not there), the command fails, and the
/// failure keeps `다시 받기`, which goes through once the check does.
#[tokio::test]
async fn a_retry_whose_torrent_check_fails_starts_nothing_and_can_be_asked_again() {
    let s = World::new().await;
    s.v2_ended_with_no_video().await;
    s.tr.reject_verify_of(NEW_HASH, Some("busy"));

    let done = s
        .retry(
            s.item(&v2()).await.id,
            "00000000-0000-4000-8000-000000000d04",
        )
        .await;
    assert_eq!(done.unwrap().state, CommandState::Failed);
    assert_eq!(s.starts(), 0);
    assert_eq!(s.state_of(&v2()).await.code(), "abandoned");
    assert!(s.can_retry(&v2()).await);

    s.tr.reject_verify_of(NEW_HASH, None);
    s.retry(
        s.item(&v2()).await.id,
        "00000000-0000-4000-8000-000000000d05",
    )
    .await
    .unwrap();
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    assert_eq!(s.starts(), 1);
}

/// `14v2`'s torrent had left Transmission when it was received again, and
/// Transmission says the new torrent is complete with no file under its
/// name. The replacement removed `14` itself, so the empty episode name is
/// no sign the failure was resolved: it stays a failure before the video
/// was received, with `다시 받기`, which has Transmission check the torrent
/// and download the file.
#[tokio::test]
async fn a_replacement_received_again_whose_file_is_not_there_stays_a_failure() {
    let s = World::new().await;
    s.v2_ended_with_no_video().await;
    s.tr.remove(NEW_HASH);
    s.retry(
        s.item(&v2()).await.id,
        "00000000-0000-4000-8000-000000000d06",
    )
    .await
    .unwrap();
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    // Transmission says it is whole and seeds it; its file is gone again.
    s.complete(NEW_HASH);
    std::fs::remove_file(s.file(&v2())).unwrap();

    s.cycle().await;
    s.cycle().await;
    let row = s.row_of(&v2()).await;
    assert_eq!(row.state, RevisionState::Failed, "{row:?}");
    assert_eq!(row.received_name, None);
    // The replacement removed the old video before it was received again:
    // a failure before the new video was received, with the old one gone.
    let failures = s.failures().await;
    let failure = one_failure(&failures);
    assert!(failure.not_received(), "{failure:?}");
    assert!(failure.claimed_at.is_some(), "{failure:?}");
    assert!(s.can_retry(&v2()).await);

    s.retry(
        s.item(&v2()).await.id,
        "00000000-0000-4000-8000-000000000d07",
    )
    .await
    .unwrap();
    assert_eq!(s.verifies(), 1);
    assert_eq!(s.state_of(&v2()).await, RevisionState::Receiving);
    std::fs::write(s.file(&v2()), NEW_BYTES).unwrap();
    s.complete(NEW_HASH);
    s.cycle().await;
    assert_eq!(s.state_of(&v2()).await, RevisionState::Done);
    assert_eq!(read(&s.file(EPISODE_NAME)), NEW_BYTES);
}
