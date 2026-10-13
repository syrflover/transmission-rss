//! A movie's Anissia link, the movie-only work, and what follows from it: the
//! work title, the first season, the automatic search's row and the archive
//! merge. What the scan reads as a movie is `discovery`'s rule; the first test
//! here only shows that a scan of real folders reaches the store as one.

use std::{collections::BTreeSet, fs};

use rusqlite::TransactionBehavior;

use crate::{
    discovery::{self, EpisodeFile, FileKind, Scan, ScannedWork, WorkRead},
    store::{
        library::{Followed, LibraryStore},
        seasons::{
            anissia::{set_in as set_season_in, AnissiaLink},
            movie::{is_movie_only, link_in, set_in, MovieLink},
            SeasonError, SeasonStore,
        },
    },
};
use trss_core::{Db, DbError};

fn file(path: &str, season: u32, episode: &str, kind: FileKind) -> EpisodeFile {
    EpisodeFile {
        path: path.to_owned(),
        kind,
        season,
        episode: episode.to_owned(),
    }
}

/// A work read as a movie-only work is: season 0, episode 1, its video and a
/// subtitle directly in the folder.
fn movie_only(name: &str) -> ScannedWork {
    ScannedWork {
        dir_name: name.to_owned(),
        seasons: BTreeSet::from([0]),
        files: vec![
            file(&format!("{name}.mkv"), 0, "1", FileKind::Video),
            file(&format!("{name}.ass"), 0, "1", FileKind::Subtitle),
        ],
        unrecognized: Vec::new(),
    }
}

/// A work with seasons `seasons` and no file.
fn seasons_only(name: &str, seasons: &[u32]) -> ScannedWork {
    ScannedWork {
        dir_name: name.to_owned(),
        seasons: BTreeSet::from_iter(seasons.iter().copied()),
        ..ScannedWork::default()
    }
}

fn scan(works: Vec<ScannedWork>) -> Scan {
    Scan {
        works: works.into_iter().map(WorkRead::Read).collect(),
    }
}

/// A library with `/w` holding a movie-only work `Movie` and `Show` (seasons 0
/// and 1), and two Anissia anime.
async fn env() -> (Db, LibraryStore, SeasonStore, String, String, String) {
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
        .add_folder(
            "/w".into(),
            scan(vec![movie_only("Movie"), seasons_only("Show", &[0, 1])]),
            100,
            &[],
        )
        .await
        .unwrap();
    let works = library.works(&folder.id).await.unwrap();
    let id = |name: &str| {
        works
            .iter()
            .find(|w| w.dir_name == name)
            .unwrap()
            .id
            .clone()
    };
    (
        db.clone(),
        library,
        SeasonStore::new(db),
        folder.id.clone(),
        id("Movie"),
        id("Show"),
    )
}

async fn set(db: &Db, work: &str, episode: u32, anime_no: Option<i64>) -> MovieLink {
    let work = work.to_owned();
    db.run::<_, DbError, _>(move |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let link = set_in(&tx, &work, episode, anime_no)?;
        tx.commit()?;
        Ok(link)
    })
    .await
    .unwrap()
}

async fn set_season(db: &Db, work: &str, season: u32, anime_no: Option<i64>) -> AnissiaLink {
    let work = work.to_owned();
    db.run::<_, DbError, _>(move |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let link = set_season_in(&tx, &work, season, anime_no)?;
        tx.commit()?;
        Ok(link)
    })
    .await
    .unwrap()
}

async fn movie_only_of(db: &Db, work: &str) -> bool {
    let work = work.to_owned();
    db.run::<_, DbError, _>(move |c| Ok(is_movie_only(c, &work)?))
        .await
        .unwrap()
}

#[tokio::test]
async fn folders_read_from_a_disk_are_movie_only_by_the_scans_rule_and_not_otherwise() {
    let dir = tempfile::tempdir().unwrap();
    let touch = |relative: &str| {
        let path = dir.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
    };
    for relative in [
        // One video and its subtitles, a download beside it that is not yet one.
        "One/One.mkv",
        "One/One.ko.smi",
        "One/Another.mkv.part",
        // Two videos.
        "Two/A.mkv",
        "Two/B.mkv",
        // A video beside a season folder.
        "Both/Both.mkv",
        "Both/Season 01/Both S01E01.mkv",
    ] {
        touch(relative);
    }
    let db = Db::open_blocking(":memory:").unwrap();
    let library = LibraryStore::new(db.clone());
    let (folder, _) = library
        .add_folder("/w".into(), discovery::scan(dir.path()).unwrap(), 100, &[])
        .await
        .unwrap();
    let works = library.works(&folder.id).await.unwrap();
    let id = |name: &str| {
        works
            .iter()
            .find(|w| w.dir_name == name)
            .unwrap()
            .id
            .clone()
    };

    assert!(movie_only_of(&db, &id("One")).await);
    assert!(!movie_only_of(&db, &id("Two")).await);
    assert!(!movie_only_of(&db, &id("Both")).await);

    let one = library.work_detail(&id("One")).await.unwrap().unwrap();
    assert!(one.movie_only);
    assert_eq!(one.seasons.len(), 1);
    assert_eq!(one.seasons[0].number, 0);
    let episodes = &one.seasons[0].episodes;
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].episode, "1");
    assert_eq!(
        episodes[0]
            .video
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["One.mkv"]
    );
    assert_eq!(
        episodes[0]
            .subtitle
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["One.ko.smi"]
    );
    let two = library.work_detail(&id("Two")).await.unwrap().unwrap();
    assert!(!two.movie_only && two.seasons.is_empty());
    assert_eq!(two.unrecognized.len(), 2);
    let both = library.work_detail(&id("Both")).await.unwrap().unwrap();
    assert!(!both.movie_only);
    assert_eq!(
        both.seasons.iter().map(|s| s.number).collect::<Vec<_>>(),
        [1]
    );
}

#[tokio::test]
async fn a_movie_that_stops_being_the_only_video_stops_being_movie_only() {
    let (db, library, _store, folder, movie, _show) = env().await;
    assert!(movie_only_of(&db, &movie).await);
    // A second video came: the next scan reads the folder without a movie.
    let mut two = ScannedWork {
        dir_name: "Movie".to_owned(),
        ..ScannedWork::default()
    };
    two.unrecognized.push(discovery::Unrecognized {
        path: "Movie.mkv".to_owned(),
        reason: discovery::Reason::OutsideSeason,
        check: None,
    });
    library
        .record_scan(&folder, Ok(scan(vec![two])), 200)
        .await
        .unwrap()
        .unwrap();
    assert!(!movie_only_of(&db, &movie).await);
    assert!(
        !library
            .work_detail(&movie)
            .await
            .unwrap()
            .unwrap()
            .movie_only
    );
}

#[tokio::test]
async fn a_movie_never_linked_has_version_zero_and_each_change_moves_the_version_on() {
    let (db, _library, store, _folder, movie, _show) = env().await;

    assert_eq!(
        store.movie_link(&movie, 1).await.unwrap(),
        MovieLink {
            work_id: movie.clone(),
            episode: 1,
            version: 0,
            anime_no: None
        }
    );
    assert_eq!(set(&db, &movie, 1, Some(7)).await.version, 1);
    // The same link again changes nothing, not even the version.
    assert_eq!(set(&db, &movie, 1, Some(7)).await.version, 1);
    assert_eq!(set(&db, &movie, 1, Some(8)).await.version, 2);
    // Cutting the link keeps the row, so the version goes on.
    let cut = set(&db, &movie, 1, None).await;
    assert_eq!((cut.version, cut.anime_no), (3, None));
    assert_eq!(set(&db, &movie, 1, None).await.version, 3);
    assert_eq!(set(&db, &movie, 1, Some(7)).await.version, 4);
    assert_eq!(
        store
            .movie_links_of(&movie)
            .await
            .unwrap()
            .values()
            .map(|l| (l.episode, l.version, l.anime_no))
            .collect::<Vec<_>>(),
        [(1, 4, Some(7))]
    );
}

#[tokio::test]
async fn only_an_episode_of_season_0_has_a_movie_link_to_read() {
    let (_db, _library, store, _folder, movie, show) = env().await;
    // The movie-only work has episode 1 only; another episode, a work with no
    // such episode and no work at all are not found.
    assert!(store.movie_link(&movie, 1).await.is_ok());
    for (work, episode) in [(&movie, 2), (&show, 1), (&"nobody".to_owned(), 1)] {
        assert!(
            matches!(
                store.movie_link(work, episode).await,
                Err(SeasonError::NotFound)
            ),
            "{work} {episode}"
        );
    }
}

#[tokio::test]
async fn an_episode_0_of_season_0_is_no_movie() {
    let (_db, library, store, folder, movie, show) = env().await;
    library
        .record_scan(
            &folder,
            Ok(scan(vec![
                movie_only("Movie"),
                ScannedWork {
                    files: vec![file("Season 00/Show S00E00.mkv", 0, "00", FileKind::Video)],
                    ..seasons_only("Show", &[0, 1])
                },
            ])),
            200,
        )
        .await
        .unwrap()
        .unwrap();
    // A link needs an episode from 1, so episode 0 is not found rather than
    // failing to be stored.
    for work in [&show, &movie] {
        assert!(
            matches!(store.movie_link(work, 0).await, Err(SeasonError::NotFound)),
            "{work}"
        );
    }
}

#[tokio::test]
async fn a_movie_link_is_a_movies_and_not_a_seasons_and_an_anime_linked_to_either_counts_as_linked()
{
    let (db, _library, store, _folder, movie, show) = env().await;
    assert!(!store.anime_is_linked(7).await.unwrap());
    set(&db, &movie, 1, Some(7)).await;
    assert!(store.anime_is_linked(7).await.unwrap());
    assert!(!store.anime_is_linked(8).await.unwrap());
    // The season links are another table, and untouched.
    assert!(store.anissia_links_of(&movie).await.unwrap().is_empty());
    set_season(&db, &show, 1, Some(8)).await;
    assert!(store.anime_is_linked(8).await.unwrap());
    set(&db, &movie, 1, None).await;
    assert!(!store.anime_is_linked(7).await.unwrap());
}

/// The title the library list and the work detail give `work`, which are one.
async fn title(library: &LibraryStore, work: &str) -> String {
    let listed = library
        .overview()
        .await
        .unwrap()
        .into_iter()
        .find(|w| w.id == work)
        .unwrap()
        .title;
    let detail = library.work_detail(work).await.unwrap().unwrap().title;
    assert_eq!(listed, detail);
    listed
}

#[tokio::test]
async fn the_list_marks_a_movie_only_work_and_holds_its_movie_as_season_0() {
    let (_db, library, _store, _folder, movie, show) = env().await;
    let list = library.overview().await.unwrap();
    let row = |id: &str| list.iter().find(|w| w.id == id).unwrap();
    assert!(row(&movie).movie_only);
    assert_eq!(row(&movie).latest_season, Some(0));
    assert_eq!(row(&movie).video.len(), 1);
    assert_eq!(row(&movie).subtitle.len(), 1);
    assert!(!row(&show).movie_only);
}

#[tokio::test]
async fn the_title_of_a_movie_only_work_is_its_movies_anissia_title_and_a_work_with_seasons_is_not_named_by_a_movie(
) {
    let (db, library, _store, folder, movie, show) = env().await;
    assert_eq!(title(&library, &movie).await, "Movie");
    set(&db, &movie, 1, Some(7)).await;
    assert_eq!(title(&library, &movie).await, "하나");
    set(&db, &movie, 1, Some(8)).await;
    assert_eq!(title(&library, &movie).await, "둘");
    // Cut, the folder name again.
    set(&db, &movie, 1, None).await;
    assert_eq!(title(&library, &movie).await, "Movie");
    set(&db, &movie, 1, Some(7)).await;

    // A movie link of a work with seasons (a movie of `Season 00`) never names
    // it: its season 1 does, and with none linked, its folder.
    set(&db, &show, 1, Some(8)).await;
    set(&db, &show, 1, None).await;
    assert_eq!(title(&library, &show).await, "Show");
    library
        .record_scan(
            &folder,
            Ok(scan(vec![ScannedWork {
                files: vec![file("Season 00/Show S00E01.mkv", 0, "01", FileKind::Video)],
                ..seasons_only("Show", &[0, 1])
            }])),
            200,
        )
        .await
        .unwrap()
        .unwrap();
    set(&db, &show, 1, Some(7)).await;
    assert_eq!(title(&library, &show).await, "Show");
    set_season(&db, &show, 1, Some(8)).await;
    assert_eq!(title(&library, &show).await, "둘");

    // The movie link of a work that is no movie-only work any more names
    // nothing: here a `Season 01` folder came.
    library
        .record_scan(&folder, Ok(scan(vec![seasons_only("Movie", &[0, 1])])), 300)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(title(&library, &movie).await, "Movie");
}

#[tokio::test]
async fn a_movie_links_survive_a_scan_that_misses_its_video_for_a_moment() {
    let (db, library, store, folder, movie, _show) = env().await;
    set(&db, &movie, 1, Some(7)).await;
    library
        .record_scan(&folder, Ok(scan(vec![seasons_only("Movie", &[])])), 200)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        store.movie_link(&movie, 1).await,
        Err(SeasonError::NotFound)
    ));
    let kept = db
        .run::<_, DbError, _>({
            let movie = movie.clone();
            move |c| Ok(link_in(c, &movie, 1)?)
        })
        .await
        .unwrap();
    assert_eq!((kept.version, kept.anime_no), (1, Some(7)));
    library
        .record_scan(&folder, Ok(scan(vec![movie_only("Movie")])), 300)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(store.movie_link(&movie, 1).await.unwrap().anime_no, Some(7));
}

#[tokio::test]
async fn the_first_season_of_a_movie_only_work_is_season_0_and_a_season_from_1_on_comes_first() {
    let (_db, library, store, folder, movie, show) = env().await;
    assert_eq!(store.first_season(&movie).await.unwrap(), Some(0));
    // Season 0 of a work with seasons is not its first.
    assert_eq!(store.first_season(&show).await.unwrap(), Some(1));
    library
        .record_scan(
            &folder,
            Ok(scan(vec![ScannedWork {
                files: vec![file("Season 00/Show S00E01.mkv", 0, "01", FileKind::Video)],
                ..seasons_only("Show", &[0])
            }])),
            200,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(store.first_season(&show).await.unwrap(), None);
}

#[tokio::test]
async fn a_movie_only_work_asks_for_its_automatic_search_once_and_a_season_0_folder_does_not() {
    let (db, library, store, folder, movie, show) = env().await;
    let link = store.link(&movie, 0).await.unwrap();
    assert!(link.job.is_some(), "{link:?}");
    assert_eq!(link.version, 1);
    // `Show`'s season 1 waits for its search too, asked for in the same
    // millisecond, so the work ids would order the two: it is put aside.
    let id = show.clone();
    db.run::<_, DbError, _>(move |c| {
        c.execute("UPDATE season_info SET job = NULL WHERE work_id = ?1", [id])?;
        Ok(())
    })
    .await
    .unwrap();
    // The search is the first season's: the movie's.
    let claimed = store
        .next_search(10_000_000_000_000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (claimed.work_id.as_str(), claimed.season),
        (movie.as_str(), 0)
    );
    // The season-0 folder of a work with seasons gets none.
    assert!(store.link(&show, 0).await.unwrap().job.is_none());

    // Recording the same movie again asks for nothing more.
    library
        .record_scan(&folder, Ok(scan(vec![movie_only("Movie")])), 200)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(store.link(&movie, 0).await.unwrap().version, 1);
}

#[tokio::test]
async fn an_archive_merge_carries_the_movie_links_the_kept_work_has_no_row_for() {
    let (db, library, store, collect, moved, _show) = env().await;
    let folders = library.folders().await.unwrap();
    let (archive, _) = library
        .add_folder(
            "/archive".into(),
            scan(vec![movie_only("Movie")]),
            150,
            &folders,
        )
        .await
        .unwrap();
    let kept = library.works(&archive.id).await.unwrap()[0].id.clone();
    assert_ne!(kept, moved);

    set(&db, &moved, 1, Some(7)).await;
    assert_eq!(
        library
            .follow_move(&collect, &archive.id, "Movie")
            .await
            .unwrap(),
        Followed::Merged
    );
    let links = store.movie_links_of(&kept).await.unwrap();
    assert_eq!(
        links
            .values()
            .map(|l| (l.episode, l.version, l.anime_no))
            .collect::<Vec<_>>(),
        [(1, 2, Some(7))]
    );
    assert!(store.movie_links_of(&moved).await.unwrap().is_empty());
}

#[tokio::test]
async fn an_archive_merge_leaves_a_kept_movie_link_even_a_cut_one_as_it_is() {
    let (db, library, store, collect, moved, _show) = env().await;
    let folders = library.folders().await.unwrap();
    let (archive, _) = library
        .add_folder(
            "/archive".into(),
            scan(vec![movie_only("Movie")]),
            150,
            &folders,
        )
        .await
        .unwrap();
    let kept = library.works(&archive.id).await.unwrap()[0].id.clone();
    set(&db, &moved, 1, Some(7)).await;
    set(&db, &kept, 1, Some(8)).await;
    set(&db, &kept, 1, None).await;

    library
        .follow_move(&collect, &archive.id, "Movie")
        .await
        .unwrap();
    let links = store.movie_links_of(&kept).await.unwrap();
    assert_eq!(
        links
            .values()
            .map(|l| (l.episode, l.version, l.anime_no))
            .collect::<Vec<_>>(),
        [(1, 2, None)]
    );
}

#[tokio::test]
async fn an_archive_merge_into_a_work_with_seasons_does_not_make_it_movie_only() {
    let (db, library, _store, collect, moved, _show) = env().await;
    let folders = library.folders().await.unwrap();
    let (archive, _) = library
        .add_folder(
            "/archive".into(),
            scan(vec![seasons_only("Movie", &[1])]),
            150,
            &folders,
        )
        .await
        .unwrap();
    let kept = library.works(&archive.id).await.unwrap()[0].id.clone();
    set(&db, &moved, 1, Some(7)).await;

    library
        .follow_move(&collect, &archive.id, "Movie")
        .await
        .unwrap();
    // The merge copies the moved movie's root video in until the next scan of
    // the archive; with a season from 1 on, the kept work is still not named
    // by a movie.
    assert!(!movie_only_of(&db, &kept).await);
    assert_eq!(title(&library, &kept).await, "Movie");
}

fn entry(id: i64) -> trss_anilist::Entry {
    trss_anilist::Entry {
        id,
        romaji: Some(format!("Entry {id}")),
        english: None,
        native: None,
        format: Some("MOVIE".into()),
        status: Some("FINISHED".into()),
        episodes: Some(1),
        start: Default::default(),
        end: Default::default(),
        studios: Vec::new(),
        genres: Vec::new(),
        description: None,
        airing: Vec::new(),
        korean_titles: Vec::new(),
        sequels: Vec::new(),
        fetched_at: 0,
    }
}

#[tokio::test]
async fn only_a_movie_only_works_season_0_can_ask_again_and_its_entry_is_what_the_cover_follows() {
    let (db, _library, store, _folder, movie, show) = env().await;
    store.put_entry(entry(1)).await.unwrap();
    store.put_entry(entry(2)).await.unwrap();

    // Season 0 of a work with seasons has no automatic search to ask for, and
    // is no cover's source.
    assert!(matches!(
        store.restart_auto(&show, 0, 0, 1).await,
        Err(SeasonError::Invalid(_))
    ));
    store.set_links(&show, 0, 0, vec![2]).await.unwrap();
    let target = |work: String| {
        db.run::<_, DbError, _>(move |c| Ok(crate::store::seasons::cover_target(c, &work)?))
    };
    assert_eq!(target(show.clone()).await.unwrap(), None);

    // The movie-only work's season 0 is its first season.
    assert_eq!(target(movie.clone()).await.unwrap(), None);
    store.set_links(&movie, 0, 1, vec![1]).await.unwrap();
    assert_eq!(target(movie.clone()).await.unwrap(), Some(1));
    let again = store.restart_auto(&movie, 0, 2, 5).await.unwrap();
    assert!(again.entries.is_empty() && again.job.is_some());
    assert_eq!(target(movie).await.unwrap(), None);
}
