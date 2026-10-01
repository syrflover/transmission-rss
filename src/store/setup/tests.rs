use super::*;

async fn store() -> SetupStore {
    SetupStore::new(Db::open(":memory:").await.unwrap())
}

#[tokio::test]
async fn an_install_that_began_empty_has_a_first_run_with_nothing_skipped() {
    let store = store().await;
    assert_eq!(store.first_run().await.unwrap(), Some(FirstRun::default()));
}

#[tokio::test]
async fn skipping_a_step_and_taking_it_back_is_per_step() {
    let store = store().await;

    assert!(store.set_skipped(Step::Import, true, 10).await.unwrap());
    let run = store.first_run().await.unwrap().unwrap();
    assert!(run.skipped(Step::Import));
    assert!(!run.skipped(Step::Folder));

    assert!(store.set_skipped(Step::Folder, true, 20).await.unwrap());
    assert!(store.set_skipped(Step::Import, false, 30).await.unwrap());
    let run = store.first_run().await.unwrap().unwrap();
    assert!(run.skipped(Step::Folder));
    assert!(!run.skipped(Step::Import));
}

#[tokio::test]
async fn skipping_twice_keeps_the_first_time() {
    let db = Db::open(":memory:").await.unwrap();
    let store = SetupStore::new(db.clone());
    store.set_skipped(Step::Folder, true, 10).await.unwrap();
    store.set_skipped(Step::Folder, true, 99).await.unwrap();
    let at: i64 = db
        .run::<_, DbError, _>(|c| {
            Ok(c.query_row("SELECT folder_skipped_at FROM first_run", [], |r| r.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(at, 10);
}

#[tokio::test]
async fn an_install_that_did_not_begin_empty_has_nothing_to_skip() {
    let db = Db::open(":memory:").await.unwrap();
    db.run::<_, DbError, _>(|c| Ok(c.execute("DELETE FROM first_run", []).map(|_| ())?))
        .await
        .unwrap();
    let store = SetupStore::new(db);
    assert_eq!(store.first_run().await.unwrap(), None);
    assert!(!store.set_skipped(Step::Folder, true, 10).await.unwrap());
    assert_eq!(store.first_run().await.unwrap(), None);
}

#[test]
fn a_step_is_named_by_its_code() {
    for step in Step::ALL {
        assert_eq!(Step::parse(step.code()), Some(step));
    }
    assert_eq!(Step::parse("other"), None);
}
