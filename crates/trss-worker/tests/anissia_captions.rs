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
        let (folder, _) = state
            .library
            .add_folder("/c".into(), scan, 1, &[])
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
