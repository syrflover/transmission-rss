//! The integration tests of trss-jobs, one module per file, linked as one
//! binary (docs/adr/0014-one-integration-test-binary-per-crate.md).
//! `tests/extract_process.rs` stays a binary of its own: its tests write
//! scripts and run them, which fails ("text file busy") when another thread
//! of the process starts a process meanwhile.

mod airtime_sample;
mod blogger;
mod choose;
mod cleanup;
mod drive_fonts;
mod find;
mod follow;
mod naver;
mod place;
mod placement;
mod recheck;
mod relocate;
mod replace;
mod replace_many;
mod runner;
mod screen;
mod tistory;
mod todo;
mod unpack;
mod upload;
mod winpng;
mod world;

use trss_core::Db;
use trss_jobs::{JobRequests, JobRun, JobViews, PlaceStore};

/// The handles over one database that the tests reach the job records with:
/// the code under test holds one or two of them, and a test that sets up or
/// checks the records reaches any.
#[derive(Clone)]
pub struct Handles {
    pub views: JobViews,
    pub requests: JobRequests,
    pub run: JobRun,
    pub place: PlaceStore,
}

impl Handles {
    pub fn new(db: Db) -> Handles {
        Handles {
            views: JobViews::new(db.clone()),
            requests: JobRequests::new(db.clone()),
            run: JobRun::new(db.clone()),
            place: PlaceStore::new(db),
        }
    }
}
