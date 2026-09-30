use crate::store::{channels::ChannelStore, commands::CommandStore, history::HistoryStore, Db};

/// Shared handles every API handler can reach. Cheap to clone.
///
/// Feature stores are added here as fields as their tickets land; handlers
/// take `State<AppState>` and use only the stores they need.
#[derive(Clone)]
pub struct AppState {
    pub channels: ChannelStore,
    pub history: HistoryStore,
    /// Commands the web accepts and the worker carries out.
    pub commands: CommandStore,
}

impl AppState {
    pub fn new(db: Db) -> Self {
        AppState {
            channels: ChannelStore::new(db.clone()),
            history: HistoryStore::new(db.clone()),
            commands: CommandStore::new(db),
        }
    }
}
