//! A `다시 받기` that ends at once, without adding anything (`receive_once::run`
//! on a command the web accepted, then found ineligible: the request was
//! old), and one that ends with the result Transmission's torrent has
//! already. Each ends the command with the reason, leaves the item as it
//! was, and takes the command's label off a torrent an earlier start may
//! have put in. The refusal at accept time is the web's.

use serde_json::json;
use trss_core::commands::{Command, CommandState};

use super::{fixtures::*, *};
use crate::{
    store::{
        channels::RuleInput,
        history::{HistoryItem, HistoryResult},
    },
    test_world::World,
};

/// What leaves an accepted retry with nothing to do.
#[derive(Debug, Clone, Copy)]
enum Cause {
    /// The rule that picked the item was deleted.
    RuleDeleted,
    /// The rule was archived.
    RuleArchived,
    /// The rule was edited so that it no longer picks the item, and a cycle
    /// judged the item again.
    NoRuleAnyMore,
    /// A database that has no collect folder.
    NoCollectFolder,
    /// The failure has no rule recorded: what the receive-once that took any
    /// item into a typed folder left behind.
    NoRuleRecorded,
    /// A command stored while a person chose the folder, whose earlier start
    /// put a torrent in under its label.
    LegacyFolder,
    /// The item was never picked and its channel is gone, with an earlier
    /// start's torrent under the command's label.
    NotPickedAndChannelGone,
    /// The rule asked to receive a past item was archived after the request.
    RuleArchivedAfterAskedForAPastItem,
}

/// A command made ready to run, the item as it was, and the sentence the
/// command ends with.
struct Arranged {
    s: World,
    command: Command,
    before: HistoryItem,
    says: &'static str,
    /// A torrent of the command's earlier start is in Transmission.
    left_a_torrent: bool,
}

impl Cause {
    async fn arrange(self) -> Arranged {
        match self {
            Cause::RuleDeleted
            | Cause::RuleArchived
            | Cause::NoRuleAnyMore
            | Cause::NoCollectFolder => {
                let (s, item) = World::liar_failed().await;
                let command = s.start_retry(item.id, CMD).await;
                let says = match self {
                    Cause::RuleDeleted => {
                        s.delete_rule(0).await;
                        "지워져서"
                    }
                    Cause::RuleArchived => {
                        s.archive_rule(0).await;
                        "복원한 뒤"
                    }
                    Cause::NoRuleAnyMore => {
                        let rule = s.rule_of(0).await;
                        s.ctx
                            .channels
                            .update_rule(
                                &rule.id,
                                rule.version,
                                &rule.channel_id,
                                RuleInput {
                                    r#match: Some("Something Else".into()),
                                    ..rule.to_input()
                                },
                            )
                            .await
                            .unwrap();
                        s.cycle().await;
                        "규칙이 고르지 않아서"
                    }
                    _ => {
                        // The app cannot unset the folder once it is set; this
                        // is a database that has none, as a fresh one does.
                        s.sql("DELETE FROM collection_settings");
                        "수집 폴더"
                    }
                };
                s.tr.clear_calls();
                let before = s.item(LIAR).await;
                Arranged {
                    s,
                    command,
                    before,
                    says,
                    left_a_torrent: false,
                }
            }
            Cause::NoRuleRecorded => {
                let s = World::with_rules(unrelated_rule()).await;
                s.feed(&[(&hash(26), LIAR)]);
                s.cycle().await;
                let item = s.item(LIAR).await;
                assert_eq!(item.result, HistoryResult::NoMatch);
                s.ctx
                    .history
                    .record_outcome(
                        item.id,
                        s.now(),
                        HistoryResult::AddFailed,
                        None,
                        Some("Transmission이 응답하지 않았어요".into()),
                        None,
                    )
                    .await
                    .unwrap();
                let before = s.item(LIAR).await;
                assert_eq!(before.rule_id, None);
                // A command accepted before, run now.
                s.accept_receive(
                    CMD,
                    &format!(r#"{{"item_id":{},"folder":""}}"#, item.id),
                    item.id,
                )
                .await;
                let command = s.claim().await;
                Arranged {
                    s,
                    command,
                    before,
                    says: "규칙 없이",
                    left_a_torrent: false,
                }
            }
            Cause::LegacyFolder => {
                let (s, item) = World::liar_failed().await;
                s.accept_receive(
                    CMD,
                    &format!(r#"{{"item_id":{},"folder":"Somewhere/Else"}}"#, item.id),
                    item.id,
                )
                .await;
                // An earlier start of it put a torrent in under its label, and
                // the worker died before it wrote the result.
                s.claim().await;
                s.tr.preload(s.taken_by_the_commands_add(&item));
                let command = s.claim().await;
                Arranged {
                    s,
                    command,
                    before: item,
                    says: "폴더를 고르던 예전 요청이라 실행하지 않았어요. 필요하면 다시 받기로 받아요.",
                    left_a_torrent: true,
                }
            }
            Cause::NotPickedAndChannelGone => {
                let s = World::with_rules(vec![rule("LIAR GAME", LIAR_DIR)]).await;
                s.fail_adds(&[(&hash(26), LIAR), (&hash(3), OTHER)]).await;
                let picked = s.item_containing("LIAR GAME - 26").await;
                let unpicked = s.item_containing("Another Show - 03").await;
                assert_eq!(unpicked.result, HistoryResult::NoMatch);
                s.delete_channel().await;
                // Accepted before the channel went; an earlier start left a
                // torrent with its label.
                s.accept_receive(CMD, &ReceiveOnce::new(unpicked.id).canonical(), unpicked.id)
                    .await;
                s.claim().await;
                s.tr.preload(s.taken_by_the_commands_add(&picked));
                let command = s.claim().await;
                Arranged {
                    s,
                    command,
                    before: unpicked,
                    says: "규칙이 고르지",
                    left_a_torrent: true,
                }
            }
            Cause::RuleArchivedAfterAskedForAPastItem => {
                let s = World::with_rules(unrelated_rule()).await;
                s.feed(&[(&hash(26), LIAR)]);
                s.cycle().await;
                s.advance(1_000);
                let sub = s.subscribe("LIAR GAME", LIAR_DIR, 3320, 1).await;
                let before = s.item(LIAR).await;
                let command = s.start_receive_for(before.id, &sub.id, CMD).await;
                s.archive_rule(1).await;
                Arranged {
                    s,
                    command,
                    before,
                    says: "복원한 뒤",
                    left_a_torrent: false,
                }
            }
        }
    }
}

#[tokio::test]
async fn an_accepted_retry_found_ineligible_ends_at_once_and_leaves_the_item_as_it_was() {
    for cause in [
        Cause::RuleDeleted,
        Cause::RuleArchived,
        Cause::NoRuleAnyMore,
        Cause::NoCollectFolder,
        Cause::NoRuleRecorded,
        Cause::LegacyFolder,
        Cause::NotPickedAndChannelGone,
        Cause::RuleArchivedAfterAskedForAPastItem,
    ] {
        let Arranged {
            s,
            command,
            before,
            says,
            left_a_torrent,
        } = cause.arrange().await;

        let finished = s.run_command(&command).await.unwrap();

        // The command ended `add_failed` with the reason, nothing went to
        // Transmission, and the item is as it was.
        assert_eq!(finished.state, CommandState::Failed, "{cause:?}");
        assert_eq!(finished.outcome.result, "add_failed", "{cause:?}");
        let reason = finished.outcome.reason.as_deref().unwrap();
        assert!(reason.contains(says), "{cause:?}: {reason}");
        let stored = s.command(CMD).await;
        assert_eq!(stored.state, CommandState::Failed, "{cause:?}");
        assert_eq!(stored.outcome, Some(finished.outcome.clone()), "{cause:?}");
        assert!(
            s.tr.calls_of("torrent-add").is_empty(),
            "{cause:?}: nothing went to Transmission"
        );
        assert_eq!(s.item(&before.title).await, before, "{cause:?}");
        if left_a_torrent {
            // The torrent is not removed; its command's label comes off.
            assert_eq!(s.tr.torrents().len(), 1, "{cause:?}");
            assert!(!s.carries_a_command_label(), "{cause:?}");
        }
    }
}

// --- the item is held already -----------------------------------------------------------

#[tokio::test]
async fn an_item_a_rule_added_after_the_request_ends_with_the_items_result_without_adding_again() {
    let (s, item) = World::liar_failed().await;
    let command = s.start_retry(item.id, CMD).await;
    // Before the worker gets to the command, the rule's own cycle adds the item.
    s.cycle().await;
    assert_eq!(s.tr.torrents()[0].name, "LIAR GAME S01E26.mkv");
    s.tr.clear_calls();

    let finished = s.run_command(&command).await.unwrap();

    // Nothing is added or named a second time.
    assert!(s.tr.calls_of("torrent-add").is_empty());
    assert!(s.tr.calls_of("torrent-rename-path").is_empty());
    assert_eq!(s.tr.torrents().len(), 1);
    // It ends with what the item says, without calling it a duplicate.
    assert_eq!(finished.state, CommandState::Done);
    assert_eq!(finished.outcome.result, "received");
    assert_eq!(finished.outcome.reason, None);
    let received = s.item_containing("LIAR GAME - 26").await;
    assert_eq!(received.result, HistoryResult::Received);
    assert_eq!(received.rule_id, item.rule_id);
    assert_eq!(received.reason, None);
}

#[tokio::test]
async fn items_the_rules_took_after_the_requests_end_with_the_items_results() {
    let s = World::with_rules(picked_rules()).await;
    s.fail_adds(&[(&hash(26), LIAR), (&hash(3), OTHER)]).await;
    let liar_item = s.item_containing("LIAR GAME - 26").await;
    let other_item = s.item_containing("Another Show").await;
    for item in [&liar_item, &other_item] {
        let id = if item.id == liar_item.id {
            CMD
        } else {
            SECOND_CMD
        };
        s.accept_receive(id, &ReceiveOnce::new(item.id).canonical(), item.id)
            .await;
    }
    // Before the worker gets to the commands, the rules take both items: one
    // is received, the other was in Transmission already.
    s.tr.preload(
        trss_transmission::fake::FakeTorrent::new(&hash(3), OTHER)
            .bot()
            .status(6),
    );
    s.cycle().await;
    // Then Transmission refuses adds, which the commands must not even try.
    s.tr.reject_adds(Some("nope"));
    s.tr.clear_calls();

    let first = s.claim().await;
    assert_eq!(first.id, CMD);
    let received = s.run_command(&first).await.unwrap();
    let second = s.claim().await;
    assert_eq!(second.id, SECOND_CMD);
    let duplicate = s.run_command(&second).await.unwrap();

    assert!(s.tr.calls_of("torrent-add").is_empty());
    assert_eq!(received.state, CommandState::Done);
    assert_eq!(received.outcome.result, "received");
    assert_eq!(received.outcome.reason, None);
    assert_eq!(duplicate.state, CommandState::Done);
    assert_eq!(duplicate.outcome.result, "duplicate");
    assert!(duplicate
        .outcome
        .reason
        .unwrap()
        .contains("이미 같은 토렌트"));
    assert_eq!(
        s.item_containing("LIAR GAME - 26").await.result,
        HistoryResult::Received
    );
    assert_eq!(
        s.item_containing("Another Show").await.result,
        HistoryResult::Duplicate
    );
}

#[tokio::test]
async fn a_held_item_whose_channel_is_gone_ends_with_its_result_and_the_label_comes_off() {
    let (s, item) = World::liar_failed().await;
    s.accept_receive(
        CMD,
        &format!(r#"{{"item_id":{},"folder":""}}"#, item.id),
        item.id,
    )
    .await;
    s.claim().await;
    // An earlier start's add put the torrent in; the rule's cycle then met it
    // and recorded the item, as it does when it finds one already there.
    s.tr.preload(s.taken_by_the_commands_add(&item));
    s.cycle().await;
    let held = s.item_containing("LIAR GAME - 26").await;
    assert_eq!(held.result, HistoryResult::Duplicate);
    s.delete_channel().await;
    let command = s.claim().await;

    let finished = s.run_command(&command).await.unwrap();

    assert_eq!(finished.state, CommandState::Done);
    assert_eq!(finished.outcome.result, "duplicate");
    assert_eq!(s.item_containing("LIAR GAME - 26").await, held);
    assert!(!s.carries_a_command_label());
}

#[tokio::test]
async fn a_torrent_transmission_already_had_ends_duplicate_and_the_cycle_keeps_it() {
    let (s, item) = World::liar_failed().await;
    // Transmission holds it already (the bot added it some time ago).
    s.tr.preload(
        trss_transmission::fake::FakeTorrent::new(&hash(26), LIAR)
            .bot()
            .status(6),
    );

    let finished = s.retry(item.id, CMD).await.unwrap();

    assert_eq!(
        s.item_containing("LIAR GAME - 26").await.result,
        HistoryResult::Duplicate
    );
    assert_eq!(finished.state, CommandState::Done);
    assert_eq!(finished.outcome.result, "duplicate");

    s.cycle().await;
    assert!(s.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.tr.torrents().len(), 1);
}

#[test]
fn the_request_that_names_a_folder_is_read_as_the_older_version_wrote_it() {
    // What an older version stored, as these tests accept it.
    let old = json!({ "item_id": 7, "folder": "Somewhere/Else" }).to_string();
    assert!(serde_json::from_str::<ReceiveOnce>(&old)
        .unwrap()
        .names_a_folder());
}
