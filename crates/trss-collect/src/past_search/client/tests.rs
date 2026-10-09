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
async fn a_search_redirected_to_another_host_is_read_without_the_channel_url_going_along() {
    let hosts = crate::feed::testing::Redirect::start().await;
    let (client, _pace, _dir) = client(Duration::from_millis(10), system_clock()).await;
    let channel = Channel {
        url: format!(
            "{}/?page=rss&passkey={}",
            hosts.base,
            crate::feed::testing::SECRET
        ),
        ..channel()
    };
    let page = client
        .page(&channel, "Show", &Redactor::none())
        .await
        .unwrap();
    assert_eq!(page.raw_count, 1);
    hosts.assert_nothing_leaked();
}

/// The only test of how a tracker's `Retry-After` reaches the pace: the page
/// read fails with the wait (`FetchError::Busy` of [`crate::feed::fetch`]) and
/// the host is blocked from the clock's time for that long.
#[tokio::test]
async fn a_429_with_a_retry_after_fails_the_page_with_that_wait_and_blocks_the_host_for_it() {
    let nyaa = crate::fake::FakeNyaa::start().await;
    nyaa.refuse(Some((429, Some(30))));
    let (client, pace, _dir) = client(Duration::from_millis(10), fixed_clock(1_000)).await;
    let channel = Channel {
        url: nyaa.url("token"),
        ..channel()
    };

    let err = client
        .page(&channel, "Show", &Redactor::none())
        .await
        .unwrap_err();

    assert!(
        matches!(err, SearchError::Busy(wait) if wait == Duration::from_secs(30)),
        "{err}"
    );
    assert_eq!(nyaa.queries().len(), 1);
    let blocked = pace.host("127.0.0.1").blocked_until().await.unwrap();
    assert_eq!(blocked, Some(1_000 + 30_000));
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
