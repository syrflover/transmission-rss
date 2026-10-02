use std::sync::Arc;

use crate::past_search::client::*;
use trss_core::Db;

fn channel() -> Channel {
    Channel {
        id: "c".into(),
        position: 0,
        version: 1,
        // Nothing listens here: a request that is sent fails to connect.
        url: "http://127.0.0.1:9/?page=rss".into(),
        excludes: Vec::new(),
        secret_query: Vec::new(),
        past_search: None,
        name: None,
    }
}

async fn client(spacing: Duration, clock: Clock) -> (SearchClient, SearchPace, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let pace = SearchPace::new(Db::open(dir.path().join("app.db")).await.unwrap());
    let client = SearchClient::new(pace.clone())
        .unwrap()
        .with_spacing(spacing)
        .with_clock(clock);
    (client, pace, dir)
}

fn fixed_clock(at: i64) -> Clock {
    Arc::new(move || at)
}

/// The answer of a page, or `None` when it was not given within `limit`.
async fn page(client: &SearchClient, limit: Duration) -> Option<Result<Page, SearchError>> {
    tokio::time::timeout(limit, client.page(&channel(), "Show", &Redactor::none()))
        .await
        .ok()
}

#[tokio::test]
async fn a_host_blocked_for_an_hour_fails_the_search_at_once() {
    let (client, pace, _dir) = client(Duration::from_millis(10), fixed_clock(1_000)).await;
    pace.block("127.0.0.1", 1_000 + 3_600_000).await.unwrap();
    let answer = page(&client, Duration::from_secs(2)).await;
    let Some(Err(err)) = answer else {
        panic!("the search waited instead of failing: {answer:?}");
    };
    assert!(matches!(err, SearchError::Wait(_)), "{err}");
}

#[tokio::test]
async fn a_block_that_comes_while_a_request_waits_for_its_slot_stops_the_request() {
    let clock = system_clock();
    let (client, pace, _dir) = client(Duration::from_millis(600), clock.clone()).await;
    let host = "127.0.0.1";
    // Another search's request holds the slot before this one's.
    pace.take_slot(host, clock(), 600, None)
        .await
        .unwrap()
        .unwrap();
    let blocker = tokio::spawn({
        let (pace, clock) = (pace.clone(), clock.clone());
        async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            pace.block(host, clock() + 10_000).await.unwrap();
        }
    });
    let answer = page(&client, Duration::from_secs(5)).await;
    blocker.await.unwrap();
    let Some(Err(err)) = answer else {
        panic!("unexpected answer: {answer:?}");
    };
    // The request must not be sent into the block: sent, it would fail to connect.
    assert!(matches!(err, SearchError::Wait(_)), "{err}");
}

#[test]
fn a_wait_is_told_in_seconds_minutes_or_hours_rounded_up() {
    let secs = Duration::from_secs;
    assert_eq!(wait_phrase(secs(45)), "45초");
    assert_eq!(wait_phrase(Duration::from_millis(100)), "1초");
    assert_eq!(wait_phrase(secs(60)), "1분");
    assert_eq!(wait_phrase(secs(61)), "2분");
    assert_eq!(wait_phrase(secs(3599)), "60분");
    assert_eq!(wait_phrase(secs(3600)), "1시간");
    assert_eq!(wait_phrase(secs(3601)), "2시간");
}
