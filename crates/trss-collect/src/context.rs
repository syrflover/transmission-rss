//! What the collection work shares: the cycle, the commands, the season link
//! and the revisions.
//!
//! The worker builds one [`CollectContext`] with everything, and the cycle
//! runs on it. Each command and each part the cycle hands work to takes only
//! the stores it uses, in its own context, made from this one:
//!
//! - `receive_once` and `receive_past`: [`ReceiveContext`];
//! - `rule_archive`: [`ArchiveContext`];
//! - `episode_undo`: [`UndoContext`];
//! - the video revisions: [`RevisionsContext`];
//! - the episode offsets: [`OffsetsContext`];
//! - the season link: [`LinkContext`].

use url::Url;

use crate::commands::rule_archive::work_folder::MovePolicy;
use crate::store::{channels::ChannelStore, history::HistoryStore, revisions::RevisionStore};
use trss_core::{commands::CommandStore, folder_locks::FolderLocks, settings::SettingsStore};
use trss_library::{
    live::LiveWatch,
    store::{library::LibraryStore, seasons::SeasonStore},
};
use trss_transmission as transmission;
use trss_transmission::{Redactor, RenamePolicy};

pub use crate::commands::episode_undo::UndoContext;
pub use crate::commands::receive_once::ReceiveContext;
pub use crate::commands::rule_archive::ArchiveContext;
pub use crate::offsets::OffsetsContext;
pub use crate::revisions::RevisionsContext;
pub use crate::season_link::LinkContext;

/// Longest failure reason kept in history, in characters.
pub const MAX_REASON_CHARS: usize = 300;

/// How the collection work reaches Transmission. Cheap to clone.
#[derive(Clone)]
pub struct TransmissionLink {
    pub url: Url,
    /// The client for Transmission's requests; they time out
    /// (see [`trss_transmission::http_client`]).
    pub http: reqwest012::Client,
}

impl TransmissionLink {
    /// A client for Transmission, with timeouts.
    pub fn client(&self) -> transmission_rpc::TransClient {
        transmission::client(self.url.clone(), &self.http)
    }
}

/// What the collection work needs. Cheap to clone.
#[derive(Clone)]
pub struct CollectContext {
    pub channels: ChannelStore,
    /// Where the collect folder is read from.
    pub settings: SettingsStore,
    pub history: HistoryStore,
    /// The replacements of video revisions (see [`super::revisions`]).
    pub revisions: RevisionStore,
    /// The commands, for what a command records while it runs (the name a
    /// retried torrent had before its rename).
    pub commands: CommandStore,
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
    pub transmission: TransmissionLink,
    pub http: reqwest::Client,
    pub rename: RenamePolicy,
    /// How a work folder move waits for Transmission (the `rule_archive`
    /// command; see [`crate::commands::rule_archive`]).
    pub moves: MovePolicy,
    /// Knows secrets that do not come from channels, such as credentials in
    /// the Transmission URL.
    pub redactor: Redactor,
    /// The worker's turns at its folders ([`trss_core::folder_locks`]), shared
    /// with the reading of the watch folders.
    pub folders: FolderLocks,
}

impl CollectContext {
    /// What `receive_once` and `receive_past` use.
    pub fn receive(&self) -> ReceiveContext {
        ReceiveContext {
            channels: self.channels.clone(),
            settings: self.settings.clone(),
            history: self.history.clone(),
            revisions: self.revisions.clone(),
            commands: self.commands.clone(),
            seasons: self.seasons.clone(),
            library: self.library.clone(),
            transmission: self.transmission.clone(),
            http: self.http.clone(),
            rename: self.rename,
            redactor: self.redactor.clone(),
        }
    }

    /// What `rule_archive` uses.
    pub fn archive(&self) -> ArchiveContext {
        ArchiveContext {
            channels: self.channels.clone(),
            settings: self.settings.clone(),
            library: self.library.clone(),
            live: self.live.clone(),
            transmission: self.transmission.clone(),
            moves: self.moves,
            redactor: self.redactor.clone(),
        }
    }

    /// What `episode_undo` uses.
    pub fn undo(&self) -> UndoContext {
        UndoContext {
            channels: self.channels.clone(),
            history: self.history.clone(),
            settings: self.settings.clone(),
            transmission: self.transmission.clone(),
        }
    }

    /// What the video revisions use.
    pub fn revision_work(&self) -> RevisionsContext {
        RevisionsContext {
            channels: self.channels.clone(),
            history: self.history.clone(),
            revisions: self.revisions.clone(),
            transmission: self.transmission.clone(),
        }
    }

    /// What the episode offsets use.
    pub fn offsets(&self) -> OffsetsContext {
        OffsetsContext {
            channels: self.channels.clone(),
            history: self.history.clone(),
            library: self.library.clone(),
            seasons: self.seasons.clone(),
        }
    }

    /// What the season link uses.
    pub fn link(&self) -> LinkContext {
        LinkContext {
            channels: self.channels.clone(),
            history: self.history.clone(),
            library: self.library.clone(),
            season_link: self.season_link.clone(),
            transmission: self.transmission.clone(),
            redactor: self.redactor.clone(),
        }
    }
}

impl ReceiveContext {
    /// What the video revisions use, of these stores.
    pub fn revision_work(&self) -> RevisionsContext {
        RevisionsContext {
            channels: self.channels.clone(),
            history: self.history.clone(),
            revisions: self.revisions.clone(),
            transmission: self.transmission.clone(),
        }
    }

    /// What the episode offsets use, of these stores.
    pub fn offsets(&self) -> OffsetsContext {
        OffsetsContext {
            channels: self.channels.clone(),
            history: self.history.clone(),
            library: self.library.clone(),
            seasons: self.seasons.clone(),
        }
    }
}
