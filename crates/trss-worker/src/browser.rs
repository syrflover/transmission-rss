//! The server browser of the worker ([`trss_browser`]), made from the
//! environment and the common policy in the app database.

use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

use trss_browser::{BrowserPolicy, BrowserPool, PolicySource, PoolConfig};
use trss_core::{settings::SettingsStore, Clock, CycleLock, Db};

use crate::env::BrowserEnv;

/// The policy the pool follows: the common policy's idle time and cap on
/// concurrent browser jobs, read when they are needed, so a change applies
/// without a restart. A policy that cannot be read is the defaults.
pub fn policy_source(db: Db) -> PolicySource {
    let settings = SettingsStore::new(db);
    PolicySource::new(move || {
        let settings = settings.clone();
        Box::pin(async move {
            match settings.policy().await {
                Ok(policy) => BrowserPolicy::from_policy(&policy),
                Err(err) => {
                    eprintln!("Browser: cannot read the common policy ({err}); using the defaults");
                    BrowserPolicy::default()
                }
            }
        })
    })
}

/// The lock file of the server browser for a database file: the database
/// path plus `.browser.lock`, next to the database like the worker's own lock.
pub fn lock_path_for(db_path: &Path) -> PathBuf {
    let mut name: OsString = db_path.as_os_str().to_owned();
    name.push(".browser.lock");
    PathBuf::from(name)
}

/// Takes the right to use the browser container, for as long as the guard
/// lives (the worker keeps it for its whole life; the operating system lets it
/// go if the worker dies). `Ok(None)` means another worker has it.
///
/// A pool resets the container when it is made and its reaper ends the runs
/// it does not know, so two workers on one container would end each other's
/// runs. One worker at a time uses it.
pub fn take_lock(db_path: &Path) -> io::Result<Option<CycleLock>> {
    CycleLock::try_acquire(&lock_path_for(db_path))
}

/// The pool over the browser container `env` names. This resets the
/// container, so a browser an earlier worker left open is closed.
pub async fn connect(db: Db, env: &BrowserEnv, clock: Clock) -> Result<BrowserPool, String> {
    let config = PoolConfig::new(env.url.clone(), env.token.clone(), env.downloads.clone());
    BrowserPool::new(config, clock, policy_source(db))
        .await
        .map_err(|e| format!("cannot set up the server browser: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_worker_at_a_time_has_the_browser() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("trss.db");
        assert_eq!(lock_path_for(&db), dir.path().join("trss.db.browser.lock"));

        let first = take_lock(&db).unwrap();
        assert!(first.is_some());
        assert!(take_lock(&db).unwrap().is_none(), "a second worker got it");
        // It is the browser's own lock, not the worker's.
        assert!(CycleLock::try_acquire(&trss_core::lock_path_for(&db))
            .unwrap()
            .is_some());

        drop(first);
        assert!(take_lock(&db).unwrap().is_some());
    }
}
