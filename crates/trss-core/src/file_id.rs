//! Recognizing a file again: the one place that says whether a file found now
//! is a file recorded earlier.
//!
//! A record keeps what the file system names the file by, its device number
//! and inode ([`FileId`]), in three forms that all stay readable:
//!
//! - the receive area's object text `<device>:<inode>`, which records of an
//!   earlier glibc build may follow with a birth time (`:<ns>`);
//! - the video revisions' identity text
//!   `<device>:<inode>:<size>:<mtime>.<ns>:<ctime>.<ns>`, whose first two
//!   fields are the same;
//! - the `dev` and `ino` columns of `artwork_files`.
//!
//! **The rule** ([`FileId::same_file`]): a recorded file and a file found
//! later are the same file when their inodes are equal. The device number is
//! left out, since a file system mounted again may give the same file another
//! one (btrfs numbers its devices at each mount; on the dev PC an unchanged
//! file went from 47 to 46 after a reboot), and a record made before a
//! restart would then name no file. No birth time is compared either: the
//! image builds for musl, where the standard library reads none. The risk
//! that remains is accepted: an inode number of another file system, or one
//! reused by a later file, passes for the recorded file. Where that matters
//! the feature compares more, such as the bytes, the size and change times, or
//! what else refers to the file; those checks stay in the feature crates.
//!
//! Two ids read at the same moment ([`FileId::same_file_now`]) are compared by
//! device and inode, since both come from one mount state and the device
//! number then tells files of different file systems apart.

use std::{fmt, fs::Metadata, os::unix::fs::MetadataExt};

/// A file as its file system names it: device number and inode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId {
    dev: u64,
    ino: u64,
}

impl FileId {
    /// The id of the file `meta` describes.
    pub fn of(meta: &Metadata) -> FileId {
        FileId {
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }

    /// The id with the given device number and inode, as a record keeps them.
    pub fn new(dev: u64, ino: u64) -> FileId {
        FileId { dev, ino }
    }

    pub fn dev(self) -> u64 {
        self.dev
    }

    pub fn ino(self) -> u64 {
        self.ino
    }

    /// Reads the id from the start of a record's text: `<device>:<inode>`,
    /// whatever follows (a birth time, or the size and times of a video
    /// revisions' identity) ignored. `None` for text that names no inode.
    pub fn parse(text: &str) -> Option<FileId> {
        let mut fields = text.split(':');
        let dev = fields.next()?.parse().ok()?;
        let ino = fields.next()?.parse().ok()?;
        Some(FileId { dev, ino })
    }

    /// Whether `self`, a file found now, is the file `recorded` names: the
    /// same inode (see the module docs).
    pub fn same_file(self, recorded: FileId) -> bool {
        self.ino == recorded.ino
    }

    /// Whether `self` and `other`, read at the same moment, are one file:
    /// the same device and inode.
    pub fn same_file_now(self, other: FileId) -> bool {
        self == other
    }
}

/// The text a record keeps: `<device>:<inode>`, no birth time.
impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.dev, self.ino)
    }
}

/// Whether two records' texts ([`FileId::parse`]) name the same file
/// ([`FileId::same_file`]). Text that names no inode names no file.
pub fn same_recorded_file(a: &str, b: &str) -> bool {
    match (FileId::parse(a), FileId::parse(b)) {
        (Some(a), Some(b)) => a.same_file(b),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_the_file_s_device_and_inode_and_its_text_has_no_birth_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.ass");
        std::fs::write(&path, b"a").unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let id = FileId::of(&meta);
        assert_eq!((id.dev(), id.ino()), (meta.dev(), meta.ino()));
        assert_eq!(id.to_string(), format!("{}:{}", meta.dev(), meta.ino()));
        assert_eq!(FileId::parse(&id.to_string()), Some(id));
    }

    #[test]
    fn the_object_text_is_read_with_and_without_the_old_birth_time() {
        let id = FileId::new(47, 8716384);
        assert_eq!(FileId::parse("47:8716384"), Some(id));
        assert_eq!(FileId::parse("47:8716384:1759700000123456789"), Some(id));
    }

    #[test]
    fn the_video_identity_text_is_read_by_its_first_two_fields() {
        let id = FileId::new(47, 8716384);
        assert_eq!(
            FileId::parse("47:8716384:5:1759700000.1:1759700000.2"),
            Some(id)
        );
    }

    #[test]
    fn the_artwork_columns_make_an_id() {
        let id = FileId::new(47, 8716384);
        assert_eq!((id.dev(), id.ino()), (47, 8716384));
        assert_eq!(id.to_string(), "47:8716384");
    }

    #[test]
    fn text_that_names_no_inode_is_no_id() {
        for text in ["", "8716384", "47:", ":8716384", "a:b", "47:-1"] {
            assert_eq!(FileId::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_recorded_file_is_the_same_file_whatever_its_device_number() {
        // The same file after its file system was mounted again.
        assert!(FileId::new(46, 8716384).same_file(FileId::new(47, 8716384)));
        assert!(same_recorded_file("47:8716384", "46:8716384"));
        // A record a build that read birth times made is the file of its
        // inode, and so is a video revisions' identity text.
        assert!(same_recorded_file(
            "47:8716384:1759700000123456789",
            "46:8716384"
        ));
        assert!(same_recorded_file(
            "47:8716384:1759700000123456789",
            "46:8716384:1759800000000000000"
        ));
        assert!(same_recorded_file(
            "47:8716384:5:1759700000.1:1759700000.2",
            "46:8716384"
        ));
    }

    #[test]
    fn another_inode_is_another_file() {
        assert!(!FileId::new(47, 8716385).same_file(FileId::new(47, 8716384)));
        assert!(!same_recorded_file("47:8716384", "47:8716385"));
        assert!(!same_recorded_file(
            "47:8716384:1759700000123456789",
            "47:8716385"
        ));
        // What names no inode is no file, not even the same as itself.
        assert!(!same_recorded_file("", ""));
        assert!(!same_recorded_file("8716384", "8716384"));
    }

    #[test]
    fn two_ids_read_at_the_same_moment_are_one_file_by_device_and_inode() {
        let id = FileId::new(47, 8716384);
        assert!(id.same_file_now(FileId::new(47, 8716384)));
        assert!(!id.same_file_now(FileId::new(46, 8716384)));
        assert!(!id.same_file_now(FileId::new(47, 8716385)));
    }

    #[test]
    fn a_hard_link_and_a_rename_keep_the_id_and_a_new_file_has_another() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a");
        std::fs::write(&path, b"a").unwrap();
        let id = |p: &std::path::Path| FileId::of(&std::fs::metadata(p).unwrap());
        let before = id(&path);
        let linked = dir.path().join("b");
        std::fs::hard_link(&path, &linked).unwrap();
        assert!(id(&linked).same_file_now(before));
        let moved = dir.path().join("c");
        std::fs::rename(&path, &moved).unwrap();
        assert!(id(&moved).same_file(before));
        std::fs::write(&path, b"a").unwrap();
        assert!(!id(&path).same_file(before));
    }
}
