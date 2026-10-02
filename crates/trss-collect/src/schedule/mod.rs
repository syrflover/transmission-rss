//! The rules of the weekly schedule (`docs/specs/web-app.md`, 이번 주 편성),
//! apart from the stores and the API that feed them:
//!
//! - `trss_core::calendar`: the Seoul calendar and its Monday-to-Sunday week;
//! - `trss_anissia::slot`: where a subscribed anime falls in a week and which
//!   episode airs there;
//! - [`state`]: the status lines of a card.
//!
//! The API that gathers the inputs and answers is
//! [`trss_legacy::web::schedule_api`].

pub mod state;
