//! How many dialogue lines a subtitle file has, for the version lines of a
//! replacement's decision card (`docs/specs/subtitles.md`, 교체 비교와
//! 승인). It looks for each format's line markers only and parses nothing
//! else; the content comparison is the diff's.

/// The dialogue lines of a subtitle whose name ends in `extension`: ASS and
/// SSA `Dialogue:` lines, SRT and WebVTT cues, SMI `<SYNC>` tags. `None` for
/// an image subtitle, another format, or bytes that do not look like their
/// format.
pub fn count(bytes: &[u8], extension: &str) -> Option<u64> {
    let text = text_of(bytes);
    let lower = text.to_lowercase();
    match extension.to_ascii_lowercase().as_str() {
        "ass" | "ssa" => {
            if !lower.contains("[script info]") && !lower.contains("[events]") {
                return None;
            }
            Some(
                lower
                    .lines()
                    .filter(|l| l.trim_start().starts_with("dialogue:"))
                    .count() as u64,
            )
        }
        "srt" | "vtt" => match lower.lines().filter(|l| l.contains("-->")).count() {
            0 => None,
            n => Some(n as u64),
        },
        "smi" | "sami" => {
            if !lower.contains("<sami") && !lower.contains("<sync") {
                return None;
            }
            Some(lower.matches("<sync").count() as u64)
        }
        _ => None,
    }
}

/// The bytes as text: UTF-16 by its BOM, else what reads of them (the
/// markers are ASCII, so a legacy encoding such as CP949 keeps them).
fn text_of(bytes: &[u8]) -> String {
    let utf16 = |pairs: &[u8], big: bool| {
        let units: Vec<u16> = pairs
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&p| match big {
                true => u16::from_be_bytes(p),
                false => u16::from_le_bytes(p),
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, false),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, true),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_format_counts_its_dialogue_lines() {
        let ass = b"[Script Info]\nTitle: x\n[Events]\nFormat: Layer, Start\n\
                    Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,a\n\
                    Comment: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,b\n\
                    Dialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,c\n";
        assert_eq!(count(ass, "ass"), Some(2));
        let srt = b"1\n00:00:01,000 --> 00:00:02,000\na\n\n2\n00:00:03,000 --> 00:00:04,000\nb\n";
        assert_eq!(count(srt, "srt"), Some(2));
        let smi = b"<SAMI><BODY><SYNC Start=1000><P>a<SYNC Start=2000><P>&nbsp;</BODY></SAMI>";
        assert_eq!(count(smi, "smi"), Some(2));
    }

    #[test]
    fn utf16_and_legacy_bytes_keep_their_markers() {
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "<SAMI><SYNC Start=1><P>가".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(count(&utf16, "smi"), Some(1));
        // CP949 bytes for 가 around an SRT cue.
        let cp949 = b"1\n00:00:01,000 --> 00:00:02,000\n\xB0\xA1\n";
        assert_eq!(count(cp949, "srt"), Some(1));
    }

    #[test]
    fn what_is_not_its_format_is_not_counted() {
        assert_eq!(count(b"not a subtitle", "ass"), None);
        assert_eq!(count(b"not a subtitle", "srt"), None);
        assert_eq!(count(b"\x00\x01PG", "sup"), None);
    }
}
