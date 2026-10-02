//! Discovering works, seasons and episodes in a watch folder
//! (`docs/specs/library.md`, 작품 발견과 감시 폴더).
//!
//! The web (when a watch folder is added) and the worker (every cycle, and on
//! `다시 확인`) read a folder with the same scan, which only looks: it never
//! creates, moves, renames or deletes anything, and it does not touch the
//! database. What a scan found is handed to [`crate::store::library`], which
//! decides what is new and what has gone.
//!
//! # The layout
//!
//! trname writes `<watch folder>/<work>/Season NN/<work> SxxEyy.ext`:
//!
//! - a **work** is a folder directly under the watch folder;
//! - a **season** is a `Season NN` folder in a work (`NN` is an integer, 0 or
//!   more);
//! - an **episode** is an `SxxEyy` file name in a season folder. `yy` is kept as
//!   written (`01`, `17.5`), so two spellings are two episodes;
//! - a **subtitle** is a file of the same episode with a subtitle extension; a
//!   language fragment before the extension (`… S01E01.ko.smi`) is allowed.
//!
//! What does not fit is counted on the work as a file whose episode could not
//! be told ([`Unrecognized`]): a name whose `SxxEyy` is not its season folder's
//! season, a video or subtitle outside a season folder, anything in a folder
//! inside a season folder (a multi-file torrent's folder), a name with no
//! `SxxEyy`, and `.part` files (still downloading), wherever they are. Files of
//! other kinds (images, `.nfo`, text) are not media and are left out of
//! everything.
//!
//! # What is skipped
//!
//! Hidden entries (a leading `.`, which includes a work's `.trss/`) and the
//! folders NAS appliances and file systems add (`@eaDir`, `#recycle`,
//! `$RECYCLE.BIN`, `lost+found`, `System Volume Information`). A link is followed
//! only when it resolves to a place inside the watch folder, so a link can not
//! lead the scan out of it; a link that leaves, or that points nowhere, is
//! skipped.
//!
//! A video, subtitle or `.part` file whose name is not valid UTF-8 cannot be
//! stored by name: it is counted as unrecognized ([`Reason::InvalidName`]) under
//! its name with the invalid bytes replaced. A folder with such a name is read
//! under the replaced name too, except directly under the watch folder, where it
//! would be a work: that one is [`WorkRead::Unreadable`], so that the folder's
//! row says a work folder could not be read.
//!
//! # Unchanged directories
//!
//! The worker's periodic scan uses [`scan_incremental`]: it keeps what the last
//! scan listed in each directory ([`DirCache`]) and lists a directory again only
//! when its modification time or identity changed (or it was modified moments
//! before the last listing, or it holds a link). Entries are classified from
//! `readdir`'s own file type, so listing costs no call per file; only a link
//! needs more. The web's first reading of a folder and `다시 확인` list
//! everything. See [`scan_incremental`] for what this can miss.
//!
//! # Failures
//!
//! A watch folder that cannot be read is a [`ScanError`] and says nothing about
//! what is in it. A work folder that cannot be read is [`WorkRead::Unreadable`]
//! and the other works are still read, so one bad folder does not hide the rest.
//! So is a work folder in which any entry could not be examined (a failed
//! `file_type` or `canonicalize` other than "it is gone"): a transient error must
//! not look like a file that vanished. Neither may make a caller forget what it
//! knew.

use std::{
    collections::{BTreeSet, HashMap},
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::{SystemTime, UNIX_EPOCH},
};

use regex::Regex;

/// Video extensions, lower case.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "avi", "m4v", "webm", "ts", "mov", "wmv", "flv", "m2ts",
];
/// Subtitle extensions, lower case.
pub const SUBTITLE_EXTENSIONS: &[&str] = &["ass", "ssa", "smi", "srt", "vtt", "sup"];

/// Folders that are never works or seasons, whatever is in them.
const IGNORED_NAMES: &[&str] = &[
    "@eaDir",
    "#recycle",
    "$RECYCLE.BIN",
    "lost+found",
    "System Volume Information",
];

/// How far below a season folder (or another folder of a work) files are
/// looked for. Deep enough for any torrent folder, and it ends a link loop.
const MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Video,
    Subtitle,
}

impl FileKind {
    pub fn code(self) -> &'static str {
        match self {
            FileKind::Video => "video",
            FileKind::Subtitle => "subtitle",
        }
    }

    pub fn from_code(code: &str) -> Option<FileKind> {
        match code {
            "video" => Some(FileKind::Video),
            "subtitle" => Some(FileKind::Subtitle),
            _ => None,
        }
    }
}

/// Why a file is not attached to an episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    /// The `SxxEyy` in the name is not the season of the folder it is in.
    SeasonMismatch,
    /// A video or subtitle in the work folder, or in a folder that is not a
    /// `Season NN` folder.
    OutsideSeason,
    /// In a folder inside a season folder.
    InSubfolder,
    /// A `.part` file: still downloading.
    Partial,
    /// In a season folder, but the name has no `SxxEyy`.
    NoEpisode,
    /// A media or `.part` file whose name is not valid UTF-8.
    InvalidName,
}

impl Reason {
    pub fn code(self) -> &'static str {
        match self {
            Reason::SeasonMismatch => "season_mismatch",
            Reason::OutsideSeason => "outside_season",
            Reason::InSubfolder => "in_subfolder",
            Reason::Partial => "partial",
            Reason::NoEpisode => "no_episode",
            Reason::InvalidName => "invalid_name",
        }
    }

    pub fn from_code(code: &str) -> Option<Reason> {
        [
            Reason::SeasonMismatch,
            Reason::OutsideSeason,
            Reason::InSubfolder,
            Reason::Partial,
            Reason::NoEpisode,
            Reason::InvalidName,
        ]
        .into_iter()
        .find(|reason| reason.code() == code)
    }

    /// The reason as a sentence fragment for a screen.
    pub fn message(self) -> &'static str {
        match self {
            Reason::SeasonMismatch => "파일 이름의 시즌이 들어 있는 시즌 폴더와 달라요",
            Reason::OutsideSeason => "시즌 폴더 밖에 있어요",
            Reason::InSubfolder => "시즌 폴더 안의 하위 폴더에 있어요",
            Reason::Partial => "아직 받는 중인 파일이에요",
            Reason::NoEpisode => "이름에서 회차를 읽지 못했어요",
            Reason::InvalidName => "파일 이름이 UTF-8이 아니라서 읽지 못했어요",
        }
    }
}

/// A video or subtitle attached to an episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeFile {
    /// Relative to the work folder, `/`-separated.
    pub path: String,
    pub kind: FileKind,
    pub season: u32,
    /// The episode as written in the name: `01`, `17.5`.
    pub episode: String,
}

/// A file that could not be attached to an episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unrecognized {
    /// Relative to the work folder, `/`-separated.
    pub path: String,
    pub reason: Reason,
}

/// One work folder as read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScannedWork {
    /// The folder's name under the watch folder.
    pub dir_name: String,
    /// The seasons that have a folder, even an empty one.
    pub seasons: BTreeSet<u32>,
    pub files: Vec<EpisodeFile>,
    pub unrecognized: Vec<Unrecognized>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkRead {
    Read(ScannedWork),
    /// The folder exists but could not be read; `reason` is a sentence.
    Unreadable {
        dir_name: String,
        reason: String,
    },
}

impl WorkRead {
    pub fn dir_name(&self) -> &str {
        match self {
            WorkRead::Read(work) => &work.dir_name,
            WorkRead::Unreadable { dir_name, .. } => dir_name,
        }
    }
}

/// Everything one scan of a watch folder found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scan {
    /// In folder-name order.
    pub works: Vec<WorkRead>,
}

/// A watch folder that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanError {
    /// A sentence for the folder's row on the screen.
    pub message: String,
    /// What the system said, for logs only.
    pub detail: String,
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.detail)
    }
}

impl std::error::Error for ScanError {}

fn scan_error(error: &io::Error) -> ScanError {
    let message = match error.kind() {
        io::ErrorKind::NotFound => {
            "폴더를 찾지 못했어요. 마운트가 풀렸거나 이름이 바뀌었는지 확인해 주세요."
        }
        io::ErrorKind::PermissionDenied => "폴더를 읽을 권한이 없어요.",
        io::ErrorKind::NotADirectory => "폴더가 아니에요.",
        _ => "폴더를 읽지 못했어요.",
    };
    ScanError {
        message: message.to_owned(),
        detail: error.to_string(),
    }
}

fn unreadable_reason(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::PermissionDenied => "읽을 권한이 없어요.".to_owned(),
        io::ErrorKind::NotFound => "읽는 도중 없어졌어요.".to_owned(),
        _ => "읽지 못했어요.".to_owned(),
    }
}

/// The entry's name with invalid UTF-8 replaced, and whether it was valid.
fn entry_name(entry: &fs::DirEntry) -> (String, bool) {
    match entry.file_name().into_string() {
        Ok(name) => (name, true),
        Err(raw) => (raw.to_string_lossy().into_owned(), false),
    }
}

/// How many directories a scan read and how many it did not need to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanStats {
    /// Directories listed from the file system.
    pub dirs_read: usize,
    /// Directories whose listing was taken from the earlier scan because they
    /// had not changed.
    pub dirs_reused: usize,
}

/// What an earlier scan saw of each directory it listed, to skip the directories
/// that have not changed since (see [`scan_incremental`]).
#[derive(Debug, Clone, Default)]
pub struct DirCache {
    dirs: HashMap<PathBuf, CachedDir>,
}

impl DirCache {
    /// How many directories the cache holds.
    pub fn len(&self) -> usize {
        self.dirs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty()
    }

    /// Takes what `other` saw: its directories replace the ones of the same
    /// path, and the rest stay (each is checked against the directory's own
    /// stamp before it is used, so an old one is only ever a miss).
    pub fn merge(&mut self, other: DirCache) {
        self.dirs.extend(other.dirs);
    }
}

/// The identity and modification time of a directory when it was listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    dev: u64,
    ino: u64,
    /// Nanoseconds since the Unix epoch.
    mtime_ns: i128,
}

#[cfg(unix)]
fn stamp_of(metadata: &fs::Metadata) -> Option<Stamp> {
    use std::os::unix::fs::MetadataExt;
    Some(Stamp {
        dev: metadata.dev(),
        ino: metadata.ino(),
        mtime_ns: i128::from(metadata.mtime()) * 1_000_000_000 + i128::from(metadata.mtime_nsec()),
    })
}

#[cfg(not(unix))]
fn stamp_of(_: &fs::Metadata) -> Option<Stamp> {
    None
}

/// A directory modified this close to the moment it was listed cannot be told
/// from an unchanged one by its modification time: a change in the same clock
/// tick (or the same two seconds, on a file system with coarse times) leaves the
/// time as it was. Such a listing is not reused; the next scan lists it again.
const RACY_WINDOW_NS: i128 = 5_000_000_000;

fn now_ns() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i128)
}

#[derive(Debug, Clone)]
struct CachedDir {
    stamp: Stamp,
    /// When the listing was made (nanoseconds since the Unix epoch).
    listed_at_ns: i128,
    entries: Arc<Vec<Listed>>,
}

impl CachedDir {
    /// Whether the listing can stand for the directory as `now` is (see [`RACY_WINDOW_NS`]).
    fn is_valid_for(&self, now: Stamp) -> bool {
        self.stamp == now && self.stamp.mtime_ns < self.listed_at_ns - RACY_WINDOW_NS
    }
}

/// One entry of a listed directory.
#[derive(Debug, Clone)]
struct Listed {
    /// The name, with invalid UTF-8 replaced.
    name: String,
    /// The raw name when it was not valid UTF-8.
    raw: Option<OsString>,
    /// What the entry is, or why it could not be examined (a sentence).
    node: Result<Node, String>,
    /// The entry is a link.
    link: bool,
}

impl Listed {
    fn valid(&self) -> bool {
        self.raw.is_none()
    }

    fn path_in(&self, dir: &Path) -> PathBuf {
        match &self.raw {
            Some(raw) => dir.join(raw),
            None => dir.join(&self.name),
        }
    }
}

/// What [`scan_incremental`] returns.
#[derive(Debug)]
pub struct Scanned {
    pub result: Result<Scan, ScanError>,
    /// What this scan saw, for the next one. Empty when the scan failed.
    pub cache: DirCache,
    pub stats: ScanStats,
}

/// Reads the watch folder at `root`, listing every directory. See the module docs.
pub fn scan(root: &Path) -> Result<Scan, ScanError> {
    scan_incremental(root, None).result
}

/// Reads the watch folder at `root`, taking the listing of a directory from
/// `previous` (what the scan before saw) when the directory has not changed
/// since: same device and inode, same modification time, and not modified
/// within [`RACY_WINDOW_NS`] of that listing. A directory's modification time
/// changes when an entry is added, removed or renamed directly in it, so a new
/// file in one season folder is found, and only that folder is listed again.
/// Nothing else is taken from `previous`: the directories below an unchanged
/// one are each checked on their own.
///
/// A directory is always listed when `previous` is `None`, when it holds a link
/// (a link's target can change without the directory changing), when it is
/// reached through a link, and when its modification time cannot be read. The
/// modification time only decides whether to list again; it never reaches an
/// added time, which the library takes from the scan's own time.
pub fn scan_incremental(root: &Path, previous: Option<DirCache>) -> Scanned {
    let mut walker = Walker {
        real_root: PathBuf::new(),
        previous: previous.unwrap_or_default(),
        next: DirCache::default(),
        stats: ScanStats::default(),
    };
    let result = walker.scan(root);
    let cache = if result.is_ok() {
        walker.next
    } else {
        DirCache::default()
    };
    Scanned {
        result,
        cache,
        stats: walker.stats,
    }
}

/// Reads only the works named `names` of the watch folder at `root`, with the
/// same rules as [`scan_incremental`] (`previous` is used in the same way, and
/// the returned cache holds only what this read looked at; see
/// [`DirCache::merge`]). A name that is not a work folder now (gone, a file,
/// hidden) is simply not in the result. `Err` says the watch folder itself
/// could not be read.
pub fn scan_works(root: &Path, names: &[String], previous: Option<DirCache>) -> Scanned {
    let mut walker = Walker {
        real_root: PathBuf::new(),
        previous: previous.unwrap_or_default(),
        next: DirCache::default(),
        stats: ScanStats::default(),
    };
    let result = walker.scan_named(root, names);
    Scanned {
        result,
        cache: walker.next,
        stats: walker.stats,
    }
}

struct Walker {
    real_root: PathBuf,
    previous: DirCache,
    next: DirCache,
    stats: ScanStats,
}

impl Walker {
    fn scan(&mut self, root: &Path) -> Result<Scan, ScanError> {
        self.real_root = fs::canonicalize(root).map_err(|e| scan_error(&e))?;
        let real_root = self.real_root.clone();
        let entries = self
            .listing(&real_root, false)
            .map_err(|e| scan_error(&e))?;

        let mut works = Vec::new();
        for listed in entries.iter() {
            if skipped(&listed.name) {
                continue;
            }
            // An entry that cannot be examined may be a work: say so, so that
            // the record of a work by that name is kept.
            let node = match &listed.node {
                Ok(node) => *node,
                Err(reason) => {
                    works.push(WorkRead::Unreadable {
                        dir_name: listed.name.clone(),
                        reason: reason.clone(),
                    });
                    continue;
                }
            };
            if node != Node::Dir {
                continue;
            }
            if !listed.valid() {
                works.push(WorkRead::Unreadable {
                    dir_name: listed.name.clone(),
                    reason: "폴더 이름이 UTF-8이 아니라서 읽지 못했어요.".to_owned(),
                });
                continue;
            }
            let read = match self.read_work(&listed.path_in(&real_root), &listed.name, listed.link)
            {
                Ok(work) => WorkRead::Read(work),
                Err(reason) => WorkRead::Unreadable {
                    dir_name: listed.name.clone(),
                    reason,
                },
            };
            works.push(read);
        }
        Ok(Scan { works })
    }

    /// Reads the works called `names` only.
    fn scan_named(&mut self, root: &Path, names: &[String]) -> Result<Scan, ScanError> {
        self.real_root = fs::canonicalize(root).map_err(|e| scan_error(&e))?;
        let real_root = self.real_root.clone();
        // A watch folder that cannot be examined is the folder's error.
        fs::metadata(&real_root).map_err(|e| scan_error(&e))?;
        let mut works = Vec::new();
        for name in names {
            if skipped(name) {
                continue;
            }
            let path = real_root.join(name);
            let file_type = fs::symlink_metadata(&path).map(|m| m.file_type());
            let link = file_type.as_ref().is_ok_and(|t| t.is_symlink());
            match classify_at(&path, file_type, &real_root) {
                Ok(Node::Dir) => works.push(match self.read_work(&path, name, link) {
                    Ok(work) => WorkRead::Read(work),
                    Err(reason) => WorkRead::Unreadable {
                        dir_name: name.clone(),
                        reason,
                    },
                }),
                Ok(_) => {}
                Err(error) => works.push(WorkRead::Unreadable {
                    dir_name: name.clone(),
                    reason: unreadable_reason(&error),
                }),
            }
        }
        Ok(Scan { works })
    }

    /// The entries of `dir`: from the earlier scan when `dir` has not changed,
    /// else from the file system. `via_link`: `dir` is a link or inside one.
    fn listing(&mut self, dir: &Path, via_link: bool) -> io::Result<Arc<Vec<Listed>>> {
        let stamp = if via_link {
            None
        } else {
            fs::metadata(dir).ok().and_then(|m| stamp_of(&m))
        };
        if let (Some(stamp), Some(cached)) = (stamp, self.previous.dirs.get(dir)) {
            if cached.is_valid_for(stamp) {
                self.stats.dirs_reused += 1;
                let entries = cached.entries.clone();
                self.next.dirs.insert(dir.to_path_buf(), cached.clone());
                return Ok(entries);
            }
        }

        let listed_at_ns = now_ns();
        self.stats.dirs_read += 1;
        let mut listed = Vec::new();
        for entry in sorted_entries(dir)? {
            let (name, valid) = entry_name(&entry);
            let raw = (!valid).then(|| entry.file_name());
            // What is skipped is not looked at, so no error of its own matters.
            let (node, link) = if skipped(&name) {
                (Ok(Node::Skip), false)
            } else {
                (
                    classify(&entry, &self.real_root).map_err(|e| unreadable_reason(&e)),
                    entry.file_type().is_ok_and(|t| t.is_symlink()),
                )
            };
            listed.push(Listed {
                name,
                raw,
                node,
                link,
            });
        }
        let cacheable = listed.iter().all(|l| l.node.is_ok() && !l.link);
        let entries = Arc::new(listed);
        if let (Some(stamp), true) = (stamp, cacheable) {
            self.next.dirs.insert(
                dir.to_path_buf(),
                CachedDir {
                    stamp,
                    listed_at_ns,
                    entries: entries.clone(),
                },
            );
        }
        Ok(entries)
    }

    /// Reads one work folder. The error is a sentence for the folder's row.
    fn read_work(&mut self, dir: &Path, name: &str, via_link: bool) -> Result<ScannedWork, String> {
        let mut work = ScannedWork {
            dir_name: name.to_owned(),
            ..ScannedWork::default()
        };
        let entries = self
            .listing(dir, via_link)
            .map_err(|e| unreadable_reason(&e))?;
        for listed in entries.iter() {
            if skipped(&listed.name) {
                continue;
            }
            match listed.node.clone()? {
                Node::Skip => {}
                Node::File => {
                    if is_media_or_partial(&listed.name) {
                        let reason = if !listed.valid() {
                            Reason::InvalidName
                        } else if is_partial(&listed.name) {
                            Reason::Partial
                        } else {
                            Reason::OutsideSeason
                        };
                        work.unrecognized.push(Unrecognized {
                            path: listed.name.clone(),
                            reason,
                        });
                    }
                }
                Node::Dir => {
                    let path = listed.path_in(dir);
                    let via_link = via_link || listed.link;
                    match season_of_folder(&listed.name) {
                        Some(season) => {
                            work.seasons.insert(season);
                            self.read_season(&path, &listed.name, season, via_link, &mut work)?;
                        }
                        None => self.collect_all(
                            &path,
                            &listed.name,
                            Reason::OutsideSeason,
                            1,
                            via_link,
                            &mut work,
                        )?,
                    }
                }
            }
        }
        Ok(work)
    }

    /// Reads the files directly in a season folder; what is in folders below it
    /// is all unrecognized.
    fn read_season(
        &mut self,
        dir: &Path,
        folder: &str,
        season: u32,
        via_link: bool,
        work: &mut ScannedWork,
    ) -> Result<(), String> {
        let entries = self
            .listing(dir, via_link)
            .map_err(|e| unreadable_reason(&e))?;
        for listed in entries.iter() {
            if skipped(&listed.name) {
                continue;
            }
            let name = &listed.name;
            let path = format!("{folder}/{name}");
            match listed.node.clone()? {
                Node::Skip => {}
                Node::Dir => self.collect_all(
                    &listed.path_in(dir),
                    &path,
                    Reason::InSubfolder,
                    1,
                    via_link || listed.link,
                    work,
                )?,
                Node::File => {
                    if !listed.valid() {
                        if is_media_or_partial(name) {
                            work.unrecognized.push(Unrecognized {
                                path,
                                reason: Reason::InvalidName,
                            });
                        }
                        continue;
                    }
                    if is_partial(name) {
                        work.unrecognized.push(Unrecognized {
                            path,
                            reason: Reason::Partial,
                        });
                        continue;
                    }
                    let Some(kind) = kind_of(name) else {
                        continue;
                    };
                    match episode_of(name, kind) {
                        None => work.unrecognized.push(Unrecognized {
                            path,
                            reason: Reason::NoEpisode,
                        }),
                        Some((found, _)) if found != season => {
                            work.unrecognized.push(Unrecognized {
                                path,
                                reason: Reason::SeasonMismatch,
                            })
                        }
                        Some((_, episode)) => work.files.push(EpisodeFile {
                            path,
                            kind,
                            season,
                            episode,
                        }),
                    }
                }
            }
        }
        Ok(())
    }

    /// Every video, subtitle and `.part` file below `dir`, as unrecognized for
    /// `reason` (a `.part` file is always [`Reason::Partial`]).
    fn collect_all(
        &mut self,
        dir: &Path,
        shown: &str,
        reason: Reason,
        depth: usize,
        via_link: bool,
        work: &mut ScannedWork,
    ) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Ok(());
        }
        let entries = self
            .listing(dir, via_link)
            .map_err(|e| unreadable_reason(&e))?;
        for listed in entries.iter() {
            if skipped(&listed.name) {
                continue;
            }
            let name = &listed.name;
            let path = format!("{shown}/{name}");
            match listed.node.clone()? {
                Node::Skip => {}
                Node::Dir => self.collect_all(
                    &listed.path_in(dir),
                    &path,
                    reason,
                    depth + 1,
                    via_link || listed.link,
                    work,
                )?,
                Node::File => {
                    if !listed.valid() {
                        if is_media_or_partial(name) {
                            work.unrecognized.push(Unrecognized {
                                path,
                                reason: Reason::InvalidName,
                            });
                        }
                    } else if is_partial(name) {
                        work.unrecognized.push(Unrecognized {
                            path,
                            reason: Reason::Partial,
                        });
                    } else if kind_of(name).is_some() {
                        work.unrecognized.push(Unrecognized { path, reason });
                    }
                }
            }
        }
        Ok(())
    }
}

/// Whether discovery leaves an entry of this name out of everything: hidden
/// entries and the folders appliances add.
pub fn is_skipped(name: &str) -> bool {
    skipped(name)
}

fn skipped(name: &str) -> bool {
    name.starts_with('.') || IGNORED_NAMES.contains(&name)
}

/// The entries of `dir`, by name.
fn sorted_entries(dir: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(dir)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_cached_key(|entry| entry.file_name());
    Ok(entries)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Node {
    File,
    Dir,
    /// Not followed or not a file: a link out of the watch folder, a broken
    /// link, a socket.
    Skip,
}

/// Whether an error says the path is not there (a link to nothing, a link
/// loop, an entry removed since the folder was listed), as opposed to a failure
/// to look.
fn is_gone(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
        || error.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error())
}

/// What an entry is, following a link only when it stays inside `real_root`.
/// An error other than "it is gone" is returned: the entry could not be
/// examined, which is not the same as it not being there.
fn classify(entry: &fs::DirEntry, real_root: &Path) -> io::Result<Node> {
    classify_at(&entry.path(), entry.file_type(), real_root)
}

/// [`classify`] for the entry at `path`, given what the system said its type is
/// (without following a link).
fn classify_at(
    path: &Path,
    file_type: io::Result<fs::FileType>,
    real_root: &Path,
) -> io::Result<Node> {
    let file_type = match file_type {
        Ok(file_type) => file_type,
        Err(error) if is_gone(&error) => return Ok(Node::Skip),
        Err(error) => return Err(error),
    };
    if file_type.is_dir() {
        return Ok(Node::Dir);
    }
    if file_type.is_file() {
        return Ok(Node::File);
    }
    if !file_type.is_symlink() {
        return Ok(Node::Skip);
    }
    let real = match fs::canonicalize(path) {
        Ok(real) => real,
        Err(error) if is_gone(&error) => return Ok(Node::Skip),
        Err(error) => return Err(error),
    };
    if !real.starts_with(real_root) {
        return Ok(Node::Skip);
    }
    match fs::metadata(&real) {
        Ok(metadata) if metadata.is_dir() => {
            // A link to the folder it is in, or to one above it, only loops.
            let Some(parent) = path.parent().map(fs::canonicalize).transpose()? else {
                return Ok(Node::Skip);
            };
            Ok(if parent.starts_with(&real) {
                Node::Skip
            } else {
                Node::Dir
            })
        }
        Ok(metadata) if metadata.is_file() => Ok(Node::File),
        Ok(_) => Ok(Node::Skip),
        Err(error) if is_gone(&error) => Ok(Node::Skip),
        Err(error) => Err(error),
    }
}

/// The season of a `Season NN` folder name.
pub fn season_of_folder(name: &str) -> Option<u32> {
    static SEASON: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^Season\s+(\d{1,4})$").unwrap());
    SEASON.captures(name)?[1].parse().ok()
}

fn extension(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// Whether a file name is a video or a subtitle, by its extension.
pub fn kind_of(name: &str) -> Option<FileKind> {
    let ext = extension(name)?;
    if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        Some(FileKind::Video)
    } else if SUBTITLE_EXTENSIONS.contains(&ext.as_str()) {
        Some(FileKind::Subtitle)
    } else {
        None
    }
}

fn is_partial(name: &str) -> bool {
    name.len() > ".part".len() && name.to_ascii_lowercase().ends_with(".part")
}

/// Whether a file is something discovery counts: media, or a download in progress.
fn is_media_or_partial(name: &str) -> bool {
    is_partial(name) || kind_of(name).is_some()
}

/// The `SxxEyy` a file name ends with (before its extension and, for a
/// subtitle, before language fragments): the season and the episode as written.
pub fn episode_of(name: &str, kind: FileKind) -> Option<(u32, String)> {
    static EPISODE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:^|[^A-Za-z0-9])S(\d{1,4})E(\d{1,4}(?:\.\d{1,2})?)$").unwrap()
    });
    static LANGUAGE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9_-]{0,15}$").unwrap());

    let stem = &name[..name.rfind('.')?];
    let mut stem = stem;
    let mut fragments = 0;
    loop {
        if let Some(caught) = EPISODE.captures(stem) {
            return Some((caught[1].parse().ok()?, caught[2].to_owned()));
        }
        // Only a subtitle carries a language (and flags such as `forced`).
        if kind != FileKind::Subtitle || fragments == 2 {
            return None;
        }
        let dot = stem.rfind('.')?;
        if !LANGUAGE.is_match(&stem[dot + 1..]) {
            return None;
        }
        stem = &stem[..dot];
        fragments += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, relative: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
    }

    fn work<'a>(scan: &'a Scan, name: &str) -> &'a ScannedWork {
        scan.works
            .iter()
            .find_map(|w| match w {
                WorkRead::Read(work) if work.dir_name == name => Some(work),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no work {name}"))
    }

    #[test]
    fn episodes_are_read_as_written() {
        let video = |n: &str| episode_of(n, FileKind::Video);
        assert_eq!(video("Show S01E01.mkv"), Some((1, "01".into())));
        assert_eq!(video("Show S02E17.5.mkv"), Some((2, "17.5".into())));
        assert_eq!(video("Show S01E105.mkv"), Some((1, "105".into())));
        assert_eq!(video("show s01e03.MP4"), Some((1, "03".into())));
        assert_eq!(video("Show S01E01 1080p.mkv"), None);
        assert_eq!(video("ShowS01E01.mkv"), None);
        assert_eq!(video("Show.mkv"), None);
        // Only a subtitle has a language fragment.
        assert_eq!(video("Show S01E01.ko.mkv"), None);
        let sub = |n: &str| episode_of(n, FileKind::Subtitle);
        assert_eq!(sub("Show S01E01.ass"), Some((1, "01".into())));
        assert_eq!(sub("Show S01E01.ko.smi"), Some((1, "01".into())));
        assert_eq!(sub("Show S01E02.5.ko.forced.ass"), Some((1, "02.5".into())));
        assert_eq!(sub("Show S01E02.a.b.c.ass"), None);
        assert_eq!(sub("Show S01E02.한국어.ass"), None);
    }

    #[test]
    fn season_folders_are_season_and_a_number() {
        assert_eq!(season_of_folder("Season 01"), Some(1));
        assert_eq!(season_of_folder("Season 0"), Some(0));
        assert_eq!(season_of_folder("season 12"), Some(12));
        assert_eq!(season_of_folder("Season"), None);
        assert_eq!(season_of_folder("Season 1 extras"), None);
        assert_eq!(season_of_folder("Specials"), None);
        assert_eq!(season_of_folder("Season -1"), None);
    }

    #[test]
    fn a_trname_folder_is_read_into_seasons_episodes_and_files() {
        let dir = tempfile::tempdir().unwrap();
        for file in [
            "Lycoris Recoil/Season 01/Lycoris Recoil S01E01.mkv",
            "Lycoris Recoil/Season 01/Lycoris Recoil S01E01.smi",
            "Lycoris Recoil/Season 01/Lycoris Recoil S01E02.mkv",
            "Lycoris Recoil/Season 02/Lycoris Recoil S02E01.mkv",
        ] {
            touch(dir.path(), file);
        }
        let scan = scan(dir.path()).unwrap();
        assert_eq!(scan.works.len(), 1);
        let work = work(&scan, "Lycoris Recoil");
        assert_eq!(work.seasons, BTreeSet::from([1, 2]));
        assert!(work.unrecognized.is_empty());
        let files: Vec<_> = work
            .files
            .iter()
            .map(|f| (f.season, f.episode.as_str(), f.kind, f.path.as_str()))
            .collect();
        assert_eq!(
            files,
            [
                (
                    1,
                    "01",
                    FileKind::Video,
                    "Season 01/Lycoris Recoil S01E01.mkv"
                ),
                (
                    1,
                    "01",
                    FileKind::Subtitle,
                    "Season 01/Lycoris Recoil S01E01.smi"
                ),
                (
                    1,
                    "02",
                    FileKind::Video,
                    "Season 01/Lycoris Recoil S01E02.mkv"
                ),
                (
                    2,
                    "01",
                    FileKind::Video,
                    "Season 02/Lycoris Recoil S02E01.mkv"
                ),
            ]
        );
    }

    #[test]
    fn what_does_not_fit_is_counted_with_a_reason_and_hidden_things_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        for file in [
            "W/Season 01/W S01E01.mkv",
            // The season in the name is not the folder's.
            "W/Season 01/W S03E01.mkv",
            // No episode in the name.
            "W/Season 01/extra.mkv",
            // Outside a season folder.
            "W/W S01E02.mkv",
            "W/Extras/Making of.mp4",
            // In a folder inside a season folder.
            "W/Season 01/[Group] Batch/ep 02.mkv",
            "W/Season 01/[Group] Batch/deeper/ep 03.mkv",
            // Still downloading.
            "W/Season 01/W S01E03.mkv.part",
            "W/loose.part",
            // Not media, or not discovered.
            "W/Season 01/cover.jpg",
            "W/Season 01/W S01E01.nfo",
            "W/notes.txt",
            "W/.trss/subs/W S01E01.ass",
            "W/Season 01/.hidden S01E09.mkv",
            ".Hidden/Season 01/H S01E01.mkv",
            "@eaDir/Season 01/E S01E01.mkv",
        ] {
            touch(dir.path(), file);
        }
        let scan = scan(dir.path()).unwrap();
        assert_eq!(
            scan.works.len(),
            1,
            "hidden and appliance folders are not works"
        );
        let work = work(&scan, "W");
        assert_eq!(work.files.len(), 1);
        assert_eq!(work.files[0].path, "Season 01/W S01E01.mkv");
        let mut unrecognized: Vec<_> = work
            .unrecognized
            .iter()
            .map(|u| (u.path.as_str(), u.reason))
            .collect();
        unrecognized.sort();
        assert_eq!(
            unrecognized,
            [
                ("Extras/Making of.mp4", Reason::OutsideSeason),
                ("Season 01/W S01E03.mkv.part", Reason::Partial),
                ("Season 01/W S03E01.mkv", Reason::SeasonMismatch),
                (
                    "Season 01/[Group] Batch/deeper/ep 03.mkv",
                    Reason::InSubfolder
                ),
                ("Season 01/[Group] Batch/ep 02.mkv", Reason::InSubfolder),
                ("Season 01/extra.mkv", Reason::NoEpisode),
                ("W S01E02.mkv", Reason::OutsideSeason),
                ("loose.part", Reason::Partial),
            ]
        );
    }

    #[test]
    fn links_are_followed_only_inside_the_watch_folder() {
        let dir = tempfile::tempdir().unwrap();
        let watch = dir.path().join("watch");
        let outside = dir.path().join("outside");
        touch(&watch, "Real/Season 01/Real S01E01.mkv");
        touch(&outside, "Out/Season 01/Out S01E01.mkv");
        touch(&outside, "loose/Linked S01E01.mkv");
        std::os::unix::fs::symlink(outside.join("Out"), watch.join("Linked")).unwrap();
        std::os::unix::fs::symlink(
            outside.join("loose/Linked S01E01.mkv"),
            watch.join("Real/Season 01/Real S01E02.mkv"),
        )
        .unwrap();
        // Inside links work, and a link to an ancestor does not loop forever.
        std::os::unix::fs::symlink(watch.join("Real"), watch.join("Alias")).unwrap();
        std::os::unix::fs::symlink(&watch, watch.join("Real/Season 01/loop")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("missing"), watch.join("Broken")).unwrap();

        let scan = scan(&watch).unwrap();
        let names: Vec<_> = scan.works.iter().map(WorkRead::dir_name).collect();
        assert_eq!(names, ["Alias", "Real"]);
        let real = work(&scan, "Real");
        assert_eq!(real.files.len(), 1, "{:?}", real.files);
        assert_eq!(real.files[0].path, "Season 01/Real S01E01.mkv");
        // The link to an ancestor is not walked.
        assert!(real.unrecognized.is_empty(), "{:?}", real.unrecognized);
    }

    #[test]
    fn an_unreadable_work_folder_does_not_hide_the_other_works() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Good/Season 01/Good S01E01.mkv");
        touch(dir.path(), "Locked/Season 01/Locked S01E01.mkv");
        let locked = dir.path().join("Locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let scan = scan(dir.path());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        let scan = scan.unwrap();
        assert_eq!(scan.works.len(), 2);
        assert_eq!(work(&scan, "Good").files.len(), 1);
        match &scan.works[1] {
            WorkRead::Unreadable { dir_name, reason } => {
                assert_eq!(dir_name, "Locked");
                assert!(reason.contains("권한"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_entry_that_cannot_be_examined_makes_its_work_unreadable_not_empty() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Hold/inner/target.mkv");
        touch(dir.path(), "W/Season 01/W S01E01.mkv");
        // A link whose target cannot be resolved because a folder on the way
        // cannot be searched: that is a failure to look, not a file that is gone.
        std::os::unix::fs::symlink(
            dir.path().join("Hold/inner/target.mkv"),
            dir.path().join("W/Season 01/W S01E02.mkv"),
        )
        .unwrap();
        let hold = dir.path().join("Hold");
        fs::set_permissions(&hold, fs::Permissions::from_mode(0o000)).unwrap();
        let scan = scan(dir.path());
        fs::set_permissions(&hold, fs::Permissions::from_mode(0o755)).unwrap();
        let scan = scan.unwrap();
        let w = scan
            .works
            .iter()
            .find(|w| w.dir_name() == "W")
            .expect("W is listed");
        assert!(
            matches!(w, WorkRead::Unreadable { .. }),
            "a work with an entry that could not be examined is unreadable, not read as smaller: {w:?}"
        );
    }

    #[test]
    fn a_link_that_points_nowhere_or_loops_is_just_skipped() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "W/Season 01/W S01E01.mkv");
        std::os::unix::fs::symlink(
            dir.path().join("nothing"),
            dir.path().join("W/Season 01/W S01E02.mkv"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("W/Season 01/W S01E03.mkv"),
            dir.path().join("W/Season 01/W S01E03.mkv"),
        )
        .unwrap();
        let scan = scan(dir.path()).unwrap();
        let w = work(&scan, "W");
        assert_eq!(w.files.len(), 1, "{:?}", w.files);
        assert!(w.unrecognized.is_empty(), "{:?}", w.unrecognized);
    }

    #[test]
    fn names_that_are_not_utf8_are_counted_with_a_reason_not_dropped() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "W/Season 01/W S01E01.mkv");
        let bad = |parent: &str, bytes: &[u8]| {
            let mut path = dir.path().join(parent);
            fs::create_dir_all(&path).unwrap();
            path.push(OsStr::from_bytes(bytes));
            fs::write(path, "x").unwrap();
        };
        bad("W/Season 01", b"W S01E02 \xff.mkv");
        bad("W", b"loose \xfe.mp4");
        bad("W/Season 01/batch", b"ep \xfd.mkv");
        // Not media: not counted, like any other such file.
        bad("W/Season 01", b"notes \xff.txt");
        // A work folder whose name cannot be stored is reported, not skipped.
        fs::create_dir(dir.path().join(OsStr::from_bytes(b"Bad \xff"))).unwrap();

        let scan = scan(dir.path()).unwrap();
        let w = work(&scan, "W");
        assert_eq!(w.files.len(), 1);
        let mut unrecognized: Vec<_> = w
            .unrecognized
            .iter()
            .map(|u| (u.path.as_str(), u.reason))
            .collect();
        unrecognized.sort();
        assert_eq!(
            unrecognized,
            [
                ("Season 01/W S01E02 \u{fffd}.mkv", Reason::InvalidName),
                ("Season 01/batch/ep \u{fffd}.mkv", Reason::InvalidName),
                ("loose \u{fffd}.mp4", Reason::InvalidName),
            ]
        );
        let names: Vec<_> = scan.works.iter().map(WorkRead::dir_name).collect();
        assert_eq!(names, ["Bad \u{fffd}", "W"]);
        assert!(matches!(scan.works[0], WorkRead::Unreadable { .. }));
    }

    // --- skipping directories that have not changed ---------------------------------

    /// Gives every directory under `root` (and `root`) a modification time in
    /// 2001, as a folder that has not been touched for a long time has.
    fn age_dirs(root: &Path) {
        fn walk(dir: &Path) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() && !path.is_symlink() {
                    walk(&path);
                }
            }
            set_mtime(dir, 1_000_000_000);
        }
        walk(root);
    }

    fn set_mtime(dir: &Path, secs: u64) {
        fs::File::open(dir)
            .unwrap()
            .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(secs))
            .unwrap();
    }

    fn file_paths(scan: &Scan, name: &str) -> Vec<String> {
        work(scan, name)
            .files
            .iter()
            .map(|f| f.path.clone())
            .collect()
    }

    /// 2 works, 3 seasons: the root, 2 work folders and 3 season folders are 6 directories.
    fn two_works(root: &Path) {
        for file in [
            "A/Season 01/A S01E01.mkv",
            "A/Season 01/A S01E02.mkv",
            "A/Season 02/A S02E01.mkv",
            "B/Season 01/B S01E01.mkv",
        ] {
            touch(root, file);
        }
        age_dirs(root);
    }

    #[test]
    fn named_works_are_read_alone_and_what_is_not_a_work_is_left_out() {
        let dir = tempfile::tempdir().unwrap();
        two_works(dir.path());
        touch(dir.path(), "loose.mkv");
        touch(dir.path(), ".Hidden/Season 01/H S01E01.mkv");

        let names: Vec<String> = ["A", "gone", "loose.mkv", ".Hidden"]
            .map(str::to_owned)
            .to_vec();
        let read = scan_works(dir.path(), &names, None);
        let scan = read.result.unwrap();
        let found: Vec<_> = scan.works.iter().map(WorkRead::dir_name).collect();
        assert_eq!(found, ["A"], "only a folder that is a work now is returned");
        // Work A, its two seasons: three directories; B and the root are not read.
        assert_eq!(read.stats.dirs_read, 3);
        assert_eq!(file_paths(&scan, "A").len(), 3);

        // The same read of what has not changed lists nothing, and what it keeps
        // merges into an earlier cache.
        let mut cache = scan_incremental(dir.path(), None).cache;
        let again = scan_works(dir.path(), &names, Some(cache.clone()));
        assert_eq!(again.stats.dirs_read, 0);
        assert_eq!(again.stats.dirs_reused, 3);
        cache.merge(again.cache);
        assert_eq!(cache.len(), 6);
    }

    #[test]
    fn reading_named_works_of_a_folder_that_is_gone_is_the_folders_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nowhere");
        let read = scan_works(&missing, &["A".to_owned()], None);
        assert!(read.result.unwrap_err().message.contains("찾지 못했어요"));
    }

    #[test]
    fn an_unchanged_tree_is_not_listed_again_and_reads_the_same() {
        let dir = tempfile::tempdir().unwrap();
        two_works(dir.path());

        let first = scan_incremental(dir.path(), None);
        assert_eq!(
            first.stats,
            ScanStats {
                dirs_read: 6,
                dirs_reused: 0
            }
        );
        assert_eq!(first.cache.len(), 6);

        let second = scan_incremental(dir.path(), Some(first.cache));
        assert_eq!(
            second.stats,
            ScanStats {
                dirs_read: 0,
                dirs_reused: 6
            },
            "nothing changed, so nothing is listed"
        );
        assert_eq!(second.result.unwrap(), first.result.unwrap());
        // And the cache it leaves is as good for the next time.
        let third = scan_incremental(dir.path(), Some(second.cache));
        assert_eq!(third.stats.dirs_read, 0);
    }

    #[test]
    fn a_file_added_or_removed_in_one_season_folder_is_found_and_only_that_folder_is_listed() {
        let dir = tempfile::tempdir().unwrap();
        two_works(dir.path());
        let first = scan_incremental(dir.path(), None);

        touch(dir.path(), "A/Season 01/A S01E03.mkv");
        let second = scan_incremental(dir.path(), Some(first.cache));
        assert_eq!(
            second.stats,
            ScanStats {
                dirs_read: 1,
                dirs_reused: 5
            }
        );
        let scan = second.result.unwrap();
        assert_eq!(
            file_paths(&scan, "A"),
            [
                "Season 01/A S01E01.mkv",
                "Season 01/A S01E02.mkv",
                "Season 01/A S01E03.mkv",
                "Season 02/A S02E01.mkv"
            ]
        );
        assert_eq!(file_paths(&scan, "B").len(), 1);

        // Back to old, as a folder that has settled: the removal is found too.
        set_mtime(&dir.path().join("A/Season 01"), 1_000_000_000);
        let third = scan_incremental(dir.path(), Some(second.cache));
        // The change was within the window of the last listing, so it was listed again.
        assert_eq!(third.stats.dirs_read, 1);
        fs::remove_file(dir.path().join("A/Season 01/A S01E01.mkv")).unwrap();
        let fourth = scan_incremental(dir.path(), Some(third.cache));
        assert_eq!(fourth.stats.dirs_read, 1);
        assert_eq!(
            file_paths(&fourth.result.unwrap(), "A"),
            [
                "Season 01/A S01E02.mkv",
                "Season 01/A S01E03.mkv",
                "Season 02/A S02E01.mkv"
            ]
        );
    }

    #[test]
    fn a_new_season_folder_and_a_new_work_are_found() {
        let dir = tempfile::tempdir().unwrap();
        two_works(dir.path());
        let first = scan_incremental(dir.path(), None);

        touch(dir.path(), "A/Season 03/A S03E01.mkv");
        touch(dir.path(), "C/Season 01/C S01E01.mkv");
        let second = scan_incremental(dir.path(), Some(first.cache));
        let scan = second.result.unwrap();
        assert_eq!(work(&scan, "A").seasons, BTreeSet::from([1, 2, 3]));
        assert_eq!(file_paths(&scan, "C"), ["Season 01/C S01E01.mkv"]);
        // The root, A, its new season, and the new work's two folders were
        // listed; A's other two seasons, B and B's season were not.
        assert_eq!(
            second.stats,
            ScanStats {
                dirs_read: 5,
                dirs_reused: 4
            }
        );
    }

    #[test]
    fn a_file_renamed_in_place_is_found() {
        let dir = tempfile::tempdir().unwrap();
        two_works(dir.path());
        let first = scan_incremental(dir.path(), None);

        fs::rename(
            dir.path().join("B/Season 01/B S01E01.mkv"),
            dir.path().join("B/Season 01/B S01E05.mkv"),
        )
        .unwrap();
        let second = scan_incremental(dir.path(), Some(first.cache));
        assert_eq!(second.stats.dirs_read, 1);
        assert_eq!(
            file_paths(&second.result.unwrap(), "B"),
            ["Season 01/B S01E05.mkv"]
        );
    }

    #[test]
    fn a_full_read_lists_everything_and_finds_a_change_that_kept_the_modification_time() {
        let dir = tempfile::tempdir().unwrap();
        two_works(dir.path());
        let first = scan_incremental(dir.path(), None);

        // A change that leaves the directory's time as it was (a coarse clock):
        // an incremental scan cannot see it, a full read can.
        touch(dir.path(), "B/Season 01/B S01E02.mkv");
        set_mtime(&dir.path().join("B/Season 01"), 1_000_000_000);
        let periodic = scan_incremental(dir.path(), Some(first.cache.clone()));
        assert_eq!(periodic.stats.dirs_read, 0);
        assert_eq!(file_paths(&periodic.result.unwrap(), "B").len(), 1);

        let full = scan_incremental(dir.path(), None);
        assert_eq!(
            full.stats,
            ScanStats {
                dirs_read: 6,
                dirs_reused: 0
            }
        );
        assert_eq!(file_paths(&full.result.unwrap(), "B").len(), 2);
    }

    #[test]
    fn a_directory_changed_within_the_window_of_its_listing_is_listed_again() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "A/Season 01/A S01E01.mkv");
        // Not aged: every directory was modified a moment ago.
        let first = scan_incremental(dir.path(), None);
        let second = scan_incremental(dir.path(), Some(first.cache));
        assert_eq!(second.stats.dirs_reused, 0, "{:?}", second.stats);
        assert_eq!(second.stats.dirs_read, 3);
    }

    #[test]
    fn links_are_always_listed_and_so_is_what_is_below_one() {
        let dir = tempfile::tempdir().unwrap();
        let watch = dir.path().join("watch");
        touch(&watch, "Real/Season 01/Real S01E01.mkv");
        touch(&watch, "Plain/Season 01/Plain S01E01.mkv");
        std::os::unix::fs::symlink(watch.join("Real"), watch.join("Alias")).unwrap();
        age_dirs(&watch);

        let first = scan_incremental(&watch, None);
        let second = scan_incremental(&watch, Some(first.cache));
        // The watch folder holds a link, so it is listed; Alias is a link, so it
        // and its season folder are; Real and Plain are not.
        assert_eq!(second.stats.dirs_read, 3, "{:?}", second.stats);
        assert_eq!(second.stats.dirs_reused, 4, "{:?}", second.stats);
        assert_eq!(second.result.unwrap().works.len(), 3);
    }

    #[test]
    fn a_failed_scan_leaves_no_cache_and_an_unreadable_work_is_tried_again() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        two_works(dir.path());
        let missing = scan_incremental(&dir.path().join("nope"), None);
        assert!(missing.result.is_err() && missing.cache.is_empty());

        let locked = dir.path().join("B");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let first = scan_incremental(dir.path(), None);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        set_mtime(&locked, 1_000_000_000);
        assert!(first
            .result
            .as_ref()
            .unwrap()
            .works
            .iter()
            .any(|w| matches!(w, WorkRead::Unreadable { .. })));
        // B was not cached, so it is read now that it can be.
        let second = scan_incremental(dir.path(), Some(first.cache));
        assert_eq!(file_paths(&second.result.unwrap(), "B").len(), 1);
    }

    #[test]
    fn a_missing_or_file_watch_folder_is_an_error_with_a_sentence() {
        let dir = tempfile::tempdir().unwrap();
        let error = scan(&dir.path().join("nope")).unwrap_err();
        assert!(error.message.contains("찾지 못했어요"), "{error}");
        let file = dir.path().join("file");
        fs::write(&file, "x").unwrap();
        assert!(scan(&file).is_err());
    }
}
