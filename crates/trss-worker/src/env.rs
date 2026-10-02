//! The worker's settings from environment variables.
//!
//! Transmission's address, speed limits, queue sizes and download directory
//! use the same variables as the former cron binary. Channels and rules come
//! from the app database, not from an environment variable.

use std::{str::FromStr, time::Duration};

use url::Url;

use trss_legacy::transmission::SessionConfig;

pub const TRANSMISSION_URL_VAR: &str = "TRANSMISSION_URL";
pub const DOWNLOAD_DIR_VAR: &str = "DOWNLOAD_DIR";
pub const SPEED_LIMIT_UP_VAR: &str = "SPEED_LIMIT_UP";
pub const SPEED_LIMIT_DOWN_VAR: &str = "SPEED_LIMIT_DOWN";
pub const DOWNLOAD_QUEUE_SIZE_VAR: &str = "DOWNLOAD_QUEUE_SIZE";
pub const SEED_QUEUE_SIZE_VAR: &str = "SEED_QUEUE_SIZE";
/// Seconds between collection cycles.
pub const INTERVAL_VAR: &str = "TRSS_WORKER_INTERVAL_SECS";

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

#[derive(Clone)]
pub struct WorkerEnv {
    /// May carry credentials (`http://user:password@host/`); never print it.
    pub transmission_url: Url,
    pub session: SessionConfig,
    pub interval: Duration,
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
