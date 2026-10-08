//! The file effects of storing and applying, on a work folder's file system
//! (or the app data folder's, for the files of a package that are neither
//! subtitles nor fonts). Blocking: the caller runs them off the runtime.
//!
//! Every effect writes its own temporary file in the work folder's
//! `.trss/tmp/` (the app data folder's `subtitle-files/.tmp/`), on the file
//! system of where it is published, and publishes
//! it by a rename that replaces nothing ([`rename_noreplace`]). A rename keeps
//! the file's object, so the published file is known for this effect's by
//! the object recorded before the rename.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use trss_core::{file_id::FileId, files::rename_noreplace};

use crate::area::{hex, read_facts, sync_dir};

/// The hidden folder of the app in a work folder.
pub const TRSS_DIR: &str = ".trss";
/// Where stored subtitles and fonts go, relative to the work folder.
pub const SUBTITLES_DIR: &str = ".trss/subtitles";
/// Where the temporary files of effects go, relative to the work folder.
pub const TEMP_DIR: &str = ".trss/tmp";
/// Where a package's attachments and companion files go, relative to the
/// app data folder: `<work id>/<creator>/<name>` under it.
pub const APP_FILES_DIR: &str = "subtitle-files";
/// Where the temporary files of effects in the app data folder go, relative
/// to it.
pub const APP_TEMP_DIR: &str = "subtitle-files/.tmp";

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
        if let Err(err) = make_dirs(parent) {
            return Copied::Failed(err);
        }
    }
    let mut reader = match open_regular(source) {
        Ok(file) => file,
        Err(err) => return Copied::Failed(err),
    };
    let mut writer = match OpenOptions::new().write(true).create_new(true).open(temp) {
        Ok(file) => file,
        Err(err) => return Copied::Failed(err),
    };
    let written = (|| -> io::Result<(u64, String)> {
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
        writer.sync_all()?;
        Ok((total, hex(&hasher.finalize())))
    })();
    drop(writer);
    let gone = |result: Copied| {
        let _ = fs::remove_file(temp);
        result
    };
    match written {
        Err(err) => gone(Copied::Failed(err)),
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

/// Makes `dir` and the folders above it that are missing, syncing the
/// folder each new one is in: a synced file is kept by a power loss only
/// while its folders are.
fn make_dirs(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    if let Some(parent) = dir.parent() {
        make_dirs(parent)?;
    }
    match fs::create_dir(dir) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists && dir.is_dir() => return Ok(()),
        Err(err) => return Err(err),
    }
    match dir.parent() {
        Some(parent) => sync_dir(parent),
        None => Ok(()),
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
        if let Err(err) = make_dirs(parent) {
            return Published::Failed(err);
        }
    }
    match rename_noreplace(temp, target) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => return Published::Occupied,
        Err(err) => return Published::Failed(err),
    }
    for folder in [target.parent(), temp.parent()].into_iter().flatten() {
        if let Err(err) = sync_dir(folder) {
            return Published::Failed(err);
        }
    }
    Published::Done
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

/// Whether something is at `path` (a link counts, whatever it points to).
pub fn occupied(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
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
        let temp = dir.path().join(".trss/tmp/e1");
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
        let temp = dir.path().join(".trss/tmp/e2");
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
        let temp = dir.path().join(".trss/tmp/e1");
        assert!(matches!(
            copy_to_temp(&source, &temp, 14, &sha(b"[Script Info]\n")),
            Copied::SourceChanged(_)
        ));
        assert!(!temp.exists());
    }
}
