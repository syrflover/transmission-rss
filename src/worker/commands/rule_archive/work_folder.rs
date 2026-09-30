//! Moving one work folder between the collect folder and the archive folder
//! (`docs/specs/collection.md`, 보관과 복원의 폴더 이동).
//!
//! Where a work folder is, is never stored: every run looks at the disk and at
//! Transmission and does what is left. That is what lets a move cut short (the
//! worker was stopped or died) finish on the next run, and what makes a folder
//! a person moved by hand count as moved.
//!
//! A move is, in this order:
//!
//! 1. **Checks, before anything changes** ([`check`]). The two folders exist,
//!    are different, neither is inside the other, and they are on one
//!    filesystem. The work folder is a direct child of the source folder
//!    ([`is_work_folder_name`]) and is not a link. Nothing inside it is a link
//!    leading out of the two folders or sits on another filesystem. And no
//!    relative path is on both sides unless it is a real directory on both:
//!    one such file refuses the whole move, with the list. A refused move has
//!    touched nothing.
//! 2. **Transmission first** ([`move_torrents`]). Every torrent whose data is
//!    inside the work folder is moved with `torrent-set-location` (files
//!    moved), and the move waits until Transmission reports the new folder for
//!    each. Transmission keeps seeing its files where they are, and the folders
//!    it creates for them are its own user's.
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
//! it moves, which belong to Transmission's user, as its downloads do. So a
//! worker running as root never leaves a root-owned folder that Transmission
//! could not write into after a restore.
//!
//! A rerun after a stop at any point repeats the three steps on what is left:
//! the checks see the part already moved at the destination and the rest at
//! the source (disjoint, so no conflict), Transmission's torrents that already
//! report the destination are not touched again, and the renames move what is
//! still at the source. Each step only ever moves things from the source to
//! the destination, so reruns converge on everything at the destination.

use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use tokio_util::sync::CancellationToken;
use transmission_rpc::TransClient;

use crate::transmission::{self, Redactor, TorrentPlace};

/// How the move waits for Transmission to report the new folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovePolicy {
    /// Wait between two looks.
    pub poll: Duration,
    /// How long Transmission may take for all the torrents of one move. Within
    /// one filesystem Transmission renames too, which takes moments.
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
    /// Shutdown was asked for while waiting for Transmission. What moved so
    /// far stays; the next run carries on.
    Stopped,
}

/// Tells which filesystem a path is on. The worker uses the metadata's
/// `st_dev` ([`RealDisk`]); tests stand in another filesystem.
pub trait Disk: Send + Sync {
    fn device(&self, path: &Path, metadata: &fs::Metadata) -> u64;
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
pub async fn move_work_folder(
    transmission: &mut TransClient,
    redactor: &Redactor,
    request: &Request,
    policy: MovePolicy,
    disk: Arc<dyn Disk>,
    cancel: &CancellationToken,
) -> Result<Moved, MoveError> {
    let checked = {
        let request = request.clone();
        blocking(move || check(&request, disk.as_ref())).await?
    };

    let torrents = move_torrents(transmission, redactor, request, policy, cancel).await?;

    let renamed = {
        let (source, destination) = (request.source(), request.destination());
        let to = request.to;
        blocking(move || move_entries(&source, &destination, to)).await?
    };

    Ok(if renamed || torrents > 0 {
        Moved::Moved
    } else if checked.destination_exists {
        Moved::AlreadyThere
    } else {
        Moved::Nowhere
    })
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, MoveError> {
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result.map_err(MoveError::Failed),
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

/// The checks of step 1. Reads only.
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
    let Some(source_meta) = source_meta else {
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
        let total = walk.conflicts.len();
        let listed: Vec<String> = walk
            .conflicts
            .iter()
            .take(LISTED_CONFLICTS)
            .map(|p| quoted(p))
            .collect();
        let more = if total > LISTED_CONFLICTS {
            format!(" 외 {}개", total - LISTED_CONFLICTS)
        } else {
            String::new()
        };
        return Err(format!(
            "{}의 `{}`에 같은 이름의 파일이 있어서 아무것도 옮기지 않았어요: {}{more}. 한쪽을 정리한 뒤 다시 옮겨 주세요.",
            request.to.name(),
            request.name,
            listed.join(", ")
        ));
    }
    Ok(Checked {
        destination_exists: destination_meta.is_some(),
    })
}

fn different_filesystems() -> String {
    "수집 폴더와 보관 폴더가 서로 다른 파일시스템에 있어서 옮기지 않았어요. 복사하지 않고 이름만 바꿔 옮기므로, 같은 파일시스템 안의 폴더여야 해요."
        .to_owned()
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

/// `base` with `rest` joined on, without the trailing slash an empty `rest`
/// would add.
fn joined(base: &Path, rest: &Path) -> PathBuf {
    if rest.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        base.join(rest)
    }
}

/// Where the torrent at `place` goes when `request` moves, or `None` when its
/// data is not in the work folder: its folder is the work folder or inside it,
/// or it sits right in the source folder under the work folder's name.
pub fn new_location(place: &TorrentPlace, request: &Request) -> Option<PathBuf> {
    let folder = Path::new(&place.download_dir);
    if let Ok(rest) = folder.strip_prefix(request.source()) {
        return Some(joined(&request.destination(), rest));
    }
    let named_like_it = Path::new(&place.name)
        .components()
        .next()
        .is_some_and(|first| first.as_os_str() == request.name.as_str());
    (folder == request.from_root && named_like_it).then(|| request.to_root.clone())
}

/// Step 2: moves the torrents in the work folder and waits until Transmission
/// reports them at the destination. Returns how many it asked to move.
async fn move_torrents(
    transmission: &mut TransClient,
    redactor: &Redactor,
    request: &Request,
    policy: MovePolicy,
    cancel: &CancellationToken,
) -> Result<usize, MoveError> {
    let places = transmission::torrent_places(transmission, None)
        .await
        .map_err(|err| {
            eprintln!("{}", redactor.apply(&err.to_string()));
            MoveError::Failed(
                "Transmission에 연결하지 못해서 옮기지 않았어요. 잠시 뒤 다시 옮겨 주세요."
                    .to_owned(),
            )
        })?;
    let moves: Vec<(String, PathBuf)> = places
        .iter()
        .filter_map(|place| Some((place.hash.clone(), new_location(place, request)?)))
        .collect();
    if moves.is_empty() {
        return Ok(0);
    }

    for (hash, location) in &moves {
        if let Err(err) = transmission::set_location(transmission, hash, location).await {
            let err = redactor.apply(&err);
            eprintln!("torrent-set-location {hash}: {err}");
            return Err(MoveError::Failed(format!(
                "Transmission이 토렌트의 위치를 옮기지 않았어요({err}). 잠시 뒤 다시 옮겨 주세요."
            )));
        }
        println!("Moving torrent {hash} to {}", location.display());
    }

    let hashes: Vec<String> = moves.iter().map(|(hash, _)| hash.clone()).collect();
    let started = Instant::now();
    loop {
        // A torrent removed meanwhile has nothing left to wait for.
        let waiting = match transmission::torrent_places(transmission, Some(&hashes)).await {
            Ok(places) => moves
                .iter()
                .filter(|(hash, location)| {
                    places
                        .iter()
                        .any(|p| &p.hash == hash && Path::new(&p.download_dir) != location)
                })
                .count(),
            Err(err) => {
                eprintln!("{}", redactor.apply(&err.to_string()));
                moves.len()
            }
        };
        if waiting == 0 {
            return Ok(moves.len());
        }
        if started.elapsed() >= policy.timeout {
            return Err(MoveError::Failed(format!(
                "Transmission이 토렌트 {waiting}개를 {}초 안에 옮기지 못했어요. 옮긴 것은 그대로 두었고, 다시 옮기면 남은 것만 옮겨요.",
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

/// Renames `from` to `to` unless `to` exists (an `AlreadyExists` error then).
/// Never replaces anything, a directory included.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use rustix::fs::{renameat_with, RenameFlags, CWD};
    match renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE) {
        Ok(()) => Ok(()),
        // A filesystem without the flag: check, then rename. The worker holds
        // the lock that keeps its own cycles out meanwhile.
        Err(rustix::io::Errno::INVAL) | Err(rustix::io::Errno::NOSYS) => {
            if fs::symlink_metadata(to).is_ok() {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            fs::rename(from, to)
        }
        Err(errno) => Err(errno.into()),
    }
}

/// Step 3: moves what is left at `source` to `destination`. Returns whether
/// anything was renamed.
pub fn move_entries(source: &Path, destination: &Path, to: Side) -> Result<bool, String> {
    match fs::symlink_metadata(source) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{}를 읽지 못했어요: {e}", quoted(source))),
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            return Err(format!(
                "{}가 링크이거나 폴더가 아니라서 옮기지 않았어요.",
                quoted(source)
            ));
        }
        Ok(_) => {}
    }
    let mut merge = Merge {
        renamed: false,
        left: Vec::new(),
    };
    merge.entry(source, destination, Path::new(""))?;
    if merge.left.is_empty() {
        return Ok(merge.renamed);
    }
    let total = merge.left.len();
    let listed: Vec<String> = merge
        .left
        .iter()
        .take(LISTED_CONFLICTS)
        .map(|p| quoted(p))
        .collect();
    let more = if total > LISTED_CONFLICTS {
        format!(" 외 {}개", total - LISTED_CONFLICTS)
    } else {
        String::new()
    };
    Err(format!(
        "옮기는 사이 {}에 같은 이름의 파일이 생겨서 {}{more}는 옮기지 않았어요. 한쪽을 정리한 뒤 다시 옮겨 주세요.",
        to.name(),
        listed.join(", ")
    ))
}

struct Merge {
    renamed: bool,
    /// Relative paths left at the source because the destination had them.
    left: Vec<PathBuf>,
}

impl Merge {
    fn entry(&mut self, from: &Path, to: &Path, rel: &Path) -> Result<(), String> {
        match rename_noreplace(from, to) {
            Ok(()) => {
                self.renamed = true;
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                return Err(format!(
                    "{}를 {}로 옮기지 못했어요: {e}. 옮긴 것은 그대로 두었고, 다시 옮기면 남은 것만 옮겨요.",
                    quoted(from),
                    quoted(to)
                ))
            }
        }
        let both_dirs = [from, to].iter().all(|path| {
            fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        });
        if !both_dirs {
            self.left.push(rel.to_path_buf());
            return Ok(());
        }
        let mut names: Vec<_> = fs::read_dir(from)
            .map_err(|e| format!("{}를 읽지 못했어요: {e}", quoted(from)))?
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .collect();
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
