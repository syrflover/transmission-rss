//! The start-up check that this process can write the app data folder.
//!
//! The web and the worker run as one user and share the database (WAL and
//! shared-memory files included), the lock files, the wake socket and the
//! receive area. A file an earlier deploy made as another user (root, before
//! the image ran as 1000:1000) is not writable here, and without this check it
//! would fail later and in pieces: a lock that cannot be opened, a database
//! that opens read-only. [`check_writable`] looks once at what trss writes in
//! the folder and names what cannot be written and who is running, so the
//! start stops with the path and the uid that is needed. Anything else in the
//! folder (`lost+found` of a disk of its own, a copy of the database made for
//! a backup) is not trss's and is left alone.

use std::{
    fmt, fs, io,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
};

use rustix::{
    fs::{access, Access},
    process::{getegid, geteuid},
};

use crate::app_data::{ARTWORK_DIR, RECEIVE_DIR, SUBTITLE_FILES_DIR};

/// How many unwritable paths the message names; the rest is counted.
const NAMED: usize = 5;

/// A path this process cannot write, and who owns it.
#[derive(Debug)]
pub struct Unwritable {
    pub path: PathBuf,
    /// Owner uid, owner gid and permission bits; `None` when the path could
    /// not even be looked at.
    pub owner: Option<(u32, u32, u32)>,
    pub error: io::Error,
}

/// What [`check_writable`] found.
#[derive(Debug)]
pub struct AccessError {
    pub root: PathBuf,
    /// The uid and gid this process runs as.
    pub uid: u32,
    pub gid: u32,
    /// The first few paths.
    pub named: Vec<Unwritable>,
    /// All of them, `named` included.
    pub total: usize,
}

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot write the app data folder {} as uid {} gid {}",
            self.root.display(),
            self.uid,
            self.gid
        )?;
        for item in &self.named {
            write!(f, "\n  {}: ", item.path.display())?;
            match item.owner {
                Some((uid, gid, mode)) => write!(
                    f,
                    "owned by uid {uid} gid {gid}, mode {:04o} ({})",
                    mode & 0o7777,
                    item.error
                )?,
                None => write!(f, "{}", item.error)?,
            }
        }
        if self.total > self.named.len() {
            write!(f, "\n  and {} more", self.total - self.named.len())?;
        }
        write!(
            f,
            "\nGive these files to uid {} gid {} (for example `chown -R {}:{}` on the folder \
             that holds them), or run this process as their owner.",
            self.uid, self.gid, self.uid, self.gid
        )
    }
}

impl std::error::Error for AccessError {}

/// What trss writes in the app data folder next to the database file, as
/// suffixes of the database's file name: the database's WAL, shared-memory
/// and journal files, the lock files (`lock_path_for` of the worker, the
/// browser, Anissia, its captions, the artwork and the seasons) and the wake
/// socket (`wake_path_for`). The test `app_data_files_cover_every_lock_and_the_wake_socket` in
/// the worker's tests keeps this in step with those functions.
pub const DATABASE_FILE_SUFFIXES: &[&str] = &[
    "",
    "-wal",
    "-shm",
    "-journal",
    ".worker.lock",
    ".browser.lock",
    ".anissia.lock",
    ".anissia-captions.lock",
    ".artwork.lock",
    ".seasons.lock",
    ".wake",
];

/// The folders in the app data folder that trss writes into, with their
/// subfolders: the receive area, the work covers (and their staging) and the
/// subtitle packages' attachments and companion files.
pub const APP_DATA_FOLDERS: &[&str] = &[RECEIVE_DIR, ARTWORK_DIR, SUBTITLE_FILES_DIR];

/// [`check_writable`] for the app data folder of the database at `db_path`
/// (its folder), with the files [`DATABASE_FILE_SUFFIXES`] names and the
/// folders [`APP_DATA_FOLDERS`]. A bare file name is a run in the current
/// folder, for development, and is not checked.
pub fn check_app_data(db_path: &Path) -> Result<(), AccessError> {
    let (Some(root), Some(name)) = (
        db_path.parent().filter(|p| !p.as_os_str().is_empty()),
        db_path.file_name(),
    ) else {
        return Ok(());
    };
    let files: Vec<PathBuf> = DATABASE_FILE_SUFFIXES
        .iter()
        .map(|suffix| {
            let mut file = name.to_owned();
            file.push(suffix);
            root.join(file)
        })
        .collect();
    let folders: Vec<PathBuf> = APP_DATA_FOLDERS.iter().map(|f| root.join(f)).collect();
    check_writable(root, &files, &folders)
}

/// Checks that this process can write what it needs in the folder `root`:
/// the folder itself (read, write and search, to add and remove what it
/// holds), each of `files` that exists (read and write), and each of `folders`
/// that exists with the folders under it (read, write and search). Nothing
/// else in `root` is looked at, and the files inside the folders are not
/// checked: whoever may write their folder may remove them, and some are
/// another user's on purpose (a download of the browser container, moved into
/// the receive area with its owner, uid 10001).
///
/// `root` may be reached through a symbolic link; a link inside it is not
/// followed. `Err` is the start-up refusal: it names the first few paths with
/// their owner and the uid this process runs as.
pub fn check_writable(
    root: &Path,
    files: &[PathBuf],
    folders: &[PathBuf],
) -> Result<(), AccessError> {
    let mut found = Found::default();
    match fs::metadata(root) {
        Ok(meta) if meta.is_dir() => {
            check_one(
                root,
                &meta,
                Access::WRITE_OK | Access::EXEC_OK | Access::READ_OK,
                &mut found,
            );
            for file in files {
                match fs::symlink_metadata(file) {
                    Ok(meta) if meta.is_file() || meta.file_type().is_socket() => {
                        check_one(file, &meta, Access::WRITE_OK | Access::READ_OK, &mut found)
                    }
                    // Missing, or something else: not a file trss made.
                    _ => {}
                }
            }
            for folder in folders {
                walk(folder, &mut found);
            }
        }
        Ok(meta) => found.add(root, Some(&meta), io::Error::other("not a folder")),
        Err(error) => found.add(root, None, error),
    }
    if found.total == 0 {
        return Ok(());
    }
    Err(AccessError {
        root: root.to_owned(),
        uid: geteuid().as_raw(),
        gid: getegid().as_raw(),
        named: found.named,
        total: found.total,
    })
}

fn check_one(path: &Path, meta: &fs::Metadata, wanted: Access, found: &mut Found) {
    if let Err(errno) = access(path, wanted) {
        found.add(path, Some(meta), errno.into());
    }
}

#[derive(Default)]
struct Found {
    named: Vec<Unwritable>,
    total: usize,
}

impl Found {
    fn add(&mut self, path: &Path, meta: Option<&fs::Metadata>, error: io::Error) {
        self.total += 1;
        if self.named.len() < NAMED {
            self.named.push(Unwritable {
                path: path.to_owned(),
                owner: meta.map(|m| (m.uid(), m.gid(), m.mode())),
                error,
            });
        }
    }
}

/// Checks the folder `top` and the folders under it; a missing one, a link
/// and a file are skipped.
fn walk(top: &Path, found: &mut Found) {
    let mut pending = Vec::new();
    match fs::symlink_metadata(top) {
        Ok(meta) if meta.is_dir() => pending.push(top.to_owned()),
        _ => return,
    }
    while let Some(dir) = pending.pop() {
        let meta = match fs::symlink_metadata(&dir) {
            Ok(meta) => meta,
            Err(error) => {
                found.add(&dir, None, error);
                continue;
            }
        };
        if let Err(errno) = access(&dir, Access::WRITE_OK | Access::EXEC_OK | Access::READ_OK) {
            found.add(&dir, Some(&meta), errno.into());
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) => {
                found.add(&dir, Some(&meta), error);
                continue;
            }
        };
        for entry in entries.flatten() {
            // Gone since the listing: nothing to write. A link is not followed.
            if let Ok(meta) = fs::symlink_metadata(entry.path()) {
                if meta.is_dir() {
                    pending.push(entry.path());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    /// A user with root's powers writes anything, so the refusals cannot be
    /// seen as that user.
    fn is_root() -> bool {
        geteuid().is_root()
    }

    fn db_in(dir: &Path) -> PathBuf {
        dir.join("trss.db")
    }

    #[test]
    fn a_folder_this_process_owns_passes_with_its_files_and_subfolders() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("trss.db"), "").unwrap();
        fs::create_dir_all(dir.path().join("receive/job")).unwrap();
        fs::write(dir.path().join("receive/job/a.ass"), "").unwrap();
        check_app_data(&db_in(dir.path())).unwrap();
    }

    #[test]
    fn a_database_file_that_cannot_be_written_is_named_with_its_owner_and_the_needed_uid() {
        if is_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("trss.db.worker.lock");
        fs::write(&lock, "").unwrap();
        mode(&lock, 0o444);

        let err = check_app_data(&db_in(dir.path())).unwrap_err();

        assert_eq!(err.total, 1);
        assert_eq!(err.named[0].path, lock);
        let (uid, gid, mode) = err.named[0].owner.unwrap();
        assert_eq!((uid, gid), (geteuid().as_raw(), getegid().as_raw()));
        assert_eq!(mode & 0o7777, 0o444);
        let message = err.to_string();
        assert!(message.contains(&lock.display().to_string()), "{message}");
        assert!(
            message.contains(&format!("uid {}", geteuid().as_raw())),
            "{message}"
        );
        assert!(message.contains("chown -R"), "{message}");
    }

    #[test]
    fn every_file_trss_writes_next_to_the_database_is_checked() {
        if is_root() {
            return;
        }
        for suffix in DATABASE_FILE_SUFFIXES {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join(format!("trss.db{suffix}"));
            fs::write(&file, "").unwrap();
            mode(&file, 0o444);

            let err = check_app_data(&db_in(dir.path())).unwrap_err();

            assert_eq!(err.named[0].path, file, "suffix {suffix:?}");
        }
    }

    #[test]
    fn a_subfolder_that_cannot_be_written_is_named_and_not_entered() {
        if is_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let receive = dir.path().join("receive");
        fs::create_dir_all(receive.join("job")).unwrap();
        mode(&receive, 0o555);

        let result = check_app_data(&db_in(dir.path()));
        mode(&receive, 0o755);

        let err = result.unwrap_err();
        assert_eq!(err.total, 1);
        assert_eq!(err.named[0].path, receive);
    }

    #[test]
    fn the_staging_folder_in_the_artwork_folder_is_checked() {
        if is_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("artwork/.staging");
        fs::create_dir_all(&staging).unwrap();
        mode(&staging, 0o555);

        let result = check_app_data(&db_in(dir.path()));
        mode(&staging, 0o755);

        assert_eq!(result.unwrap_err().named[0].path, staging);
    }

    #[test]
    fn what_trss_does_not_write_is_left_alone() {
        if is_root() {
            return;
        }
        // The folders and files of the disk or the admin, not of trss: a
        // `lost+found` of a disk of its own (root's, closed), a copy of the
        // database made for a backup, and the browser's downloads folder.
        let dir = tempfile::tempdir().unwrap();
        let lost = dir.path().join("lost+found");
        fs::create_dir_all(lost.join("inner")).unwrap();
        mode(&lost, 0o000);
        let copy = dir.path().join("copy.db");
        fs::write(&copy, "").unwrap();
        mode(&copy, 0o444);
        let downloads = dir.path().join("browser-downloads");
        fs::create_dir(&downloads).unwrap();
        fs::write(downloads.join("file"), "").unwrap();
        mode(&downloads.join("file"), 0o444);
        mode(&downloads, 0o555);

        let result = check_app_data(&db_in(dir.path()));
        mode(&lost, 0o755);
        mode(&downloads, 0o755);

        result.unwrap();
    }

    #[test]
    fn only_the_first_few_paths_are_named_and_all_are_counted() {
        if is_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut folders = Vec::new();
        for n in 0..NAMED + 3 {
            let folder = dir.path().join(format!("f{n}"));
            fs::create_dir(&folder).unwrap();
            mode(&folder, 0o555);
            folders.push(folder);
        }

        let result = check_writable(dir.path(), &[], &folders);
        for folder in &folders {
            mode(folder, 0o755);
        }

        let err = result.unwrap_err();
        assert_eq!(err.total, NAMED + 3);
        assert_eq!(err.named.len(), NAMED);
        assert!(err.to_string().contains("and 3 more"), "{err}");
    }

    #[test]
    fn a_file_deeper_down_that_is_another_users_does_not_stop_the_start() {
        if is_root() {
            return;
        }
        // What the browser container downloaded and the worker moved into the
        // receive area keeps its owner (uid 10001); a folder's owner removes it.
        let dir = tempfile::tempdir().unwrap();
        let job = dir.path().join("receive/job");
        fs::create_dir_all(&job).unwrap();
        fs::write(job.join("a.srt"), "").unwrap();
        mode(&job.join("a.srt"), 0o444);

        check_app_data(&db_in(dir.path())).unwrap();
    }

    #[test]
    fn a_missing_folder_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        let err = check_app_data(&missing.join("trss.db")).unwrap_err();
        assert_eq!(err.total, 1);
        assert!(err.named[0].owner.is_none());
    }

    #[test]
    fn a_bare_database_name_is_not_checked() {
        check_app_data(Path::new("trss.db")).unwrap();
    }

    #[test]
    fn a_data_folder_reached_through_a_link_is_checked_not_refused() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        fs::write(real.join("trss.db"), "").unwrap();

        check_app_data(&link.join("trss.db")).unwrap();
    }

    #[test]
    fn a_link_inside_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/proc/1/mem", dir.path().join("trss.db")).unwrap();
        std::os::unix::fs::symlink("/proc/1", dir.path().join("receive")).unwrap();
        check_app_data(&db_in(dir.path())).unwrap();
    }
}
