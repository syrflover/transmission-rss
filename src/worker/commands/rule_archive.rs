//! `rule_archive`, shown on the screen as `보관` and `복원` of a rule: turns a
//! rule off or on and moves its work folder between the collect folder and
//! the archive folder (`docs/specs/collection.md`, 규칙 보관 and 보관과 복원의
//! 폴더 이동).
//!
//! The web accepts the command with a [`RuleArchive`] payload (the rule and a
//! [`Direction`]); the worker runs it with [`run`], under the lock that also
//! guards the collection cycles, so no cycle adds a torrent while a folder
//! moves. The web never changes a rule's state itself: the order below is
//! the worker's.
//!
//! **Archive** turns the rule off first, so nothing new arrives at the old
//! place while it moves, and then moves the work folder (the first part of the
//! rule's save folder, `Clevatess` of `Clevatess/Season 02`) into the archive
//! folder, unless:
//!
//! - no archive folder is set, or no collect folder: the rule is only
//!   archived;
//! - the rule saves into the collect folder itself (or outside it): there is no
//!   work folder;
//! - another active rule, of any channel, saves into the same work folder:
//!   the folder stays, and the outcome names that rule. It moves when the last
//!   of them is archived.
//!
//! A move that fails leaves the rule archived, and the command ends `failed`
//! with the reason; archiving again (`다시 옮기기`) tries again. The checks
//! refuse before anything moves; a failure after Transmission began moving
//! leaves what moved where it is, says so, and the next try moves the rest.
//!
//! **Restore** moves the work folder back into the collect folder first and
//! then turns the rule on, so a new episode never makes a second work folder
//! in the collect folder that the move back would run into. A move that fails
//! leaves the rule archived, with the reason. When the work folder is in the
//! collect folder already, or in neither, the rule is just turned on.
//!
//! Every step is safe to repeat, and where the folder is comes from the disk
//! each time ([`work_folder`]), so a command cut short (the worker stopped or
//! died) is claimed again by the next worker and finishes what is left. So is
//! a start that waited for Transmission longer than [`work_folder::MovePolicy`]
//! allows ([`Retry::Later`]): the command stays `running` for the next look,
//! and only its last start ([`MAX_ATTEMPTS`]) ends it `failed`, with that
//! reason.

pub mod work_folder;

use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{
    folders::has_parent_dir,
    store::{
        channels::{Rule, RuleState},
        commands::{Command, CommandState, Outcome, MAX_ATTEMPTS},
    },
    transmission,
    worker::CycleContext,
};

use work_folder::{move_work_folder, Disk, Hold, MoveError, Moved, RealDisk, Request, Side};

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
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// `보관`: off, then into the archive folder.
    Archive,
    /// `복원`: back into the collect folder, then on.
    Restore,
}

impl Direction {
    pub fn code(self) -> &'static str {
        match self {
            Direction::Archive => "archive",
            Direction::Restore => "restore",
        }
    }
}

/// The content of a `rule_archive` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleArchive {
    pub rule_id: String,
    pub direction: Direction,
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

fn done(result: &str, reason: Option<String>) -> Finished {
    Finished {
        state: CommandState::Done,
        outcome: Outcome {
            result: result.to_owned(),
            reason,
        },
    }
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

/// `path` with `.` and `..` resolved by text alone.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Where the rule saving to `directory` (relative to `collect`, or absolute)
/// keeps its files, by path components. A directory with a `..` component is
/// [`WorkFolder::Outside`]: its text does not say where it lands once links
/// are followed (see [`crate::folders::has_parent_dir`]), so its folder is
/// never moved.
pub fn work_folder(collect: &Path, directory: &str) -> WorkFolder {
    if has_parent_dir(Path::new(directory)) {
        return WorkFolder::Outside;
    }
    lexical_work_folder(collect, directory)
}

/// [`work_folder`] with `..` resolved by text: where a rule would land if no
/// link were involved.
fn lexical_work_folder(collect: &Path, directory: &str) -> WorkFolder {
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

/// The active rules other than `rule` whose save folders are in `name`. A
/// save folder with `..` counts when its text lands there: the folder then
/// stays rather than leave a rule saving into a moved work folder.
fn holders<'a>(rules: &'a [Rule], rule: &Rule, collect: &Path, name: &str) -> Vec<&'a Rule> {
    rules
        .iter()
        .filter(|other| other.id != rule.id && other.state == RuleState::Active)
        .filter(|other| {
            lexical_work_folder(collect, &other.directory) == WorkFolder::Named(name.to_owned())
        })
        .collect()
}

/// The sentence for a work folder kept because other rules still save in it.
fn held_reason(holders: &[&Rule]) -> String {
    let first = &holders[0].directory;
    match holders.len() {
        1 => format!("‘{first}’ 규칙이 아직 이 작품 폴더에 받고 있어서 옮기지 않았어요."),
        n => format!(
            "‘{first}’ 규칙 외 {}개가 아직 이 작품 폴더에 받고 있어서 옮기지 않았어요.",
            n - 1
        ),
    }
}

/// One start of a command, and what its moves need.
struct Start<'a> {
    ctx: &'a CycleContext,
    disk: Arc<dyn Disk>,
    /// Kept until the move's blocking work returns: the worker's lock.
    hold: Hold,
    /// The last start the command gets: a move not finished yet ends it.
    last: bool,
    cancel: &'a CancellationToken,
}

/// Runs a `rule_archive` command to its end. See the module docs. `hold` is
/// the worker's lock, kept until the move's blocking work returns.
pub async fn run(
    ctx: &CycleContext,
    command: &Command,
    hold: Hold,
    cancel: &CancellationToken,
) -> Result<Finished, Retry> {
    run_on(ctx, command, Arc::new(RealDisk), hold, cancel).await
}

/// [`run`] with the filesystems told by `disk`.
pub async fn run_on(
    ctx: &CycleContext,
    command: &Command,
    disk: Arc<dyn Disk>,
    hold: Hold,
    cancel: &CancellationToken,
) -> Result<Finished, Retry> {
    let start = Start {
        ctx,
        disk,
        hold,
        last: command.attempts >= MAX_ATTEMPTS,
        cancel,
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

    match payload.direction {
        Direction::Archive => archive(&start, rule).await,
        Direction::Restore => restore(&start, rule).await,
    }
}

/// What the move of a rule's work folder would be, or why there is none (the
/// command then ends with that reason, `kept`).
fn plan_move(
    settings: Option<(String, Option<String>)>,
    rule: &Rule,
    direction: Direction,
) -> Result<Request, String> {
    let Some((collect, archive)) = settings else {
        return Err("수집 폴더를 정하지 않아서 폴더는 옮기지 않았어요.".to_owned());
    };
    let Some(archive) = archive else {
        return Err("보관 폴더를 정하지 않아서 폴더는 옮기지 않았어요.".to_owned());
    };
    let name = match work_folder(Path::new(&collect), &rule.directory) {
        WorkFolder::Named(name) => name,
        WorkFolder::CollectItself => {
            return Err("저장 폴더가 수집 폴더 자체라서 옮길 작품 폴더가 없어요.".to_owned())
        }
        WorkFolder::Outside => {
            return Err("저장 폴더가 수집 폴더 밖이라서 옮기지 않았어요.".to_owned())
        }
    };
    let (collect, archive) = (PathBuf::from(collect), PathBuf::from(archive));
    Ok(match direction {
        Direction::Archive => Request {
            from_root: collect,
            from: Side::Collect,
            to_root: archive,
            to: Side::Archive,
            name,
        },
        Direction::Restore => Request {
            from_root: archive,
            from: Side::Archive,
            to_root: collect,
            to: Side::Collect,
            name,
        },
    })
}

async fn settings(ctx: &CycleContext) -> Result<Option<(String, Option<String>)>, Retry> {
    Ok(ctx
        .settings
        .collection()
        .await
        .map_err(Retry::store)?
        .map(|s| (s.folder, s.archive_folder)))
}

/// Moves the work folder of `request`, as the outcome of the command.
async fn move_folder(
    start: &Start<'_>,
    request: &Request,
) -> Result<Result<Finished, Finished>, Retry> {
    let ctx = start.ctx;
    let mut client = transmission::client(ctx.transmission_url.clone(), &ctx.transmission_http);
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

async fn archive(start: &Start<'_>, rule: Rule) -> Result<Finished, Retry> {
    let ctx = start.ctx;
    // Off first: no new episode arrives at the old place while it moves.
    let Some(rule) = ctx
        .channels
        .set_rule_state(&rule.id, RuleState::Archived)
        .await
        .map_err(Retry::store)?
    else {
        return Ok(failed(RULE_GONE));
    };

    let settings = settings(ctx).await?;
    let request = match plan_move(settings.clone(), &rule, Direction::Archive) {
        Ok(request) => request,
        Err(reason) => return Ok(done(KEPT, Some(reason))),
    };

    let rules: Vec<Rule> = ctx
        .channels
        .list_channels_with_rules()
        .await
        .map_err(Retry::store)?
        .into_iter()
        .flat_map(|cwr| cwr.rules)
        .collect();
    let holding = holders(&rules, &rule, &request.from_root, &request.name);
    if !holding.is_empty() {
        return Ok(done(KEPT, Some(held_reason(&holding))));
    }

    Ok(move_folder(start, &request)
        .await?
        .unwrap_or_else(|failed| failed))
}

async fn restore(start: &Start<'_>, rule: Rule) -> Result<Finished, Retry> {
    let ctx = start.ctx;
    let settings = settings(ctx).await?;
    let finished = match plan_move(settings, &rule, Direction::Restore) {
        Ok(request) => match move_folder(start, &request).await? {
            Ok(finished) => finished,
            // The rule stays archived: the folder is not back.
            Err(failed) => return Ok(failed),
        },
        Err(reason) => done(KEPT, Some(reason)),
    };

    // On only once the folder is back.
    match ctx
        .channels
        .set_rule_state(&rule.id, RuleState::Active)
        .await
        .map_err(Retry::store)?
    {
        Some(_) => Ok(finished),
        None => Ok(failed(RULE_GONE)),
    }
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
            direction: Direction::Restore,
        };
        assert_eq!(
            payload.canonical(),
            r#"{"rule_id":"r1","direction":"restore"}"#
        );
        assert!(serde_json::from_str::<RuleArchive>(
            r#"{"rule_id":"r1","direction":"archive","folder":"x"}"#
        )
        .is_err());
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

        let archiving = rule("Clevatess/Season 02");
        let mut archived = rule("Clevatess/Season 01");
        archived.state = RuleState::Archived;
        let other = rule("Other/Season 01");
        let dotted = rule("Other/../Clevatess/Season 05");
        let rules = [
            archiving.clone(),
            archived,
            one.clone(),
            other,
            dotted.clone(),
        ];
        let found = holders(&rules, &archiving, Path::new("/c"), "Clevatess");
        let ids: Vec<&str> = found.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, [one.id.as_str(), dotted.id.as_str()]);
    }
}
