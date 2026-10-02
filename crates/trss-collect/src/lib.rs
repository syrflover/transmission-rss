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
//! - [`revisions`], [`revision`]: replacing a video with its revision;
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
pub mod episode_offset;
pub mod feed;
pub mod offsets;
pub mod past_search;
pub mod plan;
pub mod revision;
pub mod revisions;
pub mod rss;
pub mod rule;
pub mod schedule;
pub mod season_link;
pub mod store;
pub mod subscriptions;
