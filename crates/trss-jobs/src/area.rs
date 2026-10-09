//! The receive area: `<app data>/receive/`, where jobs put what they receive.
//!
//! - `<job id>/<name>`: a job's received files, under the names the site gave
//!   (made safe for a file name; a second file of the same name becomes
//!   `<stem> (2).<ext>`), or `<job id>/<folders>/<name>` for a file the post
//!   shows in folders (a WinPNG image's).
//! - `<job id>/.unpack/<file id>/<n>`: the members a received archive was
//!   unpacked to, under the numbers the unpacking gave them (no name from the
//!   archive is a path here); removed with the archive once its members are
//!   kept ([`crate::place::unpack`]). A received name never starts with a
//!   dot, so the folder is no receipt's.
//! - `.tmp/winpng-<job id>/`: the files a server browser took out of a post's
//!   images before the job receives them; removed when the item ends.
//! - `.tmp/check-<job id>-<item id>/`: the file a server browser downloaded
//!   when a person passed a site's check on the job's screen, until the item
//!   received it ([`crate::screen`]).
//! - `.tmp/<attempt id>/<name>`: the temporary file of one attempt to receive
//!   a file. Nothing but that attempt writes there, and a published file
//!   leaves it by a rename that never replaces anything
//!   ([`trss_core::files::rename_noreplace`]).
//!
//! The area is the app's own; the library's folders are not touched here.

use std::{
    io,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
pub use trss_core::files::hex;
use trss_core::{app_data::RECEIVE_DIR, file_id::FileId};

/// The folder of received files.
#[derive(Debug, Clone)]
pub struct ReceiveArea {
    root: PathBuf,
    app_data: Option<PathBuf>,
}

impl ReceiveArea {
    /// An area at `root`, with no app data folder known.
    pub fn new(root: impl Into<PathBuf>) -> ReceiveArea {
        ReceiveArea {
            root: root.into(),
            app_data: None,
        }
    }

    /// The area in the app data folder `app_data` (the database's folder).
    pub fn in_app_data(app_data: &Path) -> ReceiveArea {
        ReceiveArea {
            root: app_data.join(RECEIVE_DIR),
            app_data: Some(app_data.to_owned()),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The app data folder the area is in, when it was made in one: where a
    /// package's attachments and companion files are kept.
    pub fn app_data(&self) -> Option<&Path> {
        self.app_data.as_deref()
    }

    /// A job's folder, relative to the area.
    pub fn job_dir(job_id: &str) -> String {
        job_id.to_owned()
    }

    /// The folder a received archive is unpacked into, relative to the area.
    pub fn unpack_dir(job_id: &str, file_id: &str) -> String {
        format!("{job_id}/.unpack/{file_id}")
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

/// `name` as a single safe file name: no separators, control characters,
/// invisible format characters or leading dots, not empty, and at most 200
/// bytes.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|&c| !invisible(c))
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

/// The most folders deep a file of a source may be put.
pub const MAX_FOLDER_DEPTH: usize = 8;
/// The most bytes the folders of a file may take together (with the `/`s),
/// so the path of a file stays far under what the file system takes.
pub const MAX_FOLDER_BYTES: usize = 600;

/// `folder` (a relative path of folders joined by `/`) with every folder made
/// a safe name ([`safe_name`]), without empty parts or parts that walk out;
/// `Ok(None)` when nothing is left. A path deeper than [`MAX_FOLDER_DEPTH`]
/// or longer than [`MAX_FOLDER_BYTES`] is refused, with the reason in a
/// sentence: cutting it would put the file somewhere the source did not.
pub fn safe_folder(folder: &str) -> Result<Option<String>, String> {
    let parts: Vec<String> = folder
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|p| !p.is_empty() && *p != "." && *p != "..")
        .map(safe_name)
        .collect();
    if parts.len() > MAX_FOLDER_DEPTH {
        return Err(format!(
            "파일이 폴더 {MAX_FOLDER_DEPTH}단계보다 깊은 곳에 있어서 받지 않았어요"
        ));
    }
    let joined = parts.join("/");
    if joined.len() > MAX_FOLDER_BYTES {
        return Err("파일이 있는 폴더 경로가 너무 길어서 받지 않았어요".to_owned());
    }
    Ok((!parts.is_empty()).then_some(joined))
}

/// A format character that shows nothing but can change how a name reads:
/// the bidirectional marks, embeddings, overrides and isolates (which can
/// show `gpj.ass` as `ssa.jpg`), zero-width spaces, word joiners and
/// invisible operators, the byte order mark, the soft hyphen, interlinear
/// annotation marks and tag characters. The zero-width joiner and non-joiner
/// stay, as they shape emoji and scripts.
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{AD}'
            | '\u{61C}'
            | '\u{180E}'
            | '\u{200B}'
            | '\u{200E}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
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

/// What names a file on its file system, as a record keeps it:
/// `<device>:<inode>` ([`FileId`]). A recorded object is compared with a file
/// found later by [`trss_core::file_id::same_recorded_file`], which reads the
/// records an earlier glibc build made with a birth time after the inode too.
pub fn object_of(meta: &std::fs::Metadata) -> String {
    FileId::of(meta).to_string()
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
    if !FileId::of(&meta).same_file_now(FileId::of(&seen)) {
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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;

    #[test]
    fn invisible_format_characters_are_dropped_from_names() {
        // A right-to-left override would show this name as `13화ssa.exe`.
        assert_eq!(safe_name("13화\u{202E}exe.ass"), "13화exe.ass");
        assert_eq!(
            safe_name("\u{200E}\u{200F}\u{202A}\u{202B}\u{202C}\u{202D}a.ass"),
            "a.ass"
        );
        assert_eq!(safe_name("\u{2066}b\u{2067}\u{2068}\u{2069}.srt"), "b.srt");
        assert_eq!(
            safe_name("\u{FEFF}c\u{200B}d\u{2060}\u{61C}\u{AD}.ass"),
            "cd.ass"
        );
        assert_eq!(safe_name("e\u{E0041}.ass"), "e.ass");
        // Leading dots under an invisible mark are still trimmed, and a name
        // of nothing else is still a name.
        assert_eq!(safe_name("\u{200F}.hidden.ass"), "hidden.ass");
        assert_eq!(safe_name("\u{202E}\u{2066}"), "file");
        // The joiners that shape emoji and scripts stay.
        assert_eq!(safe_name("👩\u{200D}💻 1화.ass"), "👩\u{200D}💻 1화.ass");
        assert_eq!(safe_name("a\u{200C}b.srt"), "a\u{200C}b.srt");
    }

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

    #[test]
    fn folders_are_made_safe_and_capped() {
        assert_eq!(safe_folder("").unwrap(), None);
        assert_eq!(safe_folder(".././/").unwrap(), None);
        assert_eq!(
            safe_folder("회차/../2화\\예고/").unwrap().as_deref(),
            Some("회차/2화/예고")
        );
        // Eight deep is the most; nine is refused rather than cut.
        let deep = |n: usize| vec!["d"; n].join("/");
        assert!(safe_folder(&deep(MAX_FOLDER_DEPTH)).unwrap().is_some());
        assert!(safe_folder(&deep(MAX_FOLDER_DEPTH + 1)).is_err());
        // A few long names add up past the cap.
        let long = vec!["가".repeat(60); 4].join("/");
        assert!(long.len() > MAX_FOLDER_BYTES);
        assert!(safe_folder(&long).is_err());
    }

    #[test]
    fn an_object_is_the_file_s_device_and_inode() {
        // No birth time, though the file system keeps one: the image's musl
        // build cannot read it, and the tests run what it records.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.ass");
        std::fs::write(&path, b"a").unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(object_of(&meta), format!("{}:{}", meta.dev(), meta.ino()));
    }
}
