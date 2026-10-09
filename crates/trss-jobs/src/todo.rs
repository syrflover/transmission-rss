//! The to-dos that need a person (`docs/specs/jobs.md`, 할 일): `처리 필요`,
//! the cards of the to-do screen and the library list's badges.
//!
//! This is where they are gathered from what waits for a person (the jobs,
//! the subscribed creators' mappings, the library's videos and the failed
//! receipts), grouped by work, summed (a `교체 승인` card's [`Changes`]) and
//! put in order ([`list`]); the web only serializes them. Each kind's fields
//! are documented where the web answers with them (`todo_api` of `trss-web`).
//!
//! - [`Todo`]: one to-do, whatever its kind.
//! - [`badges_by_work`]: each work's kinds of to-do, which the library list
//!   shows as badges.
//! - [`Changes`] and [`Received`]: what a `교체 승인` to-do's open plans change
//!   and when their subtitles were received.

mod changes;
mod gather;
mod receive_failed;

use std::collections::HashMap;

use serde::Serialize;
use thiserror::Error;

use trss_collect::store::channels::ChannelError;
use trss_library::discovery::SeenFile;

pub use changes::{Changes, Received};
pub use gather::Sources;
pub use receive_failed::ADD_FAILURES;

/// Why the to-dos could not be read.
#[derive(Debug, Error)]
pub enum TodoError {
    /// The channels' rules, whose error the web tells apart.
    #[error(transparent)]
    Channel(#[from] ChannelError),
    /// Any other store; its message.
    #[error("{0}")]
    Read(String),
}

impl TodoError {
    /// Wraps the error of a store read.
    pub(crate) fn read(e: impl std::fmt::Display) -> Self {
        TodoError::Read(e.to_string())
    }
}

/// The work a to-do is about, with its cover's address.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkRef {
    pub id: String,
    pub name: String,
    pub cover_url: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Todo {
    Auth {
        key: String,
        at: i64,
        work: Option<WorkRef>,
        title: String,
        season: Option<i64>,
        episodes: Vec<String>,
        creator: Option<String>,
        reason: String,
        job_id: String,
        jobs: usize,
    },
    ReceiveFailed {
        key: String,
        at: i64,
        context: &'static str,
        work: Option<WorkRef>,
        title: String,
        season: Option<u32>,
        episodes: Vec<String>,
        count: usize,
        reason: Option<String>,
        channel_id: Option<String>,
    },
    EpisodeCheck {
        key: String,
        at: i64,
        work: Option<WorkRef>,
        title: String,
        season: u32,
        creator: String,
        source_id: String,
        episodes: Vec<String>,
        reason: Option<String>,
        sources: usize,
    },
    PlacementCheck {
        key: String,
        at: i64,
        work: Option<WorkRef>,
        title: String,
        season: Option<i64>,
        creator: Option<String>,
        origin: String,
        source: Option<String>,
        files: Vec<String>,
        reason: Option<String>,
        job_id: String,
    },
    VideoCheck {
        key: String,
        at: i64,
        work: Option<WorkRef>,
        title: String,
        season: u32,
        path: String,
        reason: String,
        seen: String,
    },
    Replacement {
        key: String,
        at: i64,
        work: Option<WorkRef>,
        title: String,
        season: Option<i64>,
        episodes: Vec<i64>,
        creator: Option<String>,
        job_id: String,
        jobs: usize,
        changes: Changes,
        current_received_at: Option<i64>,
        current_changed_at: Option<i64>,
        new_received_at: Option<i64>,
    },
}

impl Todo {
    fn at(&self) -> i64 {
        match self {
            Todo::Auth { at, .. }
            | Todo::ReceiveFailed { at, .. }
            | Todo::EpisodeCheck { at, .. }
            | Todo::PlacementCheck { at, .. }
            | Todo::VideoCheck { at, .. }
            | Todo::Replacement { at, .. } => *at,
        }
    }

    /// The library's work it is about, when it has one.
    pub fn work_id(&self) -> Option<&str> {
        match self {
            Todo::Auth { work, .. }
            | Todo::ReceiveFailed { work, .. }
            | Todo::EpisodeCheck { work, .. }
            | Todo::PlacementCheck { work, .. }
            | Todo::VideoCheck { work, .. }
            | Todo::Replacement { work, .. } => work.as_ref().map(|w| w.id.as_str()),
        }
    }

    /// Its kind as a badge names it: a job's 배치 확인 and a video's episode
    /// are a `회차 확인 필요` like a mapping's.
    pub fn badge(&self) -> &'static str {
        match self {
            Todo::Auth { .. } => "auth",
            Todo::ReceiveFailed { .. } => "receive_failed",
            Todo::Replacement { .. } => "replacement",
            Todo::EpisodeCheck { .. } | Todo::PlacementCheck { .. } | Todo::VideoCheck { .. } => {
                "episode_check"
            }
        }
    }
}

/// A video as the scan saw it, as the `회차 확인 필요` to-do of a video gives it
/// to the screen: its size and modification time in nanoseconds, which a JSON
/// number in a browser could not hold exactly (`size:mtime_ns`).
pub fn seen(identity: SeenFile) -> String {
    format!("{}:{}", identity.size, identity.mtime_ns)
}

/// Each work's to-do badges (the library grid's): the kinds of its to-dos,
/// once each, in the order of `todos` ([`list`]'s: the red kinds first). A
/// to-do of no work has none.
pub fn badges_by_work(todos: &[Todo]) -> HashMap<String, Vec<&'static str>> {
    let mut badges: HashMap<String, Vec<&'static str>> = HashMap::new();
    for todo in todos {
        let Some(work) = todo.work_id() else {
            continue;
        };
        let kinds = badges.entry(work.to_owned()).or_default();
        if !kinds.contains(&todo.badge()) {
            kinds.push(todo.badge());
        }
    }
    badges
}

#[derive(Debug, Serialize)]
pub struct TodoList {
    pub needs: Vec<Todo>,
    pub count: usize,
}

/// The to-dos that need the person: `auth` before `receive_failed`, that
/// before `replacement`, and that before the `회차 확인 필요` kinds
/// (`episode_check`, `placement_check`, `video_check`), each group newest
/// first.
pub async fn list(sources: &Sources<'_>) -> Result<TodoList, TodoError> {
    let auth = gather::auth(sources).await?;
    let failed = receive_failed::todos(sources).await?;
    let replacements = gather::replacements(sources).await?;
    // `회차 확인 필요` is a question on top of the others: when it cannot be
    // read, the rest of the list (and the badge) still answers.
    let mut checks = match gather::episode_checks(sources).await {
        Ok(checks) => checks,
        Err(e) => {
            eprintln!("Cannot read the 회차 확인 필요 to-dos: {e:?}");
            Vec::new()
        }
    };
    checks.extend(gather::placement_checks(sources).await?);
    // So is a video's: the library's read failing leaves the rest answering.
    match gather::video_checks(sources).await {
        Ok(videos) => checks.extend(videos),
        Err(e) => eprintln!("Cannot read the videos' 회차 확인 필요 to-dos: {e:?}"),
    }
    Ok(ordered(auth, failed, replacements, checks))
}

/// The to-dos of each group, newest first, the groups one after the other:
/// `auth`, `receive_failed`, `replacement`, then the `회차 확인 필요` kinds.
fn ordered(
    mut auth: Vec<Todo>,
    mut failed: Vec<Todo>,
    mut replacements: Vec<Todo>,
    mut checks: Vec<Todo>,
) -> TodoList {
    auth.sort_by_key(|t| std::cmp::Reverse(t.at()));
    failed.sort_by_key(|t| std::cmp::Reverse(t.at()));
    replacements.sort_by_key(|t| std::cmp::Reverse(t.at()));
    checks.sort_by_key(|t| std::cmp::Reverse(t.at()));
    auth.extend(failed);
    auth.extend(replacements);
    auth.extend(checks);
    TodoList {
        count: auth.len(),
        needs: auth,
    }
}

#[cfg(test)]
mod tests;
