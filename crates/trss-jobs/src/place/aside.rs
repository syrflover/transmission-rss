//! Renaming a file aside, checking it is the expected one, and putting it
//! back or removing it: what a replacement ([`super::replace`]) does to the
//! subtitle it takes the place of and a relocation ([`super::relocate`]) to
//! an applied copy it takes off. Blocking, like [`super::files`]: the caller
//! runs them off the runtime. Nothing here writes a record; each caller
//! records what an outcome means to it and words it.
//!
//! 1. [`set_aside`] renames the file to the aside path, replacing nothing,
//!    syncs the two folders ([`trss_core::files::sync_renamed`]) and
//!    [`check`]s what is there.
//! 2. [`check`] looks at the aside file ([`look`]). The expected file stays;
//!    anything else goes back where it was, and nothing is changed when the
//!    file is gone.
//! 3. [`remove`] removes the files set aside (and whatever else the caller
//!    names) and syncs their folders.
//!
//! What the expected file is ([`Expected`]) is the caller's: its length and
//! SHA-256 always, and its object (the inode) when the caller has one to
//! compare. A replacement compares it, since the plan saw that very file; a
//! relocation does not, since a file system mounted again may give the same
//! file another device number and a file of the applied bytes loses nothing
//! when removed, its stored subtitle keeping them.
//!
//! The aside path's folder is made by the caller, when it wants it made: a
//! missing one makes the rename fail ([`Moved::Missing`]).

use std::{io, path::Path};

use trss_core::{
    file_id::same_recorded_file,
    files::{rename_noreplace, sync_dir, sync_renamed},
};

use super::files;

/// What a file must be to count as the expected one.
#[derive(Debug, Clone, Copy)]
pub struct Expected<'a> {
    pub size: u64,
    pub sha256: &'a str,
    /// Its object as a record keeps it, compared by inode
    /// ([`same_recorded_file`]); `None` compares the bytes only.
    pub object: Option<&'a str>,
}

impl Expected<'_> {
    /// Whether `found` (a file's length, SHA-256 and object, or nothing) is
    /// the expected file.
    pub fn matches(&self, found: &Option<(u64, String, String)>) -> bool {
        found.as_ref().is_some_and(|(size, sha256, object)| {
            *size == self.size
                && *sha256 == self.sha256
                && self.object.is_none_or(|r| same_recorded_file(object, r))
        })
    }
}

/// What a look at a path found.
#[derive(Debug)]
pub enum Look {
    /// The expected file.
    Expected,
    /// Nothing there.
    Gone,
    /// Another file there.
    Differs,
    /// A folder, a link, or a file that could not be read.
    Unreadable(io::Error),
}

/// Looks at `path`: whether the expected file is there.
pub fn look(path: &Path, expected: &Expected) -> Look {
    match files::facts(path) {
        Ok(None) => Look::Gone,
        Ok(found) if expected.matches(&found) => Look::Expected,
        Ok(Some(_)) => Look::Differs,
        Err(err) => Look::Unreadable(err),
    }
}

/// What came of checking a file set aside.
#[derive(Debug)]
pub enum Check {
    /// The expected file is aside.
    Expected,
    /// Nothing is at the aside path.
    Gone,
    /// Another file (or one that could not be read) was aside: it is back on
    /// its path, and both folders are synced.
    PutBack,
    /// It is back on its path, and the folders could not be synced.
    PutBackUnsynced(io::Error),
    /// It could not be put back (its path is taken, say): it is still aside.
    NotPutBack(io::Error),
}

/// Looks at the file aside and puts one that is not the expected file back on
/// `from`, replacing nothing.
pub fn check(aside: &Path, from: &Path, expected: &Expected) -> Check {
    match look(aside, expected) {
        Look::Expected => Check::Expected,
        Look::Gone => Check::Gone,
        Look::Differs | Look::Unreadable(_) => match rename_noreplace(aside, from) {
            Ok(()) => match sync_renamed(aside, from) {
                Ok(()) => Check::PutBack,
                Err(err) => Check::PutBackUnsynced(err),
            },
            Err(err) => Check::NotPutBack(err),
        },
    }
}

/// What came of renaming a file aside.
#[derive(Debug)]
pub enum Moved {
    /// The rename failed with `NotFound`, no file at the file's path or no
    /// folder for the aside path: nothing moved.
    Missing,
    /// The rename failed for another reason, one that says the aside path is
    /// taken included: nothing moved.
    NotMoved(io::Error),
    /// It is aside, and the folders could not be synced.
    Unsynced(io::Error),
    /// It is aside and was checked.
    Checked(Check),
}

impl Moved {
    /// What a failed rename (or a folder that could not be made before it)
    /// is.
    pub fn refused(err: io::Error) -> Moved {
        match err.kind() {
            io::ErrorKind::NotFound => Moved::Missing,
            _ => Moved::NotMoved(err),
        }
    }
}

/// Renames `from` to `aside`, replacing nothing, syncs the folders and
/// [`check`]s the file there.
pub fn set_aside(from: &Path, aside: &Path, expected: &Expected) -> Moved {
    if let Err(err) = rename_noreplace(from, aside) {
        return Moved::refused(err);
    }
    if let Err(err) = sync_renamed(from, aside) {
        return Moved::Unsynced(err);
    }
    Moved::Checked(check(aside, from, expected))
}

/// Removes each of `paths` (one that is not there is removed already), then
/// syncs the folders they were in, each once, those that are there.
pub fn remove(paths: &[&Path]) -> io::Result<()> {
    for path in paths {
        files::remove_known(path)?;
    }
    let mut synced: Vec<&Path> = Vec::new();
    for folder in paths.iter().filter_map(|path| path.parent()) {
        if !synced.contains(&folder) && folder.is_dir() {
            sync_dir(folder)?;
            synced.push(folder);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use trss_core::files::testing::{fail_syncs_of, syncs_of};

    use super::*;
    use crate::area::{hex, read_facts};

    fn sha(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        hex(&Sha256::digest(bytes))
    }

    /// A folder with the file `a.ass` of `BYTES` in it and the folder of its
    /// aside path beside it.
    struct Setup {
        _dir: tempfile::TempDir,
        folder: std::path::PathBuf,
        from: std::path::PathBuf,
        tmp: std::path::PathBuf,
        aside: std::path::PathBuf,
        sha256: String,
        object: String,
    }

    const BYTES: &[u8] = b"[Script Info]\n";

    fn setup() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().canonicalize().unwrap();
        let from = folder.join("a.ass");
        fs::write(&from, BYTES).unwrap();
        let tmp = folder.join(".trss/tmp");
        fs::create_dir_all(&tmp).unwrap();
        let aside = tmp.join("e1.aside");
        let (_, sha256, object) = read_facts(&from).unwrap();
        Setup {
            _dir: dir,
            folder,
            from,
            tmp,
            aside,
            sha256,
            object,
        }
    }

    impl Setup {
        fn expected(&self) -> Expected<'_> {
            self.expecting(None)
        }

        fn expecting<'a>(&'a self, object: Option<&'a str>) -> Expected<'a> {
            Expected {
                size: BYTES.len() as u64,
                sha256: &self.sha256,
                object,
            }
        }
    }

    #[test]
    fn the_expected_file_is_left_aside_and_both_folders_are_synced() {
        let s = setup();
        let moved = set_aside(&s.from, &s.aside, &s.expected());
        assert!(
            matches!(moved, Moved::Checked(Check::Expected)),
            "{moved:?}"
        );
        assert!(!s.from.exists());
        assert_eq!(fs::read(&s.aside).unwrap(), BYTES);
        assert!(syncs_of(&s.folder) >= 1 && syncs_of(&s.tmp) >= 1);
    }

    #[test]
    fn a_file_changed_before_the_rename_is_put_back_where_it_was() {
        let s = setup();
        fs::write(&s.from, b"changed one!!").unwrap();
        let moved = set_aside(&s.from, &s.aside, &s.expected());
        assert!(matches!(moved, Moved::Checked(Check::PutBack)), "{moved:?}");
        assert_eq!(fs::read(&s.from).unwrap(), b"changed one!!");
        assert!(!s.aside.exists());
        // Both folders once for the rename aside and once for the put-back.
        assert_eq!((syncs_of(&s.folder), syncs_of(&s.tmp)), (2, 2));
    }

    #[test]
    fn the_bytes_make_the_file_and_the_inode_too_when_the_object_is_given() {
        let s = setup();
        let (dev, ino) = {
            let mut fields = s.object.split(':');
            let dev: u64 = fields.next().unwrap().parse().unwrap();
            let ino: u64 = fields.next().unwrap().parse().unwrap();
            (dev, ino)
        };
        let found = Some((BYTES.len() as u64, s.sha256.clone(), s.object.clone()));
        let (same_device, other_inode) = (s.object.clone(), format!("{dev}:{}", ino + 1));
        let matches = |object: Option<&str>, found: &Option<(u64, String, String)>| {
            s.expecting(object).matches(found)
        };
        assert!(matches(None, &found), "the bytes alone");
        assert!(matches(Some(&same_device), &found));
        // The object given is compared (what counts as the same object is
        // trss-core's `same_recorded_file`, tested there).
        assert!(!matches(Some(&other_inode), &found));
        // Other bytes are not the file, whatever the object.
        let other = Some((BYTES.len() as u64, sha(b"another one"), s.object.clone()));
        assert!(!matches(None, &other));
        assert!(!matches(Some(&same_device), &other));
        let shorter = Some((3, s.sha256.clone(), s.object.clone()));
        assert!(!matches(None, &shorter));
        assert!(!matches(None, &None));
    }

    #[test]
    fn a_file_of_the_bytes_but_another_inode_is_put_back_only_when_the_object_is_given() {
        let s = setup();
        let (dev, ino) = s.object.split_once(':').unwrap();
        let other_inode = format!("{dev}:{}", ino.parse::<u64>().unwrap() + 1);
        let strict = Expected {
            object: Some(&other_inode),
            ..s.expected()
        };
        let moved = set_aside(&s.from, &s.aside, &strict);
        assert!(matches!(moved, Moved::Checked(Check::PutBack)), "{moved:?}");
        assert!(s.from.exists() && !s.aside.exists());
        let moved = set_aside(&s.from, &s.aside, &s.expected());
        assert!(
            matches!(moved, Moved::Checked(Check::Expected)),
            "{moved:?}"
        );
    }

    #[test]
    fn nothing_at_the_path_moves_nothing() {
        let s = setup();
        fs::remove_file(&s.from).unwrap();
        let moved = set_aside(&s.from, &s.aside, &s.expected());
        assert!(matches!(moved, Moved::Missing), "{moved:?}");
        assert!(!s.aside.exists());
    }

    #[test]
    fn a_missing_folder_for_the_aside_path_is_not_made() {
        let s = setup();
        let aside = s.folder.join("nowhere/e1.aside");
        let moved = set_aside(&s.from, &aside, &s.expected());
        assert!(matches!(moved, Moved::Missing), "{moved:?}");
        assert_eq!(fs::read(&s.from).unwrap(), BYTES);
        assert!(!s.folder.join("nowhere").exists());
    }

    #[test]
    fn an_aside_path_that_is_taken_refuses_the_rename_and_both_files_stay() {
        let s = setup();
        fs::write(&s.aside, b"taken").unwrap();
        let moved = set_aside(&s.from, &s.aside, &s.expected());
        let Moved::NotMoved(err) = moved else {
            panic!("{moved:?}");
        };
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&s.from).unwrap(), BYTES);
        assert_eq!(fs::read(&s.aside).unwrap(), b"taken");
    }

    #[test]
    fn a_failed_sync_after_the_rename_leaves_the_file_aside_unchecked() {
        let s = setup();
        let _failing = fail_syncs_of(&s.tmp);
        let moved = set_aside(&s.from, &s.aside, &s.expected());
        assert!(matches!(moved, Moved::Unsynced(_)), "{moved:?}");
        assert!(!s.from.exists());
        assert_eq!(fs::read(&s.aside).unwrap(), BYTES);
    }

    #[test]
    fn nothing_aside_is_gone_and_nothing_is_put_back() {
        let s = setup();
        fs::remove_file(&s.from).unwrap();
        let checked = check(&s.aside, &s.from, &s.expected());
        assert!(matches!(checked, Check::Gone), "{checked:?}");
        assert!(!s.from.exists());
    }

    #[test]
    fn the_expected_file_found_aside_stays_there() {
        let s = setup();
        fs::rename(&s.from, &s.aside).unwrap();
        let checked = check(&s.aside, &s.from, &s.expected());
        assert!(matches!(checked, Check::Expected), "{checked:?}");
        assert!(s.aside.exists() && !s.from.exists());
    }

    #[test]
    fn another_file_found_aside_goes_back_unless_its_path_is_taken() {
        let s = setup();
        fs::write(&s.aside, b"other bytes").unwrap();
        // The path was taken meanwhile: the file stays aside, untouched.
        let checked = check(&s.aside, &s.from, &s.expected());
        let Check::NotPutBack(err) = checked else {
            panic!("{checked:?}");
        };
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&s.from).unwrap(), BYTES);
        assert_eq!(fs::read(&s.aside).unwrap(), b"other bytes");

        fs::remove_file(&s.from).unwrap();
        let checked = check(&s.aside, &s.from, &s.expected());
        assert!(matches!(checked, Check::PutBack), "{checked:?}");
        assert_eq!(fs::read(&s.from).unwrap(), b"other bytes");
        assert!(!s.aside.exists());
    }

    #[test]
    fn a_failed_sync_after_the_put_back_leaves_the_file_back_on_its_path() {
        let s = setup();
        fs::remove_file(&s.from).unwrap();
        fs::write(&s.aside, b"other bytes").unwrap();
        let _failing = fail_syncs_of(&s.tmp);
        let checked = check(&s.aside, &s.from, &s.expected());
        assert!(matches!(checked, Check::PutBackUnsynced(_)), "{checked:?}");
        assert_eq!(fs::read(&s.from).unwrap(), b"other bytes");
        assert!(!s.aside.exists());
    }

    #[test]
    fn a_file_that_cannot_be_read_is_not_the_expected_one_and_goes_back() {
        let s = setup();
        fs::remove_file(&s.from).unwrap();
        fs::create_dir(&s.aside).unwrap();
        assert!(matches!(look(&s.aside, &s.expected()), Look::Unreadable(_)));
        let checked = check(&s.aside, &s.from, &s.expected());
        assert!(matches!(checked, Check::PutBack), "{checked:?}");
        assert!(s.from.is_dir());
    }

    #[test]
    fn a_look_tells_the_expected_file_another_and_none() {
        let s = setup();
        assert!(matches!(look(&s.from, &s.expected()), Look::Expected));
        assert!(matches!(look(&s.aside, &s.expected()), Look::Gone));
        fs::write(&s.aside, b"other bytes").unwrap();
        assert!(matches!(look(&s.aside, &s.expected()), Look::Differs));
    }

    #[test]
    fn removing_takes_each_file_and_syncs_their_folders_once() {
        let s = setup();
        let second = s.tmp.join("e1");
        fs::write(&s.aside, b"a").unwrap();
        fs::write(&second, b"b").unwrap();
        let before = syncs_of(&s.tmp);
        remove(&[&s.aside, &second]).unwrap();
        assert!(!s.aside.exists() && !second.exists());
        assert_eq!(syncs_of(&s.tmp), before + 1);

        // Folders that differ are each synced; a file that is not there is
        // removed already.
        let before = (syncs_of(&s.tmp), syncs_of(&s.folder));
        fs::write(&s.aside, b"a").unwrap();
        remove(&[&s.aside, &s.folder.join("missing")]).unwrap();
        assert_eq!(
            (syncs_of(&s.tmp), syncs_of(&s.folder)),
            (before.0 + 1, before.1 + 1)
        );
    }

    #[test]
    fn removing_in_a_folder_that_is_not_there_is_done() {
        let s = setup();
        remove(&[&s.folder.join("nowhere/e1.aside")]).unwrap();
    }

    #[test]
    fn a_removal_that_fails_stops_before_the_next_and_the_syncs() {
        let s = setup();
        let folder = s.tmp.join("a folder");
        fs::create_dir(&folder).unwrap();
        fs::write(&s.aside, b"a").unwrap();
        let before = syncs_of(&s.tmp);
        assert!(remove(&[&folder, &s.aside]).is_err());
        assert!(s.aside.exists(), "the next file is not touched");
        assert_eq!(syncs_of(&s.tmp), before);
    }

    #[test]
    fn a_failed_sync_after_the_removal_is_reported_with_the_files_gone() {
        let s = setup();
        fs::write(&s.aside, b"a").unwrap();
        let _failing = fail_syncs_of(&s.tmp);
        assert!(remove(&[&s.aside]).is_err());
        assert!(!s.aside.exists());
    }
}
