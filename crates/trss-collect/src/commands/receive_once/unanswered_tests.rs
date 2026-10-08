//! An add of `다시 받기` that got no answer from Transmission, though
//! Transmission may have taken the torrent (`Retry::AddUnanswered`, ticket
//! 0013), and the torrent a later start meets in the rule's folder: it is the
//! command's own only when it carries the command's label. A command's start
//! after the first is [`World::look_again`]; the process parts (the cycle
//! that removes nothing meanwhile, the lock, a worker that dies) are the
//! worker's.

use std::time::Duration;

use trss_core::commands::{Command, CommandState, MAX_ATTEMPTS};
use trss_transmission::{fake::FakeTorrent, BOT_LABEL};

use super::fixtures::*;
use crate::{
    commands::receive_once::{ReceiveOnce, Retry},
    store::history::{HistoryItem, HistoryResult},
    test_world::World,
};

/// How long the client waits for an answer in these tests.
const PATIENCE: Duration = Duration::from_millis(300);

impl World {
    /// [`World::liar_failed`] with a client that gives up on an answer soon.
    async fn liar_failed_impatient() -> (World, HistoryItem) {
        let (s, item) = World::liar_failed().await;
        (s.impatient(PATIENCE), item)
    }

    /// A command whose first add got no answer though Transmission took the
    /// torrent, claimed for the next look.
    async fn after_an_unanswered_add(&self, item: &HistoryItem) -> Command {
        let first = self.start_retry(item.id, CMD).await;
        let late = self.tr.hold_answer("torrent-add");
        let ran = self.run_command(&first).await;
        assert!(matches!(ran, Err(Retry::AddUnanswered)), "{ran:?}");
        late.release_all();
        self.look_again(&first).await
    }
}

#[tokio::test]
async fn a_command_add_that_timed_out_after_transmission_took_it_is_received_on_the_next_look() {
    let (s, item) = World::liar_failed_impatient().await;
    let first = s.start_retry(item.id, CMD).await;
    // Transmission takes the torrent but answers only after the client gave up.
    let late = s.tr.hold_answer("torrent-add");

    let ran = s.run_command(&first).await;

    // Not ended: the command waits for the next look.
    assert!(matches!(ran, Err(Retry::AddUnanswered)), "{ran:?}");
    assert_eq!(s.command(CMD).await.state, CommandState::Running);
    assert_eq!(s.tr.torrents().len(), 1, "Transmission has it all the same");
    assert_eq!(s.item_containing("LIAR GAME - 26").await.torrent_hash, None);

    // The next look adds it again; Transmission answers that it has it, with
    // its hash, and the command takes that torrent as its own.
    late.release_all();
    let second = s.look_again(&first).await;
    assert_eq!(second.attempts, 2);
    assert!(second.add_unconfirmed);
    let finished = s.run_command(&second).await.unwrap();

    assert_eq!(finished.state, CommandState::Done);
    assert_eq!(finished.outcome.result, "received");
    assert!(
        !finished.add_unconfirmed,
        "the hash is learned, so nothing is unaccounted for"
    );
    let held = s.item_containing("LIAR GAME - 26").await;
    assert_eq!(held.result, HistoryResult::Received);
    let torrent = s.tr.torrents().into_iter().next().unwrap();
    assert_eq!(held.torrent_hash.as_deref(), Some(torrent.hash.as_str()));
    assert_eq!(torrent.name, "LIAR GAME S01E26.mkv");

    // Kept by the cycles after that, while the item is in the feed.
    s.feed(&[(&hash(26), LIAR)]);
    s.cycle().await;
    s.cycle().await;
    assert!(s.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.tr.torrents().len(), 1);
}

/// What makes an add after an unanswered one fail.
#[derive(Debug, Clone, Copy)]
enum Trouble {
    /// Transmission refuses the torrent: fetching a `.torrent` URL again can
    /// fail (a one-time link, a rate limit) whatever Transmission holds.
    Refuses,
    /// Transmission restarts: the start cannot connect.
    Unreachable,
}

#[tokio::test]
async fn an_add_that_fails_after_an_unanswered_one_leaves_the_command_for_the_next_look() {
    for trouble in [Trouble::Refuses, Trouble::Unreachable] {
        let (mut s, item) = World::liar_failed_impatient().await;
        let look = s.after_an_unanswered_add(&item).await;
        match trouble {
            Trouble::Refuses => s.tr.reject_adds(Some("gotMetadataFromURL: http error 429")),
            Trouble::Unreachable => s.tr.stop().await,
        }

        let ran = s.run_command(&look).await;

        assert!(
            matches!(ran, Err(Retry::AddUnanswered)),
            "{trouble:?}: {ran:?}"
        );
        assert_eq!(s.command(CMD).await.state, CommandState::Running);
        assert_eq!(
            s.item_containing("LIAR GAME - 26").await.result,
            HistoryResult::AddFailed,
            "{trouble:?}: the item is as it was"
        );

        match trouble {
            Trouble::Refuses => s.tr.reject_adds(None),
            Trouble::Unreachable => s.tr.restart().await,
        }
        let next = s.look_again(&look).await;
        assert_eq!(next.attempts, 3, "{trouble:?}");
        let finished = s.run_command(&next).await.unwrap();
        assert_eq!(finished.outcome.result, "received", "{trouble:?}");
        assert_eq!(s.tr.torrents().len(), 1, "{trouble:?}");
    }
}

#[tokio::test]
async fn a_last_start_refused_after_an_unanswered_add_fails_and_keeps_the_add_unaccounted_for() {
    let (s, item) = World::liar_failed_impatient().await;
    let mut look = s.after_an_unanswered_add(&item).await;
    s.tr.reject_adds(Some("gotMetadataFromURL: http error 429"));

    // Every start but the last leaves the command for the next look.
    for attempts in 2..MAX_ATTEMPTS {
        assert_eq!(look.attempts, attempts);
        let ran = s.run_command(&look).await;
        assert!(matches!(ran, Err(Retry::AddUnanswered)), "{ran:?}");
        look = s.look_again(&look).await;
    }
    assert_eq!(look.attempts, MAX_ATTEMPTS);
    let finished = s.run_command(&look).await.unwrap();

    assert_eq!(finished.state, CommandState::Failed);
    assert_eq!(finished.outcome.result, "add_failed");
    // The first add's torrent is still unaccounted for: the command is ended
    // with that recorded, which makes the next cleanup remove nothing.
    assert!(finished.add_unconfirmed);
    let ended = s.command(CMD).await;
    assert_eq!(ended.state, CommandState::Failed);
    assert!(ended.add_unconfirmed);
    assert_eq!(s.tr.torrents().len(), 1);
}

#[tokio::test]
async fn a_deleted_channel_after_an_unanswered_add_ends_the_command_at_once() {
    let (s, item) = World::liar_failed_impatient().await;
    let look = s.after_an_unanswered_add(&item).await;
    // Trying again cannot learn the hash any more: the save folder and the
    // original link came from the channel.
    s.delete_channel().await;

    let finished = s.run_command(&look).await.unwrap();

    assert_eq!(finished.state, CommandState::Failed);
    assert!(finished.outcome.reason.unwrap().contains("채널이 삭제"));
    // The first add's torrent is still unaccounted for.
    assert!(finished.add_unconfirmed);
    assert!(s.command(CMD).await.add_unconfirmed);
}

// --- whose torrent it is -------------------------------------------------------------

/// How the command got to the torrent it meets.
#[derive(Debug, Clone, Copy)]
enum Before {
    /// An earlier start put the torrent in with the command's label and the
    /// worker died before it wrote the result.
    AnEarlierStart,
    /// The add found the torrent there already.
    Nothing,
    /// An add of the command got no answer before the torrent was there.
    AnUnansweredAdd,
}

#[tokio::test]
async fn a_torrent_in_the_rules_folder_is_the_commands_own_only_when_it_carries_its_label() {
    let in_the_folder = |s: &World| s.media.join(LIAR_DIR);
    // (what the torrent is, how the command got there, whether it is the
    // command's own)
    type Torrent = fn(&World, &HistoryItem) -> FakeTorrent;
    let cases: [(&str, Torrent, Before, bool); 4] = [
        (
            "the command's own add, by its label",
            World::taken_by_the_commands_add,
            Before::AnEarlierStart,
            true,
        ),
        (
            "a bot torrent without the command's label",
            |s, _| {
                FakeTorrent::new(&hash(26), LIAR)
                    .in_dir(s.media.join(LIAR_DIR))
                    .bot()
            },
            Before::Nothing,
            false,
        ),
        (
            "a torrent the bot did not add, after an unanswered add",
            |s, _| FakeTorrent::new(&hash(26), LIAR).in_dir(s.media.join(LIAR_DIR)),
            Before::AnUnansweredAdd,
            false,
        ),
        (
            "another item's bot torrent, after an unanswered add",
            |s, _| FakeTorrent {
                labels: vec![
                    BOT_LABEL.to_owned(),
                    trss_transmission::item_label("another-channel", "guid:another-item"),
                ],
                ..FakeTorrent::new(&hash(26), LIAR).in_dir(s.media.join(LIAR_DIR))
            },
            Before::AnUnansweredAdd,
            false,
        ),
    ];
    for (what, torrent, before, own) in cases {
        let (s, item) = World::liar_failed_impatient().await;
        s.tr.preload(torrent(&s, &item));
        let command = match before {
            Before::AnEarlierStart => {
                s.accept_receive(CMD, &ReceiveOnce::new(item.id).canonical(), item.id)
                    .await;
                s.claim().await;
                s.claim().await
            }
            Before::Nothing => s.start_retry(item.id, CMD).await,
            Before::AnUnansweredAdd => s.after_an_unanswered_add(&item).await,
        };

        let finished = s.run_command(&command).await.unwrap();

        let torrents = s.tr.torrents();
        assert_eq!(torrents.len(), 1, "{what}");
        let held = s.item_containing("LIAR GAME - 26").await;
        if own {
            // The rerun renames it and takes the label off, and the hash is recorded.
            assert_eq!(finished.outcome.result, "received", "{what}");
            assert_eq!(held.result, HistoryResult::Received, "{what}");
            assert_eq!(held.torrent_hash.as_deref(), Some(hash(26).as_str()));
            assert_eq!(torrents[0].name, "LIAR GAME S01E26.mkv", "{what}");
            assert_eq!(
                torrents[0].labels,
                [BOT_LABEL, s.item_label(&item).as_str()],
                "{what}"
            );
        } else {
            assert_eq!(finished.outcome.result, "duplicate", "{what}");
            assert_eq!(held.result, HistoryResult::Duplicate, "{what}");
            assert!(s.tr.calls_of("torrent-rename-path").is_empty(), "{what}");
            assert_eq!(torrents[0].name, LIAR, "{what}: its name is left alone");
            assert_eq!(held.reason, None, "{what}");
        }
        assert_eq!(
            torrents[0].download_dir,
            in_the_folder(&s).to_str().unwrap()
        );
    }
}
