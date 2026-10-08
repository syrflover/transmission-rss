//! The collection feature: the channels and their rules, the feeds, the
//! decisions about what to receive, the commands that receive or move videos,
//! and the subscriptions to airing anime (`docs/specs/collection.md`).
//!
//! The loop that runs the cycles and the commands is in `trss-worker`; the web
//! calls the checks and the requests here. The crate holds:
//!
//! - [`store`]: the queries and rules of the collection's tables;
//! - [`rss`], [`rule`], [`config`], [`plan`] and [`feed`]: the feeds and what
//!   a rule picks from them;
//! - [`release_name`]: what a release name says (work, episode, revision, CRC32);
//! - [`revisions`], [`revision`]: replacing a video with its revision;
//! - [`cycle`]: what a collection cycle decides item by item, which the
//!   worker's cycle calls;
//! - [`receive`]: how one item's torrent is added, recorded and renamed, for
//!   the cycle and the commands alike;
//! - [`commands`]: each kind of web command and how it is carried out;
//! - [`offsets`], [`episode_offset`]: a rule's episode offset;
//! - [`season_link`]: connecting subscriptions to the seasons of the library;
//! - [`past_search`], [`subscriptions`], [`archive_suggestions`], [`anissia`],
//!   [`schedule`]: subscriptions to airing anime and what they offer.
//!
//! [`context::CollectContext`] is what the collection work shares; each command
//! takes only the part it uses (see [`context`]).

pub mod anissia;
pub mod archive_suggestions;
pub mod commands;
pub mod config;
pub mod context;
pub mod cycle;
pub mod episode_offset;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
pub mod feed;
pub mod offsets;
pub mod past_search;
pub mod plan;
pub mod receive;
pub mod release_name;
#[cfg(test)]
mod release_names;
pub mod revision;
pub mod revisions;
pub mod rss;
pub mod rule;
pub mod schedule;
pub mod season_link;
pub mod store;
pub mod subscriptions;
#[cfg(test)]
mod test_world;
