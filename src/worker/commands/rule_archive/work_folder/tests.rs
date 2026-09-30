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
        files: vec![crate::transmission::TorrentFile {
            name: name.into(),
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

    // Through a link into the work folder: moved, to where it really is.
    let moves = plan_torrents(
        &request,
        &[
            place("a", "e01.mkv", &f.collect.join("Alias/Season 02")),
            place("b", "o01.mkv", &f.collect.join("Other/Season 01")),
        ],
    )
    .unwrap();
    assert_eq!(moves.len(), 1, "{moves:?}");
    assert_eq!(moves[0].hash, "a");
    assert_eq!(moves[0].to, f.archive.join("X/Season 02"));

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
    assert!(err.contains("다 받은 뒤"), "{err}");

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

    // A complete file found only at the destination is the torrent's own,
    // moved by an earlier start or by hand.
    write(&to.join("e03.mkv"), "moved");
    let moves = plan_torrents(&request, &[place("c", "e03.mkv", &from)]).unwrap();
    assert_eq!(moves[0].to, to);
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
