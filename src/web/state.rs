use crate::{
    anissia::{Anissia, AnissiaConfig},
    artwork::{AnilistConfig, Artwork},
    seasons::Seasons,
    store::{
        channels::ChannelStore, commands::CommandStore, history::HistoryStore,
        library::LibraryStore, settings::SettingsStore, setup::SetupStore, status::StatusStore, Db,
    },
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
    /// Anissia's schedule, and the snapshots of the anime that are subscribed.
    pub anissia: Anissia,
    /// Which steps of the first run's checklist the user skipped.
    pub setup: SetupStore,
}

impl AppState {
    pub fn new(db: Db) -> Self {
        let artwork = Artwork::new(db.clone(), None, AnilistConfig::default());
        AppState {
            anissia: Anissia::with_defaults(db.clone(), AnissiaConfig::default()),
            seasons: Seasons::over(db.clone(), &artwork),
            channels: ChannelStore::new(db.clone()),
            history: HistoryStore::new(db.clone()),
            status: StatusStore::new(db.clone()),
            commands: CommandStore::new(db.clone()),
            settings: SettingsStore::new(db.clone()),
            library: LibraryStore::new(db.clone()),
            setup: SetupStore::new(db.clone()),
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

    /// Replaces the artwork services (the app data folder and AniList's address).
    pub fn with_artwork(mut self, artwork: Artwork) -> Self {
        self.seasons = self.seasons.alongside(&artwork);
        self.artwork = artwork;
        self
    }
}
