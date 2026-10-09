//! The runner reading a Tistory post whose subtitle is in WinPNG images
//! (`docs/specs/jobs.md`, 출처별 단계 초안). The browser behind the reader is
//! a fake that puts files where the real one would; the viewer's own page
//! script is tested against a real viewer (`trss-subtitles`'s ignored
//! `winpng_sample`).

use crate::Handles;
use std::{
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use tokio_util::sync::CancellationToken;
use trss_core::{Clock, Db};
use trss_jobs::{
    area::ReceiveArea,
    store::{FileRow, JobDetail},
    Created, FailureKind, FileState, Format, ItemState, JobState, NewItem, NewJob, Runner,
    StepKind, StepState, Wait,
};
use trss_subtitles::{
    testing::{PostAnswer, SourceServer},
    winpng::{BoxFuture, Staged, ViewRequest, Viewed, WinpngReader, NO_READER},
    Failure, Source, Sources,
};
use url::Url;

const SMI: &[u8] = b"\xEF\xBB\xBF<SAMI>\n<BODY>\n<SYNC Start=1000><P>hi\n</BODY></SAMI>";
const ASS: &[u8] = b"\xEF\xBB\xBF[Script Info]\nTitle: x\n\n[Events]\n";
const IMAGE: &str = "blog.kakaocdn.net/dna/pic/1/2/img.png";
const SECOND_IMAGE: &str = "blog.kakaocdn.net/dna/pic/3/4/more.png";

/// What the fake browser does for one reading.
enum Plan {
    /// Puts these (folder, name, bytes) of one image in the staging folder,
    /// the way the real reader moves each download into a folder of its own.
    Files(Vec<(Option<&'static str>, &'static str, &'static [u8])>),
    /// The same for two images of the post.
    TwoImages(
        Vec<(Option<&'static str>, &'static str, &'static [u8])>,
        Vec<(Option<&'static str>, &'static str, &'static [u8])>,
    ),
    NoSubtitle,
    NoImages,
    NeedsKey,
    Fail(Failure),
}

#[derive(Default)]
struct FakeReader {
    plans: Mutex<Vec<Plan>>,
    /// The job and whether the staging folder was empty, per reading.
    reads: Mutex<Vec<(String, bool)>>,
    released: Mutex<Vec<String>>,
}

impl FakeReader {
    fn with(plans: Vec<Plan>) -> Arc<FakeReader> {
        Arc::new(FakeReader {
            plans: Mutex::new(plans),
            ..FakeReader::default()
        })
    }
}

fn stage(
    request: &ViewRequest<'_>,
    image: &str,
    first: usize,
    files: Vec<(Option<&'static str>, &'static str, &'static [u8])>,
) -> Vec<Staged> {
    let mut staged = Vec::new();
    for (n, (folder, name, bytes)) in files.into_iter().enumerate() {
        let dir = request.staging.join((first + n + 1).to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        staged.push(Staged {
            image: image.to_owned(),
            folder: folder.map(str::to_owned),
            name: name.to_owned(),
            path,
        });
    }
    staged
}

impl WinpngReader for FakeReader {
    fn read<'a>(&'a self, request: ViewRequest<'a>) -> BoxFuture<'a, Result<Viewed, Failure>> {
        Box::pin(async move {
            let empty = std::fs::read_dir(request.staging)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false);
            self.reads
                .lock()
                .unwrap()
                .push((request.job.to_owned(), empty));
            let plan = self.plans.lock().unwrap().remove(0);
            match plan {
                Plan::Files(files) => Ok(Viewed::Files(stage(&request, IMAGE, 0, files))),
                Plan::TwoImages(first, second) => {
                    let mut staged = stage(&request, IMAGE, 0, first);
                    let more = staged.len();
                    staged.extend(stage(&request, SECOND_IMAGE, more, second));
                    Ok(Viewed::Files(staged))
                }
                Plan::NoSubtitle => Ok(Viewed::NoSubtitle),
                Plan::NoImages => Ok(Viewed::NoImages),
                Plan::NeedsKey => Ok(Viewed::NeedsKey),
                Plan::Fail(failure) => Err(failure),
            }
        })
    }

    fn release<'a>(&'a self, job: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move { self.released.lock().unwrap().push(job.to_owned()) })
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    store: Handles,
    runner: Runner,
    area: ReceiveArea,
    server: SourceServer,
    reader: Option<Arc<FakeReader>>,
}

fn ticking_clock() -> Clock {
    let now = Arc::new(AtomicI64::new(1_000));
    Arc::new(move || now.fetch_add(10, Ordering::SeqCst))
}

async fn setup(reader: Option<Arc<FakeReader>>) -> Setup {
    let server = SourceServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let store = Handles::new(db);
    let area = ReceiveArea::in_app_data(dir.path());
    let mut runner = Runner::new(
        store.run.clone(),
        Sources::none().with_tistory(server.source()),
        area.clone(),
        ticking_clock(),
    )
    .with_retry_waits(vec![Duration::from_millis(10), Duration::from_millis(10)]);
    if let Some(reader) = &reader {
        runner = runner.with_winpng(reader.clone());
    }
    // Every post of the tests has a picture and no attachment.
    server.post("blog", 1, vec![PostAnswer::Files(Vec::new())]);
    Setup {
        _dir: dir,
        store,
        runner,
        area,
        server,
        reader,
    }
}

async fn make(s: &Setup) -> String {
    let job = NewJob {
        command_id: "c1".to_owned(),
        request: "{}".to_owned(),
        origin: "pick".to_owned(),
        work_id: None,
        season: Some(1),
        anime_no: None,
        source_id: None,
        creator: Some("제작자".to_owned()),
        revision_of: None,
        revises_attributed: false,
        items: vec![NewItem {
            observation_id: None,
            episode: "1".to_owned(),
            post_url: s.server.post_url("blog", 1),
            found_at: 500,
        }],
    };
    match s.store.requests.create(job, 900).await.unwrap() {
        Created::Created(id) => id,
        other => panic!("created: {other:?}"),
    }
}

async fn run(s: &Setup) {
    s.runner.run_ready(&CancellationToken::new()).await.unwrap();
}

async fn detail(s: &Setup, id: &str) -> JobDetail {
    s.store.views.detail(id).await.unwrap().unwrap()
}

fn step(d: &JobDetail, kind: StepKind) -> Option<StepState> {
    d.steps.iter().find(|s| s.step == kind).map(|s| s.state)
}

/// Every file under `dir`, relative, sorted.
fn tree(dir: &std::path::Path) -> Vec<String> {
    fn walk(base: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.push(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[tokio::test]
async fn files_out_of_an_image_are_received_under_their_names_and_folders() {
    let reader = FakeReader::with(vec![Plan::Files(vec![
        (None, "Show.smi", SMI),
        (Some("회차/"), "01.smi", SMI),
        (Some("회차/2기/"), "02.smi", SMI),
        // The same name in another folder is a file of its own.
        (Some("회차/2기/"), "01.smi", SMI),
    ])]);
    let s = setup(Some(reader.clone())).await;
    let id = make(&s).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(step(&d, StepKind::Open), Some(StepState::Done));
    assert_eq!(step(&d, StepKind::Receive), Some(StepState::Done));
    let files = &d.items[0].files;
    assert_eq!(files.len(), 4);
    assert!(files
        .iter()
        .all(|f| f.state == FileState::Done && f.format == Some(Format::Smi)));
    let folders: Vec<(Option<&str>, &str)> = files
        .iter()
        .map(|f| (f.folder.as_deref(), f.name.as_str()))
        .collect();
    assert_eq!(
        folders,
        [
            (None, "Show.smi"),
            (Some("회차"), "01.smi"),
            (Some("회차/2기"), "02.smi"),
            (Some("회차/2기"), "01.smi"),
        ]
    );

    // On the disk under the job's folder, each a subtitle from its first byte.
    let job_dir = s.area.at(&ReceiveArea::job_dir(&id));
    assert_eq!(
        tree(&job_dir),
        [
            "Show.smi",
            "회차/01.smi",
            "회차/2기/01.smi",
            "회차/2기/02.smi"
        ]
    );
    for file in files {
        let path = s.area.at(file.path.as_deref().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes, SMI, "{}", path.display());
        assert!(bytes[3..].starts_with(b"<SAMI>"));
    }
    // Nothing is left of the reading or of the attempts.
    assert_eq!(tree(&s.area.at(".tmp")), Vec::<String>::new());
    assert!(!s.area.at(&format!(".tmp/winpng-{id}")).exists());
    // The browser run goes with the job.
    let reader = s.reader.as_ref().unwrap();
    assert_eq!(*reader.released.lock().unwrap(), std::slice::from_ref(&id));
    assert_eq!(*reader.reads.lock().unwrap(), [(id, true)]);
}

#[tokio::test]
async fn a_jamaker_image_gives_the_converted_smi_and_ass_together_and_no_jmk() {
    // What the viewer lists for a JMK: its SMI and ASS, its own link removed.
    let reader = FakeReader::with(vec![Plan::Files(vec![
        (Some("에피소드/"), "ep01.smi", SMI),
        (Some("에피소드/"), "ep01.ass", ASS),
        (Some("에피소드/"), "ep02.smi", SMI),
        (Some("에피소드/"), "ep02.ass", ASS),
    ])]);
    let s = setup(Some(reader)).await;
    let id = make(&s).await;
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
            ("ep01.smi", Some(Format::Smi)),
            ("ep01.ass", Some(Format::Ass)),
            ("ep02.smi", Some(Format::Smi)),
            ("ep02.ass", Some(Format::Ass)),
        ]
    );
    let names = tree(&s.area.at(&ReceiveArea::job_dir(&id)));
    assert_eq!(
        names,
        [
            "에피소드/ep01.ass",
            "에피소드/ep01.smi",
            "에피소드/ep02.ass",
            "에피소드/ep02.smi"
        ]
    );
    assert!(names.iter().all(|n| !n.ends_with(".jmk")));
}

#[tokio::test]
async fn a_place_the_viewer_lists_twice_is_one_file() {
    let reader = FakeReader::with(vec![Plan::Files(vec![
        (Some("회차"), "01.smi", SMI),
        (Some("회차"), "01.smi", b"\xEF\xBB\xBF<SAMI>\n<BODY>other"),
    ])]);
    let s = setup(Some(reader)).await;
    let id = make(&s).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(d.items[0].files.len(), 1);
    assert_eq!(
        tree(&s.area.at(&ReceiveArea::job_dir(&id))),
        ["회차/01.smi"]
    );
}

#[tokio::test]
async fn the_same_folder_and_name_in_two_images_is_two_files_the_second_numbered() {
    let other: &[u8] = b"\xEF\xBB\xBF<SAMI>\n<BODY>other";
    let reader = FakeReader::with(vec![Plan::TwoImages(
        vec![(Some("회차"), "01.smi", SMI)],
        vec![(Some("회차"), "01.smi", other)],
    )]);
    let s = setup(Some(reader)).await;
    let id = make(&s).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    let files = &d.items[0].files;
    assert_eq!(files.len(), 2);
    assert!(files[0].file_key.ends_with("img.png#회차/01.smi"));
    assert!(files[1].file_key.ends_with("more.png#회차/01.smi"));
    assert_eq!(
        tree(&s.area.at(&ReceiveArea::job_dir(&id))),
        ["회차/01 (2).smi", "회차/01.smi"]
    );
    let job_dir = s.area.at(&ReceiveArea::job_dir(&id));
    assert_eq!(std::fs::read(job_dir.join("회차/01.smi")).unwrap(), SMI);
    assert_eq!(
        std::fs::read(job_dir.join("회차/01 (2).smi")).unwrap(),
        other
    );
}

#[tokio::test]
async fn an_image_that_is_not_winpng_fails_as_no_subtitle() {
    let reader = FakeReader::with(vec![Plan::NoSubtitle]);
    let s = setup(Some(reader.clone())).await;
    let id = make(&s).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Failed);
    assert_eq!(d.row.failure, Some(FailureKind::NoSubtitle));
    assert_eq!(d.items[0].state, ItemState::Failed);
    assert_eq!(d.items[0].failure, Some(FailureKind::NoSubtitle));
    assert!(d.items[0].files.is_empty());
    assert_eq!(step(&d, StepKind::Open), Some(StepState::Failed));
    // No retry: it is not a network failure.
    assert_eq!(reader.reads.lock().unwrap().len(), 1);
    assert_eq!(*reader.released.lock().unwrap(), [id]);
    assert!(tree(&s.area.at(".tmp")).is_empty());
}

#[tokio::test]
async fn an_image_that_needs_a_key_fails_as_needs_input() {
    let reader = FakeReader::with(vec![Plan::NeedsKey]);
    let s = setup(Some(reader.clone())).await;
    let id = make(&s).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Failed);
    assert_eq!(d.row.failure, Some(FailureKind::NeedsInput));
    assert_eq!(d.items[0].failure, Some(FailureKind::NeedsInput));
    assert!(d.items[0].reason.as_deref().unwrap().contains("키"));
    assert!(d.items[0].files.is_empty());
    assert_eq!(reader.reads.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_worker_without_a_browser_leaves_the_item_waiting_for_a_source() {
    let s = setup(None).await;
    let id = make(&s).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Subtitle))
    );
    assert_eq!(d.items[0].state, ItemState::Waiting);
    assert_eq!(d.items[0].reason.as_deref(), Some(NO_READER));
    assert!(d.items[0].files.is_empty());
    // A worker start puts it back in line, so a build with a browser tries it.
    assert_eq!(s.runner.requeue_waiting_for_sources().await.unwrap(), 1);
}

#[tokio::test]
async fn a_browser_trouble_is_tried_again_and_then_fails_as_a_network_failure() {
    let trouble = || {
        Plan::Fail(Failure::new(
            FailureKind::Network,
            "서버 브라우저를 쓰지 못했어요",
        ))
    };
    // Once, then the files come.
    let reader = FakeReader::with(vec![trouble(), Plan::Files(vec![(None, "a.smi", SMI)])]);
    let s = setup(Some(reader.clone())).await;
    let id = make(&s).await;
    run(&s).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Done);
    // Each reading starts from an empty staging folder.
    assert_eq!(
        *reader.reads.lock().unwrap(),
        [(id.clone(), true), (id.clone(), true)]
    );

    // Every time: the item fails after the waits.
    let reader = FakeReader::with(vec![trouble(), trouble(), trouble()]);
    let s = setup(Some(reader.clone())).await;
    let id = make(&s).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Failed);
    assert_eq!(d.items[0].failure, Some(FailureKind::Network));
    assert_eq!(reader.reads.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn a_file_out_of_an_image_is_not_read_again_by_a_recheck() {
    let s = setup(None).await;
    let post = Url::parse(&s.server.post_url("blog", 1)).unwrap();
    let source = Source::Tistory(s.server.source());
    let key = format!("winpng:{IMAGE}#회차/01.smi");
    let answers = source.recheck(&post, std::slice::from_ref(&key)).await;
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].0, key);
    let failure = answers[0].1.as_ref().unwrap_err();
    // Neither gone nor changed in a way that asks for a new receipt: the
    // recheck reads it as unreadable, and the post is not even asked.
    assert_eq!(failure.kind, FailureKind::Changed);
    assert!(s.server.seen().is_empty());
}

#[tokio::test]
async fn a_restart_publishes_a_file_that_came_whole_under_its_folder_and_reads_it_as_received() {
    let reader = FakeReader::with(vec![Plan::Files(vec![(Some("회차"), "01.smi", SMI)])]);
    let s = setup(Some(reader)).await;
    let id = make(&s).await;
    // A worker that died after the bytes of a file came whole, before it
    // published them: the job `running`, the receipt `intended`.
    s.store.run.claim_next(950).await.unwrap().unwrap();
    let item = s.store.views.items(&id).await.unwrap()[0].id;
    s.store
        .run
        .set_item(item, ItemState::Running, None, None, 960)
        .await
        .unwrap();
    let attempt = uuid::Uuid::new_v4().to_string();
    let temp_rel = ReceiveArea::temp_dir(&attempt);
    s.store
        .run
        .file_intend(FileRow {
            id: attempt.clone(),
            item_id: item,
            file_key: format!("winpng:{IMAGE}#회차/01.smi"),
            name: "01.smi".to_owned(),
            state: FileState::Intended,
            same_as: None,
            temp_dir: Some(temp_rel.clone()),
            expected_size: None,
            size: None,
            sha256: None,
            object: None,
            path: None,
            reason: None,
            created_at: 970,
            format: None,
            failure: None,
            http_status: None,
            content_type: None,
            response_size: None,
            snapshot: None,
            kind: None,
            archive: None,
            folder: Some("회차".to_owned()),
            cleared_at: None,
            volume_of: None,
            unpacked_at: None,
            unpack_error: None,
            unpack_tries: 0,
            unpack_failure: None,
            unpack_retry_at: None,
            unchanged_asset: None,
        })
        .await
        .unwrap();
    s.store
        .run
        .file_expect(&attempt, Some(SMI.len() as u64), 971)
        .await
        .unwrap();
    let temp = s.area.at(&temp_rel);
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::write(temp.join("01.smi"), SMI).unwrap();

    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    // The recovered file, found again by its key, is the item's one file.
    let files: Vec<_> = d.items[0]
        .files
        .iter()
        .filter(|f| f.state == FileState::Done)
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, attempt);
    assert_eq!(files[0].folder.as_deref(), Some("회차"));
    assert_eq!(
        tree(&s.area.at(&ReceiveArea::job_dir(&id))),
        ["회차/01.smi"]
    );
}

/// A post whose body has a picture on the CDN and a Drive folder link.
const PICTURE_AND_FOLDER: &str = r#"<!doctype html><html><body>
<div class="tt_article_useless_p_margin contents_style"><p><a href="https://drive.google.com/drive/folders/10YFO-jkkgsybQnPpl5TVAwPdx2P0SE-y">자막 모음</a></p><p><img src="https://blog.kakaocdn.net/dna/P/Q/R/img.png?credential=x"></p></div>
</body></html>"#;

#[tokio::test]
async fn pictures_without_a_subtitle_beside_a_drive_folder_leave_the_item_waiting_for_the_folder() {
    for plan in [Plan::NoSubtitle, Plan::NoImages] {
        let reader = FakeReader::with(vec![plan]);
        let s = setup(Some(reader.clone())).await;
        s.server.post(
            "blog",
            1,
            vec![PostAnswer::Page(PICTURE_AND_FOLDER.to_owned())],
        );
        let id = make(&s).await;
        run(&s).await;

        let d = detail(&s, &id).await;
        assert_eq!(
            (d.row.state, d.row.wait),
            (JobState::Waiting, Some(Wait::Subtitle))
        );
        assert_eq!(d.items[0].state, ItemState::Waiting);
        assert!(d.items[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("Google Drive 폴더"));
        assert_eq!(reader.reads.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn a_browser_that_finds_none_of_the_images_is_a_changed_source_not_no_subtitle() {
    let reader = FakeReader::with(vec![Plan::NoImages]);
    let s = setup(Some(reader)).await;
    let id = make(&s).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Failed);
    assert_eq!(d.items[0].failure, Some(FailureKind::Changed));
}

#[tokio::test]
async fn a_file_too_deep_in_folders_fails_alone_as_not_a_file() {
    let deep: &'static str = "a/b/c/d/e/f/g/h/i";
    let reader = FakeReader::with(vec![Plan::Files(vec![
        (None, "ok.smi", SMI),
        (Some(deep), "deep.smi", SMI),
    ])]);
    let s = setup(Some(reader)).await;
    let id = make(&s).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    let files = &d.items[0].files;
    let ok = files.iter().find(|f| f.name == "ok.smi").unwrap();
    assert_eq!(ok.state, FileState::Done);
    let deep = files.iter().find(|f| f.name == "deep.smi").unwrap();
    assert_eq!(deep.state, FileState::Failed);
    assert_eq!(deep.failure, Some(FailureKind::NotAFile));
    assert!(deep.reason.as_deref().unwrap().contains("깊은"));
    assert_eq!(tree(&s.area.at(&ReceiveArea::job_dir(&id))), ["ok.smi"]);
    assert!(tree(&s.area.at(".tmp")).is_empty());
}

#[tokio::test]
async fn a_file_and_a_folder_of_one_name_do_not_meet_in_either_order() {
    // The file first: the folder of the same name is numbered.
    let reader = FakeReader::with(vec![Plan::Files(vec![
        (None, "01.smi", SMI),
        (Some("01.smi"), "02.smi", SMI),
    ])]);
    let s = setup(Some(reader)).await;
    let id = make(&s).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(
        tree(&s.area.at(&ReceiveArea::job_dir(&id))),
        ["01.smi", "01.smi (2)/02.smi"]
    );

    // The folder first: the file of the same name is numbered.
    let reader = FakeReader::with(vec![Plan::Files(vec![
        (Some("01.smi"), "02.smi", SMI),
        (None, "01.smi", SMI),
    ])]);
    let s = setup(Some(reader)).await;
    let id = make(&s).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(
        tree(&s.area.at(&ReceiveArea::job_dir(&id))),
        ["01 (2).smi", "01.smi/02.smi"]
    );
}
