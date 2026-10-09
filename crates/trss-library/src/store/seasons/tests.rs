use std::collections::BTreeSet;

use trss_anilist::{Airing, FuzzyDate, Sequel};

use crate::{
    discovery::{Scan, ScannedWork, WorkRead},
    store::{library::LibraryStore, seasons::*},
};

fn scanned(name: &str, seasons: &[u32]) -> WorkRead {
    WorkRead::Read(ScannedWork {
        dir_name: name.to_owned(),
        seasons: BTreeSet::from_iter(seasons.iter().copied()),
        files: Vec::new(),
        unrecognized: Vec::new(),
    })
}

fn scan(works: &[(&str, &[u32])]) -> Scan {
    Scan {
        works: works.iter().map(|(n, s)| scanned(n, s)).collect(),
    }
}

struct Env {
    library: LibraryStore,
    store: SeasonStore,
    db: Db,
    folder: String,
}

impl Env {
    async fn new(works: &[(&str, &[u32])]) -> Env {
        let db = Db::open_blocking(":memory:").unwrap();
        let library = LibraryStore::new(db.clone());
        let (folder, _) = library
            .add_folder("/w".into(), scan(works), 100, &[])
            .await
            .unwrap();
        Env {
            library,
            store: SeasonStore::new(db.clone()),
            db,
            folder: folder.id,
        }
    }

    async fn id(&self, name: &str) -> String {
        self.library
            .works(&self.folder)
            .await
            .unwrap()
            .into_iter()
            .find(|w| w.dir_name == name)
            .unwrap()
            .id
    }

    /// Reads the folder again as it is now (`works`), as a later scan does.
    async fn rescan(&self, works: &[(&str, &[u32])]) {
        self.library
            .record_scan(&self.folder, Ok(scan(works)), 200)
            .await
            .unwrap()
            .unwrap();
    }

    /// The seasons that have a search to do, by work name.
    async fn searches(&self) -> Vec<(String, u32)> {
        self.db
            .run::<_, DbError, _>(|c| {
                let mut stmt = c.prepare(
                    "SELECT w.dir_name, i.season FROM season_info i JOIN works w ON w.id = i.work_id
                      WHERE i.job = 'search' ORDER BY w.dir_name, i.season",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                Ok(rows.collect::<rusqlite::Result<_>>()?)
            })
            .await
            .unwrap()
    }

    async fn rows(&self) -> i64 {
        self.db
            .run::<_, DbError, _>(|c| {
                Ok(c.query_row("SELECT count(*) FROM season_info", [], |r| r.get(0))?)
            })
            .await
            .unwrap()
    }
}

fn entry(id: i64, status: &str, fetched_at: i64) -> Entry {
    Entry {
        id,
        romaji: Some(format!("Entry {id}")),
        english: None,
        native: Some(format!("エントリー {id}")),
        format: Some("TV".into()),
        status: Some(status.into()),
        episodes: Some(12),
        start: FuzzyDate {
            year: Some(2022),
            month: Some(4),
            day: None,
        },
        end: FuzzyDate::default(),
        studios: vec!["Studio".into()],
        genres: vec!["Action".into()],
        description: Some("Text<br>more".into()),
        airing: vec![Airing {
            episode: 1,
            at: 100,
        }],
        korean_titles: Vec::new(),
        sequels: vec![Sequel {
            id: id + 100,
            romaji: Some("Next".into()),
            english: None,
            native: None,
            format: Some("TV".into()),
            status: None,
            start: FuzzyDate::default(),
        }],
        fetched_at,
    }
}

#[tokio::test]
async fn a_newly_recorded_first_season_gets_one_search_and_nothing_else_does() {
    let env = Env::new(&[
        ("Lycoris Recoil", &[1, 2, 3]),
        ("Specials Only", &[0]),
        ("Bare", &[]),
    ])
    .await;
    // Season 1 only: not the later seasons, not season 0 (specials), not a work with no season.
    assert_eq!(env.searches().await, [("Lycoris Recoil".to_owned(), 1)]);

    // Reading the folder again, any number of times, adds none and changes none.
    env.rescan(&[
        ("Lycoris Recoil", &[1, 2, 3]),
        ("Specials Only", &[0]),
        ("Bare", &[]),
    ])
    .await;
    env.rescan(&[
        ("Lycoris Recoil", &[1, 2, 3]),
        ("Specials Only", &[0]),
        ("Bare", &[]),
    ])
    .await;
    assert_eq!(env.rows().await, 1);
    let id = env.id("Lycoris Recoil").await;
    let link = env.store.link(&id, 1).await.unwrap();
    assert_eq!((link.version, link.origin), (1, Origin::Auto));
    assert!(link.job.is_some());
    // A season that has no row is untouched: version 0, nothing linked, no search.
    let later = env.store.link(&id, 2).await.unwrap();
    assert_eq!(
        (later.version, later.job, later.entries.len()),
        (0, None, 0)
    );
    assert!(matches!(
        env.store.link(&id, 9).await,
        Err(SeasonError::NotFound)
    ));

    // Several seasons at once: a recorded one has its link, one the work has no
    // record of is `None`, in the order asked.
    let many = env
        .store
        .links_of_seasons(vec![
            (id.clone(), 9),
            (id.clone(), 1),
            ("no-such".into(), 1),
        ])
        .await
        .unwrap();
    assert_eq!(many.len(), 3);
    assert!(many[0].is_none() && many[2].is_none());
    assert_eq!(many[1].as_ref().unwrap().version, 1);
}

#[tokio::test]
async fn a_lower_season_that_appears_later_takes_the_search_and_the_higher_ones_is_dropped() {
    let env = Env::new(&[("Show", &[2])]).await;
    assert_eq!(env.searches().await, [("Show".to_owned(), 2)]);
    env.rescan(&[("Show", &[1, 2])]).await;
    assert_eq!(
        env.searches().await,
        [("Show".to_owned(), 1), ("Show".to_owned(), 2)]
    );
    // Whichever is claimed first, season 2 is not the first any more and has no search left.
    let job = env.store.next_search(1_000).await.unwrap().unwrap();
    assert_eq!((job.dir_name.as_str(), job.season), ("Show", 1));
    assert_eq!(env.store.first_season(&job.work_id).await.unwrap(), Some(1));
    env.store
        .search_later(&job.work_id, 1, job.version, None, false, Note::NoMatch)
        .await
        .unwrap();
    assert_eq!(env.store.next_search(1_000).await.unwrap(), None);
    assert!(env.searches().await.is_empty());
}

#[tokio::test]
async fn a_season_folder_that_goes_and_comes_back_keeps_its_link_and_gets_no_new_search() {
    let env = Env::new(&[("Show", &[1, 2])]).await;
    let id = env.id("Show").await;
    env.store.put_entry(entry(5, "FINISHED", 1)).await.unwrap();
    env.store.set_links(&id, 1, 1, vec![5]).await.unwrap();
    // The folder of season 1 is moved away: the season is not recorded, the link stays.
    env.rescan(&[("Show", &[2])]).await;
    assert!(matches!(
        env.store.link(&id, 1).await,
        Err(SeasonError::NotFound)
    ));
    assert_eq!(env.store.links_of(&id).await.unwrap()[&1].entries.len(), 1);
    // It comes back: the link is there, nothing was asked for again.
    env.rescan(&[("Show", &[1, 2])]).await;
    let link = env.store.link(&id, 1).await.unwrap();
    assert_eq!(
        (link.version, link.origin, link.job),
        (2, Origin::User, None)
    );
    assert_eq!(link.entries[0].id, 5);
    assert!(env.searches().await.is_empty());
}

#[tokio::test]
async fn the_migration_creates_no_search_for_what_exists_already() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trss.db");
    {
        // A database as the build before season info left it, with a work and its seasons.
        let conn = trss_core::db::database_at(&path, 12);
        conn.execute_batch(
            "INSERT INTO watch_folders (id, path, created_at, baselined) VALUES ('f', '/w', 1, 1);
             INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f', 'Old Work');
             INSERT INTO seasons (work_id, number) VALUES ('w1', 1), ('w1', 2);",
        )
        .unwrap();
    }
    let db = Db::open(&path).await.unwrap();
    let store = SeasonStore::new(db.clone());
    assert_eq!(store.next_search(i64::MAX).await.unwrap(), None);
    assert_eq!(store.links_of("w1").await.unwrap().len(), 0);
    // A season recorded after the migration does.
    db.run::<_, DbError, _>(|c| {
        c.execute(
            "INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w2', 'f', 'New Work')",
            [],
        )?;
        c.execute("INSERT INTO seasons (work_id, number) VALUES ('w2', 1)", [])?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        store
            .next_search(i64::MAX)
            .await
            .unwrap()
            .map(|j| j.work_id),
        Some("w2".to_owned())
    );
}

#[tokio::test]
async fn a_change_names_the_version_it_was_made_from() {
    let env = Env::new(&[("Show", &[1, 2])]).await;
    let id = env.id("Show").await;
    for n in 1..=3 {
        env.store.put_entry(entry(n, "FINISHED", 1)).await.unwrap();
    }
    // Season 2 was never touched: version 0, and the first change creates the row.
    let first = env.store.set_links(&id, 2, 0, vec![1, 2]).await.unwrap();
    assert_eq!((first.version, first.origin), (1, Origin::User));
    assert_eq!(
        first.entries.iter().map(|e| e.id).collect::<Vec<_>>(),
        [1, 2]
    );

    // Order is the user's: the same entries the other way round.
    let reordered = env.store.set_links(&id, 2, 1, vec![2, 1]).await.unwrap();
    assert_eq!(reordered.version, 2);
    assert_eq!(
        reordered.entries.iter().map(|e| e.id).collect::<Vec<_>>(),
        [2, 1]
    );

    // A change from an older version changes nothing and carries the current link.
    match env.store.set_links(&id, 2, 1, vec![3]).await {
        Err(SeasonError::Conflict(current)) => {
            assert_eq!(current.version, 2);
            assert_eq!(
                current.entries.iter().map(|e| e.id).collect::<Vec<_>>(),
                [2, 1]
            );
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert!(matches!(
        env.store.set_links(&id, 2, 0, vec![3]).await,
        Err(SeasonError::Conflict(_))
    ));
    // Two screens changing from the same version: the first stays.
    env.store.set_links(&id, 2, 2, vec![3]).await.unwrap();
    assert!(matches!(
        env.store.set_links(&id, 2, 2, vec![1]).await,
        Err(SeasonError::Conflict(_))
    ));
    // Unlinking is a change too, by the user.
    let none = env.store.set_links(&id, 2, 3, vec![]).await.unwrap();
    assert_eq!(
        (none.version, none.origin, none.entries.len()),
        (4, Origin::User, 0)
    );

    // The same entry twice, an entry that was never received, a season that is not there.
    assert!(matches!(
        env.store.set_links(&id, 2, 4, vec![1, 1]).await,
        Err(SeasonError::Invalid(_))
    ));
    assert!(matches!(
        env.store.set_links(&id, 2, 4, vec![77]).await,
        Err(SeasonError::MissingEntry(77))
    ));
    assert!(matches!(
        env.store.set_links(&id, 9, 0, vec![1]).await,
        Err(SeasonError::NotFound)
    ));
    // The same entry may serve two seasons.
    env.store.set_links(&id, 1, 1, vec![1]).await.unwrap();
    env.store.set_links(&id, 2, 4, vec![1]).await.unwrap();
}

#[tokio::test]
async fn a_late_automatic_result_is_dropped_after_a_user_change() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.store.put_entry(entry(1, "FINISHED", 1)).await.unwrap();
    env.store.put_entry(entry(2, "FINISHED", 1)).await.unwrap();

    // The job is taken at version 1; the user links another entry meanwhile.
    let job = env.store.next_search(10).await.unwrap().unwrap();
    assert_eq!((job.version, job.attempts), (1, 0));
    env.store.set_links(&id, 1, 1, vec![2]).await.unwrap();
    assert!(!env.store.auto_linked(&id, 1, job.version, 1).await.unwrap());
    assert!(!env
        .store
        .search_later(&id, 1, job.version, None, false, Note::NoMatch)
        .await
        .unwrap());
    let link = env.store.link(&id, 1).await.unwrap();
    assert_eq!((link.version, link.origin), (2, Origin::User));
    assert_eq!(link.entries[0].id, 2);
    assert_eq!((link.job, link.note), (None, None));
    // And the user's change cleared the search: nothing is left to claim.
    assert_eq!(env.store.next_search(10).await.unwrap(), None);
}

#[tokio::test]
async fn an_automatic_result_links_as_auto_and_a_failure_waits_or_gives_up() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    env.store.put_entry(entry(1, "FINISHED", 1)).await.unwrap();

    let job = env.store.next_search(10).await.unwrap().unwrap();
    // A failed try waits until `retry_at` and counts.
    assert!(env
        .store
        .search_later(&id, 1, job.version, Some(5_000), true, Note::Failed)
        .await
        .unwrap());
    assert_eq!(env.store.next_search(4_999).await.unwrap(), None);
    let again = env.store.next_search(5_000).await.unwrap().unwrap();
    assert_eq!((again.attempts, again.version), (1, 1));
    // An answer that is busy waits without counting.
    env.store
        .search_later(&id, 1, 1, Some(9_000), false, Note::Failed)
        .await
        .unwrap();
    assert_eq!(
        env.store
            .next_search(9_000)
            .await
            .unwrap()
            .unwrap()
            .attempts,
        1
    );
    // Giving up leaves the note and no job; the version is the same.
    env.store
        .search_later(&id, 1, 1, None, false, Note::Ambiguous)
        .await
        .unwrap();
    let left = env.store.link(&id, 1).await.unwrap();
    assert_eq!(
        (left.version, left.note, left.job),
        (1, Some(Note::Ambiguous), None)
    );

    // The user asks again: unlinked, a new search, a new version.
    let again = env.store.restart_auto(&id, 1, 1, 50).await.unwrap();
    assert_eq!(
        (again.version, again.origin, again.note),
        (2, Origin::Auto, None)
    );
    assert_eq!(again.job.map(|j| j.requested_at), Some(50));
    let job = env.store.next_search(60).await.unwrap().unwrap();
    assert_eq!(job.version, 2);
    assert!(env.store.auto_linked(&id, 1, 2, 1).await.unwrap());
    let linked = env.store.link(&id, 1).await.unwrap();
    assert_eq!(
        (linked.version, linked.origin, linked.job),
        (3, Origin::Auto, None)
    );
    assert_eq!(linked.entries[0].id, 1);
    // The job is done: a second result for it is nothing.
    assert!(!env.store.auto_linked(&id, 1, 2, 1).await.unwrap());
}

#[tokio::test]
async fn only_the_first_season_can_ask_for_an_automatic_search() {
    let env = Env::new(&[("Show", &[0, 1, 2])]).await;
    let id = env.id("Show").await;
    for season in [0, 2] {
        assert!(matches!(
            env.store.restart_auto(&id, season, 0, 1).await,
            Err(SeasonError::Invalid(_))
        ));
    }
    assert!(matches!(
        env.store.restart_auto(&id, 1, 0, 1).await,
        Err(SeasonError::Conflict(_))
    ));
    assert!(env.store.restart_auto(&id, 1, 1, 1).await.is_ok());
}

#[tokio::test]
async fn only_linked_entries_that_are_not_finished_are_due_a_day_after_they_were_received() {
    const DAY: i64 = 24 * 60 * 60 * 1000;
    let env = Env::new(&[("Show", &[1, 2, 3])]).await;
    let id = env.id("Show").await;
    env.store
        .put_entry(entry(1, "RELEASING", 1_000))
        .await
        .unwrap();
    env.store
        .put_entry(entry(2, "NOT_YET_RELEASED", 2_000))
        .await
        .unwrap();
    env.store.put_entry(entry(3, "FINISHED", 0)).await.unwrap();
    env.store.put_entry(entry(4, "RELEASING", 0)).await.unwrap(); // linked by nobody
    env.store.put_entry(entry(5, "HIATUS", 0)).await.unwrap();
    // Season 1 has its version-1 row from the search the trigger made.
    env.store.set_links(&id, 1, 1, vec![1, 5]).await.unwrap();
    env.store.set_links(&id, 2, 0, vec![2]).await.unwrap();
    env.store.set_links(&id, 3, 0, vec![3]).await.unwrap();

    assert_eq!(env.store.next_refresh(1_000 + DAY - 1).await.unwrap(), None);
    // Oldest first; a finished, an unlinked and a hiatus entry never.
    assert_eq!(env.store.next_refresh(1_000 + DAY).await.unwrap(), Some(1));
    assert_eq!(env.store.next_refresh(2_000 + DAY).await.unwrap(), Some(1));
    env.store
        .put_entry(entry(1, "RELEASING", 1_000 + DAY))
        .await
        .unwrap();
    assert_eq!(env.store.next_refresh(2_000 + DAY).await.unwrap(), Some(2));
    // Unlinked, an entry is not refreshed any more.
    env.store.set_links(&id, 1, 2, vec![]).await.unwrap();
    assert_eq!(env.store.next_refresh(2_000 + DAY).await.unwrap(), Some(2));
    // A refresh that failed waits until the time it names; one that found the entry gone waits a day.
    env.store.refresh_later(2, 2_000 + DAY + 500).await.unwrap();
    assert_eq!(
        env.store.next_refresh(2_000 + DAY + 499).await.unwrap(),
        None
    );
    assert_eq!(
        env.store.next_refresh(2_000 + DAY + 500).await.unwrap(),
        Some(2)
    );
    env.store.refresh_gone(2, 2_000 + DAY + 600).await.unwrap();
    assert_eq!(
        env.store.next_refresh(2_000 + 2 * DAY + 599).await.unwrap(),
        None
    );
    assert_eq!(
        env.store.next_refresh(2_000 + 2 * DAY + 600).await.unwrap(),
        Some(2)
    );
    env.store.set_links(&id, 2, 1, vec![]).await.unwrap();
    assert_eq!(env.store.next_refresh(i64::MAX / 2).await.unwrap(), None);
}

#[tokio::test]
async fn a_finished_entry_whose_schedule_was_cut_at_25_is_read_again_once() {
    const DAY: i64 = 24 * 60 * 60 * 1000;
    let env = Env::new(&[("Show", &[1, 2])]).await;
    let id = env.id("Show").await;
    let airings = |n: u32| {
        (1..=n)
            .map(|episode| Airing {
                episode,
                at: i64::from(episode),
            })
            .collect::<Vec<_>>()
    };
    let long = |airing: Vec<Airing>, fetched_at: i64| Entry {
        episodes: Some(50),
        airing,
        ..entry(1, "FINISHED", fetched_at)
    };
    // As an earlier build stored it: the first page of a 50-episode season.
    env.store.put_entry(long(airings(25), 0)).await.unwrap();
    // A finished 25-episode season with its whole schedule is not.
    env.store
        .put_entry(Entry {
            episodes: Some(25),
            airing: airings(25),
            ..entry(2, "FINISHED", 0)
        })
        .await
        .unwrap();
    env.store.set_links(&id, 1, 1, vec![1]).await.unwrap();
    env.store.set_links(&id, 2, 0, vec![2]).await.unwrap();

    assert_eq!(env.store.next_refresh(DAY).await.unwrap(), Some(1));
    // Read in full, it is finished and due no more.
    env.store.put_entry(long(airings(50), DAY)).await.unwrap();
    assert_eq!(env.store.next_refresh(10 * DAY).await.unwrap(), None);
}

#[tokio::test]
async fn an_entry_round_trips_and_an_unregistered_work_keeps_its_links() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    let stored = entry(7, "RELEASING", 42);
    env.store.put_entry(stored.clone()).await.unwrap();
    assert_eq!(env.store.entry(7).await.unwrap(), Some(stored));
    assert_eq!(env.store.entry(8).await.unwrap(), None);
    // Putting it again replaces it.
    let changed = Entry {
        status: Some("FINISHED".into()),
        episodes: None,
        ..entry(7, "RELEASING", 99)
    };
    env.store.put_entry(changed.clone()).await.unwrap();
    assert_eq!(env.store.entry(7).await.unwrap(), Some(changed));
    env.store.set_links(&id, 1, 1, vec![7]).await.unwrap();
    // Unregistering the watch folder hides the work and keeps its link; the
    // daily refresh leaves the entry alone meanwhile.
    let rows = env.rows().await;
    env.library.remove_folder(&env.folder, 3_000).await.unwrap();
    assert_eq!(env.rows().await, rows);
    assert!(matches!(
        env.store.link(&id, 1).await,
        Err(SeasonError::NotFound)
    ));
    assert_eq!(env.store.next_refresh(i64::MAX / 2).await.unwrap(), None);
    assert!(env.store.entry(7).await.unwrap().is_some());
}

#[tokio::test]
async fn an_entrys_korean_titles_are_stored_and_replaced_when_it_is_received_again() {
    let env = Env::new(&[("Show", &[1])]).await;
    let id = env.id("Show").await;
    // An entry stored without them (as before they were kept) has none.
    env.store.put_entry(entry(5, "RELEASING", 1)).await.unwrap();
    env.store.set_links(&id, 1, 1, vec![5]).await.unwrap();
    assert!(env.store.link(&id, 1).await.unwrap().entries[0]
        .korean_titles
        .is_empty());

    // Received again, it carries them, in order.
    let mut again = entry(5, "RELEASING", 2);
    again.korean_titles = vec!["봇치 더 록!".into(), "외톨이 THE ROCK!".into()];
    env.store.put_entry(again).await.unwrap();
    assert_eq!(
        env.store.link(&id, 1).await.unwrap().entries[0].korean_titles,
        ["봇치 더 록!", "외톨이 THE ROCK!"]
    );
    assert_eq!(
        env.store.entry(5).await.unwrap().unwrap().korean_titles,
        ["봇치 더 록!", "외톨이 THE ROCK!"]
    );
}

/// An entry of `status` that starts in `year` (`None` when AniList does not say).
fn aired(id: i64, status: &str, year: Option<i32>) -> Entry {
    Entry {
        start: FuzzyDate {
            year,
            month: None,
            day: None,
        },
        ..entry(id, status, 1)
    }
}

#[tokio::test]
async fn the_overview_takes_the_year_and_airing_of_the_latest_season_from_its_linked_entries() {
    let env = Env::new(&[
        ("Alpha", &[1]),
        ("Beta", &[1]),
        ("Delta", &[1, 2]),
        ("Epsilon", &[1]),
        ("Gamma", &[1]),
        ("Zeta", &[1, 2]),
        ("Eta", &[1]),
    ])
    .await;
    for entry in [
        aired(1, "FINISHED", Some(2019)),
        aired(2, "RELEASING", Some(2024)),
        aired(3, "RELEASING", Some(2010)),
        aired(4, "RELEASING", Some(2025)),
        aired(5, "NOT_YET_RELEASED", None),
        aired(6, "FINISHED", Some(2015)),
        aired(7, "RELEASING", Some(2026)),
    ] {
        env.store.put_entry(entry).await.unwrap();
    }
    let link = |name: &'static str, season: u32, ids: Vec<i64>| {
        let env = &env;
        async move {
            let work = env.id(name).await;
            let version = env.store.link(&work, season).await.unwrap().version;
            env.store
                .set_links(&work, season, version, ids)
                .await
                .unwrap();
        }
    };
    link("Alpha", 1, vec![1]).await;
    link("Beta", 1, vec![2]).await;
    // Season 1 is releasing and season 2 is the latest: the latest decides.
    link("Delta", 1, vec![3]).await;
    link("Delta", 2, vec![4]).await;
    link("Epsilon", 1, vec![5]).await;
    // Season 1 is releasing, but the latest season, 2, has no link.
    link("Zeta", 1, vec![3]).await;
    // The year is the first entry's; any releasing entry of the season is airing.
    link("Eta", 1, vec![6, 7]).await;

    let overview = env.library.overview().await.unwrap();
    let of = |name: &str| overview.iter().find(|w| w.dir_name == name).unwrap();
    let facts = |name: &str| (of(name).airing_year, of(name).airing);
    assert_eq!(facts("Alpha"), (Some(2019), false));
    assert_eq!(facts("Beta"), (Some(2024), true));
    assert_eq!(facts("Delta"), (Some(2025), true));
    assert_eq!(facts("Epsilon"), (None, false));
    assert_eq!(facts("Gamma"), (None, false));
    assert_eq!(facts("Zeta"), (None, false));
    assert_eq!(facts("Eta"), (Some(2015), true));

    // Every linked entry of every season lends its titles, the native one first.
    assert_eq!(
        of("Delta").linked_titles,
        ["エントリー 3", "Entry 3", "エントリー 4", "Entry 4"]
    );
    assert!(of("Gamma").linked_titles.is_empty());
}
