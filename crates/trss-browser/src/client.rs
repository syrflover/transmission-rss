//! The worker's HTTP client for the launcher ([`crate::protocol`]).

use std::time::Duration;

use reqwest::{Method, StatusCode};
use url::Url;

use crate::protocol::{ErrorBody, RunInfo, RunList, StartRequest, Started};

#[derive(Debug, thiserror::Error)]
pub enum LauncherError {
    /// The container did not answer. The message has no address and no token.
    #[error("cannot reach the browser container: {0}")]
    Unreachable(String),
    #[error("the browser container answered {status}: {message}")]
    Status { status: u16, message: String },
}

impl LauncherError {
    pub fn status(&self) -> Option<u16> {
        match self {
            LauncherError::Status { status, .. } => Some(*status),
            LauncherError::Unreachable(_) => None,
        }
    }
}

/// Calls to the launcher at `base`, with its token.
#[derive(Clone)]
pub struct LauncherClient {
    http: reqwest::Client,
    base: Url,
    token: String,
}

impl std::fmt::Debug for LauncherClient {
    /// Without the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LauncherClient")
            .field("base", &self.base.as_str())
            .finish_non_exhaustive()
    }
}

impl LauncherClient {
    pub fn new(base: Url, token: impl Into<String>) -> Result<LauncherClient, reqwest::Error> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            // A start waits for Chromium, an end for its grace period.
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(LauncherClient {
            http,
            base,
            token: token.into(),
        })
    }

    fn url(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        url.set_path(path);
        url
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<String>,
    ) -> Result<(StatusCode, Vec<u8>), LauncherError> {
        let mut request = self
            .http
            .request(method, self.url(path))
            .bearer_auth(&self.token);
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body);
        }
        let unreachable =
            |e: reqwest::Error| LauncherError::Unreachable(e.without_url().to_string());
        let response = request.send().await.map_err(unreachable)?;
        let status = response.status();
        let body = response.bytes().await.map_err(unreachable)?;
        Ok((status, body.to_vec()))
    }

    fn refuse(status: StatusCode, body: &[u8]) -> LauncherError {
        let message = serde_json::from_slice::<ErrorBody>(body)
            .map(|b| b.error)
            .unwrap_or_default();
        LauncherError::Status {
            status: status.as_u16(),
            message,
        }
    }

    /// Starts the run `run`, or gets the one that exists.
    pub async fn start(&self, run: &str) -> Result<Started, LauncherError> {
        let body = serde_json::to_string(&StartRequest {
            run: run.to_owned(),
        })
        .expect("json");
        let (status, body) = self.call(Method::POST, "/runs", Some(body)).await?;
        if status != StatusCode::OK {
            return Err(Self::refuse(status, &body));
        }
        serde_json::from_slice(&body)
            .map_err(|e| LauncherError::Unreachable(format!("an answer that is not a run: {e}")))
    }

    /// The live runs. Changes nothing.
    pub async fn list(&self) -> Result<Vec<RunInfo>, LauncherError> {
        let (status, body) = self.call(Method::GET, "/runs", None).await?;
        if status != StatusCode::OK {
            return Err(Self::refuse(status, &body));
        }
        serde_json::from_slice::<RunList>(&body)
            .map(|l| l.runs)
            .map_err(|e| LauncherError::Unreachable(format!("an answer that is not a list: {e}")))
    }

    /// Ends the run `run`; fine for one that is not there.
    pub async fn end(&self, run: &str) -> Result<(), LauncherError> {
        let (status, body) = self
            .call(Method::DELETE, &format!("/runs/{run}"), None)
            .await?;
        if status.is_success() {
            Ok(())
        } else {
            Err(Self::refuse(status, &body))
        }
    }

    /// Ends every run and removes every profile.
    pub async fn reset(&self) -> Result<(), LauncherError> {
        let (status, body) = self.call(Method::POST, "/reset", None).await?;
        if status.is_success() {
            Ok(())
        } else {
            Err(Self::refuse(status, &body))
        }
    }

    /// The address of the run's DevTools proxy (a WebSocket).
    pub fn cdp_url(&self, run: &str) -> String {
        let mut url = self.url(&format!("/runs/{run}/cdp"));
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        // `http` and `https` are special schemes, as are `ws` and `wss`.
        let _ = url.set_scheme(scheme);
        url.to_string()
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_proxy_address_follows_the_launchers() {
        let client = LauncherClient::new("http://trss-browser:9230".parse().unwrap(), "t").unwrap();
        assert_eq!(client.cdp_url("a-1"), "ws://trss-browser:9230/runs/a-1/cdp");
        let client = LauncherClient::new("https://b.example/".parse().unwrap(), "t").unwrap();
        assert_eq!(client.cdp_url("a"), "wss://b.example/runs/a/cdp");
        let secret = LauncherClient::new("http://x/".parse().unwrap(), "s3cret-token").unwrap();
        assert!(!format!("{secret:?}").contains("s3cret"));
    }
}
