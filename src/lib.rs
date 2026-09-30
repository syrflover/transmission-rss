#[cfg(feature = "anissia")]
pub mod anissia;
pub mod config;
pub mod import;
pub mod rss;
pub mod rule;
pub mod store;
pub mod web;

pub const USER_AGENT: &str = "trss/0.3";
