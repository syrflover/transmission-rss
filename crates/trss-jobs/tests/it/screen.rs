//! A job whose post needs a person's check in the server browser
//! (`docs/specs/jobs.md`, 작업 화면 안의 인증과 브라우저 수명): the run that
//! shows the check is bound to the job, the worker watches it for the file,
//! and a run that ends leaves the job waiting with no run. The browser is a
//! fake; the real one is tested by `trss-subtitles`'s ignored `auth_sample`
//! and `trss-web`'s ignored `remote_screen_docker`.

use crate::Handles;
use std::{
    collections::HashSet,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use tokio::sync::{mpsc, Notify};
use tokio_util::sync::CancellationToken;
use trss_core::{Clock, Db, DbError};
use trss_jobs::{
    area::ReceiveArea,
    runner::{NO_AUTH_BROWSER, OTHER_CHECK_FIRST},
    screen::{CHECK_PREPARED_AGAIN, FILE_REFUSED, RESTARTED_CHECK, RUN_ENDED, WORKER_RESTARTED},
    store::JobDetail,
    Created, FileState, Format, ItemState, JobState, NewItem, NewJob, Runner, ScreenState,
    ScreenStore, StepKind, StepState, Wait,
};
use trss_subtitles::{
    auth::{AuthBrowser, BoxFuture, PrepareRequest, Prepared, Waited},
    fake::{self, FakeSource},
    Failure, FailureKind, Sources,
};

/// What the fake browser's run gives the watch next.
enum Next {
    /// A download of `name` with the fake post's subtitle.
    File(&'static str),
    /// The answer that should have been the file was a web page.
    Refused(Failure),
    NotTaken,
    Ended,
}

#[derive(Default)]
struct FakeBrowser {
    /// What each preparation answers, in order; a run ID each, live from
    /// then on. Missing: `run-<n>`.
    failures: Mutex<Vec<Option<Failure>>>,
    prepared: Mutex<Vec<(String, String)>>,
    live: Mutex<HashSet<String>>,
    touched: Mutex<Vec<String>>,
    released: Mutex<Vec<String>>,
    next: Mutex<Option<mpsc::UnboundedReceiver<Next>>>,
    feed: Mutex<Option<mpsc::UnboundedSender<Next>>>,
    /// Every preparation in the first run (`run-1`), each on a page of its
    /// own, as the pool does for a job whose run is still live.
    one_run: AtomicBool,
    /// The runs `rearm` was asked for, in order.
    rearmed: Mutex<Vec<String>>,
    /// Whether `rearm` waits for `rearm_go` (a check being brought back).
    hold_rearm: AtomicBool,
    rearm_go: Notify,
    /// The pages the run has open; the first is the one it was prepared
    /// with.
    pages: Mutex<Vec<String>>,
    /// Every page asked to close, and those that were closed.
    close_calls: Mutex<Vec<String>>,
    closed: Mutex<Vec<String>>,
    /// A download of the run is on its way.
    under_way: AtomicBool,
    /// With a store: the state of the job at each release, to tell whether
    /// the job was put back in line before its run ended.
    store: Mutex<Option<Handles>>,
    states_at_release: Mutex<Vec<JobState>>,
}

impl FakeBrowser {
    fn new() -> Arc<FakeBrowser> {
        let (tx, rx) = mpsc::unbounded_channel();
        Arc::new(FakeBrowser {
            next: Mutex::new(Some(rx)),
            feed: Mutex::new(Some(tx)),
            ..FakeBrowser::default()
        })
    }

    fn give(&self, next: Next) {
        self.feed
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .send(next)
            .unwrap();
    }

    fn prepares(&self) -> usize {
        self.prepared.lock().unwrap().len()
    }
}

impl AuthBrowser for FakeBrowser {
    fn prepare<'a>(
        &'a self,
        request: PrepareRequest<'a>,
    ) -> BoxFuture<'a, Result<Prepared, Failure>> {
        Box::pin(async move {
            let mut prepared = self.prepared.lock().unwrap();
            prepared.push((request.job.to_owned(), request.post.to_string()));
            let failure = {
                let mut failures = self.failures.lock().unwrap();
                if failures.is_empty() {
                    None
                } else {
                    failures.remove(0)
                }
            };
            if let Some(failure) = failure {
                return Err(failure);
            }
            let run = if self.one_run.load(Ordering::SeqCst) {
                "run-1".to_owned()
            } else {
                format!("run-{}", prepared.len())
            };
            self.live.lock().unwrap().insert(run.clone());
            let target = format!("target-{}", prepared.len());
            *self.pages.lock().unwrap() = vec![target.clone()];
            Ok(Prepared {
                run_id: run,
                target_id: target,
            })
        })
    }

    fn wait_file<'a>(
        &'a self,
        _job: &'a str,
        run_id: &'a str,
        staging: &'a Path,
    ) -> BoxFuture<'a, Waited> {
        Box::pin(async move {
            if !self.live.lock().unwrap().contains(run_id) {
                return Waited::Ended;
            }
            let mut rx = self
                .next
                .lock()
                .unwrap()
                .take()
                .expect("one watch at a time");
            let next = rx.recv().await;
            *self.next.lock().unwrap() = Some(rx);
            match next {
                Some(Next::File(name)) => {
                    std::fs::create_dir_all(staging).unwrap();
                    let path = staging.join(name);
                    std::fs::write(&path, fake::srt("ep1")).unwrap();
                    Waited::File {
                        name: name.to_owned(),
                        path,
                    }
                }
                Some(Next::Refused(failure)) => Waited::Refused(failure),
                Some(Next::NotTaken) => Waited::NotTaken {
                    reason: "취소됐어요".to_owned(),
                },
                Some(Next::Ended) | None => {
                    self.live.lock().unwrap().remove(run_id);
                    Waited::Ended
                }
            }
        })
    }

    fn is_live(&self, _job: &str, run_id: &str) -> bool {
        self.live.lock().unwrap().contains(run_id)
    }

    fn touch(&self, _job: &str, run_id: &str) -> bool {
        self.touched.lock().unwrap().push(run_id.to_owned());
        self.is_live(_job, run_id)
    }

    fn release<'a>(&'a self, job: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let store = self.store.lock().unwrap().clone();
            if let Some(store) = store {
                let state = store.views.detail(job).await.unwrap().unwrap().row.state;
                self.states_at_release.lock().unwrap().push(state);
            }
            self.released.lock().unwrap().push(job.to_owned());
            self.live.lock().unwrap().clear();
        })
    }

    fn downloading(&self, _job: &str, _run_id: &str) -> bool {
        self.under_way.load(Ordering::SeqCst)
    }

    fn pages(&self, job: &str, run_id: &str) -> Vec<String> {
        match self.is_live(job, run_id) {
            true => self.pages.lock().unwrap().clone(),
            false => Vec::new(),
        }
    }

    fn close_page<'a>(
        &'a self,
        job: &'a str,
        run_id: &'a str,
        target: &'a str,
    ) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            self.close_calls.lock().unwrap().push(target.to_owned());
            let mut pages = self.pages.lock().unwrap();
            if !self.is_live(job, run_id) || pages.first().map(String::as_str) == Some(target) {
                return false;
            }
            let before = pages.len();
            pages.retain(|p| p != target);
            let closed = pages.len() < before;
            if closed {
                self.closed.lock().unwrap().push(target.to_owned());
            }
            closed
        })
    }

    fn rearm<'a>(&'a self, _job: &'a str, run_id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.rearmed.lock().unwrap().push(run_id.to_owned());
            if self.hold_rearm.load(Ordering::SeqCst) {
                self.rearm_go.notified().await;
            }
        })
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    db: Db,
    store: Handles,
    screens: ScreenStore,
    runner: Runner,
    area: ReceiveArea,
    browser: Arc<FakeBrowser>,
    wake: Arc<Notify>,
    /// The worker's shutdown.
    shutdown: CancellationToken,
}

fn ticking_clock() -> Clock {
    let now = Arc::new(AtomicI64::new(1_000));
    Arc::new(move || now.fetch_add(10, Ordering::SeqCst))
}

async fn setup(with_browser: bool) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let store = Handles::new(db.clone());
    let area = ReceiveArea::in_app_data(dir.path());
    let browser = FakeBrowser::new();
    let mut runner = Runner::new(
        store.run.clone(),
        Sources::none().with_fake(FakeSource),
        area.clone(),
        ticking_clock(),
    );
    if with_browser {
        runner = runner.with_auth(browser.clone());
    }
    Setup {
        _dir: dir,
        store,
        screens: ScreenStore::new(db.clone()),
        db,
        runner,
        area,
        browser,
        wake: Arc::new(Notify::new()),
        shutdown: CancellationToken::new(),
    }
}

async fn make(s: &Setup, posts: &[&str]) -> String {
    let job = NewJob {
        command_id: "c1".to_owned(),
        request: "{}".to_owned(),
        origin: "pick".to_owned(),
        work_id: None,
        season: Some(1),
        anime_no: None,
        source_id: None,
        creator: Some("제작자".to_owned()),
        revision_of: None,
        revises_attributed: false,
        items: posts
            .iter()
            .enumerate()
            .map(|(n, post)| NewItem {
                observation_id: None,
                episode: (n + 1).to_string(),
                post_url: (*post).to_owned(),
                found_at: 500,
            })
            .collect(),
    };
    match s.store.requests.create(job, 900).await.unwrap() {
        Created::Created(id) => id,
        other => panic!("created: {other:?}"),
    }
}

const POST: &str = "https://fake.trss.invalid/check/ep1";

async fn run(s: &Setup) {
    s.runner.run_ready(&CancellationToken::new()).await.unwrap();
}

async fn detail(s: &Setup, id: &str) -> JobDetail {
    s.store.views.detail(id).await.unwrap().unwrap()
}

fn step(d: &JobDetail, kind: StepKind) -> Option<StepState> {
    d.steps.iter().find(|s| s.step == kind).map(|s| s.state)
}

async fn tend(s: &Setup) {
    s.runner.tend_screens(&s.wake, &s.shutdown).await.unwrap();
}

/// Waits until `check` holds, for up to two seconds.
async fn until<F, Fut>(check: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("not in time");
}

/// A job brought to the check: waiting, its run bound.
async fn waiting(s: &Setup) -> String {
    let id = make(s, &[POST]).await;
    run(s).await;
    let d = detail(s, &id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Auth))
    );
    id
}

#[tokio::test]
async fn a_check_post_waits_with_its_run_bound_and_a_passed_check_brings_its_file_in() {
    let s = setup(true).await;
    let id = waiting(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(
        d.row.note.as_deref(),
        Some("사이트 확인을 기다려요 (가짜 사람 확인)")
    );
    assert_eq!(step(&d, StepKind::Open), Some(StepState::Done));
    assert_eq!(step(&d, StepKind::Auth), Some(StepState::Waiting));
    assert_eq!(
        (d.items[0].state, d.items[0].wait),
        (ItemState::Waiting, Some(Wait::Auth))
    );
    let screen = s.screens.screen(&id).await.unwrap().unwrap();
    assert_eq!(screen.state, ScreenState::Ready);
    assert_eq!(screen.run_id.as_deref(), Some("run-1"));
    assert_eq!(screen.target_id.as_deref(), Some("target-1"));
    // The run is kept for the check.
    assert!(s.browser.released.lock().unwrap().is_empty());
    assert_eq!(s.browser.prepares(), 1);

    // The watch: a download that did not finish is logged and waited past.
    tend(&s).await;
    s.browser.give(Next::NotTaken);
    until(|| async {
        detail(&s, &id)
            .await
            .events
            .iter()
            .any(|e| e.message == "브라우저가 파일을 받지 못했어요")
    })
    .await;
    // A second tend does not watch the same run twice (the fake allows one).
    tend(&s).await;

    let woken = s.wake.notified();
    s.browser.give(Next::File("ep1.srt"));
    tokio::time::timeout(Duration::from_secs(2), woken)
        .await
        .expect("the worker is woken");
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Pending);
    assert_eq!(step(&d, StepKind::Auth), Some(StepState::Done));
    assert_eq!(d.items[0].state, ItemState::Pending);
    assert!(s.screens.screen(&id).await.unwrap().is_none());

    // Received like any other file, then the run goes.
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.events);
    assert_eq!(s.browser.prepares(), 1);
    let file = &d.items[0].files[0];
    assert_eq!(file.state, FileState::Done);
    assert_eq!(file.name, "ep1.srt");
    assert_eq!(file.format, Some(Format::Srt));
    assert_eq!(file.file_key, "browser:fake.trss.invalid/check/ep1#ep1.srt");
    assert_eq!(
        std::fs::read(s.area.at(file.path.as_deref().unwrap())).unwrap(),
        fake::srt("ep1")
    );
    assert!(!s
        .area
        .at(&format!(".tmp/check-{id}-{}", d.items[0].id))
        .exists());
    assert_eq!(*s.browser.released.lock().unwrap(), vec![id.clone()]);
}

#[tokio::test]
async fn a_run_that_ends_clears_its_binding_and_leaves_the_job_waiting_until_asked_again() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    tend(&s).await;
    s.browser.give(Next::Ended);
    until(|| async { s.screens.screen(&id).await.unwrap().unwrap().state == ScreenState::Closed })
        .await;
    let screen = s.screens.screen(&id).await.unwrap().unwrap();
    assert_eq!(screen.run_id, None);
    assert_eq!(screen.note.as_deref(), Some(RUN_ENDED));
    let d = detail(&s, &id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Auth))
    );
    assert_eq!(step(&d, StepKind::Auth), Some(StepState::Waiting));
    assert!(d
        .events
        .iter()
        .any(|e| e.message == "사이트 확인을 기다리던 서버 브라우저가 닫혔어요"));

    // Nothing starts again by itself.
    tend(&s).await;
    run(&s).await;
    assert_eq!(s.browser.prepares(), 1);
    assert_eq!(detail(&s, &id).await.row.state, JobState::Waiting);

    // A person opens the job's page: it is prepared again with a new run.
    let asked = s
        .screens
        .request_prepare(&id, 5_000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(asked.state, ScreenState::Preparing);
    let woken = s.wake.notified();
    tend(&s).await;
    tokio::time::timeout(Duration::from_secs(1), woken)
        .await
        .expect("the worker is woken");
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Pending);
    assert_eq!(
        s.screens.screen(&id).await.unwrap().unwrap().state,
        ScreenState::Preparing
    );
    run(&s).await;
    assert_eq!(s.browser.prepares(), 2);
    let screen = s.screens.screen(&id).await.unwrap().unwrap();
    assert_eq!(screen.state, ScreenState::Ready);
    assert_eq!(screen.run_id.as_deref(), Some("run-2"));
    assert!(s.screens.prepare_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn opening_the_page_of_a_live_run_starts_nothing_and_counts_as_use() {
    let s = setup(true).await;
    let id = waiting(&s).await;

    // Reading the screen, as the job's page and the to-do lists do, writes
    // nothing and asks for nothing.
    for _ in 0..3 {
        s.screens.screen(&id).await.unwrap();
        s.store.views.auth_waits().await.unwrap();
        s.store.views.open_jobs().await.unwrap();
    }
    assert!(s.screens.prepare_requests().await.unwrap().is_empty());
    tend(&s).await;
    assert!(s.browser.touched.lock().unwrap().is_empty());

    let screen = s
        .screens
        .request_prepare(&id, 5_000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(screen.state, ScreenState::Ready);
    assert_eq!(s.screens.prepare_requests().await.unwrap().len(), 1);
    tend(&s).await;
    assert_eq!(*s.browser.touched.lock().unwrap(), vec!["run-1".to_owned()]);
    assert!(s.screens.prepare_requests().await.unwrap().is_empty());
    // Only touched: an ordinary request never ends the run.
    assert!(s.browser.released.lock().unwrap().is_empty());
    run(&s).await;
    assert_eq!(s.browser.prepares(), 1);
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Waiting);
    assert_eq!(
        s.screens
            .screen(&id)
            .await
            .unwrap()
            .unwrap()
            .run_id
            .as_deref(),
        Some("run-1")
    );
}

#[tokio::test]
async fn a_job_that_does_not_wait_for_its_check_takes_no_request() {
    let s = setup(true).await;
    let id = make(&s, &[POST]).await;
    // Pending: no screen yet.
    assert_eq!(s.screens.request_prepare(&id, 5_000).await.unwrap(), None);
    run(&s).await;
    // Once it ended otherwise, its screen is gone.
    tend(&s).await;
    s.browser.give(Next::File("ep1.srt"));
    until(|| async { detail(&s, &id).await.row.state == JobState::Pending }).await;
    run(&s).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Done);
    assert_eq!(s.screens.request_prepare(&id, 9_000).await.unwrap(), None);
    assert!(s.screens.prepare_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_preparation_that_fails_leaves_the_screen_closed_with_why_and_a_request_tries_again() {
    let s = setup(true).await;
    s.browser.failures.lock().unwrap().push(Some(Failure::new(
        FailureKind::Network,
        "서버 브라우저에 빈자리가 나지 않았어요",
    )));
    let id = waiting(&s).await;
    let screen = s.screens.screen(&id).await.unwrap().unwrap();
    assert_eq!(screen.state, ScreenState::Closed);
    assert_eq!(
        screen.note.as_deref(),
        Some("확인 화면을 준비하지 못했어요: 서버 브라우저에 빈자리가 나지 않았어요")
    );
    // Not a failure of the job: it waits for the check.
    let d = detail(&s, &id).await;
    assert_eq!(d.items[0].reason.as_deref(), Some(fake::CHECK_REASON));

    s.screens.request_prepare(&id, 5_000).await.unwrap();
    tend(&s).await;
    run(&s).await;
    assert_eq!(s.browser.prepares(), 2);
    assert_eq!(
        s.screens.screen(&id).await.unwrap().unwrap().state,
        ScreenState::Ready
    );
}

#[tokio::test]
async fn the_tabs_and_requests_of_a_binding_never_reach_the_next_one() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    let item = s.screens.bound(&id).await.unwrap().unwrap().item_id;
    let pages = ["target-1".to_owned(), "popup-1".to_owned()];
    // The same run bound again (another check of the job), and bound again
    // after a worker restart cleared it: a request written for the binding
    // before is not the new one's.
    for restart in [false, true] {
        s.screens
            .set_pages(&id, "run-1", &pages, 1_000)
            .await
            .unwrap();
        let bound = shown(&s, &id).await.bound_at.unwrap();
        assert!(s
            .screens
            .request_switch(&id, "run-1", bound, "popup-1", 2_000)
            .await
            .unwrap());
        assert!(s
            .screens
            .request_close(&id, "run-1", bound, Some("popup-1"), 2_000)
            .await
            .unwrap());
        if restart {
            assert_eq!(s.screens.unbind_all(2_500).await.unwrap(), 1);
            let screen = shown(&s, &id).await;
            assert!(screen.pages.is_empty() && screen.first_target_id.is_none());
        }
        s.screens
            .bind(&id, item, "run-1", "target-1", 3_000 + i64::from(restart))
            .await
            .unwrap();
        let screen = shown(&s, &id).await;
        assert_eq!(screen.pages, vec!["target-1".to_owned()]);
        assert_eq!(s.screens.take_switch(&id, "run-1").await.unwrap(), None);
        assert!(s.screens.take_close(&id, "run-1").await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn a_worker_restart_clears_every_binding() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    assert_eq!(s.screens.unbind_all(9_000).await.unwrap(), 1);
    let screen = s.screens.screen(&id).await.unwrap().unwrap();
    assert_eq!(screen.state, ScreenState::Closed);
    assert_eq!(screen.note.as_deref(), Some(WORKER_RESTARTED));
    assert_eq!(detail(&s, &id).await.row.wait, Some(Wait::Auth));
}

#[tokio::test]
async fn a_persons_input_is_kept_for_the_bound_run_only() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    assert!(s.screens.live_inputs().await.unwrap().is_empty());
    assert!(!s.screens.record_input(&id, "run-0", 2_000).await.unwrap());
    assert!(s.screens.record_input(&id, "run-1", 2_000).await.unwrap());
    // Never back in time.
    assert!(s.screens.record_input(&id, "run-1", 1_500).await.unwrap());
    assert_eq!(
        s.screens.live_inputs().await.unwrap(),
        vec![("run-1".to_owned(), 2_000)]
    );
    // The input is the run's: it goes with the binding.
    s.screens
        .unbind(&id, "run-1", RUN_ENDED, 3_000)
        .await
        .unwrap();
    assert!(s.screens.live_inputs().await.unwrap().is_empty());
}

#[tokio::test]
async fn one_check_of_a_job_is_on_the_screen_at_a_time() {
    let s = setup(true).await;
    let id = make(&s, &[POST, "https://fake.trss.invalid/check/ep2"]).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(s.browser.prepares(), 1);
    assert_eq!(d.items[0].reason.as_deref(), Some(fake::CHECK_REASON));
    assert_eq!(
        (d.items[1].state, d.items[1].reason.as_deref()),
        (ItemState::Waiting, Some(OTHER_CHECK_FIRST))
    );

    // The first one's file comes: the first is received, the second brought
    // to its check.
    tend(&s).await;
    s.browser.give(Next::File("ep1.srt"));
    until(|| async { detail(&s, &id).await.row.state == JobState::Pending }).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.items[0].state, ItemState::Done);
    assert_eq!(d.items[1].reason.as_deref(), Some(fake::CHECK_REASON));
    assert_eq!(s.browser.prepares(), 2);
    let screen = s.screens.screen(&id).await.unwrap().unwrap();
    assert_eq!(screen.run_id.as_deref(), Some("run-2"));
}

#[tokio::test]
async fn a_second_check_in_the_same_run_is_a_new_binding_and_is_watched() {
    let s = setup(true).await;
    s.browser.one_run.store(true, Ordering::SeqCst);
    let id = make(&s, &[POST, "https://fake.trss.invalid/check/ep2"]).await;
    run(&s).await;
    let first = s.screens.screen(&id).await.unwrap().unwrap();
    assert_eq!(
        (first.run_id.as_deref(), first.target_id.as_deref()),
        (Some("run-1"), Some("target-1"))
    );

    tend(&s).await;
    s.browser.give(Next::File("ep1.srt"));
    until(|| async { detail(&s, &id).await.row.state == JobState::Pending }).await;
    run(&s).await;
    // The second check is on another page of the same run, bound anew.
    let second = s.screens.screen(&id).await.unwrap().unwrap();
    assert_eq!(second.state, ScreenState::Ready);
    assert_eq!(
        (second.run_id.as_deref(), second.target_id.as_deref()),
        (Some("run-1"), Some("target-2"))
    );
    assert!(second.bound_at > first.bound_at);

    // And watched for its own file, in its own item's folder.
    tend(&s).await;
    s.browser.give(Next::File("ep2.srt"));
    until(|| async { detail(&s, &id).await.row.state == JobState::Pending }).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.events);
    assert_eq!(d.items[1].files[0].name, "ep2.srt");
    for item in &d.items {
        assert!(!s.area.at(&format!(".tmp/check-{id}-{}", item.id)).exists());
    }
}

#[tokio::test]
async fn a_file_that_comes_after_its_binding_changed_is_kept_for_the_items_next_run() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    let item = detail(&s, &id).await.items[0].id;
    let folder = s.area.at(&format!(".tmp/check-{id}-{item}"));
    tend(&s).await;
    // The binding goes (as when the run is taken for gone) while the
    // download finishes.
    s.screens
        .unbind(&id, "run-1", RUN_ENDED, 5_000)
        .await
        .unwrap();
    s.browser.give(Next::File("ep1.srt"));
    until(|| async { folder.join("ep1.srt").exists() }).await;
    // The watch is done with it, and the job still waits.
    until(|| async {
        // A second watch would take the fake's only feed; it is free again.
        s.browser.next.lock().unwrap().is_some()
    })
    .await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Waiting);
    assert!(folder.join("ep1.srt").exists());

    // The person opens the page: the item's next run takes the kept file
    // without another check.
    s.screens.request_prepare(&id, 6_000).await.unwrap();
    tend(&s).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done, "{:?}", d.events);
    assert_eq!(s.browser.prepares(), 1);
    assert!(!folder.exists());
}

#[tokio::test]
async fn a_file_that_comes_for_an_item_that_settled_is_removed() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    let item = detail(&s, &id).await.items[0].id;
    let folder = s.area.at(&format!(".tmp/check-{id}-{item}"));
    tend(&s).await;
    s.screens
        .unbind(&id, "run-1", RUN_ENDED, 5_000)
        .await
        .unwrap();
    s.store
        .run
        .set_item(
            item,
            ItemState::Failed,
            None,
            Some("다른 곳에서 받았어요".to_owned()),
            5_000,
        )
        .await
        .unwrap();
    s.browser.give(Next::File("ep1.srt"));
    until(|| async { s.browser.next.lock().unwrap().is_some() && !folder.exists() }).await;
}

#[tokio::test]
async fn a_run_the_workers_shutdown_ends_leaves_the_screen_to_the_next_start() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    tend(&s).await;
    s.shutdown.cancel();
    s.browser.give(Next::Ended);
    until(|| async { s.browser.next.lock().unwrap().is_some() }).await;
    // No note of a run that ended, and nothing logged about it.
    let screen = s.screens.screen(&id).await.unwrap().unwrap();
    assert_eq!(screen.state, ScreenState::Ready);
    assert!(!detail(&s, &id)
        .await
        .events
        .iter()
        .any(|e| e.message == "사이트 확인을 기다리던 서버 브라우저가 닫혔어요"));
    // The next start closes it with its own note.
    assert_eq!(s.screens.unbind_all(9_000).await.unwrap(), 1);
    assert_eq!(
        s.screens
            .screen(&id)
            .await
            .unwrap()
            .unwrap()
            .note
            .as_deref(),
        Some(WORKER_RESTARTED)
    );
}

#[tokio::test]
async fn without_a_server_browser_a_check_post_waits_for_a_source() {
    let s = setup(false).await;
    let id = make(&s, &[POST]).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Subtitle))
    );
    assert_eq!(d.items[0].reason.as_deref(), Some(NO_AUTH_BROWSER));
    assert!(s.screens.screen(&id).await.unwrap().is_none());
}

/// The page that came instead of the file, as the browser tells it.
fn refusal(kind: FailureKind, status: u16) -> Failure {
    Failure::new(kind, format!("가짜 거절 (HTTP {status})")).with_response(
        Some(status),
        Some("text/html".to_owned()),
        None,
    )
}

/// The item's folder of the check.
fn folder_of(s: &Setup, id: &str, item: i64) -> std::path::PathBuf {
    s.area.at(&format!(".tmp/check-{id}-{item}"))
}

#[tokio::test]
async fn a_refused_download_fails_the_item_with_its_class_and_removes_its_folder() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    let item = detail(&s, &id).await.items[0].id;
    let folder = folder_of(&s, &id, item);
    tend(&s).await;

    let woken = s.wake.notified();
    s.browser
        .give(Next::Refused(refusal(FailureKind::Expired, 403)));
    tokio::time::timeout(Duration::from_secs(2), woken)
        .await
        .expect("the worker is woken");
    // The check is passed and the screen is over; the refusal waits in the
    // item's folder for its next run.
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Pending);
    assert_eq!(step(&d, StepKind::Auth), Some(StepState::Done));
    assert!(s.screens.screen(&id).await.unwrap().is_none());
    assert!(folder.join(".refused").exists());
    let event = d
        .events
        .iter()
        .find(|e| e.message == FILE_REFUSED)
        .expect("the refusal is logged");
    assert_eq!(event.detail.as_deref(), Some("만료 · 가짜 거절 (HTTP 403)"));

    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Failed, "{:?}", d.events);
    assert_eq!(d.items[0].state, ItemState::Failed);
    assert_eq!(d.items[0].failure, Some(FailureKind::Expired));
    assert_eq!(d.items[0].reason.as_deref(), Some("가짜 거절 (HTTP 403)"));
    assert!(d.items[0].files.is_empty());
    assert!(d.events.iter().any(|e| e
        .message
        .ends_with("사이트가 파일 대신 웹 페이지를 보냈어요")));
    // Nothing asked the address or the check again, and the folder is gone.
    assert_eq!(s.browser.prepares(), 1);
    assert!(!folder.exists());
}

#[tokio::test]
async fn a_refusal_that_is_the_sites_failure_ends_the_item_at_once_as_a_network_failure() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    let item = detail(&s, &id).await.items[0].id;
    tend(&s).await;
    let woken = s.wake.notified();
    s.browser
        .give(Next::Refused(refusal(FailureKind::Network, 503)));
    tokio::time::timeout(Duration::from_secs(2), woken)
        .await
        .expect("the worker is woken");

    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.items[0].state, ItemState::Failed, "{:?}", d.events);
    assert_eq!(d.items[0].failure, Some(FailureKind::Network));
    // No second attempt: a new address needs a new check.
    assert_eq!(s.browser.prepares(), 1);
    assert!(d
        .events
        .iter()
        .any(|e| e.message.ends_with("사이트가 파일을 주지 못했어요")));
    assert!(!d.events.iter().any(|e| e
        .message
        .ends_with("사이트가 파일 대신 웹 페이지를 보냈어요")));
    assert!(!folder_of(&s, &id, item).exists());
}

#[tokio::test]
async fn a_refusal_that_comes_after_its_binding_changed_is_kept_or_removed_with_its_item() {
    // The item still waits for its file: its next run fails with the kept
    // refusal, without another check.
    let s = setup(true).await;
    let id = waiting(&s).await;
    let item = detail(&s, &id).await.items[0].id;
    let folder = folder_of(&s, &id, item);
    tend(&s).await;
    s.screens
        .unbind(&id, "run-1", RUN_ENDED, 5_000)
        .await
        .unwrap();
    s.browser
        .give(Next::Refused(refusal(FailureKind::Missing, 404)));
    until(|| async {
        folder.join(".refused").exists() && s.browser.next.lock().unwrap().is_some()
    })
    .await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Waiting);

    s.screens.request_prepare(&id, 6_000).await.unwrap();
    tend(&s).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(
        d.items[0].failure,
        Some(FailureKind::Missing),
        "{:?}",
        d.events
    );
    assert_eq!(s.browser.prepares(), 1);
    assert!(!folder.exists());

    // The item settled meanwhile: the kept refusal goes.
    let s = setup(true).await;
    let id = waiting(&s).await;
    let item = detail(&s, &id).await.items[0].id;
    let folder = folder_of(&s, &id, item);
    tend(&s).await;
    s.screens
        .unbind(&id, "run-1", RUN_ENDED, 5_000)
        .await
        .unwrap();
    s.store
        .run
        .set_item(
            item,
            ItemState::Failed,
            None,
            Some("다른 곳에서 받았어요".to_owned()),
            5_000,
        )
        .await
        .unwrap();
    s.browser
        .give(Next::Refused(refusal(FailureKind::Expired, 403)));
    until(|| async { s.browser.next.lock().unwrap().is_some() && !folder.exists() }).await;
}

#[tokio::test]
async fn opening_the_screen_of_a_live_run_brings_its_check_back_once_at_a_time() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    assert!(s.browser.rearmed.lock().unwrap().is_empty());
    s.browser.hold_rearm.store(true, Ordering::SeqCst);

    s.screens.request_prepare(&id, 5_000).await.unwrap();
    tend(&s).await;
    until(|| async { s.browser.rearmed.lock().unwrap().len() == 1 }).await;
    assert_eq!(*s.browser.rearmed.lock().unwrap(), vec!["run-1".to_owned()]);

    // Opened again while the card is still being clicked: nothing starts.
    s.screens.request_prepare(&id, 6_000).await.unwrap();
    tend(&s).await;
    assert_eq!(s.browser.touched.lock().unwrap().len(), 2);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(s.browser.rearmed.lock().unwrap().len(), 1);

    // Once it is done, the next opening brings it back again.
    s.browser.rearm_go.notify_one();
    let at = AtomicI64::new(7_000);
    until(|| async {
        let now = at.fetch_add(1_000, Ordering::SeqCst);
        s.screens.request_prepare(&id, now).await.unwrap();
        tend(&s).await;
        s.browser.rearmed.lock().unwrap().len() == 2
    })
    .await;
    s.browser.rearm_go.notify_one();

    // A run that is not live is prepared anew, not brought back.
    s.browser.live.lock().unwrap().clear();
    let before = s.browser.rearmed.lock().unwrap().len();
    s.screens.request_prepare(&id, 99_000).await.unwrap();
    tend(&s).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(s.browser.rearmed.lock().unwrap().len(), before);
}

/// The job's screen as the web reads it.
async fn shown(s: &Setup, id: &str) -> trss_jobs::Screen {
    s.screens.screen(id).await.unwrap().unwrap()
}

/// Waits until the screen shows `target`, for up to five seconds.
async fn shows(s: &Setup, id: &str, target: &str) {
    for _ in 0..500 {
        if shown(s, id).await.target_id.as_deref() == Some(target) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the screen does not show {target}");
}

fn open_page(s: &Setup, page: &str) {
    s.browser.pages.lock().unwrap().push(page.to_owned());
}

/// A check's screen with a popup open and shown: `target-1` is the first page.
async fn with_a_popup(s: &Setup) -> String {
    let id = waiting(s).await;
    tend(s).await;
    open_page(s, "popup-1");
    shows(s, &id, "popup-1").await;
    id
}

#[tokio::test(start_paused = true)]
async fn a_check_screen_follows_a_popup_that_stays_and_lists_its_pages() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    tend(&s).await;
    let first = shown(&s, &id).await;
    assert_eq!(first.pages, vec!["target-1".to_owned()]);
    assert_eq!(first.first_target_id.as_deref(), Some("target-1"));

    // A page that opens and closes at once is no tab.
    open_page(&s, "blink");
    tokio::time::sleep(Duration::from_millis(900)).await;
    s.browser.pages.lock().unwrap().retain(|p| p != "blink");
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert_eq!(shown(&s, &id).await.bound_at, first.bound_at);

    // One that stays is shown, as a new binding of the same run, and a tab.
    open_page(&s, "popup-1");
    shows(&s, &id, "popup-1").await;
    let popup = shown(&s, &id).await;
    assert_eq!(popup.run_id, first.run_id);
    assert!(popup.bound_at > first.bound_at);
    assert!(popup.popup);
    assert_eq!(
        popup.pages,
        vec!["target-1".to_owned(), "popup-1".to_owned()]
    );
}

#[tokio::test(start_paused = true)]
async fn a_persons_switch_on_a_check_screen_is_kept_and_a_closed_shown_tab_goes_to_the_newest_left()
{
    let s = setup(true).await;
    let id = with_a_popup(&s).await;
    let popup = shown(&s, &id).await;
    let run_id = popup.run_id.clone().unwrap();

    // Back to the first page: not undone at the follower's next looks.
    assert!(s
        .screens
        .request_switch(&id, &run_id, popup.bound_at.unwrap(), "target-1", 5_000)
        .await
        .unwrap());
    shows(&s, &id, "target-1").await;
    let back = shown(&s, &id).await;
    assert!(back.bound_at > popup.bound_at);
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let kept = shown(&s, &id).await;
    assert_eq!(kept.target_id.as_deref(), Some("target-1"));
    assert_eq!(kept.bound_at, back.bound_at);

    // A second popup that stays is shown.
    open_page(&s, "popup-2");
    shows(&s, &id, "popup-2").await;
    // Closing the shown page: the newest page left is shown.
    let second = shown(&s, &id).await;
    assert!(s
        .screens
        .request_close(&id, &run_id, second.bound_at.unwrap(), None, 6_000)
        .await
        .unwrap());
    shows(&s, &id, "popup-1").await;
    assert_eq!(
        *s.browser.closed.lock().unwrap(),
        vec!["popup-2".to_owned()]
    );
    assert_eq!(
        shown(&s, &id).await.pages,
        vec!["target-1".to_owned(), "popup-1".to_owned()]
    );
}

#[tokio::test(start_paused = true)]
async fn closing_a_tab_that_is_not_shown_closes_only_that_page() {
    let s = setup(true).await;
    let id = with_a_popup(&s).await;
    let popup = shown(&s, &id).await;
    let run_id = popup.run_id.clone().unwrap();
    // The first page is shown now; the popup is the hidden one.
    s.screens
        .request_switch(&id, &run_id, popup.bound_at.unwrap(), "target-1", 5_000)
        .await
        .unwrap();
    shows(&s, &id, "target-1").await;
    let first = shown(&s, &id).await;

    assert!(s
        .screens
        .request_close(
            &id,
            &run_id,
            first.bound_at.unwrap(),
            Some("popup-1"),
            6_000
        )
        .await
        .unwrap());
    until(|| async { s.browser.pages.lock().unwrap().len() == 1 }).await;
    assert_eq!(
        *s.browser.closed.lock().unwrap(),
        vec!["popup-1".to_owned()]
    );
    // The screen is as it was: the same binding, with one tab.
    until(|| async { shown(&s, &id).await.pages == vec!["target-1".to_owned()] }).await;
    let now = shown(&s, &id).await;
    assert_eq!(now.target_id.as_deref(), Some("target-1"));
    assert_eq!(now.bound_at, first.bound_at);
}

#[tokio::test(start_paused = true)]
async fn a_request_for_another_binding_an_unlisted_page_or_the_first_page_is_refused() {
    let s = setup(true).await;
    let id = with_a_popup(&s).await;
    let popup = shown(&s, &id).await;
    let (run_id, bound) = (popup.run_id.clone().unwrap(), popup.bound_at.unwrap());
    let stale = bound - 1;
    for (what, asked) in [
        (
            "a binding the person no longer sees",
            s.screens
                .request_switch(&id, &run_id, stale, "target-1", 5_000)
                .await
                .unwrap(),
        ),
        (
            "a page that is not listed",
            s.screens
                .request_switch(&id, &run_id, bound, "elsewhere", 5_000)
                .await
                .unwrap(),
        ),
        (
            "another run",
            s.screens
                .request_switch(&id, "run-0", bound, "target-1", 5_000)
                .await
                .unwrap(),
        ),
        (
            "closing on a binding the person no longer sees",
            s.screens
                .request_close(&id, &run_id, stale, Some("popup-1"), 5_000)
                .await
                .unwrap(),
        ),
        (
            "closing a page that is not listed",
            s.screens
                .request_close(&id, &run_id, bound, Some("elsewhere"), 5_000)
                .await
                .unwrap(),
        ),
        (
            "closing the first page",
            s.screens
                .request_close(&id, &run_id, bound, Some("target-1"), 5_000)
                .await
                .unwrap(),
        ),
        ("closing the page shown when it is the first", {
            s.screens
                .request_switch(&id, &run_id, bound, "target-1", 5_000)
                .await
                .unwrap();
            shows(&s, &id, "target-1").await;
            let first = shown(&s, &id).await;
            s.screens
                .request_close(&id, &run_id, first.bound_at.unwrap(), None, 5_000)
                .await
                .unwrap()
        }),
    ] {
        assert!(!asked, "{what} was written");
    }
    // Nothing was left for the worker, and no page was closed or asked of the
    // browser: the first page is not even asked to close.
    assert_eq!(s.screens.take_switch(&id, &run_id).await.unwrap(), None);
    assert!(s.screens.take_close(&id, &run_id).await.unwrap().is_empty());
    assert!(s.browser.close_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_request_that_was_written_is_taken_once_and_counts_as_the_persons_input() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    // Pages as the worker listed them: no follower is needed to judge a request.
    assert!(s
        .screens
        .set_pages(
            &id,
            "run-1",
            &["target-1".to_owned(), "popup-1".to_owned()],
            1_500
        )
        .await
        .unwrap());
    // The same list is not written again.
    assert!(!s
        .screens
        .set_pages(
            &id,
            "run-1",
            &["target-1".to_owned(), "popup-1".to_owned()],
            1_600
        )
        .await
        .unwrap());
    let bound = shown(&s, &id).await.bound_at.unwrap();
    assert!(s.screens.live_inputs().await.unwrap().is_empty());

    assert!(s
        .screens
        .request_switch(&id, "run-1", bound, "popup-1", 2_000)
        .await
        .unwrap());
    assert_eq!(
        s.screens.live_inputs().await.unwrap(),
        vec![("run-1".to_owned(), 2_000)]
    );
    // The newest request is the one taken, once.
    assert!(s
        .screens
        .request_switch(&id, "run-1", bound, "target-1", 2_100)
        .await
        .unwrap());
    assert_eq!(
        s.screens
            .take_switch(&id, "run-1")
            .await
            .unwrap()
            .as_deref(),
        Some("target-1")
    );
    assert_eq!(s.screens.take_switch(&id, "run-1").await.unwrap(), None);

    assert!(s
        .screens
        .request_close(&id, "run-1", bound, Some("popup-1"), 3_000)
        .await
        .unwrap());
    // The same page asked twice is one request.
    assert!(s
        .screens
        .request_close(&id, "run-1", bound, Some("popup-1"), 3_100)
        .await
        .unwrap());
    assert_eq!(
        s.screens.live_inputs().await.unwrap(),
        vec![("run-1".to_owned(), 3_100)]
    );
    assert_eq!(
        s.screens.take_close(&id, "run-1").await.unwrap(),
        vec!["popup-1".to_owned()]
    );
    assert!(s.screens.take_close(&id, "run-1").await.unwrap().is_empty());

    // The pages and requests go with the binding.
    assert!(s
        .screens
        .request_switch(&id, "run-1", bound, "popup-1", 4_000)
        .await
        .unwrap());
    s.screens
        .unbind(&id, "run-1", RUN_ENDED, 5_000)
        .await
        .unwrap();
    assert!(s
        .screens
        .screen(&id)
        .await
        .unwrap()
        .unwrap()
        .pages
        .is_empty());
    assert_eq!(s.screens.take_switch(&id, "run-1").await.unwrap(), None);
}

/// Waits until the fake's watch has given up its feed (it ended), for up to
/// two seconds.
async fn watch_ended(s: &Setup) {
    until(|| async { s.browser.next.lock().unwrap().is_some() }).await;
}

/// Puts the job in `state` whatever its binding, as another part of the app
/// would (`waiting` is the check's wait).
async fn force_state(s: &Setup, id: &str, state: &'static str) {
    let id = id.to_owned();
    s.db.run::<_, DbError, _>(move |c| {
        c.execute(
            "UPDATE subtitle_jobs SET state = ?2, wait = CASE ?2 WHEN 'waiting' THEN 'auth' END
             WHERE id = ?1",
            rusqlite::params![id, state],
        )?;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_request_to_start_a_run_anew_is_refused_for_another_binding_or_a_job_not_waiting() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    let bound = shown(&s, &id).await.bound_at.unwrap();
    for (what, asked) in [
        (
            "another run",
            s.screens
                .request_restart(&id, "run-0", bound, 5_000)
                .await
                .unwrap(),
        ),
        (
            "a binding the person no longer sees",
            s.screens
                .request_restart(&id, "run-1", bound - 1, 5_000)
                .await
                .unwrap(),
        ),
        (
            "a job that is gone",
            s.screens
                .request_restart("no-such-job", "run-1", bound, 5_000)
                .await
                .unwrap(),
        ),
    ] {
        assert!(!asked, "{what} was written");
    }
    assert!(s.screens.prepare_requests().await.unwrap().is_empty());
    assert!(s.screens.live_inputs().await.unwrap().is_empty());

    // A job that no longer waits for its check, with its binding still
    // there, takes no request either.
    force_state(&s, &id, "running").await;
    assert!(!s
        .screens
        .request_restart(&id, "run-1", bound, 7_000)
        .await
        .unwrap());
    assert!(s.screens.live_inputs().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_request_to_start_a_run_anew_is_a_prepare_request_with_restart_and_counts_as_input() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    let bound = shown(&s, &id).await.bound_at.unwrap();

    // An ordinary opening of the page is no restart.
    s.screens.request_prepare(&id, 4_000).await.unwrap();
    let asked = s.screens.prepare_requests().await.unwrap();
    assert_eq!(asked.len(), 1);
    assert!(!asked[0].restart);
    s.screens
        .mark_prepared(&id, asked[0].asked_at, 4_500)
        .await
        .unwrap();
    assert!(s.screens.prepare_requests().await.unwrap().is_empty());

    assert!(s
        .screens
        .request_restart(&id, "run-1", bound, 5_000)
        .await
        .unwrap());
    let asked = s.screens.prepare_requests().await.unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        (asked[0].job_id.as_str(), asked[0].run_id.as_deref()),
        (id.as_str(), Some("run-1"))
    );
    assert!(asked[0].restart && !asked[0].find);
    assert!(asked[0].asked_at >= 5_000);
    assert_eq!(
        s.screens.live_inputs().await.unwrap(),
        vec![("run-1".to_owned(), 5_000)]
    );
    // The screen still shows the run until the worker answers.
    assert_eq!(shown(&s, &id).await.state, ScreenState::Ready);

    // A person opening the page meanwhile does not take it back.
    s.screens.request_prepare(&id, 6_000).await.unwrap();
    assert!(s.screens.prepare_requests().await.unwrap()[0].restart);
}

#[tokio::test]
async fn a_restart_does_not_outlive_its_binding() {
    for how in ["bind", "unbind_all", "requeue_for_check"] {
        let s = setup(true).await;
        let id = waiting(&s).await;
        let item = s.screens.bound(&id).await.unwrap().unwrap().item_id;
        let bound = shown(&s, &id).await.bound_at.unwrap();
        assert!(s
            .screens
            .request_restart(&id, "run-1", bound, 5_000)
            .await
            .unwrap());
        let asked = s.screens.prepare_requests().await.unwrap()[0].asked_at;
        match how {
            "bind" => {}
            "unbind_all" => {
                assert_eq!(s.screens.unbind_all(5_500).await.unwrap(), 1);
            }
            _ => {
                assert!(s
                    .screens
                    .requeue_for_check(&id, asked, true, 5_500)
                    .await
                    .unwrap());
                // The job waits for its check again (the next run is the same run).
                force_state(&s, &id, "waiting").await;
            }
        }
        // The same run, bound anew (another check of the job).
        s.screens
            .bind(&id, item, "run-1", "target-2", 6_000)
            .await
            .unwrap();
        let screen = shown(&s, &id).await;
        assert_eq!(screen.run_id.as_deref(), Some("run-1"), "{how}");
        s.screens.request_prepare(&id, 7_000).await.unwrap();
        let asked = s.screens.prepare_requests().await.unwrap();
        assert_eq!(asked.len(), 1, "{how}");
        assert!(!asked[0].restart, "{how}: the new binding is a restart");
    }
}

#[tokio::test]
async fn a_failed_binding_and_a_retarget_treat_a_pending_restart_by_whether_the_run_stays() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    let item = s.screens.bound(&id).await.unwrap().unwrap().item_id;
    let bound = shown(&s, &id).await.bound_at.unwrap();
    assert!(s
        .screens
        .request_restart(&id, "run-1", bound, 5_000)
        .await
        .unwrap());
    // The same run on another page: still the run the person wants anew.
    assert!(s
        .screens
        .retarget(&id, "run-1", "popup-1", 5_500)
        .await
        .unwrap());
    assert!(s.screens.prepare_requests().await.unwrap()[0].restart);
    // No run bound: nothing is left to start anew.
    s.screens
        .bind_failed(&id, item, "안 열렸어요".to_owned(), 6_000)
        .await
        .unwrap();
    s.screens.request_prepare(&id, 7_000).await.unwrap();
    let asked = s.screens.prepare_requests().await.unwrap();
    assert_eq!(asked.len(), 1);
    assert!(!asked[0].restart && asked[0].run_id.is_none());
}

#[tokio::test]
async fn a_run_is_not_unbound_as_ended_while_its_restart_is_pending() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    let bound = shown(&s, &id).await.bound_at.unwrap();

    // Without a restart pending it is, as before.
    assert!(!s
        .screens
        .unbind(&id, "run-0", RUN_ENDED, 4_000)
        .await
        .unwrap());
    assert!(s
        .screens
        .request_restart(&id, "run-1", bound, 5_000)
        .await
        .unwrap());
    assert!(!s
        .screens
        .unbind(&id, "run-1", RUN_ENDED, 5_500)
        .await
        .unwrap());
    let screen = shown(&s, &id).await;
    assert_eq!(screen.run_id.as_deref(), Some("run-1"));
    assert_eq!(screen.note, None);
    assert!(s.screens.prepare_requests().await.unwrap()[0].restart);

    // Another job's run ending is not held either.
    let other = setup(true).await;
    let other_id = waiting(&other).await;
    assert!(other
        .screens
        .unbind(&other_id, "run-1", RUN_ENDED, 5_500)
        .await
        .unwrap());
}

#[tokio::test]
async fn a_restart_of_a_live_run_ends_it_and_puts_the_job_back_in_line_with_its_own_event() {
    let s = setup(true).await;
    *s.browser.store.lock().unwrap() = Some(s.store.clone());
    let id = waiting(&s).await;
    tend(&s).await;
    let bound = shown(&s, &id).await.bound_at.unwrap();
    assert!(s
        .screens
        .request_restart(&id, "run-1", bound, 5_000)
        .await
        .unwrap());

    let woken = s.wake.notified();
    tend(&s).await;
    tokio::time::timeout(Duration::from_secs(1), woken)
        .await
        .expect("the worker is woken");
    // The run was ended, and only then was the job put back in line.
    assert_eq!(*s.browser.released.lock().unwrap(), vec![id.clone()]);
    assert_eq!(
        *s.browser.states_at_release.lock().unwrap(),
        vec![JobState::Waiting]
    );
    assert!(s.browser.touched.lock().unwrap().is_empty());
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Pending);
    assert!(d.events.iter().any(|e| e.message == RESTARTED_CHECK));
    assert!(!d.events.iter().any(|e| e.message == CHECK_PREPARED_AGAIN));
    assert_eq!(shown(&s, &id).await.state, ScreenState::Preparing);
    assert!(s.screens.prepare_requests().await.unwrap().is_empty());

    // The run that was watched ends with the release; the person is told of
    // no run that closed by itself.
    s.browser.give(Next::Ended);
    watch_ended(&s).await;
    let d = detail(&s, &id).await;
    assert!(!d.events.iter().any(|e| e.message.contains("닫혔어요")));
    assert_eq!(shown(&s, &id).await.note, None);

    // The job's next run opens the post in a new run, bound to the screen.
    run(&s).await;
    assert_eq!(s.browser.prepares(), 2);
    let screen = shown(&s, &id).await;
    assert_eq!(screen.state, ScreenState::Ready);
    assert_eq!(screen.run_id.as_deref(), Some("run-2"));
    assert!(s.screens.prepare_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_restart_waits_for_a_download_on_its_way_and_is_answered_once_it_ended() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    tend(&s).await;
    let bound = shown(&s, &id).await.bound_at.unwrap();
    s.browser.under_way.store(true, Ordering::SeqCst);
    assert!(s
        .screens
        .request_restart(&id, "run-1", bound, 5_000)
        .await
        .unwrap());

    for _ in 0..3 {
        tend(&s).await;
    }
    assert!(s.browser.released.lock().unwrap().is_empty());
    assert!(s.browser.touched.lock().unwrap().is_empty());
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Waiting);
    assert!(!d.events.iter().any(|e| e.message == RESTARTED_CHECK));
    let asked = s.screens.prepare_requests().await.unwrap();
    assert!(asked.len() == 1 && asked[0].restart);
    assert_eq!(shown(&s, &id).await.run_id.as_deref(), Some("run-1"));

    s.browser.under_way.store(false, Ordering::SeqCst);
    tend(&s).await;
    assert_eq!(*s.browser.released.lock().unwrap(), vec![id.clone()]);
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Pending);
    assert!(d.events.iter().any(|e| e.message == RESTARTED_CHECK));
    assert!(s.screens.prepare_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_restart_of_a_run_that_is_gone_puts_the_job_back_in_line_without_ending_anything() {
    let s = setup(true).await;
    let id = waiting(&s).await;
    let bound = shown(&s, &id).await.bound_at.unwrap();
    // The run went away while no one looked.
    s.browser.live.lock().unwrap().clear();
    assert!(s
        .screens
        .request_restart(&id, "run-1", bound, 5_000)
        .await
        .unwrap());

    tend(&s).await;
    assert!(s.browser.released.lock().unwrap().is_empty());
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Pending);
    assert!(d.events.iter().any(|e| e.message == RESTARTED_CHECK));
    // The watch that found the run gone did not report it as closed.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let d = detail(&s, &id).await;
    assert!(!d.events.iter().any(|e| e.message.contains("닫혔어요")));
    assert_eq!(shown(&s, &id).await.note, None);
    run(&s).await;
    assert_eq!(s.browser.prepares(), 2);
    assert_eq!(shown(&s, &id).await.state, ScreenState::Ready);
}
