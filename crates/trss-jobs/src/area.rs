//! The receive area: `<app data>/receive/`, where jobs put what they receive.
//!
//! - `<job id>/<name>`: a job's received files, under the names the site gave
//!   (made safe for a file name; a second file of the same name becomes
//!   `<stem> (2).<ext>`).
//! - `.tmp/<attempt id>/<name>`: the temporary file of one attempt to receive
//!   a file. Nothing but that attempt writes there, and a published file
//!   leaves it by a rename that never replaces anything
//!   ([`trss_core::files::rename_noreplace`]).
//!
//! The area is the app's own; the library's folders are not touched here.

use std::{
    io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

/// The folder of received files.
#[derive(Debug, Clone)]
pub struct ReceiveArea {
    root: PathBuf,
}

impl ReceiveArea {
    pub fn new(root: impl Into<PathBuf>) -> ReceiveArea {
        ReceiveArea { root: root.into() }
    }

    /// The area in the app data folder `app_data` (the database's folder).
    pub fn in_app_data(app_data: &Path) -> ReceiveArea {
        ReceiveArea::new(app_data.join("receive"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A job's folder, relative to the area.
    pub fn job_dir(job_id: &str) -> String {
        job_id.to_owned()
    }

    /// An attempt's temporary folder, relative to the area.
    pub fn temp_dir(attempt_id: &str) -> String {
        format!(".tmp/{attempt_id}")
    }

    /// `relative` within the area.
    pub fn at(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}

/// `name` as a single safe file name: no separators, control characters or
/// leading dots, not empty, and at most 200 bytes.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned
        .trim_start_matches(|c: char| c == '.' || c.is_whitespace())
        .trim_end();
    let cleaned = match cleaned.is_empty() {
        true => "file",
        false => cleaned,
    };
    if cleaned.len() <= MAX_NAME {
        return cleaned.to_owned();
    }
    let (stem, ext) = split_ext(cleaned);
    // An extension too long to be one is cut with the rest.
    let ext = match ext.len() <= MAX_EXT {
        true => ext,
        false => "",
    };
    let stem = match ext.is_empty() {
        true => cleaned,
        false => stem,
    };
    format!("{}{ext}", cut(stem, MAX_NAME - ext.len()))
}

const MAX_NAME: usize = 200;
const MAX_EXT: usize = 16;

/// The longest start of `text` of at most `bytes` bytes.
fn cut(text: &str, bytes: usize) -> &str {
    let mut end = bytes.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// `name`'s stem and its extension with the dot (`""` without one).
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// The names to try for `name`, in order: `name`, `<stem> (2).<ext>`, ….
pub fn name_candidates(name: &str) -> impl Iterator<Item = String> + '_ {
    let (stem, ext) = split_ext(name);
    std::iter::once(name.to_owned()).chain((2..).map(move |n| format!("{stem} ({n}){ext}")))
}

/// What names a file on its file system: `<device>:<inode>`, and its birth
/// time when the file system keeps one (`:<ns>`), so a number reused by a
/// later file does not pass for the first.
pub fn object_of(meta: &std::fs::Metadata) -> String {
    let born = meta
        .created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
    match born {
        Some(born) => format!("{}:{}:{}", meta.dev(), meta.ino(), born.as_nanos()),
        None => format!("{}:{}", meta.dev(), meta.ino()),
    }
}

/// A regular file's length, SHA-256 (lower-case hex) and object, read whole.
/// A link or anything else at `path` is refused, and so is a file that is
/// replaced while it is opened.
pub fn read_facts(path: &Path) -> io::Result<(u64, String, String)> {
    use std::io::Read;
    let seen = std::fs::symlink_metadata(path)?;
    if !seen.file_type().is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    let mut file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    if object_of(&meta) != object_of(&seen) {
        return Err(io::Error::other("replaced while opened"));
    }
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hasher.update(&buf[..n]);
    }
    Ok((size, hex(&hasher.finalize()), object_of(&meta)))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Syncs a folder, so a rename into it outlives a power loss.
pub fn sync_dir(path: &Path) -> io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_made_safe_and_numbered() {
        assert_eq!(safe_name("../a/b.ass"), "_a_b.ass");
        assert_eq!(safe_name("  .hidden.srt "), "hidden.srt");
        assert_eq!(safe_name(""), "file");
        assert_eq!(safe_name("x\u{0}y.ass"), "x_y.ass");
        assert_eq!(safe_name(". .x"), "x");
        let long = format!("{}.ass", "가".repeat(100));
        let safe = safe_name(&long);
        assert!(safe.len() <= 200 && safe.ends_with(".ass"));
        let long_ext = format!("a.{}", "b".repeat(300));
        let safe = safe_name(&long_ext);
        assert!(safe.len() <= 200 && safe.starts_with("a."));

        let names: Vec<String> = name_candidates("a.ass").take(3).collect();
        assert_eq!(names, ["a.ass", "a (2).ass", "a (3).ass"]);
        let names: Vec<String> = name_candidates("noext").take(2).collect();
        assert_eq!(names, ["noext", "noext (2)"]);
    }
}
