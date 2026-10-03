use std::collections::BTreeSet;

use rusqlite::TransactionBehavior;

use crate::{
    discovery::{Scan, ScannedWork, WorkRead},
    store::{
        library::{Followed, LibraryStore},
        seasons::{
            anissia::{link_in, set_in, AnissiaLink},
            SeasonError, SeasonStore,
        },
    },
};
use trss_core::{Db, DbError};

fn scan(works: &[(&str, &[u32])]) -> Scan {
    Scan {
        works: works
            .iter()
            .map(|(name, seasons)| {
                WorkRead::Read(ScannedWork {
                    dir_name: (*name).to_owned(),
                    seasons: BTreeSet::from_iter(seasons.iter().copied()),
                    files: Vec::new(),
                    unrecognized: Vec::new(),
                })
            })
            .collect(),
    }
}

/// A library with `/w` holding `A` (seasons 1 and 2), and two Anissia anime.
async fn env() -> (Db, LibraryStore, SeasonStore, String, String) {
    let db = Db::open_blocking(":memory:").unwrap();
    db.run::<_, DbError, _>(|c| {
        c.execute_batch(
            "INSERT INTO anissia_anime (anime_no, subject, week, status, fetched_at)
             VALUES (7, '하나', 1, 'END', 10), (8, '둘', 2, 'ON', 10);",
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let library = LibraryStore::new(db.clone());
    let (folder, _) = library
        .add_folder("/w".into(), scan(&[("A", &[1, 2])]), 100, &[])
        .await
        .unwrap();
    let work = library.works(&folder.id).await.unwrap()[0].id.clone();
    (db.clone(), library, SeasonStore::new(db), folder.id, work)
}

async fn set(db: &Db, work: &str, season: u32, anime_no: Option<i64>) -> AnissiaLink {
    let work = work.to_owned();
    db.run::<_, DbError, _>(move |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let link = set_in(&tx, &work, season, anime_no)?;
        tx.commit()?;
        Ok(link)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_season_never_linked_has_version_zero_and_each_change_moves_the_version_on() {
    let (db, _library, store, _folder, work) = env().await;

    let untouched = store.anissia_link(&work, 1).await.unwrap();
    assert_eq!(
        untouched,
        AnissiaLink {
            work_id: work.clone(),
            season: 1,
            version: 0,
            anime_no: None
        }
    );

    assert_eq!(set(&db, &work, 1, Some(7)).await.version, 1);
    // The same link again changes nothing, not even the version.
    assert_eq!(set(&db, &work, 1, Some(7)).await.version, 1);
    assert_eq!(set(&db, &work, 1, Some(8)).await.version, 2);
    // Cutting the link keeps the row, so the version goes on.
    let cut = set(&db, &work, 1, None).await;
    assert_eq!((cut.version, cut.anime_no), (3, None));
    // Cutting a link that is cut changes nothing.
    assert_eq!(set(&db, &work, 1, None).await.version, 3);
    assert_eq!(set(&db, &work, 1, Some(7)).await.version, 4);

    // Another season of the work is its own link.
    assert_eq!(store.anissia_link(&work, 2).await.unwrap().version, 0);
    set(&db, &work, 2, Some(8)).await;
    let links = store.anissia_links_of(&work).await.unwrap();
    assert_eq!(
        links
            .values()
            .map(|l| (l.season, l.version, l.anime_no))
            .collect::<Vec<_>>(),
        [(1, 4, Some(7)), (2, 1, Some(8))]
    );
}

#[tokio::test]
async fn only_a_season_of_the_work_has_a_link_to_read() {
    let (_db, _library, store, _folder, work) = env().await;
    assert!(matches!(
        store.anissia_link(&work, 5).await,
        Err(SeasonError::NotFound)
    ));
    assert!(matches!(
        store.anissia_link("nobody", 1).await,
        Err(SeasonError::NotFound)
    ));
}

#[tokio::test]
async fn a_link_outlives_its_season_row_and_goes_with_its_work() {
    let (db, library, store, folder, work) = env().await;
    set(&db, &work, 2, Some(7)).await;

    // The season folder is gone for a moment, then back.
    library
        .record_scan(&folder, Ok(scan(&[("A", &[1])])), 200)
        .await
        .unwrap()
        .unwrap();
    let kept = db
        .run::<_, DbError, _>({
            let work = work.clone();
            move |c| Ok(link_in(c, &work, 2)?)
        })
        .await
        .unwrap();
    assert_eq!((kept.version, kept.anime_no), (1, Some(7)));
    library
        .record_scan(&folder, Ok(scan(&[("A", &[1, 2])])), 300)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        store.anissia_link(&work, 2).await.unwrap().anime_no,
        Some(7)
    );

    library.remove_folder(&folder, 400).await.unwrap();
    let rows: i64 = db
        .run::<_, DbError, _>(|c| {
            Ok(c.query_row("SELECT count(*) FROM season_anissia", [], |r| r.get(0))?)
        })
        .await
        .unwrap();
    // Out of the library the work is hidden and kept, its link with it.
    assert_eq!(rows, 1);
}

/// Adds the archive folder holding its own work `A`: the folder's ID and the
/// ID of that (kept) work.
async fn archive(library: &LibraryStore, moved: &str) -> (String, String) {
    let folders = library.folders().await.unwrap();
    let (archive, _) = library
        .add_folder("/archive".into(), scan(&[("A", &[1, 2])]), 150, &folders)
        .await
        .unwrap();
    let kept = library.works(&archive.id).await.unwrap()[0].id.clone();
    assert_ne!(kept, moved);
    (archive.id, kept)
}

fn summary(links: &std::collections::BTreeMap<u32, AnissiaLink>) -> Vec<(u32, i64, Option<i64>)> {
    links
        .values()
        .map(|l| (l.season, l.version, l.anime_no))
        .collect()
}

#[tokio::test]
async fn an_archive_move_that_merges_two_works_fills_only_the_seasons_the_kept_work_has_no_row_for()
{
    let (db, library, store, collect, moved) = env().await;
    let (archive, kept) = archive(&library, &moved).await;

    // The moved work links seasons 1 to 3; the kept one has season 1 cut (a row
    // with no anime), season 2 linked to another anime and no row for season 3.
    set(&db, &moved, 1, Some(7)).await;
    set(&db, &moved, 2, Some(7)).await;
    set(&db, &moved, 3, Some(7)).await;
    set(&db, &kept, 1, Some(8)).await;
    set(&db, &kept, 1, None).await;
    set(&db, &kept, 2, Some(8)).await;

    assert_eq!(
        library.follow_move(&collect, &archive, "A").await.unwrap(),
        Followed::Merged
    );

    // The cut and the other anime stay as they were; only season 3, which the
    // kept work had no row for, comes over, with a version past both.
    assert_eq!(
        summary(&store.anissia_links_of(&kept).await.unwrap()),
        [(1, 2, None), (2, 1, Some(8)), (3, 2, Some(7))]
    );
    assert!(store.anissia_links_of(&moved).await.unwrap().is_empty());
}

/// A rule `id` on channel `c`, with a subscription to `anime` connected to `season`.
async fn subscribe(db: &Db, id: &str, anime: i64, season: Option<String>) {
    let id = id.to_owned();
    db.run::<_, DbError, _>(move |c| {
        c.execute_batch(
            "INSERT OR IGNORE INTO channels (id, position, url, excludes, secret_query, version)
             VALUES ('c', 0, 'http://x/feed', '[]', '[]', 1);",
        )?;
        c.execute(
            "INSERT INTO rules (id, channel_id, position, match_text, regex, case_insensitive,
                                directory, episode, episode_auto, state, version)
             VALUES (?1, 'c', (SELECT count(*) FROM rules), 'A', 0, 1, 'A', 1, 0, 'active', 1)",
            [&id],
        )?;
        c.execute(
            "INSERT INTO rule_subscriptions (rule_id, anissia_anime_no, subtitles, creator,
                                             season_id, subscribed_at)
             VALUES (?1, ?2, 'none', NULL, ?3, 1)",
            rusqlite::params![id, anime, season],
        )?;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn an_archive_move_that_merges_two_works_keeps_the_subtitle_sources_mappings() {
    let (db, library, _, collect, moved) = env().await;
    let (archive, kept) = archive(&library, &moved).await;
    let (m, k) = (moved.clone(), kept.clone());
    db.run::<_, DbError, _>(move |c| {
        c.execute_batch(
            "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                 VALUES ('s1', 7, '가', 1), ('s2', 7, '나', 1), ('s3', 7, '다', 1);",
        )?;
        let put = |work: &str, source: &str, kind: &str, offset: Option<i64>| {
            c.execute(
                "INSERT INTO subtitle_episode_mappings
                     (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
                 VALUES (?1, 1, ?2, ?3, ?4, ?5, 1)",
                rusqlite::params![work, source, kind, offset, format!("{work} {source}")],
            )
        };
        // Only the moved work maps s1 (the app's, gone undecided from an
        // offset it keeps); both map s2 (the kept work's is the app's, the
        // moved one the user's) and s3 (both the app's).
        put(&m, "s1", "undecided", None)?;
        c.execute(
            "UPDATE subtitle_episode_mappings SET retired_offset = 2
              WHERE work_id = ?1 AND source_id = 's1'",
            [&m],
        )?;
        put(&m, "s2", "user", Some(-12))?;
        put(&k, "s2", "auto", Some(0))?;
        put(&m, "s3", "auto", Some(-1))?;
        put(&k, "s3", "undecided", None)?;
        // Each mapping has the conflicts it was found against.
        let conflict = |work: &str, source: &str, episode: &str| {
            c.execute(
                "INSERT INTO subtitle_mapping_conflicts
                     (work_id, season, source_id, episode, reason, found_at)
                 VALUES (?1, 1, ?2, ?3, '근거', 1)",
                rusqlite::params![work, source, episode],
            )
        };
        conflict(&m, "s1", "13.5")?;
        conflict(&m, "s2", "14.5")?;
        conflict(&k, "s2", "SP")?;
        conflict(&m, "s3", "15.5")?;
        conflict(&k, "s3", "SP")?;
        Ok(())
    })
    .await
    .unwrap();

    assert_eq!(
        library.follow_move(&collect, &archive, "A").await.unwrap(),
        Followed::Merged
    );
    type Row = (String, String, String, Option<i64>, Option<i64>);
    let rows: Vec<Row> = db
        .run::<_, DbError, _>(|c| {
            let mut stmt = c.prepare(
                "SELECT work_id, source_id, kind, episode_offset, retired_offset
                   FROM subtitle_episode_mappings ORDER BY source_id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .unwrap();
    let k = |source: &str, kind: &str, offset: Option<i64>, retired: Option<i64>| {
        (
            kept.clone(),
            source.to_owned(),
            kind.to_owned(),
            offset,
            retired,
        )
    };
    // The offset an undecided mapping keeps goes with it.
    assert_eq!(
        rows,
        [
            k("s1", "undecided", None, Some(2)),
            k("s2", "user", Some(-12), None),
            k("s3", "undecided", None, None)
        ]
    );
    // The conflicts follow the mapping that is kept: the moved work's for s1
    // (nothing to replace) and s2 (the user's mapping won), the kept work's
    // for s3 (its own mapping stayed).
    let conflicts: Vec<(String, String, String)> = db
        .run::<_, DbError, _>(|c| {
            let mut stmt = c.prepare(
                "SELECT work_id, source_id, episode
                   FROM subtitle_mapping_conflicts ORDER BY source_id, episode",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .unwrap();
    let c = |source: &str, episode: &str| (kept.clone(), source.to_owned(), episode.to_owned());
    assert_eq!(conflicts, [c("s1", "13.5"), c("s2", "14.5"), c("s3", "SP")]);
}

/// A rule's season, its note and its version.
async fn rule(db: &Db, id: &str) -> (Option<String>, Option<String>, i64) {
    let id = id.to_owned();
    db.run::<_, DbError, _>(move |c| {
        Ok(c.query_row(
            "SELECT s.season_id, s.season_blocked, r.version
               FROM rule_subscriptions s JOIN rules r ON r.id = s.rule_id WHERE s.rule_id = ?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn an_archive_move_that_merges_two_works_moves_the_subscriptions_with_their_links() {
    let (db, library, store, collect, moved) = env().await;
    let (archive, kept) = archive(&library, &moved).await;

    // Season 1 is subscribed (its link is the subscription's); the kept work
    // has no row for it. Season 2 is subscribed to anime 7 while the kept
    // work's season 2 was cut by the user.
    subscribe(&db, "r1", 7, Some(format!("{moved}:1"))).await;
    set(&db, &moved, 1, Some(7)).await;
    subscribe(&db, "r2", 7, Some(format!("{moved}:2"))).await;
    set(&db, &moved, 2, Some(7)).await;
    set(&db, &kept, 2, Some(8)).await;
    set(&db, &kept, 2, None).await;

    assert_eq!(
        library.follow_move(&collect, &archive, "A").await.unwrap(),
        Followed::Merged
    );

    // Both subscriptions name the kept work's seasons and the links agree.
    assert_eq!(rule(&db, "r1").await, (Some(format!("{kept}:1")), None, 2));
    assert_eq!(rule(&db, "r2").await, (Some(format!("{kept}:2")), None, 2));
    assert_eq!(
        summary(&store.anissia_links_of(&kept).await.unwrap()),
        [(1, 2, Some(7)), (2, 3, Some(7))]
    );
}

#[tokio::test]
async fn a_merged_subscription_whose_season_is_held_by_another_anime_is_noted_not_connected() {
    let (db, library, store, collect, moved) = env().await;
    let (archive, kept) = archive(&library, &moved).await;

    // Season 1: the kept work's season is linked to another anime. Season 2:
    // another subscription of another anime holds the kept work's season.
    subscribe(&db, "r1", 7, Some(format!("{moved}:1"))).await;
    set(&db, &moved, 1, Some(7)).await;
    set(&db, &kept, 1, Some(8)).await;
    subscribe(&db, "r2", 7, Some(format!("{moved}:2"))).await;
    set(&db, &moved, 2, Some(7)).await;
    subscribe(&db, "r3", 8, Some(format!("{kept}:2"))).await;
    set(&db, &kept, 2, Some(8)).await;
    // A rule that was already noting a season of the moved work.
    subscribe(&db, "r4", 8, None).await;
    db.run::<_, DbError, _>({
        let note = format!("{moved}:2");
        move |c| {
            c.execute(
                "UPDATE rule_subscriptions SET season_blocked = ?1 WHERE rule_id = 'r4'",
                [note],
            )?;
            Ok(())
        }
    })
    .await
    .unwrap();

    library.follow_move(&collect, &archive, "A").await.unwrap();

    // Neither is connected to the season it moved to, and both note it; the
    // worker finds them without a season and says why in the rule's detail.
    assert_eq!(rule(&db, "r1").await, (None, Some(format!("{kept}:1")), 2));
    assert_eq!(rule(&db, "r2").await, (None, Some(format!("{kept}:2")), 2));
    // The holder is untouched, and so are the kept work's links.
    assert_eq!(rule(&db, "r3").await, (Some(format!("{kept}:2")), None, 1));
    assert_eq!(
        summary(&store.anissia_links_of(&kept).await.unwrap()),
        [(1, 1, Some(8)), (2, 1, Some(8))]
    );
    // A note about the moved work's season is about the kept work's now.
    assert_eq!(rule(&db, "r4").await, (None, Some(format!("{kept}:2")), 2));
}

#[tokio::test]
async fn an_anime_is_linked_while_some_season_links_it() {
    let (db, _library, store, _folder, work) = env().await;
    assert!(!store.anime_is_linked(7).await.unwrap());

    set(&db, &work, 1, Some(7)).await;
    assert!(store.anime_is_linked(7).await.unwrap());
    assert!(!store.anime_is_linked(8).await.unwrap());

    // A cut link links nothing.
    set(&db, &work, 1, None).await;
    assert!(!store.anime_is_linked(7).await.unwrap());
}
