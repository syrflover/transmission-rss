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
    commands::{Command, CommandState, Outcome},
    folder_locks::Section,
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
}
