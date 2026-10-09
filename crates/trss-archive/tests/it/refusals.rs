//! Each way an archive is refused, and that nothing is made outside `out/`.

use crate::common;

use common::*;
use rars::{ArchiveVersion, Builder};
use trss_archive::{ExtractError, Limits, Refusal};

fn refused(case: &Case, name: &str, bytes: &[u8]) -> Refusal {
    let refusal = case.unpack(name, bytes).unwrap_err();
    case.assert_contained();
    refusal
}

fn refused_fixture(name: &str) -> Refusal {
    let case = Case::new();
    let refusal = case.extract(&[fixture(name)]).unwrap_err();
    case.assert_contained();
    refusal
}

// Limits of size and count

#[test]
fn a_zip_bomb_is_refused_by_what_it_declares_before_it_is_written() {
    // 24 MiB of zeros deflate to about 24 KiB: over the ratio.
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    writer.start_file("zeros.bin", options).unwrap();
    let chunk = vec![0u8; 1 << 20];
    for _ in 0..24 {
        std::io::Write::write_all(&mut writer, &chunk).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    assert!(bytes.len() < 100 << 10);
    let case = Case::new();
    assert_eq!(refused(&case, "bomb.zip", &bytes), Refusal::Ratio);
    // Nothing was written: the listing was enough.
    assert_eq!(std::fs::read_dir(case.out()).unwrap().count(), 0);
}

#[test]
fn a_stream_that_inflates_past_the_ratio_is_refused_while_it_is_written() {
    // The size of a stream is not declared: the written bytes are counted.
    let limits = Limits {
        ratio: 20,
        ratio_slack: 1 << 20,
        ..Limits::default()
    };
    let case = Case::new();
    let bytes = gz_zeros(24 << 20);
    assert_eq!(
        case.unpack_with("bomb.gz", &bytes, &limits),
        Err(Refusal::Ratio)
    );
    case.assert_contained();
    // What was written is less than the ratio allows, and less than the stream holds.
    let written: u64 = std::fs::read_dir(case.out())
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum();
    let allowed = 20 * bytes.len() as u64 + (1 << 20);
    assert!(allowed < 24 << 20, "the sample is too small: {allowed}");
    assert!(written <= allowed, "{written}");
}

#[test]
fn the_ratio_counts_against_the_outer_parts_of_a_split_archive() {
    let limits = Limits {
        ratio: 10,
        ratio_slack: 0,
        ..Limits::default()
    };
    // 1 MiB of zeros in a gzip of about 1 KiB, in two pieces.
    let bytes = gz_zeros(1 << 20);
    let case = Case::new();
    let half = bytes.len() / 2;
    let parts = [
        case.write("z.gz.001", &bytes[..half]),
        case.write("z.gz.002", &bytes[half..]),
    ];
    assert_eq!(case.extract_with(&parts, &limits), Err(Refusal::Ratio));
    case.assert_contained();
}

#[test]
fn a_member_over_the_member_limit_is_refused_by_its_declared_size_and_by_its_count() {
    let limits = Limits {
        member: 1000,
        ..Limits::default()
    };
    // Declared (a ZIP lists the size).
    let case = Case::new();
    let bytes = zip_of(&[("big.bin", &sample(2000, 1))]);
    assert_eq!(
        case.unpack_with("a.zip", &bytes, &limits),
        Err(Refusal::MemberTooLarge {
            path: "big.bin".to_owned(),
            limit: 1000
        })
    );
    case.assert_contained();
    assert_eq!(std::fs::read_dir(case.out()).unwrap().count(), 0);
    // Counted (a stream does not).
    let case = Case::new();
    let bytes = gz(&sample(2000, 1));
    assert_eq!(
        case.unpack_with("big.bin.gz", &bytes, &limits),
        Err(Refusal::MemberTooLarge {
            path: "big.bin".to_owned(),
            limit: 1000
        })
    );
    case.assert_contained();
}

#[test]
fn a_zip_that_lies_about_its_size_in_the_directory_is_refused() {
    let refusal = refused_fixture("liar_central.zip");
    assert!(
        matches!(refusal, Refusal::MemberTooLarge { .. }),
        "{refusal:?}"
    );
}

/// A ZIP of one deflated member `a.ass` of 5000 bytes, which `declared_size`
/// bytes it says it has, in its local header and in its central directory both,
/// so that the declared size passes the checks made before it is written.
fn zip_that_declares(declared_size: u32, deflated: bool) -> Vec<u8> {
    let mut bytes = if deflated {
        trss_archive::testing::deflated_zip(&[("a.ass", &sample(5000, 1))])
    } else {
        zip_of(&[("a.ass", &sample(5000, 1))])
    };
    let size = declared_size.to_le_bytes();
    // The uncompressed size is at 22 of the local header and at 24 of the
    // central one, which the end record (22 bytes, no comment) points at; a
    // stored member's compressed size, at 18 and 20, is the same number.
    let eocd = bytes.len() - 22;
    let directory = u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
    assert_eq!(&bytes[..4], b"PK\x03\x04");
    assert_eq!(&bytes[directory..directory + 4], b"PK\x01\x02");
    bytes[22..26].copy_from_slice(&size);
    bytes[directory + 24..directory + 28].copy_from_slice(&size);
    if !deflated {
        bytes[18..22].copy_from_slice(&size);
        bytes[directory + 20..directory + 24].copy_from_slice(&size);
    }
    bytes
}

#[test]
fn a_zip_that_lies_about_a_small_size_is_stopped_while_written() {
    // The directory and the local header say 10 bytes, under the member limit
    // of 100, so nothing is refused by the declaration; the data is 5000.
    let limits = Limits {
        member: 100,
        ..Limits::default()
    };
    // A deflate stream is not cut at the declared size by the crate: what is
    // written is counted, and the count stops it.
    let case = Case::new();
    assert_eq!(
        case.unpack_with("a.zip", &zip_that_declares(10, true), &limits),
        Err(Refusal::MemberTooLarge {
            path: "a.ass".to_owned(),
            limit: 100
        })
    );
    case.assert_contained();
    // A stored member is cut at the declared size by the crate, which fails
    // its checksum, so no more than 10 bytes were written.
    let case = Case::new();
    let refusal = case
        .unpack_with("a.zip", &zip_that_declares(10, false), &limits)
        .unwrap_err();
    assert!(matches!(refusal, Refusal::Corrupt { .. }), "{refusal:?}");
    case.assert_contained();
    let written: u64 = std::fs::read_dir(case.out())
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum();
    assert!(written <= 10, "{written}");
}

#[test]
fn the_total_limit_refuses_what_a_listing_adds_up_to() {
    let limits = Limits {
        total: 2500,
        ..Limits::default()
    };
    let bytes = zip_of(&[
        ("a", &sample(1000, 1)),
        ("b", &sample(1000, 2)),
        ("c", &sample(1000, 3)),
    ]);
    let case = Case::new();
    assert_eq!(
        case.unpack_with("a.zip", &bytes, &limits),
        Err(Refusal::TooLarge { limit: 2500 })
    );
    assert_eq!(std::fs::read_dir(case.out()).unwrap().count(), 0);
    // And what a stream adds up to as it is written.
    let bytes = tar_of(&[
        ("a", &sample(1000, 1)),
        ("b", &sample(1000, 2)),
        ("c", &sample(1000, 3)),
    ]);
    let case = Case::new();
    assert_eq!(
        case.unpack_with("a.tar", &bytes, &limits),
        Err(Refusal::TooLarge { limit: 2500 })
    );
    case.assert_contained();
}

#[test]
fn too_many_members_are_refused_in_every_format() {
    let names: Vec<String> = (0..2001).map(|i| format!("f{i}.ass")).collect();
    let empty: Vec<(&str, &[u8])> = names.iter().map(|name| (name.as_str(), &b""[..])).collect();
    let too_many = Refusal::TooManyMembers { limit: 2000 };
    let case = Case::new();
    assert_eq!(refused(&case, "many.zip", &zip_of(&empty)), too_many);
    // The end record's total is enough: nothing was written.
    assert_eq!(std::fs::read_dir(case.out()).unwrap().count(), 0);
    let case = Case::new();
    assert_eq!(refused(&case, "many.tar", &tar_of(&empty)), too_many);
    let case = Case::new();
    assert_eq!(refused(&case, "many.7z", &sevenz(&empty)), too_many);
    // A RAR lists its members first as well.
    for version in [ArchiveVersion::Rar50, ArchiveVersion::Rar29] {
        let mut builder = Builder::new(version);
        for name in &names {
            builder
                .add_bytes(name.clone().into_bytes(), Vec::new(), None, None)
                .unwrap();
        }
        let case = Case::new();
        assert_eq!(
            refused(&case, "many.rar", &builder.to_bytes().unwrap()),
            too_many,
            "{version:?}"
        );
        assert_eq!(std::fs::read_dir(case.out()).unwrap().count(), 0);
    }
    // Exactly the limit is fine.
    let case = Case::new();
    let members = case.unpack("many.zip", &zip_of(&empty[..2000])).unwrap();
    assert_eq!(members.len(), 2000);
}

#[test]
fn folders_count_as_members() {
    let limits = Limits {
        members: 2,
        ..Limits::default()
    };
    let bytes = zip(&[
        ZipEntry::dir("a/"),
        ZipEntry::dir("b/"),
        ZipEntry::file("c", b"x"),
    ]);
    let case = Case::new();
    assert_eq!(
        case.unpack_with("a.zip", &bytes, &limits),
        Err(Refusal::TooManyMembers { limit: 2 })
    );
}

#[test]
fn a_7z_header_bigger_than_the_bound_is_refused_before_it_is_parsed() {
    // The crate takes about 96 bytes for each file a header declares, and a
    // header declares as many as it has bytes for: 512 bytes for each member
    // the limits allow and 64 KiB more are the most a header may have.
    let limits = Limits::default();
    let bound = 2000 * 512 + (64 << 10);
    // A start header that points at a header of `size` bytes, which are there.
    let seven_zip_of_header_size = |size: u64| {
        let mut bytes = b"7z\xbc\xaf\x27\x1c\x00\x04".to_vec();
        bytes.extend_from_slice(&[0; 4]); // the start header's CRC
        bytes.extend_from_slice(&0u64.to_le_bytes()); // where the header begins
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]); // the header's CRC
        bytes.resize(32 + size as usize, 0);
        bytes
    };
    let case = Case::new();
    let refusal = case
        .unpack_with("h.7z", &seven_zip_of_header_size(bound + 1), &limits)
        .unwrap_err();
    assert_eq!(
        refusal,
        Refusal::Corrupt {
            detail: "7z 목록이 너무 커요".to_owned()
        }
    );
    case.assert_contained();
    // At the bound it is the crate that reads it, and finds it damaged.
    let case = Case::new();
    let refusal = case
        .unpack_with("h.7z", &seven_zip_of_header_size(bound), &limits)
        .unwrap_err();
    assert!(
        matches!(&refusal, Refusal::Corrupt { detail } if !detail.contains("너무 커요")),
        "{refusal:?}"
    );
    // A 7z that is legitimate is not in the way of it.
    let case = Case::new();
    let members = case
        .unpack_with("l.7z", &sevenz(&[("a.ass", b"x")]), &limits)
        .unwrap();
    assert_eq!(members.len(), 1);
    // A 7z of many members is a listing that is too long, as before.
    let limits = Limits {
        members: 10,
        ..Limits::default()
    };
    let names: Vec<String> = (0..11).map(|i| format!("f{i}.ass")).collect();
    let empty: Vec<(&str, &[u8])> = names.iter().map(|name| (name.as_str(), &b""[..])).collect();
    let case = Case::new();
    assert_eq!(
        case.unpack_with("m.7z", &sevenz(&empty), &limits),
        Err(Refusal::TooManyMembers { limit: 10 })
    );
}

// Dictionaries

#[test]
fn a_big_dictionary_is_refused_and_the_limit_itself_is_fine() {
    for name in [
        "dict256m.xz",
        "dict1536m.xz",
        "dict1536m.tar.xz",
        "dict256m.7z",
        "dict1536m.7z",
        "dict4095m.7z",
        "d128m.rar",
        "dict_rar50_1g.rar",
    ] {
        assert_eq!(
            refused_fixture(name),
            Refusal::Dictionary { limit: 64 << 20 },
            "{name}"
        );
    }
    for name in ["dict64m.xz", "dict64m.7z", "d64m.rar"] {
        let case = Case::new();
        let members = case.extract(&[fixture(name)]).unwrap();
        assert_eq!(members.len(), 1, "{name}");
        case.contents(&members);
    }
}

// Encryption

#[test]
fn encrypted_archives_are_refused() {
    for name in [
        "enc_aes.zip",
        "enc_zipcrypto.zip",
        "enc_7z_data.7z",
        "enc_7z_hdr.7z",
        "enc_rar5_data.rar",
        "enc_rar5_hdr.rar",
        "enc_rar4_data.rar",
    ] {
        assert_eq!(refused_fixture(name), Refusal::Encrypted, "{name}");
    }
}

// Paths

#[test]
fn paths_that_leave_the_folder_or_cannot_be_one_are_refused() {
    let long_name = "a".repeat(256);
    let many_parts = vec!["d"; 17].join("/") + "/x.ass";
    let long_path = vec!["d".repeat(200); 6].join("/") + "/x.ass";
    let bad: Vec<&str> = vec![
        "../x.ass",
        "../../x.ass",
        "a/../../x.ass",
        "a/../x.ass",
        "/etc/x.ass",
        "a\\b.ass",
        "C:/x.ass",
        "C:x.ass",
        "a//b.ass",
        "./x.ass",
        "a/./x.ass",
        "a\0b.ass",
        &long_name,
        &many_parts,
        &long_path,
    ];
    for path in bad {
        let case = Case::new();
        let refusal = refused(&case, "p.zip", &zip_of(&[(path, b"x")]));
        assert!(
            matches!(refusal, Refusal::Path { .. }),
            "zip {path:?}: {refusal:?}"
        );
        // Nothing was written and nothing is outside out/.
        assert_eq!(
            std::fs::read_dir(case.out()).unwrap().count(),
            0,
            "{path:?}"
        );

        // A tar drops a leading ./, cannot hold a NUL in a name, and the
        // header of the helper holds 100 bytes of it.
        if path != "./x.ass" && !path.contains('\0') && path.len() <= 100 {
            let case = Case::new();
            let refusal = refused(&case, "p.tar", &tar_of(&[(path, b"x")]));
            assert!(
                matches!(refusal, Refusal::Path { .. }),
                "tar {path:?}: {refusal:?}"
            );
        }
    }
    // The names a 7z can have too.
    for path in ["../x.ass", "/etc/x.ass", "a/../x.ass", "C:/x.ass"] {
        let case = Case::new();
        let refusal = refused(&case, "p.7z", &sevenz(&[(path, b"x")]));
        assert!(
            matches!(refusal, Refusal::Path { .. }),
            "7z {path:?}: {refusal:?}"
        );
    }
}

#[test]
fn a_path_over_the_limits_counts_the_nested_archives_in_front_of_it() {
    // 15 parts alone, 17 with the nested archive's path in front.
    let inner_path = vec!["d"; 15].join("/");
    let inner = zip_of(&[(&inner_path, b"x")]);
    let outer = zip_of(&[("a/inner.zip", &inner)]);
    let case = Case::new();
    assert!(matches!(
        refused(&case, "o.zip", &outer),
        Refusal::Path { .. }
    ));
}

#[test]
fn escaping_paths_in_the_samples_are_refused() {
    for name in ["esc.zip", "esc.tar", "rar5_esc.rar", "deeppath.zip"] {
        let refusal = refused_fixture(name);
        assert!(
            matches!(refusal, Refusal::Path { .. }),
            "{name}: {refusal:?}"
        );
    }
}

#[test]
fn a_rar_path_that_cannot_be_one_is_refused() {
    // The writer refuses the names that leave the folder (`../x.ass`, an
    // absolute one, a drive letter) itself, which is why `rar5_esc.rar`, in
    // `escaping_paths_in_the_samples_are_refused`, stands for a RAR 5 that
    // does. These are the names it writes, and each is built or the test fails.
    for (version, path) in [
        (ArchiveVersion::Rar50, "a//b.ass"),
        (ArchiveVersion::Rar50, "a/./b.ass"),
        (ArchiveVersion::Rar50, "a\\b.ass"),
        (ArchiveVersion::Rar29, "a//b.ass"),
        (ArchiveVersion::Rar29, "a/./b.ass"),
    ] {
        let mut builder = Builder::new(version);
        builder
            .add_bytes(path.as_bytes().to_vec(), b"x".to_vec(), None, None)
            .unwrap();
        let bytes = builder.to_bytes().unwrap();
        let case = Case::new();
        let refusal = refused(&case, "p.rar", &bytes);
        assert!(
            matches!(refusal, Refusal::Path { .. }),
            "{version:?} {path:?}: {refusal:?}"
        );
    }
}

// Links and special files

#[test]
fn links_are_refused_in_every_format() {
    for name in [
        "symlink.zip",
        "symlink7.zip",
        "symlink.7z",
        "links.tar",
        "rar5_links.rar",
    ] {
        let refusal = refused_fixture(name);
        assert!(
            matches!(refusal, Refusal::Link { .. }),
            "{name}: {refusal:?}"
        );
    }
}

#[test]
fn a_zip_symlink_and_a_tar_hard_link_are_refused() {
    let case = Case::new();
    let bytes = zip(&[ZipEntry::file("link.ass", b"../../etc/passwd").with_mode(0o120777)]);
    assert_eq!(
        refused(&case, "l.zip", &bytes),
        Refusal::Link {
            path: "link.ass".to_owned()
        }
    );
    let case = Case::new();
    let bytes = tar(&[
        TarEntry::file("a.ass", b"x"),
        TarEntry::link("hard.ass", b'1', "a.ass"),
    ]);
    assert_eq!(
        refused(&case, "l.tar", &bytes),
        Refusal::Link {
            path: "hard.ass".to_owned()
        }
    );
    let case = Case::new();
    let bytes = tar(&[TarEntry::link("sym.ass", b'2', "/etc/passwd")]);
    assert_eq!(
        refused(&case, "l.tar", &bytes),
        Refusal::Link {
            path: "sym.ass".to_owned()
        }
    );
}

#[test]
fn a_rar4_symlink_is_refused() {
    let mut builder = Builder::new(ArchiveVersion::Rar29);
    builder
        .add_unix_symlink(
            b"link.ass".to_vec(),
            b"/etc/passwd".to_vec(),
            false,
            None,
            None,
        )
        .unwrap();
    let bytes = builder.to_bytes().unwrap();
    let case = Case::new();
    let refusal = refused(&case, "l.rar", &bytes);
    assert!(matches!(refusal, Refusal::Link { .. }), "{refusal:?}");
}

#[test]
fn devices_and_fifos_are_refused() {
    for (kind, label) in [(b'3', "char"), (b'4', "block"), (b'6', "fifo")] {
        let case = Case::new();
        let bytes = tar(&[TarEntry::of_kind("dev", kind)]);
        assert_eq!(
            refused(&case, "s.tar", &bytes),
            Refusal::Special {
                path: "dev".to_owned()
            },
            "{label}"
        );
    }
    for mode in [0o010644, 0o020644, 0o060644, 0o140755] {
        let case = Case::new();
        let bytes = zip(&[ZipEntry::file("dev", b"").with_mode(mode)]);
        assert!(
            matches!(refused(&case, "s.zip", &bytes), Refusal::Special { .. }),
            "{mode:o}"
        );
    }
}

// Duplicates

#[test]
fn two_members_with_one_path_are_refused() {
    let case = Case::new();
    let bytes = zip_of(&[("a.ass", b"1"), ("b.ass", b"2"), ("a.ass", b"3")]);
    assert_eq!(
        refused(&case, "d.zip", &bytes),
        Refusal::Duplicate {
            path: "a.ass".to_owned()
        }
    );
    assert_eq!(std::fs::read_dir(case.out()).unwrap().count(), 0);
    let case = Case::new();
    let bytes = tar_of(&[("a.ass", b"1"), ("a.ass", b"2")]);
    assert_eq!(
        refused(&case, "d.tar", &bytes),
        Refusal::Duplicate {
            path: "a.ass".to_owned()
        }
    );
    let case = Case::new();
    let bytes = zip(&[ZipEntry::dir("a/"), ZipEntry::file("a", b"1")]);
    assert_eq!(
        refused(&case, "d.zip", &bytes),
        Refusal::Duplicate {
            path: "a".to_owned()
        }
    );
    // Two members of two nested archives are not the same path.
    let inner = zip_of(&[("a.ass", b"1")]);
    let outer = zip_of(&[("x/in.zip", &inner), ("y/in.zip", &inner)]);
    let case = Case::new();
    assert!(case.unpack("n.zip", &outer).is_ok());
}

/// More bytes than a comment can be (64 KiB), so that the last end record of
/// the ZIP in front of them is further from its end than the tail the
/// extraction looks at.
fn junk() -> Vec<u8> {
    vec![b'x'; 70 << 10]
}

/// Where the end record of `zip`, which has no comment, begins.
fn end_record(zip: &[u8]) -> usize {
    let at = zip.len() - 22;
    assert_eq!(&zip[at..at + 4], b"PK\x05\x06");
    at
}

#[test]
fn a_zip_whose_end_record_is_not_in_the_tail_is_refused_as_corrupt() {
    // The crate looks for an end record in the whole file, junk after its
    // comment included, so these would be read (and, the first, parsed whole
    // before the member limit is looked at) were they handed to it.
    let names: Vec<String> = (0..2001).map(|i| format!("f{i}.ass")).collect();
    let empty: Vec<(&str, &[u8])> = names.iter().map(|name| (name.as_str(), &b""[..])).collect();
    let twice = zip_of(&[("a.ass", b"1"), ("a.ass", b"2")]);
    for (label, mut bytes) in [("many", zip_of(&empty)), ("twice", twice)] {
        bytes.extend_from_slice(&junk());
        let case = Case::new();
        let refusal = refused(&case, "j.zip", &bytes);
        assert_eq!(
            refusal,
            Refusal::Corrupt {
                detail: "ZIP의 끝을 찾지 못했어요".to_owned()
            },
            "{label}"
        );
        assert_eq!(std::fs::read_dir(case.out()).unwrap().count(), 0, "{label}");
    }
    // A comment of the longest length is still found.
    let mut bytes = zip_of(&[("a.ass", b"1")]);
    let at = end_record(&bytes);
    bytes[at + 20..at + 22].copy_from_slice(&u16::MAX.to_le_bytes());
    bytes.extend_from_slice(&vec![b'c'; usize::from(u16::MAX)]);
    let case = Case::new();
    assert_eq!(case.unpack("c.zip", &bytes).unwrap().len(), 1);
}

#[test]
fn a_zip_with_a_bad_end_record_after_the_one_the_crate_uses_is_checked_against_the_crates() {
    // Two members of one path, and a last end record that says there is one
    // member and that the crate rejects (its directory is past itself), so the
    // crate falls back to the record before it, which counts both. What the
    // crate keeps is one member, as many as the last record counts.
    let mut bytes = zip_of(&[("a.ass", b"1"), ("a.ass", b"2")]);
    bytes.extend_from_slice(b"PK\x05\x06");
    bytes.extend_from_slice(&[0, 0, 0, 0]); // this disk, the directory's disk
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // the size of the directory
    bytes.extend_from_slice(&0x00ff_ffffu32.to_le_bytes()); // where it begins
    bytes.extend_from_slice(&0u16.to_le_bytes());
    let case = Case::new();
    assert_eq!(
        refused(&case, "f.zip", &bytes),
        Refusal::Duplicate {
            path: "a.ass".to_owned()
        }
    );
    assert_eq!(std::fs::read_dir(case.out()).unwrap().count(), 0);
}

#[test]
fn a_zip_whose_end_record_counts_fewer_members_than_its_directory_has_is_refused() {
    // The record says two members and the directory has three: the crate reads
    // two, and the third would be dropped without a word.
    let mut bytes = zip_of(&[("a.ass", b"1"), ("b.ass", b"2"), ("c.ass", b"3")]);
    let at = end_record(&bytes);
    bytes[at + 8..at + 10].copy_from_slice(&2u16.to_le_bytes());
    bytes[at + 10..at + 12].copy_from_slice(&2u16.to_le_bytes());
    let case = Case::new();
    let refusal = refused(&case, "s.zip", &bytes);
    assert!(matches!(refusal, Refusal::Corrupt { .. }), "{refusal:?}");
    assert_eq!(std::fs::read_dir(case.out()).unwrap().count(), 0);
    // The record that counts them all is fine.
    let case = Case::new();
    let bytes = zip_of(&[("a.ass", b"1"), ("b.ass", b"2"), ("c.ass", b"3")]);
    assert_eq!(case.unpack("s.zip", &bytes).unwrap().len(), 3);
}

#[test]
fn duplicates_in_a_7z_and_a_rar_are_refused() {
    let case = Case::new();
    let bytes = sevenz(&[("a.ass", b"1"), ("a.ass", b"2")]);
    assert_eq!(
        refused(&case, "d.7z", &bytes),
        Refusal::Duplicate {
            path: "a.ass".to_owned()
        }
    );
    let mut builder = Builder::new(ArchiveVersion::Rar50).allow_duplicate_names(true);
    builder
        .add_bytes(b"a.ass".to_vec(), b"1".to_vec(), None, None)
        .unwrap();
    builder
        .add_bytes(b"a.ass".to_vec(), b"2".to_vec(), None, None)
        .unwrap();
    let case = Case::new();
    assert_eq!(
        refused(&case, "d.rar", &builder.to_bytes().unwrap()),
        Refusal::Duplicate {
            path: "a.ass".to_owned()
        }
    );
}

// RAR members over the RAR limit

#[test]
fn a_rar_with_a_member_over_the_rar_member_limit_is_refused_before_extracting() {
    let limits = Limits {
        rar_member: 4000,
        ..Limits::default()
    };
    for version in [ArchiveVersion::Rar50, ArchiveVersion::Rar29] {
        let mut builder = Builder::new(version).store(true);
        builder
            .add_bytes(b"small.ass".to_vec(), sample(1000, 1), None, None)
            .unwrap();
        builder
            .add_bytes(b"dir/big.ttf".to_vec(), sample(5000, 2), None, None)
            .unwrap();
        let case = Case::new();
        let refusal = case.unpack_with("r.rar", &builder.to_bytes().unwrap(), &limits);
        assert_eq!(
            refusal,
            Err(Refusal::MemberTooLarge {
                path: "dir/big.ttf".to_owned(),
                limit: 4000
            }),
            "{version:?}"
        );
        // Nothing was extracted, the small member before it included.
        assert_eq!(std::fs::read_dir(case.out()).unwrap().count(), 0);
    }
    // The same archive is fine under the default limit.
    let mut builder = Builder::new(ArchiveVersion::Rar50).store(true);
    builder
        .add_bytes(b"dir/big.ttf".to_vec(), sample(5000, 2), None, None)
        .unwrap();
    let case = Case::new();
    assert!(case.unpack("r.rar", &builder.to_bytes().unwrap()).is_ok());
}

// Broken and strange archives

#[test]
fn nesting_bombs_and_odd_depths_are_refused() {
    assert_eq!(
        refused_fixture("nested30.zip"),
        Refusal::TooDeep { limit: 3 }
    );
}

#[test]
fn damaged_archives_are_corrupt_and_never_hang_or_panic() {
    let a = sample(3000, 1);
    let zipped = zip_of(&[("a.ass", &a)]);
    let mut bad_crc = zipped.clone();
    bad_crc[40] ^= 0xff; // a byte of the stored data
    let tarred = tar_of(&[("a.ass", &a)]);
    let mut bad_tar = tarred.clone();
    bad_tar[148] = b'9'; // the checksum
    let gzipped = gz(&a);
    let sevened = sevenz(&[("a.ass", &a)]);
    let mut bad_7z = sevened.clone();
    let last = bad_7z.len() - 1;
    bad_7z[last] ^= 0xff;
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("crc.zip", bad_crc),
        ("cut.zip", zipped[..zipped.len() - 30].to_vec()),
        ("bad.tar", bad_tar),
        ("cut.tar.gz", gzipped[..gzipped.len() / 2].to_vec()),
        ("cut.gz", gzipped[..gzipped.len() - 6].to_vec()),
        ("cut.bz2", bz2(&a)[..20].to_vec()),
        ("cut.xz", xz(&a)[..30].to_vec()),
        ("bad.7z", bad_7z),
        ("cut.7z", sevened[..sevened.len() / 2].to_vec()),
        ("cut.rar", rar_bytes()[..100].to_vec()),
    ];
    for (name, bytes) in cases {
        let case = Case::new();
        let refusal = refused(&case, name, &bytes);
        assert!(
            matches!(
                refusal,
                Refusal::Corrupt { .. } | Refusal::Unsupported { .. }
            ),
            "{name}: {refusal:?}"
        );
    }
}

fn rar_bytes() -> Vec<u8> {
    let mut builder = Builder::new(ArchiveVersion::Rar50);
    builder
        .add_bytes(b"a.ass".to_vec(), sample(3000, 1), None, None)
        .unwrap();
    builder.to_bytes().unwrap()
}

// Failures of the machine are not refusals of the archive

#[test]
fn an_output_folder_that_cannot_be_made_is_a_failure_not_a_refusal() {
    let bytes = zip_of(&[("a.ass", b"x")]);
    // Its parent does not exist.
    let case = Case::new();
    let part = case.write("a.zip", &bytes);
    let out = case.input().join("missing/out");
    let result = trss_archive::extract(&[part], "a.zip", &out, &Limits::default());
    let Err(ExtractError::Failed(message)) = result else {
        panic!("{result:?}");
    };
    assert!(
        message.starts_with("압축을 풀 폴더를 만들지 못했어요"),
        "{message}"
    );
    assert!(!out.exists());
    // It is there already: `out` must not exist.
    let case = Case::new();
    std::fs::create_dir(case.out()).unwrap();
    let part = case.write("a.zip", &bytes);
    let result = case.extract_raw(&[part], &Limits::default());
    assert!(matches!(result, Err(ExtractError::Failed(_))), "{result:?}");
}

#[test]
fn a_member_path_never_names_a_file_of_out() {
    // A member named like an index, or like a file already there, is not it.
    let case = Case::new();
    let bytes = zip_of(&[("0", b"first"), ("1", b"second"), ("2/0", b"third")]);
    let members = case.unpack("a.zip", &bytes).unwrap();
    let contents = case.contents(&members);
    assert_eq!(contents["0"], b"first");
    assert_eq!(contents["2/0"], b"third");
}
