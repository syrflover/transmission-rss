//! Replacing the subtitle an episode has once a person approves it
//! (`docs/requirements/subtitles.md`, 교체 비교와 승인 and 승인 증거와 반영 직전
//! 검사; 4-store-apply-and-replace/0068).

use crate::{
    world::{cut_and_restart, Base, Shows},
    Handles,
};
use std::{
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use tokio_util::sync::CancellationToken;
use trss_core::{files::testing, Db};
use trss_jobs::{
    model::{Chosen, Outcome, PathAction, PlanState},
    place::replace::{ADOPTED, AWAITING_APPROVAL},
    place::{
        records::{StoredChoice, StoredOnly},
        replace::records::{Compared, Decided, Plan, PlanView},
    },
    store::{JobDetail, DECIDED},
    Created, JobState, NewItem, NewJob, Runner, StepKind, StepState, Wait,
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
    store: Handles,
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

    /// What is left in the work folder's temporary folders: the staging
    /// folder of new copies, what a replacement moves aside, and the
    /// folder earlier releases used for both.
    fn temps(&self) -> Vec<String> {
        let mut left = names(&self.at(".trss/tmp"));
        left.extend(names(&self.at(".trss/staging")));
        left.extend(names(&self.at(".trss/aside")));
        left
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

/// A library with the work `Show` (season 1) whose episode 2 has a video.
async fn setup() -> Setup {
    let base = Base::new().await;
    base.library(&Shows {
        episodes: vec![2],
        ..Shows::default()
    })
    .await;
    let runner = base.runner(Sources::none().with_fake(FakeSource));
    Setup {
        dir: base.dir,
        db: base.db,
        store: base.store,
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
        movie: None,
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
    let mut views = s.store.place.replacements(job).await.unwrap();
    assert_eq!(views.len(), 1, "{views:?}");
    views.remove(0)
}

async fn decide(s: &Setup, job: &str, plan: &Plan, replace: bool) -> Decided {
    s.store
        .place
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
    let original = inode(&s.at(TARGET));
    assert_eq!(path.file.as_ref().unwrap().lines, Some(24));
    assert_eq!(plan.asset_lines, Some(24));
    let v = view(&s, &job).await;
    assert_eq!(v.previous, None);
    assert_eq!(v.new.as_ref().unwrap().creator.as_deref(), Some(CREATOR));
    assert_eq!(v.applied.len(), 1);
    // The row waits, the job's other steps are done, nothing changed yet.
    assert_eq!(s.store.place.plan(&job).await.unwrap()[0].outcome, None);
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
        s.store.place.plan(&job).await.unwrap()[0].outcome,
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
    // The new copy took the original's place in one swap, the original going
    // into `.trss/aside/`, which is empty again: the original's file is
    // nowhere once the replacement is recorded. No protective copy was made,
    // and nothing staged is left.
    assert_ne!(inode(&s.at(TARGET)), original);
    assert!(s.at(".trss/aside").is_dir());
    assert!(with_inode(&s.work(), original).is_empty());
    assert!(!s.at(".trss/tmp").exists());
    assert_eq!(s.temps(), Vec::<String>::new());
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
        s.store.place.plan(&job).await.unwrap()[0].outcome,
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
            .place
            .decide_replacement(&job, &plan.id, plan.version + 1, true, 1)
            .await
            .unwrap(),
        Decided::Stale
    );
    assert_eq!(
        s.store
            .place
            .decide_replacement(&other, &plan.id, plan.version, true, 1)
            .await
            .unwrap(),
        Decided::NotFound
    );
    assert_eq!(
        s.store
            .place
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
async fn an_import_finds_the_file_an_import_cut_short_published_and_records_it() {
    let s = setup().await;
    let (job, plan) = unmanaged_waiting(&s).await;
    decide(&s, &job, &plan, true).await;
    // The bytes beside the video are under the import's name already, with
    // no record: what an import cut between publishing and recording leaves.
    let dir = s.at(".trss/subtitles/제작자 알 수 없음");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Show S01E02.ass"), MINE.as_bytes()).unwrap();
    let inode = std::fs::metadata(dir.join("Show S01E02.ass"))
        .unwrap()
        .ino();
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
    // That file is the import, as it is: no numbered copy.
    assert_eq!(names(&dir), ["Show S01E02.ass"]);
    assert_eq!(
        std::fs::metadata(dir.join("Show S01E02.ass"))
            .unwrap()
            .ino(),
        inode
    );
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_stored st
               JOIN subtitle_assets a ON a.id = st.subtitle_asset_id
              WHERE st.creator IS NULL AND st.episode = 2
                AND a.relative_path = '.trss/subtitles/제작자 알 수 없음/Show S01E02.ass'"
        )
        .await,
        1
    );
    assert_eq!(s.temps(), Vec::<String>::new());
}

#[tokio::test]
async fn a_jobs_first_apply_beside_the_same_bytes_records_that_file_as_the_applied_copy() {
    let s = setup().await;
    // A subtitle the app did not manage, with the bytes the job receives.
    std::fs::write(s.at(TARGET), fake::ass("Show-02")).unwrap();
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
    assert_eq!(s.read(TARGET), fake::ass("Show-02"), "left as it is");
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_replacements").await,
        0
    );
    // The file is the stored copy's applied copy, the row says so, and the
    // job's apply step has nothing else to say.
    let rows = s.store.place.plan(&job).await.unwrap();
    assert_eq!(rows[0].outcome, Some(Outcome::Applied));
    assert_eq!(rows[0].note.as_deref(), Some(ADOPTED));
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_applied
              WHERE removed_at IS NULL AND path = 'Season 01/Show S01E02.ass'
                AND stored_id = (SELECT stored_id FROM subtitle_job_plan)
                AND id = (SELECT applied_id FROM subtitle_job_plan)"
        )
        .await,
        1
    );
    let apply = d.steps.iter().find(|s| s.step == StepKind::Apply).unwrap();
    assert_eq!(
        (apply.state, apply.note.as_deref()),
        (StepState::Done, None)
    );
    assert!(d.events.iter().any(|e| e.message.contains(ADOPTED)));
}

#[tokio::test]
async fn other_bytes_beside_the_video_still_wait_for_approval() {
    let s = setup().await;
    std::fs::write(s.at(TARGET), MINE).unwrap();
    let job = make(&s, "c1", CREATOR, "/ok/Show-02").await;
    run(&s).await;
    waiting_for_approval(&detail(&s, &job).await);
    assert_eq!(s.read(TARGET), MINE.as_bytes());
    assert_eq!(s.count("SELECT count(*) FROM subtitle_applied").await, 0);
}

/// `MINE` at `TARGET` replaced by a job's `/ok/Show-02` once approved: the
/// job, done, and the stored subtitle the replacement imported `MINE` as.
async fn imported_by_a_replacement(s: &Setup) -> (String, String) {
    let (job, plan) = unmanaged_waiting(s).await;
    decide(s, &job, &plan, true).await;
    run(s).await;
    assert_eq!(detail(s, &job).await.row.state, JobState::Done);
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    let imported =
        s.db.run(|c| {
            c.query_row(
                "SELECT id FROM subtitle_stored WHERE creator IS NULL",
                [],
                |r| r.get::<_, String>(0),
            )
            .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    (job, imported)
}

#[tokio::test]
async fn an_imported_copy_on_an_episode_with_a_subtitle_is_compared_then_put_back_beside_the_video()
{
    let s = setup().await;
    let (job, imported) = imported_by_a_replacement(&s).await;
    let replaced = s.store.place.plan(&job).await.unwrap()[0]
        .stored_id
        .clone()
        .unwrap();

    // The 자막 card offers it a comparison, as any stored copy, and the
    // episode's row offers it too.
    let copies = s.store.place.work_copies(WORK).await.unwrap();
    let copy = copies.iter().find(|c| c.id == imported).expect("listed");
    assert_eq!(
        (
            copy.creator.as_deref(),
            copy.options.applied,
            &copy.options.apply
        ),
        (None, false, &Ok(true))
    );
    let only = s.store.place.stored_only(WORK).await.unwrap();
    let only = only.iter().find(|c| c.id == imported).expect("listed");
    assert_eq!(
        (only.job_id.as_deref(), only.compare),
        (Some(job.as_str()), true)
    );

    // Choosing it asks the job that imported it for a comparison.
    assert_eq!(
        s.store
            .place
            .choose_stored(WORK, &imported, Chosen::Apply, 5_000_000)
            .await
            .unwrap(),
        StoredChoice::Queued {
            job_id: job.clone(),
            compare: true
        }
    );
    run(&s).await;
    waiting_for_approval(&detail(&s, &job).await);
    let rows = s.store.place.plan(&job).await.unwrap();
    assert_eq!(rows.len(), 2, "the import's row joined the job's plan");
    assert_eq!(rows[1].chosen, Some(Chosen::Apply));
    // While the job waits for the approval, the copy is not offered again.
    let copies = s.store.place.work_copies(WORK).await.unwrap();
    let copy = copies.iter().find(|c| c.id == imported).expect("listed");
    assert_eq!(
        copy.options.apply,
        Err("이 보관본을 받은 작업이 교체 승인을 기다리고 있어요. 작업 상세에서 정해 주세요.")
    );
    // The plan is the replacement of the applied copy by the imported bytes;
    // nothing beside the video changed yet.
    let v = s
        .store
        .place
        .replacements(&job)
        .await
        .unwrap()
        .into_iter()
        .find(|v| v.plan.stored_id == imported)
        .expect("its plan");
    assert_eq!(actions(&v.plan), [(TARGET, PathAction::Replace)]);
    assert!(v.plan.paths[0].applied_id.is_some());
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));

    decide(&s, &job, &v.plan, true).await;
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(TARGET), MINE.as_bytes());
    // The replaced applied copy is its own stored copy still, which can be
    // chosen again; the imported one is the applied copy now.
    assert_eq!(
        s.read(".trss/subtitles/제작자/Show-02.ass"),
        fake::ass("Show-02")
    );
    let copies = s.store.place.work_copies(WORK).await.unwrap();
    let find = |id: &str| copies.iter().find(|c| c.id == id).expect("listed");
    assert!(find(&imported).options.applied);
    assert_eq!(find(&imported).applied.len(), 1);
    assert_eq!(find(&replaced).options.apply, Ok(true));
    assert!(!find(&replaced).options.applied);
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_stored WHERE creator IS NULL")
            .await,
        1,
        "nothing was imported again"
    );
    assert_eq!(s.temps(), Vec::<String>::new());
}

#[tokio::test]
async fn an_imported_copy_on_an_episode_with_no_subtitle_is_applied_at_once() {
    let s = setup().await;
    let (job, imported) = imported_by_a_replacement(&s).await;
    // The episode has no subtitle any more: the applied copy was removed.
    std::fs::remove_file(s.at(TARGET)).unwrap();
    s.sql("UPDATE subtitle_applied SET removed_at = 1".to_owned())
        .await;
    let plans = s.count("SELECT count(*) FROM subtitle_replacements").await;

    let copies = s.store.place.work_copies(WORK).await.unwrap();
    let copy = copies.iter().find(|c| c.id == imported).expect("listed");
    assert_eq!(copy.options.apply, Ok(false));
    assert_eq!(
        s.store
            .place
            .choose_stored(WORK, &imported, Chosen::Apply, 5_000_000)
            .await
            .unwrap(),
        StoredChoice::Queued {
            job_id: job.clone(),
            compare: false
        }
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
    assert_eq!(s.read(TARGET), MINE.as_bytes());
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_replacements").await,
        plans,
        "no comparison"
    );
    let copies = s.store.place.work_copies(WORK).await.unwrap();
    let copy = copies.iter().find(|c| c.id == imported).expect("listed");
    assert!(copy.options.applied);
    assert_eq!(copy.applied.len(), 1);
}

#[tokio::test]
async fn an_imported_copy_whose_job_is_gone_stays_blocked_with_the_reason() {
    let s = setup().await;
    let (_, imported) = imported_by_a_replacement(&s).await;
    s.sql("UPDATE subtitle_stored SET job_id = NULL WHERE creator IS NULL".to_owned())
        .await;
    let copies = s.store.place.work_copies(WORK).await.unwrap();
    let copy = copies.iter().find(|c| c.id == imported).expect("listed");
    assert_eq!(
        copy.options.apply,
        Err("이 보관본을 받은 작업의 기록이 없어 적용할 수 없어요.")
    );
    assert_eq!(
        s.store
            .place
            .choose_stored(WORK, &imported, Chosen::Apply, 5_000_000)
            .await
            .unwrap(),
        StoredChoice::Refused("이 보관본을 받은 작업의 기록이 없어 적용할 수 없어요.")
    );
}

/// The plan of `stored` that `job` waits on a person for.
async fn plan_of(s: &Setup, job: &str, stored: &str) -> Plan {
    s.store
        .place
        .replacements(job)
        .await
        .unwrap()
        .into_iter()
        .find(|v| v.plan.stored_id == stored)
        .expect("its plan")
        .plan
}

#[tokio::test]
async fn a_copy_kept_off_the_episode_leaves_its_row_until_chosen_again_or_the_subtitle_goes() {
    let s = setup().await;
    let (job, imported) = imported_by_a_replacement(&s).await;
    let on_row = |only: &[StoredOnly]| only.iter().find(|c| c.id == imported).cloned();
    let choose = || {
        s.store
            .place
            .choose_stored(WORK, &imported, Chosen::Apply, 5_000_000)
    };

    // Compared and kept: the episode keeps its subtitle, and the row no longer
    // offers the copy; the 자막 card still does.
    choose().await.unwrap();
    run(&s).await;
    let plan = plan_of(&s, &job, &imported).await;
    assert_eq!(
        decide(&s, &job, &plan, false).await,
        Decided::Done(PlanState::Kept)
    );
    run(&s).await;
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    assert_eq!(
        on_row(&s.store.place.stored_only(WORK).await.unwrap()),
        None
    );
    let copies = s.store.place.work_copies(WORK).await.unwrap();
    let copy = copies.iter().find(|c| c.id == imported).expect("listed");
    assert_eq!(copy.options.apply, Ok(true));

    // Chosen again from the card: the row shows it waiting for the approval.
    choose().await.unwrap();
    run(&s).await;
    waiting_for_approval(&detail(&s, &job).await);
    let row = on_row(&s.store.place.stored_only(WORK).await.unwrap()).expect("on the row");
    assert_eq!(row.awaiting_approval.as_deref(), Some(job.as_str()));

    // Kept again, then the episode loses its subtitle: nothing is kept over
    // any more, and the row offers the copy to apply.
    let plan = plan_of(&s, &job, &imported).await;
    assert_eq!(
        decide(&s, &job, &plan, false).await,
        Decided::Done(PlanState::Kept)
    );
    run(&s).await;
    assert_eq!(
        on_row(&s.store.place.stored_only(WORK).await.unwrap()),
        None
    );
    std::fs::remove_file(s.at(TARGET)).unwrap();
    s.sql("UPDATE subtitle_applied SET removed_at = 1".to_owned())
        .await;
    let row = on_row(&s.store.place.stored_only(WORK).await.unwrap()).expect("on the row");
    assert!(!row.compare);
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
async fn a_subtitle_changed_right_before_the_swap_is_swapped_back_and_the_new_copy_goes() {
    let s = setup().await;
    let (job, _) = revision_approved(&s).await;
    let original = inode(&s.at(TARGET));
    let stored = names(&s.at(&format!(".trss/subtitles/{CREATOR}")));
    // A person edits the subtitle after every check of the run, right before
    // the swap: the swap takes the edited file off.
    let mut edited = fake::ass("Show-02");
    edited.extend_from_slice(b"Dialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,edited\n");
    let (at, bytes) = (s.at(TARGET), edited.clone());
    let _edit =
        testing::before_rename_of(&s.at(TARGET), move || std::fs::write(at, bytes).unwrap());
    run(&s).await;

    // Swapped back: the edited file is in its place, the same file, and the
    // new copy the swap back put aside is gone, as is anything staged.
    assert_eq!(s.read(TARGET), edited);
    assert_eq!(inode(&s.at(TARGET)), original);
    assert_eq!(s.temps(), Vec::<String>::new());
    // The plan is stale as for a file changed after approval, and nothing
    // else changed: the earlier applied copy's record and the stored copies
    // are as they were.
    let v = versions(&s, &job).await;
    assert_eq!(v[0].1, PlanState::Stale, "{v:?}");
    assert!(
        v[0].2
            .as_deref()
            .is_some_and(|r| r.contains("기존 자막이 비교한 뒤 바뀌었어요")),
        "{v:?}"
    );
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_applied WHERE removed_at IS NULL")
            .await,
        1
    );
    assert_eq!(s.count("SELECT count(*) FROM subtitle_applied").await, 1);
    assert_eq!(names(&s.at(&format!(".trss/subtitles/{CREATOR}"))), stored);
    // The person compares again, with the edited file not the app's.
    waiting_for_approval(&detail(&s, &job).await);
    let next = view(&s, &job).await.plan;
    assert_eq!(next.version, 2);
    assert_eq!(next.paths[0].applied_id, None);
}

#[tokio::test]
async fn an_unmanaged_subtitle_changed_right_before_the_swap_stays_and_its_import_is_kept() {
    let s = setup().await;
    let (job, plan) = unmanaged_waiting(&s).await;
    decide(&s, &job, &plan, true).await;
    // The person edits their subtitle after the import, right before the
    // swap.
    let edited = format!("{MINE}Dialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,edited\n");
    let (at, bytes) = (s.at(TARGET), edited.clone());
    let _edit =
        testing::before_rename_of(&s.at(TARGET), move || std::fs::write(at, bytes).unwrap());
    run(&s).await;

    // Swapped back and stale: the edited file is in its place, nothing is
    // applied, and the bytes imported before the swap, the approved ones,
    // stay stored as the unknown creator's: a copy more.
    assert_eq!(s.read(TARGET), edited.as_bytes());
    assert_eq!(s.temps(), Vec::<String>::new());
    let v = versions(&s, &job).await;
    assert_eq!(v[0].1, PlanState::Stale, "{v:?}");
    assert_eq!(s.count("SELECT count(*) FROM subtitle_applied").await, 0);
    assert_eq!(
        s.read(".trss/subtitles/제작자 알 수 없음/Show S01E02.ass"),
        MINE.as_bytes()
    );
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_stored WHERE creator IS NULL")
            .await,
        1
    );
    waiting_for_approval(&detail(&s, &job).await);
}

/// A recorded object as a file system mounted again (after the machine
/// restarted) shows the same file: another device number, the same inode.
fn renumbered(column: &str) -> String {
    format!("'999999' || substr({column}, instr({column}, ':'))")
}

/// Rewrites the objects the job's plans recorded to [`renumbered`] ones,
/// past the triggers that keep a plan's evidence: `which` of the plans'
/// videos (`video`) and the files beside them (`paths`).
async fn mounted_again(s: &Setup, job: &str, which: &'static [&'static str]) {
    let job = job.to_owned();
    s.db.run(move |c| -> Result<(), trss_core::DbError> {
        let fixed = [
            "subtitle_replacements_evidence_fixed",
            "subtitle_replacement_paths_fixed",
        ];
        let mut triggers = Vec::new();
        for name in fixed {
            triggers.push(c.query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
                [name],
                |r| r.get::<_, String>(0),
            )?);
        }
        let tx = c.transaction()?;
        for name in fixed {
            tx.execute_batch(&format!("DROP TRIGGER {name}"))?;
        }
        for what in which {
            let sql = match *what {
                "video" => format!(
                    "UPDATE subtitle_replacements SET video_object = {} WHERE job_id = ?1",
                    renumbered("video_object")
                ),
                "paths" => format!(
                    "UPDATE subtitle_replacement_paths SET object = {}
                      WHERE object IS NOT NULL
                        AND plan_id IN (SELECT id FROM subtitle_replacements WHERE job_id = ?1)",
                    renumbered("object")
                ),
                other => panic!("no such record: {other}"),
            };
            tx.execute(&sql, [&job])?;
        }
        for sql in &triggers {
            tx.execute_batch(sql)?;
        }
        tx.commit()?;
        Ok(())
    })
    .await
    .unwrap();
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
async fn no_room_for_the_new_copy_stops_before_anything_beside_the_video_changes() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    // The new copy is the only one written: no room for it fails the plan.
    let staging = s.at(".trss/staging");
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o555)).unwrap();
    run(&s).await;
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755)).unwrap();
    let v = versions(&s, &job).await;
    assert_eq!(v[0].1, PlanState::Failed, "{v:?}");
    assert!(
        v[0].2
            .as_deref()
            .is_some_and(|r| r.contains("새 자막을 준비하지 못해 교체하지 않았어요")),
        "{v:?}"
    );
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    assert_eq!(s.temps(), Vec::<String>::new());
    assert_eq!(
        s.store.place.plan(&job).await.unwrap()[0].outcome,
        Some(Outcome::Failed)
    );
    let _ = plan;
}

#[tokio::test]
async fn an_unmanaged_subtitle_that_cannot_be_imported_fails_the_plan_before_anything_changes() {
    let s = setup().await;
    let (job, plan) = unmanaged_waiting(&s).await;
    let original = inode(&s.at(TARGET));
    decide(&s, &job, &plan, true).await;
    // The unknown creator's folder cannot be read, so no name can be chosen
    // for the import.
    write(&s, ".trss/subtitles/제작자 알 수 없음", b"not a folder");
    run(&s).await;
    let v = versions(&s, &job).await;
    assert_eq!(v[0].1, PlanState::Failed, "{v:?}");
    assert!(
        v[0].2
            .as_deref()
            .is_some_and(|r| r.contains("관리하지 않던 자막을 들이지 못해 교체하지 않았어요")),
        "{v:?}"
    );
    // The import comes before the swap: the file never left its place,
    // nothing went aside, and the staged new copy is gone.
    assert_eq!(s.read(TARGET), MINE.as_bytes());
    assert_eq!(inode(&s.at(TARGET)), original);
    assert!(!s.at(".trss/aside").exists());
    assert_eq!(s.temps(), Vec::<String>::new());
    assert_eq!(s.count("SELECT count(*) FROM subtitle_applied").await, 0);
    assert_eq!(
        s.store.place.plan(&job).await.unwrap()[0].outcome,
        Some(Outcome::Failed)
    );
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
            .run
            .settle(
                &job,
                JobState::Waiting,
                Some(Wait::Approval),
                Some(AWAITING_APPROVAL.to_owned()),
                6_000_000,
            )
            .await
            .unwrap();
        assert_eq!(back, Some(DECIDED), "{replace}");
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
        .run
        .settle(
            &job,
            JobState::Waiting,
            Some(Wait::Approval),
            Some(AWAITING_APPROVAL.to_owned()),
            6_000_000,
        )
        .await
        .unwrap();
    assert_eq!(back, None);
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
        s.store.place.plan(&job).await.unwrap()[0].outcome,
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
        s.store.place.plan(&job).await.unwrap()[0].outcome,
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
// A run cut short while carrying out an approved plan
//
// Nothing records a replacement's files before its transaction: each test
// leaves the work folder as a run cut at one point would, cuts the job and
// restarts it, then runs it again.

/// The files in the work folder's `.trss/aside/`, by name.
fn asides(s: &Setup) -> Vec<String> {
    names(&s.at(".trss/aside"))
}

fn inode(path: &Path) -> u64 {
    std::fs::metadata(path).unwrap().ino()
}

/// The paths under `dir` whose file has the inode `ino`.
fn with_inode(dir: &Path, ino: u64) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let meta = std::fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            found.extend(with_inode(&path, ino));
        } else if meta.ino() == ino {
            found.push(path);
        }
    }
    found
}

/// Writes `bytes` to the new file `path` (its folders made).
fn write(s: &Setup, path: &str, bytes: &[u8]) {
    let at = s.at(path);
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(&at, bytes).unwrap();
}

/// The job, its plan approved, cut short in the middle of its run and
/// restarted by a person.
async fn cut(s: &Setup, job: &str) {
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'running' WHERE id = '{job}'"
    ))
    .await;
    cut_and_restart(&s.store, job).await;
}

/// The row ended as the comparison ends a first apply beside the same
/// bytes: the new copy at `TARGET`, recorded as the applied copy
/// ([`ADOPTED`]) once, the plan stale, the job done.
async fn adopted_after_the_cut(s: &Setup, job: &str, new: &[u8]) {
    let d = detail(s, job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(TARGET), new);
    let rows = s.store.place.plan(job).await.unwrap();
    assert_eq!(rows[0].outcome, Some(Outcome::Applied));
    assert_eq!(rows[0].note.as_deref(), Some(ADOPTED));
    let v = versions(s, job).await;
    assert_eq!(v.len(), 1, "{v:?}");
    assert_eq!(v[0].1, PlanState::Stale, "{v:?}");
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_applied
              WHERE removed_at IS NULL AND path = 'Season 01/Show S01E02.ass'
                AND id = (SELECT applied_id FROM subtitle_job_plan
                           WHERE job_id = (SELECT job_id FROM subtitle_replacements))"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn a_replacement_cut_before_its_swap_is_carried_out_anew_and_leaves_old_folders_alone() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    let original = inode(&s.at(TARGET));
    let new = fake::ass("Show-02v2");
    // What the cut run left: half a staged copy, and its whole copy waiting
    // in `.trss/aside/` for the swap. An earlier release's set-aside
    // original in `.trss/tmp/` is there too.
    write(
        &s,
        ".trss/staging/3c9a3f7e-8f0e-4f5a-9d1c-2b6d9e0f1a11",
        &new[..new.len() / 2],
    );
    let waiting = ".trss/aside/7b1e0c55-4f7a-4d38-a0b9-6c2d1e9f3a22";
    write(&s, waiting, &new);
    write(&s, ".trss/tmp/r1.aside", b"an earlier release's original");
    cut(&s, &job).await;
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
    assert_eq!(s.read(TARGET), new);
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_applied WHERE removed_at IS NULL")
            .await,
        1
    );
    // The staging folder was emptied; what the cut left aside stays and the
    // run's own files there went; the old release's folder is untouched.
    assert_eq!(names(&s.at(".trss/staging")), Vec::<String>::new());
    assert_eq!(asides(&s), [file_name(waiting)]);
    assert_eq!(s.read(waiting), new);
    assert!(with_inode(&s.work(), original).is_empty());
    assert_eq!(names(&s.at(".trss/tmp")), ["r1.aside"]);
    assert_eq!(
        s.read(".trss/tmp/r1.aside"),
        b"an earlier release's original"
    );
    let _ = plan;
}

#[tokio::test]
async fn a_replacement_cut_after_importing_the_original_imports_it_once_and_replaces_once() {
    let s = setup().await;
    let (job, plan) = unmanaged_waiting(&s).await;
    decide(&s, &job, &plan, true).await;
    let original = inode(&s.at(TARGET));
    let import = ".trss/subtitles/제작자 알 수 없음/Show S01E02.ass";
    // The run is cut right before its swap: what the import and the path
    // held then is kept, and the run stops there as a stopped process would.
    // The import's bytes, if any, and the path's.
    type Seen = Option<(Option<Vec<u8>>, Vec<u8>)>;
    let seen: Arc<Mutex<Seen>> = Arc::default();
    let (at_import, at_target, held) = (s.at(import), s.at(TARGET), seen.clone());
    let _cut = testing::before_rename_of(&s.at(TARGET), move || {
        let found = (
            std::fs::read(at_import).ok(),
            std::fs::read(at_target).unwrap(),
        );
        *held.lock().unwrap() = Some(found);
        panic!("the run is cut right before its swap");
    });
    let runner = s.runner.clone();
    let cut_short = tokio::spawn(async move { runner.run_ready(&CancellationToken::new()).await });
    assert!(cut_short.await.unwrap_err().is_panic());

    // The original was imported, and recorded, while it was still at its
    // path: only the swap takes it off.
    let found = seen.lock().unwrap().take();
    assert_eq!(
        found,
        Some((Some(MINE.as_bytes().to_vec()), MINE.as_bytes().to_vec()))
    );
    let unknown = "SELECT count(*) FROM subtitle_stored WHERE creator IS NULL AND episode = 2";
    assert_eq!(s.count(unknown).await, 1);
    assert_eq!(inode(&s.at(TARGET)), original);
    let imported = inode(&s.at(import));
    let left = asides(&s);
    assert_eq!(left.len(), 1, "the new copy waiting for the swap");

    cut(&s, &job).await;
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    // Carried out once: the plan done, the new copy applied.
    assert_eq!(versions(&s, &job).await.len(), 1);
    assert_eq!(versions(&s, &job).await[0].1, PlanState::Done);
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_applied WHERE removed_at IS NULL")
            .await,
        1
    );
    // The import was found, not made again: one stored subtitle, one asset
    // and one file, the one the cut run wrote.
    assert_eq!(s.count(unknown).await, 1);
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_assets
              WHERE relative_path LIKE '.trss/subtitles/제작자 알 수 없음/%'"
        )
        .await,
        1
    );
    let dir = s.at(".trss/subtitles/제작자 알 수 없음");
    assert_eq!(names(&dir), ["Show S01E02.ass"]);
    assert_eq!(inode(&s.at(import)), imported);
    assert_eq!(
        d.events
            .iter()
            .filter(|e| e.message.contains("보관본으로 들였어요"))
            .count(),
        1
    );
    // The original went once the replacement was recorded; the cut run's
    // new copy stays aside, and nothing staged is left.
    assert!(with_inode(&s.work(), original).is_empty());
    assert_eq!(asides(&s), left);
    assert_eq!(names(&s.at(".trss/staging")), Vec::<String>::new());
}

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap().to_owned()
}

#[tokio::test]
async fn a_replacement_cut_after_its_swap_records_the_new_copy_and_keeps_the_original_aside() {
    let s = setup().await;
    let (job, _) = revision_approved(&s).await;
    let original = inode(&s.at(TARGET));
    let new = fake::ass("Show-02v2");
    // The swap done, nothing recorded: the new copy at the target and the
    // original, the same file, in `.trss/aside/`.
    let aside = ".trss/aside/0e4c2d9b-6a51-4f0e-8b7d-5f3a2c1e9d33";
    write(&s, ".trss/aside/new", &new);
    std::fs::create_dir_all(s.at(".trss/aside")).unwrap();
    std::fs::rename(s.at(TARGET), s.at(aside)).unwrap();
    std::fs::rename(s.at(".trss/aside/new"), s.at(TARGET)).unwrap();
    cut(&s, &job).await;
    run(&s).await;

    adopted_after_the_cut(&s, &job, &new).await;
    let v = versions(&s, &job).await;
    assert!(
        v[0].2
            .as_deref()
            .is_some_and(|r| r.contains("기존 자막이 비교한 뒤 바뀌었어요")),
        "{v:?}"
    );
    // The earlier applied copy, whose file is not at its path any more, is
    // recorded removed; its file stays aside, never removed.
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_applied WHERE removed_at IS NOT NULL")
            .await,
        1
    );
    assert_eq!(asides(&s), [file_name(aside)]);
    assert_eq!(inode(&s.at(aside)), original);
    assert_eq!(s.read(aside), fake::ass("Show-02"));
}

#[tokio::test]
async fn a_replacement_cut_after_taking_off_an_earlier_copy_leaves_that_file_aside() {
    let s = setup().await;
    let (job, plan) = ass_waiting_beside_srt(&s).await;
    decide(&s, &job, &plan, true).await;
    let new = fake::ass("Show-02");
    let srt = s.read(SRT);
    let srt_inode = inode(&s.at(SRT));
    // The new copy published at the target, the applied `.srt` moved
    // aside; nothing recorded.
    let aside = ".trss/aside/5a8f1b2c-3d4e-4f60-9a7b-8c9d0e1f2a44";
    write(&s, TARGET, &new);
    std::fs::create_dir_all(s.at(".trss/aside")).unwrap();
    std::fs::rename(s.at(SRT), s.at(aside)).unwrap();
    cut(&s, &job).await;
    run(&s).await;

    adopted_after_the_cut(&s, &job, &new).await;
    let v = versions(&s, &job).await;
    assert!(
        v[0].2
            .as_deref()
            .is_some_and(|r| r.contains("새 자막을 둘 자리에 파일이 생겼어요")),
        "{v:?}"
    );
    // The `.srt` stays aside as it was, and nothing puts it anywhere else.
    // Its applied record is not ended: nothing recorded that the cut run
    // took it off, and the next comparison of the episode plans it.
    assert!(!s.at(SRT).exists());
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_applied
              WHERE path = 'Season 01/Show S01E02.srt' AND removed_at IS NULL"
        )
        .await,
        1
    );
    assert_eq!(asides(&s), [file_name(aside)]);
    assert_eq!(s.read(aside), srt);
    assert_eq!(inode(&s.at(aside)), srt_inode);
}

#[tokio::test]
async fn a_replacement_cut_after_its_records_leaves_its_files_aside() {
    let s = setup().await;
    let (job, _) = revision_approved(&s).await;
    // The swap done and recorded as the run's transaction records it: the
    // new copy applied, the earlier one removed, the plan done. The run was
    // cut before it removed the original it moved aside.
    let aside = ".trss/aside/9d2e3f4a-5b6c-4d7e-8f90-a1b2c3d4e555";
    std::fs::create_dir_all(s.at(".trss/aside")).unwrap();
    std::fs::rename(s.at(TARGET), s.at(aside)).unwrap();
    write(&s, TARGET, &fake::ass("Show-02v2"));
    let (stored, plan_id) =
        s.db.run(move |c| {
            c.query_row("SELECT stored_id, id FROM subtitle_replacements", [], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    s.sql(format!(
        "UPDATE subtitle_applied SET removed_at = 2 WHERE removed_at IS NULL;
         INSERT INTO subtitle_applied
             (id, work_id, stored_id, season, episode, video_path, path, byte_size, sha256,
              job_id, applied_at)
         SELECT 'ap2', '{WORK}', '{stored}', 1, 2, '{VIDEO}', '{TARGET}', byte_size, sha256,
                '{job}', 2
           FROM subtitle_assets
          WHERE id = (SELECT subtitle_asset_id FROM subtitle_stored WHERE id = '{stored}');
         UPDATE subtitle_job_plan SET outcome = 'applied', applied_id = 'ap2'
          WHERE job_id = '{job}';
         UPDATE subtitle_replacements SET state = 'done' WHERE id = '{plan_id}';"
    ))
    .await;
    cut(&s, &job).await;
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
    assert_eq!(s.read(TARGET), fake::ass("Show-02v2"));
    assert_eq!(asides(&s), [file_name(aside)]);
    assert_eq!(s.read(aside), fake::ass("Show-02"));
}

#[tokio::test]
async fn a_job_held_for_its_starts_leaves_its_approved_plan_and_its_files() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    // A cut run left the original aside after the swap.
    let aside = ".trss/aside/1f2e3d4c-5b6a-4978-8a9b-0c1d2e3f4a66";
    write(&s, ".trss/aside/new", &fake::ass("Show-02v2"));
    std::fs::rename(s.at(TARGET), s.at(aside)).unwrap();
    std::fs::rename(s.at(".trss/aside/new"), s.at(TARGET)).unwrap();
    s.store
        .run
        .hold_stuck(&job, "여러 번 끊겼어요".to_owned(), 9)
        .await
        .unwrap();
    // Nothing of the replacement is on record to hold: the plan stays
    // approved, and its next run checks it again from the start.
    assert_eq!(versions(&s, &job).await[0].1, PlanState::Approved);
    assert_eq!(s.store.place.plan(&job).await.unwrap()[0].outcome, None);
    assert_eq!(detail(&s, &job).await.row.state, JobState::Held);
    assert_eq!(s.read(aside), fake::ass("Show-02"));
    let _ = plan;
}

#[tokio::test]
async fn an_approved_plan_mounted_again_with_another_device_number_is_carried_out() {
    let s = setup().await;
    let (job, _) = revision_approved(&s).await;
    // The machine restarted before the run: the video and the file beside
    // it have another device number.
    mounted_again(&s, &job, &["video", "paths"]).await;
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
    assert_eq!(s.read(TARGET), fake::ass("Show-02v2"));
}

// ---- the comparison a plan is made with (ticket 4-store-apply-and-replace/0069)

/// The dialogue lines of an ASS as text lines, replaced by `edit`, which gets
/// each one's index and returns its new text (`None` drops the line).
fn edited_ass(bytes: &[u8], edit: impl Fn(usize, &str) -> Option<String>) -> Vec<u8> {
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let mut index = 0;
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        if line.starts_with("Dialogue:") {
            let at = index;
            index += 1;
            if let Some(line) = edit(at, line) {
                out.push_str(&line);
            }
        } else {
            out.push_str(line);
        }
    }
    out.into_bytes()
}

/// `current` beside the video at `path`, then the plan of the fake post `post`
/// to replace it (or go beside it), waiting: the job and its view.
async fn waiting_beside(s: &Setup, path: &str, current: &[u8], post: &str) -> (String, PlanView) {
    write(s, path, current);
    let job = make(s, "c1", CREATOR, post).await;
    run(s).await;
    waiting_for_approval(&detail(s, &job).await);
    let v = view(s, &job).await;
    (job, v)
}

/// What the plan's comparison found, the lines left out.
fn found(v: &PlanView) -> &trss_jobs::place::replace::records::Diff {
    match v.comparison.as_ref().map(|c| &c.result) {
        Some(Compared::Diff(diff)) => diff,
        other => panic!("comparison: {other:?}"),
    }
}

/// Why the plan's comparison was not made.
fn not_made(v: &PlanView) -> &str {
    match v.comparison.as_ref().map(|c| &c.result) {
        Some(Compared::Unreadable(reason)) => reason,
        other => panic!("comparison: {other:?}"),
    }
}

/// The stored lines of the plan's difference.
async fn lines_of(s: &Setup, job: &str, plan: &Plan) -> Option<serde_json::Value> {
    s.store
        .place
        .replacement_lines(job, &plan.id)
        .await
        .unwrap()
        .map(|json| serde_json::from_str(&json).unwrap())
}

#[tokio::test]
async fn an_ass_with_two_lines_added_and_twelve_changed_is_stored_with_its_counts_and_lines() {
    let s = setup().await;
    let new = fake::ass("Show-02");
    // The current file lacks the last two lines and says other things in
    // twelve of the rest.
    let current = edited_ass(&new, |i, line| match i {
        22 | 23 => None,
        2..=13 => Some(line.replace("가짜 자막", "예전 자막")),
        _ => Some(line.to_owned()),
    });
    let (job, v) = waiting_beside(&s, TARGET, &current, "/ok/Show-02").await;

    let diff = found(&v);
    assert_eq!(
        (
            diff.dialogue.added,
            diff.dialogue.changed,
            diff.dialogue.removed
        ),
        (2, 12, 0)
    );
    assert_eq!((diff.old.cues, diff.new.cues), (22, 24));
    assert_eq!(v.comparison.as_ref().unwrap().path, TARGET);
    // The detail carries the counts only; the lines are kept apart.
    assert!(diff.dialogue.lines.is_empty() && diff.timing.lines.is_empty());
    let lines = lines_of(&s, &job, &v.plan).await.unwrap();
    let dialogue = lines["dialogue"].as_array().unwrap();
    assert_eq!(dialogue.len(), 14);
    let kinds = |kind: &str| dialogue.iter().filter(|l| l["kind"] == kind).count();
    assert_eq!(
        (kinds("added"), kinds("changed"), kinds("removed")),
        (2, 12, 0)
    );
    let first_changed = dialogue.iter().find(|l| l["kind"] == "changed").unwrap();
    assert_eq!(first_changed["old"]["text"], "예전 자막 Show-02 3");
    assert_eq!(first_changed["new"]["text"], "가짜 자막 Show-02 3");
    assert_eq!(first_changed["new"]["start"], 4_000);
    assert_eq!(lines["timing"], serde_json::json!([]));
}

#[tokio::test]
async fn an_srt_with_only_its_timing_moved_has_no_dialogue_change_and_counts_its_moved_lines() {
    let s = setup().await;
    let new = String::from_utf8(fake::srt("Show - 02")).unwrap();
    let mut current = new.clone();
    for i in 0..5 {
        let (a, b) = (i * 2, i * 2 + 1);
        current = current.replace(
            &format!("00:00:{a:02},000 --> 00:00:{b:02},500"),
            &format!("00:00:{a:02},300 --> 00:00:{b:02},800"),
        );
    }
    let (job, v) = waiting_beside(&s, SRT, current.as_bytes(), "/pack/Show - 02.srt").await;

    let diff = found(&v);
    assert_eq!(
        (
            diff.dialogue.added,
            diff.dialogue.changed,
            diff.dialogue.removed
        ),
        (0, 0, 0)
    );
    assert_eq!(diff.timing.count, 5);
    assert_eq!((diff.styles.is_none(), diff.fonts.is_none()), (true, true));
    let lines = lines_of(&s, &job, &v.plan).await.unwrap();
    let timing = lines["timing"].as_array().unwrap();
    assert_eq!(timing.len(), 5);
    assert_eq!(
        timing[0]["old"],
        serde_json::json!({"start": 300, "end": 1800})
    );
    assert_eq!(
        timing[0]["new"],
        serde_json::json!({"start": 0, "end": 1500})
    );
    assert_eq!(lines["dialogue"], serde_json::json!([]));
}

#[tokio::test]
async fn an_ass_whose_styles_font_only_changed_has_a_style_change_and_fonts_added_and_removed() {
    let s = setup().await;
    let current = String::from_utf8(fake::ass("Show-02"))
        .unwrap()
        .replace("Style: Default,Arial,", "Style: Default,Noto Sans CJK KR,");
    let (_, v) = waiting_beside(&s, TARGET, current.as_bytes(), "/ok/Show-02").await;

    let diff = found(&v);
    assert_eq!(diff.dialogue.total(), 0);
    let styles = diff.styles.as_ref().unwrap();
    assert_eq!((styles.added.len(), styles.removed.len()), (0, 0));
    assert_eq!(styles.changed.len(), 1);
    assert_eq!(styles.changed[0].name, "Default");
    let fields: Vec<_> = styles.changed[0]
        .fields
        .iter()
        .map(|f| (f.field.as_str(), f.old.as_str(), f.new.as_str()))
        .collect();
    assert_eq!(fields, [("Fontname", "Noto Sans CJK KR", "Arial")]);
    let fonts = diff.fonts.as_ref().unwrap();
    assert_eq!(fonts.added, ["Arial"]);
    assert_eq!(fonts.removed, ["Noto Sans CJK KR"]);
}

#[tokio::test]
async fn a_cp949_smi_and_a_utf8_smi_of_the_same_content_do_not_differ() {
    use trss_jobs::place::replace::records::{Encoding, Side};

    let s = setup().await;
    let text = String::from_utf8(fake::bytes_of("Show - 02.smi")).unwrap();
    let (cp949, _, failed) = encoding_rs::EUC_KR.encode(&text);
    assert!(!failed && cp949.as_ref() != text.as_bytes());
    let smi = "Season 01/Show S01E02.smi";
    let (_, v) = waiting_beside(&s, smi, &cp949, "/pack/Show - 02.smi").await;

    let diff = found(&v);
    assert_eq!(
        (
            diff.dialogue.added,
            diff.dialogue.changed,
            diff.dialogue.removed
        ),
        (0, 0, 0)
    );
    assert_eq!(diff.timing.count, 0);
    let side = |encoding| Side {
        format: trss_subtitles::compare::Format::Smi,
        encoding,
        cues: 1,
    };
    assert_eq!(diff.old, side(Encoding::Cp949));
    assert_eq!(diff.new, side(Encoding::Utf8));
}

#[tokio::test]
async fn a_current_smi_whose_encoding_is_unknown_is_not_compared_and_says_which_side() {
    let s = setup().await;
    let smi = "Season 01/Show S01E02.smi";
    let current = b"<SAMI>\n<BODY>\n<SYNC Start=0><P Class=KRCC>\x80\x80\xff\n</BODY>\n</SAMI>\n";
    let (job, v) = waiting_beside(&s, smi, current, "/pack/Show - 02.smi").await;

    let reason = not_made(&v);
    assert!(reason.starts_with("현재 자막: ") && reason.len() > "현재 자막: ".len());
    assert_eq!(v.comparison.as_ref().unwrap().path, smi);
    // Not made means no lines, and never a difference of nothing.
    assert_eq!(lines_of(&s, &job, &v.plan).await, None);
}

#[tokio::test]
async fn a_picture_subtitle_beside_the_video_is_the_current_one_and_is_not_compared() {
    let s = setup().await;
    let sup = "Season 01/Show S01E02.sup";
    let (_, v) = waiting_beside(&s, sup, b"PG\x00\x01picture", "/ok/Show-02").await;

    assert_eq!(
        actions(&v.plan),
        [(TARGET, PathAction::Add), (sup, PathAction::Keep)]
    );
    assert_eq!(v.plan.current().map(|(p, _)| p.path.as_str()), Some(sup));
    assert_eq!(
        not_made(&v),
        "현재 자막: 이미지 자막이라 내용을 비교할 수 없어요"
    );
    assert_eq!(v.comparison.as_ref().unwrap().path, sup);
}

#[tokio::test]
async fn a_current_file_over_eight_mib_is_not_compared() {
    let s = setup().await;
    let mut big = fake::srt("Show - 02");
    big.resize(8 * 1024 * 1024 + 1, b'\n');
    let (job, v) = waiting_beside(&s, SRT, &big, "/pack/Show - 02.srt").await;

    assert_eq!(
        not_made(&v),
        "현재 자막: 파일이 커서 내용을 비교하지 않았어요"
    );
    assert_eq!(lines_of(&s, &job, &v.plan).await, None);
    // The plan itself is made as for any file.
    assert_eq!(actions(&v.plan), [(SRT, PathAction::Replace)]);
}

#[tokio::test]
async fn a_plan_made_again_after_a_change_has_a_comparison_of_its_own() {
    let s = setup().await;
    let (job, plan) = revision_approved(&s).await;
    assert!(matches!(
        s.store.place.replacements(&job).await.unwrap()[0].comparison,
        Some(trss_jobs::place::replace::records::Comparison {
            result: Compared::Diff(_),
            ..
        })
    ));
    let mut bytes = fake::ass("Show-02");
    let last = bytes.len() - 2;
    bytes[last] = b'X';
    std::fs::write(s.at(TARGET), &bytes).unwrap();
    run(&s).await;

    let v = view(&s, &job).await;
    assert_eq!((v.plan.version, v.plan.state), (2, PlanState::Open));
    assert_ne!(v.plan.id, plan.id);
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_replacement_diffs")
            .await,
        2
    );
    // The new comparison is with the file as it is now.
    assert_eq!(found(&v).dialogue.total(), 24);
    assert!(lines_of(&s, &job, &v.plan).await.is_some());
    // The first plan's lines are not another job's or plan's to read.
    assert_eq!(lines_of(&s, "another", &v.plan).await, None);
    // Nor is a plan the job has none of.
    assert_eq!(
        s.store
            .place
            .replacement_lines(&job, "no-plan")
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_plan_made_by_an_earlier_build_has_no_comparison() {
    let s = setup().await;
    let (job, plan) = revision_waiting(&s).await;
    s.sql(format!(
        "DROP TRIGGER subtitle_replacement_diffs_fixed;
         DELETE FROM subtitle_replacement_diffs WHERE plan_id = '{}'",
        plan.id
    ))
    .await;

    assert_eq!(view(&s, &job).await.comparison, None);
    assert_eq!(lines_of(&s, &job, &plan).await, None);
}

#[tokio::test]
async fn migration_57_keeps_a_comparison_for_a_plan_and_refuses_what_is_not_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let conn = trss_core::db::database_at(&path, 56);
    let there = |c: &rusqlite::Connection| -> bool {
        c.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'subtitle_replacement_diffs'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            == 1
    };
    assert!(!there(&conn));
    // A plan of an earlier build, its parents not written (foreign keys off
    // for it only).
    conn.pragma_update(None, "foreign_keys", false).unwrap();
    conn.execute_batch(
        "INSERT INTO subtitle_replacements
             (id, job_id, position, version, state, work_id, season, episode, assignment,
              folder, video_path, video_object, video_size, video_mtime, stored_id, asset_id,
              asset_path, asset_size, asset_sha256, target, created_at, updated_at)
             VALUES ('r1', 'j1', 0, 1, 'open', 'w', 1, 1, 'explicit', '/w', 'v.mkv', '1:3', 10,
                     11, 's1', 'a1', 'a.ass', 1, printf('%064d', 1), 'v.ass', 0, 0);",
    )
    .unwrap();
    drop(conn);
    let db = Db::open(&path).await.unwrap();

    db.run(move |c| {
        assert!(there(c));
        let refused = |sql: &str| c.execute(sql, []).is_err();
        let insert = |plan: &str, path: &str, diff: &str, lines: &str, unreadable: &str| {
            format!(
                "INSERT INTO subtitle_replacement_diffs (plan_id, path, diff, lines, unreadable)
                 VALUES ({plan}, {path}, {diff}, {lines}, {unreadable})"
            )
        };
        // Either a difference with its lines, or why there is none: not both,
        // not neither, not a difference without its lines.
        assert!(refused(&insert("'r1'", "'v.ass'", "NULL", "NULL", "NULL")));
        assert!(refused(&insert("'r1'", "'v.ass'", "'{}'", "'{}'", "'why'")));
        assert!(refused(&insert("'r1'", "'v.ass'", "'{}'", "NULL", "NULL")));
        assert!(refused(&insert("'r1'", "'v.ass'", "NULL", "'{}'", "'why'")));
        // Valid JSON, a path, a reason.
        assert!(refused(&insert("'r1'", "'v.ass'", "'{'", "'{}'", "NULL")));
        assert!(refused(&insert("'r1'", "'v.ass'", "'{}'", "'['", "NULL")));
        assert!(refused(&insert("'r1'", "''", "'{}'", "'{}'", "NULL")));
        assert!(refused(&insert("'r1'", "'v.ass'", "NULL", "NULL", "''")));
        // Of a plan that is there (the foreign key).
        assert!(refused(&insert(
            "'gone'", "'v.ass'", "NULL", "NULL", "'why'"
        )));
        c.execute(&insert("'r1'", "'v.ass'", "'{}'", "'{}'", "NULL"), [])
            .unwrap();
        // One for a plan.
        assert!(refused(&insert("'r1'", "'v.ass'", "NULL", "NULL", "'why'")));
        // It does not change.
        assert!(refused(
            "UPDATE subtitle_replacement_diffs SET unreadable = 'why', diff = NULL, lines = NULL"
        ));
        assert!(refused(
            "UPDATE subtitle_replacement_diffs SET path = 'other.ass'"
        ));
        // It goes with its plan.
        c.execute("DELETE FROM subtitle_replacements WHERE id = 'r1'", [])
            .unwrap();
        let left: i64 = c
            .query_row("SELECT count(*) FROM subtitle_replacement_diffs", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(left, 0);
        Ok::<_, trss_core::DbError>(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn the_current_subtitle_is_the_replaced_file_else_the_removed_copy_else_a_kept_file() {
    let s = setup().await;
    // An applied SRT to remove, beside nothing at the new copy's path.
    let (_, plan) = ass_waiting_beside_srt(&s).await;
    assert_eq!(
        actions(&plan),
        [(TARGET, PathAction::Add), (SRT, PathAction::Remove)]
    );
    assert_eq!(plan.current().map(|(p, _)| p.path.as_str()), Some(SRT));

    // A file to replace comes before a removal and a kept file.
    let mut both = plan.clone();
    both.paths[0].action = PathAction::Keep;
    both.paths[0].file = both.paths[1].file.clone();
    both.paths
        .push(trss_jobs::place::replace::records::PlanPath {
            path: "x.ass".to_owned(),
            action: PathAction::Replace,
            ..both.paths[1].clone()
        });
    assert_eq!(both.current().map(|(p, _)| p.path.as_str()), Some("x.ass"));
    // A plan that only adds has none.
    let mut adds = plan;
    adds.paths.truncate(1);
    assert!(adds.current().is_none());
}
