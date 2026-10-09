//! Uploads (`trss_jobs::upload`) against the records and the receive area:
//! the job they make has its files received and waits for the worker, which
//! unpacks and analyses them for a person's 배치 확인, a refused or abandoned
//! upload leaves no byte, and uploads wait for their turn.

use crate::{
    world::{tree, Base},
    Handles,
};
use std::time::Duration;

use trss_jobs::{
    upload::{Counts, Dropped, Limits, UploadError, UploadRequest, UPLOAD_SLOTS},
    Finished, JobState, ReceiveArea, Uploads,
};

const ASS: &[u8] = b"[Script Info]\nTitle: x\n";
const SRT: &[u8] = b"1\r\n00:00:01,000 --> 00:00:02,500\r\nHello\r\n";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06";

fn ttf() -> Vec<u8> {
    let mut bytes = b"\x00\x01\x00\x00\x00\x0C".to_vec();
    bytes.resize(12 + 16 * 12, 0);
    bytes
}

struct Setup {
    _dir: tempfile::TempDir,
    store: Handles,
    uploads: Uploads,
    area: ReceiveArea,
}

async fn setup() -> Setup {
    let base = Base::new().await;
    let uploads = Uploads::new(base.store.requests.clone(), base.area.clone());
    Setup {
        _dir: base.dir,
        store: base.store,
        uploads,
        area: base.area,
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
async fn an_upload_is_a_received_job_that_waits_for_the_worker() {
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
    // The text file is dropped by its content, and the job names it.
    let not_them = Dropped {
        name: "note.txt".to_owned(),
        reason: trss_subtitles::upload::NOT_THEM.to_owned(),
    };
    assert_eq!(dropped, std::slice::from_ref(&not_them));

    // The worker takes it next, to analyse it for its 배치 확인; a
    // restart's look at the waiting jobs leaves it as it is.
    assert!(s.store.run.has_ready().await.unwrap());
    assert_eq!(
        s.store
            .run
            .requeue_waiting_for_sources(6_000)
            .await
            .unwrap(),
        0
    );
    let detail = s.store.views.detail(&job_id).await.unwrap().unwrap();
    assert_eq!(detail.row.state, JobState::Pending);
    assert_eq!(detail.row.finished_at, None);
    assert_eq!(detail.row.origin, "upload");
    assert_eq!(detail.row.episodes, Vec::<String>::new());
    assert_eq!(detail.row.source, None);
    assert_eq!(
        detail
            .dropped
            .iter()
            .map(|d| (d.name.as_str(), d.reason.as_str()))
            .collect::<Vec<_>>(),
        [(not_them.name.as_str(), not_them.reason.as_str())]
    );
    // The log says what was left out.
    assert!(detail.events[0]
        .detail
        .as_deref()
        .unwrap()
        .contains("뺀 파일 1개"));
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
    assert_eq!(
        s.store
            .run
            .claim_next(6_000)
            .await
            .unwrap()
            .map(|(id, ..)| id),
        Some(job_id)
    );
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
    assert_eq!(s.store.views.open_jobs().await.unwrap().len(), 1);
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
    // A file of exactly the size of one file is taken.
    let mut staging = tight.begin().await.unwrap();
    staging.start("a.srt").await.unwrap();
    staging.write(&[b'x'; 12]).await.unwrap();
    staging.end().await.unwrap();
    drop(staging);

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
        if let Some(job) = s.store.views.open_jobs().await.unwrap().first() {
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
        .run.db()
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
    let none = Uploads::new(
        s.store.requests.clone(),
        ReceiveArea::new("/nonexistent/receive"),
    );
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
    let Created::Created(job) = s.store.requests.create(job, 900).await.unwrap() else {
        panic!("not created");
    };
    let items = s.store.views.items(&job).await.unwrap();
    let (open, settled) = (items[0].id, items[1].id);
    s.store
        .run
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
    let detail = s.store.views.detail(job_id).await.unwrap().unwrap();
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
        s.store.run.claim_next(2_000).await.unwrap(),
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
        .requests
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
    let row = s.store.views.detail(&id).await.unwrap().unwrap().row;
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
    let detail = s.store.views.detail(job_id).await.unwrap().unwrap();
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

#[tokio::test]
async fn a_file_dropped_by_its_content_is_not_stored_whatever_its_name_says() {
    let s = setup().await;
    let made = upload_of(
        &s,
        "img",
        &[
            ("cover.ass", PNG.to_vec()),
            ("real.ass", ASS.to_vec()),
            // A subtitle under a name that is no subtitle's is kept all the same.
            ("notes.txt", SRT.to_vec()),
        ],
    )
    .await;
    let Finished::Created {
        job_id,
        counts,
        dropped,
    } = &made
    else {
        panic!("{made:?}");
    };
    assert_eq!(
        dropped.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
        ["cover.ass"]
    );
    assert_eq!(
        *counts,
        Counts {
            subtitles: 2,
            fonts: 0,
            archives: 0
        }
    );
    let detail = s.store.views.detail(job_id).await.unwrap().unwrap();
    let names: Vec<_> = detail.items[0]
        .files
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, ["real.ass", "notes.txt"]);
    // The image was not stored.
    assert_eq!(tree(&s.area.at(job_id)), ["notes.txt", "real.ass"]);
}

#[tokio::test]
async fn a_zip_stays_whole_in_the_package_and_one_cut_short_is_dropped_with_its_reason() {
    use trss_subtitles::{upload::Kind, verify::Format};
    let s = setup().await;
    let zip = trss_subtitles::verify::zip_of(&[
        ("01.ass", ASS),
        ("fonts/Font.ttf", &ttf()),
        ("readme.txt", b"hi"),
    ]);
    let made = upload_of(&s, "z1", &[("pack.zip", zip.clone())]).await;
    let Finished::Created {
        job_id,
        counts,
        dropped,
    } = &made
    else {
        panic!("{made:?}");
    };
    assert_eq!(
        *counts,
        Counts {
            subtitles: 0,
            fonts: 0,
            archives: 1
        }
    );
    // What is inside is not judged here: the readme is not a dropped file.
    assert!(dropped.is_empty());
    let detail = s.store.views.detail(job_id).await.unwrap().unwrap();
    let file = &detail.items[0].files[0];
    assert_eq!(file.name, "pack.zip");
    assert_eq!(file.kind, Some(Kind::Archive));
    assert_eq!(file.format, Some(Format::Zip));
    assert_eq!(file.size, Some(zip.len() as u64));
    assert_eq!(
        std::fs::read(s.area.at(file.path.as_deref().unwrap())).unwrap(),
        zip
    );

    // A ZIP that cannot be read to its end is dropped, with the reason.
    let made = upload_of(
        &s,
        "z2",
        &[
            ("broken.zip", zip[..zip.len() / 2].to_vec()),
            ("ok.srt", SRT.to_vec()),
        ],
    )
    .await;
    let reasons = dropped_reasons(&made);
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert_eq!(reasons[0].0, "broken.zip");
    assert!(reasons[0].1.contains("ZIP"), "{reasons:?}");
}

/// What an upload sends: the files, the names the browser left out, and the names the
/// server drops.
type Sent<'a> = (&'a [(&'a str, &'a [u8])], &'a [&'a str], &'a [&'a str]);

#[tokio::test]
async fn nothing_to_keep_makes_no_job_no_folder_and_leaves_nothing_staged() {
    let s = setup().await;
    let cases: [Sent; 4] = [
        (
            &[("cover.jpg", PNG), ("readme.txt", b"text")],
            &[],
            &["cover.jpg", "readme.txt"],
        ),
        (&[], &["a.mkv", "b.nfo"], &["a.mkv", "b.nfo"]),
        (&[], &[], &[]),
        (&[("empty.ass", b"")], &[], &["empty.ass"]),
    ];
    for (i, (files, skipped, names)) in cases.into_iter().enumerate() {
        let mut staging = stage(&s.uploads, files).await;
        for name in skipped {
            staging.skip(name).unwrap();
        }
        let made = s
            .uploads
            .finish(staging, request(&format!("n{i}")), 1_000)
            .await
            .unwrap();
        assert!(matches!(made, Finished::Nothing { .. }), "{i}: {made:?}");
        let dropped: Vec<_> = dropped_reasons(&made).into_iter().map(|(n, _)| n).collect();
        assert_eq!(dropped, names, "{i}");
    }
    assert!(s.store.views.open_jobs().await.unwrap().is_empty());
    assert_eq!(s.store.views.done_page(None, 50).await.unwrap().total, 0);
    // No folder of a job, and nothing staged is left.
    let folders: Vec<_> = std::fs::read_dir(s.area.root())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != ".tmp")
        .collect();
    assert!(folders.is_empty(), "{folders:?}");
    assert_eq!(tmp_entries(&s.area), 0);
}

#[tokio::test]
async fn names_cannot_leave_the_jobs_folder_and_a_folders_paths_are_kept_as_names() {
    let s = setup().await;
    let made = upload_of(
        &s,
        "names",
        &[
            ("../../evil.ass", ASS.to_vec()),
            ("Show/Season 1/01.ass", ASS.to_vec()),
            ("Show/Season 2/01.ass", SRT.to_vec()),
            ("Show/fonts/\u{202E}gpj.ttf", ttf()),
            ("/abs/path/ep.srt", SRT.to_vec()),
            ("..\\..\\win.ass", ASS.to_vec()),
        ],
    )
    .await;
    let Finished::Created { job_id: id, .. } = &made else {
        panic!("{made:?}");
    };
    let detail = s.store.views.detail(id).await.unwrap().unwrap();
    let names: Vec<_> = detail.items[0]
        .files
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "evil.ass",
            "Show/Season 1/01.ass",
            "Show/Season 2/01.ass",
            "Show/fonts/gpj.ttf",
            "abs/path/ep.srt",
            "win.ass"
        ]
    );
    // Every file is flat in the job's folder, the second `01.ass` numbered,
    // and nothing is anywhere else under the receive area.
    assert_eq!(
        tree(s.area.root()),
        [
            "01 (2).ass",
            "01.ass",
            "ep.srt",
            "evil.ass",
            "gpj.ttf",
            "win.ass"
        ]
        .map(|file| format!("{id}/{file}"))
    );
    assert!(!s._dir.path().join("evil.ass").exists());
    assert!(!s._dir.path().parent().unwrap().join("evil.ass").exists());
}

fn font_upload() -> Vec<(&'static str, Vec<u8>)> {
    vec![("01.ass", ASS.to_vec()), ("Font.ttf", ttf())]
}

#[tokio::test]
async fn an_upload_under_a_used_id_that_differs_in_creator_season_names_or_bytes_is_a_mismatch() {
    let s = setup().await;
    s.store
        .run
        .db()
        .run::<_, trss_core::DbError, _>(|c| {
            c.execute_batch(
                "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                 VALUES ('s1', 3424, '에루샤', 5), ('s2', 3424, '다른', 5);",
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let base = UploadRequest {
        anime_no: Some(3424),
        source_id: Some("s1".to_owned()),
        creator: Some("에루샤".to_owned()),
        ..request("same")
    };
    let send = |request: UploadRequest,
                skipped: &'static [&'static str],
                files: Vec<(&'static str, Vec<u8>)>| {
        let uploads = s.uploads.clone();
        async move {
            let mut staging = uploads.begin().await.unwrap();
            for name in skipped {
                staging.skip(name).unwrap();
            }
            for (name, bytes) in &files {
                staging.start(name).await.unwrap();
                staging.write(bytes).await.unwrap();
                staging.end().await.unwrap();
            }
            uploads.finish(staging, request, 1_000).await.unwrap()
        }
    };
    let Finished::Created { job_id, .. } = send(base.clone(), &[], font_upload()).await else {
        panic!("not created");
    };
    // The answer was lost: the same upload again.
    assert_eq!(
        send(base.clone(), &[], font_upload()).await,
        Finished::Existing(job_id.clone())
    );

    let variants = [
        (
            "one file fewer",
            base.clone(),
            &[][..],
            vec![font_upload().remove(0)],
        ),
        (
            "another creator",
            UploadRequest {
                source_id: Some("s2".to_owned()),
                creator: Some("다른".to_owned()),
                ..base.clone()
            },
            &[],
            font_upload(),
        ),
        (
            "another season",
            UploadRequest {
                season: 2,
                anime_no: None,
                source_id: None,
                creator: None,
                ..base.clone()
            },
            &[],
            font_upload(),
        ),
        ("a name left out", base.clone(), &["x"][..], font_upload()),
        (
            "other bytes",
            base.clone(),
            &[],
            vec![("01.ass", SRT.to_vec()), font_upload().remove(1)],
        ),
    ];
    for (what, request, skipped, files) in variants {
        assert_eq!(
            send(request, skipped, files).await,
            Finished::Mismatch(job_id.clone()),
            "{what}"
        );
    }
    // Only the first upload's files were kept, and nothing staged is left.
    assert_eq!(s.store.views.open_jobs().await.unwrap().len(), 1);
    assert_eq!(
        tree(s.area.root()),
        ["01.ass", "Font.ttf"].map(|file| format!("{job_id}/{file}"))
    );
    assert_eq!(tmp_entries(&s.area), 0);
}

#[tokio::test]
async fn two_deliveries_at_once_under_one_id_make_one_job() {
    let s = setup().await;
    let files = [("01.ass", ASS.to_vec()), ("02.srt", SRT.to_vec())];
    let (first, second) =
        tokio::join!(upload_of(&s, "race", &files), upload_of(&s, "race", &files));
    let (created, existing): (Vec<_>, Vec<_>) = [first, second]
        .into_iter()
        .partition(|made| matches!(made, Finished::Created { .. }));
    assert_eq!(
        (created.len(), existing.len()),
        (1, 1),
        "{created:?} {existing:?}"
    );
    let Finished::Created { job_id, .. } = &created[0] else {
        unreachable!()
    };
    assert_eq!(existing[0], Finished::Existing(job_id.clone()));
    assert_eq!(s.store.views.open_jobs().await.unwrap().len(), 1);
    assert_eq!(
        tree(s.area.root()),
        ["01.ass", "02.srt"].map(|file| format!("{job_id}/{file}"))
    );
    assert_eq!(tmp_entries(&s.area), 0);
}
