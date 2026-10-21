//! The integration tests of the trss crate, one module per file, linked as one
//! binary (docs/adr/0014-one-integration-test-binary-per-crate.md).

mod common;

mod anissia_captions;
mod archive_move;
mod collect_folder;
mod episode_offset;
mod library_watch;
mod live_watch;
mod past_search;
mod receive_once;
// Needs Docker and the browser image: built only with its feature.
#[cfg(feature = "remote-screen-docker")]
mod remote_screen_docker;
mod season_link;
mod status_snapshots_from_cycle;
mod video_revisions;
mod worker_cycle;
mod worker_process;
