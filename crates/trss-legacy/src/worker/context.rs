//! What a cycle, a command and the watches of the worker share.

use url::Url;

use super::commands::rule_archive::work_folder::MovePolicy;
use crate::store::{
    channels::ChannelStore, history::HistoryStore, library::LibraryStore, revisions::RevisionStore,
    seasons::SeasonStore,
};
use trss_core::settings::SettingsStore;
use trss_transmission as transmission;
use trss_transmission::{Redactor, RenamePolicy, SessionConfig};

/// Longest failure reason kept in history, in characters.
pub const MAX_REASON_CHARS: usize = 300;

/// What a cycle needs. Cheap to clone.
#[derive(Clone)]
pub struct CycleContext {
    pub channels: ChannelStore,
    /// Where the collect folder is read from.
    pub settings: SettingsStore,
    pub history: HistoryStore,
    /// The replacements of video revisions (see [`super::revisions`]).
    pub revisions: RevisionStore,
    /// The watch folders the worker rescans (see [`crate::worker::watch`]).
    pub library: LibraryStore,
    /// The AniList entries linked to the library's seasons (the episodes of
    /// the seasons before a rule's: [`crate::episode_offset`]).
    pub seasons: SeasonStore,
    /// What the worker remembers of each watch folder's directories between
    /// scans (see [`crate::worker::watch`]).
    pub scan_cache: super::watch::ScanCaches,
    /// What the season link remembers between cycles (see
    /// [`crate::worker::season_link`]).
    pub season_link: super::season_link::Memory,
    /// The inotify watches of the watch folders, which say what the cycle has
    /// to read of them (see [`crate::worker::live`]).
    pub live: super::live::LiveWatch,
    pub transmission_url: Url,
    /// The client for Transmission's requests; they time out
    /// (see [`trss_transmission::http_client`]).
    pub transmission_http: reqwest012::Client,
    pub session: SessionConfig,
    pub http: reqwest::Client,
    pub rename: RenamePolicy,
    /// How a work folder move waits for Transmission (the `rule_archive`
    /// command; see [`crate::worker::commands::rule_archive`]).
    pub moves: MovePolicy,
    /// Knows secrets that do not come from channels, such as credentials in
    /// the Transmission URL.
    pub redactor: Redactor,
}

impl CycleContext {
    /// A client for Transmission, with timeouts.
    pub fn transmission(&self) -> transmission_rpc::TransClient {
        transmission::client(self.transmission_url.clone(), &self.transmission_http)
    }
}
