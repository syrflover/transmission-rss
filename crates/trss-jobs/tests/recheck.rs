//! The 14-day recheck of the files of the episodes received from the
//! subscribed creator (`docs/specs/subtitles.md`, 구독 제작자 자동 수신), with
//! the local servers shaped like Drive, Tistory and Naver
//! ([`trss_subtitles::testing`]), and once against the real sites (ignored).
//!
//! The runner's clock ticks from `NOW`, so a receipt is `NOW` old to the
//! millisecond; a pass of the recheck is given the time it is to run at.

use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use trss_collect::store::channels::{
    ChannelInput, ChannelStore, NewSubscription, Rule, RuleInput, SubtitleMode,
};
use trss_core::{Clock, Db, DbError};
use trss_jobs::{
    area::ReceiveArea, recheck::Report, store::JobDetail, Created, Follow, ItemState, JobState,
    JobStore, NewItem, NewJob, Recheck, Runner, Wait, AUTO,
};
use trss_subtitles::{
    auth::{self, Answered, AuthBrowser, BoxFuture, PrepareRequest, Prepared, Waited},
    testing::{
        blogger_page, drive_link, erulabo_page, naver_file, spec, DriveAnswer, FileAnswer,
        PostAnswer, SourceServer,
    },
    verify, Failure, Sources,
};

const ANIME: i64 = 3441;
const WORK: &str = "w1";
const NOW: i64 = 1_000_000;
const DAY: i64 = 24 * 60 * 60 * 1000;
const HOUR: i64 = 60 * 60 * 1000;

/// The Drive file of the creator's 5화 post, and what it was when received.
const DRIVE_ID: &str = "1kairan0013episode";
const SRT_A: &[u8] = b"1\n00:00:01,000 --> 00:00:02,000\nHello\n";
const SRT_B: &[u8] = b"1\n00:00:01,000 --> 00:00:02,000\nHello there\n";
const MODIFIED_A: &str = "Tue, 29 Sep 2026 19:53:19 GMT";
const MODIFIED_B: &str = "Fri, 02 Oct 2026 04:15:40 GMT";

struct World {
    _dir: tempfile::TempDir,
    db: Db,
    follow: Follow,
    jobs: JobStore,
    channels: ChannelStore,
    runner: Runner,
    recheck: Recheck,
    sources: Sources,
    server: SourceServer,
    rule: Rule,
}

/// A clock that shows `at`.
fn fixed(at: i64) -> Clock {
    Arc::new(move || at)
}

fn ticking_clock() -> Clock {
    let now = Arc::new(AtomicI64::new(NOW));
    Arc::new(move || now.fetch_add(10, Ordering::SeqCst))
}

/// Every item of the job is received. A package this build does not
/// analyse yet (an archive) leaves the job waiting for it; the recheck reads
/// receipts whatever came of them.
fn assert_received(d: &JobDetail) {
    assert!(
        d.items.iter().all(|i| i.state == ItemState::Done),
        "{:?}",
        d.events
    );
    match d.row.state {
        JobState::Done => {}
        JobState::Waiting => assert_eq!(d.row.wait, Some(Wait::Subtitle), "{:?}", d.events),
        other => panic!("{other:?}: {:?}", d.events),
    }
}

impl World {
    async fn new() -> World {
        let server = SourceServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        // The work's folder, where what its jobs receive is stored.
        let media = dir.path().join("media");
        std::fs::create_dir_all(media.join("Show")).unwrap();
        let media = media.to_string_lossy().into_owned();
        db.run::<_, DbError, _>(move |c| {
            c.execute(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', ?1, 1)",
                [media],
            )?;
            c.execute_batch(
                "INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
                 INSERT INTO seasons (work_id, number) VALUES ('w1', 1);
                 INSERT OR IGNORE INTO season_info (work_id, season) VALUES ('w1', 1);
                 INSERT INTO anilist_entries (id, format, episodes, fetched_at)
                     VALUES (1, 'TV', 12, 1);
                 INSERT INTO season_entries (work_id, season, position, anilist_id)
                     VALUES ('w1', 1, 0, 1);",
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let channels = ChannelStore::new(db.clone());
        let channel = channels
            .create_channel(ChannelInput::new("https://feed.test/rss"))
            .await
            .unwrap();
        let rule = channels
            .create_subscription_rule(
                &channel.id,
                RuleInput {
                    r#match: Some("Show".into()),
                    directory: "Show".into(),
                    episode: 0,
                    ..RuleInput::default()
                },
                NewSubscription {
                    anime: trss_anissia::Anime {
                        anime_no: ANIME,
                        subject: "작품".into(),
                        original_subject: None,
                        week: 3,
                        air_time: Some("22:30".into()),
                        start_date: None,
                        end_date: None,
                        status: "ON".into(),
                        fetched_at: 1,
                    },
                    subtitles: SubtitleMode::Follow,
                    creator: Some("에루샤".to_owned()),
                    subscribed_at: 1,
                },
            )
            .await
            .unwrap();
        channels
            .link_season(&rule.id, &format!("{WORK}:1"))
            .await
            .unwrap();
        let rule = channels.get_rule(&rule.id).await.unwrap().unwrap();
        // These tests are about what is read again once an episode is received,
        // not about the mapping the app decides: the user has mapped the
        // creator's episodes as they are, which the app never changes.
        db.run::<_, DbError, _>(|c| {
            c.execute_batch(
                "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('src-에루샤', 3441, '에루샤', 1);
                 INSERT INTO subtitle_episode_mappings
                     (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
                     VALUES ('w1', 1, 'src-에루샤', 'user', 0, '사용자가 정했어요', 1);",
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let sources = Sources::none()
            .with_blogger(server.blogger())
            .with_tistory(server.source())
            .with_naver(server.naver())
            .with_erulabo(server.erulabo());
        let jobs = JobStore::new(db.clone());
        let runner = Runner::new(
            jobs.clone(),
            sources.clone(),
            ReceiveArea::in_app_data(dir.path()),
            ticking_clock(),
        );
        World {
            follow: Follow::new(db.clone()),
            recheck: Recheck::new(db.clone(), sources.clone()),
            _dir: dir,
            db,
            jobs,
            channels,
            runner,
            sources,
            server,
            rule,
        }
    }

    /// A line of the anime as the reading stores it, of `creator`.
    async fn observe(&self, creator: &str, episode: &str, post: &str, updated: &str) -> i64 {
        let (creator, episode, post, updated) = (
            creator.to_owned(),
            episode.to_owned(),
            post.to_owned(),
            updated.to_owned(),
        );
        self.db
            .run::<_, DbError, _>(move |c| {
                c.execute(
                    "INSERT OR IGNORE INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('src-' || ?1, ?2, ?1, 1)",
                    rusqlite::params![creator, ANIME],
                )?;
                c.execute(
                    "INSERT INTO caption_observations
                         (source_id, post_url, episode, updated, first_seen_at)
                     VALUES ('src-' || ?1, ?2, ?3, ?4, 2)",
                    rusqlite::params![creator, post, episode, updated],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    /// The library's files of the season's episode: a video (`mkv`) or a
    /// subtitle (`ass`).
    async fn file(&self, episode: u32, ext: &'static str) {
        let kind = if ext == "mkv" { "video" } else { "subtitle" };
        self.db
            .run::<_, DbError, _>(move |c| {
                let episode = format!("{episode:02}");
                c.execute(
                    "INSERT OR IGNORE INTO episodes (work_id, season, episode) VALUES ('w1', 1, ?1)",
                    [&episode],
                )?;
                c.execute(
                    "INSERT INTO media_files (work_id, path, season, episode, kind)
                     VALUES ('w1', ?1, 1, ?2, ?3)",
                    rusqlite::params![format!("Season 01/Show S01E{episode}.{ext}"), episode, kind],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn library(&self) -> Vec<String> {
        self.db
            .run::<_, DbError, _>(|c| {
                let mut stmt = c.prepare("SELECT path FROM media_files ORDER BY path")?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
            })
            .await
            .unwrap()
    }

    /// Makes the subscribed creator's job for what was observed and runs it:
    /// the receipt every recheck starts from.
    async fn receive(&self) -> String {
        let made = self.follow.evaluate(NOW).await.unwrap();
        assert_eq!(made.len(), 1, "one job for the observation");
        self.run().await;
        let d = self.detail(&made[0]).await;
        assert_received(&d);
        made[0].clone()
    }

    async fn run(&self) {
        self.runner
            .run_ready(&CancellationToken::new())
            .await
            .unwrap();
    }

    /// A pass of the recheck at `NOW` plus `after`.
    async fn recheck_at(&self, after: i64) -> Report {
        self.recheck
            .run(&fixed(NOW + after), &CancellationToken::new())
            .await
            .unwrap()
    }

    async fn detail(&self, id: &str) -> JobDetail {
        self.jobs.detail(id).await.unwrap().unwrap()
    }

    async fn job_count(&self) -> i64 {
        self.db
            .run::<_, DbError, _>(|c| {
                Ok(c.query_row("SELECT count(*) FROM subtitle_jobs", [], |r| r.get(0))?)
            })
            .await
            .unwrap()
    }

    /// The recheck record of the first item of job `job`:
    /// (result, checks, the job it made).
    async fn record(&self, job: &str) -> Option<(Option<String>, i64, Option<String>)> {
        let job = job.to_owned();
        self.db
            .run::<_, DbError, _>(move |c| {
                let mut stmt = c.prepare(
                    "SELECT r.result, r.checks, r.job_id FROM subtitle_item_rechecks r
                       JOIN subtitle_job_items i ON i.id = r.item_id WHERE i.job_id = ?1",
                )?;
                let mut rows = stmt.query_map([job], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                Ok(rows.next().transpose()?)
            })
            .await
            .unwrap()
    }

    /// The Drive file's script: what it answers to the receipt, and to the
    /// reading after it.
    fn drive(&self, answers: Vec<DriveAnswer>) {
        self.server.drive(DRIVE_ID, answers);
    }

    /// The creator's 5화 post on Blogger with the Drive file, observed, with a
    /// video in the library.
    async fn drive_post(&self) -> i64 {
        self.server.blogger_post(
            "kairan03",
            "2026/09/13.html",
            vec![PostAnswer::Page(blogger_page(&[(
                "5화",
                drive_link(DRIVE_ID).as_str(),
            )]))],
        );
        self.file(5, "mkv").await;
        self.observe(
            "에루샤",
            "5",
            &self.server.blogger_url("kairan03", "2026/09/13.html"),
            "2026-10-02T11:00:00",
        )
        .await
    }

    fn drive_head_count(&self) -> usize {
        self.server
            .seen()
            .iter()
            .filter(|s| s.method == "HEAD")
            .count()
    }
}

fn file(name: &str, bytes: &[u8], modified: &str) -> DriveAnswer {
    DriveAnswer::FileAt {
        name: name.to_owned(),
        bytes: bytes.to_vec(),
        modified: modified.to_owned(),
    }
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[tokio::test]
async fn a_drive_file_whose_size_changed_three_days_after_makes_a_revision_job_and_the_subtitle_stays(
) {
    let w = World::new().await;
    let first_observed = w.drive_post().await;
    // The file as received, then as the creator replaced it.
    w.drive(vec![
        file("Show 05.srt", SRT_A, MODIFIED_A),
        file("Show 05.srt", SRT_B, MODIFIED_B),
    ]);
    let first = w.receive().await;
    // The receipt was put in place by the application.
    w.file(5, "ass").await;
    let library = w.library().await;
    let post_reads = || {
        w.server
            .seen()
            .iter()
            .filter(|s| s.host.contains("blogspot"))
            .count()
    };
    let reads_before = post_reads();

    // Anissia's line is as it was; three days later the Drive file is longer.
    let report = w.recheck_at(3 * DAY).await;
    assert_eq!((report.changed, report.read()), (1, 1));
    assert_eq!(report.jobs.len(), 1);
    // A Drive file's address is fixed: the post was not read again, and the
    // job and its log carry the time of the reading.
    assert_eq!(post_reads(), reads_before);
    let made = w.detail(&report.jobs[0]).await;
    assert!(made.events.iter().all(|e| e.at == NOW + 3 * DAY));

    let d = w.detail(&report.jobs[0]).await;
    assert_eq!(d.row.origin, AUTO);
    assert_eq!(d.row.state, JobState::Pending);
    assert_eq!(d.row.revision_of, Some(first_observed));
    assert_eq!(d.row.revises_job.as_deref(), Some(first.as_str()));
    assert_eq!(d.row.creator.as_deref(), Some("에루샤"));
    assert_eq!(d.items[0].observation_id, Some(first_observed));
    assert_eq!(d.items[0].episode, "5");
    assert_eq!(d.events.len(), 2);
    assert_eq!(
        d.events[0].message,
        "받은 파일의 정보가 받은 때와 달라서 다시 받아요"
    );
    assert_eq!(
        d.events[0].detail.as_deref(),
        Some(format!("Show 05.srt: {} → {}바이트", SRT_A.len(), SRT_B.len()).as_str())
    );
    assert_eq!(
        w.record(&first).await,
        Some((Some("changed".into()), 1, Some(report.jobs[0].clone())))
    );

    // The post was not read (a Drive file's address is fixed); the file was
    // asked for by a `HEAD`, with no cookie and no `Referer`.
    let seen = w.server.seen();
    let after_receipt = seen.iter().filter(|s| s.method == "HEAD").count();
    assert_eq!(after_receipt, 1);
    assert!(seen.iter().all(|s| !s.cookie && !s.referer));

    // Run: the post is received again; the library's subtitle is untouched.
    w.run().await;
    let d = w.detail(&report.jobs[0]).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(
        d.items[0].files[0].sha256.as_deref(),
        Some(sha(SRT_B).as_str())
    );
    assert_eq!(d.items[0].unchanged_from, None);
    assert_eq!(w.library().await, library);
    assert_eq!(w.job_count().await, 2);
}

#[tokio::test]
async fn a_tistory_post_whose_modified_time_changed_with_the_same_total_is_not_received_again() {
    let w = World::new().await;
    let zip = verify::zip_of(&[("Show - 05.srt", SRT_A)]);
    let post = w.server.post_url("sumomomo", 491);
    w.server.post(
        "sumomomo",
        491,
        vec![
            PostAnswer::Files(vec![spec("z1", "Show - 05.zip", "0.01MB")]),
            // Read again two days later: only the post was edited.
            PostAnswer::FilesAt(
                vec![spec("z1", "Show - 05.zip", "0.01MB")],
                "2026-10-03T09:00:00+09:00".to_owned(),
            ),
        ],
    );
    w.server.file("z1", vec![FileAnswer::Bytes(zip.clone())]);
    w.file(5, "mkv").await;
    w.observe("에루샤", "5", &post, "2026-10-02T11:00:00").await;
    let first = w.receive().await;

    let report = w.recheck_at(2 * DAY).await;
    assert_eq!((report.same, report.read()), (1, 1));
    assert!(report.jobs.is_empty());
    assert_eq!(w.job_count().await, 1);
    assert_eq!(w.record(&first).await, Some((Some("same".into()), 1, None)));
    // The file was asked for one byte; the post was read again; no `HEAD`
    // (the CDN refuses it) and no whole file the second time.
    let seen = w.server.seen();
    let ranged: Vec<_> = seen
        .iter()
        .filter(|s| s.range.as_deref() == Some("bytes=0-0"))
        .collect();
    assert_eq!(ranged.len(), 1);
    assert_eq!(ranged[0].method, "GET");
    assert!(seen.iter().all(|s| s.method != "HEAD"));

    // The file itself grew: now it is received again.
    let bigger = verify::zip_of(&[("Show - 05.srt", SRT_B)]);
    w.server.file("z1", vec![FileAnswer::Bytes(bigger.clone())]);
    let report = w.recheck_at(3 * DAY + HOUR).await;
    assert_eq!((report.changed, report.jobs.len()), (1, 1));
    w.run().await;
    let d = w.detail(&report.jobs[0]).await;
    assert_received(&d);
    assert_eq!(d.items[0].files[0].size, Some(bigger.len() as u64));
}

#[tokio::test]
async fn a_naver_attachment_is_read_from_the_page_and_a_size_change_makes_a_revision() {
    let w = World::new().await;
    let post = w.server.naver_url("elainalove1017", "224324105274");
    w.server.naver_post(
        "elainalove1017",
        "224324105274",
        vec![
            PostAnswer::Naver(vec![naver_file("Show 05.srt", SRT_A.len())]),
            PostAnswer::Naver(vec![naver_file("Show 05.srt", SRT_A.len())]),
            PostAnswer::Naver(vec![naver_file("Show 05.srt", SRT_B.len())]),
        ],
    );
    w.server
        .naver_attachment("Show 05.srt", vec![FileAnswer::Bytes(SRT_A.to_vec())]);
    w.file(5, "mkv").await;
    w.observe("에루샤", "5", &post, "2026-10-02T11:00:00").await;
    w.receive().await;

    // The size the page gives is the size received.
    let report = w.recheck_at(DAY + HOUR).await;
    assert_eq!((report.same, report.jobs.len()), (1, 0), "{report:?}");
    // The attachment was never asked for again.
    assert_eq!(
        w.server
            .seen()
            .iter()
            .filter(|s| s.host == trss_subtitles::naver::FILE_HOST)
            .count(),
        1
    );

    let report = w.recheck_at(2 * DAY + 2 * HOUR).await;
    assert_eq!((report.changed, report.jobs.len()), (1, 1));
}

#[tokio::test]
async fn a_receipt_fifteen_days_old_is_not_read() {
    let w = World::new().await;
    w.drive_post().await;
    w.drive(vec![file("Show 05.srt", SRT_A, MODIFIED_A)]);
    let first = w.receive().await;

    // Fifteen days later: no request at all, and no record.
    let report = w.recheck_at(15 * DAY).await;
    assert_eq!(report.read(), 0);
    assert_eq!(w.drive_head_count(), 0);
    assert_eq!(w.record(&first).await, None);
    // Just inside the window it is read.
    let report = w.recheck_at(14 * DAY - HOUR).await;
    assert_eq!(report.read(), 1);
    assert_eq!(w.drive_head_count(), 1);
    // And not before the first day is out.
    let w = World::new().await;
    w.drive_post().await;
    w.drive(vec![file("Show 05.srt", SRT_A, MODIFIED_A)]);
    w.receive().await;
    assert_eq!(w.recheck_at(DAY - 2 * HOUR).await.read(), 0);
    assert_eq!(w.drive_head_count(), 0);
}

#[tokio::test]
async fn a_restart_the_same_day_reads_nothing_twice_and_the_next_day_reads_again() {
    let w = World::new().await;
    w.drive_post().await;
    w.drive(vec![file("Show 05.srt", SRT_A, MODIFIED_A)]);
    let first = w.receive().await;

    assert_eq!(w.recheck_at(2 * DAY).await.read(), 1);
    assert_eq!(w.drive_head_count(), 1);

    // The worker restarts: a new recheck over the same database.
    let restarted = Recheck::new(w.db.clone(), w.sources.clone());
    for later in [2 * DAY + 10, 2 * DAY + 5 * HOUR, 3 * DAY - 2 * HOUR] {
        let report = restarted
            .run(&fixed(NOW + later), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(report.read(), 0, "{later}");
    }
    assert_eq!(w.drive_head_count(), 1);
    // Two workers at once: the claim in the database keeps one reading.
    let cancel = CancellationToken::new();
    let clock = fixed(NOW + 3 * DAY + HOUR);
    let (a, b) = tokio::join!(
        restarted.run(&clock, &cancel),
        w.recheck.run(&clock, &cancel),
    );
    assert_eq!(a.unwrap().read() + b.unwrap().read(), 1);
    assert_eq!(w.drive_head_count(), 2);
    assert_eq!(w.record(&first).await.unwrap().1, 2);
}

#[tokio::test]
async fn passes_that_start_at_different_times_do_not_claim_one_day_twice() {
    let w = World::new().await;
    w.drive_post().await;
    w.drive(vec![file("Show 05.srt", SRT_A, MODIFIED_A)]);
    let first = w.receive().await;
    let cancel = CancellationToken::new();
    let other = Recheck::new(w.db.clone(), w.sources.clone());

    // One worker's pass reads at 5 minutes past; another's, whose clock shows
    // a little earlier (it started before and was slow to claim), finds the
    // claim made.
    let late = fixed(NOW + 2 * DAY + 5 * 60 * 1000);
    assert_eq!(w.recheck.run(&late, &cancel).await.unwrap().read(), 1);
    for behind in [5 * 60 * 1000, 30 * 60 * 1000, 59 * 60 * 1000] {
        let report = other
            .run(&fixed(NOW + 2 * DAY + 5 * 60 * 1000 - behind), &cancel)
            .await
            .unwrap();
        assert_eq!(report.read(), 0, "{behind}");
    }
    assert_eq!(w.drive_head_count(), 1);
    // The reading is stamped with the time it was claimed.
    let (result_at, checked_at): (i64, i64) =
        w.db.run::<_, DbError, _>(|c| {
            Ok(c.query_row(
                "SELECT result_at, checked_at FROM subtitle_item_rechecks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(
        (result_at, checked_at),
        (NOW + 2 * DAY + 5 * 60 * 1000, result_at)
    );
    // A clock that went back by hours is another matter: the reading is made.
    let back = fixed(NOW + 2 * DAY - 3 * HOUR);
    assert_eq!(other.run(&back, &cancel).await.unwrap().read(), 1);
    assert_eq!(w.record(&first).await.unwrap().1, 2);
}

#[tokio::test]
async fn hourly_looks_read_a_received_episode_fourteen_times_in_fourteen_days() {
    let w = World::new().await;
    w.drive_post().await;
    w.drive(vec![file("Show 05.srt", SRT_A, MODIFIED_A)]);
    w.receive().await;

    let mut reads = Vec::new();
    for hour in 1..=15 * 24 {
        if w.recheck_at(hour * HOUR).await.read() == 1 {
            reads.push(hour);
        }
    }
    assert_eq!(reads.len(), 14, "{reads:?}");
    // About a day apart, the first one a day after the receipt, the last one
    // within the 14 days.
    assert!(reads.windows(2).all(|w| (23..=24).contains(&(w[1] - w[0]))));
    assert!(reads[0] <= 24 && reads[13] <= 14 * 24);
    assert_eq!(w.drive_head_count(), 14);
}

#[tokio::test]
async fn a_file_saved_again_every_day_with_the_same_bytes_makes_one_job_not_one_a_day() {
    let w = World::new().await;
    w.drive_post().await;
    // Only the modified time moves each day; the bytes never do. The sixth
    // answer is a real change of the file.
    w.drive(vec![
        file("Show 05.srt", SRT_A, MODIFIED_A),
        file("Show 05.srt", SRT_A, "Fri, 02 Oct 2026 04:15:40 GMT"),
        file("Show 05.srt", SRT_A, "Fri, 02 Oct 2026 04:15:40 GMT"),
        file("Show 05.srt", SRT_A, "Sat, 03 Oct 2026 04:15:40 GMT"),
        file("Show 05.srt", SRT_A, "Sun, 04 Oct 2026 04:15:40 GMT"),
        file("Show 05.srt", SRT_B, "Mon, 05 Oct 2026 04:15:40 GMT"),
    ]);
    w.receive().await;

    // Day 2: the modified time differs from the receipt's: a job; its receipt
    // is the same bytes, so it is recorded as replacing nothing.
    let report = w.recheck_at(2 * DAY).await;
    assert_eq!(report.jobs.len(), 1);
    w.run().await;
    let d = w.detail(&report.jobs[0]).await;
    assert!(d.items[0].unchanged_from.is_some());

    // Days 3 and 4: the modified time moves again and makes no job.
    for day in [3, 4] {
        let report = w.recheck_at(day * DAY).await;
        assert_eq!((report.same, report.jobs.len()), (1, 0), "day {day}");
    }
    assert_eq!(w.job_count().await, 2);
    // The window is the first receipt's: the same bytes did not start another.
    // A change of the size is still seen.
    let report = w.recheck_at(5 * DAY).await;
    assert_eq!((report.changed, report.jobs.len()), (1, 1));
    assert_eq!(w.job_count().await, 3);
}

#[tokio::test]
async fn a_size_that_differs_between_the_receipt_and_the_recheck_makes_one_job_not_one_a_day() {
    let w = World::new().await;
    w.drive_post().await;
    // The recheck always reads a length that is not the received bytes'.
    w.drive(vec![
        file("Show 05.srt", SRT_A, MODIFIED_A),
        file("Show 05.srt", SRT_B, MODIFIED_A),
        file("Show 05.srt", SRT_A, MODIFIED_A),
        file("Show 05.srt", SRT_B, MODIFIED_A),
    ]);
    w.receive().await;

    let report = w.recheck_at(2 * DAY).await;
    assert_eq!(report.jobs.len(), 1);
    w.run().await;
    assert!(w.detail(&report.jobs[0]).await.items[0]
        .unchanged_from
        .is_some());
    // The same length read again is what made the job, not a change.
    for day in [3, 4, 5, 6] {
        let report = w.recheck_at(day * DAY).await;
        assert_eq!((report.same, report.jobs.len()), (1, 0), "day {day}");
    }
    assert_eq!(w.job_count().await, 2);
    // The window ended 14 days after the first receipt.
    assert_eq!(w.recheck_at(15 * DAY).await.read(), 0);
}

#[tokio::test]
async fn a_tistory_attachment_deleted_and_attached_again_is_received_again() {
    let w = World::new().await;
    let post = w.server.post_url("sumomomo", 491);
    let zip = verify::zip_of(&[("Show - 05.srt", SRT_A)]);
    w.server.post(
        "sumomomo",
        491,
        vec![PostAnswer::Files(vec![spec(
            "z1",
            "Show - 05.zip",
            "0.01MB",
        )])],
    );
    w.server.file("z1", vec![FileAnswer::Bytes(zip)]);
    w.file(5, "mkv").await;
    w.observe("에루샤", "5", &post, "2026-10-02T11:00:00").await;
    let first = w.receive().await;

    // The creator deleted the attachment and attached the fixed file: a new
    // address (so a new key) with the same name.
    let fixed_zip = verify::zip_of(&[("Show - 05.srt", SRT_B)]);
    w.server.post(
        "sumomomo",
        491,
        vec![PostAnswer::Files(vec![spec(
            "z2",
            "Show - 05.zip",
            "0.01MB",
        )])],
    );
    w.server
        .file("z2", vec![FileAnswer::Bytes(fixed_zip.clone())]);
    let report = w.recheck_at(2 * DAY).await;
    assert_eq!((report.changed, report.jobs.len()), (1, 1), "{report:?}");
    assert_eq!(
        w.record(&first).await,
        Some((Some("changed".into()), 1, Some(report.jobs[0].clone())))
    );
    let made = w.detail(&report.jobs[0]).await;
    assert!(made
        .events
        .iter()
        .any(|e| e.detail.as_deref() == Some("Show - 05.zip: 다른 파일로 바뀌었어요")));

    w.run().await;
    let done = w.detail(&report.jobs[0]).await;
    assert_received(&done);
    assert_eq!(done.items[0].files[0].size, Some(fixed_zip.len() as u64));
    // Asked again the next day, the new receipt is what is read: nothing.
    assert_eq!(w.recheck_at(3 * DAY + HOUR).await.same, 1);
    assert_eq!(w.job_count().await, 2);
}

#[tokio::test]
async fn a_post_that_still_offers_its_other_files_but_not_the_missing_one_is_only_missing() {
    let w = World::new().await;
    let post = w.server.post_url("sumomomo", 491);
    let zip = verify::zip_of(&[("Show - 05.srt", SRT_A)]);
    w.server.post(
        "sumomomo",
        491,
        vec![PostAnswer::Files(vec![spec(
            "z1",
            "Show - 05.zip",
            "0.01MB",
        )])],
    );
    w.server.file("z1", vec![FileAnswer::Bytes(zip)]);
    w.file(5, "mkv").await;
    w.observe("에루샤", "5", &post, "2026-10-02T11:00:00").await;
    let first = w.receive().await;

    // The attachment is gone and the post offers nothing for the episode.
    w.server
        .post("sumomomo", 491, vec![PostAnswer::Files(Vec::new())]);
    let report = w.recheck_at(2 * DAY).await;
    assert_eq!((report.missing, report.jobs.len()), (1, 0), "{report:?}");
    assert_eq!(
        w.record(&first).await,
        Some((Some("missing".into()), 1, None))
    );
}

#[tokio::test]
async fn a_drive_file_gone_with_a_new_link_in_the_post_is_received_again() {
    let w = World::new().await;
    w.drive_post().await;
    w.drive(vec![file("Show 05.srt", SRT_A, MODIFIED_A)]);
    let first = w.receive().await;

    // The creator unshared the old file and linked a new one in the post.
    const NEW_ID: &str = "1kairan0013replaced";
    w.server.blogger_post(
        "kairan03",
        "2026/09/13.html",
        vec![PostAnswer::Page(blogger_page(&[(
            "5화",
            drive_link(NEW_ID).as_str(),
        )]))],
    );
    w.server.drive(DRIVE_ID, vec![DriveAnswer::Missing]);
    w.server
        .drive(NEW_ID, vec![file("Show 05 v2.srt", SRT_B, MODIFIED_B)]);
    let report = w.recheck_at(2 * DAY).await;
    assert_eq!((report.changed, report.jobs.len()), (1, 1), "{report:?}");
    assert_eq!(
        w.record(&first).await,
        Some((Some("changed".into()), 1, Some(report.jobs[0].clone())))
    );
    w.run().await;
    let done = w.detail(&report.jobs[0]).await;
    assert_eq!(done.row.state, JobState::Done);
    assert_eq!(
        done.items[0].files[0].sha256.as_deref(),
        Some(sha(SRT_B).as_str())
    );
    // The new link is what is read from now on, with a HEAD and no post read.
    let reads = w.server.seen().len();
    assert_eq!(w.recheck_at(3 * DAY + HOUR).await.same, 1);
    assert_eq!(w.server.seen().len(), reads + 1);
}

#[tokio::test]
async fn a_revision_with_the_same_bytes_records_that_there_is_nothing_to_replace() {
    let w = World::new().await;
    w.drive_post().await;
    // Drive's modified time moved (the creator saved the file again), the
    // bytes are the same.
    w.drive(vec![
        file("Show 05.srt", SRT_A, MODIFIED_A),
        file("Show 05.srt", SRT_A, MODIFIED_B),
    ]);
    let first = w.receive().await;

    let report = w.recheck_at(2 * DAY).await;
    assert_eq!(report.jobs.len(), 1);
    w.run().await;
    let d = w.detail(&report.jobs[0]).await;
    assert_eq!(d.row.state, JobState::Done);
    // Recorded on the item and in the log, naming the earlier receipt.
    assert_eq!(d.items[0].unchanged_from.as_deref(), Some(first.as_str()));
    let nothing = d
        .events
        .iter()
        .find(|e| e.message == "5화: 받은 파일이 지난번과 바이트가 같아 바꿀 것이 없어요")
        .expect("the log says there is nothing to replace");
    assert_eq!(nothing.detail.as_deref(), Some("파일 1개"));
    assert_eq!(d.items[0].state, ItemState::Done);
    assert_eq!(
        d.items[0].files[0].sha256,
        w.detail(&first).await.items[0].files[0].sha256
    );
    // From now on the newest receipt is the one read (against what it
    // received: the modified time of the re-saved file is not a change), and
    // the earlier one is left alone.
    let report = w.recheck_at(3 * DAY + HOUR).await;
    assert_eq!((report.same, report.read()), (1, 1));
    assert_eq!(w.record(&first).await.unwrap().1, 1);
    assert_eq!(w.record(&d.row.id).await.unwrap().1, 1);
}

#[tokio::test]
async fn the_same_change_is_one_job_even_when_the_state_of_the_check_is_lost() {
    let w = World::new().await;
    w.drive_post().await;
    // The revision's receipt fails (the file is gone when it is asked for),
    // then the file is back as changed.
    w.drive(vec![
        file("Show 05.srt", SRT_A, MODIFIED_A),
        file("Show 05.srt", SRT_B, MODIFIED_B),
        DriveAnswer::Missing,
        file("Show 05.srt", SRT_B, MODIFIED_B),
    ]);
    let first = w.receive().await;
    let report = w.recheck_at(2 * DAY).await;
    assert_eq!(report.jobs.len(), 1);
    w.run().await;
    assert_eq!(w.detail(&report.jobs[0]).await.row.state, JobState::Failed);

    // The record of the reading is lost; the same change is found again.
    w.db.run::<_, DbError, _>(|c| {
        c.execute("DELETE FROM subtitle_item_rechecks", [])?;
        Ok(())
    })
    .await
    .unwrap();
    let again = w.recheck_at(3 * DAY).await;
    assert_eq!((again.changed, again.jobs.len()), (1, 0));
    assert_eq!(w.job_count().await, 2);
    assert_eq!(
        w.record(&first).await.unwrap().2.as_deref(),
        Some(report.jobs[0].as_str())
    );
}

#[tokio::test]
async fn a_file_that_is_gone_is_recorded_makes_no_job_and_is_read_again_only_within_the_window() {
    let w = World::new().await;
    w.drive_post().await;
    w.drive(vec![
        file("Show 05.srt", SRT_A, MODIFIED_A),
        DriveAnswer::Missing,
    ]);
    let first = w.receive().await;

    let report = w.recheck_at(2 * DAY).await;
    assert_eq!(
        (report.missing, report.read(), report.jobs.len()),
        (1, 1, 0)
    );
    assert_eq!(
        w.record(&first).await,
        Some((Some("missing".into()), 1, None))
    );
    assert_eq!(w.job_count().await, 1);
    // Tried again a day later, since a file may be shared again; then no more.
    assert_eq!(w.recheck_at(3 * DAY).await.missing, 1);
    assert_eq!(w.recheck_at(15 * DAY).await.read(), 0);
    assert_eq!(w.drive_head_count(), 2);
}

#[tokio::test]
async fn a_failing_site_is_recorded_and_tried_again_the_next_day() {
    let w = World::new().await;
    w.drive_post().await;
    w.drive(vec![
        file("Show 05.srt", SRT_A, MODIFIED_A),
        DriveAnswer::Status(503),
        file("Show 05.srt", SRT_A, MODIFIED_A),
    ]);
    let first = w.receive().await;
    assert_eq!(w.recheck_at(2 * DAY).await.failed, 1);
    assert_eq!(w.record(&first).await.unwrap().0.as_deref(), Some("failed"));
    assert_eq!(w.recheck_at(2 * DAY + HOUR).await.read(), 0);
    assert_eq!(w.recheck_at(3 * DAY).await.same, 1);
    assert_eq!(w.job_count().await, 1);
}

#[tokio::test]
async fn only_the_subscribed_creators_receipts_are_read_while_the_subscription_takes_part() {
    let w = World::new().await;
    w.drive_post().await;
    w.drive(vec![file("Show 05.srt", SRT_A, MODIFIED_A)]);
    let first = w.receive().await;

    // A person's pick of the subscribed creator's other episode counts; another
    // creator's pick does not.
    let pick = |creator: &str, episode: &str, command: &str, post: String| NewJob {
        command_id: command.to_owned(),
        request: "{}".to_owned(),
        origin: "pick".to_owned(),
        work_id: Some(WORK.to_owned()),
        season: Some(1),
        anime_no: Some(ANIME),
        source_id: Some(format!("src-{creator}")),
        creator: Some(creator.to_owned()),
        revision_of: None,
        revises_attributed: false,
        items: vec![NewItem {
            observation_id: None,
            episode: episode.to_owned(),
            post_url: post,
            found_at: 3,
        }],
    };
    let blogger = |n: u32| {
        w.server
            .blogger_url("kairan03", &format!("2026/09/{n}.html"))
    };
    let mut picked = Vec::new();
    for (creator, episode, path) in [("에루샤", "6", 14), ("다른이", "7", 15)] {
        let observation = w
            .observe(creator, episode, &blogger(path), "2026-10-02T12:00:00")
            .await;
        let mut job = pick(creator, episode, &format!("pick-{episode}"), blogger(path));
        job.items[0].observation_id = Some(observation);
        let Created::Created(id) = w.jobs.create(job, NOW).await.unwrap() else {
            panic!("created");
        };
        picked.push(id);
        w.server.blogger_post(
            "kairan03",
            &format!("2026/09/{path}.html"),
            vec![PostAnswer::Page(blogger_page(&[(
                &format!("{episode}화"),
                drive_link(&format!("1drive00000ep{episode}")).as_str(),
            )]))],
        );
        w.server.drive(
            &format!("1drive00000ep{episode}"),
            vec![file(&format!("Show 0{episode}.srt"), SRT_A, MODIFIED_A)],
        );
    }
    w.run().await;
    for id in &picked {
        assert_eq!(w.detail(id).await.row.state, JobState::Done);
    }

    let report = w.recheck_at(2 * DAY).await;
    // Episode 5 (auto) and 6 (the pick of the subscribed creator).
    assert_eq!((report.same, report.read()), (2, 2));
    assert!(w.record(&first).await.is_some());
    assert!(w.record(&picked[0]).await.is_some());
    assert!(w.record(&picked[1]).await.is_none());

    // The rule paused (`영상 받기` off): nothing is read.
    let rule = w
        .channels
        .get_rule(&w.rule.id)
        .await
        .unwrap()
        .expect("the rule");
    w.channels
        .set_video_receiving(&rule.id, rule.version, false, NOW)
        .await
        .unwrap();
    let head = w.drive_head_count();
    assert_eq!(w.recheck_at(4 * DAY).await.read(), 0);
    assert_eq!(w.drive_head_count(), head);
}

#[tokio::test]
async fn an_upload_of_the_subscribed_creator_is_never_read_again() {
    let w = World::new().await;
    // Uploaded as the subscribed creator's: no observation, so no post to read.
    let made = w
        .jobs
        .create_upload(
            trss_jobs::store::NewUpload {
                id: "up1".to_owned(),
                command_id: "upload-1".to_owned(),
                request: "{}".to_owned(),
                work_id: WORK.to_owned(),
                season: 1,
                anime_no: Some(ANIME),
                source_id: None,
                creator: Some("에루샤".to_owned()),
                files: vec![trss_jobs::store::UploadedFile {
                    id: "f1".to_owned(),
                    file_key: "Show 05.srt".to_owned(),
                    name: "Show 05.srt".to_owned(),
                    path: "up1/Show 05.srt".to_owned(),
                    size: SRT_A.len() as u64,
                    sha256: sha(SRT_A),
                    object: "Show 05.srt".to_owned(),
                    format: verify::Format::Srt,
                    kind: trss_subtitles::upload::Kind::Subtitle,
                    archive: None,
                }],
                dropped: Vec::new(),
            },
            NOW,
        )
        .await
        .unwrap();
    assert!(matches!(made, Created::Created(_)));
    w.db.run::<_, DbError, _>(|c| {
        c.execute(
            "INSERT OR IGNORE INTO subtitle_sources (id, anime_no, creator_name, created_at)
             VALUES ('src-에루샤', ?1, '에루샤', 1)",
            [ANIME],
        )?;
        c.execute(
            "UPDATE subtitle_jobs SET source_id = 'src-에루샤' WHERE id = 'up1'",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();

    for day in 1..=3 {
        let report = w.recheck_at(day * DAY).await;
        assert_eq!(report.read(), 0);
        assert!(report.jobs.is_empty());
    }
    assert!(w.server.seen().is_empty());
    assert_eq!(w.job_count().await, 1);
}

#[tokio::test]
async fn a_receipt_that_revised_a_file_the_user_named_is_read_again_like_any_other() {
    let w = World::new().await;
    // The user's own subtitle of 5화, named as the subscribed creator's before
    // the creator's line was first seen.
    w.file(5, "ass").await;
    w.db.run::<_, DbError, _>(|c| {
        c.execute(
            "INSERT OR IGNORE INTO subtitle_sources (id, anime_no, creator_name, created_at)
             VALUES ('src-에루샤', ?1, '에루샤', 1)",
            [ANIME],
        )?;
        c.execute(
            "UPDATE media_files SET creator_source_id = 'src-에루샤', creator_set_at = 1,
                    creator_version = creator_version + 1
              WHERE kind = 'subtitle'",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let observed = w.drive_post().await;
    w.drive(vec![
        file("Show 05.srt", SRT_A, MODIFIED_A),
        file("Show 05.srt", SRT_B, MODIFIED_B),
    ]);
    let first = w.receive().await;
    assert!(w.detail(&first).await.row.revises_attributed);

    // The Drive file changes: a revision of that receipt, as for any other.
    let report = w.recheck_at(3 * DAY).await;
    assert_eq!((report.changed, report.jobs.len()), (1, 1));
    let d = w.detail(&report.jobs[0]).await;
    assert_eq!(d.row.revision_of, Some(observed));
    assert_eq!(d.row.revises_job.as_deref(), Some(first.as_str()));
    assert!(!d.row.revises_attributed);
}

/// Reads the real posts of 2026-10-03 and compares what they say now with
/// what was received then. Run by hand:
/// `cargo test -p trss-jobs --test recheck -- --ignored --nocapture`.
///
/// - Drive (kairan03.blogspot.com/2026/09/13.html): the file's `HEAD` gave
///   `Content-Length: 39453` and `Last-Modified: Tue, 29 Sep 2026 19:53:19 GMT`.
/// - Tistory (sumomomo.tistory.com/491): a range request for one byte of the
///   attachment gave `Content-Range: bytes 0-0/11724` and no `Last-Modified`
///   or `ETag`; a `HEAD` is refused (404).
/// - Naver (blog.naver.com/elainalove1017/224324105274): `aPostFiles` gave
///   `attachFileSize` `106,408` for the ZIP and `44,480` for the ASS.
#[tokio::test]
#[ignore = "reaches the real Blogger, Google Drive, Tistory and Naver"]
async fn real_posts_are_read_again_and_agree_with_what_was_received() {
    use trss_subtitles::{
        blogger::BloggerSource, drive::Drive, naver::NaverSource, tistory::TistorySource, Opened,
        Sources,
    };
    use url::Url;

    let drive = Drive::new();
    let sources = Sources::none()
        .with_tistory(TistorySource::new(drive.clone()))
        .with_blogger(BloggerSource::new(drive.clone()))
        .with_naver(NaverSource::new(drive));

    // For each post: the episode to open it for, and what each file of it was.
    type Expected = Vec<(&'static str, Option<u64>, Option<&'static str>)>;
    let posts: [(&str, &str, Expected); 3] = [
        (
            "https://kairan03.blogspot.com/2026/09/13.html",
            "13",
            vec![("", Some(39_453), Some("Tue, 29 Sep 2026 19:53:19 GMT"))],
        ),
        (
            "https://sumomomo.tistory.com/491",
            "1",
            vec![("", Some(11_724), None)],
        ),
        (
            "https://blog.naver.com/elainalove1017/224324105274",
            "8",
            vec![("1~8화", Some(106_408), None), ("- 08", Some(44_480), None)],
        ),
    ];
    for (post, episode, expected) in posts {
        let post = Url::parse(post).unwrap();
        let source = sources.for_post(&post).expect("a source reads the post");
        let Opened::Files(offered) = source.open(&post, episode).await.unwrap() else {
            panic!("{post}: files");
        };
        let keys: Vec<String> = offered.iter().map(|f| f.key.clone()).collect();
        let answers = source.recheck(&post, &keys).await;
        for (key, info) in &answers {
            println!("{key}: {info:?}");
        }
        for (part, size, modified) in expected {
            let (key, info) = answers
                .iter()
                .find(|(key, _)| key.contains(part))
                .unwrap_or_else(|| panic!("{post}: a file with {part:?} in {keys:?}"));
            let info = info.as_ref().unwrap_or_else(|f| panic!("{key}: {f}"));
            assert_eq!(info.size, size, "{key}");
            assert_eq!(info.last_modified.as_deref(), modified, "{key}");
        }
        // What a receipt gets of each file agrees with what the recheck read,
        // or every recheck would find a difference: the bytes' length is the
        // size and the receipt's `Last-Modified` is the recheck's string.
        for file in &offered {
            let Some((_, Ok(info))) = answers.iter().find(|(key, _)| *key == file.key) else {
                continue;
            };
            let mut fetch = source.fetch(&post, file).await.unwrap();
            let mut length = 0u64;
            while let Some(chunk) = fetch.chunk().await.unwrap() {
                length += chunk.len() as u64;
            }
            let modified = fetch
                .snapshot
                .entries()
                .iter()
                .find(|(name, _)| name == trss_subtitles::http::LAST_MODIFIED)
                .map(|(_, value)| value.clone());
            println!("{}: received {length} bytes, {modified:?}", file.key);
            assert_eq!(Some(length), info.size, "{}", file.key);
            assert_eq!(modified, info.last_modified, "{}", file.key);
        }
    }
}

#[tokio::test]
async fn an_episode_the_user_does_not_receive_is_not_read_again() {
    let w = World::new().await;
    w.drive_post().await;
    w.drive(vec![file("Show 05.srt", SRT_A, MODIFIED_A)]);
    let first = w.receive().await;

    // The user says the creator's 5화 is not received: inside the window, where the
    // recheck would read it (see `a_receipt_fifteen_days_old_is_not_read`), nothing is.
    let version = w
        .follow
        .mappings(WORK, 1)
        .await
        .unwrap()
        .get("src-에루샤")
        .map_or(0, |m| m.version);
    let saved = w
        .follow
        .set_user_mapping(
            WORK,
            1,
            "src-에루샤",
            version,
            trss_jobs::mapping::UserMapping::new(0, vec![("05".to_owned(), None)]).unwrap(),
            NOW,
        )
        .await
        .unwrap();
    assert!(matches!(saved, trss_jobs::mapping::Saved::Done(Some(_))));
    let report = w.recheck_at(14 * DAY - HOUR).await;
    assert_eq!(report.read(), 0);
    assert_eq!(w.drive_head_count(), 0);
    assert_eq!(w.record(&first).await, None);
}

// erulabo (ticket 0050): the files come only through the site's check, so the
// post's `dateModified` is what the recheck compares, and the Drive file of
// the receipt is only observed.

/// The Drive file the creator's 5화 download of erulabo came from, as the
/// receipt's snapshot kept its ID. It is the key to the file: it is in the
/// receipt's snapshot and nowhere else.
const ERULABO_DRIVE_ID: &str = "1erulaboepisode05";
const POST_A: &str = "2026-10-01T09:00:00+09:00";
const POST_B: &str = "2026-10-02T21:30:00+09:00";
const ERULABO_POST: u32 = 859;
const ERULABO_FILE: &str = "Show 05.srt";

/// A browser that brings every post to its check and never gets a file: the
/// job waits for a person (`인증 필요`).
struct CheckOnly;

impl AuthBrowser for CheckOnly {
    fn prepare<'a>(
        &'a self,
        _request: PrepareRequest<'a>,
    ) -> BoxFuture<'a, Result<Prepared, Failure>> {
        Box::pin(async {
            Ok(Prepared {
                run_id: "run-1".to_owned(),
                target_id: "target-1".to_owned(),
            })
        })
    }

    fn wait_file<'a>(
        &'a self,
        _job: &'a str,
        _run_id: &'a str,
        _staging: &'a std::path::Path,
    ) -> BoxFuture<'a, Waited> {
        Box::pin(async { Waited::Ended })
    }

    fn is_live(&self, _job: &str, _run_id: &str) -> bool {
        true
    }

    fn touch(&self, _job: &str, _run_id: &str) -> bool {
        true
    }

    fn release<'a>(&'a self, _job: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

impl World {
    /// The creator's 5화 post on erulabo, observed, with a video in the
    /// library. `modified` is the post's `dateModified` for each reading of
    /// it, one after another, the last one again: the receipt's, the recheck's,
    /// the next receipt's.
    async fn erulabo_post(&self, modified: &[&str]) -> i64 {
        self.server.erulabo_post(
            ERULABO_POST,
            modified
                .iter()
                .map(|m| {
                    PostAnswer::Page(erulabo_page(
                        &[
                            ("/file/aaaa-5", "에루샤 (5)"),
                            ("/file/bbbb-6", "에루샤 (6)"),
                        ],
                        m,
                    ))
                })
                .collect(),
        );
        self.file(5, "mkv").await;
        self.observe(
            "에루샤",
            "5",
            &self.server.erulabo_url(ERULABO_POST),
            "2026-10-02T11:00:00",
        )
        .await
    }

    /// What a person's passed check left in the item's folder: the file the
    /// browser downloaded, and what its answer said (the Drive file it came
    /// from).
    async fn pass_check(&self, job: &str, bytes: &[u8], modified: &str) {
        let item = self.detail(job).await.items[0].id;
        let dir =
            ReceiveArea::in_app_data(self._dir.path()).at(&format!(".tmp/check-{job}-{item}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(ERULABO_FILE), bytes).unwrap();
        auth::record_answer(
            &dir,
            &Answered {
                status: Some(200),
                content_type: Some("application/octet-stream".to_owned()),
                content_length: Some(bytes.len() as u64),
                last_modified: Some(modified.to_owned()),
                drive_id: Some(ERULABO_DRIVE_ID.to_owned()),
            },
        )
        .await
        .unwrap();
    }

    /// The subscribed creator's job for what was observed, run to the check,
    /// the check passed (the file of `bytes`), and received.
    async fn receive_through_the_check(&self, bytes: &[u8], modified: &str) -> String {
        let made = self.follow.evaluate(NOW).await.unwrap();
        assert_eq!(made.len(), 1, "one job for the observation");
        self.pass_check(&made[0], bytes, modified).await;
        self.run().await;
        let d = self.detail(&made[0]).await;
        assert_received(&d);
        made[0].clone()
    }

    /// The recheck record of the first item of job `job`: (result, checks,
    /// the job it made, what it observed).
    async fn erulabo_record(&self, job: &str) -> (String, i64, Option<String>, serde_json::Value) {
        let job = job.to_owned();
        self.db
            .run::<_, DbError, _>(move |c| {
                Ok(c.query_row(
                    "SELECT r.result, r.checks, r.job_id, r.observed FROM subtitle_item_rechecks r
                       JOIN subtitle_job_items i ON i.id = r.item_id WHERE i.job_id = ?1",
                    [job],
                    |r| {
                        let observed: String = r.get(3)?;
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            serde_json::from_str(&observed).unwrap(),
                        ))
                    },
                )?)
            })
            .await
            .unwrap()
    }

    /// Where `needle` is in the database, as `table.column`.
    async fn where_is(&self, needle: &str) -> Vec<String> {
        let needle = needle.to_owned();
        self.db
            .run::<_, DbError, _>(move |c| {
                let tables: Vec<String> = c
                    .prepare(
                        "SELECT name FROM sqlite_master
                          WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
                    )?
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                let mut found = Vec::new();
                for table in tables {
                    let columns: Vec<String> = c
                        .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?
                        .query_map([], |r| r.get(0))?
                        .collect::<rusqlite::Result<_>>()?;
                    for column in columns {
                        let hits: i64 = c.query_row(
                            &format!(
                                "SELECT count(*) FROM \"{table}\"
                                  WHERE instr(CAST(\"{column}\" AS TEXT), ?1) > 0"
                            ),
                            [&needle],
                            |r| r.get(0),
                        )?;
                        if hits > 0 {
                            found.push(format!("{table}.{column}"));
                        }
                    }
                }
                Ok(found)
            })
            .await
            .unwrap()
    }

    /// How many times erulabo's post was read.
    fn erulabo_reads(&self) -> usize {
        self.server
            .seen()
            .iter()
            .filter(|s| s.host == "erulabo.com")
            .count()
    }
}

#[tokio::test]
async fn an_erulabo_post_whose_modified_time_changed_raises_a_check_to_do_and_receives_nothing() {
    let w = World::new().await;
    let first_observed = w.erulabo_post(&[POST_A, POST_B]).await;
    // The Drive file at the recheck: the same as received.
    w.server.drive(
        ERULABO_DRIVE_ID,
        vec![file(ERULABO_FILE, SRT_A, MODIFIED_A)],
    );
    let first = w.receive_through_the_check(SRT_A, MODIFIED_A).await;
    // What the receipt kept of the file.
    let kept = w.detail(&first).await.items[0].files[0].clone();
    assert_eq!(kept.file_key, "browser:erulabo.com/859#Show 05.srt");
    w.file(5, "ass").await;
    let library = w.library().await;
    let reads_before = w.erulabo_reads();

    // Three days later the post says it was fixed.
    let report = w.recheck_at(3 * DAY).await;
    assert_eq!((report.changed, report.read()), (1, 1));
    assert_eq!(report.jobs.len(), 1);
    // The post was read once, by HTTP.
    assert_eq!(w.erulabo_reads(), reads_before + 1);
    let d = w.detail(&report.jobs[0]).await;
    assert_eq!(d.row.origin, AUTO);
    assert_eq!(d.row.state, JobState::Pending);
    assert_eq!(d.row.revision_of, Some(first_observed));
    assert_eq!(d.row.revises_job.as_deref(), Some(first.as_str()));
    assert_eq!(d.row.creator.as_deref(), Some("에루샤"));
    assert_eq!(d.items[0].observation_id, Some(first_observed));
    assert_eq!(d.items[0].post_url, w.server.erulabo_url(ERULABO_POST));
    assert!(d.events.iter().all(|e| e.at == NOW + 3 * DAY));
    assert_eq!(
        d.events[0].message,
        "게시물의 수정 시각이 받은 때와 달라서 다시 받아요"
    );
    assert_eq!(
        d.events[0].detail.as_deref(),
        Some(format!("{POST_A} → {POST_B} · 사이트 확인을 거쳐야 받을 수 있어요").as_str())
    );
    let (result, checks, made, _) = w.erulabo_record(&first).await;
    assert_eq!(
        (result.as_str(), checks, made.as_deref()),
        ("changed", 1, Some(report.jobs[0].as_str()))
    );
    // Nothing was received, and the library is as it was.
    assert!(d.items[0].files.is_empty());
    assert_eq!(w.library().await, library);
    assert_eq!(w.job_count().await, 2);

    // The job opens the post and stops at the site's check: a to-do for a
    // person (`인증 필요`), which the person passes on the remote screen.
    let runner = Runner::new(
        w.jobs.clone(),
        w.sources.clone(),
        ReceiveArea::in_app_data(w._dir.path()),
        ticking_clock(),
    )
    .with_auth(Arc::new(CheckOnly));
    runner.run_ready(&CancellationToken::new()).await.unwrap();
    let d = w.detail(&report.jobs[0]).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Auth))
    );
    assert_eq!(
        (d.items[0].state, d.items[0].wait),
        (ItemState::Waiting, Some(Wait::Auth))
    );
    assert!(d.items[0].files.is_empty());
    let waits = w.jobs.auth_waits().await.unwrap();
    assert_eq!(
        waits.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(),
        [report.jobs[0].as_str()]
    );
    assert_eq!(w.library().await, library);

    // While that to-do waits the episode is not read again, so no second
    // to-do comes of the same post.
    let reads = w.erulabo_reads();
    let report = w.recheck_at(4 * DAY).await;
    assert_eq!(report.read(), 0);
    assert_eq!(w.erulabo_reads(), reads);
    assert_eq!(w.job_count().await, 2);
    // The revision job's request, its events and its wait at the check hold
    // no Drive ID either.
    assert_eq!(
        w.where_is(ERULABO_DRIVE_ID).await,
        ["subtitle_job_files.snapshot"]
    );
}

#[tokio::test]
async fn a_post_edited_in_its_text_only_is_received_once_more_through_the_check_and_then_agrees() {
    let w = World::new().await;
    w.erulabo_post(&[POST_A, POST_B]).await;
    w.server.drive(
        ERULABO_DRIVE_ID,
        vec![file(ERULABO_FILE, SRT_A, MODIFIED_A)],
    );
    let first = w.receive_through_the_check(SRT_A, MODIFIED_A).await;
    w.file(5, "ass").await;

    let report = w.recheck_at(3 * DAY).await;
    assert_eq!(report.jobs.len(), 1);
    // The person passes the check; the file is the same bytes.
    w.pass_check(&report.jobs[0], SRT_A, MODIFIED_A).await;
    w.run().await;
    let d = w.detail(&report.jobs[0]).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.events);
    assert_eq!(d.items[0].unchanged_from.as_deref(), Some(first.as_str()));

    // That receipt has the post's new time, so the next reading finds the post
    // as it is and makes nothing; the window is the first receipt's.
    let report = w.recheck_at(4 * DAY).await;
    assert_eq!((report.same, report.read()), (1, 1));
    assert!(report.jobs.is_empty());
    assert_eq!(w.job_count().await, 2);
    assert_eq!(w.recheck_at(14 * DAY + HOUR).await.read(), 0);
    // Two receipts through the check and a revision job: the ID is still only
    // in the receipts' snapshots.
    assert_eq!(
        w.where_is(ERULABO_DRIVE_ID).await,
        ["subtitle_job_files.snapshot"]
    );
}

#[tokio::test]
async fn a_changed_drive_file_with_the_post_as_it_was_is_only_recorded_and_the_id_is_nowhere_else()
{
    let w = World::new().await;
    w.erulabo_post(&[POST_A]).await;
    // The Drive file at the first look, and as it is on the next day.
    w.server.drive(
        ERULABO_DRIVE_ID,
        vec![
            file(ERULABO_FILE, SRT_B, MODIFIED_B),
            file(ERULABO_FILE, SRT_A, MODIFIED_A),
        ],
    );
    let first = w.receive_through_the_check(SRT_A, MODIFIED_A).await;
    w.file(5, "ass").await;
    let library = w.library().await;
    let jobs = w.job_count().await;
    assert_eq!(w.drive_head_count(), 0);

    // The first look: the file is longer than it was received, the post is not
    // fixed. Only the observation is recorded.
    let report = w.recheck_at(3 * DAY).await;
    assert_eq!((report.same, report.changed, report.read()), (1, 0, 1));
    assert!(report.jobs.is_empty());
    assert_eq!(w.drive_head_count(), 1);
    let (result, checks, made, observed) = w.erulabo_record(&first).await;
    assert_eq!((result.as_str(), checks, made), ("same", 1, None));
    assert_eq!(
        observed,
        serde_json::json!([
            {"post": true, "modified": POST_A, "was": POST_A},
            {
                "key": "browser:erulabo.com/859#Show 05.srt",
                "size": SRT_B.len(),
                "last_modified": MODIFIED_B,
            },
        ])
    );
    // No to-do, no job, nothing received.
    assert_eq!(w.job_count().await, jobs);
    assert!(w.jobs.auth_waits().await.unwrap().is_empty());
    assert_eq!(w.library().await, library);

    // The next day's observation replaces it.
    let report = w.recheck_at(4 * DAY).await;
    assert_eq!((report.same, report.jobs.len()), (1, 0));
    let (result, checks, _, observed) = w.erulabo_record(&first).await;
    assert_eq!((result.as_str(), checks), ("same", 2));
    assert_eq!(observed[1]["size"], SRT_A.len());
    assert_eq!(observed[1]["last_modified"], MODIFIED_A);
    assert_eq!(w.job_count().await, jobs);

    // Drive was asked with a `HEAD`, no cookie and no `Referer`, and the ID is
    // in the receipt's snapshot and nowhere else in the database: not in the
    // record, the jobs' requests, their events or their errors. What is
    // printed and what the web API shows are seen by reading the code.
    let seen = w.server.seen();
    let heads: Vec<_> = seen.iter().filter(|s| s.method == "HEAD").collect();
    assert_eq!(heads.len(), 2);
    assert!(heads.iter().all(|s| !s.cookie && !s.referer));
    assert_eq!(
        w.where_is(ERULABO_DRIVE_ID).await,
        ["subtitle_job_files.snapshot"]
    );
}

#[tokio::test]
async fn an_erulabo_post_that_cannot_be_compared_or_read_is_recorded_and_makes_nothing() {
    let w = World::new().await;
    w.erulabo_post(&[POST_A]).await;
    w.server.drive(
        ERULABO_DRIVE_ID,
        vec![file(ERULABO_FILE, SRT_A, MODIFIED_A)],
    );
    let first = w.receive_through_the_check(SRT_A, MODIFIED_A).await;
    w.file(5, "ass").await;
    let jobs = w.job_count().await;

    // The Drive file is gone and the post is as it was: the observation says
    // so and the post's verdict stands.
    w.server.drive(ERULABO_DRIVE_ID, vec![DriveAnswer::Missing]);
    let report = w.recheck_at(2 * DAY).await;
    assert_eq!((report.same, report.read()), (1, 1));
    let (result, _, _, observed) = w.erulabo_record(&first).await;
    assert_eq!(result, "same");
    assert_eq!(observed[1]["problem"], "missing");

    // A post that is gone, one that cannot be reached, a page that is not the
    // post, and a post that does not say when it was modified (nothing to
    // compare).
    for (answer, expected) in [
        (PostAnswer::Status(404), "missing"),
        (PostAnswer::Status(503), "failed"),
        (
            PostAnswer::Page("<html><body>점검 중</body></html>".into()),
            "unreadable",
        ),
        (
            PostAnswer::Page(r#"<html><body><div id="post-body"></div></body></html>"#.into()),
            "unreadable",
        ),
    ] {
        w.server.erulabo_post(ERULABO_POST, vec![answer]);
        let day = 2 + w.erulabo_record(&first).await.1;
        let report = w.recheck_at(day * DAY).await;
        assert_eq!(report.read(), 1, "{expected}");
        let (result, _, made, _) = w.erulabo_record(&first).await;
        assert_eq!((result.as_str(), made), (expected, None));
        assert!(report.jobs.is_empty());
    }
    assert_eq!(w.job_count().await, jobs);
    assert_eq!(
        w.where_is(ERULABO_DRIVE_ID).await,
        ["subtitle_job_files.snapshot"]
    );
}
