//! Whether received bytes are a file (`docs/specs/jobs.md`, 공통 수신 결과와
//! 실패 분류), for every source alike. A job checks a file's bytes with
//! [`check`] before it counts them as received, after a restart as well.
//!
//! - Nothing, or a web page (after a BOM and blanks it starts with `<`, and
//!   it is no SMI), is not a file. A site's error page is often `200`.
//! - A file over [`MAX_FILE_BYTES`] is not one.
//! - A ZIP (`PK\x03\x04`) is opened and every member read to its end, so each
//!   CRC is checked. Before the archive is opened, the member count its end
//!   record claims is held to [`ZIP_MAX_MEMBERS`] and to what the file's
//!   length can hold, so a crafted directory costs nothing. What the members are is the analysis's to judge (a font
//!   ZIP is a file too); only a member this build cannot read (another
//!   compression, a password) is left unchecked.
//! - ASS (`[Script Info]`), SMI (`<SAMI`) and SRT (a `-->` time line) are
//!   told by their first bytes, in UTF-8 or UTF-16 with its BOM, or a legacy
//!   encoding whose ASCII is ASCII. WebVTT (`WEBVTT`) has SRT's time lines
//!   but is not SRT. Anything else is `other`: kept, for the analysis to
//!   decide.
//! - A name whose extension promises one of those formats must have its
//!   bytes.
//!
//! The length against the announced one is the job's check, as the bytes
//! come.

use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

use crate::{Failure, FailureKind, MAX_FILE_BYTES};

/// What received bytes are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Zip,
    Ass,
    Srt,
    Smi,
    /// Not one of the four, and not a web page: the analysis decides.
    Other,
}

impl Format {
    pub const ALL: [Format; 5] = [
        Format::Zip,
        Format::Ass,
        Format::Srt,
        Format::Smi,
        Format::Other,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::Ass => "ass",
            Format::Srt => "srt",
            Format::Smi => "smi",
            Format::Other => "other",
        }
    }

    pub fn parse(code: &str) -> Option<Format> {
        Format::ALL.into_iter().find(|f| f.code() == code)
    }

    /// What the screens and the log call it.
    pub fn label(self) -> &'static str {
        match self {
            Format::Zip => "ZIP",
            Format::Ass => "ASS",
            Format::Srt => "SRT",
            Format::Smi => "SMI",
            Format::Other => "그 밖",
        }
    }

    /// The format a file name's extension promises, if any.
    pub fn promised_by(name: &str) -> Option<Format> {
        let (_, ext) = name.rsplit_once('.')?;
        match ext.to_ascii_lowercase().as_str() {
            "zip" => Some(Format::Zip),
            "ass" | "ssa" => Some(Format::Ass),
            "srt" => Some(Format::Srt),
            "smi" | "sami" => Some(Format::Smi),
            _ => None,
        }
    }
}

/// How many bytes of a ZIP's members [`check`] reads at most: far more than
/// any subtitle ZIP, and a bound on what an archive that inflates without end
/// can cost.
pub const ZIP_MAX_BYTES: u64 = 1 << 30;
/// How many members a ZIP may have.
pub const ZIP_MAX_MEMBERS: usize = 10_000;

/// How much of the start [`sniff`] looks at.
const HEAD: usize = 8 * 1024;

/// What the start of some bytes says they are, before a ZIP's members are
/// read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    Empty,
    Html,
    Format(Format),
}

/// Checks the file at `path`, received under `name`: its format, or why it is
/// not a file (always [`FailureKind::NotAFile`]). Blocking: it reads the file.
pub fn check(path: &Path, name: &str) -> Result<Format, Failure> {
    let mut unlimited = u64::MAX;
    check_within(path, name, &mut unlimited)
}

/// [`check`] whose ZIP check counts what it inflates against `budget`, which
/// several files share ([`check_zip_within`]): an archive's members, so that
/// many ZIPs among them cost no more than one budget.
pub fn check_within(path: &Path, name: &str, budget: &mut u64) -> Result<Format, Failure> {
    let not_a_file = |reason: String| Failure::new(FailureKind::NotAFile, reason);
    let unreadable = |_: io::Error| not_a_file("받은 파일을 읽지 못했어요".to_owned());
    let mut file = File::open(path).map_err(unreadable)?;
    let len = file.metadata().map_err(unreadable)?.len();
    if len > MAX_FILE_BYTES {
        return Err(not_a_file(format!(
            "받은 파일이 {} MB를 넘어 파일로 받지 않아요",
            MAX_FILE_BYTES >> 20
        )));
    }
    let mut head = Vec::with_capacity(HEAD);
    (&mut file)
        .take(HEAD as u64)
        .read_to_end(&mut head)
        .map_err(unreadable)?;
    drop(file);

    let format = match sniff(&head) {
        Sniffed::Empty => return Err(not_a_file("받은 파일이 비어 있어요".to_owned())),
        Sniffed::Html => {
            return Err(not_a_file(
                "받은 내용이 파일이 아니라 웹 페이지(HTML)예요".to_owned(),
            ))
        }
        Sniffed::Format(Format::Zip) => {
            check_zip_within(path, budget)?;
            Format::Zip
        }
        Sniffed::Format(format) => format,
    };
    if let Some(promised) = Format::promised_by(name).filter(|p| *p != format) {
        return Err(not_a_file(format!(
            "이름은 {} 파일인데 받은 바이트는 {} 형식이 아니에요",
            promised.label(),
            promised.label()
        )));
    }
    Ok(format)
}

/// What the first bytes say (see the module docs).
pub fn sniff(head: &[u8]) -> Sniffed {
    if head.is_empty() {
        return Sniffed::Empty;
    }
    if head.starts_with(b"PK\x03\x04") {
        return Sniffed::Format(Format::Zip);
    }
    let text = ascii_view(head);
    let text = text.trim_ascii_start();
    if text.is_empty() {
        return Sniffed::Format(Format::Other);
    }
    let lower = text.to_ascii_lowercase();
    if lower.starts_with(b"<") {
        // An SMI may open with a comment before `<SAMI>`; a web page has none.
        return match contains(&lower, b"<sami") {
            true => Sniffed::Format(Format::Smi),
            false => Sniffed::Html,
        };
    }
    if lower.starts_with(b"[script info]") {
        return Sniffed::Format(Format::Ass);
    }
    // WebVTT's cues have SRT's time lines.
    if lower.starts_with(b"webvtt") {
        return Sniffed::Format(Format::Other);
    }
    if lower.split(|b| *b == b'\n').any(is_time_line) {
        return Sniffed::Format(Format::Srt);
    }
    Sniffed::Format(Format::Other)
}

/// The text's ASCII, a byte per character: UTF-16 by its BOM (anything not
/// ASCII becomes `0xFF`), a UTF-8 BOM dropped, other bytes as they are.
pub(crate) fn ascii_view(head: &[u8]) -> Vec<u8> {
    let utf16 = |rest: &[u8], le: bool| {
        rest.as_chunks::<2>()
            .0
            .iter()
            .map(|&[a, b]| {
                let (lo, hi) = match le {
                    true => (a, b),
                    false => (b, a),
                };
                match hi == 0 && lo.is_ascii() {
                    true => lo,
                    false => 0xFF,
                }
            })
            .collect()
    };
    match head {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => rest.to_vec(),
        _ => head.to_vec(),
    }
}

pub(crate) fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// An SRT time line: `00:00:01,000 --> 00:00:02,500`.
fn is_time_line(line: &[u8]) -> bool {
    let Some(at) = line.windows(3).position(|w| w == b"-->") else {
        return false;
    };
    is_timestamp(line[..at].trim_ascii()) && {
        let after = line[at + 3..].trim_ascii();
        let end = after
            .iter()
            .position(|b| b.is_ascii_whitespace())
            .unwrap_or(after.len());
        is_timestamp(&after[..end])
    }
}

/// `hh:mm:ss,mmm` (or with a `.`).
fn is_timestamp(text: &[u8]) -> bool {
    let digits = |s: &[u8]| !s.is_empty() && s.iter().all(u8::is_ascii_digit);
    let parts: Vec<&[u8]> = text.split(|b| *b == b':').collect();
    let [h, m, s] = parts.as_slice() else {
        return false;
    };
    let Some(sep) = s.iter().position(|b| *b == b',' || *b == b'.') else {
        return false;
    };
    digits(h) && digits(m) && digits(&s[..sep]) && digits(&s[sep + 1..])
}

/// Reads every member of the ZIP at `path` to its end, which checks its CRC.
pub fn check_zip(path: &Path) -> Result<(), Failure> {
    let mut unlimited = u64::MAX;
    check_zip_within(path, &mut unlimited)
}

/// The reason a ZIP is not read because the bytes its upload may have
/// inflated in all are spent.
pub const INFLATE_BUDGET_SPENT: &str =
    "한 번에 올린 압축 파일을 확인할 수 있는 양을 넘어서 이 압축 파일은 받지 않아요";

/// [`check_zip`] that also counts what it inflates against `budget`, which
/// several ZIPs share: a ZIP that would take more than is left fails with
/// [`INFLATE_BUDGET_SPENT`], and what was read is taken from the budget.
pub fn check_zip_within(path: &Path, budget: &mut u64) -> Result<(), Failure> {
    let not_a_file = |reason: String| Failure::new(FailureKind::NotAFile, reason);
    let mut file =
        File::open(path).map_err(|_| not_a_file("받은 파일을 읽지 못했어요".to_owned()))?;
    let claimed = claimed_members(&mut file)
        .map_err(|_| not_a_file("받은 파일을 읽지 못했어요".to_owned()))?;
    match claimed {
        Claimed::NoEnd => return Err(not_a_file("ZIP으로 시작하지만 열 수 없어요".to_owned())),
        Claimed::TooMany(count) => {
            return Err(not_a_file(format!(
                "ZIP이 담았다고 하는 파일 수({count}개)가 너무 많거나 파일 크기와 맞지 않아요"
            )))
        }
        Claimed::Within => {}
    }
    if *budget == 0 {
        return Err(not_a_file(INFLATE_BUDGET_SPENT.to_owned()));
    }
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| not_a_file("ZIP으로 시작하지만 열 수 없어요".to_owned()))?;
    if archive.len() > ZIP_MAX_MEMBERS {
        return Err(not_a_file(format!(
            "ZIP에 파일이 너무 많아요 ({}개)",
            archive.len()
        )));
    }
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut member = match archive.by_index(i) {
            Ok(member) => member,
            // Another compression or a password: not this check's to judge.
            Err(zip::result::ZipError::UnsupportedArchive(_)) => continue,
            Err(_) => {
                return Err(not_a_file(format!(
                    "ZIP 안 {}번째 파일을 읽지 못했어요",
                    i + 1
                )))
            }
        };
        if member.is_dir() {
            continue;
        }
        let left = ZIP_MAX_BYTES - total;
        // The upload's budget may be the nearer limit.
        let shared = *budget < left;
        let allowed = left.min(*budget);
        let read =
            io::copy(&mut (&mut member).take(allowed + 1), &mut io::sink()).map_err(|_| {
                not_a_file(format!(
                    "ZIP 안 {}번째 파일이 손상됐어요 (CRC가 맞지 않거나 끝까지 읽히지 않아요)",
                    i + 1
                ))
            })?;
        *budget -= read.min(*budget);
        total += read;
        if read > allowed {
            return Err(not_a_file(match shared {
                true => INFLATE_BUDGET_SPENT.to_owned(),
                false => "ZIP을 풀면 확인할 수 있는 크기를 넘어요".to_owned(),
            }));
        }
    }
    Ok(())
}

/// What a ZIP's end records claim about its member count.
#[derive(Debug, PartialEq, Eq)]
enum Claimed {
    /// No end record: no ZIP.
    NoEnd,
    /// More members than [`ZIP_MAX_MEMBERS`], or than the file's length can
    /// hold (a central directory entry is at least 46 bytes).
    TooMany(u64),
    Within,
}

/// The end of central directory record's signature, and ZIP64's locator and
/// record.
const EOCD: &[u8; 4] = b"PK\x05\x06";
const ZIP64_LOCATOR: &[u8; 4] = b"PK\x06\x07";
const ZIP64_EOCD: &[u8; 4] = b"PK\x06\x06";
/// An end record's length without its comment, and the longest comment.
const EOCD_LEN: usize = 22;
const MAX_COMMENT: usize = u16::MAX as usize;
const CENTRAL_ENTRY_MIN: u64 = 46;

/// Reads the member count every end record in the file's tail claims (ZIP64's
/// when it has one), before the archive is opened: an archive reader sizes
/// its directory by it. Every candidate is held to the bounds, so a record
/// hidden in a comment cannot pass one and let the reader take another.
fn claimed_members(file: &mut File) -> io::Result<Claimed> {
    use std::io::{Seek, SeekFrom};
    let len = file.metadata()?.len();
    let tail_len = len.min((EOCD_LEN + MAX_COMMENT) as u64);
    let tail_at = len - tail_len;
    file.seek(SeekFrom::Start(tail_at))?;
    let mut tail = Vec::with_capacity(tail_len as usize);
    (&mut *file).take(tail_len).read_to_end(&mut tail)?;
    let u16_at = |b: &[u8], at: usize| u16::from_le_bytes([b[at], b[at + 1]]);
    let u64_at =
        |b: &[u8], at: usize| u64::from_le_bytes(b[at..at + 8].try_into().expect("eight bytes"));

    let mut found = false;
    let mut claims = Vec::new();
    for at in (0..tail.len().saturating_sub(EOCD_LEN - 1)).rev() {
        if &tail[at..at + 4] != EOCD {
            continue;
        }
        found = true;
        claims.push(u64::from(u16_at(&tail, at + 10)));
        // ZIP64: a locator just before the record points at the ZIP64 record,
        // whose count is the real one.
        if at >= 20 && &tail[at - 20..at - 16] == ZIP64_LOCATOR {
            let record = u64_at(&tail, at - 20 + 8);
            if record.checked_add(56).is_some_and(|end| end <= len) {
                let mut head = [0u8; 40];
                file.seek(SeekFrom::Start(record))?;
                file.read_exact(&mut head)?;
                if &head[..4] == ZIP64_EOCD {
                    claims.push(u64_at(&head, 32));
                }
            }
        }
    }
    if !found {
        return Ok(Claimed::NoEnd);
    }
    let room = len / CENTRAL_ENTRY_MIN;
    Ok(
        match claims
            .into_iter()
            .find(|&n| n > ZIP_MAX_MEMBERS as u64 || n > room)
        {
            Some(count) => Claimed::TooMany(count),
            None => Claimed::Within,
        },
    )
}

#[cfg(test)]
mod tests {
    use trss_archive::testing::{deflated_zip, zip_of, Method};

    use super::*;

    const ASS: &[u8] = b"\xEF\xBB\xBF[Script Info]\nTitle: x\n\n[Events]\n";
    const SRT: &[u8] = b"1\r\n00:00:01,000 --> 00:00:02,500\r\nHello\r\n";

    fn checked(name: &str, bytes: &[u8]) -> Result<Format, FailureKind> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, bytes).unwrap();
        check(&path, name).map_err(|f| f.kind)
    }

    fn utf16le(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        bytes
    }

    #[test]
    fn subtitle_formats_are_told_by_their_first_bytes() {
        assert_eq!(checked("a.ass", ASS), Ok(Format::Ass));
        assert_eq!(checked("a.srt", SRT), Ok(Format::Srt));
        assert_eq!(checked("a.smi", b"<SAMI>\n<BODY>"), Ok(Format::Smi));
        assert_eq!(
            checked("a.smi", &utf16le("<SAMI>\n<BODY>가")),
            Ok(Format::Smi)
        );
        assert_eq!(
            checked("a.smi", b"<!-- made by x -->\r\n<sami>"),
            Ok(Format::Smi)
        );
        // An SRT in a legacy encoding still has ASCII time lines.
        assert_eq!(
            checked("a.srt", b"1\n00:00:01.000 --> 00:00:02.000\n\xC7\xD1"),
            Ok(Format::Srt)
        );
        // Bytes no format names, under a name that promises none: kept.
        assert_eq!(checked("font.ttf", b"\x00\x01\x00\x00"), Ok(Format::Other));
        assert_eq!(checked("notes.txt", b"just text"), Ok(Format::Other));
    }

    #[test]
    fn a_webvtt_file_is_other_not_srt() {
        let vtt = b"\xEF\xBB\xBFWEBVTT\n\n00:00:01.000 --> 00:00:02.500 align:start\nHello\n";
        assert_eq!(sniff(vtt), Sniffed::Format(Format::Other));
        assert_eq!(checked("a.vtt", vtt), Ok(Format::Other));
        // Under a name that promises SRT, its bytes are not SRT.
        assert_eq!(checked("a.srt", vtt), Err(FailureKind::NotAFile));
    }

    #[test]
    fn a_zip_whose_end_record_claims_too_many_members_is_refused_before_it_is_opened() {
        let zip = deflated_zip(&[("a.srt", SRT)]);
        let end = zip.windows(4).rposition(|w| w == EOCD).unwrap();
        let with_count = |count: u16| {
            let mut crafted = zip.clone();
            crafted[end + 8..end + 10].copy_from_slice(&count.to_le_bytes());
            crafted[end + 10..end + 12].copy_from_slice(&count.to_le_bytes());
            crafted
        };
        let path = |bytes: &[u8]| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("f");
            std::fs::write(&path, bytes).unwrap();
            (dir, path)
        };
        let claimed = |bytes: &[u8]| {
            let (_dir, at) = path(bytes);
            claimed_members(&mut File::open(at).unwrap()).unwrap()
        };
        assert_eq!(claimed(&zip), Claimed::Within);
        // More than the archive's length can hold, and more than the limit.
        assert_eq!(claimed(&with_count(40)), Claimed::TooMany(40));
        assert_eq!(claimed(&with_count(u16::MAX)), Claimed::TooMany(65535));
        assert_eq!(
            checked("a.zip", &with_count(u16::MAX)),
            Err(FailureKind::NotAFile)
        );
        // A second end record hidden in the comment is held to the bounds too.
        let mut commented = zip.clone();
        let mut fake = EOCD.to_vec();
        fake.extend([0u8; 6]);
        fake.extend(60000u16.to_le_bytes());
        fake.extend([0u8; 10]);
        commented[end + 20..end + 22].copy_from_slice(&(fake.len() as u16).to_le_bytes());
        commented.extend(&fake);
        assert_eq!(claimed(&commented), Claimed::TooMany(60000));
        assert_eq!(claimed(b"PK\x03\x04 no end"), Claimed::NoEnd);
    }

    #[test]
    fn a_file_over_the_byte_cap_is_not_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.srt");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        let failure = check(&path, "big.srt").unwrap_err();
        assert_eq!(failure.kind, FailureKind::NotAFile);
        assert!(failure.reason.contains("200 MB"));
    }

    #[test]
    fn nothing_a_web_page_or_bytes_unlike_their_name_are_not_a_file() {
        assert_eq!(checked("a.zip", b""), Err(FailureKind::NotAFile));
        for page in [
            &b"<!DOCTYPE html><html><body>404</body></html>"[..],
            b"\n  <html><head><title>Error</title>",
            b"\xEF\xBB\xBF<h1>Not Found</h1>",
        ] {
            assert_eq!(checked("a.zip", page), Err(FailureKind::NotAFile));
            assert_eq!(checked("a.bin", page), Err(FailureKind::NotAFile));
        }
        assert_eq!(checked("a.zip", ASS), Err(FailureKind::NotAFile));
        assert_eq!(checked("a.srt", ASS), Err(FailureKind::NotAFile));
        assert_eq!(checked("a.ass", b"plain"), Err(FailureKind::NotAFile));
    }

    #[test]
    fn files_checked_within_one_budget_share_what_their_zips_inflate() {
        let dir = tempfile::tempdir().unwrap();
        let member = vec![b'x'; 1000];
        let zip = deflated_zip(&[("a.txt", &member)]);
        let (first, second) = (dir.path().join("a.docx"), dir.path().join("b.docx"));
        std::fs::write(&first, &zip).unwrap();
        std::fs::write(&second, &zip).unwrap();
        let mut budget = 1500;
        assert_eq!(
            check_within(&first, "a.docx", &mut budget).map_err(|f| f.reason),
            Ok(Format::Zip)
        );
        assert_eq!(budget, 500);
        assert_eq!(
            check_within(&second, "b.docx", &mut budget).map_err(|f| f.reason),
            Err(INFLATE_BUDGET_SPENT.to_owned())
        );
        // Alone, each passes.
        assert_eq!(
            check(&second, "b.docx").map_err(|f| f.reason),
            Ok(Format::Zip)
        );
    }

    #[test]
    fn a_zip_passes_when_every_member_has_its_crc_and_any_members_will_do() {
        let zip = deflated_zip(&[("Seihantai - 24.srt", SRT), ("font.ttf", b"\x00\x01")]);
        assert_eq!(checked("Seihantai - 24.zip", &zip), Ok(Format::Zip));
        // Under a name with no extension it is a ZIP all the same.
        assert_eq!(checked("download", &zip), Ok(Format::Zip));

        // One flipped byte of a member's data fails its CRC (or its inflate).
        let stored = zip_of(&[("a.srt", SRT)], Method::Stored);
        let at = stored.windows(3).position(|w| w == b"-->").unwrap();
        let mut broken = stored.clone();
        broken[at] = b'=';
        assert_eq!(checked("a.zip", &stored), Ok(Format::Zip));
        assert_eq!(checked("a.zip", &broken), Err(FailureKind::NotAFile));
        // Cut short: no central directory.
        assert_eq!(
            checked("a.zip", &zip[..zip.len() / 2]),
            Err(FailureKind::NotAFile)
        );
    }
}
