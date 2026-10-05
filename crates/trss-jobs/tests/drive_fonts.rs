//! A Google Drive font that did not change is not received again (ticket
//! 0074, `trss_jobs::place::unchanged`): the runner against a local server
//! shaped like Blogger, Naver and Google Drive ([`trss_subtitles::testing`]),
//! storing into a work's folder, with what the job's detail says of each font
//! and the restarts around a font not received.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use rusqlite::params;
use tokio_util::sync::CancellationToken;
use trss_core::{Clock, Db, DbError};
use trss_jobs::{
    place::{
        cleanup::{Asked, JOB_FILE},
        unchanged::{font_receipt, new_assets, revoke},
        ARCHIVE_LATER,
    },
    store::{FileRow, JobDetail},
    Created, FileState, ItemState, JobState, JobStore, NewItem, NewJob, ReceiveArea, Runner,
    Unpacker, Wait,
};
use trss_subtitles::{
    fake,
    testing::{
        blogger_page, drive_link, naver_file, DriveAnswer, FileAnswer, PostAnswer, SourceServer,
        DRIVE_FILES, DRIVE_MODIFIED,
    },
    verify, Sources,
};

const WORK: &str = "w1";
const CREATOR: &str = "C소라";
const FONT_ID: &str = "1fontOfThePosts";
const FONT: &[u8] = b"\x00\x01\x00\x00font one";
const OTHER_FONT: &[u8] = b"\x00\x01\x00\x00font two, longer";

struct Setup {
    dir: tempfile::TempDir,
    db: Db,
    store: JobStore,
    area: ReceiveArea,
    server: SourceServer,
    /// One clock for every runner: receipts are ordered by when they were
    /// made.
    clock: Clock,
}

fn ticking_clock() -> Clock {
    let now = Arc::new(AtomicI64::new(1_000));
    Arc::new(move || now.fetch_add(10, Ordering::SeqCst))
}

impl Setup {
    fn work(&self) -> PathBuf {
        self.dir.path().join("shows/Show")
    }

    fn stored_dir(&self) -> PathBuf {
        self.work().join(".trss/subtitles").join(CREATOR)
    }

    /// A runner over the Blogger and Naver sources of the server, with the
    /// program that unpacks archives or without.
    fn runner(&self, unpacks: bool) -> Runner {
        let runner = Runner::new(
            self.store.clone(),
            Sources::none()
                .with_blogger(self.server.blogger())
                .with_naver(self.server.naver()),
            self.area.clone(),
            self.clock.clone(),
        )
        .with_retry_waits(vec![Duration::from_millis(10), Duration::from_millis(10)]);
        match unpacks {
            true => runner.with_unpacker(Unpacker::new(PathBuf::from(env!(
                "CARGO_BIN_EXE_trss-extract"
            )))),
            false => runner,
        }
    }

    async fn run(&self) {
        self.runner(true)
            .run_ready(&CancellationToken::new())
            .await
            .unwrap();
    }

    async fn detail(&self, id: &str) -> JobDetail {
        self.store.detail(id).await.unwrap().unwrap()
    }

    async fn one<T: rusqlite::types::FromSql + Send + 'static>(&self, sql: String) -> T {
        self.db
            .run(move |c| c.query_row(&sql, [], |r| r.get(0)).map_err(DbError::from))
            .await
            .unwrap()
    }

    async fn sql(&self, sql: String) {
        self.db
            .run(move |c| c.execute_batch(&sql).map_err(DbError::from))
            .await
            .unwrap();
    }

    /// The fonts kept and not removed.
    async fn fonts_kept(&self) -> i64 {
        self.one(
            "SELECT count(*) FROM subtitle_assets WHERE kind = 'font' AND removed_at IS NULL"
                .into(),
        )
        .await
    }

    /// What the job's detail says of each font it kept, by name
    /// (`font_receipt` of its placements).
    async fn font_receipts(&self, id: &str) -> Vec<(String, &'static str)> {
        let d = self.detail(id).await;
        let receipts: HashMap<&str, &FileRow> = d
            .items
            .iter()
            .flat_map(|i| i.files.iter())
            .map(|f| (f.id.as_str(), f))
            .collect();
        let made: HashMap<i64, bool> = self
            .store
            .plan_paths(id)
            .await
            .unwrap()
            .into_iter()
            .map(|(position, at)| (position, at.made))
            .collect();
        let mut said: Vec<(String, &'static str)> = self
            .store
            .plan(id)
            .await
            .unwrap()
            .iter()
            .filter_map(|row| {
                let receipt = receipts.get(row.file_id.as_str()).copied();
                let made = made.get(&row.position).copied().unwrap_or(false);
                let said = font_receipt(row, receipt, made)?;
                Some((row.name.clone(), said.code()))
            })
            .collect();
        said.sort();
        said
    }

    /// The requests for Drive file `id` with `method`.
    fn asked(&self, id: &str, method: &str) -> usize {
        let query = format!("id={id}&");
        self.server
            .seen()
            .iter()
            .filter(|r| r.host == DRIVE_FILES && r.query.starts_with(&query) && r.method == method)
            .count()
    }

    /// A Blogger post of C소라 at `path` with these links (words, Drive ID).
    fn post(&self, path: &str, links: &[(&str, &str)]) -> String {
        let links: Vec<(String, String)> = links
            .iter()
            .map(|(words, id)| ((*words).to_owned(), drive_link(id)))
            .collect();
        let links: Vec<(&str, &str)> = links
            .iter()
            .map(|(w, h)| (w.as_str(), h.as_str()))
            .collect();
        self.server.blogger_post(
            "csora556",
            path,
            vec![PostAnswer::Page(blogger_page(&links))],
        );
        self.server.blogger_url("csora556", path)
    }

    /// Episode `n`'s post: its subtitle (`Show - 0n.ass` on Drive) and the
    /// font [`FONT_ID`].
    fn episode_post(&self, n: u32) -> String {
        let id = format!("1subtitleOfEpisode{n}");
        self.server.drive(
            &id,
            vec![DriveAnswer::File {
                name: format!("Show - 0{n}.ass"),
                bytes: fake::ass(&format!("Show - 0{n}")),
            }],
        );
        self.post(
            &format!("2026/07/{n}.html"),
            &[(&format!("{n}화"), &id), ("폰트", FONT_ID)],
        )
    }

    /// How the font answers from now on.
    fn font(&self, answers: Vec<DriveAnswer>) {
        self.server.drive(FONT_ID, answers);
    }

    /// A job of the work for episode `episode` of `post`.
    async fn make(&self, command: &str, episode: &str, post: String) -> String {
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
            items: vec![NewItem {
                observation_id: None,
                episode: episode.to_owned(),
                post_url: post,
                found_at: 500,
            }],
        };
        match self.store.create(job, 900).await.unwrap() {
            Created::Created(id) => id,
            other => panic!("created: {other:?}"),
        }
    }

    /// Runs a job of episode `n`'s post to its end, done.
    async fn episode(&self, n: u32) -> String {
        let id = self
            .make(&format!("c{n}"), &n.to_string(), self.episode_post(n))
            .await;
        self.run().await;
        let d = self.detail(&id).await;
        assert_eq!(d.row.state, JobState::Done, "{n}: {:?}", d.row.note);
        id
    }

    /// The font receipt of the job's item.
    async fn font_file(&self, id: &str) -> FileRow {
        self.detail(id)
            .await
            .items
            .iter()
            .flat_map(|i| i.files.iter())
            .find(|f| f.file_key == format!("drive:{FONT_ID}") && f.state != FileState::Abandoned)
            .cloned()
            .unwrap()
    }
}

/// A library with the work `Show` (season 1) whose episodes 1 to 6 have a
/// video.
async fn setup() -> Setup {
    let server = SourceServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let shows = dir.path().join("shows");
    std::fs::create_dir_all(shows.join("Show/Season 01")).unwrap();
    for n in 1..=6 {
        std::fs::write(
            shows.join(format!("Show/Season 01/Show S01E0{n}.mkv")),
            b"video",
        )
        .unwrap();
    }
    let path = shows.to_string_lossy().into_owned();
    db.run(move |c| {
        c.execute(
            "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', ?1, 0)",
            [path],
        )?;
        c.execute_batch(&format!(
            "INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('{WORK}', 'f1', 'Show');
             INSERT INTO seasons (work_id, number) VALUES ('{WORK}', 1);"
        ))?;
        for n in 1..=6 {
            c.execute(
                "INSERT INTO episodes (work_id, season, episode) VALUES (?1, 1, ?2)",
                params![WORK, format!("0{n}")],
            )?;
            c.execute(
                "INSERT INTO media_files (work_id, path, season, episode, kind)
                 VALUES (?1, ?2, 1, ?3, 'video')",
                params![
                    WORK,
                    format!("Season 01/Show S01E0{n}.mkv"),
                    format!("0{n}")
                ],
            )?;
        }
        Ok::<_, DbError>(())
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
        server,
        clock: ticking_clock(),
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

fn font_file(bytes: &[u8]) -> Vec<DriveAnswer> {
    vec![DriveAnswer::File {
        name: "Font.ttf".to_owned(),
        bytes: bytes.to_vec(),
    }]
}

fn font_at(bytes: &[u8], modified: &str) -> DriveAnswer {
    DriveAnswer::FileAt {
        name: "Font.ttf".to_owned(),
        bytes: bytes.to_vec(),
        modified: modified.to_owned(),
    }
}

/// The ID of the stored subtitle of episode `n`'s file and the fonts linked
/// to it.
async fn linked_fonts(s: &Setup, n: u32) -> Vec<String> {
    s.db.run(move |c| {
        let mut stmt = c.prepare(
            "SELECT a.relative_path FROM subtitle_stored_assets l
               JOIN subtitle_assets a ON a.id = l.asset_id
               JOIN subtitle_stored st ON st.id = l.stored_id
               JOIN subtitle_assets sa ON sa.id = st.subtitle_asset_id
              WHERE sa.relative_path LIKE ?1 ORDER BY a.relative_path",
        )?;
        let rows = stmt.query_map([format!("%/Show - 0{n}.ass")], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()
            .map_err(DbError::from)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn the_next_episodes_unchanged_drive_font_is_not_received_and_its_font_is_used() {
    let s = setup().await;
    s.font(font_file(FONT));
    let first = s.episode(1).await;
    // A key never kept as a font is received with no `HEAD`.
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (1, 0));
    assert_eq!(
        s.font_receipts(&first).await,
        [("Font.ttf".to_owned(), "new")]
    );

    let second = s.episode(2).await;
    // One `HEAD` and no transfer of the font; the subtitle is received.
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (1, 1));
    assert_eq!(s.asked("1subtitleOfEpisode2", "HEAD"), 0);
    assert_eq!(s.asked("1subtitleOfEpisode2", "GET"), 1);
    assert_eq!(
        s.font_receipts(&second).await,
        [("Font.ttf".to_owned(), "unchanged")]
    );
    let receipt = s.font_file(&second).await;
    assert_eq!(receipt.state, FileState::Done);
    assert_eq!(receipt.path, None);
    assert!(receipt.unchanged_asset.is_some());
    // The font is the first job's, linked to the new subtitle as well.
    assert_eq!(s.fonts_kept().await, 1);
    let plan = s.store.plan(&second).await.unwrap();
    let row = plan.iter().find(|r| r.name == "Font.ttf").unwrap();
    assert_eq!(row.asset_id, receipt.unchanged_asset);
    let entries: i64 = s
        .one(format!(
            "SELECT count(*) FROM subtitle_package_entries e
               JOIN subtitle_packages p ON p.id = e.package_id
              WHERE p.job_id = '{second}' AND e.asset_id = '{}'",
            receipt.unchanged_asset.clone().unwrap()
        ))
        .await;
    assert_eq!(entries, 1);
    let dir = format!(".trss/subtitles/{CREATOR}");
    assert_eq!(linked_fonts(&s, 2).await, [format!("{dir}/Font.ttf")]);
    assert_eq!(
        names(&s.stored_dir()),
        ["Font.ttf", "Show - 01.ass", "Show - 02.ass"]
    );
    assert!(s.work().join("Season 01/Show S01E02.ass").exists());
    let log = s.detail(&second).await.events;
    assert!(log
        .iter()
        .any(|e| e.message == "2화: 바뀌지 않은 폰트라 받지 않았어요"));
}

#[tokio::test]
async fn a_changed_last_modified_receives_the_font_and_keeps_new_bytes_numbered() {
    let s = setup().await;
    s.font(font_file(FONT));
    s.episode(1).await;

    // Other bytes under a new `Last-Modified`: received, kept beside the
    // first under a numbered name.
    let later = "Sat, 03 Oct 2026 09:00:00 GMT";
    s.font(vec![font_at(OTHER_FONT, later)]);
    let second = s.episode(2).await;
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (2, 1));
    assert_eq!(
        s.font_receipts(&second).await,
        [("Font.ttf".to_owned(), "new")]
    );
    assert_eq!(s.fonts_kept().await, 2);
    assert_eq!(
        std::fs::read(s.stored_dir().join("Font (2).ttf")).unwrap(),
        OTHER_FONT
    );

    // The same bytes under a later `Last-Modified`: received and compared, no
    // new font.
    let latest = "Sun, 04 Oct 2026 09:00:00 GMT";
    s.font(vec![font_at(OTHER_FONT, latest)]);
    let third = s.episode(3).await;
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (3, 2));
    assert_eq!(
        s.font_receipts(&third).await,
        [("Font.ttf".to_owned(), "same")]
    );
    assert_eq!(s.fonts_kept().await, 2);

    // That receipt is the record now: the next one is not received.
    let fourth = s.episode(4).await;
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (3, 3));
    assert_eq!(
        s.font_receipts(&fourth).await,
        [("Font.ttf".to_owned(), "unchanged")]
    );
    let dir = format!(".trss/subtitles/{CREATOR}");
    assert_eq!(linked_fonts(&s, 4).await, [format!("{dir}/Font (2).ttf")]);
}

/// A Naver post of `n`'s SMI and a font attached, the same bytes each time.
fn naver_post(s: &Setup, n: u32) -> String {
    let smi = format!("Show S01E0{n}.smi");
    let bytes = fake::srt(&format!("Show {n}"));
    let sami = format!(
        "<SAMI><BODY><SYNC Start=1000><P Class=KRCC>{}</BODY></SAMI>",
        String::from_utf8_lossy(&bytes)
            .lines()
            .next()
            .unwrap_or("x")
    )
    .into_bytes();
    let log_no = format!("22442842402{n}");
    s.server.naver_post(
        "gkdlfn0850",
        &log_no,
        vec![PostAnswer::Naver(vec![
            naver_file(&smi, sami.len()),
            naver_file("Font.ttf", FONT.len()),
        ])],
    );
    s.server
        .naver_attachment(&smi, vec![FileAnswer::Bytes(sami)]);
    s.server
        .naver_attachment("Font.ttf", vec![FileAnswer::Bytes(FONT.to_vec())]);
    s.server.naver_url("gkdlfn0850", &log_no)
}

#[tokio::test]
async fn a_naver_font_is_received_compared_and_not_kept_twice() {
    let s = setup().await;
    let first = s.make("n1", "1", naver_post(&s, 1)).await;
    s.run().await;
    assert_eq!(s.detail(&first).await.row.state, JobState::Done);
    assert_eq!(
        s.font_receipts(&first).await,
        [("Font.ttf".to_owned(), "new")]
    );
    let fetched = |s: &Setup| {
        s.server
            .seen()
            .iter()
            .filter(|r| r.host == trss_subtitles::naver::FILE_HOST && r.path.contains("/Font"))
            .count()
    };
    assert_eq!(fetched(&s), 1);

    let second = s.make("n2", "2", naver_post(&s, 2)).await;
    s.run().await;
    let d = s.detail(&second).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    // Received again (Naver tells no modified time), the same bytes: no new
    // font, and no `HEAD` of anything.
    assert_eq!(fetched(&s), 2);
    assert!(s.server.seen().iter().all(|r| r.method != "HEAD"));
    assert_eq!(
        s.font_receipts(&second).await,
        [("Font.ttf".to_owned(), "same")]
    );
    assert_eq!(s.fonts_kept().await, 1);
    let receipt = d.items[0]
        .files
        .iter()
        .find(|f| f.name == "Font.ttf")
        .unwrap();
    assert!(receipt.unchanged_asset.is_none() && receipt.path.is_some());
}

#[tokio::test]
async fn a_zip_whose_font_is_kept_is_received_whole_and_adds_no_font() {
    let s = setup().await;
    s.font(font_file(FONT));
    s.episode(1).await;

    let zip = verify::zip_of(&[("Font.ttf", FONT)]);
    s.server.drive(
        "1fontsInAZipFile",
        vec![DriveAnswer::File {
            name: "폰트.zip".to_owned(),
            bytes: zip,
        }],
    );
    s.server.drive(
        "1subtitleOfEpisode2",
        vec![DriveAnswer::File {
            name: "Show - 02.ass".to_owned(),
            bytes: fake::ass("Show - 02"),
        }],
    );
    let url = s.post(
        "2026/07/zip.html",
        &[("2화", "1subtitleOfEpisode2"), ("폰트", "1fontsInAZipFile")],
    );
    let id = s.make("z", "2", url).await;
    s.run().await;
    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    // The archive is received whole (no `HEAD`: it was never kept as a font).
    assert_eq!(s.asked("1fontsInAZipFile", "GET"), 1);
    assert_eq!(s.asked("1fontsInAZipFile", "HEAD"), 0);
    assert_eq!(
        s.font_receipts(&id).await,
        [("Font.ttf".to_owned(), "same")]
    );
    assert_eq!(s.fonts_kept().await, 1);
    let plan = s.store.plan(&id).await.unwrap();
    let made: HashMap<i64, bool> = s
        .store
        .plan_paths(&id)
        .await
        .unwrap()
        .into_iter()
        .map(|(p, at)| (p, at.made))
        .collect();
    let archive = d.items[0]
        .files
        .iter()
        .find(|f| f.name == "폰트.zip")
        .unwrap();
    assert_eq!(new_assets(&plan, &made).get(archive.id.as_str()), Some(&0));
}

#[tokio::test]
async fn a_font_is_received_when_its_head_fails_or_has_no_last_modified() {
    let s = setup().await;
    s.font(font_file(FONT));
    s.episode(1).await;

    // A `HEAD` Drive cannot answer: the font is received (the next answer).
    s.font(vec![
        DriveAnswer::Status(503),
        font_at(FONT, DRIVE_MODIFIED),
    ]);
    let second = s.episode(2).await;
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (2, 1));
    assert_eq!(
        s.font_receipts(&second).await,
        [("Font.ttf".to_owned(), "same")]
    );
    assert!(s
        .detail(&second)
        .await
        .events
        .iter()
        .any(|e| e.message == "2화: 폰트가 바뀌었는지 확인하지 못해 받아요"));

    // A `HEAD` with no `Last-Modified`: received.
    s.font(vec![DriveAnswer::Undated {
        name: "Font.ttf".to_owned(),
        bytes: FONT.to_vec(),
    }]);
    let third = s.episode(3).await;
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (3, 2));
    assert_eq!(
        s.font_receipts(&third).await,
        [("Font.ttf".to_owned(), "same")]
    );
    // That receipt is the record now, and it has no `Last-Modified`: the
    // next is received with no `HEAD`, and its own `Last-Modified` makes it
    // the record the one after is not received by.
    s.font(font_file(FONT));
    let fourth = s.episode(4).await;
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (4, 2));
    assert_eq!(
        s.font_receipts(&fourth).await,
        [("Font.ttf".to_owned(), "same")]
    );
    let fifth = s.episode(5).await;
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (4, 3));
    assert_eq!(
        s.font_receipts(&fifth).await,
        [("Font.ttf".to_owned(), "unchanged")]
    );
    assert_eq!(s.fonts_kept().await, 1);
}

#[tokio::test]
async fn a_font_whose_kept_file_is_missing_is_received_with_no_head() {
    let s = setup().await;
    s.font(font_file(FONT));
    s.episode(1).await;
    std::fs::remove_file(s.stored_dir().join("Font.ttf")).unwrap();

    let second = s.episode(2).await;
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (2, 0));
    let receipt = s.font_file(&second).await;
    assert!(receipt.unchanged_asset.is_none());
    assert_eq!(
        s.font_receipts(&second).await,
        [("Font.ttf".to_owned(), "new")]
    );
    assert_eq!(
        std::fs::read(s.stored_dir().join("Font (2).ttf")).unwrap(),
        FONT
    );
}

#[tokio::test]
async fn a_font_a_cleanup_removed_is_received_with_no_head() {
    let s = setup().await;
    s.font(font_file(FONT));
    // Episode 9 has no video: its subtitle is stored only, and a person may
    // clean it with its font.
    let id = s.make("c9", "9", s.episode_post(9)).await;
    s.run().await;
    let d = s.detail(&id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Video))
    );
    let entry = s
        .store
        .work_files(WORK)
        .await
        .unwrap()
        .cleanable
        .into_iter()
        .find(|e| e.name == "Show - 09.ass")
        .unwrap();
    let assets: Vec<String> = entry.with.iter().map(|f| f.id.clone()).collect();
    assert_eq!(assets.len(), 2, "the subtitle and its font");
    assert!(matches!(
        s.store
            .clean_stored(WORK, &entry.id, assets, 60_000)
            .await
            .unwrap(),
        Asked::Asked(_)
    ));
    assert_eq!(s.runner(true).run_cleanups().await.unwrap(), 1);
    assert_eq!(s.fonts_kept().await, 0);

    let second = s.episode(2).await;
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (2, 0));
    assert_eq!(
        s.font_receipts(&second).await,
        [("Font.ttf".to_owned(), "new")]
    );
    assert_eq!(names(&s.stored_dir()), ["Font.ttf", "Show - 02.ass"]);
}

#[tokio::test]
async fn a_run_cut_before_the_font_was_recorded_asks_its_head_again() {
    let s = setup().await;
    s.font(font_file(FONT));
    s.episode(1).await;

    // A run that asked the font's `HEAD` and stopped before recording what
    // it said leaves the job and its item running, with no receipt of it.
    let id = s.make("c2", "2", s.episode_post(2)).await;
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'running', stage = 'receive' WHERE id = '{id}';
         UPDATE subtitle_job_items SET state = 'running' WHERE job_id = '{id}';"
    ))
    .await;
    s.run().await;
    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (1, 1));
    assert_eq!(
        s.font_receipts(&id).await,
        [("Font.ttf".to_owned(), "unchanged")]
    );
}

/// Episode 2's post with the font [`FONT_ID`] and an archive besides: a
/// worker without the program that unpacks receives it and leaves its
/// placement for later, the font not received.
async fn skipped_and_waiting(s: &Setup) -> String {
    s.font(font_file(FONT));
    s.episode(1).await;
    episode_2_waiting(s).await
}

/// The job of [`skipped_and_waiting`] for episode 2, once a job before kept
/// the font.
async fn episode_2_waiting(s: &Setup) -> String {
    s.server.drive(
        "1subtitleOfEpisode2",
        vec![DriveAnswer::File {
            name: "Show - 02.ass".to_owned(),
            bytes: fake::ass("Show - 02"),
        }],
    );
    s.server.drive(
        "1moreFontsInAZip",
        vec![DriveAnswer::File {
            name: "더 많은 폰트.zip".to_owned(),
            bytes: verify::zip_of(&[("Other.ttf", OTHER_FONT)]),
        }],
    );
    let url = s.post(
        "2026/07/2.html",
        &[
            ("2화", "1subtitleOfEpisode2"),
            ("폰트", FONT_ID),
            ("폰트 모음", "1moreFontsInAZip"),
        ],
    );
    let id = s.make("c2", "2", url).await;
    s.runner(false)
        .run_ready(&CancellationToken::new())
        .await
        .unwrap();
    let d = s.detail(&id).await;
    assert_eq!(
        (d.row.state, d.row.wait, d.row.note.as_deref()),
        (JobState::Waiting, Some(Wait::Subtitle), Some(ARCHIVE_LATER))
    );
    assert!(s.font_file(&id).await.unchanged_asset.is_some());
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (1, 1));
    id
}

/// The job of [`skipped_and_waiting`] taken up again: the font it did not
/// receive went away meanwhile, so it receives it, kept as `kept`.
async fn received_instead(s: &Setup, id: &str, kept: &str) {
    s.store.requeue_waiting_for_sources(5_000).await.unwrap();
    s.run().await;
    let d = s.detail(id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert!(d.items.iter().all(|i| i.state == ItemState::Done));
    // The font received anew, with no second `HEAD`.
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (2, 1));
    let receipts: Vec<(FileState, bool)> = d.items[0]
        .files
        .iter()
        .filter(|f| f.file_key == format!("drive:{FONT_ID}"))
        .map(|f| (f.state, f.unchanged_asset.is_some()))
        .collect();
    assert_eq!(
        receipts,
        [(FileState::Abandoned, true), (FileState::Done, false)]
    );
    assert!(d
        .events
        .iter()
        .any(|e| e.message == format!("Font.ttf: {}", trss_jobs::place::unchanged::RECEIVE_AGAIN)));
    assert_eq!(std::fs::read(s.stored_dir().join(kept)).unwrap(), FONT);
    assert_eq!(
        s.font_receipts(id).await,
        [
            ("Font.ttf".to_owned(), "new"),
            ("Other.ttf".to_owned(), "new")
        ]
    );
    // The subtitle is linked to the font received and to the archive's.
    let dir = format!(".trss/subtitles/{CREATOR}");
    assert_eq!(
        linked_fonts(s, 2).await,
        [format!("{dir}/{kept}"), format!("{dir}/Other.ttf")]
    );
}

#[tokio::test]
async fn a_font_not_received_whose_kept_file_changed_meanwhile_is_received() {
    let s = setup().await;
    let id = skipped_and_waiting(&s).await;
    // A person wrote over the kept font: the received one is kept beside it.
    std::fs::write(s.stored_dir().join("Font.ttf"), b"changed by a person").unwrap();
    received_instead(&s, &id, "Font (2).ttf").await;
}

#[tokio::test]
async fn a_font_not_received_whose_kept_file_was_removed_meanwhile_is_received() {
    let s = setup().await;
    let id = skipped_and_waiting(&s).await;
    // What a cleanup leaves: the file gone, the font recorded as removed.
    std::fs::remove_file(s.stored_dir().join("Font.ttf")).unwrap();
    s.sql("UPDATE subtitle_assets SET removed_at = 1 WHERE kind = 'font'".to_owned())
        .await;
    received_instead(&s, &id, "Font.ttf").await;
}

#[tokio::test]
async fn a_cleanup_keeps_the_font_a_job_did_not_receive_until_it_stores_it() {
    let s = setup().await;
    s.font(font_file(FONT));
    // Episode 9 has no video: its subtitle is stored only, and a person may
    // clean it with its font.
    s.make("c9", "9", s.episode_post(9)).await;
    s.run().await;
    let id = episode_2_waiting(&s).await;

    // Episode 2's job uses the font when it stores its rows: cleaning
    // episode 9 leaves it.
    let entry = s
        .store
        .work_files(WORK)
        .await
        .unwrap()
        .cleanable
        .into_iter()
        .find(|e| e.name == "Show - 09.ass")
        .unwrap();
    let going: Vec<&str> = entry.with.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(going, ["Show - 09.ass"]);
    let staying: Vec<(&str, &str)> = entry
        .kept
        .iter()
        .map(|f| (f.name.as_str(), f.reason))
        .collect();
    assert_eq!(staying, [("Font.ttf", JOB_FILE)]);
    let assets: Vec<String> = entry.with.iter().map(|f| f.id.clone()).collect();
    assert!(matches!(
        s.store
            .clean_stored(WORK, &entry.id, assets, 60_000)
            .await
            .unwrap(),
        Asked::Asked(_)
    ));
    assert_eq!(s.runner(true).run_cleanups().await.unwrap(), 1);
    assert_eq!(s.fonts_kept().await, 1);

    // The job goes on with the font, still not received.
    s.store.requeue_waiting_for_sources(5_000).await.unwrap();
    s.run().await;
    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (1, 1));
    assert_eq!(
        s.font_receipts(&id).await,
        [
            ("Font.ttf".to_owned(), "unchanged"),
            ("Other.ttf".to_owned(), "new")
        ]
    );
    let dir = format!(".trss/subtitles/{CREATOR}");
    assert_eq!(
        linked_fonts(&s, 2).await,
        [format!("{dir}/Font.ttf"), format!("{dir}/Other.ttf")]
    );
}

#[tokio::test]
async fn a_font_found_unchanged_by_a_run_cut_short_is_not_told_as_bytes_checked() {
    let s = setup().await;
    let id = skipped_and_waiting(&s).await;
    // A run cut after the font's receipt was recorded and before its item
    // ended leaves the job and the item running.
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'running', wait = NULL, stage = 'receive'
          WHERE id = '{id}';
         UPDATE subtitle_job_items SET state = 'running' WHERE job_id = '{id}';"
    ))
    .await;
    s.run().await;
    let d = s.detail(&id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.row.note);
    assert_eq!((s.asked(FONT_ID, "GET"), s.asked(FONT_ID, "HEAD")), (1, 1));
    let told: Vec<(&str, Option<&str>)> = d
        .events
        .iter()
        .filter(|e| {
            e.detail
                .as_deref()
                .is_some_and(|d| d.starts_with("Font.ttf"))
        })
        .map(|e| (e.message.as_str(), e.detail.as_deref()))
        .collect();
    // The first run's line, with what the `HEAD` said, and the resumed
    // run's; never a check of bytes that were not received.
    assert!(
        told.contains(&("2화: 바뀌지 않은 폰트라 받지 않았어요", Some("Font.ttf"))),
        "{told:?}"
    );
    assert!(
        !told.iter().any(|(m, _)| m.contains("이미 받은 파일")),
        "{told:?}"
    );
    assert_eq!(
        s.font_receipts(&id).await,
        [
            ("Font.ttf".to_owned(), "unchanged"),
            ("Other.ttf".to_owned(), "new")
        ]
    );
}

#[tokio::test]
async fn a_font_that_went_away_puts_back_only_the_items_that_received_it() {
    let s = setup().await;
    let id = skipped_and_waiting(&s).await;
    let receipt = s.font_file(&id).await;
    // Another item of the job shares the receipt and was held for a reason
    // of its own.
    s.sql(format!(
        "INSERT INTO subtitle_job_items (job_id, position, episode, post_url, found_at, state,
                                         reason, updated_at)
         VALUES ('{id}', 9, '3', 'https://example.invalid/3', 500, 'held', '다른 까닭', 1);
         INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state, same_as,
                                         size, sha256, format, kind, unchanged_asset,
                                         created_at, updated_at)
         SELECT 'sharer', job_id,
                (SELECT id FROM subtitle_job_items WHERE job_id = '{id}' AND position = 9),
                file_key, name, 'done', id, size, sha256, format, kind, unchanged_asset, 2, 2
           FROM subtitle_job_files WHERE id = '{}';",
        receipt.id
    ))
    .await;

    let gone = receipt.id.clone();
    s.db.run(move |c| revoke(c, &gone, "없어졌어요", 3_000))
        .await
        .unwrap();
    let d = s.detail(&id).await;
    let items: Vec<(&str, ItemState, Option<&str>)> = d
        .items
        .iter()
        .map(|i| (i.episode.as_str(), i.state, i.reason.as_deref()))
        .collect();
    assert_eq!(
        items,
        [
            ("2", ItemState::Pending, None),
            ("3", ItemState::Held, Some("다른 까닭"))
        ]
    );
    let states: Vec<String> =
        s.db.run(|c| {
            let mut stmt = c.prepare(
                "SELECT state FROM subtitle_job_files
                  WHERE unchanged_asset IS NOT NULL ORDER BY id = 'sharer'",
            )?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<String>>>()
                .map_err(DbError::from)
        })
        .await
        .unwrap();
    assert_eq!(states, ["abandoned", "abandoned"]);
}

#[tokio::test]
async fn the_migration_adds_the_font_a_receipt_uses_to_receipts_of_earlier_builds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let conn = trss_core::db::database_at(&path, 55);
    conn.execute_batch(
        "INSERT INTO subtitle_jobs (id, command_id, request, origin, state, created_at,
                                    updated_at, state_at)
         VALUES ('j', 'j', '{}', 'pick', 'done', 0, 0, 0);
         INSERT INTO subtitle_job_items (id, job_id, position, episode, post_url, found_at,
                                         state, updated_at)
         VALUES (1, 'j', 0, '01', 'https://x/', 0, 'done', 5);
         INSERT INTO subtitle_job_files (id, job_id, item_id, file_key, name, state, path,
                                         size, created_at, updated_at, snapshot)
         VALUES ('f', 'j', 1, 'drive:1fontOfThePosts', 'Font.ttf', 'done', 'j/Font.ttf', 5, 0,
                 0, '[[\"last_modified\",\"Fri, 02 Oct 2026 02:11:00 GMT\"]]');
         INSERT INTO subtitle_assets (id, work_id, kind, base, relative_path, byte_size, sha256,
                                      created_at)
         VALUES ('a', 'w', 'font', 'work', '.trss/subtitles/c/Font.ttf', 5, printf('%064d', 0),
                 0);",
    )
    .unwrap();
    drop(conn);
    let db = Db::open(&path).await.unwrap();
    let (kept, refused, accepted): (Option<String>, [bool; 2], bool) = db
        .run(|c| {
            let kept = c.query_row(
                "SELECT unchanged_asset FROM subtitle_job_files WHERE id = 'f'",
                [],
                |r| r.get(0),
            )?;
            let fails = |sql: &str| c.execute(sql, []).is_err();
            let refused = [
                // A receipt with its own file names no font it uses instead.
                fails("UPDATE subtitle_job_files SET unchanged_asset = 'a' WHERE id = 'f'"),
                // Nor a font that is not there.
                fails(
                    "UPDATE subtitle_job_files SET path = NULL, unchanged_asset = 'gone'
                      WHERE id = 'f'",
                ),
            ];
            let accepted = c
                .execute(
                    "UPDATE subtitle_job_files SET path = NULL, unchanged_asset = 'a'
                      WHERE id = 'f'",
                    [],
                )
                .is_ok();
            Ok::<_, DbError>((kept, refused, accepted))
        })
        .await
        .unwrap();
    assert_eq!(kept, None);
    assert_eq!(refused, [true, true]);
    assert!(accepted);
}
