//! Moving one work folder between the collect folder and the archive folder
//! (`docs/specs/collection.md`, 보관과 복원의 폴더 이동).
//!
//! Where a work folder is, is never stored: every run looks at the disk and at
//! Transmission and does what is left. That is what lets a move cut short (the
//! worker was stopped or died, or Transmission was slow) finish on the next
//! run, and what makes a folder a person moved by hand count as moved.
//!
//! A move is, in this order:
//!
//! 1. **Checks, before anything changes** ([`check`]). The two folders exist,
//!    are different, neither is inside the other, and they are on one
//!    filesystem, and so are the work folder at the destination and every
//!    directory there that the source merges into. The work folder is a direct
//!    child of the source folder ([`is_work_folder_name`]) and is not a link.
//!    Nothing inside it is a link leading out of the two folders or sits on
//!    another filesystem. No relative path is on both sides unless it is a
//!    real directory on both: one such file refuses the whole move, with the
//!    list. And the filesystem renames without replacing: an empty probe file
//!    is renamed from one folder to the other with `RENAME_NOREPLACE` and
//!    removed ([`probe_renames`]). A refused move has moved nothing.
//! 2. **Transmission first.** The torrents whose data is in the work folder
//!    are found by their folders' text and by where the folders really are
//!    ([`plan_torrents`]); a torrent not finished (downloading, verifying, or
//!    a magnet without its metadata yet), one with a local error, one whose
//!    folder is written with `..` or whose two readings disagree, or one with
//!    a file the destination already has that is not provably its own data,
//!    refuses the whole move before any torrent moves. Each is
//!    then moved with `torrent-set-location` (files moved), and the move waits
//!    until Transmission reports the new folder for each ([`move_torrents`]).
//!    Transmission 4 moves after it has answered and shows a failed move only
//!    as a local error on the torrent, which ends the wait with its text. A
//!    wait that runs out leaves the command for the next start
//!    ([`MoveError::Later`]).
//! 3. **The rest is renamed** ([`move_entries`]). The work folder is renamed
//!    whole when the destination has none. Otherwise it is merged: an entry the
//!    destination lacks is renamed whole, and a directory both sides have is
//!    merged in turn, file by file. Every rename refuses to replace
//!    (`RENAME_NOREPLACE`), so a file that appeared at the destination since
//!    the checks is left where it is and reported, never overwritten. Source
//!    directories emptied by the merge are removed.
//!
//! Only renames within one filesystem move anything, so a moved file or folder
//! keeps its owner. The worker creates no directory of its own: what the
//! destination lacks arrives whole by rename, and what it has is merged into.
//! The only new directories are the ones Transmission makes for the torrents
//! it moves, which belong to Transmission's user, as its downloads do. (The
//! spec asks new folders to take the owner and group of the destination's
//! parent; that holds when Transmission's user owns the media, and the rest is
//! an open decision, see ticket 0011.)
//!
//! A rerun after a stop at any point repeats the three steps on what is left:
//! the checks see the part already moved at the destination and the rest at
//! the source (disjoint, so no conflict), Transmission's torrents that already
//! report the destination are not touched again, and the renames move what is
//! still at the source. Each step only ever moves things from the source to
//! the destination, so reruns converge on everything at the destination. The
//! blocking steps keep the worker's lock ([`Hold`]) until they return, and the
//! renames stop between two entries once shutdown is asked for.

use std::{
    any::Any,
    fs, io,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use tokio_util::sync::CancellationToken;
use transmission_rpc::TransClient;
use trss_core::{files::rename_noreplace, folders::has_parent_dir};

use crate::transmission::{self, Redactor, TorrentFile, TorrentPlace};

/// How the move waits for Transmission to report the new folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovePolicy {
    /// Wait between two looks.
    pub poll: Duration,
    /// How long one start of the command waits for Transmission to report
    /// every torrent at its new folder. Past it the command is left for the
    /// next look (see [`MoveError::Later`]). Within one filesystem Transmission
    /// renames too, which takes moments.
    pub timeout: Duration,
}

impl Default for MovePolicy {
    fn default() -> Self {
        MovePolicy {
            poll: Duration::from_secs(1),
            timeout: Duration::from_secs(300),
        }
    }
}

/// One of the two folders, named in sentences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Collect,
    Archive,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Collect => "수집 폴더",
            Side::Archive => "보관 폴더",
        }
    }
}

/// What to move: the work folder `name` from one folder to the other. The
/// folders are written as the settings hold them, which is also how
/// Transmission names its folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub from_root: PathBuf,
    pub from: Side,
    pub to_root: PathBuf,
    pub to: Side,
    pub name: String,
}

impl Request {
    fn source(&self) -> PathBuf {
        self.from_root.join(&self.name)
    }

    fn destination(&self) -> PathBuf {
        self.to_root.join(&self.name)
    }
}

/// How a move ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved {
    /// Something moved on this run: files, torrents or both.
    Moved,
    /// The work folder was at the destination only; nothing had to move.
    AlreadyThere,
    /// Neither folder has the work folder and no torrent is in it.
    Nowhere,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveError {
    /// Not moved (or not all of it), for the reason given as a sentence.
    Failed(String),
    /// Not finished yet, for the reason given as a sentence: Transmission has
    /// not reported every torrent at its new folder in time. What moved stays;
    /// the next start carries on.
    Later(String),
    /// Shutdown was asked for. What moved so far stays; the next start
    /// carries on.
    Stopped,
}

/// Something kept alive until the move's blocking work has returned, even
/// when the task that waits for it is aborted: the worker passes its lock,
/// so no cycle or other worker starts while files are still being renamed.
pub type Hold = Arc<dyn Any + Send + Sync>;

/// The filesystem as the move sees it. The worker uses the metadata's
/// `st_dev` and `renameat2` ([`RealDisk`]); tests stand in another filesystem.
pub trait Disk: Send + Sync {
    /// Which filesystem `path` is on.
    fn device(&self, path: &Path, metadata: &fs::Metadata) -> u64;

    /// Renames `from` to `to` unless `to` exists (see [`rename_noreplace`]).
    fn rename_noreplace(&self, from: &Path, to: &Path) -> io::Result<()> {
        rename_noreplace(from, to)
    }
}

/// The filesystems as the operating system reports them.
pub struct RealDisk;

impl Disk for RealDisk {
    fn device(&self, _path: &Path, metadata: &fs::Metadata) -> u64 {
        use std::os::unix::fs::MetadataExt;
        metadata.dev()
    }
}

/// Whether `name` can be a work folder: exactly one ordinary path component.
pub fn is_work_folder_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(one)), None) if one == name
    )
}

/// Moves the work folder of `request`. See the module docs for the steps.
/// `hold` stays alive until every blocking step has returned.
pub async fn move_work_folder(
    transmission: &mut TransClient,
    redactor: &Redactor,
    request: &Request,
    policy: MovePolicy,
    disk: Arc<dyn Disk>,
    hold: Hold,
    cancel: &CancellationToken,
) -> Result<Moved, MoveError> {
    let checked = {
        let (request, disk) = (request.clone(), disk.clone());
        blocking(&hold, move || {
            check(&request, disk.as_ref()).map_err(MoveError::Failed)
        })
        .await?
    };

    let places = transmission::torrent_places(transmission, None, true)
        .await
        .map_err(|err| {
            eprintln!("{}", redactor.apply(&err.to_string()));
            MoveError::Failed(
                "Transmission에 연결하지 못해서 옮기지 않았어요. 잠시 뒤 다시 옮겨 주세요."
                    .to_owned(),
            )
        })?
        .into_iter()
        .map(|mut place| {
            place.local_error = place.local_error.map(|e| redactor.apply(&e));
            place
        })
        .collect::<Vec<_>>();
    let moves = {
        let request = request.clone();
        blocking(&hold, move || {
            plan_torrents(&request, &places).map_err(MoveError::Failed)
        })
        .await?
    };

    let torrents = move_torrents(transmission, redactor, &moves, policy, cancel).await?;

    let renamed = {
        let (source, destination) = (request.source(), request.destination());
        let (to, cancel) = (request.to, cancel.clone());
        blocking(&hold, move || {
            move_entries(&source, &destination, to, disk.as_ref(), &cancel)
        })
        .await?
    };

    Ok(if renamed || torrents > 0 {
        Moved::Moved
    } else if checked.destination_exists {
        Moved::AlreadyThere
    } else {
        Moved::Nowhere
    })
}

/// Runs `work` on the blocking pool with `hold` inside it, so that `hold`
/// lives until `work` returns even if the caller stops waiting.
pub async fn blocking<T: Send + 'static>(
    hold: &Hold,
    work: impl FnOnce() -> Result<T, MoveError> + Send + 'static,
) -> Result<T, MoveError> {
    let hold = hold.clone();
    let task = tokio::task::spawn_blocking(move || {
        let _hold = hold;
        work()
    });
    match task.await {
        Ok(result) => result,
        Err(_) => Err(MoveError::Failed(
            "폴더를 옮기다 내부 오류가 났어요.".to_owned(),
        )),
    }
}

// ---------------------------------------------------------------------------
// 1. Checks
// ---------------------------------------------------------------------------

/// What the checks found.
#[derive(Debug)]
pub struct Checked {
    pub destination_exists: bool,
}

/// How many conflicting files a refusal names.
const LISTED_CONFLICTS: usize = 5;

fn quoted(path: &Path) -> String {
    format!("`{}`", path.display())
}

/// `paths` quoted, the first [`LISTED_CONFLICTS`] of them, and how many more.
fn listed(paths: &[PathBuf]) -> String {
    let names: Vec<String> = paths
        .iter()
        .take(LISTED_CONFLICTS)
        .map(|p| quoted(p))
        .collect();
    let more = if paths.len() > LISTED_CONFLICTS {
        format!(" 외 {}개", paths.len() - LISTED_CONFLICTS)
    } else {
        String::new()
    };
    format!("{}{more}", names.join(", "))
}

/// How a refusal asks to try the move again toward `to`: the button the
/// rule shows for it.
fn again(to: Side) -> &'static str {
    match to {
        Side::Archive => "`다시 옮기기`를 눌러",
        Side::Collect => "다시 `복원`해",
    }
}

/// What a refusal over same-name files asks for. Two files with one name
/// may hold different data, so it never calls either side safe to clear.
fn compare_then_again(to: Side) -> String {
    format!(
        "이름이 같아도 내용이 다를 수 있으니, 겹치는 파일마다 두 쪽을 견줘 남길 것을 정하거나 한쪽 이름을 바꾼 뒤 {} 주세요.",
        again(to)
    )
}

/// The checks of step 1. They change nothing on disk except the rename probe
/// ([`probe_renames`]), which leaves nothing behind.
pub fn check(request: &Request, disk: &dyn Disk) -> Result<Checked, String> {
    if !is_work_folder_name(&request.name) {
        return Err(format!(
            "`{}`는 작품 폴더 이름이 아니라서 옮기지 않았어요.",
            request.name
        ));
    }
    let (from_real, from_dev) = root(&request.from_root, request.from, disk)?;
    let (to_real, to_dev) = root(&request.to_root, request.to, disk)?;
    if from_real.starts_with(&to_real) || to_real.starts_with(&from_real) {
        return Err(
            "수집 폴더와 보관 폴더가 같거나 한쪽이 다른 쪽 안에 있어서 옮기지 않았어요. 설정에서 두 폴더를 확인해 주세요."
                .to_owned(),
        );
    }
    if from_dev != to_dev {
        return Err(different_filesystems());
    }

    let source = request.source();
    let destination = request.destination();
    let source_meta = entry(&source)?;
    let destination_meta = entry(&destination)?;
    for (path, meta, side) in [
        (&source, &source_meta, request.from),
        (&destination, &destination_meta, request.to),
    ] {
        if meta.as_ref().is_some_and(|m| m.file_type().is_symlink()) {
            return Err(format!(
                "{}의 작품 폴더 {}가 링크라서 옮기지 않았어요. 링크가 아닌 폴더만 옮겨요.",
                side.name(),
                quoted(path)
            ));
        }
    }
    // A work folder at the destination that is a mount of its own would take
    // no rename from the source.
    if let Some(meta) = &destination_meta {
        if disk.device(&destination, meta) != from_dev {
            return Err(different_filesystems());
        }
    }
    let Some(source_meta) = source_meta else {
        // Nothing for the worker to rename; Transmission's torrents are
        // checked on their own (plan_torrents).
        return Ok(Checked {
            destination_exists: destination_meta.is_some(),
        });
    };
    if !source_meta.is_dir() {
        return Err(format!(
            "{}가 폴더가 아니라서 옮기지 않았어요.",
            quoted(&source)
        ));
    }
    if disk.device(&source, &source_meta) != from_dev {
        return Err(different_filesystems());
    }

    let mut walk = Walk {
        disk,
        device: from_dev,
        roots: [from_real, to_real],
        conflicts: Vec::new(),
    };
    let merge_into = match &destination_meta {
        Some(meta) if meta.is_dir() => Some(destination.as_path()),
        Some(_) => {
            // The work folder's own name is a file at the destination.
            walk.conflicts.push(PathBuf::from(&request.name));
            None
        }
        None => None,
    };
    walk.visit(&source, merge_into, Path::new(""))?;

    if !walk.conflicts.is_empty() {
        return Err(format!(
            "{}의 `{}`에 같은 이름의 파일이 있어서 아무것도 옮기지 않았어요: {}. {}",
            request.to.name(),
            request.name,
            listed(&walk.conflicts),
            compare_then_again(request.to)
        ));
    }

    probe_renames(request, disk)?;
    Ok(Checked {
        destination_exists: destination_meta.is_some(),
    })
}

fn different_filesystems() -> String {
    "수집 폴더와 보관 폴더가 서로 다른 파일시스템에 있어서 옮기지 않았어요. 복사하지 않고 이름만 바꿔 옮기므로, 같은 파일시스템 안의 폴더여야 해요."
        .to_owned()
}

/// Renames an empty file of its own from the source folder to the destination
/// folder the way the renames will ([`Disk::rename_noreplace`]), and removes
/// it. A filesystem that cannot rename without replacing, or that refuses the
/// rename, refuses the move here, before anything has moved.
fn probe_renames(request: &Request, disk: &dyn Disk) -> Result<(), String> {
    let name = format!(
        ".trss-move-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    );
    let (here, there) = (request.from_root.join(&name), request.to_root.join(&name));
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&here)
        .map_err(|e| {
            format!(
                "{} {}에 쓸 수 없어서 옮기지 않았어요: {e}",
                request.from.name(),
                quoted(&request.from_root)
            )
        })?;
    let renamed = disk.rename_noreplace(&here, &there);
    let probe = if renamed.is_ok() { &there } else { &here };
    if let Err(e) = fs::remove_file(probe) {
        eprintln!("Could not remove the rename probe {}: {e}", probe.display());
    }
    match renamed {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => Err(different_filesystems()),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
            ) =>
        {
            Err(
                "이 파일시스템은 덮어쓰지 않는 이름 바꾸기(RENAME_NOREPLACE)를 지원하지 않아서 옮기지 않았어요. 지원하는 파일시스템의 폴더여야 해요."
                    .to_owned(),
            )
        }
        Err(e) => Err(format!(
            "{}에서 {}로 이름을 바꿔 옮길 수 없어서 옮기지 않았어요: {e}",
            request.from.name(),
            request.to.name()
        )),
    }
}

/// A root folder with links resolved, and its filesystem.
fn root(path: &Path, side: Side, disk: &dyn Disk) -> Result<(PathBuf, u64), String> {
    let missing = || {
        format!(
            "{} {}를 찾지 못해서 옮기지 않았어요.",
            side.name(),
            quoted(path)
        )
    };
    let meta = fs::metadata(path).map_err(|_| missing())?;
    if !meta.is_dir() {
        return Err(missing());
    }
    let real = fs::canonicalize(path).map_err(|_| missing())?;
    Ok((real, disk.device(path, &meta)))
}

/// The entry at `path` itself (a link is not followed), or `None` when there
/// is none.
fn entry(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!(
            "{}를 읽지 못해서 옮기지 않았어요: {e}",
            quoted(path)
        )),
    }
}

struct Walk<'a> {
    disk: &'a dyn Disk,
    /// The source folder's filesystem.
    device: u64,
    /// Both folders with links resolved: where a link may lead.
    roots: [PathBuf; 2],
    /// Relative paths on both sides that are not a directory on both.
    conflicts: Vec<PathBuf>,
}

impl Walk<'_> {
    /// Checks the directory `dir` at the source (relative path `rel`) against
    /// `other`, the same directory at the destination when there is one.
    fn visit(&mut self, dir: &Path, other: Option<&Path>, rel: &Path) -> Result<(), String> {
        let read = |e: io::Error| format!("{}를 읽지 못해서 옮기지 않았어요: {e}", quoted(dir));
        let mut names: Vec<_> = fs::read_dir(dir)
            .map_err(read)?
            .map(|e| e.map(|e| e.file_name()))
            .collect::<Result<_, _>>()
            .map_err(read)?;
        names.sort();
        for name in names {
            let path = dir.join(&name);
            let rel = rel.join(&name);
            let meta = fs::symlink_metadata(&path)
                .map_err(|e| format!("{}를 읽지 못해서 옮기지 않았어요: {e}", quoted(&path)))?;
            if self.disk.device(&path, &meta) != self.device {
                return Err(different_filesystems());
            }
            if meta.file_type().is_symlink() {
                let inside = fs::canonicalize(&path)
                    .is_ok_and(|target| self.roots.iter().any(|root| target.starts_with(root)));
                if !inside {
                    return Err(format!(
                        "{}가 수집 폴더와 보관 폴더 밖을 가리키는 링크라서 옮기지 않았어요.",
                        quoted(&path)
                    ));
                }
            }

            let counterpart = match other {
                Some(other) => entry(&other.join(&name))?.map(|m| (other.join(&name), m)),
                None => None,
            };
            let is_dir = meta.is_dir();
            match counterpart {
                Some((there, there_meta)) if is_dir && there_meta.is_dir() => {
                    // Merged into: it must take renames from the source.
                    if self.disk.device(&there, &there_meta) != self.device {
                        return Err(different_filesystems());
                    }
                    self.visit(&path, Some(&there), &rel)?;
                }
                Some(_) => self.conflicts.push(rel),
                None if is_dir => self.visit(&path, None, &rel)?,
                None => {}
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 2. Transmission
// ---------------------------------------------------------------------------

/// One torrent to move: from its folder as Transmission reports it to `to`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentMove {
    pub hash: String,
    pub name: String,
    pub from: PathBuf,
    pub to: PathBuf,
}

/// `base` with `rest` joined on, without the trailing slash an empty `rest`
/// would add.
fn joined(base: &Path, rest: &Path) -> PathBuf {
    if rest.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        base.join(rest)
    }
}

/// `path` with links resolved as far as it exists; the missing rest is joined
/// on as written. For paths without `..`.
fn resolve(path: &Path) -> PathBuf {
    let parts: Vec<Component> = path.components().collect();
    for keep in (1..=parts.len()).rev() {
        let head: PathBuf = parts[..keep].iter().collect();
        if let Ok(mut real) = fs::canonicalize(&head) {
            for part in &parts[keep..] {
                real.push(part);
            }
            return real;
        }
    }
    path.to_path_buf()
}

/// Where a torrent in `folder` (where it really is) named `name` goes when
/// `request` moves, or `None` when its data is not in the work folder: its
/// folder is the work folder `work` or inside it, or it sits right in the
/// source folder `root` under the work folder's name. `work` and `root` are
/// the work folder and the source folder in the same form as `folder`.
pub fn new_location(
    folder: &Path,
    name: &str,
    work: &Path,
    root: &Path,
    request: &Request,
) -> Option<PathBuf> {
    if let Ok(rest) = folder.strip_prefix(work) {
        return Some(joined(&request.destination(), rest));
    }
    let named_like_it = Path::new(name)
        .components()
        .next()
        .is_some_and(|first| first.as_os_str() == request.name.as_str());
    (folder == root && named_like_it).then(|| request.to_root.clone())
}

/// Which of the torrents `places` are in the work folder of `request`, and
/// whether all of them can move now. A torrent's folder is judged both by its
/// text and by where it really is (links followed, as Transmission's own file
/// access follows them). Refuses the whole move, before any torrent moves,
/// when one of them:
///
/// - has a folder written with `..` that lands in the work folder (by text or
///   by the disk): Transmission's and the worker's readings of it could part;
/// - has a folder whose text and real place disagree on being in the work
///   folder (a link into it from outside, or out of it from inside): one
///   reading would leave its data behind or take another work's along;
/// - is not finished ([`TorrentPlace::unfinished`]): Transmission would keep
///   writing it, possibly from its incomplete folder, and finishes a file by
///   renaming it over whatever has its name;
/// - reports a local error;
/// - has a file (or its `.part` name) at the new folder, unless the torrent's
///   data is all there already (see [`own_data_at_destination`]):
///   Transmission's move replaces what it meets, and a torrent pointed at a
///   file not its own later writes its pieces into it.
pub fn plan_torrents(
    request: &Request,
    places: &[TorrentPlace],
) -> Result<Vec<TorrentMove>, String> {
    let (source, from_root) = (request.source(), request.from_root.clone());
    let (work_real, root_real) = (resolve(&source), resolve(&from_root));
    let (work_text, root_text) = (super::lexical(&source), super::lexical(&from_root));

    let mut moves = Vec::new();
    for place in places {
        let folder = Path::new(&place.download_dir);
        let by_text = new_location(
            &super::lexical(folder),
            &place.name,
            &work_text,
            &root_text,
            request,
        );
        if has_parent_dir(folder) {
            let by_disk = fs::canonicalize(folder)
                .ok()
                .and_then(|real| new_location(&real, &place.name, &work_real, &root_real, request));
            if by_text.is_some() || by_disk.is_some() {
                return Err(format!(
                    "Transmission 토렌트 `{}`의 받는 폴더 {}에 `..`가 있어서 아무것도 옮기지 않았어요. Transmission에서 그 토렌트의 위치를 `..` 없이 바로잡은 뒤 다시 옮겨 주세요.",
                    place.name,
                    quoted(folder)
                ));
            }
            continue;
        }
        let by_disk = new_location(
            &resolve(folder),
            &place.name,
            &work_real,
            &root_real,
            request,
        );
        match (by_text, by_disk) {
            (None, None) => {}
            (Some(_), Some(to)) => moves.push((place, to)),
            _ => {
                return Err(format!(
                    "Transmission 토렌트 `{}`의 받는 폴더 {}가 링크를 거쳐 작품 폴더 안팎을 오가서 아무것도 옮기지 않았어요. Transmission에서 그 토렌트의 위치를 링크 없는 실제 폴더로 바로잡은 뒤 다시 옮겨 주세요.",
                    place.name,
                    quoted(folder)
                ));
            }
        }
    }

    let unfinished: Vec<PathBuf> = moves
        .iter()
        .filter(|(place, _)| place.unfinished)
        .map(|(place, _)| PathBuf::from(&place.name))
        .collect();
    if !unfinished.is_empty() {
        return Err(format!(
            "Transmission에 아직 다 받지 않은 토렌트가 있어서 아무것도 옮기지 않았어요: {}. Transmission에서 그 토렌트를 다 받거나, 더 받지 않을 거면 지운 뒤 {} 주세요.",
            listed(&unfinished),
            again(request.to)
        ));
    }
    if let Some((place, error)) = moves
        .iter()
        .find_map(|(place, _)| Some((place, place.local_error.as_ref()?)))
    {
        return Err(format!(
            "Transmission이 토렌트 `{}`에 오류를 알리고 있어서 아무것도 옮기지 않았어요: {error}. Transmission에서 그 토렌트를 확인한 뒤 다시 옮겨 주세요.",
            place.name
        ));
    }

    let destination = request.destination();
    let mut conflicts = Vec::new();
    let mut split = Vec::new();
    for (place, to) in &moves {
        let from = Path::new(&place.download_dir);
        let at_destination: Vec<(&TorrentFile, PathBuf)> = place
            .files
            .iter()
            .filter_map(|file| Some((file, present(to, &file.name)?)))
            .collect();
        if at_destination.is_empty() || own_data_at_destination(place, from, &at_destination) {
            continue;
        }
        if split_between(place, from, &at_destination) {
            split.push(PathBuf::from(&place.name));
            continue;
        }
        conflicts.extend(at_destination.into_iter().map(|(_, there)| {
            there
                .strip_prefix(&destination)
                .map(Path::to_path_buf)
                .unwrap_or(there)
        }));
    }
    if !conflicts.is_empty() || !split.is_empty() {
        let mut reasons = Vec::new();
        if !conflicts.is_empty() {
            reasons.push(format!(
                "{}의 `{}`에 Transmission 토렌트의 파일과 같은 이름의 파일이 있어요: {}. 이름이 같아도 내용이 다를 수 있으니, 겹치는 파일마다 두 쪽을 견줘 남길 것을 정하거나 한쪽 이름을 바꿔 주세요.",
                request.to.name(),
                request.name,
                listed(&conflicts)
            ));
        }
        if !split.is_empty() {
            // Each side holds the only copy of different files: clearing
            // either one loses data.
            reasons.push(format!(
                "Transmission 토렌트 {}의 파일이 {}와 {}에 나뉘어 있어요. 두 쪽에 서로 다른 파일이 있으니 어느 쪽도 지우지 말고, 남은 파일을 한쪽으로 모으거나 Transmission에서 그 토렌트의 위치를 한쪽으로 바꿔 주세요.",
                listed(&split),
                request.from.name(),
                request.to.name()
            ));
        }
        return Err(format!(
            "아무것도 옮기지 않았어요. {} 그런 뒤 {} 주세요.",
            reasons.join(" "),
            again(request.to)
        ));
    }

    Ok(moves
        .into_iter()
        .map(|(place, to)| TorrentMove {
            hash: place.hash.clone(),
            name: place.name.clone(),
            from: PathBuf::from(&place.download_dir),
            to,
        })
        .collect())
}

/// The file `name` in `dir`, or its `.part` name, when either is there.
fn present(dir: &Path, name: &str) -> Option<PathBuf> {
    [name.to_owned(), format!("{name}.part")]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|path| fs::symlink_metadata(path).is_ok())
}

/// Whether the files of `place` lie partly in its folder `from` and partly
/// at the destination (`at_destination`), with no name on both sides and
/// each one at the destination looking like the torrent's own
/// ([`own_file`]), as a move of the torrent's data stopped halfway leaves
/// them. Each side then holds the only copy of its files.
fn split_between(
    place: &TorrentPlace,
    from: &Path,
    at_destination: &[(&TorrentFile, PathBuf)],
) -> bool {
    let at_source: Vec<&TorrentFile> = place
        .files
        .iter()
        .filter(|file| present(from, &file.name).is_some())
        .collect();
    !at_source.is_empty()
        && at_source.iter().all(|file| {
            at_destination
                .iter()
                .all(|(there, _)| there.name != file.name)
        })
        && at_destination
            .iter()
            .all(|(file, there)| own_file(file, there))
}

/// Whether `there` looks like the torrent's own `file`: a plain file under
/// its own name, complete, with the torrent's size for it.
fn own_file(file: &TorrentFile, there: &Path) -> bool {
    file.complete
        && there.file_name() == Path::new(&file.name).file_name()
        && fs::symlink_metadata(there).is_ok_and(|meta| {
            meta.is_file() && i64::try_from(meta.len()).is_ok_and(|len| len == file.length)
        })
}

/// Whether the files of `place` found at the destination (`at_destination`)
/// are the torrent's own data, moved there by an earlier start or by hand, so
/// that pointing the torrent there overwrites nothing. Only when none of its
/// files is left in its folder `from` (by name or `.part` name) and each one
/// at the destination looks like the torrent's own ([`own_file`]). Anything
/// else may be another file that only shares the name.
fn own_data_at_destination(
    place: &TorrentPlace,
    from: &Path,
    at_destination: &[(&TorrentFile, PathBuf)],
) -> bool {
    let none_left = place
        .files
        .iter()
        .all(|file| present(from, &file.name).is_none());
    none_left
        && at_destination
            .iter()
            .all(|(file, there)| own_file(file, there))
}

/// Step 2: asks Transmission to move `moves` and waits until it reports each
/// at its new folder. Transmission answers before it moves the files and
/// shows a failed move only as a local error on the torrent, so the wait
/// watches for that too. Returns how many it asked to move.
async fn move_torrents(
    transmission: &mut TransClient,
    redactor: &Redactor,
    moves: &[TorrentMove],
    policy: MovePolicy,
    cancel: &CancellationToken,
) -> Result<usize, MoveError> {
    if moves.is_empty() {
        return Ok(0);
    }

    for (asked, m) in moves.iter().enumerate() {
        if let Err(err) = transmission::set_location(transmission, &m.hash, &m.to).await {
            let err = redactor.apply(&err);
            eprintln!("torrent-set-location {}: {err}", m.hash);
            let after = if asked == 0 {
                "아무것도 옮기지 않았어요. 잠시 뒤 다시 옮겨 주세요.".to_owned()
            } else {
                format!(
                    "먼저 맡긴 토렌트 {asked}개는 Transmission이 새 위치로 옮기고 있을 수 있어요. 다시 옮기면 남은 것만 옮겨요."
                )
            };
            return Err(MoveError::Failed(format!(
                "Transmission이 토렌트 `{}`의 위치 옮기기를 거절했어요({err}). {after}",
                m.name
            )));
        }
        println!("Moving torrent {} to {}", m.hash, m.to.display());
    }

    let hashes: Vec<String> = moves.iter().map(|m| m.hash.clone()).collect();
    let started = Instant::now();
    loop {
        // A torrent removed meanwhile has nothing left to wait for.
        let waiting = match transmission::torrent_places(transmission, Some(&hashes), false).await {
            Ok(places) => {
                let mut waiting = 0;
                for m in moves {
                    let Some(place) = places.iter().find(|p| p.hash == m.hash) else {
                        continue;
                    };
                    if Path::new(&place.download_dir) == m.to {
                        continue;
                    }
                    if let Some(error) = &place.local_error {
                        return Err(MoveError::Failed(format!(
                            "Transmission이 토렌트 `{}`를 옮기다 오류를 알렸어요: {}. 옮겨진 파일은 그대로 두었어요. Transmission에서 그 토렌트를 확인한 뒤 다시 옮기면 남은 것만 옮겨요.",
                            m.name,
                            redactor.apply(error)
                        )));
                    }
                    waiting += 1;
                }
                waiting
            }
            Err(err) => {
                eprintln!("{}", redactor.apply(&err.to_string()));
                moves.len()
            }
        };
        if waiting == 0 {
            return Ok(moves.len());
        }
        if started.elapsed() >= policy.timeout {
            return Err(MoveError::Later(format!(
                "Transmission이 {}초 안에 토렌트 {waiting}개를 새 위치로 옮겼다고 알리지 않았어요. 옮긴 것은 그대로 두었고, 다시 옮기면 남은 것만 옮겨요.",
                policy.timeout.as_secs()
            )));
        }
        tokio::select! {
            _ = tokio::time::sleep(policy.poll) => {}
            _ = cancel.cancelled() => return Err(MoveError::Stopped),
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Renames
// ---------------------------------------------------------------------------

/// Step 3: moves what is left at `source` to `destination`. Returns whether
/// anything was renamed. Stops between two entries when `cancel` is set.
pub fn move_entries(
    source: &Path,
    destination: &Path,
    to: Side,
    disk: &dyn Disk,
    cancel: &CancellationToken,
) -> Result<bool, MoveError> {
    match fs::symlink_metadata(source) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => {
            return Err(MoveError::Failed(format!(
                "{}를 읽지 못했어요: {e}",
                quoted(source)
            )))
        }
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            return Err(MoveError::Failed(format!(
                "{}가 링크이거나 폴더가 아니라서 옮기지 않았어요.",
                quoted(source)
            )));
        }
        Ok(_) => {}
    }
    let mut merge = Merge {
        disk,
        cancel,
        renamed: false,
        left: Vec::new(),
    };
    merge.entry(source, destination, Path::new(""))?;
    if merge.left.is_empty() {
        return Ok(merge.renamed);
    }
    Err(MoveError::Failed(format!(
        "옮기는 사이 {}에 같은 이름의 파일이 생겨서 {}는 옮기지 않았어요. {}",
        to.name(),
        listed(&merge.left),
        compare_then_again(to)
    )))
}

struct Merge<'a> {
    disk: &'a dyn Disk,
    cancel: &'a CancellationToken,
    renamed: bool,
    /// Relative paths left at the source because the destination had them.
    left: Vec<PathBuf>,
}

impl Merge<'_> {
    fn entry(&mut self, from: &Path, to: &Path, rel: &Path) -> Result<(), MoveError> {
        if self.cancel.is_cancelled() {
            return Err(MoveError::Stopped);
        }
        match self.disk.rename_noreplace(from, to) {
            Ok(()) => {
                self.renamed = true;
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                return Err(MoveError::Failed(format!(
                    "{}와 {}가 서로 다른 파일시스템에 있어서 더 옮기지 않았어요. 옮긴 것은 그대로 두었어요. 두 폴더를 같은 파일시스템에 두면 다시 옮길 때 남은 것만 옮겨요.",
                    quoted(from),
                    quoted(to)
                )))
            }
            Err(e) => {
                return Err(MoveError::Failed(format!(
                    "{}를 {}로 옮기지 못했어요: {e}. 옮긴 것은 그대로 두었고, 다시 옮기면 남은 것만 옮겨요.",
                    quoted(from),
                    quoted(to)
                )))
            }
        }
        let both_dirs = [from, to].iter().all(|path| {
            fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        });
        if !both_dirs {
            self.left.push(rel.to_path_buf());
            return Ok(());
        }
        let read = |e: io::Error| {
            MoveError::Failed(format!(
                "{}를 읽지 못해서 더 옮기지 않았어요: {e}. 옮긴 것은 그대로 두었고, 다시 옮기면 남은 것만 옮겨요.",
                quoted(from)
            ))
        };
        let mut names: Vec<_> = fs::read_dir(from)
            .map_err(read)?
            .map(|e| e.map(|e| e.file_name()))
            .collect::<Result<_, _>>()
            .map_err(read)?;
        names.sort();
        for name in names {
            self.entry(&from.join(&name), &to.join(&name), &rel.join(&name))?;
        }
        // Emptied by the merge; one that still holds something stays.
        let _ = fs::remove_dir(from);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
