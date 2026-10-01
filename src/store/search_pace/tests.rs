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
        pace.take_slot("nyaa.si", 1_000, 3_000).await.unwrap(),
        1_000
    );
    assert_eq!(
        pace.take_slot("nyaa.si", 1_000, 3_000).await.unwrap(),
        4_000
    );
    assert_eq!(
        pace.take_slot("nyaa.si", 1_500, 3_000).await.unwrap(),
        7_000
    );
    // Later than the next slot: no waiting.
    assert_eq!(
        pace.take_slot("nyaa.si", 20_000, 3_000).await.unwrap(),
        20_000
    );
}

#[tokio::test]
async fn each_host_has_its_own_pace_and_case_does_not_matter() {
    let (pace, _dir) = pace().await;
    assert_eq!(
        pace.take_slot("Nyaa.si", 1_000, 3_000).await.unwrap(),
        1_000
    );
    assert_eq!(
        pace.take_slot("other.test", 1_000, 3_000).await.unwrap(),
        1_000
    );
    assert_eq!(
        pace.take_slot("nyaa.SI", 1_000, 3_000).await.unwrap(),
        4_000
    );
}

#[tokio::test]
async fn a_block_holds_every_request_until_it_has_passed() {
    let (pace, _dir) = pace().await;
    pace.block("nyaa.si", 60_000).await.unwrap();
    assert_eq!(
        pace.take_slot("nyaa.si", 1_000, 3_000).await.unwrap(),
        60_000
    );
    assert_eq!(
        pace.take_slot("nyaa.si", 1_000, 3_000).await.unwrap(),
        63_000
    );
    // A shorter block does not shorten a longer one.
    pace.block("nyaa.si", 10_000).await.unwrap();
    assert_eq!(
        pace.take_slot("nyaa.si", 1_000, 3_000).await.unwrap(),
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
    assert_eq!(web.take_slot("nyaa.si", 1_000, 3_000).await.unwrap(), 1_000);
    assert_eq!(
        worker.take_slot("nyaa.si", 1_000, 3_000).await.unwrap(),
        4_000
    );
    assert_eq!(web.take_slot("nyaa.si", 1_000, 3_000).await.unwrap(), 7_000);
}
