//! `anissia_captions`, shown on the screen as `새로고침` of a season's
//! `자막 후보` section, and made by the web itself when a season is linked to an
//! Anissia anime: the worker reads that anime's subtitle lines now
//! (`/anime/caption/animeNo/<n>`) and observes them like the 30-minute reading
//! of the recent list does ([`crate::anissia::captions`]), reaching the lines
//! that list does not hold.
//!
//! The payload is the Anissia anime's number; one anime has at most one open
//! command. It touches no folder and no torrent, so it takes no turn, and it
//! asks Anissia at the app's shared pace. The web accepts it only for an anime
//! some season is linked to, so the command is not a way to ask Anissia about
//! any anime.
//!
//! The command ends `done` (`read`) when Anissia answered and what it listed was
//! observed, whether or not anything was new, and `failed` with the reason when
//! Anissia did not answer or asked to wait: what a failed read reached is kept,
//! and asking again is the user's `새로고침`. A repeat of a command already run
//! is answered by the web from the stored command, so the anime is read once
//! per command ID.

use serde::{Deserialize, Serialize};

use trss_core::{
    commands::{Accepted, Command, CommandError, CommandState, CommandStore, NewCommand, Outcome},
    folder_locks::Section,
    Millis,
};

use crate::anissia::captions::{CaptionObserver, End, Read};

/// The `kind` of the command.
pub const KIND: &str = "anissia_captions";

/// The outcome's `result` when the anime's lines were read.
pub const READ: &str = "read";
/// The outcome's `result` of a command that ended `failed`.
pub const FAILED: &str = "failed";

/// The content of an `anissia_captions` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnissiaCaptions {
    /// Anissia's `animeNo`.
    pub anime_no: i64,
}

impl AnissiaCaptions {
    /// The canonical text stored with the command and compared to tell a
    /// repeat of a request from a different one.
    pub fn canonical(&self) -> String {
        serde_json::to_string(self).expect("a payload serializes")
    }

    /// The subject stored with the command: the anime.
    pub fn subject(&self) -> String {
        self.anime_no.to_string()
    }
}

/// Stores a command that makes the worker read anime `anime_no` now, with an
/// ID of its own (made by the web when a season is linked, and by the worker
/// when it connects a subscription to a season: no browser sent one). One anime
/// has at most one open read, so a read already waiting or running stands for
/// this one. `true` when a command was stored, so the caller can wake the
/// worker.
pub async fn ask(
    commands: &CommandStore,
    anime_no: i64,
    now: Millis,
) -> Result<bool, CommandError> {
    let payload = AnissiaCaptions { anime_no };
    let new = NewCommand {
        id: format!("captions-{anime_no}-{now}"),
        kind: KIND.to_owned(),
        payload: payload.canonical(),
        subject: Some(payload.subject()),
    };
    Ok(matches!(
        commands.accept(new, now).await?,
        Accepted::Created(_)
    ))
}

/// How an executed command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub state: CommandState,
    pub outcome: Outcome,
}

fn failed(reason: impl Into<String>) -> Finished {
    Finished {
        state: CommandState::Failed,
        outcome: Outcome {
            result: FAILED.to_owned(),
            reason: Some(reason.into()),
        },
    }
}

/// The turn the command takes before it runs: none, it works in no folder.
pub fn section() -> Section {
    Section::new()
}

/// Runs an `anissia_captions` command to its end.
pub async fn run(observer: &CaptionObserver, command: &Command) -> Finished {
    let Ok(payload) = serde_json::from_str::<AnissiaCaptions>(&command.payload) else {
        return failed("요청 내용을 읽지 못했어요.");
    };
    ended(observer.read_anime(payload.anime_no).await)
}

/// The command's end for how the reading came out.
fn ended(read: Read) -> Finished {
    match read.end {
        End::Complete => Finished {
            state: CommandState::Done,
            outcome: Outcome {
                result: READ.to_owned(),
                reason: None,
            },
        },
        End::Busy(_) => failed(
            "Anissia가 잠시 요청을 받지 않아요. 잠시 뒤에 다시 새로고침해 주세요. 30분마다 하는 읽기는 그대로 이어져요.",
        ),
        End::Failed(why) => {
            eprintln!("Anissia captions of an anime: {why}");
            failed("Anissia의 자막 정보를 읽지 못했어요. 잠시 뒤에 다시 새로고침해 주세요.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_is_the_anime_and_nothing_else() {
        let payload = AnissiaCaptions { anime_no: 3492 };
        assert_eq!(payload.canonical(), r#"{"anime_no":3492}"#);
        assert_eq!(payload.subject(), "3492");
        assert!(serde_json::from_str::<AnissiaCaptions>(r#"{"anime_no":1,"x":1}"#).is_err());
        assert!(serde_json::from_str::<AnissiaCaptions>(r#"{"anime_no":"1"}"#).is_err());
    }

    async fn commands() -> (tempfile::TempDir, CommandStore) {
        let dir = tempfile::tempdir().unwrap();
        let db = trss_core::Db::open(dir.path().join("app.db"))
            .await
            .unwrap();
        (dir, CommandStore::new(db))
    }

    /// One anime has at most one open read, so a read waiting or running stands
    /// for the next ask; another anime is independent, and once the read has
    /// ended a new one is stored.
    #[tokio::test]
    async fn an_anime_has_one_open_read_at_a_time() {
        let (_dir, commands) = commands().await;

        assert!(ask(&commands, 3320, 1_000).await.unwrap());
        let stored = commands.get("captions-3320-1000").await.unwrap().unwrap();
        assert_eq!(stored.kind, KIND);
        assert_eq!(stored.payload, r#"{"anime_no":3320}"#);
        assert_eq!(stored.subject.as_deref(), Some("3320"));
        assert_eq!(stored.state, CommandState::Pending);

        // Waiting, and then running: the open read stands for the ask.
        assert!(!ask(&commands, 3320, 1_100).await.unwrap());
        commands.claim_next(1_200).await.unwrap().unwrap();
        assert!(!ask(&commands, 3320, 1_300).await.unwrap());
        assert_eq!(commands.get("captions-3320-1100").await.unwrap(), None);
        assert!(ask(&commands, 3321, 1_400).await.unwrap());

        // Once it has ended, whatever way, the anime is read again.
        let outcome = Outcome {
            result: READ.to_owned(),
            reason: None,
        };
        commands
            .finish("captions-3320-1000", CommandState::Done, outcome, 1_500)
            .await
            .unwrap();
        assert!(ask(&commands, 3320, 1_600).await.unwrap());
        assert!(commands.get("captions-3320-1600").await.unwrap().is_some());
    }

    fn command(payload: &str) -> Command {
        Command {
            id: "refresh-0001".to_owned(),
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

    /// The command ends by how Anissia answered: `read` when the lines were
    /// observed, and `failed` with a sentence of its own when Anissia failed or
    /// asked to wait. What a failed read does not reach is left as it was.
    #[tokio::test]
    async fn the_command_ends_read_when_anissia_answered_and_failed_with_a_sentence_when_it_did_not(
    ) {
        use std::{sync::Arc, time::Duration};

        use serde_json::json;
        use trss_anissia::{fake::Fake, Anissia};

        use crate::store::anissia::AnissiaStore;

        let db = trss_core::Db::open_blocking(":memory:").unwrap();
        let fake = Fake::start().await;
        let clock: trss_core::Clock = Arc::new(|| 1_790_942_400_000);
        let anissia = Anissia::new(db.clone(), fake.config(), clock).with_spacing(Duration::ZERO);
        let store = AnissiaStore::new(db);
        let observer = CaptionObserver::new(anissia, store.clone());
        let refresh = command(r#"{"anime_no":3492}"#);
        fake.set_captions(
            3492,
            vec![json!({"episode": "3", "updDt": "2026-10-02 21:00:00",
                        "website": "https://a.test/3", "name": "에루샤"})],
        );

        let read = run(&observer, &refresh).await;
        assert_eq!(read.state, CommandState::Done);
        assert_eq!(read.outcome.result, READ);
        assert_eq!(read.outcome.reason, None);
        assert_eq!(fake.count("/anime/caption/animeNo/3492"), 1);
        assert_eq!(store.candidates(3492, Vec::new()).await.unwrap().len(), 1);

        // Anissia fails: the command says so, and the candidates stay as they were.
        fake.state.lock().unwrap().failing = 1;
        let failed = run(&observer, &refresh).await;
        assert_eq!(failed.state, CommandState::Failed);
        assert_eq!(failed.outcome.result, FAILED);
        assert!(failed.outcome.reason.unwrap().contains("읽지 못했어요"));
        assert_eq!(store.candidates(3492, Vec::new()).await.unwrap().len(), 1);

        // Anissia asks to wait: also a failed command, with a sentence of its own.
        {
            let mut state = fake.state.lock().unwrap();
            state.rate_limited = 1;
            state.retry_after = Some(120);
        }
        let busy = run(&observer, &refresh).await;
        assert_eq!(busy.state, CommandState::Failed);
        assert!(busy
            .outcome
            .reason
            .unwrap()
            .contains("잠시 요청을 받지 않아요"));
        assert_eq!(store.candidates(3492, Vec::new()).await.unwrap().len(), 1);

        // A payload that cannot be read asks Anissia nothing.
        let asked = fake.requests().len();
        let bad = run(&observer, &command(r#"{"anime_no":"x"}"#)).await;
        assert_eq!(bad.state, CommandState::Failed);
        assert!(bad
            .outcome
            .reason
            .unwrap()
            .contains("요청 내용을 읽지 못했어요"));
        assert_eq!(fake.requests().len(), asked);
    }
}
