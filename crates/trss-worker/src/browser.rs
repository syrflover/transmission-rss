//! The server browser of the worker ([`trss_browser`]), made from the
//! environment and the common policy in the app database.

use std::{
    io,
    path::{Path, PathBuf},
};

use trss_browser::{ActivitySource, BrowserPolicy, BrowserPool, PolicySource, PoolConfig};
use trss_core::{settings::SettingsStore, Clock, CycleLock, Db, LockFile};
use trss_jobs::ScreenStore;

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

/// A person's input on the remote screens of the jobs that wait for a site's
/// check (`trss_jobs::screen`), which the web records: the pool's idle end
/// counts it as use of the bound run. Input that cannot be read is none.
pub fn screen_activity(db: Db) -> ActivitySource {
    let screens = ScreenStore::new(db);
    ActivitySource::new(move || {
        let screens = screens.clone();
        Box::pin(async move {
            screens.live_inputs().await.unwrap_or_else(|err| {
                eprintln!("Browser: cannot read the remote screens' input ({err})");
                Vec::new()
            })
        })
    })
}

/// The lock file of the server browser for a database file: the database
/// path plus `.browser.lock`, next to the database like the worker's own lock.
pub fn lock_path_for(db_path: &Path) -> PathBuf {
    LockFile::Browser.path_for(db_path)
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
/// container, so a browser an earlier worker left open is closed. Its idle end
/// counts a person's input on a job's remote screen ([`screen_activity`]).
pub async fn connect(db: Db, env: &BrowserEnv, clock: Clock) -> Result<BrowserPool, String> {
    let config = PoolConfig::new(env.url.clone(), env.token.clone(), env.downloads.clone())
        .with_activity(screen_activity(db.clone()));
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
