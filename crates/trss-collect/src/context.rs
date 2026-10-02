//! What the collection work shares: the cycle, the commands, the season link
//! and the revisions.

use url::Url;

use super::commands::rule_archive::work_folder::MovePolicy;
use crate::store::{channels::ChannelStore, history::HistoryStore, revisions::RevisionStore};
use trss_core::settings::SettingsStore;
use trss_library::{
    live::LiveWatch,
    store::{library::LibraryStore, seasons::SeasonStore},
};
use trss_transmission as transmission;
use trss_transmission::{Redactor, RenamePolicy};

/// Longest failure reason kept in history, in characters.
pub const MAX_REASON_CHARS: usize = 300;

/// What the collection work needs. Cheap to clone.
#[derive(Clone)]
pub struct CollectContext {
    pub channels: ChannelStore,
    /// Where the collect folder is read from.
    pub settings: SettingsStore,
    pub history: HistoryStore,
    /// The replacements of video revisions (see [`super::revisions`]).
    pub revisions: RevisionStore,
    /// The AniList entries linked to the library's seasons (the episodes of
    /// the seasons before a rule's: [`crate::episode_offset`]).
    pub seasons: SeasonStore,
    /// What the season link remembers between cycles (see
    /// [`crate::season_link`]).
    pub season_link: super::season_link::Memory,
    /// The works the library knows (the season link finds the videos a rule
    /// received there; the episode offset reads the seasons they belong to).
    pub library: LibraryStore,
    /// The inotify watches of the watch folders, which a work folder move
    /// tells (see [`trss_library::watch::follow_move`]).
    pub live: LiveWatch,
    pub transmission_url: Url,
    /// The client for Transmission's requests; they time out
    /// (see [`trss_transmission::http_client`]).
    pub transmission_http: reqwest012::Client,
    pub http: reqwest::Client,
    pub rename: RenamePolicy,
    /// How a work folder move waits for Transmission (the `rule_archive`
    /// command; see [`crate::commands::rule_archive`]).
    pub moves: MovePolicy,
    /// Knows secrets that do not come from channels, such as credentials in
    /// the Transmission URL.
    pub redactor: Redactor,
}

impl CollectContext {
    /// A client for Transmission, with timeouts.
    pub fn transmission(&self) -> transmission_rpc::TransClient {
        transmission::client(self.transmission_url.clone(), &self.transmission_http)
    }
}
