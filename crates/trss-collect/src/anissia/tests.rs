//! The daily refresh of the snapshots against a fake Anissia
//! ([`trss_anissia::fake`]). Nothing here reaches the real Anissia.

use std::{
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use trss_anissia::{fake::Fake, parse, Anissia};
use trss_core::{Clock, Db, Millis};

use crate::{
    anissia::*,
    store::channels::{ChannelInput, ChannelStore, NewSubscription, RuleInput, SubtitleMode},
};

const DAY: i64 = REFRESH_AFTER_MS;

struct Env {
    db: Db,
    fake: Fake,
    queue: AnissiaQueue,
    store: AnissiaStore,
    now: Arc<AtomicI64>,
}

impl Env {
    /// A queue with no spacing between requests and a clock the test sets.
    async fn new() -> Env {
        let db = Db::open_blocking(":memory:").unwrap();
        let fake = Fake::start().await;
        let now = Arc::new(AtomicI64::new(10 * DAY));
        let clock: Clock = {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        };
        let anissia = Anissia::new(db.clone(), fake.config(), clock).with_spacing(Duration::ZERO);
        let store = AnissiaStore::new(db.clone());
        Env {
            db,
            fake,
            queue: AnissiaQueue::new(anissia, store.clone()),
            store,
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

    let ran = env.queue.run_next().await.unwrap();
    assert_eq!(
        ran,
        Ran {
            refreshed: 2,
            missing: 0,
            failed: 0
        }
    );
    assert_eq!(env.schedule_requests(), ["/anime/schedule/3"]);
    let snapshot = env.store.anime(3320).await.unwrap().unwrap();
    assert_eq!(snapshot.subject, "새 제목");
    assert_eq!(snapshot.air_time.as_deref(), Some("23:30"));
    assert_eq!(snapshot.fetched_at, 10 * DAY);
    // Nothing is due any more.
    assert!(env.queue.run_next().await.is_none());
}

#[tokio::test]
async fn an_anime_that_moved_from_the_upcoming_list_to_its_weekday_is_found_there() {
    let env = Env::new().await;
    env.subscribe(7, 8, 10 * DAY - DAY).await;
    env.fake
        .set_week(2, vec![env.fake.entry(2, 7, "21:00", "작품 7", "")]);

    let ran = env.queue.run_next().await.unwrap();
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
    let snapshot = env.store.anime(7).await.unwrap().unwrap();
    assert_eq!(snapshot.week, 2);
    assert_eq!(snapshot.air_time.as_deref(), Some("21:00"));
}

#[tokio::test]
async fn an_anime_no_week_lists_keeps_its_snapshot_and_is_looked_for_a_day_later() {
    let env = Env::new().await;
    env.subscribe(9, 3, 10 * DAY - DAY).await;
    // The schedule lists other anime, among them on its own weekday, but not this one.
    env.fake
        .set_week(3, vec![env.fake.entry(3, 5000, "22:00", "남", "")]);

    let ran = env.queue.run_next().await.unwrap();
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
    let snapshot = env.store.anime(9).await.unwrap().unwrap();
    assert_eq!(snapshot.subject, "작품 9");
    assert_eq!(snapshot.fetched_at, 10 * DAY - DAY);
    // Anissia answered everywhere and the anime was not there: it is recorded.
    assert_eq!(unlisted(&env, &[9]).await, [9]);

    assert!(env.queue.run_next().await.is_none());
    env.advance(DAY - 1);
    assert!(env.queue.run_next().await.is_none());
    env.advance(1);
    assert!(env.queue.run_next().await.is_some());
}

async fn unlisted(env: &Env, nos: &[i64]) -> Vec<i64> {
    let mut found: Vec<i64> = env
        .store
        .unlisted(nos.to_vec())
        .await
        .unwrap()
        .into_iter()
        .collect();
    found.sort();
    found
}

#[tokio::test]
async fn an_unlisted_anime_is_listed_again_when_a_later_refresh_finds_it() {
    let env = Env::new().await;
    env.subscribe(9, 3, 10 * DAY - DAY).await;
    env.fake
        .set_week(3, vec![env.fake.entry(3, 5000, "22:00", "남", "")]);
    env.queue.run_next().await.unwrap();
    assert_eq!(unlisted(&env, &[9]).await, [9]);

    // Anissia lists it again (on another weekday) by the next refresh.
    env.advance(DAY);
    env.fake
        .set_week(6, vec![env.fake.entry(6, 9, "21:00", "작품 9", "")]);
    assert_eq!(env.queue.run_next().await.unwrap().refreshed, 1);
    assert_eq!(unlisted(&env, &[9]).await, Vec::<i64>::new());
    assert_eq!(env.store.anime(9).await.unwrap().unwrap().week, 6);
}

#[tokio::test]
async fn a_refresh_that_fails_or_stops_halfway_never_records_an_anime_as_unlisted() {
    let env = Env::new().await;
    env.subscribe(9, 3, 10 * DAY - DAY).await;
    env.fake
        .set_week(1, vec![env.fake.entry(1, 5000, "22:00", "남", "")]);

    // Week 6 cannot be read: the weeks before it listed nothing of the anime,
    // but the weeks after were never asked.
    env.fake
        .state
        .lock()
        .unwrap()
        .failing_paths
        .insert("/anime/schedule/6".into());
    let ran = env.queue.run_next().await.unwrap();
    assert_eq!(ran.failed, 1);
    assert_eq!(unlisted(&env, &[9]).await, Vec::<i64>::new());

    // A 429 at the first request.
    env.advance(REFRESH_RETRY.as_millis() as i64);
    env.fake.state.lock().unwrap().failing_paths.clear();
    {
        let mut state = env.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = Some(30);
    }
    assert_eq!(env.queue.run_next().await.unwrap().failed, 1);
    assert_eq!(unlisted(&env, &[9]).await, Vec::<i64>::new());
}

#[tokio::test]
async fn a_schedule_that_lists_nothing_at_all_does_not_make_an_anime_unlisted() {
    let env = Env::new().await;
    env.subscribe(9, 3, 10 * DAY - DAY).await;

    let ran = env.queue.run_next().await.unwrap();
    assert_eq!(ran.missing, 1);
    assert_eq!(env.schedule_requests().len(), 9);
    assert_eq!(unlisted(&env, &[9]).await, Vec::<i64>::new());
    // Still looked for a day later.
    assert!(env.queue.run_next().await.is_none());
    env.advance(DAY);
    assert!(env.queue.run_next().await.is_some());
}

#[tokio::test]
async fn a_weekday_that_comes_back_empty_does_not_unlist_the_anime_it_held() {
    let env = Env::new().await;
    // The last refresh found two anime on each of Monday (1) and Tuesday (2).
    env.subscribe(11, 1, 10 * DAY - DAY).await;
    env.subscribe(12, 1, 10 * DAY - DAY).await;
    env.subscribe(21, 2, 10 * DAY - DAY).await;
    env.subscribe(22, 2, 10 * DAY - DAY).await;
    // Now Anissia answers normally, but Monday's list is empty and Tuesday's no
    // longer has 22.
    env.fake
        .set_week(2, vec![env.fake.entry(2, 21, "22:00", "작품 21", "")]);

    let ran = env.queue.run_next().await.unwrap();
    assert_eq!(ran.refreshed, 1);
    // Monday's answer is not believed, so its anime are not recorded as gone.
    // Tuesday's was a real list, and 22 is not on it.
    assert_eq!(unlisted(&env, &[11, 12, 21, 22]).await, [22]);

    // Monday's anime are looked for again with the next daily refresh, and a
    // Monday that lists them again leaves them as they were.
    assert!(env.queue.run_next().await.is_none());
    env.advance(DAY);
    env.fake.set_week(
        1,
        vec![
            env.fake.entry(1, 11, "22:00", "작품 11", ""),
            env.fake.entry(1, 12, "22:00", "작품 12", ""),
        ],
    );
    // 21 is a day old by now too, and Tuesday still lists it.
    assert_eq!(env.queue.run_next().await.unwrap().refreshed, 3);
    assert_eq!(unlisted(&env, &[11, 12, 21, 22]).await, [22]);
}

#[tokio::test]
async fn a_failed_refresh_puts_the_anime_off_for_an_hour_and_a_429_for_as_long_as_asked() {
    let env = Env::new().await;
    env.subscribe(1, 3, 10 * DAY - DAY).await;
    env.fake.state.lock().unwrap().failing = 1;

    let ran = env.queue.run_next().await.unwrap();
    assert_eq!(
        ran,
        Ran {
            refreshed: 0,
            missing: 0,
            failed: 1
        }
    );
    assert_eq!(env.schedule_requests().len(), 1);
    assert!(env.queue.run_next().await.is_none());
    env.advance(REFRESH_RETRY.as_millis() as i64 - 1);
    assert!(env.queue.run_next().await.is_none());
    env.advance(1);
    env.fake
        .set_week(3, vec![env.fake.entry(3, 1, "22:00", "작품 1", "")]);
    assert_eq!(env.queue.run_next().await.unwrap().refreshed, 1);

    // A 429 holds it for as long as Anissia said.
    env.subscribe(2, 3, env.now.load(Ordering::SeqCst) - DAY)
        .await;
    {
        let mut state = env.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = Some(90);
    }
    assert_eq!(env.queue.run_next().await.unwrap().failed, 1);
    env.advance(89_000);
    assert!(env.queue.run_next().await.is_none());
    env.advance(1_000);
    let due = env.store.due(env.queue.now()).await.unwrap();
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
    let path = lock_path_for(std::path::Path::new("/data/trss.db"));
    assert_eq!(path, std::path::PathBuf::from("/data/trss.db.anissia.lock"));
}
