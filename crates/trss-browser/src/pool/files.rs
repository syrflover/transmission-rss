//! Moving a finished download out of the shared downloads folder.

use std::{
    fs::File,
    io,
    os::fd::OwnedFd,
    path::{Path, PathBuf},
};

use rustix::{
    fs::{fstat, linkat, open, openat, statat, unlinkat, AtFlags, FileType, Mode, OFlags, Stat},
    io::Errno,
};
use trss_core::files::{noreplace_unsupported, rename_noreplace_at};

use super::BrowserError;

/// The longest file name kept, in bytes.
const MAX_NAME_BYTES: usize = 200;

/// A file name safe to write in a folder of ours from the name a site
/// suggested: its last path component, without control characters or
/// surrounding dots and spaces.
pub fn safe_file_name(suggested: &str) -> String {
    let last = suggested.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last.chars().filter(|c| !c.is_control()).collect();
    let mut name = cleaned
        .trim_matches(|c: char| c == '.' || c.is_whitespace())
        .to_owned();
    if name.len() > MAX_NAME_BYTES {
        let mut end = MAX_NAME_BYTES;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    if name.is_empty() {
        "download".to_owned()
    } else {
        name
    }
}

/// A file moved into a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedFile {
    /// The name it has in the folder.
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
}

/// Whether `guid` is what Chromium names a download: a UUID with hyphens. A
/// name the browser makes up is never used as a part of a path otherwise.
pub fn is_download_guid(guid: &str) -> bool {
    guid.len() == 36 && uuid::Uuid::parse_str(guid).is_ok()
}

fn unsafe_download(why: &'static str) -> BrowserError {
    BrowserError::UnsafeDownload(why)
}

/// Whether `stat` is a regular file with one name: a link to a file somewhere
/// else is neither.
fn check_plain(stat: &Stat) -> Result<(), BrowserError> {
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(unsafe_download("it is not a regular file"));
    }
    if stat.st_nlink != 1 {
        return Err(unsafe_download("it is a link to another file"));
    }
    Ok(())
}

/// Moves the download `guid` of the folder `run_dir` into `dir` as `name`,
/// which must not exist there: a file already there is never replaced.
/// Blocking.
///
/// The run's folder is writable by the browser, which can be compromised (it
/// runs without its own sandbox), so nothing in it is trusted: the folder is
/// opened without following a link and must be a folder, the file is looked
/// at without following links and must be a regular file with a single name,
/// and the move is done relative to the folder that was opened. The file that
/// arrives in `dir` is looked at again, since the browser could have changed
/// the name between the look and the move.
pub(super) fn move_into(
    run_dir: &Path,
    guid: &str,
    dir: &Path,
    name: &str,
) -> Result<MovedFile, BrowserError> {
    if !is_download_guid(guid) {
        return Err(unsafe_download("its name is not a download's"));
    }
    let run_fd = open(
        run_dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| unsafe_download("the run's downloads folder is not a plain folder"))?;
    let stat = statat(&run_fd, guid, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from)?;
    check_plain(&stat)?;

    std::fs::create_dir_all(dir)?;
    let dir_fd = open(
        dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io::Error::from)?;

    match rename_noreplace_at(&run_fd, guid, &dir_fd, name) {
        Ok(()) => {}
        Err(err) => match err.kind() {
            io::ErrorKind::AlreadyExists => {
                return Err(BrowserError::AlreadyExists(name.to_owned()))
            }
            io::ErrorKind::CrossesDevices => copy_across(&run_fd, guid, &dir_fd, name)?,
            // A filesystem without `RENAME_NOREPLACE`: a link fails on a name
            // that exists too.
            _ if noreplace_unsupported(&err) => {
                match linkat(&run_fd, guid, &dir_fd, name, AtFlags::empty()) {
                    Ok(()) => forget(&run_fd, guid)?,
                    Err(errno) if errno == Errno::EXIST => {
                        return Err(BrowserError::AlreadyExists(name.to_owned()))
                    }
                    Err(_) => copy_across(&run_fd, guid, &dir_fd, name)?,
                }
            }
            _ => return Err(err.into()),
        },
    }

    let placed = statat(&dir_fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from)?;
    if let Err(why) = check_plain(&placed) {
        // Not what was looked at: whatever came is not kept.
        if FileType::from_raw_mode(placed.st_mode) == FileType::Directory {
            let _ = std::fs::remove_dir_all(dir.join(name));
        } else {
            let _ = unlinkat(&dir_fd, name, AtFlags::empty());
        }
        return Err(why);
    }
    Ok(MovedFile {
        name: name.to_owned(),
        path: dir.join(name),
        size: placed.st_size as u64,
    })
}

/// Removes the download once it is placed. A download already gone (the run
/// ended meanwhile and its folder was cleaned) is not a failure: the file is
/// where it was wanted.
fn forget(run_fd: &OwnedFd, guid: &str) -> Result<(), BrowserError> {
    match unlinkat(run_fd, guid, AtFlags::empty()) {
        Ok(()) => Ok(()),
        Err(errno) if errno == Errno::NOENT => Ok(()),
        Err(errno) => Err(io::Error::from(errno).into()),
    }
}

/// Copies across filesystems: the file opened without following a link, into
/// a temporary name beside the target, then into place without replacing
/// anything.
fn copy_across(
    run_fd: &OwnedFd,
    guid: &str,
    dir_fd: &OwnedFd,
    name: &str,
) -> Result<(), BrowserError> {
    let source = openat(
        run_fd,
        guid,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io::Error::from)?;
    check_plain(&fstat(&source).map_err(io::Error::from)?)?;

    let partial = format!(".{}.part", uuid::Uuid::new_v4().simple());
    let target = openat(
        dir_fd,
        &partial,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o644),
    )
    .map_err(io::Error::from)?;
    let copied = io::copy(&mut File::from(source), &mut File::from(target));
    if let Err(err) = copied {
        let _ = unlinkat(dir_fd, &partial, AtFlags::empty());
        return Err(err.into());
    }
    let placed = rename_noreplace_at(dir_fd, &partial, dir_fd, name);
    if let Err(err) = placed {
        let _ = unlinkat(dir_fd, &partial, AtFlags::empty());
        return Err(if err.kind() == io::ErrorKind::AlreadyExists {
            BrowserError::AlreadyExists(name.to_owned())
        } else {
            err.into()
        });
    }
    forget(run_fd, guid)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    const GUID: &str = "0f6e2b0e-4a52-4d3c-9d3e-2f1c7a5b8e90";

    #[test]
    fn a_suggested_name_is_made_safe() {
        assert_eq!(safe_file_name("자막 - 01.zip"), "자막 - 01.zip");
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name("C:\\Users\\x\\a.srt"), "a.srt");
        assert_eq!(safe_file_name("..hidden. "), "hidden");
        assert_eq!(safe_file_name("a\u{0}b\n.ass"), "ab.ass");
        assert_eq!(safe_file_name(""), "download");
        assert_eq!(safe_file_name(".."), "download");
        assert_eq!(safe_file_name("a/"), "download");
        let long = "가".repeat(100);
        let name = safe_file_name(&long);
        assert!(name.len() <= 200 && name.chars().all(|c| c == '가'));
    }

    #[test]
    fn only_a_uuid_is_a_download_name() {
        assert!(is_download_guid(GUID));
        assert!(is_download_guid(&GUID.to_uppercase()));
        for bad in [
            "",
            "..",
            ".",
            "g1",
            "../0f6e2b0e-4a52-4d3c-9d3e-2f1c7a5b8e9",
            "0f6e2b0e4a524d3c9d3e2f1c7a5b8e90",
            "{0f6e2b0e-4a52-4d3c-9d3e-2f1c7a5b8e90}",
            "0f6e2b0e-4a52-4d3c-9d3e-2f1c7a5b8e9/",
        ] {
            assert!(!is_download_guid(bad), "{bad:?}");
        }
    }

    fn folder() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let run = root.path().join("run");
        std::fs::create_dir(&run).unwrap();
        (root, run)
    }

    #[test]
    fn a_file_moves_and_nothing_is_replaced() {
        let (root, run) = folder();
        let from = run.join(GUID);
        std::fs::write(&from, b"12345").unwrap();
        let dest = root.path().join("receive/job");

        let moved = move_into(&run, GUID, &dest, "a.zip").unwrap();
        assert_eq!(moved.name, "a.zip");
        assert_eq!(moved.size, 5);
        assert_eq!(moved.path, dest.join("a.zip"));
        assert_eq!(std::fs::read(&moved.path).unwrap(), b"12345");
        assert!(!from.exists());

        std::fs::write(&from, b"other").unwrap();
        let again = move_into(&run, GUID, &dest, "a.zip");
        assert!(matches!(again, Err(BrowserError::AlreadyExists(ref n)) if n == "a.zip"));
        assert_eq!(std::fs::read(dest.join("a.zip")).unwrap(), b"12345");
        assert!(from.exists(), "the download stays when the move is refused");
    }

    #[test]
    fn a_guid_that_is_not_a_uuid_is_refused() {
        let (root, run) = folder();
        std::fs::write(root.path().join("secret"), b"s").unwrap();
        let dest = root.path().join("out");
        for guid in ["..", "../secret", "g1", ""] {
            let refused = move_into(&run, guid, &dest, "a");
            assert!(
                matches!(refused, Err(BrowserError::UnsafeDownload(_))),
                "{guid:?}: {refused:?}"
            );
        }
        assert!(root.path().join("secret").exists());
        assert!(!dest.join("a").exists());
    }

    #[test]
    fn a_symlink_for_the_file_is_refused_and_its_target_left_alone() {
        let (root, run) = folder();
        let secret = root.path().join("trss.db");
        std::fs::write(&secret, b"the database").unwrap();
        symlink(&secret, run.join(GUID)).unwrap();
        let dest = root.path().join("out");

        let refused = move_into(&run, GUID, &dest, "a.zip");
        assert!(
            matches!(refused, Err(BrowserError::UnsafeDownload(_))),
            "{refused:?}"
        );
        assert_eq!(std::fs::read(&secret).unwrap(), b"the database");
        assert!(!dest.join("a.zip").exists());
    }

    #[test]
    fn a_symlink_for_the_run_folder_is_refused() {
        let (root, run) = folder();
        let elsewhere = root.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::write(elsewhere.join(GUID), b"somebody's file").unwrap();
        std::fs::remove_dir(&run).unwrap();
        symlink(&elsewhere, &run).unwrap();
        let dest = root.path().join("out");

        let refused = move_into(&run, GUID, &dest, "a.zip");
        assert!(
            matches!(refused, Err(BrowserError::UnsafeDownload(_))),
            "{refused:?}"
        );
        assert!(elsewhere.join(GUID).exists());
        assert!(!dest.join("a.zip").exists());
    }

    #[test]
    fn a_hard_link_to_another_file_is_refused() {
        let (root, run) = folder();
        let other = root.path().join("other");
        std::fs::write(&other, b"someone else's").unwrap();
        std::fs::hard_link(&other, run.join(GUID)).unwrap();
        let dest = root.path().join("out");

        let refused = move_into(&run, GUID, &dest, "a.zip");
        assert!(
            matches!(refused, Err(BrowserError::UnsafeDownload(_))),
            "{refused:?}"
        );
        assert_eq!(std::fs::read(&other).unwrap(), b"someone else's");
        assert!(!dest.join("a.zip").exists());
    }

    #[test]
    fn a_folder_for_the_file_is_refused() {
        let (root, run) = folder();
        std::fs::create_dir(run.join(GUID)).unwrap();
        let dest = root.path().join("out");
        let refused = move_into(&run, GUID, &dest, "a.zip");
        assert!(
            matches!(refused, Err(BrowserError::UnsafeDownload(_))),
            "{refused:?}"
        );
        assert!(!dest.join("a.zip").exists());
    }

    #[test]
    fn a_copy_across_filesystems_places_the_file_and_forgives_a_download_that_went() {
        let (root, run) = folder();
        let dest = root.path().join("out");
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(run.join(GUID), b"copied").unwrap();
        let run_fd = open(
            &run,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        let dest_fd = open(
            &dest,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();

        copy_across(&run_fd, GUID, &dest_fd, "a.zip").unwrap();
        assert_eq!(std::fs::read(dest.join("a.zip")).unwrap(), b"copied");
        assert!(!run.join(GUID).exists());
        assert_eq!(
            std::fs::read_dir(&dest).unwrap().count(),
            1,
            "no .part left"
        );

        // The source is gone by the time it is removed (the run ended and its
        // folder was emptied while the copy ran): the file was still placed.
        assert!(forget(&run_fd, GUID).is_ok());

        // A name taken is refused, and nothing is left behind.
        std::fs::write(run.join(GUID), b"second").unwrap();
        let taken = copy_across(&run_fd, GUID, &dest_fd, "a.zip");
        assert!(
            matches!(taken, Err(BrowserError::AlreadyExists(_))),
            "{taken:?}"
        );
        assert_eq!(std::fs::read(dest.join("a.zip")).unwrap(), b"copied");
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 1);
        assert!(run.join(GUID).exists());

        // A link is not copied from.
        std::fs::remove_file(run.join(GUID)).unwrap();
        symlink(root.path().join("nowhere"), run.join(GUID)).unwrap();
        let linked = copy_across(&run_fd, GUID, &dest_fd, "b.zip");
        assert!(linked.is_err());
        assert!(!dest.join("b.zip").exists());
    }
}
