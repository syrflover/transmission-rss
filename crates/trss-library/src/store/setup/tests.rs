use crate::store::setup::*;

async fn store() -> SetupStore {
    SetupStore::new(Db::open(":memory:").await.unwrap())
}

async fn exec(db: &Db, sql: &'static str) {
    db.run::<_, DbError, _>(move |c| Ok(c.execute_batch(sql)?))
        .await
        .unwrap();
}

const ADD_FOLDER: &str = "INSERT INTO watch_folders (id, path, created_at) VALUES ('w', '/w', 5)";
const REMOVE_FOLDER: &str = "UPDATE watch_folders SET unregistered_at = 9 WHERE id = 'w'";
const ADD_CHANNEL: &str =
    "INSERT INTO channels (id, position, url, excludes, secret_query, version)
                           VALUES ('c1', 0, 'http://x/', '[]', '[]', 1)";

#[tokio::test]
async fn an_install_that_began_empty_has_a_first_run_with_nothing_done_or_skipped() {
    let store = store().await;
    let run = store.first_run().await.unwrap().unwrap();
    assert_eq!(run, FirstRun::default());
    assert!(run.active());
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
    assert_eq!(store.settle(1).await.unwrap(), None);
    assert!(!store.set_skipped(Step::Folder, true, 10).await.unwrap());
    store.mark_import_applied(10).await.unwrap();
    assert_eq!(store.first_run().await.unwrap(), None);
}

#[tokio::test]
async fn registering_a_folder_finishes_the_folder_step_for_good() {
    let db = Db::open(":memory:").await.unwrap();
    let store = SetupStore::new(db.clone());

    exec(&db, ADD_FOLDER).await;
    let run = store.first_run().await.unwrap().unwrap();
    assert!(run.done(Step::Folder));
    assert!(!run.done(Step::Import));

    // Unregistering the folder does not undo the step.
    exec(&db, REMOVE_FOLDER).await;
    assert!(store.first_run().await.unwrap().unwrap().done(Step::Folder));
}

#[tokio::test]
async fn a_folder_that_comes_back_finishes_the_folder_step_too() {
    let db = Db::open(":memory:").await.unwrap();
    let store = SetupStore::new(db.clone());
    // A folder unregistered before the first run's row existed in this test.
    exec(&db, "DELETE FROM first_run").await;
    exec(&db, ADD_FOLDER).await;
    exec(&db, REMOVE_FOLDER).await;
    exec(&db, "INSERT INTO first_run (id) VALUES (1)").await;
    assert!(!store.first_run().await.unwrap().unwrap().done(Step::Folder));

    exec(
        &db,
        "UPDATE watch_folders SET unregistered_at = NULL, created_at = 40 WHERE id = 'w'",
    )
    .await;
    assert!(store.first_run().await.unwrap().unwrap().done(Step::Folder));
}

#[tokio::test]
async fn a_channel_alone_does_not_finish_the_import_step_but_an_applied_import_does() {
    let db = Db::open(":memory:").await.unwrap();
    let store = SetupStore::new(db.clone());

    exec(&db, ADD_CHANNEL).await;
    assert!(!store.first_run().await.unwrap().unwrap().done(Step::Import));

    store.mark_import_applied(10).await.unwrap();
    store.mark_import_applied(99).await.unwrap();
    assert!(store.first_run().await.unwrap().unwrap().done(Step::Import));
    let at: i64 = db
        .run::<_, DbError, _>(|c| {
            Ok(c.query_row("SELECT import_applied_at FROM first_run", [], |r| r.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(at, 10, "the first time is kept");
}

#[tokio::test]
async fn the_checklist_ends_once_both_steps_are_done_or_skipped_and_stays_ended() {
    let db = Db::open(":memory:").await.unwrap();
    let store = SetupStore::new(db.clone());

    exec(&db, ADD_FOLDER).await;
    assert!(!store.settle(100).await.unwrap().unwrap().ended);

    store.set_skipped(Step::Import, true, 110).await.unwrap();
    let run = store.first_run().await.unwrap().unwrap();
    assert!(run.ended, "the skip that settled the last step ended it");
    assert!(!run.active());

    // Removing the folder, adding channels or applying an import changes nothing.
    exec(&db, REMOVE_FOLDER).await;
    exec(&db, ADD_CHANNEL).await;
    store.mark_import_applied(120).await.unwrap();
    let run = store.settle(130).await.unwrap().unwrap();
    assert!(run.ended);
    assert!(!run.active());
}

#[tokio::test]
async fn settling_ends_a_checklist_finished_by_the_data_and_keeps_its_first_end() {
    let db = Db::open(":memory:").await.unwrap();
    let store = SetupStore::new(db.clone());
    exec(&db, ADD_FOLDER).await;
    store.mark_import_applied(10).await.unwrap();
    // Nobody has asked yet: the end is not written until someone looks.
    assert!(!store.first_run().await.unwrap().unwrap().ended);

    assert!(store.settle(100).await.unwrap().unwrap().ended);
    store.settle(200).await.unwrap();
    let at: i64 = db
        .run::<_, DbError, _>(|c| {
            Ok(c.query_row("SELECT ended_at FROM first_run", [], |r| r.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(at, 100);
}

#[tokio::test]
async fn taking_back_the_last_skip_brings_the_checklist_back() {
    let db = Db::open(":memory:").await.unwrap();
    let store = SetupStore::new(db.clone());
    exec(&db, ADD_FOLDER).await;
    store.set_skipped(Step::Import, true, 10).await.unwrap();
    assert!(!store.first_run().await.unwrap().unwrap().active());

    store.set_skipped(Step::Import, false, 20).await.unwrap();
    let run = store.first_run().await.unwrap().unwrap();
    assert!(run.active());
    assert!(!run.ended);
}

#[tokio::test]
async fn taking_back_a_skip_of_a_step_that_is_done_keeps_the_end() {
    let db = Db::open(":memory:").await.unwrap();
    let store = SetupStore::new(db.clone());
    exec(&db, ADD_FOLDER).await;
    store.set_skipped(Step::Import, true, 10).await.unwrap();
    // The import is applied after the skip: the step is done as well.
    store.mark_import_applied(15).await.unwrap();

    store.set_skipped(Step::Import, false, 20).await.unwrap();
    let run = store.first_run().await.unwrap().unwrap();
    assert!(run.ended);
    assert!(!run.active());
}

#[test]
fn a_step_is_named_by_its_code() {
    for step in Step::ALL {
        assert_eq!(Step::parse(step.code()), Some(step));
    }
    assert_eq!(Step::parse("other"), None);
}
