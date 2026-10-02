//! Turns at the folders the worker's tasks work in, so that work which touches
//! the same files runs one after the other and the rest runs side by side.
//!
//! The worker runs its collection cycle, the web's commands and the readings
//! its folder watches ask for at the same time (see the `trss-worker` crate).
//! Each piece of work that looks at or changes files names the folders it
//! touches as a [`Section`] and waits for its turn at them ([`FolderLocks`]):
//!
//! - **Read** (`read`): the work looks at what is under the folder or adds to
//!   it (a torrent saved into a work folder, a scan that lists it). Reads share
//!   a folder.
//! - **Write** (`write`): the work moves or renames what is under the folder
//!   (a work folder moved to the archive folder, videos renamed back). It
//!   waits for every read and write of the folder, and they wait for it.
//! - **Reading** (`reading`): the work records a reading of a watch folder in
//!   the library. Two readings of one watch folder never run together (a
//!   later reading must not be overwritten by an earlier one that ends after
//!   it, and a folder's scan must not be started twice); a reading asks for
//!   a read of the folders it lists besides.
//! - **Item** (`item`): the work adds or renames the torrent of one feed
//!   item. An item is not a folder: it overlaps the same item alone, never a
//!   folder, so two receives of one item (a retry and the cycle's add of the
//!   same release) go one after the other while each only reads its work
//!   folder, and receives of other items and readings of the folder go on
//!   beside them. The rename of a receive's own torrent is part of its add:
//!   it touches that item's file alone, which the item keeps apart.
//!
//! Two folders overlap when they are the same or one is inside the other, by
//! whole path components after `.` and `..` are resolved by text (links are
//! not followed). Two sections conflict when a place of one overlaps a place
//! of the other and their kinds exclude each other as above.
//!
//! The table knows only the names it is given. A folder reached through a
//! link, or named before a setting changed which folder the work will touch,
//! is not seen as the same folder: the work itself still checks what it finds
//! on disk, and the turns only keep the usual paths apart.
//!
//! Turns are first come, first served: [`FolderLocks::reserve`] puts a section
//! in line at once, and [`Reservation::ready`] waits until no section that came
//! before it and conflicts with it is left, whether that one is at work or
//! still waiting itself. So a write is not starved by a stream of reads, and a
//! read that does not conflict with a waiting write passes it.
//!
//! **One section per task at a time.** A task that holds a section must not
//! wait for another one: behind a write that came in between, the second
//! would wait for the first, which never ends. Work that needs several
//! folders names them all in one section.

use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

use tokio::sync::Notify;

/// How a section uses one of its folders; see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    Read,
    Write,
    Reading,
}

impl Access {
    fn excludes(self, other: Access) -> bool {
        // A reading meets the writes through the read it asks for besides.
        matches!(
            (self, other),
            (Access::Write, Access::Read | Access::Write)
                | (Access::Read, Access::Write)
                | (Access::Reading, Access::Reading)
        )
    }
}

/// What a section names: a folder, or a feed item (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    Folder(PathBuf),
    /// A channel's ID and the item's identity key.
    Item(String, String),
}

impl Place {
    fn overlaps(&self, other: &Place) -> bool {
        match (self, other) {
            (Place::Folder(a), Place::Folder(b)) => a.starts_with(b) || b.starts_with(a),
            (Place::Item(..), Place::Item(..)) => self == other,
            _ => false,
        }
    }
}

/// The folders and items one piece of work touches, and how. Empty sections
/// wait for nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Section {
    parts: Vec<(Place, Access)>,
}

impl Section {
    pub fn new() -> Section {
        Section::default()
    }

    /// The work looks at what is under `folder`, or adds to it.
    pub fn read(self, folder: impl AsRef<Path>) -> Section {
        self.with(folder.as_ref(), Access::Read)
    }

    /// The work moves or renames what is under `folder`.
    pub fn write(self, folder: impl AsRef<Path>) -> Section {
        self.with(folder.as_ref(), Access::Write)
    }

    /// The work records a reading of the watch folder `folder` (and reads
    /// what it lists, which the caller names with [`Section::read`]).
    pub fn reading(self, folder: impl AsRef<Path>) -> Section {
        self.with(folder.as_ref(), Access::Reading)
    }

    /// The work adds or renames the torrent of the item `identity_key` of
    /// channel `channel_id`: alone, among the work on that item.
    pub fn item(mut self, channel_id: &str, identity_key: &str) -> Section {
        self.parts.push((
            Place::Item(channel_id.to_owned(), identity_key.to_owned()),
            Access::Write,
        ));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    fn with(mut self, folder: &Path, access: Access) -> Section {
        self.parts.push((Place::Folder(lexical(folder)), access));
        self
    }

    fn conflicts(&self, other: &Section) -> bool {
        self.parts.iter().any(|(mine, access)| {
            other.parts.iter().any(|(theirs, their_access)| {
                access.excludes(*their_access) && mine.overlaps(theirs)
            })
        })
    }
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

#[derive(Default)]
struct State {
    next: u64,
    /// The sections in line, at work or waiting, by their place in line.
    entries: BTreeMap<u64, Section>,
}

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    /// Told whenever a section leaves the line.
    left: Notify,
}

/// The line of sections of one worker. Cheap to clone; clones share the line.
#[derive(Clone, Default)]
pub struct FolderLocks {
    inner: Arc<Inner>,
}

impl FolderLocks {
    pub fn new() -> FolderLocks {
        FolderLocks::default()
    }

    /// Puts `section` in line now. Its turn comes with [`Reservation::ready`];
    /// dropping the reservation leaves the line.
    pub fn reserve(&self, section: Section) -> Reservation {
        if section.is_empty() {
            return Reservation { entry: None };
        }
        let mut state = self.state();
        let place = state.next;
        state.next += 1;
        state.entries.insert(place, section);
        Reservation {
            entry: Some(Entry {
                locks: self.clone(),
                place,
            }),
        }
    }

    /// Waits for the turn of `section`, put in line now.
    pub async fn lock(&self, section: Section) -> FolderGuard {
        self.reserve(section).ready().await
    }

    /// The turn of `section` if it is free now; `None` (and out of line again)
    /// when some section before it conflicts with it.
    pub fn try_lock(&self, section: Section) -> Option<FolderGuard> {
        let reservation = self.reserve(section);
        match &reservation.entry {
            Some(entry) if entry.blocked() => None,
            _ => Some(FolderGuard {
                _entry: reservation.entry,
            }),
        }
    }

    /// How many sections are in line, at work or waiting.
    pub fn len(&self) -> usize {
        self.state().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A section in line (see [`FolderLocks::reserve`]).
pub struct Reservation {
    entry: Option<Entry>,
}

impl Reservation {
    /// Waits until no section that came before this one conflicts with it.
    pub async fn ready(self) -> FolderGuard {
        let Some(entry) = self.entry else {
            return FolderGuard { _entry: None };
        };
        let inner = entry.locks.inner.clone();
        loop {
            // Listening before looking, so a section that leaves in between
            // is not missed.
            let left = inner.left.notified();
            tokio::pin!(left);
            left.as_mut().enable();
            if !entry.blocked() {
                return FolderGuard {
                    _entry: Some(entry),
                };
            }
            left.await;
        }
    }
}

/// The turn of a section; the section leaves the line when this is dropped.
pub struct FolderGuard {
    _entry: Option<Entry>,
}

struct Entry {
    locks: FolderLocks,
    place: u64,
}

impl Entry {
    fn blocked(&self) -> bool {
        let state = self.locks.state();
        let Some(mine) = state.entries.get(&self.place) else {
            return false;
        };
        state
            .entries
            .range(..self.place)
            .any(|(_, earlier)| mine.conflicts(earlier))
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.locks.state().entries.remove(&self.place);
        self.locks.inner.left.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Whether `reservation` gets its turn within a short while.
    async fn turn_comes(reservation: Reservation) -> Option<FolderGuard> {
        tokio::time::timeout(Duration::from_millis(50), reservation.ready())
            .await
            .ok()
    }

    #[tokio::test]
    async fn reads_share_a_folder_and_a_write_waits_for_them() {
        let locks = FolderLocks::new();
        let first = locks.lock(Section::new().read("/media/A")).await;
        let second = locks.lock(Section::new().read("/media/A")).await;

        let write = tokio::spawn({
            let locks = locks.clone();
            async move { locks.lock(Section::new().write("/media/A")).await }
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!write.is_finished());

        drop(first);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!write.is_finished(), "one read is still at work");
        drop(second);
        let _write = tokio::time::timeout(Duration::from_secs(5), write)
            .await
            .expect("the write's turn comes once the reads are done")
            .unwrap();
    }

    #[tokio::test]
    async fn a_folder_overlaps_the_folders_inside_it_but_not_its_neighbours() {
        let locks = FolderLocks::new();
        let _moving = locks.lock(Section::new().write("/media/A")).await;

        assert!(locks.try_lock(Section::new().read("/media")).is_none());
        assert!(locks
            .try_lock(Section::new().read("/media/A/Season 01"))
            .is_none());
        assert!(locks.try_lock(Section::new().read("/media/./A/")).is_none());
        assert!(locks
            .try_lock(Section::new().read("/media/B/../A"))
            .is_none());
        // Whole components: `/media/AB` is not inside `/media/A`.
        assert!(locks.try_lock(Section::new().read("/media/AB")).is_some());
        assert!(locks.try_lock(Section::new().write("/media/B")).is_some());
        assert!(locks.try_lock(Section::new()).is_some());
        // A refused try leaves no trace in the line.
        assert_eq!(locks.len(), 1);
    }

    #[tokio::test]
    async fn an_item_goes_alone_among_its_own_work_and_meets_no_folder() {
        let locks = FolderLocks::new();
        let retry = locks
            .try_lock(Section::new().read("/media/A").item("ch", "guid:1"))
            .expect("free");
        // The cycle's add of the same item waits; another item's goes in.
        let same = Section::new().read("/media/A").item("ch", "guid:1");
        assert!(locks.try_lock(same.clone()).is_none());
        let other = locks.try_lock(Section::new().read("/media/A").item("ch", "guid:2"));
        assert!(other.is_some());
        assert!(locks
            .try_lock(Section::new().read("/media/A").item("other", "guid:1"))
            .is_some());
        // Readings of the whole folder go on beside them; a move waits.
        assert!(locks
            .try_lock(Section::new().reading("/media").read("/media"))
            .is_some());
        assert!(locks.try_lock(Section::new().write("/media/A")).is_none());
        // No folder overlaps an item, whatever its text.
        assert!(locks.try_lock(Section::new().write("/")).is_none());
        assert!(locks
            .try_lock(
                Section::new()
                    .write("ch")
                    .write("guid:1")
                    .write("ch/guid:1")
            )
            .is_some());

        drop((retry, other));
        assert!(locks.try_lock(same).is_some());
    }

    #[tokio::test]
    async fn turns_are_first_come_first_served() {
        let locks = FolderLocks::new();
        let reading = locks.lock(Section::new().read("/media/A")).await;
        // A write waits for the read, and a read after it waits for the write
        // rather than pass it.
        let write = locks.reserve(Section::new().write("/media/A"));
        let late_read = locks.reserve(Section::new().read("/media/A/Season 01"));
        // A read of another folder conflicts with neither and goes at once.
        let elsewhere = locks.reserve(Section::new().read("/media/B"));
        assert!(turn_comes(elsewhere).await.is_some());

        let late_read = tokio::spawn(late_read.ready());
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!late_read.is_finished(), "the read waits behind the write");

        drop(reading);
        let writing = turn_comes(write).await.expect("the write is next");
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!late_read.is_finished());
        drop(writing);
        tokio::time::timeout(Duration::from_secs(5), late_read)
            .await
            .expect("the read follows the write")
            .unwrap();
    }

    #[tokio::test]
    async fn dropping_a_waiting_reservation_lets_the_ones_behind_it_go() {
        let locks = FolderLocks::new();
        let reading = locks.lock(Section::new().read("/media/A")).await;
        let write = locks.reserve(Section::new().write("/media/A"));
        let behind = tokio::spawn(locks.reserve(Section::new().read("/media/A")).ready());
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!behind.is_finished());

        drop(write);
        tokio::time::timeout(Duration::from_secs(5), behind)
            .await
            .expect("nothing conflicting is left before it")
            .unwrap();
        drop(reading);
        assert!(locks.is_empty());
    }

    #[tokio::test]
    async fn readings_of_one_watch_folder_go_one_at_a_time_beside_reads() {
        let locks = FolderLocks::new();
        let whole = Section::new().reading("/media").read("/media");
        let first = locks.lock(whole.clone()).await;

        // A second reading of the folder, even of one work, waits.
        let work = Section::new().reading("/media").read("/media/A");
        assert!(locks.try_lock(work.clone()).is_none());
        // Another watch folder's reading does not, nor does a read of a work.
        assert!(locks
            .try_lock(Section::new().reading("/archive").read("/archive"))
            .is_some());
        assert!(locks.try_lock(Section::new().read("/media/A")).is_some());
        // A write in the folder meets the reading's read.
        assert!(locks.try_lock(Section::new().write("/media/A")).is_none());

        drop(first);
        assert!(locks.try_lock(work).is_some());
    }

    #[tokio::test]
    async fn every_waiter_hears_about_a_section_that_leaves() {
        let locks = FolderLocks::new();
        let moving = locks.lock(Section::new().write("/media")).await;
        let waiters: Vec<_> = (0..20)
            .map(|i| {
                let ready = locks
                    .reserve(Section::new().read(format!("/media/{i}")))
                    .ready();
                tokio::spawn(ready)
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(waiters.iter().all(|w| !w.is_finished()));

        drop(moving);
        for waiter in waiters {
            tokio::time::timeout(Duration::from_secs(5), waiter)
                .await
                .expect("every read gets its turn")
                .unwrap();
        }
    }
}
