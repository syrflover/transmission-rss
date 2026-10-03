//! The creator of a subtitle file (`creators.rs`).

use std::collections::BTreeSet;

use crate::{
    discovery::{EpisodeFile, ScannedWork, WorkRead},
    store::library::*,
};

fn file(kind: FileKind, season: u32, episode: &str, name: &str) -> EpisodeFile {
    EpisodeFile {
        path: format!("Season {season:02}/{name}"),
        kind,
        season,
        episode: episode.to_owned(),
    }
}

fn video(season: u32, episode: &str) -> EpisodeFile {
    file(
        FileKind::Video,
        season,
        episode,
        &format!("S{season:02}E{episode}.mkv"),
    )
}

fn subtitle(season: u32, episode: &str) -> EpisodeFile {
    file(
        FileKind::Subtitle,
        season,
        episode,
        &format!("S{season:02}E{episode}.ass"),
    )
}

fn work(name: &str, files: Vec<EpisodeFile>) -> WorkRead {
    WorkRead::Read(ScannedWork {
        dir_name: name.to_owned(),
        seasons: files.iter().map(|f| f.season).collect::<BTreeSet<_>>(),
        files,
        unrecognized: Vec::new(),
    })
}

/// The version a file starts at: the time of the scan that found it (`library()`
/// adds its folder at 100).
const SEED: i64 = 100;

/// The time the user names a creator in these tests.
const SET_AT: i64 = 5_000;

fn scan(works: Vec<WorkRead>) -> Scan {
    Scan { works }
}

struct Library {
    store: LibraryStore,
    folder: WatchFolder,
    work: String,
}

/// A library with the work `Show`: season 1 has a video and a subtitle for
/// episodes 1 to 3, season 2 a subtitle for episode 1; and two creators of
/// the anime 3441 (sources `하느` and `카이란`).
async fn library() -> Library {
    let db = Db::open_blocking(":memory:").unwrap();
    db.run::<_, DbError, _>(|c| {
        c.execute_batch(
            "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at) VALUES
                 ('hanu', 3441, '하느', 1), ('kairan', 3441, '카이란', 1)",
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let store = LibraryStore::new(db);
    let (folder, _) = store
        .add_folder("/w".into(), scan(vec![work("Show", shown())]), 100, &[])
        .await
        .unwrap();
    let work = store.works(&folder.id).await.unwrap().remove(0).id;
    Library {
        store,
        folder,
        work,
    }
}

fn shown() -> Vec<EpisodeFile> {
    let mut files = Vec::new();
    for episode in ["01", "02", "03"] {
        files.push(video(1, episode));
        files.push(subtitle(1, episode));
    }
    files.push(subtitle(2, "01"));
    files
}

impl Library {
    async fn creators(&self) -> Vec<(String, Option<String>, i64)> {
        let detail = self.store.work_detail(&self.work).await.unwrap().unwrap();
        let mut out = Vec::new();
        for season in &detail.seasons {
            for episode in &season.episodes {
                assert!(episode.video.iter().all(|v| v.creator.is_none()));
                for file in &episode.subtitle {
                    out.push((
                        file.path.clone(),
                        file.creator.as_ref().map(|c| c.name.clone()),
                        file.creator_version,
                    ));
                }
            }
        }
        out
    }

    async fn rescan(&self, files: Vec<EpisodeFile>) {
        self.rescan_at(files, 200).await
    }

    async fn rescan_at(&self, files: Vec<EpisodeFile>, now: i64) {
        self.store
            .record_scan(&self.folder.id, Ok(scan(vec![work("Show", files)])), now)
            .await
            .unwrap()
            .unwrap();
    }
}

fn row(path: &str, creator: Option<&str>, version: i64) -> (String, Option<String>, i64) {
    (path.to_owned(), creator.map(str::to_owned), version)
}

#[tokio::test]
async fn files_are_by_an_unknown_creator_until_the_user_names_one() {
    let lib = library().await;
    assert!(lib
        .creators()
        .await
        .iter()
        .all(|(_, creator, version)| creator.is_none() && *version == SEED));
    assert!(lib
        .store
        .attributed_subtitles(&lib.work, 1)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn naming_a_creator_covers_the_seasons_unknown_subtitles_only_and_leaves_the_rest() {
    let lib = library().await;
    // One file of season 1 already has the other creator.
    lib.store
        .set_file_creator(
            &lib.work,
            1,
            "Season 01/S01E02.ass",
            SEED,
            Some("kairan"),
            SET_AT,
        )
        .await
        .unwrap();
    let generation = lib.store.generation().await.unwrap();

    let named = lib
        .store
        .name_unknown_creators(&lib.work, 1, "hanu", SET_AT)
        .await
        .unwrap();
    assert_eq!(named, 2, "the two that were unknown");
    assert_eq!(
        lib.creators().await,
        [
            row("Season 01/S01E01.ass", Some("하느"), SEED + 1),
            row("Season 01/S01E02.ass", Some("카이란"), SEED + 1),
            row("Season 01/S01E03.ass", Some("하느"), SEED + 1),
            // Another season is not touched.
            row("Season 02/S02E01.ass", None, SEED),
        ]
    );
    // A second naming finds none unknown and replaces nothing.
    assert_eq!(
        lib.store
            .name_unknown_creators(&lib.work, 1, "kairan", SET_AT)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        lib.creators().await[0],
        row("Season 01/S01E01.ass", Some("하느"), SEED + 1)
    );

    // The files are as they were: the video lookups are not affected.
    assert_eq!(lib.store.generation().await.unwrap(), generation);
    let attributed = lib.store.attributed_subtitles(&lib.work, 1).await.unwrap();
    assert_eq!(
        attributed
            .iter()
            .map(|a| (a.episode.as_str(), a.source_id.as_str()))
            .collect::<Vec<_>>(),
        [("01", "hanu"), ("02", "kairan"), ("03", "hanu")]
    );
}

#[tokio::test]
async fn one_files_creator_changes_by_its_version_and_a_stale_version_changes_nothing() {
    let lib = library().await;
    let path = "Season 01/S01E01.ass";

    let first = lib
        .store
        .set_file_creator(&lib.work, 1, path, SEED, Some("hanu"), SET_AT)
        .await
        .unwrap();
    assert_eq!(first.version, SEED + 1);
    assert_eq!(first.creator.as_ref().unwrap().name, "하느");
    assert_eq!(first.creator.as_ref().unwrap().anime_no, 3441);

    // The same creator again is no change.
    let same = lib
        .store
        .set_file_creator(&lib.work, 1, path, SEED + 1, Some("hanu"), SET_AT)
        .await
        .unwrap();
    assert_eq!(same.version, SEED + 1);

    // A second screen that still holds the first version is refused, and the file is
    // as the first screen left it.
    let stale = lib
        .store
        .set_file_creator(&lib.work, 1, path, SEED, Some("kairan"), SET_AT)
        .await;
    assert!(matches!(stale, Err(CreatorError::Conflict)), "{stale:?}");
    let now = lib
        .store
        .file_creator(&lib.work, 1, path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(now.creator.unwrap().name, "하느");
    assert_eq!(now.version, SEED + 1);

    // From the current version it changes, and can go back to unknown.
    let changed = lib
        .store
        .set_file_creator(&lib.work, 1, path, SEED + 1, Some("kairan"), SET_AT)
        .await
        .unwrap();
    assert_eq!(
        (changed.creator.unwrap().name, changed.version),
        ("카이란".to_owned(), SEED + 2)
    );
    let unknown = lib
        .store
        .set_file_creator(&lib.work, 1, path, SEED + 2, None, SET_AT)
        .await
        .unwrap();
    assert_eq!((unknown.creator, unknown.version), (None, SEED + 3));

    // A file the work does not have, a video and another season's file are none.
    for (season, path) in [
        (1, "Season 01/nope.ass"),
        (1, "Season 01/S01E01.mkv"),
        (2, path),
    ] {
        let missing = lib
            .store
            .set_file_creator(&lib.work, season, path, SEED, Some("hanu"), SET_AT)
            .await;
        assert!(
            matches!(missing, Err(CreatorError::NoFile)),
            "{path}: {missing:?}"
        );
    }
    assert!(lib
        .store
        .file_creator(&lib.work, 1, "Season 01/S01E01.mkv")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_rescan_keeps_the_creator_of_a_file_it_finds_and_carries_it_when_the_episode_is_read_again(
) {
    let lib = library().await;
    lib.store
        .name_unknown_creators(&lib.work, 1, "hanu", SET_AT)
        .await
        .unwrap();
    let before = lib.creators().await;

    // The same files: nothing changes, not even the versions.
    lib.rescan(shown()).await;
    assert_eq!(lib.creators().await, before);

    // The file name is read as another episode now: the file is recorded again,
    // with its creator and version.
    let mut reread = shown();
    reread
        .iter_mut()
        .find(|f| f.path == "Season 01/S01E03.ass")
        .unwrap()
        .episode = "13".to_owned();
    lib.rescan(reread).await;
    let after = lib.creators().await;
    assert!(
        after.contains(&row("Season 01/S01E03.ass", Some("하느"), SEED + 1)),
        "{after:?}"
    );

    // A file read as a video now has no creator.
    let mut as_video = shown();
    as_video
        .iter_mut()
        .find(|f| f.path == "Season 01/S01E03.ass")
        .unwrap()
        .kind = FileKind::Video;
    lib.rescan(as_video).await;
    assert!(!lib
        .creators()
        .await
        .iter()
        .any(|(path, _, _)| path == "Season 01/S01E03.ass"));
    assert!(lib
        .store
        .attributed_subtitles(&lib.work, 1)
        .await
        .unwrap()
        .iter()
        .all(|a| a.path != "Season 01/S01E03.ass"));
}

#[tokio::test]
async fn a_renamed_file_or_one_that_left_and_came_back_is_by_an_unknown_creator_again() {
    let lib = library().await;
    lib.store
        .name_unknown_creators(&lib.work, 1, "hanu", SET_AT)
        .await
        .unwrap();

    // Renamed: the old name is gone, the new one is a new file.
    let mut renamed = shown();
    renamed
        .iter_mut()
        .find(|f| f.path == "Season 01/S01E01.ass")
        .unwrap()
        .path = "Season 01/Show - 01.ass".to_owned();
    lib.rescan(renamed).await;
    let after = lib.creators().await;
    assert!(
        after.contains(&row("Season 01/Show - 01.ass", None, 200)),
        "{after:?}"
    );
    assert!(after.contains(&row("Season 01/S01E02.ass", Some("하느"), SEED + 1)));

    // Gone for a scan, then back.
    let without: Vec<EpisodeFile> = shown()
        .into_iter()
        .filter(|f| f.path != "Season 01/S01E02.ass")
        .collect();
    lib.rescan(without).await;
    lib.rescan_at(shown(), 300).await;
    assert!(lib
        .creators()
        .await
        .contains(&row("Season 01/S01E02.ass", None, 300)));
}

#[tokio::test]
async fn a_moved_work_folder_keeps_its_files_creators_and_a_merge_carries_them_over() {
    let lib = library().await;
    lib.store
        .name_unknown_creators(&lib.work, 1, "hanu", SET_AT)
        .await
        .unwrap();
    let (to, _) = lib
        .store
        .add_folder(
            "/to".into(),
            scan(Vec::new()),
            300,
            std::slice::from_ref(&lib.folder),
        )
        .await
        .unwrap();

    // The work folder moves to another watch folder: same work, same paths.
    assert_eq!(
        lib.store
            .follow_move(&lib.folder.id, &to.id, "Show")
            .await
            .unwrap(),
        Followed::Moved
    );
    assert!(lib
        .creators()
        .await
        .contains(&row("Season 01/S01E01.ass", Some("하느"), SEED + 1)));

    // A work of that name appears in the first folder and is moved onto it:
    // the destination keeps its own files, and takes the moved work's others
    // with their creators.
    lib.store
        .record_scan(
            &lib.folder.id,
            Ok(scan(vec![work(
                "Show",
                vec![subtitle(1, "01"), subtitle(1, "04")],
            )])),
            400,
        )
        .await
        .unwrap();
    let other = lib.store.works(&lib.folder.id).await.unwrap().remove(0).id;
    assert_ne!(other, lib.work);
    lib.store
        .name_unknown_creators(&other, 1, "kairan", SET_AT)
        .await
        .unwrap();
    assert_eq!(
        lib.store
            .follow_move(&lib.folder.id, &to.id, "Show")
            .await
            .unwrap(),
        Followed::Merged
    );
    let kept = lib.creators().await;
    assert!(
        kept.contains(&row("Season 01/S01E01.ass", Some("하느"), SEED + 1)),
        "{kept:?}"
    );
    assert!(
        kept.contains(&row("Season 01/S01E04.ass", Some("카이란"), 400 + 1)),
        "{kept:?}"
    );
}

#[tokio::test]
async fn a_rescan_that_clears_the_creator_raises_the_version() {
    let lib = library().await;
    let path = "Season 01/S01E03.ass";
    lib.store
        .name_unknown_creators(&lib.work, 1, "hanu", SET_AT)
        .await
        .unwrap();
    // A second screen read the file with its creator, at SEED + 1.

    // The file is read as a video for a scan: the creator is gone with it.
    let mut as_video = shown();
    as_video.iter_mut().find(|f| f.path == path).unwrap().kind = FileKind::Video;
    lib.rescan(as_video).await;
    // And as a subtitle again: no creator, and a version past the one the
    // screen holds.
    lib.rescan(shown()).await;
    assert!(lib.creators().await.contains(&row(path, None, SEED + 2)));

    let stale = lib
        .store
        .set_file_creator(&lib.work, 1, path, SEED + 1, Some("kairan"), SET_AT)
        .await;
    assert!(matches!(stale, Err(CreatorError::Conflict)), "{stale:?}");
    assert!(lib
        .store
        .set_file_creator(&lib.work, 1, path, SEED + 2, Some("kairan"), SET_AT)
        .await
        .is_ok());
}

#[tokio::test]
async fn a_file_removed_and_added_again_is_a_new_file_no_older_version_can_change() {
    let lib = library().await;
    let path = "Season 01/S01E02.ass";
    // The file's versions a screen can hold: SEED (unknown), then SEED + 1, 2.
    lib.store
        .set_file_creator(&lib.work, 1, path, SEED, Some("hanu"), SET_AT)
        .await
        .unwrap();
    lib.store
        .set_file_creator(&lib.work, 1, path, SEED + 1, None, SET_AT)
        .await
        .unwrap();

    let without: Vec<EpisodeFile> = shown().into_iter().filter(|f| f.path != path).collect();
    lib.rescan_at(without, 300).await;
    lib.rescan_at(shown(), 400).await;

    // It starts at the time of the scan that found it, past every version the
    // path had.
    let now = lib
        .store
        .file_creator(&lib.work, 1, path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((now.creator.is_none(), now.version), (true, 400));
    for held in [SEED, SEED + 1, SEED + 2] {
        let stale = lib
            .store
            .set_file_creator(&lib.work, 1, path, held, Some("kairan"), SET_AT)
            .await;
        assert!(
            matches!(stale, Err(CreatorError::Conflict)),
            "{held}: {stale:?}"
        );
    }
}

#[tokio::test]
async fn a_merge_keeps_a_named_creator_over_an_unknown_one_for_the_same_path() {
    let lib = library().await;
    let (to, _) = lib
        .store
        .add_folder(
            "/to".into(),
            scan(Vec::new()),
            300,
            std::slice::from_ref(&lib.folder),
        )
        .await
        .unwrap();
    lib.store
        .follow_move(&lib.folder.id, &to.id, "Show")
        .await
        .unwrap();
    // The destination: episode 2 named, episodes 1 and 3 unknown.
    lib.store
        .set_file_creator(
            &lib.work,
            1,
            "Season 01/S01E02.ass",
            SEED,
            Some("hanu"),
            SET_AT,
        )
        .await
        .unwrap();

    // Another work of the name with episode 1 named and episode 2 unknown.
    lib.store
        .record_scan(
            &lib.folder.id,
            Ok(scan(vec![work(
                "Show",
                vec![subtitle(1, "01"), subtitle(1, "02")],
            )])),
            400,
        )
        .await
        .unwrap();
    let other = lib.store.works(&lib.folder.id).await.unwrap().remove(0).id;
    lib.store
        .set_file_creator(
            &other,
            1,
            "Season 01/S01E01.ass",
            400,
            Some("kairan"),
            SET_AT,
        )
        .await
        .unwrap();

    assert_eq!(
        lib.store
            .follow_move(&lib.folder.id, &to.id, "Show")
            .await
            .unwrap(),
        Followed::Merged
    );
    let merged = lib.creators().await;
    // Unknown there, named in the moved work: the named one is kept, at a
    // version past both (the moved work's file was at 401).
    assert!(
        merged.contains(&row("Season 01/S01E01.ass", Some("카이란"), 402)),
        "{merged:?}"
    );
    // Named there, unknown in the moved work: it stays.
    assert!(
        merged.contains(&row("Season 01/S01E02.ass", Some("하느"), SEED + 1)),
        "{merged:?}"
    );
    // The stale version of the destination's episode 1 is refused.
    let stale = lib
        .store
        .set_file_creator(
            &lib.work,
            1,
            "Season 01/S01E01.ass",
            SEED,
            Some("hanu"),
            SET_AT,
        )
        .await;
    assert!(matches!(stale, Err(CreatorError::Conflict)), "{stale:?}");
}

/// The time each attributed file's creator was named, by episode.
async fn named_at(lib: &Library, season: u32) -> Vec<(String, i64)> {
    lib.store
        .attributed_subtitles(&lib.work, season)
        .await
        .unwrap()
        .into_iter()
        .map(|a| (a.episode, a.set_at))
        .collect()
}

#[tokio::test]
async fn the_time_a_creator_was_named_is_kept_with_it_and_follows_it() {
    let lib = library().await;
    let path = "Season 01/S01E02.ass";
    assert!(named_at(&lib, 1).await.is_empty());

    // Naming the season's files records the time for each.
    lib.store
        .name_unknown_creators(&lib.work, 1, "hanu", SET_AT)
        .await
        .unwrap();
    assert_eq!(
        named_at(&lib, 1).await,
        [
            ("01".to_owned(), SET_AT),
            ("02".to_owned(), SET_AT),
            ("03".to_owned(), SET_AT)
        ]
    );

    // The same creator again changes nothing, the time included; another
    // creator is named at its own time, and unknown clears it.
    lib.store
        .set_file_creator(&lib.work, 1, path, SEED + 1, Some("hanu"), SET_AT + 10)
        .await
        .unwrap();
    assert!(named_at(&lib, 1).await.contains(&("02".to_owned(), SET_AT)));
    lib.store
        .set_file_creator(&lib.work, 1, path, SEED + 1, Some("kairan"), SET_AT + 20)
        .await
        .unwrap();
    assert!(named_at(&lib, 1)
        .await
        .contains(&("02".to_owned(), SET_AT + 20)));
    lib.store
        .set_file_creator(&lib.work, 1, path, SEED + 2, None, SET_AT + 30)
        .await
        .unwrap();
    assert!(!named_at(&lib, 1)
        .await
        .iter()
        .any(|(episode, _)| episode == "02"));

    // A rescan that finds the files as they were, or reads one's episode
    // again, keeps the time; one that reads a file as a video drops it.
    lib.rescan(shown()).await;
    let mut reread = shown();
    reread
        .iter_mut()
        .find(|f| f.path == "Season 01/S01E03.ass")
        .unwrap()
        .episode = "13".to_owned();
    lib.rescan(reread).await;
    assert_eq!(
        named_at(&lib, 1).await,
        [("01".to_owned(), SET_AT), ("13".to_owned(), SET_AT)]
    );
    let mut as_video = shown();
    as_video
        .iter_mut()
        .find(|f| f.path == "Season 01/S01E01.ass")
        .unwrap()
        .kind = FileKind::Video;
    lib.rescan(as_video).await;
    lib.rescan(shown()).await;
    assert!(!named_at(&lib, 1)
        .await
        .iter()
        .any(|(episode, _)| episode == "01"));
}

#[tokio::test]
async fn a_merge_brings_the_time_of_the_creator_it_keeps() {
    let lib = library().await;
    let (to, _) = lib
        .store
        .add_folder(
            "/to".into(),
            scan(Vec::new()),
            300,
            std::slice::from_ref(&lib.folder),
        )
        .await
        .unwrap();
    lib.store
        .follow_move(&lib.folder.id, &to.id, "Show")
        .await
        .unwrap();
    lib.store
        .record_scan(
            &lib.folder.id,
            Ok(scan(vec![work(
                "Show",
                vec![subtitle(1, "01"), subtitle(1, "04")],
            )])),
            400,
        )
        .await
        .unwrap();
    let other = lib.store.works(&lib.folder.id).await.unwrap().remove(0).id;
    lib.store
        .name_unknown_creators(&other, 1, "kairan", SET_AT + 50)
        .await
        .unwrap();
    lib.store
        .follow_move(&lib.folder.id, &to.id, "Show")
        .await
        .unwrap();

    // Episode 1 was unknown in the destination: the moved work's creator and
    // its time. Episode 4 only the moved work had.
    let at = named_at(&lib, 1).await;
    assert!(at.contains(&("01".to_owned(), SET_AT + 50)), "{at:?}");
    assert!(at.contains(&("04".to_owned(), SET_AT + 50)), "{at:?}");
}
