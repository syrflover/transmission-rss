//! What keeps the cycle's removal of departed torrents from meeting a web
//! command's add.
//!
//! A cycle removes the finished torrents it cannot account for (see
//! [`crate::worker::cycle::run_cycle`]): their hash is not in history for an item that
//! is still in a feed, and no label names such an item. A command that has
//! handed a torrent to Transmission and not yet recorded it looks exactly like
//! that. Commands now run while a cycle runs, so two things keep them apart:
//!
//! - **The torrent gate** ([`TorrentGate`]). Every command that may add, move,
//!   rename or take labels off torrents holds it shared from before it starts
//!   until its end (or the note of an add that got no answer) is written. The
//!   removal holds it alone, from reading the hashes history keeps through
//!   the last removal. So no removal runs between a command's add and the
//!   record that accounts for it, and the hashes the removal reads include
//!   every command that ended before it. The removal does not wait for the
//!   gate: it is periodic cleanup, and a removal waiting behind a long
//!   command (a move waits minutes for Transmission) would hold back every
//!   command after it. When a command holds the gate, the cycle removes
//!   nothing and the next cycle tries again.
//! - **The check at removal time** ([`Removal::commands`]), under the gate.
//!   A command still `running` that this worker is not carrying out now is
//!   one an earlier start left (a worker died in it, or this worker left it
//!   for a later look after Transmission did not answer its add): it may have
//!   put a torrent in that history does not know, so the cycle removes
//!   nothing. So is a command this worker is carrying out again
//!   ([`InFlight`] counts only the first start of a command as its own). A
//!   command whose add got no answer that ended since the previous cycle
//!   started counts the same way. A `rule_archive_batch` is not counted: it
//!   touches no torrent, and the `rule_archive` commands it stores do.
//!
//! The order is always the folder's turn first ([`trss_core::folder_locks`]),
//! then the gate: whoever holds the gate never waits for a folder.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use trss_core::{
    commands::{Command, CommandError, CommandStore},
    Millis,
};

/// The gate between the commands' adds and the cycle's removal (see the
/// module docs). Cheap to clone; clones share the gate.
#[derive(Clone, Default)]
pub struct TorrentGate(Arc<RwLock<()>>);

impl TorrentGate {
    /// A command's turn: shared with the other commands, never with a removal.
    pub(crate) async fn command(&self) -> OwnedRwLockReadGuard<()> {
        self.0.clone().read_owned().await
    }

    /// The removal's turn, alone, if no command holds the gate now. Never
    /// waits, so no command ever waits behind a removal that waits itself.
    pub(crate) fn try_removal(&self) -> Option<OwnedRwLockWriteGuard<()>> {
        self.0.clone().try_write_owned().ok()
    }
}

/// The commands this worker is carrying out now, from their claim until their
/// end is written. Cheap to clone; clones share the set.
#[derive(Clone, Default)]
pub struct InFlight(Arc<Mutex<HashMap<String, bool>>>);

impl InFlight {
    /// Notes `command`, just claimed, until the returned entry is dropped.
    pub(crate) fn enter(&self, command: &Command) -> Entry {
        // A first start (no earlier one, so no earlier add) is this worker's
        // own; a later one may follow an add nobody recorded.
        let first = command.attempts == 1 && !command.add_unconfirmed;
        self.set().insert(command.id.clone(), first);
        Entry {
            set: self.clone(),
            id: command.id.clone(),
        }
    }

    /// Every command carried out now.
    pub(crate) fn ids(&self) -> Vec<String> {
        self.set().keys().cloned().collect()
    }

    /// The commands carried out now on their first start.
    fn first_starts(&self) -> Vec<String> {
        self.set()
            .iter()
            .filter(|(_, first)| **first)
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn set(&self) -> std::sync::MutexGuard<'_, HashMap<String, bool>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A command in [`InFlight`]; it leaves the set when this is dropped.
pub(crate) struct Entry {
    set: InFlight,
    id: String,
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.set.set().remove(&self.id);
    }
}

/// What a cycle's removal of departed torrents waits for and checks. Built by
/// the worker for each cycle.
pub struct Removal {
    pub(crate) gate: TorrentGate,
    pub(crate) commands: CommandStore,
    pub(crate) in_flight: InFlight,
    /// When the previous cycle started, read before this one began.
    pub(crate) previous_start: Option<Millis>,
}

/// What the web commands look like when the removal is about to run, as far as
/// they bear on it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CommandsAtRemoval {
    /// Commands `running` that an earlier start left (see the module docs).
    pub running: usize,
    /// Commands ended since the previous cycle started whose add to
    /// Transmission got no answer (or whose task panicked): Transmission may
    /// hold their torrent under a hash history does not know.
    pub unconfirmed_adds: usize,
}

impl Removal {
    /// What the commands say now. Asked with the gate held for the removal.
    pub(crate) async fn commands(&self) -> Result<CommandsAtRemoval, CommandError> {
        Ok(CommandsAtRemoval {
            running: self
                .commands
                .running_count_excluding_kinds(
                    self.in_flight.first_starts(),
                    // A batch of archives touches no torrent itself; the
                    // commands it stores for its rules do, and count on their own.
                    vec![crate::archive_batch::KIND.to_owned()],
                )
                .await?,
            unconfirmed_adds: self
                .commands
                .unconfirmed_adds_since(self.previous_start)
                .await?,
        })
    }
}

#[cfg(test)]
mod tests {
    use trss_core::{
        commands::{CommandStore, NewCommand},
        Db,
    };

    use super::*;

    fn removal(commands: &CommandStore) -> Removal {
        Removal {
            gate: TorrentGate::default(),
            commands: commands.clone(),
            in_flight: InFlight::default(),
            previous_start: None,
        }
    }

    async fn claim_twice(commands: &CommandStore, id: &str, kind: &str) {
        commands
            .accept(
                NewCommand {
                    id: id.to_owned(),
                    kind: kind.to_owned(),
                    payload: "{}".to_owned(),
                    subject: Some(id.to_owned()),
                },
                1_000,
            )
            .await
            .unwrap();
        // The second claim is the restart's: the first start is not this
        // worker's, so nothing in `InFlight` vouches for the command.
        for _ in 0..2 {
            commands.claim_next(2_000).await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn a_batch_claimed_again_after_a_restart_does_not_hold_back_the_removal() {
        let commands = CommandStore::new(Db::open_blocking(":memory:").unwrap());
        claim_twice(&commands, "batch-1", crate::archive_batch::KIND).await;

        assert_eq!(removal(&commands).commands().await.unwrap().running, 0);
    }

    #[tokio::test]
    async fn a_torrent_command_claimed_again_after_a_restart_still_holds_it_back() {
        let commands = CommandStore::new(Db::open_blocking(":memory:").unwrap());
        claim_twice(&commands, "once-1", "receive_once").await;

        assert_eq!(removal(&commands).commands().await.unwrap().running, 1);
    }
}
