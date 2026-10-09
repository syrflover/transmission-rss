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
    use std::{fs, os::unix::fs::PermissionsExt};

    use super::*;
    use crate::watch::fixture::*;

    fn command(payload: &str) -> Command {
        Command {
            id: "rescan-0001".to_owned(),
            kind: KIND.to_owned(),
            payload: payload.to_owned(),
            subject: None,
            state: CommandState::Running,
            attempts: 1,
            created_at: 0,
            updated_at: 0,
            finished_at: None,
            outcome: None,
            add_unconfirmed: false,
            original_name: None,
        }
    }

    fn rescan_of(folder_id: &str) -> Command {
        command(
            &WatchRescan {
                folder_id: folder_id.to_owned(),
            }
            .canonical(),
        )
    }

    #[test]
    fn a_payload_is_the_folder_and_nothing_else() {
        let payload = WatchRescan {
            folder_id: "f1".into(),
        };
        assert_eq!(payload.canonical(), r#"{"folder_id":"f1"}"#);
        assert_eq!(payload.subject(), "f1");
        assert!(serde_json::from_str::<WatchRescan>(r#"{"folder_id":"f1","x":1}"#).is_err());
    }

    #[tokio::test]
    async fn a_folder_that_was_read_ends_the_command_done_and_picks_up_what_changed() {
        let fx = Fixture::new().await;
        let root = fx.folder("anime");
        lycoris(&root);
        let folder = fx.register(&root).await;
        let checked = fx.stored(&folder).await.checked_at;
        touch(&root.join("Brand New/Season 01/Brand New S01E01.mkv"));
        fx.advance(5000);

        let finished = run(&fx.ctx, &rescan_of(&folder.id), &fx.clock())
            .await
            .unwrap();

        assert_eq!(finished.state, CommandState::Done);
        assert_eq!(finished.outcome.result, SCANNED);
        assert_eq!(finished.outcome.reason, None);
        assert_eq!(fx.works(&folder).await.len(), 2);
        let stored = fx.stored(&folder).await;
        assert_eq!(stored.checked_at, Some(fx.now()));
        assert_ne!(stored.checked_at, checked);
    }

    #[tokio::test]
    async fn a_rescan_of_an_unreadable_folder_ends_failed_with_the_reason_and_keeps_the_records() {
        let fx = Fixture::new().await;
        let root = fx.folder("anime");
        lycoris(&root);
        let folder = fx.register(&root).await;
        let before = fx.works(&folder).await;

        fs::set_permissions(&root, fs::Permissions::from_mode(0o000)).unwrap();
        let finished = run(&fx.ctx, &rescan_of(&folder.id), &fx.clock()).await;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();

        let finished = finished.unwrap();
        assert_eq!(finished.state, CommandState::Failed);
        assert_eq!(finished.outcome.result, FAILED);
        assert!(finished.outcome.reason.unwrap().contains("권한"));
        assert_eq!(fx.works(&folder).await, before);
        assert!(fx.stored(&folder).await.error.unwrap().contains("권한"));
    }

    #[tokio::test]
    async fn a_folder_that_is_not_registered_or_a_payload_that_cannot_be_read_ends_failed() {
        let fx = Fixture::new().await;
        let root = fx.folder("anime");
        lycoris(&root);
        let folder = fx.register(&root).await;
        fx.ctx
            .library
            .remove_folder(&folder.id, fx.now())
            .await
            .unwrap();

        let gone = run(&fx.ctx, &rescan_of(&folder.id), &fx.clock())
            .await
            .unwrap();
        assert_eq!(gone.state, CommandState::Failed);
        assert!(gone
            .outcome
            .reason
            .unwrap()
            .contains("감시 폴더를 찾지 못했어요"));

        let unreadable = run(&fx.ctx, &command("{}"), &fx.clock()).await.unwrap();
        assert_eq!(unreadable.state, CommandState::Failed);
        assert!(unreadable
            .outcome
            .reason
            .unwrap()
            .contains("요청 내용을 읽지 못했어요"));
    }
}
