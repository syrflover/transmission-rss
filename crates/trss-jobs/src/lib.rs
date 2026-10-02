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
//! - [`follow`]: the subscribed creator's subtitles, made into jobs without a
//!   pick, and the `자막 구독` suggestions; [`mapping`]: the episode mapping
//!   the app decides for that creator's source.
//!
//! The sites themselves are `trss-subtitles`'s.

pub mod area;
pub mod follow;
pub mod mapping;
pub mod model;
pub mod runner;
pub mod store;

pub use area::ReceiveArea;
pub use follow::{Follow, FollowError};
pub use model::{FileState, ItemState, JobState, StepKind, StepState, Wait};
pub use runner::Runner;
pub use store::{Created, FileProblem, JobError, JobStore, NewItem, NewJob, AUTO};
pub use trss_subtitles::{verify::Format, FailureKind};
