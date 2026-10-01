//! Replacing a video with a higher revision of the same release
//! (`docs/specs/collection.md`, 영상 수정본의 대체; ADR 0010). Only the worker
//! does this, inside its collection cycle, so one process writes the media
//! folders.
//!
//! # Deciding, when a rule selects a revision ([`plan`])
//!
//! A selected item is looked at only when its name says it is a revision
//! (`14v2`, see [`Release`]) and the episode's name in the rule's folder (the
//! one `trname` gives it) is taken by a file. Then the file's revision is
//! found:
//!
//! - **Known**: the file belongs to a torrent (Transmission's file lists,
//!   matched by path) that history records, under the same release
//!   ([`Release::stem`]). A lower revision is replaced; the same or a higher
//!   one is skipped. A torrent of another release makes the item a duplicate,
//!   not a revision: it is received as before and keeps its own name, since
//!   no rename ever takes a name that is taken.
//! - **Unknown** (no torrent, or one history does not know): the file's CRC32
//!   is read and compared with the names of the new release and of the lower
//!   revisions of it in the channel's history. Equal to the new one → skipped;
//!   to a lower one → replaced; to none → `버전 미상`, not received.
//!
//! A revision whose name carries no CRC32 is not received either: its file
//! could not be checked. It is `버전 미상` in history, and `다시 받기` receives
//! it with the request as the confirmation ([`confirm`]).
//!
//! Whatever is decided is written as a row of
//! [`RevisionStore`](crate::store::revisions::RevisionStore), so an item is
//! decided once (a CRC32 is not read again every cycle), and a cycle leaves the
//! row's item to the steps below instead of adding it again. A cycle also no
//! longer receives the old video's item once its torrent is being removed.
//!
//! # Replacing ([`advance`], every cycle)
//!
//! The new torrent keeps the name it was received under. Then, each step
//! written before the next one acts, so a restart carries on from what is on
//! disk:
//!
//! 1. **Received and checked**: Transmission has all of the torrent's single
//!    file, and its CRC32 (read as a stream) is the one its name carries (or
//!    the person confirmed). A torrent that is gone or reports a local error
//!    fails the replacement; one still downloading waits.
//! 2. **The old video is removed**: if it is the single file of a torrent trss
//!    added (a `received` record in history), that torrent is removed with its
//!    data (removing only the file would make Transmission fetch it again); if
//!    no torrent holds it, the file is deleted. A file in a torrent of several
//!    files (a batch) or in a torrent trss did not add is left alone and the
//!    replacement fails. A failure here leaves the old video in place.
//! 3. **The new video takes the episode name**, by Transmission's rename (or,
//!    for a torrent that is gone, a rename on disk that never replaces), and
//!    only while that name is free. Until it goes through, the row says why
//!    and the next cycle tries again.
//!
//! A failure before or at step 2 is final: both files stay, and the row shows
//! as `받기 실패` on the work's episode row and in the to-do source until one of
//! the two files is gone. Nothing ever overwrites a file.

use std::{
    io,
    path::{Path, PathBuf},
};

use tokio_util::sync::CancellationToken;
use transmission_rpc::types::Id;

use super::{commands::receive_once::derived_name, CycleContext};
use crate::{
    revision::{crc_text, file_crc32, Release},
    store::{
        history::{HistoryResult, Millis},
        revisions::{Revision, RevisionState, Step},
    },
    transmission::{get_torrent, torrent_places, Redactor, TorrentPlace},
};

/// Why a revision without a CRC32 in its name is not received.
pub const NO_CRC: &str = "이름에 CRC32 값이 없어서 받은 영상을 확인할 수 없어 자동으로 받지 않았어요. 다시 받기로 받으면 확인 없이 이전 영상을 대체해요.";
/// Why a revision is not received when the folder's video matches no release.
pub const UNKNOWN_FILE: &str = "폴더에 있는 영상의 CRC32가 이 릴리스의 어느 수정본과도 달라서 버전을 알 수 없어요. 다시 받기로 받으면 이전 영상을 대체해요.";
/// Why a revision the folder already holds is not received.
pub const ALREADY_THERE: &str = "폴더의 영상이 이미 이 수정본이에요.";
/// Why a revision lower than the folder's video is not received.
pub const NOT_HIGHER: &str = "폴더에 같거나 더 높은 수정본이 있어요.";

/// What a cycle does with a selected item (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Not a revision of a video in the folder: added and renamed as always.
    Normal,
    /// A revision that replaces the folder's video: added, not renamed.
    Replace(Decided),
    /// `버전 미상`: not added; `reason` says why.
    Unknown(Decided, &'static str),
    /// The folder holds it already: not added.
    Skip(Decided, &'static str),
    /// Transmission or the disk could not be read now: nothing is done or
    /// recorded, and the next cycle decides.
    Later(String),
}

/// What [`plan`] learned about a revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decided {
    pub episode_name: String,
    pub version: u32,
    /// The name's CRC32, as text.
    pub crc: Option<String>,
    pub old_item_id: Option<i64>,
    pub old_version: Option<u32>,
}

/// A selected item as [`plan`] sees it.
pub struct Selected<'a> {
    pub channel_id: &'a str,
    pub identity_key: &'a str,
    pub title: &'a str,
    pub save_path: &'a Path,
    pub episode: isize,
}

/// Whether the release name `title` is a revision at all: the cheap test that
/// leaves every other item to the cycle as before.
pub fn is_revision(title: &str) -> bool {
    Release::parse(title).version > 1
}

/// The torrent (of `places`) one of whose files is `path`.
fn owner_of<'a>(places: &'a [TorrentPlace], path: &Path) -> Option<&'a TorrentPlace> {
    places.iter().find(|place| {
        place
            .files
            .iter()
            .any(|file| Path::new(&place.download_dir).join(&file.name) == path)
    })
}

/// Reads the CRC32 of `path` off the async threads.
async fn crc_of(path: PathBuf) -> io::Result<u32> {
    tokio::task::spawn_blocking(move || file_crc32(&path))
        .await
        .map_err(io::Error::other)?
}

/// Decides what to do with a selected item (see the module docs).
pub async fn plan(ctx: &CycleContext, item: &Selected<'_>) -> Plan {
    let release = Release::parse(item.title);
    if release.version < 2 {
        return Plan::Normal;
    }
    let Some(episode_name) = derived_name(item.save_path, item.title, item.episode) else {
        return Plan::Normal;
    };
    let target = item.save_path.join(&episode_name);
    match std::fs::symlink_metadata(&target) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Plan::Normal,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Plan::Normal,
        Err(err) => return Plan::Later(format!("cannot look at {}: {err}", target.display())),
    }
    let decided = |old: Option<(i64, u32)>| Decided {
        episode_name: episode_name.clone(),
        version: release.version,
        crc: release.crc.map(crc_text),
        old_item_id: old.map(|(id, _)| id),
        old_version: old.map(|(_, v)| v),
    };

    let own = match ctx
        .history
        .item_by_key(item.channel_id.to_owned(), item.identity_key.to_owned())
        .await
    {
        Ok(own) => own.and_then(|own| own.torrent_hash),
        Err(err) => return Plan::Later(err.to_string()),
    };
    let mut transmission = ctx.transmission();
    let places = match torrent_places(&mut transmission, None, true).await {
        Ok(places) => places,
        Err(err) => return Plan::Later(err.to_string()),
    };
    if let Some(owner) = owner_of(&places, &target) {
        // The item's own torrent holds the name: it was received before the
        // folder had the episode, and renamed.
        if own.as_deref() == Some(owner.hash.as_str()) {
            return Plan::Normal;
        }
        let records = match ctx.history.items_of_hash(&owner.hash).await {
            Ok(records) => records,
            Err(err) => return Plan::Later(err.to_string()),
        };
        if let Some(old) = records
            .iter()
            .map(|record| (record.id, Release::parse(&record.title)))
            .find(|(_, old)| old.stem == release.stem)
        {
            let (id, old) = old;
            if old.version >= release.version {
                return Plan::Skip(decided(Some((id, old.version))), NOT_HIGHER);
            }
            if release.crc.is_none() {
                return Plan::Unknown(decided(Some((id, old.version))), NO_CRC);
            }
            return Plan::Replace(decided(Some((id, old.version))));
        }
        if !records.is_empty() {
            // Another release's video: a duplicate, which keeps both files.
            return Plan::Normal;
        }
    }

    // A video of unknown revision.
    let Some(crc) = release.crc else {
        return Plan::Unknown(decided(None), NO_CRC);
    };
    let file_crc = match crc_of(target.clone()).await {
        Ok(file_crc) => file_crc,
        Err(err) => return Plan::Later(format!("cannot read {}: {err}", target.display())),
    };
    if file_crc == crc {
        return Plan::Skip(decided(None), ALREADY_THERE);
    }
    let titles = match ctx
        .history
        .titles_of_channel(item.channel_id.to_owned())
        .await
    {
        Ok(titles) => titles,
        Err(err) => return Plan::Later(err.to_string()),
    };
    let old = titles.into_iter().find_map(|(id, title)| {
        let old = Release::parse(&title);
        (old.stem == release.stem && old.version < release.version && old.crc == Some(file_crc))
            .then_some((id, old.version))
    });
    match old {
        Some(old) => Plan::Replace(decided(Some(old))),
        None => Plan::Unknown(decided(None), UNKNOWN_FILE),
    }
}

/// `다시 받기` received the `버전 미상` item `item_id` as the torrent `hash`: its
/// replacement goes ahead, checking the CRC32 only when the name carries one.
/// Returns whether the item has a replacement under way, whose torrent the
/// caller must not rename.
pub async fn confirm(
    ctx: &CycleContext,
    item_id: i64,
    title: &str,
    at: Millis,
    hash: &str,
) -> bool {
    let expected = Release::parse(title).crc.map(crc_text);
    match ctx
        .revisions
        .confirm(item_id, at, hash.to_owned(), expected)
        .await
    {
        Ok(Some(row)) => row.state != RevisionState::Unknown,
        Ok(None) => false,
        Err(err) => {
            eprintln!("Cannot confirm the revision of item {item_id}: {err}");
            // Not renaming is the safe side: a rename could take the old
            // video's name.
            true
        }
    }
}

// --- the steps ----------------------------------------------------------------------

/// What one look at a step came to.
enum Next {
    /// Write this step and go on.
    Step(Step),
    /// Nothing to do until a later cycle.
    Wait,
    /// Transmission or the disk could not be read now; logged, and tried again
    /// by the next cycle.
    Later(String),
}

const RECEIVE_STOPPED: &str =
    "새 영상의 토렌트가 Transmission에서 사라져 받기가 끝나지 않았어요. 이전 영상은 그대로 있어요.";
const SEVERAL_FILES: &str =
    "새 영상의 토렌트에 파일이 여러 개라 대체하지 않았어요. 이전 영상은 그대로 있어요.";
const ELSEWHERE: &str =
    "새 영상이 규칙의 저장 폴더가 아닌 곳에 있어서 대체하지 않았어요. 이전 영상은 그대로 있어요.";
const NEW_FILE_GONE: &str = "받은 새 영상 파일이 없어요. 이전 영상은 그대로 있어요.";
const UNKNOWN_TORRENT: &str = "새 영상의 토렌트를 알 수 없어요. 이전 영상은 그대로 있어요.";
const NAME_TAKEN_BY_NEW: &str =
    "새 영상의 토렌트가 이미 회차 이름의 파일을 가리키고 있어서 대체하지 않았어요. 두 파일을 그대로 뒀어요.";
const DESTINATION_TAKEN: &str =
    "회차 이름에 다른 파일이 있어서 새 영상의 이름을 바꾸지 않았어요. 그 이름이 비면 다시 바꿔요.";
const NEW_FILE_MISSING: &str = "받은 새 영상 파일을 찾지 못해 회차 이름을 붙이지 못했어요.";

/// Carries every replacement under way as far as it goes now. `at` stamps
/// what is written.
pub async fn advance(
    ctx: &CycleContext,
    at: Millis,
    redactor: &Redactor,
    cancel: &CancellationToken,
) {
    let rows = match ctx.revisions.open().await {
        Ok(rows) => rows,
        Err(err) => return eprintln!("Cannot read the video revisions: {err}"),
    };
    for row in rows {
        if cancel.is_cancelled() {
            return;
        }
        drive(ctx, row, at, redactor).await;
    }
}

async fn drive(ctx: &CycleContext, mut row: Revision, at: Millis, redactor: &Redactor) {
    loop {
        let next = match row.state {
            RevisionState::Receiving => received(ctx, &row).await,
            RevisionState::Verified | RevisionState::Removing => {
                remove_old(ctx, &mut row, at).await
            }
            RevisionState::Removed => rename(ctx, &row).await,
            RevisionState::Failed => cleared(&row),
            _ => return,
        };
        let step = match next {
            Next::Step(step) => step,
            Next::Wait => return,
            Next::Later(why) => {
                return println!(
                    "Revision of {} waits: {}",
                    row.episode_name,
                    redactor.apply(&why)
                )
            }
        };
        // A rename that keeps failing for the same reason is written once.
        if let Step::Removed { reason } = &step {
            if row.state == RevisionState::Removed && *reason == row.reason {
                return;
            }
        }
        if let Err(err) = ctx.revisions.advance(row.id, at, step.clone()).await {
            return eprintln!("Cannot record the revision of {}: {err}", row.episode_name);
        }
        println!("Revision of {}: {step:?}", row.episode_name);
        match step {
            Step::Verified {
                received_name,
                file_crc,
            } => {
                row.state = RevisionState::Verified;
                row.received_name = Some(received_name);
                row.file_crc = Some(file_crc);
            }
            Step::Removing => row.state = RevisionState::Removing,
            Step::Removed { reason } => {
                let tried = reason.is_some();
                row.state = RevisionState::Removed;
                row.reason = reason;
                if tried {
                    return;
                }
            }
            Step::Done | Step::Failed { .. } | Step::Cleared => return,
        }
    }
}

fn failed(reason: impl Into<String>, received_name: Option<String>) -> Next {
    Next::Step(Step::Failed {
        reason: reason.into(),
        received_name,
    })
}

/// Step 1: the new video is all there and its CRC32 is the name's.
async fn received(ctx: &CycleContext, row: &Revision) -> Next {
    let Some(hash) = row.torrent_hash.clone() else {
        return failed(UNKNOWN_TORRENT, None);
    };
    let mut transmission = ctx.transmission();
    let place = match torrent_places(&mut transmission, Some(&[hash]), true).await {
        Ok(places) => places.into_iter().next(),
        Err(err) => return Next::Later(err.to_string()),
    };
    let Some(place) = place else {
        return failed(RECEIVE_STOPPED, None);
    };
    if let Some(error) = &place.local_error {
        return failed(
            format!("새 영상을 받다 Transmission이 오류를 알렸어요: {error}. 이전 영상은 그대로 있어요."),
            None,
        );
    }
    if place.unfinished {
        return Next::Wait;
    }
    let [file] = place.files.as_slice() else {
        return failed(SEVERAL_FILES, None);
    };
    if file.name.contains('/') {
        return failed(SEVERAL_FILES, None);
    }
    if Path::new(&place.download_dir) != Path::new(&row.folder) {
        return failed(ELSEWHERE, Some(file.name.clone()));
    }
    let path = Path::new(&row.folder).join(&file.name);
    let crc = match crc_of(path.clone()).await {
        Ok(crc) => crc_text(crc),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return failed(NEW_FILE_GONE, Some(file.name.clone()))
        }
        Err(err) => return Next::Later(format!("cannot read {}: {err}", path.display())),
    };
    if let Some(expected) = &row.expected_crc {
        if *expected != crc {
            return failed(
                format!(
                    "받은 영상의 CRC32({crc})가 이름의 값({expected})과 달라서 대체하지 않았어요. 이전 영상은 그대로 있어요."
                ),
                Some(file.name.clone()),
            );
        }
    }
    Next::Step(Step::Verified {
        received_name: file.name.clone(),
        file_crc: crc,
    })
}

fn exists(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

/// Step 2: the old video is removed, or the replacement fails with it in place.
/// `removing` is written before anything is removed.
async fn remove_old(ctx: &CycleContext, row: &mut Revision, at: Millis) -> Next {
    let old = Path::new(&row.folder).join(&row.episode_name);
    match exists(&old) {
        Ok(true) => {}
        Ok(false) => return Next::Step(Step::Removed { reason: None }),
        Err(err) => return Next::Later(format!("cannot look at {}: {err}", old.display())),
    }
    let mut transmission = ctx.transmission();
    let places = match torrent_places(&mut transmission, None, true).await {
        Ok(places) => places,
        Err(err) => return Next::Later(err.to_string()),
    };
    let owner = owner_of(&places, &old).cloned();
    if let Some(owner) = &owner {
        if row.torrent_hash.as_deref() == Some(owner.hash.as_str()) {
            return failed(NAME_TAKEN_BY_NEW, None);
        }
        if owner.files.len() > 1 {
            return failed(
                format!(
                    "이전 영상이 파일 여러 개를 담은 토렌트 {}에 있어서 다른 회차까지 지울 수 있어 대체하지 않았어요. 두 파일을 그대로 뒀어요.",
                    owner.name
                ),
                None,
            );
        }
        let added_by_trss = match ctx.history.items_of_hash(&owner.hash).await {
            Ok(records) => records
                .iter()
                .any(|record| record.result == HistoryResult::Received),
            Err(err) => return Next::Later(err.to_string()),
        };
        if !added_by_trss {
            return failed(
                format!(
                    "이전 영상이 trss가 추가하지 않은 토렌트 {}에 있어서 대체하지 않았어요. 두 파일을 그대로 뒀어요.",
                    owner.name
                ),
                None,
            );
        }
    }

    if row.state != RevisionState::Removing {
        if let Err(err) = ctx.revisions.advance(row.id, at, Step::Removing).await {
            return Next::Later(err.to_string());
        }
        row.state = RevisionState::Removing;
    }

    match owner {
        Some(owner) => {
            match transmission
                .torrent_remove(vec![Id::Hash(owner.hash.clone())], true)
                .await
            {
                Ok(response) if response.is_ok() => {}
                Ok(response) => {
                    return failed(
                        format!(
                            "이전 영상의 토렌트를 지우지 못했어요({}). 두 파일을 그대로 뒀어요.",
                            response.result
                        ),
                        None,
                    )
                }
                Err(err) => {
                    return failed(
                        format!(
                            "이전 영상의 토렌트를 지우지 못했어요({err}). 두 파일을 그대로 뒀어요."
                        ),
                        None,
                    )
                }
            }
        }
        None => {
            if let Err(err) = std::fs::remove_file(&old) {
                if err.kind() != io::ErrorKind::NotFound {
                    return failed(
                        format!("이전 영상을 지우지 못했어요({err}). 두 파일을 그대로 뒀어요."),
                        None,
                    );
                }
            }
        }
    }
    match exists(&old) {
        Ok(false) => Next::Step(Step::Removed { reason: None }),
        // Transmission may delete the data after it has answered.
        Ok(true) => Next::Later(format!("{} is still there", old.display())),
        Err(err) => Next::Later(format!("cannot look at {}: {err}", old.display())),
    }
}

/// Step 3: the new video takes the episode name while the name is free.
async fn rename(ctx: &CycleContext, row: &Revision) -> Next {
    let folder = Path::new(&row.folder);
    let target = folder.join(&row.episode_name);
    match exists(&target) {
        Ok(false) => {}
        Ok(true) => {
            return Next::Step(Step::Removed {
                reason: Some(DESTINATION_TAKEN.to_owned()),
            })
        }
        Err(err) => return Next::Later(format!("cannot look at {}: {err}", target.display())),
    }
    let Some(received_name) = row.received_name.clone() else {
        return Next::Step(Step::Removed {
            reason: Some(NEW_FILE_MISSING.to_owned()),
        });
    };
    let source = folder.join(&received_name);
    match exists(&source) {
        Ok(true) => {}
        Ok(false) => {
            return Next::Step(Step::Removed {
                reason: Some(NEW_FILE_MISSING.to_owned()),
            })
        }
        Err(err) => return Next::Later(format!("cannot look at {}: {err}", source.display())),
    }

    let mut transmission = ctx.transmission();
    let torrent = match &row.torrent_hash {
        Some(hash) => match get_torrent(&mut transmission, hash).await {
            Ok(torrent) => torrent,
            Err(err) => return Next::Later(err.to_string()),
        },
        None => None,
    };
    let failure = match torrent {
        Some(torrent) => {
            let hash = torrent.hash_string.unwrap_or_default();
            match transmission
                .torrent_rename_path(
                    vec![Id::Hash(hash)],
                    received_name.clone(),
                    row.episode_name.clone(),
                )
                .await
            {
                Ok(response) if response.is_ok() => None,
                Ok(response) => Some(response.result),
                Err(err) => Some(err.to_string()),
            }
        }
        // Nothing of Transmission's to keep in step with: the file is renamed
        // on disk, never over another.
        None => {
            crate::worker::commands::rule_archive::work_folder::rename_noreplace(&source, &target)
                .err()
                .map(|err| err.to_string())
        }
    };
    if let Some(failure) = failure {
        return Next::Step(Step::Removed {
            reason: Some(format!(
                "새 영상에 회차 이름을 붙이지 못했어요({failure}). 다음 확인 때 다시 해요."
            )),
        });
    }
    match (exists(&target), exists(&source)) {
        (Ok(true), Ok(false)) => Next::Step(Step::Done),
        _ => Next::Step(Step::Removed {
            reason: Some(
                "새 영상의 이름 바꾸기가 끝나지 않았어요. 다음 확인 때 다시 해요.".to_owned(),
            ),
        }),
    }
}

/// A failure the person resolved: one of the two files is gone.
fn cleared(row: &Revision) -> Next {
    let folder = Path::new(&row.folder);
    let old_gone = matches!(exists(&folder.join(&row.episode_name)), Ok(false));
    let new_gone = row
        .received_name
        .as_ref()
        .is_some_and(|name| matches!(exists(&folder.join(name)), Ok(false)));
    if old_gone || new_gone {
        Next::Step(Step::Cleared)
    } else {
        Next::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_name_with_a_revision_is_looked_at() {
        assert!(is_revision(
            "[SubsPlease] Show - 14v2 (1080p) [1A2B3C4D].mkv"
        ));
        assert!(!is_revision(
            "[SubsPlease] Show - 14 (1080p) [1A2B3C4D].mkv"
        ));
        assert!(!is_revision("[SubsPlease] Show - 14v1 (1080p).mkv"));
    }
}
