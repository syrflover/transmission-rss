//! Sample ZIPs for the tests of the crates that read or receive one (behind
//! the `test-support` feature), made by the `zip` crate into memory.
//!
//! The archives have the layout the readers' tests rely on: the sizes and the
//! CRC in the local header of each member (the writer seeks back, so there is
//! no data descriptor), no ZIP64 record below its thresholds, and an end
//! record of 22 bytes (no comment) that ends the file. A test that needs a
//! different layout (a name in CP949 without the UTF-8 flag, a Unix mode, a
//! stream that is written as it is made) builds its own bytes.

use std::io::{Cursor, Write};

use ::zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

/// How the members are compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Stored,
    Deflated,
}

/// One entry of a sample ZIP.
#[derive(Clone, Copy, Debug)]
pub enum Entry<'a> {
    /// A file: its name and bytes.
    File(&'a str, &'a [u8]),
    /// A folder, by name (`Fonts/`); always stored.
    Dir(&'a str),
}

/// A ZIP of `members` (name, bytes), compressed as `method` says.
pub fn zip_of(members: &[(&str, &[u8])], method: Method) -> Vec<u8> {
    let entries: Vec<Entry<'_>> = members
        .iter()
        .map(|&(name, bytes)| Entry::File(name, bytes))
        .collect();
    zip_with_dirs(&entries, method)
}

/// [`zip_of`] with [`Method::Deflated`].
pub fn deflated_zip(members: &[(&str, &[u8])]) -> Vec<u8> {
    zip_of(members, Method::Deflated)
}

/// A ZIP of `entries` in the order given, folders among them, its files
/// compressed as `method` says.
pub fn zip_with_dirs(entries: &[Entry<'_>], method: Method) -> Vec<u8> {
    let method = match method {
        Method::Stored => CompressionMethod::Stored,
        Method::Deflated => CompressionMethod::Deflated,
    };
    let options = SimpleFileOptions::default().compression_method(method);
    let mut out = ZipWriter::new(Cursor::new(Vec::new()));
    for entry in entries {
        match *entry {
            Entry::File(name, bytes) => {
                out.start_file(name, options).unwrap();
                out.write_all(bytes).unwrap();
            }
            Entry::Dir(name) => out.add_directory(name, options).unwrap(),
        }
    }
    out.finish().unwrap().into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    /// The end record (22 bytes, signature `PK\5\6`, no comment) is the last
    /// thing in the file; returns the number of entries it counts.
    fn entries_in_end_record(bytes: &[u8]) -> u16 {
        let end = bytes.len() - 22;
        assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
        assert_eq!(u16_at(bytes, end + 20), 0, "a comment");
        u16_at(bytes, end + 10)
    }

    #[test]
    fn a_stored_member_has_its_size_and_crc_in_the_local_header() {
        let data = b"hello hello hello";
        let bytes = zip_of(&[("a.txt", data)], Method::Stored);
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        // No data descriptor (flag bit 3), method 0, then CRC and both sizes.
        assert_eq!(u16_at(&bytes, 6) & 0x8, 0);
        assert_eq!(u16_at(&bytes, 8), 0);
        let mut crc = flate2::Crc::new();
        crc.update(data);
        assert_eq!(u32_at(&bytes, 14), crc.sum());
        assert_eq!(u32_at(&bytes, 18), data.len() as u32);
        assert_eq!(u32_at(&bytes, 22), data.len() as u32);
        // The data follows the name (`a.txt`) at once: no extra field.
        assert_eq!(u16_at(&bytes, 28), 0);
        assert_eq!(&bytes[30 + 5..30 + 5 + data.len()], data);
        assert_eq!(entries_in_end_record(&bytes), 1);
    }

    #[test]
    fn a_deflated_member_has_its_sizes_in_the_local_header_too() {
        let data = vec![b'a'; 5000];
        let bytes = zip_of(&[("a.ass", &data)], Method::Deflated);
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        assert_eq!(u16_at(&bytes, 6) & 0x8, 0);
        assert_eq!(u16_at(&bytes, 8), 8);
        assert_eq!(u32_at(&bytes, 22), 5000);
        let packed = u32_at(&bytes, 18) as usize;
        assert!((1..5000).contains(&packed), "{packed}");
        assert_eq!(entries_in_end_record(&bytes), 1);
    }

    #[test]
    fn the_end_record_points_at_the_central_directory() {
        let bytes = zip_of(&[("a", b"1"), ("b", b"2")], Method::Stored);
        let end = bytes.len() - 22;
        let directory = u32_at(&bytes, end + 16) as usize;
        assert_eq!(&bytes[directory..directory + 4], b"PK\x01\x02");
        assert_eq!(entries_in_end_record(&bytes), 2);
        // No ZIP64 record before the end record.
        assert_ne!(&bytes[end - 20..end - 16], b"PK\x06\x07");
    }

    #[test]
    fn deflated_zip_is_zip_of_deflated() {
        let members: &[(&str, &[u8])] = &[("a.srt", b"abc"), ("b.srt", b"")];
        assert_eq!(deflated_zip(members), zip_of(members, Method::Deflated),);
    }

    #[test]
    fn folders_and_files_come_in_the_order_given_and_a_folder_has_no_data() {
        let bytes = zip_with_dirs(
            &[
                Entry::File("sub/a.ass", b"x"),
                Entry::Dir("sub2/"),
                Entry::File("sub2/empty.txt", b""),
            ],
            Method::Deflated,
        );
        assert_eq!(entries_in_end_record(&bytes), 3);
        let end = bytes.len() - 22;
        let mut at = u32_at(&bytes, end + 16) as usize;
        let mut seen = Vec::new();
        for _ in 0..3 {
            assert_eq!(&bytes[at..at + 4], b"PK\x01\x02");
            let name_len = u16_at(&bytes, at + 28) as usize;
            let extra_len = u16_at(&bytes, at + 30) as usize;
            let comment_len = u16_at(&bytes, at + 32) as usize;
            let name = std::str::from_utf8(&bytes[at + 46..at + 46 + name_len]).unwrap();
            seen.push((name.to_owned(), u32_at(&bytes, at + 24)));
            at += 46 + name_len + extra_len + comment_len;
        }
        assert_eq!(
            seen,
            [
                ("sub/a.ass".to_owned(), 1),
                ("sub2/".to_owned(), 0),
                ("sub2/empty.txt".to_owned(), 0),
            ]
        );
    }

    #[test]
    fn an_archive_of_a_folder_only_is_still_an_archive() {
        let bytes = zip_with_dirs(&[Entry::Dir("Fonts/")], Method::Deflated);
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        assert_eq!(entries_in_end_record(&bytes), 1);
    }
}
