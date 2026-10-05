//! What an I/O error of a decoder means: the refusal it is, or a failure of the
//! machine that is no fault of the archive.

use std::{error::Error, io};

use crate::{ExtractError, Refusal};

/// The error for one a decoder returned while it was read: a refused
/// dictionary (liblzma's `memlimit`), a password asked for, a damaged
/// stream, or the process out of memory (a failure of the machine, not a
/// refusal). `dictionary` is the limit a refused dictionary is reported with.
pub(crate) fn from_read(error: io::Error, dictionary: u64) -> ExtractError {
    // The error may wrap a decoder's own error, which may wrap another one.
    let mut source: Option<&(dyn Error + 'static)> = error.get_ref().map(|e| e as _);
    while let Some(inner) = source {
        if let Some(lzma) = inner.downcast_ref::<liblzma::stream::Error>() {
            match lzma {
                liblzma::stream::Error::MemLimit => {
                    return Refusal::Dictionary { limit: dictionary }.into()
                }
                liblzma::stream::Error::Mem => return ExtractError::memory(),
                _ => {}
            }
        }
        if let Some(zip) = inner.downcast_ref::<zip::result::ZipError>() {
            return from_zip(zip, false);
        }
        source = inner.source();
    }
    if error.kind() == io::ErrorKind::OutOfMemory {
        return ExtractError::memory();
    }
    if error.kind() == io::ErrorKind::UnexpectedEof {
        return Refusal::corrupt("압축 파일이 중간에 끝났어요").into();
    }
    Refusal::corrupt(&error).into()
}

/// The error for a ZIP error. `split` is whether the archive is a split one,
/// whose invalid structure is most likely a missing volume.
pub(crate) fn from_zip(error: &zip::result::ZipError, split: bool) -> ExtractError {
    use zip::result::ZipError;

    match error {
        ZipError::InvalidPassword => Refusal::Encrypted,
        ZipError::UnsupportedArchive(what) if *what == ZipError::PASSWORD_REQUIRED => {
            Refusal::Encrypted
        }
        ZipError::UnsupportedArchive(what) => {
            if what.contains("AES") {
                Refusal::Encrypted
            } else {
                Refusal::unsupported(what)
            }
        }
        ZipError::InvalidArchive(_) if split => Refusal::MissingVolume,
        ZipError::Io(error) if error.kind() == io::ErrorKind::OutOfMemory => {
            return ExtractError::memory()
        }
        ZipError::Io(error) if split && error.kind() == io::ErrorKind::UnexpectedEof => {
            Refusal::MissingVolume
        }
        _ => Refusal::corrupt(error),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_out_of_memory_is_a_failure_of_the_machine() {
        // What `Vec::try_reserve` gives, converted the way `?` converts it.
        let reserve = Vec::<u8>::new().try_reserve(usize::MAX).unwrap_err();
        let errors = [
            io::Error::from(reserve),
            io::Error::from(io::ErrorKind::OutOfMemory),
            io::Error::other(liblzma::stream::Error::Mem),
        ];
        for error in errors {
            let text = format!("{error:?}");
            assert_eq!(from_read(error, 64 << 20), ExtractError::memory(), "{text}");
        }
        let zip = zip::result::ZipError::Io(io::ErrorKind::OutOfMemory.into());
        assert_eq!(from_zip(&zip, false), ExtractError::memory());
    }

    #[test]
    fn a_damaged_stream_is_still_a_refusal() {
        let error = io::Error::new(io::ErrorKind::InvalidData, "bad block");
        assert!(matches!(
            from_read(error, 64 << 20),
            ExtractError::Refused(Refusal::Corrupt { .. })
        ));
        let error = io::Error::other(liblzma::stream::Error::MemLimit);
        assert_eq!(
            from_read(error, 64 << 20),
            ExtractError::Refused(Refusal::Dictionary { limit: 64 << 20 })
        );
    }
}
