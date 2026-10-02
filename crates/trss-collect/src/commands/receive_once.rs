//! `receive_once`, shown on the screen as `다시 받기`: receive again a history
//! item that a rule picked but did not receive. With a `rule_id` it receives
//! an item the rule would pick but nobody has (`no_match`): a subscription's
//! past items, which the user chooses one by one.
//!
//! The web accepts the command with a [`ReceiveOnce`] payload; the worker runs
//! it with [`run`]. The command kind keeps its first name, which is stored
//! with every command already accepted.
//!
//! An item can be retried only when [`retry_plan`] says so: its result is one a
//! retry may repair (today `add_failed`), and the rule that picked it is
//! recorded on it, still exists and is active. The web checks that when it
//! accepts the request and the worker checks again when it runs the command,
//! because the request may be old. The retry goes where the rule's own cycle
//! would have put the torrent: [`rule_destination`] gives the folder and the
//! episode conversion.
//!
//! A video revision whose download stopped before it was received (its
//! torrent left Transmission or reported an error) can be retried too,
//! whatever its item's result, by the rule recorded on it ([`RevisionRetry`],
//! [`retry_plan_for`]); not while a higher revision of its episode is in the
//! folder or on its way, and not when the rule's folder is no longer the one
//! its replacement was decided for ([`same_destination`]). The worker also
//! refuses it, and a `버전 미상` revision, when the video at the episode's
//! place is that revision or a higher one already, told the way a cycle's
//! decision tells it ([`revisions::holds_same_or_higher`]): a higher revision
//! that found the episode name free has no replacement row to say so. Its add puts the replacement back at its first step
//! with the item's result, in one transaction; a torrent Transmission still
//! had is started again. A revision whose replacement ended after the old
//! video was removed, with no video left under the episode name
//! ([`revisions::ended_with_no_video`], or received so and whole with no file, [`revisions::checked_on_retry`]), is received again the same way;
//! a torrent Transmission still has is checked (`torrent-verify`) before it
//! is started, so it downloads the file that went missing. It is never renamed here, and an add that fails
//! leaves the item and the replacement as they were: the command alone says
//! why.
//!
//! 1. find the history item, its channel and its rule, and check that the item
//!    can be retried ([`retry_plan`]), or, for a request with a `rule_id`,
//!    received by that rule ([`adoption_plan`]); what cannot be retried ends the command
//!    at once, leaving the item as it is. A command stored with a folder chosen
//!    by hand (see [`ReceiveOnce`]) ends the same way: it is not run into the
//!    rule's folder. Every such end takes the command's label off a torrent an
//!    earlier start may have put in;
//! 2. recover the item's original link ([`link::recover`]);
//! 3. add it to Transmission, with the command's label
//!    ([`transmission::command_label`]) next to the bot's and the item's, and
//!    save the answer as the item's result, with the rule kept on the item. An
//!    add that was sent and got no answer is tried again at the next look
//!    ([`Retry::AddUnanswered`]). A `duplicate` answer for a torrent carrying
//!    the command's label counts as this command's own add: an earlier start
//!    put it in, and that add got no answer or the worker died before writing
//!    its result;
//! 4. when this command's add put the torrent in, give the file its `trname`
//!    name with the rule's episode conversion and take the command's label off.
//!    A torrent Transmission already had from elsewhere is not renamed, nor is
//!    one whose name is taken by another file. A `버전 미상` item (a higher
//!    revision the worker did not receive) is not renamed either: receiving it
//!    is the confirmation that starts its replacement
//!    ([`crate::revisions::confirm`]), which names it once the old
//!    video is gone; nor is a revision a caller decided replaces the video
//!    (`지난 회차 검색`). Their row is written in step 3, in one transaction
//!    with the item's result: a rerun of a command whose item is `received`
//!    already ends at once, so a row not written with it would never be.
//!
//! The worker ends the command after step 4. The result lands on the history
//! item (`received`, `duplicate` or `add_failed` with a reason) and on the
//! command, which reports the item's result as history holds it afterwards.
//! Only steps 1 to 3 decide the result. A rename that does not happen leaves
//! the torrent and its data under its own name (a person chose to receive this
//! item again, so it is never removed as the rule cycle does when `trname` has
//! no name), and the history item gets a note saying so ([`RenameResult::Kept`]).

use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use transmission_rpc::types::{Id, TorrentAction};
use trname::trname;

use super::link;
use crate::{
    context::TransmissionLink,
    offsets,
    plan::{picks, rule_destination, rule_work_folder, ChannelPlan},
    revision::Release,
    revisions,
    store::{
        channels::{Channel, ChannelStore, ChannelWithRules, Rule, RuleState},
        history::{HistoryItem, HistoryResult, HistoryStore},
        revisions::{
            Claim, HistoryWrite, NewRevision, Revision, RevisionError, RevisionState,
            RevisionStore, RowWrite, Step,
        },
    },
};
use trss_core::{
    commands::{Command, CommandState, CommandStore, Outcome, MAX_ATTEMPTS},
    folder_locks::Section,
    settings::SettingsStore,
    Millis,
};
use trss_library::store::{library::LibraryStore, seasons::SeasonStore};
use trss_transmission as transmission;
use trss_transmission::{
    add_item, get_torrent, get_torrents, has_label, has_trname_form, remove_label, AddError,
    AddKind, AddLabels, Redactor, RenamePolicy,
};

/// What `receive_once` and `receive_past` use (made from
/// [`CollectContext::receive`](crate::context::CollectContext::receive)).
/// Cheap to clone.
#[derive(Clone)]
pub struct ReceiveContext {
    pub channels: ChannelStore,
    /// Where the collect folder is read from.
    pub settings: SettingsStore,
    pub history: HistoryStore,
    /// The replacements of video revisions a retry carries on.
    pub revisions: RevisionStore,
    /// Where a command records the name its torrent had before the rename
    /// ([`Command::original_name`]).
    pub commands: CommandStore,
    /// With `library`, what the episode offset of a rule's first item is
    /// settled from ([`crate::offsets`]).
    pub seasons: SeasonStore,
    pub library: LibraryStore,
    pub transmission: TransmissionLink,
    /// Reads a channel's feed again for an item's link.
    pub http: reqwest::Client,
    pub rename: RenamePolicy,
    /// Knows secrets that do not come from channels.
    pub redactor: Redactor,
}

/// The `kind` of the command.
pub const KIND: &str = "receive_once";

/// Why a revision was not received again when the folder's video of its
/// episode could not be looked at. Nothing asks again by itself: the person
/// may, later.
/// Why a replacement that ended with no video left was not received again:
/// another file holds its received name, which Transmission would take for
/// the torrent's data and write over.
const RECEIVED_NAME_TAKEN: &str = "받은 이름에 확인한 새 영상이 아닌 다른 파일이 있어서 다시 받지 않았어요. Transmission이 그 파일을 덮어쓸 수 있어요. 그 파일을 옮기거나 지운 뒤 다시 받기를 누를 수 있어요.";
/// Why it was not received again: its received name could not be looked at.
const RECEIVED_NAME_UNREAD: &str = "받은 이름의 파일을 확인하지 못해서 다시 받지 않았어요. 잠시 뒤 다시 받기를 다시 누를 수 있어요.";
/// Why it was not started: Transmission did not check the torrent's data.
const NOT_VERIFIED: &str = "Transmission이 토렌트의 데이터를 다시 확인하지 않아서 시작하지 않았어요. 잠시 뒤 다시 받기를 다시 누를 수 있어요: ";
const PLACE_UNREAD: &str = "폴더의 회차 영상이 어떤 수정본인지 확인하지 못해서 받지 않았어요. 잠시 뒤 다시 받기를 다시 누를 수 있어요.";

/// Longest failure reason kept, in characters.
const MAX_REASON_CHARS: usize = 300;

/// The content of a `receive_once` request: the item, and for an item no rule
/// has picked yet, the rule that receives it. The save folder and the episode
/// conversion are the rule's.
///
/// Requests accepted while a person still chose a save folder carry a
/// `folder`; it is read so those stored commands still parse, and ignored.
/// A new request with one is refused by the web.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiveOnce {
    /// The history item to receive again.
    pub item_id: i64,
    /// The rule that receives an item it would pick but did not (`no_match`).
    /// Without it the item is retried by the rule that picked it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    /// The folder an earlier version of this command let a person choose. Never
    /// written, never used; see above.
    #[serde(default, skip_serializing)]
    pub folder: Option<String>,
}

impl ReceiveOnce {
    pub fn new(item_id: i64) -> ReceiveOnce {
        ReceiveOnce {
            item_id,
            rule_id: None,
            folder: None,
        }
    }

    /// A request for `rule_id` to receive an item it would pick but did not.
    pub fn by_rule(item_id: i64, rule_id: impl Into<String>) -> ReceiveOnce {
        ReceiveOnce {
            item_id,
            rule_id: Some(rule_id.into()),
            folder: None,
        }
    }

    /// Whether the request names a save folder, which a retry no longer takes
    /// (spacing alone does not count).
    pub fn names_a_folder(&self) -> bool {
        self.folder.as_deref().is_some_and(|f| !f.trim().is_empty())
    }

    /// The canonical text stored with the command and compared to tell a
    /// repeat of a request from a different one. It holds the item alone, so a
    /// repeat that also carries an empty `folder` is the same request.
    pub fn canonical(&self) -> String {
        let payload = ReceiveOnce {
            item_id: self.item_id,
            rule_id: self.rule_id.clone(),
            folder: None,
        };
        serde_json::to_string(&payload).expect("a payload serializes")
    }

    /// The subject stored with the command: what it is about.
    pub fn subject(&self) -> String {
        self.item_id.to_string()
    }
}

/// Whether a retry may repair an item with this result. The one place that
/// says which results are offered `다시 받기`: `add_failed`, and `버전 미상`
/// (`version_unknown`: a higher revision the worker did not receive because it
/// could not tell the revisions apart; receiving it is the person's
/// confirmation, see [`crate::revisions`]).
pub fn is_retryable_result(result: HistoryResult) -> bool {
    matches!(
        result,
        HistoryResult::AddFailed | HistoryResult::VersionUnknown
    )
}

/// Why a history item cannot be retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotRetryable {
    /// Transmission holds the item's torrent already (`received`, `duplicate`).
    Held,
    /// No rule picked the item (`no_match`, `excluded`); making a rule from it
    /// is how it gets received.
    NotPicked,
    /// The item's channel was deleted.
    ChannelDeleted,
    /// The item failed without a rule: it comes from the retry that let a
    /// person receive any item into a folder, which no longer exists. Also the
    /// reason a rule is refused such an item.
    NoRule,
    /// The rule that picked the item was deleted.
    RuleDeleted,
    /// The rule that picked the item is archived. Its work folder may be in the
    /// archive folder by now, so it is to be restored first.
    RuleArchived,
    /// The rule that picked the item is paused (`영상 받기` is off): it collects
    /// nothing until turned on.
    RulePaused,
    /// The rule asked to receive the item does not exist (any more).
    RuleMissing,
    /// The rule asked to receive the item belongs to another channel.
    WrongChannel,
    /// The rule would not pick the item: its title does not match, or the
    /// channel excludes it.
    NotMatching,
    /// A different rule picked the item and failed to add it.
    OtherRule,
    /// The item is a revision whose download stopped, and a higher revision
    /// of its episode is in the folder or on its way.
    Overtaken,
    /// The rule's folder is not the folder the revision's replacement was
    /// decided for: a torrent added now would be received elsewhere, and the
    /// replacement would fail again.
    FolderMoved,
    /// The item is a revision whose replacement waits for `다시 받기` (its
    /// download stopped, or `버전 미상`), and the folder's video of its
    /// episode is that revision or a higher one already
    /// ([`revisions::holds_same_or_higher`]). The worker is the authority; the
    /// web tells the cases its own evidence settles beforehand
    /// (`trss_web::in_place`) and leaves the rest to this.
    InPlace,
}

impl NotRetryable {
    /// The sentence for the person, which is also what the row explains.
    pub fn message(self) -> &'static str {
        match self {
            NotRetryable::Held => "이 항목은 Transmission에 이미 있어서 다시 받을 필요가 없어요.",
            NotRetryable::NotPicked => {
                "이 항목은 규칙이 고르지 않아서 다시 받을 수 없어요. 받으려면 이 항목으로 규칙을 만들어요."
            }
            NotRetryable::ChannelDeleted => {
                "이 항목의 채널이 삭제돼서 다시 받을 수 없어요."
            }
            NotRetryable::NoRule => {
                "규칙 없이 받으려다 실패한 항목이라 다시 받을 수 없어요. 이 항목으로 규칙을 만들어요."
            }
            NotRetryable::RuleDeleted => "이 항목을 고른 규칙이 지워져서 다시 받을 수 없어요.",
            NotRetryable::RuleArchived => "규칙이 보관돼 있어요. 복원한 뒤 다시 받아요.",
            NotRetryable::RulePaused => "규칙이 멈춰 있어요. 영상 받기를 켠 뒤 다시 받아요.",
            NotRetryable::RuleMissing => "받으려는 규칙을 찾지 못했어요. 삭제됐을 수 있어요.",
            NotRetryable::WrongChannel => "이 규칙은 다른 채널의 규칙이라 이 항목을 받을 수 없어요.",
            NotRetryable::NotMatching => {
                "이 규칙이 고르지 않는 항목이에요. 제목이 맞지 않거나 채널의 제외 조건에 걸려요."
            }
            NotRetryable::OtherRule => {
                "다른 규칙이 받으려다 실패한 항목이에요. 그 규칙으로 다시 받아요."
            }
            NotRetryable::Overtaken => {
                "이 회차에 더 높은 수정본이 있거나 받는 중이라 다시 받지 않아요."
            }
            NotRetryable::FolderMoved => {
                "규칙의 저장 폴더가 바뀌어서 이 수정본은 다시 받지 않아요. 새 폴더에 받으면 기존 영상과 같은 폴더가 아니라서 대체할 수 없어요."
            }
            NotRetryable::InPlace => {
                "폴더의 이 회차 영상이 이미 같거나 더 높은 수정본이라 다시 받지 않아요."
            }
        }
    }

    /// Whether the row says this to the person: the item is one a rule picked
    /// and failed to receive, so `다시 받기` would be expected on it. For the
    /// others (held, not picked) the row has nothing to add.
    pub fn explains_missing_button(self) -> bool {
        !matches!(self, NotRetryable::Held | NotRetryable::NotPicked)
    }
}

/// What a retry of an item needs: its channel and the rule that picked it.
#[derive(Debug, Clone, Copy)]
pub struct RetryPlan<'a> {
    pub channel: &'a Channel,
    pub rule: &'a Rule,
}

/// A rule that does not collect receives nothing, whatever the item.
fn inactive(rule: &Rule) -> Result<(), NotRetryable> {
    match rule.state {
        RuleState::Active => Ok(()),
        RuleState::Paused => Err(NotRetryable::RulePaused),
        RuleState::Archived => Err(NotRetryable::RuleArchived),
    }
}

/// Whether `item` can be retried, given its channel and the rule recorded on
/// it (`None` when they are gone). This is the eligibility the web checks when
/// it accepts a request, the worker checks again when it runs one, and the
/// history list uses to offer the button.
pub fn retry_plan<'a>(
    item: &HistoryItem,
    channel: Option<&'a Channel>,
    rule: Option<&'a Rule>,
) -> Result<RetryPlan<'a>, NotRetryable> {
    if !is_retryable_result(item.result) {
        return Err(if item.result.is_settled() {
            NotRetryable::Held
        } else {
            NotRetryable::NotPicked
        });
    }
    rule_plan(item, channel, rule)
}

/// The channel and the rule of a retry: the rule recorded on `item` must
/// still exist and be active.
fn rule_plan<'a>(
    item: &HistoryItem,
    channel: Option<&'a Channel>,
    rule: Option<&'a Rule>,
) -> Result<RetryPlan<'a>, NotRetryable> {
    let channel = channel.ok_or(NotRetryable::ChannelDeleted)?;
    if item.rule_id.is_none() {
        return Err(NotRetryable::NoRule);
    }
    let rule = rule.ok_or(NotRetryable::RuleDeleted)?;
    inactive(rule)?;
    Ok(RetryPlan { channel, rule })
}

/// What the replacement row of an item says about `다시 받기` of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionRetry {
    /// No row, or none `다시 받기` receives again: the item is retried as any
    /// other ([`retry_plan`]).
    None,
    /// A revision whose download stopped before it was received, or whose
    /// replacement ended with no video left
    /// ([`revisions::received_again_on_retry`]): received again whatever the
    /// item's result, and its replacement starts over with the same checks
    /// and order. It may have left the feed or come from a past episode
    /// search.
    Again(Box<Revision>),
    /// Such a revision, but a higher revision of its episode is in the folder
    /// or on its way; the replacement steps skip it.
    Overtaken,
}

/// Reads [`RevisionRetry`] for the history item `item_id`.
pub async fn revision_retry(
    store: &RevisionStore,
    item_id: i64,
) -> Result<RevisionRetry, RevisionError> {
    let Some(row) = store.by_item(item_id).await? else {
        return Ok(RevisionRetry::None);
    };
    if !revisions::received_again_on_retry(&row) {
        return Ok(RevisionRetry::None);
    }
    Ok(match store.verdict(row.id).await? {
        Claim::Overtaken => RevisionRetry::Overtaken,
        _ => RevisionRetry::Again(Box::new(row)),
    })
}

/// The row of the replacement that waits for `다시 받기` of `item`: the
/// revision received again or overtaken by a higher one (`revision`), or the
/// `버전 미상` row of an item a retry may repair. `None` for any other item.
pub async fn waiting_revision(
    store: &RevisionStore,
    item: &HistoryItem,
    revision: &RevisionRetry,
) -> Result<Option<Revision>, RevisionError> {
    Ok(match revision {
        RevisionRetry::Again(row) => Some((**row).clone()),
        RevisionRetry::Overtaken => store.by_item(item.id).await?,
        RevisionRetry::None if is_retryable_result(item.result) => store
            .by_item(item.id)
            .await?
            .filter(|row| row.state == RevisionState::Unknown),
        RevisionRetry::None => None,
    })
}

/// [`retry_plan`] for an item whose replacement row says `revision`: a
/// revision whose download stopped is retried whatever the item's result
/// (it is `received`: Transmission took it, then lost it), by the rule
/// recorded on it.
pub fn retry_plan_for<'a>(
    item: &HistoryItem,
    channel: Option<&'a Channel>,
    rule: Option<&'a Rule>,
    revision: &RevisionRetry,
) -> Result<RetryPlan<'a>, NotRetryable> {
    match revision {
        RevisionRetry::None => retry_plan(item, channel, rule),
        RevisionRetry::Again(_) => rule_plan(item, channel, rule),
        RevisionRetry::Overtaken => Err(NotRetryable::Overtaken),
    }
}

/// Whether a revision received again would go where its replacement was
/// decided for: the rule's folder under `collect_folder` is the row's. A
/// revision received into another folder is not next to the video it replaces
/// (the replacement fails, with no more tries), and the web and the worker
/// both refuse it first. Items that are not received again need nothing.
pub fn same_destination(
    revision: &RevisionRetry,
    collect_folder: &Path,
    rule: &Rule,
) -> Result<(), NotRetryable> {
    let RevisionRetry::Again(row) = revision else {
        return Ok(());
    };
    let (save_path, _) = rule_destination(collect_folder, rule);
    if revisions::same_folder(&save_path, Path::new(&row.folder)) {
        Ok(())
    } else {
        Err(NotRetryable::FolderMoved)
    }
}

/// Whether `rule` may receive `item`, an item that no rule has received: the
/// plan of a request with a `rule_id`. The rule must exist, be active and be in
/// the item's channel, and it must pick the item (`no_match`) the way the rule
/// cycle would. An item a rule picked and failed to add is received again by
/// that rule only, and one Transmission holds already needs nothing.
pub fn adoption_plan<'a>(
    item: &HistoryItem,
    channel: Option<&'a Channel>,
    rule: Option<&'a Rule>,
) -> Result<RetryPlan<'a>, NotRetryable> {
    let channel = channel.ok_or(NotRetryable::ChannelDeleted)?;
    let rule = rule.ok_or(NotRetryable::RuleMissing)?;
    if rule.channel_id != item.channel_id {
        return Err(NotRetryable::WrongChannel);
    }
    if item.result.is_settled() {
        return Err(NotRetryable::Held);
    }
    inactive(rule)?;
    match item.result {
        result if is_retryable_result(result) => match item.rule_id.as_deref() {
            Some(id) if id == rule.id => {}
            // A failure that no rule is recorded on belongs to no rule at all.
            None => return Err(NotRetryable::NoRule),
            Some(_) => return Err(NotRetryable::OtherRule),
        },
        _ if picks(channel, rule, &item.title) => {}
        _ => return Err(NotRetryable::NotMatching),
    }
    Ok(RetryPlan { channel, rule })
}

/// How an executed command ended.
#[derive(Debug, Clone)]
pub struct Finished {
    pub state: CommandState,
    pub outcome: Outcome,
    /// Set when this command's add put the torrent in (Transmission did not
    /// have it), so its file is to be renamed.
    pub rename: Option<Rename>,
    /// The command ends before it adds anything, but an earlier start may have
    /// put a torrent in under this command's label (its add got no answer, or
    /// the worker died before writing the result): the label is taken off
    /// every torrent that carries it, whatever ends the command.
    pub unlabel: Option<Redactor>,
    /// An add of this command, on this start or an earlier one, was sent and
    /// got no answer, and no later add got one: Transmission may hold the
    /// torrent all the same, under a hash history did not learn. The command is
    /// ended with this recorded, and the next collection cycle then removes no
    /// departed torrents.
    pub add_unconfirmed: bool,
}

/// What the renaming step needs.
#[derive(Debug, Clone)]
pub struct Rename {
    /// The history item to note on when the name stays as it was.
    pub item_id: i64,
    pub hash: String,
    pub save_path: std::path::PathBuf,
    /// The command that put the torrent in.
    pub command_id: String,
    /// The name the torrent's file had before an earlier start of the command
    /// renamed it ([`Command::original_name`]), as the command was claimed.
    pub original_name: Option<String>,
    /// Whether a worker started the command before: a torrent it put in then
    /// may have been renamed since, by that start or by a cycle.
    pub rerun: bool,
    /// The rule's episode conversion.
    pub episode: isize,
    pub redactor: Redactor,
}

/// A command that could not be carried out or recorded and should be tried
/// again later.
#[derive(Debug, thiserror::Error)]
pub enum Retry {
    /// The database failed.
    #[error("cannot read or write the app database: {0}")]
    Store(String),
    /// The request to add the torrent was sent and got no answer, so
    /// Transmission may have taken it; or an earlier start's did, and this
    /// start could not learn the torrent's hash either. The caller records that
    /// on the command ([`Command::add_unconfirmed`]) and leaves it for the next
    /// look, whose add learns the hash from Transmission's `duplicate` answer.
    /// The last start ([`MAX_ATTEMPTS`]) ends the command instead.
    #[error("Transmission did not answer the request to add the torrent")]
    AddUnanswered,
}

impl Retry {
    pub(crate) fn store(err: impl std::fmt::Display) -> Retry {
        Retry::Store(err.to_string())
    }
}

/// The turn the command takes before it runs: a read of the work folder its
/// rule saves into ([`rule_work_folder`]), the rule found as [`execute`] finds
/// it, and the item alone ([`Section::item`]): the add and the rename of an
/// item are not to meet another receive's or a cycle's of the same item, while
/// receives of other items and readings of the folder go on beside it. The
/// folder part is empty when the request, the item, the rule or the collect
/// folder cannot be found: the command then ends by itself.
///
/// The folder is named when the command is claimed, by its text: a rule or a
/// collect folder changed before the command runs, or a link to the folder
/// under another name, is not seen (see [`trss_core::folder_locks`]).
pub async fn section(ctx: &ReceiveContext, command: &Command) -> Result<Section, Retry> {
    let Ok(payload) = serde_json::from_str::<ReceiveOnce>(&command.payload) else {
        return Ok(Section::new());
    };
    let item = ctx
        .history
        .get(payload.item_id)
        .await
        .map_err(Retry::store)?;
    let rule_id = payload
        .rule_id
        .or_else(|| item.as_ref().and_then(|item| item.rule_id.clone()));
    let section = rule_section(ctx, rule_id.as_deref()).await?;
    Ok(match item {
        Some(item) => section.item(&item.channel_id, &item.identity_key),
        None => section,
    })
}

/// A read of the work folder the rule `rule_id` saves into, or nothing when
/// there is no such rule or no collect folder.
pub(crate) async fn rule_section(
    ctx: &ReceiveContext,
    rule_id: Option<&str>,
) -> Result<Section, Retry> {
    let Some(id) = rule_id else {
        return Ok(Section::new());
    };
    let Some(rule) = ctx.channels.get_rule(id).await.map_err(Retry::store)? else {
        return Ok(Section::new());
    };
    let Some(collect) = ctx.settings.collection().await.map_err(Retry::store)? else {
        return Ok(Section::new());
    };
    Ok(Section::new().read(rule_work_folder(Path::new(&collect.folder), &rule)))
}

/// Runs a `receive_once` command to its end, with its turn ([`section`])
/// taken: [`execute`], then, when the add put the torrent in, [`rename`] and
/// the note when the name stays. The caller ends the command with the
/// returned [`Finished`] afterwards, so a screen that re-reads the item once
/// the command has ended sees the note too.
///
/// A worker that dies before the command is ended leaves it `running`; the
/// rerun meets the torrent as a duplicate carrying the command's label and
/// goes through the same steps again (the rename leaves a name it gave already).
pub async fn run(
    ctx: &ReceiveContext,
    command: &Command,
    now: impl Fn() -> Millis,
    cancel: &CancellationToken,
) -> Result<Finished, Retry> {
    let finished = execute(ctx, command, now).await?;
    finish(ctx, command, finished, cancel).await
}

/// The steps of [`run`] after the add: the rename of a torrent this command
/// put in, the note when the name stays, and the labels that come off.
pub async fn finish(
    ctx: &ReceiveContext,
    command: &Command,
    finished: Finished,
    cancel: &CancellationToken,
) -> Result<Finished, Retry> {
    if let Some(step) = &finished.rename {
        if let RenameResult::Kept(note) = rename(ctx, step, cancel).await {
            // Only reported: the item's result is written already, and a rerun
            // would not rename or note it.
            if let Err(err) = ctx.history.note_received(step.item_id, note).await {
                eprintln!("Cannot note the kept name on item {}: {err}", step.item_id);
            }
        }
        // History holds the torrent's hash now; the command's label has done
        // its job. One left behind (this fails, or the worker dies first) only
        // names a command that has ended.
        let mut transmission = ctx.transmission.client();
        let label = transmission::command_label(&command.id);
        remove_label(&mut transmission, &step.hash, &label, &step.redactor).await;
    }
    if let Some(redactor) = &finished.unlabel {
        let mut transmission = ctx.transmission.client();
        let label = transmission::command_label(&command.id);
        match get_torrents(&mut transmission).await {
            Ok(torrents) => {
                for torrent in torrents {
                    let carries = has_label(torrent.labels.as_deref(), &label);
                    if let (true, Some(hash)) = (carries, torrent.hash_string) {
                        remove_label(&mut transmission, &hash, &label, redactor).await;
                    }
                }
            }
            Err(err) => eprintln!("{}", redactor.apply(&err.to_string())),
        }
    }
    Ok(finished)
}

/// Runs a `receive_once` command up to the point where its outcome is known and
/// recorded on the history item (steps 1 to 3); [`run`] goes on from there.
pub async fn execute(
    ctx: &ReceiveContext,
    command: &Command,
    now: impl Fn() -> Millis,
) -> Result<Finished, Retry> {
    execute_with(ctx, command, now, Settle::Offset, None, false).await
}

/// Whether receiving an item of a rule that has picked nothing decides the
/// rule's episode offset from that item (`다시 받기` of a past item does; a
/// past episode search does not, because the person confirmed a range that was
/// shown with the offset the rule has).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    Offset,
    Keep,
}

/// [`execute`], told whether the rule's offset may be decided from the item,
/// and, with `replacing`, that the item is a revision whose replacement
/// starts once this command's add puts its torrent in: the row (its torrent
/// hash filled in then) is written with the item's result, and the torrent
/// is not renamed ([`crate::revisions`]).
///
/// With `departed`, the item is one Transmission took (`received`,
/// `duplicate`) whose torrent has gone from Transmission and whose video from
/// the folder, which the caller has found out (`receive_past`): it is received
/// again like an item no rule has received, and keeps its result.
pub async fn execute_with(
    ctx: &ReceiveContext,
    command: &Command,
    now: impl Fn() -> Millis,
    settle: Settle,
    replacing: Option<NewRevision>,
    departed: bool,
) -> Result<Finished, Retry> {
    // An add of an earlier start that got no answer stays unaccounted for
    // until an add of this one puts the torrent's hash in history. Until then
    // a start that fails for a reason that may pass (the feed, the link or
    // Transmission) does not end the command, but the last start does: cycles
    // remove nothing while it runs, and the next start adds again. The check
    // comes before `refuse` writes a failure on the item.
    //
    // A failure no later start can get past (the request, the item, the
    // channel or the rule) ends the command at once with the mark kept, so
    // the next cycle removes nothing either; after that the torrent's item
    // label keeps it while its item is in a feed.
    let keep_trying = command.add_unconfirmed && command.attempts < MAX_ATTEMPTS;
    let unaccounted = |mut finished: Finished| {
        finished.add_unconfirmed |= command.add_unconfirmed;
        finished
    };
    let ended_early = |finished: Finished| end_early(ctx, command, finished);

    let Ok(payload) = serde_json::from_str::<ReceiveOnce>(&command.payload) else {
        return Ok(ended_early(failed("요청 내용을 읽지 못했어요.", None)));
    };
    // A command stored while a person chose the folder asked for that folder.
    // Receiving it into the rule's folder is not what was asked.
    if payload.names_a_folder() {
        return Ok(ended_early(failed(LEGACY_FOLDER, None)));
    }

    let Some(item) = ctx
        .history
        .get(payload.item_id)
        .await
        .map_err(Retry::store)?
    else {
        return Ok(ended_early(failed(
            "기록에서 이 항목을 찾지 못했어요.",
            None,
        )));
    };
    let channel = ctx
        .channels
        .get_channel(&item.channel_id)
        .await
        .map_err(Retry::store)?;

    // The rule that picked the item (or the one asked to receive it) decides
    // where it goes. It is read now, not when the request was accepted: the
    // rule may be gone or archived since.
    let rule = match payload.rule_id.as_ref().or(item.rule_id.as_ref()) {
        Some(id) => ctx.channels.get_rule(id).await.map_err(Retry::store)?,
        None => None,
    };
    let revision = match payload.rule_id {
        Some(_) => RevisionRetry::None,
        None => revision_retry(&ctx.revisions, item.id)
            .await
            .map_err(Retry::store)?,
    };
    let again = matches!(revision, RevisionRetry::Again(_));
    let planned = if payload.rule_id.is_some() {
        if departed {
            let unsettled = HistoryItem {
                result: HistoryResult::NoMatch,
                ..item.clone()
            };
            adoption_plan(&unsettled, channel.as_ref(), rule.as_ref())
        } else {
            adoption_plan(&item, channel.as_ref(), rule.as_ref())
        }
    } else {
        retry_plan_for(&item, channel.as_ref(), rule.as_ref(), &revision)
    };
    let plan = match planned {
        Ok(plan) => plan,
        // Something holds the item's torrent now (a rule took it after the
        // request was accepted): that is the command's result too.
        Err(NotRetryable::Held) => return Ok(ended_early(held(item.result, None))),
        // Nothing to repair and nothing to keep trying: it ends the command at
        // once. The item is left as it is; this command did not fail to add it.
        Err(reason) => return Ok(ended_early(failed(reason.message(), None))),
    };
    let channel = plan.channel.clone();
    // The folder is read now, like the rule: it decides where the torrent goes.
    let Some(collect_folder) = ctx.settings.collection().await.map_err(Retry::store)? else {
        return Ok(ended_early(failed(NO_COLLECT_FOLDER, None)));
    };
    // A past item given to a rule that has picked nothing is the rule's first
    // item: the rule's episode offset is decided before the item is named
    // (`offsets`).
    let settled = match (payload.rule_id, settle) {
        (Some(_), Settle::Offset) => {
            offsets::settle_one(
                &ctx.offsets(),
                &collect_folder.folder,
                plan.rule,
                &item.title,
            )
            .await
        }
        _ => None,
    };
    let (save_path, episode) = rule_destination(
        Path::new(&collect_folder.folder),
        settled.as_ref().unwrap_or(plan.rule),
    );
    // Checked against the folder read now, like the rule: the request may be
    // old.
    if let Err(reason) = same_destination(&revision, Path::new(&collect_folder.folder), plan.rule) {
        return Ok(ended_early(failed(reason.message(), None)));
    }
    // A revision whose replacement waits for this request is received only
    // while the folder's video of its episode is lower: a higher one may have
    // taken the episode name as an ordinary item since, with no row to say
    // so. A replacement stopped before its video was received could replace
    // nothing now, and ends as skipped; a `버전 미상` one is left as it is,
    // so a later request looks at the folder again.
    let waiting = waiting_revision(&ctx.revisions, &item, &revision)
        .await
        .map_err(Retry::store)?;
    if let Some(row) = waiting {
        match revisions::holds_same_or_higher(&ctx.revision_work(), &item, &row).await {
            Ok(false) => {}
            Ok(true) => {
                if again {
                    let skip = Step::Skipped {
                        reason: revisions::NOT_HIGHER.to_owned(),
                    };
                    if let Err(err) = ctx.revisions.advance(row.id, now(), row.state, skip).await {
                        eprintln!("Cannot record the revision of item {}: {err}", item.id);
                    }
                }
                return Ok(ended_early(failed(NotRetryable::InPlace.message(), None)));
            }
            Err(why) => {
                eprintln!(
                    "Cannot look at the episode of item {} before receiving it again: {why}",
                    item.id
                );
                return Ok(ended_early(failed(PLACE_UNREAD, None)));
            }
        }
    }
    // A replacement that ended with no video left: its torrent's data is
    // checked again, which must not find another file under its name.
    let ended = matches!(&revision, RevisionRetry::Again(row) if revisions::checked_on_retry(row));
    if let (true, RevisionRetry::Again(row)) = (ended, &revision) {
        match revisions::received_name_free(row).await {
            Ok(true) => {}
            Ok(false) => return Ok(ended_early(failed(RECEIVED_NAME_TAKEN, None))),
            Err(why) => {
                eprintln!(
                    "Cannot look at the received name of item {} before receiving it again: {why}",
                    item.id
                );
                return Ok(ended_early(failed(RECEIVED_NAME_UNREAD, None)));
            }
        }
    }
    let rule_id = plan.rule.id.clone();

    let redactor = redactor_for(ctx, &channel);
    let raw_link = match link::recover(&item, &channel, &ctx.http, &redactor).await {
        Ok(raw) => raw,
        Err(_) if keep_trying => return Err(Retry::AddUnanswered),
        // A revision received again keeps its result: the failure stays on
        // its replacement, and the command says why it did not add it. So does
        // an item whose torrent was gone: its result says Transmission took it.
        Err(reason) if again || departed => return Ok(ended_early(failed(&reason, None))),
        Err(reason) => {
            return refuse(ctx, &item, &rule_id, &reason, &now)
                .await
                .map(unaccounted)
        }
    };
    // The recovered link is a secret from here on, whatever it carried.
    let mut redactor = redactor;
    redactor.add(&raw_link);
    if let Ok(parsed) = url::Url::parse(&raw_link) {
        for (_, value) in parsed.query_pairs() {
            redactor.add_query_value(&value);
        }
    }

    let mut transmission = ctx.transmission.client();
    let item_label = transmission::item_label(&item.channel_id, &item.identity_key);
    let command_label = transmission::command_label(&command.id);
    let added = add_item(
        &mut transmission,
        &raw_link,
        &save_path,
        AddLabels {
            item: Some(&item_label),
            command: Some(&command_label),
        },
        &redactor,
    )
    .await;

    match added {
        Ok(torrent) => {
            // A torrent Transmission already had carries this command's label
            // only when an earlier start's add put it in: that add got no
            // answer, or the worker died before writing its result. A rule's
            // cycle may have met it in between and recorded it as a
            // `duplicate`; it is still this command's.
            let own = torrent.kind == AddKind::Added || torrent.has_label(&command_label);
            // Transmission still has the torrent of a replacement that ended
            // with no video left: it checks the data first, so it downloads
            // the file that went missing. Unchecked, the torrent is not
            // started (nor the row written): the failure stays, with
            // `다시 받기`.
            let check = ended && torrent.kind != AddKind::Added;
            if check {
                let verified = transmission
                    .torrent_action(TorrentAction::Verify, vec![Id::Hash(torrent.hash.clone())])
                    .await;
                let failure = match verified {
                    Ok(response) if response.is_ok() => None,
                    Ok(response) => Some(response.result),
                    Err(err) => Some(err.to_string()),
                };
                if let Some(failure) = failure {
                    let failure = redactor.apply(&failure);
                    eprintln!("Cannot check the torrent of item {}: {failure}", item.id);
                    return Ok(ended_early(failed(
                        &format!("{NOT_VERIFIED}{failure}"),
                        None,
                    )));
                }
            }
            // A revision received again is the item's own torrent, as it was
            // when it was first received.
            let result = if own || again {
                HistoryResult::Received
            } else {
                HistoryResult::Duplicate
            };
            // A revision received this way replaces the folder's video and
            // keeps its received name until the old video is gone: one the
            // caller decided replaces it (its row starts now), and a `버전
            // 미상` one, whose request is the person's confirmation. The row
            // is written with the item's result, in one transaction: once the
            // item is `received` a rerun of this command ends at once, so a
            // row not written with it would never be. One that cannot be
            // written leaves the command to be run again. A revision whose
            // download stopped goes back to its replacement's first step.
            let row = match replacing {
                Some(new) if own => Some(RowWrite::Create {
                    new: NewRevision {
                        item_id: item.id,
                        torrent_hash: Some(torrent.hash.clone()),
                        ..new
                    },
                    reopen: torrent.kind == AddKind::Added,
                }),
                _ if again => Some(RowWrite::Reopen {
                    hash: torrent.hash.clone(),
                }),
                _ if item.result == HistoryResult::VersionUnknown => {
                    Some(revisions::confirm(&item.title, &torrent.hash))
                }
                _ => None,
            };
            let (stored, replacing) = match row {
                Some(row) => {
                    let written = ctx
                        .revisions
                        .write_with_history(
                            now(),
                            HistoryWrite::Outcome {
                                item_id: item.id,
                                result,
                                rule_id: Some(rule_id.clone()),
                                reason: None,
                                torrent_hash: Some(torrent.hash.clone()),
                            },
                            row,
                        )
                        .await
                        .map_err(|err| {
                            Retry::Store(format!(
                                "cannot record item {} with its revision: {err}",
                                item.id
                            ))
                        })?;
                    let replacing = written
                        .row
                        .is_some_and(|row| row.state != RevisionState::Unknown);
                    (written.stored, replacing)
                }
                None => {
                    let stored = ctx
                        .history
                        .record_outcome(
                            item.id,
                            now(),
                            result,
                            Some(rule_id.clone()),
                            None,
                            Some(torrent.hash.clone()),
                        )
                        .await
                        .map_err(Retry::store)?;
                    (stored, false)
                }
            };
            let stored = stored.unwrap_or(result);
            // Transmission still had the revision's torrent, stopped on an
            // error: it is started again, and the replacement looks at it.
            // One checked above starts once the check is done.
            if again && torrent.kind != AddKind::Added {
                let started = transmission
                    .torrent_action(TorrentAction::Start, vec![Id::Hash(torrent.hash.clone())])
                    .await;
                if let Err(err) = started {
                    eprintln!(
                        "Cannot start the torrent of item {}: {}",
                        item.id,
                        redactor.apply(&err.to_string())
                    );
                }
            }
            // Only a torrent this command put in is renamed. One that was there
            // already keeps its name and gets no note.
            let rename = (own && !replacing).then_some(Rename {
                item_id: item.id,
                hash: torrent.hash,
                save_path,
                command_id: command.id.clone(),
                original_name: command.original_name.clone(),
                rerun: command.attempts > 1,
                episode,
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
            let unanswered = matches!(err, AddError::Rpc(_));
            if (unanswered && command.attempts < MAX_ATTEMPTS) || keep_trying {
                return Err(Retry::AddUnanswered);
            }
            if again || departed {
                let mut finished = ended_early(failed(&reason, None));
                finished.add_unconfirmed |= unanswered;
                return Ok(finished);
            }
            let finished = refuse(ctx, &item, &rule_id, &reason, &now).await?;
            // A refusal does not say what Transmission holds either: it
            // fetches a `.torrent` link before it can tell it has the torrent.
            let mut finished = unaccounted(finished);
            finished.add_unconfirmed |= unanswered;
            Ok(finished)
        }
    }
}

/// A command that ends before this start adds anything. A torrent may still
/// carry the command's label when an earlier start ran (it is claimed again
/// after its first start), so the label comes off whatever ends the command,
/// and an earlier start's add that got no answer stays recorded; the item is
/// left as it is.
pub(super) fn end_early(
    ctx: &ReceiveContext,
    command: &Command,
    mut finished: Finished,
) -> Finished {
    if command.add_unconfirmed || command.attempts > 1 {
        finished.unlabel = Some(ctx.redactor.clone());
    }
    finished.add_unconfirmed |= command.add_unconfirmed;
    finished
}

/// Why a command stored with a folder chosen by hand is not run.
const LEGACY_FOLDER: &str =
    "폴더를 고르던 예전 요청이라 실행하지 않았어요. 필요하면 다시 받기로 받아요.";

/// Why a retry does nothing while no collect folder is set: the rule's folder
/// is relative to it, so there is nowhere to put the torrent.
pub(super) const NO_COLLECT_FOLDER: &str =
    "수집 폴더가 정해지지 않아서 받지 않았어요. 설정에서 수집 폴더를 정한 뒤 다시 받아요.";

/// Why a command ended `duplicate`.
const ALREADY_THERE: &str = "Transmission에 이미 같은 토렌트가 있어서 새로 받지 않았어요.";

/// A command that ended with Transmission holding the item's torrent. The
/// outcome follows `stored`, the item's result in history afterwards, which
/// may be a rule's `received` rather than what this command's add answered.
pub(super) fn held(stored: HistoryResult, rename: Option<Rename>) -> Finished {
    Finished {
        state: CommandState::Done,
        outcome: Outcome {
            result: stored.code().to_owned(),
            reason: (stored == HistoryResult::Duplicate).then(|| ALREADY_THERE.to_owned()),
        },
        rename,
        unlabel: None,
        add_unconfirmed: false,
    }
}

/// Records `add_failed` with `reason` on the item and returns the failed
/// command. An item that Transmission holds already (a rule received it, or
/// found it there, after the command was accepted) keeps its result, and the
/// command then ends with that result instead of failing.
async fn refuse(
    ctx: &ReceiveContext,
    item: &HistoryItem,
    rule_id: &str,
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
            Some(rule_id.to_owned()),
            Some(reason.clone()),
            None,
        )
        .await
        .map_err(Retry::store)?
        .unwrap_or(HistoryResult::AddFailed);
    if stored.is_settled() {
        return Ok(held(stored, None));
    }
    Ok(Finished {
        state: CommandState::Failed,
        outcome: Outcome {
            result: stored.code().to_owned(),
            reason: Some(reason),
        },
        rename: None,
        unlabel: None,
        add_unconfirmed: false,
    })
}

/// A failed command that has no history item to write to.
pub(super) fn failed(reason: &str, rename: Option<Rename>) -> Finished {
    Finished {
        state: CommandState::Failed,
        outcome: Outcome {
            result: HistoryResult::AddFailed.code().to_owned(),
            reason: Some(reason.to_owned()),
        },
        rename,
        unlabel: None,
        add_unconfirmed: false,
    }
}

/// Knows the channel's secret values and the Transmission credentials.
fn redactor_for(ctx: &ReceiveContext, channel: &Channel) -> Redactor {
    let mut redactor = ctx.redactor.clone();
    let plan = ChannelPlan::new(
        ChannelWithRules {
            channel: channel.clone(),
            rules: Vec::new(),
        },
        Path::new(""),
    );
    redactor.extend(&plan.redactor());
    redactor
}

fn add_failure_reason(err: &AddError, redactor: &Redactor) -> String {
    let text = match err {
        AddError::Unreachable(err) => format!("Transmission에 연결하지 못했어요: {err}"),
        AddError::Rpc(err) => format!("Transmission이 응답하지 않았어요: {err}"),
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
/// The torrent has more than one file; `trname` names a single file.
pub const SEVERAL_FILES: &str = "파일이 여러 개인 토렌트라 이름을 바꾸지 않았어요.";
/// The name `trname` gives is taken by another file in the folder.
pub const NAME_TAKEN: &str = "같은 회차 이름의 파일이 이미 있어서 원래 이름 그대로 뒀어요.";
/// Renaming was tried and did not go through.
pub const NAME_NOT_CHANGED: &str = "이름을 바꾸지 못해서 원래 이름 그대로 뒀어요.";

/// Gives the torrent's single file its `trname` name for the folder it was
/// saved in, with the rule's episode conversion, derived from the name the
/// file had before the command first renamed it, which the command records
/// ([`Command::original_name`]): run again over a file it named already, it
/// finds the same name and leaves it. A rerun with no name recorded leaves a
/// name in the `trname` form. A torrent with several files is left as it is
/// at once, as the rule cycle leaves it.
///
/// Unlike the renaming after a rule's add, a torrent whose name cannot be
/// derived is left alone: that path removes the torrent and its data, which is
/// no answer to a person who chose to receive this item again (a rule saving
/// to the base folder, for one, has no title and season parts to name a file
/// after). The result says
/// when the original name was kept, so the caller can note it on the history
/// item. Attempts follow `ctx.rename`, as a magnet link's file name is only
/// known once Transmission has its metadata.
pub async fn rename(
    ctx: &ReceiveContext,
    rename: &Rename,
    cancel: &CancellationToken,
) -> RenameResult {
    let mut transmission = ctx.transmission.client();
    let mut original = rename.original_name.clone();
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
        match torrent.file_count {
            Some(1) => {}
            // Transmission counts no files until a magnet link's metadata is in.
            Some(0) | None => continue,
            Some(_) => return RenameResult::Kept(SEVERAL_FILES),
        }
        let Some(old_name) = torrent.name else {
            continue;
        };
        // The name is derived from the name the file had before any rename,
        // as the rule cycle derives it from the name of a torrent it has just
        // put in, never from the name the file has now: a name this command's
        // earlier start or a cycle gave is then the same name again and stays,
        // so no episode is converted twice, while a release that comes in the
        // `trname` form of another season or episode is still converted.
        let original = match &original {
            Some(name) => name.clone(),
            None => {
                // An earlier start put the torrent in and recorded no name: a
                // name in the `trname` form may be one it or a cycle gave.
                if rename.rerun
                    && has_trname_form(
                        &old_name,
                        &rename.save_path,
                        rename.episode,
                        Release::without_version,
                    )
                {
                    return RenameResult::Unchanged;
                }
                // Recorded before any rename, so a later start finds it.
                match ctx
                    .commands
                    .note_original_name(&rename.command_id, &old_name)
                    .await
                {
                    Ok(Some(name)) => {
                        original = Some(name.clone());
                        name
                    }
                    Ok(None) => return RenameResult::Unchanged,
                    Err(err) => {
                        eprintln!("Cannot record the name of item {}: {err}", rename.item_id);
                        continue;
                    }
                }
            }
        };
        let Some(new_name) = derived_name(&rename.save_path, &original, rename.episode) else {
            return RenameResult::Kept(NAME_NOT_DERIVED);
        };
        if new_name == old_name {
            return RenameResult::Unchanged;
        }
        // Never onto a name that is taken (see `transmission::rename_torrent`).
        let folder = torrent
            .download_dir
            .as_deref()
            .map_or(rename.save_path.as_path(), Path::new);
        if std::fs::symlink_metadata(folder.join(&new_name)).is_ok() {
            return RenameResult::Kept(NAME_TAKEN);
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

/// The name `trname` gives `file_name` in `save_path` with the rule's `episode`
/// conversion, as the rule cycle's renaming derives it: read from the name
/// without its revision ([`Release::without_version`]), because `trname` does
/// not read `06v2` as episode 6 in every name.
pub fn derived_name(save_path: &Path, file_name: &str, episode: isize) -> Option<String> {
    trname(save_path, &Release::without_version(file_name), episode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::channels::{Channel, Rule};

    #[test]
    fn the_canonical_payload_holds_the_item_alone() {
        let a = ReceiveOnce::new(7);
        let with_empty_folder = ReceiveOnce {
            item_id: 7,
            rule_id: None,
            folder: Some("  ".into()),
        };
        let other = ReceiveOnce::new(8);
        assert_eq!(a.canonical(), with_empty_folder.canonical());
        assert_ne!(a.canonical(), other.canonical());
        assert_eq!(a.canonical(), r#"{"item_id":7}"#);
    }

    #[test]
    fn a_request_for_a_rule_is_another_request_than_the_plain_one() {
        let by_rule = ReceiveOnce::by_rule(7, "r1");
        assert_eq!(by_rule.canonical(), r#"{"item_id":7,"rule_id":"r1"}"#);
        assert_ne!(by_rule.canonical(), ReceiveOnce::new(7).canonical());
        assert_ne!(
            by_rule.canonical(),
            ReceiveOnce::by_rule(7, "r2").canonical()
        );
        assert_eq!(
            serde_json::from_str::<ReceiveOnce>(r#"{"item_id":7,"rule_id":"r1"}"#).unwrap(),
            by_rule
        );
        assert_eq!(by_rule.subject(), "7");
    }

    #[test]
    fn a_rule_receives_an_item_it_would_pick_in_its_own_channel_only_while_active() {
        let (channel, active, archived, paused) = (
            channel(),
            rule(RuleState::Active),
            rule(RuleState::Archived),
            rule(RuleState::Paused),
        );
        let missed = item(HistoryResult::NoMatch, None);
        assert_eq!(
            adoption_plan(&missed, Some(&channel), Some(&active))
                .unwrap()
                .rule
                .id,
            "r1"
        );

        let why =
            |item: &HistoryItem, channel, rule| adoption_plan(item, channel, rule).unwrap_err();
        assert_eq!(
            why(&missed, Some(&channel), None),
            NotRetryable::RuleMissing
        );
        assert_eq!(
            why(&missed, None, Some(&active)),
            NotRetryable::ChannelDeleted
        );
        assert_eq!(
            why(&missed, Some(&channel), Some(&archived)),
            NotRetryable::RuleArchived
        );
        assert_eq!(
            why(&missed, Some(&channel), Some(&paused)),
            NotRetryable::RulePaused
        );
        let elsewhere = Rule {
            channel_id: "c2".into(),
            ..rule(RuleState::Active)
        };
        assert_eq!(
            why(&missed, Some(&channel), Some(&elsewhere)),
            NotRetryable::WrongChannel
        );
        // The title must match the rule, and the channel must not exclude it.
        let other_phrase = Rule {
            r#match: Some("Another Show".into()),
            ..rule(RuleState::Active)
        };
        assert_eq!(
            why(&missed, Some(&channel), Some(&other_phrase)),
            NotRetryable::NotMatching
        );
        let excluding = Channel {
            excludes: vec!["26".into()],
            ..channel.clone()
        };
        assert_eq!(
            why(&missed, Some(&excluding), Some(&active)),
            NotRetryable::NotMatching
        );
        let excluded = item(HistoryResult::Excluded, None);
        assert_eq!(
            why(&excluded, Some(&excluding), Some(&active)),
            NotRetryable::NotMatching
        );
        for result in [HistoryResult::Received, HistoryResult::Duplicate] {
            assert_eq!(
                why(&item(result, Some("r1")), Some(&channel), Some(&active)),
                NotRetryable::Held
            );
        }
        // A failed add is repaired by the rule that picked it, not another.
        let failed = item(HistoryResult::AddFailed, Some("r1"));
        assert!(adoption_plan(&failed, Some(&channel), Some(&active)).is_ok());
        let failed_elsewhere = item(HistoryResult::AddFailed, Some("r9"));
        assert_eq!(
            why(&failed_elsewhere, Some(&channel), Some(&active)),
            NotRetryable::OtherRule
        );
        // A failure with no rule recorded at all is no other rule's.
        let failed_without_rule = item(HistoryResult::AddFailed, None);
        assert_eq!(
            why(&failed_without_rule, Some(&channel), Some(&active)),
            NotRetryable::NoRule
        );
    }

    #[test]
    fn a_payload_stored_with_a_folder_still_parses_and_the_folder_is_not_kept() {
        // Commands accepted while a person chose the folder are in the database.
        let old: ReceiveOnce =
            serde_json::from_str(r#"{"item_id":7,"folder":"LIAR GAME/Season 01"}"#).unwrap();
        assert_eq!(old.item_id, 7);
        assert!(old.names_a_folder());
        assert_eq!(old.canonical(), r#"{"item_id":7}"#);
        let older: ReceiveOnce = serde_json::from_str(r#"{"item_id":7,"folder":""}"#).unwrap();
        assert!(!older.names_a_folder());
        assert_eq!(
            serde_json::from_str::<ReceiveOnce>(r#"{"item_id":7}"#).unwrap(),
            ReceiveOnce::new(7)
        );
    }

    #[test]
    fn a_payload_with_another_unknown_field_is_not_read() {
        assert!(serde_json::from_str::<ReceiveOnce>(r#"{"item_id":7,"link":"x"}"#).is_err());
        assert!(serde_json::from_str::<ReceiveOnce>(r#"{"folder":"x"}"#).is_err());
    }

    #[test]
    fn an_episode_conversion_moves_the_release_episode_as_the_rule_would() {
        let folder = Path::new("/media/anime/LIAR GAME/Season 01");
        let release = "[SubsPlease] LIAR GAME - 26 (1080p) [ABCD1234].mkv";

        // 0 and 1 leave the number as it is: the legacy default is 1.
        let expected = Some("LIAR GAME S01E26.mkv".to_owned());
        assert_eq!(derived_name(folder, release, 0), expected);
        assert_eq!(derived_name(folder, release, 1), expected);
        assert_eq!(
            derived_name(folder, release, -12),
            Some("LIAR GAME S01E14.mkv".to_owned())
        );
    }

    /// A show named with `NvM` (`Show 3v3`): its first release of an episode
    /// is named as `trname` reads it, and its revision of that episode gets
    /// the same name.
    #[test]
    fn a_show_named_with_a_number_v_number_is_named_by_its_episode() {
        let folder = Path::new("/media/anime/Show 3v3/Season 01");
        let first = "[SubsPlease] Show 3v3 - 06 (1080p) [1A2B3C4D].mkv";
        let second = "[SubsPlease] Show 3v3 - 06v2 (1080p) [5E6F7A8B].mkv";
        let named = derived_name(folder, first, 0);
        assert_eq!(named, trname(folder, first, 0));
        assert!(
            named.as_deref().is_some_and(|n| n.ends_with("E06.mkv")),
            "{named:?}"
        );
        assert_eq!(derived_name(folder, second, 0), named);
    }

    /// `trname` reads Erai-raws' `06v2` as episode 34 (from the CRC32
    /// bracket); the name is read without the revision, and a name without
    /// one is left as `trname` reads it.
    #[test]
    fn a_revision_is_named_as_its_episode() {
        let folder = Path::new("/media/anime/Show/Season 01");
        let erai = "[Erai-raws] Show - 06v2 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv";
        assert_eq!(
            trname(folder, erai, 0).as_deref(),
            Some("Show S01E34.mkv"),
            "trname alone"
        );
        assert_eq!(
            derived_name(folder, erai, 0).as_deref(),
            Some("Show S01E06.mkv")
        );
        let first = "[Erai-raws] Show - 06 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv";
        assert_eq!(derived_name(folder, first, 0), trname(folder, first, 0));
        assert_eq!(
            derived_name(folder, "[SubsPlease] Show - 14v2 (1080p) [8F2EFECC].mkv", 0).as_deref(),
            Some("Show S01E14.mkv")
        );
        for name in [
            "[SubsPlease] Show - 14 (1080p) [8F2EFECC].mkv",
            "[SubsPlease] Show - 14 (1080p).mkv",
            "[Erai-raws] Show - 06 [1080p CR WEBRip HEVC AAC][MultiSub].mkv",
            "Show S01E14.mkv",
        ] {
            assert_eq!(
                derived_name(folder, name, 0),
                trname(folder, name, 0),
                "{name}"
            );
        }
        // The rule's episode conversion applies to the revision's episode.
        assert_eq!(
            derived_name(
                folder,
                "[SubsPlease] Show - 13v2 (1080p) [8F2EFECC].mkv",
                -12
            )
            .as_deref(),
            Some("Show S01E01.mkv")
        );
        assert_eq!(
            derived_name(folder, "[SubsPlease] Show - 13 (1080p) [8F2EFECC].mkv", -12).as_deref(),
            Some("Show S01E01.mkv")
        );
    }

    #[test]
    fn a_folder_without_title_and_season_parts_gets_no_name() {
        assert_eq!(
            derived_name(
                Path::new("/media/anime"),
                "[SubsPlease] LIAR GAME - 26 (1080p) [ABCD1234].mkv",
                1
            ),
            None
        );
    }

    fn channel() -> Channel {
        Channel {
            id: "c1".into(),
            position: 0,
            version: 1,
            url: "https://feed.test/rss".into(),
            excludes: Vec::new(),
            secret_query: Vec::new(),
            past_search: None,
            name: None,
        }
    }

    fn rule(state: RuleState) -> Rule {
        Rule {
            id: "r1".into(),
            channel_id: "c1".into(),
            position: 0,
            version: 1,
            r#match: Some("LIAR GAME".into()),
            regex: false,
            case_insensitive: false,
            directory: "LIAR GAME/Season 01".into(),
            episode: 1,
            episode_auto: false,
            state,
            subscription: None,
            resumed_at: None,
        }
    }

    fn item(result: HistoryResult, rule_id: Option<&str>) -> HistoryItem {
        HistoryItem {
            first_read: false,
            id: 1,
            channel_id: "c1".into(),
            channel_label: "https://feed.test/rss".into(),
            identity_key: "guid:1".into(),
            title: "LIAR GAME - 26".into(),
            link: "magnet:?xt=urn:btih:x".into(),
            first_seen_at: 0,
            last_seen_at: 0,
            result,
            result_at: 0,
            rule_id: rule_id.map(str::to_owned),
            reason: None,
            torrent_hash: None,
        }
    }

    #[test]
    fn only_an_item_a_rule_picked_and_failed_to_add_with_an_active_rule_can_be_retried() {
        let (channel, active, archived, paused) = (
            channel(),
            rule(RuleState::Active),
            rule(RuleState::Archived),
            rule(RuleState::Paused),
        );
        let failed = item(HistoryResult::AddFailed, Some("r1"));
        let plan = retry_plan(&failed, Some(&channel), Some(&active)).unwrap();
        assert_eq!(plan.rule.id, "r1");

        let why = |item: &HistoryItem, channel, rule| retry_plan(item, channel, rule).unwrap_err();
        assert_eq!(
            why(&failed, Some(&channel), None),
            NotRetryable::RuleDeleted,
            "the rule was deleted"
        );
        assert_eq!(
            why(&failed, Some(&channel), Some(&archived)),
            NotRetryable::RuleArchived
        );
        assert_eq!(
            why(&failed, Some(&channel), Some(&paused)),
            NotRetryable::RulePaused
        );
        assert_eq!(
            why(&failed, None, Some(&active)),
            NotRetryable::ChannelDeleted
        );
        assert_eq!(
            why(
                &item(HistoryResult::AddFailed, None),
                Some(&channel),
                Some(&active)
            ),
            NotRetryable::NoRule,
            "a failure with no rule recorded"
        );
        for (result, expected) in [
            (HistoryResult::Received, NotRetryable::Held),
            (HistoryResult::Duplicate, NotRetryable::Held),
            (HistoryResult::NoMatch, NotRetryable::NotPicked),
            (HistoryResult::Excluded, NotRetryable::NotPicked),
        ] {
            assert_eq!(
                why(&item(result, Some("r1")), Some(&channel), Some(&active)),
                expected,
                "{result}"
            );
        }
    }

    #[test]
    fn a_stopped_revision_overtaken_by_a_higher_one_is_not_retried() {
        let (channel, active) = (channel(), rule(RuleState::Active));
        let received = item(HistoryResult::Received, Some("r1"));
        let plan = |revision| {
            retry_plan_for(&received, Some(&channel), Some(&active), &revision).map(|_| ())
        };
        assert_eq!(plan(RevisionRetry::None), Err(NotRetryable::Held));
        assert_eq!(plan(RevisionRetry::Overtaken), Err(NotRetryable::Overtaken));
        assert!(NotRetryable::Overtaken.explains_missing_button());
    }

    #[test]
    fn the_retry_goes_where_the_rules_own_cycle_would_put_it() {
        let (destination, episode) = rule_destination(
            Path::new("/media/anime"),
            &Rule {
                episode: -12,
                ..rule(RuleState::Active)
            },
        );
        assert_eq!(destination, Path::new("/media/anime/LIAR GAME/Season 01"));
        assert_eq!(episode, -12);
    }
}
