//! The runner with the Tistory source against a local server shaped like
//! Tistory ([`trss_subtitles::testing`]), and once against the real site
//! (ignored).

use std::{
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use trss_core::{Clock, Db, DbError};
use trss_jobs::{
    area::{self, ReceiveArea},
    store::JobDetail,
    Created, FailureKind, FileState, Format, ItemState, JobState, JobStore, NewItem, NewJob,
    Runner, StepKind, StepState, Wait,
};
use trss_subtitles::{
    testing::{spec, FileAnswer, PostAnswer, SourceServer, BODY_OPEN, CDN},
    tistory::TistorySource,
    verify, Sources,
};

const SRT: &[u8] = b"1\n00:00:01,000 --> 00:00:02,000\nHello\n";

struct Setup {
    _dir: tempfile::TempDir,
    store: JobStore,
    runner: Runner,
    area: ReceiveArea,
    server: SourceServer,
}

fn ticking_clock() -> Clock {
    let now = Arc::new(AtomicI64::new(1_000));
    Arc::new(move || now.fetch_add(10, Ordering::SeqCst))
}

async fn setup() -> Setup {
    let server = SourceServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let store = JobStore::new(db);
    let area = ReceiveArea::in_app_data(dir.path());
    let runner = Runner::new(
        store.clone(),
        Sources::none().with_tistory(server.source()),
        area.clone(),
        ticking_clock(),
    )
    .with_retry_waits(vec![Duration::from_millis(10), Duration::from_millis(10)]);
    Setup {
        _dir: dir,
        store,
        runner,
        area,
        server,
    }
}

/// Makes a job of `posts` (episode, address).
async fn make(store: &JobStore, command: &str, posts: &[(&str, String)]) -> String {
    let job = NewJob {
        command_id: command.to_owned(),
        request: "{}".to_owned(),
        origin: "pick".to_owned(),
        work_id: None,
        season: Some(1),
        anime_no: None,
        source_id: None,
        creator: Some("제작자".to_owned()),
        revision_of: None,
        items: posts
            .iter()
            .map(|(episode, post)| NewItem {
                observation_id: None,
                episode: (*episode).to_owned(),
                post_url: post.clone(),
                found_at: 500,
            })
            .collect(),
    };
    match store.create(job, 900).await.unwrap() {
        Created::Created(id) => id,
        other => panic!("created: {other:?}"),
    }
}

async fn run(s: &Setup) {
    s.runner.run_ready(&CancellationToken::new()).await.unwrap();
}

async fn detail(s: &Setup, id: &str) -> JobDetail {
    s.store.detail(id).await.unwrap().unwrap()
}

fn step(d: &JobDetail, kind: StepKind) -> Option<StepState> {
    d.steps.iter().find(|s| s.step == kind).map(|s| s.state)
}

fn files_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// How many times the CDN was asked for file `id`.
fn asked(s: &Setup, id: &str) -> usize {
    s.server
        .seen()
        .iter()
        .filter(|r| r.host == CDN && r.path.starts_with(&format!("/dna/{id}/")))
        .count()
}

fn posts_read(s: &Setup) -> usize {
    s.server
        .seen()
        .iter()
        .filter(|r| r.host.ends_with(".tistory.com"))
        .count()
}

#[tokio::test]
async fn a_tistory_post_goes_through_open_and_receive_and_leaves_its_zip_with_size_and_sha256() {
    let s = setup().await;
    let zip = verify::zip_of(&[("Seihantai - 24.srt", SRT)]);
    s.server.post(
        "sumomomo",
        492,
        vec![PostAnswer::Files(vec![spec(
            "z1",
            "Seihantai - 24.zip",
            "0.01MB",
        )])],
    );
    s.server.file("z1", vec![FileAnswer::Bytes(zip.clone())]);
    let id = make(
        &s.store,
        "c1",
        &[("24", s.server.post_url("sumomomo", 492))],
    )
    .await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    for kind in [StepKind::Found, StepKind::Open, StepKind::Receive] {
        assert_eq!(step(&d, kind), Some(StepState::Done), "{kind:?}");
    }
    assert_eq!(files_in(&s.area.at(&id)), ["Seihantai - 24.zip"]);
    assert!(files_in(&s.area.at(".tmp")).is_empty());

    let file = &d.items[0].files[0];
    assert_eq!(file.state, FileState::Done);
    assert_eq!(file.format, Some(Format::Zip));
    assert_eq!(file.size, Some(zip.len() as u64));
    assert_eq!(file.expected_size, Some(zip.len() as u64));
    assert_eq!(
        file.sha256.as_deref(),
        Some(area::hex(&Sha256::digest(&zip)).as_str())
    );
    let on_disk = std::fs::read(s.area.at(file.path.as_deref().unwrap())).unwrap();
    assert_eq!(on_disk, zip);
    assert_eq!(file.http_status, Some(200));
    assert_eq!(
        file.content_type.as_deref(),
        Some("application/octet-stream")
    );
    assert_eq!(file.failure, None);
    let snapshot: Vec<[String; 2]> =
        serde_json::from_str(file.snapshot.as_deref().unwrap()).unwrap();
    assert_eq!(
        snapshot,
        [
            [
                "article:modified_time".to_owned(),
                "2026-09-28T00:13:41+09:00".to_owned()
            ],
            ["declared_size".to_owned(), "0.01MB".to_owned()]
        ]
    );
    assert!(s.server.seen().iter().all(|r| !r.cookie && !r.referer));
    assert!(d
        .events
        .iter()
        .any(|e| e.message == "24화: 파일을 받았어요"
            && e.detail.as_deref().unwrap().ends_with("ZIP")));
}

#[tokio::test]
async fn an_error_page_instead_of_the_attachment_fails_as_not_a_file_with_its_reason() {
    let s = setup().await;
    s.server.post(
        "blog",
        1,
        vec![PostAnswer::Files(vec![spec("a", "a.zip", "1KB")])],
    );
    s.server.file("a", vec![FileAnswer::Page]);
    let id = make(&s.store, "c1", &[("1", s.server.post_url("blog", 1))]).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Failed);
    assert_eq!(d.row.failure, Some(FailureKind::NotAFile));
    let item = &d.items[0];
    assert_eq!(item.state, ItemState::Failed);
    assert_eq!(item.failure, Some(FailureKind::NotAFile));
    assert!(item.reason.as_deref().unwrap().contains("HTML"));
    let file = &item.files[0];
    assert_eq!(file.state, FileState::Failed);
    assert_eq!(file.failure, Some(FailureKind::NotAFile));
    assert_eq!(file.http_status, Some(200));
    assert_eq!(file.content_type.as_deref(), Some("text/html"));
    assert!(file.response_size.unwrap() > 0);
    assert_eq!(file.format, None);
    // Not received: its bytes are gone, nothing is in the job's folder.
    assert!(files_in(&s.area.at(&id)).is_empty());
    assert!(files_in(&s.area.at(".tmp")).is_empty());
    assert!(d.events.iter().any(|e| e
        .detail
        .as_deref()
        .unwrap_or_default()
        .contains("파일 아님")));
    // Not a network failure: asked once.
    assert_eq!(asked(&s, "a"), 1);
}

#[tokio::test]
async fn an_expired_address_is_read_from_the_post_again_and_received() {
    let s = setup().await;
    s.server.post(
        "blog",
        1,
        vec![PostAnswer::Files(vec![spec("a", "a.srt", "1KB")])],
    );
    s.server.file(
        "a",
        vec![FileAnswer::Refused, FileAnswer::Bytes(SRT.to_vec())],
    );
    let id = make(&s.store, "c1", &[("1", s.server.post_url("blog", 1))]).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(d.items[0].files.len(), 1);
    assert_eq!(d.items[0].files[0].format, Some(Format::Srt));
    assert_eq!((posts_read(&s), asked(&s, "a")), (2, 2));
}

#[tokio::test]
async fn an_address_refused_again_after_reading_the_post_fails_as_expired() {
    let s = setup().await;
    s.server.post(
        "blog",
        1,
        vec![PostAnswer::Files(vec![spec("a", "a.zip", "1KB")])],
    );
    s.server.file("a", vec![FileAnswer::Refused]);
    let id = make(&s.store, "c1", &[("1", s.server.post_url("blog", 1))]).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Failed);
    assert_eq!(d.items[0].failure, Some(FailureKind::Expired));
    let file = &d.items[0].files[0];
    assert_eq!(file.failure, Some(FailureKind::Expired));
    assert_eq!(file.http_status, Some(404));
    assert_eq!(file.content_type.as_deref(), Some("text/html"));
    assert_eq!(file.response_size, Some(150));
    // Read again once, not retried as a network failure.
    assert_eq!((posts_read(&s), asked(&s, "a")), (2, 2));
}

#[tokio::test]
async fn a_file_gone_from_the_post_read_again_fails_as_missing() {
    let s = setup().await;
    s.server.post(
        "blog",
        1,
        vec![
            PostAnswer::Files(vec![spec("a", "a.zip", "1KB")]),
            PostAnswer::Files(vec![spec("b", "b.zip", "1KB")]),
        ],
    );
    s.server.file("a", vec![FileAnswer::Refused]);
    let id = make(&s.store, "c1", &[("1", s.server.post_url("blog", 1))]).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.items[0].failure, Some(FailureKind::Missing));
    assert_eq!(d.items[0].files[0].failure, Some(FailureKind::Missing));
    assert_eq!(asked(&s, "b"), 0);
}

#[tokio::test]
async fn a_missing_post_fails_as_missing_and_a_drive_folder_post_waits_for_a_source() {
    let s = setup().await;
    let folder = format!(
        r#"<html><body>{BODY_OPEN}<p><a href="https://drive.google.com/drive/folders/10YFO-jkkgsybQnPpl5TVAwPdx2P0SE-y">자막 모음</a></p></div></body></html>"#
    );
    s.server.post("felia", 1187, vec![PostAnswer::Page(folder)]);
    let missing = make(
        &s.store,
        "c1",
        &[("1", s.server.post_url("sumomomo", 99999))],
    )
    .await;
    let drive = make(&s.store, "c2", &[("1", s.server.post_url("felia", 1187))]).await;
    run(&s).await;

    let d = detail(&s, &missing).await;
    assert_eq!(d.row.state, JobState::Failed);
    assert_eq!(d.row.failure, Some(FailureKind::Missing));
    assert_eq!(d.items[0].failure, Some(FailureKind::Missing));
    assert!(d.items[0].reason.as_deref().unwrap().contains("404"));
    assert_eq!(step(&d, StepKind::Open), Some(StepState::Failed));
    assert!(d.items[0].files.is_empty());

    let d = detail(&s, &drive).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Subtitle))
    );
    assert_eq!(d.row.failure, None);
    assert!(d.row.note.as_deref().unwrap().contains("Google Drive 폴더"));
    assert_eq!(d.items[0].state, ItemState::Waiting);
    assert!(d.items[0]
        .reason
        .as_deref()
        .unwrap()
        .contains("Google Drive 폴더"));
    // A worker start puts it back in line, as a post no source reads.
    assert_eq!(s.runner.requeue_waiting_for_sources().await.unwrap(), 1);
}

#[tokio::test]
async fn every_file_of_a_post_is_received_once_for_the_episodes_it_serves() {
    let s = setup().await;
    let series = verify::zip_of(&[("01.srt", SRT), ("02.srt", SRT)]);
    let font = verify::zip_of(&[("Hotori.ttf", b"\x00\x01\x00\x00")]);
    let smi = b"<SAMI>\n<BODY>\n<SYNC Start=1000><P>hi\n</BODY></SAMI>".to_vec();
    s.server.post(
        "isulbi",
        40,
        vec![PostAnswer::Files(vec![
            spec("all", "Kami no Shizuku1-24.Zip", "1.2MB"),
            spec("e1", "Kami no Shizuku - 01SubsPlease.smi", "40KB"),
            spec("font", "Hotori font.zip", "3MB"),
        ])],
    );
    s.server.file("all", vec![FileAnswer::Bytes(series)]);
    s.server.file("e1", vec![FileAnswer::Bytes(smi)]);
    s.server.file("font", vec![FileAnswer::Bytes(font)]);
    let post = s.server.post_url("isulbi", 40);
    let id = make(&s.store, "c1", &[("1", post.clone()), ("2", post)]).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(
        files_in(&s.area.at(&id)),
        [
            "Hotori font.zip",
            "Kami no Shizuku - 01SubsPlease.smi",
            "Kami no Shizuku1-24.Zip"
        ]
    );
    for id in ["all", "e1", "font"] {
        assert_eq!(asked(&s, id), 1, "{id}");
    }
    let formats: Vec<_> = d.items[0].files.iter().map(|f| f.format).collect();
    assert_eq!(
        formats,
        [Some(Format::Zip), Some(Format::Smi), Some(Format::Zip)]
    );
    assert!(d.items[1].files.iter().all(|f| f.same_as.is_some()));
}

#[tokio::test]
async fn a_network_failure_is_tried_again_twice_then_fails_as_network() {
    let s = setup().await;
    // The post fails once, then opens; its file never comes.
    s.server.post(
        "blog",
        1,
        vec![
            PostAnswer::Status(503),
            PostAnswer::Files(vec![spec("a", "a.zip", "1KB")]),
        ],
    );
    s.server.file("a", vec![FileAnswer::Status(502)]);
    let id = make(&s.store, "c1", &[("1", s.server.post_url("blog", 1))]).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Failed);
    assert_eq!(d.row.failure, Some(FailureKind::Network));
    assert_eq!((posts_read(&s), asked(&s, "a")), (2, 3));
    let states: Vec<_> = d.items[0].files.iter().map(|f| f.state).collect();
    assert_eq!(
        states,
        [
            FileState::Abandoned,
            FileState::Abandoned,
            FileState::Failed
        ]
    );
    assert!(d.items[0]
        .files
        .iter()
        .all(|f| f.failure == Some(FailureKind::Network) && f.http_status == Some(502)));
    let retries = d
        .events
        .iter()
        .filter(|e| e.message.contains("다시 시도해요"))
        .count();
    assert_eq!(retries, 3);

    // A failure that came back succeeds on a retry.
    s.server.file(
        "a",
        vec![FileAnswer::Status(503), FileAnswer::Bytes(SRT.to_vec())],
    );
    s.server.post(
        "blog",
        2,
        vec![PostAnswer::Files(vec![spec("a", "a.srt", "1KB")])],
    );
    let id = make(&s.store, "c2", &[("1", s.server.post_url("blog", 2))]).await;
    run(&s).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Done);
}

#[tokio::test]
async fn a_shutdown_during_a_retry_wait_stops_the_run_at_once() {
    let s = setup().await;
    let runner = s
        .runner
        .clone()
        .with_retry_waits(vec![Duration::from_secs(30)]);
    s.server.post("blog", 1, vec![PostAnswer::Status(503)]);
    let id = make(&s.store, "c1", &[("1", s.server.post_url("blog", 1))]).await;

    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let cancel = cancel.clone();
        async move { runner.run_ready(&cancel).await }
    });
    // Stop once the run waits to retry, however slow the machine is.
    let waiting = tokio::time::Instant::now();
    while !detail(&s, &id)
        .await
        .events
        .iter()
        .any(|e| e.message.contains("다시 시도해요"))
    {
        assert!(
            waiting.elapsed() < Duration::from_secs(10),
            "no retry wait began"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("stops within the wait")
        .unwrap()
        .unwrap();
    assert_eq!(detail(&s, &id).await.row.state, JobState::Running);
}

#[tokio::test]
async fn no_signed_address_reaches_the_records_or_the_log() {
    let s = setup().await;
    s.server.post(
        "blog",
        1,
        vec![PostAnswer::Files(vec![
            spec("a", "a.zip", "1KB"),
            spec("b", "b.srt", "1KB"),
            spec("c", "c.zip", "1KB"),
        ])],
    );
    s.server.file(
        "a",
        vec![
            FileAnswer::Refused,
            FileAnswer::Bytes(verify::zip_of(&[("a.srt", SRT)])),
        ],
    );
    s.server.file("b", vec![FileAnswer::Page]);
    s.server.file("c", vec![FileAnswer::Refused]);
    let id = make(&s.store, "c1", &[("1", s.server.post_url("blog", 1))]).await;
    run(&s).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Failed);

    // Every text of every job table, joined.
    let dump: String = s
        .store
        .db()
        .run::<_, DbError, _>(|c| {
            let mut all = String::new();
            for table in [
                "subtitle_jobs",
                "subtitle_job_items",
                "subtitle_job_steps",
                "subtitle_job_events",
                "subtitle_job_files",
            ] {
                let mut stmt = c.prepare(&format!("SELECT * FROM {table}"))?;
                let columns = stmt.column_count();
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    for i in 0..columns {
                        if let Ok(Some(text)) = row.get::<_, Option<String>>(i) {
                            all.push_str(&text);
                            all.push('\n');
                        }
                    }
                }
            }
            Ok(all)
        })
        .await
        .unwrap();
    assert!(dump.contains("a.zip") && dump.contains("tistory:"));
    for secret in ["signature=", "credential=", "?", "kakaocdn.net:"] {
        let hits: Vec<&str> = dump.lines().filter(|l| l.contains(secret)).collect();
        // Post addresses carry no query; a stored address never does.
        assert!(hits.is_empty(), "{secret}: {hits:?}");
    }
}

/// Receives `https://sumomomo.tistory.com/492` from the real site. Run by
/// hand: `cargo test -p trss-jobs --test tistory -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "reaches the real Tistory"]
async fn a_real_tistory_post_is_received() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let store = JobStore::new(db);
    let area = ReceiveArea::in_app_data(dir.path());
    let runner = Runner::new(
        store.clone(),
        Sources::none().with_tistory(TistorySource::new(trss_subtitles::drive::Drive::new())),
        area.clone(),
        trss_core::system_clock(),
    );
    let id = make(
        &store,
        "real",
        &[("24", "https://sumomomo.tistory.com/492".to_owned())],
    )
    .await;
    runner.run_ready(&CancellationToken::new()).await.unwrap();

    let d = store.detail(&id).await.unwrap().unwrap();
    for e in d.events.iter().rev() {
        println!("{} {}", e.message, e.detail.as_deref().unwrap_or_default());
    }
    for f in d.items.iter().flat_map(|i| &i.files) {
        println!(
            "{} state={:?} size={:?} expected={:?} format={:?} sha256={:?} status={:?} type={:?} snapshot={:?}",
            f.name, f.state, f.size, f.expected_size, f.format, f.sha256, f.http_status,
            f.content_type, f.snapshot
        );
    }
    assert_eq!(d.row.state, JobState::Done);
}

#[tokio::test]
async fn a_file_past_its_byte_limit_fails_as_not_a_file_and_leaves_no_bytes() {
    let s = setup().await;
    let runner = Runner::new(
        s.store.clone(),
        Sources::none().with_tistory(s.server.source_with(trss_subtitles::tistory::Limits {
            spacing: Duration::ZERO,
            max_file: 2048,
            ..Default::default()
        })),
        s.area.clone(),
        ticking_clock(),
    );
    s.server.post(
        "blog",
        1,
        vec![PostAnswer::Files(vec![spec("big", "a.srt", "4KB")])],
    );
    s.server
        .file("big", vec![FileAnswer::Streamed(SRT.repeat(200))]);
    let id = make(&s.store, "c1", &[("1", s.server.post_url("blog", 1))]).await;
    runner.run_ready(&CancellationToken::new()).await.unwrap();

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Failed);
    assert_eq!(d.items[0].failure, Some(FailureKind::NotAFile));
    let file = &d.items[0].files[0];
    assert_eq!(file.state, FileState::Failed);
    assert!(file.reason.as_deref().unwrap().contains("2048바이트"));
    assert_eq!(file.path, None);
    // Not tried again: the limit is no network failure.
    assert_eq!(asked(&s, "big"), 1);
    assert!(files_in(&s.area.at(".tmp")).is_empty());
    assert!(files_in(&s.area.at(&id)).is_empty());
}
