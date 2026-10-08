//! The checks of a folder a person names (the collect folder, the archive
//! folder, a watch folder) and of two folders against each other.
//!
//! The web runs them when the folders are entered, to tell the person what is
//! wrong; the worker runs them again before it moves a work folder from one to
//! the other, since the folders may have changed since. Each caller words what
//! it tells: the checks here only say which condition failed.
//!
//! Two folders are compared after links are resolved, by whole path
//! components, never by string prefix: `/media/Shows2` is not inside
//! `/media/Shows`.

use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

/// A folder that is there, with links resolved and the file system it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// The folder with links resolved, for comparing folders.
    pub real: PathBuf,
    /// The device number of the file system the folder is on.
    pub device: u64,
}

/// Why a path is no folder to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// The path does not start at the root. Only [`check_folder`] asks.
    NotAbsolute,
    /// Nothing is at the path.
    NotFound,
    /// Something is at the path, but it cannot be looked at (not allowed, an
    /// error of the file system).
    Unreadable,
    /// What is at the path is not a folder.
    NotAFolder,
    /// The folder is there, but its real path cannot be found.
    Unresolvable,
}

/// The device number of the file system `metadata` is of.
pub fn device_of(metadata: &fs::Metadata) -> u64 {
    metadata.dev()
}

/// An existing folder with links resolved. `device` tells which file system
/// the folder is on, from its path and metadata: [`device_of`], or a stand-in
/// of a test. A relative path is looked up from the current folder; see
/// [`check_folder`] for one that must start at the root.
pub fn resolve_folder(
    path: &Path,
    device: impl FnOnce(&Path, &fs::Metadata) -> u64,
) -> Result<Folder, Problem> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Problem::NotFound),
        Err(_) => return Err(Problem::Unreadable),
    };
    if !metadata.is_dir() {
        return Err(Problem::NotAFolder);
    }
    let real = fs::canonicalize(path).map_err(|_| Problem::Unresolvable)?;
    Ok(Folder {
        real,
        device: device(path, &metadata),
    })
}

/// [`resolve_folder`] for a path that must be absolute, on the real file
/// systems.
pub fn check_folder(path: &Path) -> Result<Folder, Problem> {
    if !path.is_absolute() {
        return Err(Problem::NotAbsolute);
    }
    resolve_folder(path, |_, metadata| device_of(metadata))
}

/// How two folders (real paths) lie to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlap {
    /// They are the same folder.
    Same,
    /// The first is inside the second.
    FirstInsideSecond,
    /// The second is inside the first.
    SecondInsideFirst,
}

/// How `first` and `second` overlap, by whole path components; `None` when
/// neither is the other or inside the other.
pub fn overlap(first: &Path, second: &Path) -> Option<Overlap> {
    if first == second {
        Some(Overlap::Same)
    } else if first.starts_with(second) {
        Some(Overlap::FirstInsideSecond)
    } else if second.starts_with(first) {
        Some(Overlap::SecondInsideFirst)
    } else {
        None
    }
}

/// Why two folders cannot be the two ends of a move by a rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// They are the same, or one is inside the other.
    Overlapping(Overlap),
    /// They are on different file systems, which a rename does not cross.
    DifferentDevice,
}

/// The first reason `first` and `second` cannot be the two ends of a move by
/// a rename: the overlap first, then the file systems.
pub fn conflict(first: &Folder, second: &Folder) -> Option<Conflict> {
    if let Some(overlap) = overlap(&first.real, &second.real) {
        return Some(Conflict::Overlapping(overlap));
    }
    (first.device != second.device).then_some(Conflict::DifferentDevice)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(real: &str, device: u64) -> Folder {
        Folder {
            real: PathBuf::from(real),
            device,
        }
    }

    #[test]
    fn a_folder_is_resolved_with_its_real_path_and_device() {
        let dir = tempfile::tempdir().unwrap();
        let real = fs::canonicalize(dir.path()).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let found = check_folder(&link).unwrap();
        assert_eq!(found.real, real);
        assert_eq!(found.device, device_of(&fs::metadata(&real).unwrap()));
    }

    #[test]
    fn each_failure_of_a_path_is_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "x").unwrap();
        assert_eq!(
            check_folder(Path::new("downloads")),
            Err(Problem::NotAbsolute)
        );
        assert_eq!(
            check_folder(&dir.path().join("missing")),
            Err(Problem::NotFound)
        );
        assert_eq!(check_folder(&file), Err(Problem::NotAFolder));
        // A file is no folder to look in: neither missing nor a folder.
        assert_eq!(check_folder(&file.join("b")), Err(Problem::Unreadable));
    }

    #[test]
    fn a_relative_path_is_resolved_by_resolve_folder_only() {
        let found = resolve_folder(Path::new("."), |_, _| 7).unwrap();
        assert_eq!(found.device, 7, "the caller's device stands in");
        assert_eq!(found.real, fs::canonicalize(".").unwrap());
        assert_eq!(check_folder(Path::new(".")), Err(Problem::NotAbsolute));
    }

    #[test]
    fn two_folders_overlap_by_components_after_links_are_resolved() {
        let of = |a: &str, b: &str| overlap(Path::new(a), Path::new(b));
        assert_eq!(of("/media/A", "/media/A"), Some(Overlap::Same));
        assert_eq!(
            of("/media/A/B", "/media/A"),
            Some(Overlap::FirstInsideSecond)
        );
        assert_eq!(
            of("/media/A", "/media/A/B"),
            Some(Overlap::SecondInsideFirst)
        );
        assert_eq!(of("/media/A", "/media/B"), None);
        assert_eq!(
            of("/media/Shows", "/media/Shows2"),
            None,
            "not a text prefix"
        );
        assert_eq!(of("/", "/media"), Some(Overlap::SecondInsideFirst));
    }

    #[test]
    fn a_conflict_names_the_overlap_before_the_file_systems() {
        let on = |a: &Folder, b: &Folder| conflict(a, b);
        let (a, b, inside) = (
            folder("/media/A", 1),
            folder("/media/B", 1),
            folder("/media/A/B", 2),
        );
        assert_eq!(on(&a, &b), None);
        assert_eq!(
            on(&a, &folder("/media/A", 1)),
            Some(Conflict::Overlapping(Overlap::Same))
        );
        assert_eq!(
            on(&a, &inside),
            Some(Conflict::Overlapping(Overlap::SecondInsideFirst)),
            "inside wins over a different device"
        );
        assert_eq!(
            on(&inside, &a),
            Some(Conflict::Overlapping(Overlap::FirstInsideSecond))
        );
        assert_eq!(
            on(&a, &folder("/mnt/B", 2)),
            Some(Conflict::DifferentDevice)
        );
    }
}
