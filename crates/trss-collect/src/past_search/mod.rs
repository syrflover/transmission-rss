//! The past episode search (`docs/specs/collection.md`, 지난 회차 검색; ticket
//! 0026): reading a tracker's search RSS for the episodes a feed no longer
//! holds, and judging what comes back before the person chooses.
//!
//! - [`query`]: the search words (a channel's format with the rule's match
//!   phrase, the extra searches' episode alternatives) and the search address.
//! - [`release`]: what a result's title says (episode, revision, batch) and the
//!   notation its number is written in.
//! - [`judge`]: the preview of results, against the range and what the work has.
//! - [`client`]: one request for one page, paced for the host across processes.
//! - [`range`]: the release range the search starts with.
//! - [`run`]: the search itself, first page and the extra ones.
//! - [`world`]: what the work folder and history hold, for the judgment.
//! - [`service`]: the searches running and finished in this process.
//!
//! A search leaves no channel and no history behind. Only the items the person
//! chooses to receive are recorded, by the worker, when it adds them.
//!
//! # The finite values
//!
//! - The tracker returns [`PAGE_LIMIT`] results at most and ignores the page
//!   number, so a first page that is full may be cut.
//! - Extra searches write [`BATCH_SIZE`] episodes in one query
//!   (`One Piece - (1000|1001|…)`), at most [`MAX_EXTRA_SEARCHES`] of them, so
//!   a search asks for [`BATCH_SIZE`] × [`MAX_EXTRA_SEARCHES`] = 200 more
//!   episodes at the most; a wider range is searched as far as that goes and
//!   says what it left out.
//! - Requests to one host start [`client::REQUEST_SPACING`] (3 s) apart, so a
//!   search with all its extras takes about a minute at the longest.
//! - A request waits [`client::MAX_WAIT`] (60 s) for its turn at the most. When
//!   the host asked for no request for longer (a `429`), or the pace row is
//!   further ahead than that, the search fails at once and says when to try
//!   again; a block that comes while a request waits stops that request.

pub mod client;
pub mod judge;
pub mod query;
pub mod range;
pub mod release;
pub mod run;
pub mod service;
pub mod world;

/// How many results the tracker's search RSS returns at the most.
pub const PAGE_LIMIT: usize = 75;
/// How many episodes one extra search asks for.
pub const BATCH_SIZE: usize = 10;
/// How many extra searches one search sends at the most.
pub const MAX_EXTRA_SEARCHES: usize = 20;
/// The widest range a search takes.
pub const MAX_SPAN: u32 = 2000;
/// The most results one search keeps (all pages together).
pub const MAX_RESULTS: usize = 2000;
