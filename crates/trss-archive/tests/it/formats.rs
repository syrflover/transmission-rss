//! Each format unpacks, whole and split, and the names and the nesting come out right.

use crate::common;

use common::*;
use rars::{ArchiveVersion, Builder};
use trss_archive::{Limits, Refusal};

/// The two files most tests pack.
fn files() -> (Vec<u8>, Vec<u8>) {
    (sample(3000, 1), sample(500, 2))
}

fn assert_two_files(case: &Case, members: &[trss_archive::Member]) {
    let (a, b) = files();
    let contents = case.contents(members);
    assert_eq!(contents.len(), 2, "{members:?}");
    assert_eq!(contents["sub/a.ass"], a);
    assert_eq!(contents["b.srt"], b);
}

// ZIP

#[test]
fn a_zip_unpacks_with_its_folders_and_korean_names() {
    let (a, b) = files();
    let case = Case::new();
    let bytes = zip(&[
        ZipEntry::dir("sub/"),
        ZipEntry::file("sub/a.ass", &a),
        ZipEntry::file("b.srt", &b),
        ZipEntry::file("자막/1화.smi", b"<SAMI>"),
    ]);
    let members = case.unpack("pack.zip", &bytes).unwrap();
    assert_eq!(paths(&members), ["sub/a.ass", "b.srt", "자막/1화.smi"]);
    let files: Vec<&str> = members.iter().map(|m| m.file.as_str()).collect();
    assert_eq!(files, ["0", "1", "2"]);
    let contents = case.contents(&members);
    assert_eq!(contents["자막/1화.smi"], b"<SAMI>");
}

#[test]
fn a_deflated_zip_unpacks() {
    use std::io::Write;
    let (a, _) = files();
    let case = Case::new();
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    writer.start_file("sub/a.ass", options).unwrap();
    writer.write_all(&a).unwrap();
    writer.add_directory("sub2/", options).unwrap();
    writer.start_file("sub2/empty.txt", options).unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    let members = case.unpack("pack.zip", &bytes).unwrap();
    let contents = case.contents(&members);
    assert_eq!(contents["sub/a.ass"], a);
    assert_eq!(contents["sub2/empty.txt"], b"");
}

#[test]
fn an_empty_zip_unpacks_to_nothing() {
    let case = Case::new();
    let members = case.unpack("pack.zip", &zip(&[])).unwrap();
    assert!(members.is_empty());
    case.assert_contained();
}

#[test]
fn a_cp949_name_in_a_zip_without_the_utf8_flag_is_hangul() {
    let case = Case::new();
    let (name, _, _) = encoding_rs::EUC_KR.encode("자막/1화.smi");
    let entry = ZipEntry {
        name: &name,
        data: b"<SAMI>",
        mode: None,
        utf8: false,
    };
    let members = case.unpack("pack.zip", &zip(&[entry])).unwrap();
    assert_eq!(paths(&members), ["자막/1화.smi"]);
}

#[test]
fn a_zip_name_with_the_utf8_flag_is_used_as_it_is() {
    let case = Case::new();
    let members = case
        .unpack("pack.zip", &zip(&[ZipEntry::file("한글.ass", b"x")]))
        .unwrap();
    assert_eq!(paths(&members), ["한글.ass"]);
}

#[test]
fn a_zip_cut_into_pieces_unpacks_as_one() {
    let (a, b) = files();
    let bytes = zip_of(&[("sub/a.ass", &a), ("b.srt", &b)]);
    let case = Case::new();
    let cut = bytes.len() / 2;
    let parts = [
        case.write("pack.zip.001", &bytes[..cut]),
        case.write("pack.zip.002", &bytes[cut..]),
    ];
    let members = case.extract(&parts).unwrap();
    assert_two_files(&case, &members);
}

#[test]
fn the_first_piece_of_a_zip_alone_is_a_missing_volume() {
    let bytes = zip_of(&[("a.ass", &sample(2000, 3))]);
    let case = Case::new();
    let first = case.write("pack.zip.001", &bytes[..bytes.len() / 2]);
    assert_eq!(case.extract(&[first]), Err(Refusal::MissingVolume));
}

#[test]
fn a_zip_that_spans_disks_is_unsupported() {
    let mut bytes = zip_of(&[("a.ass", b"x")]);
    // The end record: this disk is the second one.
    let end = bytes.len() - 22;
    bytes[end + 4] = 1;
    let case = Case::new();
    assert!(matches!(
        case.unpack("pack.zip", &bytes),
        Err(Refusal::Unsupported { .. })
    ));
}

#[test]
fn the_first_piece_of_a_spanned_zip_is_unsupported() {
    let case = Case::new();
    let mut bytes = b"PK\x07\x08".to_vec();
    bytes.extend_from_slice(&zip_of(&[("a.ass", b"x")]));
    assert!(matches!(
        case.unpack("pack.z01", &bytes),
        Err(Refusal::Unsupported { .. })
    ));
}

// 7z

#[test]
fn a_7z_unpacks() {
    let (a, b) = files();
    let case = Case::new();
    let bytes = sevenz(&[("sub/", b""), ("sub/a.ass", &a), ("b.srt", &b)]);
    let members = case.unpack("pack.7z", &bytes).unwrap();
    assert_two_files(&case, &members);
}

#[test]
fn a_7z_with_an_empty_file_unpacks_it_too() {
    let (a, _) = files();
    let case = Case::new();
    let bytes = sevenz(&[("a.ass", &a), ("empty.txt", b"")]);
    let members = case.unpack("pack.7z", &bytes).unwrap();
    let contents = case.contents(&members);
    assert_eq!(contents["a.ass"], a);
    assert_eq!(contents["empty.txt"], b"");
}

#[test]
fn a_7z_cut_into_pieces_unpacks_as_one() {
    let (a, b) = files();
    let bytes = sevenz(&[("sub/a.ass", &a), ("b.srt", &b)]);
    let case = Case::new();
    let cut = bytes.len() / 3;
    let parts = [
        case.write("pack.7z.001", &bytes[..cut]),
        case.write("pack.7z.002", &bytes[cut..cut * 2]),
        case.write("pack.7z.003", &bytes[cut * 2..]),
    ];
    let members = case.extract(&parts).unwrap();
    assert_two_files(&case, &members);
}

#[test]
fn the_first_piece_of_a_7z_alone_is_a_missing_volume() {
    let bytes = sevenz(&[("a.ass", &sample(4000, 4))]);
    let case = Case::new();
    let first = case.write("pack.7z.001", &bytes[..bytes.len() / 2]);
    assert_eq!(case.extract(&[first]), Err(Refusal::MissingVolume));
}

#[test]
fn a_7z_with_a_ppmd_block_unpacks_when_its_memory_fits() {
    use sevenz_rust2::{encoder_options::PpmdOptions, ArchiveEntry, ArchiveWriter};
    let make = |memory: u32| {
        let mut writer = ArchiveWriter::new(std::io::Cursor::new(Vec::new())).unwrap();
        writer.set_content_methods(vec![PpmdOptions::from_order_memory_size(6, memory).into()]);
        writer
            .push_archive_entry(ArchiveEntry::new_file("a.ass"), Some(&b"[Script Info]"[..]))
            .unwrap();
        writer.finish().unwrap().into_inner()
    };
    let case = Case::new();
    let members = case.unpack("fits.7z", &make(8 << 20)).unwrap();
    assert_eq!(case.contents(&members)["a.ass"], b"[Script Info]");

    let case = Case::new();
    let over = case.unpack("over.7z", &make(100 << 20));
    assert_eq!(over, Err(Refusal::Dictionary { limit: 64 << 20 }));
}

// tar and the streams

#[test]
fn a_tar_unpacks_and_a_leading_dot_slash_is_dropped() {
    let (a, b) = files();
    let case = Case::new();
    let bytes = tar(&[
        TarEntry::of_kind("./", b'5'),
        TarEntry::of_kind("./sub/", b'5'),
        TarEntry::file("./sub/a.ass", &a),
        TarEntry::file("./b.srt", &b),
        // The extended header of the whole archive is no member.
        TarEntry {
            name: "pax_global_header",
            kind: b'g',
            data: b"52 comment=0123456789abcdef0123456789abcdef01234567\n",
            link: "",
        },
    ]);
    let members = case.unpack("pack.tar", &bytes).unwrap();
    assert_two_files(&case, &members);
}

#[test]
fn a_cp949_name_in_a_tar_is_hangul() {
    let case = Case::new();
    let (name, _, _) = encoding_rs::EUC_KR.encode("자막.ass");
    let mut bytes = tar_of(&[("placeholder", b"x")]);
    bytes[..11].fill(0);
    bytes[..name.len()].copy_from_slice(&name);
    // The checksum covers the name.
    bytes[148..156].copy_from_slice(b"        ");
    let sum: u32 = bytes[..512].iter().map(|&b| u32::from(b)).sum();
    bytes[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    let members = case.unpack("pack.tar", &bytes).unwrap();
    assert_eq!(paths(&members), ["자막.ass"]);
}

#[test]
fn a_compressed_tar_unpacks_by_its_content_or_its_name() {
    let (a, b) = files();
    let bytes = tar_of(&[("sub/a.ass", &a), ("b.srt", &b)]);
    for (name, packed) in [
        ("pack.tar.gz", gz(&bytes)),
        ("pack.tar.bz2", bz2(&bytes)),
        ("pack.tar.xz", xz(&bytes)),
        // The name that says it is a tar does not matter when the block does.
        ("pack.gz", gz(&bytes)),
        ("pack.tgz", gz(&bytes)),
        ("pack.tbz2", bz2(&bytes)),
        ("pack.txz", xz(&bytes)),
    ] {
        let case = Case::new();
        let members = case.unpack(name, &packed).unwrap();
        assert_two_files(&case, &members);
    }
}

#[test]
fn a_single_compressed_file_is_one_member_named_after_the_archive() {
    let (a, _) = files();
    for (name, packed, member) in [
        ("sub.ass.gz", gz(&a), "sub.ass"),
        ("sub.ass.bz2", bz2(&a), "sub.ass"),
        ("sub.ass.xz", xz(&a), "sub.ass"),
        (".gz", gz(&a), "unpacked"),
        ("noextension", gz(&a), "noextension"),
    ] {
        let case = Case::new();
        let members = case.unpack(name, &packed).unwrap();
        assert_eq!(paths(&members), [member], "{name}");
        assert_eq!(case.contents(&members)[member], a, "{name}");
    }
}

#[test]
fn concatenated_streams_unpack_as_one() {
    let case = Case::new();
    let mut packed = gz(b"first ");
    packed.extend_from_slice(&gz(b"second"));
    let members = case.unpack("two.txt.gz", &packed).unwrap();
    assert_eq!(case.contents(&members)["two.txt"], b"first second");

    let case = Case::new();
    let mut packed = xz(b"first ");
    packed.extend_from_slice(&xz(b"second"));
    let members = case.unpack("two.txt.xz", &packed).unwrap();
    assert_eq!(case.contents(&members)["two.txt"], b"first second");

    let case = Case::new();
    let mut packed = bz2(b"first ");
    packed.extend_from_slice(&bz2(b"second"));
    let members = case.unpack("two.txt.bz2", &packed).unwrap();
    assert_eq!(case.contents(&members)["two.txt"], b"first second");
}

#[test]
fn a_tar_cut_into_pieces_unpacks_as_one() {
    let (a, b) = files();
    let bytes = gz(&tar_of(&[("sub/a.ass", &a), ("b.srt", &b)]));
    let case = Case::new();
    let cut = bytes.len() / 2;
    let parts = [
        case.write("pack.tar.gz.001", &bytes[..cut]),
        case.write("pack.tar.gz.002", &bytes[cut..]),
    ];
    let members = case.extract(&parts).unwrap();
    assert_two_files(&case, &members);
}

#[test]
fn bytes_that_are_no_archive_are_unsupported() {
    for (name, bytes) in [
        ("pack.zip", &b"just some text, not an archive"[..]),
        ("empty.7z", b""),
    ] {
        let case = Case::new();
        assert!(
            matches!(case.unpack(name, bytes), Err(Refusal::Unsupported { .. })),
            "{name}"
        );
    }
}

// RAR

fn rar_builder(version: ArchiveVersion) -> Builder {
    let (a, b) = files();
    let mut builder = Builder::new(version);
    builder
        .add_bytes(b"sub/a.ass".to_vec(), a, None, None)
        .unwrap();
    builder.add_bytes(b"b.srt".to_vec(), b, None, None).unwrap();
    builder
}

#[test]
fn a_rar5_unpacks() {
    let case = Case::new();
    let bytes = rar_builder(ArchiveVersion::Rar50).to_bytes().unwrap();
    let members = case.unpack("pack.rar", &bytes).unwrap();
    assert_two_files(&case, &members);
}

#[test]
fn a_rar4_unpacks_and_its_backslashes_are_slashes() {
    let (a, b) = files();
    let case = Case::new();
    let mut builder = Builder::new(ArchiveVersion::Rar29);
    builder
        .add_bytes(b"sub\\a.ass".to_vec(), a, None, None)
        .unwrap();
    builder.add_bytes(b"b.srt".to_vec(), b, None, None).unwrap();
    let members = case
        .unpack("pack.rar", &builder.to_bytes().unwrap())
        .unwrap();
    assert_two_files(&case, &members);
}

#[test]
fn a_cp949_name_in_a_rar4_is_hangul() {
    let case = Case::new();
    let (name, _, _) = encoding_rs::EUC_KR.encode("자막/1화.smi");
    let mut builder = Builder::new(ArchiveVersion::Rar29);
    builder
        .add_bytes(name.to_vec(), b"<SAMI>".to_vec(), None, None)
        .unwrap();
    let members = case
        .unpack("pack.rar", &builder.to_bytes().unwrap())
        .unwrap();
    assert_eq!(paths(&members), ["자막/1화.smi"]);
}

/// A RAR of `sub/a.ass` (3000 bytes) and, in RAR 5 (the older formats' volumes
/// hold one member), `b.srt` (500 bytes), in volumes of `size` bytes.
fn rar_volumes(version: ArchiveVersion, size: usize) -> Vec<Vec<u8>> {
    let mut builder = Builder::new(version).store(true).volume_size(Some(size));
    builder
        .add_bytes(b"sub/a.ass".to_vec(), sample(3000, 1), None, Some(0o100644))
        .unwrap();
    if version == ArchiveVersion::Rar50 {
        builder
            .add_bytes(b"b.srt".to_vec(), sample(500, 2), None, Some(0o100644))
            .unwrap();
    }
    builder.build_volumes(None).unwrap()
}

#[test]
fn rar5_volumes_unpack_as_one() {
    let volumes = rar_volumes(ArchiveVersion::Rar50, 1200);
    assert!(volumes.len() >= 3, "{}", volumes.len());
    let case = Case::new();
    let parts: Vec<_> = volumes
        .iter()
        .enumerate()
        .map(|(i, bytes)| case.write(&format!("pack.part{}.rar", i + 1), bytes))
        .collect();
    let members = case.extract(&parts).unwrap();
    assert_two_files(&case, &members);
}

#[test]
fn rar4_volumes_unpack_as_one() {
    let volumes = rar_volumes(ArchiveVersion::Rar29, 1000);
    assert!(volumes.len() >= 3, "{}", volumes.len());
    let case = Case::new();
    let parts: Vec<_> = volumes
        .iter()
        .enumerate()
        .map(|(i, bytes)| {
            let name = if i == 0 {
                "pack.rar".to_owned()
            } else {
                format!("pack.r{:02}", i - 1)
            };
            case.write(&name, bytes)
        })
        .collect();
    let members = case.extract(&parts).unwrap();
    let contents = case.contents(&members);
    assert_eq!(contents.len(), 1);
    assert_eq!(contents["sub/a.ass"], sample(3000, 1));
}

#[test]
fn rar_volumes_with_one_missing_are_a_missing_volume() {
    for version in [ArchiveVersion::Rar50, ArchiveVersion::Rar29] {
        let volumes = rar_volumes(version, 1000);
        assert!(volumes.len() >= 3, "{version:?}: {}", volumes.len());
        let case = Case::new();
        let write = |i: usize| case.write(&format!("pack{i}.rar"), &volumes[i]);
        // The last one missing.
        let without_last: Vec<_> = (0..volumes.len() - 1).map(write).collect();
        assert_eq!(
            case.extract(&without_last),
            Err(Refusal::MissingVolume),
            "{version:?} without the last"
        );
        // The first one missing.
        let case = Case::new();
        let write = |i: usize| case.write(&format!("pack{i}.rar"), &volumes[i]);
        let without_first: Vec<_> = (1..volumes.len()).map(write).collect();
        assert_eq!(
            case.extract(&without_first),
            Err(Refusal::MissingVolume),
            "{version:?} without the first"
        );
    }
}

#[test]
fn a_rar_volume_set_without_a_middle_volume_is_refused() {
    // The volumes can tell a missing middle one only where they are numbered
    // (RAR 5); RAR 4's are damaged data then.
    let case = Case::new();
    let volumes = rar_volumes(ArchiveVersion::Rar50, 1000);
    assert!(volumes.len() >= 3);
    let mut parts = vec![case.write("a0.rar", &volumes[0])];
    for (i, volume) in volumes.iter().enumerate().skip(2) {
        parts.push(case.write(&format!("a{i}.rar"), volume));
    }
    assert_eq!(case.extract(&parts), Err(Refusal::MissingVolume));

    let case = Case::new();
    let volumes = rar_volumes(ArchiveVersion::Rar29, 600);
    assert!(volumes.len() >= 4, "{}", volumes.len());
    let mut parts = vec![case.write("a0.rar", &volumes[0])];
    for (i, volume) in volumes.iter().enumerate().skip(2) {
        parts.push(case.write(&format!("a.r{:02}", i - 1), volume));
    }
    let refusal = case.extract(&parts);
    assert!(
        matches!(
            refusal,
            Err(Refusal::MissingVolume | Refusal::Corrupt { .. })
        ),
        "{refusal:?}"
    );
}

// Nesting

/// `inner` as a member `inner.zip` in a ZIP, `depth` times: depth 1 is a ZIP
/// holding `a.ass` in `sub/inner.zip`.
fn zips_in_zips(depth: u32) -> Vec<u8> {
    let mut bytes = zip_of(&[("a.ass", b"[Script Info]")]);
    for _ in 0..depth {
        bytes = zip_of(&[("sub/inner.zip", &bytes)]);
    }
    bytes
}

#[test]
fn an_archive_in_an_archive_is_unpacked_in_place() {
    let case = Case::new();
    let members = case.unpack("pack.zip", &zips_in_zips(1)).unwrap();
    assert_eq!(paths(&members), ["sub/inner.zip/a.ass"]);
    let contents = case.contents(&members);
    assert_eq!(contents["sub/inner.zip/a.ass"], b"[Script Info]");
}

#[test]
fn three_archives_deep_is_the_limit() {
    let case = Case::new();
    let members = case.unpack("pack.zip", &zips_in_zips(2)).unwrap();
    assert_eq!(paths(&members), ["sub/inner.zip/sub/inner.zip/a.ass"]);
    case.contents(&members);

    let case = Case::new();
    let refused = case.unpack("pack.zip", &zips_in_zips(3));
    assert_eq!(refused, Err(Refusal::TooDeep { limit: 3 }));
    case.assert_contained();
}

#[test]
fn nested_archives_of_every_format_are_unpacked() {
    let (a, _) = files();
    let tarball = gz(&tar_of(&[("x.ass", &a)]));
    let case = Case::new();
    let bytes = zip(&[
        ZipEntry::file("seven.7z", &sevenz(&[("s.ass", &a)])),
        ZipEntry::file("TAR.TGZ", &tarball),
        ZipEntry::file("one.ass.xz", &xz(&a)),
        ZipEntry::file(
            "r.rar",
            &rar_builder(ArchiveVersion::Rar50).to_bytes().unwrap(),
        ),
        ZipEntry::file("plain.ass", &a),
    ]);
    let members = case.unpack("pack.zip", &bytes).unwrap();
    assert_eq!(
        paths(&members),
        [
            "seven.7z/s.ass",
            "TAR.TGZ/x.ass",
            "one.ass.xz/one.ass",
            "r.rar/sub/a.ass",
            "r.rar/b.srt",
            "plain.ass"
        ]
    );
    case.contents(&members);
}

#[test]
fn a_member_that_is_not_an_archive_by_its_name_or_its_bytes_stays_a_member() {
    let case = Case::new();
    // A .docx is a ZIP, and a .zip that is not one is a file.
    let docx = zip_of(&[("word/document.xml", b"<w/>")]);
    let bytes = zip(&[
        ZipEntry::file("doc.docx", &docx),
        ZipEntry::file("fake.zip", b"not a zip"),
        ZipEntry::file("empty.zip", b""),
    ]);
    let members = case.unpack("pack.zip", &bytes).unwrap();
    assert_eq!(paths(&members), ["doc.docx", "fake.zip", "empty.zip"]);
    let contents = case.contents(&members);
    assert_eq!(contents["doc.docx"], docx);
}

#[test]
fn the_member_limit_is_shared_by_the_whole_nesting() {
    let limits = Limits {
        members: 3,
        ..Limits::default()
    };
    let inner = zip_of(&[("a.ass", b"1"), ("b.ass", b"2")]);
    // Outer: inner.zip and c.ass (2), inner: 2 more.
    let outer = zip_of(&[("inner.zip", &inner), ("c.ass", b"3")]);
    let case = Case::new();
    assert_eq!(
        case.unpack_with("pack.zip", &outer, &limits),
        Err(Refusal::TooManyMembers { limit: 3 })
    );
    let limits = Limits {
        members: 4,
        ..Limits::default()
    };
    let case = Case::new();
    assert!(case.unpack_with("pack.zip", &outer, &limits).is_ok());
}

#[test]
fn the_size_limit_is_shared_by_the_whole_nesting() {
    // 2 KiB in the outer one and 2 KiB in the inner one, counting the
    // archive itself, which was written before it was unpacked.
    let inner = zip_of(&[("a.ass", &sample(1500, 5))]);
    let outer = zip_of(&[("inner.zip", &inner), ("c.ass", &sample(1000, 6))]);
    let limits = Limits {
        total: 3000,
        ..Limits::default()
    };
    let case = Case::new();
    assert_eq!(
        case.unpack_with("pack.zip", &outer, &limits),
        Err(Refusal::TooLarge { limit: 3000 })
    );
}
