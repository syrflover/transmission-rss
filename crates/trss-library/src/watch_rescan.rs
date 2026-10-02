//! `watch_rescan`, shown on the screen as `다시 확인` of a watch folder: the
//! worker reads the folder again now instead of waiting for the next cycle.
//!
//! The web accepts the command with a [`WatchRescan`] payload (the folder's ID;
//! one folder has at most one open rescan) and the worker runs it, reading the
//! folder with the same scan the cycles use ([`crate::watch`]). The scan only
//! reads the disk. The worker takes the folder's turn for it first
//! ([`section`]), as for every reading of a watch folder; [`run`] does not.
//!
//! The command ends `done` when the folder was read, and `failed` with the
//! reason when it could not be (the reason is also on the folder's row). A
//! folder unregistered meanwhile ends it `failed` too. A repeat of a command
//! already run is answered by the web from the stored command, so the folder is
//! read once per command ID.

use serde::{Deserialize, Serialize};

use trss_core::{
    commands::{Command, CommandState, Outcome},
    folder_locks::Section,
    Clock,
};

use crate::watch::{self, WatchContext};

/// The `kind` of the command.
pub const KIND: &str = "watch_rescan";

/// The outcome's `result` when the folder was read.
pub const SCANNED: &str = "scanned";
/// The outcome's `result` of a command that ended `failed`.
pub const FAILED: &str = "failed";

/// The content of a `watch_rescan` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchRescan {
    pub folder_id: String,
}

impl WatchRescan {
    /// The canonical text stored with the command and compared to tell a
    /// repeat of a request from a different one.
    pub fn canonical(&self) -> String {
        serde_json::to_string(self).expect("a payload serializes")
    }

    /// The subject stored with the command: the folder.
    pub fn subject(&self) -> String {
        self.folder_id.clone()
    }
}

/// How an executed command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub state: CommandState,
    pub outcome: Outcome,
}

/// A command that could not be carried through now and stays `running` for the
/// next look: the database failed.
#[derive(Debug, thiserror::Error)]
#[error("cannot read or write the app database: {0}")]
pub struct Retry(String);

fn failed(reason: impl Into<String>) -> Finished {
    Finished {
        state: CommandState::Failed,
        outcome: Outcome {
            result: FAILED.to_owned(),
            reason: Some(reason.into()),
        },
    }
}

/// The turn the command takes before it runs: a reading of its folder (see
/// [`watch::reading_section`]). Empty when the request or the folder cannot be
/// found: the command then ends by itself.
pub async fn section(ctx: &WatchContext, command: &Command) -> Result<Section, Retry> {
    let Ok(payload) = serde_json::from_str::<WatchRescan>(&command.payload) else {
        return Ok(Section::new());
    };
    let folder = ctx
        .library
        .folder(&payload.folder_id)
        .await
        .map_err(|e| Retry(e.to_string()))?;
    Ok(folder.map_or_else(Section::new, |folder| {
        watch::reading_section(&folder.path, &[])
    }))
}

/// Runs a `watch_rescan` command to its end.
pub async fn run(ctx: &WatchContext, command: &Command, clock: &Clock) -> Result<Finished, Retry> {
    let Ok(payload) = serde_json::from_str::<WatchRescan>(&command.payload) else {
        return Ok(failed("요청 내용을 읽지 못했어요."));
    };
    let Some(folder) = ctx
        .library
        .folder(&payload.folder_id)
        .await
        .map_err(|e| Retry(e.to_string()))?
    else {
        return Ok(failed(
            "감시 폴더를 찾지 못했어요. 등록이 해제됐을 수 있어요.",
        ));
    };
    match watch::scan_folder(ctx, &folder, clock(), watch::ScanMode::Full)
        .await
        .map_err(|e| Retry(e.to_string()))?
    {
        watch::Scanned::Read(report) => Ok(Finished {
            state: CommandState::Done,
            outcome: Outcome {
                result: SCANNED.to_owned(),
                // The work folders a scan could not read are told on the folder's row.
                reason: report.error,
            },
        }),
        watch::Scanned::Failed(reason) => Ok(failed(reason)),
        watch::Scanned::Gone => Ok(failed(
            "감시 폴더를 찾지 못했어요. 등록이 해제됐을 수 있어요.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_is_the_folder_and_nothing_else() {
        let payload = WatchRescan {
            folder_id: "f1".into(),
        };
        assert_eq!(payload.canonical(), r#"{"folder_id":"f1"}"#);
        assert_eq!(payload.subject(), "f1");
        assert!(serde_json::from_str::<WatchRescan>(r#"{"folder_id":"f1","x":1}"#).is_err());
    }
}
