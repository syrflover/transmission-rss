//! Importing the legacy transmission-rss channels YAML into the app database.
//!
//! [`legacy`] reads and validates the file, [`fit`] places its channel folders
//! under the app's collect folder, [`plan`] decides which of its channels
//! already exist and turns the user's per-channel choices into the store's
//! [`ImportAction`](trss_collect::store::channels::import::ImportAction)s.
//! [`comments`] reads the comments above the rules and [`suggest`] turns them
//! into subscription suggestions, which become subscriptions only when the user
//! checks them; [`picks`] decides what the checked ones come to.
//! Nothing here downloads, applies or cleans up files: an import only writes
//! channels and rules (and sets the collect folder when none is set yet), and
//! the subscriptions of the rules the user chose.

pub mod comments;
pub mod fit;
pub mod legacy;
pub mod picks;
pub mod plan;
pub mod suggest;
