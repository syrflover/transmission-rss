//! Cleaning stored files (ticket 0073, `trss_jobs::place::cleanup`): what a
//! person may clean and what goes with it, the worker's removal, the race
//! with a job that takes a file up meanwhile, and the restarts a killed
//! worker leaves.

use crate::Handles;
use std::{
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
    model::{AssetKind, Chosen, Outcome},
    place::{
        cleanup::{self, Asked, CleanKind, Cleanable},
        records::StoredChoice,
    },
    Created, JobState, NewItem, NewJob, ReceiveArea, Runner, Wait,
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
    store: Handles,
    runner: Runner,
}

impl Setup {
    fn work(&self) -> PathBuf {
        self.dir.path().join("shows/Show")
    }

    /// The creator's folder of stored subtitles and fonts.
    fn stored_dir(&self) -> PathBuf {
        self.work().join(".trss/subtitles").join(CREATOR)
    }

    /// The creator's folder of attachments in the app data folder.
    fn files_dir(&self) -> PathBuf {
        self.dir
            .path()
            .join("subtitle-files")
            .join(WORK)
            .join(CREATOR)
    }

    async fn run(&self) {
        self.runner
            .run_ready(&CancellationToken::new())
            .await
            .unwrap();
    }

    async fn clean_all(&self) -> usize {
        self.runner.run_cleanups().await.unwrap()
    }

    async fn cleanable(&self) -> Vec<Cleanable> {
        self.store.place.work_files(WORK).await.unwrap().cleanable
    }

    /// The cleanable entry of the stored file `name`.
    async fn entry(&self, name: &str) -> Cleanable {
        self.cleanable()
            .await
            .into_iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("{name} is not cleanable"))
    }

    /// Confirms the cleanup of `name` with the files its entry shows.
    async fn ask(&self, name: &str) -> String {
        let entry = self.entry(name).await;
        let assets = entry.with.iter().map(|f| f.id.clone()).collect();
        match self
            .store
            .place
            .clean_stored(WORK, &entry.id, assets, 60_000)
            .await
            .unwrap()
        {
            Asked::Asked(id) => id,
            other => panic!("{other:?}"),
        }
    }

    async fn sql(&self, sql: String) {
        self.db
            .run(move |c| c.execute_batch(&sql).map_err(trss_core::DbError::from))
            .await
            .unwrap();
    }

    async fn one<T: rusqlite::types::FromSql + Send + 'static>(&self, sql: String) -> T {
        self.db
            .run(move |c| {
                c.query_row(&sql, [], |r| r.get(0))
                    .map_err(trss_core::DbError::from)
            })
            .await
            .unwrap()
    }

    /// The cleanup's state and reason.
    async fn cleanup(&self, id: &str) -> (String, Option<String>) {
        let id = id.to_owned();
        self.db
            .run(move |c| {
                c.query_row(
                    "SELECT state, reason FROM subtitle_cleanups WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(trss_core::DbError::from)
            })
            .await
            .unwrap()
    }

    /// Each file the cleanup named, by name: its state and reason.
    async fn removals(&self, id: &str) -> Vec<(String, String, Option<String>)> {
        let id = id.to_owned();
        self.db
            .run(move |c| {
                let mut stmt = c.prepare(
                    "SELECT a.relative_path, m.state, m.reason
                       FROM subtitle_asset_removals m JOIN subtitle_assets a ON a.id = m.asset_id
                      WHERE m.cleanup_id = ?1 ORDER BY a.relative_path",
                )?;
                let rows = stmt.query_map([id], |r| {
                    let path: String = r.get(0)?;
                    Ok((
                        path.rsplit('/').next().unwrap().to_owned(),
                        r.get(1)?,
                        r.get(2)?,
                    ))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(trss_core::DbError::from)
            })
            .await
            .unwrap()
    }

    /// Whether the asset at `name` (the file's name) is recorded removed.
    async fn removed(&self, name: &str) -> bool {
        self.one(format!(
            "SELECT removed_at IS NOT NULL FROM subtitle_assets
              WHERE relative_path LIKE '%/{name}' ORDER BY created_at LIMIT 1"
        ))
        .await
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
    let store = Handles::new(db.clone());
    let area = ReceiveArea::in_app_data(dir.path());
    let runner = Runner::new(
        store.run.clone(),
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

/// A candidate's job of Anissia's episode `episode` whose post offers
/// `names` (the fake source's `/pack/`).
async fn make_pack(s: &Setup, command: &str, episode: &str, names: &[&str]) -> String {
    make_pack_by(s, CREATOR, command, episode, names).await
}

/// [`make_pack`] by the creator `creator`.
async fn make_pack_by(
    s: &Setup,
    creator: &str,
    command: &str,
    episode: &str,
    names: &[&str],
) -> String {
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
            episode: episode.to_owned(),
            post_url: format!("https://{}/pack/{}", fake::HOST, names.join("/")),
            found_at: 500,
        }],
    };
    match s.store.requests.create(job, 900).await.unwrap() {
        Created::Created(id) => id,
        other => panic!("created: {other:?}"),
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
        )?;
        c.execute(
            "UPDATE library_generation SET generation = generation + 1 WHERE id = 1",
            [],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();
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

fn with_names(entry: &Cleanable) -> Vec<(&str, AssetKind)> {
    entry
        .with
        .iter()
        .map(|f| (f.name.as_str(), f.kind))
        .collect()
}

fn kept_names(entry: &Cleanable) -> Vec<(&str, &str)> {
    entry
        .kept
        .iter()
        .map(|f| (f.name.as_str(), f.reason))
        .collect()
}

/// Episode 7 has no video: a job whose post offers its subtitle, a font and
/// a readme waits for the video.
async fn waiting_alone(s: &Setup) -> String {
    let id = make_pack(s, "c7", "7", &["Show - 07.ass", "Solo.ttf", "readme.txt"]).await;
    s.run().await;
    let d = s.store.views.detail(&id).await.unwrap().unwrap();
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Video)),
        "{:?}",
        d.row.note
    );
    id
}

/// Episode 2's subtitle applied with `font` and a readme by a first job,
/// then replaced by the creator's revision (no font), which a person
/// approved: the first is a past revision (지난 수정본), the one stored
/// subtitle to use `font` and the readme. The first job.
async fn past_alone(s: &Setup, font: &str) -> String {
    let first = make_pack(s, "c1", "2", &["Show - 02.ass", font, "readme.txt"]).await;
    s.run().await;
    let second = make_pack(s, "c2", "2", &["Show - 02 [v2].ass"]).await;
    s.run().await;
    let d = s.store.views.detail(&second).await.unwrap().unwrap();
    assert_eq!(d.row.wait, Some(Wait::Approval), "{:?}", d.row.note);
    let plan = s
        .store
        .place
        .replacements(&second)
        .await
        .unwrap()
        .remove(0)
        .plan;
    s.store
        .place
        .decide_replacement(&second, &plan.id, plan.version, true, 50_000)
        .await
        .unwrap();
    s.run().await;
    for id in [&first, &second] {
        let d = s.store.views.detail(id).await.unwrap().unwrap();
        assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    }
    assert_eq!(
        std::fs::read(s.work().join("Season 01/Show S01E02.ass")).unwrap(),
        fake::bytes_of("Show - 02 [v2].ass")
    );
    first
}

// 완료 기준: 같은 폰트를 쓰는 두 수정본 가운데 하나를 정리하면 그 자막만
// 지워지고 폰트는 남아, 다른 보관본을 다시 적용할 수 있어요.
#[tokio::test]
async fn cleaning_one_of_two_revisions_sharing_a_font_keeps_the_font_for_the_other() {
    let s = setup().await;
    let first = make_pack(
        &s,
        "c1",
        "2",
        &["Show - 02.ass", "Show - 05.ass", "Shared.ttf"],
    )
    .await;
    s.run().await;
    let second = make_pack(
        &s,
        "c2",
        "2",
        &["Show - 02.ass", "Show - 05 [v2].ass", "Shared.ttf"],
    )
    .await;
    s.run().await;
    for id in [&first, &second] {
        let d = s.store.views.detail(id).await.unwrap().unwrap();
        assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    }
    assert_eq!(
        names(&s.stored_dir()),
        [
            "Shared.ttf",
            "Show - 02.ass",
            "Show - 05 [v2].ass",
            "Show - 05.ass"
        ]
    );

    // Episode 2's subtitle is beside its video: not cleanable.
    let all = s.cleanable().await;
    let listed: Vec<(&str, CleanKind)> = all.iter().map(|e| (e.name.as_str(), e.kind)).collect();
    assert_eq!(
        listed,
        [
            ("Show - 05.ass", CleanKind::Stored),
            ("Show - 05 [v2].ass", CleanKind::Stored)
        ]
    );
    let old = s.entry("Show - 05.ass").await;
    assert_eq!(old.blocked, None);
    assert_eq!(with_names(&old), [("Show - 05.ass", AssetKind::Subtitle)]);
    assert_eq!(kept_names(&old), [("Shared.ttf", cleanup::OTHER_SUBTITLE)]);

    let cleanup = s.ask("Show - 05.ass").await;
    // From the confirmation it is no stored copy: in no list.
    assert!(s
        .cleanable()
        .await
        .iter()
        .all(|e| e.name != "Show - 05.ass"));
    assert_eq!(s.clean_all().await, 1);

    assert_eq!(s.cleanup(&cleanup).await, ("done".to_owned(), None));
    assert_eq!(
        names(&s.stored_dir()),
        ["Shared.ttf", "Show - 02.ass", "Show - 05 [v2].ass"]
    );
    assert!(s.removed("Show - 05.ass").await);
    assert!(!s.removed("Shared.ttf").await);

    // The other revision is applied beside episode 5's video, its font there.
    video(&s, "05").await;
    let other = s.entry("Show - 05 [v2].ass").await;
    let choice = s
        .store
        .place
        .choose_stored(WORK, &other.id, Chosen::Apply, 70_000)
        .await
        .unwrap();
    assert_eq!(
        choice,
        StoredChoice::Queued {
            job_id: second.clone(),
            compare: false
        }
    );
    s.run().await;
    let d = s.store.views.detail(&second).await.unwrap().unwrap();
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(
        std::fs::read(s.work().join("Season 01/Show S01E05.ass")).unwrap(),
        fake::bytes_of("Show - 05 [v2].ass")
    );
    assert_eq!(
        std::fs::read(s.stored_dir().join("Shared.ttf")).unwrap(),
        fake::bytes_of("Shared.ttf")
    );
}

// 완료 기준: 폰트를 혼자 쓰던 수정본을 정리하면 자막과 그 폰트, 그 수정본만
// 쓰던 첨부가 함께 지워져요.
#[tokio::test]
async fn cleaning_a_revision_alone_with_its_font_and_attachment_removes_all_three() {
    let s = setup().await;
    past_alone(&s, "Solo.ttf").await;
    assert_eq!(names(&s.files_dir()), ["readme.txt"]);

    let entry = s.entry("Show - 02.ass").await;
    assert_eq!(entry.kind, CleanKind::Past);
    assert_eq!(entry.episode, Some(2));
    assert_eq!(
        with_names(&entry),
        [
            ("Show - 02.ass", AssetKind::Subtitle),
            ("Solo.ttf", AssetKind::Font),
            ("readme.txt", AssetKind::Attachment)
        ]
    );
    assert!(entry.kept.is_empty());
    let size =
        |names: &[&str]| -> u64 { names.iter().map(|n| fake::bytes_of(n).len() as u64).sum() };
    let total = s.store.place.work_files(WORK).await.unwrap().total;
    assert_eq!(
        total,
        size(&[
            "Show - 02.ass",
            "Solo.ttf",
            "readme.txt",
            "Show - 02 [v2].ass"
        ])
    );

    let cleanup = s.ask("Show - 02.ass").await;
    s.clean_all().await;

    assert_eq!(s.cleanup(&cleanup).await, ("done".to_owned(), None));
    assert_eq!(names(&s.stored_dir()), ["Show - 02 [v2].ass"]);
    // The attachment's folders left empty went (the creator's, then the
    // work's), up to the app data folder's `subtitle-files`.
    let work_files = s.files_dir().parent().unwrap().to_path_buf();
    assert!(!s.files_dir().exists());
    assert!(!work_files.exists());
    assert!(s.dir.path().join("subtitle-files").is_dir());
    for name in ["Show - 02.ass", "Solo.ttf", "readme.txt"] {
        assert!(s.removed(name).await, "{name}");
    }
    assert_eq!(
        s.removals(&cleanup).await,
        [
            ("Show - 02.ass".to_owned(), "done".to_owned(), None),
            ("Solo.ttf".to_owned(), "done".to_owned(), None),
            ("readme.txt".to_owned(), "done".to_owned(), None),
        ]
    );
    assert_eq!(
        s.store.place.work_files(WORK).await.unwrap().total,
        size(&["Show - 02 [v2].ass"])
    );
    let storage = s.store.place.storage().await.unwrap();
    assert_eq!(storage.len(), 1);
    assert_eq!(storage[0].total, size(&["Show - 02 [v2].ass"]));
    assert_eq!(storage[0].cleanable, 0);
}

// 완료 기준: 진행 중이거나 보류한 작업이 참조하는 보관본은 정리할 수 없고
// 까닭이 보여요.
#[tokio::test]
async fn a_stored_subtitle_a_running_or_held_job_uses_is_not_cleanable() {
    let s = setup().await;
    let id = make_pack(&s, "c1", "2", &["Show - 02.ass", "Show - 05.ass"]).await;
    s.run().await;
    let entry = s.entry("Show - 05.ass").await;
    assert_eq!(entry.blocked, None);
    let assets: Vec<String> = entry.with.iter().map(|f| f.id.clone()).collect();

    for (state, reason) in [
        ("pending", cleanup::RUNNING_JOB),
        ("running", cleanup::RUNNING_JOB),
        ("held", cleanup::HELD_JOB),
    ] {
        s.sql(format!(
            "UPDATE subtitle_jobs SET state = '{state}' WHERE id = '{id}'"
        ))
        .await;
        assert_eq!(s.entry("Show - 05.ass").await.blocked, Some(reason));
        let asked = s
            .store
            .place
            .clean_stored(WORK, &entry.id, assets.clone(), 60_000)
            .await
            .unwrap();
        assert_eq!(asked, Asked::Refused(reason), "{state}");
    }
    // A job that waits with the row not settled.
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'waiting', wait = 'placement' WHERE id = '{id}';
         UPDATE subtitle_job_plan SET outcome = NULL WHERE job_id = '{id}'
            AND name = 'Show - 05.ass';"
    ))
    .await;
    assert_eq!(
        s.entry("Show - 05.ass").await.blocked,
        Some(cleanup::WAITING_JOB)
    );
    // Nothing was cleaned.
    assert_eq!(
        s.one::<i64>("SELECT count(*) FROM subtitle_cleanups".to_owned())
            .await,
        0
    );
    assert!(s.stored_dir().join("Show - 05.ass").exists());

    // Files that are not the ones the dialog showed are refused with the
    // entry as it is now.
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'done', wait = NULL WHERE id = '{id}';
         UPDATE subtitle_job_plan SET outcome = 'stored' WHERE job_id = '{id}'
            AND name = 'Show - 05.ass';"
    ))
    .await;
    let asked = s
        .store
        .place
        .clean_stored(WORK, &entry.id, Vec::new(), 60_000)
        .await
        .unwrap();
    assert!(matches!(asked, Asked::Changed(now) if now.id == entry.id));
    // An applied one is refused, and an unknown one is not found.
    let applied = s
        .one::<String>("SELECT stored_id FROM subtitle_applied WHERE removed_at IS NULL".to_owned())
        .await;
    assert_eq!(
        s.store
            .place
            .clean_stored(WORK, &applied, Vec::new(), 60_000)
            .await
            .unwrap(),
        Asked::Refused(cleanup::APPLIED)
    );
    assert_eq!(
        s.store
            .place
            .clean_stored(WORK, "nope", Vec::new(), 60_000)
            .await
            .unwrap(),
        Asked::NotFound
    );
}

// 완료 기준: 정리를 확인하는 순간 새 작업이 그 폰트를 참조하면 폰트가 남아요.
#[tokio::test]
async fn a_font_a_new_job_takes_up_after_the_confirmation_stays() {
    let s = setup().await;
    past_alone(&s, "Race.ttf").await;
    let entry = s.entry("Show - 02.ass").await;
    assert_eq!(
        with_names(&entry),
        [
            ("Show - 02.ass", AssetKind::Subtitle),
            ("Race.ttf", AssetKind::Font),
            ("readme.txt", AssetKind::Attachment)
        ]
    );
    let cleanup = s.ask("Show - 02.ass").await;

    // Before the worker comes to the cleanup, a job keeps the same font,
    // reusing the file still there, and links its subtitle to it.
    video(&s, "05").await;
    let next = make_pack(&s, "c5", "5", &["Show - 05.ass", "Race.ttf"]).await;
    s.run().await;
    let d = s.store.views.detail(&next).await.unwrap().unwrap();
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(
        s.one::<i64>(
            "SELECT count(*) FROM subtitle_assets WHERE relative_path LIKE '%/Race.ttf'".to_owned()
        )
        .await,
        1,
        "the font was reused"
    );

    s.clean_all().await;

    assert_eq!(s.cleanup(&cleanup).await, ("done".to_owned(), None));
    assert_eq!(
        s.removals(&cleanup).await,
        [
            (
                "Race.ttf".to_owned(),
                "kept".to_owned(),
                Some(cleanup::OTHER_SUBTITLE.to_owned())
            ),
            ("Show - 02.ass".to_owned(), "done".to_owned(), None),
            ("readme.txt".to_owned(), "done".to_owned(), None),
        ]
    );
    assert_eq!(
        names(&s.stored_dir()),
        ["Race.ttf", "Show - 02 [v2].ass", "Show - 05.ass"]
    );
    assert!(!s.removed("Race.ttf").await);
    assert_eq!(
        std::fs::read(s.stored_dir().join("Race.ttf")).unwrap(),
        fake::bytes_of("Race.ttf")
    );
}

// A job cut between storing its files and linking them links them when it
// runs again: a font it kept stays until then, though the cleaned copy was
// the one subtitle linked to it.
#[tokio::test]
async fn a_font_a_job_cut_before_its_links_kept_stays() {
    let s = setup().await;
    past_alone(&s, "Cut.ttf").await;
    let cleanup = s.ask("Show - 02.ass").await;
    video(&s, "05").await;
    let next = make_pack(&s, "c5", "5", &["Show - 05.ass", "Cut.ttf"]).await;
    s.run().await;
    // The run is cut after the store (`stored()`), before the link
    // (`link()`): the subtitle is stored with its links unknown, the font's
    // row has the asset, and the job goes on at the worker's next start.
    s.sql(format!(
        "DELETE FROM subtitle_stored_assets
          WHERE stored_id IN (SELECT stored_id FROM subtitle_job_plan WHERE job_id = '{next}');
         UPDATE subtitle_stored SET links_known = 0
          WHERE id IN (SELECT stored_id FROM subtitle_job_plan WHERE job_id = '{next}');
         UPDATE subtitle_jobs SET state = 'pending' WHERE id = '{next}';"
    ))
    .await;
    assert_eq!(
        s.one::<i64>(format!(
            "SELECT count(*) FROM subtitle_job_plan p JOIN subtitle_assets a ON a.id = p.asset_id
              WHERE p.job_id = '{next}' AND a.relative_path LIKE '%/Cut.ttf'"
        ))
        .await,
        1,
        "the font's row has the asset the cleaned copy linked"
    );

    s.clean_all().await;

    assert_eq!(s.cleanup(&cleanup).await, ("done".to_owned(), None));
    assert_eq!(
        s.removals(&cleanup).await,
        [
            (
                "Cut.ttf".to_owned(),
                "kept".to_owned(),
                Some(cleanup::JOB_FILE.to_owned())
            ),
            ("Show - 02.ass".to_owned(), "done".to_owned(), None),
            ("readme.txt".to_owned(), "done".to_owned(), None),
        ]
    );
    assert!(!s.removed("Cut.ttf").await);

    // The job's next run links its subtitle to the font, which is there.
    s.run().await;
    let d = s.store.views.detail(&next).await.unwrap().unwrap();
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!(
        s.one::<i64>(format!(
            "SELECT count(*) FROM subtitle_stored_assets l
               JOIN subtitle_assets a ON a.id = l.asset_id
               JOIN subtitle_job_plan p ON p.stored_id = l.stored_id
              WHERE p.job_id = '{next}' AND a.relative_path LIKE '%/Cut.ttf'
                AND a.removed_at IS NULL"
        ))
        .await,
        1
    );
    assert_eq!(
        std::fs::read(s.stored_dir().join("Cut.ttf")).unwrap(),
        fake::bytes_of("Cut.ttf")
    );
}

// 완료 기준: `.trss/` 안의 등록되지 않은 파일은 정리할 것 목록에 없고 지우지
// 않아요.
#[tokio::test]
async fn a_file_the_app_did_not_record_is_listed_nowhere_and_stays() {
    let s = setup().await;
    past_alone(&s, "Solo.ttf").await;
    std::fs::write(s.stored_dir().join("stray.ass"), b"not the app's").unwrap();
    std::fs::write(s.stored_dir().join("Stray.ttf"), b"not the app's").unwrap();
    std::fs::write(s.files_dir().join("stray.txt"), b"not the app's").unwrap();

    let all = s.cleanable().await;
    let shown: Vec<&str> = all
        .iter()
        .flat_map(|e| {
            std::iter::once(e.name.as_str())
                .chain(e.with.iter().map(|f| f.name.as_str()))
                .chain(e.kept.iter().map(|f| f.name.as_str()))
        })
        .collect();
    assert_eq!(
        shown,
        ["Show - 02.ass", "Show - 02.ass", "Solo.ttf", "readme.txt"]
    );

    let cleanup = s.ask("Show - 02.ass").await;
    s.clean_all().await;
    assert_eq!(s.cleanup(&cleanup).await, ("done".to_owned(), None));
    assert_eq!(
        names(&s.stored_dir()),
        ["Show - 02 [v2].ass", "Stray.ttf", "stray.ass"]
    );
    assert_eq!(names(&s.files_dir()), ["stray.txt"]);
    // A folder with a file the app did not record stays.
    assert!(s.files_dir().is_dir());
}

/// A cleanup of episode 2's past revision, its font and readme, beside the
/// revision applied in its place, which nothing touches.
async fn restart_case() -> (Setup, String) {
    let s = setup().await;
    past_alone(&s, "Solo.ttf").await;
    let cleanup = s.ask("Show - 02.ass").await;
    (s, cleanup)
}

/// What a finished cleanup of [`restart_case`] leaves.
async fn after_restart(s: &Setup, cleanup: &str) {
    assert_eq!(s.cleanup(cleanup).await, ("done".to_owned(), None));
    assert_eq!(names(&s.stored_dir()), ["Show - 02 [v2].ass"]);
    assert_eq!(names(&s.files_dir()), Vec::<String>::new());
    for name in ["Show - 02.ass", "Solo.ttf", "readme.txt"] {
        assert!(s.removed(name).await, "{name}");
    }
    assert!(!s.removed("Show - 02 [v2].ass").await);
    assert_eq!(
        std::fs::read(s.work().join("Season 01/Show S01E02.ass")).unwrap(),
        fake::bytes_of("Show - 02 [v2].ass")
    );
}

// 완료 기준: 정리 도중 worker를 죽였다 살려도 거짓 완료가 없고 다른 자산을
// 잃지 않아요.
#[tokio::test]
async fn a_worker_killed_after_the_confirmation_takes_the_cleanup_up() {
    let (s, cleanup) = restart_case().await;
    assert_eq!(s.cleanup(&cleanup).await.0, "asked");
    s.clean_all().await;
    after_restart(&s, &cleanup).await;
}

#[tokio::test]
async fn a_worker_killed_with_removals_intended_removes_them_on_its_next_pass() {
    let (s, cleanup) = restart_case().await;
    // Step 1 was written; the files are still there.
    let id = cleanup.clone();
    let pass =
        s.db.run(move |c| cleanup::intend(c, &id, 61_000))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(pass.removals.len(), 3);
    assert_eq!(s.cleanup(&cleanup).await.0, "asked", "no false done");
    assert!(!s.removed("Solo.ttf").await);
    assert!(s.stored_dir().join("Solo.ttf").exists());

    s.clean_all().await;
    after_restart(&s, &cleanup).await;
}

#[tokio::test]
async fn a_worker_killed_after_removing_a_file_before_recording_it_counts_it_removed() {
    let (s, cleanup) = restart_case().await;
    let id = cleanup.clone();
    s.db.run(move |c| cleanup::intend(c, &id, 61_000))
        .await
        .unwrap()
        .unwrap();
    // The font's file went; the worker died before step 3.
    std::fs::remove_file(s.stored_dir().join("Solo.ttf")).unwrap();
    assert_eq!(s.cleanup(&cleanup).await.0, "asked", "no false done");
    assert!(!s.removed("Solo.ttf").await, "not recorded before step 3");

    s.clean_all().await;
    after_restart(&s, &cleanup).await;
}

// A file whose bytes are not the recorded ones is not removed.
#[tokio::test]
async fn a_file_changed_on_disk_is_held_and_left_as_it_is() {
    let s = setup().await;
    past_alone(&s, "Solo.ttf").await;
    let cleanup = s.ask("Show - 02.ass").await;
    std::fs::write(s.stored_dir().join("Solo.ttf"), b"someone's own font").unwrap();

    s.clean_all().await;

    assert_eq!(
        s.cleanup(&cleanup).await,
        ("held".to_owned(), Some(cleanup::CHANGED.to_owned()))
    );
    assert_eq!(
        std::fs::read(s.stored_dir().join("Solo.ttf")).unwrap(),
        b"someone's own font"
    );
    assert!(!s.removed("Solo.ttf").await);
    // The others were removed; the cleanup shows as held.
    assert!(s.removed("Show - 02.ass").await);
    assert!(s.removed("readme.txt").await);
    let cleaning = s.store.place.work_files(WORK).await.unwrap().cleaning;
    assert_eq!(cleaning.len(), 1);
    assert_eq!(cleaning[0].state, "held");
    assert_eq!(cleaning[0].name, "Show - 02.ass");
}

#[tokio::test]
async fn a_work_folder_that_is_not_there_holds_the_cleanup() {
    let s = setup().await;
    past_alone(&s, "Solo.ttf").await;
    let cleanup = s.ask("Show - 02.ass").await;
    std::fs::rename(s.work(), s.dir.path().join("shows/Moved")).unwrap();

    s.clean_all().await;

    assert_eq!(
        s.cleanup(&cleanup).await,
        ("held".to_owned(), Some(cleanup::NO_FOLDER.to_owned()))
    );
    assert!(!s.removed("Show - 02.ass").await);
    // The attachment in the app data folder went.
    assert!(s.removed("readme.txt").await);
}

// A work folder that is not there now (a share not mounted): nothing of the
// work is cleaned, so no stored copy is hidden with its files left behind.
#[tokio::test]
async fn nothing_is_cleaned_while_the_work_folder_is_away() {
    let s = setup().await;
    past_alone(&s, "Solo.ttf").await;
    let entry = s.entry("Show - 02.ass").await;
    let moved = s.dir.path().join("shows/Unmounted");
    std::fs::rename(s.work(), &moved).unwrap();

    let away = s.entry("Show - 02.ass").await;
    assert_eq!(away.blocked, Some(cleanup::FOLDER_AWAY));
    let assets = entry.with.iter().map(|f| f.id.clone()).collect();
    let asked = s
        .store
        .place
        .clean_stored(WORK, &entry.id, assets, 60_000)
        .await
        .unwrap();
    assert_eq!(asked, Asked::Refused(cleanup::FOLDER_AWAY));
    assert_eq!(
        s.one::<i64>("SELECT count(*) FROM subtitle_cleanups".to_owned())
            .await,
        0
    );
    assert_eq!(
        s.one::<i64>(
            "SELECT count(*) FROM subtitle_stored WHERE cleaned_at IS NOT NULL".to_owned()
        )
        .await,
        0
    );
    assert_eq!(s.store.place.storage().await.unwrap()[0].cleanable, 0);

    // Back: cleanable again.
    std::fs::rename(&moved, s.work()).unwrap();
    assert_eq!(s.entry("Show - 02.ass").await.blocked, None);
    assert_eq!(s.store.place.storage().await.unwrap()[0].cleanable, 1);
}

// The same subtitle and font received in two posts are one stored subtitle
// (its second store reuses the first), with entries in both packages: the
// second package is no fonts-only one, and both files go.
#[tokio::test]
async fn a_subtitle_received_twice_is_cleaned_with_its_font() {
    let s = setup().await;
    let pack = ["Show - 07.ass", "Dup.ttf"];
    let first = make_pack(&s, "c1", "7", &pack).await;
    s.run().await;
    let second = make_pack(&s, "c2", "7", &pack).await;
    s.run().await;
    let count = |sql: &str| s.one::<i64>(sql.to_owned());
    assert_eq!(count("SELECT count(*) FROM subtitle_stored").await, 1);
    assert_eq!(count("SELECT count(*) FROM subtitle_packages").await, 2);
    assert_eq!(
        count("SELECT count(DISTINCT package_id) FROM subtitle_package_entries").await,
        2
    );

    let entry = s.entry("Show - 07.ass").await;
    assert_eq!(
        with_names(&entry),
        [
            ("Show - 07.ass", AssetKind::Subtitle),
            ("Dup.ttf", AssetKind::Font)
        ]
    );
    assert!(entry.kept.is_empty(), "{:?}", entry.kept);
    let cleanup = s.ask("Show - 07.ass").await;
    s.clean_all().await;

    assert_eq!(s.cleanup(&cleanup).await, ("done".to_owned(), None));
    for name in ["Show - 07.ass", "Dup.ttf"] {
        assert!(s.removed(name).await, "{name}");
    }
    assert!(!s.stored_dir().exists());
    // Both jobs waited for the video of the one copy: they settle.
    s.run().await;
    for id in [&first, &second] {
        let d = s.store.views.detail(id).await.unwrap().unwrap();
        assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    }
}

// A package of fonts alone (no subtitle among its files, as a fonts-only
// upload is) is kept on purpose: cleaning a subtitle that uses the same font
// file leaves it. Made here by a post with the font alone: the fake source's
// font bytes are not ones an upload's check takes.
#[tokio::test]
async fn a_font_of_a_fonts_only_package_stays() {
    let s = setup().await;
    make_pack(&s, "c1", "7", &["Show - 07.ass", "Dup.ttf"]).await;
    s.run().await;
    let fonts = make_pack(&s, "c3", "7", &["Dup.ttf"]).await;
    s.run().await;
    let d = s.store.views.detail(&fonts).await.unwrap().unwrap();
    assert_eq!(d.row.state, JobState::Partial, "{:?}", d.row.note);
    assert_eq!(
        s.one::<i64>(
            "SELECT count(*) FROM subtitle_assets WHERE relative_path LIKE '%/Dup.ttf'".to_owned()
        )
        .await,
        1,
        "the fonts-only package has the stored font"
    );
    assert_eq!(
        s.one::<i64>("SELECT count(*) FROM subtitle_packages".to_owned())
            .await,
        2
    );

    let entry = s.entry("Show - 07.ass").await;
    assert_eq!(with_names(&entry), [("Show - 07.ass", AssetKind::Subtitle)]);
    assert_eq!(kept_names(&entry), [("Dup.ttf", cleanup::FONTS_ONLY)]);
    let cleanup = s.ask("Show - 07.ass").await;
    s.clean_all().await;

    assert_eq!(s.cleanup(&cleanup).await, ("done".to_owned(), None));
    assert!(s.removed("Show - 07.ass").await);
    assert!(!s.removed("Dup.ttf").await);
    assert_eq!(names(&s.stored_dir()), ["Dup.ttf"]);
}

// 영상 대기 자막도 정리할 수 있고, 정리하면 그 작업은 영상을 더 기다리지
// 않으며 나중에 들어온 영상에도 적용하지 않아요.
#[tokio::test]
async fn a_subtitle_waiting_for_its_video_is_cleaned_and_its_job_settles() {
    let s = setup().await;
    let id = waiting_alone(&s).await;

    // Its font and readme, which only it uses, go with it: the job that
    // waits for the video made its links and links nothing more.
    let entry = s.entry("Show - 07.ass").await;
    assert_eq!(entry.kind, CleanKind::AwaitingVideo);
    assert_eq!(entry.blocked, None);
    assert_eq!(
        with_names(&entry),
        [
            ("Show - 07.ass", AssetKind::Subtitle),
            ("Solo.ttf", AssetKind::Font),
            ("readme.txt", AssetKind::Attachment)
        ]
    );
    assert!(entry.kept.is_empty(), "{:?}", entry.kept);
    let cleanup = s.ask("Show - 07.ass").await;
    let d = s.store.views.detail(&id).await.unwrap().unwrap();
    assert_eq!(d.row.state, JobState::Pending);
    assert_eq!(d.row.note.as_deref(), Some(cleanup::REFLECT));
    let plan = s.store.place.plan(&id).await.unwrap();
    let row = plan.iter().find(|r| r.name == "Show - 07.ass").unwrap();
    assert_eq!(row.outcome, Some(Outcome::Stored));
    assert_eq!(row.note.as_deref(), Some(cleanup::CLEANED_WHILE_WAITING));

    // The worker's turn: the cleanup, then the job, which has nothing left.
    // The job set back in line keeps no file: it has no subtitle to store
    // and its links are made.
    s.clean_all().await;
    assert_eq!(s.cleanup(&cleanup).await, ("done".to_owned(), None));
    for name in ["Show - 07.ass", "Solo.ttf", "readme.txt"] {
        assert!(s.removed(name).await, "{name}");
    }
    // The creator folders left empty went, in the work folder up to
    // `.trss/subtitles`, in the app data folder up to `subtitle-files`.
    assert!(!s.stored_dir().exists());
    assert!(s.work().join(".trss/subtitles").is_dir());
    assert!(!s.files_dir().exists());
    assert!(!s.files_dir().parent().unwrap().exists());
    assert!(s.dir.path().join("subtitle-files").is_dir());
    s.run().await;
    let d = s.store.views.detail(&id).await.unwrap().unwrap();
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Done, None),
        "{:?}",
        d.row.note
    );

    // A video that comes later takes nothing up.
    video(&s, "07").await;
    assert_eq!(s.runner.requeue_awaiting_video().await.unwrap(), 0);
    s.run().await;
    assert!(!s.work().join("Season 01/Show S01E07.ass").exists());
    assert_eq!(
        s.store.views.detail(&id).await.unwrap().unwrap().row.state,
        JobState::Done
    );
    // And no list shows it.
    assert!(s.store.place.stored_only(WORK).await.unwrap().is_empty());
    assert!(s.cleanable().await.is_empty());
}

#[tokio::test]
async fn a_removed_path_takes_a_new_file_of_the_same_name() {
    let s = setup().await;
    past_alone(&s, "Solo.ttf").await;
    s.ask("Show - 02.ass").await;
    s.clean_all().await;

    // The same post again: the files are stored anew under their names,
    // and the subtitle waits for a person to approve it over the one
    // applied.
    let id = make_pack(&s, "c6b", "2", &["Show - 02.ass", "Solo.ttf", "readme.txt"]).await;
    s.run().await;
    let d = s.store.views.detail(&id).await.unwrap().unwrap();
    assert_eq!(d.row.wait, Some(Wait::Approval), "{:?}", d.row.note);
    assert_eq!(
        names(&s.stored_dir()),
        ["Show - 02 [v2].ass", "Show - 02.ass", "Solo.ttf"]
    );
    assert_eq!(names(&s.files_dir()), ["readme.txt"]);
    let entry = s.entry("Show - 02.ass").await;
    assert_eq!(entry.blocked, Some(cleanup::AWAITING_APPROVAL));
    for name in ["Show - 02.ass", "Solo.ttf", "readme.txt"] {
        let rows =
            format!("SELECT count(*) FROM subtitle_assets WHERE relative_path LIKE '%/{name}'");
        assert_eq!(s.one::<i64>(rows.clone()).await, 2, "{name}");
        let live = format!("{rows} AND removed_at IS NULL");
        assert_eq!(s.one::<i64>(live).await, 1, "{name}");
    }
}

// A row of a cleaned copy put back in line by some path (here by hand: no
// path of the app does, see `place::cleanup`) ends stored with why, neither
// applied, compared for a replacement nor failed.
#[tokio::test]
async fn a_row_of_a_cleaned_copy_back_in_line_ends_stored_not_failed() {
    let s = setup().await;
    let first = past_alone(&s, "Solo.ttf").await;
    s.ask("Show - 02.ass").await;
    s.clean_all().await;
    s.sql(format!(
        "UPDATE subtitle_job_plan SET outcome = NULL, applied_id = NULL
          WHERE job_id = '{first}' AND name = 'Show - 02.ass';
         UPDATE subtitle_jobs SET state = 'pending', finished_at = NULL WHERE id = '{first}';"
    ))
    .await;

    s.run().await;

    let d = s.store.views.detail(&first).await.unwrap().unwrap();
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    let plan = s.store.place.plan(&first).await.unwrap();
    let row = plan.iter().find(|r| r.name == "Show - 02.ass").unwrap();
    assert_eq!(row.outcome, Some(Outcome::Stored));
    assert_eq!(row.note.as_deref(), Some(cleanup::NOT_APPLIED));
    assert!(s.store.place.replacements(&first).await.unwrap().is_empty());
    assert_eq!(
        std::fs::read(s.work().join("Season 01/Show S01E02.ass")).unwrap(),
        fake::bytes_of("Show - 02 [v2].ass")
    );
}

// A job that kept a font and still has an item to receive may link the font
// to what that item brings (a post's rows link the post's files): the font
// stays while it does.
#[tokio::test]
async fn a_font_of_a_job_with_an_item_not_received_yet_stays() {
    let s = setup().await;
    let id = waiting_alone(&s).await;
    s.sql(format!(
        "INSERT INTO subtitle_job_items (job_id, position, episode, post_url, found_at, state,
                                         wait, updated_at)
         SELECT job_id, 1, '8', post_url, 500, 'waiting', 'auth', 0
           FROM subtitle_job_items WHERE job_id = '{id}'"
    ))
    .await;

    let entry = s.entry("Show - 07.ass").await;
    assert_eq!(with_names(&entry), [("Show - 07.ass", AssetKind::Subtitle)]);
    assert_eq!(
        kept_names(&entry),
        [
            ("Solo.ttf", cleanup::JOB_FILE),
            ("readme.txt", cleanup::JOB_FILE)
        ]
    );
    let cleanup = s.ask("Show - 07.ass").await;
    s.clean_all().await;
    assert_eq!(s.cleanup(&cleanup).await, ("done".to_owned(), None));
    assert!(s.removed("Show - 07.ass").await);
    assert!(!s.removed("Solo.ttf").await);
    assert!(s.stored_dir().join("Solo.ttf").is_file());
}

// A link is never made to a file a cleanup removed, whatever the job's
// rows say.
#[tokio::test]
async fn a_job_links_no_removed_file() {
    let s = setup().await;
    video(&s, "05").await;
    let id = make_pack(&s, "c5", "5", &["Show - 05.ass", "Gone.ttf"]).await;
    s.run().await;
    // Cut before its links; meanwhile the font was removed.
    s.sql(format!(
        "DELETE FROM subtitle_stored_assets
          WHERE stored_id IN (SELECT stored_id FROM subtitle_job_plan WHERE job_id = '{id}');
         UPDATE subtitle_stored SET links_known = 0
          WHERE id IN (SELECT stored_id FROM subtitle_job_plan WHERE job_id = '{id}');
         UPDATE subtitle_assets SET removed_at = 70000 WHERE relative_path LIKE '%/Gone.ttf';
         UPDATE subtitle_jobs SET state = 'pending' WHERE id = '{id}';"
    ))
    .await;

    s.run().await;

    assert_eq!(
        s.one::<i64>(
            "SELECT count(*) FROM subtitle_stored_assets l
               JOIN subtitle_assets a ON a.id = l.asset_id
              WHERE a.removed_at IS NOT NULL"
                .to_owned()
        )
        .await,
        0
    );
    assert_eq!(
        s.one::<i64>("SELECT min(links_known) FROM subtitle_stored".to_owned())
            .await,
        1
    );
}

// An empty mount point where the work folder was: a folder, but without the
// app's `.trss/subtitles`. Its files are not taken as removed.
#[tokio::test]
async fn an_empty_mount_point_holds_the_cleanup_without_false_done() {
    let s = setup().await;
    past_alone(&s, "Solo.ttf").await;
    let cleanup = s.ask("Show - 02.ass").await;
    let share = s.dir.path().join("share");
    std::fs::rename(s.work(), &share).unwrap();
    std::fs::create_dir(s.work()).unwrap();

    s.clean_all().await;

    assert_eq!(
        s.cleanup(&cleanup).await,
        ("held".to_owned(), Some(cleanup::NO_FOLDER.to_owned()))
    );
    assert_eq!(
        s.removals(&cleanup).await,
        [
            (
                "Show - 02.ass".to_owned(),
                "held".to_owned(),
                Some(cleanup::NO_FOLDER.to_owned())
            ),
            (
                "Solo.ttf".to_owned(),
                "held".to_owned(),
                Some(cleanup::NO_FOLDER.to_owned())
            ),
            ("readme.txt".to_owned(), "done".to_owned(), None),
        ]
    );
    assert!(!s.removed("Show - 02.ass").await);
    assert!(!s.removed("Solo.ttf").await);
    let stored = share.join(".trss/subtitles").join(CREATOR);
    assert!(stored.join("Show - 02.ass").is_file());
    assert!(stored.join("Solo.ttf").is_file());
}

// A creator folder a cleanup emptied of the app's files, kept for a file
// the app did not record, is still the creator's folder: a creator named in
// another case stores into it, not into a second folder beside it.
#[tokio::test]
async fn a_creator_folder_a_cleanup_emptied_keeps_its_name() {
    let s = setup().await;
    make_pack_by(&s, "Hanu", "c1", "7", &["Show - 07.ass", "Hanu.ttf"]).await;
    s.run().await;
    let subtitles = s.work().join(".trss/subtitles");
    std::fs::write(subtitles.join("Hanu/notes.txt"), b"a person's").unwrap();
    let cleanup = s.ask("Show - 07.ass").await;
    s.clean_all().await;
    assert_eq!(s.cleanup(&cleanup).await, ("done".to_owned(), None));
    assert_eq!(names(&subtitles.join("Hanu")), ["notes.txt"]);

    make_pack_by(&s, "hanu", "c2", "8", &["Show - 08.ass"]).await;
    s.run().await;

    assert_eq!(names(&subtitles), ["Hanu"]);
    assert_eq!(
        names(&subtitles.join("Hanu")),
        ["Show - 08.ass", "notes.txt"]
    );
}

// A cleanup the database refuses a step of stays asked; the next one is
// carried out all the same.
#[tokio::test]
async fn a_cleanup_that_fails_does_not_stop_the_others() {
    let s = setup().await;
    past_alone(&s, "Solo.ttf").await;
    waiting_alone(&s).await;
    let first = s.ask("Show - 02.ass").await;
    let second = s.ask("Show - 07.ass").await;
    s.sql(format!(
        "CREATE TRIGGER refuse BEFORE UPDATE ON subtitle_cleanups WHEN OLD.id = '{first}'
         BEGIN SELECT RAISE(ABORT, 'refused'); END;"
    ))
    .await;

    assert_eq!(s.clean_all().await, 1);

    assert_eq!(s.cleanup(&first).await, ("asked".to_owned(), None));
    assert_eq!(s.cleanup(&second).await, ("done".to_owned(), None));
    assert!(s.removed("Show - 07.ass").await);
    assert!(!s.removed("Show - 02.ass").await);
}
