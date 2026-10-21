//! trss: the web server and the collection worker.
//!
//! The two still run as two processes, the `trss-web` and `trss-worker`
//! binaries, which share this library: [`web`] serves the screen and the API,
//! and [`worker`] runs the collection cycles, the commands the web accepted
//! and the subtitle jobs.

pub mod web;
pub mod worker;
