//! File tools several features share.

use std::{io, path::Path};

/// Renames `from` to `to` unless `to` exists (an `AlreadyExists` error then),
/// with `renameat2(RENAME_NOREPLACE)`. Never replaces anything, a directory
/// included. There is no fallback: a filesystem without the flag answers
/// `InvalidInput` or `Unsupported`.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use rustix::fs::{renameat_with, RenameFlags, CWD};
    renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE).map_err(io::Error::from)
}
