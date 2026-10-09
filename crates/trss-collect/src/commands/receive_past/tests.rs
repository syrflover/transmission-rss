//! `받기` of a result of a past episode search (`receive_past::run` on a
//! command the web accepted; ticket 0026): what a confirmed result does, and
//! when a result that was offered again, because history says its episode is
//! gone from the work, is not added after all. The cases run through
//! [`World::past`] against the fake Transmission; the revision cases are in
//! `revisions/past_tests.rs`.

use std::time::Duration;

use trss_core::commands::{Accepted, CommandState};
use trss_transmission::fake::FakeTorrent;

use crate::{
    commands::{
        receive_once::Retry,
        rule_archive::{self, Direction, MOVED, MOVING_FIRST},
    },
    store::{
        channels::RuleState,
        history::{HistoryResult, Observation},
    },
    test_world::{show_hash, write, World, SECRET},
};

const CMD: &str = "0b7d5a44-6c1e-4c62-9a6a-3f0c1d2e4b55";
const SECOND_CMD: &str = "1e2d3c4b-0000-4000-8000-000000000002";
const THIRD_CMD: &str = "1e2d3c4b-0000-4000-8000-000000000003";

/// The title of the n-th episode of `[SubsPlease] Show`.
fn show(n: u32) -> String {
    format!("[SubsPlease] Show - {n:02} (1080p) [A{n:07}].mkv")
}

#[tokio::test]
async fn a_confirmed_result_is_recorded_as_a_past_item_and_added_into_the_rules_folder() {
    let s = World::new().await;
    let rule = s.rule_of(0).await;
    let four = s.past_result(&rule, &show_hash(4), &show(4));
    let nine = s.past_result(&rule, &show_hash(9), &show(9));
    // A search leaves nothing: history holds only what was confirmed.
    assert!(s.history_items().await.is_empty());

    for (result, id) in [(&four, CMD), (&nine, SECOND_CMD)] {
        let finished = s.past(result, id).await.unwrap();
        assert_eq!(finished.state, CommandState::Done, "{finished:?}");
        assert_eq!(finished.outcome.result, "received", "{finished:?}");
        assert_eq!(s.command(id).await.outcome.unwrap().result, "received");
    }

    // Each torrent once, into the rule's folder, named as the rule names it.
    assert_eq!((s.added(&show_hash(4)), s.added(&show_hash(9))), (1, 1));
    let torrents = s.tr.torrents();
    assert_eq!(torrents.len(), 2);
    for torrent in &torrents {
        assert_eq!(torrent.download_dir, s.season.to_str().unwrap());
    }
    // The items are in history as received by the rule, with the link as the
    // web gave it and the channel's address masked; no channel was made.
    let items = s.history_items().await;
    assert_eq!(items.len(), 2, "{items:?}");
    for (item, result) in [(&items[0], &nine), (&items[1], &four)] {
        assert_eq!(item.title, result.title);
        assert_eq!(item.link, result.link);
        assert_eq!(item.identity_key, result.key);
        assert_eq!(item.result, HistoryResult::Received);
        assert_eq!(item.rule_id.as_deref(), Some(rule.id.as_str()));
        assert!(item.torrent_hash.is_some());
        assert!(!item.channel_label.contains(SECRET), "{item:?}");
    }
    let channels = s.ctx.channels.list_channels_with_rules().await.unwrap();
    assert_eq!(channels.len(), 1);
    s.assert_secret_nowhere(CMD).await;
}

#[tokio::test]
async fn an_episode_received_from_another_release_can_be_chosen_by_hand_and_adds_the_chosen_torrent(
) {
    let s = World::new().await;
    let rule = s.rule_of(0).await;
    // Episode 6 was received from Erai-raws: Transmission holds that torrent
    // (history says so) and its file is still downloading.
    let erai = "[Erai-raws] Show - 06 [1080p][ABCD1234].mkv";
    let erai_hash = "eeee00000000000000000000000000000000000a";
    s.ctx
        .history
        .record(
            1,
            vec![Observation {
                channel_id: s.channel_id.clone(),
                channel_label: "nyaa".into(),
                identity_key: "guid:erai06".into(),
                title: erai.into(),
                link: format!("magnet:?xt=urn:btih:{erai_hash}"),
                result: HistoryResult::Received,
                rule_id: Some(rule.id.clone()),
                torrent_hash: Some(erai_hash.into()),
                reason: None,
            }],
        )
        .await
        .unwrap();
    write(&s.season.join("Show S01E06.mkv.part"), "x");

    // The person chooses SubsPlease's release of the same episode.
    let finished = s
        .past(&s.past_result(&rule, &show_hash(6), &show(6)), CMD)
        .await
        .unwrap();

    assert_eq!(finished.outcome.result, "received", "{finished:?}");
    assert_eq!(s.added(&show_hash(6)), 1);
    assert_eq!(s.tr.calls_of("torrent-add").len(), 1);
    assert_eq!(s.item(&show(6)).await.result, HistoryResult::Received);
}

// --- a result offered again, whose episode was gone when the search looked ------------

/// What changed after the search offered a received episode again.
#[derive(Debug, Clone, Copy)]
enum After {
    /// Nothing: the episode is still gone.
    Nothing,
    /// Its torrent is back in Transmission, added by hand.
    TheTorrentIsBack,
    /// Its video is back in the folder.
    TheVideoIsBack,
    /// The work folder is not there (a volume that is not mounted).
    TheFolderIsGone,
    /// The same episode came from another release, which Transmission holds.
    AnotherReleaseIsHeld,
}

#[tokio::test]
async fn a_received_result_is_added_again_only_while_its_torrent_and_video_are_both_gone() {
    for (after, added_again) in [
        (After::Nothing, true),
        (After::TheTorrentIsBack, false),
        (After::TheVideoIsBack, false),
        (After::TheFolderIsGone, false),
        (After::AnotherReleaseIsHeld, false),
    ] {
        let s = World::new().await;
        let rule = s.rule_of(0).await;
        let (hash, title) = (show_hash(5), show(5));
        let result = s.past_result(&rule, &hash, &title);
        let first = s.past(&result, CMD).await.unwrap();
        assert_eq!(first.outcome.result, "received", "{after:?}");
        assert!(s.tr.torrents().iter().any(|t| t.hash == hash), "{after:?}");

        // The person removed the torrent from Transmission; the video never
        // reached the folder here.
        s.tr.remove(&hash);
        match after {
            After::Nothing => {}
            After::TheTorrentIsBack => {
                s.tr.preload(FakeTorrent::new(&hash, &title).in_dir(&s.season));
            }
            After::TheVideoIsBack => write(&s.season.join("Show S01E05.mkv"), "x"),
            After::TheFolderIsGone => std::fs::remove_dir_all(&s.season).unwrap(),
            After::AnotherReleaseIsHeld => {
                let erai = "[Erai-raws] Show - 05 [1080p][ABCD1234].mkv";
                let erai_hash = "eeee00000000000000000000000000000000000a";
                s.ctx
                    .history
                    .record(
                        1,
                        vec![Observation {
                            channel_id: s.channel_id.clone(),
                            channel_label: "nyaa".into(),
                            identity_key: "guid:erai05".into(),
                            title: erai.into(),
                            link: format!("magnet:?xt=urn:btih:{erai_hash}"),
                            result: HistoryResult::Received,
                            rule_id: Some(rule.id.clone()),
                            torrent_hash: Some(erai_hash.into()),
                            reason: None,
                        }],
                    )
                    .await
                    .unwrap();
                s.tr.preload(FakeTorrent::new(erai_hash, erai).in_dir(&s.season));
            }
        }
        s.tr.clear_calls();

        let finished = s.past(&result, SECOND_CMD).await.unwrap();

        assert_eq!(
            finished.state,
            CommandState::Done,
            "{after:?}: {finished:?}"
        );
        assert_eq!(finished.outcome.result, "received", "{after:?}");
        let adds = s.tr.calls_of("torrent-add").len();
        assert_eq!(adds, usize::from(added_again), "{after:?}");
        if added_again {
            assert!(s.tr.torrents().iter().any(|t| t.hash == hash), "{after:?}");
        }
        // One history item for the episode, still received.
        let fives: Vec<_> = s
            .history_items()
            .await
            .into_iter()
            .filter(|i| i.title == title)
            .collect();
        assert_eq!(fives.len(), 1, "{after:?}");
        assert_eq!(fives[0].result, HistoryResult::Received, "{after:?}");
    }
}

// --- a start after an add that got no answer -----------------------------------------

/// How long the client waits for an answer in these tests.
const PATIENCE: Duration = Duration::from_millis(300);

/// A `받기` whose add got no answer though Transmission took the torrent, and
/// whose rule is paused before the next start: the command ends at once, the
/// torrent stays unaccounted for so the next cycle removes nothing
/// ([`crate::commands::receive_once::end_early`]), and the command's label
/// comes off the torrent.
#[tokio::test]
async fn a_paused_rule_after_an_unanswered_add_ends_the_command_with_the_add_unaccounted_for() {
    let s = World::new().await.impatient(PATIENCE);
    let rule = s.rule_of(0).await;
    let result = s.past_result(&rule, &show_hash(3), &show(3));
    let first = s.start_past(&result, CMD).await;
    let late = s.tr.hold_answer("torrent-add");

    let ran = s.run_command(&first).await;

    assert!(matches!(ran, Err(Retry::AddUnanswered)), "{ran:?}");
    late.release_all();
    assert_eq!(s.command(CMD).await.state, CommandState::Running);
    let label = trss_transmission::command_label(CMD);
    assert!(s.tr.torrents()[0].labels.contains(&label));

    s.ctx
        .channels
        .set_rule_state(&rule.id, RuleState::Paused, s.now())
        .await
        .unwrap();
    let look = s.look_again(&first).await;
    let finished = s.run_command(&look).await.unwrap();

    assert_eq!(finished.state, CommandState::Failed, "{finished:?}");
    assert!(finished.add_unconfirmed);
    let ended = s.command(CMD).await;
    assert_eq!(ended.state, CommandState::Failed);
    assert!(ended.add_unconfirmed);
    assert!(!s.tr.torrents()[0].labels.contains(&label));
    assert_eq!(s.tr.torrents().len(), 1);
}

// --- a work folder in the archive folder ----------------------------------------------

/// Ticket 0123, follow-up 2: a result received into a work folder that is in
/// the archive folder waits for the folder to come into the collect folder, as
/// every other path that adds into a rule's folder does.
#[tokio::test]
async fn a_result_received_into_an_archived_work_folder_waits_for_its_move_and_goes_in_after_it() {
    let s = World::new().await.with_archive_folder().await;
    let rule = s.rule_of(0).await;
    // The work folder went to the archive folder.
    std::fs::remove_dir_all(s.media.join("Show")).unwrap();
    write(&s.archive.join("Show/Season 01/Show S01E01.mkv"), "x");
    let result = s.past_result(&rule, &show_hash(2), &show(2));
    s.tr.content_on_add(&show_hash(2), b"video");
    s.tr.seeding_on_add(&show_hash(2));

    // The command ends first, and the move it asked for is open: the rule is off.
    let finished = s.past(&result, CMD).await.unwrap();

    assert_eq!(finished.state, CommandState::Failed, "{finished:?}");
    assert_eq!(finished.outcome.reason.as_deref(), Some(MOVING_FIRST));
    assert!(s.tr.calls_of("torrent-add").is_empty());
    assert!(!s.media.join("Show").exists());
    assert_eq!(s.stored_rule(&rule).await.state, RuleState::Paused);
    let asked = rule_archive::ask_start(&s.ctx.commands, &rule.id, Direction::Resume, s.now())
        .await
        .unwrap();
    let Accepted::Busy(open) = asked else {
        panic!("no start is open: {asked:?}");
    };

    // The move ends; the result then goes into the folder that came over.
    let start = s.claim().await;
    assert_eq!(start.id, open.id);
    let moved = s.run_archive(&start).await.unwrap();
    assert_eq!(moved.outcome.result, MOVED, "{moved:?}");
    assert_eq!(s.stored_rule(&rule).await.state, RuleState::Active);
    assert!(s.media.join("Show/Season 01/Show S01E01.mkv").exists());
    let finished = s.past(&result, THIRD_CMD).await.unwrap();
    assert_eq!(finished.outcome.result, "received", "{finished:?}");
    assert_eq!(
        s.tr.torrent(&show_hash(2)).download_dir,
        s.season.to_str().unwrap()
    );
}
