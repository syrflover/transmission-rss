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
//!    If the one on its way then fails, the skipped one starts over from
//!    step 1 on a later cycle and replaces the video after all
//!    ([`Step::Overtaken`]).
//!    What is at the episode name is looked at again just before: if it is the
//!    single file of a torrent trss added (a `received` record in history) of
//!    a lower revision of the release, that torrent is removed with its data
//!    (removing only the file would make Transmission fetch it again); if no
//!    torrent holds it, the file is deleted, but only when its CRC32 is the
//!    one of the video decided about. Anything else (a batch, a torrent trss
//!    did not add or of another release, a file two torrents hold, a file that
//!    changed) is left alone and the replacement fails. A failure here leaves
//!    the old video in place. Right before the removal the new video is
//!    looked at again too: unless its received name still holds the file
//!    whose CRC32 was read (its identity, kept at step 1), nothing is removed
//!    and the replacement waits, and ends after a second such look.
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
    revision::{crc_text, file_crc32_identified, FileIdentity, Release},
    store::{
        history::{HistoryItem, HistoryResult, Millis},
        revisions::{Claim, OldVideo, Replacement, Revision, RevisionState, RowWrite, Step},
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
/// rule cycle's renaming derives it (with the rule's `episode` conversion,
/// read from the name without its revision).
pub fn episode_name(save_path: &Path, title: &str, episode: isize) -> Option<String> {
    derived_name(save_path, title, episode)
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
pub fn same_folder(a: &Path, b: &Path) -> bool {
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
    identified_crc_of(path).await.map(|(crc, _)| crc)
}

/// [`crc_of`], with the identity of the file that was read
/// ([`file_crc32_identified`]).
async fn identified_crc_of(path: PathBuf) -> io::Result<(u32, FileIdentity)> {
    let permit = CRC_READS.acquire().await.map_err(io::Error::other)?;
    tokio::task::spawn_blocking(move || {
        // Held by the read itself: a cycle that is dropped does not let
        // another read start beside it.
        let _permit = permit;
        file_crc32_identified(&path)
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
    plan_at(ctx, item, release, episode_name, listing).await
}

/// [`plan`] for the revision `release` of the episode file `episode_name` in
/// `item.save_path`.
async fn plan_at(
    ctx: &CycleContext,
    item: &Selected<'_>,
    release: Release,
    episode_name: String,
    listing: &Listing,
) -> Plan {
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

/// Whether the video at the episode's place of `row` is already `item`'s
/// revision or a higher one of its release, told the way [`plan`] tells it
/// when a cycle decides. The replacement rows alone do not say so: a higher
/// revision that found the episode name free was received as an ordinary
/// item, with no row. `다시 받기` of `item` is refused then, since its
/// replacement could replace nothing. `Err` when Transmission or the disk
/// could not be read.
pub async fn holds_same_or_higher(
    ctx: &CycleContext,
    item: &HistoryItem,
    row: &Revision,
) -> Result<bool, String> {
    let selected = Selected {
        channel_id: &item.channel_id,
        identity_key: &item.identity_key,
        title: &item.title,
        save_path: Path::new(&row.folder),
        episode: 0,
    };
    let release = Release::parse(&item.title);
    let plan = plan_at(
        ctx,
        &selected,
        release,
        row.episode_name.clone(),
        &Listing::new(),
    )
    .await;
    match plan {
        Plan::Skip(..) => Ok(true),
        Plan::Later(why) => Err(why),
        Plan::Normal | Plan::Replace(_) | Plan::Unknown(..) => Ok(false),
    }
}

/// What `다시 받기` of the `버전 미상` item titled `title`, received as the
/// torrent `hash`, writes on its row with the item's result: its replacement
/// goes ahead, checking the CRC32 only when the name carries one. A row past
/// `버전 미상` afterwards is one whose torrent the caller must not rename.
pub fn confirm(title: &str, hash: &str) -> RowWrite {
    RowWrite::Confirm {
        hash: hash.to_owned(),
        expected_crc: Release::parse(title).crc.map(crc_text),
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

/// How the reason of a torrent that reported a local error begins.
const LOCAL_ERROR: &str = "새 영상을 받다 Transmission이 오류를 알렸어요: ";
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
const OLD_FILE_LEFT: &str = "이전 영상의 토렌트는 Transmission에서 지웠지만 회차 이름의 파일이 아직 있어요. 그 파일이 없어지면 새 영상에 회차 이름을 붙여요.";
const NEW_GONE_OLD_LEFT: &str = "이전 영상의 토렌트는 Transmission에서 지웠지만 회차 이름의 파일이 아직 있고, 받은 새 영상 파일은 없어요. 다음 확인에도 없으면 이 대체를 끝내요.";
const NEW_UNCHECKED_OLD_KEPT: &str = "받은 새 영상 파일이 없거나 CRC32를 확인한 파일과 달라서 이전 영상을 지우지 않았어요. 다음 확인에도 그러면 이 대체를 끝내요.";
/// Why a replacement whose new video is gone was ended (not shown).
const ABANDONED: &str =
    "받은 새 영상 파일이 이어진 두 번의 확인에서 모두 없어서 대체를 끝냈어요. 아무 파일도 지우지 않았어요.";

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
        Step::Abandoned { reason } => Step::Abandoned {
            reason: clean(&reason, redactor),
        },
        Step::NewMissing { reason } => Step::NewMissing {
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
            RevisionState::Removed => rename(ctx, &mut row, listing).await,
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
            Step::RemovalWaits { reason } => Some(reason) == row.reason.as_ref(),
            _ => false,
        };
        if again {
            return;
        }
        // Written only over the state it was decided on: an earlier row of
        // this pass may have moved this one on (skipped it) since it was read.
        match ctx
            .revisions
            .advance(row.id, at, row.state, step.clone())
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                return println!(
                    "Revision of {} moved on before {step:?} was written",
                    row.episode_name
                )
            }
            Err(err) => {
                return eprintln!("Cannot record the revision of {}: {err}", row.episode_name)
            }
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
                file_identity,
            } => {
                row.state = RevisionState::Verified;
                row.received_name = Some(received_name);
                row.file_crc = Some(file_crc);
                row.file_identity = Some(file_identity);
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
            Step::Done
            | Step::Failed { .. }
            | Step::Cleared
            | Step::Skipped { .. }
            | Step::Overtaken
            | Step::Abandoned { .. }
            | Step::NewMissing { .. }
            | Step::RemovalWaits { .. } => return,
        }
    }
}

/// Whether `row` failed because its download stopped before the new video
/// was received: its torrent left Transmission, or Transmission reported an
/// error on it. `다시 받기` receives such a revision again (and starts its
/// torrent when Transmission still has it); the other failures before the
/// video was received (several files, another folder, the episode's own
/// file) would end the same way again.
pub fn stopped_before_received(row: &Revision) -> bool {
    row.not_received()
        && row
            .reason
            .as_deref()
            .is_some_and(|reason| reason == RECEIVE_STOPPED || reason.starts_with(LOCAL_ERROR))
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
            format!("{LOCAL_ERROR}{error}. 이전 영상은 그대로 있어요."),
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
    // The identity of the file read: the old video is removed only while the
    // received name still holds that file.
    let (crc, identity) = match identified_crc_of(path.clone()).await {
        Ok((crc, identity)) => (crc_text(crc), identity),
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
        file_identity: identity.to_text(),
    })
}

/// A failure before the new video was received, looked at again: once the
/// torrent is right it goes on (step 1), and once the old video is gone
/// there is nothing to replace. A higher revision of the episode in place or
/// on its way skips it, as it would skip it before removing anything; and an
/// episode name left empty by another replacement of the episode, between
/// removing the old video and naming its new one, is not the old video gone.
async fn recover(ctx: &CycleContext, row: &Revision) -> Next {
    match ctx.revisions.verdict(row.id).await {
        Ok(Claim::Overtaken) => return Next::Step(Step::Overtaken),
        Ok(_) => {}
        Err(err) => return Next::Later(err.to_string()),
    }
    let old = Path::new(&row.folder).join(&row.episode_name);
    if matches!(exists(&old), Ok(false)) {
        let rows = match ctx
            .revisions
            .of_episode(row.folder.clone(), row.episode_name.clone())
            .await
        {
            Ok(rows) => rows,
            Err(err) => return Next::Later(err.to_string()),
        };
        let naming = rows.iter().any(|other| {
            other.id != row.id
                && matches!(
                    other.state,
                    RevisionState::Removing | RevisionState::Removed
                )
        });
        if !naming {
            return Next::Step(Step::Cleared);
        }
    }
    match received(ctx, row).await {
        Next::Wait => Next::Step(Step::Receiving),
        other => other,
    }
}

/// Whether the row's new video is gone: its received name is not in the
/// folder. A folder that is not there is a mount that is away, not a video
/// that was deleted: that is an error, and the look is tried again later.
fn new_video_gone(row: &Revision) -> Result<bool, String> {
    let folder = Path::new(&row.folder);
    match std::fs::metadata(folder) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(format!("{} is not a folder", folder.display())),
        Err(err) => return Err(format!("cannot look at {}: {err}", folder.display())),
    }
    let Some(name) = &row.received_name else {
        return Ok(true);
    };
    let path = folder.join(name);
    exists(&path)
        .map(|present| !present)
        .map_err(|err| format!("cannot look at {}: {err}", path.display()))
}

/// A look, with the folder there, found the row's new video missing (or not
/// the checked one): the first such look waits with `reason`, and the next
/// one in a row ends the replacement. Looks that find the video, or the
/// folder away, break the run ([`new_video_found`], [`folder_away`]).
fn new_video_missed(row: &Revision, reason: &str) -> Next {
    println!(
        "Revision of {}: the new video {} is missing or not the one checked",
        row.episode_name,
        row.received_name.as_deref().unwrap_or("(unknown)")
    );
    if row.new_missing_at.is_some() {
        Next::Step(Step::Abandoned {
            reason: ABANDONED.to_owned(),
        })
    } else {
        Next::Step(Step::NewMissing {
            reason: reason.to_owned(),
        })
    }
}

/// A look found the row's new video: an earlier miss no longer counts, and
/// its reason `miss_reason` goes with it.
async fn new_video_found(
    ctx: &CycleContext,
    row: &mut Revision,
    miss_reason: Option<&str>,
) -> Result<(), Next> {
    if row.new_missing_at.is_none() {
        return Ok(());
    }
    ctx.revisions
        .forget_miss(row.id, miss_reason.map(str::to_owned))
        .await
        .map_err(|err| Next::Later(err.to_string()))?;
    row.new_missing_at = None;
    if row.reason.is_some() && row.reason.as_deref() == miss_reason {
        row.reason = None;
    }
    Ok(())
}

/// A look found the row's folder away (`why`): it decides nothing, and an
/// earlier miss no longer counts toward two in a row.
async fn folder_away(ctx: &CycleContext, row: &mut Revision, why: String) -> Next {
    if row.new_missing_at.is_some() {
        if let Err(err) = ctx.revisions.forget_miss(row.id, None).await {
            return Next::Later(err.to_string());
        }
        row.new_missing_at = None;
    }
    Next::Later(why)
}

/// Whether the row's new video is still the file whose CRC32 was checked
/// ([`Revision::file_identity`]): under its received name in the folder,
/// with the same identity. A row checked before identities were kept needs
/// the file there with the length its torrent gives it in the folder.
/// `Err` when the folder, the file or Transmission could not be looked at.
async fn new_video_kept(ctx: &CycleContext, row: &Revision) -> Result<bool, String> {
    if new_video_gone(row)? {
        return Ok(false);
    }
    let Some(name) = &row.received_name else {
        return Ok(false);
    };
    let path = Path::new(&row.folder).join(name);
    let now = match FileIdentity::at(&path) {
        Ok(now) => now,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(format!("cannot look at {}: {err}", path.display())),
    };
    if let Some(seen) = row.file_identity.as_deref().and_then(FileIdentity::parse) {
        return Ok(seen == now);
    }
    let Some(hash) = row.torrent_hash.clone() else {
        return Ok(false);
    };
    let mut transmission = ctx.transmission();
    let place = match torrent_places(&mut transmission, Some(&[hash]), true).await {
        Ok(places) => places.into_iter().next(),
        Err(err) => return Err(err.to_string()),
    };
    Ok(place.is_some_and(|place| {
        same_folder(Path::new(&place.download_dir), Path::new(&row.folder))
            && place.files.iter().any(|file| {
                file.name == *name
                    && file.complete
                    && u64::try_from(file.length).is_ok_and(|length| length == now.size())
            })
    }))
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
            Ok(Claim::Overtaken) => return Next::Step(Step::Overtaken),
            Err(err) => return Next::Later(err.to_string()),
        }
    }
    let old = Path::new(&row.folder).join(&row.episode_name);
    let present = match exists(&old) {
        Ok(present) => present,
        Err(err) => return Next::Later(format!("cannot look at {}: {err}", old.display())),
    };
    // The old torrent was asked to go (perhaps with no answer): once
    // Transmission no longer holds it, its file is the old video's data,
    // which Transmission deletes after it answers, or could not delete. It is
    // not looked at as an old video again (the torrent that told its revision
    // is gone); the replacement waits, as removing, for the file to go. If
    // the new video is gone instead (the person deleted it, keeping the old
    // one), the replacement ends once a second look finds it gone too.
    if let (true, RevisionState::Removing, Some(hash)) =
        (present, row.state, row.old_torrent_hash.as_ref())
    {
        let mut transmission = ctx.transmission();
        match get_torrent(&mut transmission, hash).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return match new_video_gone(row) {
                    Ok(false) => match new_video_found(ctx, row, None).await {
                        Ok(()) => Next::Step(Step::RemovalWaits {
                            reason: OLD_FILE_LEFT.to_owned(),
                        }),
                        Err(next) => next,
                    },
                    Ok(true) => new_video_missed(row, NEW_GONE_OLD_LEFT),
                    Err(why) => folder_away(ctx, row, why).await,
                }
            }
            Err(err) => return Next::Later(err.to_string()),
        }
    }
    let (found, identity) = if present {
        match old_video(ctx, row, &old, listing).await {
            Ok(found) => found,
            Err(next) => return next,
        }
    } else {
        let none = OldVideo {
            item_id: None,
            version: None,
            torrent_hash: None,
        };
        (none, None)
    };
    match ctx.revisions.claim(row.id, at, found.clone()).await {
        Ok(Claim::Go) => row.state = RevisionState::Removing,
        Ok(Claim::Wait) => return Next::Wait,
        Ok(Claim::Overtaken) => return Next::Step(Step::Overtaken),
        Err(err) => return Next::Later(err.to_string()),
    }
    if !present {
        return Next::Step(Step::Removed { reason: None });
    }
    // What is at the name now must be the file that was looked at: another
    // program may have put a file there since (an atomic rename), which the
    // removal below would take with it.
    match (identity, FileIdentity::at(&old)) {
        (Some(seen), Ok(now)) if seen == now => {}
        // Gone since it was looked at (a removal Transmission is carrying
        // out, or the person): the old video is removed, which is the
        // `!present` case above, with the old video it was claimed for.
        (_, Err(err)) if err.kind() == io::ErrorKind::NotFound => {
            return Next::Step(Step::Removed { reason: None })
        }
        (_, Err(err)) => return Next::Later(format!("cannot look at {}: {err}", old.display())),
        _ => return failed(OLD_CHANGED, None),
    }
    // And the new video must still be the file whose CRC32 was checked: one
    // deleted or put over since would leave the episode with neither. Not
    // the checked one on two looks in a row (one per cycle), the replacement
    // ends; the first may be a mount that was away for a moment.
    match new_video_kept(ctx, row).await {
        Ok(true) => {
            if let Err(next) = new_video_found(ctx, row, Some(NEW_UNCHECKED_OLD_KEPT)).await {
                return next;
            }
        }
        Ok(false) => return new_video_missed(row, NEW_UNCHECKED_OLD_KEPT),
        Err(why) => return folder_away(ctx, row, why).await,
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
                // No answer: Transmission may have removed it all the same.
                // The row stays removing, and the next look tells which.
                Err(err) => {
                    return Next::Later(format!(
                        "no answer to removing the old torrent {hash}: {err}"
                    ))
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
/// row's release that may be removed, with the identity of the file that was
/// looked at; otherwise what to do instead.
async fn old_video(
    ctx: &CycleContext,
    row: &Revision,
    old: &Path,
    listing: &Listing,
) -> Result<(OldVideo, Option<FileIdentity>), Next> {
    let places = listing.get(ctx).await.map_err(Next::Later)?;
    // Taken before the torrents are matched against it.
    let seen = FileIdentity::at(old)
        .map_err(|err| Next::Later(format!("cannot look at {}: {err}", old.display())))?;
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
            if owner.files.iter().any(|file| file.name.contains('/')) {
                // One file, inside a folder of the torrent's: removing the
                // torrent with its data takes what is left of the folder too.
                return Err(failed(
                    format!(
                        "이전 영상이 토렌트 {}의 폴더 안에 있어서 폴더째 지울 수 있어 대체하지 않았어요. 두 파일을 그대로 뒀어요.",
                        owner.name
                    ),
                    None,
                ));
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
            let found = OldVideo {
                item_id: Some(id),
                version: Some(version),
                torrent_hash: Some(owner.hash.clone()),
            };
            Ok((found, Some(seen)))
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
            // The identity is the read file's own: the name is checked
            // against it right before the file is deleted.
            let (crc, read) = match identified_crc_of(old.to_owned()).await {
                Ok((crc, read)) => (crc_text(crc), read),
                Err(err) => {
                    return Err(Next::Later(format!("cannot read {}: {err}", old.display())))
                }
            };
            match known.into_iter().find(|(known, _, _)| *known == crc) {
                Some((_, item_id, version)) => {
                    let found = OldVideo {
                        item_id,
                        version,
                        torrent_hash: None,
                    };
                    Ok((found, Some(read)))
                }
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
async fn rename(ctx: &CycleContext, row: &mut Revision, listing: &Listing) -> Next {
    let folder = PathBuf::from(&row.folder);
    let folder = folder.as_path();
    let target = folder.join(&row.episode_name);
    match exists(&target) {
        Ok(false) => {}
        Ok(true) => {
            return match renamed_already(ctx, row, &target).await {
                Ok(true) => Next::Step(Step::Done),
                Ok(false) => Next::Step(Step::Removed {
                    reason: Some(DESTINATION_TAKEN.to_owned()),
                }),
                Err(why) => Next::Later(why),
            }
        }
        Err(err) => return Next::Later(format!("cannot look at {}: {err}", target.display())),
    }
    // The new video missing on two looks in a row (one per cycle) ends the
    // replacement: the first may be a mount that was away for a moment. The
    // reason of a miss stays until the rename goes through or says why not.
    match new_video_gone(row) {
        Ok(false) => {
            if let Err(next) = new_video_found(ctx, row, None).await {
                return next;
            }
        }
        Ok(true) => return new_video_missed(row, NEW_FILE_MISSING),
        Err(why) => return folder_away(ctx, row, why).await,
    }
    let Some(received_name) = row.received_name.clone() else {
        return Next::Step(Step::Removed {
            reason: Some(NEW_FILE_MISSING.to_owned()),
        });
    };
    let source = folder.join(&received_name);

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

/// Whether the file at the episode name `target` is the row's own new video,
/// renamed by an earlier look that did not get to write `done` (it stopped,
/// or the write failed): the received name is gone, and the file is the one
/// the row's torrent holds, or (the torrent gone too) its CRC32 is the one
/// read when the new video was checked.
async fn renamed_already(
    ctx: &CycleContext,
    row: &Revision,
    target: &Path,
) -> Result<bool, String> {
    let Some(received_name) = &row.received_name else {
        return Ok(false);
    };
    match exists(&Path::new(&row.folder).join(received_name)) {
        Ok(false) => {}
        Ok(true) => return Ok(false),
        Err(err) => return Err(format!("cannot look at {received_name}: {err}")),
    }
    if let Some(hash) = &row.torrent_hash {
        let mut transmission = ctx.transmission();
        let places = torrent_places(&mut transmission, Some(std::slice::from_ref(hash)), true)
            .await
            .map_err(|err| err.to_string())?;
        if !places.is_empty() {
            return match owner_of(&places, target) {
                Ok(Owner::One(_)) => Ok(true),
                Ok(_) => Ok(false),
                Err(err) => Err(format!("cannot look at {}: {err}", target.display())),
            };
        }
    }
    let Some(expected) = &row.file_crc else {
        return Ok(false);
    };
    match crc_of(target.to_owned()).await {
        Ok(crc) => Ok(crc_text(crc) == *expected),
        Err(err) => Err(format!("cannot read {}: {err}", target.display())),
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

    /// The revision's episode name is the first release's, whatever
    /// `trname` would read from the revision marker.
    #[test]
    fn a_revisions_episode_name_is_its_first_releases() {
        let folder = Path::new("/media/Show/Season 01");
        let v1 = "[Erai-raws] Show - 06 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv";
        let v2 = "[Erai-raws] Show - 06v2 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv";
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
