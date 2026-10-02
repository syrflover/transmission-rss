//! The library feature: the watch folders, the works and seasons found in them,
//! each work's cover and each season's AniList info (`docs/specs/library.md`).
//!
//! - [`discovery`] reads a watch folder's directories into works and seasons;
//! - [`automatic_watch`] plans the watch folders the collect and archive
//!   folders of the settings make;
//! - [`watch`] and [`live`] read the watch folders, the second from inotify
//!   alerts; [`watch_rescan`] is the command that reads one now;
//! - [`artwork`] and [`seasons`] keep each work's cover and each season's
//!   AniList entries, with the worker's queues that fetch them;
//! - [`store`] holds the queries and rules of the tables of these.
//!
//! The AniList and Anissia answers are read in `trss-anilist`; this crate
//! decides what the library does with them.

pub mod artwork;
pub mod automatic_watch;
pub mod discovery;
pub mod live;
pub mod seasons;
pub mod store;
pub mod watch;
pub mod watch_rescan;
