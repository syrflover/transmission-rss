//! The runner with the fake source, and its restarts from what a killed
//! worker leaves.

use crate::{
    world::{cut_and_restart, Base},
    Handles,
};
use std::{path::Path, time::Duration};

use tokio_util::sync::CancellationToken;
use trss_jobs::{
    area::ReceiveArea,
    store::{FileRow, JobDetail},
    Created, FileState, ItemState, JobState, NewItem, NewJob, Runner, StepKind, StepState, Wait,
};
use trss_subtitles::{
    fake::{self, FakeSource},
    Sources,
};

struct Setup {
    _dir: tempfile::TempDir,
    store: Handles,
    runner: Runner,
    area: ReceiveArea,
}

async fn setup() -> Setup {
    let base = Base::new().await;
    let runner = base.runner(Sources::none().with_fake(FakeSource));
    Setup {
        _dir: base.dir,
        store: base.store,
        runner,
        area: base.area,
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
        movie: None,
        anime_no: None,
        source_id: None,
        creator: Some("제작자".to_owned()),
        revision_of: None,
        revises_attributed: false,
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
    match s
        .store
        .requests
        .create(job(command, posts), 900)
        .await
        .unwrap()
    {
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
    assert!(files_in(&s.area.at(".partial")).is_empty());
    let file = &d.items[0].files[0];
    assert_eq!(file.state, FileState::Done);
    let bytes = std::fs::read(s.area.at(file.path.as_deref().unwrap())).unwrap();
    assert_eq!(bytes, fake::ass("ep1"));
    assert_eq!(file.size, Some(bytes.len() as u64));
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
    assert!(files_in(&s.area.at(".partial")).is_empty());
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
}

#[tokio::test]
async fn a_site_check_and_an_unknown_site_wait_for_different_things() {
    let s = setup().await;
    let auth = make(&s, "c1", &[("1", "/auth/1"), ("2", "/ok/2")]).await;
    let unknown = s
        .store
        .requests
        .create(
            NewJob {
                revision_of: None,
                revises_attributed: false,
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
    assert_eq!(s.store.views.auth_waits().await.unwrap().len(), 1);

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
        .requests
        .create(job("c1", &[("1", "/ok/1")]), 901)
        .await
        .unwrap();
    assert_eq!(again, Created::Existing(id.clone()));
    let other = s
        .store
        .requests
        .create(job("c1", &[("1", "/ok/1"), ("2", "/ok/2")]), 902)
        .await
        .unwrap();
    assert_eq!(other, Created::Mismatch(id));
    assert_eq!(s.store.views.open_jobs().await.unwrap().len(), 1);
}

#[tokio::test]
async fn done_jobs_come_newest_first_a_page_at_a_time() {
    let s = setup().await;
    for i in 0..23 {
        make(&s, &format!("c{i}"), &[("1", &format!("/ok/{i}"))]).await;
    }
    run(&s).await;

    let first = s.store.views.done_page(None, 5).await.unwrap();
    assert_eq!((first.items.len(), first.total), (5, 23));
    let mut seen: Vec<i64> = first.items.iter().map(|j| j.seq).collect();
    let mut next = first.next;
    while let Some(after) = next {
        let page = s.store.views.done_page(Some(after), 5).await.unwrap();
        seen.extend(page.items.iter().map(|j| j.seq));
        next = page.next;
    }
    let mut expected: Vec<i64> = seen.clone();
    expected.sort_by(|a, b| b.cmp(a));
    assert_eq!(seen, expected);
    assert_eq!(seen.len(), 23);
}

#[tokio::test]
async fn a_shutdown_in_the_middle_of_a_file_leaves_the_job_running_and_the_next_start_interrupts_it_until_a_restart_ends_it(
) {
    let s = setup().await;
    let id = make(&s, "c1", &[("1", "/ok/a?delay_ms=50")]).await;

    let cancel = CancellationToken::new();
    let runner = s.runner.clone();
    let task = tokio::spawn({
        let cancel = cancel.clone();
        async move { runner.run_ready(&cancel).await }
    });
    // Stop once the file has begun, however slow the machine is: it takes
    // about 30 × 50ms to come.
    let waiting = tokio::time::Instant::now();
    while detail(&s, &id).await.items[0].files.is_empty() {
        assert!(waiting.elapsed() < Duration::from_secs(10), "no file began");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    cancel.cancel();
    task.await.unwrap().unwrap();

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Running);
    assert_eq!(d.items[0].files[0].state, FileState::Abandoned);
    assert!(files_in(&s.area.at(".partial")).is_empty());

    // The next start only marks it; nothing runs it before a person restarts it.
    cut_and_restart(&s.store, &id).await;
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
// Restarts from what a killed worker leaves

/// A job made, claimed and left `running` with one item `running`, as a
/// worker that died while receiving its file leaves it.
async fn killed(s: &Setup, path: &str) -> (String, i64) {
    let id = make(s, "c1", &[("1", path)]).await;
    s.store.run.claim_next(950).await.unwrap().unwrap();
    let item = s.store.views.items(&id).await.unwrap()[0].id;
    s.store
        .run
        .set_item(item, ItemState::Running, None, None, 960)
        .await
        .unwrap();
    // The process died here; the next one found the job `interrupted`, and a
    // person restarted it.
    cut_and_restart(&s.store, &id).await;
    (id, item)
}

/// Records an attempt `intended`, as a run does before it fetches, and
/// writes `bytes` (if any) in its folder of `.partial/`, as the run does
/// while they come.
async fn intended(s: &Setup, item: i64, key: &str, name: &str, bytes: Option<&[u8]>) -> FileRow {
    let id = uuid::Uuid::new_v4().to_string();
    let row = FileRow {
        id: id.clone(),
        item_id: item,
        file_key: key.to_owned(),
        name: name.to_owned(),
        state: FileState::Intended,
        same_as: None,
        size: None,
        sha256: None,
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
        folder: None,
        cleared_at: None,
        volume_of: None,
        unpacked_at: None,
        unpack_error: None,
        unpack_tries: 0,
        unpack_failure: None,
        unpack_retry_at: None,
        unchanged_asset: None,
    };
    s.store.run.file_intend(row.clone()).await.unwrap();
    if let Some(bytes) = bytes {
        let dir = s.area.at(&ReceiveArea::partial_dir(&id));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    row
}

async fn sql(s: &Setup, statement: String) {
    s.store
        .run
        .db()
        .run(move |c| {
            c.execute_batch(&statement)
                .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
}

#[cfg(unix)]
fn inode(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).unwrap().ino()
}

#[tokio::test]
async fn a_cut_attempt_is_forgotten_and_its_file_received_anew_whatever_bytes_it_left() {
    let whole = fake::ass("a");
    let page: &[u8] = b"<!DOCTYPE html><html><body>expired</body></html>";
    let longer = [whole.as_slice(), b"more"].concat();
    // No bytes, an empty file, bytes cut short, every byte, more bytes than
    // the file has, and bytes that are no file: none of them is looked at.
    let left: [Option<&[u8]>; 6] = [
        None,
        Some(b""),
        Some(&whole[..100]),
        Some(&whole),
        Some(&longer),
        Some(page),
    ];
    for bytes in left {
        let s = setup().await;
        let (id, item) = killed(&s, "/ok/a").await;
        let row = intended(&s, item, "ok/a", "a.ass", bytes).await;
        run(&s).await;

        let what = bytes.map(<[u8]>::len);
        let d = detail(&s, &id).await;
        assert_eq!(d.row.state, JobState::Done, "{what:?}");
        // The cut attempt's receipt is gone and one new attempt received
        // the file.
        assert_eq!(d.items[0].files.len(), 1, "{what:?}");
        let file = &d.items[0].files[0];
        assert_ne!(file.id, row.id);
        assert_eq!(file.state, FileState::Done);
        assert_eq!(
            std::fs::read(s.area.at(&format!("{id}/a.ass"))).unwrap(),
            whole
        );
        assert_eq!(files_in(&s.area.at(&id)), ["a.ass"]);
        // Its bytes went with it.
        assert!(files_in(&s.area.at(".partial")).is_empty(), "{what:?}");
        assert!(d
            .events
            .iter()
            .any(|e| e.message == "끊긴 받기를 처음부터 다시 해요"
                && e.detail.as_deref() == Some("받던 파일 1개")));
    }
}

/// Leaves the receipt of `job`'s file `key`, which a run published, as a run
/// cut between the publishing rename and its `done` leaves it: `intended`,
/// naming no path.
async fn cut_before_done(s: &Setup, job: &str, key: &str) {
    sql(
        s,
        format!(
            "UPDATE subtitle_job_files
                SET state = 'intended', path = NULL, size = NULL, sha256 = NULL, format = NULL
              WHERE job_id = '{job}' AND file_key = '{key}';
             UPDATE subtitle_job_items SET state = 'running'
              WHERE id = (SELECT item_id FROM subtitle_job_files
                           WHERE job_id = '{job}' AND file_key = '{key}');
             UPDATE subtitle_jobs SET state = 'running', stage = 'receive', finished_at = NULL
              WHERE id = '{job}';"
        ),
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_file_published_before_its_record_is_removed_and_received_anew_under_its_name() {
    let s = setup().await;
    let id = make(&s, "c1", &[("1", "/ok/a"), ("2", "/ok/b")]).await;
    run(&s).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Done);
    // The second item's file was renamed into place, and the worker died
    // before its `done`. An unpacking's folder is there too.
    cut_before_done(&s, &id, "ok/b").await;
    let unpacked = s.area.at(&format!("{id}/.unpack/x/1"));
    std::fs::create_dir_all(unpacked.parent().unwrap()).unwrap();
    std::fs::write(&unpacked, b"member").unwrap();
    let a = s.area.at(&format!("{id}/a.ass"));
    let (a_before, b_before) = (inode(&a), inode(&s.area.at(&format!("{id}/b.ass"))));
    cut_and_restart(&s.store, &id).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    // The file no receipt named was removed and received again at its
    // name: no `b (2).ass` beside it.
    assert_eq!(files_in(&s.area.at(&id)), [".unpack", "a.ass", "b.ass"]);
    let b = &d.items[1].files;
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].state, FileState::Done);
    assert_eq!(b[0].path.as_deref(), Some(format!("{id}/b.ass").as_str()));
    assert_eq!(
        std::fs::read(s.area.at(&format!("{id}/b.ass"))).unwrap(),
        fake::ass("b")
    );
    assert_ne!(inode(&s.area.at(&format!("{id}/b.ass"))), b_before);
    // What receipts name, and the unpacking's folder, stay as they were.
    assert_eq!(inode(&a), a_before);
    assert_eq!(std::fs::read(&unpacked).unwrap(), b"member");
    assert!(d
        .events
        .iter()
        .any(|e| e.message == "끊긴 받기를 처음부터 다시 해요"
            && e.detail.as_deref() == Some("받던 파일 1개 · 기록에 없는 파일 1개 지움")));
}

#[tokio::test]
async fn what_a_cut_run_left_is_cleared_even_when_the_post_no_longer_opens() {
    let s = setup().await;
    let (id, item) = killed(&s, "/missing/a").await;
    let row = intended(&s, item, "missing/a", "a.ass", Some(&fake::ass("a"))).await;
    // Renamed into place before the worker died.
    std::fs::create_dir_all(s.area.at(&id)).unwrap();
    std::fs::rename(
        s.area.at(&ReceiveArea::partial_dir(&row.id)).join("a.ass"),
        s.area.at(&format!("{id}/a.ass")),
    )
    .unwrap();
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.items[0].state, ItemState::Failed);
    assert!(d.items[0].files.is_empty());
    assert!(files_in(&s.area.at(&id)).is_empty());
    assert!(files_in(&s.area.at(".partial")).is_empty());
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
        let item = s.store.views.items(&id).await.unwrap()[0].id;
        s.store
            .run
            .set_item(item, ItemState::Running, None, None, 2_000)
            .await
            .unwrap();
        s.store
            .run
            .settle(&id, JobState::Running, None, None, 2_000)
            .await
            .unwrap();
        cut_and_restart(&s.store, &id).await;
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
                assert_eq!(d.items[0].state, ItemState::Held);
                assert_eq!(d.items[0].files[0].state, FileState::Held);
                assert_eq!(std::fs::read(&path).unwrap(), b"other bytes");
                // A held job is not taken up again by itself.
                run(&s).await;
                let d = detail(&s, &id).await;
                assert_eq!(d.row.state, JobState::Held);
                assert_eq!(d.items[0].files.len(), 1);
                assert_eq!(std::fs::read(&path).unwrap(), b"other bytes");
            }
        }
    }
}

#[tokio::test]
async fn an_item_with_a_cut_attempt_still_records_the_shared_file() {
    let s = setup().await;
    let id = make(&s, "c1", &[("11", "/shared/s/11"), ("12", "/shared/s/12")]).await;
    s.store.run.claim_next(950).await.unwrap().unwrap();
    let items = s.store.views.items(&id).await.unwrap();
    // Item 12 had started the shared file and left no bytes; item 11 is
    // still to run.
    s.store
        .run
        .set_item(items[1].id, ItemState::Running, None, None, 960)
        .await
        .unwrap();
    intended(&s, items[1].id, "shared/s", "s.ass", None).await;
    cut_and_restart(&s.store, &id).await;
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
        s.store.run.claim_next(950).await.unwrap().unwrap();
        s.store
            .run
            .set_item(item, ItemState::Running, None, None, 950)
            .await
            .unwrap();
        s.store
            .run
            .begin_step(&stuck, StepKind::Open, 950)
            .await
            .unwrap();
        cut_and_restart(&s.store, &stuck).await;
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
        .requests
        .create(
            NewJob {
                revision_of: None,
                revises_attributed: false,
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

#[tokio::test]
async fn a_received_file_records_its_format_and_a_short_file_fails_as_not_a_file() {
    let s = setup().await;
    let id = make(&s, "c1", &[("1", "/ok/a"), ("2", "/short/b")]).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.items[0].files[0].format, Some(trss_jobs::Format::Ass));
    assert_eq!(d.items[1].failure, Some(trss_jobs::FailureKind::NotAFile));
    assert_eq!(d.row.failure, Some(trss_jobs::FailureKind::NotAFile));
    let short = &d.items[1].files[0];
    assert_eq!(short.failure, Some(trss_jobs::FailureKind::NotAFile));
    assert_eq!(short.response_size, Some(fake::ass("b").len() as u64 - 16));
}

#[tokio::test]
async fn a_jobs_failure_class_is_its_first_failed_items_even_when_that_has_none() {
    let s = setup().await;
    let id = make(&s, "c1", &[("1", "/ok/a"), ("2", "/ok/b")]).await;
    let items = s.store.views.items(&id).await.unwrap();
    s.store
        .run
        .fail_item(items[0].id, "디스크가 가득 찼어요".into(), None, 960)
        .await
        .unwrap();
    s.store
        .run
        .fail_item(
            items[1].id,
            "받은 파일이 비어 있어요".into(),
            Some(trss_jobs::FailureKind::NotAFile),
            961,
        )
        .await
        .unwrap();
    assert_eq!(detail(&s, &id).await.row.failure, None);

    let s = setup().await;
    let id = make(&s, "c1", &[("1", "/ok/a"), ("2", "/ok/b")]).await;
    let items = s.store.views.items(&id).await.unwrap();
    s.store
        .run
        .fail_item(
            items[1].id,
            "받은 파일이 비어 있어요".into(),
            Some(trss_jobs::FailureKind::NotAFile),
            961,
        )
        .await
        .unwrap();
    assert_eq!(
        detail(&s, &id).await.row.failure,
        Some(trss_jobs::FailureKind::NotAFile)
    );
}
