//! ZIP, and a ZIP cut into raw pieces (`.zip.001`, ...), with the `zip` crate.
//!
//! The end-of-central-directory record is read first, by this module, from the
//! tail of the file (the record, a comment of up to 64 KiB, the ZIP64 locator):
//! a ZIP with none there is refused as corrupt, so the crate is not left to
//! search the whole file, junk after the comment included. The record's member
//! total is checked against the limit before the crate parses a central
//! directory of that size, and a ZIP that says it spans disks (`.z01`) is not
//! unpacked.
//!
//! The crate may still use another record than the one read here: when the
//! last one in the tail is bad it falls back to an earlier one, which only a
//! crafted file has. Until the crate has parsed that directory, the member
//! limit does not hold for it; the memory limit and the OOM score of the
//! process are what stand behind. After the crate has parsed the central
//! directory, this module reads the directory's headers itself from where the
//! crate began: the crate keeps its members by name, so a name given twice is
//! one member to it, and the headers there must be as many as it kept. The
//! listing is then admitted whole, and the members are written after.

use std::{
    collections::HashSet,
    io::{BufReader, Read, Seek, SeekFrom},
    path::PathBuf,
};

use sha2::{Digest, Sha256};

use crate::{
    failure::from_zip,
    job::{Job, Kind},
    name::{decode, decode_lossy},
    source::Concat,
    ExtractError, Refusal,
};

const EOCD: &[u8] = b"PK\x05\x06";
const ZIP64_LOCATOR: &[u8] = b"PK\x06\x07";
const ZIP64_EOCD: &[u8] = b"PK\x06\x06";

/// What the end records say about the archive's shape.
struct End {
    total: u64,
    spans: bool,
}

fn u16_at(bytes: &[u8], at: usize) -> u64 {
    u64::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> u64 {
    u64::from(u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()))
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

/// Reads the end records of the archive `reader` holds, if it has any the
/// crate would not read differently.
fn read_end<R: Read + Seek>(reader: &mut R) -> std::io::Result<Option<End>> {
    let len = reader.seek(SeekFrom::End(0))?;
    // The record, a comment of up to 64 KiB and the ZIP64 locator before it.
    let tail_len = len.min(22 + 65535 + 20);
    reader.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = vec![0u8; tail_len as usize];
    reader.read_exact(&mut tail)?;
    let Some(at) = (0..=tail.len().saturating_sub(22)).rev().find(|&at| {
        tail.len() >= at + 22
            && &tail[at..at + 4] == EOCD
            && at + 22 + u16_at(&tail, at + 20) as usize <= tail.len()
    }) else {
        return Ok(None);
    };
    let (disk, directory_disk, on_disk, total) = (
        u16_at(&tail, at + 4),
        u16_at(&tail, at + 6),
        u16_at(&tail, at + 8),
        u16_at(&tail, at + 10),
    );
    let mut end = End {
        total,
        spans: disk != 0 || directory_disk != 0 || on_disk != total,
    };
    if at >= 20 && &tail[at - 20..at - 16] == ZIP64_LOCATOR {
        let locator = &tail[at - 20..at];
        let many_disks = u32_at(locator, 16) > 1;
        let mut record = [0u8; 56];
        let read = reader.seek(SeekFrom::Start(u64_at(locator, 8))).is_ok()
            && reader.read_exact(&mut record).is_ok()
            && &record[..4] == ZIP64_EOCD;
        if read {
            end.total = u64_at(&record, 32);
            end.spans = many_disks
                || u32_at(&record, 16) != 0
                || u32_at(&record, 20) != 0
                || u64_at(&record, 24) != end.total;
        } else if many_disks {
            end.spans = true;
        }
    }
    Ok(Some(end))
}

/// How many central-directory headers the directory the crate used holds, and
/// whether one of them names a path another did: reads the consecutive
/// headers (`PK\x01\x02`) from `start`, at most `count + 1` of them, and
/// refuses with [`Refusal::Duplicate`] when a raw name repeats.
///
/// The crate keeps its members by name, so a name given twice is one member to
/// it; and which end record it takes the directory from is its own search,
/// which the member total of [`read_end`] may not be about. `start` is where
/// the crate began reading (absolute) and `count` how many members it kept.
/// The headers that are there must be as many as it kept.
fn check_directory<R: Read + Seek>(mut reader: R, start: u64, count: u64) -> Result<(), Refusal> {
    const HEADER: &[u8] = b"PK\x01\x02";
    const FIXED: usize = 46;
    let unreadable = |error: std::io::Error| {
        Refusal::corrupt(format!("ZIP의 멤버 목록을 읽지 못했어요: {error}"))
    };
    // The digests of the names, not the names: a name may be 64 KiB.
    let mut seen = HashSet::new();
    let mut headers = 0u64;
    reader.seek(SeekFrom::Start(start)).map_err(unreadable)?;
    while headers <= count {
        let mut fixed = [0u8; FIXED];
        match reader.read_exact(&mut fixed) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(unreadable(error)),
        }
        if &fixed[..4] != HEADER {
            break;
        }
        let mut name = vec![0u8; u16_at(&fixed, 28) as usize];
        let skipped = u16_at(&fixed, 30) + u16_at(&fixed, 32);
        match reader.read_exact(&mut name) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(unreadable(error)),
        }
        headers += 1;
        if !seen.insert(Sha256::digest(&name)) {
            return Err(Refusal::Duplicate {
                path: decode_lossy(&name),
            });
        }
        reader
            .seek(SeekFrom::Current(skipped as i64))
            .map_err(unreadable)?;
    }
    if headers != count {
        return Err(Refusal::corrupt("ZIP의 멤버 목록이 끝 기록과 맞지 않아요"));
    }
    Ok(())
}

/// Unpacks the ZIP `parts` hold. `split` is whether it comes in pieces, whose
/// structure being invalid means a piece is missing.
pub(crate) fn extract(job: &Job, parts: &[PathBuf], split: bool) -> Result<(), ExtractError> {
    let concat =
        Concat::open(parts).map_err(|error| job.input_error("파일을 열지 못했어요", error))?;
    let mut reader = BufReader::new(concat);
    // An archive whose end record is not in the tail is not handed to the
    // crate: it searches the whole file for one, takes any junk after the
    // comment and falls back to an earlier record, so what it would read may
    // be a central directory this module did not look at.
    let Some(end) =
        read_end(&mut reader).map_err(|error| job.input_error("파일을 읽지 못했어요", error))?
    else {
        return Err(if split {
            Refusal::MissingVolume
        } else {
            Refusal::corrupt("ZIP의 끝을 찾지 못했어요")
        }
        .into());
    };
    if end.spans {
        return Err(Refusal::unsupported("여러 디스크로 나뉜 ZIP(.z01)").into());
    }
    job.has_room_for(end.total)?;
    reader
        .rewind()
        .map_err(|error| job.input_error("파일을 읽지 못했어요", error))?;
    let mut archive = zip::ZipArchive::new(reader).map_err(|error| from_zip(&error, split))?;
    job.has_room_for(archive.len() as u64)?;
    // The directory the crate read, which a crafted file can make another one
    // than the end record read here counts.
    let directory =
        Concat::open(parts).map_err(|error| job.input_error("파일을 열지 못했어요", error))?;
    check_directory(
        BufReader::new(directory),
        archive.central_directory_start(),
        archive.len() as u64,
    )?;

    let mut planner = job.planner();
    let mut planned = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|error| from_zip(&error, split))?;
        let raw = entry.name_raw().to_vec();
        let lossy = entry.name().to_owned();
        let mode = entry.unix_mode().map(|mode| mode & 0o170000);
        let (is_dir, encrypted, size) = (entry.is_dir(), entry.encrypted(), entry.size());
        drop(entry);

        if encrypted {
            return Err(Refusal::Encrypted.into());
        }
        let kind = match mode {
            Some(0o120000) => Kind::Link,
            Some(0o040000) => Kind::Dir,
            _ if is_dir => Kind::Dir,
            Some(0o100000 | 0) | None => Kind::File,
            Some(_) => Kind::Special,
        };
        let name = decode(&raw, || lossy);
        let name = if kind == Kind::Dir {
            name.strip_suffix('/').unwrap_or(&name)
        } else {
            &name
        };
        let declared = (kind == Kind::File).then_some(size);
        if let Some(member) = planner.admit(name, kind, declared)? {
            planned.push((index, member));
        }
    }

    for (index, member) in planned {
        let mut sink = job.open(member)?;
        {
            let mut entry = archive
                .by_index(index)
                .map_err(|error| from_zip(&error, false))?;
            sink.copy(&mut entry)
                .map_err(|error| job.copy_failed(error))?;
        }
        sink.finish()?;
    }
    Ok(())
}
