//! The lock that keeps two workers from running a cycle at the same time.

use std::{
    ffi::OsString,
    fs::{File, OpenOptions, TryLockError},
    io,
    path::{Path, PathBuf},
};

/// The lock file's path for a database file: the database path plus
/// `.worker.lock`, so it sits next to the database on the same local volume.
pub fn lock_path_for(db_path: &Path) -> PathBuf {
    let mut name: OsString = db_path.as_os_str().to_owned();
    name.push(".worker.lock");
    PathBuf::from(name)
}

/// An exclusive advisory lock (`flock`) on a file, held while a cycle runs.
///
/// The operating system, not the worker, owns the lock: it is released when the
/// guard is dropped and also when the process dies for any reason, so a
/// crashed worker never leaves a stale lock, and a worker that is merely slow
/// keeps its lock for as long as it runs. No time limit is involved, which is
/// what stops a second worker from entering while the first is still working.
///
/// Every `try_acquire` opens its own file description, so two guards on the
/// same path exclude each other even inside one process.
#[derive(Debug)]
pub struct CycleLock {
    _file: File,
}

impl CycleLock {
    /// Takes the lock if nobody holds it. `Ok(None)` means another worker does.
    pub fn try_acquire(path: &Path) -> io::Result<Option<CycleLock>> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;

        match file.try_lock() {
            Ok(()) => Ok(Some(CycleLock { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(err)) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_path_sits_next_to_the_database() {
        assert_eq!(
            lock_path_for(Path::new("/data/trss/app.db")),
            Path::new("/data/trss/app.db.worker.lock")
        );
    }

    #[test]
    fn a_second_holder_is_refused_until_the_first_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db.worker.lock");

        let first = CycleLock::try_acquire(&path).unwrap().expect("free lock");
        assert!(CycleLock::try_acquire(&path).unwrap().is_none());
        assert!(CycleLock::try_acquire(&path).unwrap().is_none());

        drop(first);
        assert!(CycleLock::try_acquire(&path).unwrap().is_some());
    }

    #[test]
    fn a_missing_directory_is_an_error_not_a_free_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("app.db.worker.lock");
        assert!(CycleLock::try_acquire(&path).is_err());
    }
}
