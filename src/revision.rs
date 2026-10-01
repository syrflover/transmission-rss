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
//! - **The revision** is the `vN` right after a number (`14v2`, `06v3`);
//!   without one the release is its first revision.
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
        let found = VERSION.captures(&rest).map(|c| {
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
    let mut file = File::open(path)?;
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
    Ok(hasher.finalize())
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
