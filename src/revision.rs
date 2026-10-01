//! What a release name says about its video for the replacement of video
//! revisions (`docs/specs/collection.md`, 영상 수정본의 대체): which release
//! of which episode it is, its revision (`14v2`), and the CRC32 the release
//! group put in the name; and the CRC32 of a file, read as a stream.
//!
//! - **The CRC32** is the last bracket before the extension when it holds
//!   exactly eight hexadecimal digits: `… (1080p) [E2675E51].mkv`, and with
//!   Erai-raws' several brackets `… [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv`.
//!   A name whose last bracket is something else (`[MultiSub]`) has none. RSS
//!   titles may leave the extension out; the rule is the same.
//! - **The revision** is the `vN` right after the episode's number, which
//!   follows ` - ` (`Show - 14v2`, `Show 3v3 - 06v3`), whatever follows it
//!   (`… - 03v2 1080p AAC 2.0`, `… - 03v2 - Part 2`). A name with no such
//!   marker takes the last `vN` right after a number (`Show 14v2 (1080p)`),
//!   unless ` - ` and a number follow it outside brackets: then it is part of
//!   the show's name (`Show 3v3 - 06` is the first revision of episode 6).
//!   Without one the release is its first revision.
//! - **The same release** of an episode is the name without its revision,
//!   its CRC32 bracket and its extension ([`Release::stem`]): `[SubsPlease]
//!   Show - 14 (1080p)` for both `14` and `14v2`. Another group's release of
//!   the same episode has another stem, so it is a duplicate, not a revision.

use std::{fs::File, io, io::Read, path::Path, sync::LazyLock};

use regex::Regex;

/// What a release name says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The name without its revision, CRC32 bracket and extension, trimmed.
    pub stem: String,
    /// 1 for a name without `vN`.
    pub version: u32,
    /// The CRC32 the name carries.
    pub crc: Option<u32>,
}

static EXTENSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.[A-Za-z0-9]{2,4}\s*$").unwrap());
static CRC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([0-9A-Fa-f]{8})\]\s*$").unwrap());
static VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(\d{1,4}(?:\.\d)?)v(\d{1,2})\b").unwrap());

/// ` - ` and a number, as a name gives its episode's number after the
/// show's name.
static EPISODE_AFTER_DASH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s-\s*\d{1,4}(?:\.\d)?(?:\D|$)").unwrap());

/// The episode's revision marker in `text`. The last `NvM` right after ` - `
/// is on the episode's number, whatever follows it. Without one, the last
/// `NvM`, unless ` - ` and a number follow it outside brackets: the episode's
/// number follows the show's name, so in `Show 3v3 - 06` the `3v3` is part
/// of the name and the episode `06` carries no revision.
fn last_version(text: &str) -> Option<regex::Captures<'_>> {
    let mut all: Vec<regex::Captures<'_>> = VERSION.captures_iter(text).collect();
    let on_episode = all
        .iter()
        .rposition(|c| text[..c.get(0).unwrap().start()].trim_end().ends_with('-'));
    if let Some(at) = on_episode {
        return Some(all.swap_remove(at));
    }
    let found = all.pop()?;
    let after = &text[found.get(0).unwrap().end()..];
    let episode_after = EPISODE_AFTER_DASH
        .find_iter(after)
        .any(|dash| bracket_depth(&after[..dash.start()]) == 0);
    (!episode_after).then_some(found)
}

/// How deep in brackets or parentheses the end of `text` is, counting from
/// its start (a closing one with none open counts as none).
fn bracket_depth(text: &str) -> usize {
    text.chars().fold(0usize, |depth, c| match c {
        '[' | '(' => depth + 1,
        ']' | ')' => depth.saturating_sub(1),
        _ => depth,
    })
}

impl Release {
    pub fn parse(name: &str) -> Release {
        let mut rest = name.trim().to_owned();
        if let Some(found) = EXTENSION.find(&rest) {
            rest.truncate(found.start());
        }
        let found = CRC.captures(&rest).map(|c| {
            let value = u32::from_str_radix(&c[1], 16).expect("eight hex digits");
            (value, c.get(0).unwrap().start())
        });
        let crc = found.map(|(value, start)| {
            rest.truncate(start);
            value
        });
        let mut version = 1;
        let found = last_version(&rest).map(|c| {
            let range = c.get(1).unwrap().end()..c.get(0).unwrap().end();
            (c[2].parse().unwrap_or(1).max(1), range)
        });
        if let Some((value, range)) = found {
            version = value;
            rest.replace_range(range, "");
        }
        Release {
            stem: rest.trim().to_owned(),
            version,
            crc,
        }
    }

    /// `name` without its revision (`14v2` becomes `14`), everything else as
    /// it is: what the episode's file name is derived from. `trname` does not
    /// read `06v2` as episode 6 in every name (Erai-raws' `… - 06v2 [1080p CR
    /// WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv` gives episode 34).
    pub fn without_version(name: &str) -> String {
        let name = name.trim();
        let mut end = name.len();
        if let Some(found) = EXTENSION.find(name) {
            end = found.start();
        }
        if let Some(found) = CRC.captures(&name[..end]) {
            end = found.get(0).unwrap().start();
        }
        match last_version(&name[..end]) {
            Some(c) => {
                let mut out = name.to_owned();
                out.replace_range(c.get(1).unwrap().end()..c.get(0).unwrap().end(), "");
                out
            }
            None => name.to_owned(),
        }
    }

    /// `v2`, as the version line writes a revision.
    pub fn label(version: u32) -> String {
        format!("v{version}")
    }
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

/// How much of a file [`file_crc32`] holds in memory at a time.
pub const CRC_BUFFER: usize = 1 << 20;

/// The CRC32 of the file at `path`, read from start to end through one
/// [`CRC_BUFFER`]-sized buffer: a video of any size costs that much memory.
/// Blocking; call it off the async threads.
pub fn file_crc32(path: &Path) -> io::Result<u32> {
    file_crc32_identified(path).map(|(crc, _)| crc)
}

/// What tells a file apart from another one put under its name: its device
/// and inode, its size, and its modification and status-change times (a
/// write into it changes the first; the second also catches a write that put
/// the modification time back, and any change of mode or owner).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    dev: u64,
    ino: u64,
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
            dev: meta.dev(),
            ino: meta.ino(),
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

    /// Whether `other` is the same file on disk (device and inode), whatever
    /// happened to it since: a rename changes its status-change time.
    pub fn same_file(&self, other: &FileIdentity) -> bool {
        self.dev == other.dev && self.ino == other.ino
    }

    /// The identity as text, to keep in the database ([`FileIdentity::parse`]
    /// reads it back).
    pub fn to_text(&self) -> String {
        format!(
            "{}:{}:{}:{}.{}:{}.{}",
            self.dev, self.ino, self.len, self.mtime, self.mtime_nsec, self.ctime, self.ctime_nsec
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
            dev,
            ino,
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

/// The season and episode an episode file name (`Show S01E14.mkv`) is of.
pub fn season_episode(name: &str) -> Option<(u32, String)> {
    static NAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)S(\d{2,})E(\d{2,}(?:\.\d)?)\.\w+$").unwrap());
    let found = NAME.captures(name)?;
    Some((found[1].parse().ok()?, found[2].to_owned()))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::FileExt;

    use super::*;

    #[test]
    fn an_episode_name_gives_its_season_and_episode() {
        assert_eq!(
            season_episode("Show S01E14.mkv"),
            Some((1, "14".to_owned()))
        );
        assert_eq!(
            season_episode("Show S02E05.5.mp4"),
            Some((2, "05.5".to_owned()))
        );
        assert_eq!(season_episode("[SubsPlease] Show - 14.mkv"), None);
    }

    #[test]
    fn a_subsplease_name_gives_its_crc_and_its_revision() {
        let v1 = Release::parse("[SubsPlease] Sono Bisque Doll - 14 (1080p) [E2675E51].mkv");
        let v2 = Release::parse("[SubsPlease] Sono Bisque Doll - 14v2 (1080p) [1A2B3C4D].mkv");
        assert_eq!(v1.stem, "[SubsPlease] Sono Bisque Doll - 14 (1080p)");
        assert_eq!((v1.version, v1.crc), (1, Some(0xE2675E51)));
        assert_eq!(v2.stem, v1.stem);
        assert_eq!((v2.version, v2.crc), (2, Some(0x1A2B3C4D)));
    }

    #[test]
    fn an_erai_raws_name_takes_the_hex_bracket_before_the_extension() {
        let release = Release::parse(
            "[Erai-raws] Kimi to Idol Precure - 06v2 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv",
        );
        assert_eq!(release.crc, Some(0x1BBD34E6));
        assert_eq!(release.version, 2);
        assert_eq!(
            release.stem,
            "[Erai-raws] Kimi to Idol Precure - 06 [1080p CR WEBRip HEVC AAC][MultiSub]"
        );
        // An RSS title without the extension reads the same.
        let title = Release::parse(
            "[Erai-raws] Kimi to Idol Precure - 06v2 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6]",
        );
        assert_eq!(title, release);
    }

    #[test]
    fn a_number_v_number_in_the_show_name_is_not_the_revision() {
        let name = "[SubsPlease] Show 3v3 - 06v2 (1080p) [1A2B3C4D].mkv";
        let release = Release::parse(name);
        assert_eq!(release.version, 2);
        assert_eq!(release.stem, "[SubsPlease] Show 3v3 - 06 (1080p)");
        assert_eq!(
            Release::without_version(name),
            "[SubsPlease] Show 3v3 - 06 (1080p) [1A2B3C4D].mkv"
        );
    }

    /// A show named with `NvM` whose episode carries no revision marker is
    /// the first revision of that episode: only a marker on the episode's
    /// number counts.
    #[test]
    fn a_number_v_number_in_the_show_name_of_an_unversioned_episode_is_no_revision() {
        let first = "Show 3v3 - 06 [1080p].mkv";
        let second = "Show 3v3 - 06v2 [1080p].mkv";
        let v1 = Release::parse(first);
        let v2 = Release::parse(second);
        assert_eq!((v1.version, v1.stem.as_str()), (1, "Show 3v3 - 06 [1080p]"));
        assert_eq!((v2.version, v2.stem.as_str()), (2, "Show 3v3 - 06 [1080p]"));
        assert_eq!(Release::without_version(first), first);
        assert_eq!(Release::without_version(second), first);
        // The same with a CRC32, and with the extension left out.
        let named = "[SubsPlease] Show 3v3 - 06 (1080p) [1A2B3C4D].mkv";
        assert_eq!(Release::parse(named).version, 1);
        assert_eq!(Release::without_version(named), named);
        let title = "[SubsPlease] Show 3v3 - 06 (1080p) [1A2B3C4D]";
        assert_eq!(Release::parse(title).version, 1);
        assert_eq!(
            Release::parse(title).stem,
            "[SubsPlease] Show 3v3 - 06 (1080p)"
        );
        // A revision marker in brackets right after the show is still read.
        let bracketed = "[Group] Show [06v2][1080p].mkv";
        assert_eq!(Release::parse(bracketed).version, 2);
    }

    /// The `NvM` right after ` - ` is the episode's and its revision, whatever
    /// numbers follow it (audio channels, a part); one in the show's name is
    /// followed by ` - ` and the episode's number.
    #[test]
    fn the_revision_on_the_episode_number_is_read_whatever_follows() {
        for (name, version, stem) in [
            (
                "[Group] Show - 03v2 1080p WEB AAC 2.0 x264.mkv",
                2,
                "[Group] Show - 03 1080p WEB AAC 2.0 x264",
            ),
            (
                "[Group] Show - 03v2 - Part 2.mkv",
                2,
                "[Group] Show - 03 - Part 2",
            ),
            ("Show 2 - 03v2.mkv", 2, "Show 2 - 03"),
            ("Show - 03v2 (2024).mkv", 2, "Show - 03 (2024)"),
            ("Show S2 - 03v2 [1080p].mkv", 2, "Show S2 - 03 [1080p]"),
            ("86 - 03v2.mkv", 2, "86 - 03"),
            ("Re:Zero 3v3 - 06.mkv", 1, "Re:Zero 3v3 - 06"),
            ("Re:Zero 3v3 - 06v2.mkv", 2, "Re:Zero 3v3 - 06"),
            (
                "[Group] Show 14v2 (1080p).mkv",
                2,
                "[Group] Show 14 (1080p)",
            ),
        ] {
            let release = Release::parse(name);
            assert_eq!(
                (release.version, release.stem.as_str()),
                (version, stem),
                "{name}"
            );
        }
        assert_eq!(
            Release::without_version("[Group] Show - 03v2 - Part 2.mkv"),
            "[Group] Show - 03 - Part 2.mkv"
        );
    }

    #[test]
    fn a_v_number_that_does_not_follow_a_number_is_no_revision() {
        for name in [
            "[Group] Gundam V2 - 06 (1080p) [1A2B3C4D].mkv",
            "[Group] Show Ver.2 - 06 (1080p) [1A2B3C4D].mkv",
            "[Group] Show S01E06v2 (1080p) [1A2B3C4D].mkv",
            "[Group] Show - 06 (x264v2) [1A2B3C4D].mkv",
        ] {
            let release = Release::parse(name);
            assert_eq!(release.version, 1, "{name}");
            assert_eq!(Release::without_version(name), name, "{name}");
        }
    }

    #[test]
    fn a_name_without_its_revision_keeps_everything_else() {
        assert_eq!(
            Release::without_version(
                "[Erai-raws] Show - 06v2 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv"
            ),
            "[Erai-raws] Show - 06 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv"
        );
        assert_eq!(
            Release::without_version("[SubsPlease] Show - 14v2 (1080p) [8F2EFECC].mkv"),
            "[SubsPlease] Show - 14 (1080p) [8F2EFECC].mkv"
        );
        let first = "[SubsPlease] Show - 14 (1080p) [8F2EFECC].mkv";
        assert_eq!(Release::without_version(first), first);
    }

    #[test]
    fn a_last_bracket_that_is_not_eight_hex_digits_is_no_crc() {
        for name in [
            "[Erai-raws] Show - 06 [1080p][1BBD34E6][MultiSub].mkv",
            "[SubsPlease] Show - 14v2 (1080p).mkv",
            "[Group] Show - 14 [1BBD34E].mkv",
            "[Group] Show - 14 [1BBD34EG].mkv",
        ] {
            assert_eq!(Release::parse(name).crc, None, "{name}");
        }
        assert_eq!(
            Release::parse("[SubsPlease] Show - 14v2 (1080p).mkv").version,
            2
        );
    }

    #[test]
    fn another_groups_release_of_the_episode_is_another_release() {
        let a = Release::parse("[SubsPlease] Show - 14 (1080p) [E2675E51].mkv");
        let b = Release::parse("[Erai-raws] Show - 14 [1080p][E2675E51].mkv");
        assert_ne!(a.stem, b.stem);
        let c = Release::parse("[SubsPlease] Show - 15 (1080p) [E2675E51].mkv");
        assert_ne!(a.stem, c.stem);
    }

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
        assert!(FileIdentity::at(&moved).unwrap().same_file(&before));
        std::fs::write(&path, b"aaaa").unwrap();
        assert!(!FileIdentity::at(&path).unwrap().same_file(&before));
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
