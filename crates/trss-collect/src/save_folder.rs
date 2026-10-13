//! The save folder of a subscription or a rule (`docs/specs/collection.md`,
//! 방영작 구독): a work folder and a season, `<work>/Season NN`, and how both
//! are suggested from a release's work or a work of the library.
//!
//! A release's work is read for a season marker (`S2`, `Season 2`,
//! `2nd Season`, `2기`, `第2期`), which names the season and leaves the work's
//! name; a cour (`Part 2`, `2쿨`) is not a season. The name is the library
//! work whose folder name is the same once both are cleaned the same way and
//! compared as [`trss_anilist::title::normalize`] compares titles, among the
//! works of the collect folder and then the archive folder; else it is a new
//! work.
//!
//! Without a marker, a new work starts at season 1, and a library work follows
//! how AniList splits it: the season after its last one, unless that season
//! holds this airing already, by its Anissia link or by the dates of its last
//! AniList entry. Whether the release's numbering goes on from the season
//! before is not looked at: the episode offset maps it.

use std::cmp::Ordering;
use std::collections::HashSet;

use trss_anilist::{title::normalize, FuzzyDate};
use trss_anissia::calendar::PartialDate;
use trss_core::calendar::{civil_from_days, date_text};
use trss_library::store::{
    library::{FolderWork, LibraryError, LibraryStore},
    seasons::{SeasonError, SeasonStore},
};

/// The longest folder name taken, in bytes, so a name stays well inside what
/// file systems allow.
pub const MAX_FOLDER_BYTES: usize = 120;

/// The highest season number a marker is read as.
const MAX_SEASON: u32 = 99;

/// `<work>/Season NN`, the layout `trname` reads seasons from.
pub fn directory(work: &str, season: u32) -> String {
    format!("{work}/Season {season:02}")
}

/// `text` with the characters a folder name cannot hold (`/ \ : * ? " < > |`
/// and control characters) made spaces, and every run of white space one
/// space.
fn clean(text: &str) -> String {
    let mapped: String = text
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A cleaned name cut to [`MAX_FOLDER_BYTES`] and without dots or spaces at
/// either end; `None` when nothing is left.
fn fit(cleaned: &str) -> Option<String> {
    let mut name = String::new();
    for c in cleaned.chars() {
        if name.len() + c.len_utf8() > MAX_FOLDER_BYTES {
            break;
        }
        name.push(c);
    }
    let name = name.trim_matches(|c: char| c == '.' || c.is_whitespace());
    (!name.is_empty()).then(|| name.to_owned())
}

/// The folder name `text` gives: [`clean`], then [`fit`].
pub fn folder_name(text: &str) -> Option<String> {
    fit(&clean(text))
}

/// A season marker as the release's work writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// The marker's words, as the cleaned work has them.
    pub text: String,
    pub season: u32,
}

/// A release's work read for a save folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseWork {
    /// The work's folder name: the words before the marker (after it when
    /// nothing comes before), cleaned.
    pub name: String,
    pub marker: Option<Marker>,
}

/// A season number of one or two ASCII digits, from 1 to [`MAX_SEASON`].
fn season_number(text: &str) -> Option<u32> {
    if !(1..=2).contains(&text.len()) || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok().filter(|n| (1..=MAX_SEASON).contains(n))
}

/// `2nd`, `3rd`, `4th`, `1st`, in any case.
fn ordinal(text: &str) -> Option<u32> {
    let split = text.len().checked_sub(2)?;
    if !text.is_char_boundary(split) {
        return None;
    }
    let (digits, suffix) = text.split_at(split);
    ["st", "nd", "rd", "th"]
        .iter()
        .any(|s| suffix.eq_ignore_ascii_case(s))
        .then(|| season_number(digits))
        .flatten()
}

/// A word without the brackets or punctuation around it.
fn bare(word: &str) -> &str {
    word.trim_matches(|c: char| {
        matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | ',' | '.' | '-' | '~')
    })
}

/// The marker that starts at `words[at]`: how many words it takes and its
/// season.
fn marker_at(words: &[&str], at: usize) -> Option<(usize, u32)> {
    let word = bare(words[at]);
    let next = words.get(at + 1).map(|w| bare(w));
    // `S2`, `S02`
    if let Some(n) = word.strip_prefix(['S', 's']).and_then(season_number) {
        return Some((1, n));
    }
    // `Season 2`, `Season2`
    if word.len() >= 6 && word.is_char_boundary(6) && word[..6].eq_ignore_ascii_case("season") {
        let rest = &word[6..];
        if rest.is_empty() {
            return next.and_then(season_number).map(|n| (2, n));
        }
        return season_number(rest).map(|n| (1, n));
    }
    // `2nd Season`
    if next.is_some_and(|n| n.eq_ignore_ascii_case("season")) {
        if let Some(n) = ordinal(word) {
            return Some((2, n));
        }
    }
    // `2기`
    if let Some(n) = word.strip_suffix('기').and_then(season_number) {
        return Some((1, n));
    }
    // `第2期`
    word.strip_prefix('第')
        .and_then(|w| w.strip_suffix('期'))
        .and_then(season_number)
        .map(|n| (1, n))
}

/// The words joined, without the separators a title puts before a marker.
fn joined(words: &[&str]) -> String {
    words
        .join(" ")
        .trim_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '–' | '—' | '~' | ',' | '.'))
        .to_owned()
}

/// Reads a release's work (as [`crate::release_name`] gives it) for its
/// season marker, the first one in it. `None` when no folder name is left.
pub fn read_release_work(work: &str) -> Option<ReleaseWork> {
    let cleaned = clean(work);
    let words: Vec<&str> = cleaned.split(' ').filter(|w| !w.is_empty()).collect();
    for at in 0..words.len() {
        let Some((len, season)) = marker_at(&words, at) else {
            continue;
        };
        let before = joined(&words[..at]);
        let name = if before.is_empty() {
            joined(&words[at + len..])
        } else {
            before
        };
        return fit(&name).map(|name| ReleaseWork {
            name,
            marker: Some(Marker {
                text: words[at..at + len].join(" "),
                season,
            }),
        });
    }
    fit(&cleaned).map(|name| ReleaseWork { name, marker: None })
}

/// The day an airing is placed at: the anime's start date as Anissia gives
/// it, or, without one (a rule that is no subscription), today in Seoul.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Airing {
    Start(PartialDate),
    Today(i64),
}

impl Airing {
    /// The airing's start date when it has a readable one, else `today`.
    pub fn of(start_date: Option<&str>, today: i64) -> Airing {
        start_date
            .and_then(PartialDate::parse)
            .map_or(Airing::Today(today), Airing::Start)
    }

    /// Year, month and day, as far as they are known.
    fn parts(self) -> (i32, Option<u32>, Option<u32>) {
        match self {
            Airing::Start(PartialDate::Month(year, month)) => (year, Some(month), None),
            Airing::Start(PartialDate::Day(day)) | Airing::Today(day) => {
                let (year, month, date) = civil_from_days(day);
                (year, Some(month), Some(date))
            }
        }
    }

    /// `방영 시작일(2025-01-04)` or `오늘(2026-10-13)`: the particle after it
    /// follows the word, not how the date is read.
    fn label(self) -> String {
        match self {
            Airing::Start(PartialDate::Day(day)) => format!("방영 시작일({})", date_text(day)),
            Airing::Start(PartialDate::Month(year, month)) => {
                format!("방영 시작일({year:04}-{month:02})")
            }
            Airing::Today(day) => format!("오늘({})", date_text(day)),
        }
    }
}

/// When an AniList entry airs, as AniList knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: FuzzyDate,
    pub end: FuzzyDate,
}

/// Year, month and day of an AniList date, as far as they are known; `None`
/// when the year is not.
fn fuzzy_parts(date: FuzzyDate) -> Option<(i32, Option<u32>, Option<u32>)> {
    let year = date.year?;
    let month = date.month;
    Some((year, month, month.and(date.day)))
}

/// Two dates compared as far as both know them: a month or day one of them
/// does not know does not tell them apart.
fn compare(a: (i32, Option<u32>, Option<u32>), b: (i32, Option<u32>, Option<u32>)) -> Ordering {
    let both = |x: Option<u32>, y: Option<u32>| match (x, y) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => Ordering::Equal,
    };
    a.0.cmp(&b.0).then_with(|| both(a.1, b.1)).then_with(|| {
        if a.1.is_some() && b.1.is_some() {
            both(a.2, b.2)
        } else {
            Ordering::Equal
        }
    })
}

fn fuzzy_text(date: FuzzyDate) -> Option<String> {
    let (year, month, day) = fuzzy_parts(date)?;
    Some(match (month, day) {
        (Some(m), Some(d)) => format!("{year:04}-{m:02}-{d:02}"),
        (Some(m), None) => format!("{year:04}-{m:02}"),
        _ => format!("{year:04}"),
    })
}

impl Span {
    /// Whether the airing falls in the span: from the start on, which must be
    /// known, to the end, which is open when it is not known yet.
    pub fn holds(self, airing: Airing) -> bool {
        let day = airing.parts();
        let Some(start) = fuzzy_parts(self.start) else {
            return false;
        };
        compare(start, day) != Ordering::Greater
            && fuzzy_parts(self.end).is_none_or(|end| compare(day, end) != Ordering::Greater)
    }

    /// `2024-07-06 ~ 2024-09-28`, `2024-07-06 ~` while the end is not known.
    fn text(self) -> String {
        match (fuzzy_text(self.start), fuzzy_text(self.end)) {
            (Some(start), Some(end)) => format!("{start} ~ {end}"),
            (Some(start), None) => format!("{start} ~"),
            (None, _) => "시작일 모름".to_owned(),
        }
    }
}

/// What the season of a library work is suggested from: its highest-numbered
/// recorded season from 1 on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastSeason {
    pub number: u32,
    /// The season is linked to the Anissia anime the folder is for.
    pub holds_anime: bool,
    /// The last of the AniList entries linked to the season, by their order.
    pub last_entry: Option<Span>,
}

/// Why the season was suggested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// The release's work names it.
    Marker(String),
    /// The work is not in the library: season 1.
    NewWork,
    /// The library work records no season from 1 on: season 1.
    NoSeason,
    /// The last season is linked to the anime already: that season.
    Linked,
    /// The last season's last entry holds the airing: that season.
    Within(Span),
    /// The last season's last entry does not hold it: the next season.
    After(Span),
    /// No entry is linked to the last season: the next season.
    NoEntry,
}

/// The season a library work's folder is suggested with, and why.
pub fn season_of(last: Option<LastSeason>, airing: Airing) -> (u32, Why) {
    let Some(last) = last else {
        return (1, Why::NoSeason);
    };
    let next = last.number.saturating_add(1);
    if last.holds_anime {
        return (last.number, Why::Linked);
    }
    match last.last_entry {
        None => (next, Why::NoEntry),
        Some(span) if span.holds(airing) => (last.number, Why::Within(span)),
        Some(span) => (next, Why::After(span)),
    }
}

/// A suggested save folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The work's folder name: the library work's own when there is one.
    pub work: String,
    pub season: u32,
    /// The library work the folder is, when it is one.
    pub library: Option<FolderWork>,
    pub why: Why,
    pub airing: Airing,
}

impl Suggestion {
    pub fn directory(&self) -> String {
        directory(&self.work, self.season)
    }

    /// The sentence that tells the person why the season is what it is.
    pub fn message(&self) -> String {
        let airing = self.airing.label();
        match &self.why {
            Why::Marker(marker) => format!("릴리스 제목의 ‘{marker}’ 표시를 시즌 번호로 읽었어요."),
            Why::NewWork => "수집 폴더와 보관 폴더에 없는 작품이라 시즌 1로 제안해요.".to_owned(),
            Why::NoSeason => "이 작품에는 시즌 폴더 기록이 없어서 시즌 1로 제안해요.".to_owned(),
            Why::Linked => {
                "마지막 시즌이 이미 이 Anissia 작품에 이어져 있어서 같은 시즌으로 제안해요."
                    .to_owned()
            }
            Why::Within(span) => format!(
                "마지막 시즌의 AniList 항목({})이 {airing}을 담아서 같은 시즌으로 제안해요.",
                span.text()
            ),
            Why::After(span) => format!(
                "마지막 시즌의 AniList 항목({})이 {airing}을 담지 않아서 다음 시즌으로 제안해요.",
                span.text()
            ),
            Why::NoEntry => {
                "마지막 시즌에 이은 AniList 항목이 없어서 다음 시즌으로 제안해요.".to_owned()
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SuggestError {
    #[error("cannot read the library: {0}")]
    Library(#[from] LibraryError),
    #[error("cannot read the season info: {0}")]
    Seasons(#[from] SeasonError),
}

/// What a save folder is suggested from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked<'a> {
    /// A release's work, as [`crate::release_name`] gives it.
    Release(&'a str),
    /// A work folder the person picked or typed: its name as it is.
    Work(&'a str),
}

/// The works a save folder's work is chosen from: those of `folders` (the
/// collect folder, then the archive folder, as registered), a name in two of
/// them listed once, from the first.
pub async fn folder_works(
    library: &LibraryStore,
    folders: &[String],
) -> Result<Vec<FolderWork>, LibraryError> {
    let works = library.works_in_folders(folders.to_vec()).await?;
    let mut seen = HashSet::new();
    Ok(works
        .into_iter()
        .filter(|w| seen.insert(w.dir_name.clone()))
        .collect())
}

async fn last_season(
    seasons: &SeasonStore,
    work: &FolderWork,
    anime_no: Option<i64>,
) -> Result<Option<LastSeason>, SeasonError> {
    let Some(number) = work.last_season else {
        return Ok(None);
    };
    let holds_anime = match anime_no {
        Some(no) => seasons.anissia_link(&work.id, number).await?.anime_no == Some(no),
        None => false,
    };
    let link = seasons
        .links_of_seasons(vec![(work.id.clone(), number)])
        .await?
        .into_iter()
        .next()
        .flatten();
    let last_entry = link.and_then(|l| {
        l.entries.last().map(|e| Span {
            start: e.start,
            end: e.end,
        })
    });
    Ok(Some(LastSeason {
        number,
        holds_anime,
        last_entry,
    }))
}

/// The save folder suggested for `asked`, among the works of `folders`
/// ([`folder_works`]). `anime_no` is the Anissia anime the folder is for, when
/// there is one, and `airing` when it airs. `None` when no folder name is left
/// of what was asked.
pub async fn suggest(
    library: &LibraryStore,
    seasons: &SeasonStore,
    folders: &[String],
    asked: Asked<'_>,
    anime_no: Option<i64>,
    airing: Airing,
) -> Result<Option<Suggestion>, SuggestError> {
    let works = folder_works(library, folders).await?;
    let (name, marker, found) = match asked {
        Asked::Release(work) => {
            let Some(read) = read_release_work(work) else {
                return Ok(None);
            };
            let key = normalize(&read.name);
            let found = works
                .into_iter()
                .find(|w| folder_name(&w.dir_name).is_some_and(|n| normalize(&n) == key));
            (read.name, read.marker, found)
        }
        Asked::Work(typed) => {
            let typed = typed.trim();
            let found = works.into_iter().find(|w| w.dir_name == typed);
            let Some(name) = found
                .as_ref()
                .map(|w| w.dir_name.clone())
                .or_else(|| folder_name(typed))
            else {
                return Ok(None);
            };
            (name, None, found)
        }
    };
    let work = found.as_ref().map_or(name, |w| w.dir_name.clone());
    let (season, why) = match (marker, &found) {
        (Some(marker), _) => (marker.season, Why::Marker(marker.text)),
        (None, None) => (1, Why::NewWork),
        (None, Some(w)) => season_of(last_season(seasons, w, anime_no).await?, airing),
    };
    Ok(Some(Suggestion {
        work,
        season,
        library: found,
        why,
        airing,
    }))
}

#[cfg(test)]
mod tests;
