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

use rusqlite::{Connection, OptionalExtension};

use super::{FileRecord, UnrecognizedRecord};
use crate::{
    discovery::{FileKind, Reason},
    store::history::Millis,
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

/// An episode as a key: whole and decimal numbers by value (the fraction as its
/// digits without trailing zeros, which compare as text the way fractions
/// compare as numbers), anything else by its text and after every number.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum EpisodeKey {
    Number(u128, String),
    Other(String),
}

fn key_of(episode: &str) -> EpisodeKey {
    let (whole, fraction) = match episode.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (episode, None),
    };
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    if digits(whole) && fraction.is_none_or(digits) {
        if let Ok(number) = whole.parse::<u128>() {
            let fraction = fraction.unwrap_or("").trim_end_matches('0');
            return EpisodeKey::Number(number, fraction.to_owned());
        }
    }
    EpisodeKey::Other(episode.to_owned())
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
        .query_row(
            "SELECT w.id, w.dir_name, w.missing, w.first_seen_at, f.id, f.path
               FROM works w JOIN watch_folders f ON f.id = w.watch_folder_id
              WHERE w.id = ?1",
            [id],
            |row| {
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
            },
        )
        .optional()?;
    let Some(mut work) = head else {
        return Ok(None);
    };

    let mut seasons: BTreeMap<u32, BTreeMap<EpisodeKey, Gathered>> = BTreeMap::new();
    {
        let mut stmt = conn.prepare("SELECT number FROM seasons WHERE work_id = ?1")?;
        let mut cursor = stmt.query([id])?;
        while let Some(row) = cursor.next()? {
            seasons.entry(row.get(0)?).or_default();
        }
    }
    {
        let mut stmt = conn.prepare(
            "SELECT season, episode, path, kind, added_at FROM media_files
              WHERE work_id = ?1 ORDER BY path",
        )?;
        let mut cursor = stmt.query([id])?;
        while let Some(row) = cursor.next()? {
            let season: u32 = row.get(0)?;
            let episode: String = row.get(1)?;
            let file = FileRecord {
                path: row.get(2)?,
                kind: FileKind::from_code(&row.get::<_, String>(3)?).unwrap_or(FileKind::Video),
                added_at: row.get(4)?,
            };
            let gathered = seasons
                .entry(season)
                .or_default()
                .entry(key_of(&episode))
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
                    number: matches!(key, EpisodeKey::Number(..))
                        .then(|| gathered.written.parse::<f64>().ok())
                        .flatten(),
                    episode: gathered.written,
                    video: gathered.video,
                    subtitle: gathered.subtitle,
                })
                .collect(),
        })
        .collect();

    let mut stmt = conn
        .prepare("SELECT path, reason FROM unrecognized_files WHERE work_id = ?1 ORDER BY path")?;
    work.unrecognized = stmt
        .query_map([id], |row| {
            Ok(UnrecognizedRecord {
                path: row.get(0)?,
                reason: Reason::from_code(&row.get::<_, String>(1)?).unwrap_or(Reason::NoEpisode),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some(work))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{
        discovery::{EpisodeFile, Scan, ScannedWork, Unrecognized, WorkRead},
        store::{db::Db, library::LibraryStore},
    };

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
        store.add_folder("/w".into(), scan, 100).await.unwrap();
        let id = store.overview().await.unwrap()[0].id.clone();
        (store, id)
    }

    #[test]
    fn episodes_order_as_numbers_and_other_text_comes_last() {
        let mut keys: Vec<(&str, EpisodeKey)> =
            ["10", "9", "17.5", "17", "18", "SP", "2.25", "2.5", "02"]
                .into_iter()
                .map(|e| (e, key_of(e)))
                .collect();
        keys.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(b.0)));
        let order: Vec<&str> = keys.into_iter().map(|(e, _)| e).collect();
        assert_eq!(
            order,
            ["02", "2.25", "2.5", "9", "10", "17", "17.5", "18", "SP"]
        );
        // Spellings of one number are one key.
        assert_eq!(key_of("013"), key_of("13"));
        assert_eq!(key_of("1.50"), key_of("1.5"));
        assert_ne!(key_of("1.5"), key_of("1"));
        // Larger than any float keeps its digits.
        assert!(key_of("9007199254740993") < key_of("9007199254740994"));
        // Not a number: a sign, an empty part or a second dot.
        for text in ["-1", ".5", "1.", "1.2.3", "1e3", ""] {
            assert!(matches!(key_of(text), EpisodeKey::Other(_)), "{text}");
        }
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
}
