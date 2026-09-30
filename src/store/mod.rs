//! Local SQLite app state shared by `trss-web` and `trss-worker`.
//!
//! [`db`] holds the feature-neutral base (connection, pragmas, embedded
//! migrations); each feature keeps its own tables, SQL and rules in its own
//! submodule, starting with [`channels`].

pub mod channels;
pub mod commands;
pub mod db;
pub mod history;

pub use db::{Db, DbError};
