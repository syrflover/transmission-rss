//! RAR 4 and 5, and their volumes (`.partN.rar`, `.rar` with `.r00`, ...), with
//! `rars` (reading only, no encryption feature).
//!
//! The volumes are opened one by one from the list of paths the caller gives,
//! never found by their names. The listing is admitted whole before anything
//! is extracted, which refuses a link, an encrypted member, a bad path and
//! a member over [`Limits::rar_member`] first. The crate's own limits
//! stand behind that: the dictionary, the whole-member buffered decode (8 MiB,
//! or a member is decoded into memory whole), the reader workspace and the
//! output of a member and of the whole.

use std::{
    cell::RefCell,
    io::{self, Write},
    path::PathBuf,
    rc::Rc,
};

use rars::{ArchiveFamily, ArchiveReadOptions, ArchiveReader, AttrSource, Error};

use crate::{
    job::{Job, Kind, Planned, Sink},
    name::decode_lossy,
    ExtractError, Limits, Refusal,
};

/// The most a RAR 5 member may be decoded into memory whole.
const BUFFERED_DECODE: u64 = 8 << 20;
/// The most memory the readers' decoders may hold in all.
const READER_WORKSPACE: u64 = 32 << 20;
/// The flag of a RAR 1.5 to 4 end block that says another volume follows.
const END_NEXT_VOLUME: u16 = 0x0001;

fn options(limits: &Limits) -> ArchiveReadOptions<'static> {
    ArchiveReadOptions::new()
        .with_rar50_dictionary_size_limit(limits.dictionary)
        .with_rar50_buffered_decode_limit(BUFFERED_DECODE)
        .with_max_reader_workspace_bytes(READER_WORKSPACE)
        .with_max_member_output_bytes(limits.rar_member)
        .with_max_total_output_bytes(limits.total)
}

/// Whether `error`, or the error it says happened to an entry, a volume or an
/// offset, is the process running out of memory.
fn is_out_of_memory(error: &Error) -> bool {
    match error {
        Error::AtEntry { source, .. }
        | Error::AtArchiveOffset { source, .. }
        | Error::InVolume { source, .. } => is_out_of_memory(source),
        Error::Io(io) => io.kind == io::ErrorKind::OutOfMemory,
        _ => false,
    }
}

/// The error for an error of the crate: a failure of the machine when it ran
/// out of memory, else the refusal it is.
fn error_of(error: Error, limits: &Limits) -> ExtractError {
    if is_out_of_memory(&error) {
        ExtractError::memory()
    } else {
        refusal(error, limits).into()
    }
}

/// The refusal for an error of the crate.
fn refusal(error: Error, limits: &Limits) -> Refusal {
    match error {
        Error::AtEntry { source, name, .. } => match *source {
            Error::MemberOutputLimitExceeded { .. } => Refusal::MemberTooLarge {
                path: decode_lossy(&name),
                limit: limits.rar_member,
            },
            source => refusal(source, limits),
        },
        Error::AtArchiveOffset { source, .. } | Error::InVolume { source, .. } => {
            refusal(*source, limits)
        }
        // Encryption is not built in: an encrypted member or header is asked
        // for a password the crate cannot use.
        Error::FeatureDisabled { .. }
        | Error::NeedPassword
        | Error::WrongPasswordOrCorruptData
        | Error::UnsupportedEncryption { .. }
        | Error::Rar20Crypto(_)
        | Error::Rar30Crypto(_)
        | Error::Rar50Crypto(_) => Refusal::Encrypted,
        Error::Rar50DictionaryLimitExceeded { .. } => Refusal::Dictionary {
            limit: limits.dictionary,
        },
        Error::MemberOutputLimitExceeded { .. } => Refusal::MemberTooLarge {
            path: String::new(),
            limit: limits.rar_member,
        },
        Error::TotalOutputLimitExceeded { .. } => Refusal::TooLarge {
            limit: limits.total,
        },
        Error::HeaderCountLimitExceeded { .. } => Refusal::TooManyMembers {
            limit: limits.members,
        },
        Error::Rar50BufferedDecodeLimitExceeded { .. }
        | Error::Rar50FilterMemoryLimitExceeded { .. }
        | Error::Rar50ScratchLimitExceeded { .. } => {
            Refusal::unsupported("메모리에 다 올릴 수 없는 필터가 걸린 RAR 멤버")
        }
        Error::UnsupportedSignature
        | Error::UnsupportedVersion(_)
        | Error::UnsupportedFeature { .. }
        | Error::UnsupportedFamilyFeature { .. }
        | Error::UnsupportedCompression { .. } => Refusal::unsupported(error),
        Error::Io(io) => Refusal::corrupt(io.message),
        other => Refusal::corrupt(other),
    }
}

/// What a member's attributes say it is, beside its header's own kinds.
fn kind_of(meta: &rars::ArchiveMemberMeta) -> Kind {
    if meta.is_redirection {
        return Kind::Link;
    }
    match meta.attr_source() {
        AttrSource::Unix => match meta.file_attr & 0o170000 {
            0o120000 => Kind::Link,
            0o040000 => Kind::Dir,
            0o100000 | 0 => file_or_dir(meta),
            _ => Kind::Special,
        },
        AttrSource::Dos if meta.file_attr & 0x400 != 0 => Kind::Link,
        _ => file_or_dir(meta),
    }
}

fn file_or_dir(meta: &rars::ArchiveMemberMeta) -> Kind {
    if meta.is_directory {
        Kind::Dir
    } else {
        Kind::File
    }
}

/// A writer for a member whose [`Sink`] is finished by the extraction loop,
/// which sees where the member ends.
struct Slot(Rc<RefCell<Option<Sink>>>);

impl Write for Slot {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match self.0.borrow_mut().as_mut() {
            Some(sink) => sink.write(data),
            None => Err(io::Error::other("the member was already finished")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Unpacks the RAR volumes `parts` are, in order.
pub(crate) fn extract(job: &Job, parts: &[PathBuf]) -> Result<(), ExtractError> {
    let limits = job.limits();
    let parse = ArchiveReadOptions::new()
        .with_max_header_count(limits.members.saturating_mul(4).saturating_add(64));
    let mut archives = Vec::with_capacity(parts.len());
    for path in parts {
        archives.push(
            ArchiveReader::read_path_with_options(path, parse)
                .map_err(|error| error_of(error, &limits))?,
        );
    }
    check_volumes(&archives)?;

    let members = rars::volume_members(&archives).map_err(|error| error_of(error, &limits))?;
    job.has_room_for(members.len() as u64)?;
    let mut planner = job.planner();
    // The members in the order the crate extracts them: their raw names, and
    // what to write each as.
    let mut order: Vec<(Vec<u8>, Option<Planned>)> = Vec::with_capacity(members.len());
    for member in &members {
        let meta = &member.meta;
        if meta.is_encrypted {
            return Err(Refusal::Encrypted.into());
        }
        let mut name = decode_lossy(&meta.name);
        if meta.family != ArchiveFamily::Rar50Plus {
            // Before RAR 5 a Windows host's names have backslashes for slashes.
            name = name.replace('\\', "/");
        }
        let kind = kind_of(meta);
        let name = name.strip_suffix('/').unwrap_or(&name);
        let declared = (kind == Kind::File).then_some(meta.unpacked_size);
        let planned = planner.admit(name, kind, declared)?;
        if let Some(planned) = &planned {
            if meta.unpacked_size > limits.rar_member {
                return Err(Refusal::MemberTooLarge {
                    path: planned.path.clone(),
                    limit: limits.rar_member,
                }
                .into());
            }
        }
        order.push((meta.name.clone(), planned));
    }

    let current: Rc<RefCell<Option<Sink>>> = Rc::new(RefCell::new(None));
    let mut next = 0usize;
    let finish_current = |current: &RefCell<Option<Sink>>| -> Result<(), ExtractError> {
        match current.borrow_mut().take() {
            Some(sink) => sink.finish(),
            None => Ok(()),
        }
    };
    let result = rars::extract_volumes_to_with_options(&archives, options(&limits), |meta| {
        let stopped = |error: ExtractError| Error::from(job.stop(error));
        finish_current(&current).map_err(stopped)?;
        let Some((name, planned)) = order.get_mut(next) else {
            return Err(Error::InvalidHeader("more members than the listing has"));
        };
        next += 1;
        if *name != meta.name {
            return Err(Error::InvalidHeader("members in another order than listed"));
        }
        match planned.take() {
            None => Ok(Box::new(io::sink()) as Box<dyn Write>),
            Some(planned) => {
                let sink = job.open(planned).map_err(stopped)?;
                *current.borrow_mut() = Some(sink);
                Ok(Box::new(Slot(Rc::clone(&current))) as Box<dyn Write>)
            }
        }
    });
    let finished = result
        .map_err(|error| job.resolve(error_of(error, &limits)))
        .and_then(|()| finish_current(&current));
    finished?;
    if next != order.len() {
        return Err(Refusal::corrupt("목록에 있는 멤버를 다 풀지 못했어요").into());
    }
    Ok(())
}

/// Refuses a set of volumes that is not whole: one that begins with the end of
/// a member, ends with the start of one or with a volume that says another
/// follows, or whose RAR 5 volume numbers are
/// not 0, 1, 2...
fn check_volumes(archives: &[rars::Archive]) -> Result<(), Refusal> {
    let first = archives
        .first()
        .and_then(|archive| archive.members().next());
    if first.is_some_and(|member| member.meta.is_split_before) {
        return Err(Refusal::MissingVolume);
    }
    let last = archives.last().and_then(|archive| archive.members().last());
    if last.is_some_and(|member| member.meta.is_split_after) {
        return Err(Refusal::MissingVolume);
    }
    // The last volume given says another one follows.
    let more_follow = match archives.last() {
        Some(rars::Archive::Rar50Plus(archive)) => archive.blocks.last().is_some_and(|block| {
            matches!(block, rars::rar50::Block::End(end) if end.has_next_volume())
        }),
        Some(rars::Archive::Rar15To40(archive)) => archive.blocks.last().is_some_and(|block| {
            matches!(block, rars::rar15_40::Block::End(end) if end.flags & END_NEXT_VOLUME != 0)
        }),
        _ => false,
    };
    if more_follow {
        return Err(Refusal::MissingVolume);
    }
    for (number, archive) in archives.iter().enumerate() {
        if let Some(main) = archive.as_rar50().map(|archive| &archive.main) {
            if main.volume_number.unwrap_or(0) != number as u64 {
                return Err(Refusal::MissingVolume);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_out_of_memory_is_a_failure_of_the_machine() {
        let memory = || Error::from(io::Error::from(io::ErrorKind::OutOfMemory));
        let limits = Limits::default();
        assert_eq!(error_of(memory(), &limits), ExtractError::memory());
        // Also when the crate says which member it happened to.
        let wrapped = memory()
            .at_entry(b"a.ass".to_vec(), "extract")
            .at_archive_offset(7);
        assert_eq!(error_of(wrapped, &limits), ExtractError::memory());
        // Another I/O error is the archive's.
        let other = Error::from(io::Error::from(io::ErrorKind::InvalidData));
        assert!(matches!(
            error_of(other, &limits),
            ExtractError::Refused(Refusal::Corrupt { .. })
        ));
    }
}
