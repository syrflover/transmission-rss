//! `episode_undo`, shown on the screen as `되돌리기` beside an episode offset
//! the app set (`docs/specs/collection.md`, 영상 회차 변환): puts back the
//! value the rule had before and renames the videos the rule received under
//! the app's value to the names they get with it (`S03E01` → `S03E25`).
//!
//! The web accepts the command with an [`EpisodeUndo`] payload (the rule and
//! the automatic value the user saw; one rule has at most one open undo) and
//! the worker runs it with [`run`], holding the turn of the rule's work folder
//! ([`section`]): Transmission is reached from the worker only, and no cycle
//! or command receives an item into the folder or renames a file in it while
//! the undo runs. A cycle's item for the rule that waited for it finds the
//! offset changed and is left for the next cycle.
//!
//! # What is renamed
//!
//! The rule decided its offset before it picked anything, and the offset
//! stayed automatic since (any change of the value turns `자동` off), so every
//! item the rule received (`received` in history) was named under the
//! automatic value. For each, the names under the automatic value and under
//! the previous one are derived from its release title as the cycle derives
//! them ([`episode_name`]), in the folder of its torrent (or the rule's folder
//! when the torrent is gone). A video is planned only when it still has the
//! automatic name:
//!
//! - **Its torrent is in Transmission**, with one file of that name: renamed by
//!   Transmission's rename, so it keeps seeding under the new name. The file's
//!   identity is kept with the plan too.
//! - **Its torrent is gone** and a file of that name is in the folder that no
//!   torrent holds: renamed on disk with `RENAME_NOREPLACE`, and only if it is
//!   still the file planned ([`FileIdentity::unchanged`]: inode, size and
//!   times).
//!   An RSS title without an extension (which takes its torrent's otherwise)
//!   takes the one of the video of its name in the folder; when videos of
//!   that name have more than one extension, the item's is not known and the
//!   undo lists it as kept from the start ([`WHICH_FILE`]).
//!
//! A torrent of another name (a revision that kept its received name, a file
//! the person renamed) and a file another torrent holds are left out: they
//! were not named by this value, or are another item's.
//!
//! # Never over another file
//!
//! A rename onto a name that is taken is not made: the file keeps its name and
//! the screen says so (`이름을 되돌리지 못했어요` with the names), while the
//! other files go on. Before Transmission's rename the target is looked up on
//! disk; Transmission answers success without moving anything when the target
//! is there (see [`crate::receive::rename`]), so a target that
//! appeared in between leaves the source in place, and the torrent's name is
//! then put back. A rename on disk refuses an existing target by itself.
//!
//! Transmission renames by name, whatever file is there, so right before it
//! the torrent must be finished (not downloading or verifying), still in the
//! folder, and the file at its name must be the one planned (same inode:
//! [`FileId::same_file`](trss_core::file_id::FileId::same_file)); otherwise it keeps its name with the reason.
//! A name another torrent lists in the folder is not taken either, even while
//! its file is missing or still being written: Transmission would write that
//! torrent's file there. Nor is a file another torrent lists too
//! ([`SHARED`]): Transmission would move it from under that torrent.
//!
//! The file planned is known by its inode, never by its device number: an
//! undo may be carried on after a restart, and a file system mounted again
//! may give the same file another device number (user decision, 2026-10-07).
//!
//! The undo's own names overlap when the values differ by less than the
//! numbers it renames (`−36` back to `−24`: `S03E01` becomes `S03E13`, which
//! `S03E13` leaves for `S03E25`). The files go in an order that frees a name
//! before a file takes it ([`next`]).
//!
//! # Order, and a start cut short
//!
//! The previous value goes back, together with the plan, in one transaction
//! ([`ChannelStore::begin_episode_undo`](crate::store::channels::ChannelStore::begin_episode_undo)),
//! and is refused, changing nothing, when the offset is no longer the
//! automatic one the user saw, or when a video revision replacement is under
//! way for an episode in the plan. Then each file is renamed and recorded
//! with the revision rows of its episode, which move to the new name. A start
//! cut short (the worker stopped, Transmission did not answer) leaves the
//! command `running`; the next start finds the undo begun and carries on with
//! the files still `pending`. A torrent that has the new name already counts
//! as renamed only when its planned file is at the new name and nothing at the
//! old; when the file is still at the old name (Transmission named the torrent
//! without moving it), the torrent gets its old name back first.
//!
//! A start that finds a file renamed already (an earlier start renamed it
//! and stopped before recording it: nothing at the old name, the planned
//! file at the new one) records it renamed before anything else, so a
//! replacement that began on its names since does not hold it.
//!
//! A file that cannot be renamed yet waits, still `pending` with why: its
//! torrent is still downloading ([`UNFINISHED`]; one planned before its file
//! had its name gets its identity once complete), or a replacement acts on
//! it (`REVISION_UNDER_WAY`). A start that cannot reach Transmission or the
//! database after the undo began stops there. Either way the command ends
//! `done` with the outcome [`PAUSED`]: the value is back, and the files left
//! wait for a new request rather than holding up the commands behind this
//! one or retrying on every look. That request, for the same automatic
//! value, is accepted though the rule is not automatic any more: it takes
//! the files left ([`ChannelStore::adopt_episode_undo`](crate::store::channels::ChannelStore::adopt_episode_undo)),
//! and the rule detail shows them with `이어서 되돌리기`. So does an undo that
//! ended half done otherwise (a panic, or a database that failed for every
//! start). Only while the rule is not automatic: on a rule that is automatic
//! again (an import marked it so), `되돌리기` plans a new undo of the rule as
//! it is, and the files an older undo left wait no more
//! (`SUPERSEDED`, kept).
//!
//! A user who saves the rule while an undo is unfinished does not change it:
//! the files still `pending` take the names of the value put back (the
//! undo's `to`), not of the value saved since. Those files were named under
//! the automatic value, and the undo is about that value.
//!
//! Once undone, the offset is the user's own: `자동` is off, and the app never
//! decides the rule again, even when the rule has not received anything.

use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use transmission_rpc::types::Id;

use trss_core::{
    commands::{Command, CommandState, Outcome},
    files::{occupied, rename_noreplace},
    folder_locks::Section,
    settings::SettingsStore,
    Clock,
};

use crate::{
    context::TransmissionLink,
    episode_offset::signed,
    plan::rule_work_folder,
    revision::FileIdentity,
    revisions::{episode_name, owner_of, same_folder, Owner},
    rss::save_path,
    store::{
        channels::{ChannelStore, NewUndoFile, Rule, UndoBegun, UndoFileState, REVISION_UNDER_WAY},
        history::HistoryStore,
    },
};
use trss_library::discovery::VIDEO_EXTENSIONS;
use trss_transmission::{torrent_places, TorrentPlace};

/// What `episode_undo` uses (made from
/// [`CollectContext::undo`](crate::context::CollectContext::undo)). Cheap to
/// clone.
#[derive(Clone)]
pub struct UndoContext {
    /// The rule, its offset and the files the undo renames.
    pub channels: ChannelStore,
    /// The videos the rule received, which the undo renames.
    pub history: HistoryStore,
    /// Where the collect folder is read from.
    pub settings: SettingsStore,
    pub transmission: TransmissionLink,
}

/// The `kind` of the command.
pub const KIND: &str = "episode_undo";

/// The outcome's `result` when the previous value is back.
pub const UNDONE: &str = "undone";
/// The outcome's `result` of a command that ended `failed`.
pub const FAILED: &str = "failed";
/// The outcome's `result` when the previous value is back and some files are
/// still to rename: they wait for `이어서 되돌리기`.
pub const PAUSED: &str = "paused";

/// Why a file keeps its name: the name it would take is taken.
pub const TAKEN: &str = "같은 이름의 파일이 이미 있어요.";
/// Why a file keeps its name: its torrent's file has another name now.
pub const NAME_CHANGED: &str = "토렌트 파일의 이름이 그사이 바뀌었어요.";
/// Why a file keeps its name: it is not the file planned any more.
pub const FILE_CHANGED: &str = "되돌리기를 시작한 뒤 파일이 바뀌었어요.";
/// Why a file keeps its name: it is not there.
pub const MISSING: &str = "파일을 찾지 못했어요.";
/// Why a file waits, still `pending`: Transmission is still writing it.
pub const UNFINISHED: &str = "토렌트를 아직 받는 중이에요. 다 받은 뒤 이어서 되돌릴 수 있어요.";
/// Why a file keeps its name: another torrent's file has the name it would take.
pub const CLAIMED: &str = "다른 토렌트가 그 이름을 쓰고 있어요.";
/// Why a file keeps its name: another torrent lists the file too, and
/// Transmission would move it from under that one.
pub const SHARED: &str = "다른 토렌트도 이 파일을 쓰고 있어요.";
/// Why a file keeps its name: its torrent is in another folder now.
pub const MOVED: &str = "토렌트가 다른 폴더로 옮겨졌어요.";
/// Why a file keeps its name: its torrent is gone, the RSS title has no
/// extension, and more than one video in the folder could be the item's.
pub const WHICH_FILE: &str =
    "토렌트가 없고 항목 제목에 확장자가 없는데, 확장자만 다른 영상이 여러 개 있어서 어느 것인지 알 수 없어요.";

const NO_COLLECT_FOLDER: &str =
    "수집 폴더가 정해지지 않아서 영상이 어디 있는지 알 수 없어 되돌리지 않았어요.";

const RULE_CHANGED: &str =
    "회차 변환이 그사이 바뀌어서 되돌리지 않았어요. 화면을 새로고침해 주세요.";

/// The content of an `episode_undo` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeUndo {
    pub rule_id: String,
    /// The automatic value the user saw and asks to undo.
    pub episode: i64,
}

impl EpisodeUndo {
    /// The canonical text stored with the command and compared to tell a
    /// repeat of a request from a different one.
    pub fn canonical(&self) -> String {
        serde_json::to_string(self).expect("a payload serializes")
    }

    /// The subject stored with the command: the rule.
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

/// A command that could not be carried through now. Before the undo began it
/// is left `running` for the next look; after, the start ends [`PAUSED`].
#[derive(Debug, thiserror::Error)]
pub enum Retry {
    #[error("cannot read or write the app database: {0}")]
    Store(String),
    #[error("cannot reach Transmission: {0}")]
    Transmission(String),
}

fn store(err: impl std::fmt::Display) -> Retry {
    Retry::Store(err.to_string())
}

fn transmission(err: impl std::fmt::Display) -> Retry {
    Retry::Transmission(err.to_string())
}

/// Plans and begins a new undo of the automatic value `from` of `rule`:
/// the undo as begun, or how the command ends without one.
async fn begin(
    ctx: &UndoContext,
    command: &Command,
    rule: &Rule,
    from: i64,
    clock: &Clock,
) -> Result<Result<crate::store::channels::EpisodeUndo, Finished>, Retry> {
    if rule.episode != from {
        return Ok(Err(failed(RULE_CHANGED)));
    }
    let marks = ctx
        .channels
        .episode_marks(vec![rule.id.clone()])
        .await
        .map_err(store)?;
    let Some(to) = marks.get(&rule.id).and_then(|m| m.previous) else {
        return Ok(Err(failed(
            "앱이 정하기 전 값을 몰라서 되돌릴 수 없어요. 회차 변환을 직접 적어 주세요.",
        )));
    };
    let Some(collect) = ctx.settings.collection().await.map_err(store)? else {
        return Ok(Err(failed(NO_COLLECT_FOLDER)));
    };
    let files = plan(ctx, rule, &collect.folder, from, to).await?;
    Ok(
        match ctx
            .channels
            .begin_episode_undo(&command.id, &rule.id, from, to, files, clock())
            .await
            .map_err(store)?
        {
            UndoBegun::Begun(undo) => Ok(undo),
            UndoBegun::Changed => Err(failed(RULE_CHANGED)),
            UndoBegun::Busy(names) => Err(failed(format!(
                "수정본으로 대체하는 중인 영상이 있어서 되돌리지 않았어요: {}. \
                 대체가 끝난 뒤 다시 되돌려 주세요.",
                names.join(", ")
            ))),
        },
    )
}

/// The value is back and `left` files are still to rename, for `why`.
fn paused(to: i64, left: usize, why: &str) -> Finished {
    Finished {
        state: CommandState::Done,
        outcome: Outcome {
            result: PAUSED.to_owned(),
            reason: Some(format!(
                "회차 변환은 전의 값으로 돌아왔어요. 지금 값: {}. 남은 영상 {left}개는 {why} \
                 규칙 상세의 `이어서 되돌리기`로 마저 바꿀 수 있어요.",
                signed(to)
            )),
        },
    }
}

/// Whether a reason a file keeps its name passes: the file waits, `pending`,
/// for the next request rather than being left as it is.
fn passes(reason: &str) -> bool {
    reason == UNFINISHED || reason == REVISION_UNDER_WAY
}

/// Renames one pending file of the undo `command_id` and records how it
/// ended. `false` when it waits, still `pending`.
async fn carry_on(
    ctx: &UndoContext,
    command_id: &str,
    file: &NewUndoFile,
    listing: &mut Listing,
    clock: &Clock,
) -> Result<bool, Retry> {
    // Renamed by a start cut short before it recorded the file: that is how
    // the file is, whatever a cycle began on its names since.
    if renamed_before(ctx, file, listing).await? {
        println!(
            "Episode undo {command_id}: {} was {} already",
            file.to_name, file.from_name
        );
        ctx.channels
            .finish_undo_file(command_id, file.item_id, None, clock())
            .await
            .map_err(store)?;
        return Ok(true);
    }
    // A cycle may have run since the undo began (see the store's docs).
    let held = ctx
        .channels
        .undo_file_hold(&file.folder, &file.from_name, &file.to_name)
        .await
        .map_err(store)?;
    let kept = match held {
        Some(reason) => Some(reason.to_owned()),
        None => rename(ctx, command_id, file, listing).await?,
    };
    match &kept {
        None => println!(
            "Episode undo {command_id}: {} is now {}",
            file.from_name, file.to_name
        ),
        Some(why) => println!(
            "Episode undo {command_id}: {} keeps its name: {why}",
            file.from_name
        ),
    }
    if let Some(why) = kept.as_deref().filter(|why| passes(why)) {
        ctx.channels
            .wait_undo_file(command_id, file.item_id, why)
            .await
            .map_err(store)?;
        return Ok(false);
    }
    ctx.channels
        .finish_undo_file(command_id, file.item_id, kept, clock())
        .await
        .map_err(store)?;
    Ok(true)
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

/// The turn the command takes before it runs: a write of the work folder the
/// rule saves into ([`rule_work_folder`]). Empty when the request, the rule or
/// the collect folder cannot be found: the command then ends by itself.
pub async fn section(ctx: &UndoContext, command: &Command) -> Result<Section, Retry> {
    let Ok(payload) = serde_json::from_str::<EpisodeUndo>(&command.payload) else {
        return Ok(Section::new());
    };
    let Some(rule) = ctx
        .channels
        .get_rule(&payload.rule_id)
        .await
        .map_err(store)?
    else {
        return Ok(Section::new());
    };
    let Some(collect) = ctx.settings.collection().await.map_err(store)? else {
        return Ok(Section::new());
    };
    Ok(Section::new().write(rule_work_folder(Path::new(&collect.folder), &rule)))
}

/// Runs an `episode_undo` command to its end, with its turn ([`section`])
/// taken.
pub async fn run(ctx: &UndoContext, command: &Command, clock: &Clock) -> Result<Finished, Retry> {
    let Ok(payload) = serde_json::from_str::<EpisodeUndo>(&command.payload) else {
        return Ok(failed("요청 내용을 읽지 못했어요."));
    };
    let undo = match ctx
        .channels
        .episode_undo(&command.id)
        .await
        .map_err(store)?
    {
        // Begun by an earlier start of this command.
        Some(undo) => undo,
        None => {
            let Some(rule) = ctx
                .channels
                .get_rule(&payload.rule_id)
                .await
                .map_err(store)?
            else {
                return Ok(failed("규칙을 찾지 못했어요. 삭제됐을 수 있어요."));
            };
            let begun = if rule.episode_auto {
                // A new undo of the rule as it is.
                begin(ctx, command, &rule, payload.episode, clock).await?
            } else {
                // The value is back already: an undo of it with files left
                // (`이어서 되돌리기`) is carried on.
                match ctx
                    .channels
                    .unfinished_episode_undo(&rule.id)
                    .await
                    .map_err(store)?
                    .filter(|undo| undo.from == payload.episode)
                {
                    Some(earlier) => Ok(ctx
                        .channels
                        .adopt_episode_undo(&earlier.command_id, &command.id)
                        .await
                        .map_err(store)?
                        .unwrap_or(earlier)),
                    None => Err(failed(RULE_CHANGED)),
                }
            };
            match begun {
                Ok(undo) => undo,
                Err(ended) => return Ok(ended),
            }
        }
    };

    let mut listing = Listing::default();
    let mut pending: Vec<&NewUndoFile> = undo
        .files
        .iter()
        .filter(|f| f.state == UndoFileState::Pending)
        .map(|f| &f.file)
        .collect();
    let mut waiting = 0;
    while !pending.is_empty() {
        let file = pending.remove(next(&pending));
        match carry_on(ctx, &command.id, file, &mut listing, clock).await {
            Ok(true) => {}
            Ok(false) => waiting += 1,
            // The value is back: the files left wait for a new request
            // rather than holding up the commands behind this one.
            Err(err) => {
                eprintln!("Episode undo {} stops: {err}", command.id);
                let why = match err {
                    Retry::Transmission(_) => "Transmission에 닿지 못해서",
                    Retry::Store(_) => "앱 DB를 읽거나 쓰지 못해서",
                };
                return Ok(paused(
                    undo.to,
                    waiting + pending.len() + 1,
                    &format!("{why} 아직 이름을 바꾸지 않았어요."),
                ));
            }
        }
    }
    if waiting > 0 {
        return Ok(paused(
            undo.to,
            waiting,
            "아직 바꿀 수 없어서 기다려요. 까닭은 영상마다 적었어요.",
        ));
    }

    let done = ctx
        .channels
        .episode_undo(&command.id)
        .await
        .map_err(store)?
        .map(|u| u.files)
        .unwrap_or_default();
    let renamed = done
        .iter()
        .filter(|f| f.state == UndoFileState::Renamed)
        .count();
    let kept = done
        .iter()
        .filter(|f| f.state == UndoFileState::Kept)
        .count();
    let mut reason = format!(
        "회차 변환을 되돌렸어요. 지금 값: {}. 이름을 바꾼 영상: {renamed}개.",
        signed(undo.to)
    );
    if kept > 0 {
        reason.push_str(&format!(" 이름을 되돌리지 못한 영상: {kept}개."));
    }
    Ok(Finished {
        state: CommandState::Done,
        outcome: Outcome {
            result: UNDONE.to_owned(),
            reason: Some(reason),
        },
    })
}

/// The videos to rename (see the module docs). Reads Transmission's whole
/// file list once; a Transmission that cannot be reached leaves the command
/// for the next look.
async fn plan(
    ctx: &UndoContext,
    rule: &Rule,
    collect_folder: &str,
    from: i64,
    to: i64,
) -> Result<Vec<NewUndoFile>, Retry> {
    let items = ctx
        .history
        .received_of_rule(&rule.id)
        .await
        .map_err(store)?;
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let rule_folder = save_path(Path::new(collect_folder), Path::new(&rule.directory));
    let mut client = ctx.transmission.client();
    let places = torrent_places(&mut client, None, true)
        .await
        .map_err(transmission)?;

    let mut planned: Vec<NewUndoFile> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for item in items {
        let place = places
            .iter()
            .find(|p| p.hash.eq_ignore_ascii_case(&item.torrent_hash));
        let folder = match place {
            Some(place) if same_folder(Path::new(&place.download_dir), &rule_folder) => {
                rule_folder.clone()
            }
            Some(place) => PathBuf::from(&place.download_dir),
            None => rule_folder.clone(),
        };
        let (from_name, to_name) = match names(&folder, &item.title, place, from, to) {
            Some(names) => names,
            // The torrent is gone and the title has no extension: the videos
            // of the item's name, whatever their extension.
            None if place.is_none() => {
                let mut found = by_extension(&folder, &item.title, from, to, &places);
                match found.len() {
                    0 => continue,
                    1 => found.remove(0),
                    _ => {
                        let (from_name, to_name) = found.remove(0);
                        let path = folder.join(&from_name);
                        if seen.insert(path) {
                            planned.push(NewUndoFile {
                                item_id: item.id,
                                folder: folder.to_string_lossy().into_owned(),
                                from_name,
                                to_name,
                                torrent_hash: None,
                                identity: None,
                                kept: Some(WHICH_FILE.to_owned()),
                            });
                        }
                        continue;
                    }
                }
            }
            None => continue,
        };
        if from_name == to_name {
            continue;
        }
        let path = folder.join(&from_name);
        if seen.contains(&path) {
            continue;
        }
        let file = match place {
            Some(place) => {
                let single = matches!(&place.files[..], [only] if only.name == place.name);
                if !single || place.name != from_name {
                    continue;
                }
                NewUndoFile {
                    item_id: item.id,
                    folder: folder.to_string_lossy().into_owned(),
                    from_name,
                    to_name,
                    torrent_hash: Some(place.hash.clone()),
                    identity: FileIdentity::at(&path).ok().map(|id| id.to_text()),
                    kept: None,
                }
            }
            None => {
                let Ok(meta) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                // A file a torrent holds is that torrent's item's, not this one's.
                if !matches!(owner_of(&places, &path), Ok(Owner::Nobody)) {
                    continue;
                }
                NewUndoFile {
                    item_id: item.id,
                    folder: folder.to_string_lossy().into_owned(),
                    from_name,
                    to_name,
                    torrent_hash: None,
                    identity: Some(FileIdentity::of(&meta).to_text()),
                    kept: None,
                }
            }
        };
        seen.insert(path);
        planned.push(file);
    }
    Ok(planned)
}

/// The names the release `title` gets in `folder` under `from` and `to`. An
/// RSS title without an extension takes the one of its torrent's file.
fn names(
    folder: &Path,
    title: &str,
    place: Option<&TorrentPlace>,
    from: i64,
    to: i64,
) -> Option<(String, String)> {
    both(folder, title, from, to).or_else(|| {
        let ext = Path::new(&place?.name).extension()?.to_str()?;
        both(folder, &format!("{title}.{ext}"), from, to)
    })
}

fn both(folder: &Path, title: &str, from: i64, to: i64) -> Option<(String, String)> {
    Some((
        episode_name(folder, title, from as isize)?,
        episode_name(folder, title, to as isize)?,
    ))
}

/// For an RSS title without an extension whose torrent is gone: the names of
/// the videos in `folder` that have the item's name under `from` with a video
/// extension, are plain files, and belong to no torrent, sorted. One is the
/// item's file; more cannot tell which.
fn by_extension(
    folder: &Path,
    title: &str,
    from: i64,
    to: i64,
    places: &[TorrentPlace],
) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<(String, String)> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().into_string().ok()?;
            let ext = Path::new(&name).extension()?.to_str()?.to_owned();
            if !VIDEO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
                return None;
            }
            let (from_name, to_name) = both(folder, &format!("{title}.{ext}"), from, to)?;
            if from_name != name || !entry.file_type().ok()?.is_file() {
                return None;
            }
            matches!(owner_of(places, &entry.path()), Ok(Owner::Nobody))
                .then_some((from_name, to_name))
        })
        .collect();
    found.sort();
    found
}

/// Which of the `pending` files to rename next: the first whose new name is
/// not the old name of another one still to rename, so that a name the undo
/// frees is free before a file takes it (`S03E13` goes to `S03E25` before
/// `S03E01` takes `S03E13`). When every one waits for another (names that
/// swap), the first goes, and is kept for the name being taken.
fn next(pending: &[&NewUndoFile]) -> usize {
    pending
        .iter()
        .position(|file| {
            !pending.iter().any(|other| {
                !std::ptr::eq(*other, *file)
                    && other.folder == file.folder
                    && other.from_name == file.to_name
            })
        })
        .unwrap_or(0)
}

/// Transmission's torrents with their files, read when first needed and read
/// again after a rename through Transmission changed them.
#[derive(Default)]
struct Listing {
    places: Option<Vec<TorrentPlace>>,
}

impl Listing {
    async fn get(&mut self, ctx: &UndoContext) -> Result<&[TorrentPlace], Retry> {
        if self.places.is_none() {
            let mut client = ctx.transmission.client();
            let places = torrent_places(&mut client, None, true)
                .await
                .map_err(transmission)?;
            self.places = Some(places);
        }
        Ok(self.places.as_deref().unwrap_or_default())
    }

    fn changed(&mut self) {
        self.places = None;
    }
}

/// Whether a torrent other than `hash` lists a file named `name` in `folder`:
/// that name is the torrent's, whether its file is there or not (still to be
/// written, or deleted), and Transmission would write it there.
fn claimed(places: &[TorrentPlace], hash: Option<&str>, folder: &Path, name: &str) -> bool {
    places.iter().any(|place| {
        Some(place.hash.as_str()) != hash
            && place.files.iter().any(|f| f.name == name)
            && same_folder(Path::new(&place.download_dir), folder)
    })
}

/// What a torrent that has the new name already is, by the files at its old
/// and new names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewName {
    /// The planned file is at the new name and nothing at the old: an earlier
    /// start renamed it and stopped before recording it.
    Done,
    /// The planned file is still at the old name: Transmission named the
    /// torrent without moving the file (the new name was taken). The torrent
    /// gets its old name back before anything else.
    Back,
    /// Neither: the file is not the one planned.
    Changed,
}

fn new_name(
    source: Option<FileIdentity>,
    target: Option<FileIdentity>,
    planned: &FileIdentity,
) -> NewName {
    match (source, target) {
        (None, Some(target)) if target.id().same_file(planned.id()) => NewName::Done,
        (Some(source), _) if source.id().same_file(planned.id()) => NewName::Back,
        _ => NewName::Changed,
    }
}

/// Whether an earlier start renamed the planned file and stopped before
/// recording it: nothing is at the old name and the planned file is at the
/// new one, and a torrent that still holds it has the new name.
async fn renamed_before(
    ctx: &UndoContext,
    file: &NewUndoFile,
    listing: &mut Listing,
) -> Result<bool, Retry> {
    let Some(planned) = file.identity.as_deref().and_then(FileIdentity::parse) else {
        return Ok(false);
    };
    let folder = Path::new(&file.folder);
    if let Some(hash) = file.torrent_hash.as_deref() {
        let places = listing.get(ctx).await?;
        if let Some(place) = places.iter().find(|p| p.hash.eq_ignore_ascii_case(hash)) {
            if place.name != file.to_name || !same_folder(Path::new(&place.download_dir), folder) {
                return Ok(false);
            }
        }
    }
    Ok(matches!(
        (
            identity_at(&folder.join(&file.from_name)),
            identity_at(&folder.join(&file.to_name)),
        ),
        (Ok(None), Ok(Some(there))) if there.id().same_file(planned.id())
    ))
}

/// The file at `path`, `None` when nothing is there.
fn identity_at(path: &Path) -> io::Result<Option<FileIdentity>> {
    match FileIdentity::at(path) {
        Ok(identity) => Ok(Some(identity)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Renames one planned video. `None` when it has the new name, else why it
/// keeps its old one (or waits, for a reason that [`passes`]).
async fn rename(
    ctx: &UndoContext,
    command_id: &str,
    file: &NewUndoFile,
    listing: &mut Listing,
) -> Result<Option<String>, Retry> {
    let folder = Path::new(&file.folder);
    let source = folder.join(&file.from_name);
    let target = folder.join(&file.to_name);
    let looked = |err: io::Error| Some(format!("파일을 살펴보지 못했어요({err})."));
    let mut planned = file.identity.as_deref().and_then(FileIdentity::parse);
    let hash = file.torrent_hash.as_deref();

    let place = match hash {
        Some(hash) => listing
            .get(ctx)
            .await?
            .iter()
            .find(|p| p.hash.eq_ignore_ascii_case(hash))
            .cloned(),
        None => None,
    };
    if let (Some(place), Some(hash)) = (place, hash) {
        if place.unfinished {
            return Ok(Some(UNFINISHED.to_owned()));
        }
        if !matches!(&place.files[..], [only] if only.name == place.name) {
            return Ok(Some(NAME_CHANGED.to_owned()));
        }
        if !same_folder(Path::new(&place.download_dir), folder) {
            return Ok(Some(MOVED.to_owned()));
        }
        // Planned while it was still downloading, before its file had its
        // name: the file the complete torrent has at its own name is it.
        let planned = match planned {
            Some(planned) => planned,
            None if place.name == file.from_name => match identity_at(&source) {
                Ok(Some(now)) => {
                    ctx.channels
                        .note_undo_file_identity(command_id, file.item_id, now.to_text())
                        .await
                        .map_err(store)?;
                    now
                }
                Ok(None) => return Ok(Some(MISSING.to_owned())),
                Err(err) => return Ok(looked(err)),
            },
            None => return Ok(Some(NAME_CHANGED.to_owned())),
        };
        let mut client = ctx.transmission.client();
        if place.name == file.to_name {
            let (from, to) = match (identity_at(&source), identity_at(&target)) {
                (Ok(from), Ok(to)) => (from, to),
                (Err(err), _) | (_, Err(err)) => return Ok(looked(err)),
            };
            match new_name(from, to, &planned) {
                NewName::Done => return Ok(None),
                NewName::Changed => return Ok(Some(FILE_CHANGED.to_owned())),
                NewName::Back => {
                    let answer = client
                        .torrent_rename_path(
                            vec![Id::Hash(hash.to_owned())],
                            file.to_name.clone(),
                            file.from_name.clone(),
                        )
                        .await
                        .map_err(transmission)?;
                    listing.changed();
                    if !answer.is_ok() {
                        return Ok(Some(format!(
                            "Transmission이 이름 바꾸기를 거절했어요({}).",
                            answer.result
                        )));
                    }
                }
            }
        } else if place.name != file.from_name {
            return Ok(Some(NAME_CHANGED.to_owned()));
        }
        // Transmission renames by name: the file there must be the torrent's
        // own, as planned, or another file would be moved under its name.
        match identity_at(&source) {
            Ok(Some(now)) if now.id().same_file(planned.id()) => {}
            Ok(Some(_)) => return Ok(Some(FILE_CHANGED.to_owned())),
            Ok(None) => return Ok(Some(MISSING.to_owned())),
            Err(err) => return Ok(looked(err)),
        }
        match occupied(&target) {
            Ok(false) => {}
            Ok(true) => return Ok(Some(TAKEN.to_owned())),
            Err(err) => return Ok(looked(err)),
        }
        let places = listing.get(ctx).await?;
        if claimed(places, Some(hash), folder, &file.from_name) {
            return Ok(Some(SHARED.to_owned()));
        }
        if claimed(places, Some(hash), folder, &file.to_name) {
            return Ok(Some(CLAIMED.to_owned()));
        }
        let answer = client
            .torrent_rename_path(
                vec![Id::Hash(hash.to_owned())],
                file.from_name.clone(),
                file.to_name.clone(),
            )
            .await
            .map_err(transmission)?;
        listing.changed();
        if !answer.is_ok() {
            return Ok(Some(format!(
                "Transmission이 이름 바꾸기를 거절했어요({}).",
                answer.result
            )));
        }
        // Transmission answers success without moving the file when the
        // target appeared meanwhile: both files are there, and the torrent's
        // name goes back to its own file.
        if matches!(occupied(&source), Ok(true)) && matches!(occupied(&target), Ok(true)) {
            let back = client
                .torrent_rename_path(
                    vec![Id::Hash(hash.to_owned())],
                    file.to_name.clone(),
                    file.from_name.clone(),
                )
                .await;
            if let Err(err) = back {
                eprintln!(
                    "Episode undo: cannot give torrent {hash} its name {} back: {err}",
                    file.from_name
                );
            }
            return Ok(Some(TAKEN.to_owned()));
        }
        return Ok(None);
    }

    // No torrent holds it (any more): renamed on disk, if it is still the
    // file planned.
    let Some(planned) = planned.take() else {
        // Nothing was at the name when the undo was planned.
        return Ok(Some(MISSING.to_owned()));
    };
    match identity_at(&source) {
        Ok(Some(now)) if now.unchanged(&planned) => {}
        Ok(Some(_)) => return Ok(Some(FILE_CHANGED.to_owned())),
        Ok(None) => {
            // An earlier start renamed it and stopped before recording it.
            return Ok(match identity_at(&target) {
                Ok(Some(there)) if there.id().same_file(planned.id()) => None,
                _ => Some(MISSING.to_owned()),
            });
        }
        Err(err) => return Ok(looked(err)),
    }
    if claimed(listing.get(ctx).await?, None, folder, &file.to_name) {
        return Ok(Some(CLAIMED.to_owned()));
    }
    Ok(match rename_noreplace(&source, &target) {
        Ok(()) => None,
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Some(TAKEN.to_owned()),
        Err(err) => Some(format!("이름을 바꾸지 못했어요({err}).")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_is_the_rule_and_the_value_seen() {
        let payload = EpisodeUndo {
            rule_id: "r1".into(),
            episode: -48,
        };
        assert_eq!(payload.canonical(), r#"{"rule_id":"r1","episode":-48}"#);
        assert_eq!(payload.subject(), "r1");
        assert!(serde_json::from_str::<EpisodeUndo>(
            r#"{"rule_id":"r1","episode":-48,"files":[]}"#
        )
        .is_err());
    }

    fn planned(from: &str, to: &str) -> NewUndoFile {
        NewUndoFile {
            item_id: 1,
            folder: "/shows/Show/Season 03".into(),
            from_name: from.into(),
            to_name: to.into(),
            torrent_hash: None,
            identity: None,
            kept: None,
        }
    }

    #[test]
    fn a_name_the_undo_frees_is_freed_before_it_is_taken() {
        let (a, b, c) = (
            planned("S03E01.mkv", "S03E13.mkv"),
            planned("S03E13.mkv", "S03E25.mkv"),
            planned("S03E25.mkv", "S03E37.mkv"),
        );
        // Names shift up: the last in the chain goes first.
        assert_eq!(next(&[&a, &b, &c]), 2);
        assert_eq!(next(&[&a, &b]), 1);
        assert_eq!(next(&[&a]), 0);
        // Names shift down: in the order planned.
        let (d, e) = (
            planned("S03E25.mkv", "S03E13.mkv"),
            planned("S03E13.mkv", "S03E01.mkv"),
        );
        assert_eq!(next(&[&d, &e]), 1);
        // A swap waits on itself: the first goes.
        let (f, g) = (
            planned("S03E01.mkv", "S03E02.mkv"),
            planned("S03E02.mkv", "S03E01.mkv"),
        );
        assert_eq!(next(&[&f, &g]), 0);
    }

    #[test]
    fn a_torrent_named_anew_already_is_done_only_when_its_file_moved_with_it() {
        let planned = FileIdentity::parse("1:10:5:1.0:1.0").unwrap();
        let moved = FileIdentity::parse("1:10:5:1.0:2.0").unwrap();
        let other = FileIdentity::parse("1:11:5:1.0:1.0").unwrap();
        // The file moved (its status-change time with it): done.
        assert_eq!(new_name(None, Some(moved), &planned), NewName::Done);
        // Still at the old name, whatever is at the new one: the torrent's
        // name goes back first.
        assert_eq!(
            new_name(Some(planned), Some(other), &planned),
            NewName::Back
        );
        assert_eq!(new_name(Some(planned), None, &planned), NewName::Back);
        // Another file at the new name, nothing at the old: not done.
        assert_eq!(new_name(None, Some(other), &planned), NewName::Changed);
        assert_eq!(new_name(None, None, &planned), NewName::Changed);
        assert_eq!(new_name(Some(other), None, &planned), NewName::Changed);
        // The same files after the folder was mounted again (a restart).
        let remounted = FileIdentity::parse("2:10:5:1.0:2.0").unwrap();
        assert_eq!(new_name(None, Some(remounted), &planned), NewName::Done);
        assert_eq!(new_name(Some(remounted), None, &planned), NewName::Back);
    }

    #[test]
    fn an_rss_title_without_an_extension_takes_its_torrents() {
        let folder = Path::new("/shows/Show/Season 03");
        let title = "[SubsPlease] Show - 49 (1080p) [ABCD0049]";
        assert_eq!(names(folder, title, None, -48, -24), None);
        let place = TorrentPlace {
            name: "Show S03E01.mkv".into(),
            ..TorrentPlace::default()
        };
        assert_eq!(
            names(folder, title, Some(&place), -48, -24),
            Some(("Show S03E01.mkv".into(), "Show S03E25.mkv".into()))
        );
    }
}

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod renaming_tests;
#[cfg(test)]
mod rows_tests;
#[cfg(test)]
mod waiting_tests;
