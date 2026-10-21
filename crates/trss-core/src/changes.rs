//! What changed, told to whoever listens: the channel the screens' event
//! stream (`GET /api/events`) is fed from.
//!
//! The code that changes a state a screen shows announces the kind of the
//! change and the ID of what changed, after the change is committed. It does
//! not say what the state became: a screen that hears of a change reads again
//! through the route it read the state from, so the data has one source.
//!
//! The channel keeps a bounded number of changes for a listener that is slow
//! to take them. A listener that falls further behind loses the oldest ones
//! and is told how many it lost (`broadcast::error::RecvError::Lagged`); it
//! must then read everything it shows again, as it must for any change it
//! could not have heard (a connection that dropped).

use serde::Serialize;
use tokio::sync::broadcast;

/// How many unheard changes the channel keeps per listener unless
/// [`Changes::with_capacity`] says otherwise.
pub const DEFAULT_CAPACITY: usize = 1024;

/// What kind of thing changed, which says which routes a screen reads it
/// through and what the ID in [`Change`] names. The list grows with the
/// screens that move from reading on a timer to hearing of changes; nothing
/// may depend on it being complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// A web command was accepted, started or ended; the ID is the command's.
    Command,
    /// A past episode search started, got further or ended; the ID is the
    /// search's.
    PastSearch,
}

/// One announced change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Change {
    pub kind: ChangeKind,
    /// The ID of what changed, in the kind's own ID space.
    pub id: String,
}

/// The handle to the channel. Cheap to clone; clones announce to and listen
/// on the same channel. One made with `default` has no listener until
/// somebody subscribes, so a store given none announces to nobody.
#[derive(Clone)]
pub struct Changes {
    sender: broadcast::Sender<Change>,
}

impl Default for Changes {
    fn default() -> Self {
        Changes::with_capacity(DEFAULT_CAPACITY)
    }
}

impl Changes {
    /// A channel that keeps `capacity` unheard changes per listener (tests use
    /// a tiny one to make a listener lag).
    pub fn with_capacity(capacity: usize) -> Self {
        Changes {
            sender: broadcast::channel(capacity).0,
        }
    }

    /// Tells the listeners that the thing `id` of `kind` changed. Having no
    /// listener is no failure.
    pub fn announce(&self, kind: ChangeKind, id: impl Into<String>) {
        let _ = self.sender.send(Change {
            kind,
            id: id.into(),
        });
    }

    /// Hears the changes announced from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<Change> {
        self.sender.subscribe()
    }
}

#[cfg(test)]
mod tests;
