//! Reading the parts of an archive: the volumes of a split one as one byte
//! stream, and the format a part's first bytes tell.

use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

/// The archive formats, told by their first bytes alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Zip,
    Rar,
    SevenZip,
    Gzip,
    Bzip2,
    Xz,
    Tar,
}

/// How many bytes [`Format::sniff`] needs: a tar's `ustar` ends at 262.
pub(crate) const HEAD: usize = 512;

impl Format {
    pub(crate) fn sniff(head: &[u8]) -> Option<Format> {
        // `PK\x07\x08` begins the first piece of a spanned ZIP (`.z01`).
        if head.starts_with(b"PK\x03\x04")
            || head.starts_with(b"PK\x05\x06")
            || head.starts_with(b"PK\x07\x08")
        {
            Some(Format::Zip)
        } else if head.starts_with(b"Rar!\x1a\x07") {
            Some(Format::Rar)
        } else if head.starts_with(b"7z\xbc\xaf\x27\x1c") {
            Some(Format::SevenZip)
        } else if head.starts_with(&[0x1f, 0x8b, 0x08]) {
            Some(Format::Gzip)
        } else if head.len() > 3 && head.starts_with(b"BZh") && (b'1'..=b'9').contains(&head[3]) {
            Some(Format::Bzip2)
        } else if head.starts_with(b"\xfd7zXZ\0") {
            Some(Format::Xz)
        } else if head.len() >= 262 && &head[257..262] == b"ustar" {
            Some(Format::Tar)
        } else {
            None
        }
    }
}

/// The first [`HEAD`] bytes of `path` (fewer for a shorter file).
pub(crate) fn read_head(path: &Path) -> io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(HEAD);
    File::open(path)?.take(HEAD as u64).read_to_end(&mut head)?;
    Ok(head)
}

/// Whether `name` ends in an extension of an archive a member is unpacked in
/// place for (case-insensitive).
pub(crate) fn has_archive_extension(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        ".zip", ".rar", ".7z", ".gz", ".tgz", ".bz2", ".tbz2", ".xz", ".txz", ".tar",
    ]
    .iter()
    .any(|extension| lower.ends_with(extension))
}

/// Whether `name` is the name of a split archive's volume: `.001`, `.r00`,
/// `.part1.rar`.
pub(crate) fn is_volume_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    let Some((stem, tail)) = lower.rsplit_once('.') else {
        return false;
    };
    if tail.len() >= 2 && digits(tail) {
        return true;
    }
    if tail.len() >= 3 && tail.starts_with('r') && digits(&tail[1..]) {
        return true;
    }
    tail == "rar" && stem.rsplit_once(".part").is_some_and(|(_, n)| digits(n))
}

struct Part {
    path: PathBuf,
    start: u64,
    len: u64,
}

/// The files of `parts` read as one: `Read + Seek` over their bytes one after
/// the other. Only the file the position is in is held open, so the number of
/// parts is not the number of open files.
pub(crate) struct Concat {
    parts: Vec<Part>,
    total: u64,
    pos: u64,
    open: Option<(usize, File)>,
}

impl Concat {
    pub(crate) fn open(paths: &[PathBuf]) -> io::Result<Concat> {
        let mut parts = Vec::with_capacity(paths.len());
        let mut total = 0u64;
        for path in paths {
            let len = std::fs::metadata(path)?.len();
            parts.push(Part {
                path: path.clone(),
                start: total,
                len,
            });
            total = total.checked_add(len).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "the parts are too long")
            })?;
        }
        Ok(Concat {
            parts,
            total,
            pos: 0,
            open: None,
        })
    }

    pub(crate) fn len(&self) -> u64 {
        self.total
    }

    /// The index of the part `pos` is in.
    fn part_at(&self, pos: u64) -> Option<usize> {
        // The last part that starts at or before `pos` and is not empty.
        let after = self.parts.partition_point(|part| part.start <= pos);
        (0..after)
            .rev()
            .find(|&i| pos < self.parts[i].start + self.parts[i].len)
    }
}

impl Read for Concat {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let Some(index) = self.part_at(self.pos) else {
            return Ok(0);
        };
        let part = &self.parts[index];
        let offset = self.pos - part.start;
        let file = match &mut self.open {
            Some((open, file)) if *open == index => file,
            slot => {
                *slot = None;
                let mut file = File::open(&part.path)?;
                file.seek(SeekFrom::Start(offset))?;
                &mut slot.insert((index, file)).1
            }
        };
        let room = (part.len - offset).min(buf.len() as u64) as usize;
        let n = file.read(&mut buf[..room])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Concat {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(pos) => Some(pos),
            SeekFrom::End(delta) => self.total.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.pos.checked_add_signed(delta),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before the start"))?;
        // The open file follows the position, which a read repositions it to.
        if let Some((index, file)) = &mut self.open {
            let part = &self.parts[*index];
            if target >= part.start && target < part.start + part.len {
                file.seek(SeekFrom::Start(target - part.start))?;
            } else {
                self.open = None;
            }
        }
        self.pos = target;
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_told_by_their_first_bytes() {
        assert_eq!(Format::sniff(b"PK\x03\x04rest"), Some(Format::Zip));
        assert_eq!(Format::sniff(b"PK\x05\x06"), Some(Format::Zip));
        assert_eq!(Format::sniff(b"Rar!\x1a\x07\x00x"), Some(Format::Rar));
        assert_eq!(Format::sniff(b"Rar!\x1a\x07\x01\x00x"), Some(Format::Rar));
        assert_eq!(
            Format::sniff(b"7z\xbc\xaf\x27\x1c\x00\x04"),
            Some(Format::SevenZip)
        );
        assert_eq!(Format::sniff(&[0x1f, 0x8b, 8, 0]), Some(Format::Gzip));
        assert_eq!(Format::sniff(b"BZh9x"), Some(Format::Bzip2));
        assert_eq!(Format::sniff(b"BZh0x"), None);
        assert_eq!(Format::sniff(b"\xfd7zXZ\0x"), Some(Format::Xz));
        let mut tar = vec![0u8; 512];
        tar[257..262].copy_from_slice(b"ustar");
        assert_eq!(Format::sniff(&tar), Some(Format::Tar));
        assert_eq!(Format::sniff(b"hello"), None);
        assert_eq!(Format::sniff(b""), None);
    }

    #[test]
    fn archive_extensions_ignore_case() {
        for name in [
            "a.zip", "A.ZIP", "x/b.7z", "c.tar", "d.TGZ", "e.tbz2", "f.xz", "g.txz", "h.gz",
            "i.bz2", "j.rar",
        ] {
            assert!(has_archive_extension(name), "{name}");
        }
        for name in ["a.docx", "a.ass", "zip", "a.zip.txt", "a.ttf"] {
            assert!(!has_archive_extension(name), "{name}");
        }
    }

    #[test]
    fn volume_names() {
        for name in [
            "a.7z.001",
            "a.zip.002",
            "a.part1.rar",
            "a.PART01.rar",
            "a.r00",
            "a.r12",
        ] {
            assert!(is_volume_name(name), "{name}");
        }
        for name in ["a.rar", "a.7z", "a.zip", "a.1", "a.part.rar", "a.ass"] {
            assert!(!is_volume_name(name), "{name}");
        }
    }

    #[test]
    fn parts_read_as_one_stream() {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for (i, bytes) in [&b"hello "[..], &b""[..], &b"big "[..], &b"world"[..]]
            .iter()
            .enumerate()
        {
            let path = dir.path().join(format!("p{i}"));
            std::fs::write(&path, bytes).unwrap();
            paths.push(path);
        }
        let mut concat = Concat::open(&paths).unwrap();
        assert_eq!(concat.len(), 15);
        let mut all = String::new();
        concat.read_to_string(&mut all).unwrap();
        assert_eq!(all, "hello big world");
        concat.seek(SeekFrom::Start(4)).unwrap();
        let mut some = [0u8; 6];
        concat.read_exact(&mut some).unwrap();
        assert_eq!(&some, b"o big ");
        concat.seek(SeekFrom::End(-5)).unwrap();
        let mut tail = String::new();
        concat.read_to_string(&mut tail).unwrap();
        assert_eq!(tail, "world");
        concat.seek(SeekFrom::Current(-20)).unwrap_err();
        assert_eq!(concat.seek(SeekFrom::Start(100)).unwrap(), 100);
        assert_eq!(concat.read(&mut some).unwrap(), 0);
    }
}
