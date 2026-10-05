//! A tar, and the single compressed streams (gzip, bzip2, xz) that hold either
//! a tar or one file.
//!
//! Neither has a listing to admit first, so a member is admitted as it is
//! met, and the members before the one that breaks a rule are left in `out`
//! for the caller to remove.

use std::io::{self, Cursor, Read};

use crate::{
    failure::from_read,
    job::{Job, Kind},
    name::decode_lossy,
    source::HEAD,
    ExtractError,
};

/// Whether `name` says the stream it names holds a tar: `.tgz`, `.tar.gz`, ...
pub(crate) fn names_a_tar(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        ".tgz", ".tbz", ".tbz2", ".txz", ".tar.gz", ".tar.bz2", ".tar.xz",
    ]
    .iter()
    .any(|extension| lower.ends_with(extension))
}

/// The name of the one member a stream named `name` holds: `name` without its
/// last extension, or `unpacked` when nothing is left.
pub(crate) fn single_name(name: &str) -> String {
    let name = name.rsplit('/').next().unwrap_or(name);
    let stem = match name.rsplit_once('.') {
        Some((stem, _)) => stem,
        None => name,
    };
    if stem.is_empty() {
        "unpacked".to_owned()
    } else {
        stem.to_owned()
    }
}

/// Unpacks a compressed `stream`: a tar when its first block says so (or
/// `name` does), else one member named after `name`.
pub(crate) fn extract_stream(
    job: &Job,
    mut stream: impl Read,
    name: &str,
) -> Result<(), ExtractError> {
    let dictionary = job.limits().dictionary;
    let mut head = Vec::with_capacity(HEAD);
    (&mut stream)
        .take(HEAD as u64)
        .read_to_end(&mut head)
        .map_err(|error| job.resolve(from_read(error, dictionary)))?;
    let is_tar = head.len() >= 262 && &head[257..262] == b"ustar" || names_a_tar(name);
    let mut stream = Cursor::new(head).chain(stream);
    if is_tar {
        return extract_tar(job, &mut stream);
    }
    let mut planner = job.planner();
    if let Some(member) = planner.admit(&single_name(name), Kind::File, None)? {
        let mut sink = job.open(member)?;
        sink.copy(&mut stream)
            .map_err(|error| job.copy_failed(error))?;
        sink.finish()?;
    }
    Ok(())
}

/// Unpacks the tar `reader` reads.
pub(crate) fn extract_tar(job: &Job, reader: &mut dyn Read) -> Result<(), ExtractError> {
    use tar::EntryType;

    let dictionary = job.limits().dictionary;
    let read_failed = |error: io::Error| job.resolve(from_read(error, dictionary));
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(read_failed)?;
    let mut planner = job.planner();
    for entry in entries {
        let mut entry = entry.map_err(read_failed)?;
        let kind = match entry.header().entry_type() {
            EntryType::Regular | EntryType::Continuous => Kind::File,
            EntryType::Directory => Kind::Dir,
            EntryType::Symlink | EntryType::Link => Kind::Link,
            // The extended header of the whole archive (`git archive`'s) is
            // no member.
            EntryType::XGlobalHeader => continue,
            _ => Kind::Special,
        };
        let raw = entry.path_bytes().into_owned();
        let name = decode_lossy(&raw);
        // `tar` of `.` names every member `./...`, and the folder itself `./`.
        let name = name.trim_start_matches("./");
        let name = name.strip_suffix('/').unwrap_or(name);
        if kind == Kind::Dir && (name.is_empty() || name == ".") {
            continue;
        }
        let declared = (kind == Kind::File).then(|| entry.size());
        if let Some(member) = planner.admit(name, kind, declared)? {
            let mut sink = job.open(member)?;
            sink.copy(&mut entry)
                .map_err(|error| job.copy_failed(error))?;
            sink.finish()?;
        }
    }
    Ok(())
}
