//! The integration tests of trss-worker, one module per file, linked as one
//! binary (docs/adr/0014-one-integration-test-binary-per-crate.md).

mod common;

mod anissia_captions;
mod app_data_files;
mod archive_move;
mod channel_edit_then_cycle;
mod collect_folder;
mod episode_offset;
mod library_watch;
mod live_watch;
mod past_search;
mod receive_once;
mod rules_preview_then_cycle;
mod season_link;
mod status_snapshots_from_cycle;
mod video_revisions;
mod worker_cycle;
mod worker_process;
