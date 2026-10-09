//! Unpacking a received archive (ticket 0065, `trss_jobs::place::unpack`)
//! through the `trss-extract` program: its members are placed as a package's
//! files are, a refused archive fails alone and stays in the receive area, and
//! a worker killed while it unpacked unpacks again from what it received.

use crate::{
    world::{counting_clock, ticking_clock, Base, Shows},
    Handles,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
};

use rusqlite::params;
use tokio_util::sync::CancellationToken;
use trss_core::Db;
use trss_jobs::{
    area::ReceiveArea,
    model::{AssetKind, Outcome, PlanAction, StepKind},
    place::records::{Confirmed, RowPlacing},
    place::unpack::{UNPACK_AGAIN, UNPACK_RETRY_AFTER},
    store::{FileRow, JobDetail},
    Created, JobState, NewItem, NewJob, Runner, Unpacker, Wait,
};
use trss_subtitles::{
    fake::{self, FakeSource},
    Sources,
};

const WORK: &str = "w1";
const CREATOR: &str = "제작자";

struct Setup {
    dir: tempfile::TempDir,
    db: Db,
    store: Handles,
    area: ReceiveArea,
}

impl Setup {
    fn work(&self) -> PathBuf {
        self.dir.path().join("shows/Show")
    }

    fn stored_dir(&self) -> PathBuf {
        self.work().join(".trss/subtitles").join(CREATOR)
    }

    /// A runner over the fake source, with the unpacking program or without.
    fn runner(&self, unpacks: bool) -> Runner {
        let runner = Runner::new(
            self.store.run.clone(),
            Sources::none().with_fake(FakeSource),
            self.area.clone(),
            ticking_clock(),
        );
        match unpacks {
            true => runner.with_unpacker(Unpacker::new(PathBuf::from(env!(
                "CARGO_BIN_EXE_trss-extract"
            )))),
            false => runner,
        }
    }

    /// A runner over the fake source with no program to unpack, at the
    /// times `now` gives (it ticks).
    fn bare_runner_at(&self, now: &Arc<AtomicI64>) -> Runner {
        Runner::new(
            self.store.run.clone(),
            Sources::none().with_fake(FakeSource),
            self.area.clone(),
            counting_clock(now),
        )
    }

    /// The same, unpacking with `program`.
    fn runner_at(&self, program: PathBuf, now: &Arc<AtomicI64>) -> Runner {
        self.bare_runner_at(now)
            .with_unpacker(Unpacker::new(program))
    }

    async fn run(&self) {
        self.runner(true)
            .run_ready(&CancellationToken::new())
            .await
            .unwrap();
    }

    async fn detail(&self, id: &str) -> JobDetail {
        self.store.views.detail(id).await.unwrap().unwrap()
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

/// A library with the work `Show` (season 1).
async fn setup() -> Setup {
    let base = Base::new().await;
    base.library(&Shows::default()).await;
    Setup {
        dir: base.dir,
        db: base.db,
        store: base.store,
        area: base.area,
    }
}

/// Puts a video for episode `episode` of season 1 in the work.
async fn video(s: &Setup, episode: &str) {
    let path = format!("Season 01/Show S01E{episode}.mkv");
    std::fs::write(s.work().join(&path), b"video").unwrap();
    let e = episode.to_owned();
    s.db.run(move |c| {
        c.execute(
            "INSERT OR IGNORE INTO episodes (work_id, season, episode) VALUES (?1, 1, ?2)",
            params![WORK, e],
        )?;
        c.execute(
            "INSERT INTO media_files (work_id, path, season, episode, kind)
             VALUES (?1, ?2, 1, ?3, 'video')",
            params![WORK, path, e],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
}

/// A segment of a fake post's path: a space and a folder's `/` encoded.
fn segment(name: &str) -> String {
    name.replace('%', "%25")
        .replace(' ', "%20")
        .replace('/', "%2F")
}

/// The fake post whose one file is the ZIP `archive` of `members`.
fn zip_post(archive: &str, members: &[&str]) -> String {
    let members: Vec<String> = members.iter().map(|m| segment(m)).collect();
    format!("/zip/{}/{}", segment(archive), members.join("/"))
}

/// A candidate's job of the work, one item for each `(episode, post path)`.
async fn make(s: &Setup, command: &str, posts: &[(&str, &str)]) -> String {
    let job = NewJob {
        command_id: command.to_owned(),
        request: format!("{{\"c\":\"{command}\"}}"),
        origin: "pick".to_owned(),
        work_id: Some(WORK.to_owned()),
        season: Some(1),
        anime_no: Some(7),
        source_id: None,
        creator: Some(CREATOR.to_owned()),
        revision_of: None,
        revises_attributed: false,
        items: posts
            .iter()
            .map(|(episode, path)| NewItem {
                observation_id: None,
                episode: (*episode).to_owned(),
                post_url: format!("https://{}{path}", fake::HOST),
                found_at: 500,
            })
            .collect(),
    };
    match s.store.requests.create(job, 900).await.unwrap() {
        Created::Created(id) => id,
        other => panic!("created: {other:?}"),
    }
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

/// Every file under `dir`, as paths relative to it.
fn tree(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten() {
            let path = entry.unwrap().path();
            match path.is_dir() {
                true => walk(root, &path, out),
                false => out.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                ),
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn episode_of(row: &trss_jobs::place::records::PlanRow) -> Option<i64> {
    row.placed.as_ref().map(|p| p.episode)
}

fn events(d: &JobDetail) -> Vec<(String, Option<String>)> {
    d.events
        .iter()
        .map(|e| (e.message.clone(), e.detail.clone()))
        .collect()
}

/// The archives made for these tests (`tests/fixtures`).
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Gives the receipt `file` the bytes `bytes`, as a site would have sent
/// them, with the facts recorded of them.
async fn replace_bytes(s: &Setup, file: &FileRow, bytes: &[u8]) {
    let path = s.area.at(file.path.as_deref().unwrap());
    std::fs::write(&path, bytes).unwrap();
    let (size, sha256, object) = trss_jobs::area::read_facts(&path).unwrap();
    let id = file.id.clone();
    s.db.run(move |c| {
        c.execute(
            "UPDATE subtitle_job_files SET size = ?2, sha256 = ?3, object = ?4 WHERE id = ?1",
            params![id, size as i64, sha256, object],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
}

/// Confirms the job's 배치 확인 as it was planned: each subtitle on the
/// episode it was planned on, applied when it was to be.
async fn confirm_as_planned(s: &Setup, id: &str) -> Confirmed {
    let placings = s
        .store
        .place
        .plan(id)
        .await
        .unwrap()
        .iter()
        .filter(|r| r.kind == AssetKind::Subtitle && r.outcome.is_none())
        .map(|r| RowPlacing {
            position: r.position,
            episode: episode_of(r),
            apply: r.action == PlanAction::Apply && r.placed.is_some(),
        })
        .collect();
    s.store
        .place
        .confirm_placement(id, placings, Vec::new(), None, 5_000)
        .await
        .unwrap()
}

/// The note of the job's receiving step.
fn receive_note(d: &JobDetail) -> Option<String> {
    d.steps
        .iter()
        .find(|s| s.step == StepKind::Receive)
        .and_then(|s| s.note.clone())
}

// Drive's `1-12` ZIP of a season (ticket 0065): the candidate's episode is
// applied, the other eleven stored only, and nothing is left to receive.
#[tokio::test]
async fn the_candidates_episode_of_a_season_zip_is_applied_and_the_rest_stored() {
    let s = setup().await;
    video(&s, "04").await;
    let members: Vec<String> = (1..=12).map(|n| format!("Show - {n:02}.ass")).collect();
    let refs: Vec<&str> = members.iter().map(String::as_str).collect();
    let id = make(&s, "c1", &[("4", &zip_post("Show 1-12.zip", &refs))]).await;
    s.run().await;

    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(names(&s.stored_dir()), members);
    assert_eq!(
        std::fs::read(s.work().join("Season 01/Show S01E04.ass")).unwrap(),
        fake::bytes_of("Show - 04.ass")
    );
    let plan = s.store.place.plan(&id).await.unwrap();
    assert_eq!(plan.len(), 12);
    for row in &plan {
        assert_eq!(row.member.as_deref(), Some(row.name.as_str()), "{row:?}");
        match episode_of(row).unwrap() {
            4 => assert_eq!(row.outcome, Some(Outcome::Applied)),
            _ => {
                assert_eq!(row.action, PlanAction::Store, "{row:?}");
                assert_eq!(row.outcome, Some(Outcome::Stored), "{row:?}");
            }
        }
    }
    // The archive, its unpack folder and the job's folder are gone; its
    // members stay recorded.
    assert!(!s.area.at(&id).exists());
    assert_eq!(s.count("subtitle_job_members").await, 12);
    let file = &d.items[0].files[0];
    assert!(file.unpacked_at.is_some() && file.unpack_error.is_none());
    assert!(events(&d)
        .iter()
        .any(|(m, detail)| m == "Show 1-12.zip: 압축 파일을 풀었어요"
            && detail.as_deref() == Some("파일 12개")));
}

// Members in folders, a font beside them, and one that is a web page.
#[tokio::test]
async fn members_in_folders_are_placed_and_one_that_is_no_file_is_dropped_with_why() {
    let s = setup().await;
    video(&s, "02").await;
    let id = make(
        &s,
        "c1",
        &[(
            "2",
            &zip_post(
                "pack.zip",
                &["Show/Show - 02.ass", "Show/Fonts/A.ttf", "Show/page.html"],
            ),
        )],
    )
    .await;
    s.run().await;

    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(names(&s.stored_dir()), ["A.ttf", "Show - 02.ass"]);
    let plan = s.store.place.plan(&id).await.unwrap();
    let page = plan
        .iter()
        .find(|r| r.member.as_deref() == Some("Show/page.html"))
        .unwrap();
    assert_eq!(page.action, PlanAction::Drop);
    assert_eq!(
        page.note.as_deref(),
        Some("받은 내용이 파일이 아니라 웹 페이지(HTML)예요")
    );
    assert!(!s.area.at(&id).exists());
}

// A member whose path leaves the archive: the archive alone fails, it stays
// in the receive area, and nothing is made outside its folder.
#[tokio::test]
async fn an_archive_whose_member_leaves_it_fails_and_stays_received() {
    let s = setup().await;
    video(&s, "02").await;
    let id = make(&s, "c1", &[("2", "/zip/evil.zip/..%2F..%2FShow - 02.ass")]).await;
    s.run().await;

    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Failed, "{:?}", d.row.note);
    let file = &d.items[0].files[0];
    let reason = file.unpack_error.clone().unwrap();
    assert!(d.row.note.as_deref().unwrap().starts_with("evil.zip: "));
    assert!(d.row.note.as_deref().unwrap().ends_with(&reason));
    assert!(events(&d)
        .iter()
        .any(|(m, detail)| m == "evil.zip: 압축 파일을 풀지 못했어요"
            && detail.as_deref() == Some(reason.as_str())));
    assert_eq!(s.count("subtitle_job_members").await, 0);
    assert!(s.store.place.plan(&id).await.unwrap().is_empty());

    // Only the archive is in the receive area, and nothing reached the work.
    let received = file.path.clone().unwrap();
    assert_eq!(tree(&s.area.at("")), std::slice::from_ref(&received));
    assert_eq!(names(&s.work().join("Season 01")), ["Show S01E02.mkv"]);
    assert!(!s.work().join(".trss").exists());
    assert!(tree(s.dir.path())
        .iter()
        .all(|p| !p.ends_with("Show - 02.ass")));

    // A later run leaves it as it is: it is not tried again, the archive's
    // own reason being no machine's failure.
    assert_eq!((file.unpack_tries, file.unpack_retry_at), (0, None));
    let again = s.count("subtitle_job_events").await;
    s.store
        .run
        .requeue_waiting_for_sources(5_000)
        .await
        .unwrap();
    s.run().await;
    assert_eq!(s.count("subtitle_job_events").await, again);
    assert!(s.area.at(&received).is_file());
}

// One post's subtitle is applied, the other's archive is refused: the job is
// partly failed.
#[tokio::test]
async fn a_refused_archive_beside_a_placed_file_leaves_the_job_partial() {
    let s = setup().await;
    video(&s, "02").await;
    video(&s, "03").await;
    let id = make(
        &s,
        "c1",
        &[("2", "/ok/Show-02"), ("3", "/zip/evil.zip/%2Fabs.ass")],
    )
    .await;
    s.run().await;

    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Partial, "{:?}", d.row.note);
    assert!(d.row.note.as_deref().unwrap().starts_with("evil.zip: "));
    assert!(s.work().join("Season 01/Show S01E02.ass").is_file());
    assert!(!s.work().join("Season 01/Show S01E03.ass").exists());
    // The applied file's receipt left; the refused archive stays.
    let archive = d
        .items
        .iter()
        .flat_map(|i| i.files.iter())
        .find(|f| f.name == "evil.zip")
        .unwrap();
    assert_eq!(tree(&s.area.at("")), [archive.path.clone().unwrap()]);
}

// A worker without the program leaves the archive waiting; one killed while
// it unpacked leaves a folder the next run makes anew, from the archive it
// received, without receiving it again.
#[tokio::test]
async fn an_archive_waits_for_the_program_and_a_killed_unpacking_starts_anew() {
    let s = setup().await;
    video(&s, "02").await;
    let id = make(
        &s,
        "c1",
        &[(
            "2",
            &zip_post("pack.zip", &["Show - 02.ass", "Show - 03.ass"]),
        )],
    )
    .await;
    s.runner(false)
        .run_ready(&CancellationToken::new())
        .await
        .unwrap();
    let d = s.detail(&id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Subtitle))
    );
    assert_eq!(d.row.note.as_deref(), Some(trss_jobs::place::ARCHIVE_LATER));
    let file = d.items[0].files[0].clone();
    assert!(file.unpacked_at.is_none() && file.unpack_error.is_none());

    // What a kill in the middle of unpacking left.
    let folder = s.area.at(&ReceiveArea::unpack_dir(&id, &file.id));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("0"), b"cut short").unwrap();
    std::fs::write(folder.join("7"), b"another run's").unwrap();

    // The worker starts again, now with the program.
    s.store
        .run
        .requeue_waiting_for_sources(5_000)
        .await
        .unwrap();
    s.run().await;
    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(names(&s.stored_dir()), ["Show - 02.ass", "Show - 03.ass"]);
    assert_eq!(
        std::fs::read(s.stored_dir().join("Show - 02.ass")).unwrap(),
        fake::bytes_of("Show - 02.ass")
    );
    // The same receipt, received once.
    let again = &d.items[0].files;
    assert_eq!(again.len(), 1);
    assert_eq!(
        (&again[0].id, again[0].created_at),
        (&file.id, file.created_at)
    );
    assert!(!s.area.at(&id).exists());
}

/// What `trss-extract` answers when the disk it unpacks to is full.
const DISK_FULL: &str = "압축을 풀 자리에 쓰지 못했어요: No space left on device (os error 28)";

/// A program that answers [`DISK_FULL`] for any archive.
fn full_disk() -> PathBuf {
    fixtures().join("full-disk-extract.sh")
}

fn extract() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_trss-extract"))
}

// This machine fails to unpack (a full disk, ticket 0065): the job waits
// (`자막 대기`), and the archive is unpacked again from what was received an
// hour later, or at a worker's start if that comes first; the log says which
// try each was.
#[tokio::test]
async fn an_archive_this_machine_failed_to_unpack_is_tried_again_from_its_receipt() {
    let s = setup().await;
    video(&s, "02").await;
    let id = make(
        &s,
        "c1",
        &[(
            "2",
            &zip_post("pack.zip", &["Show - 02.ass", "Show - 03.ass"]),
        )],
    )
    .await;
    let now = Arc::new(AtomicI64::new(1_000));
    let go = CancellationToken::new();
    s.runner_at(full_disk(), &now).run_ready(&go).await.unwrap();

    let d = s.detail(&id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Subtitle))
    );
    assert_eq!(d.row.note.as_deref(), Some(UNPACK_AGAIN));
    let file = d.items[0].files[0].clone();
    assert_eq!(
        (
            file.unpack_tries,
            file.unpack_failure.as_deref(),
            file.unpack_error.as_deref()
        ),
        (1, Some(DISK_FULL), None)
    );
    let retry_at = file.unpack_retry_at.unwrap();
    assert!((1_000..now.load(Ordering::SeqCst)).contains(&(retry_at - UNPACK_RETRY_AFTER)));
    assert!(events(&d).contains(&(
        "pack.zip: 압축 파일을 풀지 못해 다시 풀어요".to_owned(),
        Some(format!(
            "{DISK_FULL} (3번 중 1번째 시도). 1시간 뒤나 worker가 다시 시작할 때 다시 풀어요"
        ))
    )));
    assert_eq!(s.count("subtitle_job_members").await, 0);
    assert!(!s.area.at(&ReceiveArea::unpack_dir(&id, &file.id)).exists());
    assert!(!s.work().join(".trss").exists());

    // Before its hour nothing goes back in line, and a run of the job for
    // another reason (a video came) does not try it.
    now.store(retry_at - 100, Ordering::SeqCst);
    let full = s.runner_at(full_disk(), &now);
    assert_eq!(full.requeue_unpack_retries().await.unwrap(), 0);
    let id2 = id.clone();
    s.db.run(move |c| {
        c.execute(
            "UPDATE subtitle_jobs SET state = 'pending', wait = NULL WHERE id = ?1",
            [id2],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
    full.run_ready(&go).await.unwrap();
    let d = s.detail(&id).await;
    assert_eq!(d.row.note.as_deref(), Some(UNPACK_AGAIN));
    assert_eq!(
        (
            d.items[0].files[0].unpack_tries,
            d.items[0].files[0].unpack_retry_at
        ),
        (1, Some(retry_at))
    );

    // A worker without the program starts: it lets the try go but cannot
    // make it, and says so.
    s.store
        .run
        .requeue_waiting_for_sources(now.load(Ordering::SeqCst))
        .await
        .unwrap();
    s.bare_runner_at(&now).run_ready(&go).await.unwrap();
    let d = s.detail(&id).await;
    assert_eq!(d.row.note.as_deref(), Some(trss_jobs::place::ARCHIVE_LATER));
    assert_eq!(
        (
            d.items[0].files[0].unpack_tries,
            d.items[0].files[0].unpack_retry_at
        ),
        (1, None)
    );

    // A worker with it starts, the disk still full: the second try, at
    // once, which fails as well and waits its own hour.
    s.store
        .run
        .requeue_waiting_for_sources(now.load(Ordering::SeqCst))
        .await
        .unwrap();
    let started = now.load(Ordering::SeqCst);
    full.run_ready(&go).await.unwrap();
    let d = s.detail(&id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Subtitle))
    );
    let file = d.items[0].files[0].clone();
    assert_eq!(file.unpack_tries, 2);
    let retry_at = file.unpack_retry_at.unwrap();
    assert!(retry_at >= started + UNPACK_RETRY_AFTER);
    assert!(events(&d).iter().any(|(m, detail)| m
        == "pack.zip: 압축 파일을 풀지 못해 다시 풀어요"
        && detail.as_deref().unwrap().contains("(3번 중 2번째 시도)")));

    // Its hour is up: a job a person holds stays as it is.
    now.store(retry_at, Ordering::SeqCst);
    let set = |state: &'static str, wait: Option<&'static str>| {
        let id = id.clone();
        s.db.run(move |c| {
            c.execute(
                "UPDATE subtitle_jobs SET state = ?2, wait = ?3 WHERE id = ?1",
                params![id, state, wait],
            )
            .map_err(trss_core::DbError::from)
        })
    };
    set("held", None).await.unwrap();
    assert_eq!(full.requeue_unpack_retries().await.unwrap(), 0);
    set("waiting", Some("subtitle")).await.unwrap();
    // A worker without the program would not try it either.
    assert_eq!(
        s.bare_runner_at(&now)
            .requeue_unpack_retries()
            .await
            .unwrap(),
        0
    );

    // The waiting job goes back in line, and the third try, with room on
    // the disk, unpacks the archive it received.
    assert_eq!(full.requeue_unpack_retries().await.unwrap(), 1);
    s.runner_at(extract(), &now).run_ready(&go).await.unwrap();
    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(names(&s.stored_dir()), ["Show - 02.ass", "Show - 03.ass"]);
    assert!(s.work().join("Season 01/Show S01E02.ass").is_file());
    assert!(events(&d).contains(&(
        "pack.zip: 압축 파일을 풀었어요".to_owned(),
        Some("파일 2개 (3번 중 3번째 시도)".to_owned())
    )));
    let again = &d.items[0].files;
    assert_eq!(again.len(), 1);
    assert_eq!(
        (&again[0].id, again[0].created_at, again[0].unpack_retry_at),
        (&file.id, file.created_at, None)
    );
    assert_eq!(full.requeue_unpack_retries().await.unwrap(), 0);
}

// A try after one this machine failed that the archive's own reason ends
// (here a member leaving it) leaves it not unpacked, counted with the tries
// before it, and no later try waits: the job is not put back in line for it
// while it waits for something else.
#[tokio::test]
async fn a_refusal_after_a_failed_try_ends_the_tries() {
    let s = setup().await;
    video(&s, "02").await;
    let id = make(&s, "c1", &[("2", "/zip/evil.zip/..%2F..%2FShow - 02.ass")]).await;
    let now = Arc::new(AtomicI64::new(1_000));
    let go = CancellationToken::new();
    s.runner_at(full_disk(), &now).run_ready(&go).await.unwrap();
    let retry_at = s.detail(&id).await.items[0].files[0]
        .unpack_retry_at
        .unwrap();

    now.store(retry_at, Ordering::SeqCst);
    let unpacks = s.runner_at(extract(), &now);
    assert_eq!(unpacks.requeue_unpack_retries().await.unwrap(), 1);
    unpacks.run_ready(&go).await.unwrap();

    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Failed, "{:?}", d.row.note);
    let file = &d.items[0].files[0];
    let reason = file.unpack_error.clone().unwrap();
    assert_ne!(reason, DISK_FULL);
    assert_eq!((file.unpack_tries, file.unpack_retry_at), (2, None));
    assert!(events(&d).contains(&(
        "evil.zip: 압축 파일을 풀지 못했어요".to_owned(),
        Some(format!("{reason} (3번 중 2번째 시도)"))
    )));

    let id2 = id.clone();
    s.db.run(move |c| {
        c.execute(
            "UPDATE subtitle_jobs SET state = 'waiting', wait = 'subtitle' WHERE id = ?1",
            [id2],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
    now.fetch_add(UNPACK_RETRY_AFTER, Ordering::SeqCst);
    assert_eq!(unpacks.requeue_unpack_retries().await.unwrap(), 0);
}

// The volumes of a split archive wait for the next try together, as one
// archive, and are unpacked together by it.
#[tokio::test]
async fn a_split_archive_is_tried_again_as_one() {
    let s = setup().await;
    video(&s, "02").await;
    let id = make(
        &s,
        "c1",
        &[("2", "/pack/Show.zip.001/Show.zip.002/Show%20-%2002.ass")],
    )
    .await;
    s.runner(false)
        .run_ready(&CancellationToken::new())
        .await
        .unwrap();
    let whole = fake::zip_of(&["Show - 03.ass".to_owned(), "Show - 04.ass".to_owned()]);
    let (first, second) = whole.split_at(whole.len() / 2);
    let d = s.detail(&id).await;
    for (name, bytes) in [("Show.zip.001", first), ("Show.zip.002", second)] {
        let file = d.items[0].files.iter().find(|f| f.name == name).unwrap();
        replace_bytes(&s, file, bytes).await;
    }
    let now = Arc::new(AtomicI64::new(1_000));
    let go = CancellationToken::new();
    s.store
        .run
        .requeue_waiting_for_sources(1_000)
        .await
        .unwrap();
    s.runner_at(full_disk(), &now).run_ready(&go).await.unwrap();

    let d = s.detail(&id).await;
    assert_eq!(d.row.note.as_deref(), Some(UNPACK_AGAIN));
    let file = |d: &JobDetail, name: &str| {
        d.items[0]
            .files
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .clone()
    };
    let (one, two) = (file(&d, "Show.zip.001"), file(&d, "Show.zip.002"));
    assert_eq!((one.unpack_tries, two.unpack_tries), (1, 0));
    assert_eq!(two.volume_of.as_deref(), Some(one.id.as_str()));

    s.store
        .run
        .requeue_waiting_for_sources(now.load(Ordering::SeqCst))
        .await
        .unwrap();
    s.runner_at(extract(), &now).run_ready(&go).await.unwrap();
    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(
        names(&s.stored_dir()),
        ["Show - 02.ass", "Show - 03.ass", "Show - 04.ass"]
    );
    assert!(!s.area.at(&id).exists());
}

// An upload's archive this machine failed waits for its next try before
// the upload waits for its 배치 확인.
#[tokio::test]
async fn an_uploads_archive_is_tried_again_before_its_placement_is_confirmed() {
    let s = setup().await;
    let rar = std::fs::read(fixtures().join("pack.rar")).unwrap();
    let (id, _) = upload(&s, "u1", &[("subs", rar)]).await;
    let now = Arc::new(AtomicI64::new(1_000));
    let go = CancellationToken::new();
    s.runner_at(full_disk(), &now).run_ready(&go).await.unwrap();
    let d = s.detail(&id).await;
    assert_eq!(
        (d.row.state, d.row.wait, d.row.note.as_deref()),
        (JobState::Waiting, Some(Wait::Subtitle), Some(UNPACK_AGAIN))
    );
    assert_eq!(receive_note(&d), Some("압축 파일 1개".to_owned()));

    now.fetch_add(UNPACK_RETRY_AFTER, Ordering::SeqCst);
    let unpacks = s.runner_at(extract(), &now);
    assert_eq!(unpacks.requeue_unpack_retries().await.unwrap(), 1);
    unpacks.run_ready(&go).await.unwrap();
    let d = s.detail(&id).await;
    assert_eq!(d.row.wait, Some(Wait::Placement), "{:?}", d.row.note);
    assert_eq!(s.store.place.members(&id).await.unwrap().len(), 3);
}

// The third try this machine fails leaves the archive not unpacked: the job
// fails with why, the log says it was the last try, and no later start or
// hour tries it again.
#[tokio::test]
async fn the_third_failed_try_leaves_the_archive_not_unpacked() {
    let s = setup().await;
    video(&s, "02").await;
    let id = make(
        &s,
        "c1",
        &[("2", &zip_post("pack.zip", &["Show - 02.ass"]))],
    )
    .await;
    let now = Arc::new(AtomicI64::new(1_000));
    let go = CancellationToken::new();
    let full = s.runner_at(full_disk(), &now);
    full.run_ready(&go).await.unwrap();
    // Two worker starts, each trying it again at once.
    for _ in 0..2 {
        s.store
            .run
            .requeue_waiting_for_sources(now.load(Ordering::SeqCst))
            .await
            .unwrap();
        full.run_ready(&go).await.unwrap();
    }

    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Failed, "{:?}", d.row.note);
    assert_eq!(d.row.note, Some(format!("pack.zip: {DISK_FULL}")));
    let file = d.items[0].files[0].clone();
    assert_eq!(
        (
            file.unpack_tries,
            file.unpack_error.as_deref(),
            file.unpack_retry_at
        ),
        (3, Some(DISK_FULL), None)
    );
    let tries: Vec<(String, Option<String>)> = events(&d)
        .into_iter()
        .filter(|(m, _)| m.starts_with("pack.zip: "))
        .collect();
    assert_eq!(tries.len(), 3, "{tries:?}");
    assert!(tries.contains(&(
        "pack.zip: 압축 파일을 풀지 못했어요".to_owned(),
        Some(format!("{DISK_FULL} (3번 중 3번째 시도)"))
    )));

    let logged = s.count("subtitle_job_events").await;
    now.fetch_add(10 * UNPACK_RETRY_AFTER, Ordering::SeqCst);
    assert_eq!(full.requeue_unpack_retries().await.unwrap(), 0);
    s.store
        .run
        .requeue_waiting_for_sources(now.load(Ordering::SeqCst))
        .await
        .unwrap();
    s.runner_at(extract(), &now).run_ready(&go).await.unwrap();
    assert_eq!(s.count("subtitle_job_events").await, logged);
    assert!(s.area.at(file.path.as_deref().unwrap()).is_file());
    assert!(!s.work().join(".trss").exists());
}

// An archive with no file in it (a ZIP of a folder only; one with no entry
// at all fails its receipt's check) is the archive's own reason, not this
// machine's: it is not unpacked and not tried again.
#[tokio::test]
async fn an_empty_archive_is_not_tried_again() {
    let s = setup().await;
    let id = make(&s, "c1", &[("2", &zip_post("folder.zip", &["x.ass"]))]).await;
    s.runner(false)
        .run_ready(&CancellationToken::new())
        .await
        .unwrap();
    let d = s.detail(&id).await;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.add_directory("Fonts/", zip::write::SimpleFileOptions::default())
        .unwrap();
    let folder_only = zip.finish().unwrap().into_inner();
    replace_bytes(&s, &d.items[0].files[0], &folder_only).await;
    s.store
        .run
        .requeue_waiting_for_sources(5_000)
        .await
        .unwrap();
    s.run().await;

    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Failed, "{:?}", d.row.note);
    let file = &d.items[0].files[0];
    assert_eq!(
        (
            file.unpack_error.as_deref(),
            file.unpack_tries,
            file.unpack_retry_at
        ),
        (Some(trss_jobs::place::unpack::EMPTY_ARCHIVE), 0, None)
    );
}

// The volumes of a split ZIP (`.zip.001`, `.zip.002`) come as two files of
// the post: they are unpacked together and leave together.
#[tokio::test]
async fn the_volumes_of_a_split_archive_are_unpacked_and_cleared_together() {
    let s = setup().await;
    video(&s, "02").await;
    let id = make(
        &s,
        "c1",
        &[("2", "/pack/Show.zip.001/Show.zip.002/Show%20-%2002.ass")],
    )
    .await;
    // Received with a worker that cannot unpack yet, then given the bytes of
    // a real split ZIP, as a site would have sent them.
    s.runner(false)
        .run_ready(&CancellationToken::new())
        .await
        .unwrap();
    let whole = fake::zip_of(&["Show - 03.ass".to_owned(), "Show - 04.ass".to_owned()]);
    let (first, second) = whole.split_at(whole.len() / 2);
    let d = s.detail(&id).await;
    for (name, bytes) in [("Show.zip.001", first), ("Show.zip.002", second)] {
        let file = d.items[0].files.iter().find(|f| f.name == name).unwrap();
        replace_bytes(&s, file, bytes).await;
    }
    s.store
        .run
        .requeue_waiting_for_sources(5_000)
        .await
        .unwrap();
    s.run().await;

    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(
        names(&s.stored_dir()),
        ["Show - 02.ass", "Show - 03.ass", "Show - 04.ass"]
    );
    let files = &d.items[0].files;
    let by_name = |n: &str| files.iter().find(|f| f.name == n).unwrap();
    let first = by_name("Show.zip.001");
    assert!(first.unpacked_at.is_some());
    assert_eq!(
        by_name("Show.zip.002").volume_of.as_deref(),
        Some(first.id.as_str())
    );
    // Every receipt left, the second volume with its first, which stands for
    // it in the job's state.
    assert!(!s.area.at(&id).exists());
    let job = id.clone();
    let standing = s
        .db
        .run(move |c| trss_jobs::place::unpack::standing(c, &job).map_err(trss_core::DbError::from))
        .await
        .unwrap();
    assert_eq!(standing, (Vec::new(), false));

    // A restart cut a clearing short after the archive went: its unpack
    // folder alone is left, and the next run removes it and the job's folder.
    let leftover = s.area.at(&ReceiveArea::unpack_dir(&id, &first.id));
    std::fs::create_dir_all(&leftover).unwrap();
    std::fs::write(leftover.join("0"), b"left").unwrap();
    let job = id.clone();
    s.db.run(move |c| {
        c.execute(
            "UPDATE subtitle_jobs SET state = 'running', finished_at = NULL WHERE id = ?1",
            [job],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
    s.run().await;
    assert_eq!(s.detail(&id).await.row.state, JobState::Done);
    assert!(!s.area.at(&id).exists());
}

// Volumes of one name in two folders of a post (a Drive tree's `01/` and
// `02/`): each folder's are a set of their own.
#[tokio::test]
async fn volumes_of_one_name_in_two_folders_are_two_sets() {
    let s = setup().await;
    let id = make(
        &s,
        "c1",
        &[(
            "2",
            "/pack/01%2FShow.part1.rar/01%2FShow.part2.rar/02%2FShow.part1.rar/02%2FShow.part2.rar",
        )],
    )
    .await;
    s.runner(false)
        .run_ready(&CancellationToken::new())
        .await
        .unwrap();
    for file in &s.detail(&id).await.items[0].files {
        let fixture = fixtures().join(file.name.replace("Show", "split"));
        replace_bytes(&s, file, &std::fs::read(fixture).unwrap()).await;
    }
    s.store
        .run
        .requeue_waiting_for_sources(5_000)
        .await
        .unwrap();
    s.run().await;

    let d = s.detail(&id).await;
    let files = &d.items[0].files;
    for folder in ["01", "02"] {
        let file = |name: &str| {
            files
                .iter()
                .find(|f| f.folder.as_deref() == Some(folder) && f.name == name)
                .unwrap()
        };
        let first = file("Show.part1.rar");
        assert!(
            first.unpacked_at.is_some() && first.unpack_error.is_none(),
            "{folder}: {:?}",
            first.unpack_error
        );
        assert_eq!(
            file("Show.part2.rar").volume_of.as_deref(),
            Some(first.id.as_str())
        );
    }
}

// A set with its first volume missing is refused with why; the post's
// subtitle is applied all the same.
#[tokio::test]
async fn a_split_set_without_its_first_volume_says_why() {
    let s = setup().await;
    video(&s, "02").await;
    let id = make(&s, "c1", &[("2", "/pack/Show%20-%2002.ass/a.7z.002")]).await;
    s.run().await;

    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Partial, "{:?}", d.row.note);
    let volume = d.items[0]
        .files
        .iter()
        .find(|f| f.name == "a.7z.002")
        .unwrap();
    assert_eq!(
        volume.unpack_error.as_deref(),
        Some(trss_subtitles::upload::NO_FIRST_VOLUME)
    );
    assert_eq!(
        d.row.note,
        Some(format!(
            "a.7z.002: {}",
            trss_subtitles::upload::NO_FIRST_VOLUME
        ))
    );
    assert!(s.work().join("Season 01/Show S01E02.ass").is_file());
    assert_eq!(tree(&s.area.at("")), [volume.path.clone().unwrap()]);
}

// Uploads of RAR 5, 7z, `tar.xz` and a RAR in volumes (ticket 0065): the job
// waits for the worker, which unpacks each, and the job detail has what came
// of them. Their members wait for a person's 배치 확인 (0066).
#[tokio::test]
async fn uploaded_archives_of_each_format_are_unpacked_by_the_worker() {
    let s = setup().await;
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let uploads = trss_jobs::Uploads::new(s.store.requests.clone(), s.area.clone());
    let mut staging = uploads.begin().await.unwrap();
    let names = [
        "pack.rar",
        "pack.7z",
        "pack.tar.xz",
        "split.part1.rar",
        "split.part2.rar",
    ];
    for name in names {
        staging.start(name).await.unwrap();
        staging
            .write(&std::fs::read(fixtures.join(name)).unwrap())
            .await
            .unwrap();
        staging.end().await.unwrap();
    }
    let request = trss_jobs::upload::UploadRequest {
        command_id: "u1".to_owned(),
        work_id: WORK.to_owned(),
        season: 1,
        anime_no: None,
        source_id: None,
        creator: None,
    };
    let trss_jobs::Finished::Created { job_id: id, .. } =
        uploads.finish(staging, request, 900).await.unwrap()
    else {
        panic!("not made");
    };
    assert_eq!(s.detail(&id).await.row.state, JobState::Pending);
    s.run().await;

    let d = s.detail(&id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Placement)),
        "{:?}",
        d.row.note
    );
    assert_eq!(
        d.row.note.as_deref(),
        Some("자막 8개가 붙을 회차를 확인해 주세요")
    );
    assert_eq!(receive_note(&d).as_deref(), Some("압축 파일 5개"));
    let files = &d.items[0].files;
    let by_name = |n: &str| files.iter().find(|f| f.name == n).unwrap();
    let members = s.store.place.members(&id).await.unwrap();
    for archive in ["pack.rar", "pack.7z", "pack.tar.xz", "split.part1.rar"] {
        let file = by_name(archive);
        assert!(
            file.unpacked_at.is_some() && file.unpack_error.is_none(),
            "{archive}: {:?}",
            file.unpack_error
        );
        let of: Vec<(&str, Option<trss_subtitles::verify::Format>)> = members
            .iter()
            .filter(|m| m.file_id == file.id)
            .map(|m| (m.path.as_str(), m.format.clone().ok()))
            .collect();
        assert_eq!(
            of,
            [
                ("Show - 01.ass", Some(trss_subtitles::verify::Format::Ass)),
                ("Show - 02.ass", Some(trss_subtitles::verify::Format::Ass)),
                ("fonts/A.ttf", Some(trss_subtitles::verify::Format::Other)),
            ],
            "{archive}"
        );
    }
    assert_eq!(
        by_name("split.part2.rar").volume_of.as_deref(),
        Some(by_name("split.part1.rar").id.as_str())
    );
    // Nothing is kept before the 배치 확인: the archives and their members
    // stay received.
    let plan = s.store.place.plan(&id).await.unwrap();
    assert_eq!(plan.len(), 12);
    assert!(plan.iter().all(|r| !r.kept() && r.outcome.is_none()));
    assert!(!s.work().join(".trss").exists());
    for file in files {
        assert!(s.area.at(file.path.as_deref().unwrap()).is_file());
    }
    assert!(s
        .area
        .at(&ReceiveArea::unpack_dir(&id, &by_name("pack.rar").id))
        .join("0")
        .is_file());
    // Another run leaves them as they are.
    let seen = s.count("subtitle_job_members").await;
    assert_eq!(seen, 12);
    s.run().await;
    assert_eq!(s.count("subtitle_job_members").await, seen);
}

// An upload that kept an archive (a RAR with no extension, kept by its
// bytes) waits for a worker with the program, and keeps the note of what it
// kept on its receiving step once it is unpacked; then it waits for its
// 배치 확인.
#[tokio::test]
async fn an_upload_waits_for_the_program_and_keeps_what_it_kept() {
    let s = setup().await;
    let rar = std::fs::read(fixtures().join("pack.rar")).unwrap();
    let (id, dropped) = upload(&s, "u1", &[("subs", rar)]).await;
    assert_eq!(dropped, Vec::<String>::new());
    let received = Some("압축 파일 1개".to_owned());
    s.runner(false)
        .run_ready(&CancellationToken::new())
        .await
        .unwrap();
    let d = s.detail(&id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Subtitle))
    );
    assert_eq!(d.row.note.as_deref(), Some(trss_jobs::place::ARCHIVE_LATER));
    assert_eq!(receive_note(&d), received);

    s.store
        .run
        .requeue_waiting_for_sources(5_000)
        .await
        .unwrap();
    s.run().await;
    let d = s.detail(&id).await;
    assert_eq!(d.row.wait, Some(Wait::Placement), "{:?}", d.row.note);
    assert_eq!(
        d.row.note.as_deref(),
        Some("자막 2개가 붙을 회차를 확인해 주세요")
    );
    assert_eq!(receive_note(&d), received);
    let file = &d.items[0].files[0];
    assert!(file.unpacked_at.is_some(), "{:?}", file.unpack_error);
    assert_eq!(s.store.place.members(&id).await.unwrap().len(), 3);
}

/// Makes an upload job of the files `files` (name, bytes) and returns its ID.
async fn upload(s: &Setup, command: &str, files: &[(&str, Vec<u8>)]) -> (String, Vec<String>) {
    let uploads = trss_jobs::Uploads::new(s.store.requests.clone(), s.area.clone());
    let mut staging = uploads.begin().await.unwrap();
    for (name, bytes) in files {
        staging.start(name).await.unwrap();
        staging.write(bytes).await.unwrap();
        staging.end().await.unwrap();
    }
    let request = trss_jobs::upload::UploadRequest {
        command_id: command.to_owned(),
        work_id: WORK.to_owned(),
        season: 1,
        anime_no: None,
        source_id: None,
        creator: None,
    };
    match uploads.finish(staging, request, 900).await.unwrap() {
        trss_jobs::Finished::Created {
            job_id, dropped, ..
        } => (
            job_id,
            dropped
                .into_iter()
                .map(|d| format!("{}: {}", d.name, d.reason))
                .collect(),
        ),
        other => panic!("{other:?}"),
    }
}

/// A ZIP of `members`, deflated.
fn deflated_zip(members: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in members {
        zip.start_file(*name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

// Bombs and a password (ticket 0065): each archive alone is not unpacked,
// with why, and stays received; the rest of the job, and the next job, go on
// to their 배치 확인, and the job partly failed once it is placed.
#[tokio::test]
async fn bombs_and_a_password_fail_alone_and_the_next_job_goes_on() {
    let s = setup().await;
    let hostile = Path::new(env!("CARGO_MANIFEST_DIR")).join("../trss-archive/tests/fixtures");
    let fixture = |name: &str| std::fs::read(hostile.join(name)).unwrap();
    let zeros = vec![0u8; 64 << 20];
    let many: Vec<(String, Vec<u8>)> = (0..2001)
        .map(|i| (format!("{i}.txt"), Vec::new()))
        .collect();
    let many: Vec<(&str, &[u8])> = many
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let good = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pack.rar"))
        .unwrap();
    let (id, dropped) = upload(
        &s,
        "u1",
        &[
            ("ratio.zip", deflated_zip(&[("zeros.ass", &zeros)])),
            ("many.zip", deflated_zip(&many)),
            ("nested.zip", fixture("nested30.zip")),
            ("dict.xz", fixture("dict1536m.xz")),
            ("dict.rar", fixture("d128m.rar")),
            ("password.zip", fixture("enc_aes.zip")),
            ("good.rar", good),
        ],
    )
    .await;
    assert_eq!(dropped, Vec::<String>::new());
    let (next, _) = upload(
        &s,
        "u2",
        &[(
            "pack.7z",
            std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pack.7z"))
                .unwrap(),
        )],
    )
    .await;
    s.run().await;

    let d = s.detail(&id).await;
    let errors: Vec<(String, Option<String>)> = d.items[0]
        .files
        .iter()
        .map(|f| (f.name.clone(), f.unpack_error.clone()))
        .collect();
    let error = |n: &str| errors.iter().find(|(name, _)| name == n).unwrap().1.clone();
    assert_eq!(
        error("ratio.zip").as_deref(),
        Some("풀린 크기가 압축 파일 크기에 비해 너무 커요")
    );
    assert_eq!(error("many.zip").as_deref(), Some("멤버가 2000개를 넘어요"));
    assert_eq!(
        error("nested.zip").as_deref(),
        Some("압축 파일 안의 압축 파일이 3단을 넘어요")
    );
    assert_eq!(
        error("dict.xz").as_deref(),
        Some("사전 크기가 64 MB를 넘어요")
    );
    assert_eq!(
        error("dict.rar").as_deref(),
        Some("사전 크기가 64 MB를 넘어요")
    );
    assert_eq!(error("password.zip").as_deref(), Some("암호가 걸려 있어요"));
    assert_eq!(error("good.rar"), None);
    // What was unpacked waits for its 배치 확인.
    assert_eq!(d.row.wait, Some(Wait::Placement), "{:?}", d.row.note);
    // Each stays received, and only the good one has an unpack folder.
    for file in &d.items[0].files {
        assert!(
            s.area.at(file.path.as_deref().unwrap()).is_file(),
            "{}",
            file.name
        );
        let folder = s.area.at(&ReceiveArea::unpack_dir(&id, &file.id));
        assert_eq!(folder.exists(), file.name == "good.rar", "{}", file.name);
    }
    assert_eq!(s.detail(&next).await.row.wait, Some(Wait::Placement));

    // Placed, something of it was kept: partly failed, the first reason its
    // note.
    video(&s, "01").await;
    video(&s, "02").await;
    assert_eq!(
        confirm_as_planned(&s, &id).await,
        Confirmed::Queued {
            applied: 2,
            stored: 0
        }
    );
    s.run().await;
    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Partial, "{:?}", d.row.note);
    assert_eq!(
        d.row.note.as_deref(),
        Some("ratio.zip: 풀린 크기가 압축 파일 크기에 비해 너무 커요")
    );
    assert!(s.work().join("Season 01/Show S01E01.ass").is_file());
    assert!(s.work().join("Season 01/Show S01E02.ass").is_file());
}
