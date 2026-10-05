//! A find job (직접 찾기, `docs/specs/subtitles.md` §직접 찾기와 자막 올리기):
//! the server browser opens the chosen creator's newest post, a person browses
//! it on the job's remote screen, and every download the run completes becomes
//! a file of the job's one package, judged as an upload's files are, until the
//! person finishes it. The browser is a fake that hands out what a person's
//! clicks would download; the real one is tested by `trss-subtitles`'s ignored
//! `find_sample`.

use std::{
    collections::{HashSet, VecDeque},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use trss_core::{Clock, Db, DbError};
use trss_jobs::{
    area::ReceiveArea,
    runner::{find::FIND_NOTE, NO_AUTH_BROWSER},
    screen::{FIND_PREPARED_AGAIN, RESTARTED_FIND, RUN_ENDED},
    store::JobDetail,
    upload::Kind,
    AskedFinish, Created, FileState, ItemState, JobState, JobStore, NewFind, Runner, ScreenState,
    ScreenStore, StepKind, StepState, Wait, FIND, NOTHING_FOUND,
};
use trss_subtitles::{
    auth::{AuthBrowser, AuthPage, BoxFuture, PrepareRequest, Prepared, Waited},
    fake::{self, FakeSource},
    Failure, FailureKind, Sources,
};

/// What the fake run gives the watch next.
enum Next {
    /// A completed download of `name` with these bytes.
    File(&'static str, Vec<u8>),
    NotTaken,
    Ended,
}

/// A browser whose run hands out downloads as a person's clicks would make
/// them. Its wait takes the next download in the poll it finds it, as the
/// pool's does, so dropping a wait loses nothing.
#[derive(Default)]
struct FindBrowser {
    prepared: Mutex<Vec<(String, String, bool)>>,
    live: Mutex<HashSet<String>>,
    released: Mutex<Vec<String>>,
    queue: Mutex<VecDeque<Next>>,
    /// A download under way in the browser.
    under_way: AtomicBool,
    /// The pages the run has open; the first is the one it was prepared
    /// with.
    pages: Mutex<Vec<String>>,
    /// The pages closed at a person's request.
    closed: Mutex<Vec<String>>,
    /// Every page the worker asked to close, whether it was closed.
    close_calls: Mutex<Vec<String>>,
    /// How many times a page was brought back to its check.
    rearms: AtomicUsize,
    /// The next preparation fails.
    fail_prepare: AtomicBool,
}

impl FindBrowser {
    fn give(&self, next: Next) {
        self.queue.lock().unwrap().push_back(next);
    }

    fn prepares(&self) -> Vec<(String, String, bool)> {
        self.prepared.lock().unwrap().clone()
    }
}

impl AuthBrowser for FindBrowser {
    fn prepare<'a>(
        &'a self,
        request: PrepareRequest<'a>,
    ) -> BoxFuture<'a, Result<Prepared, Failure>> {
        Box::pin(async move {
            if self.fail_prepare.swap(false, Ordering::SeqCst) {
                return Err(Failure::new(
                    FailureKind::Network,
                    "서버 브라우저에 빈자리가 나지 않았어요",
                ));
            }
            let mut prepared = self.prepared.lock().unwrap();
            prepared.push((
                request.job.to_owned(),
                request.post.to_string(),
                matches!(request.page, AuthPage::Browse),
            ));
            let run = format!("run-{}", prepared.len());
            let target = format!("target-{}", prepared.len());
            self.live.lock().unwrap().insert(run.clone());
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
            loop {
                if !self.live.lock().unwrap().contains(run_id) {
                    return Waited::Ended;
                }
                let next = self.queue.lock().unwrap().pop_front();
                match next {
                    Some(Next::File(name, bytes)) => {
                        std::fs::create_dir_all(staging).unwrap();
                        let path = staging.join(name);
                        std::fs::write(&path, bytes).unwrap();
                        return Waited::File {
                            name: name.to_owned(),
                            path,
                        };
                    }
                    Some(Next::NotTaken) => {
                        return Waited::NotTaken {
                            reason: "취소됐어요".to_owned(),
                        }
                    }
                    Some(Next::Ended) => {
                        self.live.lock().unwrap().remove(run_id);
                        return Waited::Ended;
                    }
                    None => tokio::time::sleep(Duration::from_millis(5)).await,
                }
            }
        })
    }

    fn is_live(&self, _job: &str, run_id: &str) -> bool {
        self.live.lock().unwrap().contains(run_id)
    }

    fn touch(&self, job: &str, run_id: &str) -> bool {
        self.is_live(job, run_id)
    }

    fn release<'a>(&'a self, job: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.released.lock().unwrap().push(job.to_owned());
            self.live.lock().unwrap().clear();
        })
    }

    fn downloading(&self, _job: &str, _run_id: &str) -> bool {
        self.under_way.load(Ordering::SeqCst) || !self.queue.lock().unwrap().is_empty()
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

    fn rearm<'a>(&'a self, _job: &'a str, _run_id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.rearms.fetch_add(1, Ordering::SeqCst);
        })
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    store: JobStore,
    screens: ScreenStore,
    runner: Runner,
    area: ReceiveArea,
    browser: Arc<FindBrowser>,
    wake: Arc<Notify>,
    shutdown: CancellationToken,
}

fn ticking_clock() -> Clock {
    let now = Arc::new(AtomicI64::new(1_000));
    Arc::new(move || now.fetch_add(10, Ordering::SeqCst))
}

async fn setup(with_browser: bool) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    db.run::<_, DbError, _>(|c| {
        c.execute(
            "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
             VALUES ('src-maker', 3441, '메이커', 1)",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let store = JobStore::new(db.clone());
    let area = ReceiveArea::in_app_data(dir.path());
    let browser = Arc::new(FindBrowser::default());
    let mut runner = Runner::new(
        store.clone(),
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
        screens: ScreenStore::new(db),
        runner,
        area,
        browser,
        wake: Arc::new(Notify::new()),
        shutdown: CancellationToken::new(),
    }
}

/// The creator's newest post: the fake blog's (`fake::BLOG_POSTS`).
const POST: &str = "https://fake.trss.invalid/blog/maker";

async fn make(s: &Setup) -> String {
    let find = NewFind {
        command_id: "find-1".to_owned(),
        request: r#"{"find":{}}"#.to_owned(),
        work_id: "w1".to_owned(),
        season: 1,
        anime_no: 3441,
        source_id: "src-maker".to_owned(),
        creator: "메이커".to_owned(),
        post_url: POST.to_owned(),
    };
    match s.store.create_find(find, 900).await.unwrap() {
        Created::Created(id) => id,
        other => panic!("created: {other:?}"),
    }
}

async fn run(s: &Setup) {
    s.runner.run_ready(&CancellationToken::new()).await.unwrap();
}

async fn tend(s: &Setup) {
    s.runner.tend_screens(&s.wake, &s.shutdown).await.unwrap();
}

async fn detail(s: &Setup, id: &str) -> JobDetail {
    s.store.detail(id).await.unwrap().unwrap()
}

fn step(d: &JobDetail, kind: StepKind) -> Option<StepState> {
    d.steps.iter().find(|s| s.step == kind).map(|s| s.state)
}

/// Waits until `check` holds, for up to `secs` seconds.
async fn until<F, Fut>(secs: u64, check: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..secs * 100 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("not in time");
}

/// A find job whose run is open on its screen and watched.
async fn browsing(s: &Setup) -> String {
    let id = make(s).await;
    run(s).await;
    tend(s).await;
    id
}

async fn kept(s: &Setup, id: &str) -> usize {
    detail(s, id).await.items[0].files.len()
}

#[tokio::test]
async fn a_find_job_opens_the_creators_post_and_keeps_what_a_persons_click_downloads() {
    let s = setup(true).await;
    let id = browsing(&s).await;

    // The browser opened the creator's newest post, clicking nothing.
    assert_eq!(
        s.browser.prepares(),
        vec![(id.clone(), POST.to_owned(), true)]
    );
    let d = detail(&s, &id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Auth))
    );
    assert_eq!(d.row.note.as_deref(), Some(FIND_NOTE));
    assert_eq!(d.row.origin, FIND);
    assert!(d.row.receiving);
    assert_eq!(d.row.creator.as_deref(), Some("메이커"));
    assert!(d.row.episodes.is_empty());
    assert_eq!(step(&d, StepKind::Open), Some(StepState::Done));
    assert_eq!(step(&d, StepKind::Receive), Some(StepState::Current));
    assert_eq!(step(&d, StepKind::Auth), None);
    let screen = s.screens.screen(&id).await.unwrap().unwrap();
    assert_eq!(screen.state, ScreenState::Ready);
    assert_eq!(screen.run_id.as_deref(), Some("run-1"));
    // A find job is no site's check to pass from the 할 일 cards.
    assert!(s.store.auth_waits().await.unwrap().is_empty());

    // A person went to a past post and clicked its attachment.
    s.browser
        .give(Next::File("maker-2.srt", fake::srt("maker-2")));
    until(2, || async { kept(&s, &id).await == 1 }).await;
    let d = detail(&s, &id).await;
    let file = &d.items[0].files[0];
    assert_eq!(file.name, "maker-2.srt");
    assert_eq!(file.state, FileState::Done);
    assert_eq!(file.kind, Some(Kind::Subtitle));
    let path = file.path.clone().unwrap();
    assert_eq!(path, format!("{}/maker-2.srt", ReceiveArea::job_dir(&id)));
    assert_eq!(
        std::fs::read(s.area.at(&path)).unwrap(),
        fake::srt("maker-2")
    );
    assert!(d
        .events
        .iter()
        .any(|e| e.message == "서버 브라우저가 받은 파일을 남겼어요"));
    // The job keeps waiting for more until a person finishes it.
    assert_eq!(d.row.state, JobState::Waiting);
}

#[tokio::test]
async fn two_downloads_are_two_files_of_one_package_and_finishing_hands_them_to_placement() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    s.browser
        .give(Next::File("maker-3.srt", fake::srt("maker-3")));
    s.browser
        .give(Next::File("maker-2.srt", fake::srt("maker-2")));
    until(2, || async { kept(&s, &id).await == 2 }).await;

    assert_eq!(
        s.store.ask_finish(&id, 9_000).await.unwrap(),
        AskedFinish::Asked
    );
    assert!(detail(&s, &id).await.row.finishing);
    // Receiving is over; the package goes on to its placement.
    until(3, || async {
        detail(&s, &id).await.row.state == JobState::Pending
    })
    .await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.finished_at, None);
    assert_eq!(d.items.len(), 1);
    assert_eq!(d.items[0].state, ItemState::Done);
    let names: Vec<_> = d.items[0].files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["maker-3.srt", "maker-2.srt"]);
    let upload = d.row.upload.unwrap();
    assert_eq!((upload.subtitles, upload.dropped), (2, 0));
    assert_eq!(d.row.note.as_deref(), Some("받은 파일: 자막 2개"));
    assert!(!d.row.finishing);
    assert_eq!(step(&d, StepKind::Receive), Some(StepState::Done));
    // The run goes, and the screen with it.
    assert_eq!(*s.browser.released.lock().unwrap(), vec![id.clone()]);
    assert!(s.screens.screen(&id).await.unwrap().is_none());
    // Asked again, it is the same.
    assert_eq!(
        s.store.ask_finish(&id, 9_500).await.unwrap(),
        AskedFinish::Ended
    );

    // The worker's next run plans the package and waits for a person to
    // confirm where its files go (배치 확인), keeping nothing yet.
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Placement))
    );
    assert_eq!(
        d.row.note.as_deref(),
        Some("자막 2개가 붙을 회차를 확인해 주세요")
    );
    assert_eq!(step(&d, StepKind::Placement), Some(StepState::Waiting));
    assert_eq!(step(&d, StepKind::Store), None);
    let plan = s.store.plan(&id).await.unwrap();
    assert_eq!(plan.len(), 2);
    assert!(plan.iter().all(|r| r.outcome.is_none()));
    assert!(!d.row.receiving);
    // Its 받기 ended once: the worker's look at the screens leaves a job
    // waiting for its 배치 확인 alone, though a person had asked to finish.
    tend(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.wait, Some(Wait::Placement));
    assert!(!d.row.finishing);
    let ended = d.events.iter().filter(|e| e.message == "받기를 끝냈어요");
    assert_eq!(ended.count(), 1);
    assert_eq!(
        s.store.ask_finish(&id, 9_900).await.unwrap(),
        AskedFinish::Ended
    );
}

#[tokio::test]
async fn finishing_with_nothing_received_ends_as_nothing_found() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    s.store.ask_finish(&id, 9_000).await.unwrap();
    until(3, || async {
        detail(&s, &id).await.row.state == JobState::Done
    })
    .await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.note.as_deref(), Some(NOTHING_FOUND));
    assert!(d.items[0].files.is_empty());
    assert!(d
        .events
        .iter()
        .any(|e| e.message == "받은 파일 없이 받기를 끝냈어요"));
}

#[tokio::test]
async fn finishing_waits_for_a_download_under_way_and_keeps_it() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    s.browser.under_way.store(true, Ordering::SeqCst);
    assert_eq!(
        s.store.ask_finish(&id, 9_000).await.unwrap(),
        AskedFinish::Asked
    );
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Waiting);

    // The download completes: it is the job's, then receiving ends.
    s.browser
        .give(Next::File("maker-1.srt", fake::srt("maker-1")));
    s.browser.under_way.store(false, Ordering::SeqCst);
    until(3, || async {
        detail(&s, &id).await.row.state == JobState::Pending
    })
    .await;
    let d = detail(&s, &id).await;
    assert_eq!(d.items[0].files.len(), 1);
    assert_eq!(d.row.note.as_deref(), Some("받은 파일: 자막 1개"));
}

/// The browser may move a download into the job's folder after the watch
/// saw none on its way and before the job's folder goes with its end.
#[tokio::test]
async fn a_download_that_lands_as_the_watch_ends_the_job_is_kept() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    // The watch is past taking what an earlier run left: it took this one.
    s.browser
        .give(Next::File("maker-2.srt", fake::srt("maker-2")));
    until(2, || async { kept(&s, &id).await == 1 }).await;
    let item = detail(&s, &id).await.items[0].id;

    s.browser.under_way.store(true, Ordering::SeqCst);
    assert_eq!(
        s.store.ask_finish(&id, 9_000).await.unwrap(),
        AskedFinish::Asked
    );
    // The download lands without the watch's wait reporting it, which had
    // given way to the finish, and none is on its way any more.
    let staging = s.area.at(&format!(".tmp/check-{id}-{item}"));
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("maker-1.srt"), fake::srt("maker-1")).unwrap();
    s.browser.under_way.store(false, Ordering::SeqCst);
    until(3, || async {
        detail(&s, &id).await.row.state != JobState::Waiting
    })
    .await;

    // It is the job's, which goes on to its 배치 확인.
    let d = detail(&s, &id).await;
    let names: Vec<_> = d.items[0].files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["maker-2.srt", "maker-1.srt"]);
    assert_eq!(d.row.state, JobState::Pending);
    assert_eq!(d.row.note.as_deref(), Some("받은 파일: 자막 2개"));
    let path = d.items[0].files[1].path.clone().unwrap();
    assert_eq!(
        std::fs::read(s.area.at(&path)).unwrap(),
        fake::srt("maker-1")
    );
    assert!(!staging.exists());
}

#[tokio::test]
async fn a_download_that_is_no_subtitle_is_dropped_with_why() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    s.browser
        .give(Next::File("maker-3.txt", b"just a note\n".to_vec()));
    s.browser.give(Next::NotTaken);
    until(2, || async { !detail(&s, &id).await.dropped.is_empty() }).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.dropped[0].name, "maker-3.txt");
    assert_eq!(d.dropped[0].reason, trss_subtitles::upload::NOT_THEM);
    assert!(d.items[0].files.is_empty());
    let staging = s.area.at(&format!(".tmp/check-{id}-{}", d.items[0].id));
    let left: Vec<_> = std::fs::read_dir(&staging)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    assert!(left.is_empty(), "left in staging: {left:?}");
    until(2, || async {
        detail(&s, &id)
            .await
            .events
            .iter()
            .any(|e| e.message == "브라우저가 파일을 받지 못했어요")
    })
    .await;

    s.store.ask_finish(&id, 9_000).await.unwrap();
    until(3, || async {
        detail(&s, &id).await.row.state == JobState::Done
    })
    .await;
    let d = detail(&s, &id).await;
    assert_eq!(
        d.row.note.as_deref(),
        Some(format!("{NOTHING_FOUND} · 뺀 파일 1개").as_str())
    );
    assert_eq!(d.row.upload.unwrap().dropped, 1);
}

#[tokio::test]
async fn an_idle_end_closes_the_run_keeps_the_files_and_a_reopening_opens_the_same_post() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    s.browser
        .give(Next::File("maker-3.srt", fake::srt("maker-3")));
    s.browser.give(Next::Ended);
    until(2, || async {
        s.screens.screen(&id).await.unwrap().unwrap().state == ScreenState::Closed
    })
    .await;
    let screen = s.screens.screen(&id).await.unwrap().unwrap();
    assert_eq!(screen.run_id, None);
    assert_eq!(screen.note.as_deref(), Some(RUN_ENDED));
    let d = detail(&s, &id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Auth))
    );
    assert_eq!(d.items[0].files.len(), 1);
    let path = d.items[0].files[0].path.clone().unwrap();
    assert!(s.area.at(&path).exists());

    // A person opens the screen again: a new run opens the same post.
    s.screens
        .request_prepare(&id, 5_000)
        .await
        .unwrap()
        .unwrap();
    tend(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Pending);
    assert!(d.events.iter().any(|e| e.message == FIND_PREPARED_AGAIN));
    run(&s).await;
    tend(&s).await;
    let prepares = s.browser.prepares();
    assert_eq!(prepares.len(), 2);
    assert_eq!(prepares[1].1, POST);
    let screen = s.screens.screen(&id).await.unwrap().unwrap();
    assert_eq!(screen.run_id.as_deref(), Some("run-2"));

    // A download of the new run joins the same package.
    s.browser
        .give(Next::File("maker-2.srt", fake::srt("maker-2")));
    until(2, || async { kept(&s, &id).await == 2 }).await;
    assert!(s.area.at(&path).exists());
}

#[tokio::test]
async fn a_restart_ends_the_stuck_run_after_its_download_and_opens_the_same_post_in_a_new_one() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    let bound = shown(&s, &id).await.bound_at.unwrap();

    // A download on its way is not lost to the restart: it waits.
    s.browser.under_way.store(true, Ordering::SeqCst);
    assert!(s
        .screens
        .request_restart(&id, "run-1", bound, 5_000)
        .await
        .unwrap());
    for _ in 0..3 {
        tend(&s).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(s.browser.released.lock().unwrap().is_empty());
    assert_eq!(detail(&s, &id).await.row.state, JobState::Waiting);
    let asked = s.screens.prepare_requests().await.unwrap();
    assert!(asked.len() == 1 && asked[0].restart && asked[0].find);

    s.browser.under_way.store(false, Ordering::SeqCst);
    tend(&s).await;
    assert_eq!(*s.browser.released.lock().unwrap(), vec![id.clone()]);
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Pending);
    assert!(d.events.iter().any(|e| e.message == RESTARTED_FIND));
    assert!(!d.events.iter().any(|e| e.message == FIND_PREPARED_AGAIN));
    // The run that ended is not reported as one that closed by itself.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let d = detail(&s, &id).await;
    assert!(!d.events.iter().any(|e| e.message.contains("닫혔어요")));
    assert_eq!(shown(&s, &id).await.note, None);

    // The job's next run opens the same post in a new run.
    run(&s).await;
    tend(&s).await;
    let prepares = s.browser.prepares();
    assert_eq!(prepares.len(), 2);
    assert_eq!(prepares[1].1, POST);
    let screen = shown(&s, &id).await;
    assert_eq!(screen.state, ScreenState::Ready);
    assert_eq!(screen.run_id.as_deref(), Some("run-2"));
}

#[tokio::test]
async fn finishing_a_job_whose_run_closed_is_ended_by_the_workers_next_look() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    s.browser
        .give(Next::File("maker-3.srt", fake::srt("maker-3")));
    s.browser.give(Next::Ended);
    until(2, || async {
        s.screens.screen(&id).await.unwrap().unwrap().state == ScreenState::Closed
    })
    .await;
    // The web only asks: the job is finishing until the worker ends it.
    assert_eq!(
        s.store.ask_finish(&id, 9_000).await.unwrap(),
        AskedFinish::Asked
    );
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Waiting);
    assert!(d.row.finishing);
    tend(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Pending);
    assert!(!d.row.finishing);
    assert_eq!(d.row.note.as_deref(), Some("받은 파일: 자막 1개"));
}

#[tokio::test]
async fn a_finish_asked_while_the_run_was_bound_ends_the_job_once_the_run_ends() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    // A download under way holds the watch's finish back.
    s.browser.under_way.store(true, Ordering::SeqCst);
    assert_eq!(
        s.store.ask_finish(&id, 9_000).await.unwrap(),
        AskedFinish::Asked
    );
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Waiting);
    // The run ends (idle) before the download did: no watch is left to end it.
    s.browser.give(Next::Ended);
    until(2, || async {
        s.screens.screen(&id).await.unwrap().unwrap().state == ScreenState::Closed
    })
    .await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Waiting);
    tend(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(d.row.note.as_deref(), Some(NOTHING_FOUND));
    assert!(s.screens.screen(&id).await.unwrap().is_none());
}

#[tokio::test]
async fn a_file_a_restart_left_in_the_folder_is_received_when_the_job_is_finished() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    let item = detail(&s, &id).await.items[0].id;
    // The worker stops: its watch ends with the run and leaves the binding,
    // which the next start clears. The browser had moved a download into the
    // job's folder that the watch did not take.
    s.shutdown.cancel();
    s.browser.give(Next::Ended);
    until(2, || async { !s.browser.is_live(&id, "run-1") }).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(s.screens.unbind_all(8_000).await.unwrap(), 1);
    let staging = s.area.at(&format!(".tmp/check-{id}-{item}"));
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("maker-1.srt"), fake::srt("maker-1")).unwrap();
    std::fs::write(staging.join(".answer"), b"{}").unwrap();

    // A person finishes before reopening the screen.
    assert_eq!(
        s.store.ask_finish(&id, 9_000).await.unwrap(),
        AskedFinish::Asked
    );
    assert_eq!(detail(&s, &id).await.row.state, JobState::Waiting);
    tend(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Pending);
    assert_eq!(d.row.note.as_deref(), Some("받은 파일: 자막 1개"));
    assert_eq!(d.items[0].files[0].name, "maker-1.srt");
    assert!(!staging.exists());
}

#[tokio::test]
async fn two_takers_of_the_same_left_files_take_each_once() {
    let s = setup(false).await;
    let id = make(&s).await;
    run(&s).await;
    let item = detail(&s, &id).await.items[0].id;
    let staging = s.area.at(&format!(".tmp/check-{id}-{item}"));
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("maker-1.srt"), fake::srt("maker-1")).unwrap();
    std::fs::write(staging.join("maker-2.srt"), fake::srt("maker-2")).unwrap();
    s.store.ask_finish(&id, 9_000).await.unwrap();

    // Two looks at once both find the job to end.
    let (a, b) = tokio::join!(
        s.runner.tend_screens(&s.wake, &s.shutdown),
        s.runner.tend_screens(&s.wake, &s.shutdown)
    );
    a.unwrap();
    b.unwrap();
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Pending);
    let mut names: Vec<_> = d.items[0].files.iter().map(|f| f.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["maker-1.srt", "maker-2.srt"]);
    let unsaid: Vec<_> = d
        .events
        .iter()
        .filter(|e| e.message.contains("못했어요"))
        .map(|e| e.message.clone())
        .collect();
    assert!(unsaid.is_empty(), "{unsaid:?}");
}

#[tokio::test]
async fn a_finish_after_the_screen_failed_to_open_leaves_no_step_waiting_nor_folder() {
    let s = setup(true).await;
    s.browser.fail_prepare.store(true, Ordering::SeqCst);
    let id = make(&s).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(step(&d, StepKind::Open), Some(StepState::Waiting));
    let item = d.items[0].id;
    let staging = s.area.at(&format!(".tmp/check-{id}-{item}"));
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join(".answer"), b"{}").unwrap();

    s.store.ask_finish(&id, 9_000).await.unwrap();
    tend(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    // No run opened the post: the step it never got through is not shown.
    assert_eq!(step(&d, StepKind::Open), None);
    assert_eq!(step(&d, StepKind::Receive), Some(StepState::Done));
    assert!(!staging.exists());
}

#[tokio::test]
async fn a_finish_after_a_later_screen_failed_to_open_keeps_the_post_opened() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    s.browser.give(Next::Ended);
    until(2, || async {
        s.screens.screen(&id).await.unwrap().unwrap().state == ScreenState::Closed
    })
    .await;
    // Reopened, the next run cannot be prepared.
    s.screens.request_prepare(&id, 5_000).await.unwrap();
    tend(&s).await;
    s.browser.fail_prepare.store(true, Ordering::SeqCst);
    run(&s).await;
    assert_eq!(
        step(&detail(&s, &id).await, StepKind::Open),
        Some(StepState::Waiting)
    );
    s.store.ask_finish(&id, 9_000).await.unwrap();
    tend(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    let open = d.steps.iter().find(|s| s.step == StepKind::Open).unwrap();
    assert_eq!((open.state, open.note.as_deref()), (StepState::Done, None));
}

#[tokio::test]
async fn a_download_a_restart_left_in_the_folder_is_taken_by_the_next_watch() {
    let s = setup(true).await;
    let id = make(&s).await;
    run(&s).await;
    let item = detail(&s, &id).await.items[0].id;
    let staging = s.area.at(&format!(".tmp/check-{id}-{item}"));
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("maker-1.srt"), fake::srt("maker-1")).unwrap();
    tend(&s).await;
    until(2, || async { kept(&s, &id).await == 1 }).await;
    assert!(!staging.join("maker-1.srt").exists());
}

#[tokio::test]
async fn the_screen_follows_a_page_the_post_opens_and_comes_back_when_it_closes() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    let first = s.screens.screen(&id).await.unwrap().unwrap();
    s.browser.pages.lock().unwrap().push("popup-1".to_owned());
    until(5, || async {
        s.screens
            .screen(&id)
            .await
            .unwrap()
            .unwrap()
            .target_id
            .as_deref()
            == Some("popup-1")
    })
    .await;
    let followed = s.screens.screen(&id).await.unwrap().unwrap();
    // A new binding of the same run: screens on the old page reconnect.
    assert_eq!(followed.run_id, first.run_id);
    assert!(followed.bound_at > first.bound_at);

    s.browser.pages.lock().unwrap().retain(|p| p != "popup-1");
    until(5, || async {
        s.screens
            .screen(&id)
            .await
            .unwrap()
            .unwrap()
            .target_id
            .as_deref()
            == Some("target-1")
    })
    .await;
}

async fn shown(s: &Setup, id: &str) -> trss_jobs::Screen {
    s.screens.screen(id).await.unwrap().unwrap()
}

#[tokio::test]
async fn a_person_closes_a_popup_and_the_screen_goes_back_to_the_post() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    let first = shown(&s, &id).await;
    assert!(!first.popup);
    let run_id = first.run_id.clone().unwrap();
    // The post's own page is no popup to close.
    assert!(!s
        .screens
        .request_close(&id, &run_id, first.bound_at.unwrap(), None, 2_000)
        .await
        .unwrap());

    s.browser.pages.lock().unwrap().push("popup-1".to_owned());
    until(5, || async {
        shown(&s, &id).await.target_id.as_deref() == Some("popup-1")
    })
    .await;
    let popup = shown(&s, &id).await;
    assert!(popup.popup);
    // A request made on the binding before is not this page's.
    assert!(!s
        .screens
        .request_close(&id, &run_id, first.bound_at.unwrap(), None, 3_000)
        .await
        .unwrap());
    assert!(s
        .screens
        .request_close(&id, &run_id, popup.bound_at.unwrap(), None, 3_000)
        .await
        .unwrap());
    until(5, || async {
        shown(&s, &id).await.target_id.as_deref() == Some("target-1")
    })
    .await;
    assert_eq!(
        *s.browser.closed.lock().unwrap(),
        vec!["popup-1".to_owned()]
    );
    assert_eq!(
        *s.browser.pages.lock().unwrap(),
        vec!["target-1".to_owned()]
    );
    let back = shown(&s, &id).await;
    assert!(!back.popup);
    assert!(back.bound_at > popup.bound_at);
}

/// Waits until the screen shows `target`, for up to five seconds.
async fn shows(s: &Setup, id: &str, target: &str) {
    until(5, || async {
        shown(s, id).await.target_id.as_deref() == Some(target)
    })
    .await;
}

#[tokio::test]
async fn a_persons_choice_of_an_older_tab_is_kept_and_a_new_popup_is_shown_after_it() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    s.browser.pages.lock().unwrap().push("popup-1".to_owned());
    shows(&s, &id, "popup-1").await;
    let popup = shown(&s, &id).await;
    let run_id = popup.run_id.clone().unwrap();
    assert_eq!(
        popup.pages,
        vec!["target-1".to_owned(), "popup-1".to_owned()]
    );

    assert!(s
        .screens
        .request_switch(&id, &run_id, popup.bound_at.unwrap(), "target-1", 5_000)
        .await
        .unwrap());
    shows(&s, &id, "target-1").await;
    let chosen = shown(&s, &id).await;
    // The follower looks again and again: the choice stays.
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let kept = shown(&s, &id).await;
    assert_eq!(kept.target_id.as_deref(), Some("target-1"));
    assert_eq!(kept.bound_at, chosen.bound_at);

    // A new window that stays is shown, though the person chose another.
    s.browser.pages.lock().unwrap().push("popup-2".to_owned());
    shows(&s, &id, "popup-2").await;
    assert_eq!(
        shown(&s, &id).await.pages,
        vec![
            "target-1".to_owned(),
            "popup-1".to_owned(),
            "popup-2".to_owned()
        ]
    );
}

#[tokio::test]
async fn closing_a_hidden_tab_keeps_the_screen_and_closing_the_shown_one_shows_the_newest_left() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    for popup in ["popup-1", "popup-2"] {
        s.browser.pages.lock().unwrap().push(popup.to_owned());
        shows(&s, &id, popup).await;
    }
    let second = shown(&s, &id).await;
    let run_id = second.run_id.clone().unwrap();
    assert_eq!(second.pages.len(), 3);

    // `popup-1` is hidden: only it closes, and the screen stays on `popup-2`.
    assert!(s
        .screens
        .request_close(
            &id,
            &run_id,
            second.bound_at.unwrap(),
            Some("popup-1"),
            5_000
        )
        .await
        .unwrap());
    until(5, || async {
        *s.browser.closed.lock().unwrap() == vec!["popup-1".to_owned()]
    })
    .await;
    until(5, || async { shown(&s, &id).await.pages.len() == 2 }).await;
    let after = shown(&s, &id).await;
    assert_eq!(after.target_id.as_deref(), Some("popup-2"));
    assert_eq!(after.bound_at, second.bound_at);

    // The shown one closes: the newest page left is the post's own.
    assert!(s
        .screens
        .request_close(&id, &run_id, after.bound_at.unwrap(), None, 6_000)
        .await
        .unwrap());
    shows(&s, &id, "target-1").await;
    assert_eq!(
        *s.browser.closed.lock().unwrap(),
        vec!["popup-1".to_owned(), "popup-2".to_owned()]
    );
    assert_eq!(shown(&s, &id).await.pages, vec!["target-1".to_owned()]);
}

#[tokio::test]
async fn the_worker_never_asks_the_browser_to_close_the_first_page() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    let first = shown(&s, &id).await;
    let run_id = first.run_id.clone().unwrap();
    // The web does not write such a request; one that reaches the row anyway
    // (written by hand here) is not asked of the browser.
    s.store
        .db()
        .run::<_, DbError, _>(|c| {
            Ok(c.execute(
                "UPDATE subtitle_job_screens SET close_target_id = 'target-1 popup-9'",
                [],
            )?)
        })
        .await
        .unwrap();
    until(5, || async {
        s.browser
            .close_calls
            .lock()
            .unwrap()
            .contains(&"popup-9".to_owned())
    })
    .await;
    assert_eq!(
        *s.browser.close_calls.lock().unwrap(),
        vec!["popup-9".to_owned()]
    );
    assert!(s.screens.take_close(&id, &run_id).await.unwrap().is_empty());
    assert_eq!(shown(&s, &id).await.target_id.as_deref(), Some("target-1"));
}

#[tokio::test]
async fn a_page_that_opens_and_closes_at_once_is_not_followed() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    let first = shown(&s, &id).await;
    // Open for less than one look of the follower: seen once at most.
    s.browser.pages.lock().unwrap().push("blink".to_owned());
    tokio::time::sleep(Duration::from_millis(900)).await;
    s.browser.pages.lock().unwrap().retain(|p| p != "blink");
    tokio::time::sleep(Duration::from_millis(2_200)).await;
    let now = shown(&s, &id).await;
    assert_eq!(now.target_id.as_deref(), Some("target-1"));
    assert_eq!(now.bound_at, first.bound_at);
}

#[tokio::test]
async fn opening_a_find_jobs_screen_brings_no_check_back() {
    let s = setup(true).await;
    let id = browsing(&s).await;
    s.screens.request_prepare(&id, 5_000).await.unwrap();
    tend(&s).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(s.browser.rearms.load(Ordering::SeqCst), 0);
    // The live run answered the request as use.
    let screen = shown(&s, &id).await;
    assert_eq!(screen.state, ScreenState::Ready);
    assert!(s.screens.prepare_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn without_a_server_browser_a_find_job_waits_and_the_worker_ends_it_when_finished() {
    let s = setup(false).await;
    let id = make(&s).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Subtitle))
    );
    assert_eq!(d.row.note.as_deref(), Some(NO_AUTH_BROWSER));
    assert_eq!(
        s.store.ask_finish(&id, 9_000).await.unwrap(),
        AskedFinish::Asked
    );
    assert!(detail(&s, &id).await.row.finishing);
    // A worker with no server browser still looks at the screens for this.
    tend(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(d.row.note.as_deref(), Some(NOTHING_FOUND));
}

#[tokio::test]
async fn a_finish_asked_before_the_run_opened_ends_the_job_without_opening_it() {
    let s = setup(true).await;
    let id = make(&s).await;
    assert_eq!(
        s.store.ask_finish(&id, 950).await.unwrap(),
        AskedFinish::Asked
    );
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(d.row.note.as_deref(), Some(NOTHING_FOUND));
    assert!(s.browser.prepares().is_empty());
}

#[tokio::test]
async fn a_find_job_command_is_made_once_and_other_jobs_cannot_be_finished() {
    let s = setup(true).await;
    let id = make(&s).await;
    let again = NewFind {
        command_id: "find-1".to_owned(),
        request: r#"{"find":{}}"#.to_owned(),
        work_id: "w1".to_owned(),
        season: 1,
        anime_no: 3441,
        source_id: "src-maker".to_owned(),
        creator: "메이커".to_owned(),
        post_url: POST.to_owned(),
    };
    assert_eq!(
        s.store.create_find(again.clone(), 1_000).await.unwrap(),
        Created::Existing(id.clone())
    );
    let other = NewFind {
        request: r#"{"find":{"other":1}}"#.to_owned(),
        ..again
    };
    assert_eq!(
        s.store.create_find(other, 1_000).await.unwrap(),
        Created::Mismatch(id)
    );
    assert_eq!(
        s.store.ask_finish("nope", 1_000).await.unwrap(),
        AskedFinish::Missing
    );
}

/// A watch's finish never ends a job a run of it holds (`running`): the run
/// would write the job's wait over the ended 받기, and the job would wait
/// for its check with no screen, which nothing ends again. The worker
/// watches only jobs that wait for their check; the store keeps the order
/// itself, in the order `run_find` writes.
#[tokio::test]
async fn a_watchs_finish_waits_for_the_run_that_bound_it_to_settle_the_job() {
    let s = setup(true).await;
    let id = make(&s).await;
    let item = detail(&s, &id).await.items[0].id;
    // The job's run, as `run_find` writes it: claimed, bound, its item
    // waiting for the person.
    s.store.claim_next(950).await.unwrap().unwrap();
    s.screens
        .bind(&id, item, "run-1", "t-1", 960)
        .await
        .unwrap();
    s.store
        .set_item(
            item,
            ItemState::Waiting,
            Some(Wait::Auth),
            Some(FIND_NOTE.to_owned()),
            970,
        )
        .await
        .unwrap();
    assert_eq!(
        s.store.ask_finish(&id, 975).await.unwrap(),
        AskedFinish::Asked
    );
    // The watch's finish comes before the run settled the job.
    assert!(!s.store.end_find(&id, Some("run-1"), 980).await.unwrap());
    s.store
        .settle(
            &id,
            JobState::Waiting,
            Some(Wait::Auth),
            Some(FIND_NOTE.to_owned()),
            990,
        )
        .await
        .unwrap();
    let d = detail(&s, &id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Auth))
    );
    assert!(d.row.receiving && d.row.finishing);
    // The next watch of the run ends it.
    assert!(s.store.end_find(&id, Some("run-1"), 1_000).await.unwrap());
    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(d.row.note.as_deref(), Some(NOTHING_FOUND));
    assert!(!d.row.receiving && !d.row.finishing);
}
