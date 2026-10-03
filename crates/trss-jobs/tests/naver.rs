//! The runner with the Naver source, against a local server shaped like a
//! Naver blog ([`trss_subtitles::testing`]), and once against the real site
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
    Runner, StepKind, StepState,
};
use trss_subtitles::{
    drive::Drive,
    naver::{self, NaverSource},
    testing::{naver_file, FileAnswer, PostAnswer, SourceServer, NAVER_PUBLISHED},
    verify, Sources,
};

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
        Sources::none().with_naver(server.naver()),
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
        creator: Some("수퍼소닉EX".to_owned()),
        revision_of: None,
        revises_attributed: false,
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

/// A SAMI file in UTF-16LE with its BOM, as Naver posts attach them.
fn sami(text: &str) -> Vec<u8> {
    let body = format!(
        "<SAMI>\r\n<HEAD><TITLE>{text}</TITLE></HEAD>\r\n<BODY>\r\n<SYNC Start=1000><P Class=KRCC>{text}\r\n</BODY>\r\n</SAMI>\r\n"
    );
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(body.encode_utf16().flat_map(u16::to_le_bytes));
    bytes
}

const SMI_01: &str = "Reincarnated.as.a.Sword.S02E01.The.Island.Floating.in.the.Sky.1080p.BILI.WEB-DL.JPN.AAC2.0.H.265.MSubs-ToonsHub.smi";

/// 수퍼소닉EX 224428424028's shape: three fonts and the episode's SMI, all
/// attached inside the inner frame.
fn sword(s: &Setup) -> (String, Vec<u8>) {
    let smi = sami("전생했더니 검이었습니다 2기 1화");
    s.server.naver_post(
        "gkdlfn0850",
        "224428424028",
        vec![PostAnswer::Naver(vec![
            naver_file(SMI_01, smi.len()),
            naver_file("H2MPRB.TTF", 4),
            naver_file("a옛날목욕탕L.ttf", 4),
        ])],
    );
    s.server
        .naver_attachment(SMI_01, vec![FileAnswer::Bytes(smi.clone())]);
    for font in ["H2MPRB.TTF", "a옛날목욕탕L.ttf"] {
        s.server
            .naver_attachment(font, vec![FileAnswer::Bytes(b"\0\x01\0\0".to_vec())]);
    }
    (s.server.naver_url("gkdlfn0850", "224428424028"), smi)
}

#[tokio::test]
async fn an_attachment_inside_the_inner_frame_is_received_with_the_posts_fonts() {
    let s = setup().await;
    let (url, smi) = sword(&s);
    let id = make(&s.store, "n1", &[("1", url)]).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    for kind in [StepKind::Open, StepKind::Receive] {
        assert_eq!(step(&d, kind), Some(StepState::Done), "{kind:?}");
    }
    assert_eq!(
        files_in(&s.area.at(&id)),
        ["H2MPRB.TTF", SMI_01, "a옛날목욕탕L.ttf"]
    );
    let item = &d.items[0];
    assert_eq!(item.state, ItemState::Done);
    let file = item.files.iter().find(|f| f.name == SMI_01).unwrap();
    assert_eq!(file.state, FileState::Done);
    assert_eq!(file.format, Some(Format::Smi));
    assert_eq!(file.size, Some(smi.len() as u64));
    assert_eq!(file.expected_size, Some(smi.len() as u64));
    assert_eq!(
        file.sha256.as_deref(),
        Some(area::hex(&Sha256::digest(&smi)).as_str())
    );
    assert_eq!(file.http_status, Some(200));
    let snapshot: Vec<[String; 2]> =
        serde_json::from_str(file.snapshot.as_deref().unwrap()).unwrap();
    assert_eq!(
        snapshot,
        [
            [naver::PUBLISH_DATE.to_owned(), NAVER_PUBLISHED.to_owned()],
            [naver::ATTACH_FILE_SIZE.to_owned(), smi.len().to_string()],
        ]
    );
    let fonts: Vec<Option<Format>> = item
        .files
        .iter()
        .filter(|f| f.name != SMI_01)
        .map(|f| f.format)
        .collect();
    assert_eq!(fonts, [Some(Format::Other), Some(Format::Other)]);
    // The inner page once, and the frame never.
    assert_eq!(s.server.naver_read("224428424028"), 1);
    assert!(s
        .server
        .seen()
        .iter()
        .all(|r| !r.cookie && !r.referer && r.path != "/gkdlfn0850/224428424028"));
}

#[tokio::test]
async fn a_zip_of_a_range_and_the_episodes_file_are_both_received() {
    let s = setup().await;
    let zip = verify::zip_of(&[("네죽사 08.ass", b"[Script Info]\r\n".as_slice())]);
    let ass = trss_subtitles::fake::ass("Kimi ga Shinu 08");
    let ass_name = "[SubsPlease] Kimi ga Shinu made Koi wo Shitai - 08 (1080p) [F3B053C5].ass";
    s.server.naver_post(
        "elainalove1017",
        "224324105274",
        vec![PostAnswer::Naver(vec![
            naver_file("네죽사 1~8화 자막.zip", zip.len()),
            naver_file(ass_name, ass.len()),
        ])],
    );
    s.server
        .naver_attachment("네죽사 1~8화 자막.zip", vec![FileAnswer::Bytes(zip)]);
    s.server
        .naver_attachment(ass_name, vec![FileAnswer::Bytes(ass)]);
    let url = s.server.naver_url("elainalove1017", "224324105274");
    let id = make(&s.store, "n2", &[("8", url)]).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    let formats: Vec<(&str, Option<Format>)> = d.items[0]
        .files
        .iter()
        .map(|f| (f.name.as_str(), f.format))
        .collect();
    assert_eq!(
        formats,
        [
            ("네죽사 1~8화 자막.zip", Some(Format::Zip)),
            (ass_name, Some(Format::Ass)),
        ]
    );
}

#[tokio::test]
async fn a_captcha_or_a_page_of_no_post_is_a_failure_with_no_file_asked_for() {
    let s = setup().await;
    s.server.naver_post(
        "blog",
        "10",
        vec![PostAnswer::Page(
            r#"<html><body><form><img id="captchaimg" src="x"><p>자동입력 방지 문자를 입력해 주세요</p></form></body></html>"#
                .to_owned(),
        )],
    );
    s.server
        .naver_post("blog", "11", vec![PostAnswer::Status(404)]);
    let captcha = make(&s.store, "n3", &[("1", s.server.naver_url("blog", "10"))]).await;
    let gone = make(&s.store, "n4", &[("1", s.server.naver_url("blog", "11"))]).await;
    run(&s).await;

    let d = detail(&s, &captcha).await;
    assert_eq!(d.row.state, JobState::Failed);
    assert_eq!(d.row.failure, Some(FailureKind::Changed));
    assert!(d.items[0].files.is_empty());
    let d = detail(&s, &gone).await;
    assert_eq!(d.row.failure, Some(FailureKind::Missing));
    assert!(s.server.seen().iter().all(|r| r.host != naver::FILE_HOST));
}

#[tokio::test]
async fn a_refused_address_is_read_again_once_and_received() {
    let s = setup().await;
    let (url, smi) = sword(&s);
    s.server.naver_attachment(
        SMI_01,
        vec![FileAnswer::Refused, FileAnswer::Bytes(smi.clone())],
    );
    let id = make(&s.store, "n5", &[("1", url)]).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    let file = d.items[0].files.iter().find(|f| f.name == SMI_01).unwrap();
    assert_eq!(file.state, FileState::Done);
    assert_eq!(file.size, Some(smi.len() as u64));
    // Opened once, read again once for the new address.
    assert_eq!(s.server.naver_read("224428424028"), 2);
}

/// What a Naver receipt keeps: the post's address, the file's key and the
/// names; no download address or token.
#[tokio::test]
async fn no_download_address_or_token_reaches_the_records() {
    let s = setup().await;
    let (url, _) = sword(&s);
    s.server.naver_attachment(SMI_01, vec![FileAnswer::Refused]);
    let id = make(&s.store, "n6", &[("1", url)]).await;
    run(&s).await;
    assert_eq!(
        detail(&s, &id).await.row.failure,
        Some(FailureKind::Expired)
    );

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
    assert!(dump.contains(&format!("naver:gkdlfn0850/224428424028/{SMI_01}")));
    for secret in ["download.blog", "/open/", "/T1/", "/T2/", "PostView"] {
        let hits: Vec<&str> = dump.lines().filter(|l| l.contains(secret)).collect();
        assert!(hits.is_empty(), "{secret}: {hits:?}");
    }
}

/// Receives the real post of 공룡이 네죽사 8화: its ZIP of 1~8화 and its ASS
/// of 8화. Run by hand:
/// `cargo test -p trss-jobs --test naver -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "reaches the real Naver blog"]
async fn a_real_naver_post_is_received() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let store = JobStore::new(db);
    let area = ReceiveArea::in_app_data(dir.path());
    let runner = Runner::new(
        store.clone(),
        Sources::none().with_naver(NaverSource::new(Drive::new())),
        area.clone(),
        trss_core::system_clock(),
    );
    let id = make(
        &store,
        "real",
        &[(
            "8",
            "https://blog.naver.com/elainalove1017/224324105274".to_owned(),
        )],
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
    let formats: Vec<Option<Format>> = d
        .items
        .iter()
        .flat_map(|i| &i.files)
        .map(|f| f.format)
        .collect();
    assert_eq!(formats, [Some(Format::Zip), Some(Format::Ass)]);
}
