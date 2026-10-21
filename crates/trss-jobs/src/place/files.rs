//! The file writes of storing and applying, on a work folder's file system
//! (or the app data folder's, for the files of a package that are neither
//! subtitles nor fonts). Blocking: the caller runs them off the runtime.
//!
//! Every store or apply writes its new copy whole in the work folder's
//! `.trss/staging/` (the app data folder's `subtitle-files/.tmp/`), on the
//! file system of where it is published, syncs it, and publishes it by a
//! rename that replaces nothing ([`trss_core::files::rename_noreplace_synced`]).
//! Nothing records the copy before it is published: a run cut short leaves at
//! most a staged copy, which goes when the folder is emptied ([`Staging`] for
//! a work folder once per run, [`empty_app_temp`] for the app data folder
//! when `trss` starts), or a published file, which the next run finds by its
//! bytes ([`super::Placer`]). A replacement stages its new copy and its
//! imports the same way, and moves the new copy on to [`ASIDE_DIR`] before it
//! swaps it in ([`super::replace`]).

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use trss_core::{
    file_id::FileId,
    files::{create_dir_all_synced, rename_noreplace_synced, sync_dir, write_new},
};

use crate::area::{hex, read_facts};

/// The hidden folder of the app in a work folder.
pub const TRSS_DIR: &str = ".trss";
/// Where stored subtitles and fonts go, relative to the work folder.
pub const SUBTITLES_DIR: &str = ".trss/subtitles";
/// Where a replacement keeps the files it takes off the paths beside a
/// video, and its new copy for the while before it swaps it in
/// ([`super::replace`]), and a relocation the applied copy it takes off
/// until it removes it ([`super::relocate`]), relative to the work folder:
/// `<ID>`, a new one each. A run removes what it put there once it is done
/// with it; nothing else removes anything there, so what a run cut short
/// left stays. Releases before 0153 used `.trss/tmp/` for these and for
/// their staged copies; nothing touches that folder now.
pub const ASIDE_DIR: &str = ".trss/aside";
/// Where a store or an apply writes its new copy before publishing it,
/// relative to the work folder. Only those writes are there, and a run
/// empties it before its first write into the work folder ([`Staging`]).
pub const STAGING_DIR: &str = ".trss/staging";
// Where a package's attachments and companion files go, relative to the
// app data folder (`<work id>/<creator>/<name>` under it), and where their
// new copies are written before they are published, relative to it.
pub use trss_core::app_data::{
    SUBTITLE_FILES_DIR as APP_FILES_DIR, SUBTITLE_FILES_TEMP_DIR as APP_TEMP_DIR,
};

/// The work folders one run of a job has emptied the staging folder of
/// ([`STAGING_DIR`]). A run makes one and asks it for each new copy it
/// writes into a work folder: the folder's staging folder is emptied before
/// the first, so what a run cut short left there goes before anything is
/// written, and a copy staged in this run is never removed by it. Jobs run
/// one at a time, so no other run is writing there. Cheap to clone: the
/// clones share what was emptied.
///
/// The emptying removes what it can: an entry that cannot be removed (its
/// permissions on a share, say) is left there and told on the standard
/// error, and the folder counts as emptied. Every new copy has a new name,
/// so nothing left there is in a write's way; only the room it takes is
/// lost.
#[derive(Debug, Clone, Default)]
pub struct Staging {
    emptied: std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>>,
}

impl Staging {
    /// A new file's path in `folder`'s staging folder, relative to `folder`;
    /// the staging folder is emptied first unless this run did that already.
    /// Blocking.
    pub fn new_file(&self, folder: &Path) -> String {
        let mut emptied = self.emptied.lock().unwrap_or_else(|e| e.into_inner());
        if !emptied.iter().any(|f| f == folder) {
            let staging = within(folder, STAGING_DIR);
            match trss_core::files::empty_dir(&staging) {
                Ok(done) => {
                    for (path, err) in done.left {
                        eprintln!(
                            "trss: cannot remove {} while staging: {err}",
                            path.display()
                        );
                    }
                }
                Err(err) => eprintln!("trss: cannot empty {}: {err}", staging.display()),
            }
            emptied.push(folder.to_owned());
        }
        format!("{STAGING_DIR}/{}", uuid::Uuid::new_v4())
    }
}

/// Removes everything in the app data folder's `subtitle-files/.tmp/` it can
/// ([`trss_core::files::empty_dir`]): the new copies of stores a process left
/// when it stopped. `trss` calls it when it starts, before any job runs.
pub fn empty_app_temp(app_data: &Path) -> io::Result<trss_core::files::Emptied> {
    trss_core::files::empty_dir(&within(app_data, APP_TEMP_DIR))
}

/// What came of writing a temporary file.
#[derive(Debug)]
pub enum Copied {
    /// Written, synced and read back: its object.
    Ready(String),
    /// The source's bytes are not the ones expected: nothing is left.
    SourceChanged(String),
    Failed(io::Error),
}

/// Copies `source` to the new file `temp` (its folders made), syncs it and
/// reads it back: it must be `size` bytes of SHA-256 `sha256`. A file already
/// at `temp` is not touched (`AlreadyExists`). Whatever goes wrong after
/// `temp` was made removes it again.
pub fn copy_to_temp(source: &Path, temp: &Path, size: u64, sha256: &str) -> Copied {
    if let Some(parent) = temp.parent() {
        if let Err(err) = create_dir_all_synced(parent) {
            return Copied::Failed(err);
        }
    }
    let mut reader = match open_regular(source) {
        Ok(file) => file,
        Err(err) => return Copied::Failed(err),
    };
    let written = write_new(temp, None, |writer| {
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            writer.write_all(&buf[..n])?;
            total += n as u64;
        }
        Ok((total, hex(&hasher.finalize())))
    });
    let gone = |result: Copied| {
        let _ = fs::remove_file(temp);
        result
    };
    match written {
        Err(err) => Copied::Failed(err),
        Ok((n, sum)) if n != size || sum != sha256 => gone(Copied::SourceChanged(format!(
            "복사할 파일이 기록과 달라요 (크기 {n}, 기록 {size})"
        ))),
        Ok(_) => match read_facts(temp) {
            Ok((n, sum, object)) if n == size && sum == sha256 => {
                match temp.parent().map(sync_dir) {
                    Some(Err(err)) => gone(Copied::Failed(err)),
                    _ => Copied::Ready(object),
                }
            }
            Ok(_) => gone(Copied::Failed(io::Error::other(
                "다시 읽은 임시 파일의 바이트가 쓴 것과 달라요",
            ))),
            Err(err) => gone(Copied::Failed(err)),
        },
    }
}

/// Opens a regular file, refusing a link or anything else.
fn open_regular(path: &Path) -> io::Result<File> {
    let seen = fs::symlink_metadata(path)?;
    if !seen.file_type().is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    let file = File::open(path)?;
    if !FileId::of(&file.metadata()?).same_file_now(FileId::of(&seen)) {
        return Err(io::Error::other("replaced while opened"));
    }
    Ok(file)
}

/// What came of publishing a temporary file.
#[derive(Debug)]
pub enum Published {
    /// Renamed to its target, and both folders synced.
    Done,
    /// Something is at the target already: nothing was moved.
    Occupied,
    Failed(io::Error),
}

/// Renames `temp` to `target` (its folder made) unless `target` exists, then
/// syncs both folders.
pub fn publish(temp: &Path, target: &Path) -> Published {
    if let Some(parent) = target.parent() {
        if let Err(err) = create_dir_all_synced(parent) {
            return Published::Failed(err);
        }
    }
    match rename_noreplace_synced(temp, target) {
        Ok(()) => Published::Done,
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Published::Occupied,
        Err(err) => Published::Failed(err),
    }
}

/// A regular file's length, SHA-256 and object; `None` when nothing is at
/// `path`. Anything else there (a folder, a link) is an error.
pub fn facts(path: &Path) -> io::Result<Option<(u64, String, String)>> {
    match fs::symlink_metadata(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
        Ok(_) => read_facts(path).map(Some),
    }
}

/// The names in `folder` (none when it is not there).
pub fn names_in(folder: &Path) -> io::Result<Vec<String>> {
    match fs::read_dir(folder) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err),
        Ok(entries) => entries
            .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect(),
    }
}

/// Removes a file: whether it is gone, removed now or found missing.
pub fn remove_known(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// `relative` (`/`-separated) within `folder`.
pub fn within(folder: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(folder.to_path_buf(), |path, part| path.join(part))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    #[test]
    fn a_copy_is_checked_and_published_without_replacing() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.ass");
        fs::write(&source, b"[Script Info]\n").unwrap();
        let temp = dir.path().join(".trss/staging/e1");
        let Copied::Ready(object) = copy_to_temp(&source, &temp, 14, &sha(b"[Script Info]\n"))
        else {
            panic!("copied");
        };
        let target = dir.path().join("Season 01/a.ass");
        assert!(matches!(publish(&temp, &target), Published::Done));
        let (_, _, published) = facts(&target).unwrap().unwrap();
        assert_eq!(published, object, "a rename keeps the object");
        assert!(!temp.exists());

        // A second copy to the same name is not published over the first.
        let temp = dir.path().join(".trss/staging/e2");
        assert!(matches!(
            copy_to_temp(&source, &temp, 14, &sha(b"[Script Info]\n")),
            Copied::Ready(_)
        ));
        assert!(matches!(publish(&temp, &target), Published::Occupied));
        assert!(temp.exists());
    }

    #[test]
    fn a_source_other_than_recorded_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.ass");
        fs::write(&source, b"changed").unwrap();
        let temp = dir.path().join(".trss/staging/e1");
        assert!(matches!(
            copy_to_temp(&source, &temp, 14, &sha(b"[Script Info]\n")),
            Copied::SourceChanged(_)
        ));
        assert!(!temp.exists());
    }
}
