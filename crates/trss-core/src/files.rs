//! File tools several features share.

use std::{io, path::Path};

/// Renames `from` to `to` unless `to` exists (an `AlreadyExists` error then),
/// with `renameat2(RENAME_NOREPLACE)`. Never replaces anything, a directory
/// included. There is no fallback: a filesystem without the flag answers
/// `InvalidInput` or `Unsupported`.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use rustix::fs::{renameat_with, RenameFlags, CWD};
    renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE).map_err(io::Error::from)
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
}
