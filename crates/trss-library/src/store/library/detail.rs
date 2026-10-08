//! One work with everything recorded under it, for the work detail screen
//! (`docs/specs/library.md`, 작품 상세 화면): its seasons, the episodes of each
//! with their video and subtitle files and the files that could not be attached
//! to an episode. Read with a fixed number of queries.
//!
//! - **Episodes are compared as numbers**, like the library list does
//!   ([`super::overview`]): `013` and `13` are one episode, shown as the
//!   smallest of the written forms and holding the files of both. An episode
//!   that is a decimal number (`17.5`) sorts between its neighbours (`17`,
//!   `18`); one that is no number at all (`SP`) comes after every number.
//!   Episodes come in ascending order; the screen reverses them for
//!   latest-first.
//! - **A season folder with no episode** is a season with an empty list.
//! - **A work whose folder is gone** keeps everything recorded for it: the
//!   screen shows it as the last record, not as holdings.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    discovery::{FileKind, Reason},
    store::library::{AppliedCopy, FileRecord, UnrecognizedRecord},
};
use trss_core::{
    episode::{EpisodeKey, EpisodeNumber},
    Millis,
};

/// An episode of a season with its files.
#[derive(Debug, Clone, PartialEq)]
pub struct EpisodeDetail {
    /// As written in the file names; the smallest written form when several
    /// spellings (`013`, `13`) are one episode.
    pub episode: String,
    /// The episode as a number, for ordering and for the deep link; `None` when
    /// it is no number.
    pub number: Option<f64>,
    pub video: Vec<FileRecord>,
    pub subtitle: Vec<FileRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeasonDetail {
    pub number: u32,
    /// Ascending.
    pub episodes: Vec<EpisodeDetail>,
}

/// A recorded work with its seasons and its files not attached to an episode.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkDetail {
    pub id: String,
    pub dir_name: String,
    pub missing: bool,
    pub watch_folder_id: String,
    pub watch_folder_path: String,
    /// When the work first appeared; `None`: unknown.
    pub first_seen_at: Option<Millis>,
    /// Ascending.
    pub seasons: Vec<SeasonDetail>,
    pub unrecognized: Vec<UnrecognizedRecord>,
}

#[derive(Default)]
struct Gathered {
    written: String,
    video: Vec<FileRecord>,
    subtitle: Vec<FileRecord>,
}

/// The work `id` with everything under it, or `None` when there is no such work.
pub(super) fn detail(conn: &Connection, id: &str) -> rusqlite::Result<Option<WorkDetail>> {
    let head = conn
        .prepare_cached(
            "SELECT w.id, w.dir_name, w.missing, w.first_seen_at, f.id, f.path
               FROM works w JOIN watch_folders f ON f.id = w.watch_folder_id
              WHERE w.id = ?1 AND f.unregistered_at IS NULL",
        )?
        .query_row([id], |row| {
            Ok(WorkDetail {
                id: row.get(0)?,
                dir_name: row.get(1)?,
                missing: row.get::<_, i64>(2)? != 0,
                first_seen_at: row.get(3)?,
                watch_folder_id: row.get(4)?,
                watch_folder_path: row.get(5)?,
                seasons: Vec::new(),
                unrecognized: Vec::new(),
            })
        })
        .optional()?;
    let Some(mut work) = head else {
        return Ok(None);
    };

    let mut seasons: BTreeMap<u32, BTreeMap<EpisodeKey, Gathered>> = BTreeMap::new();
    {
        let mut stmt = conn.prepare_cached("SELECT number FROM seasons WHERE work_id = ?1")?;
        let mut cursor = stmt.query([id])?;
        while let Some(row) = cursor.next()? {
            seasons.entry(row.get(0)?).or_default();
        }
    }
    {
        let mut stmt = conn.prepare_cached(
            "SELECT m.season, m.episode, m.path, m.kind, m.added_at,
                    m.creator_source_id, s.creator_name, s.anime_no, m.creator_version,
                    ap.id IS NOT NULL, st.creator
               FROM media_files m
               LEFT JOIN subtitle_sources s ON s.id = m.creator_source_id
               LEFT JOIN subtitle_applied ap ON ap.work_id = m.work_id AND ap.path = m.path
                    AND ap.removed_at IS NULL AND m.kind = 'subtitle'
               LEFT JOIN subtitle_stored st ON st.id = ap.stored_id
              WHERE m.work_id = ?1 ORDER BY m.path",
        )?;
        let mut cursor = stmt.query([id])?;
        while let Some(row) = cursor.next()? {
            let season: u32 = row.get(0)?;
            let episode: String = row.get(1)?;
            // An applied copy shows its stored copy's creator, whatever the user
            // named for the path before the app replaced the file.
            let applied = row
                .get::<_, bool>(9)?
                .then(|| row.get(10).map(|creator| AppliedCopy { creator }))
                .transpose()?;
            let named = super::creators::creator_of(row.get(5)?, row.get(6)?, row.get(7)?);
            let file = FileRecord {
                path: row.get(2)?,
                kind: FileKind::from_code(&row.get::<_, String>(3)?).unwrap_or(FileKind::Video),
                added_at: row.get(4)?,
                creator: named.filter(|_| applied.is_none()),
                creator_version: row.get(8)?,
                applied,
            };
            let gathered = seasons
                .entry(season)
                .or_default()
                .entry(EpisodeKey::of(&episode))
                .or_default();
            if gathered.written.is_empty() || episode < gathered.written {
                gathered.written = episode;
            }
            match file.kind {
                FileKind::Video => gathered.video.push(file),
                FileKind::Subtitle => gathered.subtitle.push(file),
            }
        }
    }
    work.seasons = seasons
        .into_iter()
        .map(|(number, episodes)| SeasonDetail {
            number,
            episodes: episodes
                .into_iter()
                .map(|(key, gathered)| EpisodeDetail {
                    number: key.number().map(EpisodeNumber::to_f64),
                    episode: gathered.written,
                    video: gathered.video,
                    subtitle: gathered.subtitle,
                })
                .collect(),
        })
        .collect();

    let mut stmt = conn.prepare_cached(super::UNRECOGNIZED_OF_WORK)?;
    work.unrecognized = stmt
        .query_map([id], |row| {
            Ok(UnrecognizedRecord {
                path: row.get(0)?,
                reason: Reason::from_code(&row.get::<_, String>(1)?).unwrap_or(Reason::NoEpisode),
                checked: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some(work))
}

/// What a season of a work holds, without the files: for the places that show
/// only how far a season has come.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeasonHoldings {
    /// The work's folder name.
    pub dir_name: String,
    /// The work's folder is gone; the counts are the last record.
    pub missing: bool,
    /// How many episodes of the season have a video (`013` and `13` are one).
    pub videos: u32,
}

/// The holdings of season `season` of the work `id` (zero videos for a season
/// with none), or `None` when there is no such work in a registered folder.
pub(super) fn season_holdings(
    conn: &Connection,
    id: &str,
    season: u32,
) -> rusqlite::Result<Option<SeasonHoldings>> {
    let head: Option<(String, bool)> = conn
        .prepare_cached(
            "SELECT w.dir_name, w.missing FROM works w
               JOIN watch_folders f ON f.id = w.watch_folder_id
              WHERE w.id = ?1 AND f.unregistered_at IS NULL",
        )?
        .query_row([id], |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0)))
        .optional()?;
    let Some((dir_name, missing)) = head else {
        return Ok(None);
    };
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT episode FROM media_files
          WHERE work_id = ?1 AND season = ?2 AND kind = 'video'",
    )?;
    let episodes: std::collections::BTreeSet<EpisodeKey> = stmt
        .query_map(params![id, season], |row| {
            Ok(EpisodeKey::of(&row.get::<_, String>(0)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some(SeasonHoldings {
        dir_name,
        missing,
        videos: u32::try_from(episodes.len()).unwrap_or(u32::MAX),
    }))
}

/// What one whole-numbered episode of a season has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Held {
    pub video: bool,
    pub subtitle: bool,
}

/// The whole-numbered episodes of season `season` of the work `id` that have a
/// file, by number (`013` and `13` are one episode; `17.5` and `SP` are left
/// out). Empty for a work whose folder is gone, whose last record is not
/// holdings. `None` when there is no such work in a registered folder.
pub(super) fn season_episodes(
    conn: &Connection,
    id: &str,
    season: u32,
) -> rusqlite::Result<Option<BTreeMap<u32, Held>>> {
    let head: Option<bool> = conn
        .prepare_cached(
            "SELECT w.missing FROM works w
               JOIN watch_folders f ON f.id = w.watch_folder_id
              WHERE w.id = ?1 AND f.unregistered_at IS NULL",
        )?
        .query_row([id], |row| Ok(row.get::<_, i64>(0)? != 0))
        .optional()?;
    let Some(missing) = head else {
        return Ok(None);
    };
    let mut held: BTreeMap<u32, Held> = BTreeMap::new();
    if missing {
        return Ok(Some(held));
    }
    let mut stmt = conn.prepare_cached(
        "SELECT episode, kind FROM media_files WHERE work_id = ?1 AND season = ?2",
    )?;
    let mut cursor = stmt.query(params![id, season])?;
    while let Some(row) = cursor.next()? {
        let episode: String = row.get(0)?;
        let kind: String = row.get(1)?;
        let Some(number) = EpisodeNumber::parse(&episode)
            .and_then(|n| n.whole())
            .and_then(|n| u32::try_from(n).ok())
        else {
            continue;
        };
        let entry = held.entry(number).or_default();
        match FileKind::from_code(&kind).unwrap_or(FileKind::Video) {
            FileKind::Video => entry.video = true,
            FileKind::Subtitle => entry.subtitle = true,
        }
    }
    Ok(Some(held))
}

/// The video at one place of one work, by the keys of `works`
/// (`watch_folder_id`, `dir_name`) and `media_files` (`work_id`, `path`).
const FIND_VIDEO_SQL: &str = "SELECT m.work_id, m.season FROM works w
           JOIN media_files m ON m.work_id = w.id AND m.path = ?3
          WHERE w.watch_folder_id = ?1 AND w.dir_name = ?2 AND m.kind = 'video'";

/// The work and season number of the video recorded at each of the absolute
/// `paths` (a watch folder, a work folder, then the file's path below it), in
/// the order given: `None` for a path with no such file in a registered folder.
///
/// A path is cut at the registered watch folders it starts with and at the
/// first `/` after them (the work folder), and the rest is looked up by key,
/// so the cost of a lookup does not grow with the files of the library.
pub(super) fn find_videos(
    conn: &Connection,
    paths: &[String],
) -> rusqlite::Result<Vec<Option<(String, u32)>>> {
    let mut folders =
        conn.prepare_cached("SELECT id, path FROM watch_folders WHERE unregistered_at IS NULL")?;
    let folders: Vec<(String, String)> = folders
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut find = conn.prepare_cached(FIND_VIDEO_SQL)?;

    let mut found = Vec::with_capacity(paths.len());
    for path in paths {
        let mut hit = None;
        for (folder_id, folder_path) in &folders {
            let Some(below) = path
                .strip_prefix(folder_path.as_str())
                .and_then(|rest| rest.strip_prefix('/'))
            else {
                continue;
            };
            let Some((dir_name, relative)) = below.split_once('/') else {
                continue;
            };
            hit = find
                .query_row(params![folder_id, dir_name, relative], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .optional()?;
            if hit.is_some() {
                break;
            }
        }
        found.push(hit);
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{
        discovery::{EpisodeFile, Scan, ScannedWork, Unrecognized, WorkRead},
        store::library::LibraryStore,
    };
    use trss_core::db::Db;

    fn file(season: u32, episode: &str, name: &str, kind: FileKind) -> EpisodeFile {
        EpisodeFile {
            path: format!("Season {season:02}/{name}"),
            kind,
            season,
            episode: episode.to_owned(),
        }
    }

    fn scan(name: &str, seasons: &[u32], files: Vec<EpisodeFile>, un: Vec<Unrecognized>) -> Scan {
        Scan {
            works: vec![WorkRead::Read(ScannedWork {
                dir_name: name.to_owned(),
                seasons: seasons.iter().copied().collect::<BTreeSet<_>>(),
                files,
                unrecognized: un,
            })],
        }
    }

    async fn store_with(scan: Scan) -> (LibraryStore, String) {
        let store = LibraryStore::new(Db::open_blocking(":memory:").unwrap());
        store.add_folder("/w".into(), scan, 100, &[]).await.unwrap();
        let id = store.overview().await.unwrap()[0].id.clone();
        (store, id)
    }

    #[tokio::test]
    async fn a_work_comes_with_seasons_episodes_files_and_the_files_left_over() {
        use FileKind::{Subtitle, Video};
        let (store, id) = store_with(scan(
            "Show",
            &[1, 2, 3],
            vec![
                file(1, "02", "S01E02.mkv", Video),
                file(1, "01", "S01E01.mkv", Video),
                file(1, "01", "S01E01.ko.ass", Subtitle),
                file(1, "013", "S01E013.mkv", Video),
                file(1, "13", "S01E13.ko.srt", Subtitle),
                file(1, "17.5", "S01E17.5.mkv", Video),
                file(1, "14", "S01E14.mkv", Video),
                file(2, "01", "S02E01.mkv", Video),
            ],
            vec![Unrecognized {
                path: "Extras/PV.mkv".into(),
                reason: Reason::OutsideSeason,
                check: None,
            }],
        ))
        .await;

        let detail = store.work_detail(&id).await.unwrap().unwrap();
        assert_eq!(detail.dir_name, "Show");
        assert_eq!(detail.watch_folder_path, "/w");
        assert!(!detail.missing);
        // Season 3 has a folder and nothing in it.
        assert_eq!(
            detail.seasons.iter().map(|s| s.number).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert!(detail.seasons[2].episodes.is_empty());

        let one: Vec<_> = detail.seasons[0]
            .episodes
            .iter()
            .map(|e| e.episode.as_str())
            .collect();
        // `013` and `13` are one episode shown as `013`, after `02` and before `14`.
        assert_eq!(one, ["01", "02", "013", "14", "17.5"]);
        let first = &detail.seasons[0].episodes[0];
        assert_eq!(first.number, Some(1.0));
        assert_eq!(first.video.len(), 1);
        assert_eq!(first.subtitle[0].path, "Season 01/S01E01.ko.ass");
        assert_eq!(first.subtitle[0].added_at, None);
        let merged = &detail.seasons[0].episodes[2];
        assert_eq!(merged.number, Some(13.0));
        assert_eq!(merged.video[0].path, "Season 01/S01E013.mkv");
        assert_eq!(merged.subtitle[0].path, "Season 01/S01E13.ko.srt");
        assert_eq!(detail.seasons[0].episodes[4].number, Some(17.5));

        assert_eq!(detail.unrecognized.len(), 1);
        assert_eq!(detail.unrecognized[0].path, "Extras/PV.mkv");
        assert_eq!(detail.unrecognized[0].reason, Reason::OutsideSeason);

        assert_eq!(store.work_detail("no-such-work").await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_whole_numbered_episodes_of_a_season_say_what_they_have() {
        use FileKind::{Subtitle, Video};
        let (store, id) = store_with(scan(
            "Show",
            &[1, 2],
            vec![
                file(1, "01", "S01E01.mkv", Video),
                file(1, "01", "S01E01.ko.ass", Subtitle),
                file(1, "013", "S01E013.mkv", Video),
                file(1, "13", "S01E13.ko.srt", Subtitle),
                file(1, "02", "S01E02.ko.ass", Subtitle),
                file(1, "17.5", "S01E17.5.mkv", Video),
                file(1, "SP", "S01SP.mkv", Video),
                file(2, "01", "S02E01.mkv", Video),
            ],
            Vec::new(),
        ))
        .await;

        let held = store.season_episodes(&id, 1).await.unwrap().unwrap();
        assert_eq!(
            held.into_iter().collect::<Vec<_>>(),
            [
                (
                    1,
                    Held {
                        video: true,
                        subtitle: true
                    }
                ),
                // A subtitle without its video.
                (
                    2,
                    Held {
                        video: false,
                        subtitle: true
                    }
                ),
                // `013` and `13` are one episode.
                (
                    13,
                    Held {
                        video: true,
                        subtitle: true
                    }
                ),
            ]
        );
        assert_eq!(
            store.season_episodes(&id, 3).await.unwrap().unwrap().len(),
            0
        );
        assert_eq!(
            store.season_episodes("no-such-work", 1).await.unwrap(),
            None
        );

        // Several seasons at once read the same, in the order asked.
        let many = store
            .seasons_episodes(vec![
                ("no-such-work".into(), 1),
                (id.clone(), 2),
                (id.clone(), 1),
            ])
            .await
            .unwrap();
        assert_eq!(many.len(), 3);
        assert_eq!(many[0], None);
        assert_eq!(many[1].as_ref().unwrap().keys().collect::<Vec<_>>(), [&1]);
        assert_eq!(
            many[2].as_ref().unwrap().keys().collect::<Vec<_>>(),
            [&1, &2, &13]
        );
    }

    #[tokio::test]
    async fn a_work_whose_folder_is_gone_has_no_episodes_to_hold() {
        let (store, id) = store_with(scan(
            "Show",
            &[1],
            vec![file(1, "01", "S01E01.mkv", FileKind::Video)],
            Vec::new(),
        ))
        .await;
        let folder = store.folders().await.unwrap().remove(0);
        store
            .record_scan(&folder.id, Ok(Scan { works: Vec::new() }), 200)
            .await
            .unwrap();

        // The work stays known, but its last record is not holdings.
        let held = store.season_episodes(&id, 1).await.unwrap().unwrap();
        assert!(held.is_empty());
    }

    #[tokio::test]
    async fn a_work_whose_folder_is_gone_keeps_its_records() {
        let (store, id) = store_with(scan(
            "Show",
            &[1],
            vec![file(1, "01", "S01E01.mkv", FileKind::Video)],
            Vec::new(),
        ))
        .await;
        let folder = store.folders().await.unwrap().remove(0);
        store
            .record_scan(&folder.id, Ok(Scan { works: Vec::new() }), 200)
            .await
            .unwrap();
        let detail = store.work_detail(&id).await.unwrap().unwrap();
        assert!(detail.missing);
        assert_eq!(detail.seasons[0].episodes.len(), 1);
    }

    #[tokio::test]
    async fn videos_are_found_by_their_absolute_path_in_registered_folders_only() {
        use FileKind::{Subtitle, Video};
        let (store, id) = store_with(scan(
            "Show",
            &[0, 1, 2],
            vec![
                file(1, "01", "S01E01.mkv", Video),
                file(1, "01", "S01E01.ko.ass", Subtitle),
                file(2, "01", "S02E01.mkv", Video),
                file(0, "01", "SP01.mkv", Video),
            ],
            Vec::new(),
        ))
        .await;
        let ask = |paths: &[&str]| paths.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
        let paths = ask(&[
            "/w/Show/Season 01/S01E01.mkv",
            "/w/Show/Season 02/S02E01.mkv",
            "/w/Show/Season 00/SP01.mkv",
            // A subtitle, another folder, another work, a folder of no work,
            // a path above a work folder, and one that only starts alike.
            "/w/Show/Season 01/S01E01.ko.ass",
            "/other/Show/Season 01/S01E01.mkv",
            "/w/Nope/Season 01/S01E01.mkv",
            "/w/Show",
            "/w/Show/",
            "/w/Show/S01E01.mkv",
            "/wShow/Season 01/S01E01.mkv",
            "",
        ]);

        let found = store.find_videos(paths.clone()).await.unwrap();

        assert_eq!(
            found,
            [
                Some((id.clone(), 1)),
                Some((id.clone(), 2)),
                Some((id.clone(), 0)),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ]
        );

        // A folder that is not registered any more holds nothing.
        let folder = store.folders().await.unwrap().remove(0);
        store.remove_folder(&folder.id, 300).await.unwrap();
        let found = store.find_videos(paths).await.unwrap();
        assert!(found.iter().all(Option::is_none), "{found:?}");
    }

    /// Looking a video up by its path must go through the keys of `works` and
    /// `media_files`; comparing a built-up path scans every file of the
    /// library for every lookup.
    #[tokio::test]
    async fn a_video_is_looked_up_by_keys_not_by_scanning_the_files() {
        let (store, _id) = store_with(scan(
            "Show",
            &[1],
            vec![file(1, "01", "S01E01.mkv", FileKind::Video)],
            Vec::new(),
        ))
        .await;
        let plan: Vec<String> = store
            .db
            .run::<_, trss_core::db::DbError, _>(|c| {
                let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {FIND_VIDEO_SQL}"))?;
                let args = vec!["/w"; stmt.parameter_count()];
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(args), |row| {
                        row.get::<_, String>(3)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
            .unwrap();

        assert!(
            plan.iter().all(|step| !step.starts_with("SCAN")),
            "{plan:?}"
        );
    }
}
