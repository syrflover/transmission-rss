//! Importing the legacy transmission-rss channels YAML into the app database.
//!
//! [`legacy`] reads and validates the file, [`plan`] decides which of its
//! channels already exist and turns the user's per-channel choices into the
//! store's [`ImportAction`](crate::store::channels::import::ImportAction)s.
//! Nothing here downloads, applies or cleans up files: an import only writes
//! channels and rules.

pub mod legacy;
pub mod plan;
