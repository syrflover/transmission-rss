//! Subtitle jobs (`docs/specs/jobs.md`): what a person asked to receive, and
//! the worker carrying it out.
//!
//! - [`store`]: the records (`subtitle_jobs` and the tables beside it, see
//!   `migrations/jobs/schema.sql` in `trss-core`). The web makes jobs
//!   ([`JobStore::create`]) and reads them; the worker writes how they go.
//! - [`runner`]: carrying a job out, a file at a time, with the checkpoints
//!   that let a restarted worker reuse what it received or hold what it cannot
//!   vouch for.
//! - [`area`]: the receive area in the app data folder.
//! - [`place`]: what a job received, unpacked ([`place::unpack`], in the
//!   `trss-extract` program this crate builds), analysed, stored and applied.
//! - [`screen`]: the remote screen of a job that waits for a person's check
//!   on the site: the browser run bound to it, shared by the worker and the
//!   web.
//! - [`follow`]: the subscribed creator's subtitles, made into jobs without a
//!   pick, and the `자막 구독` suggestions; [`mapping`]: the episode mapping
//!   the app decides for that creator's source.
//! - [`todo`]: the to-dos that need a person (`처리 필요`): what the jobs, the
//!   subscriptions, the library and the failed receipts wait on, grouped and
//!   ordered, and each work's badges.
//! - [`recheck`]: the daily reading of the received episodes' files for 14
//!   days, and the revision jobs made when a file differs.
//!
//! - [`upload`]: the subtitles and fonts a person uploads, made into a job
//!   whose files are received, and the same judging for the files a find
//!   job's browser run downloads.
//! - A find job ([`FIND`], [`JobStore::create_find`]): a person browses a
//!   creator's posts on the job's remote screen and every download becomes a
//!   file of its one package, until they finish its 받기
//!   ([`JobStore::ask_finish`]).
//! - Both wait for the person's 배치 확인 before anything they received is
//!   kept ([`place`], [`place::records::confirm_placement`]).
//!
//! The sites themselves are `trss-subtitles`'s.

pub mod area;
pub mod follow;
pub mod mapping;
pub mod model;
pub mod place;
pub mod recheck;
pub mod runner;
pub mod screen;
pub mod store;
pub mod todo;
pub mod upload;

pub use area::ReceiveArea;
pub use follow::{Follow, FollowError};
pub use model::{FileState, ItemState, JobState, StepKind, StepState, Wait};
pub use recheck::Recheck;
pub use runner::Runner;
pub use screen::{Screen, ScreenState, ScreenStore};
pub use store::{
    AskedFinish, Created, FileProblem, JobError, JobStore, MappingStamp, NewFind, NewItem, NewJob,
    AUTO, FIND, NOTHING_FOUND, RELOCATE, UPLOAD,
};
pub use trss_archive::run::Unpacker;
pub use trss_subtitles::{verify::Format, FailureKind};
pub use upload::{Finished, Uploads};

/// Whether `command_id` is one the app makes for itself: the subscribed
/// creator's receipts (`auto:`, [`follow`]) and the recheck's revisions
/// (`recheck:`, [`recheck`]). A person's request may not take one, or the app's
/// own job for it would find the person's in its place.
pub fn is_app_command(command_id: &str) -> bool {
    command_id.starts_with(follow::AUTO_PREFIX) || command_id.starts_with(recheck::COMMAND_PREFIX)
}
