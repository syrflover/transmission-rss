use std::path::PathBuf;

use trss_anilist::AnilistConfig;
use trss_anissia::{Anissia, AnissiaConfig};
use trss_collect::{
    past_search::service::PastSearch,
    store::{
        anissia::AnissiaStore, channels::ChannelStore, history::HistoryStore,
        revisions::RevisionStore, search_pace::SearchPace, status::StatusStore,
    },
};
use trss_core::{commands::CommandStore, heartbeat::HeartbeatStore, settings::SettingsStore, Db};
use trss_jobs::{Follow, JobStore, ReceiveArea, Uploads};
use trss_library::{
    artwork::Artwork,
    seasons::Seasons,
    store::{library::LibraryStore, setup::SetupStore},
};

/// Shared handles every API handler can reach. Cheap to clone.
///
/// Feature stores are added here as fields as their tickets land; handlers
/// take `State<AppState>` and use only the stores they need.
#[derive(Clone)]
pub struct AppState {
    pub channels: ChannelStore,
    pub history: HistoryStore,
    pub status: StatusStore,
    /// The worker's heartbeat, which says whether it is busy or gone.
    pub heartbeat: HeartbeatStore,
    /// Commands the web accepts and the worker carries out.
    pub commands: CommandStore,
    /// App-wide settings: the collect and archive folders.
    pub settings: SettingsStore,
    /// The watch folders and what was found in them.
    pub library: LibraryStore,
    /// Work covers: their state, AniList, and the image files of the app
    /// data folder.
    pub artwork: Artwork,
    /// The AniList entries linked to each season, and the user's choices
    /// about them.
    pub seasons: Seasons,
    /// Anissia's schedule.
    pub anissia: Anissia,
    /// The snapshots of the anime that are subscribed.
    pub anissia_store: AnissiaStore,
    /// Which steps of the first run's checklist the user skipped.
    pub setup: SetupStore,
    /// The replacements of video revisions the worker carries out.
    pub revisions: RevisionStore,
    /// The past episode searches this process runs, and the request pace of
    /// the trackers they read.
    pub past_search: PastSearch,
    /// Where the worker hears that a command was accepted
    /// ([`trss_core::wake`]); `None`: it finds the command at its own next look.
    pub worker_wake: Option<PathBuf>,
    /// The subtitle jobs the worker carries out.
    pub jobs: JobStore,
    /// The subscribed creators' receipts and the `자막 구독` suggestions.
    pub follow: Follow,
    /// The receive area the worker puts the jobs' files in, for the paths the
    /// job detail shows. The web writes there only what a person uploads
    /// ([`AppState::uploads`]).
    pub receive_root: PathBuf,
    /// The subtitles and fonts a person uploads, made into jobs.
    pub uploads: Uploads,
}

impl AppState {
    pub fn new(db: Db) -> Self {
        let artwork = Artwork::new(db.clone(), None, AnilistConfig::default());
        AppState {
            anissia: Anissia::with_defaults(db.clone(), AnissiaConfig::default()),
            anissia_store: AnissiaStore::new(db.clone()),
            seasons: Seasons::over(db.clone(), &artwork),
            channels: ChannelStore::new(db.clone()),
            history: HistoryStore::new(db.clone()),
            status: StatusStore::new(db.clone()),
            heartbeat: HeartbeatStore::new(db.clone()),
            commands: CommandStore::new(db.clone()),
            settings: SettingsStore::new(db.clone()),
            library: LibraryStore::new(db.clone()),
            setup: SetupStore::new(db.clone()),
            revisions: RevisionStore::new(db.clone()),
            past_search: PastSearch::new(SearchPace::new(db.clone())),
            worker_wake: None,
            jobs: JobStore::new(db.clone()),
            follow: Follow::new(db.clone()),
            receive_root: PathBuf::from("receive"),
            uploads: Uploads::new(JobStore::new(db.clone()), ReceiveArea::new("receive")),
            // No app data folder: covers can be read and changed but no image
            // stored or served until `with_artwork` gives one.
            artwork,
        }
    }

    /// Replaces the Anissia client (its address, and a clock or pace in tests).
    pub fn with_anissia(mut self, anissia: Anissia) -> Self {
        self.anissia = anissia;
        self
    }

    /// Wakes the worker at `path` whenever a command is accepted (see
    /// [`trss_core::wake`]).
    pub fn with_worker_wake(mut self, path: PathBuf) -> Self {
        self.worker_wake = Some(path);
        self
    }

    /// Shows the jobs' files in `area` (the app data folder's).
    pub fn with_receive_area(mut self, area: &ReceiveArea) -> Self {
        self.receive_root = area.root().to_owned();
        self.uploads = Uploads::new(self.jobs.clone(), area.clone());
        self
    }

    /// Replaces the uploads (their limits and times in tests); they write to
    /// the same receive area as the one given to `with_receive_area`.
    pub fn with_uploads(mut self, uploads: Uploads) -> Self {
        self.uploads = uploads;
        self
    }

    /// Replaces the past episode search service (the time between requests in tests).
    pub fn with_past_search(mut self, past_search: PastSearch) -> Self {
        self.past_search = past_search;
        self
    }

    /// Replaces the artwork services (the app data folder and AniList's address).
    pub fn with_artwork(mut self, artwork: Artwork) -> Self {
        self.seasons = self.seasons.alongside(&artwork);
        self.artwork = artwork;
        self
    }
}
