//! The worker's side of the features: what a cycle and the commands share
//! ([`CycleContext`]) and the parts of the collection work that the web and
//! other features also use. The loop that runs the cycles and the commands is
//! in the `trss-worker` crate.
//!
//! - [`plan`] judges a channel's feed for a cycle, a preview and a past search;
//! - [`feed`] reads and parses a feed;
//! - [`revisions`] replaces a video with its revision;
//! - [`commands`] holds each kind of web command and how it is carried out;
//! - [`watch`] and [`live`] read the watch folders, the second from inotify
//!   alerts; [`heartbeat`] is what the worker leaves for the web while it
//!   holds the lock;
//! - [`season_link`] connects subscriptions to the season their videos appeared in;
//! - [`offsets`] settles a rule's episode offset.

pub mod commands;
pub mod context;
pub mod feed;
pub mod heartbeat;
pub mod live;
pub mod offsets;
pub mod plan;
pub mod revisions;
pub mod season_link;
pub mod watch;

pub use commands::rule_archive::work_folder::MovePolicy;
pub use context::CycleContext;
