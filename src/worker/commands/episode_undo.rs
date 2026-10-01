//! `episode_undo`, shown on the screen as `되돌리기` beside an episode offset
//! the app set (`docs/specs/collection.md`, 영상 회차 변환): puts back the
//! value the rule had before and renames the videos the rule received under
//! the app's value to the names they get with it (`S03E01` → `S03E25`).
//!
//! The web accepts the command with an [`EpisodeUndo`] payload (the rule and
//! the automatic value the user saw; one rule has at most one open undo) and
//! the worker runs it with [`run`], under the lock that also guards the
//! collection cycles: Transmission is reached from the worker only, and no
//! cycle receives an item or renames a file while the undo runs.
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
//!   Transmission's rename, so it keeps seeding under the new name.
//! - **Its torrent is gone** and a file of that name is in the folder that no
//!   torrent holds: renamed on disk with `RENAME_NOREPLACE`, and only if it is
//!   still the file planned ([`FileIdentity`]: device, inode, size and times).
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
//! is there (see [`crate::transmission::rename_torrent`]), so a target that
//! appeared in between leaves the source in place, and the torrent's name is
//! then put back. A rename on disk refuses an existing target by itself.
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
//! the files still `pending`, taking a file that has the new name already as
//! renamed.
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

use crate::{
    episode_offset::signed,
    revision::FileIdentity,
    rss::save_path,
    store::{
        channels::{NewUndoFile, Rule, UndoBegun, UndoFileState},
        commands::{Command, CommandState, Outcome},
    },
    transmission::{get_torrent, torrent_places, TorrentPlace},
    worker::{
        commands::rule_archive::work_folder::rename_noreplace,
        revisions::{episode_name, owner_of, same_folder, Owner},
        Clock, CycleContext,
    },
};

/// The `kind` of the command.
pub const KIND: &str = "episode_undo";

/// The outcome's `result` when the previous value is back.
pub const UNDONE: &str = "undone";
/// The outcome's `result` of a command that ended `failed`.
pub const FAILED: &str = "failed";

/// Why a file keeps its name: the name it would take is taken.
pub const TAKEN: &str = "같은 이름의 파일이 이미 있어요.";
/// Why a file keeps its name: its torrent's file has another name now.
pub const NAME_CHANGED: &str = "토렌트 파일의 이름이 그사이 바뀌었어요.";
/// Why a file keeps its name: it is not the file planned any more.
pub const FILE_CHANGED: &str = "되돌리기를 시작한 뒤 파일이 바뀌었어요.";
/// Why a file keeps its name: it is not there.
pub const MISSING: &str = "파일을 찾지 못했어요.";

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

/// A command that could not be carried through now and is left `running` for
/// the next look, which carries on with the files still to rename.
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

fn failed(reason: impl Into<String>) -> Finished {
    Finished {
        state: CommandState::Failed,
        outcome: Outcome {
            result: FAILED.to_owned(),
            reason: Some(reason.into()),
        },
    }
}

/// Runs an `episode_undo` command to its end.
pub async fn run(ctx: &CycleContext, command: &Command, clock: &Clock) -> Result<Finished, Retry> {
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
            if !rule.episode_auto || rule.episode != payload.episode {
                return Ok(failed(RULE_CHANGED));
            }
            let marks = ctx
                .channels
                .episode_marks(vec![rule.id.clone()])
                .await
                .map_err(store)?;
            let Some(to) = marks.get(&rule.id).and_then(|m| m.previous) else {
                return Ok(failed(
                    "앱이 정하기 전 값을 몰라서 되돌릴 수 없어요. 회차 변환을 직접 적어 주세요.",
                ));
            };
            let Some(collect) = ctx.settings.collection().await.map_err(store)? else {
                return Ok(failed(NO_COLLECT_FOLDER));
            };
            let files = plan(ctx, &rule, &collect.folder, payload.episode, to).await?;
            match ctx
                .channels
                .begin_episode_undo(&command.id, &rule.id, payload.episode, files, clock())
                .await
                .map_err(store)?
            {
                UndoBegun::Begun(undo) => undo,
                UndoBegun::Changed => return Ok(failed(RULE_CHANGED)),
                UndoBegun::Busy(names) => {
                    return Ok(failed(format!(
                        "수정본으로 대체하는 중인 영상이 있어서 되돌리지 않았어요: {}. \
                         대체가 끝난 뒤 다시 되돌려 주세요.",
                        names.join(", ")
                    )))
                }
            }
        }
    };

    for file in undo
        .files
        .iter()
        .filter(|f| f.state == UndoFileState::Pending)
    {
        let kept = rename(ctx, &file.file).await?;
        match &kept {
            None => println!(
                "Episode undo {}: {} is now {}",
                command.id, file.file.from_name, file.file.to_name
            ),
            Some(why) => println!(
                "Episode undo {}: {} keeps its name: {why}",
                command.id, file.file.from_name
            ),
        }
        ctx.channels
            .finish_undo_file(&command.id, file.file.item_id, kept, clock())
            .await
            .map_err(store)?;
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
    ctx: &CycleContext,
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
    let mut client = ctx.transmission();
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
        let Some((from_name, to_name)) = names(&folder, &item.title, place, from, to) else {
            continue;
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
    let both = |title: &str| {
        Some((
            episode_name(folder, title, from as isize)?,
            episode_name(folder, title, to as isize)?,
        ))
    };
    both(title).or_else(|| {
        let ext = Path::new(&place?.name).extension()?.to_str()?;
        both(&format!("{title}.{ext}"))
    })
}

/// Whether something is at `path` (a link counts as itself).
fn exists(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

/// Renames one planned video. `None` when it has the new name, else why it
/// keeps its old one.
async fn rename(ctx: &CycleContext, file: &NewUndoFile) -> Result<Option<String>, Retry> {
    let folder = Path::new(&file.folder);
    let source = folder.join(&file.from_name);
    let target = folder.join(&file.to_name);
    let looked = |err: io::Error| Some(format!("파일을 살펴보지 못했어요({err})."));

    if let Some(hash) = &file.torrent_hash {
        let mut client = ctx.transmission();
        if let Some(torrent) = get_torrent(&mut client, hash).await.map_err(transmission)? {
            let name = torrent.name.unwrap_or_default();
            if name == file.to_name {
                // An earlier start renamed it and stopped before recording it.
                return Ok(None);
            }
            if name != file.from_name {
                return Ok(Some(NAME_CHANGED.to_owned()));
            }
            match exists(&target) {
                Ok(false) => {}
                Ok(true) => return Ok(Some(TAKEN.to_owned())),
                Err(err) => return Ok(looked(err)),
            }
            let answer = client
                .torrent_rename_path(
                    vec![Id::Hash(hash.clone())],
                    file.from_name.clone(),
                    file.to_name.clone(),
                )
                .await
                .map_err(transmission)?;
            if !answer.is_ok() {
                return Ok(Some(format!(
                    "Transmission이 이름 바꾸기를 거절했어요({}).",
                    answer.result
                )));
            }
            // Transmission answers success without moving the file when the
            // target appeared meanwhile: both files are there, and the
            // torrent's name goes back to its own file.
            if matches!(exists(&source), Ok(true)) && matches!(exists(&target), Ok(true)) {
                let back = client
                    .torrent_rename_path(
                        vec![Id::Hash(hash.clone())],
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
        // The torrent is gone: its file is renamed on disk below.
    }

    let Some(planned) = file.identity.as_deref().and_then(FileIdentity::parse) else {
        return Ok(Some(MISSING.to_owned()));
    };
    match FileIdentity::at(&source) {
        Ok(now) if now == planned => {}
        Ok(_) => return Ok(Some(FILE_CHANGED.to_owned())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            // An earlier start renamed it and stopped before recording it.
            return Ok(match FileIdentity::at(&target) {
                Ok(there) if there.same_file(&planned) => None,
                _ => Some(MISSING.to_owned()),
            });
        }
        Err(err) => return Ok(looked(err)),
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
