//! Uploads (`trss_jobs::upload`) against the records and the receive area:
//! the job they make is `done` and nobody else touches it (one that kept an
//! archive waits for the worker to unpack it), a refused or abandoned upload
//! leaves no byte, and uploads wait for their turn.

use std::time::Duration;

use trss_core::Db;
use trss_jobs::{
    upload::{Counts, Limits, UploadError, UploadRequest, UPLOAD_SLOTS},
    Finished, JobState, JobStore, ReceiveArea, Uploads,
};

const ASS: &[u8] = b"[Script Info]\nTitle: x\n";

struct Setup {
    _dir: tempfile::TempDir,
    store: JobStore,
    uploads: Uploads,
    area: ReceiveArea,
}

async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let store = JobStore::new(db);
    let area = ReceiveArea::in_app_data(dir.path());
    let uploads = Uploads::new(store.clone(), area.clone());
    Setup {
        _dir: dir,
        store,
        uploads,
        area,
    }
}

fn request(id: &str) -> UploadRequest {
    UploadRequest {
        command_id: id.to_owned(),
        work_id: "w1".to_owned(),
        season: 1,
        anime_no: None,
        source_id: None,
        creator: None,
    }
}

async fn stage(uploads: &Uploads, files: &[(&str, &[u8])]) -> trss_jobs::upload::Staging {
    let mut staging = uploads.begin().await.unwrap();
    for (name, bytes) in files {
        staging.start(name).await.unwrap();
        // In pieces, as a body comes.
        for piece in bytes.chunks(7) {
            staging.write(piece).await.unwrap();
        }
        staging.end().await.unwrap();
    }
    staging
}

fn tmp_entries(area: &ReceiveArea) -> usize {
    std::fs::read_dir(area.at(".tmp")).map_or(0, |d| d.count())
}

#[tokio::test]
async fn an_upload_is_a_done_job_that_nothing_picks_up_again() {
    let s = setup().await;
    let staging = stage(&s.uploads, &[("01.ass", ASS), ("note.txt", b"hello")]).await;
    let made = s
        .uploads
        .finish(staging, request("u1"), 5_000)
        .await
        .unwrap();
    let Finished::Created {
        job_id,
        counts,
        dropped,
    } = made
    else {
        panic!("{made:?}");
    };
    assert_eq!(
        counts,
        Counts {
            subtitles: 1,
            fonts: 0,
            archives: 0
        }
    );
    assert_eq!(dropped.len(), 1);

    // Nothing is waiting for a worker, and a restart's look at the waiting
    // jobs leaves it as it is.
    assert!(!s.store.has_ready().await.unwrap());
    assert_eq!(s.store.claim_next(6_000).await.unwrap(), None);
    assert_eq!(s.store.requeue_waiting_for_sources(6_000).await.unwrap(), 0);
    let detail = s.store.detail(&job_id).await.unwrap().unwrap();
    assert_eq!(detail.row.state, JobState::Done);
    assert_eq!(detail.row.finished_at, Some(5_000));
    assert_eq!(detail.row.origin, "upload");
    assert_eq!(detail.row.episodes, Vec::<String>::new());
    assert_eq!(detail.row.source, None);
    assert_eq!(detail.dropped.len(), 1);
    assert_eq!(detail.items.len(), 1);
    let file = &detail.items[0].files[0];
    assert_eq!(file.size, Some(ASS.len() as u64));
    assert_eq!(
        std::fs::read(s.area.at(file.path.as_deref().unwrap())).unwrap(),
        ASS
    );
    // The SHA-256 is of the bytes that came.
    let (size, sha256, object) =
        trss_jobs::area::read_facts(&s.area.at(file.path.as_deref().unwrap())).unwrap();
    assert_eq!(
        (Some(size), file.sha256.as_deref()),
        (file.size, Some(sha256.as_str()))
    );
    assert_eq!(file.object.as_deref(), Some(object.as_str()));
    assert_eq!(tmp_entries(&s.area), 0);
}

#[tokio::test]
async fn an_upload_dropped_halfway_leaves_nothing_and_gives_its_turn_back() {
    let s = setup().await;
    let mut held = Vec::new();
    for _ in 0..UPLOAD_SLOTS {
        held.push(s.uploads.begin().await.unwrap());
    }
    // A third waits for a turn.
    let waiting = {
        let uploads = s.uploads.clone();
        tokio::spawn(async move { uploads.begin().await.map(|_| ()) })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!waiting.is_finished());

    // The first is dropped in the middle of a file.
    let mut first = held.remove(0);
    first.start("half.ass").await.unwrap();
    first.write(b"[Script").await.unwrap();
    assert_eq!(tmp_entries(&s.area), UPLOAD_SLOTS);
    drop(first);
    waiting.await.unwrap().unwrap();
    // What is left is the other upload's folder, and the third's was dropped.
    assert_eq!(tmp_entries(&s.area), 1);
    drop(held);
    assert_eq!(tmp_entries(&s.area), 0);
}

#[tokio::test]
async fn a_repeat_or_another_upload_under_the_same_id_stores_no_second_package() {
    let s = setup().await;
    let first = stage(&s.uploads, &[("01.ass", ASS)]).await;
    let Finished::Created { job_id, .. } =
        s.uploads.finish(first, request("same"), 1).await.unwrap()
    else {
        panic!("not created");
    };
    let again = stage(&s.uploads, &[("01.ass", ASS)]).await;
    assert_eq!(
        s.uploads.finish(again, request("same"), 2).await.unwrap(),
        Finished::Existing(job_id.clone())
    );
    let other = stage(&s.uploads, &[("01.ass", b"[Script Info]\nother")]).await;
    assert_eq!(
        s.uploads.finish(other, request("same"), 3).await.unwrap(),
        Finished::Mismatch(job_id.clone())
    );
    let folders: Vec<_> = std::fs::read_dir(s.area.root())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != ".tmp")
        .collect();
    assert_eq!(folders, std::slice::from_ref(&job_id));
    assert_eq!(tmp_entries(&s.area), 0);
    // The store itself answers the same.
    assert_eq!(s.store.done_page(None, 10).await.unwrap().total, 1);
}

#[tokio::test]
async fn the_limits_are_held_as_the_bytes_come() {
    let s = setup().await;
    let tight = s.uploads.clone().with_limits(Limits {
        files: 2,
        entries: 3,
        total_bytes: 20,
        file_bytes: 12,
        ..Limits::default()
    });
    let mut staging = tight.begin().await.unwrap();
    staging.start("a.srt").await.unwrap();
    assert!(matches!(
        staging.write(&[b'x'; 13]).await,
        Err(UploadError::FileTooLarge(12))
    ));
    drop(staging);

    let mut staging = tight.begin().await.unwrap();
    for name in ["a.srt", "b.srt"] {
        staging.start(name).await.unwrap();
        staging.write(&[b'x'; 10]).await.unwrap();
    }
    // 20 bytes are in: one more is the total's.
    assert!(matches!(
        staging.write(&[b'x'; 1]).await,
        Err(UploadError::TotalTooLarge(20))
    ));
    assert!(matches!(
        staging.start("c.srt").await,
        Err(UploadError::TooManyFiles(2))
    ));
    drop(staging);

    let mut staging = tight.begin().await.unwrap();
    staging.skip("a").unwrap();
    staging.skip("b").unwrap();
    staging.skip("c").unwrap();
    assert!(matches!(
        staging.skip("d"),
        Err(UploadError::TooManyEntries(3))
    ));
    assert!(matches!(
        staging.start("e.srt").await,
        Err(UploadError::TooManyEntries(3))
    ));
    drop(staging);
    assert_eq!(tmp_entries(&s.area), 0);
}

fn dropped_reasons(made: &Finished) -> Vec<(String, String)> {
    match made {
        Finished::Created { dropped, .. } | Finished::Nothing { dropped } => dropped
            .iter()
            .map(|d| (d.name.clone(), d.reason.clone()))
            .collect(),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn finishing_goes_on_when_the_request_is_dropped_and_leaves_no_orphan() {
    let s = setup().await;
    let staging = stage(&s.uploads, &[("01.ass", ASS)]).await;
    // The caller is dropped as soon as it has been polled once.
    let finishing = s.uploads.finish(staging, request("gone"), 1_000);
    let _ = tokio::time::timeout(Duration::from_nanos(1), finishing).await;
    // The task that was spawned runs to its end by itself.
    let mut done = None;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if let Some(job) = s.store.done_page(None, 10).await.unwrap().items.first() {
            done = Some(job.id.clone());
            break;
        }
    }
    let job_id = done.expect("the job was recorded although nobody waited for it");
    let folders: Vec<_> = std::fs::read_dir(s.area.root())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != ".tmp")
        .collect();
    assert_eq!(folders, std::slice::from_ref(&job_id));
    assert_eq!(tmp_entries(&s.area), 0);
}

fn make_old(path: &std::path::Path) {
    let old = std::time::SystemTime::now() - Duration::from_secs(3 * 3600);
    std::fs::File::open(path)
        .unwrap()
        .set_modified(old)
        .unwrap();
}

#[tokio::test]
async fn the_sweep_removes_only_old_orphans_of_a_uuid_name() {
    let s = setup().await;
    // A job with its folder, which must stay.
    let staging = stage(&s.uploads, &[("01.ass", ASS)]).await;
    let Finished::Created { job_id, .. } = s
        .uploads
        .finish(staging, request("kept"), 1_000)
        .await
        .unwrap()
    else {
        panic!()
    };
    let made = |rel: &str| {
        let dir = s.area.at(rel);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.part"), b"x").unwrap();
        dir
    };
    let orphan_job = made("4f0c2f6e-1c1f-4a52-9a43-0a7b8f1d2c01");
    let orphan_tmp = made(".tmp/6a1d7d88-3f0e-4a7f-8d57-2b6c4c0e9f02");
    let young_job = made("0b9e1b5c-6a7d-4f29-8c3e-5d1f7a2b4c03");
    let young_tmp = made(".tmp/9c2e4d6f-8a0b-4c1d-9e3f-7a5b1c2d4e04");
    // Names that are not what an upload makes are not touched.
    let other = made("covers");
    let auto = made("auto:s1:12");
    // A receipt's attempt folder has the file record's ID, so it stays.
    let attempt_id = "1e2d3c4b-5a69-4788-9a0b-1c2d3e4f5a06";
    let attempt = made(&format!(".tmp/{attempt_id}"));
    for old in [&orphan_job, &orphan_tmp, &other, &auto, &attempt] {
        make_old(old);
    }
    make_old(&s.area.at(&job_id));
    s.store
        .db()
        .run::<_, trss_core::DbError, _>(move |c| {
            c.execute_batch(&format!(
                "INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state, created_at, updated_at)
                 SELECT '{attempt_id}', job_id, id, 'k', 'n', 'intended', 1, 1 FROM subtitle_job_items LIMIT 1"
            ))?;
            Ok(())
        })
        .await
        .unwrap();

    let removed = s.uploads.sweep(Duration::from_secs(3600)).await.unwrap();
    assert_eq!(removed, 2);
    for gone in [&orphan_job, &orphan_tmp] {
        assert!(!gone.exists(), "{gone:?}");
    }
    for stays in [
        &young_job,
        &young_tmp,
        &other,
        &auto,
        &attempt,
        &s.area.at(&job_id),
    ] {
        assert!(stays.exists(), "{stays:?}");
    }
    // A second pass finds nothing more, and a missing area is no error.
    assert_eq!(s.uploads.sweep(Duration::from_secs(3600)).await.unwrap(), 0);
    let none = Uploads::new(s.store.clone(), ReceiveArea::new("/nonexistent/receive"));
    assert_eq!(none.sweep(Duration::from_secs(1)).await.unwrap(), 0);
}

#[tokio::test]
async fn the_sweep_removes_an_old_winpng_staging_folder_and_keeps_a_young_one() {
    let s = setup().await;
    let made = |rel: &str| {
        let dir = s.area.at(rel);
        std::fs::create_dir_all(dir.join("1")).unwrap();
        std::fs::write(dir.join("1/01.smi"), b"x").unwrap();
        dir
    };
    let job = "4f0c2f6e-1c1f-4a52-9a43-0a7b8f1d2c01";
    let old = made(&format!(".tmp/winpng-{job}"));
    let young = made(".tmp/winpng-6a1d7d88-3f0e-4a7f-8d57-2b6c4c0e9f02");
    // Not what a reading makes: an old folder of another name stays.
    let other = made(".tmp/winpng-notes");
    let elsewhere = made("winpng-0b9e1b5c-6a7d-4f29-8c3e-5d1f7a2b4c03");
    for dir in [&old, &other, &elsewhere] {
        make_old(dir);
    }
    let removed = s.uploads.sweep(Duration::from_secs(3600)).await.unwrap();
    assert_eq!(removed, 1);
    assert!(!old.exists());
    for stays in [&young, &other, &elsewhere] {
        assert!(stays.exists(), "{stays:?}");
    }
}

#[tokio::test]
async fn the_sweep_removes_an_old_check_folder_whose_item_settled_and_keeps_an_open_items() {
    use trss_jobs::{Created, ItemState, NewItem, NewJob};
    let s = setup().await;
    let job = NewJob {
        command_id: "c1".to_owned(),
        request: "{}".to_owned(),
        origin: "pick".to_owned(),
        work_id: None,
        season: Some(1),
        anime_no: None,
        source_id: None,
        creator: None,
        revision_of: None,
        revises_attributed: false,
        items: ["1", "2"]
            .iter()
            .map(|ep| NewItem {
                observation_id: None,
                episode: (*ep).to_owned(),
                post_url: format!("https://fake.trss.invalid/check/ep{ep}"),
                found_at: 500,
            })
            .collect(),
    };
    let Created::Created(job) = s.store.create(job, 900).await.unwrap() else {
        panic!("not created");
    };
    let items = s.store.items(&job).await.unwrap();
    let (open, settled) = (items[0].id, items[1].id);
    s.store
        .set_item(settled, ItemState::Failed, None, None, 1_000)
        .await
        .unwrap();
    let made = |rel: &str| {
        let dir = s.area.at(rel);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ep.srt"), b"x").unwrap();
        dir
    };
    // The item's next run takes the file of an open item, however old.
    let kept = made(&format!(".tmp/check-{job}-{open}"));
    let gone = made(&format!(".tmp/check-{job}-{settled}"));
    let young = made(&format!(".tmp/check-{job}-{}", settled + 100));
    let other = made(".tmp/check-notes-1");
    for dir in [&kept, &gone, &other] {
        make_old(dir);
    }
    assert_eq!(s.uploads.sweep(Duration::from_secs(3600)).await.unwrap(), 1);
    assert!(!gone.exists());
    for stays in [&kept, &young, &other] {
        assert!(stays.exists(), "{stays:?}");
    }
}

#[tokio::test]
async fn the_sweep_goes_on_in_the_background_and_catches_an_orphan_made_later() {
    use std::sync::{Arc, Mutex};
    let s = setup().await;
    let reports: Arc<Mutex<Vec<usize>>> = Arc::default();
    let seen = Arc::clone(&reports);
    let task = s.uploads.keep_sweeping(
        Duration::from_millis(40),
        Duration::from_secs(3600),
        move |result| seen.lock().unwrap().push(result.unwrap()),
    );
    // The first pass is at once and finds nothing (nothing is reported).
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(reports.lock().unwrap().is_empty());

    // An orphan made after the task started, and a young one, which stays.
    let made = |rel: &str| {
        let dir = s.area.at(rel);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.part"), b"x").unwrap();
        dir
    };
    let orphan = made(".tmp/4f0c2f6e-1c1f-4a52-9a43-0a7b8f1d2c01");
    let young = made(".tmp/0b9e1b5c-6a7d-4f29-8c3e-5d1f7a2b4c03");
    make_old(&orphan);
    // The pass reports after it removes the folder, so wait for the report.
    for _ in 0..100 {
        if !reports.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!orphan.exists());
    assert!(young.exists());
    assert_eq!(*reports.lock().unwrap(), [1]);

    // Aborting the handle ends it: a later orphan stays.
    task.abort();
    let _ = task.await;
    let later = made("6a1d7d88-3f0e-4a7f-8d57-2b6c4c0e9f02");
    make_old(&later);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(later.exists());
}

#[tokio::test]
async fn waiting_for_a_turn_is_bounded_in_number_and_in_time() {
    let s = setup().await;
    let uploads = s.uploads.clone().with_limits(Limits {
        queue: 2,
        queue_wait: Duration::from_millis(300),
        ..Limits::default()
    });
    let mut held = Vec::new();
    for _ in 0..UPLOAD_SLOTS {
        held.push(uploads.begin().await.unwrap());
    }
    let waiter = |uploads: Uploads| tokio::spawn(async move { uploads.begin().await.map(|_| ()) });
    let (a, b) = (waiter(uploads.clone()), waiter(uploads.clone()));
    tokio::time::sleep(Duration::from_millis(100)).await;
    // The queue holds two: a third is told at once.
    let started = std::time::Instant::now();
    assert!(matches!(uploads.begin().await, Err(UploadError::Busy)));
    assert!(started.elapsed() < Duration::from_millis(200));
    // Those that wait give up when their time is out, and the count goes down.
    assert!(matches!(a.await.unwrap(), Err(UploadError::Busy)));
    assert!(matches!(b.await.unwrap(), Err(UploadError::Busy)));
    drop(held.pop());
    assert!(uploads.begin().await.is_ok());
}

fn program_stream() -> Vec<u8> {
    let mut bytes = b"\x00\x00\x01\xBA\x44\x00\x04\x00\x04\x01".to_vec();
    bytes.resize(64, 0);
    bytes
}

const IDX: &[u8] = b"# VobSub index file, v7 (do not modify this line!)\nlangidx: 0\n";

#[tokio::test]
async fn a_program_stream_is_vobsub_only_with_its_index_in_the_same_upload() {
    let s = setup().await;
    let ps = program_stream();
    // A video alone is dropped, with the reason.
    let staging = stage(&s.uploads, &[("movie.mpg", &ps)]).await;
    let made = s
        .uploads
        .finish(staging, request("v1"), 1_000)
        .await
        .unwrap();
    assert!(matches!(made, Finished::Nothing { .. }));
    assert_eq!(
        dropped_reasons(&made),
        [(
            "movie.mpg".to_owned(),
            trss_jobs::upload::NO_INDEX_REASON.to_owned()
        )]
    );
    // An index of another name does not vouch for it, one of its name does
    // (whatever the case and the folder it came in).
    let staging = stage(
        &s.uploads,
        &[
            ("Show/Ep1.SUB", &ps),
            ("Show/Ep1.IDX", IDX),
            ("Show/Ep2.sub", &ps),
            ("Show/Other.idx", IDX),
            ("Show/Ep3.sub", &ps),
            ("Show/Ep3.idx", b"not an index"),
        ],
    )
    .await;
    let made = s
        .uploads
        .finish(staging, request("v2"), 2_000)
        .await
        .unwrap();
    let Finished::Created { counts, .. } = &made else {
        panic!("{made:?}");
    };
    assert_eq!(counts.subtitles, 3);
    let dropped: Vec<_> = dropped_reasons(&made).into_iter().map(|(n, _)| n).collect();
    assert_eq!(dropped, ["Show/Ep2.sub", "Show/Ep3.sub", "Show/Ep3.idx"]);
}

#[tokio::test]
async fn the_zips_of_one_upload_share_one_inflation_budget() {
    let s = setup().await;
    let uploads = s.uploads.clone().with_limits(Limits {
        inflate_budget: 100 * 1024,
        ..Limits::default()
    });
    let body = vec![b'a'; 64 * 1024];
    let zip = trss_subtitles::verify::zip_of(&[("a.srt", &body)]);
    let staging = stage(
        &uploads,
        &[("1.zip", &zip), ("2.zip", &zip), ("3.zip", &zip)],
    )
    .await;
    let made = uploads.finish(staging, request("z1"), 1_000).await.unwrap();
    let Finished::Created { counts, .. } = &made else {
        panic!("{made:?}");
    };
    // The first is read in full; the second finds 36 KiB left and the third none.
    assert_eq!(counts.archives, 1);
    let reasons = dropped_reasons(&made);
    assert_eq!(reasons.len(), 2);
    for (_, reason) in reasons {
        assert_eq!(reason, trss_subtitles::verify::INFLATE_BUDGET_SPENT);
    }
}

#[tokio::test]
async fn a_name_left_out_by_the_browser_is_recorded_with_the_reason() {
    let s = setup().await;
    let mut staging = s.uploads.begin().await.unwrap();
    staging.skip("readme.txt").unwrap();
    staging.start("01.ass").await.unwrap();
    staging.write(ASS).await.unwrap();
    staging.end().await.unwrap();
    let made = s
        .uploads
        .finish(staging, request("n1"), 1_000)
        .await
        .unwrap();
    assert_eq!(
        dropped_reasons(&made),
        [(
            "readme.txt".to_owned(),
            trss_jobs::upload::SKIPPED_REASON.to_owned()
        )]
    );
}

#[tokio::test]
async fn archives_of_every_format_are_kept_whole_and_one_that_is_none_is_dropped() {
    use trss_subtitles::upload::Archive;
    let s = setup().await;
    let mut tar = b"readme.txt".to_vec();
    tar.resize(300, 0);
    tar[257..262].copy_from_slice(b"ustar");
    let zip = trss_subtitles::verify::zip_of(&[(
        "a.srt",
        b"1\r\n00:00:01,000 --> 00:00:02,000\r\nHi\r\n",
    )]);
    let files: Vec<(&str, Vec<u8>, Archive)> = vec![
        ("a.rar", b"Rar!\x1A\x07\x00 volume".to_vec(), Archive::Rar),
        (
            "b.part2.rar",
            b"Rar!\x1A\x07\x01\x00 volume".to_vec(),
            Archive::Rar,
        ),
        (
            "c.7z.001",
            b"7z\xBC\xAF\x27\x1C\x00\x04 volume".to_vec(),
            Archive::SevenZip,
        ),
        ("d.tgz", b"\x1F\x8B\x08\x00 gz".to_vec(), Archive::Gzip),
        ("e.bz2", b"BZh9 bz".to_vec(), Archive::Bzip2),
        ("f.xz", b"\xFD7zXZ\x00 xz".to_vec(), Archive::Xz),
        ("g.tar", tar, Archive::Tar),
        ("h.zip", zip, Archive::Zip),
    ];
    let mut staging = s.uploads.begin().await.unwrap();
    for (name, bytes, _) in &files {
        staging.start(name).await.unwrap();
        staging.write(bytes).await.unwrap();
        staging.end().await.unwrap();
    }
    // Named as archives, but their content is not.
    for (name, bytes) in [
        ("fake.rar", &b"just text"[..]),
        ("fake.zip", b"just text"),
        ("pic.7z", b"\x89PNG\r\n\x1a\n"),
    ] {
        staging.start(name).await.unwrap();
        staging.write(bytes).await.unwrap();
        staging.end().await.unwrap();
    }
    let made = s
        .uploads
        .finish(staging, request("arc"), 1_000)
        .await
        .unwrap();
    let Finished::Created { job_id, counts, .. } = &made else {
        panic!("{made:?}");
    };
    assert_eq!((counts.archives, counts.subtitles, counts.fonts), (8, 0, 0));
    assert_eq!(
        dropped_reasons(&made),
        ["fake.rar", "fake.zip", "pic.7z"].map(|n| (
            n.to_owned(),
            trss_subtitles::upload::NOT_AN_ARCHIVE.to_owned()
        ))
    );
    // Each is stored byte for byte and records its format.
    let detail = s.store.detail(job_id).await.unwrap().unwrap();
    let stored = &detail.items[0].files;
    assert_eq!(stored.len(), 8);
    for ((name, bytes, archive), file) in files.iter().zip(stored) {
        assert_eq!(&file.name, name);
        assert_eq!(file.archive, Some(*archive), "{name}");
        assert_eq!(file.kind, Some(trss_subtitles::upload::Kind::Archive));
        assert_eq!(
            std::fs::read(s.area.at(file.path.as_deref().unwrap())).unwrap(),
            *bytes
        );
    }
    assert_eq!(
        stored[0].format,
        Some(trss_subtitles::verify::Format::Other)
    );
    assert_eq!(stored[7].format, Some(trss_subtitles::verify::Format::Zip));
    // The archives are the worker's to unpack: the job waits for it.
    assert_eq!(detail.row.state, JobState::Pending);
    assert_eq!(detail.row.finished_at, None);
    assert_eq!(
        s.store.claim_next(2_000).await.unwrap(),
        Some((job_id.clone(), false, 1))
    );
}

#[tokio::test]
async fn only_an_upload_has_items_without_an_episode() {
    use trss_jobs::{Created, NewItem, NewJob};
    let s = setup().await;
    // A picked candidate whose episode Anissia wrote as an empty text keeps it.
    let made = s
        .store
        .create(
            NewJob {
                command_id: "pick1".to_owned(),
                request: "{}".to_owned(),
                origin: "pick".to_owned(),
                work_id: None,
                season: Some(1),
                anime_no: None,
                source_id: None,
                creator: None,
                revision_of: None,
                revises_attributed: false,
                items: vec![NewItem {
                    observation_id: None,
                    episode: String::new(),
                    post_url: "https://example.test/p".to_owned(),
                    found_at: 1,
                }],
            },
            1_000,
        )
        .await
        .unwrap();
    let Created::Created(id) = made else {
        panic!("{made:?}")
    };
    let row = s.store.detail(&id).await.unwrap().unwrap().row;
    assert_eq!(row.episodes, [String::new()]);
}

async fn upload_of(s: &Setup, id: &str, files: &[(&str, Vec<u8>)]) -> Finished {
    let mut staging = s.uploads.begin().await.unwrap();
    for (name, bytes) in files {
        staging.start(name).await.unwrap();
        staging.write(bytes).await.unwrap();
        staging.end().await.unwrap();
    }
    s.uploads.finish(staging, request(id), 1_000).await.unwrap()
}

/// The archive type of each file of the job, by name.
async fn archive_types(
    s: &Setup,
    made: &Finished,
) -> Vec<(String, Option<trss_subtitles::upload::Archive>)> {
    let Finished::Created { job_id, .. } = made else {
        panic!("{made:?}");
    };
    let detail = s.store.detail(job_id).await.unwrap().unwrap();
    detail.items[0]
        .files
        .iter()
        .map(|f| (f.name.clone(), f.archive))
        .collect()
}

#[tokio::test]
async fn the_volumes_of_a_split_archive_are_kept_with_the_volume_that_has_the_magic() {
    use trss_subtitles::upload::{Archive, NO_FIRST_VOLUME};
    let s = setup().await;
    let volume = |tag: u8| vec![tag; 300];

    // A 7z of three volumes: the first has the magic, the others none. The
    // volumes may come in any order and under any case, in a folder.
    let made = upload_of(
        &s,
        "v7z",
        &[
            ("Show/Pack.7z.003", volume(3)),
            (
                "Show/pack.7z.001",
                [&b"7z\xBC\xAF\x27\x1C\x00\x04"[..], &volume(1)].concat(),
            ),
            ("Show/PACK.7Z.002", volume(2)),
        ],
    )
    .await;
    let Finished::Created {
        counts, dropped, ..
    } = &made
    else {
        panic!("{made:?}");
    };
    assert_eq!((counts.archives, dropped.len()), (3, 0));
    assert_eq!(
        archive_types(&s, &made).await,
        [
            ("Show/Pack.7z.003".to_owned(), Some(Archive::SevenZip)),
            ("Show/pack.7z.001".to_owned(), Some(Archive::SevenZip)),
            ("Show/PACK.7Z.002".to_owned(), Some(Archive::SevenZip)),
        ]
    );

    // An old-style RAR set: `.rar` has the magic, `.r00` and `.r01` none.
    let made = upload_of(
        &s,
        "vrar",
        &[
            ("old.rar", [&b"Rar!\x1A\x07\x00"[..], &volume(1)].concat()),
            ("old.r00", volume(2)),
            ("old.r01", volume(3)),
        ],
    )
    .await;
    assert!(
        matches!(&made, Finished::Created { counts, .. } if counts.archives == 3),
        "{made:?}"
    );
    assert!(archive_types(&s, &made)
        .await
        .iter()
        .all(|(_, a)| *a == Some(Archive::Rar)));

    // A spanned ZIP: the last part `.zip` holds the end records but cannot be
    // read alone, and the volumes `.z01` and `.z02` start with a spanning mark.
    let whole = trss_subtitles::verify::zip_of(&[("a.srt", &vec![b'a'; 4096])]);
    let made = upload_of(
        &s,
        "vzip",
        &[
            ("span.z01", [&b"PK\x07\x08"[..], &volume(1)].concat()),
            ("span.z02", volume(2)),
            ("span.zip", whole[..whole.len() / 2].to_vec()),
        ],
    )
    .await;
    assert!(
        matches!(&made, Finished::Created { counts, dropped, .. } if counts.archives == 3 && dropped.is_empty()),
        "{made:?}"
    );
    assert!(archive_types(&s, &made)
        .await
        .iter()
        .all(|(_, a)| *a == Some(Archive::Zip)));

    // `.zip.001` is itself PK-signed but spans, and its `.002` has no magic.
    let made = upload_of(
        &s,
        "vzip2",
        &[
            ("cut.zip.001", whole[..whole.len() / 2].to_vec()),
            ("cut.zip.002", volume(2)),
        ],
    )
    .await;
    assert!(
        matches!(&made, Finished::Created { counts, .. } if counts.archives == 2),
        "{made:?}"
    );

    // A volume without its first volume, or whose first volume is no archive,
    // is dropped with that reason; so is a spanned ZIP's volume with no `.zip`.
    let made = upload_of(
        &s,
        "vlone",
        &[
            ("01.ass", ASS.to_vec()),
            ("lone.7z.002", volume(2)),
            ("fake.7z.001", b"just text".to_vec()),
            ("fake.7z.002", volume(2)),
            ("nozip.z01", volume(1)),
            ("nozip.zip", b"just text".to_vec()),
            ("alone.r00", volume(1)),
        ],
    )
    .await;
    let reasons = dropped_reasons(&made);
    let reason_of = |name: &str| {
        reasons
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, r)| r.as_str())
    };
    for name in ["lone.7z.002", "fake.7z.002", "nozip.z01", "alone.r00"] {
        assert_eq!(reason_of(name), Some(NO_FIRST_VOLUME), "{name}");
    }
    assert_eq!(
        reason_of("fake.7z.001"),
        Some(trss_subtitles::upload::NOT_AN_ARCHIVE)
    );
    assert!(
        matches!(&made, Finished::Created { counts, .. } if counts.archives == 0 && counts.subtitles == 1),
        "{made:?}"
    );

    // A volume that is empty keeps the reason for that.
    let made = upload_of(
        &s,
        "vempty",
        &[
            (
                "e.7z.001",
                [&b"7z\xBC\xAF\x27\x1C"[..], &volume(1)].concat(),
            ),
            ("e.7z.002", Vec::new()),
        ],
    )
    .await;
    assert_eq!(
        reason_of_in(&made, "e.7z.002"),
        Some("파일이 비어 있어요".to_owned())
    );
}

fn reason_of_in(made: &Finished, name: &str) -> Option<String> {
    dropped_reasons(made)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, r)| r)
}
