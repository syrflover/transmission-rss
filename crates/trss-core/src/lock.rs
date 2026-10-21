//! The lock that keeps a second `trss` off the same app data.
//!
//! One `trss` process runs everything on an app database: the collection
//! cycles, the commands, the subtitle jobs, the queues, the server browser.
//! It takes [`InstanceLock`] at its start, before it opens the database, and
//! keeps it for its whole life; a second process on the same database finds
//! the lock taken and stops before it runs anything. Within the process, what
//! must not run twice at once keeps itself apart (see `trss::worker`).

use std::{
    ffi::OsString,
    fs::{File, OpenOptions, TryLockError},
    io,
    path::{Path, PathBuf},
};

/// What follows the database file's name in the lock file's name. The start-up
/// check of the app data folder ([`crate::access`]) checks this file too.
pub const SUFFIX: &str = ".lock";

/// The lock file for a database file: the database path plus [`SUFFIX`], so it
/// sits next to the database on the same local volume.
pub fn instance_lock_path(db_path: &Path) -> PathBuf {
    let mut name: OsString = db_path.as_os_str().to_owned();
    name.push(SUFFIX);
    PathBuf::from(name)
}

/// An exclusive advisory lock (`flock`) on the lock file of a database, held
/// for as long as a `trss` process runs on it.
///
/// The operating system, not the process, owns the lock: it is released when
/// the guard is dropped and also when the process dies for any reason, so a
/// crashed process never leaves a stale lock. No time limit is involved.
///
/// Every [`InstanceLock::try_take`] opens its own file description, so two
/// guards on the same path exclude each other even inside one process.
///
/// Dropping the guard unlocks before it closes. Closing alone would not be
/// enough: the lock belongs to the open file description, and a child process
/// that another thread is starting holds a copy of every descriptor from its
/// fork until its `exec` closes them, which would keep the lock taken for that
/// moment after the guard is gone.
#[derive(Debug)]
pub struct InstanceLock {
    file: File,
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        // Closing the file releases it anyway once no copy is left.
        let _ = self.file.unlock();
    }
}

impl InstanceLock {
    /// Takes the lock of the database `db_path` ([`instance_lock_path`]) if
    /// nobody holds it. `Ok(None)` means another process runs on it.
    pub fn try_take(db_path: &Path) -> io::Result<Option<InstanceLock>> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(instance_lock_path(db_path))?;

        match file.try_lock() {
            Ok(()) => Ok(Some(InstanceLock { file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(err)) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_file_sits_next_to_the_database() {
        assert_eq!(
            instance_lock_path(Path::new("/data/trss/app.db")),
            Path::new("/data/trss/app.db.lock")
        );
    }

    #[test]
    fn a_second_taker_is_refused_until_the_first_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("app.db");

        let first = InstanceLock::try_take(&db).unwrap().expect("free lock");
        assert!(instance_lock_path(&db).exists());
        assert!(InstanceLock::try_take(&db).unwrap().is_none());
        assert!(InstanceLock::try_take(&db).unwrap().is_none());

        drop(first);
        assert!(InstanceLock::try_take(&db).unwrap().is_some());
    }

    #[test]
    fn a_dropped_guard_is_free_even_while_a_copy_of_its_descriptor_lives_on() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("app.db");

        let first = InstanceLock::try_take(&db).unwrap().expect("free lock");
        // What a child process being started holds until its `exec`.
        let copy = first.file.try_clone().unwrap();
        drop(first);
        assert!(InstanceLock::try_take(&db).unwrap().is_some());
        drop(copy);
    }

    #[test]
    fn a_missing_directory_is_an_error_not_a_free_lock() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("missing").join("app.db");
        assert!(InstanceLock::try_take(&db).is_err());
    }
}
