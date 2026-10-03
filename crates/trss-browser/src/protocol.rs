//! The launcher's HTTP API, as the launcher answers and the worker reads it.
//!
//! Every request carries `Authorization: Bearer <token>`.
//!
//! | Request | Answer |
//! | --- | --- |
//! | `POST /runs` `{"run": id}` | 200 [`Started`]; the same id answers the run that exists; 429 at the cap; 409 while that id is being ended |
//! | `GET /runs` | 200 [`RunList`]; no side effect |
//! | `GET /runs/{id}/cdp` | WebSocket upgrade to the run's browser-level DevTools socket; 404 for an unknown or ended run |
//! | `DELETE /runs/{id}` | 204, also for an unknown id |
//! | `POST /reset` | 204 once every run is ended and every profile removed |

use serde::{Deserialize, Serialize};

/// The longest run id.
pub const MAX_RUN_ID_LEN: usize = 80;

/// Whether `id` is a run id: 1 to 80 of `A-Z a-z 0-9 _ -`. The id is part of
/// a path on the launcher's disk, so nothing else is allowed.
pub fn is_valid_run_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_RUN_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartRequest {
    pub run: String,
}

/// A started run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Started {
    pub run: String,
    /// Where Chromium saves the run's downloads, as the browser container
    /// sees the folder.
    pub downloads: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInfo {
    pub run: String,
    /// Unix milliseconds.
    pub started_at: i64,
    /// Whether Chromium's process is still there.
    pub pid_alive: bool,
    pub downloads: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunList {
    pub runs: Vec<RunInfo>,
}

/// The body of an error answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_ids_are_plain_names() {
        for good in ["a", "job-1_x", "A9", &"x".repeat(80)] {
            assert!(is_valid_run_id(good), "{good}");
        }
        for bad in [
            "",
            "a b",
            "../x",
            "a/b",
            "a.b",
            "한글",
            "x\n",
            &"x".repeat(81),
        ] {
            assert!(!is_valid_run_id(bad), "{bad:?}");
        }
    }
}
