//! `receive_once` (한 번 받기): receive one history item without a rule.
//!
//! The web accepts the command with a [`ReceiveOnce`] payload; the worker runs
//! it with [`execute`]:
//!
//! 1. find the history item and its channel, and resolve the save folder below
//!    the channel's base folder ([`folder::resolve`], checked again here
//!    because the request may be old);
//! 2. recover the item's original link ([`link::recover`]);
//! 3. add it to Transmission, save it as the item's result and end the command;
//! 4. when that add put the torrent in, give the file its `trname` name
//!    without any episode conversion. A torrent Transmission already had (a
//!    rule's, or this command's own from a run that died before its result was
//!    written) is not renamed.
//!
//! The result lands on the history item (`received`, `duplicate` or
//! `add_failed` with a reason) and on the command, which reports the item's
//! result as history holds it afterwards. Only steps 1 to 3 decide the result. A rename that does not happen leaves the torrent and its data under
//! its own name (a person chose to receive this item, so it is never removed as
//! the rule path does when `trname` has no name), and the history item gets a
//! note saying so ([`RenameResult::Kept`]).

use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use transmission_rpc::types::Id;
use trname::trname;

use super::{folder, link};
use crate::{
    store::{
        channels::{Channel, ChannelWithRules},
        commands::{Command, CommandState, Outcome},
        history::{HistoryItem, HistoryResult, Millis},
    },
    transmission::{self, add_item, get_torrent, AddError, AddKind, Redactor},
    worker::{plan::ChannelPlan, CycleContext},
};

/// The `kind` of the command.
pub const KIND: &str = "receive_once";

/// Longest failure reason kept, in characters.
const MAX_REASON_CHARS: usize = 300;

/// What `trname` is asked for: the file keeps the episode number it has. A
/// one-off receive changes no numbering (no `episode` offset of a rule applies).
/// `trname` adds `starts_episode_at - 1` for positive values and shifts by the
/// value for negative ones, so both `0` and `1` leave the number as it is.
pub const NO_EPISODE_CONVERSION: isize = 0;

/// The content of a `receive_once` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiveOnce {
    /// The history item to receive.
    pub item_id: i64,
    /// The save folder below the channel's base folder; empty is the base
    /// folder itself.
    #[serde(default)]
    pub folder: String,
}

impl ReceiveOnce {
    /// The canonical text stored with the command and compared to tell a
    /// repeat of a request from a different one: same fields, same order, and
    /// the folder trimmed.
    pub fn canonical(&self) -> String {
        serde_json::to_string(&ReceiveOnce {
            item_id: self.item_id,
            folder: self.folder.trim().to_owned(),
        })
        .expect("a payload serializes")
    }

    /// The subject stored with the command: what it is about.
    pub fn subject(&self) -> String {
        self.item_id.to_string()
    }
}

/// Whether the screen may offer `한 번 받기` for an item with this result.
/// An item Transmission already holds (`received`, `duplicate`) has nothing to
/// receive.
pub fn can_receive(result: HistoryResult) -> bool {
    !result.is_settled()
}

/// How an executed command ended.
#[derive(Debug, Clone)]
pub struct Finished {
    pub state: CommandState,
    pub outcome: Outcome,
    /// Set when this command's add put the torrent in (Transmission did not
    /// have it), so its file is to be renamed.
    pub rename: Option<Rename>,
}

/// What the renaming step needs.
#[derive(Debug, Clone)]
pub struct Rename {
    /// The history item to note on when the name stays as it was.
    pub item_id: i64,
    pub hash: String,
    pub save_path: std::path::PathBuf,
    pub redactor: Redactor,
}

/// A command that could not be carried out or recorded and should be tried
/// again later (the database failed).
#[derive(Debug, thiserror::Error)]
pub enum Retry {
    #[error("cannot read or write the app database: {0}")]
    Store(String),
}

impl Retry {
    fn store(err: impl std::fmt::Display) -> Retry {
        Retry::Store(err.to_string())
    }
}

/// Runs a `receive_once` command up to the point where its outcome is known and
/// recorded on the history item. The caller ends the command with the returned
/// [`Finished`], then calls [`rename`].
pub async fn execute(
    ctx: &CycleContext,
    command: &Command,
    now: impl Fn() -> Millis,
) -> Result<Finished, Retry> {
    let Ok(payload) = serde_json::from_str::<ReceiveOnce>(&command.payload) else {
        return Ok(failed("요청 내용을 읽지 못했어요.", None));
    };

    let Some(item) = ctx
        .history
        .get(payload.item_id)
        .await
        .map_err(Retry::store)?
    else {
        return Ok(failed("기록에서 이 항목을 찾지 못했어요.", None));
    };
    let channel = ctx
        .channels
        .get_channel(&item.channel_id)
        .await
        .map_err(Retry::store)?;
    let Some(channel) = channel else {
        return refuse(
            ctx,
            &item,
            "채널이 삭제돼서 저장 폴더와 원래 링크를 정할 수 없어요.",
            &now,
        )
        .await;
    };

    let save_path = match folder::resolve(&channel.base_dir, &payload.folder) {
        Ok(path) => path,
        Err(err) => return refuse(ctx, &item, err.message(), &now).await,
    };

    let redactor = redactor_for(ctx, &channel);
    let raw_link = match link::recover(&item, &channel, &ctx.http, &redactor).await {
        Ok(raw) => raw,
        Err(reason) => return refuse(ctx, &item, &reason, &now).await,
    };
    // The recovered link is a secret from here on, whatever it carried.
    let mut redactor = redactor;
    redactor.add(&raw_link);
    if let Ok(parsed) = url::Url::parse(&raw_link) {
        for (_, value) in parsed.query_pairs() {
            redactor.add_query_value(&value);
        }
    }

    let mut transmission =
        transmission::client(ctx.transmission_url.clone(), &ctx.transmission_http);
    let added = add_item(&mut transmission, &raw_link, &save_path, &redactor).await;

    match added {
        Ok(torrent) => {
            let result = match torrent.kind {
                AddKind::Added => HistoryResult::Received,
                AddKind::Duplicate => HistoryResult::Duplicate,
            };
            let stored = ctx
                .history
                .record_outcome(item.id, now(), result, None, Some(torrent.hash.clone()))
                .await
                .map_err(Retry::store)?
                .unwrap_or(result);
            // Only a torrent this command put in is renamed. One that was there
            // already (a rule's, or this command's own from a run that died
            // before its result was written) keeps its name and gets no note.
            let rename = (torrent.kind == AddKind::Added).then_some(Rename {
                item_id: item.id,
                hash: torrent.hash,
                save_path,
                redactor,
            });
            Ok(held(stored, rename))
        }
        Err(err) => {
            let reason = add_failure_reason(&err, &redactor);
            eprintln!(
                "Cannot add item {} of {}: {reason}",
                item.id, item.channel_label
            );
            refuse(ctx, &item, &reason, &now).await
        }
    }
}

/// Why a command ended `duplicate`.
const ALREADY_THERE: &str = "Transmission에 이미 같은 토렌트가 있어서 새로 받지 않았어요.";

/// A command that ended with Transmission holding the item's torrent. The
/// outcome follows `stored`, the item's result in history afterwards, which
/// may be a rule's `received` rather than what this command's add answered.
fn held(stored: HistoryResult, rename: Option<Rename>) -> Finished {
    Finished {
        state: CommandState::Done,
        outcome: Outcome {
            result: stored.code().to_owned(),
            reason: (stored == HistoryResult::Duplicate).then(|| ALREADY_THERE.to_owned()),
        },
        rename,
    }
}

/// Records `add_failed` with `reason` on the item and returns the failed command.
async fn refuse(
    ctx: &CycleContext,
    item: &HistoryItem,
    reason: &str,
    now: &impl Fn() -> Millis,
) -> Result<Finished, Retry> {
    let reason: String = reason.chars().take(MAX_REASON_CHARS).collect();
    let stored = ctx
        .history
        .record_outcome(
            item.id,
            now(),
            HistoryResult::AddFailed,
            Some(reason.clone()),
            None,
        )
        .await
        .map_err(Retry::store)?
        .unwrap_or(HistoryResult::AddFailed);
    Ok(Finished {
        state: CommandState::Failed,
        outcome: Outcome {
            result: stored.code().to_owned(),
            reason: Some(reason),
        },
        rename: None,
    })
}

/// A failed command that has no history item to write to.
fn failed(reason: &str, rename: Option<Rename>) -> Finished {
    Finished {
        state: CommandState::Failed,
        outcome: Outcome {
            result: HistoryResult::AddFailed.code().to_owned(),
            reason: Some(reason.to_owned()),
        },
        rename,
    }
}

/// Knows the channel's secret values and the Transmission credentials.
fn redactor_for(ctx: &CycleContext, channel: &Channel) -> Redactor {
    let mut redactor = ctx.redactor.clone();
    let plan = ChannelPlan::new(ChannelWithRules {
        channel: channel.clone(),
        rules: Vec::new(),
    });
    redactor.extend(&plan.redactor());
    redactor
}

fn add_failure_reason(err: &AddError, redactor: &Redactor) -> String {
    let text = match err {
        AddError::Rpc(err) => {
            format!("Transmission에 연결하지 못했거나 응답이 없어요: {err}")
        }
        AddError::Rejected(result) => format!("Transmission이 토렌트를 받지 않았어요: {result}"),
    };
    redactor
        .apply(&text)
        .chars()
        .take(MAX_REASON_CHARS)
        .collect()
}

/// What [`rename`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameResult {
    /// The file has the `trname` name now.
    Renamed,
    /// Nothing to do: the name was already right, the torrent is gone, or
    /// shutdown was asked for.
    Unchanged,
    /// The file keeps its original name; the note says why, for the history item.
    Kept(&'static str),
}

/// The file's name gave `trname` no title and episode to work with.
pub const NAME_NOT_DERIVED: &str =
    "파일 이름에서 작품과 회차를 알아내지 못해서 원래 이름 그대로 뒀어요.";
/// Renaming was tried and did not go through.
pub const NAME_NOT_CHANGED: &str = "이름을 바꾸지 못해서 원래 이름 그대로 뒀어요.";

/// Gives the torrent's single file its `trname` name for the folder it was
/// saved in, without any episode conversion.
///
/// Unlike the renaming after a rule's add, a torrent whose name cannot be
/// derived is left alone: that path removes the torrent and its data, which is
/// no answer to a person who chose to receive this item (the base folder, for
/// one, has no title and season parts to name a file after). The result says
/// when the original name was kept, so the caller can note it on the history
/// item. Attempts follow `ctx.rename`, as a magnet link's file name is only
/// known once Transmission has its metadata.
pub async fn rename(
    ctx: &CycleContext,
    rename: &Rename,
    cancel: &CancellationToken,
) -> RenameResult {
    let mut transmission =
        transmission::client(ctx.transmission_url.clone(), &ctx.transmission_http);
    for _ in 0..ctx.rename.attempts {
        tokio::select! {
            _ = tokio::time::sleep(ctx.rename.delay) => {}
            _ = cancel.cancelled() => return RenameResult::Unchanged,
        }

        let torrent = match get_torrent(&mut transmission, &rename.hash).await {
            Ok(Some(torrent)) => torrent,
            Ok(None) => return RenameResult::Unchanged,
            Err(err) => {
                println!("{}", rename.redactor.apply(&err.to_string()));
                continue;
            }
        };
        if torrent.file_count != Some(1) {
            continue;
        }
        let Some(old_name) = torrent.name else {
            continue;
        };
        let Some(new_name) = derived_name(&rename.save_path, &old_name) else {
            return RenameResult::Kept(NAME_NOT_DERIVED);
        };
        if new_name == old_name {
            return RenameResult::Unchanged;
        }
        match transmission
            .torrent_rename_path(vec![Id::Hash(rename.hash.clone())], old_name, new_name)
            .await
        {
            Ok(response) if response.result == "success" => return RenameResult::Renamed,
            Ok(_) => {}
            Err(err) => println!("{}", rename.redactor.apply(&err.to_string())),
        }
    }
    RenameResult::Kept(NAME_NOT_CHANGED)
}

/// The name `trname` gives `file_name` in `save_path`, with no episode conversion.
pub fn derived_name(save_path: &Path, file_name: &str) -> Option<String> {
    trname(save_path, file_name, NO_EPISODE_CONVERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_canonical_payload_ignores_spacing_but_not_content() {
        let a = ReceiveOnce {
            item_id: 7,
            folder: " LIAR GAME/Season 01 ".into(),
        };
        let b = ReceiveOnce {
            item_id: 7,
            folder: "LIAR GAME/Season 01".into(),
        };
        let c = ReceiveOnce {
            item_id: 8,
            folder: "LIAR GAME/Season 01".into(),
        };
        assert_eq!(a.canonical(), b.canonical());
        assert_ne!(a.canonical(), c.canonical());
        assert_eq!(
            b.canonical(),
            r#"{"item_id":7,"folder":"LIAR GAME/Season 01"}"#
        );
    }

    #[test]
    fn a_payload_with_an_unknown_field_is_not_read() {
        assert!(serde_json::from_str::<ReceiveOnce>(r#"{"item_id":7,"link":"x"}"#).is_err());
        assert!(serde_json::from_str::<ReceiveOnce>(r#"{"folder":"x"}"#).is_err());
        assert_eq!(
            serde_json::from_str::<ReceiveOnce>(r#"{"item_id":7}"#)
                .unwrap()
                .folder,
            ""
        );
    }

    #[test]
    fn no_conversion_leaves_the_release_episode_as_it_is() {
        let folder = Path::new("/media/anime/LIAR GAME/Season 01");
        let release = "[SubsPlease] LIAR GAME - 26 (1080p) [ABCD1234].mkv";

        let expected = Some("LIAR GAME S01E26.mkv".to_owned());
        assert_eq!(derived_name(folder, release), expected);
        // 1 is the value the legacy configuration defaults to; it converts nothing either.
        assert_eq!(trname(folder, release, 1), expected);
        // A rule's offset would have moved it.
        assert_ne!(trname(folder, release, -12), expected);
    }

    #[test]
    fn a_folder_without_title_and_season_parts_gets_no_name() {
        assert_eq!(
            derived_name(
                Path::new("/media/anime"),
                "[SubsPlease] LIAR GAME - 26 (1080p) [ABCD1234].mkv"
            ),
            None
        );
    }

    #[test]
    fn only_items_transmission_does_not_hold_can_be_received() {
        assert!(can_receive(HistoryResult::NoMatch));
        assert!(can_receive(HistoryResult::Excluded));
        assert!(can_receive(HistoryResult::AddFailed));
        assert!(!can_receive(HistoryResult::Received));
        assert!(!can_receive(HistoryResult::Duplicate));
    }
}
