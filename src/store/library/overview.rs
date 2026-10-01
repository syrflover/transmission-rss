//! The library list: one summary per work, read with a fixed number of queries
//! (works, media files, unrecognized files, linked season entries) however many
//! works there are.
//!
//! What a summary says (`docs/specs/library.md`, 라이브러리 화면):
//!
//! - **Holdings** are those of the work's *latest season*: the highest season
//!   number recorded for it. The episodes that have a video, and those that have
//!   a subtitle, come as [`EpisodeRange`]s.
//! - **Episodes are compared as numbers without converting them to a float**:
//!   `013` and `13` are the same episode and count once, shown as the smallest
//!   of the written forms (`013` < `13`). A range is a run of consecutive
//!   numbers, so a gap splits it (`1–3`, `5–12`). An episode that is not a
//!   whole number (`17.5`) is its own range, listed after the numeric ones.
//! - **A work whose folder is gone** has no holdings counted: its files are
//!   still recorded, but they are not there to open. It keeps its name and the
//!   times recorded for it, so it stays in the list and in the orders.
//! - **`subtitle_check_needed`** is true when an unrecognized file of the work
//!   is a subtitle (`.ass`, `.srt`, …) the scan could not attach to an episode.
//!   A download in progress (`.part`) is not one.
//! - **Airing** comes from the AniList entries the work's *latest local season*
//!   links: its year is the first entry's start year (`None` when unlinked or
//!   unknown), it is airing while any of its entries is releasing, and the
//!   titles of every linked entry of every season are kept for the search.
//! - **Added times** are `None` when unknown. The latest video / subtitle time
//!   is the latest *known* time of any season; it is unknown only when no file
//!   of that kind has a known time.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;

use crate::{
    discovery::{kind_of, FileKind, Reason},
    store::{history::Millis, seasons},
};

/// A run of consecutive episodes, as written (`first` and `last` are the
/// written forms of its ends; they are equal for a single episode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeRange {
    pub first: String,
    pub last: String,
}

/// How many of the episodes that have a video also have a subtitle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleCoverage {
    /// There is a video, and every episode with a video has a subtitle.
    All,
    /// Some subtitle exists, but not for every episode with a video.
    Some,
    /// No subtitle.
    None,
}

impl SubtitleCoverage {
    pub fn code(self) -> &'static str {
        match self {
            SubtitleCoverage::All => "all",
            SubtitleCoverage::Some => "some",
            SubtitleCoverage::None => "none",
        }
    }
}

/// One row of the library list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkOverview {
    pub id: String,
    pub dir_name: String,
    pub missing: bool,
    pub watch_folder_id: String,
    pub watch_folder_path: String,
    /// When the work first appeared; `None`: unknown.
    pub first_seen_at: Option<Millis>,
    /// `None` when no season folder is recorded.
    pub latest_season: Option<u32>,
    /// Episodes of the latest season that have a video.
    pub video: Vec<EpisodeRange>,
    /// Episodes of the latest season that have a subtitle.
    pub subtitle: Vec<EpisodeRange>,
    /// `None` for a work whose folder is gone.
    pub subtitle_coverage: Option<SubtitleCoverage>,
    pub subtitle_check_needed: bool,
    /// The latest known time a video was added, over every season.
    pub video_added_at: Option<Millis>,
    /// The latest known time a subtitle was added, over every season.
    pub subtitle_added_at: Option<Millis>,
    /// The year the latest season's first linked AniList entry started
    /// airing; `None`: unknown (no link, or the entry has no start year).
    pub airing_year: Option<i32>,
    /// Whether an entry linked to the latest season is airing now.
    pub airing: bool,
    /// The native, English and romaji titles of the entries linked to the
    /// work's recorded seasons, for the search.
    pub linked_titles: Vec<String>,
}

/// An episode as a key: whole numbers by value, anything else by its text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum EpisodeKey {
    Number(u128),
    Other(String),
}

fn key_of(episode: &str) -> EpisodeKey {
    if !episode.is_empty() && episode.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(number) = episode.parse::<u128>() {
            return EpisodeKey::Number(number);
        }
    }
    EpisodeKey::Other(episode.to_owned())
}

/// The episodes (one written form for each distinct episode) in order.
type Episodes = BTreeMap<EpisodeKey, String>;

fn add(episodes: &mut Episodes, episode: &str) {
    let written = episodes
        .entry(key_of(episode))
        .or_insert_with(|| episode.to_owned());
    if episode < written.as_str() {
        *written = episode.to_owned();
    }
}

fn ranges(episodes: &Episodes) -> Vec<EpisodeRange> {
    let mut out: Vec<EpisodeRange> = Vec::new();
    let mut last_number: Option<u128> = None;
    for (key, written) in episodes {
        match key {
            EpisodeKey::Number(n) => {
                if last_number.is_some_and(|last| last.checked_add(1) == Some(*n)) {
                    out.last_mut().expect("a run is open").last = written.clone();
                } else {
                    out.push(EpisodeRange {
                        first: written.clone(),
                        last: written.clone(),
                    });
                }
                last_number = Some(*n);
            }
            EpisodeKey::Other(_) => out.push(EpisodeRange {
                first: written.clone(),
                last: written.clone(),
            }),
        }
    }
    out
}

/// A recorded media file, as far as the list needs it.
struct Media {
    season: u32,
    episode: String,
    kind: FileKind,
    added_at: Option<Millis>,
}

struct Holdings {
    video: Vec<EpisodeRange>,
    subtitle: Vec<EpisodeRange>,
    coverage: SubtitleCoverage,
    video_added_at: Option<Millis>,
    subtitle_added_at: Option<Millis>,
}

fn holdings(latest_season: Option<u32>, files: &[Media]) -> Holdings {
    let (mut video, mut subtitle) = (Episodes::new(), Episodes::new());
    let (mut video_added_at, mut subtitle_added_at) = (None, None);
    for file in files {
        let (episodes, added) = match file.kind {
            FileKind::Video => (&mut video, &mut video_added_at),
            FileKind::Subtitle => (&mut subtitle, &mut subtitle_added_at),
        };
        // `Option` orders `None` first, so the latest known time wins.
        *added = (*added).max(file.added_at);
        if Some(file.season) == latest_season {
            add(episodes, &file.episode);
        }
    }
    let coverage = if subtitle.is_empty() {
        SubtitleCoverage::None
    } else if !video.is_empty() && video.keys().all(|k| subtitle.contains_key(k)) {
        SubtitleCoverage::All
    } else {
        SubtitleCoverage::Some
    };
    Holdings {
        video: ranges(&video),
        subtitle: ranges(&subtitle),
        coverage,
        video_added_at,
        subtitle_added_at,
    }
}

/// Every work with its summary, ordered by folder name.
pub(super) fn overview(conn: &Connection) -> rusqlite::Result<Vec<WorkOverview>> {
    struct Row {
        id: String,
        dir_name: String,
        missing: bool,
        first_seen_at: Option<Millis>,
        folder_id: String,
        folder_path: String,
        latest_season: Option<u32>,
    }
    let rows: Vec<Row> = {
        let mut stmt = conn.prepare(
            "SELECT w.id, w.dir_name, w.missing, w.first_seen_at, f.id, f.path,
                    (SELECT max(s.number) FROM seasons s WHERE s.work_id = w.id)
               FROM works w JOIN watch_folders f ON f.id = w.watch_folder_id
              ORDER BY w.dir_name, w.id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Row {
                id: row.get(0)?,
                dir_name: row.get(1)?,
                missing: row.get::<_, i64>(2)? != 0,
                first_seen_at: row.get(3)?,
                folder_id: row.get(4)?,
                folder_path: row.get(5)?,
                latest_season: row.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };

    let mut files: HashMap<String, Vec<Media>> = HashMap::new();
    {
        let mut stmt =
            conn.prepare("SELECT work_id, season, episode, kind, added_at FROM media_files")?;
        let mut cursor = stmt.query([])?;
        while let Some(row) = cursor.next()? {
            let kind = FileKind::from_code(&row.get::<_, String>(3)?).unwrap_or(FileKind::Video);
            files.entry(row.get(0)?).or_default().push(Media {
                season: row.get(1)?,
                episode: row.get(2)?,
                kind,
                added_at: row.get(4)?,
            });
        }
    }

    let mut check_needed: HashMap<String, bool> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT work_id, path, reason FROM unrecognized_files")?;
        let mut cursor = stmt.query([])?;
        while let Some(row) = cursor.next()? {
            let path: String = row.get(1)?;
            let reason: String = row.get(2)?;
            let is_subtitle = kind_of(&path) == Some(FileKind::Subtitle);
            if is_subtitle && reason != Reason::Partial.code() {
                check_needed.insert(row.get(0)?, true);
            }
        }
    }

    let mut linked = seasons::library_facts(conn)?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let facts = linked.remove(&row.id).unwrap_or_default();
            let latest = |fact: &&seasons::LinkedFact| Some(fact.season) == row.latest_season;
            let airing_year = facts
                .iter()
                .filter(latest)
                .find(|fact| fact.position == 0)
                .and_then(|fact| fact.start_year);
            let airing = facts
                .iter()
                .filter(latest)
                .any(|fact| fact.status.as_deref() == Some("RELEASING"));
            let linked_titles = facts.into_iter().flat_map(|fact| fact.titles).collect();
            let held = holdings(
                row.latest_season,
                files.get(&row.id).map_or(&[][..], Vec::as_slice),
            );
            let (video, subtitle, coverage) = if row.missing {
                (Vec::new(), Vec::new(), None)
            } else {
                (held.video, held.subtitle, Some(held.coverage))
            };
            WorkOverview {
                subtitle_check_needed: !row.missing && check_needed.contains_key(&row.id),
                id: row.id,
                dir_name: row.dir_name,
                missing: row.missing,
                watch_folder_id: row.folder_id,
                watch_folder_path: row.folder_path,
                first_seen_at: row.first_seen_at,
                latest_season: row.latest_season,
                video,
                subtitle,
                subtitle_coverage: coverage,
                video_added_at: held.video_added_at,
                subtitle_added_at: held.subtitle_added_at,
                airing_year,
                airing,
                linked_titles,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn episodes(written: &[&str]) -> Vec<(String, String)> {
        let mut set = Episodes::new();
        for e in written {
            add(&mut set, e);
        }
        ranges(&set)
            .into_iter()
            .map(|r| (r.first, r.last))
            .collect()
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn consecutive_episodes_make_one_range_and_a_gap_splits_it() {
        assert_eq!(episodes(&["01", "02", "03"]), pairs(&[("01", "03")]));
        assert_eq!(
            episodes(&["01", "02", "03", "05", "06", "12"]),
            pairs(&[("01", "03"), ("05", "06"), ("12", "12")])
        );
        assert_eq!(episodes(&[]), pairs(&[]));
    }

    #[test]
    fn episodes_compare_by_number_not_by_text() {
        // Written order is not numeric order, and `2` is not after `10` here.
        assert_eq!(episodes(&["10", "9", "11"]), pairs(&[("9", "11")]));
        // `013` and `13` are one episode, shown as the smaller written form.
        assert_eq!(episodes(&["13", "013", "14"]), pairs(&[("013", "14")]));
        // Zero is an episode, and a leading zero does not change its value.
        assert_eq!(episodes(&["00", "1"]), pairs(&[("00", "1")]));
    }

    #[test]
    fn a_number_larger_than_any_float_keeps_its_digits() {
        let big = "9007199254740993"; // 2^53 + 1: not representable as an f64
        let next = "9007199254740994";
        let apart = "9007199254740996";
        assert_eq!(
            episodes(&[big, next, apart]),
            pairs(&[(big, next), (apart, apart)])
        );
        // Beyond u128 the text is all there is: it stays a range of its own.
        let huge = "9".repeat(60);
        assert_eq!(episodes(&[&huge]), pairs(&[(&huge, &huge)]));
    }

    #[test]
    fn episodes_that_are_not_whole_numbers_stay_apart_and_come_last() {
        assert_eq!(
            episodes(&["17.5", "01", "02", "SP", "17"]),
            pairs(&[("01", "02"), ("17", "17"), ("17.5", "17.5"), ("SP", "SP")])
        );
        // `1.5` does not join `1` and `2`, and `1` and `2` still make a range.
        assert_eq!(
            episodes(&["1", "1.5", "2"]),
            pairs(&[("1", "2"), ("1.5", "1.5")])
        );
    }

    fn media(season: u32, episode: &str, kind: FileKind, added_at: Option<Millis>) -> Media {
        Media {
            season,
            episode: episode.into(),
            kind,
            added_at,
        }
    }

    #[test]
    fn holdings_follow_the_latest_season_and_times_span_every_season() {
        let files = [
            media(1, "01", FileKind::Video, Some(10)),
            media(1, "01", FileKind::Subtitle, Some(30)),
            media(2, "01", FileKind::Video, None),
            media(2, "02", FileKind::Video, Some(20)),
            media(2, "01", FileKind::Subtitle, None),
        ];
        let held = holdings(Some(2), &files);
        assert_eq!(held.video.len(), 1);
        assert_eq!(
            (held.video[0].first.as_str(), held.video[0].last.as_str()),
            ("01", "02")
        );
        assert_eq!(held.subtitle.len(), 1);
        assert_eq!(held.coverage, SubtitleCoverage::Some);
        // The latest known time of any season; an unknown one never beats a known one.
        assert_eq!(held.video_added_at, Some(20));
        assert_eq!(held.subtitle_added_at, Some(30));
    }

    #[test]
    fn coverage_is_all_some_or_none_for_the_episodes_with_a_video() {
        let v = |e: &str| media(1, e, FileKind::Video, None);
        let s = |e: &str| media(1, e, FileKind::Subtitle, None);
        let coverage = |files: &[Media]| holdings(Some(1), files).coverage;
        assert_eq!(
            coverage(&[v("1"), v("2"), s("1"), s("2")]),
            SubtitleCoverage::All
        );
        // `01` and `1` are one episode.
        assert_eq!(coverage(&[v("01"), s("1")]), SubtitleCoverage::All);
        // A subtitle of an episode without a video does not hide the missing one.
        assert_eq!(
            coverage(&[v("1"), v("2"), s("1"), s("3")]),
            SubtitleCoverage::Some
        );
        assert_eq!(coverage(&[v("1"), v("2"), s("2")]), SubtitleCoverage::Some);
        assert_eq!(coverage(&[v("1"), v("2")]), SubtitleCoverage::None);
        // Subtitles but no video at all is not "all".
        assert_eq!(coverage(&[s("1")]), SubtitleCoverage::Some);
        assert_eq!(coverage(&[]), SubtitleCoverage::None);
    }

    #[test]
    fn times_are_unknown_only_when_no_file_has_one() {
        let files = [
            media(1, "01", FileKind::Video, None),
            media(1, "02", FileKind::Video, None),
        ];
        let held = holdings(Some(1), &files);
        assert_eq!(held.video_added_at, None);
        assert_eq!(held.subtitle_added_at, None);
    }
}
