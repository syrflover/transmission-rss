use super::*;
use crate::store::Db;

async fn pace() -> (SearchPace, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    (SearchPace::new(db), dir)
}

#[tokio::test]
async fn requests_are_given_slots_a_spacing_apart() {
    let (pace, _dir) = pace().await;
    assert_eq!(
        pace.take_slot("nyaa.si", 1_000, 3_000, None)
            .await
            .unwrap()
            .unwrap(),
        1_000
    );
    assert_eq!(
        pace.take_slot("nyaa.si", 1_000, 3_000, None)
            .await
            .unwrap()
            .unwrap(),
        4_000
    );
    assert_eq!(
        pace.take_slot("nyaa.si", 1_500, 3_000, None)
            .await
            .unwrap()
            .unwrap(),
        7_000
    );
    // Later than the next slot: no waiting.
    assert_eq!(
        pace.take_slot("nyaa.si", 20_000, 3_000, None)
            .await
            .unwrap()
            .unwrap(),
        20_000
    );
}

#[tokio::test]
async fn each_host_has_its_own_pace_and_case_does_not_matter() {
    let (pace, _dir) = pace().await;
    assert_eq!(
        pace.take_slot("Nyaa.si", 1_000, 3_000, None)
            .await
            .unwrap()
            .unwrap(),
        1_000
    );
    assert_eq!(
        pace.take_slot("other.test", 1_000, 3_000, None)
            .await
            .unwrap()
            .unwrap(),
        1_000
    );
    assert_eq!(
        pace.take_slot("nyaa.SI", 1_000, 3_000, None)
            .await
            .unwrap()
            .unwrap(),
        4_000
    );
}

#[tokio::test]
async fn a_block_holds_every_request_until_it_has_passed() {
    let (pace, _dir) = pace().await;
    pace.block("nyaa.si", 60_000).await.unwrap();
    assert_eq!(
        pace.take_slot("nyaa.si", 1_000, 3_000, None)
            .await
            .unwrap()
            .unwrap(),
        60_000
    );
    assert_eq!(
        pace.take_slot("nyaa.si", 1_000, 3_000, None)
            .await
            .unwrap()
            .unwrap(),
        63_000
    );
    // A shorter block does not shorten a longer one.
    pace.block("nyaa.si", 10_000).await.unwrap();
    assert_eq!(
        pace.take_slot("nyaa.si", 1_000, 3_000, None)
            .await
            .unwrap()
            .unwrap(),
        66_000
    );
}

#[tokio::test]
async fn two_handles_on_one_file_share_the_pace() {
    // The web and the worker are separate processes with their own connection.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let web = SearchPace::new(Db::open(&path).await.unwrap());
    let worker = SearchPace::new(Db::open(&path).await.unwrap());
    assert_eq!(
        web.take_slot("nyaa.si", 1_000, 3_000, None)
            .await
            .unwrap()
            .unwrap(),
        1_000
    );
    assert_eq!(
        worker
            .take_slot("nyaa.si", 1_000, 3_000, None)
            .await
            .unwrap()
            .unwrap(),
        4_000
    );
    assert_eq!(
        web.take_slot("nyaa.si", 1_000, 3_000, None)
            .await
            .unwrap()
            .unwrap(),
        7_000
    );
}

#[tokio::test]
async fn a_slot_further_away_than_the_longest_wait_is_not_taken() {
    let (pace, _dir) = pace().await;
    pace.block("nyaa.si", 3_600_000).await.unwrap();
    // The wait is told, and the pace stays as the block left it.
    for _ in 0..2 {
        assert_eq!(
            pace.take_slot("nyaa.si", 1_000, 3_000, Some(60_000))
                .await
                .unwrap(),
            Err(3_599_000)
        );
    }
    // Once the block has passed, the first request is not behind the refused ones.
    assert_eq!(
        pace.take_slot("nyaa.si", 3_600_000, 3_000, Some(60_000))
            .await
            .unwrap(),
        Ok(3_600_000)
    );
    // A slot within the wait is taken.
    assert_eq!(
        pace.take_slot("nyaa.si", 3_600_000, 3_000, Some(60_000))
            .await
            .unwrap(),
        Ok(3_603_000)
    );
}

#[tokio::test]
async fn the_block_of_a_host_can_be_read() {
    let (pace, _dir) = pace().await;
    assert_eq!(pace.blocked_until("Nyaa.si").await.unwrap(), None);
    pace.take_slot("nyaa.si", 1_000, 3_000, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pace.blocked_until("nyaa.si").await.unwrap(), None);
    pace.block("nyaa.si", 60_000).await.unwrap();
    assert_eq!(pace.blocked_until("NYAA.si").await.unwrap(), Some(60_000));
}
