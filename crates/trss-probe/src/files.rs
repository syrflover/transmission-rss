//! The file checks: what archiving and placing a subtitle rely on.

use std::{
    fs::{self, File},
    io::{self, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use rustix::{
    fs::{major, minor, renameat_with, RenameFlags, CWD},
    io::Errno,
};

use crate::{
    mountinfo::{self, Mount},
    report::Report,
};

/// What every folder and file the probe makes is named from, so that a
/// leftover is plain to see and nothing else is ever removed.
pub const PREFIX: &str = ".trss-probe-";

/// The owner trss's files are expected to have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    pub uid: u32,
    pub gid: u32,
}

/// A name no other run of the probe has.
pub fn unique_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!("{PREFIX}{}-{:x}", std::process::id(), nanos)
}

/// What the probe made, removed at the end (and by [`Drop`] if the end is
/// never reached). Only paths named by [`PREFIX`] are ever tracked.
#[derive(Default)]
pub struct Cleanup(Vec<PathBuf>);

impl Cleanup {
    pub fn track(&mut self, path: &Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        assert!(
            name.is_some_and(|n| n.starts_with(PREFIX)),
            "the probe removes only what it named itself: {}",
            path.display()
        );
        self.0.push(path.to_owned());
    }

    /// Removes everything tracked, newest first, and reports each.
    pub fn finish(&mut self, r: &mut Report) {
        while let Some(path) = self.0.pop() {
            let mut existed = true;
            let removed = match fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_dir() => fs::remove_dir_all(&path),
                Ok(_) => fs::remove_file(&path),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    existed = false;
                    Ok(())
                }
                Err(e) => Err(e),
            };
            let gone = !path.exists();
            let detail = match &removed {
                Ok(()) if existed => format!("removed {}", path.display()),
                Ok(()) => format!("{} was not made or is already gone", path.display()),
                Err(e) => format!("cannot remove {}: {e}", path.display()),
            };
            r.check(removed.is_ok() && gone, "cleanup", detail);
        }
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        while let Some(path) = self.0.pop() {
            let _ = fs::remove_dir_all(&path).or_else(|_| fs::remove_file(&path));
        }
    }
}

fn errno_text(e: Errno) -> String {
    format!("{} (errno {})", io::Error::from(e), e.raw_os_error())
}

fn noreplace(from: &Path, to: &Path) -> Result<(), Errno> {
    renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE)
}

/// `dev:inode` of a path, links not followed.
fn identity(path: &Path) -> io::Result<(u64, u64)> {
    let meta = fs::symlink_metadata(path)?;
    Ok((meta.dev(), meta.ino()))
}

fn mode_text(meta: &fs::Metadata) -> String {
    format!("{:04o}", meta.permissions().mode() & 0o7777)
}

/// Prints where `path` is: its mount, filesystem and options.
pub fn describe(r: &mut Report, label: &str, path: &Path, mounts: &[Mount]) {
    let real = match fs::canonicalize(path) {
        Ok(real) => real,
        Err(e) => {
            r.check(
                false,
                &format!("{label} folder"),
                format!("{}: {e}", path.display()),
            );
            return;
        }
    };
    let dev = fs::metadata(&real).map(|m| m.dev()).ok();
    r.info(format!(
        "{label}: {} (real path {})",
        path.display(),
        real.display()
    ));
    if let Some(dev) = dev {
        r.info(format!("  st_dev {}:{}", major(dev), minor(dev)));
    }
    match mountinfo::holding(mounts, &real) {
        Some(m) => {
            r.info(format!(
                "  mount id {} at {} (device {})",
                m.id,
                m.point.display(),
                m.device
            ));
            r.info(format!(
                "  filesystem {} on {}, root {}",
                m.fstype, m.source, m.root
            ));
            r.info(format!("  mount options {}", m.options));
            r.info(format!("  super options {}", m.super_options));
        }
        None => r.info("  not found in /proc/self/mountinfo"),
    }
}

/// The mount a folder is on, for comparing two folders.
pub fn mount_id(mounts: &[Mount], path: &Path) -> Option<u32> {
    let real = fs::canonicalize(path).ok()?;
    mountinfo::holding(mounts, &real).map(|m| m.id)
}

/// Whether `path` belongs to `owner`, with its mode.
pub fn check_owner(r: &mut Report, name: &str, path: &Path, owner: Owner) {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            let ok = (meta.uid(), meta.gid()) == (owner.uid, owner.gid);
            r.check(
                ok,
                name,
                format!(
                    "owner {}:{} (expected {}:{}), mode {}",
                    meta.uid(),
                    meta.gid(),
                    owner.uid,
                    owner.gid,
                    mode_text(&meta)
                ),
            );
        }
        Err(e) => r.check(
            false,
            name,
            format!("cannot look at {}: {e}", path.display()),
        ),
    }
}

fn fsync_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

fn fsync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

fn result_text(result: io::Result<()>) -> (bool, String) {
    match result {
        Ok(()) => (true, "ok".to_owned()),
        Err(e) => (false, e.to_string()),
    }
}

/// The checks that need one filesystem: in `dir`, a folder this run made.
pub fn same_filesystem(r: &mut Report, label: &str, dir: &Path, owner: Owner) {
    let (a, b, c) = (dir.join("a.txt"), dir.join("b.txt"), dir.join("c.txt"));

    let (ok, text) = result_text(fsync_file(&a, b"first"));
    r.check(ok, &format!("{label}: create a file and fsync it"), text);
    if !ok {
        return;
    }
    let (ok, text) = result_text(fsync_dir(dir));
    r.check(ok, &format!("{label}: fsync the folder"), text);
    check_owner(r, &format!("{label}: owner of a new file"), &a, owner);
    check_owner(r, &format!("{label}: owner of the test folder"), dir, owner);
    let before = identity(&a);

    // A free name: the rename happens, and the file is the same one after.
    match noreplace(&a, &b) {
        Ok(()) => {
            r.check(
                true,
                &format!("{label}: renameat2(RENAME_NOREPLACE) to a free name"),
                "ok",
            );
            let after = identity(&b);
            r.check(
                before.is_ok() && before.as_ref().ok() == after.as_ref().ok(),
                &format!("{label}: dev:inode stays the same across the rename"),
                format!("{:?} -> {:?}", before.as_ref().ok(), after.as_ref().ok()),
            );
            r.check(
                !a.exists(),
                &format!("{label}: the old name is gone"),
                "checked",
            );
        }
        Err(e) => {
            let hint = if e == Errno::INVAL || e == Errno::OPNOTSUPP {
                ", the filesystem does not support RENAME_NOREPLACE"
            } else {
                ""
            };
            r.check(
                false,
                &format!("{label}: renameat2(RENAME_NOREPLACE) to a free name"),
                format!("{}{hint}", errno_text(e)),
            );
            return;
        }
    }
    let (ok, text) = result_text(fsync_dir(dir));
    r.check(
        ok,
        &format!("{label}: fsync the folder after the rename"),
        text,
    );

    // A name that is taken: refused with EEXIST, and nothing is replaced.
    let _ = fsync_file(&c, b"second");
    let refused = noreplace(&c, &b);
    r.check(
        refused == Err(Errno::EXIST),
        &format!("{label}: renameat2(RENAME_NOREPLACE) onto an existing file"),
        match refused {
            Ok(()) => "it replaced the file".to_owned(),
            Err(e) => errno_text(e),
        },
    );
    let kept = fs::read(&b).is_ok_and(|bytes| bytes == b"first") && c.exists();
    r.check(
        kept,
        &format!("{label}: the refused rename changed nothing"),
        "both files and the first contents are as they were",
    );

    // Folders: onto an existing folder, onto an existing file, and a free name.
    let (d1, d2, d3) = (dir.join("d1"), dir.join("d2"), dir.join("d3"));
    if fs::create_dir(&d1).is_ok() && fs::create_dir(&d2).is_ok() {
        let refused = noreplace(&d1, &d2);
        r.check(
            refused == Err(Errno::EXIST),
            &format!("{label}: renameat2(RENAME_NOREPLACE) of a folder onto an existing folder"),
            match refused {
                Ok(()) => "it replaced the folder".to_owned(),
                Err(e) => errno_text(e),
            },
        );
        let refused = noreplace(&d1, &b);
        r.check(
            refused == Err(Errno::EXIST),
            &format!("{label}: renameat2(RENAME_NOREPLACE) of a folder onto an existing file"),
            match refused {
                Ok(()) => "it replaced the file".to_owned(),
                Err(e) => errno_text(e),
            },
        );
        let moved = noreplace(&d1, &d3);
        r.check(
            moved.is_ok(),
            &format!("{label}: renameat2(RENAME_NOREPLACE) of a folder to a free name"),
            match moved {
                Ok(()) => "ok".to_owned(),
                Err(e) => errno_text(e),
            },
        );
        check_owner(r, &format!("{label}: owner of a new folder"), &d2, owner);
    } else {
        r.check(
            false,
            &format!("{label}: make folders"),
            "cannot make a folder in the test folder",
        );
    }

    // The staging pattern: a temporary file in a subfolder, renamed into the
    // folder above it.
    staging(r, label, dir, ".staging", "staged.txt", owner);
}

/// A file made in the subfolder `sub` of `dir` is renamed (without replacing)
/// into `dir` as `final_name`: how a file is staged in `.trss/` and placed.
/// `sub` and `final_name` are made here and removed.
pub fn staging(r: &mut Report, label: &str, dir: &Path, sub: &str, final_name: &str, owner: Owner) {
    let sub_dir = dir.join(sub);
    let temp = sub_dir.join("temp");
    let target = dir.join(final_name);
    let made = fs::create_dir(&sub_dir)
        .and_then(|()| fsync_file(&temp, b"staged"))
        .and_then(|()| fsync_dir(&sub_dir));
    let (ok, text) = result_text(made);
    r.check(
        ok,
        &format!("{label}: make a subfolder and a file in it, fsync both"),
        text,
    );
    if !ok {
        return;
    }
    check_owner(
        r,
        &format!("{label}: owner of the subfolder"),
        &sub_dir,
        owner,
    );
    check_owner(
        r,
        &format!("{label}: owner of the file in it"),
        &temp,
        owner,
    );
    let before = identity(&temp);
    let placed = noreplace(&temp, &target);
    r.check(
        placed.is_ok(),
        &format!("{label}: rename from the subfolder into the folder above"),
        match placed {
            Ok(()) => "ok".to_owned(),
            Err(e) => errno_text(e),
        },
    );
    if placed.is_ok() {
        let after = identity(&target);
        r.check(
            before.is_ok() && before.as_ref().ok() == after.as_ref().ok(),
            &format!("{label}: dev:inode stays the same across it"),
            format!("{:?} -> {:?}", before.as_ref().ok(), after.as_ref().ok()),
        );
        let (ok, text) = result_text(fsync_dir(dir));
        r.check(ok, &format!("{label}: fsync the folder above"), text);
    }
}

/// A file in `from` renamed into `to`, which are two folders the app uses
/// (the data folder and the media): it must fail with `EXDEV` where they are
/// on different mounts, which is why a file is staged next to where it ends.
pub fn across_mounts(r: &mut Report, from: &Path, to: &Path, different: Option<bool>) {
    let source = from.join("across.txt");
    let target = to.join("across.txt");
    if let Err(e) = fsync_file(&source, b"across") {
        r.check(false, "data to media: make the file", e.to_string());
        return;
    }
    let result = noreplace(&source, &target);
    match (different, result) {
        (Some(true), Err(e)) => r.check(
            e == Errno::XDEV,
            "data to media: rename between the two mounts fails with EXDEV",
            errno_text(e),
        ),
        (Some(true), Ok(())) => r.check(
            false,
            "data to media: rename between the two mounts fails with EXDEV",
            "it succeeded",
        ),
        (_, outcome) => r.info(format!(
            "data to media: {}; the two folders {} (a rename between them {})",
            match &outcome {
                Ok(()) => "the rename succeeded".to_owned(),
                Err(e) => format!("the rename failed: {}", errno_text(*e)),
            },
            match different {
                Some(false) => "are on one mount",
                _ => "may be on one mount (the mounts could not be told apart)",
            },
            "is expected to succeed there"
        )),
    }
}

/// Makes a test subfolder in an existing work folder, which stands for the
/// `.trss/` the app makes there, and stages a file through it.
pub fn work_folder(r: &mut Report, work: &Path, owner: Owner, cleanup: &mut Cleanup) {
    let label = format!("work folder {}", work.display());
    let meta = match fs::symlink_metadata(work) {
        Ok(meta) if meta.is_dir() => meta,
        Ok(_) => {
            r.check(false, &label, "is not a folder");
            return;
        }
        Err(e) => {
            r.check(false, &label, e.to_string());
            return;
        }
    };
    r.info(format!(
        "{label}: owner {}:{}, mode {}",
        meta.uid(),
        meta.gid(),
        mode_text(&meta)
    ));
    let name = unique_name();
    let sub = work.join(&name);
    let final_name = format!("{name}-final");
    // Tracked first: whatever happens next, they are removed.
    cleanup.track(&sub);
    cleanup.track(&work.join(&final_name));
    staging(r, &label, work, &name, &final_name, owner);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Owner {
        Owner {
            uid: rustix::process::getuid().as_raw(),
            gid: rustix::process::getgid().as_raw(),
        }
    }

    #[test]
    fn the_checks_pass_on_a_filesystem_that_behaves() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Report::default();
        same_filesystem(&mut r, "test", dir.path(), me());
        assert_eq!(r.failed(), 0);
    }

    #[test]
    fn a_wrong_expected_owner_fails_the_owner_checks() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Report::default();
        let other = Owner {
            uid: me().uid + 1,
            gid: me().gid + 1,
        };
        same_filesystem(&mut r, "test", dir.path(), other);
        // The new file, the test folder, a new folder, the subfolder and its file.
        assert_eq!(r.failed(), 5);
    }

    #[test]
    fn a_work_folder_gets_a_test_subfolder_that_is_removed_and_nothing_else_changes() {
        let work = tempfile::tempdir().unwrap();
        fs::write(work.path().join("ep01.mkv"), b"video").unwrap();
        let (mut r, mut cleanup) = (Report::default(), Cleanup::default());

        work_folder(&mut r, work.path(), me(), &mut cleanup);
        assert_eq!(r.failed(), 0);
        cleanup.finish(&mut r);

        assert_eq!(r.failed(), 0);
        let names: Vec<_> = fs::read_dir(work.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["ep01.mkv"]);
    }

    #[test]
    fn a_rename_between_two_folders_of_one_mount_is_reported_not_failed() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mut r = Report::default();
        across_mounts(&mut r, a.path(), b.path(), Some(false));
        assert_eq!(r.failed(), 0);
        // Between mounts it is expected to fail, and a rename that works is a failed check.
        fs::remove_file(b.path().join("across.txt")).unwrap();
        across_mounts(&mut r, a.path(), b.path(), Some(true));
        assert_eq!(r.failed(), 1);
    }

    #[test]
    #[should_panic(expected = "removes only what it named itself")]
    fn only_what_the_probe_named_is_ever_removed() {
        Cleanup::default().track(Path::new("/downloads/Work A"));
    }
}
