//! `rule_archive_batch`, shown on the screen as `모두 보관`, `N개 보관` and
//! `보관` of the archive suggestions (`docs/requirements/collection.md`, 규칙
//! 보관): the rules the person chose, archived one after another by the server.
//!
//! The screen sends one `rule_archive_batch` command with the rules in order
//! ([`ArchiveBatch`]), and [`run`] carries it out in the commands' loop, so a
//! tab that stops listening (a hidden tab closes its event stream) or a closed
//! screen leaves nothing unsent.
//!
//! **A rule is archived as its detail's `보관` archives it**
//! ([`work_archive::archive_rule`]): the rule is deleted and the move of its
//! work folder is stored in one transaction, as a `work_archive` command with
//! the ID [`child_id`] (`{batch ID}-{index}`); a rule with no work folder is
//! only deleted. The batch rings the commands' loop so that the move starts at
//! once, and waits for it to end. Everything about the folder (whether it moves,
//! which it does with the last rule that collects into it, the retries, the
//! sentences) stays with that command; the batch only keeps the order and
//! stops at the first rule that does not end well. A rule edited between the
//! batch's read and its deletion is read again and archived as it is then.
//!
//! **Waiting for a move** ([`wait_for_end`]) listens to the commands' changes
//! from before the batch's first read, and reads the move again whenever it
//! hears its ID, whenever it has lost changes by falling behind, and, as a
//! fallback that needs no event at all, every [`REREAD_EVERY`]. So a missed
//! event delays the batch by one interval at most and cannot hang it, and the
//! wait ends on shutdown.
//!
//! **A restart runs the batch again** like any command (the commands' loop
//! claims the open commands an earlier process left). A rule whose move is
//! stored under its child ID is followed from that move: a finished one is
//! skipped, an open one is followed, and a failed one stops the batch as it
//! did. A rule that is gone with no move stored was deleted already (by the
//! batch, having no work folder, or by the person) and is skipped. No rule is
//! archived twice. A batch an earlier version left open may find that
//! version's `rule_archive` command under a child ID: a finished one is
//! skipped (the upgrade deleted the rules it archived), and an unfinished one
//! stops the batch as refused ([`REFUSED`]), its rule not archived.
//!
//! **The batch ends** `done` after the last rule, and `failed` at the first
//! rule that does not end well: its move ended `failed` ([`RULE_FAILED`],
//! with that command's reason; the rule is deleted by then), or it could not
//! be archived because its work folder is moving already, or it kept changing
//! ([`REFUSED`], with a sentence; the rule is still there). The rules after it
//! are not archived. The batch records which rule it stopped at in the
//! outcome's `result` ([`stopped_at`]); a batch that was failed by something
//! else (the commands' claim giving it up after
//! [`trss_core::commands::MAX_ATTEMPTS`] starts, a panic) records no rule,
//! because its open move may still end after it.
//!
//! **The screen reads the batch** through [`view`]: how many rules are
//! finished (the leading ones whose move ended `done`, or that are gone with
//! no move), what became of each of them and the work to link to, the rule
//! being archived now and, when it failed, which rule the batch recorded and
//! why. It is computed from the batch, its moves and the rules each time, so
//! nothing else is stored.

use std::{collections::HashSet, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::{sync::broadcast::error::RecvError, time::sleep};
use tokio_util::sync::CancellationToken;

use trss_collect::{
    commands::{
        rule_archive,
        work_archive::{self, NotArchived, RuleArchived, WorkArchive},
    },
    store::channels::ChannelStore,
};
use trss_core::{
    changes::ChangeKind,
    commands::{Command, CommandState, CommandStore, Outcome},
    settings::SettingsStore,
    Clock,
};
use trss_library::store::library::LibraryStore;

use crate::{wake::Wakes, web::commands_api::CommandView};

#[cfg(test)]
mod tests;

/// The `kind` of the command.
pub const KIND: &str = "rule_archive_batch";

/// The one subject of every batch: only one is open at a time.
pub const SUBJECT: &str = "archive-batch";

/// The longest `rule_id` of a request, in bytes. Rule IDs are far shorter; the
/// bound keeps the stored payload sane.
pub const MAX_ID_BYTES: usize = 64;

/// The longest `name` of a request, in characters: a title the screen showed.
pub const MAX_NAME_CHARS: usize = 500;

/// How long the wait for a child lets pass before it reads the child again
/// without having heard of a change.
pub const REREAD_EVERY: Duration = Duration::from_secs(3);

/// The outcome's `result` of a batch that archived every rule.
pub const ARCHIVED: &str = "archived";
/// The outcome's `result` of a batch that stopped because the move of a
/// rule's work folder ended `failed`; the reason is that command's. Followed
/// by the rule's place ([`stopped_at`]).
pub const RULE_FAILED: &str = "rule_failed";
/// The outcome's `result` of a batch that stopped because a rule could not be
/// archived: its work folder is moving already, or the rule kept changing.
/// Followed by the rule's place ([`stopped_at`]).
pub const REFUSED: &str = "refused";

/// How many times a rule edited between the batch's read and its deletion is
/// read again before the batch stops at it.
const CHANGED_TRIES: usize = 3;

/// The `result` of a batch that stopped at the `index`th rule for the reason
/// `kind` ([`RULE_FAILED`] or [`REFUSED`]): `rule_failed:2`.
pub fn stopped_at(kind: &str, index: usize) -> String {
    format!("{kind}:{index}")
}

/// What [`stopped_at`] wrote: the kind and the place; `None` for any other
/// `result`, so a batch failed without a rule recorded names none.
fn stop_of(result: &str) -> Option<(&str, usize)> {
    let (kind, index) = result.split_once(':')?;
    matches!(kind, RULE_FAILED | REFUSED).then_some(())?;
    Some((kind, index.parse().ok()?))
}

/// Why a rule whose work folder is moving already is not archived; the rule's
/// detail says the same.
pub const WORK_BUSY: &str =
    "이 규칙의 작품 폴더를 이미 보관 폴더로 옮기는 중이라서 보관하지 않았어요. 그 결과가 나온 뒤 다시 보관해 주세요.";
const RULE_CHANGING: &str =
    "규칙이 계속 바뀌어서 보관하지 않았어요. 규칙 상세에서 확인한 뒤 다시 보관해 주세요.";
const UNKNOWN_REASON: &str = "까닭은 알 수 없어요.";
const ID_TAKEN: &str = "이 일괄 보관의 작품 폴더 이동 명령 ID가 이미 다른 내용으로 쓰였어요.";
const FROM_BEFORE: &str =
    "업데이트 전에 시작한 일괄 보관이라 이 규칙부터는 보관하지 않았어요. 남은 규칙은 다시 보관해 주세요.";

/// One rule of a batch: its ID, and the title the screen showed for it, which
/// is used only to say in a sentence which rule a batch stopped at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ArchiveBatchRule"))]
#[serde(deny_unknown_fields)]
pub struct BatchRule {
    pub rule_id: String,
    pub name: String,
}

/// The content of a `rule_archive_batch` request: the rules, in the order they
/// are archived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ArchiveBatch"))]
#[serde(deny_unknown_fields)]
pub struct ArchiveBatch {
    pub rules: Vec<BatchRule>,
}

impl ArchiveBatch {
    /// The canonical text stored with the command and compared to tell a
    /// repeat of a request from a different one.
    pub fn canonical(&self) -> String {
        serde_json::to_string(self).expect("a payload serializes")
    }

    pub fn subject(&self) -> String {
        SUBJECT.to_owned()
    }

    /// Why the request cannot be taken, in a sentence for the screen.
    pub fn refusal(&self) -> Option<&'static str> {
        if self.rules.is_empty() {
            return Some("보관할 규칙이 없어요. 화면을 새로고침한 뒤 다시 시도해 주세요.");
        }
        if self
            .rules
            .iter()
            .any(|rule| rule.rule_id.len() > MAX_ID_BYTES)
        {
            return Some("규칙 ID가 너무 길어요. 화면을 새로고침한 뒤 다시 시도해 주세요.");
        }
        if self
            .rules
            .iter()
            .any(|rule| rule.name.chars().count() > MAX_NAME_CHARS)
        {
            return Some("작품 이름이 너무 길어요. 화면을 새로고침한 뒤 다시 시도해 주세요.");
        }
        let mut seen = HashSet::new();
        if !self
            .rules
            .iter()
            .all(|rule| seen.insert(rule.rule_id.as_str()))
        {
            return Some("같은 규칙이 두 번 들어 있어요. 화면을 새로고침한 뒤 다시 시도해 주세요.");
        }
        None
    }
}

/// The ID of the `work_archive` command that moves the work folder of the
/// `index`th rule of the batch `batch_id`. The same for a batch run again, so a
/// command stored before a restart is found, not stored twice.
pub fn child_id(batch_id: &str, index: usize) -> String {
    format!("{batch_id}-{index}")
}

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

/// How an executed batch ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub state: CommandState,
    pub outcome: Outcome,
}

/// A batch that could not be carried on now: it stays `running` and runs
/// again at the next start (of the process, or of its command).
#[derive(Debug, thiserror::Error)]
pub enum Retry {
    #[error("cannot read or write the app database: {0}")]
    Store(String),
    #[error("shutdown was asked for while the batch was waiting")]
    Stopped,
}

impl Retry {
    fn store(err: impl std::fmt::Display) -> Retry {
        Retry::Store(err.to_string())
    }
}

/// What a batch uses.
pub struct Ctx<'a> {
    /// Where the moves are accepted and read.
    pub commands: &'a CommandStore,
    /// Where the rules are read and deleted.
    pub channels: &'a ChannelStore,
    /// The collect folder, which the rules' work folders are below.
    pub settings: &'a SettingsStore,
    /// Rung when a move is stored, so the commands' loop starts it now.
    pub wakes: &'a Wakes,
    pub clock: &'a Clock,
    /// See [`REREAD_EVERY`].
    pub reread_every: Duration,
}

fn finished(result: &str, reason: Option<String>, state: CommandState) -> Finished {
    Finished {
        state,
        outcome: Outcome {
            result: result.to_owned(),
            reason,
        },
    }
}

/// Runs a `rule_archive_batch` command to its end. See the module docs.
pub async fn run(
    ctx: &Ctx<'_>,
    command: &Command,
    cancel: &CancellationToken,
) -> Result<Finished, Retry> {
    let Ok(batch) = serde_json::from_str::<ArchiveBatch>(&command.payload) else {
        return Ok(finished(
            RULE_FAILED,
            Some("요청 내용을 읽지 못했어요.".to_owned()),
            CommandState::Failed,
        ));
    };
    // From before the first read of a child on: a change between a read and
    // the wait that follows it is kept for the wait.
    let mut heard = ctx.commands.subscribe();

    for (index, rule) in batch.rules.iter().enumerate() {
        let id = child_id(&command.id, index);
        let child = match start_child(ctx, &id, index, &rule.rule_id).await? {
            Turn::Moving(child) => child,
            Turn::Deleted => {
                ctx.commands.announce_progress(&command.id);
                continue;
            }
            Turn::Stop(stopped) => return Ok(stopped),
        };
        let child = if child.state.is_open() {
            ctx.wakes.commands();
            wait_for_end(ctx, &mut heard, &id, cancel).await?
        } else {
            child
        };
        match child.state {
            CommandState::Done => ctx.commands.announce_progress(&command.id),
            CommandState::Failed => {
                let reason = child
                    .outcome
                    .and_then(|outcome| outcome.reason)
                    .unwrap_or_else(|| UNKNOWN_REASON.to_owned());
                return Ok(finished(
                    &stopped_at(RULE_FAILED, index),
                    Some(reason),
                    CommandState::Failed,
                ));
            }
            CommandState::Pending | CommandState::Running => {
                unreachable!("the wait returns an ended command")
            }
        }
    }
    Ok(finished(ARCHIVED, None, CommandState::Done))
}

/// What archiving a rule of the batch came to.
enum Turn {
    /// The rule is deleted and this is the move of its work folder, stored now
    /// or by an earlier run.
    Moving(Command),
    /// The rule is deleted with no move: it had no work folder, or it was gone
    /// before its turn.
    Deleted,
    /// The batch ends here.
    Stop(Finished),
}

/// Archives the `index`th rule, `rule_id`, with its move stored under `id`:
/// the move stored before when there is one (a batch run again).
async fn start_child(ctx: &Ctx<'_>, id: &str, index: usize, rule_id: &str) -> Result<Turn, Retry> {
    let stop = |kind: &str, reason: &str| {
        Turn::Stop(finished(
            &stopped_at(kind, index),
            Some(reason.to_owned()),
            CommandState::Failed,
        ))
    };
    if let Some(stored) = ctx.commands.get(id).await.map_err(Retry::store)? {
        return Ok(match stored.kind.as_str() {
            work_archive::KIND => Turn::Moving(stored),
            // Stored by a version whose `보관` kept the rule as archived: a
            // finished one's rule was deleted with the archived rules by the
            // upgrade, and an unfinished one never archived its rule.
            rule_archive::KIND if stored.state == CommandState::Done => Turn::Deleted,
            rule_archive::KIND => stop(REFUSED, FROM_BEFORE),
            _ => stop(RULE_FAILED, ID_TAKEN),
        });
    }
    let collect = ctx
        .settings
        .collection()
        .await
        .map_err(Retry::store)?
        .map(|settings| settings.folder);
    for _ in 0..CHANGED_TRIES {
        let Some(rule) = ctx.channels.get_rule(rule_id).await.map_err(Retry::store)? else {
            return Ok(Turn::Deleted);
        };
        let archived = work_archive::archive_rule(
            ctx.channels,
            ctx.commands,
            collect.as_deref(),
            &rule,
            id.to_owned(),
            (ctx.clock)(),
        )
        .await
        .map_err(Retry::store)?;
        return Ok(match archived {
            Ok(RuleArchived::Moving(child)) => Turn::Moving(*child),
            Ok(RuleArchived::Deleted(_)) => Turn::Deleted,
            // Edited or deleted since it was read: as it is now.
            Err(NotArchived::Changed(_)) => continue,
            Err(NotArchived::Busy(_)) => stop(REFUSED, WORK_BUSY),
            Err(NotArchived::Mismatch(_)) => stop(RULE_FAILED, ID_TAKEN),
        });
    }
    Ok(stop(REFUSED, RULE_CHANGING))
}

/// Reads the command `id` until it has ended, and returns it. It reads again
/// when `heard` brings a change of `id` or has lost changes, and at the latest
/// after [`Ctx::reread_every`], so it ends however the events go; shutdown
/// ends it with [`Retry::Stopped`].
async fn wait_for_end(
    ctx: &Ctx<'_>,
    heard: &mut tokio::sync::broadcast::Receiver<trss_core::changes::Change>,
    id: &str,
    cancel: &CancellationToken,
) -> Result<Command, Retry> {
    loop {
        let command = ctx
            .commands
            .get(id)
            .await
            .map_err(Retry::store)?
            .ok_or_else(|| Retry::Store(format!("command {id} is gone")))?;
        if !command.state.is_open() {
            return Ok(command);
        }
        tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Retry::Stopped),
            _ = change_of(heard, id) => {}
            _ = sleep(ctx.reread_every) => {}
        }
    }
}

/// Returns when the command `id` changed, or changes may have been missed
/// (`heard` fell behind). Never returns once the channel is closed, which
/// leaves the wait to its fallback.
async fn change_of(
    heard: &mut tokio::sync::broadcast::Receiver<trss_core::changes::Change>,
    id: &str,
) {
    loop {
        match heard.recv().await {
            Ok(change)
                if change.kind == ChangeKind::Command && change.id.as_deref() == Some(id) =>
            {
                return
            }
            Ok(_) => {}
            Err(RecvError::Lagged(_)) => return,
            Err(RecvError::Closed) => std::future::pending::<()>().await,
        }
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Why a batch stopped at its rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ArchiveBatchFailureKind"))]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The rule was not archived: its work folder is moving already, or it
    /// kept changing. It is still there.
    Refused,
    /// The rule is deleted, and the move of its work folder ended without
    /// moving it.
    ArchiveFailed,
}

/// The rule a batch stopped at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ArchiveBatchFailure"))]
pub struct BatchFailure {
    pub rule_id: String,
    pub name: String,
    /// A sentence: the refusal, or the reason the move failed.
    pub reason: String,
    pub kind: FailureKind,
    /// `archive_failed`: the work in the library, whose detail moves it again.
    pub work_id: Option<String>,
}

/// What became of a rule the batch archived.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ArchiveBatchResult"))]
pub struct BatchResult {
    pub rule_id: String,
    pub name: String,
    /// The move of the rule's work folder as it ended: its outcome says
    /// whether the folder moved, or stayed and why. None when the rule was
    /// deleted with no move (it had no work folder, or it was gone before its
    /// turn).
    pub command: Option<CommandView>,
    /// The work in the library, for a link to its detail; none when the rule
    /// had no work folder or the library does not know the work.
    pub work_id: Option<String>,
}

/// A batch as the screen sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ArchiveBatchView"))]
pub struct BatchView {
    /// The batch command's ID, which the event stream names when it changes.
    pub id: String,
    /// `pending`, `running`, `done` or `failed`.
    pub state: CommandState,
    pub total: usize,
    /// How many rules, from the first on, are archived.
    pub finished: usize,
    /// What became of each of the `finished` rules, in order.
    pub results: Vec<BatchResult>,
    /// The rule being archived now; none once the batch has ended.
    pub current: Option<BatchRule>,
    /// Set when the batch stopped at a rule.
    pub failure: Option<BatchFailure>,
}

/// What [`view`] reads.
pub struct Stores<'a> {
    pub commands: &'a CommandStore,
    pub channels: &'a ChannelStore,
    pub settings: &'a SettingsStore,
    pub library: &'a LibraryStore,
}

/// A store [`view`] could not read.
#[derive(Debug, thiserror::Error)]
#[error("cannot read the archive batch: {0}")]
pub struct ViewError(String);

fn view_error(err: impl std::fmt::Display) -> ViewError {
    ViewError(err.to_string())
}

/// The library's work of the work folder `command` moves, wherever it is now.
async fn work_of(stores: &Stores<'_>, command: &Command) -> Result<Option<String>, ViewError> {
    let Ok(payload) = serde_json::from_str::<WorkArchive>(&command.payload) else {
        return Ok(None);
    };
    let Some(settings) = stores.settings.collection().await.map_err(view_error)? else {
        return Ok(None);
    };
    work_archive::work_in_library(
        stores.library,
        &settings.folder,
        settings.archive_folder.as_deref(),
        &payload.work,
    )
    .await
    .map_err(view_error)
}

/// The view of `batch`, read from the batch, its moves and the rules.
pub async fn view(stores: &Stores<'_>, batch: &Command) -> Result<BatchView, ViewError> {
    let rules = serde_json::from_str::<ArchiveBatch>(&batch.payload)
        .map(|payload| payload.rules)
        .unwrap_or_default();
    let mut children = stores
        .commands
        .get_many((0..rules.len()).map(|n| child_id(&batch.id, n)).collect())
        .await
        .map_err(view_error)?;
    // A rule is finished when its move ended `done`, or when it is gone with
    // no move; once the batch is `done`, every rule is.
    let mut results = Vec::new();
    for (n, rule) in rules.iter().enumerate() {
        let child = children.remove(&child_id(&batch.id, n));
        let finished = batch.state == CommandState::Done
            || match &child {
                Some(child) => child.state == CommandState::Done,
                None => stores
                    .channels
                    .get_rule(&rule.rule_id)
                    .await
                    .map_err(view_error)?
                    .is_none(),
            };
        if !finished {
            children.extend(child.map(|child| (child.id.clone(), child)));
            break;
        }
        let work_id = match &child {
            Some(child) => work_of(stores, child).await?,
            None => None,
        };
        results.push(BatchResult {
            rule_id: rule.rule_id.clone(),
            name: rule.name.clone(),
            command: child.as_ref().map(CommandView::from),
            work_id,
        });
    }
    let finished = results.len();

    let current = batch
        .state
        .is_open()
        .then(|| {
            rules
                .get(finished.min(rules.len().saturating_sub(1)))
                .cloned()
        })
        .flatten();
    // The rule the batch itself recorded when it stopped. A batch failed by
    // anything else names none: its open move may have ended after it, so the
    // first rule not done is not a rule it stopped at.
    let stop = (batch.state == CommandState::Failed)
        .then(|| {
            let outcome = batch.outcome.as_ref()?;
            let (kind, index) = stop_of(&outcome.result)?;
            Some((outcome, kind, index, rules.get(index)?))
        })
        .flatten();
    let failure = match stop {
        None => None,
        Some((outcome, kind, index, rule)) => {
            let kind = match kind {
                REFUSED => FailureKind::Refused,
                _ => FailureKind::ArchiveFailed,
            };
            let work_id = match children.get(&child_id(&batch.id, index)) {
                Some(child) if kind == FailureKind::ArchiveFailed => work_of(stores, child).await?,
                _ => None,
            };
            Some(BatchFailure {
                rule_id: rule.rule_id.clone(),
                name: rule.name.clone(),
                reason: outcome
                    .reason
                    .clone()
                    .unwrap_or_else(|| UNKNOWN_REASON.to_owned()),
                kind,
                work_id,
            })
        }
    };
    Ok(BatchView {
        id: batch.id.clone(),
        state: batch.state,
        total: rules.len(),
        finished,
        results,
        current,
        failure,
    })
}
