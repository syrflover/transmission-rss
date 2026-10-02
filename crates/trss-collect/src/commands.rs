//! The commands the web accepts (see [`trss_core::commands`]), one module
//! per kind. Each holds the command's payload, the checks the web makes before
//! it accepts one, and what the worker does to carry it out. The worker's loop
//! that claims waiting commands and calls the `run` of each is in the
//! `trss-worker` crate.

pub mod anissia_captions;
pub mod episode_undo;
pub mod link;
pub mod receive_once;
pub mod receive_past;
pub mod rule_archive;
