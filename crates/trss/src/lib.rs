//! trss: the web server and the collection worker.
//!
//! The `trss` binary runs both in one process: [`web`] serves the screen and
//! the API, and [`worker`] runs the collection cycles, the commands the web
//! accepted and the subtitle jobs. The web starts what it stored by ringing
//! the worker part's loops ([`wake`]).

pub mod archive_batch;
pub mod deadlines;
pub mod wake;
pub mod web;
pub mod worker;
