use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use crate::origin_guard::{AllowedHosts, HOSTS_VAR};
use url::Url;

/// Environment variable naming the address `trss-web` listens on.
pub const BIND_VAR: &str = "TRSS_WEB_BIND";
/// Environment variable naming the port `trss-web` listens on.
pub const PORT_VAR: &str = "TRSS_WEB_PORT";
/// Environment variable naming the directory that holds the frontend build.
pub const STATIC_DIR_VAR: &str = "TRSS_WEB_STATIC_DIR";
/// The server browser's launcher (the worker's variable of the same name):
/// the web shows a job's remote screen through it ([`crate::screen_api`]).
pub const BROWSER_URL_VAR: &str = "TRSS_BROWSER_URL";
/// The launcher's token, with [`BROWSER_URL_VAR`].
pub const BROWSER_TOKEN_VAR: &str = "TRSS_BROWSER_TOKEN";

const DEFAULT_BIND: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const DEFAULT_PORT: u16 = 8080;
const DEFAULT_STATIC_DIR: &str = "web/dist";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnvError {
    #[error("{BIND_VAR} must be an IP address, got {0:?}")]
    Bind(String),
    #[error("{PORT_VAR} must be a port number (0-65535), got {0:?}")]
    Port(String),
    #[error("{HOSTS_VAR} must be host names separated by commas, got {0:?}")]
    Hosts(String),
    #[error("{BROWSER_URL_VAR} must be an http address, got {0:?}")]
    BrowserUrl(String),
    #[error("{BROWSER_URL_VAR} is set but {BROWSER_TOKEN_VAR} is not")]
    BrowserToken,
}

/// How the web reaches the server browser's launcher: only through its
/// token-protected DevTools proxy of a run, never to start or end one.
#[derive(Clone, PartialEq, Eq)]
pub struct BrowserAccess {
    pub url: Url,
    pub token: String,
}

impl std::fmt::Debug for BrowserAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is a secret.
        f.debug_struct("BrowserAccess")
            .field("url", &self.url.as_str())
            .finish_non_exhaustive()
    }
}

/// Deployment settings of `trss-web`, read from the environment.
///
/// There is no app login (access is limited by the network in front of the
/// app), so the default bind address is loopback; a deployment that should be
/// reachable from other hosts must opt in with `TRSS_WEB_BIND`.
///
/// Requests are answered for IP addresses and `localhost`; a host name the
/// web is reached by (through a reverse proxy, say) must be listed in
/// `TRSS_WEB_HOSTS` (see [`crate::origin_guard`]).
///
/// With `TRSS_BROWSER_URL` (and `TRSS_BROWSER_TOKEN`) the web shows the remote
/// screens of the jobs that wait for a site's check, and refuses every
/// connection from the server browser's network ([`crate::browser_net`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebEnv {
    pub addr: SocketAddr,
    pub static_dir: PathBuf,
    pub hosts: AllowedHosts,
    pub browser: Option<BrowserAccess>,
}

impl WebEnv {
    pub fn from_env() -> Result<Self, EnvError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Reads settings through `lookup`. Empty values count as unset.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, EnvError> {
        let get = |key: &str| lookup(key).filter(|value| !value.trim().is_empty());

        let ip = match get(BIND_VAR) {
            Some(value) => value
                .trim()
                .parse::<IpAddr>()
                .map_err(|_| EnvError::Bind(value))?,
            None => DEFAULT_BIND,
        };
        let port = match get(PORT_VAR) {
            Some(value) => value
                .trim()
                .parse::<u16>()
                .map_err(|_| EnvError::Port(value))?,
            None => DEFAULT_PORT,
        };
        let static_dir = get(STATIC_DIR_VAR)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_STATIC_DIR));
        let hosts = match get(HOSTS_VAR) {
            Some(value) => AllowedHosts::parse(&value).ok_or(EnvError::Hosts(value))?,
            None => AllowedHosts::default(),
        };
        let browser = match get(BROWSER_URL_VAR) {
            Some(value) => {
                let url = Url::parse(value.trim())
                    .ok()
                    .filter(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
                    .ok_or_else(|| EnvError::BrowserUrl(value.clone()))?;
                let token = get(BROWSER_TOKEN_VAR).ok_or(EnvError::BrowserToken)?;
                Some(BrowserAccess { url, token })
            }
            None => None,
        };

        Ok(Self {
            addr: SocketAddr::new(ip, port),
            static_dir,
            hosts,
            browser,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Result<WebEnv, EnvError> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        WebEnv::from_lookup(|key| map.get(key).cloned())
    }

    #[test]
    fn defaults_to_loopback_and_local_build_dir() {
        let env = env(&[]).unwrap();
        assert_eq!(env.addr, "127.0.0.1:8080".parse().unwrap());
        assert_eq!(env.static_dir, PathBuf::from("web/dist"));
        assert_eq!(env.hosts, AllowedHosts::default());
    }

    #[test]
    fn reads_the_host_names() {
        let read = env(&[(HOSTS_VAR, "trss.example.com, NAS.lan:8080")]).unwrap();
        assert_eq!(read.hosts.names(), ["trss.example.com", "nas.lan"]);
        assert_eq!(
            env(&[(HOSTS_VAR, "https://trss.example.com")]),
            Err(EnvError::Hosts("https://trss.example.com".into()))
        );
    }

    #[test]
    fn reads_all_variables() {
        let env = env(&[
            (BIND_VAR, "0.0.0.0"),
            (PORT_VAR, "9000"),
            (STATIC_DIR_VAR, "/srv/web"),
        ])
        .unwrap();
        assert_eq!(env.addr, "0.0.0.0:9000".parse().unwrap());
        assert_eq!(env.static_dir, PathBuf::from("/srv/web"));
    }

    #[test]
    fn accepts_ipv6_and_ignores_empty_values() {
        let env = env(&[(BIND_VAR, "::1"), (PORT_VAR, " "), (STATIC_DIR_VAR, "")]).unwrap();
        assert_eq!(env.addr, "[::1]:8080".parse().unwrap());
        assert_eq!(env.static_dir, PathBuf::from("web/dist"));
    }

    #[test]
    fn the_server_browser_needs_its_address_and_token_together() {
        assert_eq!(env(&[]).unwrap().browser, None);
        let env_ok = env(&[
            (BROWSER_URL_VAR, "http://trss-browser:9230"),
            (BROWSER_TOKEN_VAR, "secret-token"),
        ])
        .unwrap();
        let browser = env_ok.browser.unwrap();
        assert_eq!(browser.url.as_str(), "http://trss-browser:9230/");
        assert_eq!(browser.token, "secret-token");
        assert!(!format!("{browser:?}").contains("secret-token"));
        assert_eq!(
            env(&[(BROWSER_URL_VAR, "http://trss-browser:9230")]),
            Err(EnvError::BrowserToken)
        );
        assert_eq!(
            env(&[
                (BROWSER_URL_VAR, "trss-browser:9230"),
                (BROWSER_TOKEN_VAR, "t")
            ]),
            Err(EnvError::BrowserUrl("trss-browser:9230".into()))
        );
    }

    #[test]
    fn rejects_bad_values() {
        assert_eq!(
            env(&[(BIND_VAR, "localhost")]),
            Err(EnvError::Bind("localhost".into()))
        );
        assert_eq!(
            env(&[(PORT_VAR, "70000")]),
            Err(EnvError::Port("70000".into()))
        );
    }
}
