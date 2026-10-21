//! What changed, told to whoever listens: the channel the screens' event
//! stream (`GET /api/events`) is fed from.
//!
//! A change is announced as the kind of what changed and the ID of the row it
//! changed, after it is committed. It does not say what the state became: a
//! screen that hears of a change reads again through the route it read the
//! state from, so the data has one source.
//!
//! Two kinds of announcer feed the channel. A store that keeps its state
//! outside the database tables (the web commands' queue, the memory-only past
//! episode search) announces by itself. Everything the database tables hold
//! is announced by the database connection: a [`crate::db::Db`] opened with
//! [`crate::db::Db::open_announcing`] notes, with TEMP triggers, the kind and
//! the target ID of every row a statement changed, and tells the channel what
//! it noted once the write is committed (`tables`, which holds the list of
//! tables and kinds). So the code that writes to a table does not announce.
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
/// through and what the ID in [`Change`] names. A kind of a database table is
/// a group of tables that screens read together; a table that is written
/// often and read by few screens has a kind of its own, so only those screens
/// read again when it changes. The list grows with the tables and the screens;
/// nothing may depend on it being complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// A web command was accepted, started or ended; the ID is the command's.
    Command,
    /// A past episode search started, got further or ended; the ID is the
    /// search's.
    PastSearch,
    /// A subtitle job, or what belongs to it: its items, steps, log, files,
    /// plan and replacements. The ID is the job's; none when the row names no
    /// job (an archive's members, a replacement plan's paths).
    SubtitleJob,
    /// A subtitle job's remote screen, which changes often and is read only by
    /// the screen of that job. The ID is the job's.
    SubtitleScreen,
    /// What a job kept of a work's subtitles: the stored copies, their assets
    /// and packages, the copies applied to episodes, and the cleanups of them.
    /// The ID is the work's; none when the row names no work.
    SubtitleStore,
    /// The episode mappings of subtitle sources and the person's exceptions to
    /// them. The ID is the work's; none for the mapping's version counter.
    SubtitleMapping,
    /// The library: watch folders, works, seasons, episodes and their files,
    /// and what a work's own settings say. The ID is the work's; none when the
    /// row names no work (a watch folder).
    Library,
    /// A work's cover and where it came from. The ID is the work's.
    Artwork,
    /// The season info of a work: the AniList entries linked to its seasons
    /// and the Anissia anime they follow. The ID is the work's; none for an
    /// AniList entry, which no work owns.
    SeasonInfo,
    /// Anissia's anime, the schedule snapshot and the subtitle sources and
    /// lines it lists. The ID is the Anissia anime number; none when the row
    /// names no anime.
    Anissia,
    /// Channels, rules and subscriptions. The ID is the rule's; none when the
    /// row names no rule (a channel).
    Rules,
    /// The collection history: the items found and how their results changed.
    /// The ID is the history item's; none when the row names no item.
    History,
    /// What the collection board shows: the collection cycle, the channels'
    /// reads and Transmission's snapshot. There is no ID.
    CollectStatus,
    /// The app-wide settings and the first run's checklist. There is no ID.
    Settings,
}

impl ChangeKind {
    /// The kind's name as the event carries it (`"subtitle_job"`).
    pub fn name(self) -> &'static str {
        match self {
            ChangeKind::Command => "command",
            ChangeKind::PastSearch => "past_search",
            ChangeKind::SubtitleJob => "subtitle_job",
            ChangeKind::SubtitleScreen => "subtitle_screen",
            ChangeKind::SubtitleStore => "subtitle_store",
            ChangeKind::SubtitleMapping => "subtitle_mapping",
            ChangeKind::Library => "library",
            ChangeKind::Artwork => "artwork",
            ChangeKind::SeasonInfo => "season_info",
            ChangeKind::Anissia => "anissia",
            ChangeKind::Rules => "rules",
            ChangeKind::History => "history",
            ChangeKind::CollectStatus => "collect_status",
            ChangeKind::Settings => "settings",
        }
    }
}

/// One announced change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Change {
    pub kind: ChangeKind,
    /// The ID of what changed, in the kind's own ID space. `null` when the
    /// changed row names no such target: anything of the kind may have
    /// changed, and a screen that shows the kind reads again whichever it
    /// shows.
    pub id: Option<String>,
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
        self.announce_target(kind, Some(id.into()));
    }

    /// [`Changes::announce`] for a change that may name no ID (`None`).
    pub fn announce_target(&self, kind: ChangeKind, id: Option<String>) {
        let _ = self.sender.send(Change { kind, id });
    }

    /// Hears the changes announced from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<Change> {
        self.sender.subscribe()
    }
}

pub(crate) mod tables;

#[cfg(test)]
mod tests;
