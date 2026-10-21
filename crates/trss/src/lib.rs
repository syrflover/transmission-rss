//! trss: the web server and the collection worker.
//!
//! The `trss` binary runs both in one process: [`web`] serves the screen and
//! the API, and [`worker`] runs the collection cycles, the commands the web
//! accepted and the subtitle jobs.

pub mod web;
pub mod worker;
