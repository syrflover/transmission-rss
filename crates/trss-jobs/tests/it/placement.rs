//! 배치 확인 (ticket 0066, `docs/specs/subtitles.md`): an upload's or a find
//! job's files go on the episodes their names say, through the source's
//! mapping, and nothing of them is kept until a person confirms the table;
//! then the job keeps and applies what the person placed.

use crate::{
    world::{ticking_clock, Base, Shows},
    Handles,
};
use std::path::PathBuf;

use rusqlite::params;
use tokio_util::sync::CancellationToken;
use trss_core::Db;
use trss_jobs::{
    model::{AssetKind, Outcome, PlanAction, PlanState, StepKind, StepState},
    place::{
        episode::{Assignment, Basis},
        records::{Confirmed, PlanRow, RowPlacing},
        replace::records::Decided,
    },
    store::JobDetail,
    upload::UploadRequest,
    Created, Finished, JobState, NewItem, NewJob, ReceiveArea, Runner, Uploads, Wait,
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

    async fn run(&self) {
        Runner::new(
            self.store.run.clone(),
            Sources::none().with_fake(FakeSource),
            self.area.clone(),
            ticking_clock(),
        )
        .run_ready(&CancellationToken::new())
        .await
        .unwrap();
    }

    async fn detail(&self, id: &str) -> JobDetail {
        self.store.views.detail(id).await.unwrap().unwrap()
    }

    async fn plan(&self, id: &str) -> Vec<PlanRow> {
        self.store.place.plan(id).await.unwrap()
    }

    async fn sql(&self, sql: &'static str) {
        self.db
            .run(move |c| c.execute_batch(sql).map_err(trss_core::DbError::from))
            .await
            .unwrap();
    }

    /// A stored subtitle's link and asset: (episode, assignment, basis,
    /// asset, the job that stored it).
    async fn stored(&self, id: &str) -> (i64, String, Option<String>, String, String) {
        let id = id.to_owned();
        self.db
            .run(move |c| {
                c.query_row(
                    "SELECT episode, assignment, basis, subtitle_asset_id, job_id
                       FROM subtitle_stored WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .map_err(trss_core::DbError::from)
            })
            .await
            .unwrap()
    }

    /// Decides the job's one live replacement plan.
    async fn decide(&self, id: &str, replace: bool) -> Decided {
        let mut views = self.store.place.replacements(id).await.unwrap();
        let plan = views.pop().unwrap().plan;
        assert_eq!(plan.state, PlanState::Open, "{plan:?}");
        self.store
            .place
            .decide_replacement(id, &plan.id, plan.version, replace, 5_000_000)
            .await
            .unwrap()
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
            .place
            .confirm_placement(id, placings, Vec::new(), Some(12), 50_000)
            .await
            .unwrap()
    }
}

/// A library with the work `Show`, whose season 2 has twelve episodes
/// (AniList), each with its video, and the creator's source `src`.
async fn setup() -> Setup {
    let base = Base::new().await;
    base.library(&Shows {
        season: 2,
        episodes: (1..=12).collect(),
        source: true,
        ..Shows::default()
    })
    .await;
    base.db
        .run(|c| {
            c.execute_batch(&format!(
                "INSERT OR IGNORE INTO season_info (work_id, season) VALUES ('{WORK}', 2);
                 INSERT INTO anilist_entries (id, format, episodes, fetched_at)
                     VALUES (1, 'TV', 12, 1);
                 INSERT INTO season_entries (work_id, season, position, anilist_id)
                     VALUES ('{WORK}', 2, 0, 1);"
            ))
            .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    Setup {
        dir: base.dir,
        db: base.db,
        store: base.store,
        area: base.area,
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
    let uploads = Uploads::new(s.store.requests.clone(), s.area.clone());
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

/// A pick's job of the creator's (`src`) fake post `path` (on
/// [`fake::HOST`]), as Anissia's episode `episode`.
async fn pick(s: &Setup, command: &str, episode: &str, path: &str) -> String {
    let job = NewJob {
        command_id: command.to_owned(),
        request: format!("{{\"c\":\"{command}\"}}"),
        origin: "pick".to_owned(),
        work_id: Some(WORK.to_owned()),
        season: Some(2),
        anime_no: Some(7),
        source_id: Some("src".to_owned()),
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
    match s.store.requests.create(job, 900).await.unwrap() {
        Created::Created(id) => id,
        other => panic!("created: {other:?}"),
    }
}

/// A subtitle at episode 2's path the app did not put there.
const MINE: &str =
    "[Script Info]\n[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,내 자막\n";

/// The creator's mapping decided (each number the season's own), episode 2
/// holding [`MINE`], and the creator's pick of `/ok/Show-02` kept stored only
/// there by the person: the pick's job, and its row.
async fn picked_and_kept(s: &Setup) -> (String, PlanRow) {
    s.sql(
        "INSERT INTO subtitle_episode_mappings
             (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
         VALUES ('w1', 2, 'src', 'user', 0, '시험', 0);",
    )
    .await;
    std::fs::write(s.work().join("Season 02/Show S02E02.ass"), MINE).unwrap();
    let first = pick(s, "c1", "2", "/ok/Show-02").await;
    s.run().await;
    assert_eq!(s.detail(&first).await.row.wait, Some(Wait::Approval));
    assert_eq!(
        s.decide(&first, false).await,
        Decided::Done(PlanState::Kept)
    );
    s.run().await;
    assert_eq!(s.detail(&first).await.row.state, JobState::Done);
    let row = s.plan(&first).await.remove(0);
    let link = row.placed.clone().unwrap();
    assert_eq!(
        (link.episode, link.assignment, link.basis),
        (2, Assignment::Mapped, Some(Basis::Anissia))
    );
    (first, row)
}

/// The same bytes under the same name, uploaded as the creator's and placed
/// on episode 2 by the file's number: the upload's job, waiting to replace
/// [`MINE`].
async fn uploaded_on_the_same_file(s: &Setup) -> String {
    let files = [("Show-02.ass".to_owned(), fake::ass("Show-02"))];
    let id = upload_files(s, "u1", &files, true).await;
    s.run().await;
    assert_eq!(
        s.confirm(&id, &[("Show-02.ass", Some(2), true)]).await,
        Confirmed::Queued {
            applied: 1,
            stored: 0
        }
    );
    s.run().await;
    assert_eq!(s.detail(&id).await.row.wait, Some(Wait::Approval));
    id
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
    let waits = s.store.views.placement_waits().await.unwrap();
    assert_eq!(waits.len(), 1);
    assert_eq!(waits[0].id, id);

    // Left alone, it stays so: no restart's look takes it back in line.
    assert_eq!(
        s.store
            .run
            .requeue_waiting_for_sources(60_000)
            .await
            .unwrap(),
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
    assert!(s.store.views.placement_waits().await.unwrap().is_empty());

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
            .place
            .confirm_placement("missing", Vec::new(), Vec::new(), Some(12), 1)
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
    let store = Handles::new(db);
    let f = store.views.detail("f").await.unwrap().unwrap();
    // The find job's 받기 had ended: it goes on to its placement.
    assert!(!f.row.receiving && !f.row.finishing);
    assert!(!store.run.placement_confirmed("u").await.unwrap());
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
            .place
            .confirm_placement(&id, Vec::new(), Vec::new(), Some(12), 50_000)
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

#[tokio::test]
async fn an_upload_of_bytes_a_pick_stored_keeps_its_own_link_and_is_applied_once_approved() {
    let s = setup().await;
    let (_, picked) = picked_and_kept(&s).await;
    let id = uploaded_on_the_same_file(&s).await;
    // The file is kept once, but the upload's link is its own: by the
    // file's number, not Anissia's episode.
    let row = s.plan(&id).await.remove(0);
    let link = row.placed.clone().unwrap();
    assert_eq!(
        (link.episode, link.assignment, link.basis),
        (2, Assignment::Mapped, Some(Basis::Attachment))
    );
    let (mine, theirs) = (
        s.stored(row.stored_id.as_deref().unwrap()).await,
        s.stored(picked.stored_id.as_deref().unwrap()).await,
    );
    assert_ne!(row.stored_id, picked.stored_id);
    assert_eq!(
        (mine.0, mine.1.as_str(), mine.2.as_deref(), &mine.4),
        (2, "mapped", Some("attachment"), &id)
    );
    assert_eq!(mine.3, theirs.3, "one asset");
    assert_eq!(theirs.2.as_deref(), Some("anissia"));

    // Approved, it replaces the subtitle there with no second look.
    assert_eq!(
        s.decide(&id, true).await,
        Decided::Done(PlanState::Approved)
    );
    s.run().await;
    let d = s.detail(&id).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(
        std::fs::read(s.work().join("Season 02/Show S02E02.ass")).unwrap(),
        fake::ass("Show-02")
    );
    let plans: i64 =
        s.db.run(move |c| {
            c.query_row(
                "SELECT count(*) FROM subtitle_replacements WHERE job_id = ?1",
                [id],
                |r| r.get(0),
            )
            .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    assert_eq!(plans, 1, "the approved plan held");
}

#[tokio::test]
async fn a_row_on_a_stored_subtitle_of_another_link_is_given_its_own_before_it_is_compared() {
    let s = setup().await;
    let (_, picked) = picked_and_kept(&s).await;
    let id = uploaded_on_the_same_file(&s).await;
    let own = s.plan(&id).await.remove(0).stored_id.unwrap();
    let theirs = picked.stored_id.clone().unwrap();
    // As a build that told stored subtitles apart by episode and assignment
    // only left it: the upload's row, and the plan made for it, on the pick's
    // stored subtitle, and none of its own (the one it had is cleaned).
    s.db.run({
        let (id, own, theirs) = (id.clone(), own.clone(), theirs.clone());
        move |c| {
            let made: String = c.query_row(
                "SELECT id FROM subtitle_replacements WHERE job_id = ?1",
                [&id],
                |r| r.get(0),
            )?;
            c.execute(
                "UPDATE subtitle_replacements SET state = 'stale' WHERE id = ?1",
                [&made],
            )?;
            c.execute(
                "INSERT INTO subtitle_replacements
                     (id, job_id, position, version, state, work_id, season, episode, assignment,
                      basis, folder, video_path, video_object, video_size, video_mtime, stored_id,
                      asset_id, asset_path, asset_size, asset_sha256, asset_lines, target,
                      created_at, updated_at)
                 SELECT 'legacy', job_id, position, version + 1, 'open', work_id, season,
                        episode, assignment, basis, folder, video_path, video_object, video_size,
                        video_mtime, ?2, asset_id, asset_path, asset_size, asset_sha256,
                        asset_lines, target, created_at, updated_at
                   FROM subtitle_replacements WHERE id = ?1",
                params![made, theirs],
            )?;
            c.execute(
                "INSERT INTO subtitle_replacement_paths
                 SELECT 'legacy', path, action, byte_size, sha256, object, mtime, lines, applied_id
                   FROM subtitle_replacement_paths WHERE plan_id = ?1",
                [&made],
            )?;
            c.execute(
                "INSERT INTO subtitle_replacement_diffs
                 SELECT 'legacy', path, diff, lines, unreadable
                   FROM subtitle_replacement_diffs WHERE plan_id = ?1",
                [&made],
            )?;
            c.execute(
                "UPDATE subtitle_job_plan SET stored_id = ?2 WHERE job_id = ?1",
                params![id, theirs],
            )?;
            c.execute(
                "UPDATE subtitle_stored SET cleaned_at = 1 WHERE id = ?1",
                [own],
            )
            .map_err(trss_core::DbError::from)
        }
    })
    .await
    .unwrap();
    let before = s.stored(&theirs).await;

    // The approved plan does not hold for the row any more: the row gets a
    // stored subtitle of its own link, and the person compares again.
    assert_eq!(
        s.decide(&id, true).await,
        Decided::Done(PlanState::Approved)
    );
    s.run().await;
    assert_eq!(s.detail(&id).await.row.wait, Some(Wait::Approval));
    let row = s.plan(&id).await.remove(0);
    let relinked = row.stored_id.clone().unwrap();
    assert!(relinked != theirs && relinked != own, "{relinked}");
    let made = s.stored(&relinked).await;
    assert_eq!(
        (made.0, made.1.as_str(), made.2.as_deref(), &made.3, &made.4),
        (2, "mapped", Some("attachment"), &before.3, &id)
    );
    // The pick's stays as it was, and the new one is as old: no newer
    // revision of the source.
    assert_eq!(s.stored(&theirs).await, before);
    let (made_at, theirs_at): (i64, i64) =
        s.db.run({
            let (relinked, theirs) = (relinked.clone(), theirs.clone());
            move |c| {
                c.query_row(
                    "SELECT (SELECT stored_at FROM subtitle_stored WHERE id = ?1),
                            (SELECT stored_at FROM subtitle_stored WHERE id = ?2)",
                    params![relinked, theirs],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(trss_core::DbError::from)
            }
        })
        .await
        .unwrap();
    assert_eq!(made_at, theirs_at);

    // Approved again, it is applied from the row's own.
    assert_eq!(
        s.decide(&id, true).await,
        Decided::Done(PlanState::Approved)
    );
    s.run().await;
    let d = s.detail(&id).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(
        std::fs::read(s.work().join("Season 02/Show S02E02.ass")).unwrap(),
        fake::ass("Show-02")
    );
    let applied_from: String =
        s.db.run(|c| {
            c.query_row(
                "SELECT stored_id FROM subtitle_applied WHERE removed_at IS NULL",
                [],
                |r| r.get(0),
            )
            .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    assert_eq!(applied_from, relinked);
}

/// A pick of the creator's post with `Show-02.ass` (the bytes of
/// `/ok/Show-02`) beside another file of its format for Anissia's episode 2:
/// both are stored and asked about. The pick's job, and the row of
/// `Show-02.ass`.
async fn asked_beside(s: &Setup, command: &str) -> (String, PlanRow) {
    let id = pick(s, command, "2", "/pack/Show-02.ass/Show-02%20[sign].ass").await;
    s.run().await;
    assert_eq!(s.detail(&id).await.row.wait, Some(Wait::Placement));
    let row = s
        .plan(&id)
        .await
        .into_iter()
        .find(|r| r.name == "Show-02.ass")
        .unwrap();
    assert!(row.question.is_some(), "{row:?}");
    (id, row)
}

/// Confirms [`asked_beside`]'s table: `Show-02.ass` applied on episode 3,
/// the other stored only.
async fn placed_on_three(s: &Setup, id: &str) {
    assert_eq!(
        s.confirm(
            id,
            &[
                ("Show-02.ass", Some(3), true),
                ("Show-02 [sign].ass", None, false)
            ]
        )
        .await,
        Confirmed::Queued {
            applied: 1,
            stored: 1
        }
    );
}

/// When the stored subtitles `a` and `b` were stored.
async fn stored_at(s: &Setup, a: &str, b: &str) -> (i64, i64) {
    let (a, b) = (a.to_owned(), b.to_owned());
    s.db.run(move |c| {
        c.query_row(
            "SELECT (SELECT stored_at FROM subtitle_stored WHERE id = ?1),
                    (SELECT stored_at FROM subtitle_stored WHERE id = ?2)",
            params![a, b],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap()
}

/// The applied copies not removed: (episode, stored subtitle).
async fn copies(s: &Setup) -> Vec<(i64, String)> {
    s.db.run(|c| {
        let mut stmt = c.prepare(
            "SELECT episode, stored_id FROM subtitle_applied
              WHERE removed_at IS NULL ORDER BY episode",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_row_placed_elsewhere_leaves_the_stored_subtitle_other_rows_use_on_its_link() {
    let s = setup().await;
    let (_, picked) = picked_and_kept(&s).await;
    let shared = picked.stored_id.clone().unwrap();
    let before = s.stored(&shared).await;
    // A second pick of the same post waits to replace [`MINE`] from the same
    // stored subtitle, and a third one's row is on it too.
    let again = pick(&s, "c2", "2", "/ok/Show-02").await;
    s.run().await;
    assert_eq!(s.detail(&again).await.row.wait, Some(Wait::Approval));
    assert_eq!(s.plan(&again).await[0].stored_id, Some(shared.clone()));
    let (asked, row) = asked_beside(&s, "c3").await;
    assert_eq!(row.stored_id, Some(shared.clone()));
    let sign = s
        .plan(&asked)
        .await
        .into_iter()
        .find(|r| r.name == "Show-02 [sign].ass")
        .unwrap()
        .stored_id
        .unwrap();

    // The person puts it on episode 3: the row takes a stored subtitle of
    // that link, as old as the one it had, and the one the other rows use
    // stays on episode 2. The other file's stored subtitle is its row's
    // alone, and goes on no episode with it.
    placed_on_three(&s, &asked).await;
    assert_eq!(s.stored(&shared).await, before);
    assert_eq!(
        s.plan(&picked.job_id).await[0].stored_id,
        Some(shared.clone())
    );
    let plan = s.plan(&asked).await;
    let own = plan[0].stored_id.clone().unwrap();
    assert_eq!(plan[0].name, "Show-02.ass");
    assert_ne!(own, shared);
    let made = s.stored(&own).await;
    assert_eq!(
        (made.0, made.1.as_str(), made.2.as_deref(), &made.3, &made.4),
        (3, "explicit", None, &before.3, &asked)
    );
    let (made_at, shared_at) = stored_at(&s, &own, &shared).await;
    assert_eq!(made_at, shared_at);
    assert_eq!(plan[1].stored_id.as_deref(), Some(sign.as_str()));
    let unplaced: (Option<i64>, Option<String>) =
        s.db.run(move |c| {
            c.query_row(
                "SELECT episode, assignment FROM subtitle_stored WHERE id = ?1",
                [sign],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    assert_eq!(unplaced, (None, None));

    // Each copy is applied from the stored subtitle of its own link, and the
    // second pick's approval holds: no second look.
    s.run().await;
    assert_eq!(s.detail(&asked).await.row.state, JobState::Done);
    assert_eq!(
        s.decide(&again, true).await,
        Decided::Done(PlanState::Approved)
    );
    s.run().await;
    let d = s.detail(&again).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    for n in [2, 3] {
        assert_eq!(
            std::fs::read(s.work().join(format!("Season 02/Show S02E{n:02}.ass"))).unwrap(),
            fake::ass("Show-02"),
            "{n}"
        );
    }
    assert_eq!(s.plan(&again).await[0].stored_id, Some(shared.clone()));
    assert_eq!(copies(&s).await, [(2, shared), (3, own)]);
    let plans: i64 =
        s.db.run(move |c| {
            c.query_row(
                "SELECT count(*) FROM subtitle_replacements WHERE job_id = ?1",
                [again],
                |r| r.get(0),
            )
            .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    assert_eq!(plans, 1, "the approved plan held");
}

#[tokio::test]
async fn an_applied_copy_alone_keeps_the_stored_subtitle_it_was_applied_from_on_its_link() {
    let s = setup().await;
    s.sql(
        "INSERT INTO subtitle_episode_mappings
             (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
         VALUES ('w1', 2, 'src', 'user', 0, '시험', 0);",
    )
    .await;
    let first = pick(&s, "c1", "2", "/ok/Show-02").await;
    s.run().await;
    assert_eq!(s.detail(&first).await.row.state, JobState::Done);
    let applied = s.plan(&first).await[0].stored_id.clone().unwrap();
    let before = s.stored(&applied).await;
    let (asked, row) = asked_beside(&s, "c2").await;
    assert_eq!(row.stored_id, Some(applied.clone()));
    // The first pick's row on a stored subtitle of its own, as one a later
    // run relinked: the copy it applied alone names the one it had.
    s.db.run({
        let (first, applied) = (first.clone(), applied.clone());
        move |c| {
            c.execute_batch(&format!(
                "CREATE TEMP TABLE own AS SELECT * FROM subtitle_stored WHERE id = '{applied}';
                 UPDATE own SET id = 'own';
                 INSERT INTO subtitle_stored SELECT * FROM own;
                 UPDATE subtitle_job_plan SET stored_id = 'own' WHERE job_id = '{first}';"
            ))
            .map_err(trss_core::DbError::from)
        }
    })
    .await
    .unwrap();

    placed_on_three(&s, &asked).await;
    assert_eq!(s.stored(&applied).await, before);
    let moved = s.plan(&asked).await[0].stored_id.clone().unwrap();
    assert!(moved != applied && moved != "own", "{moved}");
    let made = s.stored(&moved).await;
    assert_eq!(
        (made.0, made.1.as_str(), made.2.as_deref()),
        (3, "explicit", None)
    );
    s.run().await;
    assert_eq!(copies(&s).await, [(2, applied), (3, moved)]);
}

#[tokio::test]
async fn rows_of_one_stored_subtitle_placed_together_move_it_as_one() {
    let s = setup().await;
    s.sql(
        "INSERT INTO subtitle_episode_mappings
             (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
         VALUES ('w1', 2, 'src', 'user', 0, '시험', 0);",
    )
    .await;
    // The same file twice, beside another of its format: all are asked
    // about, and the two of the same bytes are on one stored subtitle.
    let id = pick(
        &s,
        "c1",
        "2",
        "/pack/Show-02.ass/b%2FShow-02.ass/Show-02%20[sign].ass",
    )
    .await;
    s.run().await;
    assert_eq!(s.detail(&id).await.row.wait, Some(Wait::Placement));
    let plan = s.plan(&id).await;
    let same: Vec<&PlanRow> = plan
        .iter()
        .filter(|r| r.name.ends_with("Show-02.ass"))
        .collect();
    assert_eq!(same.len(), 2, "{plan:?}");
    assert!(same.iter().all(|r| r.question.is_some()), "{plan:?}");
    let stored = same[0].stored_id.clone().unwrap();
    assert_eq!(same[1].stored_id.as_deref(), Some(stored.as_str()));

    // Both go on episode 3, the other file on none.
    let placings = plan
        .iter()
        .map(|r| RowPlacing {
            position: r.position,
            episode: (r.name.ends_with("Show-02.ass")).then_some(3),
            apply: r.name.ends_with("Show-02.ass"),
        })
        .collect();
    assert!(matches!(
        s.store
            .place
            .confirm_placement(&id, placings, Vec::new(), Some(12), 50_000)
            .await
            .unwrap(),
        Confirmed::Queued { .. }
    ));
    // The stored subtitle moves with them: one of that link, not two.
    let plan = s.plan(&id).await;
    for row in plan.iter().filter(|r| r.name.ends_with("Show-02.ass")) {
        assert_eq!(row.stored_id.as_deref(), Some(stored.as_str()), "{row:?}");
    }
    let moved = s.stored(&stored).await;
    assert_eq!((moved.0, moved.1.as_str(), moved.2), (3, "explicit", None));
    let asset = moved.3;
    let records: i64 =
        s.db.run(move |c| {
            c.query_row(
                "SELECT count(*) FROM subtitle_stored WHERE subtitle_asset_id = ?1",
                [asset],
                |r| r.get(0),
            )
            .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    assert_eq!(records, 1);
}
