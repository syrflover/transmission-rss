//! The rules of the weekly schedule (`docs/specs/web-app.md`, 이번 주 편성),
//! apart from the stores and the API that feed them:
//!
//! - [`calendar`]: the Seoul calendar and its Monday-to-Sunday week;
//! - [`slot`]: where a subscribed anime falls in a week and which episode airs
//!   there;
//! - [`state`]: the status lines of a card.
//!
//! The API that gathers the inputs and answers is
//! [`crate::web::schedule_api`].

pub mod calendar;
pub mod slot;
pub mod state;

/// How old an Anissia snapshot may be before the anime is taken to have left
/// the schedule. The worker asks Anissia again for a snapshot a day old, so
/// one this old was not found in any week of the schedule for two weeks
/// (Anissia lists an anime while it airs); keeping its card would show a
/// finished anime for good, since a rule is archived only when the user says so.
pub const SNAPSHOT_STALE_AFTER_MS: i64 = 14 * 24 * 60 * 60 * 1000;
