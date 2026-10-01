//! The directories of one watch folder that carry an inotify watch, and what
//! the kernel's events about them mean (see [`super`]).
//!
//! One [`WatchTree`] owns one inotify instance (so a queue overflow is about
//! one watch folder) and watches the folder's root, each work folder, and each
//! `Season NN` folder: not recursively by the kernel but directory by directory.
//! It only keeps its own bookkeeping up to date and says which works need a
//! new reading; reading and recording belong to the task around it.
//!
//! The watches are `create`, `delete`, `moved_from`, `moved_to`, `delete_self`
//! and `move_self`. Writes are not subscribed to: a file that is still being
//! written changes nothing discovery reads, and a `.part` that is renamed to
//! its final name is a rename.
//!
//! An event is a signal about *which work to read again*, never a description
//! of what changed. So a lost or reordered event costs at most a late reading.
//!
//! # Watches follow the tree
//!
//! A new work or season folder gets its watch as soon as its event is handled,
//! before anything lists it, so no file created after the watch is missed; the
//! reading that follows (after the debounce) sees what was there before it. A
//! folder that goes away or moves out loses its watch: the kernel's watch
//! follows the inode, not the path, so a moved directory would otherwise keep
//! reporting under its old name.
//!
//! A directory that cannot get a watch (the system's limit on watches, no
//! permission, a link) is remembered with the reason. Its work is read by the
//! worker every cycle instead ([`super::LiveWatch::poll_for`]), and the folder's
//! row says how many there are and why.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    ffi::{OsStr, OsString},
    fs, io,
    mem::MaybeUninit,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use rustix::{
    fd::OwnedFd,
    fs::inotify::{self, ReadFlags, WatchFlags},
    io::Errno,
};

use crate::discovery;

/// Why a directory has no watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Why {
    /// The system's limit on watches (`fs.inotify.max_user_watches`).
    Limit,
    /// No permission to read the directory.
    Denied,
    /// A link: a watch follows the directory it leads to, not the path, so the
    /// worker reads a link's work itself.
    Link,
    /// The directory is not there.
    Gone,
    /// Any other error, with its number.
    Other(i32),
}

impl Why {
    fn from_errno(errno: Errno) -> Why {
        match errno {
            Errno::NOSPC | Errno::NOMEM => Why::Limit,
            Errno::ACCESS | Errno::PERM => Why::Denied,
            Errno::NOENT | Errno::NOTDIR => Why::Gone,
            other => Why::Other(other.raw_os_error()),
        }
    }

    fn from_io(error: &io::Error) -> Why {
        match error.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => Why::Gone,
            io::ErrorKind::PermissionDenied => Why::Denied,
            _ => Why::Other(error.raw_os_error().unwrap_or(0)),
        }
    }

    /// The reason as a sentence for the folder's row.
    pub fn sentence(self) -> String {
        match self {
            Why::Limit => "감시 한도를 넘었어요. 호스트의 fs.inotify.max_user_watches를 늘리면 알림으로 지켜봐요.".to_owned(),
            Why::Denied => "폴더를 읽을 권한이 없어요.".to_owned(),
            Why::Link => "링크라서 감시를 걸지 않았어요.".to_owned(),
            Why::Gone => "폴더를 찾지 못했어요.".to_owned(),
            Why::Other(code) => format!("감시를 걸지 못했어요(오류 {code})."),
        }
    }

    /// A stable order, for choosing among reasons that are equally common.
    fn rank(self) -> (u8, i32) {
        match self {
            Why::Limit => (0, 0),
            Why::Denied => (1, 0),
            Why::Link => (2, 0),
            Why::Gone => (3, 0),
            Why::Other(code) => (4, code),
        }
    }
}

/// What a watched directory is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Role {
    /// The watch folder itself.
    Root,
    /// A work folder.
    Work(String),
    /// A `Season NN` folder of the work.
    Season(String),
}

#[derive(Debug, Clone)]
struct Dir {
    path: PathBuf,
    role: Role,
}

/// One event read from the kernel.
#[derive(Debug, Clone)]
pub struct RawEvent {
    pub wd: i32,
    pub mask: ReadFlags,
    pub name: Option<OsString>,
}

/// What the tree has to say about its watches, for the folder's status and row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeStatus {
    /// The folder's own directory has a watch.
    pub root_watched: bool,
    /// How many directories have no watch (the folder's own included).
    pub unwatched_dirs: usize,
    /// The works with at least one directory that has no watch.
    pub unwatched_works: BTreeSet<String>,
    /// How many watches there are.
    pub watches: usize,
    /// The sentence for the folder's row, when some directory has no watch.
    pub note: Option<String>,
}

/// What is due to be read again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Due {
    /// The whole folder (a queue overflow, a lost root, a name that cannot be
    /// told).
    pub folder: bool,
    /// Works to read, by folder name.
    pub works: Vec<String>,
}

impl Due {
    pub fn is_empty(&self) -> bool {
        !self.folder && self.works.is_empty()
    }
}

/// The watches of one watch folder.
pub struct WatchTree {
    root: PathBuf,
    fd: Arc<OwnedFd>,
    /// The most watches the tree may hold (a seam for tests; the system's own
    /// limit is the real one).
    limit: Option<usize>,
    debounce: Duration,
    dirs: HashMap<i32, Dir>,
    by_path: HashMap<PathBuf, i32>,
    unwatched: BTreeMap<PathBuf, Why>,
    /// Why the folder's own directory has no watch.
    root_why: Option<Why>,
    /// Works to read again, and when (the first event's time plus the debounce).
    dirty: HashMap<String, Instant>,
    folder_dirty: Option<Instant>,
}

const WATCH_EVENTS: WatchFlags = WatchFlags::CREATE
    .union(WatchFlags::DELETE)
    .union(WatchFlags::MOVED_FROM)
    .union(WatchFlags::MOVED_TO)
    .union(WatchFlags::DELETE_SELF)
    .union(WatchFlags::MOVE_SELF);

impl WatchTree {
    /// A tree with no watch yet; call [`WatchTree::sync_all`] to place them.
    pub fn new(
        root: PathBuf,
        fd: Arc<OwnedFd>,
        limit: Option<usize>,
        debounce: Duration,
    ) -> WatchTree {
        WatchTree {
            root,
            fd,
            limit,
            debounce,
            dirs: HashMap::new(),
            by_path: HashMap::new(),
            unwatched: BTreeMap::new(),
            root_why: None,
            dirty: HashMap::new(),
            folder_dirty: None,
        }
    }

    // --- placing watches ----------------------------------------------------------------

    /// Adds a watch for the directory at `path`.
    fn add(&mut self, path: &Path, role: Role) -> Result<(), Why> {
        if self.limit.is_some_and(|max| self.dirs.len() >= max) {
            return Err(Why::Limit);
        }
        // The folder itself may be a link to the folder; below it, a link is not followed.
        let flags = if role == Role::Root {
            WATCH_EVENTS | WatchFlags::ONLYDIR
        } else {
            WATCH_EVENTS | WatchFlags::ONLYDIR | WatchFlags::DONT_FOLLOW
        };
        let wd = inotify::add_watch(&*self.fd, path, flags).map_err(Why::from_errno)?;
        // The same directory under a new name keeps its descriptor.
        if let Some(old) = self.dirs.insert(
            wd,
            Dir {
                path: path.to_path_buf(),
                role,
            },
        ) {
            if old.path != path {
                self.by_path.remove(&old.path);
            }
        }
        self.by_path.insert(path.to_path_buf(), wd);
        Ok(())
    }

    /// Removes the watch on `path` and on everything below it, and forgets
    /// the directories below it that had none.
    fn drop_subtree(&mut self, path: &Path) {
        let found: Vec<i32> = self
            .by_path
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(_, wd)| *wd)
            .collect();
        for wd in found {
            let _ = inotify::remove_watch(&*self.fd, wd);
            self.forget(wd);
        }
        self.unwatched.retain(|p, _| !p.starts_with(path));
    }

    /// Drops the bookkeeping of a descriptor the kernel no longer has.
    fn forget(&mut self, wd: i32) {
        if let Some(dir) = self.dirs.remove(&wd) {
            if self.by_path.get(&dir.path) == Some(&wd) {
                self.by_path.remove(&dir.path);
            }
        }
    }

    /// Brings the watches to what is on the disk: the root, every work folder
    /// and their season folders. With `mark`, a folder that has a watch now and
    /// had none is also marked to be read (changes in between were not seen);
    /// that is for a re-sync, not for the first placement, whose reading is the
    /// worker's own first scan.
    pub fn sync_all(&mut self, mark: Option<Instant>) {
        let was_watched = self.by_path.contains_key(&self.root);
        if !was_watched {
            let root = self.root.clone();
            if let Err(why) = self.add(&root, Role::Root) {
                self.root_why = Some(why);
                let root = self.root.clone();
                // A folder that cannot be watched leaves nothing below it watched.
                self.drop_subtree(&root);
                return;
            }
        }
        self.root_why = None;
        if let (Some(at), false) = (mark, was_watched) {
            self.mark_folder(at);
        }

        let names = match work_names(&self.root) {
            Ok(names) => names,
            Err(error) => {
                eprintln!(
                    "Watch folder {}: cannot list it: {error}",
                    self.root.display()
                );
                return;
            }
        };
        let wanted: BTreeSet<&str> = names.iter().map(String::as_str).collect();
        let stale: Vec<PathBuf> = self
            .by_path
            .iter()
            .filter(|(_, wd)| matches!(self.dirs.get(wd).map(|d| &d.role), Some(Role::Work(_))))
            .map(|(p, _)| p.clone())
            .chain(
                self.unwatched
                    .keys()
                    .filter(|p| self.depth(p) == 1)
                    .cloned(),
            )
            .filter(|p| {
                p.file_name()
                    .and_then(OsStr::to_str)
                    .is_none_or(|name| !wanted.contains(name))
            })
            .collect();
        for path in stale {
            self.drop_subtree(&path);
        }
        for name in &names {
            self.sync_work(name, mark);
        }
    }

    /// How many components `path` has below the root.
    fn depth(&self, path: &Path) -> usize {
        path.strip_prefix(&self.root)
            .map_or(0, |rest| rest.components().count())
    }

    /// Brings the watches of one work folder to what is on the disk. Whether
    /// `name` is a work folder (or a link to one) now.
    pub fn sync_work(&mut self, name: &str, mark: Option<Instant>) -> bool {
        let path = self.root.join(name);
        let linked = match fs::symlink_metadata(&path) {
            Err(error) if Why::from_io(&error) == Why::Gone => {
                self.drop_subtree(&path);
                return false;
            }
            Err(error) => {
                self.drop_subtree(&path);
                self.unwatched.insert(path, Why::from_io(&error));
                return true;
            }
            Ok(meta) if meta.is_dir() => false,
            Ok(meta) if meta.file_type().is_symlink() => {
                // Only a link to a folder can be a work.
                if !fs::metadata(&path).is_ok_and(|m| m.is_dir()) {
                    self.drop_subtree(&path);
                    return false;
                }
                true
            }
            Ok(_) => {
                self.drop_subtree(&path);
                return false;
            }
        };
        if linked {
            self.drop_subtree(&path);
            self.unwatched.insert(path, Why::Link);
            return true;
        }

        let mut added = false;
        if !self.by_path.contains_key(&path) {
            match self.add(&path, Role::Work(name.to_owned())) {
                Ok(()) => {
                    // What changed before the watch was there was not seen.
                    added = true;
                    self.unwatched.remove(&path);
                }
                Err(Why::Gone) => {
                    self.drop_subtree(&path);
                    return false;
                }
                Err(why) => {
                    self.drop_subtree(&path);
                    self.unwatched.insert(path, why);
                    return true;
                }
            }
        }

        let seasons = match season_names(&path) {
            Ok(seasons) => seasons,
            // Gone meanwhile, or unreadable: the work reads as it must.
            Err(_) => return true,
        };
        let wanted: BTreeSet<&str> = seasons.iter().map(|(n, _)| n.as_str()).collect();
        let stale: Vec<PathBuf> = self
            .by_path
            .keys()
            .chain(self.unwatched.keys())
            .filter(|p| self.depth(p) == 2 && p.starts_with(&path))
            .filter(|p| {
                p.file_name()
                    .and_then(OsStr::to_str)
                    .is_none_or(|n| !wanted.contains(n))
            })
            .cloned()
            .collect();
        for season in stale {
            self.drop_subtree(&season);
        }
        for (season, link) in &seasons {
            added |= self.ensure_season(name, season, *link);
        }
        if let (Some(at), true) = (mark, added) {
            self.mark_work(name, at);
        }
        true
    }

    /// Gives the season folder `season` of `work` a watch if it has none (a
    /// link gets none). Whether a watch was added.
    fn ensure_season(&mut self, work: &str, season: &str, link: bool) -> bool {
        let path = self.root.join(work).join(season);
        if self.by_path.contains_key(&path) {
            return false;
        }
        if link {
            self.unwatched.insert(path, Why::Link);
            return false;
        }
        match self.add(&path, Role::Season(work.to_owned())) {
            Ok(()) => {
                self.unwatched.remove(&path);
                true
            }
            Err(Why::Gone) => {
                self.unwatched.remove(&path);
                false
            }
            Err(why) => {
                self.unwatched.insert(path, why);
                false
            }
        }
    }

    // --- reading what is due ------------------------------------------------------------

    fn mark_work(&mut self, name: &str, now: Instant) {
        self.dirty
            .entry(name.to_owned())
            .or_insert(now + self.debounce);
    }

    fn mark_folder(&mut self, now: Instant) {
        self.folder_dirty.get_or_insert(now + self.debounce);
    }

    /// When something is next due, if anything is.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.dirty.values().chain(&self.folder_dirty).min().copied()
    }

    /// Takes what is due at `now`. When the whole folder is due, the works due
    /// with it are part of that reading.
    pub fn take_due(&mut self, now: Instant) -> Due {
        let folder = self.folder_dirty.is_some_and(|at| at <= now);
        if folder {
            self.folder_dirty = None;
        }
        let mut works: Vec<String> = self
            .dirty
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(name, _)| name.clone())
            .collect();
        works.sort();
        for name in &works {
            self.dirty.remove(name);
        }
        if folder {
            works.clear();
        }
        Due { folder, works }
    }

    /// Puts `due` back to be taken again at `at` (the reading could not run now).
    pub fn defer(&mut self, due: Due, at: Instant) {
        if due.folder {
            self.folder_dirty = Some(self.folder_dirty.map_or(at, |old| old.min(at)));
        }
        for name in due.works {
            let slot = self.dirty.entry(name).or_insert(at);
            *slot = (*slot).min(at);
        }
    }

    // --- events -------------------------------------------------------------------------

    /// The kernel dropped events: what changed is unknown, so the watches are
    /// brought to the disk and the whole folder is read again.
    pub fn overflow(&mut self, now: Instant) {
        self.sync_all(None);
        self.mark_folder(now);
    }

    /// Handles what the kernel reported, at `now`.
    pub fn handle(&mut self, events: Vec<RawEvent>, now: Instant) {
        for event in events {
            self.handle_one(event, now);
        }
    }

    fn handle_one(&mut self, event: RawEvent, now: Instant) {
        if event.mask.contains(ReadFlags::QUEUE_OVERFLOW) {
            self.overflow(now);
            return;
        }
        let Some(dir) = self.dirs.get(&event.wd).cloned() else {
            // A watch that is gone already.
            return;
        };
        if event.mask.contains(ReadFlags::IGNORED) {
            self.forget(event.wd);
            return;
        }
        match &event.name {
            Some(name) => self.in_dir(&dir, name, event.mask, now),
            None => self.of_dir(&dir, event.mask, now),
        }
    }

    /// Something happened to an entry of a watched directory.
    fn in_dir(&mut self, dir: &Dir, name: &OsStr, mask: ReadFlags, now: Instant) {
        let Some(name) = name.to_str() else {
            // A name that cannot be spelled cannot be read alone.
            match &dir.role {
                Role::Root => self.mark_folder(now),
                Role::Work(work) | Role::Season(work) => self.mark_work(work, now),
            }
            return;
        };
        if discovery::is_skipped(name) {
            return;
        }
        let removed = mask.intersects(ReadFlags::DELETE | ReadFlags::MOVED_FROM);
        let added = mask.intersects(ReadFlags::CREATE | ReadFlags::MOVED_TO);
        match &dir.role {
            Role::Root => {
                if removed {
                    self.drop_subtree(&dir.path.join(name));
                }
                // The watch first, then the reading: nothing created after it
                // can be missed. A file made at the root is not a work.
                if !added || self.sync_work(name, None) {
                    self.mark_work(name, now);
                }
            }
            Role::Work(work) => {
                if discovery::season_of_folder(name).is_some() {
                    let path = dir.path.join(name);
                    if removed {
                        self.drop_subtree(&path);
                    }
                    if added {
                        let link =
                            fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink());
                        self.ensure_season(work, name, link);
                    }
                }
                self.mark_work(work, now);
            }
            Role::Season(work) => self.mark_work(work, now),
        }
    }

    /// Something happened to a watched directory itself.
    fn of_dir(&mut self, dir: &Dir, mask: ReadFlags, now: Instant) {
        if !mask.intersects(ReadFlags::DELETE_SELF | ReadFlags::MOVE_SELF | ReadFlags::UNMOUNT) {
            return;
        }
        match &dir.role {
            Role::Root => {
                // The folder is gone or moved: nothing is watched until it is back.
                let root = self.root.clone();
                self.drop_subtree(&root);
                self.root_why = Some(Why::Gone);
                self.mark_folder(now);
            }
            Role::Work(work) | Role::Season(work) => {
                self.drop_subtree(&dir.path);
                self.mark_work(work, now);
            }
        }
    }

    // --- status -------------------------------------------------------------------------

    pub fn status(&self) -> TreeStatus {
        let root_watched = self.by_path.contains_key(&self.root);
        let mut works = BTreeSet::new();
        for path in self.unwatched.keys() {
            if let Some(name) = path
                .strip_prefix(&self.root)
                .ok()
                .and_then(|rest| rest.components().next())
                .and_then(|c| c.as_os_str().to_str())
            {
                works.insert(name.to_owned());
            }
        }
        TreeStatus {
            root_watched,
            unwatched_dirs: self.unwatched.len() + usize::from(self.root_why.is_some()),
            unwatched_works: works,
            watches: self.dirs.len(),
            note: self.note(),
        }
    }

    /// The sentence for the folder's row.
    fn note(&self) -> Option<String> {
        let mut counts: HashMap<Why, usize> = HashMap::new();
        for why in self.unwatched.values().chain(&self.root_why) {
            *counts.entry(*why).or_default() += 1;
        }
        let kinds = counts.len();
        let (main, _) = counts
            .into_iter()
            .max_by_key(|(why, count)| (*count, std::cmp::Reverse(why.rank())))?;
        let more = if kinds > 1 {
            " 다른 까닭으로 감시를 걸지 못한 폴더도 있어요."
        } else {
            ""
        };
        Some(if let Some(why) = self.root_why {
            format!(
                "이 폴더를 알림으로 지켜보지 못해서 수집 주기마다 직접 확인해요. {}",
                why.sentence()
            )
        } else {
            format!(
                "폴더 {}개를 알림으로 지켜보지 못해서 그 작품은 수집 주기마다 직접 확인해요. {}{more}",
                self.unwatched.len(),
                main.sentence()
            )
        })
    }
}

/// The work folders of the watch folder at `root`: folders and links, by name,
/// except what discovery skips and names that are not text.
fn work_names(root: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if discovery::is_skipped(&name) {
            continue;
        }
        if entry
            .file_type()
            .is_ok_and(|t| t.is_dir() || t.is_symlink())
        {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// The season folders of the work at `work`: name, and whether it is a link.
fn season_names(work: &Path) -> io::Result<Vec<(String, bool)>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(work)? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if discovery::is_skipped(&name) || discovery::season_of_folder(&name).is_none() {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            names.push((name, false));
        } else if file_type.is_symlink() && fs::metadata(entry.path()).is_ok_and(|m| m.is_dir()) {
            names.push((name, true));
        }
    }
    names.sort();
    Ok(names)
}

/// Creates the inotify instance of a tree: non-blocking, not inherited by
/// child processes.
pub fn init() -> io::Result<OwnedFd> {
    Ok(inotify::init(
        inotify::CreateFlags::NONBLOCK | inotify::CreateFlags::CLOEXEC,
    )?)
}

/// Reads the events waiting on `fd`: `WouldBlock` when there are none.
pub fn drain(fd: &OwnedFd) -> io::Result<Vec<RawEvent>> {
    let mut buffer = [MaybeUninit::<u8>::uninit(); 16 * 1024];
    let mut reader = inotify::Reader::new(fd, &mut buffer);
    let mut events = Vec::new();
    loop {
        match reader.next() {
            Ok(event) => events.push(RawEvent {
                wd: event.wd(),
                mask: event.events(),
                name: event
                    .file_name()
                    .map(|name| OsStr::from_bytes(name.to_bytes()).to_owned()),
            }),
            Err(Errno::AGAIN) => break,
            Err(error) if events.is_empty() => return Err(error.into()),
            Err(_) => break,
        }
        // The rest stays in the kernel's queue and the descriptor stays readable.
        if events.len() >= 4096 && reader.is_buffer_empty() {
            break;
        }
    }
    if events.is_empty() {
        Err(io::ErrorKind::WouldBlock.into())
    } else {
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A tree over a fresh folder with the watches placed.
    struct Fixture {
        dir: tempfile::TempDir,
        tree: WatchTree,
    }

    const DEBOUNCE: Duration = Duration::from_millis(40);

    fn fixture(limit: Option<usize>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let fd = Arc::new(init().unwrap());
        Fixture {
            tree: WatchTree::new(root, fd, limit, DEBOUNCE),
            dir,
        }
    }

    impl Fixture {
        fn path(&self, relative: &str) -> PathBuf {
            self.dir.path().join(relative)
        }

        fn touch(&self, relative: &str) {
            let path = self.path(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x").unwrap();
        }

        /// Handles every event the kernel has queued, and what is due after the debounce.
        fn due(&mut self) -> Due {
            let events = drain(&self.tree.fd).unwrap_or_default();
            let now = Instant::now();
            self.tree.handle(events, now);
            self.tree.take_due(now + DEBOUNCE * 2)
        }
    }

    #[test]
    fn the_root_works_and_seasons_get_watches_and_a_file_marks_only_its_work() {
        let mut f = fixture(None);
        f.touch("A/Season 01/A S01E01.mkv");
        f.touch("B/Season 01/B S01E01.mkv");
        f.touch("B/Season 02/B S02E01.mkv");
        f.touch("C/extras/x.mkv");
        f.tree.sync_all(None);
        let status = f.tree.status();
        // root, A, A/S01, B, B/S01, B/S02, C (extras is not a season).
        assert_eq!(status.watches, 7);
        assert!(status.root_watched);
        assert_eq!(status.unwatched_dirs, 0);
        assert_eq!(status.note, None);
        assert_eq!(f.due(), Due::default());

        f.touch("A/Season 01/A S01E02.mkv");
        assert_eq!(
            f.due(),
            Due {
                folder: false,
                works: vec!["A".into()]
            }
        );
        // Writing into an existing file is not an event.
        fs::write(f.path("A/Season 01/A S01E02.mkv"), "more").unwrap();
        assert_eq!(f.due(), Due::default());
    }

    #[test]
    fn a_new_work_and_season_are_watched_before_they_are_read_and_their_files_are_seen() {
        let mut f = fixture(None);
        f.tree.sync_all(None);
        f.touch("New/Season 01/New S01E01.mkv");
        let due = f.due();
        assert_eq!(due.works, vec!["New".to_owned()]);
        // The root, the work and its season.
        assert_eq!(f.tree.status().watches, 3);

        f.touch("New/Season 01/New S01E02.mkv");
        assert_eq!(f.due().works, vec!["New".to_owned()]);
    }

    #[test]
    fn a_work_that_is_removed_or_moved_out_loses_its_watches() {
        let mut f = fixture(None);
        f.touch("A/Season 01/A S01E01.mkv");
        f.touch("B/Season 01/B S01E01.mkv");
        f.tree.sync_all(None);
        assert_eq!(f.tree.status().watches, 5);

        fs::remove_dir_all(f.path("A")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::rename(f.path("B"), outside.path().join("B")).unwrap();
        let mut due = f.due().works;
        due.sort();
        assert_eq!(due, ["A", "B"]);
        assert_eq!(f.tree.status().watches, 1, "only the root is left");

        // What happens to the moved folder is not reported under its old name.
        fs::write(outside.path().join("B/Season 01/B S01E02.mkv"), "x").unwrap();
        assert_eq!(f.due(), Due::default());
    }

    #[test]
    fn a_season_folder_renamed_follows_the_new_name() {
        let mut f = fixture(None);
        f.touch("A/Season 01/A S01E01.mkv");
        f.tree.sync_all(None);
        fs::rename(f.path("A/Season 01"), f.path("A/Season 02")).unwrap();
        assert_eq!(f.due().works, vec!["A".to_owned()]);
        assert_eq!(f.tree.status().watches, 3);
        f.touch("A/Season 02/A S02E01.mkv");
        assert_eq!(f.due().works, vec!["A".to_owned()]);
    }

    #[test]
    fn hidden_names_are_not_signals() {
        let mut f = fixture(None);
        f.touch("A/Season 01/A S01E01.mkv");
        f.tree.sync_all(None);
        f.touch("A/.trss/subs/a.ass");
        f.touch(".hidden/Season 01/h S01E01.mkv");
        f.touch("A/Season 01/.partial");
        assert_eq!(f.due(), Due::default());
    }

    #[test]
    fn the_directories_a_limit_leaves_out_are_counted_with_the_reason() {
        // Room for the root and two works with their seasons, not for the third.
        let mut f = fixture(Some(5));
        for work in ["A", "B", "C"] {
            f.touch(&format!("{work}/Season 01/{work} S01E01.mkv"));
        }
        f.tree.sync_all(None);
        let status = f.tree.status();
        assert_eq!(status.watches, 5);
        assert!(status.root_watched);
        assert_eq!(status.unwatched_works, BTreeSet::from(["C".to_owned()]));
        assert_eq!(status.unwatched_dirs, 1);
        let note = status.note.unwrap();
        assert!(note.contains("폴더 1개"), "{note}");
        assert!(note.contains("fs.inotify.max_user_watches"), "{note}");
    }

    #[test]
    fn a_link_is_not_watched_and_its_work_is_left_to_the_worker() {
        let mut f = fixture(None);
        f.touch("A/Season 01/A S01E01.mkv");
        std::os::unix::fs::symlink(f.path("A"), f.path("Linked")).unwrap();
        f.tree.sync_all(None);
        let status = f.tree.status();
        assert_eq!(
            status.unwatched_works,
            BTreeSet::from(["Linked".to_owned()])
        );
        assert!(status.note.unwrap().contains("링크"));
    }

    #[test]
    fn a_resync_watches_what_was_missed_and_marks_it_to_be_read() {
        let mut f = fixture(Some(3));
        f.touch("A/Season 01/A S01E01.mkv");
        f.touch("B/Season 01/B S01E01.mkv");
        f.tree.sync_all(None);
        assert!(!f.tree.status().unwatched_works.is_empty());

        // Room again: the same tree takes the rest, and what it had no watch on
        // is marked, since it may have changed unseen.
        f.tree.limit = None;
        f.tree.sync_all(Some(Instant::now()));
        let status = f.tree.status();
        assert_eq!(status.unwatched_dirs, 0);
        assert_eq!(status.note, None);
        assert_eq!(status.watches, 5);
        let due = f.tree.take_due(Instant::now() + DEBOUNCE * 2);
        assert!(!due.works.is_empty(), "{due:?}");
    }

    #[test]
    fn an_overflow_places_the_missing_watches_and_reads_the_whole_folder() {
        let mut f = fixture(None);
        f.tree.sync_all(None);
        f.touch("A/Season 01/A S01E01.mkv");
        // The events never arrive: the tree only learns that some were lost.
        let _ = drain(&f.tree.fd);
        f.tree.overflow(Instant::now());
        assert_eq!(f.tree.status().watches, 3);
        let due = f.tree.take_due(Instant::now() + DEBOUNCE * 2);
        assert!(due.folder);
        assert!(due.works.is_empty(), "the folder reading covers every work");
    }

    #[test]
    fn a_root_that_is_gone_is_unwatched_and_the_folder_is_due() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("media");
        fs::create_dir_all(root.join("A/Season 01")).unwrap();
        let mut tree = WatchTree::new(root.clone(), Arc::new(init().unwrap()), None, DEBOUNCE);
        tree.sync_all(None);
        assert!(tree.status().root_watched);

        fs::remove_dir_all(&root).unwrap();
        let events = drain(&tree.fd).unwrap();
        tree.handle(events, Instant::now());
        let status = tree.status();
        assert!(!status.root_watched);
        assert!(status.note.unwrap().contains("이 폴더를"));
        assert!(tree.take_due(Instant::now() + DEBOUNCE * 2).folder);

        // Back again: a re-sync watches it and marks it to be read.
        fs::create_dir_all(root.join("A/Season 01")).unwrap();
        tree.sync_all(Some(Instant::now()));
        assert!(tree.status().root_watched);
        assert_eq!(tree.status().note, None);
        assert!(tree.take_due(Instant::now() + DEBOUNCE * 2).folder);
    }

    #[test]
    fn due_work_waits_for_its_debounce_and_a_deferred_reading_comes_back() {
        let mut f = fixture(None);
        f.touch("A/Season 01/A S01E01.mkv");
        f.tree.sync_all(None);
        f.touch("A/Season 01/A S01E02.mkv");
        let events = drain(&f.tree.fd).unwrap();
        let now = Instant::now();
        f.tree.handle(events, now);

        assert_eq!(f.tree.take_due(now), Due::default());
        assert!(f.tree.next_deadline().is_some());
        let due = f.tree.take_due(now + DEBOUNCE * 2);
        assert_eq!(due.works, vec!["A".to_owned()]);
        assert_eq!(f.tree.next_deadline(), None);

        f.tree.defer(due, now + DEBOUNCE * 3);
        assert_eq!(f.tree.take_due(now + DEBOUNCE * 2), Due::default());
        assert_eq!(
            f.tree.take_due(now + DEBOUNCE * 3).works,
            vec!["A".to_owned()]
        );
    }
}
