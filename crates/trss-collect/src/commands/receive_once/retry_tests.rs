//! `다시 받기` of an item a rule picked and Transmission refused
//! (`receive_once::run` on a command the web accepted; tickets 0008, 0101):
//! where the torrent goes, how the file is named, and how a failed add ends.
//! The cases run through [`World::retry`] against the fake Transmission.

use serde_json::json;
use trss_core::commands::{Accepted, CommandState};
use trss_transmission::BOT_LABEL;

use super::fixtures::*;
use crate::{
    commands::{
        receive_once::{retry_plan_for, revision_retry, NotRetryable, RevisionRetry},
        rule_archive::{self, Direction, MOVED, MOVING_FIRST},
    },
    store::{
        channels::RuleState,
        history::HistoryResult,
        revisions::{NewRevision, OldVideo, RevisionState, Step},
    },
    test_world::{files, World},
};

#[tokio::test]
async fn a_failed_item_is_added_again_into_its_rules_folder() {
    let s = World::with_rules(picked_rules()).await;
    s.fail_adds(&[(&hash(26), LIAR), (&hash(3), OTHER)]).await;
    let item = s.item_containing("LIAR GAME - 26").await;
    let rule = s.rule_of(0).await;
    assert_eq!(item.result, HistoryResult::AddFailed);
    assert_eq!(item.rule_id.as_deref(), Some(rule.id.as_str()));
    assert!(s.tr.torrents().is_empty());

    let finished = s.retry(item.id, CMD).await.unwrap();
    assert_eq!(finished.state, CommandState::Done);
    assert_eq!(finished.outcome.result, "received");
    assert_eq!(finished.outcome.reason, None);

    // In the rule's folder, with the bot label and a trname name.
    let torrents = s.tr.torrents();
    assert_eq!(torrents.len(), 1);
    assert_eq!(torrents[0].hash, hash(26));
    assert_eq!(
        torrents[0].download_dir,
        s.media.join(LIAR_DIR).to_str().unwrap()
    );
    // The add carried the command's label too; the command took it off once
    // it had recorded the torrent.
    let item_label = s.item_label(&item);
    let add = &s.tr.calls_of("torrent-add")[0];
    assert_eq!(
        add.args["labels"],
        json!([BOT_LABEL, item_label, format!("trss-cmd:{CMD}")])
    );
    assert_eq!(torrents[0].labels, [BOT_LABEL, item_label.as_str()]);
    assert_eq!(torrents[0].name, "LIAR GAME S01E26.mkv");
    // The item is the rule's now: it shows `규칙 ‘…’`, not a receive by hand.
    let received = s.item_containing("LIAR GAME - 26").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert_eq!(received.rule_id.as_deref(), Some(rule.id.as_str()));
    assert_eq!(received.reason, None, "a renamed file needs no note");
    assert_eq!(received.torrent_hash.as_deref(), Some(hash(26).as_str()));
    // Its neighbour was not touched, and no rule was made.
    assert_eq!(
        s.item_containing("Another Show").await.result,
        HistoryResult::AddFailed
    );
    let rules = s.ctx.channels.list_rules(&s.channel_id).await.unwrap();
    assert_eq!(rules.len(), 2);
    // The command ended with the outcome the screen's poll reads.
    let command = s.command(CMD).await;
    assert_eq!(command.state, CommandState::Done);
    assert_eq!(command.outcome.unwrap().result, "received");
    s.assert_secret_nowhere(CMD).await;
}

#[tokio::test]
async fn a_command_stored_with_an_empty_folder_before_the_change_runs_as_a_retry() {
    let (s, item) = World::liar_failed().await;
    s.accept_receive(
        CMD,
        &format!(r#"{{"item_id":{},"folder":""}}"#, item.id),
        item.id,
    )
    .await;
    let command = s.claim().await;

    let finished = s.run_command(&command).await.unwrap();

    assert_eq!(finished.outcome.result, "received");
    let torrents = s.tr.torrents();
    assert_eq!(torrents.len(), 1);
    assert_eq!(
        torrents[0].download_dir,
        s.media.join(LIAR_DIR).to_str().unwrap()
    );
    assert_eq!(torrents[0].name, "LIAR GAME S01E26.mkv");
    assert_eq!(s.command(CMD).await.outcome.unwrap().result, "received");
}

// --- an add that does not go through ---------------------------------------------------

/// What makes the add of a retry fail.
#[derive(Debug, Clone, Copy)]
enum Trouble {
    /// Transmission is not running: nothing was sent.
    Stopped,
    /// Transmission answers and refuses the torrent.
    Refuses,
}

#[tokio::test]
async fn an_add_that_fails_leaves_add_failed_with_its_reason_and_the_rule() {
    for (trouble, says) in [
        (Trouble::Stopped, "Transmission에 연결하지 못했어요"),
        (Trouble::Refuses, "duplicate or corrupt torrent"),
    ] {
        let (mut s, item) = World::liar_failed().await;
        match trouble {
            Trouble::Stopped => s.tr.stop().await,
            Trouble::Refuses => s.tr.reject_adds(Some("duplicate or corrupt torrent")),
        }

        let finished = s.retry(item.id, CMD).await.unwrap();

        let failed = s.item_containing("LIAR GAME - 26").await;
        assert_eq!(failed.result, HistoryResult::AddFailed, "{trouble:?}");
        assert_eq!(
            failed.rule_id, item.rule_id,
            "{trouble:?}: still the rule's, so it can be tried again"
        );
        let reason = failed.reason.expect("a reason");
        assert!(reason.contains(says), "{trouble:?}: {reason}");
        assert_eq!(finished.state, CommandState::Failed, "{trouble:?}");
        assert_eq!(finished.outcome.result, "add_failed");
        assert_eq!(finished.outcome.reason.as_deref(), Some(reason.as_str()));
        assert_eq!(s.command(CMD).await.state, CommandState::Failed);
        assert!(s.tr.torrents().is_empty(), "{trouble:?}");
        s.assert_secret_nowhere(CMD).await;

        // It ended: Transmission coming back does not resurrect it.
        if let Trouble::Stopped = trouble {
            s.tr.restart().await;
        }
        assert!(
            s.ctx.commands.claim_next(s.now()).await.unwrap().is_none(),
            "{trouble:?}"
        );
    }
}

#[tokio::test]
async fn a_retry_that_failed_can_be_tried_again_with_a_new_command() {
    let (s, item) = World::liar_failed().await;
    s.tr.reject_adds(Some("nope"));
    s.retry(item.id, CMD).await.unwrap();
    let failed = s.item_containing("LIAR GAME - 26").await;
    assert_eq!(failed.result, HistoryResult::AddFailed);
    assert_eq!(failed.rule_id, item.rule_id);

    s.tr.reject_adds(None);
    // The earlier command has ended, so the store accepts another.
    let finished = s.retry(item.id, SECOND_CMD).await.unwrap();

    assert_eq!(finished.state, CommandState::Done);
    let received = s.item_containing("LIAR GAME - 26").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert_eq!(received.rule_id, item.rule_id);
    assert_eq!(received.reason, None);
    assert_eq!(s.tr.torrents().len(), 1);
}

#[tokio::test]
async fn a_retry_into_an_archived_work_folder_ends_saying_so_pauses_the_rule_and_the_move_runs() {
    let s = World::with_rules(vec![rule("Clevatess", "Clevatess/Season 02")])
        .await
        .with_archive_folder()
        .await;
    let rule = s.resumed(0, 777).await;
    // Transmission refused the rule's next episode: `다시 받기` is offered.
    s.fail_adds(&[(&hash(2), "Clevatess S02E02.mkv")]).await;
    let item = s.item_containing("Clevatess S02E02").await;
    // The work folder went to the archive folder meanwhile.
    s.seeding_in(
        1,
        "Clevatess S02E01.mkv",
        &s.archive.join("Clevatess/Season 02"),
    );

    // The retry ends first, and the move it asked for is open: the rule is off.
    let finished = s.retry(item.id, CMD).await.unwrap();

    assert_eq!(finished.state, CommandState::Failed, "{finished:?}");
    let reason = finished.outcome.reason.unwrap();
    assert!(reason.contains("먼저 수집 폴더로 옮기고"), "{reason}");
    assert_eq!(reason, MOVING_FIRST);
    assert!(s.tr.calls_of("torrent-add").is_empty());
    assert!(!s.media.join("Clevatess").exists());
    // The item is as it was, and the rule is off with its move open.
    assert_eq!(
        s.item_containing("Clevatess S02E02").await.result,
        item.result
    );
    let stored = s.stored_rule(&rule).await;
    assert_eq!(stored.state, RuleState::Paused);
    assert_eq!(stored.resumed_at, Some(777));
    let asked = rule_archive::ask_start(&s.ctx.commands, &rule.id, Direction::Resume, s.now())
        .await
        .unwrap();
    let Accepted::Busy(open) = asked else {
        panic!("no start is open: {asked:?}");
    };
    assert_eq!(
        open.payload,
        format!(r#"{{"rule_id":"{}","direction":"start"}}"#, rule.id)
    );

    // The move ends; the retry then goes into the folder that came over.
    let start = s.claim().await;
    assert_eq!(start.id, open.id);
    let moved = s.run_archive(&start).await.unwrap();
    assert_eq!(moved.outcome.result, MOVED, "{moved:?}");
    let stored = s.stored_rule(&rule).await;
    assert_eq!(stored.state, RuleState::Active);
    assert_eq!(stored.resumed_at, Some(777));
    assert!(!s.archive.join("Clevatess").exists());
    assert_eq!(
        files(&s.media),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    s.tr.content_on_add(&hash(2), b"video");
    s.tr.seeding_on_add(&hash(2));
    let finished = s.retry(item.id, SECOND_CMD).await.unwrap();
    assert_eq!(finished.outcome.result, "received", "{finished:?}");
    assert_eq!(
        s.tr.torrent(&hash(2)).download_dir,
        s.media.join("Clevatess/Season 02").to_str().unwrap()
    );
}

/// The reason of a revision whose torrent vanished from Transmission before it
/// was received (`revisions::RECEIVE_STOPPED`).
const STOPPED: &str =
    "새 영상의 토렌트가 Transmission에서 사라져 받기가 끝나지 않았어요. 이전 영상은 그대로 있어요.";

/// The worker's verdict on a stopped revision (`RevisionRetry::Overtaken`) comes
/// from the rows of the episode alone, so the web's check that a file is at the
/// episode name cannot make it looser: a higher revision done for the episode
/// refuses the retry whatever the folder holds (here, nothing).
#[tokio::test]
async fn a_stopped_revision_below_a_done_one_stays_refused_whatever_the_folder_holds() {
    let s = World::with_rules(picked_rules()).await;
    s.fail_adds(&[(&hash(26), LIAR), (&hash(3), OTHER)]).await;
    let item = s.item_containing("LIAR GAME - 26").await;
    let higher = s.item_containing("Another Show").await;
    let rule = s.rule_of(0).await;
    let row = |item_id, version, hash: &str| NewRevision {
        item_id,
        old_item_id: None,
        rule_id: rule.id.clone(),
        folder: s.media.join(LIAR_DIR).to_str().unwrap().to_owned(),
        episode_name: "LIAR GAME S01E26.mkv".into(),
        old_version: Some(1),
        new_version: version,
        old_crc: None,
        expected_crc: Some("8F2EFEC2".into()),
        torrent_hash: Some(hash.to_owned()),
        state: RevisionState::Receiving,
        reason: None,
    };
    let revisions = &s.ctx.revisions;

    // The revision 2 whose download stopped before it was received.
    let stopped = revisions
        .create(10, row(item.id, 2, &hash(26)))
        .await
        .unwrap();
    let step = Step::Failed {
        reason: STOPPED.into(),
        received_name: None,
    };
    revisions
        .advance(stopped.id, 20, RevisionState::Receiving, step)
        .await
        .unwrap();
    assert!(
        matches!(
            revision_retry(revisions, item.id).await.unwrap(),
            RevisionRetry::Again(_)
        ),
        "nothing overtakes it yet"
    );

    // A revision 3 of the episode is done.
    let done = revisions
        .create(10, row(higher.id, 3, &hash(3)))
        .await
        .unwrap();
    let verified = Step::Verified {
        received_name: "v.mkv".into(),
        file_crc: "1A2B3C4D".into(),
        file_identity: "1:2:3:4:5:6:7".into(),
    };
    revisions
        .advance(done.id, 11, RevisionState::Receiving, verified)
        .await
        .unwrap();
    let old = OldVideo {
        item_id: None,
        version: Some(1),
        torrent_hash: None,
    };
    revisions.claim(done.id, 12, old).await.unwrap();
    let removed = Step::Removed { reason: None };
    revisions
        .advance(done.id, 13, RevisionState::Removing, removed)
        .await
        .unwrap();
    revisions
        .advance(done.id, 14, RevisionState::Removed, Step::Done)
        .await
        .unwrap();

    let revision = revision_retry(revisions, item.id).await.unwrap();
    assert_eq!(revision, RevisionRetry::Overtaken);
    let channel = s.ctx.channels.get_channel(&item.channel_id).await.unwrap();
    let plan = retry_plan_for(&item, channel.as_ref(), Some(&rule), &revision);
    assert_eq!(plan.map(|_| ()), Err(NotRetryable::Overtaken));
}
