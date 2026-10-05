//! Helpers of the extraction tests: sample archives made in memory, and the
//! checks of what an extraction wrote.

#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use trss_archive::{extract, ExtractError, Limits, Member, Refusal};

/// The path of a sample archive in `tests/fixtures`.
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// A folder of one test: `in/` holds the archives, `out/` is where they are
/// unpacked, and nothing else may appear next to them.
pub struct Case {
    dir: tempfile::TempDir,
}

impl Case {
    pub fn new() -> Case {
        let case = Case {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir(case.input()).unwrap();
        case
    }

    pub fn input(&self) -> PathBuf {
        self.dir.path().join("in")
    }

    pub fn out(&self) -> PathBuf {
        self.dir.path().join("out")
    }

    /// Writes an archive (or part of one) named `name` in `in/`.
    pub fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.input().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// Unpacks the archive `parts` hold under the limits, the name being the
    /// first part's, and says how it ended whatever it is.
    pub fn extract_raw(
        &self,
        parts: &[PathBuf],
        limits: &Limits,
    ) -> Result<Vec<Member>, ExtractError> {
        let name = parts[0].file_name().unwrap().to_string_lossy().into_owned();
        extract(parts, &name, &self.out(), limits)
    }

    /// Like [`Case::extract_raw`], for a test that expects a member list or a
    /// refusal: a failure of the machine is a failure of the test.
    pub fn extract_with(&self, parts: &[PathBuf], limits: &Limits) -> Result<Vec<Member>, Refusal> {
        match self.extract_raw(parts, limits) {
            Ok(members) => Ok(members),
            Err(ExtractError::Refused(refusal)) => Err(refusal),
            Err(failed) => panic!("the machine failed the unpacking: {failed}"),
        }
    }

    pub fn extract(&self, parts: &[PathBuf]) -> Result<Vec<Member>, Refusal> {
        self.extract_with(parts, &Limits::default())
    }

    /// Unpacks the bytes of an archive named `name` with the default limits.
    pub fn unpack(&self, name: &str, bytes: &[u8]) -> Result<Vec<Member>, Refusal> {
        let path = self.write(name, bytes);
        self.extract(&[path])
    }

    pub fn unpack_with(
        &self,
        name: &str,
        bytes: &[u8],
        limits: &Limits,
    ) -> Result<Vec<Member>, Refusal> {
        let path = self.write(name, bytes);
        self.extract_with(&[path], limits)
    }

    /// Nothing but `in/` and `out/` was made next to each other, and `out/`
    /// holds files named by index only.
    pub fn assert_contained(&self) {
        let mut names: Vec<String> = std::fs::read_dir(self.dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert!(
            names == ["in", "out"] || names == ["in"],
            "files outside out/: {names:?}"
        );
        if let Ok(read) = std::fs::read_dir(self.out()) {
            for entry in read {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                assert!(
                    !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()),
                    "a file of out/ is not an index: {name}"
                );
                assert!(entry.file_type().unwrap().is_file());
            }
        }
    }

    /// Checks every member against its file in `out/` (size and SHA-256) and
    /// returns the bytes by path.
    pub fn contents(&self, members: &[Member]) -> BTreeMap<String, Vec<u8>> {
        self.assert_contained();
        let mut files = std::collections::HashSet::new();
        let mut all = BTreeMap::new();
        for member in members {
            assert!(files.insert(member.file.clone()), "{member:?} twice");
            let bytes = std::fs::read(self.out().join(&member.file)).unwrap();
            assert_eq!(bytes.len() as u64, member.size, "{member:?}");
            let sha256: String = Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert_eq!(sha256, member.sha256, "{member:?}");
            assert!(
                all.insert(member.path.clone(), bytes).is_none(),
                "{member:?} twice"
            );
        }
        // Nothing but the members is in out/.
        assert_eq!(
            std::fs::read_dir(self.out()).unwrap().count(),
            members.len()
        );
        all
    }
}

/// The paths of members, in order.
pub fn paths(members: &[Member]) -> Vec<&str> {
    members.iter().map(|member| member.path.as_str()).collect()
}

/// A sample subtitle of `n` bytes, not a run of one byte.
pub fn sample(n: usize, seed: u8) -> Vec<u8> {
    let mut state = u32::from(seed).wrapping_mul(2654435761).wrapping_add(12345);
    (0..n)
        .map(|_| {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            (state >> 16) as u8
        })
        .collect()
}

// ZIP, made by hand so that the names, the flags and the modes are anything.

pub struct ZipEntry<'a> {
    pub name: &'a [u8],
    pub data: &'a [u8],
    /// The Unix mode in the external attributes (with the file type bits).
    pub mode: Option<u32>,
    pub utf8: bool,
}

impl<'a> ZipEntry<'a> {
    pub fn file(name: &'a str, data: &'a [u8]) -> ZipEntry<'a> {
        ZipEntry {
            name: name.as_bytes(),
            data,
            mode: None,
            utf8: true,
        }
    }

    pub fn dir(name: &'a str) -> ZipEntry<'a> {
        ZipEntry {
            name: name.as_bytes(),
            data: b"",
            mode: Some(0o040755),
            utf8: true,
        }
    }

    pub fn with_mode(mut self, mode: u32) -> ZipEntry<'a> {
        self.mode = Some(mode);
        self
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(data);
    crc.sum()
}

/// A ZIP whose members are stored.
pub fn zip(entries: &[ZipEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for entry in entries {
        let crc = crc32(entry.data);
        let flags: u16 = if entry.utf8 { 0x0800 } else { 0 };
        let offset = out.len() as u32;
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // stored
        out.extend_from_slice(&[0, 0, 0x21, 0]); // time, date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(entry.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(entry.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(entry.name);
        out.extend_from_slice(entry.data);

        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&((3u16 << 8) | 20).to_le_bytes()); // made by Unix
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&flags.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&[0, 0, 0x21, 0]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(entry.data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(entry.data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]); // extra, comment, disk, internal
        let attributes = entry.mode.map_or(0, |mode| mode << 16);
        central.extend_from_slice(&attributes.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(entry.name);
    }
    let directory = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&[0, 0, 0, 0]); // this disk, the directory's disk
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&directory.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// A ZIP of files with the given paths and contents.
pub fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let entries: Vec<ZipEntry> = files
        .iter()
        .map(|(name, data)| ZipEntry::file(name, data))
        .collect();
    zip(&entries)
}

// tar, made by hand as well.

pub struct TarEntry<'a> {
    pub name: &'a str,
    pub kind: u8,
    pub data: &'a [u8],
    pub link: &'a str,
}

impl<'a> TarEntry<'a> {
    pub fn file(name: &'a str, data: &'a [u8]) -> TarEntry<'a> {
        TarEntry {
            name,
            kind: b'0',
            data,
            link: "",
        }
    }

    pub fn of_kind(name: &'a str, kind: u8) -> TarEntry<'a> {
        TarEntry {
            name,
            kind,
            data: b"",
            link: "",
        }
    }

    pub fn link(name: &'a str, kind: u8, link: &'a str) -> TarEntry<'a> {
        TarEntry {
            name,
            kind,
            data: b"",
            link,
        }
    }
}

pub fn tar(entries: &[TarEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    for entry in entries {
        let mut header = [0u8; 512];
        let put = |header: &mut [u8; 512], at: usize, bytes: &[u8]| {
            header[at..at + bytes.len()].copy_from_slice(bytes);
        };
        put(&mut header, 0, entry.name.as_bytes());
        put(&mut header, 100, b"0000644\0");
        put(&mut header, 108, b"0001000\0");
        put(&mut header, 116, b"0001000\0");
        put(
            &mut header,
            124,
            format!("{:011o}\0", entry.data.len()).as_bytes(),
        );
        put(&mut header, 136, b"00000000000\0");
        header[156] = entry.kind;
        put(&mut header, 157, entry.link.as_bytes());
        put(&mut header, 257, b"ustar\0");
        put(&mut header, 263, b"00");
        put(&mut header, 148, b"        ");
        let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        put(&mut header, 148, format!("{sum:06o}\0 ").as_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(entry.data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    out
}

/// A tar of files with the given paths and contents.
pub fn tar_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let entries: Vec<TarEntry> = files
        .iter()
        .map(|(name, data)| TarEntry::file(name, data))
        .collect();
    tar(&entries)
}

// The compressed streams.

pub fn gz(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

pub fn bz2(data: &[u8]) -> Vec<u8> {
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

pub fn xz(data: &[u8]) -> Vec<u8> {
    let mut encoder = liblzma::write::XzEncoder::new(Vec::new(), 1);
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

/// A gzip stream of `n` zero bytes, made as a stream.
pub fn gz_zeros(n: usize) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let chunk = vec![0u8; 1 << 20];
    let mut left = n;
    while left > 0 {
        let take = left.min(chunk.len());
        encoder.write_all(&chunk[..take]).unwrap();
        left -= take;
    }
    encoder.finish().unwrap()
}

/// A 7z of files with the given paths and contents; a path ending in `/` is a folder.
pub fn sevenz(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter};

    let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    for (name, data) in entries {
        match name.strip_suffix('/') {
            Some(dir) => {
                writer
                    .push_archive_entry(ArchiveEntry::new_directory(dir), None::<&[u8]>)
                    .unwrap();
            }
            None => {
                writer
                    .push_archive_entry(ArchiveEntry::new_file(name), Some(*data))
                    .unwrap();
            }
        }
    }
    writer.finish().unwrap().into_inner()
}
