use super::*;
use crate::db::Db;

async fn store() -> SettingsStore {
    SettingsStore::new(Db::open(":memory:").await.unwrap())
}

#[tokio::test]
async fn the_collection_settings_are_unset_until_written_and_versioned_after() {
    let store = store().await;
    assert_eq!(store.collection().await.unwrap(), None);

    let first = store
        .put_collection(
            0,
            "/downloads/Shows (current)".into(),
            Some("/downloads/Shows".into()),
        )
        .await
        .unwrap();
    assert_eq!(first.version, 1);
    assert_eq!(first.archive_folder.as_deref(), Some("/downloads/Shows"));
    assert_eq!(store.collection().await.unwrap(), Some(first));

    let second = store
        .put_collection(1, "/downloads/Shows (current)".into(), None)
        .await
        .unwrap();
    assert_eq!((second.version, second.archive_folder), (2, None));
}

#[tokio::test]
async fn a_write_from_a_stale_version_changes_nothing() {
    let store = store().await;
    // Nothing exists yet, so a version other than 0 is stale.
    let err = store
        .put_collection(3, "/a".into(), None)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        SettingsError::Conflict {
            expected: 3,
            actual: 0
        }
    ));
    assert_eq!(store.collection().await.unwrap(), None);

    store.put_collection(0, "/a".into(), None).await.unwrap();
    // The first writer's version no longer matches after a second write.
    store.put_collection(1, "/b".into(), None).await.unwrap();
    let err = store
        .put_collection(1, "/c".into(), None)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        SettingsError::Conflict {
            expected: 1,
            actual: 2
        }
    ));
    assert_eq!(store.collection().await.unwrap().unwrap().folder, "/b");
    // A creation over an existing row is stale too.
    let err = store
        .put_collection(0, "/d".into(), None)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        SettingsError::Conflict {
            expected: 0,
            actual: 2
        }
    ));
}

#[tokio::test]
async fn an_empty_folder_is_refused() {
    let store = store().await;
    assert!(matches!(
        store.put_collection(0, String::new(), None).await,
        Err(SettingsError::Invalid(_))
    ));
    assert!(matches!(
        store
            .put_collection(0, "/a".into(), Some(String::new()))
            .await,
        Err(SettingsError::Invalid(_))
    ));
    assert_eq!(store.collection().await.unwrap(), None);
}
