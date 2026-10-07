//! The Anissia client against a fake server ([`super::fake`]): the pace, the
//! cap, the cache, and the answers that are not the API's. Nothing here
//! reaches the real Anissia.

use std::{
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use super::{fake::Fake, *};

const DAY: i64 = 24 * 60 * 60 * 1000;

struct Env {
    db: Db,
    fake: Fake,
    anissia: Anissia,
    now: Arc<AtomicI64>,
}

impl Env {
    /// A client with no spacing between requests and a clock the test sets.
    async fn new() -> Env {
        let db = Db::open_blocking(":memory:").unwrap();
        let fake = Fake::start().await;
        let now = Arc::new(AtomicI64::new(10 * DAY));
        let clock: Clock = {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        };
        let anissia = Anissia::new(db.clone(), fake.config(), clock).with_spacing(Duration::ZERO);
        Env {
            db,
            fake,
            anissia,
            now,
        }
    }

    fn advance(&self, ms: i64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
    }
}

#[test]
fn the_base_url_comes_from_the_environment_and_a_bad_one_is_refused() {
    let none = AnissiaConfig::from_lookup(|_| None).unwrap();
    assert_eq!(none.base_url.as_str(), "https://api.anissia.net/");
    let blank = AnissiaConfig::from_lookup(|_| Some("  ".into())).unwrap();
    assert_eq!(blank, none);
    let local =
        AnissiaConfig::from_lookup(|k| (k == URL_VAR).then(|| "http://127.0.0.1:9/".into()))
            .unwrap();
    assert_eq!(
        local.url("/anime/schedule/3"),
        "http://127.0.0.1:9/anime/schedule/3"
    );
    assert!(AnissiaConfig::from_lookup(|_| Some("ftp://x.test".into())).is_err());
    assert!(AnissiaConfig::from_lookup(|_| Some("not a url".into())).is_err());
}

#[tokio::test]
async fn a_schedule_and_the_captions_of_an_anime_are_read_from_the_api() {
    let env = Env::new().await;
    env.fake
        .set_week(3, vec![env.fake.entry(3, 3320, "22:00", "작품", "原題")]);
    env.fake.set_captions(
        3320,
        vec![
            env.fake.caption("1", "2026-10-08T01:00:00", "에텔레로사"),
            env.fake.caption("2", "2026-10-15T01:00:00", "에텔레로사"),
        ],
    );

    let schedule = env.anissia.schedule(3, None).await.unwrap();
    assert!(!schedule.cached);
    assert_eq!(schedule.value.len(), 1);
    assert_eq!(schedule.value[0].anime_no, 3320);
    assert_eq!(schedule.value[0].week, 3);
    assert_eq!(schedule.value[0].air_time.as_deref(), Some("22:00"));

    let captions = env.anissia.captions(3320, None).await.unwrap();
    assert_eq!(captions.value.len(), 2);
    let creators = parse::creators(&captions.value);
    assert_eq!(creators.len(), 1);
    assert_eq!(creators[0].name, "에텔레로사");
    assert_eq!(creators[0].captions, 2);

    // An anime nobody subtitles has an empty list, not an error.
    assert!(env
        .anissia
        .captions(99, None)
        .await
        .unwrap()
        .value
        .is_empty());
    // The weeks are 0 to 8.
    assert!(matches!(
        env.anissia.schedule(9, None).await,
        Err(AnissiaError::NoSuchWeek(9))
    ));
    assert_eq!(env.fake.count("/anime/schedule/9"), 0);
}

#[tokio::test]
async fn an_answer_is_cached_for_five_minutes_and_a_failure_is_not() {
    let env = Env::new().await;
    env.fake
        .set_week(1, vec![env.fake.entry(1, 1, "01:00", "작품", "")]);

    // A failure is not remembered: the next call asks again.
    env.fake.state.lock().unwrap().failing = 1;
    assert!(matches!(
        env.anissia.schedule(1, None).await,
        Err(AnissiaError::Status(500))
    ));
    let first = env.anissia.schedule(1, None).await.unwrap();
    assert!(!first.cached);

    env.advance(CACHE_TTL.as_millis() as i64 - 1);
    let again = env.anissia.schedule(1, None).await.unwrap();
    assert!(again.cached);
    assert_eq!(again.fetched_at, first.fetched_at);
    assert_eq!(env.fake.count("/anime/schedule/1"), 2);

    env.advance(1);
    let later = env.anissia.schedule(1, None).await.unwrap();
    assert!(!later.cached);
    assert_eq!(env.fake.count("/anime/schedule/1"), 3);

    // Another week is another entry.
    env.anissia.schedule(2, None).await.unwrap();
    assert_eq!(env.fake.count("/anime/schedule/2"), 1);
}

#[tokio::test]
async fn two_calls_at_once_ask_anissia_once() {
    let env = Env::new().await;
    env.fake
        .set_week(4, vec![env.fake.entry(4, 1, "01:00", "작품", "")]);
    let (a, b) = tokio::join!(env.anissia.schedule(4, None), env.anissia.schedule(4, None));
    a.unwrap();
    b.unwrap();
    assert_eq!(env.fake.count("/anime/schedule/4"), 1);
}

#[tokio::test]
async fn requests_keep_their_spacing_across_clients_sharing_a_database() {
    let env = Env::new().await;
    // The web and the worker are two clients over one database.
    let pace = Duration::from_millis(300);
    let web = env.anissia.clone().with_spacing(pace);
    let worker = env.anissia.clone().with_spacing(pace);
    assert!(web.fetch_schedule(1, None).await.unwrap().is_empty());
    // The clock stands still, so the worker's turn is one spacing away: a
    // caller that may wait less is told so without a request being sent.
    match worker
        .fetch_schedule(2, Some(Duration::from_millis(100)))
        .await
    {
        Err(AnissiaError::Busy { retry_after }) => assert_eq!(retry_after, pace),
        other => panic!("expected Busy, got {other:?}"),
    }
    assert_eq!(env.fake.count("/anime/schedule/2"), 0);
    // A spacing later it has the turn.
    env.advance(300);
    assert!(worker
        .fetch_schedule(2, Some(Duration::from_millis(100)))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_caller_that_may_wait_waits_for_its_turn_in_real_time() {
    let db = Db::open_blocking(":memory:").unwrap();
    let fake = Fake::start().await;
    let anissia =
        Anissia::with_defaults(db, fake.config()).with_spacing(Duration::from_millis(300));
    anissia.fetch_schedule(1, None).await.unwrap();
    anissia.fetch_schedule(2, None).await.unwrap();
    let times: Vec<_> = fake.requests().into_iter().map(|(at, _)| at).collect();
    let gap = times[1] - times[0];
    assert!(gap >= Duration::from_millis(250), "{gap:?}");
}

#[tokio::test]
async fn a_block_that_comes_while_a_request_waits_for_its_turn_stops_the_request() {
    let env = Env::new().await;
    // The test's clock stays put while real time passes.
    let anissia = env.anissia.clone().with_spacing(Duration::from_millis(600));
    let now = anissia.now();
    // Another request holds the turn before the test's.
    assert_eq!(
        anissia
            .pace
            .take_request_slot(now, 600, None)
            .await
            .unwrap(),
        Ok(now)
    );
    let blocker = tokio::spawn({
        let pace = anissia.pace.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            pace.block_requests(now + 10_000).await.unwrap();
        }
    });
    let answer = anissia.fetch_schedule(1, None).await;
    blocker.await.unwrap();
    match answer {
        Err(AnissiaError::Busy { retry_after }) => {
            assert_eq!(retry_after, Duration::from_secs(10))
        }
        other => panic!("expected Busy, got {other:?}"),
    }
    assert!(env.fake.requests().is_empty());
}

#[tokio::test]
async fn a_429_blocks_every_request_until_its_retry_after_has_passed() {
    let env = Env::new().await;
    env.fake
        .set_week(1, vec![env.fake.entry(1, 1, "01:00", "작품", "")]);
    {
        let mut state = env.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = Some(30);
    }
    match env.anissia.fetch_schedule(1, None).await {
        Err(AnissiaError::Busy { retry_after }) => {
            assert_eq!(retry_after, Duration::from_secs(30))
        }
        other => panic!("expected Busy, got {other:?}"),
    }
    // The block holds the next request back without sending it, for another
    // week too.
    match env
        .anissia
        .fetch_schedule(2, Some(Duration::from_secs(1)))
        .await
    {
        Err(AnissiaError::Busy { retry_after }) => {
            assert_eq!(retry_after, Duration::from_secs(30))
        }
        other => panic!("expected Busy, got {other:?}"),
    }
    assert_eq!(env.fake.requests().len(), 1);

    env.advance(30_000);
    assert_eq!(
        env.anissia
            .fetch_schedule(1, Some(Duration::from_secs(1)))
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(env.fake.requests().len(), 2);
}

#[tokio::test]
async fn a_429_without_a_retry_after_waits_a_minute_and_a_huge_one_is_cut_to_an_hour() {
    let env = Env::new().await;
    {
        let mut state = env.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = None;
    }
    match env.anissia.fetch_schedule(1, None).await {
        Err(AnissiaError::Busy { retry_after }) => {
            assert_eq!(retry_after, Duration::from_secs(60))
        }
        other => panic!("expected Busy, got {other:?}"),
    }
    // The minute passes before the next request, or it would wait for it.
    env.advance(60_000);
    {
        let mut state = env.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = Some(999_999);
    }
    match env.anissia.fetch_schedule(1, None).await {
        Err(AnissiaError::Busy { retry_after }) => {
            assert_eq!(retry_after, Duration::from_secs(3600))
        }
        other => panic!("expected Busy, got {other:?}"),
    }
}

#[tokio::test]
async fn an_answer_over_the_cap_is_refused_whether_or_not_it_says_its_length() {
    for chunked in [false, true] {
        let env = Env::new().await;
        {
            let mut state = env.fake.state.lock().unwrap();
            state.padding = MAX_ANSWER_BYTES + 1;
            state.chunked = chunked;
        }
        env.fake
            .set_week(1, vec![env.fake.entry(1, 1, "01:00", "작품", "")]);
        match env.anissia.fetch_schedule(1, None).await {
            Err(AnissiaError::Invalid(why)) => assert!(why.contains("larger"), "{why}"),
            other => panic!("chunked={chunked}: expected Invalid, got {other:?}"),
        }
    }
    // An answer under the cap is read whole.
    let env = Env::new().await;
    env.fake.state.lock().unwrap().padding = 1024 * 1024;
    env.fake
        .set_week(1, vec![env.fake.entry(1, 1, "01:00", "작품", "")]);
    assert_eq!(env.anissia.fetch_schedule(1, None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn answers_that_are_not_the_api_s_are_reported_not_trusted() {
    let env = Env::new().await;
    for (raw, expect) in [
        ("<html>maintenance</html>", "unexpected shape"),
        (r#"{"code":"fail","message":"nope"}"#, "not ok"),
        (r#"{"code":"ok"}"#, "no data"),
        (r#"{"code":"ok","data":"x"}"#, "no data"),
        (r#"{"code":"ok","data":[{"nothing":1}]}"#, "usable"),
    ] {
        env.fake.state.lock().unwrap().raw = Some(raw.to_owned());
        match env.anissia.fetch_schedule(1, None).await {
            Err(AnissiaError::Invalid(why)) => assert!(why.contains(expect), "{raw}: {why}"),
            other => panic!("{raw}: expected Invalid, got {other:?}"),
        }
    }
    // The paged shape (`data.content`) is read too.
    env.fake.state.lock().unwrap().raw = Some(r#"{"code":"ok","data":{"content":[]}}"#.into());
    assert!(env
        .anissia
        .fetch_schedule(1, None)
        .await
        .unwrap()
        .is_empty());

    // A server error and a server that is not there.
    env.fake.state.lock().unwrap().raw = None;
    env.fake.state.lock().unwrap().failing = 1;
    assert!(matches!(
        env.anissia.fetch_schedule(1, None).await,
        Err(AnissiaError::Status(500))
    ));
    let gone = Anissia::new(
        env.db.clone(),
        AnissiaConfig::from_lookup(|_| Some("http://127.0.0.1:1".into())).unwrap(),
        env.anissia.clock.clone(),
    )
    .with_spacing(Duration::ZERO);
    assert!(matches!(
        gone.fetch_schedule(1, None).await,
        Err(AnissiaError::Unreachable(_))
    ));
}

fn subjects(page: &AnimePage) -> Vec<&str> {
    page.entries.iter().map(|e| e.subject.as_str()).collect()
}

#[tokio::test]
async fn a_title_search_reads_the_full_list_a_page_at_a_time_and_finished_anime_are_in_it() {
    let env = Env::new().await;
    env.fake.set_catalogue(vec![
        env.fake.entry(1, 30, "21:30", "안녕하세요 마녀", "魔女"),
        env.fake.finished(1, 29, "안녕, 라라", "さよならララ"),
        env.fake.finished(0, 19, "안녕, 나의 크라머", ""),
        env.fake.finished(2, 7, "다른 작품", ""),
    ]);
    env.fake.state.lock().unwrap().page_size = 2;

    let first = env.anissia.search_anime("안녕", 0, None).await.unwrap();
    assert!(!first.cached);
    assert_eq!(subjects(&first.page), ["안녕하세요 마녀", "안녕, 라라"]);
    assert!(!first.page.last);
    // The finished anime keep what Anissia says of them, `END` and the end date.
    let lara = &first.page.entries[1];
    assert_eq!(
        (
            lara.anime_no,
            lara.status.as_str(),
            lara.end_date.as_deref()
        ),
        (29, "END", Some("2021-06-27"))
    );

    let second = env.anissia.search_anime("안녕", 1, None).await.unwrap();
    assert_eq!(subjects(&second.page), ["안녕, 나의 크라머"]);
    assert!(second.page.last);
    // The text is sent as the user wrote it, and the page counts from 0.
    assert_eq!(env.fake.count("/anime/list/0?q=안녕"), 1);
    assert_eq!(env.fake.count("/anime/list/1?q=안녕"), 1);

    // No title matches: an empty last page, not an error.
    let none = env
        .anissia
        .search_anime("없는 제목", 0, None)
        .await
        .unwrap();
    assert!(none.page.entries.is_empty() && none.page.last);
}

#[tokio::test]
async fn a_searched_page_is_kept_for_five_minutes_and_only_a_few_are() {
    let env = Env::new().await;
    env.fake
        .set_catalogue(vec![env.fake.finished(1, 5, "작품", "")]);

    // A failure is not remembered.
    env.fake.state.lock().unwrap().failing = 1;
    assert!(matches!(
        env.anissia.search_anime("작품", 0, None).await,
        Err(AnissiaError::Status(500))
    ));
    let first = env.anissia.search_anime("작품", 0, None).await.unwrap();
    env.advance(CACHE_TTL.as_millis() as i64 - 1);
    let again = env.anissia.search_anime("작품", 0, None).await.unwrap();
    assert!(again.cached);
    assert_eq!(again.fetched_at, first.fetched_at);
    assert_eq!(env.fake.count("/anime/list/0?q=작품"), 2);
    env.advance(1);
    assert!(
        !env.anissia
            .search_anime("작품", 0, None)
            .await
            .unwrap()
            .cached
    );
    assert_eq!(env.fake.count("/anime/list/0?q=작품"), 3);

    // Another text or page is another entry, and the client keeps the latest few.
    for n in 0..(MAX_CACHED_PAGES + 8) {
        env.advance(1);
        env.anissia
            .search_anime(&format!("q{n}"), 0, None)
            .await
            .unwrap();
    }
    assert!(env.anissia.pages.lock().unwrap().len() <= MAX_CACHED_PAGES);
}

#[tokio::test]
async fn a_search_waits_after_a_429_and_asks_again_only_when_the_wait_is_over() {
    let env = Env::new().await;
    env.fake
        .set_catalogue(vec![env.fake.finished(1, 5, "작품", "")]);
    {
        let mut state = env.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = Some(30);
    }
    match env.anissia.search_anime("작품", 0, None).await {
        Err(AnissiaError::Busy { retry_after }) => {
            assert_eq!(retry_after, Duration::from_secs(30))
        }
        other => panic!("expected Busy, got {other:?}"),
    }
    // The block holds the next request back without sending it.
    match env
        .anissia
        .search_anime("작품", 0, Some(Duration::from_secs(1)))
        .await
    {
        Err(AnissiaError::Busy { retry_after }) => {
            assert_eq!(retry_after, Duration::from_secs(30))
        }
        other => panic!("expected Busy, got {other:?}"),
    }
    assert_eq!(env.fake.requests().len(), 1);

    env.advance(30_000);
    let page = env
        .anissia
        .search_anime("작품", 0, Some(Duration::from_secs(1)))
        .await
        .unwrap();
    assert_eq!(subjects(&page.page), ["작품"]);
    assert_eq!(env.fake.requests().len(), 2);
}

#[tokio::test]
async fn a_search_answer_that_is_not_the_lists_is_refused() {
    let env = Env::new().await;
    for (raw, expect) in [
        ("<html>maintenance</html>", "unexpected shape"),
        (r#"{"code":"fail","message":"nope"}"#, "not ok"),
        (r#"{"code":"ok"}"#, "not a page"),
        // A plain list is a schedule's shape, not a page.
        (r#"{"code":"ok","data":[]}"#, "not a page"),
        (r#"{"code":"ok","data":{"last":true}}"#, "no content"),
        (
            r#"{"code":"ok","data":{"content":{},"last":true}}"#,
            "no content",
        ),
        (r#"{"code":"ok","data":{"content":[]}}"#, "last"),
        (
            r#"{"code":"ok","data":{"content":[],"last":"yes"}}"#,
            "last",
        ),
        (
            r#"{"code":"ok","data":{"content":[{"nothing":1}],"last":true}}"#,
            "usable",
        ),
    ] {
        env.fake.state.lock().unwrap().raw = Some(raw.to_owned());
        match env.anissia.search_anime("작품", 0, None).await {
            Err(AnissiaError::Invalid(why)) => assert!(why.contains(expect), "{raw}: {why}"),
            other => panic!("{raw}: expected Invalid, got {other:?}"),
        }
    }
    // Nothing unreadable was kept.
    assert!(env.anissia.pages.lock().unwrap().is_empty());

    // A server error, and an answer over the cap, are refused too.
    env.fake.state.lock().unwrap().raw = None;
    env.fake.state.lock().unwrap().failing = 1;
    assert!(matches!(
        env.anissia.search_anime("작품", 0, None).await,
        Err(AnissiaError::Status(500))
    ));
    env.fake.state.lock().unwrap().padding = MAX_ANSWER_BYTES + 1;
    env.fake
        .set_catalogue(vec![env.fake.finished(1, 5, "작품", "")]);
    match env.anissia.search_anime("작품", 0, None).await {
        Err(AnissiaError::Invalid(why)) => assert!(why.contains("larger"), "{why}"),
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[tokio::test]
async fn the_recent_captions_are_read_a_page_at_a_time_from_page_0_to_the_empty_page() {
    let env = Env::new().await;
    let lines: Vec<_> = (1..=45)
        .map(|n| {
            env.fake
                .recent_line(n, "1", "2026-10-02T11:17:00", "https://a.test/1", "제작자")
        })
        .collect();
    env.fake.set_recent(lines);

    let mut seen = Vec::new();
    for page in 0.. {
        let read = env.anissia.fetch_recent_captions(page, None).await.unwrap();
        seen.push(read.rows);
        if read.rows == 0 {
            break;
        }
    }
    assert_eq!(seen, [20, 20, 5, 0]);
    assert_eq!(env.fake.count("/anime/caption/recent/"), 4);

    let first = env.anissia.fetch_recent_captions(0, None).await.unwrap();
    assert_eq!(first.lines[0].anime_no, Some(1));
    assert_eq!(first.lines[0].creator, "제작자");
}

#[tokio::test]
async fn the_caption_lines_of_an_anime_keep_the_text_as_written_and_a_429_is_busy() {
    let env = Env::new().await;
    env.fake.set_captions(
        3492,
        vec![env.fake.caption("13.5", "2026-10-02 21:00:00", "에루샤")],
    );
    let (lines, rows) = env.anissia.fetch_caption_lines(3492, None).await.unwrap();
    assert_eq!(rows, 1);
    assert_eq!(lines[0].episode, "13.5");
    assert_eq!(lines[0].updated_at, Some(1_790_942_400_000));
    // An anime Anissia does not know has no lines.
    let (none, rows) = env.anissia.fetch_caption_lines(1, None).await.unwrap();
    assert_eq!((none.len(), rows), (0, 0));

    env.fake.state.lock().unwrap().rate_limited = 1;
    env.fake.state.lock().unwrap().retry_after = Some(120);
    assert!(matches!(
        env.anissia.fetch_recent_captions(0, None).await,
        Err(AnissiaError::Busy { retry_after }) if retry_after == Duration::from_secs(120)
    ));
}
