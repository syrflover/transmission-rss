//! The common base of the trss crates: the app database (connection,
//! embedded migrations and the SQL of every feature's tables), the app-wide
//! settings, and the small tools several features share.
//!
//! Nothing here knows a feature, the web or the worker. The migration list is
//! one place for the whole schema because the database is one and its tables
//! reach across features (the triggers of artwork on the library's `works`,
//! those of the first run on `watch_folders`); a feature crate keeps only the
//! code that reads and writes its own tables.

mod clock;
pub mod db;
pub mod folders;
pub mod lock;
pub mod settings;

pub use clock::{system_clock, Clock, Millis};
pub use db::{Db, DbError};
pub use lock::{lock_path_for, CycleLock};

/// The `User-Agent` of the requests trss makes to feeds and outside services.
pub const USER_AGENT: &str = "trss/0.3";
