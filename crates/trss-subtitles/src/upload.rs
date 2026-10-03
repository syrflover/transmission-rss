//! What a file a person uploads is, judged by its bytes and never by its name
//! (`docs/specs/subtitles.md`, 직접 찾기와 자막 올리기).
//!
//! An upload keeps subtitles and fonts and nothing else. [`judge`] reads the
//! start of the file (and a ZIP to its end, which checks every CRC, as
//! [`crate::verify::check`] does) and answers what it keeps it as, or why it
//! drops it:
//!
//! - A subtitle: ASS (SSA has the same header), SRT and SMI, told as
//!   [`crate::verify::sniff`] tells them, and the formats that are only
//!   stored (they are [`Format::Other`]): WebVTT, TTML/DFXP, MicroDVD and
//!   VobSub text subtitles (`.sub`), the VobSub index (`.idx`) and binary
//!   VobSub (`.sub`) and PGS (`.sup`) streams.
//! - A font: TrueType, OpenType, TrueType collections, WOFF and WOFF2, by
//!   their headers (a table count that fits the file, a WOFF's own length).
//! - An archive, kept as it is (what is inside is the package analysis's to
//!   judge): a ZIP, which is also read to its end (a ZIP that cannot be is
//!   dropped, and the ZIPs of one upload share a budget of inflated bytes,
//!   [`INFLATE_BUDGET`], that one finding it spent is dropped for), and a
//!   RAR, 7z, gzip, bzip2, xz or tar, told by their first bytes alone
//!   ([`Archive`]): nothing but the magic is checked. The volumes of a split
//!   archive after the first have no magic of their own: [`volume_anchor`] names
//!   the volume a set is vouched for by, and the caller, who sees the whole
//!   upload, keeps them when it is there ([`Verdict::BadZip`] is how a ZIP
//!   that cannot be read alone, as a spanned one cannot, comes back).
//! - An MPEG program stream is VobSub's binary `.sub` only when its VobSub
//!   index comes with it, and a video otherwise: [`judge`] says
//!   [`Verdict::Stream`] and the caller, who sees the whole upload, decides
//!   ([`is_vobsub_index`]).
//! - Anything else is dropped: an image under the name `a.ass`, a web page,
//!   a text file, a video, an empty file. The name plays no part.

use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

use crate::{
    verify::{
        ascii_view, check_zip_within, contains, sniff, Format, Sniffed, INFLATE_BUDGET_SPENT,
    },
    MAX_FILE_BYTES,
};

/// How many bytes the ZIPs of one upload may inflate in all while they are
/// read to their end: 2 GiB, twice what one ZIP may.
pub const INFLATE_BUDGET: u64 = 2 << 30;

/// The archive formats an upload keeps whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Archive {
    Zip,
    Rar,
    SevenZip,
    Gzip,
    Bzip2,
    Xz,
    Tar,
}

impl Archive {
    pub const ALL: [Archive; 7] = [
        Archive::Zip,
        Archive::Rar,
        Archive::SevenZip,
        Archive::Gzip,
        Archive::Bzip2,
        Archive::Xz,
        Archive::Tar,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Archive::Zip => "zip",
            Archive::Rar => "rar",
            Archive::SevenZip => "7z",
            Archive::Gzip => "gz",
            Archive::Bzip2 => "bz2",
            Archive::Xz => "xz",
            Archive::Tar => "tar",
        }
    }

    pub fn parse(code: &str) -> Option<Archive> {
        Archive::ALL.into_iter().find(|a| a.code() == code)
    }

    /// What the screens call it.
    pub fn label(self) -> &'static str {
        match self {
            Archive::Zip => "ZIP",
            Archive::Rar => "RAR",
            Archive::SevenZip => "7z",
            Archive::Gzip => "gzip",
            Archive::Bzip2 => "bzip2",
            Archive::Xz => "xz",
            Archive::Tar => "tar",
        }
    }
}

/// The archive format the first bytes of a file say, by magic alone: RAR 4
/// and 5, 7z, gzip (deflate), bzip2 (`BZh` and a block size digit), xz, tar
/// (`ustar` at offset 257), and a ZIP's `PK\x03\x04`.
pub fn archive_by_magic(head: &[u8]) -> Option<Archive> {
    if head.starts_with(b"PK\x03\x04") {
        Some(Archive::Zip)
    } else if head.starts_with(b"Rar!\x1A\x07\x00") || head.starts_with(b"Rar!\x1A\x07\x01\x00") {
        Some(Archive::Rar)
    } else if head.starts_with(b"7z\xBC\xAF\x27\x1C") {
        Some(Archive::SevenZip)
    } else if head.starts_with(b"\x1F\x8B\x08") {
        Some(Archive::Gzip)
    } else if head.starts_with(b"BZh") && head.get(3).is_some_and(|b| (b'1'..=b'9').contains(b)) {
        Some(Archive::Bzip2)
    } else if head.starts_with(b"\xFD7zXZ\x00") {
        Some(Archive::Xz)
    } else if head.get(257..262) == Some(b"ustar") {
        Some(Archive::Tar)
    } else {
        None
    }
}

/// The reason a split archive's volume is dropped with when the first volume
/// of its set is not in the upload (or is not an archive).
pub const NO_FIRST_VOLUME: &str = "나뉜 압축 파일의 첫 조각이 없어요";

/// The name (lowercase, with its folder) of the volume that vouches for the
/// volume `name` of a split archive, or `None` when `name` is no later volume:
/// `<set>.7z.002` is vouched for by `<set>.7z.001` (the same for `.zip`,
/// `.rar`, `.gz`, `.bz2`, `.xz`, `.tar` and the digits' width), `<set>.z01` by
/// the last part `<set>.zip` of a spanned ZIP, and `<set>.r00` by `<set>.rar`.
/// The volumes after the first have no magic of their own, so an upload keeps
/// one only with the volume that has.
pub fn volume_anchor(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let dir = lower.rfind(['/', '\\']).map_or(0, |i| i + 1);
    let (stem, ext) = lower[dir..].rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let number = |s: &str| s.parse::<u64>().ok();
    let before = &lower[..dir];
    if ext.len() >= 3 && all_digits(ext) {
        if number(ext)? < 2 {
            return None;
        }
        let (_, inner) = stem.rsplit_once('.')?;
        if matches!(inner, "7z" | "zip" | "rar" | "gz" | "bz2" | "xz" | "tar") {
            return Some(format!("{before}{stem}.{:0width$}", 1, width = ext.len()));
        }
        return None;
    }
    let (letter, digits) = ext.split_at_checked(1)?;
    if digits.len() < 2 || !all_digits(digits) {
        return None;
    }
    match letter {
        "z" if number(digits)? >= 1 => Some(format!("{before}{stem}.zip")),
        "r" => Some(format!("{before}{stem}.rar")),
        _ => None,
    }
}

/// Whether the file at `path` starts with the `PK` of a ZIP's records. A
/// spanned ZIP's volumes cannot be read to their end alone, so this is all
/// that is asked of one.
pub fn is_zip_signed(path: &Path) -> bool {
    let mut head = [0u8; 2];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut head))
        .is_ok()
        && &head == b"PK"
}

/// Whether a file name says it is an archive: `.zip`, `.rar`, `.7z`, `.gz`,
/// `.tgz`, `.bz2`, `.tbz2`, `.xz`, `.txz`, `.tar`, and the volumes of a split
/// archive (`.part1.rar`, `.r00`, `.z01`, `.7z.001`). A split volume is kept
/// as a file of its own; putting the volumes together is the analysis's.
pub fn is_archive_name(name: &str) -> bool {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let lower = base.to_ascii_lowercase();
    let Some((stem, ext)) = lower.rsplit_once('.') else {
        return false;
    };
    if stem.is_empty() {
        return false;
    }
    let digits = |s: &str, n: usize| s.len() >= n && s.bytes().all(|b| b.is_ascii_digit());
    match ext {
        "zip" | "rar" | "7z" | "gz" | "tgz" | "bz2" | "tbz2" | "xz" | "txz" | "tar" => true,
        // `.r00`, `.z01`: a volume of a split RAR or ZIP.
        _ if ext.len() >= 3 && matches!(&ext[..1], "r" | "z") && digits(&ext[1..], 2) => true,
        // `.7z.001`, `.zip.001`: a volume cut from an archive.
        _ if digits(ext, 3) => stem.rsplit_once('.').is_some_and(|(_, inner)| {
            matches!(inner, "7z" | "zip" | "rar" | "gz" | "bz2" | "xz" | "tar")
        }),
        _ => false,
    }
}

/// What a kept file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Subtitle,
    Font,
    Archive,
}

impl Kind {
    pub fn code(self) -> &'static str {
        match self {
            Kind::Subtitle => "subtitle",
            Kind::Font => "font",
            Kind::Archive => "archive",
        }
    }

    pub fn parse(code: &str) -> Option<Kind> {
        [Kind::Subtitle, Kind::Font, Kind::Archive]
            .into_iter()
            .find(|k| k.code() == code)
    }

    /// What the screens call it.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Subtitle => "자막",
            Kind::Font => "폰트",
            Kind::Archive => "압축 파일",
        }
    }
}

/// A file the upload keeps: what it is, and the format its bytes were checked
/// to be (a font is [`Format::Other`]; so is a subtitle format that is only
/// stored).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kept {
    pub kind: Kind,
    pub format: Format,
    /// Which archive format a kept archive is.
    pub archive: Option<Archive>,
}

/// What [`judge`] made of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Keep(Kept),
    /// An MPEG program stream: VobSub's `.sub` if a VobSub index
    /// ([`is_vobsub_index`]) of the same name is in the upload, else a video.
    Stream,
    /// Starts as a ZIP but cannot be read to its end (the reason says why):
    /// a damaged ZIP, or one volume of a spanned one. The caller decides
    /// ([`volume_anchor`], [`is_zip_signed`]); alone it is dropped with the reason.
    BadZip(String),
    /// Dropped, with why in a sentence to show as is.
    Drop(String),
}

/// How much of the start is looked at.
const HEAD: usize = 8 * 1024;

pub const NOT_THEM: &str = "내용이 자막이나 폰트가 아니에요";

/// What [`NOT_THEM`] becomes for a file whose name says it is an archive.
pub const NOT_AN_ARCHIVE: &str = "내용이 압축 파일이 아니에요";

/// Judges the file at `path`; a ZIP's inflation is counted against `budget`
/// (start with [`INFLATE_BUDGET`] for an upload). Blocking: it reads the file.
pub fn judge(path: &Path, budget: &mut u64) -> Verdict {
    let unreadable = |_: io::Error| Verdict::Drop("파일을 읽지 못했어요".to_owned());
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) => return unreadable(e),
    };
    let len = match file.metadata() {
        Ok(meta) => meta.len(),
        Err(e) => return unreadable(e),
    };
    if len == 0 {
        return Verdict::Drop("파일이 비어 있어요".to_owned());
    }
    if len > MAX_FILE_BYTES {
        return Verdict::Drop(format!("{}MiB를 넘어요", MAX_FILE_BYTES >> 20));
    }
    let mut head = Vec::with_capacity(HEAD);
    if let Err(e) = (&mut file).take(HEAD as u64).read_to_end(&mut head) {
        return unreadable(e);
    }
    drop(file);

    match classify(&head, len) {
        Classified::Zip => match check_zip_within(path, budget) {
            Ok(()) => Verdict::Keep(Kept {
                kind: Kind::Archive,
                format: Format::Zip,
                archive: Some(Archive::Zip),
            }),
            // The budget being spent says nothing about the ZIP itself.
            Err(failure) if failure.reason == INFLATE_BUDGET_SPENT => Verdict::Drop(failure.reason),
            Err(failure) => Verdict::BadZip(failure.reason),
        },
        Classified::Keep(kept) => Verdict::Keep(kept),
        Classified::Stream => Verdict::Stream,
        Classified::Drop(reason) => Verdict::Drop(reason.to_owned()),
    }
}

enum Classified {
    /// A ZIP by its start; its members are read next.
    Zip,
    Keep(Kept),
    Stream,
    Drop(&'static str),
}

fn keep(kind: Kind, format: Format) -> Classified {
    Classified::Keep(Kept {
        kind,
        format,
        archive: None,
    })
}

/// What the first bytes of a file of `len` bytes say.
fn classify(head: &[u8], len: u64) -> Classified {
    if is_font(head, len) {
        return keep(Kind::Font, Format::Other);
    }
    if head.starts_with(PROGRAM_STREAM) {
        return Classified::Stream;
    }
    if is_pgs(head) {
        return keep(Kind::Subtitle, Format::Other);
    }
    if let Some(archive) = archive_by_magic(head).filter(|a| *a != Archive::Zip) {
        return Classified::Keep(Kept {
            kind: Kind::Archive,
            format: Format::Other,
            archive: Some(archive),
        });
    }
    let text = ascii_view(head);
    let lower = text.trim_ascii_start().to_ascii_lowercase();
    if is_ttml(&lower) {
        return keep(Kind::Subtitle, Format::Other);
    }
    match sniff(head) {
        Sniffed::Empty => Classified::Drop("파일이 비어 있어요"),
        Sniffed::Html => Classified::Drop("내용이 파일이 아니라 웹 페이지(HTML)예요"),
        Sniffed::Format(Format::Zip) => Classified::Zip,
        Sniffed::Format(Format::Other) => match is_text_subtitle(&lower) {
            true => keep(Kind::Subtitle, Format::Other),
            false => Classified::Drop(NOT_THEM),
        },
        Sniffed::Format(format) => keep(Kind::Subtitle, format),
    }
}

/// A font by its header.
fn is_font(head: &[u8], len: u64) -> bool {
    let u16_at = |at: usize| {
        head.get(at..at + 2)
            .map(|b| u64::from(u16::from_be_bytes([b[0], b[1]])))
    };
    let u32_at = |at: usize| {
        head.get(at..at + 4)
            .map(|b| u64::from(u32::from_be_bytes([b[0], b[1], b[2], b[3]])))
    };
    let Some(magic) = head.get(..4) else {
        return false;
    };
    match magic {
        // sfnt: the table directory is 16 bytes a table after a 12-byte header.
        b"\x00\x01\x00\x00" | b"true" | b"OTTO" => {
            u16_at(4).is_some_and(|tables| (1..=128).contains(&tables) && len >= 12 + 16 * tables)
        }
        b"ttcf" => {
            u32_at(4).is_some_and(|version| version == 0x0001_0000 || version == 0x0002_0000)
                && u32_at(8)
                    .is_some_and(|fonts| (1..=256).contains(&fonts) && len >= 12 + 4 * fonts)
        }
        // WOFF and WOFF2 state their own length.
        b"wOFF" | b"wOF2" => {
            u32_at(8) == Some(len) && u16_at(12).is_some_and(|tables| (1..=128).contains(&tables))
        }
        _ => false,
    }
}

/// The start of an MPEG program stream, which VobSub's `.sub` is and so is a
/// `.mpg` or `.vob` video.
const PROGRAM_STREAM: &[u8] = b"\x00\x00\x01\xBA";

/// Whether the file at `path` is a VobSub index (`.idx`), which tells a
/// program stream that comes with it is VobSub's `.sub`.
pub fn is_vobsub_index(path: &Path) -> bool {
    let mut head = Vec::with_capacity(HEAD);
    let read = File::open(path).and_then(|file| file.take(HEAD as u64).read_to_end(&mut head));
    read.is_ok()
        && ascii_view(&head)
            .trim_ascii_start()
            .to_ascii_lowercase()
            .starts_with(b"# vobsub index file")
}

/// A PGS `.sup`: `PG`, a 4-byte PTS and DTS, then the segment type (palette,
/// object, presentation, window or end).
fn is_pgs(head: &[u8]) -> bool {
    head.starts_with(b"PG")
        && head
            .get(10)
            .is_some_and(|t| matches!(t, 0x14 | 0x15 | 0x16 | 0x17 | 0x80))
}

/// TTML/DFXP: an XML document whose root is `tt` in the TTML namespace.
fn is_ttml(lower: &[u8]) -> bool {
    (lower.starts_with(b"<?xml") || lower.starts_with(b"<tt"))
        && contains(lower, b"<tt")
        && contains(lower, b"w3.org/ns/ttml")
}

/// The text formats that are only stored: WebVTT, the VobSub index, MicroDVD.
fn is_text_subtitle(lower: &[u8]) -> bool {
    lower.starts_with(b"webvtt")
        || lower.starts_with(b"# vobsub index file")
        || lower
            .split(|b| *b == b'\n')
            .take(8)
            .any(|line| is_microdvd_line(line.trim_ascii()))
}

/// A MicroDVD line: `{start}{end}text`, in frames.
fn is_microdvd_line(line: &[u8]) -> bool {
    let number = |rest: &[u8]| -> Option<usize> {
        let rest = rest.strip_prefix(b"{")?;
        let end = rest.iter().position(|b| *b == b'}')?;
        let digits = &rest[..end];
        (!digits.is_empty() && digits.iter().all(u8::is_ascii_digit)).then_some(end + 2)
    };
    number(line).is_some_and(|first| number(&line[first..]).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::zip_of;

    const ASS: &[u8] = b"\xEF\xBB\xBF[Script Info]\nTitle: x\n\n[Events]\n";
    const SRT: &[u8] = b"1\r\n00:00:01,000 --> 00:00:02,500\r\nHello\r\n";
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01";

    fn judged(bytes: &[u8]) -> Verdict {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("whatever.ass");
        std::fs::write(&path, bytes).unwrap();
        judge(&path, &mut INFLATE_BUDGET.clone())
    }

    fn kept(kind: Kind, format: Format) -> Verdict {
        Verdict::Keep(Kept {
            kind,
            format,
            archive: None,
        })
    }

    fn kept_archive(format: Format, archive: Archive) -> Verdict {
        Verdict::Keep(Kept {
            kind: Kind::Archive,
            format,
            archive: Some(archive),
        })
    }

    fn dropped(bytes: &[u8]) -> String {
        match judged(bytes) {
            Verdict::Drop(reason) | Verdict::BadZip(reason) => reason,
            other => panic!("not dropped: {other:?}"),
        }
    }

    /// A font header with `tables` tables, padded to the length it needs.
    fn sfnt(magic: &[u8; 4], tables: u16) -> Vec<u8> {
        let mut bytes = magic.to_vec();
        bytes.extend(tables.to_be_bytes());
        bytes.resize(12 + 16 * usize::from(tables), 0);
        bytes
    }

    #[test]
    fn subtitles_are_told_by_their_bytes_whatever_the_file_is_named() {
        // Every file here is named `whatever.ass`.
        assert_eq!(judged(ASS), kept(Kind::Subtitle, Format::Ass));
        assert_eq!(judged(SRT), kept(Kind::Subtitle, Format::Srt));
        assert_eq!(judged(b"<SAMI>\n<BODY>"), kept(Kind::Subtitle, Format::Smi));
    }

    #[test]
    fn an_image_renamed_to_a_subtitle_is_dropped() {
        assert_eq!(dropped(PNG), NOT_THEM);
        assert_eq!(dropped(b"\xFF\xD8\xFF\xE0\x00\x10JFIF\x00\x01"), NOT_THEM);
        assert_eq!(dropped(b"GIF89a\x01\x00\x01\x00"), NOT_THEM);
    }

    #[test]
    fn what_is_neither_a_subtitle_nor_a_font_is_dropped_by_its_content() {
        assert_eq!(dropped(b"Read me first.\nThanks for watching."), NOT_THEM);
        assert_eq!(dropped(b"   \n \n"), NOT_THEM);
        assert!(dropped(b"").contains("비어"));
        assert!(dropped(b"<!DOCTYPE html><html><body>404</body></html>").contains("HTML"));
        // A video: a matroska header.
        assert_eq!(dropped(b"\x1A\x45\xDF\xA3\x9F\x42\x86\x81\x01"), NOT_THEM);
        // A font-like start that does not hold together (a table count the
        // file cannot hold) is not a font.
        assert_eq!(dropped(&sfnt(b"OTTO", 5)[..20]), NOT_THEM);
        assert_eq!(dropped(b"\x00\x01\x00\x00\x00\x00"), NOT_THEM);
    }

    #[test]
    fn the_formats_that_are_only_stored_are_kept_as_subtitles() {
        let other = kept(Kind::Subtitle, Format::Other);
        assert_eq!(
            judged(b"\xEF\xBB\xBFWEBVTT\n\n00:00:01.000 --> 00:00:02.500\nHello\n"),
            other
        );
        assert_eq!(
            judged(b"<?xml version=\"1.0\"?><tt xmlns=\"http://www.w3.org/ns/ttml\"><body/></tt>"),
            other
        );
        assert_eq!(judged(b"{120}{240}Hello|World\r\n{300}{360}Bye\r\n"), other);
        assert_eq!(
            judged(b"# VobSub index file, v7 (do not modify this line!)\n"),
            other
        );
        // A program stream is VobSub's only with its index: the caller decides.
        assert_eq!(
            judged(b"\x00\x00\x01\xBA\x44\x00\x04\x00\x04\x01"),
            Verdict::Stream
        );
        let mut pgs = b"PG\x00\x00\x00\x01\x00\x00\x00\x00".to_vec();
        pgs.extend([0x16, 0x00, 0x0B]);
        assert_eq!(judged(&pgs), other);
        // Text that only looks like one of them.
        assert_eq!(dropped(b"{not}{frames} text"), NOT_THEM);
        assert_eq!(dropped(b"PG is a rating"), NOT_THEM);
    }

    #[test]
    fn fonts_are_told_by_their_headers() {
        let font = kept(Kind::Font, Format::Other);
        assert_eq!(judged(&sfnt(b"\x00\x01\x00\x00", 12)), font);
        assert_eq!(judged(&sfnt(b"true", 9)), font);
        assert_eq!(judged(&sfnt(b"OTTO", 14)), font);

        let mut ttc = b"ttcf\x00\x02\x00\x00\x00\x00\x00\x02".to_vec();
        ttc.resize(64, 0);
        assert_eq!(judged(&ttc), font);

        for magic in [b"wOFF", b"wOF2"] {
            let mut woff = magic.to_vec();
            woff.extend([0, 1, 0, 0]);
            woff.extend(64u32.to_be_bytes());
            woff.extend(3u16.to_be_bytes());
            woff.resize(64, 0);
            assert_eq!(judged(&woff), font);
            // A WOFF that states another length than the file's is cut or padded.
            woff.push(0);
            assert_eq!(dropped(&woff), NOT_THEM);
        }
    }

    #[test]
    fn a_zip_is_kept_as_it_is_when_it_reads_to_its_end() {
        let zip = zip_of(&[("a.srt", SRT), ("readme.txt", b"hello")]);
        assert_eq!(judged(&zip), kept_archive(Format::Zip, Archive::Zip));
        // A ZIP is a ZIP whatever its name says, and its members are not judged here.
        let only_text = zip_of(&[("readme.txt", b"hello")]);
        assert_eq!(judged(&only_text), kept_archive(Format::Zip, Archive::Zip));
        // Cut short.
        assert!(dropped(&zip[..zip.len() / 2]).contains("ZIP"));
    }

    #[test]
    fn a_vobsub_index_is_told_by_its_content() {
        let dir = tempfile::tempdir().unwrap();
        let idx = dir.path().join("a.idx");
        std::fs::write(
            &idx,
            b"# VobSub index file, v7 (do not modify this line!)\n",
        )
        .unwrap();
        assert!(is_vobsub_index(&idx));
        let other = dir.path().join("b.idx");
        std::fs::write(&other, b"hello").unwrap();
        assert!(!is_vobsub_index(&other));
        assert!(!is_vobsub_index(&dir.path().join("missing.idx")));
    }

    #[test]
    fn zips_share_one_inflation_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.zip");
        // 64 KiB of text inflates to more than a 1 KiB budget.
        std::fs::write(&path, zip_of(&[("a.srt", &vec![b'a'; 64 * 1024])])).unwrap();
        let mut budget = 1024;
        assert!(matches!(
            judge(&path, &mut budget),
            Verdict::Drop(reason) if reason == crate::verify::INFLATE_BUDGET_SPENT
        ));
        // Spent budgets drop the next ZIP without reading it.
        let small = dir.path().join("b.zip");
        std::fs::write(&small, zip_of(&[("a.srt", SRT)])).unwrap();
        let mut spent = 0;
        assert!(matches!(
            judge(&small, &mut spent),
            Verdict::Drop(reason) if reason == crate::verify::INFLATE_BUDGET_SPENT
        ));
        // What a ZIP inflates is taken from the budget, and a ZIP within it is kept.
        let mut budget = 1 << 20;
        assert!(matches!(judge(&small, &mut budget), Verdict::Keep(_)));
        assert_eq!(budget, (1 << 20) - SRT.len() as u64);
    }

    #[test]
    fn archive_names_are_told_by_their_extension_split_volumes_included() {
        for name in [
            "a.zip",
            "a.rar",
            "Show/subs.7Z",
            "x.tar.gz",
            "x.tgz",
            "y.xz",
            "z.bz2",
            "z.tar",
            "pack.part1.rar",
            "pack.r00",
            "pack.R12",
            "pack.z01",
            "pack.7z.001",
            "pack.zip.002",
            "dir\\z.bz2",
        ] {
            assert!(is_archive_name(name), "{name}");
        }
        for name in [
            "a.ass",
            "rar",
            ".rar",
            "a.rar.txt",
            "a.001",
            "a.r1",
            "a.rxx",
            "a.7z.01",
            "",
        ] {
            assert!(!is_archive_name(name), "{name}");
        }
    }

    /// A file of `len` bytes that starts with `start`, and has `at` bytes `put` at `offset`.
    fn bytes_of(start: &[u8], put: &[u8], offset: usize) -> Vec<u8> {
        let mut bytes = start.to_vec();
        bytes.resize(600, 0);
        bytes[offset..offset + put.len()].copy_from_slice(put);
        bytes
    }

    #[test]
    fn a_later_volume_names_the_volume_that_vouches_for_it() {
        let cases = [
            ("Show/Pack.7z.002", Some("show/pack.7z.001")),
            ("pack.zip.003", Some("pack.zip.001")),
            ("pack.rar.0002", Some("pack.rar.0001")),
            ("pack.tar.002", Some("pack.tar.001")),
            ("a/b/pack.z01", Some("a/b/pack.zip")),
            ("pack.Z12", Some("pack.zip")),
            ("pack.r00", Some("pack.rar")),
            ("pack.R07", Some("pack.rar")),
            // The first volume, and what is no volume.
            ("pack.7z.001", None),
            ("pack.7z", None),
            ("pack.part2.rar", None),
            ("pack.002", None),
            ("pack.txt.002", None),
            ("pack.z00", None),
            ("pack.z1", None),
            ("pack.r1", None),
            ("pack.rar", None),
            ("dir.7z/pack.002", None),
            ("", None),
        ];
        for (name, anchor) in cases {
            assert_eq!(volume_anchor(name).as_deref(), anchor, "{name}");
        }
    }

    #[test]
    fn a_zip_that_cannot_be_read_alone_comes_back_as_bad_zip_not_dropped() {
        let zip = zip_of(&[("a.srt", SRT)]);
        assert!(matches!(judged(&zip[..zip.len() / 2]), Verdict::BadZip(_)));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.zip");
        std::fs::write(&path, &zip).unwrap();
        assert!(is_zip_signed(&path));
        std::fs::write(&path, b"P").unwrap();
        assert!(!is_zip_signed(&path));
        std::fs::write(&path, b"Rar!").unwrap();
        assert!(!is_zip_signed(&path));
        assert!(!is_zip_signed(&dir.path().join("missing")));
    }

    #[test]
    fn rar_7z_gzip_bzip2_xz_and_tar_are_kept_whole_by_their_magic_alone() {
        let cases: [(Vec<u8>, Archive); 8] = [
            (b"Rar!\x1A\x07\x00abc".to_vec(), Archive::Rar),
            (b"Rar!\x1A\x07\x01\x00abc".to_vec(), Archive::Rar),
            (b"7z\xBC\xAF\x27\x1C\x00\x04abc".to_vec(), Archive::SevenZip),
            (b"\x1F\x8B\x08\x00abc".to_vec(), Archive::Gzip),
            (b"BZh9abc".to_vec(), Archive::Bzip2),
            (b"\xFD7zXZ\x00\x00abc".to_vec(), Archive::Xz),
            (bytes_of(b"readme.txt", b"ustar\x0000", 257), Archive::Tar),
            (b"Rar!\x1A\x07\x00".to_vec(), Archive::Rar),
        ];
        for (bytes, archive) in cases {
            // Whatever the file is named, and without anything else checked.
            assert_eq!(
                judged(&bytes),
                kept_archive(Format::Other, archive),
                "{archive:?}"
            );
        }
    }

    #[test]
    fn what_only_looks_like_an_archive_is_not_one() {
        // A RAR header cut before its version byte, a gzip header of another method,
        // text that starts with `BZh`, a `ustar` in the wrong place.
        assert_eq!(dropped(b"Rar!\x1A\x07"), NOT_THEM);
        assert_eq!(dropped(b"\x1F\x8B\x07abc"), NOT_THEM);
        assert_eq!(dropped(b"BZh is a name"), NOT_THEM);
        assert_eq!(dropped(&bytes_of(b"hello", b"ustar", 200)), NOT_THEM);
        assert_eq!(dropped(b"7z\xBC\xAF"), NOT_THEM);
    }

    #[test]
    fn archive_types_have_codes_and_labels() {
        for archive in Archive::ALL {
            assert_eq!(Archive::parse(archive.code()), Some(archive));
        }
        assert_eq!(Archive::parse("iso"), None);
        assert_eq!(Archive::Rar.label(), "RAR");
    }

    #[test]
    fn a_file_over_the_byte_cap_is_dropped_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.srt");
        File::create(&path)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        assert!(
            matches!(judge(&path, &mut INFLATE_BUDGET.clone()), Verdict::Drop(reason) if reason.contains("200MiB"))
        );
    }

    #[test]
    fn kinds_have_codes_and_labels() {
        for kind in [Kind::Subtitle, Kind::Font, Kind::Archive] {
            assert_eq!(Kind::parse(kind.code()), Some(kind));
        }
        assert_eq!(Kind::parse("video"), None);
        assert_eq!(Kind::Font.label(), "폰트");
    }
}
