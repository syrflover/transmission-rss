//! Local SQLite app state shared by `trss-web` and `trss-worker`.
//!
//! [`db`] (connection, pragmas, embedded migrations with the SQL of every
//! table) and [`settings`] live in `trss-core`; each feature keeps its own
//! queries and rules in its own submodule, starting with [`channels`].

pub mod anissia;
pub mod artwork;
pub mod channels;
pub mod commands;
pub mod history;
pub mod library;
pub mod revisions;
pub mod search_pace;
pub mod seasons;
pub mod setup;
pub mod status;

pub use trss_core::{db, settings, Db, DbError};

#[cfg(test)]
mod collect_folder_migration_tests;
