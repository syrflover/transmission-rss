//! `receive_past`, shown on the screen as `받기` of a past episode search
//! (`docs/specs/collection.md`, 지난 회차 검색): adds one result of a search
//! the person confirmed.
//!
//! The web names the result by what history would store for it: its identity
//! key, its title and its link with the channel's secret values masked
//! ([`ReceivePast`]); the browser never supplies them. The worker:
//!
//! 1. checks the rule the way `다시 받기` of a past item does
//!    ([`receive_once::adoption_plan`]): the rule exists, is active, belongs to
//!    the channel and picks the title. A refusal ends the command and writes
//!    **nothing** to history: a search leaves only the items that were received.
//!    An item history says Transmission took is refused as held, unless it has
//!    gone from the work ([`departed`]): its torrent is no longer in
//!    Transmission and the work folder holds no video of its episode. The
//!    screen offers exactly those again (`past_search::world::departed`), and
//!    such an item is received once more, keeping its `received` result;
//! 2. records the item in history as a past item (`no_match`, as an item the
//!    channel's feed showed and no rule received), so the receive below is the
//!    one `다시 받기` makes;
//! 3. decides a revision of a video the folder holds ([`revisions`]): a result
//!    the person chose is the confirmation for a `버전 미상` one, as `다시 받기`
//!    is, and a revision the folder holds already is not added;
//! 4. receives the item as `receive_once` does ([`receive_once::execute_with`],
//!    without deciding the rule's episode conversion from this item: the
//!    person confirmed a range that was shown with the one the rule has), and
//!    renames it ([`receive_once::finish`]) unless it replaces a video.
//!
//! The result lands on the history item and on the command like `receive_once`'s:
//! `received`, `duplicate` or `add_failed`. An item whose add failed stays in
//! history as `add_failed` with the rule on it, so `다시 받기` repeats it.
//!
//! A start after one whose add got no answer that ends before it adds anything
//! (the rule was paused, the folder holds the revision) ends as `receive_once`'s
//! does ([`receive_once::end_early`]): the unanswered add stays recorded on the
//! command, so the next cycle removes no departed torrents, and the command's
//! label comes off the torrent.
//!
//! The step that reads the revisions' records is the one place that knows how
//! [`revisions`] keeps them ([`decide_revision`]).

use std::{collections::HashSet, path::Path};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use transmission_rpc::types::TorrentGetField;

use super::receive_once::{self, failed, held, Finished, NotRetryable, Retry, Settle};
use crate::{
    past_search::{release::Episode, world},
    store::{
        channels::{Channel, Rule},
        commands::{Command, CommandState, Outcome},
        history::{HistoryItem, HistoryResult, Millis, Observation},
        revisions::{HistoryWrite, NewRevision, RevisionState, RowWrite},
        status::TorrentListing,
    },
    worker::{
        plan::rule_destination,
        revisions::{self, Decided, Listing, Plan, Replaced, Selected},
        CycleContext,
    },
};

/// The `kind` of the command.
pub const KIND: &str = "receive_past";

/// The content of a `receive_past` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceivePast {
    /// The rule that receives the result.
    pub rule_id: String,
    /// The result's identity key.
    pub key: String,
    /// The title as history stores it.
    pub title: String,
    /// The link as history stores it, secret values masked.
    pub link: String,
}

impl ReceivePast {
    /// The text stored with the command and compared to tell a repeat from a
    /// different request.
    pub fn canonical(&self) -> String {
        serde_json::to_string(self).expect("a payload serializes")
    }

    /// What the command is about: one result of one rule.
    pub fn subject(&self) -> String {
        format!("{}:{}", self.rule_id, self.key)
    }
}

fn store(err: impl std::fmt::Display) -> Retry {
    Retry::Store(err.to_string())
}

/// Runs a `receive_past` command to its end (see the module docs).
pub async fn run(
    ctx: &CycleContext,
    command: &Command,
    now: impl Fn() -> Millis,
    cancel: &CancellationToken,
) -> Result<Finished, Retry> {
    let Ok(payload) = serde_json::from_str::<ReceivePast>(&command.payload) else {
        return ended(
            ctx,
            command,
            failed("요청 내용을 읽지 못했어요.", None),
            cancel,
        )
        .await;
    };
    let rule = ctx
        .channels
        .get_rule(&payload.rule_id)
        .await
        .map_err(store)?;
    let channel = match &rule {
        Some(rule) => ctx
            .channels
            .get_channel(&rule.channel_id)
            .await
            .map_err(store)?,
        None => None,
    };
    let existing = match &channel {
        Some(channel) => ctx
            .history
            .item_by_key(channel.id.clone(), payload.key.clone())
            .await
            .map_err(store)?,
        None => None,
    };

    // The same checks as `다시 받기` of a past item, on the item as it would be
    // recorded, before anything is written.
    let probe = existing
        .clone()
        .unwrap_or_else(|| past_item(&payload, channel.as_ref().map_or("", |c| c.id.as_str()), 0));
    let gone = match (&existing, &rule) {
        (Some(item), Some(rule)) if item.result.is_settled() => {
            match ctx.settings.collection().await.map_err(store)? {
                Some(collect) => {
                    let (save_path, offset) = rule_destination(Path::new(&collect.folder), rule);
                    departed(ctx, item, &save_path, offset as i64).await?
                }
                None => false,
            }
        }
        _ => false,
    };
    // A departed item is checked as one nothing has received.
    let unsettled = HistoryItem {
        result: if gone {
            HistoryResult::NoMatch
        } else {
            probe.result
        },
        ..probe.clone()
    };
    match receive_once::adoption_plan(&unsettled, channel.as_ref(), rule.as_ref()) {
        Ok(_) => {}
        Err(NotRetryable::Held) => {
            return ended(ctx, command, held(probe.result, None), cancel).await
        }
        Err(reason) => return ended(ctx, command, failed(reason.message(), None), cancel).await,
    }
    let (Some(rule), Some(channel)) = (rule, channel) else {
        let finished = failed(NotRetryable::RuleMissing.message(), None);
        return ended(ctx, command, finished, cancel).await;
    };
    let Some(collect) = ctx.settings.collection().await.map_err(store)? else {
        let finished = failed(receive_once::NO_COLLECT_FOLDER, None);
        return ended(ctx, command, finished, cancel).await;
    };
    let (save_path, episode) = rule_destination(Path::new(&collect.folder), &rule);

    let mut item = match existing {
        Some(item) => item,
        None => record(ctx, &payload, &channel, now()).await?,
    };

    let mut replacing = None;
    if !gone
        && matches!(
            item.result,
            HistoryResult::NoMatch | HistoryResult::Excluded | HistoryResult::AddFailed
        )
    {
        match decide_revision(ctx, &item, &rule, &save_path, episode, &now).await? {
            Revision::Normal => {}
            Revision::Replace(decided) => replacing = Some(decided),
            Revision::Unknown => {
                item = ctx
                    .history
                    .get(item.id)
                    .await
                    .map_err(store)?
                    .unwrap_or(item);
            }
            Revision::Held(finished) => return ended(ctx, command, finished, cancel).await,
        }
    }

    // `execute` reads the request as a `receive_once` of this item by this rule.
    let delegated = Command {
        payload: receive_once::ReceiveOnce::by_rule(item.id, rule.id.clone()).canonical(),
        ..command.clone()
    };
    // A revision that replaces the folder's video starts its replacement
    // with the item's result, and keeps its received name until the old
    // video is gone.
    let replacing = replacing.map(|decided| {
        new_revision(
            &item,
            &rule,
            &save_path,
            decided,
            RevisionState::Receiving,
            None,
            None,
        )
    });
    let finished =
        receive_once::execute_with(ctx, &delegated, &now, Settle::Keep, replacing, gone).await?;
    receive_once::finish(ctx, &delegated, finished, cancel).await
}

/// Whether `item`, which history says Transmission took, has gone from the
/// work: [`world::departed`] with Transmission's torrents and the folder as
/// they are now.
///
/// What cannot be found out leaves the item held, as it is for the screen: an
/// item without a torrent hash, or a folder that cannot be read. A Transmission
/// that cannot be asked leaves the command to the next look, as the add would
/// fail all the same.
async fn departed(
    ctx: &CycleContext,
    item: &HistoryItem,
    save_path: &Path,
    offset: i64,
) -> Result<bool, Retry> {
    if item.torrent_hash.is_none() {
        return Ok(false);
    }
    let mut transmission = ctx.transmission();
    let listed = transmission
        .torrent_get(Some(vec![TorrentGetField::HashString]), None)
        .await
        .map_err(|err| {
            Retry::Store(format!(
                "cannot list the torrents in Transmission: {}",
                ctx.redactor.apply(&err.to_string())
            ))
        })?;
    // The list is of now: every item that has a result is older than it.
    let listing = TorrentListing {
        taken_at: Millis::MAX,
        hashes: listed
            .arguments
            .torrents
            .into_iter()
            .filter_map(|torrent| torrent.hash_string)
            .map(|hash| hash.to_ascii_lowercase())
            .collect(),
    };
    let folder = save_path.to_owned();
    let files = tokio::task::spawn_blocking(move || world::read_folder(&folder))
        .await
        .map_err(|err| Retry::Store(err.to_string()))?;
    let files = match files {
        Ok(files) => files,
        Err(err) => {
            eprintln!(
                "Cannot read the folder {} to tell whether item {} is gone: {err}",
                save_path.display(),
                item.id
            );
            return Ok(false);
        }
    };
    let in_folder: HashSet<Episode> = files.into_iter().map(|(episode, _)| episode).collect();
    Ok(world::departed(item, offset, &in_folder, Some(&listing)))
}

/// A command that ends before anything is added ([`receive_once::end_early`]):
/// an earlier start's add that got no answer stays recorded, so the next cycle
/// removes no departed torrents, and the command's label comes off.
async fn ended(
    ctx: &CycleContext,
    command: &Command,
    finished: Finished,
    cancel: &CancellationToken,
) -> Result<Finished, Retry> {
    let finished = receive_once::end_early(ctx, command, finished);
    receive_once::finish(ctx, command, finished, cancel).await
}

/// The history item a result would be.
pub fn past_item(payload: &ReceivePast, channel_id: &str, id: i64) -> HistoryItem {
    HistoryItem {
        id,
        channel_id: channel_id.to_owned(),
        channel_label: String::new(),
        identity_key: payload.key.clone(),
        title: payload.title.clone(),
        link: payload.link.clone(),
        first_seen_at: 0,
        last_seen_at: 0,
        result: HistoryResult::NoMatch,
        result_at: 0,
        rule_id: None,
        reason: None,
        torrent_hash: None,
    }
}

/// Records the result as a past item and returns it.
async fn record(
    ctx: &CycleContext,
    payload: &ReceivePast,
    channel: &Channel,
    at: Millis,
) -> Result<HistoryItem, Retry> {
    ctx.history
        .record(
            at,
            vec![Observation {
                channel_id: channel.id.clone(),
                channel_label: channel.masked_url(),
                identity_key: payload.key.clone(),
                title: payload.title.clone(),
                link: payload.link.clone(),
                result: HistoryResult::NoMatch,
                rule_id: None,
                torrent_hash: None,
                reason: None,
            }],
        )
        .await
        .map_err(store)?;
    ctx.history
        .item_by_key(channel.id.clone(), payload.key.clone())
        .await
        .map_err(store)?
        .ok_or_else(|| Retry::Store("the recorded item cannot be found".to_owned()))
}

/// What the revision step decided.
enum Revision {
    /// Not a revision of a video the folder holds: received as always.
    Normal,
    /// Replaces the folder's video: received without its rename, then the
    /// replacement goes on.
    Replace(Decided),
    /// `버전 미상`, now recorded as such: receiving it is the confirmation.
    Unknown,
    /// The folder holds this revision already; the command ends.
    Held(Finished),
}

fn new_revision(
    item: &HistoryItem,
    rule: &Rule,
    save_path: &Path,
    decided: Decided,
    state: RevisionState,
    reason: Option<String>,
    hash: Option<String>,
) -> NewRevision {
    NewRevision {
        item_id: item.id,
        old_item_id: decided.old_item_id,
        rule_id: rule.id.clone(),
        folder: save_path.to_string_lossy().into_owned(),
        episode_name: decided.episode_name,
        old_version: decided.old_version,
        new_version: decided.version,
        expected_crc: decided.crc,
        old_crc: decided.old_crc,
        torrent_hash: hash,
        state,
        reason,
    }
}

/// Decides a result that is a revision, with the worker's own judgment
/// ([`revisions::plan`]) and records what a revision that is not added needs.
async fn decide_revision(
    ctx: &CycleContext,
    item: &HistoryItem,
    rule: &Rule,
    save_path: &Path,
    episode: isize,
    now: &impl Fn() -> Millis,
) -> Result<Revision, Retry> {
    // A lower revision of a release that replaced, or is replacing, the
    // folder's video is not received there again, as a cycle withholds it.
    let replaced = Replaced::new(ctx.revisions.replacements().await.map_err(store)?);
    if replaced.holds_higher(save_path, &item.title) {
        ctx.history
            .record_outcome(
                item.id,
                now(),
                HistoryResult::Duplicate,
                Some(rule.id.clone()),
                Some(revisions::NOT_HIGHER.to_owned()),
                None,
            )
            .await
            .map_err(store)?;
        return Ok(Revision::Held(duplicate(revisions::NOT_HIGHER)));
    }
    if !revisions::is_revision(&item.title) {
        return Ok(Revision::Normal);
    }
    let plan = revisions::plan(
        ctx,
        &Selected {
            channel_id: &item.channel_id,
            identity_key: &item.identity_key,
            title: &item.title,
            save_path,
            episode,
        },
        &Listing::new(),
    )
    .await;
    match plan {
        Plan::Normal => Ok(Revision::Normal),
        Plan::Replace(decided) => Ok(Revision::Replace(decided)),
        Plan::Later(why) => Err(Retry::Store(why)),
        Plan::Unknown(decided, reason) => {
            // The person chose this result, which is what `다시 받기` of a
            // `버전 미상` item asks: the item is recorded as such, with its
            // decision, and received.
            let row = new_revision(
                item,
                rule,
                save_path,
                decided,
                RevisionState::Unknown,
                Some(reason.to_owned()),
                None,
            );
            withhold(
                ctx,
                item,
                rule,
                now(),
                HistoryResult::VersionUnknown,
                reason,
                row,
            )
            .await?;
            Ok(Revision::Unknown)
        }
        Plan::Skip(decided, reason) => {
            let row = new_revision(
                item,
                rule,
                save_path,
                decided,
                RevisionState::Skipped,
                Some(reason.to_owned()),
                None,
            );
            withhold(
                ctx,
                item,
                rule,
                now(),
                HistoryResult::Duplicate,
                reason,
                row,
            )
            .await?;
            Ok(Revision::Held(duplicate(reason)))
        }
    }
}

/// Records `result` with `reason` on `item` and the revision decision `row`
/// that says why, together.
async fn withhold(
    ctx: &CycleContext,
    item: &HistoryItem,
    rule: &Rule,
    at: Millis,
    result: HistoryResult,
    reason: &str,
    row: NewRevision,
) -> Result<(), Retry> {
    ctx.revisions
        .write_with_history(
            at,
            HistoryWrite::Outcome {
                item_id: item.id,
                result,
                rule_id: Some(rule.id.clone()),
                reason: Some(reason.to_owned()),
                torrent_hash: None,
            },
            RowWrite::Create {
                new: row,
                reopen: false,
            },
        )
        .await
        .map_err(store)?;
    Ok(())
}

/// A command that ends `duplicate` because the folder holds the revision.
fn duplicate(reason: &str) -> Finished {
    Finished {
        state: CommandState::Done,
        outcome: Outcome {
            result: HistoryResult::Duplicate.code().to_owned(),
            reason: Some(reason.to_owned()),
        },
        rename: None,
        unlabel: None,
        add_unconfirmed: false,
    }
}
