//! The server browser (`docs/specs/jobs.md`, 작업 화면 안의 인증과 브라우저 수명).
//!
//! Two halves, in one crate because they speak one protocol:
//!
//! - [`launcher`] and the binary `trss-browserd`: what runs in the browser
//!   container. A virtual display and a small HTTP service that starts and
//!   ends one Chromium per run, with a profile of its own that is deleted with
//!   the run, and proxies the run's DevTools socket behind a token: to the
//!   worker, and to the web, which shows a run's page to a person and relays
//!   the person's input through a connection of its own (a run takes more
//!   than one). No port of the container is published.
//! - [`pool`]: the worker's side. A [`BrowserPool`] gives each job a
//!   [`BrowserRun`], drives it through [`cdp`], keeps within the policy's cap
//!   on concurrent browser jobs, and ends runs that are idle past the policy's
//!   idle time.
//!
//! The requests between them are in [`protocol`]. Which subtitle source uses
//! the browser, and the screen shown to a person, are not here.

pub mod blocklist;
pub mod cdp;
pub mod client;
pub mod launcher;
pub mod pool;
pub mod protocol;

pub use pool::{
    ActivitySource, BrowserError, BrowserPolicy, BrowserPool, BrowserRun, DialogSeen, Dialogs,
    Download, DownloadAnswer, DownloadSource, DownloadState, MovedFile, Page, PageDocument,
    PolicySource, PoolConfig, RunStatus, DOWNLOAD_STALL, MAX_DOWNLOAD_BYTES,
    MAX_RUN_DOWNLOAD_BYTES,
};
