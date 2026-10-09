//! File tools several features share: the rename that never replaces, the
//! folder sync that makes a rename outlast a power loss, and the writes built
//! on them.
//!
//! A write that must survive a power loss goes through a new file (written
//! and synced), a rename that replaces nothing, and a sync of the folders the
//! rename changed; a synced file is kept only while the folder entries that
//! name it are. The parts here are those steps, so the places that write
//! files do not each put them together by hand.

use std::{
    fs::{File, OpenOptions},
    io,
    os::{fd::AsFd, unix::fs::OpenOptionsExt},
    path::Path,
};

use rustix::fs::{renameat_with, OFlags, RenameFlags, CWD};

/// Renames `from` to `to` unless `to` exists (an `AlreadyExists` error then),
/// with `renameat2(RENAME_NOREPLACE)`. Never replaces anything, a directory
/// included. There is no fallback: a filesystem without the flag answers
/// `InvalidInput` or `Unsupported` ([`noreplace_unsupported`]).
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    rename_noreplace_at(CWD, from, CWD, to)
}

/// [`rename_noreplace`] of `from` in the folder `from_dir` (an open file
/// descriptor) to `to` in the folder `to_dir`, for a caller that must not
/// follow a path somebody else can change between its look and the rename.
pub fn rename_noreplace_at(
    from_dir: impl AsFd,
    from: impl rustix::path::Arg,
    to_dir: impl AsFd,
    to: impl rustix::path::Arg,
) -> io::Result<()> {
    renameat_with(from_dir, from, to_dir, to, RenameFlags::NOREPLACE).map_err(io::Error::from)
}

/// Whether `err`, from [`rename_noreplace`], says the filesystem cannot rename
/// without replacing (`RENAME_NOREPLACE` is not supported there).
pub fn noreplace_unsupported(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
    )
}

/// Syncs a folder, so a rename into it, a file made in it or a name removed
/// from it outlives a power loss.
pub fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(any(test, feature = "test-support"))]
    testing::before_sync(path)?;
    File::open(path)?.sync_all()?;
    #[cfg(any(test, feature = "test-support"))]
    testing::synced(path);
    Ok(())
}

/// [`sync_dir`] of a folder already open as `dir`.
pub fn sync_dir_fd(dir: impl AsFd) -> io::Result<()> {
    #[cfg(any(test, feature = "test-support"))]
    let path = testing::path_of(&dir)?;
    #[cfg(any(test, feature = "test-support"))]
    testing::before_sync(&path)?;
    rustix::fs::fsync(dir).map_err(io::Error::from)?;
    #[cfg(any(test, feature = "test-support"))]
    testing::synced(&path);
    Ok(())
}

/// Syncs a file already written, so its bytes outlive a power loss.
pub fn sync_file(file: &File, path: &Path) -> io::Result<()> {
    #[cfg(any(test, feature = "test-support"))]
    testing::before_sync(path)?;
    file.sync_all()?;
    #[cfg(any(test, feature = "test-support"))]
    testing::synced(path);
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = path;
    Ok(())
}

/// Makes `dir` and the folders above it that are missing, syncing the folder
/// each new one is in: a synced file is kept by a power loss only while its
/// folders are. A folder that is there already is not synced.
pub fn create_dir_all_synced(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    if let Some(parent) = dir.parent() {
        create_dir_all_synced(parent)?;
    }
    match std::fs::create_dir(dir) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists && dir.is_dir() => return Ok(()),
        Err(err) => return Err(err),
    }
    match dir.parent() {
        Some(parent) => sync_dir(parent),
        None => Ok(()),
    }
}

/// Syncs the two folders a rename of `from` to `to` changed, the one `to` is
/// in first. For a caller that tells a failed rename from a rename whose
/// folders could not be synced, and so calls [`rename_noreplace`] itself.
pub fn sync_renamed(from: &Path, to: &Path) -> io::Result<()> {
    for folder in [to.parent(), from.parent()].into_iter().flatten() {
        sync_dir(folder)?;
    }
    Ok(())
}

/// [`rename_noreplace`], then [`sync_renamed`]. An error from the sync leaves
/// the file renamed: it is in the new place, and the move is not known to
/// outlast a power loss.
pub fn rename_noreplace_synced(from: &Path, to: &Path) -> io::Result<()> {
    rename_noreplace(from, to)?;
    sync_renamed(from, to)
}

/// Makes the new file `path`, lets `fill` write it, and syncs it, so what
/// `fill` wrote is on the disk when this returns. Anything at `path` already,
/// a link included, makes the call fail with `AlreadyExists` and is left
/// alone. A file that `fill` or the sync fails on is removed again. `mode` is
/// the file's permissions as it is made (less the umask), `0o666` when
/// `None`; `fill` may change them before they are synced.
///
/// The folder `path` is in is not synced: that is the caller's, after the
/// rename that publishes the file ([`rename_noreplace_synced`]).
pub fn write_new<T>(
    path: &Path,
    mode: Option<u32>,
    fill: impl FnOnce(&mut File) -> io::Result<T>,
) -> io::Result<T> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .custom_flags(OFlags::NOFOLLOW.bits() as i32);
    if let Some(mode) = mode {
        options.mode(mode);
    }
    let mut file = options.open(path)?;
    let written = fill(&mut file).and_then(|value| sync_file(&file, path).map(|()| value));
    drop(file);
    if written.is_err() {
        let _ = std::fs::remove_file(path);
    }
    written
}

/// Whether something is at `path`: a link counts as itself, whatever it
/// points to, and a missing path is `false`, not an error. Any other failure
/// to look (a folder that cannot be searched) is the error.
pub fn occupied(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

/// `bytes` as lowercase hex, two digits each.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The suffix of the file Transmission is still downloading, in lower case
/// as Transmission writes it.
const PART_SUFFIX: &str = ".part";

/// The name Transmission gives the file `name` while it is downloading it.
pub fn part_name(name: &str) -> String {
    format!("{name}{PART_SUFFIX}")
}

/// The name of the file being downloaded, when `name` is the name
/// Transmission gives a download in progress: a non-empty name and `.part`,
/// in lower case.
pub fn without_part(name: &str) -> Option<&str> {
    name.strip_suffix(PART_SUFFIX)
        .filter(|stem| !stem.is_empty())
}

/// What a test sees of the syncs: which files and folders were synced, and a
/// sync that fails on request, since a sync leaves nothing on the disk a test
/// could look at. Paths are compared as the system spells them (links
/// resolved), and every test keeps to its own temporary folder.
#[cfg(any(test, feature = "test-support"))]
pub mod testing {
    use std::{
        collections::{HashMap, HashSet},
        io,
        os::fd::{AsFd, AsRawFd},
        path::{Path, PathBuf},
        sync::{LazyLock, Mutex, MutexGuard},
    };

    #[derive(Default)]
    struct State {
        synced: HashMap<PathBuf, usize>,
        failing: HashSet<PathBuf>,
    }

    static STATE: LazyLock<Mutex<State>> = LazyLock::new(Mutex::default);

    fn state() -> MutexGuard<'static, State> {
        STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn spelled(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    pub(super) fn before_sync(path: &Path) -> io::Result<()> {
        match state().failing.contains(&spelled(path)) {
            true => Err(io::Error::other("the sync was made to fail")),
            false => Ok(()),
        }
    }

    pub(super) fn synced(path: &Path) {
        *state().synced.entry(spelled(path)).or_default() += 1;
    }

    /// The folder a descriptor is open on, as the system names it.
    pub(super) fn path_of(fd: &impl AsFd) -> io::Result<PathBuf> {
        std::fs::read_link(format!("/proc/self/fd/{}", fd.as_fd().as_raw_fd()))
    }

    /// How many times `path` (a file or a folder) was synced.
    pub fn syncs_of(path: &Path) -> usize {
        state().synced.get(&spelled(path)).copied().unwrap_or(0)
    }

    /// Makes the syncs of `path` fail until the guard is dropped.
    pub fn fail_syncs_of(path: &Path) -> FailingSyncs {
        let path = spelled(path);
        state().failing.insert(path.clone());
        FailingSyncs(path)
    }

    pub struct FailingSyncs(PathBuf);

    impl Drop for FailingSyncs {
        fn drop(&mut self) {
            state().failing.remove(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occupied_tells_a_file_a_folder_and_a_link_from_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.mkv");
        std::fs::write(&file, "x").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path().join("gone"), &link).unwrap();
        assert!(occupied(&file).unwrap());
        assert!(occupied(dir.path()).unwrap());
        assert!(
            occupied(&link).unwrap(),
            "a link counts as itself, though it points nowhere"
        );
        assert!(!occupied(&dir.path().join("missing")).unwrap());
    }

    #[test]
    fn occupied_reports_a_failure_other_than_a_missing_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a");
        std::fs::write(&file, "x").unwrap();
        // A file is no folder to look in: `ENOTDIR`, not "missing".
        assert!(occupied(&file.join("b")).is_err());
    }

    #[test]
    fn hex_writes_two_lowercase_digits_a_byte() {
        assert_eq!(hex(&[]), "");
        assert_eq!(hex(&[0x00, 0x0f, 0xab, 0xff]), "000fabff");
    }

    #[test]
    fn a_part_name_is_the_name_and_a_lowercase_suffix() {
        assert_eq!(part_name("Show S01E01.mkv"), "Show S01E01.mkv.part");
        assert_eq!(
            without_part("Show S01E01.mkv.part"),
            Some("Show S01E01.mkv")
        );
        assert_eq!(without_part(&part_name("a b.mkv")), Some("a b.mkv"));
    }

    #[test]
    fn only_a_nonempty_name_before_a_lowercase_suffix_is_a_part() {
        assert_eq!(without_part("loose.part"), Some("loose"));
        assert_eq!(without_part(".part"), None, "no name before the suffix");
        assert_eq!(without_part("Show.mkv.PART"), None);
        assert_eq!(without_part("Show.mkv.Part"), None);
        assert_eq!(without_part("Show.mkv"), None);
        assert_eq!(without_part("Show.part.mkv"), None);
        assert_eq!(without_part("Show.part1.rar"), None);
    }

    fn put(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    fn text(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn a_rename_to_a_free_name_moves_the_file_and_the_folder_whole() {
        let dir = tempfile::tempdir().unwrap();
        put(&dir.path().join("a"), "file");
        rename_noreplace(&dir.path().join("a"), &dir.path().join("b")).unwrap();
        assert!(!occupied(&dir.path().join("a")).unwrap());
        assert_eq!(text(&dir.path().join("b")), "file");

        std::fs::create_dir(dir.path().join("d")).unwrap();
        put(&dir.path().join("d/x"), "inside");
        rename_noreplace(&dir.path().join("d"), &dir.path().join("e")).unwrap();
        assert_eq!(text(&dir.path().join("e/x")), "inside");
    }

    #[test]
    fn a_rename_never_replaces_whatever_is_at_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let at = |name: &str| dir.path().join(name);
        put(&at("file"), "mine");
        put(&at("other"), "theirs");
        std::fs::create_dir(at("folder")).unwrap();
        put(&at("folder/x"), "inside");
        std::fs::create_dir(at("empty")).unwrap();
        std::os::unix::fs::symlink(at("gone"), at("link")).unwrap();

        for (from, to) in [
            ("file", "other"),
            ("file", "folder"),
            ("file", "empty"),
            ("file", "link"),
            ("folder", "other"),
            ("empty", "folder"),
            ("empty", "empty2"),
        ] {
            if to == "empty2" {
                std::fs::create_dir(at("empty2")).unwrap();
            }
            let err = rename_noreplace(&at(from), &at(to)).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{from} onto {to}");
        }
        assert_eq!(text(&at("file")), "mine");
        assert_eq!(text(&at("other")), "theirs");
        assert_eq!(text(&at("folder/x")), "inside");
        assert!(std::fs::read_dir(at("empty")).unwrap().next().is_none());
        assert!(std::fs::symlink_metadata(at("link")).unwrap().is_symlink());
    }

    #[test]
    fn a_rename_of_nothing_is_not_found_and_into_a_missing_folder_too() {
        let dir = tempfile::tempdir().unwrap();
        put(&dir.path().join("a"), "x");
        let err = rename_noreplace(&dir.path().join("none"), &dir.path().join("b")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        let err = rename_noreplace(&dir.path().join("a"), &dir.path().join("no/b")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(text(&dir.path().join("a")), "x");
    }

    #[test]
    fn a_rename_between_open_folders_names_the_files_in_them() {
        use rustix::fs::{open, Mode, OFlags};
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("from")).unwrap();
        std::fs::create_dir(dir.path().join("to")).unwrap();
        put(&dir.path().join("from/a"), "one");
        put(&dir.path().join("to/b"), "two");
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
        let from = open(dir.path().join("from"), flags, Mode::empty()).unwrap();
        let to = open(dir.path().join("to"), flags, Mode::empty()).unwrap();
        let err = rename_noreplace_at(&from, "a", &to, "b").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        rename_noreplace_at(&from, "a", &to, "c").unwrap();
        assert_eq!(text(&dir.path().join("to/c")), "one");
        assert_eq!(text(&dir.path().join("to/b")), "two");
        assert!(!occupied(&dir.path().join("from/a")).unwrap());
    }

    #[test]
    fn only_the_two_errors_of_a_missing_flag_say_it_is_unsupported() {
        for kind in [io::ErrorKind::InvalidInput, io::ErrorKind::Unsupported] {
            assert!(noreplace_unsupported(&io::Error::from(kind)));
        }
        for kind in [
            io::ErrorKind::AlreadyExists,
            io::ErrorKind::NotFound,
            io::ErrorKind::CrossesDevices,
            io::ErrorKind::PermissionDenied,
        ] {
            assert!(!noreplace_unsupported(&io::Error::from(kind)), "{kind:?}");
        }
    }

    #[test]
    fn a_folder_sync_reaches_the_folder_and_fails_on_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        sync_dir(dir.path()).unwrap();
        assert_eq!(testing::syncs_of(dir.path()), 1);
        let err = sync_dir(&dir.path().join("none")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(testing::syncs_of(&dir.path().join("none")), 0);
    }

    #[test]
    fn a_folder_open_as_a_descriptor_is_synced_too() {
        use rustix::fs::{open, Mode, OFlags};
        let dir = tempfile::tempdir().unwrap();
        let fd = open(
            dir.path(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        sync_dir_fd(&fd).unwrap();
        assert_eq!(testing::syncs_of(dir.path()), 1);
    }

    #[test]
    fn missing_folders_are_made_and_the_folder_each_is_in_is_synced() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("a")).unwrap();
        create_dir_all_synced(&root.join("a/b/c")).unwrap();
        assert!(root.join("a/b/c").is_dir());
        // `a` was there; `b` is in `a` and `c` in `b`.
        assert_eq!(testing::syncs_of(root), 0);
        assert_eq!(testing::syncs_of(&root.join("a")), 1);
        assert_eq!(testing::syncs_of(&root.join("a/b")), 1);
        assert_eq!(testing::syncs_of(&root.join("a/b/c")), 0);

        // Folders that are there are neither made nor synced again.
        create_dir_all_synced(&root.join("a/b/c")).unwrap();
        assert_eq!(testing::syncs_of(&root.join("a")), 1);
        assert_eq!(testing::syncs_of(&root.join("a/b")), 1);
    }

    #[test]
    fn a_folder_that_cannot_be_made_is_the_error_and_a_file_in_its_way_is_one() {
        let dir = tempfile::tempdir().unwrap();
        put(&dir.path().join("file"), "x");
        assert!(create_dir_all_synced(&dir.path().join("file/under")).is_err());
        assert_eq!(text(&dir.path().join("file")), "x");
    }

    #[test]
    fn a_synced_rename_moves_the_file_and_syncs_both_folders() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("from"), dir.path().join("to"));
        std::fs::create_dir(&from).unwrap();
        std::fs::create_dir(&to).unwrap();
        put(&from.join("a"), "bytes");
        rename_noreplace_synced(&from.join("a"), &to.join("b")).unwrap();
        assert_eq!(text(&to.join("b")), "bytes");
        assert!(!occupied(&from.join("a")).unwrap());
        assert_eq!(testing::syncs_of(&from), 1);
        assert_eq!(testing::syncs_of(&to), 1);
    }

    #[test]
    fn a_synced_rename_onto_a_taken_name_moves_and_syncs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("from"), dir.path().join("to"));
        std::fs::create_dir(&from).unwrap();
        std::fs::create_dir(&to).unwrap();
        put(&from.join("a"), "mine");
        put(&to.join("b"), "theirs");
        let err = rename_noreplace_synced(&from.join("a"), &to.join("b")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(text(&from.join("a")), "mine");
        assert_eq!(text(&to.join("b")), "theirs");
        assert_eq!(testing::syncs_of(&from), 0);
        assert_eq!(testing::syncs_of(&to), 0);
    }

    #[test]
    fn a_synced_rename_whose_sync_fails_is_an_error_though_the_file_moved() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("from"), dir.path().join("to"));
        std::fs::create_dir(&from).unwrap();
        std::fs::create_dir(&to).unwrap();
        put(&from.join("a"), "bytes");
        let _failing = testing::fail_syncs_of(&from);
        assert!(rename_noreplace_synced(&from.join("a"), &to.join("b")).is_err());
        assert_eq!(text(&to.join("b")), "bytes");
        // The folder the file went to was synced before the failing one.
        assert_eq!(testing::syncs_of(&to), 1);
    }

    #[test]
    fn a_new_file_is_written_and_synced_and_the_filler_tells_its_result() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new");
        let size = write_new(&path, None, |file| {
            use std::io::Write;
            file.write_all(b"bytes")?;
            Ok(5)
        })
        .unwrap();
        assert_eq!(size, 5);
        assert_eq!(text(&path), "bytes");
        assert_eq!(testing::syncs_of(&path), 1);
    }

    #[test]
    fn a_new_file_never_takes_the_place_of_what_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let (file, link) = (dir.path().join("file"), dir.path().join("link"));
        put(&file, "theirs");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        for path in [&file, &link, dir.path()] {
            let mut filled = false;
            let err = write_new(path, None, |_| {
                filled = true;
                Ok(())
            })
            .unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{path:?}");
            assert!(!filled, "{path:?}");
        }
        assert_eq!(text(&file), "theirs");
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(testing::syncs_of(&file), 0);
    }

    #[test]
    fn a_new_file_that_could_not_be_filled_or_synced_is_not_left() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("broken");
        let err = write_new(&broken, None, |file| {
            use std::io::Write;
            file.write_all(b"half")?;
            Err::<(), _>(io::Error::other("the source failed"))
        })
        .unwrap_err();
        assert_eq!(err.to_string(), "the source failed");
        assert!(!occupied(&broken).unwrap());

        let unsynced = dir.path().join("unsynced");
        let _failing = testing::fail_syncs_of(&unsynced);
        assert!(write_new(&unsynced, None, |_| Ok(())).is_err());
        assert!(!occupied(&unsynced).unwrap());
    }

    #[test]
    fn a_new_file_gets_the_mode_asked_for_whatever_the_umask() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private");
        write_new(&path, Some(0o600), |_| Ok(())).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
