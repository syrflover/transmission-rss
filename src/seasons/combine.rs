//! What a season shows out of its linked entries (`docs/specs/library.md`, 시즌
//! 정보): one entry's values as they are, several in order as one.
//!
//! - **Airing** runs from the first entry's start to the last entry's end. It
//!   is *airing now* while any entry is releasing.
//! - **Episodes** are the sum of the entries'; one entry that does not know its
//!   count makes the whole unknown.
//! - **Studios** and **genres** are the entries' lists joined in order without
//!   repeating one.
//! - **Episode air dates** come only from an entry that is releasing and has a
//!   per-episode schedule. An entry's episodes follow the earlier entries' in
//!   the season (a Part 2 of 13 episodes after a Part 1 of 12 starts at 13),
//!   which is known only while every earlier entry knows its episode count.

use std::collections::BTreeMap;

use crate::store::seasons::{Entry, FuzzyDate, Sequel};

/// The values of a season's linked entries taken together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combined {
    pub start: FuzzyDate,
    pub end: FuzzyDate,
    /// Where the season stands: `releasing` while any entry is releasing,
    /// otherwise the last entry's status in lower case (`finished`,
    /// `not_yet_released`, `cancelled`, `hiatus`), `None` when unknown.
    pub state: Option<String>,
    pub episodes: Option<u32>,
    pub studios: Vec<String>,
    pub genres: Vec<String>,
}

/// `entries` taken together; `None` for no entry (nothing is known).
pub fn combine(entries: &[Entry]) -> Option<Combined> {
    let first = entries.first()?;
    let last = entries.last()?;
    let episodes = entries
        .iter()
        .try_fold(0u32, |sum, e| e.episodes.and_then(|n| sum.checked_add(n)));
    let state = if entries.iter().any(Entry::is_releasing) {
        Some("releasing".to_owned())
    } else {
        last.status.as_ref().map(|s| s.to_ascii_lowercase())
    };
    let mut studios: Vec<String> = Vec::new();
    let mut genres: Vec<String> = Vec::new();
    for entry in entries {
        for studio in &entry.studios {
            if !studios.contains(studio) {
                studios.push(studio.clone());
            }
        }
        for genre in &entry.genres {
            if !genres.contains(genre) {
                genres.push(genre.clone());
            }
        }
    }
    Some(Combined {
        start: first.start,
        end: last.end,
        state,
        episodes,
        studios,
        genres,
    })
}

/// The scheduled airing time (Unix milliseconds) of each episode of a season
/// whose schedule is known, by the episode's number in the season.
pub fn air_times(entries: &[Entry]) -> BTreeMap<u32, i64> {
    let mut times = BTreeMap::new();
    let mut offset = 0u32;
    for entry in entries {
        if entry.is_releasing() {
            for airing in &entry.airing {
                if let Some(episode) = offset.checked_add(airing.episode) {
                    times.insert(episode, airing.at.saturating_mul(1000));
                }
            }
        }
        // The entries after this one start where it ends, if it says where.
        match entry.episodes.and_then(|n| offset.checked_add(n)) {
            Some(next) => offset = next,
            None => break,
        }
    }
    times
}

/// The entries to offer for a season that has none: the sequels of the last
/// entry of the season before it, when that has any.
pub fn suggestions(previous: Option<&[Entry]>, this: &[Entry]) -> Vec<Sequel> {
    if !this.is_empty() {
        return Vec::new();
    }
    previous
        .and_then(<[Entry]>::last)
        .map(|entry| entry.sequels.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::seasons::Airing;

    fn entry(id: i64) -> Entry {
        Entry {
            id,
            romaji: None,
            english: None,
            native: None,
            format: Some("TV".into()),
            status: Some("FINISHED".into()),
            episodes: None,
            start: FuzzyDate::default(),
            end: FuzzyDate::default(),
            studios: Vec::new(),
            genres: Vec::new(),
            description: None,
            airing: Vec::new(),
            sequels: Vec::new(),
            fetched_at: 0,
        }
    }

    fn date(year: i32, month: u32) -> FuzzyDate {
        FuzzyDate {
            year: Some(year),
            month: Some(month),
            day: None,
        }
    }

    #[test]
    fn two_parts_are_one_season() {
        let part1 = Entry {
            episodes: Some(12),
            start: date(2022, 4),
            end: date(2022, 6),
            studios: vec!["A".into(), "B".into()],
            genres: vec!["Action".into(), "Drama".into()],
            ..entry(1)
        };
        let part2 = Entry {
            episodes: Some(13),
            start: date(2022, 7),
            end: date(2022, 9),
            studios: vec!["B".into(), "C".into()],
            genres: vec!["Drama".into(), "Comedy".into()],
            ..entry(2)
        };
        let both = combine(&[part1.clone(), part2.clone()]).unwrap();
        assert_eq!(both.episodes, Some(25));
        assert_eq!((both.start, both.end), (date(2022, 4), date(2022, 9)));
        assert_eq!(both.studios, ["A", "B", "C"]);
        assert_eq!(both.genres, ["Action", "Drama", "Comedy"]);
        assert_eq!(both.state.as_deref(), Some("finished"));

        // One entry with an unknown count makes the amount unknown.
        let unknown = Entry {
            episodes: None,
            ..part2
        };
        assert_eq!(combine(&[part1.clone(), unknown]).unwrap().episodes, None);
        // One entry is its own values; none is nothing.
        assert_eq!(combine(&[part1]).unwrap().episodes, Some(12));
        assert_eq!(combine(&[]), None);
    }

    #[test]
    fn a_releasing_entry_makes_the_season_airing() {
        let done = Entry {
            episodes: Some(12),
            ..entry(1)
        };
        let airing = Entry {
            status: Some("RELEASING".into()),
            ..entry(2)
        };
        assert_eq!(
            combine(&[done.clone(), airing]).unwrap().state.as_deref(),
            Some("releasing")
        );
        let upcoming = Entry {
            status: Some("NOT_YET_RELEASED".into()),
            ..entry(3)
        };
        assert_eq!(
            combine(&[done, upcoming]).unwrap().state.as_deref(),
            Some("not_yet_released")
        );
        let unknown = Entry {
            status: None,
            ..entry(4)
        };
        assert_eq!(combine(&[unknown]).unwrap().state, None);
    }

    #[test]
    fn air_times_come_only_from_a_releasing_entry_after_known_counts() {
        let schedule = |episodes: &[(u32, i64)]| -> Vec<Airing> {
            episodes
                .iter()
                .map(|(episode, at)| Airing {
                    episode: *episode,
                    at: *at,
                })
                .collect()
        };
        let finished = Entry {
            episodes: Some(12),
            airing: schedule(&[(1, 10)]),
            ..entry(1)
        };
        let releasing = Entry {
            status: Some("RELEASING".into()),
            airing: schedule(&[(1, 100), (2, 200)]),
            ..entry(2)
        };
        // A finished entry's schedule is not used; the second entry's episodes follow the first's 12.
        let times = air_times(&[finished.clone(), releasing.clone()]);
        assert_eq!(
            times.into_iter().collect::<Vec<_>>(),
            [(13, 100_000), (14, 200_000)]
        );
        // Alone, it starts at 1.
        assert_eq!(
            air_times(std::slice::from_ref(&releasing))
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [1, 2]
        );
        // An earlier entry that does not know its count leaves the later episodes without a date.
        let unknown = Entry {
            episodes: None,
            ..finished
        };
        assert!(air_times(&[unknown, releasing]).is_empty());
        assert!(air_times(&[]).is_empty());
    }

    #[test]
    fn only_a_season_without_entries_gets_the_previous_seasons_sequels() {
        let sequel = Sequel {
            id: 9,
            romaji: Some("Next".into()),
            english: None,
            native: None,
            format: Some("TV".into()),
            status: None,
            start: FuzzyDate::default(),
        };
        let earlier = Entry {
            sequels: vec![Sequel {
                id: 5,
                ..sequel.clone()
            }],
            ..entry(1)
        };
        let last = Entry {
            sequels: vec![sequel.clone()],
            ..entry(2)
        };
        let previous = [earlier, last];
        assert_eq!(suggestions(Some(&previous), &[]), [sequel]);
        assert!(suggestions(Some(&previous), &[entry(9)]).is_empty());
        assert!(suggestions(None, &[]).is_empty());
        assert!(suggestions(Some(&[]), &[]).is_empty());
    }
}
