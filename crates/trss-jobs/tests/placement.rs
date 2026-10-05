//! 배치 확인 (ticket 0066, `docs/specs/subtitles.md`): an upload's or a find
//! job's files go on the episodes their names say, through the source's
//! mapping, and nothing of them is kept until a person confirms the table;
//! then the job keeps and applies what the person placed.

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
};

use rusqlite::params;
use tokio_util::sync::CancellationToken;
use trss_core::{Clock, Db};
use trss_jobs::{
    model::{AssetKind, Outcome, PlanAction, StepKind, StepState},
    place::{
        episode::{Assignment, Basis},
        records::{Confirmed, PlanRow, RowPlacing},
    },
    store::JobDetail,
    upload::UploadRequest,
    Finished, JobState, JobStore, ReceiveArea, Runner, Uploads, Wait,
};
use trss_subtitles::{fake::FakeSource, Sources};

const WORK: &str = "w1";
const CREATOR: &str = "제작자";

struct Setup {
    dir: tempfile::TempDir,
    db: Db,
    store: JobStore,
    area: ReceiveArea,
}

impl Setup {
    fn work(&self) -> PathBuf {
        self.dir.path().join("shows/Show")
    }

    async fn run(&self) {
        Runner::new(
            self.store.clone(),
            Sources::none().with_fake(FakeSource),
            self.area.clone(),
            ticking_clock(),
        )
        .run_ready(&CancellationToken::new())
        .await
        .unwrap();
    }

    async fn detail(&self, id: &str) -> JobDetail {
        self.store.detail(id).await.unwrap().unwrap()
    }

    async fn plan(&self, id: &str) -> Vec<PlanRow> {
        self.store.plan(id).await.unwrap()
    }

    async fn sql(&self, sql: &'static str) {
        self.db
            .run(move |c| c.execute_batch(sql).map_err(trss_core::DbError::from))
            .await
            .unwrap();
    }

    /// Confirms the job's table: `placings` by the row's name.
    async fn confirm(&self, id: &str, placings: &[(&str, Option<i64>, bool)]) -> Confirmed {
        let plan = self.plan(id).await;
        let placings = placings
            .iter()
            .map(|(name, episode, apply)| RowPlacing {
                position: plan.iter().find(|r| r.name == *name).unwrap().position,
                episode: *episode,
                apply: *apply,
            })
            .collect();
        self.store
            .confirm_placement(id, placings, Some(12), 50_000)
            .await
            .unwrap()
    }
}

fn ticking_clock() -> Clock {
    let now = Arc::new(AtomicI64::new(1_000));
    Arc::new(move || now.fetch_add(10, Ordering::SeqCst))
}

/// A library with the work `Show`, whose season 2 has twelve episodes
/// (AniList), each with its video, and the creator's source `src`.
async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let shows = dir.path().join("shows");
    std::fs::create_dir_all(shows.join("Show/Season 02")).unwrap();
    for n in 1..=12 {
        std::fs::write(shows.join(format!("Show/{}", video(n))), b"video").unwrap();
    }
    let path = shows.to_string_lossy().into_owned();
    db.run(move |c| {
        c.execute(
            "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', ?1, 0)",
            [path],
        )?;
        c.execute_batch(&format!(
            "INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('{WORK}', 'f1', 'Show');
             INSERT INTO seasons (work_id, number) VALUES ('{WORK}', 2);
             INSERT OR IGNORE INTO season_info (work_id, season) VALUES ('{WORK}', 2);
             INSERT INTO anilist_entries (id, format, episodes, fetched_at)
                 VALUES (1, 'TV', 12, 1);
             INSERT INTO season_entries (work_id, season, position, anilist_id)
                 VALUES ('{WORK}', 2, 0, 1);
             INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                 VALUES ('src', 7, '{CREATOR}', 0);"
        ))?;
        for n in 1..=12 {
            let episode = format!("{n:02}");
            c.execute(
                "INSERT INTO episodes (work_id, season, episode) VALUES (?1, 2, ?2)",
                params![WORK, episode],
            )?;
            c.execute(
                "INSERT INTO media_files (work_id, path, season, episode, kind)
                 VALUES (?1, ?2, 2, ?3, 'video')",
                params![WORK, video(n), episode],
            )?;
        }
        Ok::<_, trss_core::DbError>(())
    })
    .await
    .unwrap();
    let store = JobStore::new(db.clone());
    let area = ReceiveArea::in_app_data(dir.path());
    Setup {
        dir,
        db,
        store,
        area,
    }
}

/// The video of episode `n`, from the work's folder.
fn video(n: i64) -> String {
    format!("Season 02/Show S02E{n:02}.mkv")
}

/// An ASS subtitle of its own bytes.
fn ass(n: i64) -> Vec<u8> {
    format!(
        "[Script Info]\nTitle: {n}\nScriptType: v4.00+\n\n[Events]\n\
         Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
         Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{n}\n"
    )
    .into_bytes()
}

/// Uploads `Show - NN.ass` for each of `numbers` to season 2, as the
/// creator's (`src`) or as `제작자 알 수 없음`.
async fn upload(
    s: &Setup,
    command: &str,
    numbers: impl Iterator<Item = i64>,
    creator: bool,
) -> String {
    let files: Vec<(String, Vec<u8>)> = numbers
        .map(|n| (format!("Show - {n:02}.ass"), ass(n)))
        .collect();
    upload_files(s, command, &files, creator).await
}

/// Uploads `files` (name, bytes) to season 2.
async fn upload_files(
    s: &Setup,
    command: &str,
    files: &[(String, Vec<u8>)],
    creator: bool,
) -> String {
    let uploads = Uploads::new(s.store.clone(), s.area.clone());
    let mut staging = uploads.begin().await.unwrap();
    for (name, bytes) in files {
        staging.start(name).await.unwrap();
        staging.write(bytes).await.unwrap();
        staging.end().await.unwrap();
    }
    let request = UploadRequest {
        command_id: command.to_owned(),
        work_id: WORK.to_owned(),
        season: 2,
        anime_no: Some(7),
        source_id: creator.then(|| "src".to_owned()),
        creator: creator.then(|| CREATOR.to_owned()),
    };
    match uploads.finish(staging, request, 900).await.unwrap() {
        Finished::Created { job_id, .. } => job_id,
        other => panic!("{other:?}"),
    }
}

fn placed(row: &PlanRow) -> Option<(i64, Assignment)> {
    row.placed.as_ref().map(|p| (p.episode, p.assignment))
}

fn step(d: &JobDetail, kind: StepKind) -> Option<(StepState, Option<String>)> {
    d.steps
        .iter()
        .find(|s| s.step == kind)
        .map(|s| (s.state, s.note.clone()))
}

/// Every file under `dir`, as paths relative to it.
fn tree(dir: &std::path::Path) -> Vec<String> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
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

/// What the work's folder had before any subtitle: the twelve videos.
fn videos() -> Vec<String> {
    let mut all: Vec<String> = (1..=12).map(video).collect();
    all.sort();
    all
}

#[tokio::test]
async fn an_upload_of_an_unknown_creator_waits_with_its_names_and_holds_one_outside_the_season() {
    let s = setup().await;
    let id = upload(&s, "u1", 1..=13, false).await;
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
        Some("자막 13개가 붙을 회차를 확인해 주세요 · 회차를 정할 파일 1개")
    );
    assert_eq!(
        step(&d, StepKind::Placement).map(|(state, _)| state),
        Some(StepState::Waiting)
    );
    assert_eq!(step(&d, StepKind::Store), None);
    let plan = s.plan(&id).await;
    assert_eq!(plan.len(), 13);
    // Each name's number is the episode: no mapping, so the person's own.
    for (n, row) in (1..=12).zip(&plan) {
        assert_eq!(row.name, format!("Show - {n:02}.ass"));
        assert_eq!(row.kind, AssetKind::Subtitle);
        assert_eq!(placed(row), Some((n, Assignment::Explicit)), "{row:?}");
        assert_eq!(row.action, PlanAction::Apply);
        assert_eq!((row.question.as_deref(), row.outcome), (None, None));
    }
    let held = &plan[12];
    assert_eq!(placed(held), None);
    assert_eq!(
        held.question.as_deref(),
        Some("13화가 시즌의 1–12화 밖이에요")
    );
    // Nothing is written before the person confirms: one to-do asks for it.
    assert!(!s.work().join(".trss").exists());
    assert_eq!(tree(&s.work()), videos());
    let waits = s.store.placement_waits().await.unwrap();
    assert_eq!(waits.len(), 1);
    assert_eq!(waits[0].id, id);

    // Left alone, it stays so: no restart's look takes it back in line.
    assert_eq!(
        s.store.requeue_waiting_for_sources(60_000).await.unwrap(),
        0
    );
    s.run().await;
    assert_eq!(s.detail(&id).await.row.wait, Some(Wait::Placement));
    assert_eq!(tree(&s.work()), videos());
}

#[tokio::test]
async fn the_confirmed_table_applies_its_episodes_and_stores_the_one_not_applied() {
    let s = setup().await;
    let id = upload(&s, "u1", 1..=13, false).await;
    s.run().await;
    let mut placings: Vec<(String, Option<i64>, bool)> = (1..=12)
        .map(|n| (format!("Show - {n:02}.ass"), Some(n), true))
        .collect();
    placings.push(("Show - 13.ass".to_owned(), None, false));
    let placings: Vec<(&str, Option<i64>, bool)> = placings
        .iter()
        .map(|(n, e, a)| (n.as_str(), *e, *a))
        .collect();
    assert_eq!(
        s.confirm(&id, &placings).await,
        Confirmed::Queued {
            applied: 12,
            stored: 1
        }
    );
    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Pending);
    assert_eq!(
        step(&d, StepKind::Placement),
        Some((StepState::Done, Some("적용 12개 · 보관만 1개".to_owned())))
    );
    assert!(s.plan(&id).await.iter().all(|r| r.question.is_none()));
    assert!(s.store.placement_waits().await.unwrap().is_empty());

    s.run().await;
    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    let plan = s.plan(&id).await;
    for row in &plan[..12] {
        assert_eq!(row.outcome, Some(Outcome::Applied), "{row:?}");
    }
    assert_eq!(plan[12].outcome, Some(Outcome::Stored));
    assert_eq!(placed(&plan[12]), None);
    for n in 1..=12 {
        let applied = s.work().join(format!("Season 02/Show S02E{n:02}.ass"));
        assert_eq!(std::fs::read(&applied).unwrap(), ass(n), "{n}");
    }
    // All thirteen are kept in the work's `.trss/`, the thirteenth beside no
    // video.
    let kept: Vec<String> = tree(&s.work())
        .into_iter()
        .filter(|p| p.starts_with(".trss/subtitles/"))
        .collect();
    assert_eq!(kept.len(), 13, "{kept:?}");
    let beside: Vec<String> = tree(&s.work())
        .into_iter()
        .filter(|p| p.starts_with("Season 02/") && p.ends_with(".ass"))
        .collect();
    assert_eq!(beside.len(), 12, "{beside:?}");
    // A second confirmation finds nothing to place.
    assert_eq!(s.confirm(&id, &placings).await, Confirmed::NotWaiting);
}

#[tokio::test]
async fn the_creators_mapping_moves_the_names_onto_the_season_and_a_moved_row_is_the_persons() {
    let s = setup().await;
    s.sql(
        "INSERT INTO subtitle_episode_mappings
             (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
         VALUES ('w1', 2, 'src', 'user', -12, '시험', 0);",
    )
    .await;
    let id = upload(&s, "u1", 13..=24, true).await;
    s.run().await;
    let d = s.detail(&id).await;
    assert_eq!(d.row.wait, Some(Wait::Placement), "{:?}", d.row.note);
    assert_eq!(
        d.row.note.as_deref(),
        Some("자막 12개가 붙을 회차를 확인해 주세요")
    );
    let plan = s.plan(&id).await;
    for (n, row) in (1..=12).zip(&plan) {
        assert_eq!(row.name, format!("Show - {:02}.ass", n + 12));
        assert_eq!(placed(row), Some((n, Assignment::Mapped)), "{row:?}");
        assert_eq!(row.placed.as_ref().unwrap().basis, Some(Basis::Attachment));
        assert_eq!(row.attachment_episode, Some(format!("{:02}", n + 12)));
    }

    // The person swaps the first two: those are theirs, the rest the
    // mapping's.
    let mut placings: Vec<(String, Option<i64>, bool)> = (3..=12)
        .map(|n| (format!("Show - {:02}.ass", n + 12), Some(n), true))
        .collect();
    placings.push(("Show - 13.ass".to_owned(), Some(2), true));
    placings.push(("Show - 14.ass".to_owned(), Some(1), true));
    let placings: Vec<(&str, Option<i64>, bool)> = placings
        .iter()
        .map(|(n, e, a)| (n.as_str(), *e, *a))
        .collect();
    assert_eq!(
        s.confirm(&id, &placings).await,
        Confirmed::Queued {
            applied: 12,
            stored: 0
        }
    );
    let plan = s.plan(&id).await;
    assert_eq!(placed(&plan[0]), Some((2, Assignment::Explicit)));
    assert_eq!(placed(&plan[1]), Some((1, Assignment::Explicit)));
    for (n, row) in (3..=12).zip(&plan[2..]) {
        assert_eq!(placed(row), Some((n, Assignment::Mapped)), "{row:?}");
    }
    s.run().await;
    assert_eq!(s.detail(&id).await.row.state, JobState::Done);
    assert_eq!(
        std::fs::read(s.work().join("Season 02/Show S02E01.ass")).unwrap(),
        ass(14)
    );
    assert_eq!(
        std::fs::read(s.work().join("Season 02/Show S02E02.ass")).unwrap(),
        ass(13)
    );
}

#[tokio::test]
async fn a_table_that_cannot_be_kept_is_refused_and_one_of_other_rows_is_stale() {
    let s = setup().await;
    let id = upload(&s, "u1", 1..=3, false).await;
    s.run().await;
    let all = |third: (Option<i64>, bool)| {
        vec![
            ("Show - 01.ass", Some(1), true),
            ("Show - 02.ass", Some(2), true),
            ("Show - 03.ass", third.0, third.1),
        ]
    };
    let refused = |c: Confirmed| match c {
        Confirmed::Refused(why) => why,
        other => panic!("{other:?}"),
    };
    // An episode outside the season.
    assert!(refused(s.confirm(&id, &all((Some(13), true))).await).contains("13"));
    // Applied on no episode.
    refused(s.confirm(&id, &all((None, true))).await);
    // Two different files of one format applied on one episode.
    let why = refused(s.confirm(&id, &all((Some(2), true))).await);
    assert!(why.contains("2화"), "{why}");
    // A row left out is not the table the job asks about.
    assert_eq!(
        s.confirm(&id, &all((Some(3), true))[..2]).await,
        Confirmed::Stale
    );
    // Nothing of these was taken.
    let d = s.detail(&id).await;
    assert_eq!(d.row.wait, Some(Wait::Placement));
    assert!(s.plan(&id).await.iter().all(|r| r.outcome.is_none()));
    // The same file stored only beside another applied there is fine.
    assert_eq!(
        s.confirm(&id, &all((Some(2), false))).await,
        Confirmed::Queued {
            applied: 2,
            stored: 1
        }
    );
    assert_eq!(
        s.store
            .confirm_placement("missing", Vec::new(), Some(12), 1)
            .await
            .unwrap(),
        Confirmed::NotFound
    );
}

#[tokio::test]
async fn the_migration_puts_received_uploads_and_find_jobs_back_in_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let conn = trss_core::db::database_at(&path, 53);
    conn.execute_batch(
        "INSERT INTO subtitle_jobs (id, command_id, request, origin, state, created_at,
                                    updated_at, state_at, wait, stage, finished_at, note)
         VALUES ('u', 'u', '{}', 'upload', 'done', 0, 0, 0, NULL, NULL, 5, '올린 파일: 자막 1개'),
                ('f', 'f', '{}', 'find', 'done', 0, 0, 0, NULL, NULL, 5, '받은 파일: 자막 1개'),
                ('n', 'n', '{}', 'find', 'done', 0, 0, 0, NULL, NULL, 5, '받은 파일 없음'),
                ('c', 'c', '{}', 'upload', 'done', 0, 0, 0, NULL, NULL, 5, '올린 파일: 자막 1개'),
                ('p', 'p', '{}', 'pick', 'done', 0, 0, 0, NULL, NULL, 5, NULL);
         INSERT INTO subtitle_job_items (id, job_id, position, episode, post_url, found_at,
                                         state, updated_at)
         VALUES (1, 'u', 0, '', 'upload:', 0, 'done', 5), (2, 'f', 0, '', 'https://x/', 0, 'done', 5),
                (3, 'n', 0, '', 'https://x/', 0, 'done', 5), (4, 'c', 0, '', 'upload:', 0, 'done', 5),
                (5, 'p', 0, '01', 'https://x/', 0, 'done', 5);
         INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state, path,
                                         created_at, updated_at, cleared_at)
         VALUES ('fu', 'u', 1, 'k', '01.ass', 'done', 'u/01.ass', 0, 0, NULL),
                ('ff', 'f', 2, 'k', '01.ass', 'done', 'f/01.ass', 0, 0, NULL),
                ('fc', 'c', 4, 'k', '01.ass', 'done', NULL, 0, 0, 3),
                ('fp', 'p', 5, 'k', '01.ass', 'done', 'p/01.ass', 0, 0, NULL);",
    )
    .unwrap();
    drop(conn);
    let db = Db::open(&path).await.unwrap();
    let states: Vec<(String, String, bool, Option<String>)> = db
        .run(|c| {
            let mut stmt = c.prepare(
                "SELECT id, state, finished_at IS NOT NULL, note FROM subtitle_jobs ORDER BY id",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    let again = Some("받아 둔 파일의 배치 확인을 준비해요".to_owned());
    assert_eq!(
        states,
        [
            // An upload whose files are gone, a find job with none, and a
            // candidate's job stay as they were.
            (
                "c".to_owned(),
                "done".to_owned(),
                true,
                Some("올린 파일: 자막 1개".to_owned())
            ),
            ("f".to_owned(), "pending".to_owned(), false, again.clone()),
            (
                "n".to_owned(),
                "done".to_owned(),
                true,
                Some("받은 파일 없음".to_owned())
            ),
            ("p".to_owned(), "done".to_owned(), true, None),
            ("u".to_owned(), "pending".to_owned(), false, again),
        ]
    );
    let store = JobStore::new(db);
    let f = store.detail("f").await.unwrap().unwrap();
    // The find job's 받기 had ended: it goes on to its placement.
    assert!(!f.row.receiving && !f.row.finishing);
    assert!(!store.placement_confirmed("u").await.unwrap());
}

#[tokio::test]
async fn a_package_of_fonts_alone_is_confirmed_with_no_row_and_then_kept() {
    let s = setup().await;
    let mut font = b"\x00\x01\x00\x00\x00\x0C".to_vec();
    font.resize(12 + 16 * 12, 0);
    let id = upload_files(&s, "u1", &[("A.ttf".to_owned(), font)], false).await;
    s.run().await;
    let d = s.detail(&id).await;
    assert_eq!(d.row.wait, Some(Wait::Placement), "{:?}", d.row.note);
    assert_eq!(
        d.row.note.as_deref(),
        Some("받은 폰트와 첨부를 보관하기 전에 확인해 주세요")
    );
    assert_eq!(tree(&s.work()), videos());

    assert_eq!(
        s.store
            .confirm_placement(&id, Vec::new(), Some(12), 50_000)
            .await
            .unwrap(),
        Confirmed::Queued {
            applied: 0,
            stored: 0
        }
    );
    assert_eq!(
        step(&s.detail(&id).await, StepKind::Placement),
        Some((StepState::Done, Some("폰트와 첨부만 보관해요".to_owned())))
    );
    s.run().await;
    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    let plan = s.plan(&id).await;
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].kind, AssetKind::Font);
    assert!(plan[0].kept(), "{:?}", plan[0]);
}
