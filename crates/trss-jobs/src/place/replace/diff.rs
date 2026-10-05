//! The content comparison a plan keeps (`docs/specs/subtitles.md`, 교체 비교와
//! 승인; ticket 0069): what differs between the subtitle the episode has and
//! the new one, made when the plan is, by the engine of
//! [`trss_subtitles::compare`].
//!
//! A comparison that cannot be made is [`Compared::Unreadable`] with a Korean
//! reason, never an empty difference: a file whose content cannot be read
//! (the reason names the side, `현재 자막` or `새 자막`), a file larger than
//! [`MAX_BYTES`], and a current file that is not the bytes the plan recorded
//! for it any more. The worker makes it, never a web request: both files are
//! read whole.

use std::{
    io::Read,
    panic::{catch_unwind, AssertUnwindSafe},
    path::Path,
};

use sha2::{Digest, Sha256};
use trss_subtitles::compare::{self, Unreadable};

use super::records::{Compared, FileSeen};
use crate::{area::hex, place::package::extension};

/// The most bytes of a file whose content is compared (8 MiB). A larger one
/// is a rewritten file nobody reads line by line, and reading it whole would
/// strain the worker's memory.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

const CURRENT: &str = "현재 자막";
const NEW: &str = "새 자막";
const TOO_LARGE: &str = "파일이 커서 내용을 비교하지 않았어요";
const CHANGED: &str = "비교하는 사이 현재 자막이 바뀌었어요";
const FAILED: &str = "내용을 읽다가 오류가 나 비교하지 않았어요";

fn unreadable(side: &str, why: &str) -> Compared {
    Compared::Unreadable(format!("{side}: {why}"))
}

/// Compares the current subtitle at `at`, which the plan saw as `seen`, with
/// the new subtitle `new` (an `new_extension` file). The file is read again
/// here, so it must still be the bytes the plan recorded: a change in between
/// is no difference to show, as the plan goes stale on its next check anyway.
pub fn compare_files(at: &Path, seen: &FileSeen, new: &[u8], new_extension: &str) -> Compared {
    if seen.size > MAX_BYTES {
        return unreadable(CURRENT, TOO_LARGE);
    }
    if new.len() as u64 > MAX_BYTES {
        return unreadable(NEW, TOO_LARGE);
    }
    let current = match read_current(at) {
        Ok(bytes) => bytes,
        Err(why) => return unreadable(CURRENT, &why),
    };
    // The file was not larger when the plan saw it: it changed.
    if current.len() as u64 > MAX_BYTES || hex(&Sha256::digest(&current)) != seen.sha256 {
        return Compared::Unreadable(CHANGED.to_owned());
    }
    let ext = extension(&at.to_string_lossy()).unwrap_or_default();
    compare_bytes(&current, &ext, new, new_extension)
}

/// A regular file's bytes, at most one byte more than [`MAX_BYTES`].
fn read_current(at: &Path) -> Result<Vec<u8>, String> {
    let failed = |e: std::io::Error| format!("읽지 못했어요: {e}");
    let meta = std::fs::symlink_metadata(at).map_err(failed)?;
    if !meta.file_type().is_file() {
        return Err("일반 파일이 아니라 읽지 못했어요".to_owned());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(at)
        .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(failed)?;
    Ok(bytes)
}

/// Compares two files' bytes by their extensions: the difference, or why a
/// side's content cannot be read (the current one first).
pub fn compare_bytes(current: &[u8], current_ext: &str, new: &[u8], new_ext: &str) -> Compared {
    guarded(|| {
        let current = compare::read(current, current_ext);
        let new = compare::read(new, new_ext);
        match (current, new) {
            (Ok(old), Ok(new)) => Compared::Diff(Box::new(compare::compare(&old, &new))),
            (Err(Unreadable { reason }), _) => unreadable(CURRENT, &reason),
            (_, Err(Unreadable { reason })) => unreadable(NEW, &reason),
        }
    })
}

/// The engine reads files from the internet; a bug it hits on one must not
/// fail the plan's job on every run until it is held: the comparison is not
/// made, and says so.
fn guarded(work: impl FnOnce() -> Compared) -> Compared {
    catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|_| Compared::Unreadable(FAILED.to_owned()))
}

#[cfg(test)]
mod tests;
