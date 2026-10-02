//! The observation of the subtitle lines against a fake Anissia
//! ([`trss_anissia::fake`]), whose answers are the shapes read from the real
//! one on 2026-10-02. Nothing here reaches the real Anissia.

use std::{
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use serde_json::Value;
use trss_anissia::{fake::Fake, Anissia};
use trss_core::{Clock, Db};

use crate::{
    anissia::captions::*,
    store::anissia::{AnissiaStore, Candidate},
};

const MIN: i64 = 60_000;
const START: i64 = 10 * 24 * 60 * MIN;
/// 2026-10-02 12:00:00 UTC.
const NOON_UTC: i64 = 1_790_942_400_000;

struct Env {
    db: Db,
    fake: Fake,
    observer: CaptionObserver,
    store: AnissiaStore,
    now: Arc<AtomicI64>,
}

impl Env {
    /// An observer with no spacing between requests and a clock the test sets.
    async fn new() -> Env {
        let db = Db::open_blocking(":memory:").unwrap();
        let fake = Fake::start().await;
        let now = Arc::new(AtomicI64::new(START));
        let clock: Clock = {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        };
        let anissia = Anissia::new(db.clone(), fake.config(), clock).with_spacing(Duration::ZERO);
        let store = AnissiaStore::new(db.clone());
        Env {
            db,
            fake,
            observer: CaptionObserver::new(anissia, store.clone()),
            store,
            now,
        }
    }

    fn advance(&self, ms: i64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
    }

    /// A line of the recent list.
    fn line(
        &self,
        anime_no: i64,
        episode: &str,
        updated: &str,
        post: &str,
        creator: &str,
    ) -> Value {
        self.fake
            .recent_line(anime_no, episode, updated, post, creator)
    }

    /// Anime 1..=`n`, one line each: `n` lines at 20 to a page.
    fn many(&self, n: i64) -> Vec<Value> {
        (1..=n)
            .map(|no| {
                self.line(
                    no,
                    "1",
                    "2026-10-02T11:17:00",
                    &format!("https://blog.test/{no}"),
                    "제작자",
                )
            })
            .collect()
    }

    fn recent_requests(&self) -> usize {
        self.fake.count("/anime/caption/recent/")
    }

    async fn candidates(&self, anime_no: i64) -> Vec<Candidate> {
        self.store.candidates(anime_no).await.unwrap()
    }
}

#[tokio::test]
async fn a_reading_takes_every_page_to_the_empty_one_and_observes_every_anime() {
    let env = Env::new().await;
    // 45 lines: pages of 20, 20 and 5, then the empty page. Anime 1 is the one
    // a season will be linked to; the app does not know that, nor need to.
    env.fake.set_recent(env.many(45));

    let read = env.observer.run_due().await.unwrap();

    assert_eq!(
        read,
        Read {
            pages: 3,
            added: 45,
            unchanged: 0,
            skipped: 0,
            end: End::Complete
        }
    );
    // Pages 0, 1, 2 and the empty page 3: four requests, from page 0.
    assert_eq!(env.recent_requests(), 4);
    let paths: Vec<String> = env.fake.requests().into_iter().map(|(_, p)| p).collect();
    assert_eq!(
        paths,
        [0, 1, 2, 3].map(|n| format!("/anime/caption/recent/{n}"))
    );
    assert_eq!(env.candidates(1).await.len(), 1);
    assert_eq!(env.candidates(45).await.len(), 1);
    assert_eq!(env.candidates(46).await.len(), 0);
    // The reading is on record.
    let (next_at, last_read_at) = env.store.caption_poll().await.unwrap().unwrap();
    assert_eq!((next_at, last_read_at), (START + 30 * MIN, Some(START)));
}

#[tokio::test]
async fn nothing_is_read_again_within_thirty_minutes_even_after_a_restart() {
    let env = Env::new().await;
    env.fake.set_recent(env.many(5));
    assert!(env.observer.run_due().await.is_some());
    let asked = env.fake.requests().len();

    env.advance(29 * MIN);
    assert!(env.observer.run_due().await.is_none());
    // Another process over the same database (a restarted worker).
    let restarted = CaptionObserver::new(
        Anissia::new(env.db.clone(), env.fake.config(), {
            let now = env.now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        })
        .with_spacing(Duration::ZERO),
        env.store.clone(),
    );
    assert!(restarted.run_due().await.is_none());
    assert_eq!(env.fake.requests().len(), asked);

    env.advance(MIN);
    assert!(env.observer.run_due().await.is_some());
    assert!(env.fake.requests().len() > asked);
}

#[tokio::test]
async fn a_reading_after_which_no_line_changed_adds_nothing() {
    let env = Env::new().await;
    env.fake.set_recent(env.many(25));
    env.observer.run_due().await.unwrap();

    env.advance(30 * MIN);
    let read = env.observer.run_due().await.unwrap();

    assert_eq!(
        (read.added, read.unchanged, read.end),
        (0, 25, End::Complete)
    );
    assert_eq!(env.candidates(1).await.len(), 1);
}

#[tokio::test]
async fn a_creator_moving_to_the_next_episode_leaves_both_candidates() {
    let env = Env::new().await;
    env.fake.set_recent(vec![env.line(
        7,
        "3",
        "2026-10-02T11:00:00",
        "https://blog.test/a",
        "에루샤",
    )]);
    env.observer.run_due().await.unwrap();

    env.advance(30 * MIN);
    env.fake.set_recent(vec![env.line(
        7,
        "4",
        "2026-10-02T11:40:00",
        "https://blog.test/b",
        "에루샤",
    )]);
    let read = env.observer.run_due().await.unwrap();

    assert_eq!((read.added, read.unchanged), (1, 0));
    let all = env.candidates(7).await;
    let mut seen: Vec<_> = all
        .iter()
        .map(|c| (c.episode.as_str(), c.post_url.as_str()))
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        [("3", "https://blog.test/a"), ("4", "https://blog.test/b")]
    );
    // Same creator, one source of the app.
    assert_eq!(all[0].source_id, all[1].source_id);
    // A new episode is a candidate of its own, not a revision.
    assert!(all.iter().all(|c| c.revision.is_none()));
}

#[tokio::test]
async fn an_update_time_that_alone_changes_is_the_same_post_updated_and_marks_a_revision() {
    let env = Env::new().await;
    env.fake.set_recent(vec![env.line(
        7,
        "3",
        "2026-10-02T11:00:00",
        "https://blog.test/a",
        "에루샤",
    )]);
    env.observer.run_due().await.unwrap();

    env.advance(30 * MIN);
    env.fake.set_recent(vec![env.line(
        7,
        "3",
        "2026-10-02T11:50:00",
        "https://blog.test/a",
        "에루샤",
    )]);
    let read = env.observer.run_due().await.unwrap();

    assert_eq!((read.added, read.unchanged), (1, 0));
    let all = env.candidates(7).await;
    // Both observations stay, the update newest first.
    assert_eq!(all.len(), 2);
    let (newer, older) = (&all[0], &all[1]);
    assert!(newer.updated_at > older.updated_at);
    assert_eq!(newer.post_url, older.post_url);
    let revision = newer.revision.as_ref().expect("a revision candidate");
    assert_eq!((revision.of, revision.same_post), (older.id, true));
    assert_eq!(older.revision, None);
}

#[tokio::test]
async fn a_line_that_leaves_the_list_leaves_its_observations() {
    let env = Env::new().await;
    env.fake.set_recent(env.many(3));
    env.observer.run_due().await.unwrap();
    assert_eq!(env.candidates(2).await.len(), 1);

    env.advance(30 * MIN);
    // The creator finished: Anissia no longer lists the line (and the list is
    // shorter now). Even a list with nothing in it deletes nothing.
    env.fake.set_recent(vec![env.many(3).remove(0)]);
    let read = env.observer.run_due().await.unwrap();
    assert_eq!((read.added, read.unchanged), (0, 1));
    env.advance(30 * MIN);
    env.fake.set_recent(Vec::new());
    let read = env.observer.run_due().await.unwrap();
    assert_eq!((read.pages, read.end), (0, End::Complete));
    for no in 1..=3 {
        assert_eq!(env.candidates(no).await.len(), 1, "anime {no}");
    }
}

#[tokio::test]
async fn an_anime_read_after_its_lines_were_observed_shows_the_earlier_ones_and_the_older_ones() {
    let env = Env::new().await;
    // The recent list holds one creator's 12th episode of anime 3492.
    env.fake.set_recent(vec![env.line(
        3492,
        "12",
        "2026-09-10T12:10:00",
        "https://erulabo.com/837",
        "에루샤",
    )]);
    env.observer.run_due().await.unwrap();

    // Anissia's own list for the anime also has a creator whose line is too old
    // for the recent list, and the creator above again, as it is.
    env.fake.set_captions(
        3492,
        vec![
            serde_json::json!({
                "episode": "12", "updDt": "2026-09-10T12:10:00",
                "website": "https://erulabo.com/837", "name": "에루샤",
            }),
            env.fake.caption("24", "2026-03-01T00:00:00", "코코렛"),
        ],
    );
    env.advance(MIN);
    let read = env.observer.read_anime(3492).await;

    assert_eq!(
        read,
        Read {
            pages: 1,
            added: 1,
            unchanged: 1,
            skipped: 0,
            end: End::Complete
        }
    );
    let all = env.candidates(3492).await;
    let mut seen: Vec<_> = all
        .iter()
        .map(|c| (c.creator.as_str(), c.episode.as_str()))
        .collect();
    seen.sort();
    assert_eq!(seen, [("에루샤", "12"), ("코코렛", "24")]);
    assert_eq!(env.fake.count("/anime/caption/animeNo/3492"), 1);
}

#[tokio::test]
async fn an_update_time_without_a_zone_is_seoul_time_and_an_invalid_one_is_kept_and_marked() {
    let env = Env::new().await;
    env.fake.set_recent(vec![
        env.line(1, "1", "2026-10-02 21:00:00", "https://blog.test/1", "가"),
        env.line(2, "1", "2026-10-02T21:00:00", "https://blog.test/2", "나"),
        env.line(3, "1", "2026-10-02T12:00:00Z", "https://blog.test/3", "다"),
        env.line(4, "1", "2026-02-30T21:00:00", "https://blog.test/4", "라"),
    ]);
    env.observer.run_due().await.unwrap();

    for no in 1..=3 {
        let one = &env.candidates(no).await[0];
        assert_eq!(one.updated_at, Some(NOON_UTC), "anime {no}");
    }
    let bad = &env.candidates(4).await[0];
    assert_eq!(bad.updated_at, None);
    // The text stays, and the observation is dated by when it was first seen.
    assert_eq!(bad.updated, "2026-02-30T21:00:00");
    assert_eq!(bad.first_seen_at, START);
    assert_eq!(bad.sort_at(), START);
}

#[tokio::test]
async fn episodes_zero_and_thirteen_and_a_half_are_kept_as_written() {
    let env = Env::new().await;
    env.fake.set_recent(vec![
        env.line(1, "0", "2026-10-02T11:00:00", "https://blog.test/1", "가"),
        env.line(
            2,
            "13.5",
            "2026-10-02T11:00:00",
            "https://blog.test/2",
            "나",
        ),
        env.line(3, "07", "2026-10-02T11:00:00", "https://blog.test/3", "다"),
    ]);
    env.observer.run_due().await.unwrap();

    assert_eq!(env.candidates(1).await[0].episode, "0");
    assert_eq!(env.candidates(2).await[0].episode, "13.5");
    assert_eq!(env.candidates(3).await[0].episode, "07");
}

#[tokio::test]
async fn a_line_that_cannot_be_used_is_counted_and_does_not_end_the_reading() {
    let env = Env::new().await;
    let mut lines = env.many(2);
    lines.insert(
        1,
        env.line(9, "1", "2026-10-02T11:00:00", "javascript:alert(1)", "나쁜"),
    );
    env.fake.set_recent(lines);

    let read = env.observer.run_due().await.unwrap();

    assert_eq!((read.pages, read.added, read.skipped), (1, 2, 1));
    assert_eq!(env.candidates(9).await.len(), 0);
}

#[tokio::test]
async fn a_429_ends_the_reading_and_the_next_period_reads_again() {
    let env = Env::new().await;
    env.fake.set_recent(env.many(5));
    {
        let mut state = env.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = Some(120);
    }

    let read = env.observer.run_due().await.unwrap();

    assert_eq!(read.end, End::Busy(Duration::from_secs(120)));
    assert_eq!(read.added, 0);
    // Due again after the period, not at once.
    env.advance(29 * MIN);
    assert!(env.observer.run_due().await.is_none());
    env.advance(MIN);
    let read = env.observer.run_due().await.unwrap();
    assert_eq!((read.added, read.end), (5, End::Complete));
}

#[tokio::test]
async fn a_429_that_asks_for_longer_than_the_period_holds_the_next_reading_until_then() {
    let env = Env::new().await;
    env.fake.set_recent(env.many(5));
    {
        let mut state = env.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = Some(3600);
    }
    let read = env.observer.run_due().await.unwrap();
    assert_eq!(read.end, End::Busy(Duration::from_secs(3600)));

    env.advance(31 * MIN);
    // Still inside the wait Anissia asked for: nothing is sent.
    let asked = env.fake.requests().len();
    assert!(env.observer.run_due().await.is_none());
    assert_eq!(env.fake.requests().len(), asked);
    env.advance(30 * MIN);
    assert_eq!(env.observer.run_due().await.unwrap().added, 5);
}

#[tokio::test]
async fn a_failure_is_read_again_next_period_and_leaves_the_observations_there() {
    let env = Env::new().await;
    env.fake.set_recent(env.many(5));
    env.observer.run_due().await.unwrap();
    env.advance(30 * MIN);

    env.fake.state.lock().unwrap().failing = 1;
    let read = env.observer.run_due().await.unwrap();
    assert!(matches!(read.end, End::Failed(_)), "{:?}", read.end);
    assert_eq!(read.pages, 0);
    for no in 1..=5 {
        assert_eq!(env.candidates(no).await.len(), 1);
    }
    // Not retried inside the period; read again in the next.
    env.advance(29 * MIN);
    assert!(env.observer.run_due().await.is_none());
    env.advance(MIN);
    let read = env.observer.run_due().await.unwrap();
    assert_eq!((read.end, read.unchanged), (End::Complete, 5));
}

#[tokio::test]
async fn a_failure_in_the_middle_keeps_the_pages_already_read_and_the_next_period_goes_on() {
    let env = Env::new().await;
    env.fake.set_recent(env.many(45));
    env.fake
        .state
        .lock()
        .unwrap()
        .failing_paths
        .insert("/anime/caption/recent/1".into());

    let read = env.observer.run_due().await.unwrap();

    assert!(matches!(read.end, End::Failed(_)));
    // Page 0 (20 lines) was observed before page 1 failed.
    assert_eq!((read.pages, read.added), (1, 20));
    assert_eq!(env.candidates(1).await.len(), 1);
    assert_eq!(env.candidates(21).await.len(), 0);
    // The failed reading was not a complete one.
    let (_, last_read_at) = env.store.caption_poll().await.unwrap().unwrap();
    assert_eq!(last_read_at, None);

    env.fake.state.lock().unwrap().failing_paths.clear();
    env.advance(30 * MIN);
    let read = env.observer.run_due().await.unwrap();
    assert_eq!(
        (read.pages, read.added, read.unchanged, read.end),
        (3, 25, 20, End::Complete)
    );
}

#[tokio::test]
async fn an_answer_that_is_not_the_apis_is_a_failure_not_an_end() {
    let env = Env::new().await;
    env.fake.state.lock().unwrap().raw = Some(r#"{"code":"error","message":"x"}"#.into());
    let read = env.observer.run_due().await.unwrap();
    assert!(matches!(read.end, End::Failed(_)));
    assert_eq!(read.pages, 0);
}

#[tokio::test]
async fn a_list_that_never_ends_is_cut_at_the_page_limit() {
    let env = Env::new().await;
    // One line per page, more pages than the limit.
    {
        let mut state = env.fake.state.lock().unwrap();
        state.recent_page_size = 1;
    }
    env.fake.set_recent(env.many(MAX_RECENT_PAGES as i64 + 5));
    let read = env.observer.run_due().await.unwrap();
    assert!(matches!(read.end, End::Failed(_)));
    assert_eq!(read.pages, MAX_RECENT_PAGES);
    assert_eq!(read.added, MAX_RECENT_PAGES as usize);
}

#[test]
fn the_lock_sits_next_to_the_database_and_apart_from_the_other_queues() {
    let path = lock_path_for(std::path::Path::new("/data/trss.db"));
    assert_eq!(path.to_str(), Some("/data/trss.db.anissia-captions.lock"));
    assert_ne!(
        path,
        crate::anissia::lock_path_for(std::path::Path::new("/data/trss.db"))
    );
}

#[tokio::test]
async fn the_queue_holds_its_lock_and_a_second_observer_waits_for_it() {
    let dir = tempfile::tempdir().unwrap();
    let lock = dir.path().join("x.lock");
    let held = trss_core::CycleLock::try_acquire(&lock).unwrap().unwrap();
    // While the lock is held elsewhere the queue reads nothing.
    let env = Env::new().await;
    env.fake.set_recent(env.many(3));
    let cancel = tokio_util::sync::CancellationToken::new();
    let queue = tokio::spawn({
        let (observer, lock, cancel) = (env.observer.clone(), lock.clone(), cancel.clone());
        async move { observer.run_queue(lock, cancel).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(env.fake.requests().len(), 0);
    cancel.cancel();
    queue.await.unwrap();
    drop(held);

    // With the lock free the queue reads once and then waits for the period.
    let cancel = tokio_util::sync::CancellationToken::new();
    let queue = tokio::spawn({
        let (observer, lock, cancel) = (env.observer.clone(), lock.clone(), cancel.clone());
        async move { observer.run_queue(lock, cancel).await }
    });
    for _ in 0..100 {
        if !env.candidates(1).await.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    cancel.cancel();
    queue.await.unwrap();
    assert_eq!(env.candidates(1).await.len(), 1);
    assert_eq!(env.recent_requests(), 2);
}
