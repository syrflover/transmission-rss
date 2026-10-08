//! The move's checks and renames on real temporary folders, and the Transmission
//! step against the fake Transmission (`trss_transmission::fake`): which
//! torrents move, what stops the move, and how it waits for Transmission. The
//! command around the move is in `run_tests.rs`, and the interplay with the
//! cycle and the commands' turns in trss-worker's `tests/it/archive_move.rs`.

use std::{
    fs,
    os::unix::fs::{symlink, MetadataExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use tokio_util::sync::CancellationToken;
use trss_transmission::fake::{FakeTorrent, FakeTransmission};

use super::*;
use crate::test_world::{files, write};

struct Folders {
    _tmp: tempfile::TempDir,
    collect: PathBuf,
    archive: PathBuf,
    outside: PathBuf,
}

fn folders() -> Folders {
    let tmp = tempfile::tempdir().unwrap();
    let collect = tmp.path().join("Shows (current)");
    let archive = tmp.path().join("Shows");
    let outside = tmp.path().join("elsewhere");
    for dir in [&collect, &archive, &outside] {
        fs::create_dir(dir).unwrap();
    }
    Folders {
        _tmp: tmp,
        collect,
        archive,
        outside,
    }
}

fn archive_request(f: &Folders, name: &str) -> Request {
    Request {
        from_root: f.collect.clone(),
        from: Side::Collect,
        to_root: f.archive.clone(),
        to: Side::Archive,
        name: name.to_owned(),
    }
}

fn restore_request(f: &Folders, name: &str) -> Request {
    Request {
        from_root: f.archive.clone(),
        from: Side::Archive,
        to_root: f.collect.clone(),
        to: Side::Collect,
        name: name.to_owned(),
    }
}

/// `move_entries` of the work folder `X` from the collect to the archive folder.
fn entries(f: &Folders) -> Result<bool, MoveError> {
    move_entries(
        &f.collect.join("X"),
        &f.archive.join("X"),
        Side::Archive,
        &RealDisk,
        &CancellationToken::new(),
    )
}

/// A disk on which everything under `other` is another filesystem.
struct SplitDisk {
    other: PathBuf,
}

impl Disk for SplitDisk {
    fn device(&self, path: &Path, metadata: &fs::Metadata) -> u64 {
        if path.starts_with(&self.other) {
            metadata.dev() + 1
        } else {
            metadata.dev()
        }
    }
}

#[test]
fn a_work_folder_name_is_one_ordinary_component() {
    for good in ["Clevatess", "Sono Bisque Doll (2025)", ".trss"] {
        assert!(is_work_folder_name(good), "{good}");
    }
    for bad in ["", ".", "..", "a/b", "/abs", "a/", "./a"] {
        assert!(!is_work_folder_name(bad), "{bad}");
    }
}

#[test]
fn the_checks_list_every_file_on_both_sides_and_let_shared_directories_merge() {
    let f = folders();
    write(&f.collect.join("Clevatess/Season 02/e01.mkv"), "new");
    write(&f.collect.join("Clevatess/Season 02/e02.mkv"), "new");
    write(&f.collect.join("Clevatess/.trss/subs/e01.ass"), "sub");
    // Season 01 is archived already: a shared directory, no conflict.
    write(&f.archive.join("Clevatess/Season 01/e01.mkv"), "old");
    write(&f.archive.join("Clevatess/.trss/subs/s1e01.ass"), "sub");
    let request = archive_request(&f, "Clevatess");
    check(&request, &RealDisk).unwrap();

    // The same relative file on both sides refuses the whole move.
    write(&f.archive.join("Clevatess/Season 02/e02.mkv"), "other");
    write(&f.archive.join("Clevatess/.trss/subs/e01.ass"), "other");
    let err = check(&request, &RealDisk).unwrap_err();
    assert!(err.contains("`.trss/subs/e01.ass`"), "{err}");
    assert!(err.contains("`Season 02/e02.mkv`"), "{err}");
    assert!(err.contains("보관 폴더의 `Clevatess`"), "{err}");
    assert!(err.contains("아무것도 옮기지 않았어요"), "{err}");
    assert!(err.contains("두 쪽을 견줘"), "{err}");
    assert!(!err.contains("정리"), "{err}");

    // A file where the other side has a directory is a conflict too.
    let f = folders();
    write(&f.collect.join("X/Season 01"), "a file");
    write(&f.archive.join("X/Season 01/e01.mkv"), "old");
    let err = check(&archive_request(&f, "X"), &RealDisk).unwrap_err();
    assert!(err.contains("`Season 01`"), "{err}");
}

#[test]
fn a_long_conflict_list_is_cut_short() {
    let f = folders();
    for n in 0..8 {
        write(&f.collect.join(format!("X/e{n}.mkv")), "a");
        write(&f.archive.join(format!("X/e{n}.mkv")), "b");
    }
    let err = check(&archive_request(&f, "X"), &RealDisk).unwrap_err();
    assert!(err.contains("`e4.mkv` 외 3개"), "{err}");
    assert!(!err.contains("e5.mkv"), "{err}");
}

#[test]
fn links_out_of_the_two_folders_are_refused_and_links_inside_are_not() {
    let f = folders();
    // The work folder itself a link (to a folder outside).
    fs::create_dir(f.outside.join("Clevatess")).unwrap();
    symlink(f.outside.join("Clevatess"), f.collect.join("Clevatess")).unwrap();
    let err = check(&archive_request(&f, "Clevatess"), &RealDisk).unwrap_err();
    assert!(err.contains("링크"), "{err}");

    // A link inside it that leads out.
    let f = folders();
    write(&f.collect.join("X/Season 01/e01.mkv"), "a");
    write(&f.outside.join("secret.txt"), "b");
    symlink(
        f.outside.join("secret.txt"),
        f.collect.join("X/Season 01/extra"),
    )
    .unwrap();
    let err = check(&archive_request(&f, "X"), &RealDisk).unwrap_err();
    assert!(err.contains("밖을 가리키는 링크"), "{err}");

    // A dangling link counts as leading out.
    let f = folders();
    write(&f.collect.join("X/e01.mkv"), "a");
    symlink(f.outside.join("gone"), f.collect.join("X/dangling")).unwrap();
    assert!(check(&archive_request(&f, "X"), &RealDisk).is_err());

    // One that stays inside the collect folder is moved like a file.
    let f = folders();
    write(&f.collect.join("X/e01.mkv"), "a");
    symlink(f.collect.join("X/e01.mkv"), f.collect.join("X/latest.mkv")).unwrap();
    check(&archive_request(&f, "X"), &RealDisk).unwrap();

    // The destination's work folder a link.
    let f = folders();
    write(&f.collect.join("X/e01.mkv"), "a");
    symlink(&f.outside, f.archive.join("X")).unwrap();
    assert!(check(&archive_request(&f, "X"), &RealDisk)
        .unwrap_err()
        .contains("링크"));
}

#[test]
fn folders_on_different_filesystems_are_refused() {
    let f = folders();
    write(&f.collect.join("X/e01.mkv"), "a");
    let disk = SplitDisk {
        other: f.archive.clone(),
    };
    let err = check(&archive_request(&f, "X"), &disk).unwrap_err();
    assert!(err.contains("다른 파일시스템"), "{err}");

    // A folder inside the work folder mounted from elsewhere, too.
    let f = folders();
    write(&f.collect.join("X/mnt/e01.mkv"), "a");
    let disk = SplitDisk {
        other: f.collect.join("X/mnt"),
    };
    let err = check(&archive_request(&f, "X"), &disk).unwrap_err();
    assert!(err.contains("다른 파일시스템"), "{err}");
}

#[test]
fn a_destination_on_another_filesystem_is_refused_even_below_the_same_root() {
    // The work folder at the destination is a mount of its own.
    let f = folders();
    write(&f.collect.join("X/Season 02/e01.mkv"), "a");
    fs::create_dir(f.archive.join("X")).unwrap();
    let disk = SplitDisk {
        other: f.archive.join("X"),
    };
    let err = check(&archive_request(&f, "X"), &disk).unwrap_err();
    assert!(err.contains("다른 파일시스템"), "{err}");

    // So is a directory the merge would go into.
    let f = folders();
    write(&f.collect.join("X/Season 01/e02.mkv"), "a");
    write(&f.archive.join("X/Season 01/e01.mkv"), "old");
    let disk = SplitDisk {
        other: f.archive.join("X/Season 01"),
    };
    let err = check(&archive_request(&f, "X"), &disk).unwrap_err();
    assert!(err.contains("다른 파일시스템"), "{err}");
}

#[test]
fn nested_or_missing_folders_and_bad_names_are_refused() {
    let f = folders();
    let mut request = archive_request(&f, "X");
    request.to_root = f.collect.join("inner");
    fs::create_dir(&request.to_root).unwrap();
    assert!(check(&request, &RealDisk).unwrap_err().contains("안에"));

    let mut request = archive_request(&f, "X");
    request.to_root = f.outside.join("missing");
    assert!(check(&request, &RealDisk)
        .unwrap_err()
        .contains("보관 폴더"));

    assert!(check(&archive_request(&f, "../elsewhere"), &RealDisk).is_err());
}

#[test]
fn a_rename_never_replaces_a_file_or_a_directory() {
    let f = folders();
    write(&f.collect.join("a"), "a");
    write(&f.archive.join("a"), "b");
    let err = rename_noreplace(&f.collect.join("a"), &f.archive.join("a")).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(f.archive.join("a")).unwrap(), "b");

    // An empty directory is not replaced either, as plain rename(2) would.
    fs::create_dir(f.collect.join("d")).unwrap();
    fs::create_dir(f.archive.join("d")).unwrap();
    let err = rename_noreplace(&f.collect.join("d"), &f.archive.join("d")).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
}

#[test]
fn the_renames_move_a_whole_folder_or_merge_it_and_clear_what_they_emptied() {
    // Nothing at the destination: one rename.
    let f = folders();
    write(&f.collect.join("X/Season 02/e01.mkv"), "a");
    write(&f.collect.join("X/.trss/fonts/f.ttf"), "f");
    let before = fs::metadata(f.collect.join("X")).unwrap().ino();
    assert!(entries(&f).unwrap());
    assert!(!f.collect.join("X").exists());
    assert_eq!(fs::metadata(f.archive.join("X")).unwrap().ino(), before);
    assert_eq!(
        files(&f.archive.join("X")),
        [".trss/fonts/f.ttf", "Season 02/e01.mkv"]
    );

    // A destination with the work folder: merged, file by file where shared.
    let f = folders();
    write(&f.collect.join("X/Season 02/e01.mkv"), "a");
    write(&f.collect.join("X/Season 02/e02.mkv"), "a");
    write(&f.collect.join("X/.trss/s2.ass"), "s");
    write(&f.archive.join("X/Season 01/e01.mkv"), "old");
    write(
        &f.archive.join("X/Season 02/e03.mkv"),
        "moved by Transmission",
    );
    write(&f.archive.join("X/.trss/s1.ass"), "s");
    assert!(entries(&f).unwrap());
    assert!(!f.collect.join("X").exists(), "emptied folders are removed");
    assert_eq!(
        files(&f.archive.join("X")),
        [
            ".trss/s1.ass",
            ".trss/s2.ass",
            "Season 01/e01.mkv",
            "Season 02/e01.mkv",
            "Season 02/e02.mkv",
            "Season 02/e03.mkv",
        ]
    );

    // Nothing at the source: nothing to do.
    assert!(!entries(&f).unwrap());
}

#[test]
fn a_file_that_appeared_at_the_destination_is_left_at_the_source_and_named() {
    let f = folders();
    write(&f.collect.join("X/S/e01.mkv"), "mine");
    write(&f.collect.join("X/S/e02.mkv"), "mine");
    write(&f.archive.join("X/S/e01.mkv"), "theirs");
    let Err(MoveError::Failed(err)) = entries(&f) else {
        panic!("not refused");
    };
    assert!(err.contains("`S/e01.mkv`"), "{err}");
    assert!(err.contains("두 쪽을 견줘"), "{err}");
    assert!(!err.contains("정리"), "{err}");
    assert_eq!(
        fs::read_to_string(f.archive.join("X/S/e01.mkv")).unwrap(),
        "theirs"
    );
    assert_eq!(
        fs::read_to_string(f.collect.join("X/S/e01.mkv")).unwrap(),
        "mine"
    );
    // The rest moved; a rerun after the conflict is cleared moves the file.
    assert_eq!(files(&f.archive.join("X")), ["S/e01.mkv", "S/e02.mkv"]);
    fs::remove_file(f.archive.join("X/S/e01.mkv")).unwrap();
    assert!(entries(&f).unwrap());
    assert!(!f.collect.join("X").exists());
}

#[test]
fn a_torrent_moves_when_its_data_is_in_the_work_folder() {
    let request = Request {
        from_root: PathBuf::from("/downloads/Shows (current)"),
        from: Side::Collect,
        to_root: PathBuf::from("/downloads/Shows"),
        to: Side::Archive,
        name: "Clevatess".into(),
    };
    let (work, root) = (request.source(), request.from_root.clone());
    let at = |dir: &str, name: &str| new_location(Path::new(dir), name, &work, &root, &request);
    assert_eq!(
        at("/downloads/Shows (current)/Clevatess/Season 02", "e.mkv"),
        Some(PathBuf::from("/downloads/Shows/Clevatess/Season 02"))
    );
    assert_eq!(
        at("/downloads/Shows (current)/Clevatess/", "e.mkv"),
        Some(PathBuf::from("/downloads/Shows/Clevatess"))
    );
    // A torrent whose own folder is the work folder, right in the collect folder.
    assert_eq!(
        at("/downloads/Shows (current)", "Clevatess"),
        Some(PathBuf::from("/downloads/Shows"))
    );
    for (dir, name) in [
        ("/downloads/Shows (current)/Clevatess2/Season 01", "e.mkv"),
        ("/downloads/Shows (current)", "Clevatess S02E01.mkv"),
        ("/downloads/Shows (current)/Other", "Clevatess"),
        ("/downloads/downloads", "e.mkv"),
    ] {
        assert_eq!(at(dir, name), None, "{dir} {name}");
    }
}

fn place(hash: &str, name: &str, dir: &Path) -> TorrentPlace {
    TorrentPlace {
        hash: hash.into(),
        name: name.into(),
        download_dir: dir.to_str().unwrap().into(),
        files: vec![trss_transmission::TorrentFile {
            name: name.into(),
            length: 5,
            complete: true,
        }],
        ..TorrentPlace::default()
    }
}

#[test]
fn torrents_are_matched_by_where_their_folders_really_are() {
    let f = folders();
    write(&f.collect.join("X/Season 02/e01.mkv"), "a");
    write(&f.collect.join("Other/Season 01/o01.mkv"), "o");
    symlink(f.collect.join("X"), f.collect.join("Alias")).unwrap();
    let request = archive_request(&f, "X");

    // A link inside the work folder to itself: both readings agree, and it
    // moves to where it really is.
    symlink(f.collect.join("X/Season 02"), f.collect.join("X/Latest")).unwrap();
    let moves = plan_torrents(
        &request,
        &[
            place("a", "e01.mkv", &f.collect.join("X/Latest")),
            place("b", "o01.mkv", &f.collect.join("Other/Season 01")),
        ],
    )
    .unwrap();
    assert_eq!(moves.len(), 1, "{moves:?}");
    assert_eq!(moves[0].hash, "a");
    assert_eq!(moves[0].to, f.archive.join("X/Season 02"));

    // Through a link into the work folder from outside, or out of it from
    // inside: the text and the disk disagree, and the move is refused.
    symlink(
        f.collect.join("Other/Season 01"),
        f.collect.join("X/Borrowed"),
    )
    .unwrap();
    for dir in ["Alias/Season 02", "X/Borrowed"] {
        let err =
            plan_torrents(&request, &[place("c", "e01.mkv", &f.collect.join(dir))]).unwrap_err();
        assert!(err.contains("링크"), "{dir}: {err}");
        assert!(err.contains("`e01.mkv`"), "{dir}: {err}");
    }

    // `..` that lands elsewhere is not this work's; one that lands in it
    // refuses the move.
    let elsewhere = place("c", "o01.mkv", &f.collect.join("X/../Other/Season 01"));
    assert!(plan_torrents(&request, std::slice::from_ref(&elsewhere))
        .unwrap()
        .is_empty());
    let inside = place("d", "e01.mkv", &f.collect.join("Other/../X/Season 02"));
    let err = plan_torrents(&request, &[elsewhere, inside]).unwrap_err();
    assert!(err.contains("`..`"), "{err}");

    // A work folder that is gone still has its torrents found (by text).
    let f = folders();
    let request = archive_request(&f, "X");
    let moves = plan_torrents(&request, &[place("e", "e01.mkv", &f.collect.join("X/S"))]).unwrap();
    assert_eq!(moves[0].to, f.archive.join("X/S"));
}

#[test]
fn a_torrent_transmission_still_writes_or_reports_an_error_for_refuses_the_move() {
    let f = folders();
    let request = archive_request(&f, "X");
    let dir = f.collect.join("X/S");
    let unfinished = TorrentPlace {
        unfinished: true,
        ..place("a", "e02.mkv", &dir)
    };
    let err = plan_torrents(&request, &[place("b", "e01.mkv", &dir), unfinished]).unwrap_err();
    assert!(err.contains("`e02.mkv`"), "{err}");
    assert!(err.contains("지운 뒤 `다시 옮기기`"), "{err}");
    // A restore says to restore again instead.
    let back = Request {
        from_root: f.archive.clone(),
        from: Side::Archive,
        to_root: f.collect.clone(),
        to: Side::Collect,
        name: "X".into(),
    };
    let stuck = TorrentPlace {
        unfinished: true,
        ..place("g", "e07.mkv", &f.archive.join("X/S"))
    };
    let err = plan_torrents(&back, &[stuck]).unwrap_err();
    assert!(err.contains("`e07.mkv`"), "{err}");
    assert!(err.contains("다시 `복원`해"), "{err}");

    let broken = TorrentPlace {
        local_error: Some("No data found!".into()),
        ..place("c", "e03.mkv", &dir)
    };
    let err = plan_torrents(&request, &[broken]).unwrap_err();
    assert!(err.contains("No data found!"), "{err}");
}

#[test]
fn a_torrent_file_the_destination_has_refuses_the_move_unless_it_is_only_there() {
    let f = folders();
    let request = archive_request(&f, "X");
    let (from, to) = (f.collect.join("X/S"), f.archive.join("X/S"));
    write(&from.join("e01.mkv"), "mine");
    // Its `.part` name at the destination: Transmission's move would meet it.
    write(&to.join("e01.mkv.part"), "theirs");
    let err = plan_torrents(&request, &[place("a", "e01.mkv", &from)]).unwrap_err();
    assert!(err.contains("`S/e01.mkv.part`"), "{err}");

    // A file not complete counts wherever its data is.
    let f = folders();
    let request = archive_request(&f, "X");
    let (from, to) = (f.collect.join("X/S"), f.archive.join("X/S"));
    write(&to.join("e02.mkv"), "theirs");
    let mut partial = place("b", "e02.mkv", &from);
    partial.files[0].complete = false;
    let err = plan_torrents(&request, &[partial]).unwrap_err();
    assert!(err.contains("`S/e02.mkv`"), "{err}");

    // A complete file found only at the destination, with the torrent's size
    // for it, is the torrent's own, moved by an earlier start or by hand.
    write(&to.join("e03.mkv"), "moved");
    let moves = plan_torrents(&request, &[place("c", "e03.mkv", &from)]).unwrap();
    assert_eq!(moves[0].to, to);

    // Another size is another file that shares the name.
    write(&to.join("e04.mkv"), "a different file");
    let err = plan_torrents(&request, &[place("d", "e04.mkv", &from)]).unwrap_err();
    assert!(err.contains("`S/e04.mkv`"), "{err}");

    // Anything is refused while a file of the torrent is still at its
    // folder; one with its name on both sides is named as a conflict.
    let mut both = place("e", "e03.mkv", &from);
    both.files.push(trss_transmission::TorrentFile {
        name: "e05.mkv".into(),
        length: 5,
        complete: true,
    });
    write(&from.join("e03.mkv"), "still");
    let err = plan_torrents(&request, &[both]).unwrap_err();
    assert!(err.contains("`S/e03.mkv`"), "{err}");
    assert!(err.contains("견줘"), "{err}");
    assert!(!err.contains("나뉘어"), "{err}");
    fs::remove_file(from.join("e03.mkv")).unwrap();

    // And only its `.part` name at the destination.
    write(&to.join("e06.mkv.part"), "moved");
    let err = plan_torrents(&request, &[place("f", "e06.mkv", &from)]).unwrap_err();
    assert!(err.contains("`S/e06.mkv.part`"), "{err}");
}

fn two_files(hash: &str, name: &str, dir: &Path, files: [&str; 2]) -> TorrentPlace {
    TorrentPlace {
        files: files
            .into_iter()
            .map(|name| trss_transmission::TorrentFile {
                name: name.into(),
                length: 5,
                complete: true,
            })
            .collect(),
        ..place(hash, name, dir)
    }
}

#[test]
fn a_torrent_split_between_the_two_folders_is_named_without_asking_to_clear_a_side() {
    // Some of its files still in its folder, the rest at the destination,
    // no name on both sides: each side holds the only copy of its files.
    let f = folders();
    let (from, to) = (f.collect.join("X/S"), f.archive.join("X/S"));
    write(&from.join("e04.mkv"), "still");
    write(&to.join("e05.mkv"), "moved");
    let split = two_files("a", "Pack", &from, ["e04.mkv", "e05.mkv"]);
    let err = plan_torrents(&archive_request(&f, "X"), &[split]).unwrap_err();
    assert!(err.contains("아무것도 옮기지 않았어요"), "{err}");
    assert!(err.contains("`Pack`"), "{err}");
    assert!(err.contains("수집 폴더와 보관 폴더에 나뉘어"), "{err}");
    assert!(err.contains("어느 쪽도 지우지 말고"), "{err}");
    assert!(err.contains("남은 파일을 한쪽으로 모으거나"), "{err}");
    assert!(err.contains("Transmission에서 그 토렌트의 위치를"), "{err}");
    assert!(err.contains("`다시 옮기기`를 눌러"), "{err}");
    assert!(!err.contains("정리"), "{err}");
    assert!(!err.contains("`S/e05.mkv`"), "{err}");

    // Toward the collect folder, the retry is `복원`.
    let f = folders();
    let (from, to) = (f.archive.join("X/S"), f.collect.join("X/S"));
    write(&from.join("e04.mkv"), "still");
    write(&to.join("e05.mkv"), "moved");
    let split = two_files("a", "Pack", &from, ["e04.mkv", "e05.mkv"]);
    let err = plan_torrents(&restore_request(&f, "X"), std::slice::from_ref(&split)).unwrap_err();
    assert!(err.contains("보관 폴더와 수집 폴더에 나뉘어"), "{err}");
    assert!(err.contains("다시 `복원`해"), "{err}");

    // A file at the destination that is not the torrent's size is another
    // file: named as a conflict, with the split torrent beside it.
    write(&to.join("e05.mkv"), "a different file");
    let err = plan_torrents(&restore_request(&f, "X"), &[split]).unwrap_err();
    assert!(err.contains("`S/e05.mkv`"), "{err}");
    assert!(!err.contains("나뉘어"), "{err}");

    // A conflict and a split torrent together: both are told.
    write(&to.join("e05.mkv"), "moved");
    write(&to.join("e07.mkv"), "theirs!");
    let other = place("b", "e07.mkv", &from);
    write(&from.join("e07.mkv"), "mine!");
    let split = two_files("a", "Pack", &from, ["e04.mkv", "e05.mkv"]);
    let err = plan_torrents(&restore_request(&f, "X"), &[split, other]).unwrap_err();
    assert!(err.contains("`S/e07.mkv`"), "{err}");
    assert!(err.contains("`Pack`"), "{err}");
    assert!(err.contains("나뉘어"), "{err}");
}

/// A disk whose renames fail with `errno` for paths under `under`.
struct FailingDisk {
    under: PathBuf,
    errno: rustix::io::Errno,
}

impl Disk for FailingDisk {
    fn device(&self, _path: &Path, metadata: &fs::Metadata) -> u64 {
        metadata.dev()
    }

    fn rename_noreplace(&self, from: &Path, to: &Path) -> io::Result<()> {
        if from.starts_with(&self.under) || to.starts_with(&self.under) {
            return Err(self.errno.into());
        }
        rename_noreplace(from, to)
    }
}

#[test]
fn a_filesystem_that_cannot_rename_without_replacing_is_refused_before_anything_moves() {
    for errno in [rustix::io::Errno::INVAL, rustix::io::Errno::NOSYS] {
        let f = folders();
        write(&f.collect.join("X/e01.mkv"), "a");
        let disk = FailingDisk {
            under: f.archive.clone(),
            errno,
        };
        let err = check(&archive_request(&f, "X"), &disk).unwrap_err();
        assert!(err.contains("RENAME_NOREPLACE"), "{err}");
        // The probe left nothing behind.
        assert_eq!(files(&f.collect), ["X/e01.mkv"]);
        assert!(files(&f.archive).is_empty());
    }
    // A real one passes and leaves nothing either.
    let f = folders();
    write(&f.collect.join("X/e01.mkv"), "a");
    check(&archive_request(&f, "X"), &RealDisk).unwrap();
    assert_eq!(files(&f.collect), ["X/e01.mkv"]);
    assert!(files(&f.archive).is_empty());
}

#[test]
fn a_rename_across_filesystems_stops_the_merge_with_the_reason() {
    let f = folders();
    write(&f.collect.join("X/a/e01.mkv"), "a");
    write(&f.collect.join("X/b/e02.mkv"), "b");
    write(&f.archive.join("X/a/old.mkv"), "old");
    write(&f.archive.join("X/b/old.mkv"), "old");
    let disk = FailingDisk {
        under: f.archive.join("X/a"),
        errno: rustix::io::Errno::XDEV,
    };
    let err = move_entries(
        &f.collect.join("X"),
        &f.archive.join("X"),
        Side::Archive,
        &disk,
        &CancellationToken::new(),
    )
    .unwrap_err();
    let MoveError::Failed(reason) = err else {
        panic!("{err:?}");
    };
    assert!(reason.contains("다른 파일시스템"), "{reason}");
    // It stopped there: `b` was not merged after the failure.
    assert_eq!(files(&f.collect.join("X")), ["a/e01.mkv", "b/e02.mkv"]);
}

#[test]
fn the_renames_stop_between_entries_once_shutdown_is_asked_for() {
    let f = folders();
    write(&f.collect.join("X/S/e01.mkv"), "a");
    write(&f.archive.join("X/S/old.mkv"), "old");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let err = move_entries(
        &f.collect.join("X"),
        &f.archive.join("X"),
        Side::Archive,
        &RealDisk,
        &cancel,
    )
    .unwrap_err();
    assert_eq!(err, MoveError::Stopped);
    assert_eq!(files(&f.collect), ["X/S/e01.mkv"]);
}

#[tokio::test]
async fn the_blocking_work_keeps_its_hold_after_the_waiting_task_is_aborted() {
    let hold: Hold = Arc::new(());
    let watch = Arc::downgrade(&hold);
    let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
    let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let waiting = tokio::spawn(async move {
        blocking(&hold, move || {
            started_tx.send(()).unwrap();
            go_rx.recv().unwrap();
            done_tx.send(()).unwrap();
            Ok(())
        })
        .await
    });
    tokio::task::spawn_blocking(move || started_rx.recv().unwrap())
        .await
        .unwrap();
    // The task waiting for the work is aborted (a shutdown past its grace).
    waiting.abort();
    let _ = waiting.await;
    assert!(
        watch.upgrade().is_some(),
        "the work still runs, so it still holds the lock"
    );
    go_tx.send(()).unwrap();
    tokio::task::spawn_blocking(move || done_rx.recv().unwrap())
        .await
        .unwrap();
    for _ in 0..100 {
        if watch.upgrade().is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the hold outlived the work");
}

/// A Transmission client for an address nothing listens on.
fn unreachable_transmission() -> TransClient {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    trss_transmission::client(
        format!("http://{addr}/transmission/rpc").parse().unwrap(),
        &trss_transmission::http_client(Duration::from_secs(2)).unwrap(),
    )
}

#[tokio::test]
async fn a_refused_move_changes_nothing_and_asks_transmission_nothing() {
    let f = folders();
    write(&f.collect.join("X/Season 02/e01.mkv"), "a");
    let before = files(&f.collect);
    let disk = Arc::new(SplitDisk {
        other: f.archive.clone(),
    });
    // Transmission cannot be reached: a check that refuses never asks it.
    let result = move_work_folder(
        &mut unreachable_transmission(),
        &Redactor::none(),
        &archive_request(&f, "X"),
        MovePolicy::default(),
        disk,
        Arc::new(()),
        &CancellationToken::new(),
    )
    .await;
    let Err(MoveError::Failed(reason)) = result else {
        panic!("{result:?}");
    };
    assert!(reason.contains("다른 파일시스템"), "{reason}");
    assert_eq!(files(&f.collect), before);
    assert!(files(&f.archive).is_empty());

    // Checks passed but Transmission is unreachable: nothing moves either.
    let result = move_work_folder(
        &mut unreachable_transmission(),
        &Redactor::none(),
        &archive_request(&f, "X"),
        MovePolicy::default(),
        Arc::new(RealDisk),
        Arc::new(()),
        &CancellationToken::new(),
    )
    .await;
    let Err(MoveError::Failed(reason)) = result else {
        panic!("{result:?}");
    };
    assert!(
        reason.contains("Transmission에 연결하지 못해서"),
        "{reason}"
    );
    assert_eq!(files(&f.collect), before);
    assert!(files(&f.archive).is_empty());
}

/// A Transmission client for the fake.
fn client_of(transmission: &FakeTransmission) -> TransClient {
    trss_transmission::client(
        transmission.url().parse().unwrap(),
        &trss_transmission::http_client(Duration::from_secs(2)).unwrap(),
    )
}

fn hash(n: u32) -> String {
    format!("dddd{n:036}")
}

/// A seeding torrent of a person (no bot label) with its single file in `dir`.
fn seeding(transmission: &FakeTransmission, n: u32, name: &str, dir: &Path) {
    write(&dir.join(name), "video");
    transmission.preload(FakeTorrent::new(&hash(n), name).in_dir(dir).status(6));
}

/// A wait for Transmission that ends after `timeout`.
fn waiting(timeout: Duration) -> MovePolicy {
    MovePolicy {
        poll: Duration::from_millis(10),
        timeout,
    }
}

/// The move of `request` against the fake, on the real disk.
async fn move_of(
    transmission: &FakeTransmission,
    request: &Request,
    policy: MovePolicy,
) -> Result<Moved, MoveError> {
    move_work_folder(
        &mut client_of(transmission),
        &Redactor::none(),
        request,
        policy,
        Arc::new(RealDisk),
        Arc::new(()),
        &CancellationToken::new(),
    )
    .await
}

#[tokio::test]
async fn a_transmission_that_refuses_the_move_leaves_everything_in_place() {
    let f = folders();
    let season = f.collect.join("Clevatess/Season 02");
    let transmission = FakeTransmission::start().await;
    seeding(&transmission, 1, "Clevatess S02E01.mkv", &season);
    transmission.reject_locations(Some("permission denied"));

    let result = move_of(
        &transmission,
        &archive_request(&f, "Clevatess"),
        waiting(Duration::from_secs(5)),
    )
    .await;

    let Err(MoveError::Failed(reason)) = result else {
        panic!("{result:?}");
    };
    assert!(reason.contains("permission denied"), "{reason}");
    assert_eq!(
        files(&f.collect),
        ["Clevatess/Season 02/Clevatess S02E01.mkv"]
    );
    assert!(files(&f.archive).is_empty());
}

#[tokio::test]
async fn every_torrent_in_the_work_folder_moves_with_its_files_whoever_added_it() {
    let f = folders();
    let transmission = FakeTransmission::start().await;
    let season = f.collect.join("Clevatess/Season 02");
    seeding(&transmission, 1, "Clevatess S02E01.mkv", &season);
    // A bot torrent too: every torrent in the folder moves, whoever added it.
    write(&season.join("Clevatess S02E02.mkv"), "video");
    transmission.preload(
        FakeTorrent::new(&hash(2), "Clevatess S02E02.mkv")
            .in_dir(&season)
            .bot()
            .status(6),
    );
    write(&f.collect.join("Clevatess/.trss/subs/S02E01.ass"), "sub");
    write(&season.join("notes.txt"), "not a torrent");
    // Another work that stays.
    seeding(
        &transmission,
        3,
        "Other S01E01.mkv",
        &f.collect.join("Other/Season 01"),
    );

    let moved = move_of(
        &transmission,
        &archive_request(&f, "Clevatess"),
        waiting(Duration::from_secs(5)),
    )
    .await;

    assert_eq!(moved, Ok(Moved::Moved));
    assert!(!f.collect.join("Clevatess").exists());
    assert_eq!(
        files(&f.archive.join("Clevatess")),
        [
            ".trss/subs/S02E01.ass",
            "Season 02/Clevatess S02E01.mkv",
            "Season 02/Clevatess S02E02.mkv",
            "Season 02/notes.txt",
        ]
    );
    assert_eq!(files(&f.collect), ["Other/Season 01/Other S01E01.mkv"]);
    // Transmission moved its torrents first, with their files, and keeps them.
    let new_dir = f.archive.join("Clevatess/Season 02");
    for n in [1, 2] {
        let torrent = transmission.torrent(&hash(n));
        assert_eq!(torrent.download_dir, new_dir.to_str().unwrap());
        assert_eq!(torrent.status, 6);
    }
    assert_eq!(
        transmission.torrent(&hash(3)).download_dir,
        f.collect.join("Other/Season 01").to_str().unwrap()
    );
    let moves = transmission.calls_of("torrent-set-location");
    assert_eq!(moves.len(), 2);
    for call in &moves {
        assert_eq!(call.args["move"], true);
        assert_eq!(call.args["location"], new_dir.to_str().unwrap());
    }
    assert!(transmission.calls_of("torrent-add").is_empty());
}

#[tokio::test]
async fn a_season_merges_beside_the_archived_one_and_new_folders_take_the_parents_owner() {
    let f = folders();
    let transmission = FakeTransmission::start().await;
    write(
        &f.archive.join("Clevatess/Season 01/Clevatess S01E01.mkv"),
        "old",
    );
    write(
        &f.archive.join("Clevatess/.trss/subs/S01E01.ass"),
        "old sub",
    );
    seeding(
        &transmission,
        1,
        "Clevatess S02E01.mkv",
        &f.collect.join("Clevatess/Season 02"),
    );
    write(&f.collect.join("Clevatess/Season 02/extra.txt"), "x");
    write(&f.collect.join("Clevatess/.trss/subs/S02E01.ass"), "sub");
    write(&f.collect.join("Clevatess/.trss/fonts/a.ttf"), "font");
    let fonts_inode = fs::metadata(f.collect.join("Clevatess/.trss/fonts"))
        .unwrap()
        .ino();

    let moved = move_of(
        &transmission,
        &archive_request(&f, "Clevatess"),
        waiting(Duration::from_secs(5)),
    )
    .await;

    assert_eq!(moved, Ok(Moved::Moved));
    assert!(
        !f.collect.join("Clevatess").exists(),
        "the emptied source is removed"
    );
    assert_eq!(
        files(&f.archive.join("Clevatess")),
        [
            ".trss/fonts/a.ttf",
            ".trss/subs/S01E01.ass",
            ".trss/subs/S02E01.ass",
            "Season 01/Clevatess S01E01.mkv",
            "Season 02/Clevatess S02E01.mkv",
            "Season 02/extra.txt",
        ]
    );
    // `Season 02` is new in the archive: Transmission made it for its torrent,
    // as its own user; the move makes no folder. Every folder here belongs to
    // the user running the tests, so this cannot tell owners apart: it only
    // holds the expected result in place.
    let parent = fs::metadata(f.archive.join("Clevatess")).unwrap();
    let season = fs::metadata(f.archive.join("Clevatess/Season 02")).unwrap();
    assert_eq!((season.uid(), season.gid()), (parent.uid(), parent.gid()));
    // What the move renamed is renamed, not copied: it keeps its inode, and so
    // its owner.
    assert_eq!(
        fs::metadata(f.archive.join("Clevatess/.trss/fonts"))
            .unwrap()
            .ino(),
        fonts_inode
    );
    assert_eq!(
        transmission.torrent(&hash(1)).download_dir,
        f.archive.join("Clevatess/Season 02").to_str().unwrap()
    );
}

#[tokio::test]
async fn a_move_stopped_during_the_renames_finishes_the_rest_when_it_runs_again() {
    let f = folders();
    let transmission = FakeTransmission::start().await;
    // The state a move that was stopped midway through the renames leaves:
    // the torrent at the archive, and the files split between the two folders.
    seeding(
        &transmission,
        1,
        "Clevatess S02E01.mkv",
        &f.archive.join("Clevatess/Season 02"),
    );
    write(
        &f.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv"),
        "moved",
    );
    write(
        &f.collect.join("Clevatess/Season 02/Clevatess S02E03.mkv"),
        "left",
    );
    write(&f.collect.join("Clevatess/.trss/subs/S02E01.ass"), "left");

    let moved = move_of(
        &transmission,
        &archive_request(&f, "Clevatess"),
        waiting(Duration::from_secs(5)),
    )
    .await;

    assert_eq!(moved, Ok(Moved::Moved));
    assert!(!f.collect.join("Clevatess").exists());
    assert_eq!(
        files(&f.archive),
        [
            "Clevatess/.trss/subs/S02E01.ass",
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/Clevatess S02E02.mkv",
            "Clevatess/Season 02/Clevatess S02E03.mkv",
        ]
    );
    assert!(transmission.calls_of("torrent-set-location").is_empty());
}

#[tokio::test]
async fn a_torrent_transmission_would_still_write_stops_the_move_whatever_its_state() {
    struct Case {
        what: &'static str,
        torrent: FakeTorrent,
        /// The work folder is not on disk at all: its torrent's data is in
        /// Transmission's incomplete folder, and the archive has a file of
        /// that name.
        folder_gone: bool,
        says: &'static [&'static str],
    }
    const UNFINISHED: &[&str] = &[
        "`Clevatess S02E02.mkv`",
        "다 받거나",
        "지운 뒤 `다시 옮기기`",
    ];
    const NO_METADATA: &[&str] = &["`Clevatess S02E03", "지운 뒤", "`다시 옮기기`"];
    let cases = [
        Case {
            what: "downloading",
            torrent: FakeTorrent::new(&hash(1), "Clevatess S02E02.mkv").unfinished(),
            folder_gone: false,
            says: UNFINISHED,
        },
        Case {
            what: "verifying",
            torrent: FakeTorrent::new(&hash(2), "Clevatess S02E02.mkv").status(2),
            folder_gone: false,
            says: UNFINISHED,
        },
        Case {
            what: "a magnet still fetching its metadata, stopped",
            torrent: FakeTorrent::new(&hash(1), "Clevatess S02E03")
                .without_metadata()
                .status(0),
            folder_gone: false,
            says: NO_METADATA,
        },
        Case {
            what: "a magnet still fetching its metadata, downloading",
            torrent: FakeTorrent::new(&hash(2), "Clevatess S02E03")
                .without_metadata()
                .status(4),
            folder_gone: false,
            says: NO_METADATA,
        },
        Case {
            what: "downloading, although it reports nothing left",
            torrent: FakeTorrent::new(&hash(3), "Clevatess S02E03.mkv").status(4),
            folder_gone: false,
            says: NO_METADATA,
        },
        Case {
            what: "downloading, with the work folder gone",
            torrent: FakeTorrent::new(&hash(1), "Clevatess S02E02.mkv").unfinished(),
            folder_gone: true,
            says: UNFINISHED,
        },
    ];
    for case in cases {
        let f = folders();
        let transmission = FakeTransmission::start().await;
        let season = f.collect.join("Clevatess/Season 02");
        let hash = case.torrent.hash.clone();
        if case.folder_gone {
            write(
                &f.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv"),
                "archived",
            );
        } else {
            seeding(&transmission, 9, "Clevatess S02E01.mkv", &season);
        }
        transmission.preload(case.torrent.in_dir(&season));
        let (collect_before, archive_before) = (files(&f.collect), files(&f.archive));

        let result = move_of(
            &transmission,
            &archive_request(&f, "Clevatess"),
            waiting(Duration::from_secs(5)),
        )
        .await;

        let Err(MoveError::Failed(reason)) = result else {
            panic!("{}: {result:?}", case.what);
        };
        for part in case.says {
            assert!(reason.contains(part), "{}: {reason}", case.what);
        }
        assert!(
            transmission.calls_of("torrent-set-location").is_empty(),
            "{}",
            case.what
        );
        assert_eq!(files(&f.collect), collect_before, "{}", case.what);
        assert_eq!(files(&f.archive), archive_before, "{}", case.what);
        if case.folder_gone {
            assert_eq!(
                transmission.torrent(&hash).download_dir,
                season.to_str().unwrap()
            );
            assert_eq!(
                fs::read_to_string(f.archive.join("Clevatess/Season 02/Clevatess S02E02.mkv"))
                    .unwrap(),
                "archived"
            );
        }
    }
}

#[tokio::test]
async fn the_renames_wait_until_transmission_reports_the_new_folder() {
    let f = folders();
    let transmission = FakeTransmission::start().await;
    seeding(
        &transmission,
        1,
        "Clevatess S02E01.mkv",
        &f.collect.join("Clevatess/Season 02"),
    );
    write(&f.collect.join("Clevatess/Season 02/notes.txt"), "x");
    // Transmission 4 answers first and moves the data in the background.
    transmission.async_locations(true);
    transmission.lag_locations(3);

    let moved = move_of(
        &transmission,
        &archive_request(&f, "Clevatess"),
        waiting(Duration::from_secs(5)),
    )
    .await;

    assert_eq!(moved, Ok(Moved::Moved));
    // One look before the move and the ones until the new folder showed.
    assert!(transmission.calls_of("torrent-get").len() >= 5);
    assert_eq!(
        transmission.torrent(&hash(1)).download_dir,
        f.archive.join("Clevatess/Season 02").to_str().unwrap()
    );
    assert!(!f.collect.join("Clevatess").exists());
}

#[tokio::test]
async fn a_move_error_transmission_reports_fails_the_move_with_its_reason_without_waiting() {
    let f = folders();
    let transmission = FakeTransmission::start().await;
    seeding(
        &transmission,
        1,
        "Clevatess S02E01.mkv",
        &f.collect.join("Clevatess/Season 02"),
    );
    write(&f.collect.join("Clevatess/Season 02/notes.txt"), "x");
    transmission.async_locations(true);
    transmission.fail_location_of(&hash(1), Some("Permission denied"));
    let policy = MovePolicy {
        poll: Duration::from_millis(20),
        timeout: Duration::from_secs(60),
    };

    let started = std::time::Instant::now();
    let result = move_of(&transmission, &archive_request(&f, "Clevatess"), policy).await;
    assert!(started.elapsed() < Duration::from_secs(20));

    let Err(MoveError::Failed(reason)) = result else {
        panic!("{result:?}");
    };
    assert!(reason.contains("Permission denied"), "{reason}");
    assert!(!reason.contains("정리"), "{reason}");
    // The move renamed nothing of its own.
    assert_eq!(
        files(&f.collect),
        [
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/notes.txt"
        ]
    );
    assert!(files(&f.archive).is_empty());
}

#[tokio::test]
async fn transmission_slower_than_the_wait_leaves_the_move_for_the_next_look() {
    let f = folders();
    let transmission = FakeTransmission::start().await;
    seeding(
        &transmission,
        1,
        "Clevatess S02E01.mkv",
        &f.collect.join("Clevatess/Season 02"),
    );
    write(&f.collect.join("Clevatess/Season 02/notes.txt"), "x");
    transmission.async_locations(true);
    transmission.lag_locations(u32::MAX);
    let request = archive_request(&f, "Clevatess");
    let slow = MovePolicy {
        poll: Duration::from_millis(20),
        timeout: Duration::from_millis(300),
    };

    let result = move_of(&transmission, &request, slow).await;

    assert!(matches!(result, Err(MoveError::Later(_))), "{result:?}");
    // The move renamed nothing of its own meanwhile.
    assert_eq!(
        files(&f.collect),
        [
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/notes.txt"
        ]
    );

    // Transmission catches up; the next look finishes the same move.
    transmission.settle_locations();
    transmission.lag_locations(0);
    let moved = move_of(&transmission, &request, slow).await;
    assert_eq!(moved, Ok(Moved::Moved));
    assert!(!f.collect.join("Clevatess").exists());
    assert_eq!(
        files(&f.archive),
        [
            "Clevatess/Season 02/Clevatess S02E01.mkv",
            "Clevatess/Season 02/notes.txt",
        ]
    );
}

#[tokio::test]
async fn a_refusal_after_some_torrents_moved_says_so_and_moving_again_finishes() {
    let f = folders();
    let transmission = FakeTransmission::start().await;
    let season = f.collect.join("Clevatess/Season 02");
    seeding(&transmission, 1, "Clevatess S02E01.mkv", &season);
    seeding(&transmission, 2, "Clevatess S02E02.mkv", &season);
    transmission.reject_location_of(&hash(2), Some("torrent is busy"));
    let request = archive_request(&f, "Clevatess");

    let result = move_of(&transmission, &request, waiting(Duration::from_secs(5))).await;

    let Err(MoveError::Failed(reason)) = result else {
        panic!("{result:?}");
    };
    assert!(reason.contains("torrent is busy"), "{reason}");
    assert!(reason.contains("1개"), "{reason}");
    assert!(reason.contains("다시 옮기면 남은 것만"), "{reason}");
    assert!(!reason.contains("정리"), "{reason}");

    transmission.reject_location_of(&hash(2), None);
    let moved = move_of(&transmission, &request, waiting(Duration::from_secs(5))).await;
    assert_eq!(moved, Ok(Moved::Moved));
    assert!(!f.collect.join("Clevatess").exists());
    for n in [1, 2] {
        assert_eq!(
            transmission.torrent(&hash(n)).download_dir,
            f.archive.join("Clevatess/Season 02").to_str().unwrap()
        );
    }
}
