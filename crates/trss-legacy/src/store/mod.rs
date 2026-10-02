//! Local SQLite app state shared by `trss-web` and `trss-worker`.
//!
//! The database (connection, pragmas, embedded migrations with the SQL of every
//! table) and the settings live in `trss-core`; each feature keeps its own
//! queries and rules in its own submodule, starting with [`channels`].

pub mod anissia;
pub mod channels;
pub mod history;
pub mod revisions;
pub mod search_pace;
pub mod status;

#[cfg(test)]
mod collect_folder_migration_tests;
