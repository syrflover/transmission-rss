//! The CRC32 of a video and the identity of the file it was read from, for
//! the replacement of video revisions (`docs/specs/collection.md`, 영상
//! 수정본의 대체). What a release name says about its revision and its CRC32
//! is read by [`crate::release_name`].

use std::{
    fs::File,
    io,
    io::Read,
    path::{Path, PathBuf},
};

use trss_core::{
    file_id::FileId,
    files::{rename_noreplace, sync_renamed},
};

/// `v2`, as the version line writes a revision.
pub fn label(version: u32) -> String {
    format!("v{version}")
}

/// A CRC32 as release names write it: eight upper-case hexadecimal digits.
pub fn crc_text(crc: u32) -> String {
    format!("{crc:08X}")
}

/// Reads `crc` back from [`crc_text`].
pub fn parse_crc(text: &str) -> Option<u32> {
    (text.len() == 8)
        .then(|| u32::from_str_radix(text, 16).ok())
        .flatten()
}

/// Renames the video `source` to `target`, which must be free
/// ([`rename_noreplace`]: `AlreadyExists` otherwise, and nothing moves), and
/// syncs the two folders so the name outlives a power loss, off the async
/// threads. This is for a video renamed by trss itself; a torrent's is renamed
/// by Transmission.
///
/// A folder that cannot be synced is no failure of the rename: the video has
/// its new name and nothing can take it back, so the failure is only logged.
pub async fn rename_video(source: PathBuf, target: PathBuf) -> io::Result<()> {
    tokio::task::spawn_blocking(move || {
        rename_noreplace(&source, &target)?;
        if let Err(err) = sync_renamed(&source, &target) {
            eprintln!(
                "Could not sync the folders of {} renamed to {}: {err}",
                source.display(),
                target.display()
            );
        }
        Ok(())
    })
    .await
    .map_err(io::Error::other)?
}

/// How much of a file [`file_crc32`] holds in memory at a time.
pub const CRC_BUFFER: usize = 1 << 20;

/// The CRC32 of the file at `path`, read from start to end through one
/// [`CRC_BUFFER`]-sized buffer: a video of any size costs that much memory.
/// Blocking; call it off the async threads.
pub fn file_crc32(path: &Path) -> io::Result<u32> {
    file_crc32_identified(path).map(|(crc, _)| crc)
}

/// What tells a file apart from another one put under its name: its
/// [`FileId`] (device and inode), its size, and its modification and
/// status-change times (a write into it changes the first; the second also
/// catches a write that put the modification time back, and any change of
/// mode or owner).
///
/// `==` compares all of it. For an identity kept to be compared with the file
/// found later, [`FileIdentity::unchanged`] leaves the device number out as
/// [`FileId::same_file`] does, and the file itself is recognized by
/// [`FileIdentity::id`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    id: FileId,
    len: u64,
    mtime: i64,
    mtime_nsec: i64,
    ctime: i64,
    ctime_nsec: i64,
}

impl FileIdentity {
    pub fn of(meta: &std::fs::Metadata) -> FileIdentity {
        use std::os::unix::fs::MetadataExt;
        FileIdentity {
            id: FileId::of(meta),
            len: meta.len(),
            mtime: meta.mtime(),
            mtime_nsec: meta.mtime_nsec(),
            ctime: meta.ctime(),
            ctime_nsec: meta.ctime_nsec(),
        }
    }

    /// The identity of what is at `path` itself (a symbolic link is not
    /// followed, so it is never the file it points to).
    pub fn at(path: &Path) -> io::Result<FileIdentity> {
        std::fs::symlink_metadata(path).map(|meta| FileIdentity::of(&meta))
    }

    /// The file's id, which recognizes it again whatever happened to it since
    /// (a rename changes its status-change time): [`FileId::same_file`].
    pub fn id(&self) -> FileId {
        self.id
    }

    /// Whether `other` is the same file ([`FileId::same_file`]) as it was:
    /// this identity but for the device number.
    pub fn unchanged(&self, other: &FileIdentity) -> bool {
        other.id.same_file(self.id)
            && FileIdentity {
                id: other.id,
                ..*self
            } == *other
    }

    /// The identity as text, to keep in the database ([`FileIdentity::parse`]
    /// reads it back).
    pub fn to_text(&self) -> String {
        format!(
            "{}:{}:{}:{}.{}:{}.{}",
            self.id.dev(),
            self.id.ino(),
            self.len,
            self.mtime,
            self.mtime_nsec,
            self.ctime,
            self.ctime_nsec
        )
    }

    /// The identity [`FileIdentity::to_text`] wrote, or `None` for other text.
    pub fn parse(text: &str) -> Option<FileIdentity> {
        let mut parts = text.split(':');
        let mut next = || parts.next();
        let dev = next()?.parse().ok()?;
        let ino = next()?.parse().ok()?;
        let len = next()?.parse().ok()?;
        let (mtime, mtime_nsec) = next()?.split_once('.')?;
        let (ctime, ctime_nsec) = next()?.split_once('.')?;
        if next().is_some() {
            return None;
        }
        Some(FileIdentity {
            id: FileId::new(dev, ino),
            len,
            mtime: mtime.parse().ok()?,
            mtime_nsec: mtime_nsec.parse().ok()?,
            ctime: ctime.parse().ok()?,
            ctime_nsec: ctime_nsec.parse().ok()?,
        })
    }
}

/// [`file_crc32`], with the identity of the file that was read, taken from
/// the open file itself: whatever is put under `path` while it is read, the
/// CRC32 is this file's. A file that changes while it is read is an error.
pub fn file_crc32_identified(path: &Path) -> io::Result<(u32, FileIdentity)> {
    let mut file = File::open(path)?;
    let before = FileIdentity::of(&file.metadata()?);
    let mut hasher = crc32fast::Hasher::new();
    let mut buffer = vec![0; CRC_BUFFER];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buffer[..n]),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    let after = FileIdentity::of(&file.metadata()?);
    if after != before {
        return Err(io::Error::other(format!(
            "{} changed while it was read",
            path.display()
        )));
    }
    Ok((hasher.finalize(), after))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::FileExt;

    use super::*;

    #[test]
    fn a_crc_is_written_as_eight_upper_case_digits() {
        assert_eq!(crc_text(0x1bbd34e6), "1BBD34E6");
        assert_eq!(crc_text(0x5), "00000005");
        assert_eq!(parse_crc("1BBD34E6"), Some(0x1BBD34E6));
        assert_eq!(parse_crc("1BBD34E"), None);
    }

    #[test]
    fn a_rewrite_that_restores_the_modification_time_changes_the_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mkv");
        std::fs::write(&path, b"aaaa").unwrap();
        let before = FileIdentity::at(&path).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        // The kernel's clock ticks coarsely: let the rewrite fall in another.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.write_all_at(b"bbbb", 0).unwrap();
        file.set_modified(modified).unwrap();
        drop(file);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 4);
        assert_ne!(FileIdentity::at(&path).unwrap(), before);
    }

    #[test]
    fn an_identity_kept_as_text_reads_back_and_a_rename_keeps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mkv");
        std::fs::write(&path, b"aaaa").unwrap();
        let before = FileIdentity::at(&path).unwrap();
        assert_eq!(FileIdentity::parse(&before.to_text()), Some(before));
        assert_eq!(FileIdentity::parse("1:2:3"), None);
        assert_eq!(
            FileIdentity::parse(&format!("{}:9", before.to_text())),
            None
        );

        let moved = dir.path().join("b.mkv");
        std::fs::rename(&path, &moved).unwrap();
        assert!(FileIdentity::at(&moved)
            .unwrap()
            .id()
            .same_file(before.id()));
        std::fs::write(&path, b"aaaa").unwrap();
        assert!(!FileIdentity::at(&path).unwrap().id().same_file(before.id()));
    }

    #[test]
    fn a_kept_identity_is_unchanged_whatever_its_device_number_until_the_file_is_written() {
        let kept = FileIdentity::parse("47:8716384:5:1759700000.1:1759700000.2").unwrap();
        // The same file after its file system was mounted again.
        let remounted = FileIdentity::parse("46:8716384:5:1759700000.1:1759700000.2").unwrap();
        assert_ne!(remounted, kept);
        assert!(remounted.unchanged(&kept));
        // Another inode.
        let other = FileIdentity::parse("47:8716385:5:1759700000.1:1759700000.2").unwrap();
        assert!(!other.unchanged(&kept));
        // The same file renamed, or written into, since.
        for since in [
            "46:8716384:5:1759700000.1:1759800000.0",
            "46:8716384:5:1759800000.0:1759800000.0",
            "46:8716384:6:1759700000.1:1759700000.2",
        ] {
            let since = FileIdentity::parse(since).unwrap();
            assert!(!since.unchanged(&kept), "{since:?}");
        }
    }

    #[test]
    fn a_file_crc_is_the_crc_of_its_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mkv");
        // Longer than the buffer, so it takes several reads.
        let bytes: Vec<u8> = (0..(CRC_BUFFER * 2 + 123))
            .map(|i| (i % 251) as u8)
            .collect();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(file_crc32(&path).unwrap(), crc32fast::hash(&bytes));
        assert!(file_crc32(&dir.path().join("missing")).is_err());
    }
}
