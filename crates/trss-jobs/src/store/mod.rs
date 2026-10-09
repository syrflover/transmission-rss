//! The job records: what the web asks for, what the runner writes as it goes,
//! and what the screens read.
//!
//! Files group by feature, each with the records and the SQL of its feature:
//!
//! - [`create`]: making a job (a pick, an app-made job, an upload).
//! - [`find`]: a find job, from its creation to the end of its 받기.
//! - [`run`]: claiming a job and the state it goes through, with the log.
//! - [`receipts`]: the records of the files a job receives.
//! - [`views`]: what the lists and the detail of the jobs show.
//! - [`rows`]: the row types and the mappers more than one of them shares.

mod create;
mod find;
mod receipts;
mod rows;
mod run;
mod views;

pub use create::{Created, MappingStamp, NewItem, NewJob, NewUpload, UploadedFile};
pub use find::{AskedFinish, Found, NewFind, NOTHING_FOUND};
pub use receipts::{snapshot_json, FileProblem};
pub use rows::{
    DonePage, DroppedRow, EventRow, FileRow, ItemRow, JobDetail, JobRow, Pick, Progress, StepRow,
    UploadSummary,
};
pub use run::DECIDED;

use trss_core::{Db, DbError};

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("no record of {0}")]
    Missing(&'static str),
    /// What the season info or a work's library says could not be read.
    #[error("{0}")]
    Other(String),
}

/// The origin of a job the app made without a pick: the subscribed creator's
/// subtitle, received on its own ([`crate::follow`]).
pub const AUTO: &str = "auto";

/// The origin of a job made from files a person uploaded
/// ([`crate::upload`]).
pub const UPLOAD: &str = "upload";

/// The origin of a job a person makes to find a subtitle in the server
/// browser, starting at a creator's post (`crate::runner`, find jobs).
pub const FIND: &str = "find";

/// The origin of the job the app makes when a source's episode mapping
/// changes and moves a subtitle it applied to another episode
/// ([`crate::place::relocate`]).
pub const RELOCATE: &str = "relocate";

/// Async access to the job records. Cheap to clone.
#[derive(Clone)]
pub struct JobStore {
    db: Db,
}

impl JobStore {
    pub fn new(db: Db) -> JobStore {
        JobStore { db }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }
}
