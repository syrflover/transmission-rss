//! The move's checks and renames on real temporary folders. The Transmission
//! step, the command around it and the interplay with the cycle are tested end
//! to end in `tests/archive_move.rs`, against a fake Transmission.

use std::{
    fs,
    os::unix::fs::{symlink, MetadataExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use tokio_util::sync::CancellationToken;

use super::*;

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

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
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

/// Every file below `root`, relative, sorted.
fn files(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                walk(&path, root, out);
            } else {
                out.push(path.strip_prefix(root).unwrap().display().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
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
    assert!(move_entries(&f.collect.join("X"), &f.archive.join("X"), Side::Archive).unwrap());
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
    assert!(move_entries(&f.collect.join("X"), &f.archive.join("X"), Side::Archive).unwrap());
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
    assert!(!move_entries(&f.collect.join("X"), &f.archive.join("X"), Side::Archive).unwrap());
}

#[test]
fn a_file_that_appeared_at_the_destination_is_left_at_the_source_and_named() {
    let f = folders();
    write(&f.collect.join("X/S/e01.mkv"), "mine");
    write(&f.collect.join("X/S/e02.mkv"), "mine");
    write(&f.archive.join("X/S/e01.mkv"), "theirs");
    let err = move_entries(&f.collect.join("X"), &f.archive.join("X"), Side::Archive).unwrap_err();
    assert!(err.contains("`S/e01.mkv`"), "{err}");
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
    assert!(move_entries(&f.collect.join("X"), &f.archive.join("X"), Side::Archive).unwrap());
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
    let place = |dir: &str, name: &str| TorrentPlace {
        hash: "h".into(),
        name: name.into(),
        download_dir: dir.into(),
    };
    assert_eq!(
        new_location(
            &place("/downloads/Shows (current)/Clevatess/Season 02", "e.mkv"),
            &request
        ),
        Some(PathBuf::from("/downloads/Shows/Clevatess/Season 02"))
    );
    assert_eq!(
        new_location(
            &place("/downloads/Shows (current)/Clevatess/", "e.mkv"),
            &request
        ),
        Some(PathBuf::from("/downloads/Shows/Clevatess"))
    );
    // A torrent whose own folder is the work folder, right in the collect folder.
    assert_eq!(
        new_location(&place("/downloads/Shows (current)", "Clevatess"), &request),
        Some(PathBuf::from("/downloads/Shows"))
    );
    for (dir, name) in [
        ("/downloads/Shows (current)/Clevatess2/Season 01", "e.mkv"),
        ("/downloads/Shows (current)", "Clevatess S02E01.mkv"),
        ("/downloads/Shows (current)/Other", "Clevatess"),
        ("/downloads/downloads", "e.mkv"),
    ] {
        assert_eq!(
            new_location(&place(dir, name), &request),
            None,
            "{dir} {name}"
        );
    }
}

/// A Transmission client for an address nothing listens on.
fn unreachable_transmission() -> TransClient {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    crate::transmission::client(
        format!("http://{addr}/transmission/rpc").parse().unwrap(),
        &crate::transmission::http_client(Duration::from_secs(2)).unwrap(),
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
