//! 7z, and a 7z cut into raw pieces (`.7z.001`, ...), with `sevenz-rust2`.
//!
//! The crate reads a block's dictionary size from its properties and
//! allocates it whole before it decodes anything, so the coders of every
//! block are read first and a dictionary (or PPMd memory) over the limit is
//! refused then, as is an encrypted block. The listing (names, kinds, sizes) is
//! admitted whole before the first member is decoded.
//!
//! The crate allocates about 96 bytes per file the header declares, before this
//! module sees the listing, and a header can declare as many as it has bytes
//! for. So the size of the header, which the start header gives, is checked
//! first against [`header_bound`], which keeps what a crafted one makes the
//! crate allocate to about 100 MiB at the default limits.
//! An encoded (compressed) header is as long as its packed form, so a small
//! one can still declare many files: that is held to the memory limit of the
//! process and not to the member limit, until the crate has parsed it.

use std::{
    collections::HashMap,
    io::{BufReader, Read, Seek},
    path::PathBuf,
};

use sevenz_rust2::{ArchiveReader, Error, Password};

use crate::{
    failure::from_read,
    job::{Job, Kind},
    source::Concat,
    ExtractError, Limits, Refusal,
};

/// The first bytes of a 7z.
const SIGNATURE: &[u8] = b"7z\xbc\xaf\x27\x1c";

/// The method IDs the limits are about.
const AES: &[u8] = &[0x06, 0xf1, 0x07, 0x01];
const LZMA: &[u8] = &[0x03, 0x01, 0x01];
const LZMA2: &[u8] = &[0x21];
const PPMD: &[u8] = &[0x03, 0x04, 0x01];

/// `FILE_ATTRIBUTE_REPARSE_POINT`, and the flag that the high 16 bits hold a
/// Unix mode.
const REPARSE_POINT: u32 = 0x400;
const UNIX_EXTENSION: u32 = 0x8000;

/// The dictionary (or model memory) a coder asks for, if it is one that has one.
fn memory_of(id: &[u8], properties: &[u8]) -> Option<u64> {
    if id == LZMA2 {
        // One byte: the dictionary is 2 or 3 times a power of two.
        let bits = u64::from(*properties.first()?);
        Some(if bits >= 40 {
            u64::from(u32::MAX)
        } else {
            (2 | (bits & 1)) << (bits / 2 + 11)
        })
    } else if id == LZMA {
        // `lc/lp/pb`, then the dictionary size as 4 bytes.
        let dictionary: [u8; 4] = properties.get(1..5)?.try_into().ok()?;
        Some(u64::from(u32::from_le_bytes(dictionary)))
    } else if id == PPMD {
        // The order, then the memory size as 4 bytes.
        let memory: [u8; 4] = properties.get(1..5)?.try_into().ok()?;
        Some(u64::from(u32::from_le_bytes(memory)))
    } else {
        None
    }
}

/// The most bytes the header of a 7z may have: 512 for each member the limits
/// allow and 64 KiB for the rest (about 1 MiB at 2000 members). The crate takes
/// about 96 bytes of memory per declared file whatever its name, so this is
/// what bounds the memory a header that only declares files can take
/// (about 100 MiB). It is an allowance for names averaging about 250 bytes
/// (UTF-16 with its properties), far beyond a subtitle package's, not a limit
/// of a listing: a legitimate listing over it is refused as too big as well.
fn header_bound(limits: &Limits) -> u64 {
    limits.members.saturating_mul(512).saturating_add(64 << 10)
}

/// The error for an error of the crate.
fn error_of(error: Error, split: bool, limits: &Limits) -> ExtractError {
    match error {
        Error::PasswordRequired | Error::MaybeBadPassword(_) => Refusal::Encrypted,
        Error::UnsupportedCompressionMethod(method) => {
            if method.to_ascii_uppercase().contains("AES") {
                Refusal::Encrypted
            } else {
                Refusal::unsupported(format!("7z {method}"))
            }
        }
        Error::Unsupported(what) => Refusal::unsupported(what),
        Error::ExternalUnsupported => Refusal::unsupported("7z 바깥에 둔 헤더"),
        Error::MaxMemLimited { .. } => Refusal::Dictionary {
            limit: limits.dictionary,
        },
        // A piece of a split archive missing leaves its header or its end
        // cut short.
        Error::BadSignature(_) | Error::NextHeaderCrcMismatch if split => Refusal::MissingVolume,
        Error::Io(error, _) | Error::FileOpen(error, _) => {
            if split && error.kind() == std::io::ErrorKind::UnexpectedEof {
                Refusal::MissingVolume
            } else {
                return from_read(error, limits.dictionary);
            }
        }
        other => Refusal::corrupt(other),
    }
    .into()
}

/// Unpacks the 7z `parts` hold. `split` is whether it comes in pieces, a cut
/// short header of which means a piece is missing.
pub(crate) fn extract(job: &Job, parts: &[PathBuf], split: bool) -> Result<(), ExtractError> {
    let limits = job.limits();
    let mut concat =
        Concat::open(parts).map_err(|error| job.input_error("파일을 열지 못했어요", error))?;
    // The start header says where the end header is. Past the data is an
    // archive cut short, a missing piece when it comes in pieces.
    let mut start = [0u8; 32];
    if concat.read_exact(&mut start).is_ok() && start.starts_with(SIGNATURE) {
        let offset = u64::from_le_bytes(start[12..20].try_into().unwrap());
        let size = u64::from_le_bytes(start[20..28].try_into().unwrap());
        if 32u64.saturating_add(offset).saturating_add(size) > concat.len() {
            return Err(if split {
                Refusal::MissingVolume
            } else {
                Refusal::corrupt("7z가 중간에 끝났어요")
            }
            .into());
        }
        if size > header_bound(&limits) {
            return Err(Refusal::corrupt("7z 목록이 너무 커요").into());
        }
    }
    concat
        .rewind()
        .map_err(|error| job.input_error("파일을 읽지 못했어요", error))?;
    let mut archive = ArchiveReader::new(BufReader::new(concat), Password::empty())
        .map_err(|error| error_of(error, split, &limits))?;
    // The crate decodes a multithreaded LZMA2 stream on threads otherwise.
    archive.set_thread_count(1);

    let listing = archive.archive();
    job.has_room_for(listing.files.len() as u64)?;
    for block in &listing.blocks {
        for coder in &block.coders {
            let id = coder.encoder_method_id();
            if id == AES {
                return Err(Refusal::Encrypted.into());
            }
            if memory_of(id, coder.properties()).is_some_and(|memory| memory > limits.dictionary) {
                return Err(Refusal::Dictionary {
                    limit: limits.dictionary,
                }
                .into());
            }
        }
        if block.get_unpack_size() > limits.total {
            return Err(Refusal::TooLarge {
                limit: limits.total,
            }
            .into());
        }
    }

    let mut planner = job.planner();
    let mut planned = HashMap::new();
    for entry in &listing.files {
        let attributes = entry
            .has_windows_attributes
            .then_some(entry.windows_attributes);
        let unix_mode = attributes
            .filter(|attributes| attributes & UNIX_EXTENSION != 0)
            .map(|attributes| (attributes >> 16) & 0o170000);
        let kind = if entry.is_anti_item {
            Kind::Special
        } else if attributes.is_some_and(|attributes| attributes & REPARSE_POINT != 0)
            || unix_mode == Some(0o120000)
        {
            Kind::Link
        } else if entry.is_directory || unix_mode == Some(0o040000) {
            Kind::Dir
        } else if matches!(unix_mode, None | Some(0o100000 | 0)) {
            Kind::File
        } else {
            Kind::Special
        };
        let name = entry.name.strip_suffix('/').unwrap_or(&entry.name);
        let declared = (kind == Kind::File).then_some(entry.size);
        if let Some(member) = planner.admit(name, kind, declared)? {
            planned.insert(entry.name.clone(), member);
        }
    }

    let result = archive.for_each_entries(|entry, reader| {
        if entry.is_directory() {
            return Ok(true);
        }
        let Some(member) = planned.remove(entry.name()) else {
            return Err(Error::Other("a member the listing did not have".into()));
        };
        let stopped = |error: ExtractError| Error::from(job.stop(error));
        let mut sink = job.open(member).map_err(stopped)?;
        sink.copy(reader).map_err(|error| match error {
            crate::job::CopyError::Read(error) => Error::from(error),
            crate::job::CopyError::Write(error) => stopped(error),
        })?;
        sink.finish().map_err(stopped)?;
        Ok(true)
    });
    result.map_err(|error| job.resolve(error_of(error, false, &limits)))?;
    // Every member the listing admitted was met, empty files after the ones
    // with data. A member left means the decoder ended early.
    if !planned.is_empty() {
        return Err(Refusal::corrupt("목록에 있는 멤버를 다 풀지 못했어요").into());
    }
    Ok(())
}
