//! The Anissia client and its daily refresh against a fake server
//! ([`super::fake`]): the pace, the cap, the cache, and the answers that are
//! not the API's. Nothing here reaches the real Anissia.

use std::{
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use super::{fake::Fake, queue::Ran, *};
use crate::store::{
    anissia::{Due, REFRESH_AFTER_MS},
    channels::{ChannelInput, ChannelStore, NewSubscription, RuleInput, SubtitleMode},
};

const DAY: i64 = REFRESH_AFTER_MS;

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

    /// Subscribes a rule to `no`, whose snapshot was taken at `fetched_at` in
    /// `week`.
    async fn subscribe(&self, no: i64, week: u8, fetched_at: Millis) {
        let channels = ChannelStore::new(self.db.clone());
        let channel = channels
            .create_channel(ChannelInput::new(format!("https://feed{no}.test/rss")))
            .await
            .unwrap()
            .id;
        let entry = self
            .fake
            .entry(week, no, "22:00", &format!("작품 {no}"), "原題");
        let list = vec![entry];
        let mut snapshot = parse::schedule(&list, week)
            .unwrap()
            .remove(0)
            .snapshot(fetched_at);
        snapshot.week = week;
        channels
            .create_subscription_rule(
                &channel,
                RuleInput {
                    r#match: Some(format!("Work {no}")),
                    directory: format!("Work {no}/Season 01"),
                    ..RuleInput::default()
                },
                NewSubscription {
                    anime: snapshot,
                    subtitles: SubtitleMode::Undecided,
                    creator: None,
                    subscribed_at: fetched_at,
                },
            )
            .await
            .unwrap();
    }

    fn schedule_requests(&self) -> Vec<String> {
        self.fake
            .requests()
            .into_iter()
            .map(|(_, path)| path)
            .filter(|p| p.starts_with("/anime/schedule/"))
            .collect()
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

#[tokio::test]
async fn the_daily_refresh_asks_the_week_of_the_snapshot_and_stops_when_it_has_found_them() {
    let env = Env::new().await;
    env.subscribe(3320, 3, 10 * DAY - DAY).await;
    env.subscribe(3321, 3, 10 * DAY - DAY).await;
    env.subscribe(5000, 5, 10 * DAY - 60_000).await; // fresh: not due
    env.fake.set_week(
        3,
        vec![
            env.fake.entry(3, 3320, "23:30", "새 제목", "原題"),
            env.fake.entry(3, 3321, "22:00", "작품 3321", "原題"),
        ],
    );

    let ran = env.anissia.run_next().await.unwrap();
    assert_eq!(
        ran,
        Ran {
            refreshed: 2,
            missing: 0,
            failed: 0
        }
    );
    assert_eq!(env.schedule_requests(), ["/anime/schedule/3"]);
    let snapshot = env.anissia.store.anime(3320).await.unwrap().unwrap();
    assert_eq!(snapshot.subject, "새 제목");
    assert_eq!(snapshot.air_time.as_deref(), Some("23:30"));
    assert_eq!(snapshot.fetched_at, 10 * DAY);
    // Nothing is due any more.
    assert!(env.anissia.run_next().await.is_none());
}

#[tokio::test]
async fn an_anime_that_moved_from_the_upcoming_list_to_its_weekday_is_found_there() {
    let env = Env::new().await;
    env.subscribe(7, 8, 10 * DAY - DAY).await;
    env.fake
        .set_week(2, vec![env.fake.entry(2, 7, "21:00", "작품 7", "")]);

    let ran = env.anissia.run_next().await.unwrap();
    assert_eq!(ran.refreshed, 1);
    assert_eq!(
        env.schedule_requests(),
        [
            "/anime/schedule/8",
            "/anime/schedule/0",
            "/anime/schedule/1",
            "/anime/schedule/2"
        ]
    );
    let snapshot = env.anissia.store.anime(7).await.unwrap().unwrap();
    assert_eq!(snapshot.week, 2);
    assert_eq!(snapshot.air_time.as_deref(), Some("21:00"));
}

#[tokio::test]
async fn an_anime_no_week_lists_keeps_its_snapshot_and_is_looked_for_a_day_later() {
    let env = Env::new().await;
    env.subscribe(9, 3, 10 * DAY - DAY).await;

    let ran = env.anissia.run_next().await.unwrap();
    assert_eq!(
        ran,
        Ran {
            refreshed: 0,
            missing: 1,
            failed: 0
        }
    );
    // Every week was asked once, its own first.
    let asked = env.schedule_requests();
    assert_eq!(asked.len(), 9);
    assert_eq!(asked[0], "/anime/schedule/3");
    let snapshot = env.anissia.store.anime(9).await.unwrap().unwrap();
    assert_eq!(snapshot.subject, "작품 9");
    assert_eq!(snapshot.fetched_at, 10 * DAY - DAY);

    assert!(env.anissia.run_next().await.is_none());
    env.advance(DAY - 1);
    assert!(env.anissia.run_next().await.is_none());
    env.advance(1);
    assert!(env.anissia.run_next().await.is_some());
}

#[tokio::test]
async fn a_failed_refresh_puts_the_anime_off_for_an_hour_and_a_429_for_as_long_as_asked() {
    let env = Env::new().await;
    env.subscribe(1, 3, 10 * DAY - DAY).await;
    env.fake.state.lock().unwrap().failing = 1;

    let ran = env.anissia.run_next().await.unwrap();
    assert_eq!(
        ran,
        Ran {
            refreshed: 0,
            missing: 0,
            failed: 1
        }
    );
    assert_eq!(env.schedule_requests().len(), 1);
    assert!(env.anissia.run_next().await.is_none());
    env.advance(queue::REFRESH_RETRY.as_millis() as i64 - 1);
    assert!(env.anissia.run_next().await.is_none());
    env.advance(1);
    env.fake
        .set_week(3, vec![env.fake.entry(3, 1, "22:00", "작품 1", "")]);
    assert_eq!(env.anissia.run_next().await.unwrap().refreshed, 1);

    // A 429 holds it for as long as Anissia said.
    env.subscribe(2, 3, env.now.load(Ordering::SeqCst) - DAY)
        .await;
    {
        let mut state = env.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = Some(90);
    }
    assert_eq!(env.anissia.run_next().await.unwrap().failed, 1);
    env.advance(89_000);
    assert!(env.anissia.run_next().await.is_none());
    env.advance(1_000);
    let due = env.anissia.store.due(env.anissia.now()).await.unwrap();
    assert_eq!(
        due,
        [Due {
            anime_no: 2,
            week: Some(3)
        }]
    );
}

#[test]
fn the_lock_file_sits_next_to_the_database_and_apart_from_the_other_queues() {
    let path = queue::lock_path_for(std::path::Path::new("/data/trss.db"));
    assert_eq!(path, std::path::PathBuf::from("/data/trss.db.anissia.lock"));
}
