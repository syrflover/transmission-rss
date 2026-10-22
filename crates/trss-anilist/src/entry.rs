//! What AniList says about one anime entry: the models `season` reads from its
//! answer and the season info of `trss-library` keeps.

use serde::{Deserialize, Serialize};
use trss_core::Millis;

/// AniList's date, any part of which may be unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FuzzyDate {
    pub year: Option<i32>,
    pub month: Option<u32>,
    pub day: Option<u32>,
}

impl FuzzyDate {
    pub fn is_known(&self) -> bool {
        self.year.is_some()
    }

    /// A date written `YYYY-MM-DD`, `YYYY-MM` or `YYYY` (as Anissia writes its
    /// start dates); a part that cannot be read is unknown, and so is all that
    /// follows it.
    pub fn from_text(text: &str) -> FuzzyDate {
        let mut parts = text.trim().splitn(3, '-');
        let year = parts
            .next()
            .and_then(|y| y.parse::<i32>().ok())
            .filter(|y| (1800..=2300).contains(y));
        let month = year.and(
            parts
                .next()
                .and_then(|m| m.parse::<u32>().ok())
                .filter(|m| (1..=12).contains(m)),
        );
        let day = month.and(
            parts
                .next()
                .and_then(|d| d.parse::<u32>().ok())
                .filter(|d| (1..=31).contains(d)),
        );
        FuzzyDate { year, month, day }
    }
}

/// An entry AniList relates to another as its sequel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sequel {
    pub id: i64,
    pub romaji: Option<String>,
    pub english: Option<String>,
    pub native: Option<String>,
    pub format: Option<String>,
    pub status: Option<String>,
    pub start: FuzzyDate,
}

/// One scheduled airing of an episode of an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Airing {
    pub episode: u32,
    /// Unix seconds, as AniList gives it.
    pub at: i64,
}

/// What AniList said about one anime entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// AniList's `Media` ID.
    pub id: i64,
    pub romaji: Option<String>,
    pub english: Option<String>,
    pub native: Option<String>,
    /// `TV`, `TV_SHORT`, `MOVIE`, `OVA`, ...
    pub format: Option<String>,
    /// `FINISHED`, `RELEASING`, `NOT_YET_RELEASED`, `CANCELLED`, `HIATUS`.
    pub status: Option<String>,
    pub episodes: Option<u32>,
    pub start: FuzzyDate,
    pub end: FuzzyDate,
    /// The main studios AniList flags as animation studios, in its order.
    pub studios: Vec<String>,
    pub genres: Vec<String>,
    /// The entry's AniList synonyms that contain Hangul, in AniList's order.
    pub korean_titles: Vec<String>,
    /// AniList's text, as it came (see `trss_library::seasons::describe`).
    pub description: Option<String>,
    /// The per-episode schedule, as far as AniList has one.
    pub airing: Vec<Airing>,
    pub sequels: Vec<Sequel>,
    /// When the row was last received.
    pub fetched_at: Millis,
}

impl Entry {
    pub fn is_releasing(&self) -> bool {
        self.status.as_deref() == Some("RELEASING")
    }

    /// Whether the entry is still to air or airing, so it is refreshed daily.
    pub fn is_active(&self) -> bool {
        matches!(
            self.status.as_deref(),
            Some("RELEASING" | "NOT_YET_RELEASED")
        )
    }

    /// The native title, or the first title there is.
    pub fn display_title(&self) -> &str {
        self.english
            .as_deref()
            .or(self.romaji.as_deref())
            .or(self.native.as_deref())
            .unwrap_or("")
    }

    /// Every title the entry has, for searching.
    pub fn titles(&self) -> impl Iterator<Item = &str> {
        [&self.native, &self.english, &self.romaji]
            .into_iter()
            .flatten()
            .map(String::as_str)
    }
}
