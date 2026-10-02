use std::fmt;
use std::str::FromStr;

use trss_core::Millis;

/// What became of an RSS item. The codes are stored in the database and are
/// stable; the Korean labels are what the screens show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistoryResult {
    /// `received` (추가함): Transmission took the torrent (a rule selected the
    /// item, or a retry of a failed one added it). Not that the download is done.
    Received,
    /// `no_match` (규칙 불일치): no rule matched the title.
    NoMatch,
    /// `excluded` (제외): the title contains one of the channel's excludes.
    Excluded,
    /// `duplicate` (중복): a rule selected the item but Transmission already
    /// had that torrent.
    Duplicate,
    /// `add_failed` (추가 실패): a rule selected the item but adding it to
    /// Transmission failed; the reason is kept next to it.
    AddFailed,
    /// `version_unknown` (버전 미상): a rule selected a higher revision of an
    /// episode the folder holds, and the worker could not tell the revisions
    /// apart (the name carries no CRC32, or the video in the folder matches no
    /// known release), so it did not receive it. `다시 받기` receives it and
    /// replaces the video (see [`crate::worker::revisions`]).
    VersionUnknown,
}

impl HistoryResult {
    pub const ALL: [HistoryResult; 6] = [
        HistoryResult::Received,
        HistoryResult::NoMatch,
        HistoryResult::Excluded,
        HistoryResult::Duplicate,
        HistoryResult::AddFailed,
        HistoryResult::VersionUnknown,
    ];

    /// The stable code stored in the database.
    pub fn code(self) -> &'static str {
        match self {
            HistoryResult::Received => "received",
            HistoryResult::NoMatch => "no_match",
            HistoryResult::Excluded => "excluded",
            HistoryResult::Duplicate => "duplicate",
            HistoryResult::AddFailed => "add_failed",
            HistoryResult::VersionUnknown => "version_unknown",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            HistoryResult::Received => "추가함",
            HistoryResult::NoMatch => "규칙 불일치",
            HistoryResult::Excluded => "제외",
            HistoryResult::Duplicate => "중복",
            HistoryResult::AddFailed => "추가 실패",
            HistoryResult::VersionUnknown => "버전 미상",
        }
    }

    pub fn parse(code: &str) -> Option<HistoryResult> {
        HistoryResult::ALL.into_iter().find(|r| r.code() == code)
    }

    /// True once Transmission is known to hold the item's torrent. Such a
    /// result is not overwritten by later evaluations (see [`Transition`]).
    pub fn is_settled(self) -> bool {
        matches!(self, HistoryResult::Received | HistoryResult::Duplicate)
    }
}

impl fmt::Display for HistoryResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for HistoryResult {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        HistoryResult::parse(s).ok_or(())
    }
}

/// How a new result relates to the one already stored for the same item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// Keep the stored result and its details; only the sighting is refreshed.
    Keep,
    /// Same failed result again: keep it, refresh the reason and rule.
    Refresh,
    /// Replace the stored result and add a row to the change trail.
    Change,
}

impl Transition {
    /// The rules for an item seen again:
    ///
    /// - The same result again is no change. For `add_failed` and
    ///   `version_unknown` the latest reason replaces the old one.
    /// - `received` is final: a later duplicate answer (the worker asks
    ///   Transmission again every cycle), a failed retry, or an edited rule that
    ///   no longer matches does not undo the fact that the torrent was added.
    /// - `duplicate` only ever becomes `received` (the torrent had gone and was
    ///   added again); other later evaluations leave it alone.
    /// - `no_match`, `excluded`, `add_failed` and `version_unknown` follow the
    ///   newest evaluation, and each such move is a change.
    pub fn between(stored: HistoryResult, new: HistoryResult) -> Transition {
        use HistoryResult::*;

        if stored == new {
            return if matches!(stored, AddFailed | VersionUnknown) {
                Transition::Refresh
            } else {
                Transition::Keep
            };
        }
        match (stored, new) {
            (Received, _) => Transition::Keep,
            (Duplicate, Received) => Transition::Change,
            (Duplicate, _) => Transition::Keep,
            _ => Transition::Change,
        }
    }
}

/// One sighting of an RSS item with what the worker decided about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub channel_id: String,
    /// The channel's masked URL. Never the original URL.
    pub channel_label: String,
    /// From [`identity_key`](super::identity_key).
    pub identity_key: String,
    pub title: String,
    /// The item's link, with the channel's secret query values masked.
    pub link: String,
    pub result: HistoryResult,
    /// ID of the applied rule, when a rule selected the item.
    pub rule_id: Option<String>,
    pub torrent_hash: Option<String>,
    /// Why adding failed. Callers must have removed secret values.
    pub reason: Option<String>,
}

/// What recording an [`Observation`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// First sighting: a new record was created.
    New,
    /// The result changed; the previous one is kept in the change trail.
    Changed { from: HistoryResult },
    /// Seen again with nothing to change in the result.
    Unchanged,
}

/// A stored history record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryItem {
    pub id: i64,
    pub channel_id: String,
    pub channel_label: String,
    pub identity_key: String,
    pub title: String,
    pub link: String,
    pub first_seen_at: Millis,
    /// The last cycle in which the item was in the channel's feed.
    pub last_seen_at: Millis,
    pub result: HistoryResult,
    /// When the current result was recorded.
    pub result_at: Millis,
    pub rule_id: Option<String>,
    pub reason: Option<String>,
    pub torrent_hash: Option<String>,
    /// Whether the channel's first read recorded the item: the feed already
    /// held it then. See [`HistoryStore::first_sightings`](super::HistoryStore::first_sightings).
    pub first_read: bool,
}

/// What a cycle needs to know of an item history already has: when it was
/// first seen, what became of it, and whether the channel's first read
/// recorded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownItem {
    pub first_seen_at: Millis,
    pub result: HistoryResult,
    pub first_read: bool,
}

impl From<&HistoryItem> for KnownItem {
    fn from(item: &HistoryItem) -> Self {
        KnownItem {
            first_seen_at: item.first_seen_at,
            result: item.result,
            first_read: item.first_read,
        }
    }
}

/// One entry of an item's change trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryChange {
    pub id: i64,
    pub item_id: i64,
    pub changed_at: Millis,
    pub from: HistoryResult,
    pub to: HistoryResult,
    pub rule_id: Option<String>,
    pub reason: Option<String>,
    pub torrent_hash: Option<String>,
}

/// A position in the newest-first list. It is the last item of a page; the
/// next page starts right after it. It stays valid while new items arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryCursor {
    pub first_seen_at: Millis,
    pub id: i64,
}

impl fmt::Display for HistoryCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.first_seen_at, self.id)
    }
}

impl FromStr for HistoryCursor {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        let (first_seen_at, id) = s.split_once('.').ok_or(())?;
        Ok(HistoryCursor {
            first_seen_at: first_seen_at.parse().map_err(|_| ())?,
            id: id.parse().map_err(|_| ())?,
        })
    }
}

pub const DEFAULT_PAGE_SIZE: usize = 50;
pub const MAX_PAGE_SIZE: usize = 500;

/// Newest-first listing parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryQuery {
    pub result: Option<HistoryResult>,
    /// When not empty, only items with one of these results (and with
    /// `result`, when that is set too).
    pub results: Vec<HistoryResult>,
    pub channel_id: Option<String>,
    /// Continue after this item (the previous page's `next`).
    pub after: Option<HistoryCursor>,
    /// Clamped to `1..=MAX_PAGE_SIZE`.
    pub limit: usize,
}

impl Default for HistoryQuery {
    fn default() -> Self {
        HistoryQuery {
            result: None,
            results: Vec::new(),
            channel_id: None,
            after: None,
            limit: DEFAULT_PAGE_SIZE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPage {
    pub items: Vec<HistoryItem>,
    /// Pass as `after` for the next page; `None` on the last page.
    pub next: Option<HistoryCursor>,
}

/// When the worker last ran a cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleState {
    pub started_at: Millis,
    /// `None` while that cycle is running or if the worker died in it.
    pub finished_at: Option<Millis>,
}
