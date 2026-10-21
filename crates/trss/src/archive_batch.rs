//! `rule_archive_batch`, shown on the screen as `모두 보관` and `N개 보관` of
//! the archive suggestions (`docs/specs/collection.md`, 규칙 보관): the rules
//! the person chose, archived one after another by the server.
//!
//! The screen used to send a `rule_archive` command per rule and wait for each
//! to end before it sent the next, so a tab that stopped listening (a hidden
//! tab closes its event stream) or a closed screen left the rest unsent. The
//! screen now sends one `rule_archive_batch` command with the rules in order
//! ([`ArchiveBatch`]), and [`run`] carries it out in the commands' loop.
//!
//! **A rule is archived by its own command.** The rule's detail shows the last
//! move of its folder by reading the rule's latest `rule_archive` command, and
//! a rule has at most one open `rule_archive` command, so the batch does not
//! archive inline. For each rule, in order, it accepts an ordinary
//! `rule_archive` command (direction `archive`) with the ID
//! [`child_id`] (`{batch ID}-{index}`), rings the commands' loop so that it
//! starts at once, and waits for it to end. Everything about archiving one
//! rule (turning it off, moving the work folder, which is moved once, with
//! the last rule that saves in it, the retries, the sentences) stays with that
//! command ([`trss_collect::commands::rule_archive`]); the batch only keeps
//! the order and stops at the first rule that does not end well.
//!
//! **Waiting for a child** ([`wait_for_end`]) listens to the commands' changes
//! from before the batch's first read, and reads the child again whenever it
//! hears the child's ID, whenever it has lost changes by falling behind, and,
//! as a fallback that needs no event at all, every [`REREAD_EVERY`]. So a
//! missed event delays the batch by one interval at most and cannot hang it,
//! and the wait ends on shutdown.
//!
//! **A restart runs the batch again** like any command (the commands' loop
//! claims the open commands an earlier process left). The batch accepts the
//! same child IDs, which answer with the commands they stored: a finished child
//! is skipped, an open one is followed, and a failed one stops the batch as it
//! did. No rule is archived twice.
//!
//! **The batch ends** `done` after the last rule, and `failed` at the first
//! rule that does not end well: its archive ended `failed`
//! ([`RULE_FAILED`], with that command's reason), or it could not be
//! started because another `rule_archive` command of the rule is open or the
//! rule is gone ([`REFUSED`], with the sentence the rule's own refusal has).
//! The rules after it are not archived. The batch records which rule it
//! stopped at in the outcome's `result` ([`stopped_at`]); a batch that was
//! failed by something else (the commands' claim giving it up after
//! [`trss_core::commands::MAX_ATTEMPTS`] starts, a panic) records no rule,
//! because its open child may still end after it.
//!
//! **The screen reads the batch** through [`view`]: how many rules are
//! finished (the leading ones whose command ended `done`), the rule being
//! archived now and, when it failed, which rule the batch recorded and why.
//! It is computed from the batch and its children each time, so nothing else
//! is stored.

use std::{collections::HashSet, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::{sync::broadcast::error::RecvError, time::sleep};
use tokio_util::sync::CancellationToken;

use trss_collect::{
    commands::rule_archive::{self, Direction, RuleArchive},
    store::channels::ChannelStore,
};
use trss_core::{
    changes::ChangeKind,
    commands::{Accepted, Command, CommandError, CommandState, CommandStore, NewCommand, Outcome},
    Clock,
};

use crate::wake::Wakes;

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
/// The outcome's `result` of a batch that stopped because a rule's own
/// archive ended `failed`; the reason is that command's. Followed by the
/// rule's place ([`stopped_at`]).
pub const RULE_FAILED: &str = "rule_failed";
/// The outcome's `result` of a batch that stopped because a rule could not be
/// started: another `rule_archive` command of it is open, or it is gone.
/// Followed by the rule's place ([`stopped_at`]).
pub const REFUSED: &str = "refused";

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

/// What a second open command of a rule is told (the rule's own refusal).
pub const RULE_BUSY: &str =
    "이 규칙은 이미 보관하거나 복원하거나 작품 폴더를 옮기는 중이에요. 그 결과가 나올 때까지 기다려 주세요.";
const RULE_GONE: &str = "규칙을 찾지 못했어요. 삭제됐을 수 있어요.";
const UNKNOWN_REASON: &str = "까닭은 알 수 없어요.";
const ID_TAKEN: &str = "이 일괄 보관의 규칙 보관 명령 ID가 이미 다른 내용으로 쓰였어요.";

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

/// The ID of the `rule_archive` command that archives the `index`th rule of
/// the batch `batch_id`. The same for a batch run again, so a command stored
/// before a restart is found, not stored twice.
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
    /// Where the children are accepted and read.
    pub commands: &'a CommandStore,
    /// Tells whether a rule is still there before its command is accepted.
    pub channels: &'a ChannelStore,
    /// Rung when a child is stored, so the commands' loop starts it now.
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
            Ok(child) => child,
            Err(stopped) => return Ok(stopped),
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

/// The `rule_archive` command that archives `rule_id` for the child `id`: the
/// one stored before when there is one (a batch run again), else a new one.
/// `Err` is the end of the batch when the rule, the `index`th, cannot be started.
async fn start_child(
    ctx: &Ctx<'_>,
    id: &str,
    index: usize,
    rule_id: &str,
) -> Result<Result<Command, Finished>, Retry> {
    let stored = ctx.commands.get(id).await.map_err(Retry::store)?;
    if stored.is_none()
        && ctx
            .channels
            .get_rule(rule_id)
            .await
            .map_err(Retry::store)?
            .is_none()
    {
        return Ok(Err(finished(
            &stopped_at(REFUSED, index),
            Some(RULE_GONE.to_owned()),
            CommandState::Failed,
        )));
    }
    let payload = RuleArchive {
        rule_id: rule_id.to_owned(),
        direction: Direction::Archive,
        receive: Vec::new(),
    };
    let accepted = ctx
        .commands
        .accept(
            NewCommand {
                id: id.to_owned(),
                kind: rule_archive::KIND.to_owned(),
                payload: payload.canonical(),
                subject: Some(payload.subject()),
            },
            (ctx.clock)(),
        )
        .await
        .map_err(Retry::store)?;
    Ok(match accepted {
        Accepted::Created(child) | Accepted::Existing(child) => Ok(child),
        Accepted::Busy(_) => Err(finished(
            &stopped_at(REFUSED, index),
            Some(RULE_BUSY.to_owned()),
            CommandState::Failed,
        )),
        Accepted::Mismatch(_) => Err(finished(
            &stopped_at(RULE_FAILED, index),
            Some(ID_TAKEN.to_owned()),
            CommandState::Failed,
        )),
    })
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
            Ok(change) if change.kind == ChangeKind::Command && change.id == id => return,
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
    /// The rule could not be started: another command of it is open, or it is gone.
    Refused,
    /// The rule's archive ended without archiving it.
    ArchiveFailed,
}

/// The rule a batch stopped at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ArchiveBatchFailure"))]
pub struct BatchFailure {
    pub rule_id: String,
    pub name: String,
    /// A sentence: the refusal, or the reason the rule's archive failed.
    pub reason: String,
    pub kind: FailureKind,
}

/// A batch as the screen sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ArchiveBatchView"))]
pub struct BatchView {
    /// The batch command's ID, which the event stream names when it changes.
    pub id: String,
    /// `pending`, `running`, `done` or `failed`.
    pub state: CommandState,
    pub total: usize,
    /// How many rules, from the first on, are archived.
    pub finished: usize,
    /// The rule being archived now; none once the batch has ended.
    pub current: Option<BatchRule>,
    /// Set when the batch stopped at a rule.
    pub failure: Option<BatchFailure>,
}

/// The view of `batch`, read from the batch and its children.
pub async fn view(commands: &CommandStore, batch: &Command) -> Result<BatchView, CommandError> {
    let rules = serde_json::from_str::<ArchiveBatch>(&batch.payload)
        .map(|payload| payload.rules)
        .unwrap_or_default();
    let children = commands
        .get_many((0..rules.len()).map(|n| child_id(&batch.id, n)).collect())
        .await?;
    let finished = if batch.state == CommandState::Done {
        rules.len()
    } else {
        (0..rules.len())
            .take_while(|&n| {
                children
                    .get(&child_id(&batch.id, n))
                    .is_some_and(|child| child.state == CommandState::Done)
            })
            .count()
    };

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
    // anything else names none: its open child may have ended after it, so
    // the first rule not done is not a rule it stopped at.
    let failure = (batch.state == CommandState::Failed)
        .then(|| {
            let outcome = batch.outcome.as_ref()?;
            let (kind, index) = stop_of(&outcome.result)?;
            let rule = rules.get(index)?;
            Some(BatchFailure {
                rule_id: rule.rule_id.clone(),
                name: rule.name.clone(),
                reason: outcome
                    .reason
                    .clone()
                    .unwrap_or_else(|| UNKNOWN_REASON.to_owned()),
                kind: match kind {
                    REFUSED => FailureKind::Refused,
                    _ => FailureKind::ArchiveFailed,
                },
            })
        })
        .flatten();
    Ok(BatchView {
        id: batch.id.clone(),
        state: batch.state,
        total: rules.len(),
        finished,
        current,
        failure,
    })
}
