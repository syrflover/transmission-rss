use std::collections::BTreeSet;

use super::*;
use crate::{
    discovery::{Scan, ScannedWork, WorkRead},
    store::library::{Followed, LibraryStore},
};

fn read(name: &str) -> WorkRead {
    WorkRead::Read(ScannedWork {
        dir_name: name.to_owned(),
        seasons: BTreeSet::new(),
        files: Vec::new(),
        unrecognized: Vec::new(),
    })
}

fn scan(names: &[&str]) -> Scan {
    Scan {
        works: names.iter().map(|n| read(n)).collect(),
    }
}

/// A library with a folder `/w` holding `names`, and the ID of each work.
async fn library(names: &[&str]) -> (Db, LibraryStore, String, Vec<String>) {
    let db = Db::open_blocking(":memory:").unwrap();
    let library = LibraryStore::new(db.clone());
    let (folder, _) = library
        .add_folder("/w".into(), scan(names), 100, &[])
        .await
        .unwrap();
    let ids = library
        .works(&folder.id)
        .await
        .unwrap()
        .into_iter()
        .map(|w| w.id)
        .collect();
    (db, library, folder.id, ids)
}

fn image(path: &str) -> ImageRef {
    ImageRef {
        id: new_id(),
        origin: Source::Upload,
        relative_path: path.to_owned(),
        byte_size: 3,
        sha256: "a".repeat(64),
        format: Format::Png,
    }
}

async fn staged_image(store: &ArtworkStore, path: &str, origin: Source) -> ImageRef {
    store
        .reserve_file(path, &format!("{path}.tmp"), 1)
        .await
        .unwrap();
    ImageRef {
        origin,
        ..image(path)
    }
}

async fn jobs(db: &Db) -> Vec<(String, Option<String>)> {
    db.run::<_, DbError, _>(|c| {
        let mut stmt = c.prepare("SELECT work_id, job FROM work_artwork ORDER BY work_id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_newly_recorded_work_is_auto_with_a_search_and_a_rescan_asks_for_none() {
    let (db, library, folder, ids) = library(&["Lycoris Recoil"]).await;
    let store = ArtworkStore::new(db.clone());
    let selection = store.selection(&ids[0]).await.unwrap();
    assert_eq!(selection.mode, Mode::Auto);
    assert_eq!(
        (
            selection.source,
            selection.anilist_media_id,
            &selection.image
        ),
        (None, None, &None)
    );
    assert_eq!(
        selection.job.as_ref().map(|j| j.kind),
        Some(JobKind::Search)
    );

    // The search ran and left nothing; a rescan (the worker's every cycle)
    // asks for no new one, a new work does.
    let claimed = store.next_job(200).await.unwrap().unwrap();
    assert_eq!(claimed.dir_name, "Lycoris Recoil");
    assert!(store
        .searched(&ids[0], claimed.version, Searched::Left(Note::NoMatch), 200)
        .await
        .unwrap());
    library
        .record_scan(&folder, Ok(scan(&["Lycoris Recoil", "Clevatess"])), 300)
        .await
        .unwrap();
    let pending: Vec<_> = jobs(&db)
        .await
        .into_iter()
        .filter(|(_, job)| job.is_some())
        .collect();
    assert_eq!(pending.len(), 1);
    assert_ne!(pending[0].0, ids[0]);
    assert_eq!(
        store.selection(&ids[0]).await.unwrap().note,
        Some(Note::NoMatch)
    );
}

#[tokio::test]
async fn the_migration_gives_works_recorded_before_it_one_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    {
        let conn = crate::store::db::database_at(&path, 10);
        conn.execute_batch(
            "INSERT INTO watch_folders (id, path, created_at) VALUES ('w', '/w', 1);
             INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('a', 'w', 'A');",
        )
        .unwrap();
    }
    let db = Db::open(&path).await.unwrap();
    assert_eq!(
        jobs(&db).await,
        [("a".to_owned(), Some("search".to_owned()))]
    );
    // Opening it again (a restart) adds nothing.
    drop(db);
    let db = Db::open(&path).await.unwrap();
    let store = ArtworkStore::new(db.clone());
    let claimed = store.next_job(i64::MAX).await.unwrap().unwrap();
    store
        .searched("a", claimed.version, Searched::Left(Note::NoMatch), 5)
        .await
        .unwrap();
    drop(db);
    let db = Db::open(&path).await.unwrap();
    assert_eq!(jobs(&db).await, [("a".to_owned(), None)]);
}

#[tokio::test]
async fn the_table_refuses_states_the_spec_does_not_allow() {
    let (db, _, _, ids) = library(&["A"]).await;
    let id = ids[0].clone();
    let refused = |sql: &'static str| {
        let (db, id) = (db.clone(), id.clone());
        async move {
            db.run::<_, DbError, _>(move |c| Ok(c.execute(sql, [&id]).map(|_| ())?))
                .await
                .is_err()
        }
    };
    // manual without a selection or without an image
    assert!(refused("UPDATE work_artwork SET mode = 'manual' WHERE work_id = ?1").await);
    assert!(
        refused(
            "UPDATE work_artwork SET mode = 'manual', source = 'anilist', anilist_media_id = 1
              WHERE work_id = ?1"
        )
        .await
    );
    // auto with an upload
    assert!(
        refused(
            "UPDATE work_artwork SET source = 'upload', image_id = 'i', image_origin = 'upload',
                 image_path = 'p', image_size = 1, image_sha256 = printf('%064d', 0),
                 image_format = 'png'
              WHERE work_id = ?1"
        )
        .await
    );
    // an AniList source without its ID, an ID without the source
    assert!(refused("UPDATE work_artwork SET source = 'anilist' WHERE work_id = ?1").await);
    assert!(refused("UPDATE work_artwork SET anilist_media_id = 5 WHERE work_id = ?1").await);
    // disabled with a selection
    assert!(
        refused(
            "UPDATE work_artwork SET mode = 'disabled', source = 'anilist', anilist_media_id = 1
              WHERE work_id = ?1"
        )
        .await
    );
    // half an image, a bad hash
    assert!(refused("UPDATE work_artwork SET image_id = 'x' WHERE work_id = ?1").await);
    assert!(
        refused(
            "UPDATE work_artwork SET mode = 'manual', source = 'upload', image_id = 'i',
                 image_origin = 'upload', image_path = 'p', image_size = 1,
                 image_sha256 = 'ABC', image_format = 'png'
              WHERE work_id = ?1"
        )
        .await
    );
    // An auto selection whose image is not ready is allowed.
    assert!(
        !refused(
            "UPDATE work_artwork SET source = 'anilist', anilist_media_id = 7 WHERE work_id = ?1"
        )
        .await
    );
}

#[tokio::test]
async fn a_change_from_an_old_version_changes_nothing() {
    let (db, _, _, ids) = library(&["A"]).await;
    let store = ArtworkStore::new(db);
    let v1 = store.selection(&ids[0]).await.unwrap().version;

    let first = store
        .select_manual(
            &ids[0],
            v1,
            None,
            staged_image(&store, "artwork/a.png", Source::Upload).await,
        )
        .await
        .unwrap();
    assert_eq!(first.mode, Mode::Manual);
    assert_eq!(first.version, v1 + 1);

    // A second screen still at v1.
    let late = staged_image(&store, "artwork/b.png", Source::Upload).await;
    match store.select_manual(&ids[0], v1, None, late).await {
        Err(ArtworkError::Conflict(current)) => assert_eq!(*current, first),
        other => panic!("expected a conflict, got {other:?}"),
    }
    match store.change(&ids[0], v1, UserChange::Clear, 5).await {
        Err(ArtworkError::Conflict(current)) => assert_eq!(*current, first),
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert_eq!(store.selection(&ids[0]).await.unwrap(), first);
    // The refused file is not a published one the cleanup would keep: still
    // `staging` until the caller hands it over.
}

#[tokio::test]
async fn a_late_automatic_result_never_undoes_a_newer_user_choice() {
    let (db, _, _, ids) = library(&["A", "B", "C"]).await;
    let store = ArtworkStore::new(db);
    let job = |id: &str| {
        let (store, id) = (store.clone(), id.to_owned());
        async move {
            let s = store.selection(&id).await.unwrap();
            assert_eq!(s.job.unwrap().kind, JobKind::Search);
            s.version
        }
    };

    // A: the user uploads while the search runs.
    let va = job(&ids[0]).await;
    store
        .select_manual(
            &ids[0],
            va,
            None,
            staged_image(&store, "artwork/a.png", Source::Upload).await,
        )
        .await
        .unwrap();
    let selected = Searched::Selected {
        anilist_media_id: 9,
        image_url: None,
    };
    assert!(!store
        .searched(&ids[0], va, selected.clone(), 9)
        .await
        .unwrap());
    let a = store.selection(&ids[0]).await.unwrap();
    assert_eq!((a.mode, a.source), (Mode::Manual, Some(Source::Upload)));
    assert_eq!(a.job, None);

    // B: the user clears while the search runs.
    let vb = job(&ids[1]).await;
    store
        .change(&ids[1], vb, UserChange::Clear, 5)
        .await
        .unwrap();
    assert!(!store
        .searched(&ids[1], vb, selected.clone(), 9)
        .await
        .unwrap());
    let b = store.selection(&ids[1]).await.unwrap();
    assert_eq!((b.mode, b.source, b.job), (Mode::Disabled, None, None));

    // C: the search selects, then the user picks another entry while the
    // image of the first is being fetched; the fetched image is dropped.
    let vc = job(&ids[2]).await;
    assert!(store.searched(&ids[2], vc, selected, 9).await.unwrap());
    let c = store.selection(&ids[2]).await.unwrap();
    assert_eq!(
        (c.mode, c.source, c.anilist_media_id),
        (Mode::Auto, Some(Source::Anilist), Some(9))
    );
    assert_eq!(c.job.as_ref().unwrap().kind, JobKind::Fetch);
    assert!(c.image.is_none(), "no image before it is verified");
    let fetch_version = c.version;
    let picked = staged_image(&store, "artwork/picked.jpg", Source::Anilist).await;
    store
        .select_manual(&ids[2], fetch_version, Some(12), picked)
        .await
        .unwrap();
    let auto_image = staged_image(&store, "artwork/auto.jpg", Source::Anilist).await;
    assert!(!store
        .fetched(&ids[2], fetch_version, 9, auto_image)
        .await
        .unwrap());
    let c = store.selection(&ids[2]).await.unwrap();
    assert_eq!((c.mode, c.anilist_media_id), (Mode::Manual, Some(12)));
    assert_eq!(c.image.unwrap().relative_path, "artwork/picked.jpg");
    // The dropped image is handed to the cleanup.
    let published = store
        .run(|c| Ok(files_of_state(c, "published")?))
        .await
        .unwrap();
    assert!(published
        .iter()
        .any(|f| f.relative_path == "artwork/auto.jpg"));
}

#[tokio::test]
async fn back_to_auto_searches_again_and_repair_fetches_the_same_id() {
    let (db, _, _, ids) = library(&["A"]).await;
    let store = ArtworkStore::new(db);
    let v = store.selection(&ids[0]).await.unwrap().version;
    let manual = store
        .select_manual(
            &ids[0],
            v,
            Some(4),
            staged_image(&store, "artwork/a.jpg", Source::Anilist).await,
        )
        .await
        .unwrap();

    // Repair keeps the choice and asks for its image.
    let repaired = store
        .change(&ids[0], manual.version, UserChange::Repair, 50)
        .await
        .unwrap();
    assert_eq!(
        repaired.version, manual.version,
        "what is selected did not change"
    );
    assert_eq!(repaired.mode, Mode::Manual);
    let claimed = store.next_job(60).await.unwrap().unwrap();
    assert_eq!(
        (claimed.kind, claimed.anilist_media_id),
        (JobKind::Fetch, Some(4))
    );

    let auto = store
        .change(&ids[0], repaired.version, UserChange::Auto, 70)
        .await
        .unwrap();
    assert_eq!(
        (auto.mode, auto.source, auto.anilist_media_id, &auto.image),
        (Mode::Auto, None, None, &None)
    );
    assert_eq!(auto.job.unwrap().kind, JobKind::Search);

    // An upload cannot be repaired: there is nothing to fetch.
    let up = store
        .select_manual(
            &ids[0],
            auto.version,
            None,
            staged_image(&store, "artwork/u.png", Source::Upload).await,
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .change(&ids[0], up.version, UserChange::Repair, 80)
            .await,
        Err(ArtworkError::Invalid(_))
    ));
}

#[tokio::test]
async fn a_selection_follows_its_work_through_an_archive_move() {
    let db = Db::open_blocking(":memory:").unwrap();
    let library = LibraryStore::new(db.clone());
    let (collect, _) = library
        .add_folder("/collect".into(), scan(&["Lycoris Recoil"]), 1, &[])
        .await
        .unwrap();
    let folders = library.folders().await.unwrap();
    let (archive, _) = library
        .add_folder("/archive".into(), scan(&[]), 2, &folders)
        .await
        .unwrap();
    let id = library.works(&collect.id).await.unwrap()[0].id.clone();
    let store = ArtworkStore::new(db);
    let v = store.selection(&id).await.unwrap().version;
    let before = store
        .select_manual(
            &id,
            v,
            None,
            staged_image(&store, "artwork/l.png", Source::Upload).await,
        )
        .await
        .unwrap();

    assert_eq!(
        library
            .follow_move(&collect.id, &archive.id, "Lycoris Recoil")
            .await
            .unwrap(),
        Followed::Moved
    );
    assert_eq!(library.works(&archive.id).await.unwrap()[0].id, id);
    assert_eq!(store.selection(&id).await.unwrap(), before);
}

#[tokio::test]
async fn requests_take_turns_and_a_block_holds_them_all() {
    let db = Db::open_blocking(":memory:").unwrap();
    let store = ArtworkStore::new(db);
    assert_eq!(
        store.take_request_slot(1000, 2000, None).await.unwrap(),
        Ok(1000)
    );
    assert_eq!(
        store.take_request_slot(1000, 2000, None).await.unwrap(),
        Ok(3000)
    );
    // A caller that may wait 1s does not take a slot 4s away.
    assert_eq!(
        store
            .take_request_slot(1000, 2000, Some(1000))
            .await
            .unwrap(),
        Err(4000)
    );
    assert_eq!(
        store.take_request_slot(1000, 2000, None).await.unwrap(),
        Ok(5000)
    );
    store.block_requests(60_000).await.unwrap();
    assert_eq!(
        store.take_request_slot(7000, 2000, None).await.unwrap(),
        Ok(60_000)
    );
    // A shorter block later does not shorten it.
    store.block_requests(10_000).await.unwrap();
    assert_eq!(
        store.take_request_slot(8000, 2000, None).await.unwrap(),
        Ok(62_000)
    );
}

#[tokio::test]
async fn removing_a_work_removes_its_selection_but_not_its_file_record() {
    let (db, library, folder, ids) = library(&["A"]).await;
    let store = ArtworkStore::new(db.clone());
    let v = store.selection(&ids[0]).await.unwrap().version;
    store
        .select_manual(
            &ids[0],
            v,
            None,
            staged_image(&store, "artwork/a.png", Source::Upload).await,
        )
        .await
        .unwrap();
    library.remove_folder(&folder).await.unwrap();
    assert!(matches!(
        store.selection(&ids[0]).await,
        Err(ArtworkError::NotFound)
    ));
    // The file stays on record (published, unreferenced) for the cleanup.
    let published = store
        .run(|c| Ok(files_of_state(c, "published")?))
        .await
        .unwrap();
    assert_eq!(published.len(), 1);
    assert!(store
        .run(|c| Ok(referenced_paths(c)?))
        .await
        .unwrap()
        .is_empty());
}
