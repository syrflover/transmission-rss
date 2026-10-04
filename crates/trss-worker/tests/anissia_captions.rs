//! Subtitle candidates end to end (ticket 0035): the real web API (the link of
//! a season, the candidates, the `새로고침` command), the real worker carrying
//! out the `anissia_captions` command, the observer, and a fake Anissia whose
//! answers are the shapes read from the real one on 2026-10-02. Nothing here
//! reaches the real Anissia.

mod common;

use std::{
    collections::BTreeSet,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use axum::{http::StatusCode, Router};
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use trss_anissia::{fake::Fake, Anissia};
use trss_collect::{anissia::captions::CaptionObserver, store::anissia::AnissiaStore};
use trss_library::discovery::{Scan, ScannedWork, WorkRead};
use trss_web::AppState;
use trss_worker::{CommandsOutcome, Worker};

struct Env {
    h: Harness,
    fake: Fake,
    router: Router,
    observer: CaptionObserver,
    work: String,
}

impl Env {
    async fn new() -> Env {
        let h = Harness::new().await;
        let fake = Fake::start().await;
        let clock = {
            let now = h.clock.clone();
            Arc::new(move || now.load(Ordering::SeqCst)) as trss_core::Clock
        };
        let anissia = Anissia::new(h.db.clone(), fake.config(), clock).with_spacing(Duration::ZERO);
        let state = AppState::new(h.db.clone()).with_anissia(anissia.clone());
        let scan = Scan {
            works: vec![WorkRead::Read(ScannedWork {
                dir_name: "Sayonara Lara".into(),
                seasons: BTreeSet::from([1]),
                files: Vec::new(),
                unrecognized: Vec::new(),
            })],
        };
        // The work's folder, where what its jobs receive is stored; its video
        // keeps season 1 when the worker's watcher reads the folder.
        let root = h.dir.path().join("c");
        std::fs::create_dir_all(root.join("Sayonara Lara/Season 01")).unwrap();
        std::fs::write(
            root.join("Sayonara Lara/Season 01/Sayonara Lara S01E01.mkv"),
            b"v",
        )
        .unwrap();
        let (folder, _) = state
            .library
            .add_folder(root.to_string_lossy().into_owned(), scan, 1, &[])
            .await
            .unwrap();
        let work = state.library.works(&folder.id).await.unwrap().remove(0).id;
        let observer = CaptionObserver::new(anissia, AnissiaStore::new(h.db.clone()));
        Env {
            router: Router::new().nest("/api", trss_web::api::router().with_state(state)),
            h,
            fake,
            observer,
            work,
        }
    }

    fn worker(&self) -> Worker {
        self.h.worker().with_captions(self.observer.clone())
    }

    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let mut request = axum::http::Request::builder().method(method).uri(uri);
        let body = match body {
            Some(json) => {
                request = request.header("content-type", "application/json");
                axum::body::Body::from(json.to_string())
            }
            None => axum::body::Body::empty(),
        };
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn season_path(&self, tail: &str) -> String {
        format!("/api/library/works/{}/seasons/1/anissia{tail}", self.work)
    }

    async fn link(&self) {
        self.fake.set_week(
            3,
            vec![self.fake.entry(3, 3492, "22:30", "작품", "Original")],
        );
        let (status, body) = self
            .call(
                "POST",
                &self.season_path("/link"),
                Some(json!({ "version": 0, "anime_no": 3492, "week": 3 })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    async fn candidates(&self) -> Value {
        let (status, body) = self
            .call("GET", &self.season_path("/candidates"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn run_commands(&self, worker: &Worker) -> CommandsOutcome {
        worker
            .run_commands(&CancellationToken::new())
            .await
            .unwrap()
    }
}

fn creators(shown: &Value) -> Vec<(String, String)> {
    let mut seen: Vec<(String, String)> = shown["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["creator"].as_str().unwrap().to_owned(),
                c["episode"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    seen.sort();
    seen
}

#[tokio::test]
async fn linking_a_season_reads_the_anime_in_the_worker_and_the_older_lines_become_candidates() {
    let env = Env::new().await;
    // The recent list has one creator's line, observed before any link.
    env.fake.set_recent(vec![env.fake.recent_line(
        3492,
        "12",
        "2026-09-10T12:10:00",
        "https://erulabo.com/837",
        "에루샤",
    )]);
    env.observer.run_due().await.unwrap();
    // Anissia's own list for the anime also holds a line older than the recent
    // list reaches.
    env.fake.set_captions(
        3492,
        vec![
            json!({"episode": "12", "updDt": "2026-09-10T12:10:00",
                   "website": "https://erulabo.com/837", "name": "에루샤"}),
            json!({"episode": "24", "updDt": "2026-03-01T00:00:00",
                   "website": "https://felia.tistory.com/1", "name": "코코렛"}),
        ],
    );

    env.link().await;
    // Linked: the observation of the recent list shows at once, the older line
    // not before the worker has read the anime.
    let before = env.candidates().await;
    assert_eq!(creators(&before), [("에루샤".to_owned(), "12".to_owned())]);
    assert_eq!(before["refresh"]["state"], "pending");
    assert_eq!(env.fake.count("/anime/caption/animeNo/3492"), 0);

    let worker = env.worker();
    assert_eq!(env.run_commands(&worker).await, CommandsOutcome::Ran(1));

    let after = env.candidates().await;
    assert_eq!(after["refresh"]["state"], "done");
    assert_eq!(after["refresh"]["outcome"]["result"], "read");
    assert_eq!(
        creators(&after),
        [
            ("에루샤".to_owned(), "12".to_owned()),
            ("코코렛".to_owned(), "24".to_owned())
        ]
    );
    assert_eq!(env.fake.count("/anime/caption/animeNo/3492"), 1);
    // The line the recent list gave is not observed twice.
    assert_eq!(after["candidates"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn refresh_reads_the_anime_again_and_a_failed_read_says_so_and_keeps_the_candidates() {
    let env = Env::new().await;
    env.fake.set_captions(
        3492,
        vec![json!({"episode": "3", "updDt": "2026-10-02 21:00:00",
                    "website": "https://a.test/3", "name": "에루샤"})],
    );
    env.link().await;
    let worker = env.worker();
    assert_eq!(env.run_commands(&worker).await, CommandsOutcome::Ran(1));
    assert_eq!(
        env.candidates().await["candidates"][0]["updated_at"],
        1_790_942_400_000_i64
    );

    // The user's 새로고침 sees the creator move on to episode 4.
    env.fake.set_captions(
        3492,
        vec![json!({"episode": "4", "updDt": "2026-10-09T21:00:00",
                    "website": "https://a.test/4", "name": "에루샤"})],
    );
    let (status, body) = env
        .call(
            "POST",
            "/api/commands",
            Some(json!({ "id": "refresh-0001", "kind": "anissia_captions",
                         "payload": { "anime_no": 3492 } })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(env.run_commands(&worker).await, CommandsOutcome::Ran(1));
    assert_eq!(
        creators(&env.candidates().await),
        [
            ("에루샤".to_owned(), "3".to_owned()),
            ("에루샤".to_owned(), "4".to_owned())
        ]
    );

    // Anissia fails: the command says so, and the candidates stay as they were.
    env.fake.state.lock().unwrap().failing = 1;
    let (status, _) = env
        .call(
            "POST",
            "/api/commands",
            Some(json!({ "id": "refresh-0002", "kind": "anissia_captions",
                         "payload": { "anime_no": 3492 } })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(env.run_commands(&worker).await, CommandsOutcome::Ran(1));
    let shown = env.candidates().await;
    assert_eq!(shown["refresh"]["state"], "failed");
    assert!(shown["refresh"]["outcome"]["reason"]
        .as_str()
        .unwrap()
        .contains("읽지 못했어요"));
    assert_eq!(shown["candidates"].as_array().unwrap().len(), 2);

    // Anissia asks to wait: also a failed command with a sentence of its own.
    {
        let mut state = env.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = Some(120);
    }
    env.call(
        "POST",
        "/api/commands",
        Some(json!({ "id": "refresh-0003", "kind": "anissia_captions",
                     "payload": { "anime_no": 3492 } })),
    )
    .await;
    assert_eq!(env.run_commands(&worker).await, CommandsOutcome::Ran(1));
    let shown = env.candidates().await;
    assert_eq!(shown["refresh"]["state"], "failed");
    assert!(shown["refresh"]["outcome"]["reason"]
        .as_str()
        .unwrap()
        .contains("잠시 요청을 받지 않아요"));
    assert_eq!(shown["candidates"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn a_worker_that_does_not_read_anissia_fails_the_command_instead_of_holding_it() {
    let env = Env::new().await;
    env.link().await;
    let plain = env.h.worker();
    assert_eq!(env.run_commands(&plain).await, CommandsOutcome::Ran(1));
    let shown = env.candidates().await;
    assert_eq!(shown["refresh"]["state"], "failed");
    assert_eq!(env.fake.count("/anime/caption/animeNo/"), 0);
}

impl Env {
    /// Season 1's AniList entry, of 12 episodes.
    async fn season_entry(&self) {
        let work = self.work.clone();
        self.h
            .db
            .run::<_, trss_core::DbError, _>(move |c| {
                // Episode 1 aired an hour before the creator's line (`updDt`
                // 2026-10-02T11:00:00), then one a week.
                let first =
                    trss_anissia::observe::updated_at("2026-10-02T11:00:00").unwrap() / 1000 - 3600;
                let airing = (1..=12)
                    .map(|k| format!(r#"{{"episode":{k},"at":{}}}"#, first + (k - 1) * 7 * 86_400))
                    .collect::<Vec<_>>()
                    .join(",");
                c.execute(
                    "INSERT INTO anilist_entries (id, format, episodes, airing, fetched_at)
                     VALUES (1, 'TV', 12, ?1, 1)",
                    [format!("[{airing}]")],
                )?;
                c.execute(
                    "INSERT INTO season_entries (work_id, season, position, anilist_id)
                     VALUES (?1, 1, 0, 1)",
                    [work],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// A collecting subscription of season 1 that follows 에루샤.
    async fn follow(&self) {
        use trss_collect::store::channels::{
            ChannelInput, NewSubscription, RuleInput, SubtitleMode,
        };

        let channel = self
            .h
            .channels
            .create_channel(ChannelInput::new("https://feed.test/follow"))
            .await
            .unwrap();
        let rule = self
            .h
            .channels
            .create_subscription_rule(
                &channel.id,
                RuleInput {
                    r#match: Some("Sayonara Lara".into()),
                    directory: "Sayonara Lara".into(),
                    episode: 0,
                    ..RuleInput::default()
                },
                NewSubscription {
                    anime: trss_anissia::Anime {
                        anime_no: 3492,
                        subject: "작품".into(),
                        original_subject: None,
                        week: 3,
                        air_time: None,
                        start_date: None,
                        end_date: None,
                        status: "ON".into(),
                        fetched_at: 1,
                    },
                    subtitles: SubtitleMode::Follow,
                    creator: Some("에루샤".into()),
                    subscribed_at: 1,
                },
            )
            .await
            .unwrap();
        self.h
            .channels
            .link_season(&rule.id, &format!("{}:1", self.work))
            .await
            .unwrap();
    }

    /// A worker that runs the jobs, with the fake source, and polls for
    /// nothing in a test's time.
    fn job_worker(&self) -> (Worker, trss_jobs::JobStore) {
        use trss_jobs::{area::ReceiveArea, JobStore, Runner};
        use trss_subtitles::{fake::FakeSource, Sources};

        let jobs = JobStore::new(self.h.db.clone());
        let runner = Runner::new(
            jobs.clone(),
            Sources::none().with_fake(FakeSource),
            ReceiveArea::in_app_data(self.h.dir.path()),
            Arc::new(|| 2_000),
        );
        let worker = self
            .worker()
            .with_jobs(runner)
            .with_command_poll(Duration::from_secs(3600));
        (worker, jobs)
    }
}

/// Whether the subscribed creator's job is done within a few seconds.
async fn auto_job_done(jobs: &trss_jobs::JobStore) -> bool {
    let done = || async {
        let page = jobs.done_page(None, 10).await.unwrap();
        page.items.len() == 1 && page.items[0].origin == trss_jobs::AUTO
    };
    for _ in 0..200 {
        if done().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

#[tokio::test]
async fn a_reading_that_finds_the_subscribed_creators_new_episode_makes_and_runs_its_job() {
    let env = Env::new().await;
    env.season_entry().await;
    env.follow().await;
    let (worker, jobs) = env.job_worker();
    let cancel = CancellationToken::new();
    let running = tokio::spawn({
        let (worker, cancel) = (worker.clone(), cancel.clone());
        async move { worker.run(cancel).await }
    });
    // The worker's first look at its start found nothing.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // The reading of the recent list finds the creator's episode 1: the worker
    // makes its job and runs it without waiting for its next poll.
    env.fake.set_recent(vec![env.fake.recent_line(
        3492,
        "1",
        "2026-10-02T11:00:00",
        "https://fake.trss.invalid/ok/lara1",
        "에루샤",
    )]);
    env.observer.run_due().await.unwrap();
    assert!(
        auto_job_done(&jobs).await,
        "the subscribed creator's episode was received"
    );

    cancel.cancel();
    running.await.unwrap();
}

#[tokio::test]
async fn a_season_entry_stored_later_has_the_worker_look_at_the_subscribed_creator_again() {
    let env = Env::new().await;
    env.follow().await;
    // The creator's episode 1 was observed while the season had no AniList
    // entry: how its episodes map to the season is not decided.
    env.fake.set_recent(vec![env.fake.recent_line(
        3492,
        "1",
        "2026-10-02T11:00:00",
        "https://fake.trss.invalid/ok/lara1",
        "에루샤",
    )]);
    env.observer.run_due().await.unwrap();
    let stored = Arc::new(tokio::sync::Notify::new());
    let (worker, jobs) = env.job_worker();
    let worker = worker.with_season_info(Arc::clone(&stored));
    let cancel = CancellationToken::new();
    let running = tokio::spawn({
        let (worker, cancel) = (worker.clone(), cancel.clone());
        async move { worker.run(cancel).await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(jobs.open_jobs().await.unwrap().is_empty());
    assert!(jobs.done_page(None, 10).await.unwrap().items.is_empty());

    // The season queue stores the entry and rings: the worker looks again
    // without waiting for anything else.
    env.season_entry().await;
    stored.notify_one();
    assert!(
        auto_job_done(&jobs).await,
        "the episode was received once the season's count was known"
    );

    cancel.cancel();
    running.await.unwrap();
}

/// How many times the recheck has read the items (their `checks` summed).
async fn rechecks_read(env: &Env) -> i64 {
    env.h
        .db
        .run::<_, trss_core::DbError, _>(|c| {
            Ok(c.query_row(
                "SELECT COALESCE(SUM(checks), 0) FROM subtitle_item_rechecks",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap()
}

/// Whether the recheck's reading count reaches `count` within a few seconds.
async fn rechecks_reach(env: &Env, count: i64) -> bool {
    for _ in 0..200 {
        if rechecks_read(env).await == count {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

#[tokio::test]
async fn the_worker_reads_the_received_episode_once_a_day_for_fourteen_days() {
    const DAY: i64 = 24 * 60 * 60 * 1000;
    let env = Env::new().await;
    env.season_entry().await;
    env.follow().await;
    let (worker, jobs) = env.job_worker();
    let worker = worker.with_recheck_every(Duration::from_millis(40));
    let cancel = CancellationToken::new();
    let running = tokio::spawn({
        let (worker, cancel) = (worker.clone(), cancel.clone());
        async move { worker.run(cancel).await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    env.fake.set_recent(vec![env.fake.recent_line(
        3492,
        "1",
        "2026-10-02T11:00:00",
        "https://fake.trss.invalid/ok/lara1",
        "에루샤",
    )]);
    env.observer.run_due().await.unwrap();
    assert!(auto_job_done(&jobs).await);

    // The receipt is minutes old: the loop's passes read nothing.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(rechecks_read(&env).await, 0);

    // Three days later: one reading, and no second one however often the loop
    // looks the same day.
    let start = env.h.clock.load(Ordering::SeqCst);
    env.h.clock.store(start + 3 * DAY, Ordering::SeqCst);
    assert!(rechecks_reach(&env, 1).await, "read on the third day");
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(rechecks_read(&env).await, 1);

    // The next day another one; past the fourteenth day none.
    env.h.clock.store(start + 4 * DAY + 1, Ordering::SeqCst);
    assert!(rechecks_reach(&env, 2).await, "read on the fourth day");
    env.h.clock.store(start + 20 * DAY, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(rechecks_read(&env).await, 2);
    // The fake source's files do not differ from what was received.
    assert_eq!(jobs.done_page(None, 10).await.unwrap().items.len(), 1);

    cancel.cancel();
    running.await.unwrap();
}
