//! Unpacks a received archive under hard limits (`docs/specs/subtitles.md`,
//! 압축 해제의 격리와 한도).
//!
//! The unpacking runs in a child process of its own, so that a decompression
//! bomb, a decoder's bug or a hang ends the one package and not the worker:
//!
//! - [`extract`] unpacks an archive (ZIP, RAR 4 and 5, 7z, gzip, bzip2, xz,
//!   tar, and the volumes of a split one) into a folder under [`Limits`], and
//!   says why it did not: [`Refusal`] for what is wrong with the archive,
//!   [`ExtractError::Failed`] for what is wrong with the machine (a full disk,
//!   no memory). Members are written to `out/<index>`,
//!   never to a path the archive names, and an archive in an archive is
//!   unpacked in place.
//! - [`child::main`] is the child process: it puts the process limits on
//!   itself, runs [`extract`] and answers on its standard output.
//! - [`run::Unpacker`] is the parent's side: it starts the child, watches the
//!   time, kills it with its whole process group when the time is up or the
//!   job is cancelled, and reads the answer, which it does not trust.

mod failure;
mod job;
mod model;
mod name;
mod rar;
mod sevenz;
mod source;
mod stream;
mod zip;

pub mod child;
pub mod run;

use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

pub use model::{ExtractError, Limits, Member, Refusal};

use job::Job;
use source::{is_volume_name, read_head, Concat, Format};

/// Unpacks the archive whose volumes are `parts` (in order; one path for a
/// whole archive) into the folder `out`, which it creates (its parent exists;
/// `out` must not exist).
///
/// `name` is the archive's own file name, which names the member of a single
/// gzip, bzip2 or xz stream (without its last extension) and says a stream
/// holds a tar (`.tgz`, `.tar.xz`, ...). The members come back in the order
/// they were written. A refusal leaves what was written in `out` for the
/// caller to remove.
///
/// Whatever is wrong with the archive is an [`ExtractError::Refused`]. A
/// failure of the machine (`out` cannot be created, a member cannot be
/// written, the memory runs out) is an [`ExtractError::Failed`], not a refusal.
pub fn extract(
    parts: &[PathBuf],
    name: &str,
    out: &Path,
    limits: &Limits,
) -> Result<Vec<Member>, ExtractError> {
    if parts.is_empty() {
        return Err(Refusal::corrupt("압축 파일이 없어요").into());
    }
    let mut outer_size = 0u64;
    for part in parts {
        let len = std::fs::metadata(part)
            .map_err(|error| Refusal::corrupt(format!("파일을 열지 못했어요: {error}")))?
            .len();
        outer_size = outer_size.saturating_add(len);
    }
    std::fs::create_dir(out)
        .map_err(|error| ExtractError::failed("압축을 풀 폴더를 만들지 못했어요", error))?;
    let job = Job::new(limits, out, outer_size);
    let split = parts.len() > 1 || is_volume_name(name);
    unpack(&job, parts, name, split).map_err(|error| job.resolve(error))?;
    File::open(out)
        .and_then(|folder| folder.sync_all())
        .map_err(ExtractError::write)?;
    Ok(job.into_members())
}

/// Unpacks the archive `parts` hold into the job: the outer one or a nested one.
/// `split` is whether it is a split archive, the sign of a missing piece being
/// an archive cut short.
fn unpack(job: &Job, parts: &[PathBuf], name: &str, split: bool) -> Result<(), ExtractError> {
    let head =
        read_head(&parts[0]).map_err(|error| job.input_error("파일을 읽지 못했어요", error))?;
    let limits = job.limits();
    let reader = || {
        Concat::open(parts)
            .map(BufReader::new)
            .map_err(|error| job.input_error("파일을 열지 못했어요", error))
    };
    match Format::sniff(&head) {
        None => Err(Refusal::unsupported("알 수 없는 압축 형식").into()),
        Some(Format::Zip) if head.starts_with(b"PK\x07\x08") => {
            Err(Refusal::unsupported("여러 디스크로 나뉜 ZIP(.z01)").into())
        }
        Some(Format::Zip) => zip::extract(job, parts, split),
        Some(Format::Rar) => rar::extract(job, parts),
        Some(Format::SevenZip) => sevenz::extract(job, parts, split),
        Some(Format::Gzip) => {
            stream::extract_stream(job, flate2::read::MultiGzDecoder::new(reader()?), name)
        }
        Some(Format::Bzip2) => {
            stream::extract_stream(job, bzip2::read::MultiBzDecoder::new(reader()?), name)
        }
        Some(Format::Xz) => {
            // The decoder needs its dictionary and some working memory.
            let memory = limits.dictionary.saturating_add(16 << 20);
            let decoder =
                liblzma::stream::Stream::new_stream_decoder(memory, liblzma::stream::CONCATENATED)
                    .map_err(|error| match error {
                        liblzma::stream::Error::Mem => ExtractError::memory(),
                        error => Refusal::corrupt(error).into(),
                    })?;
            stream::extract_stream(
                job,
                liblzma::read::XzDecoder::new_stream(reader()?, decoder),
                name,
            )
        }
        Some(Format::Tar) => stream::extract_tar(job, &mut reader()?),
    }
}
