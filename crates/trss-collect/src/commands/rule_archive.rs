//! `rule_archive`: a rule that starts collecting while its work folder is in
//! the archive folder (`docs/requirements/collection.md`, 보관과 수집 시작의 폴더 이동)
//! waits for the folder to come into the collect folder first.
//!
//! The web stores the command itself ([`ask_start`]) with a [`RuleArchive`]
//! payload (the rule and a [`Direction`]); the worker runs it with [`run`],
//! holding the turn of the work folder in the collect folder and in the
//! archive folder ([`section`]), so no reading of the watch folders reads the
//! folder while it moves. `보관` is no direction of it any more: it deletes
//! the rule and moves the work folder into the archive folder through
//! [`super::work_archive`] ([`super::work_archive::archive_rule`]). The kind
//! keeps its name for the commands stored before.
//!
//! **Start** and **Resume** are a new rule or subscription beginning to
//! collect and `영상 받기` turning on, for a rule whose work folder is in the
//! archive folder. The web leaves such a rule `paused` (it collects nothing,
//! so no cycle can make a work folder in the collect folder first) and stores
//! the command; the worker moves the work folder from the archive folder into
//! the collect folder (merging into one that is there) and only then turns the
//! rule on, so a new episode never makes a second work folder in the collect
//! folder that the move would run into. A move that fails leaves the rule
//! paused, with the reason; turning `영상 받기` on again tries again. The web
//! decides whether this is needed ([`plan_start`]) and the command decides
//! again from the disk when it runs: a work folder found in no archive folder
//! just turns the rule on. The time the rule waited does not cost it the items
//! that came meanwhile: a new rule is never noted as resumed, and a `영상 받기`
//! is on from when it was pressed.
//!
//! A `start` of a subscription carries the past items the person ticked while
//! subscribing ([`RuleArchive::receive`], ticket 5-deployed-verification/0125). Once the rule is on, the
//! command accepts a `receive_once` for each, in order, which the worker runs
//! after this command like any other: the same receive as `다시 받기`, which
//! checks the rule picks the item and decides the rule's first episode offset.
//! A start that does not turn the rule on (the move failed) receives none of
//! them; they stay past items to pick from the rule's detail.
//!
//! **The library follows the folder.** When a work folder has moved (or is found
//! moved already), the work the library holds under the old place keeps its ID
//! and belongs to the new place, or is merged into the work the destination
//! already had ([`trss_library::watch::follow_move`]). Nothing follows a folder
//! moved by hand.
//!
//! Every step is safe to repeat, and where the folder is comes from the disk
//! each time ([`work_folder`]), so a command cut short (the worker stopped or
//! died) is claimed again by the next worker and finishes what is left. So is
//! a start that waited for Transmission longer than [`work_folder::MovePolicy`]
//! allows ([`Retry::Later`]): the command stays `running` for the next look,
//! and only its last start ([`MAX_ATTEMPTS`]) ends it `failed`, with that
//! reason.

#[cfg(test)]
mod run_tests;
#[cfg(test)]
mod start_tests;
pub mod work_folder;

use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use trss_core::{
    commands::{
        Accepted, Command, CommandError, CommandState, CommandStore, NewCommand, Outcome,
        MAX_ATTEMPTS,
    },
    folder_locks::Section,
    folders::{has_parent_dir, lexical},
    settings::SettingsStore,
    Clock, Millis,
};

use super::receive_once;
use crate::{
    context::TransmissionLink,
    store::channels::{ChannelStore, Rule, RuleState},
};
use trss_library::{
    live::LiveWatch,
    store::library::{LibraryError, LibraryStore},
    watch,
};
use trss_transmission as transmission;

use work_folder::{
    move_work_folder, Disk, Hold, MoveError, MovePolicy, Moved, RealDisk, Request, Side,
};

/// What `rule_archive` and `work_archive` use (made from
/// [`CollectContext::archive`](crate::context::CollectContext::archive)).
/// Cheap to clone.
#[derive(Clone)]
pub struct ArchiveContext {
    /// The rule, and the other rules that save in its work folder.
    pub channels: ChannelStore,
    /// Where the collect and archive folders are read from.
    pub settings: SettingsStore,
    /// The works the library knows, which follow the move.
    pub library: LibraryStore,
    /// The inotify watches of the watch folders, which the move tells.
    pub live: LiveWatch,
    pub transmission: TransmissionLink,
    /// How the move waits for Transmission.
    pub moves: MovePolicy,
    pub redactor: transmission::Redactor,
    /// Where a `start` accepts the receives of the items it carries.
    pub commands: CommandStore,
}

/// The `kind` of the command.
pub const KIND: &str = "rule_archive";

/// The outcome's `result` when the work folder is where the command puts it:
/// moved now, or found there already.
pub const MOVED: &str = "moved";
/// The outcome's `result` when the rule changed state and its folder was left
/// where it is on purpose; the reason says why.
pub const KEPT: &str = "kept";
/// The outcome's `result` of a command that ended `failed`.
pub const FAILED: &str = "failed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "ArchiveDirection"))]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// A new rule or subscription: into the collect folder when its work
    /// folder is in the archive folder, then on. The web made the rule paused
    /// for the time between, which counts as if it had collected all along: the
    /// rule is not noted as resumed.
    Start,
    /// `영상 받기` on, for a rule whose work folder is in the archive folder:
    /// into the collect folder, then on. The rule is noted as resumed when the
    /// switch was pressed, as the switch does for a rule that needs no move.
    Resume,
}

impl Direction {
    pub fn code(self) -> &'static str {
        match self {
            Direction::Start => "start",
            Direction::Resume => "resume",
        }
    }
}

/// The content of a `rule_archive` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct RuleArchive {
    pub rule_id: String,
    pub direction: Direction,
    /// With a `start` of a subscription: the past items the person ticked, to
    /// receive in this order once the rule is on ([`receive_ticked`]). Empty
    /// otherwise, and then left out of the stored text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub receive: Vec<i64>,
}

impl RuleArchive {
    /// The canonical text stored with the command and compared to tell a
    /// repeat of a request from a different one.
    pub fn canonical(&self) -> String {
        serde_json::to_string(self).expect("a payload serializes")
    }

    /// The subject stored with the command: the rule. One rule has at most one
    /// open command, whichever its direction.
    pub fn subject(&self) -> String {
        self.rule_id.clone()
    }
}

/// How an executed command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub state: CommandState,
    pub outcome: Outcome,
}

/// A command that could not be carried through now and is left `running` for
/// the next look, which carries on from what the disk shows.
#[derive(Debug, thiserror::Error)]
pub enum Retry {
    #[error("cannot read or write the app database: {0}")]
    Store(String),
    #[error("shutdown was asked for while the folder was moving")]
    Stopped,
    /// Not finished yet (Transmission is still moving); the next look carries on.
    #[error("{0}")]
    Later(String),
}

impl Retry {
    fn store(err: impl std::fmt::Display) -> Retry {
        Retry::Store(err.to_string())
    }
}

pub(super) fn done(result: &str, reason: Option<String>) -> Finished {
    Finished {
        state: CommandState::Done,
        outcome: Outcome {
            result: result.to_owned(),
            reason,
        },
    }
}

pub(super) fn failed(reason: impl Into<String>) -> Finished {
    Finished {
        state: CommandState::Failed,
        outcome: Outcome {
            result: FAILED.to_owned(),
            reason: Some(reason.into()),
        },
    }
}

const RULE_GONE: &str = "규칙을 찾지 못했어요. 삭제됐을 수 있어요.";

/// Where a rule's save folder is, seen from the collect folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkFolder {
    /// The first part of the save folder below the collect folder.
    Named(String),
    /// The rule saves into the collect folder itself.
    CollectItself,
    /// The save folder is not inside the collect folder.
    Outside,
}

/// Where the rule saving to `directory` (relative to `collect`, or absolute)
/// keeps its files, by path components. A directory with a `..` component is
/// [`WorkFolder::Outside`]: its text does not say where it lands once links
/// are followed (see [`trss_core::folders::has_parent_dir`]), so its folder is
/// never moved.
pub fn work_folder(collect: &Path, directory: &str) -> WorkFolder {
    if has_parent_dir(Path::new(directory)) {
        return WorkFolder::Outside;
    }
    lexical_work_folder(collect, directory)
}

/// [`work_folder`] with `..` resolved by text: where a rule would land if no
/// link were involved. What says which rules save into a work folder, to list
/// them, delete them with it, or keep it in place for them; never what says
/// which folder to move.
pub fn lexical_work_folder(collect: &Path, directory: &str) -> WorkFolder {
    let collect = lexical(collect);
    let save = lexical(&collect.join(directory));
    let Ok(below) = save.strip_prefix(&collect) else {
        return WorkFolder::Outside;
    };
    match below.components().next() {
        None => WorkFolder::CollectItself,
        Some(Component::Normal(name)) => match name.to_str() {
            Some(name) => WorkFolder::Named(name.to_owned()),
            None => WorkFolder::Outside,
        },
        Some(_) => WorkFolder::Outside,
    }
}

/// The sentence for a work folder kept because rules that collect still save
/// in it, the first of them named.
pub(super) fn held_reason(holders: &[&Rule]) -> String {
    let first = &holders[0].directory;
    match holders.len() {
        1 => format!("‘{first}’ 규칙이 아직 이 작품 폴더에 받고 있어서 옮기지 않았어요."),
        n => format!(
            "‘{first}’ 규칙 외 {}개가 아직 이 작품 폴더에 받고 있어서 옮기지 않았어요.",
            n - 1
        ),
    }
}

/// The turn the command takes before it runs: a write of the rule's work
/// folder in the collect folder and in the archive folder (the ones that are
/// set). Empty when there is no work folder to move (no collect folder, a rule
/// that saves into the collect folder itself or outside it) or the request or
/// the rule cannot be found: the command then changes the rule's state alone,
/// or ends by itself.
pub async fn section(ctx: &ArchiveContext, command: &Command) -> Result<Section, Retry> {
    let Ok(payload) = serde_json::from_str::<RuleArchive>(&command.payload) else {
        return Ok(Section::new());
    };
    let Some(rule) = ctx
        .channels
        .get_rule(&payload.rule_id)
        .await
        .map_err(Retry::store)?
    else {
        return Ok(Section::new());
    };
    let Some((collect, archive)) = settings(ctx).await? else {
        return Ok(Section::new());
    };
    let WorkFolder::Named(name) = work_folder(Path::new(&collect), &rule.directory) else {
        return Ok(Section::new());
    };
    let section = Section::new().write(Path::new(&collect).join(&name));
    Ok(match archive {
        Some(archive) => section.write(Path::new(&archive).join(&name)),
        None => section,
    })
}

/// One start of a command, and what its moves need.
pub(super) struct Start<'a> {
    pub(super) ctx: &'a ArchiveContext,
    pub(super) disk: Arc<dyn Disk>,
    /// Kept until the move's blocking work returns: the worker's turn at the
    /// folders and the torrent gate.
    pub(super) hold: Hold,
    /// The last start the command gets: a move not finished yet ends it.
    pub(super) last: bool,
    pub(super) cancel: &'a CancellationToken,
    /// Notes when a rule the move turns on was turned back on.
    pub(super) clock: &'a Clock,
    /// When the command was accepted: a `영상 받기` pressed then is on from then.
    pub(super) requested_at: Millis,
}

/// Runs a `rule_archive` command to its end, with its turn ([`section`]) taken.
/// See the module docs. `hold` keeps the folders' turn and the torrent gate
/// until the move's blocking work returns.
pub async fn run(
    ctx: &ArchiveContext,
    command: &Command,
    hold: Hold,
    clock: &Clock,
    cancel: &CancellationToken,
) -> Result<Finished, Retry> {
    run_on(ctx, command, Arc::new(RealDisk), hold, clock, cancel).await
}

/// [`run`] with the filesystems told by `disk`.
pub async fn run_on(
    ctx: &ArchiveContext,
    command: &Command,
    disk: Arc<dyn Disk>,
    hold: Hold,
    clock: &Clock,
    cancel: &CancellationToken,
) -> Result<Finished, Retry> {
    let start = Start {
        ctx,
        disk,
        hold,
        last: command.attempts >= MAX_ATTEMPTS,
        cancel,
        clock,
        requested_at: command.created_at,
    };
    let Ok(payload) = serde_json::from_str::<RuleArchive>(&command.payload) else {
        return Ok(failed("요청 내용을 읽지 못했어요."));
    };
    let Some(rule) = ctx
        .channels
        .get_rule(&payload.rule_id)
        .await
        .map_err(Retry::store)?
    else {
        return Ok(failed(RULE_GONE));
    };

    bring_in(&start, rule, &command.id, &payload).await
}

/// Why a rule's work folder is not moved at all (the command then ends with
/// that reason, `kept`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoMove {
    NoCollectFolder,
    NoArchiveFolder,
    CollectItself,
    Outside,
}

impl NoMove {
    /// The sentence a command ends with.
    pub fn reason(self) -> String {
        match self {
            NoMove::NoCollectFolder => "수집 폴더를 정하지 않아서 폴더는 옮기지 않았어요.",
            NoMove::NoArchiveFolder => "보관 폴더를 정하지 않아서 폴더는 옮기지 않았어요.",
            NoMove::CollectItself => "저장 폴더가 수집 폴더 자체라서 옮길 작품 폴더가 없어요.",
            NoMove::Outside => "저장 폴더가 수집 폴더 밖이라서 옮기지 않았어요.",
        }
        .to_owned()
    }

    /// The same, said before the rule is archived.
    pub(super) fn forecast(self) -> &'static str {
        match self {
            NoMove::NoCollectFolder => "수집 폴더를 정하지 않아서 폴더는 옮기지 않아요.",
            NoMove::NoArchiveFolder => "보관 폴더를 정하지 않아서 폴더는 옮기지 않아요.",
            NoMove::CollectItself => "저장 폴더가 수집 폴더 자체라서 옮길 작품 폴더가 없어요.",
            NoMove::Outside => "저장 폴더가 수집 폴더 밖이라서 옮기지 않아요.",
        }
    }
}

/// The move of a rule's work folder from the archive folder into the collect
/// folder, or why there is none (the command then ends with that reason,
/// `kept`).
fn plan_bring_in(
    settings: Option<(String, Option<String>)>,
    rule: &Rule,
) -> Result<Request, NoMove> {
    let Some((collect, archive)) = settings else {
        return Err(NoMove::NoCollectFolder);
    };
    let Some(archive) = archive else {
        return Err(NoMove::NoArchiveFolder);
    };
    let name = match work_folder(Path::new(&collect), &rule.directory) {
        WorkFolder::Named(name) => name,
        WorkFolder::CollectItself => return Err(NoMove::CollectItself),
        WorkFolder::Outside => return Err(NoMove::Outside),
    };
    Ok(Request {
        from_root: PathBuf::from(archive),
        from: Side::Archive,
        to_root: PathBuf::from(collect),
        to: Side::Collect,
        name,
    })
}

/// What a rule that is about to collect needs first ([`plan_start`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartPlan {
    /// Nothing to bring in: turn the rule on now.
    Now,
    /// The work folder `work` is in the archive folder (or it cannot be told it
    /// is not): the rule stays off until a `start` command has moved it into
    /// the collect folder.
    MoveFirst { work: String },
}

/// Whether a rule saving to `directory` has to wait for its work folder to come
/// out of the archive folder before it collects: when the archive folder is set,
/// the rule has a work folder, and the archive folder holds an entry of that
/// name. `settings` are the collect folder and the archive folder.
///
/// This reads the disk (one `lstat`), as the move does; the screens' notice
/// before a rule is made reads the library instead ([`archived_work`]). An
/// entry that cannot be told apart from a missing one (the disk says anything
/// but "not found") counts as there, so the rule waits and the command reports
/// what it finds, rather than the rule collecting beside a work folder it
/// cannot see.
pub async fn plan_start(settings: Option<(String, Option<String>)>, directory: &str) -> StartPlan {
    let Some((collect, Some(archive))) = settings else {
        return StartPlan::Now;
    };
    let WorkFolder::Named(name) = work_folder(Path::new(&collect), directory) else {
        return StartPlan::Now;
    };
    let entry = Path::new(&archive).join(&name);
    let found = tokio::task::spawn_blocking(move || match std::fs::symlink_metadata(entry) {
        Ok(_) => true,
        Err(err) => err.kind() != std::io::ErrorKind::NotFound,
    })
    .await
    // A panic of the blocking task tells nothing: wait for the command.
    .unwrap_or(true);
    if found {
        StartPlan::MoveFirst { work: name }
    } else {
        StartPlan::Now
    }
}

/// A work of the archive folder that a new rule would bring into the collect
/// folder, as the screens tell before the rule is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedWork {
    /// The work folder's name.
    pub work: String,
    pub archive_folder: String,
    pub collect_folder: String,
    /// The collect folder holds a work of that name too: the archive's work
    /// merges into it.
    pub merges: bool,
}

/// The work a rule saving to `directory` would bring out of the archive folder,
/// judged by the library's records of the two watch folders (a work by folder
/// name), not by the disk: a screen asks it on every keystroke. `None` when
/// there is no such work, no archive folder, or the rule has no work folder.
pub async fn archived_work(
    settings: Option<(String, Option<String>)>,
    library: &LibraryStore,
    directory: &str,
) -> Result<Option<ArchivedWork>, LibraryError> {
    let Some((collect, Some(archive))) = settings else {
        return Ok(None);
    };
    let WorkFolder::Named(name) = work_folder(Path::new(&collect), directory) else {
        return Ok(None);
    };
    let collect_folder = collect.trim_end_matches('/').to_owned();
    let archive_folder = archive.trim_end_matches('/').to_owned();
    if library
        .work_in_folder(&archive_folder, &name)
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let merges = library
        .work_in_folder(&collect_folder, &name)
        .await?
        .is_some();
    Ok(Some(ArchivedWork {
        work: name,
        archive_folder,
        collect_folder,
        merges,
    }))
}

/// Stores the `start` command of a new rule, or the `resume` command of a
/// `영상 받기` switched on, for `rule_id` (the web makes it itself, as it does
/// the read of Anissia's captions). Its ID is made of the direction, the rule
/// and the time. `Accepted::Busy` when the rule has another `rule_archive`
/// command open.
pub async fn ask_start(
    commands: &CommandStore,
    rule_id: &str,
    direction: Direction,
    now: Millis,
) -> Result<Accepted, CommandError> {
    ask_start_receiving(commands, rule_id, direction, Vec::new(), now).await
}

/// [`ask_start`] carrying the past items `receive` to receive, in this order,
/// once the rule is on ([`RuleArchive::receive`]).
pub async fn ask_start_receiving(
    commands: &CommandStore,
    rule_id: &str,
    direction: Direction,
    receive: Vec<i64>,
    now: Millis,
) -> Result<Accepted, CommandError> {
    let payload = RuleArchive {
        rule_id: rule_id.to_owned(),
        direction,
        receive,
    };
    commands
        .accept(
            NewCommand {
                id: format!("{}-{rule_id}-{now}", direction.code()),
                kind: KIND.to_owned(),
                payload: payload.canonical(),
                subject: Some(payload.subject()),
            },
            now,
        )
        .await
}

/// What a path that is about to add a torrent into a rule's save folder does
/// first ([`move_before_receiving`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Receiving {
    /// The work folder is not in the archive folder: receive.
    Go,
    /// The work folder `work` is in the archive folder (or it cannot be told it
    /// is not). Nothing is received now: the item is left, and a `start`
    /// command is bringing the folder into the collect folder. `asked` is set
    /// when this call stored that command and paused the rule for it; it is
    /// not when another `rule_archive` command of the rule was open already,
    /// which this call leaves as it is.
    MoveFirst { work: String, asked: bool },
}

/// Why [`move_before_receiving`] could not decide or ask. The item is left for
/// a later look, as for a move.
#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    #[error("cannot read the collect folder: {0}")]
    Settings(String),
    #[error("cannot store the command that brings the work folder in: {0}")]
    Command(String),
}

/// The sentence a command that adds an item ends with when the item was left
/// because the rule's work folder is moving into the collect folder first.
pub const MOVING_FIRST: &str = "작품 폴더가 보관 폴더에 있어서 먼저 수집 폴더로 옮기고 있어요. 다 옮긴 뒤 다시 받기를 다시 누를 수 있어요.";

/// The one rule for every path that adds a torrent into the save folder of the
/// active rule `rule`: a work is never split across the collect folder and the
/// archive folder, so nothing is received while the archive folder holds the
/// rule's work folder ([`plan_start`]).
///
/// When it does, the rule is paused and a `start` command is stored for it (as
/// the web does for a rule it turns on), so the folder is moved in before the
/// rule collects: the rule keeps its `resumed_at`, and the items seen meanwhile
/// are received afterwards. An existing split (the collect folder holds the work
/// too) is the same case: the move merges the archive's folder into the collect
/// one.
///
/// The caller holds the work folder's turn (a read of it), so the command, which
/// needs write turns on both folders, runs after the caller lets go: never wait
/// for it here. The caller leaves the item, records nothing for it, and wakes the
/// command runner when `asked` is set.
///
/// Another open command of the rule (`rule_archive` has one per rule) is left to
/// finish and the rule as it is.
pub async fn move_before_receiving(
    channels: &ChannelStore,
    settings: &SettingsStore,
    commands: &CommandStore,
    rule: &Rule,
    now: Millis,
) -> Result<Receiving, GuardError> {
    let folders = settings
        .collection()
        .await
        .map_err(|err| GuardError::Settings(err.to_string()))?
        .map(|s| (s.folder, s.archive_folder));
    let StartPlan::MoveFirst { work } = plan_start(folders, &rule.directory).await else {
        return Ok(Receiving::Go);
    };
    // The command is stored first: a rule paused with no command to turn it on
    // again would stay off.
    let accepted = ask_start(commands, &rule.id, Direction::Start, now)
        .await
        .map_err(|err| GuardError::Command(err.to_string()))?;
    let asked = matches!(accepted, Accepted::Created(_));
    if asked {
        // Left active, the command's turning the rule on changes nothing.
        if let Err(err) = channels
            .set_rule_state(&rule.id, RuleState::Paused, now)
            .await
        {
            eprintln!(
                "Cannot pause rule {} for its work folder's move: {err}",
                rule.id
            );
        }
    }
    Ok(Receiving::MoveFirst { work, asked })
}

pub(super) async fn settings(
    ctx: &ArchiveContext,
) -> Result<Option<(String, Option<String>)>, Retry> {
    Ok(ctx
        .settings
        .collection()
        .await
        .map_err(Retry::store)?
        .map(|s| (s.folder, s.archive_folder)))
}

/// Moves the work folder of `request`, as the outcome of the command.
pub(super) async fn move_folder(
    start: &Start<'_>,
    request: &Request,
) -> Result<Result<Finished, Finished>, Retry> {
    let ctx = start.ctx;
    let mut client = ctx.transmission.client();
    println!(
        "Moving work folder {:?} from {} to {}",
        request.name,
        request.from_root.display(),
        request.to_root.display()
    );
    let moved = move_work_folder(
        &mut client,
        &ctx.redactor,
        request,
        ctx.moves,
        start.disk.clone(),
        start.hold.clone(),
        start.cancel,
    )
    .await;
    if matches!(moved, Ok(Moved::Moved | Moved::AlreadyThere)) {
        // The work the library knows under the old place is this one: keep its
        // ID (see `watch::follow_move`). A failure leaves the command to run
        // again, which finds the folder moved already and follows it then.
        let followed = watch::follow_move(
            &ctx.library,
            &ctx.live,
            request.from_root.clone(),
            request.to_root.clone(),
            request.name.clone(),
        )
        .await
        .map_err(Retry::store)?;
        println!(
            "Library: work {:?} after the move: {followed:?}",
            request.name
        );
    }
    match moved {
        Ok(Moved::Moved) => Ok(Ok(done(MOVED, None))),
        Ok(Moved::AlreadyThere) => Ok(Ok(done(
            MOVED,
            Some(format!(
                "작품 폴더 `{}`가 이미 {}에 있어요.",
                request.name,
                request.to.name()
            )),
        ))),
        Ok(Moved::Nowhere) => Ok(Ok(done(
            KEPT,
            Some(format!(
                "작품 폴더 `{}`가 {}에 없어서 옮길 것이 없었어요.",
                request.name,
                request.from.name()
            )),
        ))),
        Err(MoveError::Failed(reason)) => Ok(Err(failed(reason))),
        Err(MoveError::Later(reason)) if start.last => Ok(Err(failed(reason))),
        Err(MoveError::Later(reason)) => Err(Retry::Later(reason)),
        Err(MoveError::Stopped) => Err(Retry::Stopped),
    }
}

/// A start or a resume: the work folder comes into the collect folder, and the
/// rule is turned on once it has. A start then receives the items it carries
/// ([`receive_ticked`]).
async fn bring_in(
    start: &Start<'_>,
    rule: Rule,
    command_id: &str,
    payload: &RuleArchive,
) -> Result<Finished, Retry> {
    let ctx = start.ctx;
    let direction = payload.direction;
    let settings = settings(ctx).await?;
    let finished = match plan_bring_in(settings, &rule) {
        Ok(request) => match move_folder(start, &request).await? {
            Ok(finished) => finished,
            // The rule stays paused: the folder is not in the collect folder.
            Err(failed) => return Ok(failed),
        },
        Err(no_move) => done(KEPT, Some(no_move.reason())),
    };

    // On only once the folder is back.
    let on = match direction {
        // The rule counts as collecting since it was made.
        Direction::Start => ctx.channels.begin_rule(&rule.id).await,
        // A switch is on from when it was pressed.
        Direction::Resume => {
            ctx.channels
                .set_rule_state(&rule.id, RuleState::Active, start.requested_at)
                .await
        }
    };
    let Some(rule) = on.map_err(Retry::store)? else {
        return Ok(failed(RULE_GONE));
    };
    if direction != Direction::Start || payload.receive.is_empty() {
        return Ok(finished);
    }
    let asked = receive_ticked(start, &rule, command_id, &payload.receive).await?;
    Ok(Finished {
        outcome: Outcome {
            reason: Some(match finished.outcome.reason {
                Some(reason) => format!("{reason} {asked}"),
                None => asked,
            }),
            ..finished.outcome
        },
        ..finished
    })
}

/// Accepts a `receive_once` of `rule` for each of `items`, in order, and says so
/// in a sentence for the command's outcome. Each one's ID is made of the start's
/// and the item, so a start run again after it was cut short accepts each once
/// (a repeat is `Existing`). An item with another receive open already is left
/// to it.
async fn receive_ticked(
    start: &Start<'_>,
    rule: &Rule,
    command_id: &str,
    items: &[i64],
) -> Result<String, Retry> {
    for &item in items {
        let payload = receive_once::ReceiveOnce::by_rule(item, rule.id.clone());
        let accepted = start
            .ctx
            .commands
            .accept(
                NewCommand {
                    id: format!("{command_id}-receive-{item}"),
                    kind: receive_once::KIND.to_owned(),
                    payload: payload.canonical(),
                    subject: Some(payload.subject()),
                },
                (start.clock)(),
            )
            .await
            .map_err(Retry::store)?;
        if let Accepted::Mismatch(_) | Accepted::Busy(_) = accepted {
            println!(
                "Rule {}: item {item} ticked when subscribing has another receive open",
                rule.id
            );
        }
    }
    Ok(format!(
        "구독할 때 체크한 지난 항목 {}개를 이어서 받아요.",
        items.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_work_folder_is_the_first_part_below_the_collect_folder() {
        let collect = Path::new("/downloads/Shows (current)");
        assert_eq!(
            work_folder(collect, "Clevatess/Season 02"),
            WorkFolder::Named("Clevatess".into())
        );
        assert_eq!(
            work_folder(collect, "Clevatess"),
            WorkFolder::Named("Clevatess".into())
        );
        assert_eq!(
            work_folder(collect, "./Clevatess//Season 02/"),
            WorkFolder::Named("Clevatess".into())
        );
        assert_eq!(work_folder(collect, ""), WorkFolder::CollectItself);
        assert_eq!(work_folder(collect, "."), WorkFolder::CollectItself);
        assert_eq!(work_folder(collect, "../Shows/X"), WorkFolder::Outside);
        // `..` is not placed by its text.
        assert_eq!(
            work_folder(collect, "Other/../Clevatess/Season 02"),
            WorkFolder::Outside
        );
        assert_eq!(
            work_folder(collect, "/downloads/Shows (current)/Other/S1"),
            WorkFolder::Named("Other".into())
        );
        assert_eq!(work_folder(collect, "/elsewhere/X"), WorkFolder::Outside);
        // Components, not text: `Shows (current)x` is not inside.
        assert_eq!(
            work_folder(Path::new("/d/Shows"), "/d/Showsx/X"),
            WorkFolder::Outside
        );
    }

    #[test]
    fn a_payload_is_the_rule_and_a_direction() {
        let payload = RuleArchive {
            rule_id: "r1".into(),
            direction: Direction::Resume,
            receive: Vec::new(),
        };
        assert_eq!(
            payload.canonical(),
            r#"{"rule_id":"r1","direction":"resume"}"#
        );
        assert!(serde_json::from_str::<RuleArchive>(
            r#"{"rule_id":"r1","direction":"start","folder":"x"}"#
        )
        .is_err());
        // `보관` and `복원` are no directions any more: a command stored with
        // one before cannot be read, and ends saying so.
        for gone in ["archive", "restore"] {
            assert!(serde_json::from_str::<RuleArchive>(&format!(
                r#"{{"rule_id":"r1","direction":"{gone}"}}"#
            ))
            .is_err());
        }
        assert!(
            serde_json::from_str::<RuleArchive>(r#"{"rule_id":"r1","direction":"away"}"#).is_err()
        );
    }

    #[test]
    fn a_kept_folder_names_the_rule_still_saving_there() {
        let rule = |directory: &str| Rule {
            id: directory.into(),
            channel_id: "c".into(),
            position: 0,
            version: 1,
            r#match: None,
            regex: false,
            case_insensitive: false,
            directory: directory.into(),
            episode: 1,
            episode_auto: false,
            state: RuleState::Active,
            subscription: None,
            resumed_at: None,
        };
        let one = rule("Clevatess/Season 03");
        assert_eq!(
            held_reason(&[&one]),
            "‘Clevatess/Season 03’ 규칙이 아직 이 작품 폴더에 받고 있어서 옮기지 않았어요."
        );
        let two = rule("Clevatess/Season 04");
        assert_eq!(
            held_reason(&[&one, &two]),
            "‘Clevatess/Season 03’ 규칙 외 1개가 아직 이 작품 폴더에 받고 있어서 옮기지 않았어요."
        );
    }
}
