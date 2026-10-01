//! Replacing a video with a higher revision of the same release
//! (`docs/specs/collection.md`, 영상 수정본의 대체; ADR 0010). Only the worker
//! does this, inside its collection cycle, so one process writes the media
//! folders.
//!
//! # Deciding, when a rule selects a revision ([`plan`])
//!
//! A selected item is looked at only when its name says it is a revision
//! (`14v2`, see [`Release`]) and the episode's name in the rule's folder (the
//! one `trname` gives the name without its revision, [`episode_name`]) is
//! taken by a file. Then the file's revision is found:
//!
//! - **Known**: the file belongs to a torrent that history records, under the
//!   same release ([`Release::stem`]). A lower revision is replaced; the same
//!   or a higher one is skipped. A torrent of another release makes the item a
//!   duplicate, not a revision: it is received as before and keeps its own
//!   name, since no rename ever takes a name that is taken. The item's own
//!   torrent is asked about first, alone; Transmission's whole file list
//!   ([`Listing`]) is read at most once for all the items of a cycle.
//! - **Unknown** (no torrent, or one history does not know): the file's CRC32
//!   is read (one file at a time) and compared with the names of the new
//!   release and of the other revisions of it in the channel's history, and
//!   with the replacements done for the episode. Equal to the new one or a
//!   higher one → skipped; to a lower one → replaced; to none → `버전 미상`,
//!   not received.
//!
//! Which torrent holds a file is told by the file itself: a torrent's file of
//! the same name whose device and inode are the file's, whatever way
//! Transmission spells its folder ([`owner_of`]). A file two torrents hold is
//! not replaced.
//!
//! A revision whose name carries no CRC32 is not received either: its file
//! could not be checked. It is `버전 미상` in history, and `다시 받기` receives
//! it with the request as the confirmation ([`confirm`]).
//!
//! Whatever is decided is written as a row of
//! [`RevisionStore`](crate::store::revisions::RevisionStore), so an item is
//! decided once (a CRC32 is not read again every cycle), and a cycle leaves the
//! row's item to the steps below instead of adding it again. A cycle also no
//! longer receives the old video's release (through any channel) once its
//! torrent is being removed, nor a lower revision of a release that replaced
//! a video in the folder.
//!
//! # Replacing ([`advance`], every cycle)
//!
//! The new torrent keeps the name it was received under. Then, each step
//! written before the next one acts, so a restart carries on from what is on
//! disk:
//!
//! 1. **Received and checked**: Transmission has all of the torrent's single
//!    file, not empty, in the rule's folder and under a name of its own, and
//!    its CRC32 (read as a stream) is the one its name carries (or the person
//!    confirmed). One still downloading waits. A torrent that is gone, reports
//!    a local error, is elsewhere or is not one file fails the replacement for
//!    now: the worker looks at the torrent again every cycle and goes on once
//!    it is right, and a cycle that meets the item in its feed receives it
//!    again when the torrent is gone.
//! 2. **The old video is removed**, by one replacement of the episode at a
//!    time ([`RevisionStore::claim`](crate::store::revisions::RevisionStore::claim));
//!    a replacement whose revision is lower than one in place, or than one
//!    on its way, is skipped instead and its video keeps its received name.
//!    What is at the episode name is looked at again just before: if it is the
//!    single file of a torrent trss added (a `received` record in history) of
//!    a lower revision of the release, that torrent is removed with its data
//!    (removing only the file would make Transmission fetch it again); if no
//!    torrent holds it, the file is deleted, but only when its CRC32 is the
//!    one of the video decided about. Anything else (a batch, a torrent trss
//!    did not add or of another release, a file two torrents hold, a file that
//!    changed) is left alone and the replacement fails. A failure here leaves
//!    the old video in place.
//! 3. **The new video takes the episode name**, by Transmission's rename (or,
//!    for a torrent that is gone, a rename on disk that never replaces), and
//!    only while that name is free. Until it goes through, the row says why
//!    and the next cycle tries again.
//!
//! A failure at step 2, or at step 1 with the new video received, is final:
//! both files stay, and the row shows as `받기 실패` on the work's episode row
//! and in the to-do source until one of the two files is gone. Nothing ever
//! overwrites a file. Reasons pass through the redactor and are capped, as
//! history's are.

use std::{
    io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;
use transmission_rpc::types::Id;

use super::{commands::receive_once::derived_name, cycle::MAX_REASON_CHARS, CycleContext};
use crate::{
    revision::{crc_text, file_crc32, Release},
    store::{
        history::{HistoryResult, Millis},
        revisions::{Claim, OldVideo, Replacement, Revision, RevisionState, Step, OVERTAKEN},
    },
    transmission::{get_torrent, torrent_places, Redactor, TorrentPlace},
};

/// Why a revision without a CRC32 in its name is not received.
pub const NO_CRC: &str = "이름에 CRC32 값이 없어서 받은 영상을 확인할 수 없어 자동으로 받지 않았어요. 다시 받기로 받으면 확인 없이 이전 영상을 대체해요.";
/// Why a revision is not received when the folder's video matches no release.
pub const UNKNOWN_FILE: &str = "폴더에 있는 영상의 CRC32가 이 릴리스의 어느 수정본과도 달라서 버전을 알 수 없어요. 다시 받기로 받으면 이전 영상을 대체해요.";
/// Why a revision is not received when several torrents hold the folder's video.
pub const SHARED_FILE: &str = "폴더에 있는 영상을 토렌트 여러 개가 함께 가리키고 있어서 버전을 알 수 없어 자동으로 받지 않았어요.";
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
    /// The CRC32 of the episode's file, when it was read.
    pub old_crc: Option<String>,
}

impl Decided {
    /// What a row decided already.
    pub fn of(row: &Revision) -> Decided {
        Decided {
            episode_name: row.episode_name.clone(),
            version: row.new_version,
            crc: row.expected_crc.clone(),
            old_item_id: row.old_item_id,
            old_version: row.old_version,
            old_crc: row.old_crc.clone(),
        }
    }
}

/// A selected item as [`plan`] sees it.
pub struct Selected<'a> {
    pub channel_id: &'a str,
    pub identity_key: &'a str,
    pub title: &'a str,
    pub save_path: &'a Path,
    pub episode: isize,
}

/// The releases that replaced, or are replacing, a video
/// ([`crate::store::revisions::RevisionStore::replacements`]): a lower
/// revision of one of them, its first release included, is not received into
/// the same folder again, whichever channel or search it comes from.
pub struct Replaced(Vec<(String, String, u32)>);

impl Replaced {
    pub fn new(rows: Vec<Replacement>) -> Replaced {
        Replaced(
            rows.into_iter()
                .map(|r| (r.folder, Release::parse(&r.title).stem, r.new_version))
                .collect(),
        )
    }

    /// Whether `title`, received into `folder`, is lower than a release that
    /// replaced a video there.
    pub fn holds_higher(&self, folder: &Path, title: &str) -> bool {
        if self.0.is_empty() {
            return false;
        }
        let release = Release::parse(title);
        let folder = folder.to_string_lossy();
        self.0.iter().any(|(at, stem, version)| {
            *at == folder && *stem == release.stem && release.version < *version
        })
    }
}

/// Whether the release name `title` is a revision at all: the cheap test that
/// leaves every other item to the cycle as before.
pub fn is_revision(title: &str) -> bool {
    Release::parse(title).version > 1
}

/// The episode's file name in `save_path` for the release `title`, as the
/// rule cycle's renaming derives it (with the rule's `episode` conversion),
/// read from the name without its revision ([`Release::without_version`]).
pub fn episode_name(save_path: &Path, title: &str, episode: isize) -> Option<String> {
    derived_name(save_path, &Release::without_version(title), episode)
}

/// Transmission's whole file list, read when first needed and then shared:
/// one cycle's decisions ([`plan`]) read it at most once, and [`advance`]
/// reads it again only after it changed Transmission itself. Its answer grows
/// with everything Transmission holds.
#[derive(Default)]
pub struct Listing {
    read: Mutex<Option<Result<Arc<Vec<TorrentPlace>>, String>>>,
}

impl Listing {
    pub fn new() -> Listing {
        Listing::default()
    }

    async fn get(&self, ctx: &CycleContext) -> Result<Arc<Vec<TorrentPlace>>, String> {
        let mut read = self.read.lock().await;
        if let Some(places) = &*read {
            return places.clone();
        }
        let mut transmission = ctx.transmission();
        let places = torrent_places(&mut transmission, None, true)
            .await
            .map(Arc::new)
            .map_err(|err| err.to_string());
        *read = Some(places.clone());
        places
    }

    /// Transmission changed: the next [`Listing::get`] reads it again.
    async fn forget(&self) {
        *self.read.lock().await = None;
    }
}

/// Which torrents hold a file.
#[derive(Debug)]
enum Owner<'a> {
    Nobody,
    One(&'a TorrentPlace),
    Several(Vec<&'a TorrentPlace>),
    /// A torrent's file has the file's path, but cannot be looked at to tell.
    Unsure,
}

/// The torrents (of `places`) one of whose files is the file at `path`: a
/// file of the same name whose device and inode are `path`'s, so a folder
/// Transmission spells another way (a symbolic link, a doubled slash) is
/// still the same file.
fn owner_of<'a>(places: &'a [TorrentPlace], path: &Path) -> io::Result<Owner<'a>> {
    let file = std::fs::metadata(path)?;
    let mut owners: Vec<&TorrentPlace> = Vec::new();
    let mut unsure = false;
    for place in places {
        for listed in &place.files {
            let candidate = Path::new(&place.download_dir).join(&listed.name);
            if candidate.file_name() != path.file_name() {
                continue;
            }
            match std::fs::metadata(&candidate) {
                Ok(meta) if meta.dev() == file.dev() && meta.ino() == file.ino() => {
                    if !owners.iter().any(|owner| owner.hash == place.hash) {
                        owners.push(place);
                    }
                }
                Ok(_) => {}
                Err(_) => unsure |= candidate == path,
            }
        }
    }
    Ok(match owners.len() {
        0 if unsure => Owner::Unsure,
        0 => Owner::Nobody,
        1 => Owner::One(owners[0]),
        _ => Owner::Several(owners),
    })
}

/// Whether `a` and `b` are the same folder, however spelled.
fn same_folder(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// One CRC32 read at a time, whatever runs them: each holds a buffer of
/// [`crate::revision::CRC_BUFFER`] and reads a whole video.
static CRC_READS: Semaphore = Semaphore::const_new(1);

/// Reads the CRC32 of `path` off the async threads, one file at a time.
async fn crc_of(path: PathBuf) -> io::Result<u32> {
    let permit = CRC_READS.acquire().await.map_err(io::Error::other)?;
    tokio::task::spawn_blocking(move || {
        // Held by the read itself: a cycle that is dropped does not let
        // another read start beside it.
        let _permit = permit;
        file_crc32(&path)
    })
    .await
    .map_err(io::Error::other)?
}

/// Decides what to do with a selected item (see the module docs).
pub async fn plan(ctx: &CycleContext, item: &Selected<'_>, listing: &Listing) -> Plan {
    let release = Release::parse(item.title);
    if release.version < 2 {
        return Plan::Normal;
    }
    let Some(episode_name) = episode_name(item.save_path, item.title, item.episode) else {
        return Plan::Normal;
    };
    let target = item.save_path.join(&episode_name);
    match std::fs::symlink_metadata(&target) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Plan::Normal,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Plan::Normal,
        Err(err) => return Plan::Later(format!("cannot look at {}: {err}", target.display())),
    }
    let decided = |old: Option<(i64, u32)>, old_crc: Option<u32>| Decided {
        episode_name: episode_name.clone(),
        version: release.version,
        crc: release.crc.map(crc_text),
        old_item_id: old.map(|(id, _)| id),
        old_version: old.map(|(_, v)| v),
        old_crc: old_crc.map(crc_text),
    };

    let own = match ctx
        .history
        .item_by_key(item.channel_id.to_owned(), item.identity_key.to_owned())
        .await
    {
        Ok(own) => own.and_then(|own| own.torrent_hash),
        Err(err) => return Plan::Later(err.to_string()),
    };
    // The item's own torrent holds the name: it was received before the
    // folder had the episode, and renamed. Asked about alone, which settles
    // every cycle after the first for such an item.
    if let Some(own) = &own {
        let mut transmission = ctx.transmission();
        match torrent_places(&mut transmission, Some(std::slice::from_ref(own)), true).await {
            Ok(places) => match owner_of(&places, &target) {
                Ok(Owner::One(_)) => return Plan::Normal,
                Ok(_) => {}
                Err(err) => {
                    return Plan::Later(format!("cannot look at {}: {err}", target.display()))
                }
            },
            Err(err) => return Plan::Later(err.to_string()),
        }
    }
    let places = match listing.get(ctx).await {
        Ok(places) => places,
        Err(err) => return Plan::Later(err),
    };
    match owner_of(&places, &target) {
        Err(err) => return Plan::Later(format!("cannot look at {}: {err}", target.display())),
        Ok(Owner::Unsure) => {
            return Plan::Later(format!(
                "cannot tell which torrent holds {}",
                target.display()
            ))
        }
        Ok(Owner::Several(_)) => return Plan::Unknown(decided(None, None), SHARED_FILE),
        Ok(Owner::One(owner)) => {
            if own.as_deref() == Some(owner.hash.as_str()) {
                return Plan::Normal;
            }
            let records = match ctx.history.items_of_hash(&owner.hash).await {
                Ok(records) => records,
                Err(err) => return Plan::Later(err.to_string()),
            };
            if let Some((id, old)) = records
                .iter()
                .map(|record| (record.id, Release::parse(&record.title)))
                .find(|(_, old)| old.stem == release.stem)
            {
                let old = Some((id, old.version));
                if old.is_some_and(|(_, version)| version >= release.version) {
                    return Plan::Skip(decided(old, None), NOT_HIGHER);
                }
                if release.crc.is_none() {
                    return Plan::Unknown(decided(old, None), NO_CRC);
                }
                return Plan::Replace(decided(old, None));
            }
            if !records.is_empty() {
                // Another release's video: a duplicate, which keeps both files.
                return Plan::Normal;
            }
        }
        Ok(Owner::Nobody) => {}
    }

    // A video of unknown revision: its content says which it is. Read even
    // for a name without a CRC32, so a later `다시 받기` removes this file only.
    let file_crc = match crc_of(target.clone()).await {
        Ok(file_crc) => file_crc,
        Err(err) => return Plan::Later(format!("cannot read {}: {err}", target.display())),
    };
    if release.crc == Some(file_crc) {
        return Plan::Skip(decided(None, Some(file_crc)), ALREADY_THERE);
    }
    let done = match ctx
        .revisions
        .of_episode(
            item.save_path.to_string_lossy().into_owned(),
            episode_name.clone(),
        )
        .await
    {
        Ok(rows) => rows,
        Err(err) => return Plan::Later(err.to_string()),
    };
    let file_text = crc_text(file_crc);
    if done.iter().any(|row| {
        row.state == RevisionState::Done
            && row.new_version >= release.version
            && row.file_crc.as_deref() == Some(file_text.as_str())
    }) {
        return Plan::Skip(decided(None, Some(file_crc)), NOT_HIGHER);
    }
    let titles = match ctx
        .history
        .titles_of_channel(item.channel_id.to_owned())
        .await
    {
        Ok(titles) => titles,
        Err(err) => return Plan::Later(err.to_string()),
    };
    let same: Vec<(i64, Release)> = titles
        .into_iter()
        .map(|(id, title)| (id, Release::parse(&title)))
        .filter(|(_, other)| other.stem == release.stem && other.crc == Some(file_crc))
        .collect();
    if same
        .iter()
        .any(|(_, other)| other.version >= release.version)
    {
        return Plan::Skip(decided(None, Some(file_crc)), NOT_HIGHER);
    }
    let old = same
        .iter()
        .find(|(_, other)| other.version < release.version)
        .map(|(id, other)| (*id, other.version));
    if release.crc.is_none() {
        return Plan::Unknown(decided(old, Some(file_crc)), NO_CRC);
    }
    match old {
        Some(_) => Plan::Replace(decided(old, Some(file_crc))),
        None => Plan::Unknown(decided(None, Some(file_crc)), UNKNOWN_FILE),
    }
}

/// `다시 받기` received the `버전 미상` item `item_id` as the torrent `hash`: its
/// replacement goes ahead, checking the CRC32 only when the name carries one.
/// Returns whether the item has a replacement row past `버전 미상`, whose
/// torrent the caller must not rename; an error when that cannot be known
/// (the caller tries the whole request again rather than rename).
pub async fn confirm(
    ctx: &CycleContext,
    item_id: i64,
    title: &str,
    at: Millis,
    hash: &str,
) -> Result<bool, String> {
    let expected = Release::parse(title).crc.map(crc_text);
    match ctx
        .revisions
        .confirm(item_id, at, hash.to_owned(), expected)
        .await
    {
        Ok(Some(row)) => Ok(row.state != RevisionState::Unknown),
        Ok(None) => Ok(false),
        Err(err) => Err(format!(
            "cannot confirm the revision of item {item_id}: {err}"
        )),
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
const NOT_COMPLETE: &str = "새 영상 파일을 다 받지 않았어요. 이전 영상은 그대로 있어요.";
const EMPTY: &str = "받은 새 영상 파일이 비어 있어서 대체하지 않았어요. 이전 영상은 그대로 있어요.";
const NEW_FILE_GONE: &str = "받은 새 영상 파일이 없어요. 이전 영상은 그대로 있어요.";
const UNKNOWN_TORRENT: &str = "새 영상의 토렌트를 알 수 없어요. 이전 영상은 그대로 있어요.";
const NAME_TAKEN_BY_NEW: &str =
    "새 영상의 토렌트가 이미 회차 이름의 파일을 가리키고 있어서 대체하지 않았어요. 두 파일을 그대로 뒀어요.";
const OTHER_RELEASE: &str =
    "회차 이름의 영상이 이 릴리스의 낮은 수정본이 아니라서 대체하지 않았어요. 두 파일을 그대로 뒀어요.";
const IN_PLACE: &str =
    "회차 이름의 영상이 이미 같거나 더 높은 수정본이라 이 수정본은 받은 이름 그대로 뒀어요.";
const OLD_UNCHECKED: &str =
    "회차 이름의 파일이 대체를 정한 영상인지 확인할 수 없어서 지우지 않았어요. 두 파일을 그대로 뒀어요.";
const OLD_CHANGED: &str =
    "회차 이름의 파일이 대체를 정할 때의 영상과 달라서 지우지 않았어요. 두 파일을 그대로 뒀어요.";
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
    let listing = Listing::new();
    for row in rows {
        if cancel.is_cancelled() {
            return;
        }
        drive(ctx, row, at, redactor, &listing).await;
    }
}

/// `text` as a row keeps it: free of secret values and capped like history's
/// reasons.
fn clean(text: &str, redactor: &Redactor) -> String {
    redactor
        .apply(text)
        .chars()
        .take(MAX_REASON_CHARS)
        .collect()
}

/// `step` with its reason [`clean`].
fn cleaned(step: Step, redactor: &Redactor) -> Step {
    match step {
        Step::Failed {
            reason,
            received_name,
        } => Step::Failed {
            reason: clean(&reason, redactor),
            received_name,
        },
        Step::Removed { reason } => Step::Removed {
            reason: reason.map(|reason| clean(&reason, redactor)),
        },
        Step::Skipped { reason } => Step::Skipped {
            reason: clean(&reason, redactor),
        },
        other => other,
    }
}

async fn drive(
    ctx: &CycleContext,
    mut row: Revision,
    at: Millis,
    redactor: &Redactor,
    listing: &Listing,
) {
    loop {
        let next = match row.state {
            RevisionState::Receiving => received(ctx, &row).await,
            RevisionState::Verified | RevisionState::Removing => {
                remove_old(ctx, &mut row, at, listing).await
            }
            RevisionState::Removed => rename(ctx, &row, listing).await,
            RevisionState::Failed if row.not_received() => recover(ctx, &row).await,
            RevisionState::Failed => cleared(&row),
            _ => return,
        };
        let step = match next {
            Next::Step(step) => cleaned(step, redactor),
            Next::Wait => return,
            Next::Later(why) => {
                return println!(
                    "Revision of {} waits: {}",
                    row.episode_name,
                    redactor.apply(&why)
                )
            }
        };
        // A rename that keeps failing, or a failure the worker keeps looking
        // at, is written once for the same reason.
        let again = match &step {
            Step::Removed { reason } => {
                row.state == RevisionState::Removed && *reason == row.reason
            }
            Step::Failed { reason, .. } => {
                row.state == RevisionState::Failed && Some(reason) == row.reason.as_ref()
            }
            _ => false,
        };
        if again {
            return;
        }
        if let Err(err) = ctx.revisions.advance(row.id, at, step.clone()).await {
            return eprintln!("Cannot record the revision of {}: {err}", row.episode_name);
        }
        println!("Revision of {}: {step:?}", row.episode_name);
        match step {
            Step::Receiving => {
                row.state = RevisionState::Receiving;
                row.reason = None;
            }
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
            Step::Done | Step::Failed { .. } | Step::Cleared | Step::Skipped { .. } => return,
        }
    }
}

fn failed(reason: impl Into<String>, received_name: Option<String>) -> Next {
    Next::Step(Step::Failed {
        reason: reason.into(),
        received_name,
    })
}

fn skipped(reason: &str) -> Next {
    Next::Step(Step::Skipped {
        reason: reason.to_owned(),
    })
}

/// Step 1: the new video is all there, a file of its own in the rule's
/// folder, and its CRC32 is the name's.
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
    // Not received here: the received name would not be in the folder.
    if !same_folder(Path::new(&place.download_dir), Path::new(&row.folder)) {
        return failed(ELSEWHERE, None);
    }
    // The torrent names the episode's file: the old video, not a new one.
    if file.name == row.episode_name {
        return failed(NAME_TAKEN_BY_NEW, None);
    }
    if !file.complete {
        return failed(NOT_COMPLETE, None);
    }
    if file.length <= 0 {
        return failed(EMPTY, Some(file.name.clone()));
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

/// A failure before the new video was received, looked at again: once the
/// torrent is right it goes on (step 1), and once the old video is gone
/// there is nothing to replace.
async fn recover(ctx: &CycleContext, row: &Revision) -> Next {
    let old = Path::new(&row.folder).join(&row.episode_name);
    if matches!(exists(&old), Ok(false)) {
        return Next::Step(Step::Cleared);
    }
    match received(ctx, row).await {
        Next::Wait => Next::Step(Step::Receiving),
        other => other,
    }
}

fn exists(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

/// Step 2: the old video is removed, or the replacement fails (or is
/// skipped) with it in place. `removing` is written before anything is
/// removed, by a claim that lets one replacement of the episode at a time.
async fn remove_old(ctx: &CycleContext, row: &mut Revision, at: Millis, listing: &Listing) -> Next {
    // Another replacement of the episode first: before what is at the name
    // is looked at, which it may be changing.
    if row.state == RevisionState::Verified {
        match ctx.revisions.verdict(row.id).await {
            Ok(Claim::Go) => {}
            Ok(Claim::Wait) => return Next::Wait,
            Ok(Claim::Overtaken) => return skipped(OVERTAKEN),
            Err(err) => return Next::Later(err.to_string()),
        }
    }
    let old = Path::new(&row.folder).join(&row.episode_name);
    let present = match exists(&old) {
        Ok(present) => present,
        Err(err) => return Next::Later(format!("cannot look at {}: {err}", old.display())),
    };
    let found = if present {
        match old_video(ctx, row, &old, listing).await {
            Ok(found) => found,
            Err(next) => return next,
        }
    } else {
        OldVideo {
            item_id: None,
            version: None,
            torrent_hash: None,
        }
    };
    match ctx.revisions.claim(row.id, at, found.clone()).await {
        Ok(Claim::Go) => row.state = RevisionState::Removing,
        Ok(Claim::Wait) => return Next::Wait,
        Ok(Claim::Overtaken) => return skipped(OVERTAKEN),
        Err(err) => return Next::Later(err.to_string()),
    }
    if !present {
        return Next::Step(Step::Removed { reason: None });
    }

    match &found.torrent_hash {
        Some(hash) => {
            let mut transmission = ctx.transmission();
            let removed = transmission
                .torrent_remove(vec![Id::Hash(hash.clone())], true)
                .await;
            listing.forget().await;
            match removed {
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

/// What is at the episode name `old` now, when it is a lower revision of the
/// row's release that may be removed; otherwise what to do instead.
async fn old_video(
    ctx: &CycleContext,
    row: &Revision,
    old: &Path,
    listing: &Listing,
) -> Result<OldVideo, Next> {
    let places = listing.get(ctx).await.map_err(Next::Later)?;
    let owner = owner_of(&places, old)
        .map_err(|err| Next::Later(format!("cannot look at {}: {err}", old.display())))?;
    match owner {
        Owner::Unsure => Err(Next::Later(format!(
            "cannot tell which torrent holds {}",
            old.display()
        ))),
        Owner::Several(owners) => {
            let names: Vec<&str> = owners.iter().map(|owner| owner.name.as_str()).collect();
            Err(failed(
                format!(
                    "이전 영상을 토렌트 여러 개({})가 함께 가리키고 있어서 지우면 다른 토렌트의 데이터까지 지울 수 있어 대체하지 않았어요. 두 파일을 그대로 뒀어요.",
                    names.join(", ")
                ),
                None,
            ))
        }
        Owner::One(owner) => {
            if row.torrent_hash.as_deref() == Some(owner.hash.as_str()) {
                return Err(failed(NAME_TAKEN_BY_NEW, None));
            }
            if owner.files.len() > 1 {
                return Err(failed(
                    format!(
                        "이전 영상이 파일 여러 개를 담은 토렌트 {}에 있어서 다른 회차까지 지울 수 있어 대체하지 않았어요. 두 파일을 그대로 뒀어요.",
                        owner.name
                    ),
                    None,
                ));
            }
            let records = ctx
                .history
                .items_of_hash(&owner.hash)
                .await
                .map_err(|err| Next::Later(err.to_string()))?;
            if !records
                .iter()
                .any(|record| record.result == HistoryResult::Received)
            {
                return Err(failed(
                    format!(
                        "이전 영상이 trss가 추가하지 않은 토렌트 {}에 있어서 대체하지 않았어요. 두 파일을 그대로 뒀어요.",
                        owner.name
                    ),
                    None,
                ));
            }
            let new = new_release(ctx, row).await?;
            let Some((id, version)) = records.iter().find_map(|record| {
                let release = Release::parse(&record.title);
                (release.stem == new.stem).then_some((record.id, release.version))
            }) else {
                return Err(failed(OTHER_RELEASE, None));
            };
            if version >= row.new_version {
                return Err(skipped(IN_PLACE));
            }
            Ok(OldVideo {
                item_id: Some(id),
                version: Some(version),
                torrent_hash: Some(owner.hash.clone()),
            })
        }
        Owner::Nobody => {
            // A file no torrent holds is deleted only when it is the video
            // decided about (or one a replacement of the episode put there),
            // as its CRC32 tells.
            let mut known: Vec<(String, Option<i64>, Option<u32>)> = Vec::new();
            if let Some(crc) = &row.old_crc {
                known.push((crc.clone(), row.old_item_id, row.old_version));
            }
            if let Some(id) = row.old_item_id {
                let item = ctx
                    .history
                    .get(id)
                    .await
                    .map_err(|err| Next::Later(err.to_string()))?;
                if let Some(item) = item {
                    let release = Release::parse(&item.title);
                    if let Some(crc) = release.crc {
                        known.push((crc_text(crc), Some(id), Some(release.version)));
                    }
                }
            }
            let rows = ctx
                .revisions
                .of_episode(row.folder.clone(), row.episode_name.clone())
                .await
                .map_err(|err| Next::Later(err.to_string()))?;
            for done in rows {
                if let (RevisionState::Done, Some(crc)) = (done.state, done.file_crc) {
                    if done.new_version < row.new_version {
                        known.push((crc, Some(done.item_id), Some(done.new_version)));
                    }
                }
            }
            if known.is_empty() {
                return Err(failed(OLD_UNCHECKED, None));
            }
            let crc = match crc_of(old.to_owned()).await {
                Ok(crc) => crc_text(crc),
                Err(err) => {
                    return Err(Next::Later(format!("cannot read {}: {err}", old.display())))
                }
            };
            match known.into_iter().find(|(known, _, _)| *known == crc) {
                Some((_, item_id, version)) => Ok(OldVideo {
                    item_id,
                    version,
                    torrent_hash: None,
                }),
                None => Err(failed(OLD_CHANGED, None)),
            }
        }
    }
}

/// The row's new release, as its history item names it.
async fn new_release(ctx: &CycleContext, row: &Revision) -> Result<Release, Next> {
    match ctx.history.get(row.item_id).await {
        Ok(Some(item)) => Ok(Release::parse(&item.title)),
        Ok(None) => Err(failed(OTHER_RELEASE, None)),
        Err(err) => Err(Next::Later(err.to_string())),
    }
}

/// Step 3: the new video takes the episode name while the name is free.
async fn rename(ctx: &CycleContext, row: &Revision, listing: &Listing) -> Next {
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
            let renamed = transmission
                .torrent_rename_path(
                    vec![Id::Hash(hash)],
                    received_name.clone(),
                    row.episode_name.clone(),
                )
                .await;
            listing.forget().await;
            match renamed {
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
    use std::path::Path;

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

    /// `trname` reads Erai-raws' `06v2` as episode 34 (from the CRC32
    /// bracket); the revision's episode name is the first release's.
    #[test]
    fn a_revisions_episode_name_is_its_first_releases() {
        let folder = Path::new("/media/Show/Season 01");
        let v1 = "[Erai-raws] Show - 06 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv";
        let v2 = "[Erai-raws] Show - 06v2 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv";
        assert_eq!(
            derived_name(folder, v2, 0).as_deref(),
            Some("Show S01E34.mkv"),
            "trname alone"
        );
        assert_eq!(
            episode_name(folder, v2, 0).as_deref(),
            Some("Show S01E06.mkv")
        );
        assert_eq!(episode_name(folder, v1, 0), derived_name(folder, v1, 0));
        let subsplease = "[SubsPlease] Show - 14v2 (1080p) [8F2EFECC].mkv";
        assert_eq!(
            episode_name(folder, subsplease, 0).as_deref(),
            Some("Show S01E14.mkv")
        );
        // An RSS title without the extension gives no name at all.
        assert_eq!(
            episode_name(folder, "[SubsPlease] Show - 14v2 (1080p) [8F2EFECC]", 0),
            None
        );
    }

    #[test]
    fn a_kept_reason_is_redacted_and_capped() {
        let mut redactor = Redactor::default();
        redactor.add("s3cr3t-token");
        let long = format!("Transmission said s3cr3t-token {}", "가".repeat(400));
        let step = cleaned(
            Step::Failed {
                reason: long,
                received_name: None,
            },
            &redactor,
        );
        let Step::Failed { reason, .. } = step else {
            panic!("{step:?}")
        };
        assert!(!reason.contains("s3cr3t-token"), "{reason}");
        assert_eq!(reason.chars().count(), MAX_REASON_CHARS);
        let Step::Removed { reason } = cleaned(
            Step::Removed {
                reason: Some("busy s3cr3t-token".into()),
            },
            &redactor,
        ) else {
            panic!()
        };
        assert!(!reason.unwrap().contains("s3cr3t-token"));
    }

    #[test]
    fn a_file_is_held_by_the_torrents_whose_file_it_is_however_spelled() {
        let dir = tempfile::tempdir().unwrap();
        let season = dir.path().join("Season 01");
        std::fs::create_dir(&season).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&season, &link).unwrap();
        let file = season.join("Show S01E14.mkv");
        std::fs::write(&file, b"video").unwrap();
        let place = |hash: &str, dir: &Path, name: &str| TorrentPlace {
            hash: hash.into(),
            name: name.into(),
            download_dir: dir.to_str().unwrap().into(),
            files: vec![crate::transmission::TorrentFile {
                name: name.into(),
                length: 5,
                complete: true,
            }],
            ..Default::default()
        };

        let places = vec![place("a", &link, "Show S01E14.mkv")];
        assert!(matches!(owner_of(&places, &file).unwrap(), Owner::One(p) if p.hash == "a"));

        let places = vec![
            place("a", &link, "Show S01E14.mkv"),
            place("b", &season, "Show S01E14.mkv"),
        ];
        assert!(matches!(
            owner_of(&places, &file).unwrap(),
            Owner::Several(_)
        ));

        // Another file of the same name elsewhere is not it.
        let other = dir.path().join("Other");
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("Show S01E14.mkv"), b"video").unwrap();
        let places = vec![place("c", &other, "Show S01E14.mkv")];
        assert!(matches!(owner_of(&places, &file).unwrap(), Owner::Nobody));
    }
}
