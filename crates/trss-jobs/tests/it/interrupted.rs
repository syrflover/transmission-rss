//! A job the process left `running` (`docs/specs/jobs.md`, 체크포인트와 중단
//! 복구; `docs/adr/0020-interrupted-jobs-wait-for-a-restart.md`): the start of
//! the process makes it `interrupted`, a person's restart puts it back to
//! `running` from the step it was cut in, and the readers that tell a job's
//! state judge it as the spec says. The to-do count, the library's apply
//! status, the cleanup and the other readers have their tests in the files of
//! the crates and tests that own them (`todo.rs`, `cleanup.rs`, `choose.rs`,
//! `follow.rs`, `place.rs`).

use crate::world::{cut_and_restart, Base};

use tokio_util::sync::CancellationToken;
use trss_jobs::{
    store::{AskedFinish, INTERRUPTED, RESTARTED},
    Created, FileState, ItemState, JobState, NewItem, NewJob, Restarted, StepKind, Wait,
};
use trss_subtitles::{
    fake::{self, FakeSource},
    Sources,
};

fn job(command: &str, origin: &str, path: &str) -> NewJob {
    NewJob {
        command_id: command.to_owned(),
        request: format!("{{\"c\":\"{command}\"}}"),
        origin: origin.to_owned(),
        work_id: None,
        season: Some(1),
        movie: None,
        anime_no: None,
        source_id: None,
        creator: Some("제작자".to_owned()),
        revision_of: None,
        revises_attributed: false,
        items: vec![NewItem {
            observation_id: None,
            episode: "1".to_owned(),
            post_url: format!("https://{}{path}", fake::HOST),
            found_at: 500,
        }],
    }
}

async fn make(base: &Base, command: &str, origin: &str) -> String {
    match base
        .store
        .requests
        .create(job(command, origin, "/ok/a"), 900)
        .await
        .unwrap()
    {
        Created::Created(id) => id,
        other => panic!("created: {other:?}"),
    }
}

async fn sql(base: &Base, statement: String) {
    base.db
        .run(move |c| {
            c.execute_batch(&statement)
                .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
}

async fn detail(base: &Base, id: &str) -> trss_jobs::store::JobDetail {
    base.store.views.detail(id).await.unwrap().unwrap()
}

#[tokio::test]
async fn a_job_left_running_in_any_step_is_interrupted_there_and_one_that_waits_is_not() {
    let base = Base::new().await;
    // The steps a job is cut in: 받기, 보관, 적용, 교체 승인, 배치 확인, and a
    // relocation (재배치) in its apply step.
    let cut = [
        ("receive", "pick"),
        ("store", "pick"),
        ("apply", "pick"),
        ("approval", "pick"),
        ("placement", "upload"),
        ("apply", "relocate"),
    ];
    let mut running = Vec::new();
    for (n, (stage, origin)) in cut.iter().enumerate() {
        let id = make(&base, &format!("r{n}"), origin).await;
        sql(
            &base,
            format!(
                "UPDATE subtitle_jobs SET state = 'running', stage = '{stage}', attempts = 2
                 WHERE id = '{id}'"
            ),
        )
        .await;
        running.push((id, *stage));
    }
    // A job cut before it began a step has no stage to keep.
    let unstarted = make(&base, "r-none", "pick").await;
    sql(
        &base,
        format!("UPDATE subtitle_jobs SET state = 'running' WHERE id = '{unstarted}'"),
    )
    .await;

    // What does not run is left as it is: every wait, held, in line, done.
    let mut kept = Vec::new();
    for (n, (state, wait)) in [
        ("waiting", Some("auth")),
        ("waiting", Some("subtitle")),
        ("waiting", Some("placement")),
        ("waiting", Some("approval")),
        ("waiting", Some("video")),
        ("held", None),
        ("pending", None),
        ("failed", None),
        ("done", None),
    ]
    .into_iter()
    .enumerate()
    {
        let id = make(&base, &format!("k{n}"), "pick").await;
        let wait = wait.map_or("NULL".to_owned(), |w| format!("'{w}'"));
        sql(
            &base,
            format!(
                "UPDATE subtitle_jobs SET state = '{state}', wait = {wait}, stage = 'apply',
                        state_at = 5, updated_at = 5 WHERE id = '{id}'"
            ),
        )
        .await;
        kept.push(id);
    }
    let before = rows_of(&base, &kept).await;

    // The start of the process, before any job is claimed.
    let n = base.store.run.interrupt_running(7_000).await.unwrap();
    assert_eq!(n, running.len() + 1);

    for (id, stage) in &running {
        let d = detail(&base, id).await;
        assert_eq!(d.row.state, JobState::Interrupted, "{stage}");
        assert_eq!(d.row.stage.map(|s| s.code()), Some(*stage), "{stage}");
        assert_eq!(d.row.state_at, 7_000);
        assert_eq!(attempts(&base, id).await, 2, "a start is not counted again");
        let line = d.events.first().unwrap();
        assert_eq!(line.message, INTERRUPTED);
        assert_eq!(line.at, 7_000);
        assert!(
            line.detail
                .as_deref()
                .unwrap()
                .ends_with("단계에서 멈췄어요"),
            "{line:?}"
        );
    }
    let d = detail(&base, &unstarted).await;
    assert_eq!((d.row.state, d.row.stage), (JobState::Interrupted, None));
    assert_eq!(
        d.events.first().unwrap().detail.as_deref(),
        Some("단계를 시작하기 전에 멈췄어요")
    );

    assert_eq!(
        rows_of(&base, &kept).await,
        before,
        "a job that does not run is not touched"
    );
    for id in &kept {
        assert!(detail(&base, id)
            .await
            .events
            .iter()
            .all(|e| e.message != INTERRUPTED));
    }
    // A second start finds none left running.
    assert_eq!(base.store.run.interrupt_running(7_100).await.unwrap(), 0);
    // The loop takes the job in line, made after the interrupted ones, and
    // none of these: an interrupted job is not in line.
    let (first, _, _) = base.store.run.claim_next(7_200).await.unwrap().unwrap();
    assert_eq!(first, kept[6], "the pending job");
    sql(
        &base,
        format!("UPDATE subtitle_jobs SET state = 'done' WHERE id = '{first}'"),
    )
    .await;
    assert_eq!(base.store.run.claim_next(7_300).await.unwrap(), None);
    for (id, stage) in &running {
        assert_eq!(
            detail(&base, id).await.row.state,
            JobState::Interrupted,
            "{stage}"
        );
    }
}

async fn rows_of(
    base: &Base,
    ids: &[String],
) -> Vec<(JobState, Option<Wait>, Option<StepKind>, i64)> {
    let mut rows = Vec::new();
    for id in ids {
        let row = detail(base, id).await.row;
        rows.push((row.state, row.wait, row.stage, row.state_at));
    }
    rows
}

async fn attempts(base: &Base, id: &str) -> i64 {
    let id = id.to_owned();
    base.db
        .run(move |c| {
            c.query_row(
                "SELECT attempts FROM subtitle_jobs WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap()
}

/// The migrations of a release before ticket 0153, which recorded file
/// effects (`subtitle_file_effects`) and compared them with the disk.
const BEFORE_NO_EFFECT_RECORDS: usize = 65;

#[tokio::test]
async fn a_job_an_earlier_release_left_running_with_its_effects_is_interrupted_by_the_upgrade() {
    // What the migration makes of each record is trss-core's rule (`db.rs`
    // tests); this is the upgrade a person sees: the job waits for a restart
    // and no file is touched.
    let dir = tempfile::tempdir().unwrap();
    let file = |relative: &str, bytes: &[u8]| {
        let path = dir.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    };
    // What that release left on disk: a received file, a fetched one not yet
    // recorded, a store's temporary file and a removal's copy moved aside.
    let left = [
        (
            file("receive/j1/a.ass", b"received"),
            b"received".as_slice(),
        ),
        (file("receive/j1/b.ass", b"fetched"), b"fetched".as_slice()),
        (file("work/.trss/tmp/e1", b"store"), b"store".as_slice()),
        (
            file("work/.trss/tmp/x1.aside", b"applied"),
            b"applied".as_slice(),
        ),
    ];
    {
        let conn = trss_core::db::database_at(&dir.path().join("app.db"), BEFORE_NO_EFFECT_RECORDS);
        conn.execute_batch(
            "INSERT INTO subtitle_jobs (id, command_id, request, origin, work_id, season, state,
                                        stage, attempts, created_at, updated_at, state_at)
                 VALUES ('j1', 'c1', '{}', 'pick', 'w', 1, 'running', 'store', 1, 0, 0, 0),
                        ('j2', 'c2', '{}', 'relocate', 'w', 1, 'running', 'apply', 1, 0, 0, 0);
             INSERT INTO subtitle_job_items (id, job_id, position, episode, post_url, found_at,
                                             state, updated_at)
                 VALUES (1, 'j1', 0, '1', 'https://example.org/1', 0, 'running', 0);
             INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state, temp_dir,
                                             expected_size, size, sha256, object, path, format,
                                             created_at, updated_at)
                 VALUES ('f1', 'j1', 1, 'k1', 'a.ass', 'done', NULL, NULL, 8,
                         printf('%064d', 1), '1:1', 'j1/a.ass', 'ass', 0, 0),
                        ('f2', 'j1', 1, 'k2', 'b.ass', 'fetched', '.tmp/f2', 7, 7,
                         printf('%064d', 2), '1:2', 'j1/b.ass', NULL, 1, 1);
             INSERT INTO subtitle_job_plan (job_id, position, file_id, name, kind, format, size,
                                            sha256, assignment, episode, action, updated_at)
                 VALUES ('j1', 0, 'f1', 'a.ass', 'subtitle', 'ass', 8, printf('%064d', 1),
                         'explicit', 1, 'apply', 0);
             INSERT INTO subtitle_file_effects (id, job_id, position, kind, state, folder, temp,
                                                target, size, sha256, created_at, updated_at)
                 VALUES ('e1', 'j1', 0, 'store', 'intended', 'work', '.trss/tmp/e1',
                         '.trss/subtitles/x/a.ass', 8, printf('%064d', 1), 0, 0);
             INSERT INTO subtitle_packages (id, work_id, job_id, source_kind, created_at)
                 VALUES ('p1', 'w', 'j2', 'post', 0);
             INSERT INTO subtitle_assets (id, work_id, kind, base, relative_path, byte_size,
                                          sha256, created_at)
                 VALUES ('a1', 'w', 'subtitle', 'work', '.trss/subtitles/y/a.ass', 7,
                         printf('%064d', 3), 0);
             INSERT INTO subtitle_package_entries (package_id, position, asset_id, original_name)
                 VALUES ('p1', 0, 'a1', 'a.ass');
             INSERT INTO subtitle_stored (id, work_id, season, package_id, subtitle_asset_id,
                                          assignment, episode, format, stored_at)
                 VALUES ('s1', 'w', 1, 'p1', 'a1', 'explicit', 3, 'ass', 0);
             INSERT INTO subtitle_applied (id, work_id, stored_id, season, episode, video_path,
                                           path, byte_size, sha256, object, applied_at)
                 VALUES ('ap1', 'w', 's1', 1, 2, 'v2.mkv', 'v2.ass', 7, printf('%064d', 3),
                         '1:4', 0);
             INSERT INTO subtitle_relocations (id, job_id, applied_id, episode, path, state,
                                               folder, aside, created_at, updated_at)
                 VALUES ('x1', 'j2', 'ap1', 2, 'v2.ass', 'set_aside', 'work',
                         '.trss/tmp/x1.aside', 0, 0);",
        )
        .unwrap();
    }

    // The start of the process: the database is migrated as it is opened,
    // then the jobs left running are interrupted.
    let base = Base::open(dir).await;
    assert_eq!(base.store.run.interrupt_running(7_000).await.unwrap(), 2);

    let d = detail(&base, "j1").await;
    assert_eq!(
        (d.row.state, d.row.stage),
        (JobState::Interrupted, Some(StepKind::Store))
    );
    assert_eq!(d.events.first().unwrap().message, INTERRUPTED);
    // The receipt the release left fetched is received anew after a restart.
    assert_eq!(
        d.items[0]
            .files
            .iter()
            .map(|f| (f.id.as_str(), f.state))
            .collect::<Vec<_>>(),
        [("f1", FileState::Done)]
    );
    let d = detail(&base, "j2").await;
    assert_eq!(d.row.state, JobState::Interrupted);
    let removals = base
        .db
        .run(|c| trss_jobs::place::relocate::removals(c, "j2").map_err(trss_core::DbError::from))
        .await
        .unwrap();
    assert_eq!(
        (removals[0].state, removals[0].reason.as_deref()),
        (
            trss_jobs::model::RemovalState::Kept,
            Some("이전 버전에서 지우다 끊겨 그대로 뒀어요")
        )
    );
    for (path, bytes) in left {
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{path:?}");
    }
}

#[tokio::test]
async fn a_restart_takes_only_an_interrupted_job_back_to_running_in_its_step() {
    let base = Base::new().await;
    let id = make(&base, "c1", "pick").await;
    sql(
        &base,
        format!(
            "UPDATE subtitle_jobs SET state = 'running', stage = 'store', attempts = 2
             WHERE id = '{id}'"
        ),
    )
    .await;

    // Only an interrupted job is restarted; another state is told back.
    for state in [
        JobState::Running,
        JobState::Pending,
        JobState::Held,
        JobState::Failed,
    ] {
        sql(
            &base,
            format!(
                "UPDATE subtitle_jobs SET state = '{}' WHERE id = '{id}'",
                state.code()
            ),
        )
        .await;
        assert_eq!(
            base.store.requests.restart(&id, 8_000).await.unwrap(),
            Restarted::NotInterrupted(state)
        );
        assert_eq!(detail(&base, &id).await.row.state, state);
    }
    assert_eq!(
        base.store
            .requests
            .restart("no-such-job", 8_000)
            .await
            .unwrap(),
        Restarted::Missing
    );
    assert!(detail(&base, &id)
        .await
        .events
        .iter()
        .all(|e| e.message != RESTARTED));

    sql(
        &base,
        format!("UPDATE subtitle_jobs SET state = 'running' WHERE id = '{id}'"),
    )
    .await;
    assert_eq!(base.store.run.interrupt_running(7_000).await.unwrap(), 1);
    assert_eq!(
        base.store.requests.restart(&id, 8_000).await.unwrap(),
        Restarted::Restarted
    );
    let d = detail(&base, &id).await;
    assert_eq!(
        (d.row.state, d.row.stage),
        (JobState::Running, Some(StepKind::Store))
    );
    assert_eq!(d.row.state_at, 8_000);
    assert_eq!(attempts(&base, &id).await, 2, "a restart is not a start");
    let line = d.events.first().unwrap();
    assert_eq!(line.message, RESTARTED);
    assert_eq!(line.detail.as_deref(), Some("보관 단계부터"));

    // Asked twice, the second finds it running.
    assert_eq!(
        base.store.requests.restart(&id, 8_100).await.unwrap(),
        Restarted::NotInterrupted(JobState::Running)
    );
}

#[tokio::test]
async fn a_restarted_job_whose_package_is_in_the_receive_area_does_not_fetch_it_again() {
    let base = Base::new().await;
    let id = make(&base, "c1", "pick").await;
    base.runner(Sources::none().with_fake(FakeSource))
        .run_ready(&CancellationToken::new())
        .await
        .unwrap();
    let received = detail(&base, &id).await;
    assert_eq!(received.row.state, JobState::Done);
    let files = || async {
        let d = detail(&base, &id).await;
        d.items[0]
            .files
            .iter()
            .map(|f| (f.id.clone(), f.state, f.path.clone()))
            .collect::<Vec<_>>()
    };
    let area_before = crate::world::tree(&base.area.at(&id));
    assert_eq!(area_before, ["a.ass"]);
    let files_before = files().await;
    assert_eq!(files_before.len(), 1);
    assert_eq!(files_before[0].1, FileState::Done);

    // The process died after the package was received, before its item and
    // the job ended: both `running`, the receipt `done`.
    sql(
        &base,
        format!(
            "UPDATE subtitle_jobs SET state = 'running', stage = 'receive', finished_at = NULL
             WHERE id = '{id}';
             UPDATE subtitle_job_items SET state = 'running' WHERE job_id = '{id}';"
        ),
    )
    .await;
    assert_eq!(detail(&base, &id).await.items[0].state, ItemState::Running);
    cut_and_restart(&base.store, &id).await;

    // The receipt and the file in the receive area are checked and kept: a
    // second fetch would leave a second receipt, or another file.
    let object_before = std::fs::metadata(base.area.at(&format!("{id}/a.ass"))).unwrap();
    base.runner(Sources::none().with_fake(FakeSource))
        .run_ready(&CancellationToken::new())
        .await
        .unwrap();
    let d = detail(&base, &id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(d.items[0].state, ItemState::Done);
    assert!(d
        .events
        .iter()
        .any(|e| e.message.contains("다시 받지 않았어요")));
    assert_eq!(files().await, files_before);
    assert_eq!(crate::world::tree(&base.area.at(&id)), area_before);
    let object_after = std::fs::metadata(base.area.at(&format!("{id}/a.ass"))).unwrap();
    assert_eq!(
        trss_jobs::area::object_of(&object_after),
        trss_jobs::area::object_of(&object_before),
        "the same file, not a new one in its place"
    );
}

#[tokio::test]
async fn an_interrupted_job_is_a_failure_to_show_and_an_unfinished_one_for_the_rest() {
    let base = Base::new().await;
    let make_in = |n: usize, state: &'static str, state_at: i64| {
        let base = &base;
        async move {
            let id = make(base, &format!("g{n}"), "pick").await;
            sql(
                base,
                format!(
                    "UPDATE subtitle_jobs SET state = '{state}', state_at = {state_at}
                     WHERE id = '{id}'"
                ),
            )
            .await;
            id
        }
    };
    let failed = make_in(0, "failed", 100).await;
    let cut = make_in(1, "interrupted", 300).await;
    let partial = make_in(2, "partial", 200).await;
    let held = make_in(3, "held", 400).await;
    let pending = make_in(4, "pending", 500).await;
    let done = make_in(5, "done", 600).await;

    let groups = base.store.views.open_groups().await.unwrap();
    let ids =
        |rows: &[trss_jobs::store::JobRow]| rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
    // With the failures, the newest change first; never among the waits.
    assert_eq!(ids(&groups.failed), [cut.clone(), partial, failed]);
    assert_eq!(ids(&groups.waiting), [held, pending]);
    assert!(groups.running.is_empty());
    let open = base.store.views.open_jobs().await.unwrap();
    assert!(open.iter().any(|r| r.id == cut));
    assert!(open.iter().all(|r| r.id != done));

    assert!(!JobState::Interrupted.is_finished());
    // An interrupted job asks nothing of a person: it waits for neither a
    // check nor a placement.
    let row = open.iter().find(|r| r.id == cut).unwrap();
    assert!(!row.waits_for_check() && !row.waits_for_placement());
}

#[tokio::test]
async fn a_find_job_cut_short_is_not_finished_until_it_is_restarted() {
    let base = Base::new().await;
    let id = make(&base, "f1", "find").await;
    sql(
        &base,
        format!(
            "UPDATE subtitle_jobs SET state = 'interrupted', stage = 'receive' WHERE id = '{id}'"
        ),
    )
    .await;
    assert_eq!(
        base.store.requests.ask_finish(&id, 9_000).await.unwrap(),
        AskedFinish::Interrupted
    );
    let d = detail(&base, &id).await;
    assert!(!d.row.finishing, "the ask was not taken");
    assert_eq!(d.row.state, JobState::Interrupted);
}

#[tokio::test]
async fn the_start_empties_the_receive_attempts_folder_and_leaves_the_rest_of_the_area() {
    let base = Base::new().await;
    let made = |relative: &str| {
        let path = base.area.at(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"bytes").unwrap();
        relative.to_owned()
    };
    // What attempts the earlier process cut short wrote.
    made(".partial/6a1d7d88-3f0e-4a7f-8d57-2b6c4c0e9f02/a.ass");
    made(".partial/9c2e4d6f-8a0b-4c1d-9e3f-7a5b1c2d4e04/b (2).zip");
    made(".partial/stray");
    // A site check's download, an upload's staging, an attempt of a release
    // before 0153 and a job's received file: none of them is the start's.
    let kept = [
        made(".tmp/check-job-1/ep1.srt"),
        made(".tmp/4f0c2f6e-1c1f-4a52-9a43-0a7b8f1d2c01/upload.ass"),
        made(".tmp/0b9e1b5c-6a7d-4f29-8c3e-5d1f7a2b4c03/a.ass"),
        made(".tmp/winpng-job/1/01.smi"),
        made("job/a.ass"),
    ];

    assert_eq!(base.area.empty_partial().unwrap(), 3);
    assert!(crate::world::tree(&base.area.at(".partial")).is_empty());
    let mut left = crate::world::tree(base.area.root());
    left.sort();
    let mut kept = kept.to_vec();
    kept.sort();
    assert_eq!(left, kept);
    // An empty or missing folder is nothing to remove.
    assert_eq!(base.area.empty_partial().unwrap(), 0);
    std::fs::remove_dir(base.area.at(".partial")).unwrap();
    assert_eq!(base.area.empty_partial().unwrap(), 0);
}
