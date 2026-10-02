//! The database handle behind a [`ChannelStore`].

use super::ChannelStore;
use trss_core::Db;

impl ChannelStore {
    /// The database this store works on, for a store of another feature that
    /// has to share it (the worker builds its status snapshots this way).
    pub fn db(&self) -> &Db {
        &self.db
    }
}
