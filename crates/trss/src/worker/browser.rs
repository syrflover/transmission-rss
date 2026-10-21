//! The server browser of the worker ([`trss_browser`]), made from the
//! environment and the common policy in the app database.

use std::sync::atomic::{AtomicBool, Ordering};

use trss_browser::{ActivitySource, BrowserPolicy, BrowserPool, PolicySource, PoolConfig};
use trss_core::{settings::SettingsStore, Clock, Db};
use trss_jobs::ScreenStore;

use crate::worker::env::BrowserEnv;

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

/// Whether this process has made its pool ([`connect`]).
static CONNECTED: AtomicBool = AtomicBool::new(false);

/// The pool over the browser container `env` names. This resets the
/// container, so a browser an earlier process left open is closed. Its idle
/// end counts a person's input on a job's remote screen ([`screen_activity`]).
///
/// A process makes one pool: a second would reset the container again, and
/// each pool's reaper ends the runs it does not know, so the two would end
/// each other's runs. A second call is refused.
pub async fn connect(db: Db, env: &BrowserEnv, clock: Clock) -> Result<BrowserPool, String> {
    if CONNECTED.swap(true, Ordering::SeqCst) {
        return Err("a pool over the browser container is made already in this process".into());
    }
    let config = PoolConfig::new(env.url.clone(), env.token.clone(), env.downloads.clone())
        .with_activity(screen_activity(db.clone()));
    let pool = BrowserPool::new(config, clock, policy_source(db)).await;
    if pool.is_err() {
        // No pool was made: a later call may make one.
        CONNECTED.store(false, Ordering::SeqCst);
    }
    pool.map_err(|e| format!("cannot set up the server browser: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_process_makes_one_pool_over_the_browser_container() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        // No container answers there: the pool is made all the same and
        // resets the container at its first use.
        let env = BrowserEnv {
            url: "http://127.0.0.1:9/".parse().unwrap(),
            token: "token".into(),
            downloads: dir.path().join("downloads"),
        };

        assert!(connect(db.clone(), &env, trss_core::system_clock())
            .await
            .is_ok());
        match connect(db, &env, trss_core::system_clock()).await {
            Ok(_) => panic!("a second pool was made"),
            Err(reason) => assert!(reason.contains("made already"), "{reason}"),
        }
    }
}
