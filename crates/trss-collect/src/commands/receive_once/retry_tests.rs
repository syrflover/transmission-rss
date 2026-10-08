//! `다시 받기` of an item a rule picked and Transmission refused
//! (`receive_once::run` on a command the web accepted; tickets 0008, 0101):
//! where the torrent goes, how the file is named, and how a failed add ends.
//! The cases run through [`World::retry`] against the fake Transmission.

use serde_json::json;
use trss_core::commands::CommandState;
use trss_transmission::BOT_LABEL;

use super::fixtures::*;
use crate::{store::history::HistoryResult, test_world::World};

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
