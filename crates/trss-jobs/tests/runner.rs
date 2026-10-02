//! The runner with the fake source, and its restarts from records left as a
//! killed worker leaves them.

use std::{
    path::Path,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use tokio_util::sync::CancellationToken;
use trss_core::{Clock, Db};
use trss_jobs::{
    area::{self, ReceiveArea},
    store::{FileRow, JobDetail},
    Created, FileState, ItemState, JobState, JobStore, NewItem, NewJob, Runner, StepKind,
    StepState, Wait,
};
use trss_subtitles::{
    fake::{self, FakeSource},
    Sources,
};

struct Setup {
    _dir: tempfile::TempDir,
    store: JobStore,
    runner: Runner,
    area: ReceiveArea,
}

fn ticking_clock() -> Clock {
    let now = Arc::new(AtomicI64::new(1_000));
    Arc::new(move || now.fetch_add(10, Ordering::SeqCst))
}

async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let store = JobStore::new(db);
    let area = ReceiveArea::in_app_data(dir.path());
    let runner = Runner::new(
        store.clone(),
        Sources::none().with_fake(FakeSource),
        area.clone(),
        ticking_clock(),
    );
    Setup {
        _dir: dir,
        store,
        runner,
        area,
    }
}

fn post(path: &str) -> String {
    format!("https://{}{path}", fake::HOST)
}

fn job(command: &str, posts: &[(&str, &str)]) -> NewJob {
    NewJob {
        command_id: command.to_owned(),
        request: format!("{{\"posts\":{}}}", posts.len()),
        origin: "pick".to_owned(),
        work_id: None,
        season: Some(1),
        anime_no: None,
        source_id: None,
        creator: Some("제작자".to_owned()),
        items: posts
            .iter()
            .map(|(episode, path)| NewItem {
                observation_id: None,
                episode: (*episode).to_owned(),
                post_url: post(path),
                found_at: 500,
            })
            .collect(),
    }
}

async fn make(s: &Setup, command: &str, posts: &[(&str, &str)]) -> String {
    match s.store.create(job(command, posts), 900).await.unwrap() {
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

fn files_in(dir: &Path) -> Vec<String> {
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

#[tokio::test]
async fn a_job_goes_from_found_to_receive_and_leaves_its_files_in_the_receive_area() {
    let s = setup().await;
    let id = make(&s, "c1", &[("1", "/ok/ep1"), ("2", "/ok/ep2")]).await;

    // Accepted is not done.
    let before = detail(&s, &id).await;
    assert_eq!(before.row.state, JobState::Pending);
    assert_eq!(step(&before, StepKind::Found), Some(StepState::Done));
    assert_eq!(step(&before, StepKind::Receive), None);

    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(d.row.progress.done, 2);
    assert_eq!(d.row.source.as_deref(), Some(fake::HOST));
    assert!(d.row.finished_at.is_some());
    for kind in [StepKind::Found, StepKind::Open, StepKind::Receive] {
        assert_eq!(step(&d, kind), Some(StepState::Done), "{kind:?}");
    }
    assert_eq!(step(&d, StepKind::Auth), None);
    // The steps' times go forward.
    let at = |k| d.steps.iter().find(|s| s.step == k).unwrap().at;
    assert!(at(StepKind::Found) < at(StepKind::Open));
    assert!(at(StepKind::Open) < at(StepKind::Receive));

    assert_eq!(files_in(&s.area.at(&id)), ["ep1.ass", "ep2.ass"]);
    assert!(files_in(&s.area.at(".tmp")).is_empty());
    let file = &d.items[0].files[0];
    assert_eq!(file.state, FileState::Done);
    let bytes = std::fs::read(s.area.at(file.path.as_deref().unwrap())).unwrap();
    assert_eq!(bytes, fake::ass("ep1"));
    assert_eq!(file.size, Some(bytes.len() as u64));
    assert_eq!(file.expected_size, file.size);
}

#[tokio::test]
async fn a_job_with_one_of_three_failed_is_partial_with_the_reason() {
    let s = setup().await;
    let id = make(
        &s,
        "c1",
        &[("1", "/ok/a"), ("2", "/missing/b"), ("3", "/short/c")],
    )
    .await;
    let other = make(
        &s,
        "c2",
        &[("4", "/ok/d"), ("5", "/ok/e"), ("6", "/empty/f")],
    )
    .await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Partial);
    let d2 = detail(&s, &other).await;
    assert_eq!(d2.row.state, JobState::Partial);
    assert_eq!((d2.row.progress.done, d2.row.progress.failed), (2, 1));
    assert_eq!(d2.items[2].state, ItemState::Failed);
    assert_eq!(
        d2.items[2].reason.as_deref(),
        Some("게시물에 받을 파일이 없어요")
    );
    assert_eq!(d2.row.note.as_deref(), Some("3개 중 1개를 받지 못했어요"));
    assert_eq!(step(&d2, StepKind::Receive), Some(StepState::Partial));

    // A short file is not received: its bytes are gone and it is no success.
    assert_eq!(d.items[1].reason.as_deref(), Some("게시물이 없어요 (404)"));
    assert_eq!(d.items[2].state, ItemState::Failed);
    assert!(d.items[2].reason.as_deref().unwrap().contains("크기"));
    assert_eq!(d.items[2].files[0].state, FileState::Failed);
    assert_eq!(files_in(&s.area.at(&id)), ["a.ass"]);
    assert!(files_in(&s.area.at(".tmp")).is_empty());
}

#[tokio::test]
async fn a_file_two_posts_share_is_received_once() {
    let s = setup().await;
    let id = make(&s, "c1", &[("11", "/shared/s/11"), ("12", "/shared/s/12")]).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(files_in(&s.area.at(&id)), ["s.ass"]);
    let (first, second) = (&d.items[0].files[0], &d.items[1].files[0]);
    assert_eq!(first.same_as, None);
    assert_eq!(second.same_as.as_deref(), Some(first.id.as_str()));
    assert_eq!(second.path, first.path);
    assert_eq!(second.temp_dir, None);
}

#[tokio::test]
async fn a_site_check_and_an_unknown_site_wait_for_different_things() {
    let s = setup().await;
    let auth = make(&s, "c1", &[("1", "/auth/1"), ("2", "/ok/2")]).await;
    let unknown = s
        .store
        .create(
            NewJob {
                items: vec![NewItem {
                    observation_id: None,
                    episode: "1".into(),
                    post_url: "https://somebody.tistory.com/31".into(),
                    found_at: 500,
                }],
                ..job("c2", &[])
            },
            900,
        )
        .await
        .unwrap();
    let Created::Created(unknown) = unknown else {
        panic!()
    };
    run(&s).await;

    let d = detail(&s, &auth).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Auth))
    );
    assert_eq!(
        d.row.note.as_deref(),
        Some("사이트 확인을 기다려요 (CAPTCHA)")
    );
    assert_eq!(step(&d, StepKind::Auth), Some(StepState::Waiting));
    assert_eq!(d.items[1].state, ItemState::Done);
    assert_eq!(s.store.auth_waits().await.unwrap().len(), 1);

    let d = detail(&s, &unknown).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Subtitle))
    );
    assert_eq!(d.row.source.as_deref(), Some("somebody.tistory.com"));

    // A start puts the source waits back in line, not the check.
    assert_eq!(s.runner.requeue_waiting_for_sources().await.unwrap(), 1);
    assert_eq!(detail(&s, &unknown).await.row.state, JobState::Pending);
    assert_eq!(detail(&s, &auth).await.row.state, JobState::Waiting);
}

#[tokio::test]
async fn the_same_browser_id_finds_its_job_and_another_request_with_it_is_refused() {
    let s = setup().await;
    let id = make(&s, "c1", &[("1", "/ok/1")]).await;
    let again = s
        .store
        .create(job("c1", &[("1", "/ok/1")]), 901)
        .await
        .unwrap();
    assert_eq!(again, Created::Existing(id.clone()));
    let other = s
        .store
        .create(job("c1", &[("1", "/ok/1"), ("2", "/ok/2")]), 902)
        .await
        .unwrap();
    assert_eq!(other, Created::Mismatch(id));
    assert_eq!(s.store.open_jobs().await.unwrap().len(), 1);
}

#[tokio::test]
async fn done_jobs_come_newest_first_a_page_at_a_time() {
    let s = setup().await;
    for i in 0..23 {
        make(&s, &format!("c{i}"), &[("1", &format!("/ok/{i}"))]).await;
    }
    run(&s).await;

    let first = s.store.done_page(None, 5).await.unwrap();
    assert_eq!((first.items.len(), first.total), (5, 23));
    let mut seen: Vec<i64> = first.items.iter().map(|j| j.seq).collect();
    let mut next = first.next;
    while let Some(after) = next {
        let page = s.store.done_page(Some(after), 5).await.unwrap();
        seen.extend(page.items.iter().map(|j| j.seq));
        next = page.next;
    }
    let mut expected: Vec<i64> = seen.clone();
    expected.sort_by(|a, b| b.cmp(a));
    assert_eq!(seen, expected);
    assert_eq!(seen.len(), 23);
}

#[tokio::test]
async fn a_shutdown_in_the_middle_of_a_file_leaves_the_job_running_and_the_next_start_ends_it() {
    let s = setup().await;
    let id = make(&s, "c1", &[("1", "/ok/a?delay_ms=20")]).await;

    let cancel = CancellationToken::new();
    let runner = s.runner.clone();
    let task = tokio::spawn({
        let cancel = cancel.clone();
        async move { runner.run_ready(&cancel).await }
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    cancel.cancel();
    task.await.unwrap().unwrap();

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Running);
    assert_eq!(d.items[0].files[0].state, FileState::Abandoned);
    assert!(files_in(&s.area.at(".tmp")).is_empty());

    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert!(d
        .events
        .iter()
        .any(|e| e.message == "멈췄던 작업을 이어가요"));
    assert_eq!(files_in(&s.area.at(&id)), ["a.ass"]);
}

// ---------------------------------------------------------------------------
// Restarts from records a killed worker leaves

/// A job made, claimed and left `running` with one item `running`, as a
/// worker that died while receiving its file leaves it.
async fn killed(s: &Setup, path: &str) -> (String, i64) {
    let id = make(s, "c1", &[("1", path)]).await;
    s.store.claim_next(950).await.unwrap().unwrap();
    let item = s.store.items(&id).await.unwrap()[0].id;
    s.store
        .set_item(item, ItemState::Running, None, None, 960)
        .await
        .unwrap();
    (id, item)
}

/// Records an attempt `intended` (with the announced length) and writes
/// `bytes` (if any) as its temporary file.
async fn intended(
    s: &Setup,
    item: i64,
    key: &str,
    name: &str,
    expected: Option<u64>,
    bytes: Option<&[u8]>,
) -> FileRow {
    let id = uuid::Uuid::new_v4().to_string();
    let row = FileRow {
        id: id.clone(),
        item_id: item,
        file_key: key.to_owned(),
        name: name.to_owned(),
        state: FileState::Intended,
        same_as: None,
        temp_dir: Some(ReceiveArea::temp_dir(&id)),
        expected_size: None,
        size: None,
        sha256: None,
        object: None,
        path: None,
        reason: None,
        created_at: 970,
    };
    s.store.file_intend(row.clone()).await.unwrap();
    s.store.file_expect(&id, expected, 971).await.unwrap();
    if let Some(bytes) = bytes {
        let dir = s.area.at(row.temp_dir.as_deref().unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    row
}

/// Takes an `intended` attempt with a whole temporary file to `fetched`,
/// planning `path`.
async fn fetched(s: &Setup, row: &FileRow, path: &str) {
    let temp = s.area.at(row.temp_dir.as_deref().unwrap()).join(&row.name);
    let (size, sha, object) = area::read_facts(&temp).unwrap();
    s.store
        .file_fetched(&row.id, size, sha, object, path.to_owned(), 972)
        .await
        .unwrap();
}

fn object(path: &Path) -> String {
    area::object_of(&std::fs::metadata(path).unwrap())
}

#[tokio::test]
async fn an_intent_with_no_bytes_is_abandoned_and_received_anew() {
    let s = setup().await;
    let (id, item) = killed(&s, "/ok/a").await;
    let row = intended(&s, item, "ok/a", "a.ass", None, None).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    let states: Vec<_> = d.items[0]
        .files
        .iter()
        .map(|f| (f.id.clone(), f.state))
        .collect();
    assert!(states.contains(&(row.id, FileState::Abandoned)));
    assert_eq!(
        states.iter().filter(|(_, s)| *s == FileState::Done).count(),
        1
    );
}

#[tokio::test]
async fn bytes_known_to_stop_short_are_received_again_and_whole_ones_are_published_as_they_are() {
    let s = setup().await;
    let whole = fake::ass("a");

    let (id, item) = killed(&s, "/ok/a").await;
    let row = intended(
        &s,
        item,
        "ok/a",
        "a.ass",
        Some(whole.len() as u64),
        Some(&whole[..100]),
    )
    .await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(d.items[0].files[0].state, FileState::Abandoned);
    assert_eq!(d.items[0].files.len(), 2);
    assert!(!s.area.at(row.temp_dir.as_deref().unwrap()).exists());

    // A temporary file of the announced length is the file: published, not
    // received a second time (the published file is the temporary file).
    let s = setup().await;
    let (id, item) = killed(&s, "/ok/a").await;
    let row = intended(
        &s,
        item,
        "ok/a",
        "a.ass",
        Some(whole.len() as u64),
        Some(&whole),
    )
    .await;
    let temp_object = object(&s.area.at(row.temp_dir.as_deref().unwrap()).join("a.ass"));
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(d.items[0].files.len(), 1);
    assert_eq!(d.items[0].files[0].state, FileState::Done);
    assert_eq!(object(&s.area.at(&format!("{id}/a.ass"))), temp_object);
}

#[tokio::test]
async fn bytes_with_no_announced_length_or_more_than_it_are_held_not_received_again() {
    for expected in [None, Some(10)] {
        let s = setup().await;
        let (id, item) = killed(&s, "/ok/a").await;
        let row = intended(&s, item, "ok/a", "a.ass", expected, Some(&fake::ass("a"))).await;
        run(&s).await;

        let d = detail(&s, &id).await;
        assert_eq!(d.row.state, JobState::Held, "{expected:?}");
        assert_eq!(d.items[0].state, ItemState::Held);
        assert_eq!(d.items[0].files.len(), 1);
        assert_eq!(d.items[0].files[0].state, FileState::Held);
        // Its bytes stay where they were.
        assert!(s
            .area
            .at(row.temp_dir.as_deref().unwrap())
            .join("a.ass")
            .exists());
        assert_eq!(step(&d, StepKind::Receive), Some(StepState::Waiting));

        // A held job is not taken up again by itself.
        run(&s).await;
        assert_eq!(detail(&s, &id).await.items[0].files.len(), 1);
    }
}

#[tokio::test]
async fn a_rename_done_before_its_record_is_found_by_its_object_and_bytes() {
    let s = setup().await;
    let (id, item) = killed(&s, "/ok/a").await;
    let row = intended(&s, item, "ok/a", "a.ass", None, Some(&fake::ass("a"))).await;
    let path = format!("{id}/a.ass");
    fetched(&s, &row, &path).await;
    // The rename happened; the worker died before writing `done`.
    std::fs::create_dir_all(s.area.at(&id)).unwrap();
    std::fs::rename(
        s.area.at(row.temp_dir.as_deref().unwrap()).join("a.ass"),
        s.area.at(&path),
    )
    .unwrap();
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(d.items[0].files.len(), 1);
    assert_eq!(d.items[0].files[0].state, FileState::Done);
}

#[tokio::test]
async fn a_fetched_file_is_published_but_a_copy_with_the_same_bytes_at_its_path_is_held() {
    // Fetched, the temporary file still there: published.
    let s = setup().await;
    let (id, item) = killed(&s, "/ok/a").await;
    let row = intended(&s, item, "ok/a", "a.ass", None, Some(&fake::ass("a"))).await;
    fetched(&s, &row, &format!("{id}/a.ass")).await;
    run(&s).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Done);

    // Fetched, the temporary file gone, the same bytes at the path but as
    // another file: nothing says this job put them there.
    let s = setup().await;
    let (id, item) = killed(&s, "/ok/a").await;
    let row = intended(&s, item, "ok/a", "a.ass", None, Some(&fake::ass("a"))).await;
    let path = format!("{id}/a.ass");
    fetched(&s, &row, &path).await;
    std::fs::remove_dir_all(s.area.at(row.temp_dir.as_deref().unwrap())).unwrap();
    std::fs::create_dir_all(s.area.at(&id)).unwrap();
    std::fs::write(s.area.at(&path), fake::ass("a")).unwrap();
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Held);
    assert_eq!(d.items[0].files.len(), 1);
}

#[tokio::test]
async fn a_done_file_is_reused_after_its_bytes_are_checked_and_held_when_they_changed() {
    for changed in [false, true] {
        let s = setup().await;
        let id = make(&s, "c1", &[("1", "/ok/a")]).await;
        run(&s).await;
        let path = s.area.at(&format!("{id}/a.ass"));
        if changed {
            std::fs::write(&path, b"other bytes").unwrap();
        }
        // As if the worker died after the file was done, before the item was.
        let item = s.store.items(&id).await.unwrap()[0].id;
        s.store
            .set_item(item, ItemState::Running, None, None, 2_000)
            .await
            .unwrap();
        s.store
            .settle(&id, JobState::Running, None, None, 2_000)
            .await
            .unwrap();
        run(&s).await;

        let d = detail(&s, &id).await;
        assert_eq!(d.items[0].files.len(), 1, "changed: {changed}");
        match changed {
            false => {
                assert_eq!(d.row.state, JobState::Done);
                assert!(d
                    .events
                    .iter()
                    .any(|e| e.message.contains("다시 받지 않았어요")));
            }
            true => {
                assert_eq!(d.row.state, JobState::Held);
                assert_eq!(d.items[0].files[0].state, FileState::Held);
                assert_eq!(std::fs::read(&path).unwrap(), b"other bytes");
            }
        }
    }
}

#[tokio::test]
async fn an_unfinished_receipt_is_settled_from_the_disk_even_when_the_post_no_longer_opens() {
    // The worker died after publishing; the post is gone when it comes back.
    let s = setup().await;
    let (id, item) = killed(&s, "/missing/a").await;
    let row = intended(&s, item, "ok/a", "a.ass", None, Some(&fake::ass("a"))).await;
    let path = format!("{id}/a.ass");
    fetched(&s, &row, &path).await;
    std::fs::create_dir_all(s.area.at(&id)).unwrap();
    std::fs::rename(
        s.area.at(row.temp_dir.as_deref().unwrap()).join("a.ass"),
        s.area.at(&path),
    )
    .unwrap();
    run(&s).await;

    let d = detail(&s, &id).await;
    // The receipt is done, not left `fetched`; the item fails on the post.
    assert_eq!(d.items[0].files[0].state, FileState::Done);
    assert_eq!(d.items[0].state, ItemState::Failed);

    // One it cannot vouch for holds the item before the post is read.
    let s = setup().await;
    let (id, item) = killed(&s, "/missing/a").await;
    intended(&s, item, "ok/a", "a.ass", None, Some(&fake::ass("a"))).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.items[0].state, ItemState::Held);
    assert_eq!(d.row.state, JobState::Held);
}

#[tokio::test]
async fn an_empty_temporary_file_is_no_bytes_even_when_nothing_was_announced() {
    let s = setup().await;
    let (id, item) = killed(&s, "/ok/a").await;
    let row = intended(&s, item, "ok/a", "a.ass", Some(0), Some(b"")).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    let first = d.items[0].files.iter().find(|f| f.id == row.id).unwrap();
    assert_eq!(first.state, FileState::Abandoned);
    assert_eq!(
        std::fs::read(s.area.at(&format!("{id}/a.ass"))).unwrap(),
        fake::ass("a")
    );
}

#[tokio::test]
async fn an_item_with_an_abandoned_attempt_still_records_the_shared_file() {
    let s = setup().await;
    let id = make(&s, "c1", &[("11", "/shared/s/11"), ("12", "/shared/s/12")]).await;
    s.store.claim_next(950).await.unwrap().unwrap();
    let items = s.store.items(&id).await.unwrap();
    // Item 12 had started the shared file and left no bytes; item 11 is
    // still to run.
    s.store
        .set_item(items[1].id, ItemState::Running, None, None, 960)
        .await
        .unwrap();
    intended(&s, items[1].id, "shared/s", "s.ass", None, None).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    let done = |i: usize| {
        d.items[i]
            .files
            .iter()
            .filter(|f| f.state == FileState::Done)
            .count()
    };
    assert_eq!((done(0), done(1)), (1, 1));
    assert_eq!(files_in(&s.area.at(&id)), ["s.ass"]);
}

#[tokio::test]
async fn a_job_started_too_many_times_is_held_instead_of_blocking_the_line() {
    let s = setup().await;
    let stuck = make(&s, "c1", &[("1", "/ok/a")]).await;
    let next = make(&s, "c2", &[("1", "/ok/b")]).await;
    // Each start died before it ended, in the middle of the first item.
    let item = detail(&s, &stuck).await.items[0].id;
    for _ in 0..trss_jobs::runner::MAX_STARTS {
        s.store.claim_next(950).await.unwrap().unwrap();
        s.store
            .set_item(item, ItemState::Running, None, None, 950)
            .await
            .unwrap();
        s.store
            .begin_step(&stuck, StepKind::Open, 950)
            .await
            .unwrap();
    }
    run(&s).await;

    let d = detail(&s, &stuck).await;
    assert_eq!(d.row.state, JobState::Held);
    assert_eq!(d.items[0].state, ItemState::Held);
    assert_eq!(d.items[0].files.len(), 0);
    assert_eq!(step(&d, StepKind::Open), Some(StepState::Waiting));
    assert_eq!(detail(&s, &next).await.row.state, JobState::Done);
}

#[tokio::test]
async fn a_job_that_ends_each_run_waiting_for_a_source_is_never_held_for_its_starts() {
    let s = setup().await;
    let waits = s
        .store
        .create(
            NewJob {
                items: vec![NewItem {
                    observation_id: None,
                    episode: "1".into(),
                    post_url: "https://somebody.tistory.com/31".into(),
                    found_at: 500,
                }],
                ..job("c1", &[])
            },
            900,
        )
        .await
        .unwrap();
    let Created::Created(waits) = waits else {
        panic!()
    };
    // Each worker start puts it back in line, and each run ends waiting.
    for _ in 0..trss_jobs::runner::MAX_STARTS * 2 {
        s.runner.requeue_waiting_for_sources().await.unwrap();
        run(&s).await;
    }

    let d = detail(&s, &waits).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Subtitle))
    );
}
