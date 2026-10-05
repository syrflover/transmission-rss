//! Replacing the subtitle an episode has once a person approves it
//! (`docs/specs/subtitles.md`, 교체 비교와 승인 and 승인 증거와 반영 직전
//! 검사; `docs/tickets/0068-replacement-approval.md`).

use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
};

use rusqlite::params;
use tokio_util::sync::CancellationToken;
use trss_core::{Clock, Db};
use trss_jobs::{
    area::{object_of, ReceiveArea},
    model::{Outcome, PathAction, PlanState},
    place::replace::records::{Decided, Plan, PlanView},
    place::replace::AWAITING_APPROVAL,
    store::{JobDetail, DECIDED},
    Created, JobState, JobStore, NewItem, NewJob, Runner, Wait,
};
use trss_subtitles::{
    fake::{self, FakeSource},
    Sources,
};

const WORK: &str = "w1";
const CREATOR: &str = "제작자";
const OTHER: &str = "다른 제작자";
const VIDEO: &str = "Season 01/Show S01E02.mkv";
const TARGET: &str = "Season 01/Show S01E02.ass";
const SRT: &str = "Season 01/Show S01E02.srt";
/// A subtitle at `TARGET` the app did not manage.
const MINE: &str =
    "[Script Info]\n[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,내 자막\n";

struct Setup {
    dir: tempfile::TempDir,
    db: Db,
    store: JobStore,
    runner: Runner,
}

impl Setup {
    /// The work's folder.
    fn work(&self) -> PathBuf {
        self.dir.path().join("shows/Show")
    }

    fn at(&self, path: &str) -> PathBuf {
        self.work().join(path)
    }

    fn read(&self, path: &str) -> Vec<u8> {
        std::fs::read(self.at(path)).unwrap()
    }

    fn temps(&self) -> Vec<String> {
        names(&self.at(".trss/tmp"))
    }

    async fn sql(&self, sql: String) {
        self.db
            .run(move |c| c.execute_batch(&sql).map_err(trss_core::DbError::from))
            .await
            .unwrap();
    }

    async fn count(&self, sql: &'static str) -> i64 {
        self.db
            .run(move |c| {
                c.query_row(sql, [], |r| r.get(0))
                    .map_err(trss_core::DbError::from)
            })
            .await
            .unwrap()
    }
}

fn ticking_clock() -> Clock {
    let now = Arc::new(AtomicI64::new(1_000));
    Arc::new(move || now.fetch_add(10, Ordering::SeqCst))
}

/// A library with the work `Show` (season 1) whose episode 2 has a video.
async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let shows = dir.path().join("shows");
    std::fs::create_dir_all(shows.join("Show/Season 01")).unwrap();
    std::fs::write(shows.join("Show").join(VIDEO), b"video").unwrap();
    let path = shows.to_string_lossy().into_owned();
    db.run(move |c| {
        c.execute(
            "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', ?1, 0)",
            [path],
        )?;
        c.execute_batch(&format!(
            "INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('{WORK}', 'f1', 'Show');
             INSERT INTO seasons (work_id, number) VALUES ('{WORK}', 1);
             INSERT INTO episodes (work_id, season, episode) VALUES ('{WORK}', 1, '02');
             INSERT INTO media_files (work_id, path, season, episode, kind)
                 VALUES ('{WORK}', '{VIDEO}', 1, '02', 'video');"
        ))
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
    let store = JobStore::new(db.clone());
    let area = ReceiveArea::in_app_data(dir.path());
    let runner = Runner::new(
        store.clone(),
        Sources::none().with_fake(FakeSource),
        area,
        ticking_clock(),
    );
    Setup {
        dir,
        db,
        store,
        runner,
    }
}

/// A pick's job of the work for the fake post `path` (on [`fake::HOST`]) by
/// `creator`, as Anissia's episode 2.
async fn make(s: &Setup, command: &str, creator: &str, path: &str) -> String {
    let job = NewJob {
        command_id: command.to_owned(),
        request: format!("{{\"c\":\"{command}\"}}"),
        origin: "pick".to_owned(),
        work_id: Some(WORK.to_owned()),
        season: Some(1),
        anime_no: Some(7),
        source_id: None,
        creator: Some(creator.to_owned()),
        revision_of: None,
        revises_attributed: false,
        items: vec![NewItem {
            observation_id: None,
            episode: "2".to_owned(),
            post_url: format!("https://{}{path}", fake::HOST),
            found_at: 500,
        }],
    };
    match s.store.create(job, 900).await.unwrap() {
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

fn names(dir: &Path) -> Vec<String> {
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

fn waiting_for_approval(d: &JobDetail) {
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Approval)),
        "{:?} {:?}",
        d.row.note,
        d.events
    );
}

/// The job's one latest plan.
async fn view(s: &Setup, job: &str) -> PlanView {
    let mut views = s.store.replacements(job).await.unwrap();
    assert_eq!(views.len(), 1, "{views:?}");
    views.remove(0)
}

async fn decide(s: &Setup, job: &str, plan: &Plan, replace: bool) -> Decided {
    s.store
        .decide_replacement(job, &plan.id, plan.version, replace, 5_000_000)
        .await
        .unwrap()
}

/// Every plan of the job's first row, by version: its state and reason.
async fn versions(s: &Setup, job: &str) -> Vec<(i64, PlanState, Option<String>)> {
    let job = job.to_owned();
    s.db.run(move |c| {
        let mut stmt = c.prepare(
            "SELECT version, state, reason FROM subtitle_replacements
              WHERE job_id = ?1 ORDER BY version",
        )?;
        let rows = stmt.query_map([job], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap()
}

/// The episode's subtitle applied by a first job (`/ok/Show-02`), then a
/// second job's (`/ok/Show-02v2`, the creator's revision) waiting to replace
/// it: the second job and its open plan.
async fn revision_waiting(s: &Setup) -> (String, Plan) {
    let first = make(s, "c1", CREATOR, "/ok/Show-02").await;
    run(s).await;
    assert_eq!(detail(s, &first).await.row.state, JobState::Done);
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    let second = make(s, "c2", CREATOR, "/ok/Show-02v2").await;
    run(s).await;
    waiting_for_approval(&detail(s, &second).await);
    let plan = view(s, &second).await.plan;
    (second, plan)
}

/// [`revision_waiting`], approved.
async fn revision_approved(s: &Setup) -> (String, Plan) {
    let (job, plan) = revision_waiting(s).await;
    assert_eq!(
        decide(s, &job, &plan, true).await,
        Decided::Done(PlanState::Approved)
    );
    (job, plan)
}

/// The episode's subtitle applied as `SRT` by a first job (a package's
/// `.srt`), then another creator's `.ass` (`/ok/Show-02`) waiting to go
/// beside the video: the second job and its open plan.
async fn ass_waiting_beside_srt(s: &Setup) -> (String, Plan) {
    let first = make(s, "c1", CREATOR, "/pack/Show - 02.srt").await;
    run(s).await;
    assert_eq!(detail(s, &first).await.row.state, JobState::Done);
    assert!(s.at(SRT).exists());
    let job = make(s, "c2", OTHER, "/ok/Show-02").await;
    run(s).await;
    waiting_for_approval(&detail(s, &job).await);
    let plan = view(s, &job).await.plan;
    (job, plan)
}

/// `MINE` at `TARGET`, then a job's plan (`/ok/Show-02`) to replace it
/// waiting.
async fn unmanaged_waiting(s: &Setup) -> (String, Plan) {
    std::fs::write(s.at(TARGET), MINE).unwrap();
    let job = make(s, "c1", CREATOR, "/ok/Show-02").await;
    run(s).await;
    waiting_for_approval(&detail(s, &job).await);
    let plan = view(s, &job).await.plan;
    (job, plan)
}

/// What the plan does to each path.
fn actions(plan: &Plan) -> Vec<(&str, PathAction)> {
    plan.paths
        .iter()
        .map(|p| (p.path.as_str(), p.action))
        .collect()
}

#[tokio::test]
async fn a_revision_waits_for_approval_then_replaces_and_the_earlier_stored_copy_stays() {
    let s = setup().await;
    let (job, plan) = revision_waiting(&s).await;
    // The plan binds the episode's file and the new asset.
    assert_eq!((plan.version, plan.state), (1, PlanState::Open));
    assert_eq!((plan.episode, plan.target.as_str()), (2, TARGET));
    assert_eq!(plan.video.path, VIDEO);
    assert_eq!(plan.paths.len(), 1);
    let path = &plan.paths[0];
    assert_eq!(
        (path.path.as_str(), path.action),
        (TARGET, PathAction::Replace)
    );
    assert!(path.applied_id.is_some(), "the app's applied copy");
    assert_eq!(path.file.as_ref().unwrap().lines, Some(24));
    assert_eq!(plan.asset_lines, Some(24));
    let v = view(&s, &job).await;
    assert_eq!(v.previous, None);
    assert_eq!(v.new.as_ref().unwrap().creator.as_deref(), Some(CREATOR));
    assert_eq!(v.applied.len(), 1);
    // The row waits, the job's other steps are done, nothing changed yet.
    assert_eq!(s.store.plan(&job).await.unwrap()[0].outcome, None);
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));

    // Looked at again with nothing changed: the same plan.
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'pending' WHERE id = '{job}'"
    ))
    .await;
    run(&s).await;
    waiting_for_approval(&detail(&s, &job).await);
    assert_eq!(versions(&s, &job).await.len(), 1);

    assert_eq!(
        decide(&s, &job, &plan, true).await,
        Decided::Done(PlanState::Approved)
    );
    let d = detail(&s, &job).await;
    assert_eq!(
        (d.row.state, d.row.note.as_deref()),
        (JobState::Pending, Some(DECIDED))
    );
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(TARGET), fake::ass("Show-02v2"));
    assert_eq!(
        s.store.plan(&job).await.unwrap()[0].outcome,
        Some(Outcome::Applied)
    );
    assert_eq!(versions(&s, &job).await[0].1, PlanState::Done);
    assert!(d
        .events
        .iter()
        .any(|e| e.message == "2화: 새 자막으로 교체했어요"));
    // Both stored copies stay; the first applied copy is recorded removed.
    let stored = s.at(&format!(".trss/subtitles/{CREATOR}"));
    assert_eq!(names(&stored), ["Show-02.ass", "Show-02v2.ass"]);
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_applied WHERE removed_at IS NULL")
            .await,
        1
    );
    assert_eq!(s.count("SELECT count(*) FROM subtitle_applied").await, 2);
    // Nothing protective or temporary is left, and every effect ended done.
    assert_eq!(s.temps(), Vec::<String>::new());
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_file_effects
              WHERE plan_id IS NOT NULL AND state <> 'done'"
        )
        .await,
        0
    );
    // An approval is used once.
    assert_eq!(decide(&s, &job, &plan, true).await, Decided::Stale);
}

#[tokio::test]
async fn keeping_the_current_subtitle_leaves_the_file_and_ends_the_job() {
    let s = setup().await;
    let (job, plan) = revision_waiting(&s).await;
    assert_eq!(
        decide(&s, &job, &plan, false).await,
        Decided::Done(PlanState::Kept)
    );
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    assert_eq!(
        s.store.plan(&job).await.unwrap()[0].outcome,
        Some(Outcome::Existing)
    );
    // The new stored copy stays, to be chosen later.
    assert!(s
        .at(&format!(".trss/subtitles/{CREATOR}/Show-02v2.ass"))
        .exists());
    assert_eq!(versions(&s, &job).await[0].1, PlanState::Kept);
    assert_eq!(s.count("SELECT count(*) FROM subtitle_applied").await, 1);
}

#[tokio::test]
async fn a_decision_on_a_plan_that_is_not_the_rows_to_decide_is_refused() {
    let s = setup().await;
    let (job, plan) = revision_waiting(&s).await;
    let other = make(&s, "c9", OTHER, "/ok/Other-02").await;
    assert_eq!(
        s.store
            .decide_replacement(&job, &plan.id, plan.version + 1, true, 1)
            .await
            .unwrap(),
        Decided::Stale
    );
    assert_eq!(
        s.store
            .decide_replacement(&other, &plan.id, plan.version, true, 1)
            .await
            .unwrap(),
        Decided::NotFound
    );
    assert_eq!(
        s.store
            .decide_replacement(&job, "nothing", 1, true, 1)
            .await
            .unwrap(),
        Decided::NotFound
    );
    assert_eq!(versions(&s, &job).await[0].1, PlanState::Open);
}

#[tokio::test]
async fn an_unmanaged_subtitle_is_imported_as_the_unknown_creators_before_it_is_replaced() {
    let s = setup().await;
    let (job, plan) = unmanaged_waiting(&s).await;
    let path = &plan.paths[0];
    assert_eq!(
        (path.path.as_str(), path.action),
        (TARGET, PathAction::Replace)
    );
    assert_eq!(path.applied_id, None, "not the app's");
    assert_eq!(path.file.as_ref().unwrap().lines, Some(1));

    decide(&s, &job, &plan, true).await;
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    // The bytes that were there are the unknown creator's stored subtitle of
    // the episode, from an `existing` package.
    let imported = s.at(".trss/subtitles/제작자 알 수 없음/Show S01E02.ass");
    assert_eq!(std::fs::read(imported).unwrap(), MINE.as_bytes());
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_stored st
               JOIN subtitle_packages p ON p.id = st.package_id
              WHERE st.creator IS NULL AND st.source_id IS NULL AND st.episode = 2
                AND p.source_kind = 'existing'"
        )
        .await,
        1
    );
    assert!(d.events.iter().any(|e| e
        .message
        .contains("관리하지 않던 자막을 '제작자 알 수 없음' 보관본으로 들였어요")));
    assert_eq!(s.temps(), Vec::<String>::new());
}

#[tokio::test]
async fn another_creators_ass_removes_the_applied_srt_and_keeps_its_stored_copy() {
    let s = setup().await;
    let (job, plan) = ass_waiting_beside_srt(&s).await;
    assert_eq!(
        actions(&plan),
        [(TARGET, PathAction::Add), (SRT, PathAction::Remove)]
    );

    decide(&s, &job, &plan, true).await;
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert!(!s.at(SRT).exists());
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    assert!(s
        .at(&format!(".trss/subtitles/{CREATOR}/Show - 02.srt"))
        .exists());
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_applied a
               JOIN subtitle_job_plan r ON r.applied_id = a.id
              WHERE r.job_id <> (SELECT job_id FROM subtitle_replacements)
                AND a.removed_at IS NOT NULL"
        )
        .await,
        1
    );
    assert_eq!(s.temps(), Vec::<String>::new());
}

/// After approval, `change` happens before the worker carries it out: the
/// plan goes stale for `reason` and nothing beside the video changes.
async fn changed_after_approval(
    s: &Setup,
    job: &str,
    plan: &Plan,
    reason: &str,
    unchanged: &[(&str, Vec<u8>)],
) {
    run(s).await;
    let v = versions(s, job).await;
    assert_eq!(v[0].0, 1);
    assert_eq!(v[0].1, PlanState::Stale, "{v:?}");
    assert!(
        v[0].2.as_deref().is_some_and(|r| r.contains(reason)),
        "{v:?}"
    );
    for (path, bytes) in unchanged {
        assert_eq!(&s.read(path), bytes, "{path}");
    }
    assert_eq!(s.count("SELECT count(*) FROM subtitle_applied").await, 1);
    assert_eq!(s.temps(), Vec::<String>::new());
    let _ = plan;
}

#[tokio::test]
async fn a_video_replaced_after_approval_asks_to_compare_again() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    // A new video file takes the old one's place.
    let new = s.at("Season 01/new.mkv");
    std::fs::write(&new, b"video").unwrap();
    std::fs::rename(&new, s.at(VIDEO)).unwrap();
    changed_after_approval(
        &s,
        &job,
        &plan,
        "영상이 바뀌었어요",
        &[(TARGET, fake::ass("Show-02"))],
    )
    .await;
    // The person compares again: the next version waits.
    waiting_for_approval(&detail(&s, &job).await);
    let v = view(&s, &job).await;
    assert_eq!((v.plan.version, v.plan.state), (2, PlanState::Open));
    assert_eq!(v.previous.as_deref(), Some("영상이 바뀌었어요"));
}

#[tokio::test]
async fn the_existing_subtitle_changed_to_other_bytes_of_its_size_asks_to_compare_again() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    let mut bytes = fake::ass("Show-02");
    let last = bytes.len() - 2;
    bytes[last] = b'X';
    std::fs::write(s.at(TARGET), &bytes).unwrap();
    changed_after_approval(
        &s,
        &job,
        &plan,
        "기존 자막이 비교한 뒤 바뀌었어요",
        &[(TARGET, bytes)],
    )
    .await;
    waiting_for_approval(&detail(&s, &job).await);
    let v = view(&s, &job).await;
    assert_eq!(v.plan.version, 2);
    // Its bytes are not the app's copy any more.
    assert_eq!(v.plan.paths[0].applied_id, None);
}

#[tokio::test]
async fn a_stored_asset_changed_after_approval_is_not_applied() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    let mut bytes = fake::ass("Show-02v2");
    let last = bytes.len() - 2;
    bytes[last] = b'X';
    std::fs::write(s.at(&plan.asset_path), &bytes).unwrap();
    changed_after_approval(
        &s,
        &job,
        &plan,
        "새 자막의 보관 파일이 기록과 달라요",
        &[(TARGET, fake::ass("Show-02"))],
    )
    .await;
    // Compared again, the stored copy is not what was recorded: held.
    let d = detail(&s, &job).await;
    assert_eq!(d.row.state, JobState::Held, "{:?}", d.row.note);
    assert_eq!(versions(&s, &job).await.len(), 1);
}

#[tokio::test]
async fn a_mapping_changed_after_approval_is_not_applied() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    // The row and its stored subtitle are episode 3's now, which has no
    // video.
    s.sql(format!(
        "INSERT INTO episodes (work_id, season, episode) VALUES ('{WORK}', 1, '03');
         UPDATE subtitle_job_plan SET episode = 3 WHERE job_id = '{job}';
         UPDATE subtitle_stored SET episode = 3 WHERE id = '{}';",
        plan.stored_id
    ))
    .await;
    changed_after_approval(
        &s,
        &job,
        &plan,
        "회차 대응이 바뀌었어요",
        &[(TARGET, fake::ass("Show-02"))],
    )
    .await;
    let d = detail(&s, &job).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Video)),
        "{:?}",
        d.row.note
    );
}

#[tokio::test]
async fn a_file_at_the_path_to_add_after_approval_asks_to_compare_again() {
    let s = setup().await;
    let smi = "Season 01/Show S01E02.smi";
    std::fs::write(s.at(smi), b"<SAMI><SYNC Start=1>a</SAMI>").unwrap();
    let job = make(&s, "c1", CREATOR, "/ok/Show-02").await;
    run(&s).await;
    waiting_for_approval(&detail(&s, &job).await);
    let plan = view(&s, &job).await.plan;
    assert_eq!(
        actions(&plan),
        [(TARGET, PathAction::Add), (smi, PathAction::Keep)]
    );
    decide(&s, &job, &plan, true).await;
    // Someone puts a subtitle where the new copy would go.
    std::fs::write(s.at(TARGET), b"mine").unwrap();
    run(&s).await;
    let v = versions(&s, &job).await;
    assert_eq!(v[0].1, PlanState::Stale, "{v:?}");
    assert!(v[0]
        .2
        .as_deref()
        .is_some_and(|r| r.contains("새 자막을 둘 자리에 파일이 생겼어요")));
    assert_eq!(s.read(TARGET), b"mine");
    assert_eq!(s.read(smi), b"<SAMI><SYNC Start=1>a</SAMI>");
    waiting_for_approval(&detail(&s, &job).await);
    let next = view(&s, &job).await.plan;
    assert_eq!(next.paths[0].action, PathAction::Replace);
}

#[tokio::test]
async fn approving_a_plan_with_a_kept_file_adds_beside_it() {
    let s = setup().await;
    let smi = "Season 01/Show S01E02.smi";
    std::fs::write(s.at(smi), b"<SAMI><SYNC Start=1>a</SAMI>").unwrap();
    let job = make(&s, "c1", CREATOR, "/ok/Show-02").await;
    run(&s).await;
    let plan = view(&s, &job).await.plan;
    decide(&s, &job, &plan, true).await;
    run(&s).await;
    assert_eq!(detail(&s, &job).await.row.state, JobState::Done);
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    // A subtitle the app does not manage at another path is not guessed
    // away.
    assert_eq!(s.read(smi), b"<SAMI><SYNC Start=1>a</SAMI>");
    assert_eq!(s.count("SELECT count(*) FROM subtitle_stored").await, 1);
}

#[tokio::test]
async fn no_room_for_the_copies_stops_before_anything_beside_the_video_changes() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    let tmp = s.at(".trss/tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o555)).unwrap();
    run(&s).await;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
    let v = versions(&s, &job).await;
    assert_eq!(v[0].1, PlanState::Failed, "{v:?}");
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    assert_eq!(s.temps(), Vec::<String>::new());
    assert_eq!(
        s.store.plan(&job).await.unwrap()[0].outcome,
        Some(Outcome::Failed)
    );
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_file_effects
              WHERE plan_id IS NOT NULL AND state <> 'abandoned'"
        )
        .await,
        0
    );
    let _ = plan;
}

#[tokio::test]
async fn of_two_approved_jobs_on_one_path_the_later_compares_again() {
    let s = setup().await;
    let first = make(&s, "c1", CREATOR, "/ok/Show-02").await;
    run(&s).await;
    assert_eq!(detail(&s, &first).await.row.state, JobState::Done);
    let a = make(&s, "c2", CREATOR, "/ok/Show-02v2").await;
    let b = make(&s, "c3", OTHER, "/ok/Show-02v3").await;
    run(&s).await;
    for job in [&a, &b] {
        waiting_for_approval(&detail(&s, job).await);
        let plan = view(&s, job).await.plan;
        decide(&s, job, &plan, true).await;
    }
    run(&s).await;
    // A came first in line and replaced the subtitle; B's plan saw the one
    // before, so B compares again with A's copy as the current one.
    assert_eq!(detail(&s, &a).await.row.state, JobState::Done);
    assert_eq!(s.read(TARGET), fake::ass("Show-02v2"));
    waiting_for_approval(&detail(&s, &b).await);
    let v = versions(&s, &b).await;
    assert_eq!(v[0].1, PlanState::Stale, "{v:?}");
    let next = view(&s, &b).await;
    assert_eq!((next.plan.version, next.plan.state), (2, PlanState::Open));
    assert_eq!(
        next.plan.paths[0].file.as_ref().unwrap().sha256,
        trss_jobs::area::hex(&sha2_of(&fake::ass("Show-02v2")))
    );
}

#[tokio::test]
async fn a_decision_made_while_the_job_runs_is_carried_out_by_its_next_run() {
    for replace in [true, false] {
        let s = setup().await;
        let (job, plan) = revision_waiting(&s).await;
        // A run counted the open plan; the person decides before it ends.
        s.sql(format!(
            "UPDATE subtitle_jobs SET state = 'running', wait = NULL WHERE id = '{job}'"
        ))
        .await;
        decide(&s, &job, &plan, replace).await;
        assert_eq!(detail(&s, &job).await.row.state, JobState::Running);
        let back = s
            .store
            .settle(
                &job,
                JobState::Waiting,
                Some(Wait::Approval),
                Some(AWAITING_APPROVAL.to_owned()),
                6_000_000,
            )
            .await
            .unwrap();
        assert!(back, "{replace}");
        let d = detail(&s, &job).await;
        assert_eq!(
            (d.row.state, d.row.wait, d.row.note.as_deref()),
            (JobState::Pending, None, Some(DECIDED)),
            "{replace}"
        );
        run(&s).await;
        let d = detail(&s, &job).await;
        assert_eq!(d.row.state, JobState::Done, "{replace} {:?}", d.row.note);
        let expected = if replace { "Show-02v2" } else { "Show-02" };
        assert_eq!(s.read(TARGET), fake::ass(expected));
    }
}

#[tokio::test]
async fn a_job_that_still_has_a_plan_to_decide_waits_for_it() {
    let s = setup().await;
    let (job, _) = revision_waiting(&s).await;
    let back = s
        .store
        .settle(
            &job,
            JobState::Waiting,
            Some(Wait::Approval),
            Some(AWAITING_APPROVAL.to_owned()),
            6_000_000,
        )
        .await
        .unwrap();
    assert!(!back);
    waiting_for_approval(&detail(&s, &job).await);
}

#[tokio::test]
async fn a_decision_on_a_held_job_leaves_it_held() {
    let s = setup().await;
    let (job, plan) = revision_waiting(&s).await;
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'held', wait = NULL WHERE id = '{job}'"
    ))
    .await;
    assert_eq!(
        decide(&s, &job, &plan, true).await,
        Decided::Done(PlanState::Approved)
    );
    let d = detail(&s, &job).await;
    assert_eq!((d.row.state, d.row.wait), (JobState::Held, None));
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
}

#[tokio::test]
async fn a_new_copy_published_but_not_recorded_holds_the_plan_and_removes_nothing_more() {
    let s = setup().await;
    let (job, plan) = ass_waiting_beside_srt(&s).await;
    decide(&s, &job, &plan, true).await;
    // The database refuses the record of the new copy.
    s.sql(format!(
        "CREATE TRIGGER refuse BEFORE INSERT ON subtitle_applied WHEN NEW.path = '{TARGET}'
         BEGIN SELECT RAISE(ABORT, 'refused'); END;"
    ))
    .await;
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Held,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    let v = versions(&s, &job).await;
    assert_eq!(v[0].1, PlanState::Held);
    assert!(
        v[0].2
            .as_deref()
            .is_some_and(|r| r.contains("공개한 파일을 기록하지 못했어요")),
        "{v:?}"
    );
    // The earlier applied copy stays where it is, still the one applied.
    assert!(s.at(SRT).exists());
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_applied
              WHERE path = 'Season 01/Show S01E02.srt' AND removed_at IS NULL"
        )
        .await,
        1
    );
    assert!(!d
        .events
        .iter()
        .any(|e| e.message.contains("새 자막으로 교체했어요")));
}

#[tokio::test]
async fn a_subtitle_the_library_recorded_that_is_gone_is_not_one_to_replace() {
    let s = setup().await;
    // The watcher has not dropped the record of a subtitle removed.
    s.sql(format!(
        "INSERT INTO media_files (work_id, path, season, episode, kind)
             VALUES ('{WORK}', '{SRT}', 1, '02', 'subtitle')"
    ))
    .await;
    let job = make(&s, "c1", CREATOR, "/ok/Show-02").await;
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_replacements").await,
        0
    );
}

#[tokio::test]
async fn of_two_names_differing_in_case_only_the_targets_own_is_replaced() {
    let s = setup().await;
    let upper = "Season 01/Show S01E02.ASS";
    std::fs::write(s.at(upper), b"[Script Info]\nupper").unwrap();
    let (job, plan) = unmanaged_waiting(&s).await;
    assert_eq!(
        actions(&plan),
        [(TARGET, PathAction::Replace), (upper, PathAction::Keep)]
    );
    decide(&s, &job, &plan, true).await;
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    assert_eq!(s.read(upper), b"[Script Info]\nupper");
}

#[tokio::test]
async fn two_files_to_take_off_differing_in_case_leave_the_subtitle_stored_only() {
    let s = setup().await;
    let first = make(&s, "c1", CREATOR, "/ok/Show-02").await;
    run(&s).await;
    assert_eq!(detail(&s, &first).await.row.state, JobState::Done);
    // The video's name changes in case only: the applied copy keeps the old
    // name, and a file the app did not manage takes the new target's.
    let video = "Season 01/show s01e02.mkv";
    std::fs::rename(s.at(VIDEO), s.at(video)).unwrap();
    s.sql(format!(
        "UPDATE media_files SET path = '{video}' WHERE path = '{VIDEO}'"
    ))
    .await;
    let target = "Season 01/show s01e02.ass";
    std::fs::write(s.at(target), MINE).unwrap();
    let job = make(&s, "c2", OTHER, "/ok/Show-02v2").await;
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(
        s.store.plan(&job).await.unwrap()[0].outcome,
        Some(Outcome::Existing)
    );
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_replacements").await,
        0
    );
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    assert_eq!(s.read(target), MINE.as_bytes());
}

#[tokio::test]
async fn an_applied_copy_whose_record_changed_after_approval_asks_to_compare_again() {
    let s = setup().await;
    let (job, plan) = ass_waiting_beside_srt(&s).await;
    decide(&s, &job, &plan, true).await;
    // The app's record of the `.srt` ended; the file stays as it was.
    s.sql(format!(
        "UPDATE subtitle_applied SET removed_at = 2 WHERE path = '{SRT}'"
    ))
    .await;
    run(&s).await;
    let v = versions(&s, &job).await;
    assert_eq!(v[0].1, PlanState::Stale, "{v:?}");
    assert!(
        v[0].2
            .as_deref()
            .is_some_and(|r| r.contains("기존 자막의 적용 기록이 바뀌었어요")),
        "{v:?}"
    );
    assert!(s.at(SRT).exists());
    assert!(!s.at(TARGET).exists());
    // Not the app's any more, the `.srt` is kept beside the new copy.
    waiting_for_approval(&detail(&s, &job).await);
    let next = view(&s, &job).await.plan;
    assert_eq!(
        actions(&next),
        [(TARGET, PathAction::Add), (SRT, PathAction::Keep)]
    );
}

#[tokio::test]
async fn a_second_video_after_approval_leaves_the_subtitle_stored_only() {
    let s = setup().await;
    let (job, _) = revision_approved(&s).await;
    let second = "Season 01/Show S01E02 v2.mkv";
    std::fs::write(s.at(second), b"video 2").unwrap();
    s.sql(format!(
        "INSERT INTO media_files (work_id, path, season, episode, kind)
             VALUES ('{WORK}', '{second}', 1, '02', 'video')"
    ))
    .await;
    run(&s).await;
    let v = versions(&s, &job).await;
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].1, PlanState::Stale, "{v:?}");
    assert!(
        v[0].2
            .as_deref()
            .is_some_and(|r| r.contains("이 회차의 영상이 바뀌었어요")),
        "{v:?}"
    );
    let d = detail(&s, &job).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(
        s.store.plan(&job).await.unwrap()[0].outcome,
        Some(Outcome::Stored)
    );
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    assert_eq!(s.temps(), Vec::<String>::new());
}

fn sha2_of(bytes: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).to_vec()
}

// ---------------------------------------------------------------------------
// A worker killed while carrying out an approved plan
//
// The revision's plan replaces the app's copy at `TARGET`: an apply of the
// new copy and a removal of the old. Each test leaves the two effects as a
// worker killed at one point would, then runs the job again.

struct Left {
    apply_temp: String,
    removal_temp: String,
    aside: String,
}

/// Records an effect of `plan` as a killed worker left it.
#[allow(clippy::too_many_arguments)]
async fn left_effect(
    s: &Setup,
    plan: &Plan,
    id: &str,
    kind: &'static str,
    source: &str,
    state: &'static str,
    temp: &str,
    target: &str,
    bytes: &[u8],
    object: Option<String>,
) {
    let sha = trss_jobs::area::hex(&sha2_of(bytes));
    let (video, source) = match kind {
        "apply" => (Some(plan.video.path.clone()), None),
        _ => (None, Some(source.to_owned())),
    };
    let (id, job, plan_id, position) = (
        id.to_owned(),
        plan.job_id.clone(),
        plan.id.clone(),
        plan.position,
    );
    let (temp, target, folder) = (temp.to_owned(), target.to_owned(), plan.folder.clone());
    let size = bytes.len() as i64;
    s.db.run(move |c| {
        c.execute(
            "INSERT INTO subtitle_file_effects
                 (id, job_id, position, kind, state, folder, temp, target, video, source,
                  plan_id, size, sha256, object, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 1, 1)",
            params![
                id, job, position, kind, state, folder, temp, target, video, source, plan_id, size,
                sha, object
            ],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
}

fn write(s: &Setup, path: &str, bytes: &[u8]) -> String {
    let at = s.at(path);
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(&at, bytes).unwrap();
    object_of(&std::fs::metadata(&at).unwrap())
}

fn object(s: &Setup, path: &str) -> String {
    object_of(&std::fs::metadata(s.at(path)).unwrap())
}

/// The plan's two effects left `apply_state` and `removal_state`, with the
/// files a worker at that point had written: the new copy's temporary file
/// once prepared, the protective copy once the removal is prepared, and the
/// old copy moved aside once set aside.
async fn killed(
    s: &Setup,
    plan: &Plan,
    apply_state: &'static str,
    removal_state: &'static str,
) -> Left {
    let left = Left {
        apply_temp: ".trss/tmp/a1".to_owned(),
        removal_temp: ".trss/tmp/r1".to_owned(),
        aside: ".trss/tmp/r1.aside".to_owned(),
    };
    let new = fake::ass("Show-02v2");
    let old = fake::ass("Show-02");
    let apply_object = match apply_state {
        "intended" => None,
        _ => Some(write(s, &left.apply_temp, &new)),
    };
    let removal_object = match removal_state {
        "intended" => None,
        _ => Some(write(s, &left.removal_temp, &old)),
    };
    if removal_state == "set_aside" {
        std::fs::rename(s.at(TARGET), s.at(&left.aside)).unwrap();
    }
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'running' WHERE id = '{}'",
        plan.job_id
    ))
    .await;
    left_effect(
        s,
        plan,
        "e-apply",
        "apply",
        TARGET,
        apply_state,
        &left.apply_temp,
        TARGET,
        &new,
        apply_object,
    )
    .await;
    left_effect(
        s,
        plan,
        "e-remove",
        "remove",
        TARGET,
        removal_state,
        &left.removal_temp,
        &left.aside,
        &old,
        removal_object,
    )
    .await;
    left
}

/// The replacement ended as one: the new copy at the target, recorded once,
/// the old copy recorded removed, nothing protective left, the plan done.
async fn replaced_once(s: &Setup, job: &str) {
    let d = detail(s, job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(TARGET), fake::ass("Show-02v2"));
    assert_eq!(s.count("SELECT count(*) FROM subtitle_applied").await, 2);
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_applied WHERE removed_at IS NULL")
            .await,
        1
    );
    assert_eq!(s.temps(), Vec::<String>::new());
    assert_eq!(versions(s, job).await[0].1, PlanState::Done);
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_file_effects
              WHERE plan_id IS NOT NULL AND state NOT IN ('done', 'abandoned')"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn killed_after_claiming_the_effects_carries_the_plan_out_anew() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    killed(&s, &plan, "intended", "intended").await;
    run(&s).await;
    replaced_once(&s, &job).await;
}

#[tokio::test]
async fn killed_after_the_protective_copy_carries_the_plan_out_anew() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    killed(&s, &plan, "prepared", "prepared").await;
    run(&s).await;
    replaced_once(&s, &job).await;
    // The first try's effects were abandoned, their files removed.
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_file_effects WHERE state = 'abandoned'")
            .await,
        2
    );
}

#[tokio::test]
async fn killed_after_setting_the_old_copy_aside_publishes_the_new_one() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    killed(&s, &plan, "prepared", "set_aside").await;
    assert!(!s.at(TARGET).exists());
    run(&s).await;
    replaced_once(&s, &job).await;
}

#[tokio::test]
async fn killed_after_setting_aside_but_before_its_record_is_found_set_aside() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    let left = killed(&s, &plan, "prepared", "prepared").await;
    std::fs::rename(s.at(TARGET), s.at(&left.aside)).unwrap();
    run(&s).await;
    replaced_once(&s, &job).await;
}

#[tokio::test]
async fn killed_after_publishing_before_its_record_records_it_once() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    let left = killed(&s, &plan, "prepared", "set_aside").await;
    std::fs::rename(s.at(&left.apply_temp), s.at(TARGET)).unwrap();
    run(&s).await;
    replaced_once(&s, &job).await;
}

/// As recording the publish of the plan's new copy `bytes` (the file
/// `object` at `TARGET`) would have: the apply effect done, the copy
/// applied, an earlier copy recorded at the path removed.
async fn recorded_applied(s: &Setup, plan: &Plan, bytes: &[u8], object: String) {
    let (job, stored) = (plan.job_id.clone(), plan.stored_id.clone());
    let sha = trss_jobs::area::hex(&sha2_of(bytes));
    let size = bytes.len() as i64;
    s.db.run(move |c| {
        c.execute(
            "UPDATE subtitle_applied SET removed_at = 2
              WHERE work_id = ?1 AND path = ?2 AND removed_at IS NULL",
            params![WORK, TARGET],
        )?;
        c.execute(
            "INSERT INTO subtitle_applied
                 (id, work_id, stored_id, season, episode, video_path, path, byte_size,
                  sha256, object, job_id, applied_at)
             VALUES ('ap2', ?1, ?2, 1, 2, ?3, ?4, ?5, ?6, ?7, ?8, 2)",
            params![WORK, stored, VIDEO, TARGET, size, sha, object, job],
        )?;
        c.execute(
            "UPDATE subtitle_file_effects SET state = 'done' WHERE id = 'e-apply'",
            [],
        )?;
        c.execute(
            "UPDATE subtitle_job_plan SET outcome = 'applied', applied_id = 'ap2'
              WHERE job_id = ?1",
            [job],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
}

/// The revision's plan published, recorded and done, as a worker killed
/// before the clean-up left it: the protective copy and the old copy aside
/// still there.
async fn done_before_clean_up(s: &Setup, plan: &Plan) -> Left {
    let left = killed(s, plan, "prepared", "set_aside").await;
    let publish = object(s, &left.apply_temp);
    std::fs::rename(s.at(&left.apply_temp), s.at(TARGET)).unwrap();
    recorded_applied(s, plan, &fake::ass("Show-02v2"), publish).await;
    s.sql(format!(
        "UPDATE subtitle_replacements SET state = 'done' WHERE id = '{}'",
        plan.id
    ))
    .await;
    left
}

#[tokio::test]
async fn killed_after_the_plan_was_done_cleans_up() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    done_before_clean_up(&s, &plan).await;
    run(&s).await;
    replaced_once(&s, &job).await;
}

#[tokio::test]
async fn killed_while_cleaning_up_removes_what_is_left() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    let left = done_before_clean_up(&s, &plan).await;
    // The old copy aside went; its protective copy did not yet.
    std::fs::remove_file(s.at(&left.aside)).unwrap();
    assert!(s.at(&left.removal_temp).exists());
    run(&s).await;
    replaced_once(&s, &job).await;
}

#[tokio::test]
async fn a_job_held_before_its_clean_up_keeps_its_replacement_done() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    let left = done_before_clean_up(&s, &plan).await;
    s.store
        .hold_stuck(&job, "여러 번 끊겼어요".to_owned(), 9)
        .await
        .unwrap();
    assert_eq!(versions(&s, &job).await[0].1, PlanState::Done);
    assert_eq!(
        s.store.plan(&job).await.unwrap()[0].outcome,
        Some(Outcome::Applied)
    );
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_file_effects WHERE state = 'held'")
            .await,
        0
    );
    // Its leftovers wait for the clean-up of the job's next run.
    assert!(s.at(&left.aside).exists());
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'pending' WHERE id = '{job}'"
    ))
    .await;
    run(&s).await;
    replaced_once(&s, &job).await;
}

#[tokio::test]
async fn killed_after_importing_before_its_record_records_the_import_once() {
    let s = setup().await;
    let (job, plan) = unmanaged_waiting(&s).await;
    decide(&s, &job, &plan, true).await;
    // The new copy and the protective copy prepared, the protective copy
    // imported as the unknown creator's and published; nothing recorded it.
    let new = fake::ass("Show-02");
    let imported = ".trss/subtitles/제작자 알 수 없음/Show S01E02.ass";
    let apply_object = write(&s, ".trss/tmp/a1", &new);
    let removal_object = write(&s, ".trss/tmp/r1", MINE.as_bytes());
    let import_object = write(&s, imported, MINE.as_bytes());
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'running' WHERE id = '{job}'"
    ))
    .await;
    #[rustfmt::skip]
    let effects = [
        ("e-apply", "apply", ".trss/tmp/a1", TARGET, new.clone(), apply_object),
        ("e-remove", "remove", ".trss/tmp/r1", ".trss/tmp/r1.aside", MINE.as_bytes().to_vec(), removal_object),
        ("e-import", "import", ".trss/tmp/i1", imported, MINE.as_bytes().to_vec(), import_object),
    ];
    for (id, kind, temp, target, bytes, object) in effects {
        left_effect(
            &s,
            &plan,
            id,
            kind,
            TARGET,
            "prepared",
            temp,
            target,
            &bytes,
            Some(object),
        )
        .await;
    }
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(TARGET), new);
    assert_eq!(std::fs::read(s.at(imported)).unwrap(), MINE.as_bytes());
    // One stored subtitle of the creator nobody named: the carrying out
    // anew found the recorded import's bytes and reused them.
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_stored WHERE creator IS NULL")
            .await,
        1
    );
    // The import published before its record is the one recorded: nothing
    // else came of it in the unknown creator's folder.
    let unknown = s.at(".trss/subtitles/제작자 알 수 없음");
    assert_eq!(names(&unknown), ["Show S01E02.ass"]);
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_assets
              WHERE relative_path = '.trss/subtitles/제작자 알 수 없음/Show S01E02.ass'"
        )
        .await,
        1
    );
    assert_eq!(s.temps(), Vec::<String>::new());
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_file_effects
              WHERE plan_id IS NOT NULL AND state NOT IN ('done', 'abandoned')"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn killed_after_setting_an_earlier_applied_copy_aside_before_its_record_ends_the_replacement()
{
    let s = setup().await;
    let (job, plan) = ass_waiting_beside_srt(&s).await;
    decide(&s, &job, &plan, true).await;
    // The new copy published and recorded; the applied `.srt` moved aside
    // after its protective copy, and nothing recorded the move.
    let (new, old) = (fake::ass("Show-02"), s.read(SRT));
    let publish = write(&s, TARGET, &new);
    let protective = write(&s, ".trss/tmp/r1", &old);
    std::fs::rename(s.at(SRT), s.at(".trss/tmp/r1.aside")).unwrap();
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'running' WHERE id = '{job}'"
    ))
    .await;
    left_effect(
        &s,
        &plan,
        "e-apply",
        "apply",
        TARGET,
        "prepared",
        ".trss/tmp/a1",
        TARGET,
        &new,
        Some(publish.clone()),
    )
    .await;
    left_effect(
        &s,
        &plan,
        "e-remove",
        "remove",
        SRT,
        "prepared",
        ".trss/tmp/r1",
        ".trss/tmp/r1.aside",
        &old,
        Some(protective),
    )
    .await;
    recorded_applied(&s, &plan, &new, publish).await;
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(versions(&s, &job).await[0].1, PlanState::Done);
    assert!(!s.at(SRT).exists());
    assert_eq!(s.read(TARGET), new);
    // The `.srt` is recorded removed, the new copy is the one applied.
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_applied WHERE removed_at IS NULL")
            .await,
        1
    );
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_applied
              WHERE path = 'Season 01/Show S01E02.srt' AND removed_at IS NOT NULL"
        )
        .await,
        1
    );
    assert_eq!(s.temps(), Vec::<String>::new());
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_file_effects
              WHERE plan_id IS NOT NULL AND state NOT IN ('done', 'abandoned')"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn an_old_copy_set_aside_that_is_not_the_one_compared_is_held_with_its_copies() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    let left = killed(&s, &plan, "prepared", "set_aside").await;
    // Something else is aside now.
    std::fs::write(s.at(&left.aside), b"not it").unwrap();
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Held,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(versions(&s, &job).await[0].1, PlanState::Held);
    // Nothing published, every file kept for a person to look at.
    assert!(!s.at(TARGET).exists());
    assert_eq!(s.read(&left.removal_temp), fake::ass("Show-02"));
    assert!(s.at(&left.apply_temp).exists());
    assert!(s.at(&left.aside).exists());
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_file_effects WHERE state = 'held'")
            .await,
        2
    );
}

#[tokio::test]
async fn a_job_held_while_its_old_copy_is_aside_holds_the_plan_and_its_effects() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    killed(&s, &plan, "prepared", "set_aside").await;
    s.store
        .hold_stuck(&job, "여러 번 끊겼어요".to_owned(), 9)
        .await
        .unwrap();
    assert_eq!(versions(&s, &job).await[0].1, PlanState::Held);
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_file_effects WHERE state = 'held'")
            .await,
        2
    );
    assert_eq!(
        s.store.plan(&job).await.unwrap()[0].outcome,
        Some(Outcome::Held)
    );
}
