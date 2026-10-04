//! Storing and applying what a candidate's job received (ticket 0063,
//! `trss_jobs::place`), with the restarts a killed worker leaves.

use std::{
    os::unix::fs::MetadataExt,
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
    model::{Outcome, PlanAction},
    store::JobDetail,
    Created, JobState, JobStore, NewItem, NewJob, Runner, StepKind, StepState, Wait,
};
use trss_subtitles::{
    fake::{self, FakeSource},
    Sources,
};

const WORK: &str = "w1";
const CREATOR: &str = "제작자";
const VIDEO: &str = "Season 01/Show S01E02.mkv";

struct Setup {
    dir: tempfile::TempDir,
    db: Db,
    store: JobStore,
    runner: Runner,
    area: ReceiveArea,
}

impl Setup {
    /// The work's folder.
    fn work(&self) -> PathBuf {
        self.dir.path().join("shows/Show")
    }

    fn stored_dir(&self) -> PathBuf {
        self.work().join(".trss/subtitles").join(CREATOR)
    }

    async fn sql(&self, sql: &'static str) {
        self.db
            .run(move |c| c.execute_batch(sql).map_err(trss_core::DbError::from))
            .await
            .unwrap();
    }

    async fn count(&self, table: &'static str) -> i64 {
        self.db
            .run(move |c| {
                c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
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
                 VALUES ('{WORK}', '{VIDEO}', 1, '02', 'video');
             INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                 VALUES ('src', 7, '{CREATOR}', 0);"
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
        area.clone(),
        ticking_clock(),
    );
    Setup {
        dir,
        db,
        store,
        runner,
        area,
    }
}

/// A candidate's job of the work for the fake post `path`, as Anissia's
/// episode `episode`, from the source `src` when `source` says so.
async fn make(s: &Setup, command: &str, episode: &str, path: &str, source: bool) -> String {
    let job = NewJob {
        command_id: command.to_owned(),
        request: format!("{{\"c\":\"{command}\"}}"),
        origin: "pick".to_owned(),
        work_id: Some(WORK.to_owned()),
        season: Some(1),
        anime_no: Some(7),
        source_id: source.then(|| "src".to_owned()),
        creator: Some(CREATOR.to_owned()),
        revision_of: None,
        revises_attributed: false,
        items: vec![NewItem {
            observation_id: None,
            episode: episode.to_owned(),
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

fn step(d: &JobDetail, kind: StepKind) -> Option<StepState> {
    d.steps.iter().find(|s| s.step == kind).map(|s| s.state)
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

#[tokio::test]
async fn a_received_subtitle_is_stored_applied_beside_its_video_and_leaves_the_receive_area() {
    let s = setup().await;
    let id = make(&s, "c1", "2", "/ok/Show-02", false).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    for kind in [StepKind::Receive, StepKind::Store, StepKind::Apply] {
        assert_eq!(step(&d, kind), Some(StepState::Done), "{kind:?}");
    }
    let at = |k| d.steps.iter().find(|s| s.step == k).unwrap().at;
    assert!(at(StepKind::Receive) <= at(StepKind::Store));
    assert!(at(StepKind::Store) <= at(StepKind::Apply));

    let bytes = fake::ass("Show-02");
    let stored = s.stored_dir().join("Show-02.ass");
    let applied = s.work().join("Season 01/Show S01E02.ass");
    assert_eq!(std::fs::read(&stored).unwrap(), bytes);
    assert_eq!(std::fs::read(&applied).unwrap(), bytes);
    // Two files, not one file under two names.
    assert_ne!(
        std::fs::metadata(&stored).unwrap().ino(),
        std::fs::metadata(&applied).unwrap().ino()
    );
    // Nothing temporary is left, and the receipt is gone from the area.
    assert_eq!(names(&s.work().join(".trss/tmp")), Vec::<String>::new());
    assert!(!s.area.at(&id).exists());

    let plan = s.store.plan(&id).await.unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].outcome, Some(Outcome::Applied));
    assert_eq!(plan[0].placed.as_ref().map(|p| p.episode), Some(2));
    for table in [
        "subtitle_packages",
        "subtitle_package_entries",
        "subtitle_assets",
        "subtitle_stored",
        "subtitle_applied",
    ] {
        assert_eq!(s.count(table).await, 1, "{table}");
    }

    // The applied copy is the person's to change or remove: the stored one
    // stays as it was.
    std::fs::write(&applied, b"changed").unwrap();
    std::fs::remove_file(&applied).unwrap();
    assert_eq!(std::fs::read(&stored).unwrap(), bytes);
}

#[tokio::test]
async fn a_job_received_before_storing_goes_on_without_receiving_again() {
    let s = setup().await;
    let id = make(&s, "c1", "2", "/ok/Show-02", false).await;
    // As a build before storing left it: received and done, nothing stored.
    let job = id.clone();
    s.db.run(move |c| {
        c.execute(
            "UPDATE subtitle_jobs SET work_id = NULL WHERE id = ?1",
            [&job],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
    run(&s).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Done);
    let receipts = s.count("subtitle_job_files").await;
    let job = id.clone();
    s.db.run(move |c| {
        c.execute(
            "UPDATE subtitle_jobs SET work_id = ?2, state = 'pending' WHERE id = ?1",
            params![job, WORK],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();

    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(
        s.count("subtitle_job_files").await,
        receipts,
        "no new receipt"
    );
    assert!(d
        .events
        .iter()
        .any(|e| e.message == "받아 둔 파일의 보관과 적용을 이어가요"));
    assert!(s.work().join("Season 01/Show S01E02.ass").exists());
}

#[tokio::test]
async fn a_file_whose_name_says_another_episode_is_stored_but_not_applied() {
    let s = setup().await;
    s.sql(
        "INSERT INTO subtitle_episode_mappings
             (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
         VALUES ('w1', 1, 'src', 'user', -12, '시험', 0);",
    )
    .await;
    let id = make(&s, "c1", "14", "/ok/Show-13", true).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Waiting);
    assert_eq!(d.row.wait, Some(Wait::Placement));
    let plan = s.store.plan(&id).await.unwrap();
    assert!(
        plan[0].question.as_deref().unwrap().contains("13화"),
        "{plan:?}"
    );
    assert_eq!(plan[0].placed, None);
    // Kept, on no episode, and nothing beside the video.
    assert!(s.stored_dir().join("Show-13.ass").exists());
    assert_eq!(names(&s.work().join("Season 01")), ["Show S01E02.mkv"]);
    assert_eq!(s.count("subtitle_applied").await, 0);
    assert_eq!(step(&d, StepKind::Apply), None);

    // The same candidate's file of the right number goes to episode 2.
    let id = make(&s, "c2", "14", "/ok/Show-14", true).await;
    run(&s).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Done);
    assert!(s.work().join("Season 01/Show S01E02.ass").exists());
}

#[tokio::test]
async fn a_name_taken_by_other_bytes_is_numbered_and_the_same_bytes_are_one_file() {
    let s = setup().await;
    // Another file under the name, which the app did not store.
    std::fs::create_dir_all(s.stored_dir()).unwrap();
    std::fs::write(s.stored_dir().join("show-02.ass"), b"other").unwrap();
    let id = make(&s, "c1", "2", "/ok/Show-02", false).await;
    run(&s).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Done);
    assert_eq!(names(&s.stored_dir()), ["Show-02 (2).ass", "show-02.ass"]);
    assert_eq!(
        std::fs::read(s.stored_dir().join("show-02.ass")).unwrap(),
        b"other"
    );

    // The same bytes again (the episode has its subtitle now): the stored
    // file and its record are used, not copied.
    let again = make(&s, "c2", "2", "/ok/Show-02", false).await;
    run(&s).await;
    let d = detail(&s, &again).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(names(&s.stored_dir()).len(), 2);
    assert_eq!(s.count("subtitle_assets").await, 1);
    assert_eq!(s.count("subtitle_stored").await, 1);
    let plan = s.store.plan(&again).await.unwrap();
    assert_eq!(plan[0].outcome, Some(Outcome::Existing));
}

#[tokio::test]
async fn an_episode_with_a_subtitle_keeps_it_and_one_without_a_video_is_stored_only() {
    let s = setup().await;
    let smi = s.work().join("Season 01/Show S01E02.smi");
    std::fs::write(&smi, b"<SAMI></SAMI>").unwrap();
    let id = make(&s, "c1", "2", "/ok/Show-02", false).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(
        s.store.plan(&id).await.unwrap()[0].outcome,
        Some(Outcome::Existing)
    );
    assert_eq!(std::fs::read(&smi).unwrap(), b"<SAMI></SAMI>");
    assert!(!s.work().join("Season 01/Show S01E02.ass").exists());
    assert!(s.stored_dir().join("Show-02.ass").exists());

    // Episode 3 has no video.
    let id = make(&s, "c2", "3", "/ok/Show-03", false).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    let plan = s.store.plan(&id).await.unwrap();
    assert_eq!(plan[0].outcome, Some(Outcome::NoVideo));
    assert_eq!(plan[0].action, PlanAction::Apply);
    assert!(s.stored_dir().join("Show-03.ass").exists());
}

// ---------------------------------------------------------------------------
// Restarts

/// Runs a job whose work folder is not there yet, so it is received and
/// planned but nothing is stored: it waits for the folder. Then the folder
/// comes and a worker's start puts the job in line again.
async fn received_not_stored(s: &Setup, path: &str) -> String {
    let id = received_away(s, "c1", path).await;
    assert_eq!(s.runner.requeue_waiting_for_sources().await.unwrap(), 1);
    id
}

/// [`received_not_stored`] before the worker's start: the job waits.
async fn received_away(s: &Setup, command: &str, path: &str) -> String {
    let work = s.work();
    let hidden = s.dir.path().join("shows/.Show-away");
    std::fs::rename(&work, &hidden).unwrap();
    let id = make(s, command, "2", path, false).await;
    run(s).await;
    let d = detail(s, &id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Video)),
        "{:?}",
        d.row.note
    );
    assert_eq!(step(&d, StepKind::Store), None);
    assert_eq!(s.store.plan(&id).await.unwrap().len(), 1);
    std::fs::rename(&hidden, &work).unwrap();
    id
}

async fn requeue(s: &Setup, id: &str) {
    let job = id.to_owned();
    s.db.run(move |c| {
        c.execute(
            "UPDATE subtitle_jobs SET state = 'pending' WHERE id = ?1",
            [job],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
}

/// Records an effect as a killed worker left it.
#[allow(clippy::too_many_arguments)]
async fn left_effect(
    s: &Setup,
    job: &str,
    kind: &'static str,
    state: &'static str,
    temp: &str,
    target: &str,
    video: Option<&str>,
    bytes: &[u8],
    object: Option<String>,
) {
    use sha2::{Digest, Sha256};
    let sha = trss_jobs::area::hex(&Sha256::digest(bytes));
    let (job, temp, target) = (job.to_owned(), temp.to_owned(), target.to_owned());
    let video = video.map(str::to_owned);
    let folder = s.work().to_string_lossy().into_owned();
    let size = bytes.len() as i64;
    s.db.run(move |c| {
        c.execute(
            "INSERT INTO subtitle_file_effects
                 (id, job_id, position, kind, state, folder, temp, target, video, size, sha256,
                  object, created_at, updated_at)
             VALUES ('e1', ?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, 1)",
            params![job, kind, state, folder, temp, target, video, size, sha, object],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
}

fn write_temp(s: &Setup, name: &str, bytes: &[u8]) -> (String, String) {
    let relative = format!(".trss/tmp/{name}");
    let path = s.work().join(&relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    let object = object_of(&std::fs::metadata(&path).unwrap());
    (relative, object)
}

#[tokio::test]
async fn a_store_intended_before_a_kill_is_done_anew_once() {
    let s = setup().await;
    let id = received_not_stored(&s, "/ok/Show-02").await;
    let bytes = fake::ass("Show-02");
    // Half of the bytes made it to the temporary file.
    let (temp, _) = write_temp(&s, "t1", &bytes[..bytes.len() / 2]);
    let target = format!(".trss/subtitles/{CREATOR}/Show-02.ass");
    left_effect(
        &s, &id, "store", "intended", &temp, &target, None, &bytes, None,
    )
    .await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(names(&s.stored_dir()), ["Show-02.ass"]);
    assert!(!s.work().join(&temp).exists());
    assert_eq!(s.count("subtitle_assets").await, 1);
}

#[tokio::test]
async fn a_prepared_store_is_published_and_one_already_renamed_is_found_by_its_object() {
    for renamed in [false, true] {
        let s = setup().await;
        let id = received_not_stored(&s, "/ok/Show-02").await;
        let bytes = fake::ass("Show-02");
        let (temp, object) = write_temp(&s, "t1", &bytes);
        let target = format!(".trss/subtitles/{CREATOR}/Show-02.ass");
        if renamed {
            std::fs::create_dir_all(s.stored_dir()).unwrap();
            std::fs::rename(s.work().join(&temp), s.work().join(&target)).unwrap();
        }
        left_effect(
            &s,
            &id,
            "store",
            "prepared",
            &temp,
            &target,
            None,
            &bytes,
            Some(object),
        )
        .await;
        run(&s).await;

        let d = detail(&s, &id).await;
        assert_eq!(
            d.row.state,
            JobState::Done,
            "renamed {renamed}: {:?}",
            d.row.note
        );
        assert_eq!(names(&s.stored_dir()), ["Show-02.ass"], "renamed {renamed}");
        assert_eq!(s.count("subtitle_assets").await, 1);
        assert!(s.work().join("Season 01/Show S01E02.ass").exists());
    }
}

#[tokio::test]
async fn a_prepared_effect_whose_files_are_not_what_it_wrote_is_held() {
    let s = setup().await;
    let id = received_not_stored(&s, "/ok/Show-02").await;
    let bytes = fake::ass("Show-02");
    // The temporary file is gone and the target holds a copy with the same
    // bytes that is not the file this effect wrote.
    let (temp, object) = write_temp(&s, "t1", &bytes);
    std::fs::remove_file(s.work().join(&temp)).unwrap();
    let target = format!(".trss/subtitles/{CREATOR}/Show-02.ass");
    std::fs::create_dir_all(s.stored_dir()).unwrap();
    std::fs::write(s.work().join(&target), &bytes).unwrap();
    left_effect(
        &s,
        &id,
        "store",
        "prepared",
        &temp,
        &target,
        None,
        &bytes,
        Some(object),
    )
    .await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Held);
    assert_eq!(s.count("subtitle_assets").await, 0, "no false record");
    assert_eq!(names(&s.stored_dir()), ["Show-02.ass"]);
    // A held job is not taken up again by itself.
    run(&s).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Held);
}

/// A job stored with no video yet, so its row waits for an apply; the video
/// is there now and the job is in line again.
async fn stored_not_applied(s: &Setup) -> String {
    let video = s.work().join(VIDEO);
    std::fs::remove_file(&video).unwrap();
    let id = make(s, "c1", "2", "/ok/Show-02", false).await;
    run(s).await;
    assert_eq!(
        s.store.plan(&id).await.unwrap()[0].outcome,
        Some(Outcome::NoVideo)
    );
    std::fs::write(&video, b"video").unwrap();
    let job = id.clone();
    s.db.run(move |c| {
        c.execute(
            "UPDATE subtitle_job_plan SET outcome = NULL, note = NULL WHERE job_id = ?1",
            [job],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
    requeue(s, &id).await;
    id
}

#[tokio::test]
async fn an_apply_renamed_before_its_record_is_recorded_once() {
    let s = setup().await;
    let id = stored_not_applied(&s).await;
    let bytes = fake::ass("Show-02");
    let (temp, object) = write_temp(&s, "t1", &bytes);
    let target = "Season 01/Show S01E02.ass";
    std::fs::rename(s.work().join(&temp), s.work().join(target)).unwrap();
    left_effect(
        &s,
        &id,
        "apply",
        "prepared",
        &temp,
        target,
        Some(VIDEO),
        &bytes,
        Some(object),
    )
    .await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(s.count("subtitle_applied").await, 1);
    assert_eq!(
        names(&s.work().join("Season 01")),
        ["Show S01E02.ass", "Show S01E02.mkv"]
    );
}

#[tokio::test]
async fn a_file_that_takes_the_name_before_publishing_is_kept_and_the_episode_held() {
    let s = setup().await;
    let id = stored_not_applied(&s).await;
    let bytes = fake::ass("Show-02");
    let (temp, object) = write_temp(&s, "t1", &bytes);
    let target = "Season 01/Show S01E02.ass";
    left_effect(
        &s,
        &id,
        "apply",
        "prepared",
        &temp,
        target,
        Some(VIDEO),
        &bytes,
        Some(object),
    )
    .await;
    // Someone's own subtitle comes under the name in between.
    std::fs::write(s.work().join(target), b"mine").unwrap();
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Held, "{:?}", d.row.note);
    assert_eq!(std::fs::read(s.work().join(target)).unwrap(), b"mine");
    assert!(!s.work().join(&temp).exists());
    assert_eq!(s.count("subtitle_applied").await, 0);
}

/// Puts `job` last in line, as a job a killed worker left `running` behind
/// newer ones (made before this).
async fn last_in_line(s: &Setup, job: &str) {
    let job = job.to_owned();
    s.db.run(move |c| {
        c.execute(
            "UPDATE subtitle_jobs SET seq = 99999, state = 'running' WHERE id = ?1",
            [job],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn an_applied_copy_a_person_removed_is_applied_again_by_a_later_job() {
    let s = setup().await;
    let first = make(&s, "c1", "2", "/ok/Show-02", false).await;
    run(&s).await;
    let applied = s.work().join("Season 01/Show S01E02.ass");
    std::fs::remove_file(&applied).unwrap();

    let again = make(&s, "c2", "2", "/ok/Show-02", false).await;
    run(&s).await;
    let d = detail(&s, &again).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(
        s.store.plan(&again).await.unwrap()[0].outcome,
        Some(Outcome::Applied)
    );
    assert!(applied.exists());
    // The first copy is recorded as removed; the second is the one there.
    let removed: Vec<(String, bool)> =
        s.db.run(|c| {
            let mut stmt = c.prepare(
                "SELECT job_id, removed_at IS NOT NULL FROM subtitle_applied ORDER BY applied_at",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    assert_eq!(removed, [(first, true), (again, false)]);
}

#[tokio::test]
async fn a_name_another_jobs_store_is_about_to_take_is_taken_whatever_its_case() {
    let s = setup().await;
    // A killed worker left job A's store prepared under `show-02.ass`, and
    // A behind a newer job in line.
    let a = received_away(&s, "c1", "/ok/Show-02").await;
    let bytes = fake::ass("Show-02");
    let (temp, object) = write_temp(&s, "t1", &bytes);
    let target = format!(".trss/subtitles/{CREATOR}/show-02.ass");
    left_effect(
        &s,
        &a,
        "store",
        "prepared",
        &temp,
        &target,
        None,
        &bytes,
        Some(object),
    )
    .await;
    let b = make(&s, "c2", "2", "/ok/Show-02", false).await;
    last_in_line(&s, &a).await;
    run(&s).await;

    for job in [&a, &b] {
        let d = detail(&s, job).await;
        assert_eq!(d.row.state, JobState::Done, "{job}: {:?}", d.row.note);
    }
    assert_eq!(names(&s.stored_dir()), ["Show-02 (2).ass", "show-02.ass"]);
    assert_eq!(s.count("subtitle_assets").await, 2);
    // B came first and applied its copy; A's found the episode's subtitle.
    assert_eq!(
        s.store.plan(&b).await.unwrap()[0].outcome,
        Some(Outcome::Applied)
    );
    assert_eq!(
        s.store.plan(&a).await.unwrap()[0].outcome,
        Some(Outcome::Existing)
    );
}

#[tokio::test]
async fn an_episode_another_jobs_apply_is_about_to_take_keeps_it() {
    let s = setup().await;
    let a = stored_not_applied(&s).await;
    let bytes = fake::ass("Show-02");
    let (temp, object) = write_temp(&s, "t1", &bytes);
    let target = "Season 01/Show S01E02.ass";
    left_effect(
        &s,
        &a,
        "apply",
        "prepared",
        &temp,
        target,
        Some(VIDEO),
        &bytes,
        Some(object),
    )
    .await;
    let b = make(&s, "c2", "2", "/ok/Show-02", false).await;
    last_in_line(&s, &a).await;
    run(&s).await;

    let plan = s.store.plan(&b).await.unwrap();
    assert_eq!(plan[0].outcome, Some(Outcome::Existing), "{plan:?}");
    let d = detail(&s, &a).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(
        s.store.plan(&a).await.unwrap()[0].outcome,
        Some(Outcome::Applied)
    );
    assert_eq!(s.count("subtitle_applied").await, 1);
}

#[tokio::test]
async fn a_job_held_after_its_starts_holds_its_effects_and_frees_their_names() {
    let s = setup().await;
    let id = received_not_stored(&s, "/ok/Show-02").await;
    let bytes = fake::ass("Show-02");
    let (temp, object) = write_temp(&s, "t1", &bytes);
    let target = format!(".trss/subtitles/{CREATOR}/Show-02.ass");
    left_effect(
        &s,
        &id,
        "store",
        "prepared",
        &temp,
        &target,
        None,
        &bytes,
        Some(object),
    )
    .await;
    let job = id.clone();
    s.db.run(move |c| {
        c.execute(
            "UPDATE subtitle_jobs SET state = 'running', attempts = ?2 WHERE id = ?1",
            params![job, trss_jobs::runner::MAX_STARTS],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Held);
    let plan = s.store.plan(&id).await.unwrap();
    assert_eq!(plan[0].outcome, Some(Outcome::Held));
    let state: String =
        s.db.run(|c| {
            c.query_row("SELECT state FROM subtitle_file_effects", [], |r| r.get(0))
                .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    assert_eq!(state, "held");
    // Its temporary file stays, and the name is free for another job.
    assert!(s.work().join(&temp).exists());
    let other = make(&s, "c2", "2", "/ok/Show-02", false).await;
    run(&s).await;
    assert_eq!(detail(&s, &other).await.row.state, JobState::Done);
    assert_eq!(names(&s.stored_dir()), ["Show-02.ass"]);
}

#[tokio::test]
async fn a_row_only_stored_is_settled_by_its_store() {
    let s = setup().await;
    let id = received_not_stored(&s, "/ok/Show-02").await;
    // As a format the app keeps but does not apply.
    let job = id.clone();
    s.db.run(move |c| {
        c.execute(
            "UPDATE subtitle_job_plan SET action = 'store', format = 'other' WHERE job_id = ?1",
            [job],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    let plan = s.store.plan(&id).await.unwrap();
    assert_eq!(plan[0].outcome, Some(Outcome::Stored));
    assert!(plan[0].stored_id.is_some());
    assert_eq!(names(&s.work().join("Season 01")), ["Show S01E02.mkv"]);
}

#[tokio::test]
async fn a_file_two_posts_share_is_planned_once_and_shared_again_once_stored() {
    let s = setup().await;
    let job = NewJob {
        command_id: "c1".to_owned(),
        request: "{\"c\":\"c1\"}".to_owned(),
        origin: "pick".to_owned(),
        work_id: Some(WORK.to_owned()),
        season: Some(1),
        anime_no: Some(7),
        source_id: None,
        creator: Some(CREATOR.to_owned()),
        revision_of: None,
        revises_attributed: false,
        items: ["2", "3"]
            .into_iter()
            .map(|episode| NewItem {
                observation_id: None,
                episode: episode.to_owned(),
                post_url: format!("https://{}/shared/Show/{episode}", fake::HOST),
                found_at: 500,
            })
            .collect(),
    };
    let Created::Created(id) = s.store.create(job, 900).await.unwrap() else {
        panic!("not created");
    };
    run(&s).await;

    // The second post's receipt is the first's: one row, and both lost
    // their bytes when it was stored.
    assert_eq!(s.store.plan(&id).await.unwrap().len(), 1);
    let d = detail(&s, &id).await;
    let files: Vec<_> = d.items.iter().flat_map(|i| i.files.iter()).collect();
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|f| f.cleared_at.is_some()), "{files:?}");

    // Episode 3's item again, as one a later run receives: the stored
    // receipt is not held for its missing bytes but shared again.
    let job = id.clone();
    s.db.run(move |c| {
        c.execute_batch(&format!(
            "DELETE FROM subtitle_job_files WHERE job_id = '{job}' AND same_as IS NOT NULL;
             UPDATE subtitle_job_items SET state = 'pending' WHERE job_id = '{job}'
                 AND episode = '3';
             UPDATE subtitle_jobs SET state = 'pending' WHERE id = '{job}';"
        ))
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    let files: Vec<_> = d.items.iter().flat_map(|i| i.files.iter()).collect();
    assert_eq!(files.len(), 2);
    assert!(
        files
            .iter()
            .all(|f| f.state == trss_jobs::FileState::Done && f.cleared_at.is_some()),
        "{files:?}"
    );
    assert_eq!(s.store.plan(&id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn the_migration_puts_received_candidate_jobs_back_in_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let conn = trss_core::db::database_at(&path, 48);
    conn.execute_batch(
        "INSERT INTO subtitle_jobs (id, command_id, request, origin, state, created_at,
                                    updated_at, state_at, wait, stage, finished_at)
         VALUES ('p', 'p', '{}', 'pick', 'done', 0, 0, 0, NULL, NULL, 5),
                ('a', 'auto:1', '{}', 'auto', 'partial', 0, 0, 0, NULL, NULL, 5),
                ('u', 'u', '{}', 'upload', 'done', 0, 0, 0, NULL, NULL, 5),
                ('w', 'w', '{}', 'pick', 'waiting', 0, 0, 0, 'auth', 'auth', NULL);
         INSERT INTO subtitle_job_steps (job_id, step, state, at) VALUES ('p', 'receive', 'done', 5);",
    )
    .unwrap();
    drop(conn);
    let db = Db::open(&path).await.unwrap();
    let states: Vec<(String, String, Option<String>, bool)> = db
        .run(|c| {
            let mut stmt = c.prepare(
                "SELECT id, state, wait, finished_at IS NOT NULL FROM subtitle_jobs ORDER BY id",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    assert_eq!(
        states,
        [
            ("a".to_owned(), "pending".to_owned(), None, false),
            ("p".to_owned(), "pending".to_owned(), None, false),
            ("u".to_owned(), "done".to_owned(), None, true),
            (
                "w".to_owned(),
                "waiting".to_owned(),
                Some("auth".to_owned()),
                false
            ),
        ]
    );
    let step: String = db
        .run(|c| {
            c.query_row("SELECT step FROM subtitle_job_steps", [], |r| r.get(0))
                .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    assert_eq!(step, "receive");
}
