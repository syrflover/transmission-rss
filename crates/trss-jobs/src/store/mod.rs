//! The job records: what the web asks for, what the runner writes as it goes,
//! and what the screens read.
//!
//! Files group by feature, each with the records and the SQL of its feature;
//! the types group by who calls: [`JobViews`] for what the web reads,
//! [`JobRequests`] for what makes a job and a person's requests on it, and
//! [`JobRun`] for what the worker writes. The three are handles over one
//! database, and no function is on two of them.
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

/// What the web and the to-do gatherer read of the jobs: the lists, a job's
/// detail and the candidates the jobs took. Cheap to clone.
#[derive(Clone)]
pub struct JobViews {
    db: Db,
}

impl JobViews {
    pub fn new(db: Db) -> JobViews {
        JobViews { db }
    }
}

/// What makes a job, and a person's requests on one: the web, the uploads, the
/// subscribed creator's receipts and the recheck. Cheap to clone.
#[derive(Clone)]
pub struct JobRequests {
    db: Db,
}

impl JobRequests {
    pub fn new(db: Db) -> JobRequests {
        JobRequests { db }
    }
}

/// What the worker writes as a job goes: claiming it, its state, items, steps
/// and log, the find job's end and the file receipts. Only the worker's
/// [`crate::Runner`] and the parts it drives (and the code that tests them)
/// hold one; the web holds [`JobViews`], [`JobRequests`] and
/// [`crate::place::PlaceStore`]. Cheap to clone.
#[derive(Clone)]
pub struct JobRun {
    db: Db,
}

impl JobRun {
    pub fn new(db: Db) -> JobRun {
        JobRun { db }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }
}
