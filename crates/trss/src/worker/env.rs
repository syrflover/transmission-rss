//! The worker's settings from environment variables.
//!
//! Transmission's address, speed limits, queue sizes and download directory
//! use the same variables as the former cron binary. Channels and rules come
//! from the app database, not from an environment variable.

use std::{path::PathBuf, str::FromStr, time::Duration};

use url::Url;

use trss_transmission::SessionConfig;

pub const TRANSMISSION_URL_VAR: &str = "TRANSMISSION_URL";
pub const DOWNLOAD_DIR_VAR: &str = "DOWNLOAD_DIR";
pub const SPEED_LIMIT_UP_VAR: &str = "SPEED_LIMIT_UP";
pub const SPEED_LIMIT_DOWN_VAR: &str = "SPEED_LIMIT_DOWN";
pub const DOWNLOAD_QUEUE_SIZE_VAR: &str = "DOWNLOAD_QUEUE_SIZE";
pub const SEED_QUEUE_SIZE_VAR: &str = "SEED_QUEUE_SIZE";
/// Seconds between collection cycles.
pub const INTERVAL_VAR: &str = "TRSS_WORKER_INTERVAL_SECS";
/// `1` turns on the fake subtitle source ([`trss_subtitles::fake`]) for the
/// development environment and the tests. Never set in production.
pub const FAKE_SUBTITLE_SOURCE_VAR: &str = "TRSS_FAKE_SUBTITLE_SOURCE";

/// The browser container's launcher (`trss_browser::launcher`), for example
/// `http://trss-browser:9230`. Unset: the worker runs no browser.
pub const BROWSER_URL_VAR: &str = "TRSS_BROWSER_URL";
/// What the launcher requires of every request. Required with the address.
pub const BROWSER_TOKEN_VAR: &str = "TRSS_BROWSER_TOKEN";
/// The shared downloads folder, at the path this worker sees it. Required
/// with the address.
pub const BROWSER_DOWNLOADS_VAR: &str = "TRSS_BROWSER_DOWNLOADS";

/// Five minutes, the period cron ran the former binary at.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(300);

/// A bad setting. Messages name the variable and never its value, because
/// `TRANSMISSION_URL` may carry credentials.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnvError {
    #[error("environment variable {0} is not set")]
    Missing(&'static str),
    #[error("environment variable {0} has an invalid value")]
    Invalid(&'static str),
}

/// Where the server browser is and how to reach it.
#[derive(Clone)]
pub struct BrowserEnv {
    pub url: Url,
    /// Never printed.
    pub token: String,
    pub downloads: PathBuf,
}

impl std::fmt::Debug for BrowserEnv {
    /// Without the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserEnv")
            .field("url", &self.url.as_str())
            .field("downloads", &self.downloads)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct WorkerEnv {
    /// May carry credentials (`http://user:password@host/`); never print it.
    pub transmission_url: Url,
    pub session: SessionConfig,
    pub interval: Duration,
    /// The server browser; `None`: the worker runs without one.
    pub browser: Option<BrowserEnv>,
}

impl std::fmt::Debug for WorkerEnv {
    /// Shows the Transmission address without its credentials.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut url = self.transmission_url.clone();
        let has_credentials = !url.username().is_empty() || url.password().is_some();
        let _ = url.set_username("");
        let _ = url.set_password(None);
        f.debug_struct("WorkerEnv")
            .field("transmission_url", &url.as_str())
            .field("has_credentials", &has_credentials)
            .field("session", &self.session)
            .field("interval", &self.interval)
            .field("browser", &self.browser)
            .finish()
    }
}

impl WorkerEnv {
    pub fn from_env() -> Result<WorkerEnv, EnvError> {
        WorkerEnv::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<WorkerEnv, EnvError> {
        fn optional<T: FromStr>(
            get: &impl Fn(&str) -> Option<String>,
            key: &'static str,
        ) -> Result<Option<T>, EnvError> {
            get(key)
                .map(|value| value.parse().map_err(|_| EnvError::Invalid(key)))
                .transpose()
        }

        let transmission_url = get(TRANSMISSION_URL_VAR)
            .ok_or(EnvError::Missing(TRANSMISSION_URL_VAR))?
            .parse::<Url>()
            .map_err(|_| EnvError::Invalid(TRANSMISSION_URL_VAR))?;

        let interval = match optional::<u64>(&get, INTERVAL_VAR)? {
            None => DEFAULT_INTERVAL,
            Some(0) => return Err(EnvError::Invalid(INTERVAL_VAR)),
            Some(secs) => Duration::from_secs(secs),
        };

        let browser = match get(BROWSER_URL_VAR).filter(|v| !v.is_empty()) {
            None => None,
            Some(url) => Some(BrowserEnv {
                url: url
                    .parse()
                    .map_err(|_| EnvError::Invalid(BROWSER_URL_VAR))?,
                token: get(BROWSER_TOKEN_VAR)
                    .filter(|v| !v.is_empty())
                    .ok_or(EnvError::Missing(BROWSER_TOKEN_VAR))?,
                downloads: get(BROWSER_DOWNLOADS_VAR)
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from)
                    .ok_or(EnvError::Missing(BROWSER_DOWNLOADS_VAR))?,
            }),
        };

        Ok(WorkerEnv {
            transmission_url,
            session: SessionConfig {
                download_dir: optional(&get, DOWNLOAD_DIR_VAR)?,
                speed_limit_up: optional(&get, SPEED_LIMIT_UP_VAR)?,
                speed_limit_down: optional(&get, SPEED_LIMIT_DOWN_VAR)?,
                download_queue_size: optional(&get, DOWNLOAD_QUEUE_SIZE_VAR)?,
                seed_queue_size: optional(&get, SEED_QUEUE_SIZE_VAR)?,
            },
            interval,
            browser,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn only_the_transmission_url_is_required() {
        let env =
            WorkerEnv::from_lookup(lookup(&[("TRANSMISSION_URL", "http://tr:9091/rpc")])).unwrap();
        assert_eq!(env.transmission_url.as_str(), "http://tr:9091/rpc");
        assert_eq!(env.session, SessionConfig::default());
        assert_eq!(env.interval, DEFAULT_INTERVAL);

        assert_eq!(
            WorkerEnv::from_lookup(lookup(&[])).unwrap_err(),
            EnvError::Missing("TRANSMISSION_URL")
        );
    }

    #[test]
    fn channels_config_url_is_not_needed() {
        // The former binary panicked without it; the worker ignores it.
        assert!(WorkerEnv::from_lookup(lookup(&[("TRANSMISSION_URL", "http://tr/")])).is_ok());
    }

    #[test]
    fn session_settings_use_the_legacy_variable_names() {
        let env = WorkerEnv::from_lookup(lookup(&[
            ("TRANSMISSION_URL", "http://tr/"),
            ("DOWNLOAD_DIR", "/downloads"),
            ("SPEED_LIMIT_UP", "100"),
            ("SPEED_LIMIT_DOWN", "2000"),
            ("DOWNLOAD_QUEUE_SIZE", "3"),
            ("SEED_QUEUE_SIZE", "4"),
            ("TRSS_WORKER_INTERVAL_SECS", "60"),
        ]))
        .unwrap();
        assert_eq!(
            env.session,
            SessionConfig {
                download_dir: Some("/downloads".into()),
                speed_limit_up: Some(100),
                speed_limit_down: Some(2000),
                download_queue_size: Some(3),
                seed_queue_size: Some(4),
            }
        );
        assert_eq!(env.interval, Duration::from_secs(60));
    }

    #[test]
    fn the_browser_is_off_unless_its_address_is_set() {
        let env = WorkerEnv::from_lookup(lookup(&[("TRANSMISSION_URL", "http://tr/")])).unwrap();
        assert!(env.browser.is_none());

        // The token and the folder alone do not turn it on.
        let env = WorkerEnv::from_lookup(lookup(&[
            ("TRANSMISSION_URL", "http://tr/"),
            ("TRSS_BROWSER_TOKEN", "t"),
            ("TRSS_BROWSER_DOWNLOADS", "/data/browser-downloads"),
        ]))
        .unwrap();
        assert!(env.browser.is_none());

        let env = WorkerEnv::from_lookup(lookup(&[
            ("TRANSMISSION_URL", "http://tr/"),
            ("TRSS_BROWSER_URL", "http://trss-browser:9230"),
            ("TRSS_BROWSER_TOKEN", "s3cret-token"),
            ("TRSS_BROWSER_DOWNLOADS", "/data/browser-downloads"),
        ]))
        .unwrap();
        let browser = env.browser.as_ref().unwrap();
        assert_eq!(browser.url.as_str(), "http://trss-browser:9230/");
        assert_eq!(browser.token, "s3cret-token");
        assert_eq!(browser.downloads, PathBuf::from("/data/browser-downloads"));
        assert!(!format!("{env:?}").contains("s3cret"));
    }

    #[test]
    fn a_browser_address_needs_its_token_and_folder() {
        let with = |extra: &[(&str, &str)]| {
            let mut vars = vec![
                ("TRANSMISSION_URL", "http://tr/"),
                ("TRSS_BROWSER_URL", "http://trss-browser:9230"),
            ];
            vars.extend_from_slice(extra);
            WorkerEnv::from_lookup(lookup(&vars)).unwrap_err()
        };
        assert_eq!(
            with(&[("TRSS_BROWSER_DOWNLOADS", "/d")]),
            EnvError::Missing("TRSS_BROWSER_TOKEN")
        );
        assert_eq!(
            with(&[("TRSS_BROWSER_TOKEN", "t")]),
            EnvError::Missing("TRSS_BROWSER_DOWNLOADS")
        );
        let bad = WorkerEnv::from_lookup(lookup(&[
            ("TRANSMISSION_URL", "http://tr/"),
            ("TRSS_BROWSER_URL", "not a url"),
        ]))
        .unwrap_err();
        assert_eq!(bad, EnvError::Invalid("TRSS_BROWSER_URL"));
    }

    #[test]
    fn debug_output_has_no_credentials() {
        let env = WorkerEnv::from_lookup(lookup(&[(
            "TRANSMISSION_URL",
            "http://admin:hunter2@tr:9091/transmission/rpc",
        )]))
        .unwrap();
        // The credentials are still there for the client to use.
        assert_eq!(env.transmission_url.password(), Some("hunter2"));

        let shown = format!("{env:?}");
        assert!(
            !shown.contains("hunter2") && !shown.contains("admin"),
            "{shown}"
        );
        assert!(shown.contains("tr:9091/transmission/rpc"), "{shown}");
    }

    #[test]
    fn invalid_values_are_named_but_not_echoed() {
        let err = WorkerEnv::from_lookup(lookup(&[
            ("TRANSMISSION_URL", "http://user:hunter2@tr/"),
            ("SPEED_LIMIT_UP", "fast"),
        ]))
        .unwrap_err();
        assert_eq!(err, EnvError::Invalid("SPEED_LIMIT_UP"));

        let err = WorkerEnv::from_lookup(lookup(&[("TRANSMISSION_URL", "not a url hunter2")]))
            .unwrap_err();
        assert!(!err.to_string().contains("hunter2"));

        assert_eq!(
            WorkerEnv::from_lookup(lookup(&[
                ("TRANSMISSION_URL", "http://tr/"),
                ("TRSS_WORKER_INTERVAL_SECS", "0"),
            ]))
            .unwrap_err(),
            EnvError::Invalid("TRSS_WORKER_INTERVAL_SECS")
        );
    }
}
