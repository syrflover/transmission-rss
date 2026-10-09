//! The release range a search starts with (`docs/specs/collection.md`, 지난
//! 회차 검색): `릴리스 13–24 → S02E01–12`, the correspondence between the
//! numbers the releases carry and the work folder's.
//!
//! The grounds are what the app already knows, and the person confirms the
//! range or writes it when the app knows nothing:
//!
//! - the **episode count** of the season by AniList, and the rule's **episode
//!   conversion** (ticket 0024), which says how release numbers move into the
//!   season: a season of 12 episodes whose rule converts by −12 is the releases
//!   13 to 24;
//! - the **first release** the feed showed the rule after it started
//!   (`HistoryStore::first_titles_of_rule`), when the count is not known:
//!   everything before it is past.

use serde::Serialize;

use crate::episode_offset::{leaves_numbers, shift};
use trss_core::episode::signed;

/// What the app knows of the range.
#[derive(Debug, Clone, Default)]
pub struct Grounds {
    /// The rule's episode conversion.
    pub offset: i64,
    /// The season number of the work folder, if the rule's folder names one.
    pub season: Option<u32>,
    /// The season's episode count by AniList.
    pub episodes: Option<u32>,
    /// The lowest whole release number among the rule's first items.
    pub first_release: Option<u32>,
}

/// The range offered, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    /// Release numbers; `None` when there is nothing to offer and the person writes them.
    pub from: Option<u32>,
    pub to: Option<u32>,
    /// A sentence for the screen.
    pub basis: String,
}

/// The release number that lands on the folder episode `folder` by the plain
/// shift of the conversion (`trname` also leaves a number alone when a negative
/// conversion would take it below 1, so the lowest releases land there too).
fn release_of(folder: u32, offset: i64) -> u32 {
    let release = i64::from(folder) - shift(offset);
    u32::try_from(release.max(1)).unwrap_or(u32::MAX)
}

/// Offers a range from the grounds.
pub fn suggest(grounds: &Grounds) -> Suggestion {
    let offset = grounds.offset;
    let first = release_of(1, offset);
    let season = grounds
        .season
        .map_or(String::new(), |s| format!("시즌 {s}의 "));

    if let Some(total) = grounds.episodes.filter(|n| *n > 0) {
        let last = release_of(total, offset);
        let converted = if leaves_numbers(offset) {
            "릴리스 번호가 그대로 시즌 회차예요.".to_owned()
        } else {
            format!("회차 변환 {}이 적용돼요.", signed(offset))
        };
        return Suggestion {
            from: Some(first),
            to: Some(last.max(first)),
            basis: format!(
                "AniList 기준 {season}회차는 1–{total}화예요. {converted} 릴리스 {first}–{}화가 이 범위예요.",
                last.max(first)
            ),
        };
    }
    if let Some(seen) = grounds.first_release {
        if seen > first {
            return Suggestion {
                from: Some(first),
                to: Some(seen - 1),
                basis: format!(
                    "AniList의 회차 수를 몰라요. 이 규칙이 처음 본 릴리스가 {seen}화라서, 그 앞의 {first}–{}화를 제안해요.",
                    seen - 1
                ),
            };
        }
        return Suggestion {
            from: None,
            to: None,
            basis: format!(
                "이 규칙이 처음 본 릴리스가 {seen}화라서 그 앞에 받을 회차가 없어 보여요. 범위를 직접 적을 수 있어요."
            ),
        };
    }
    Suggestion {
        from: None,
        to: None,
        basis: "AniList의 회차 수도, 이 규칙이 처음 본 릴리스도 몰라서 범위를 제안할 수 없어요. 릴리스 회차의 범위를 직접 적어 주세요."
            .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grounds(offset: i64, episodes: Option<u32>, first: Option<u32>) -> Grounds {
        Grounds {
            offset,
            season: Some(2),
            episodes,
            first_release: first,
        }
    }

    #[test]
    fn the_count_and_the_conversion_make_the_range() {
        let s = suggest(&grounds(-12, Some(12), None));
        assert_eq!((s.from, s.to), (Some(13), Some(24)));
        assert!(
            s.basis.contains("1–12화") && s.basis.contains("−12"),
            "{}",
            s.basis
        );
        assert!(s.basis.contains("13–24"), "{}", s.basis);
    }

    #[test]
    fn numbers_that_are_the_seasons_own_need_no_conversion() {
        for offset in [0, 1] {
            let s = suggest(&grounds(offset, Some(12), None));
            assert_eq!((s.from, s.to), (Some(1), Some(12)), "{offset}");
        }
    }

    #[test]
    fn a_positive_conversion_moves_the_start() {
        // The release's 1 is the folder's 13: folder 1–12 cannot be reached.
        let s = suggest(&grounds(13, Some(24), None));
        assert_eq!(s.from, Some(1));
    }

    #[test]
    fn the_first_release_seen_bounds_the_range_when_the_count_is_unknown() {
        let s = suggest(&grounds(0, None, Some(7)));
        assert_eq!((s.from, s.to), (Some(1), Some(6)));
        assert!(s.basis.contains("7화"), "{}", s.basis);
        let converted = suggest(&grounds(-12, None, Some(19)));
        assert_eq!((converted.from, converted.to), (Some(13), Some(18)));
    }

    #[test]
    fn the_count_wins_over_the_first_release() {
        let s = suggest(&grounds(0, Some(12), Some(7)));
        assert_eq!((s.from, s.to), (Some(1), Some(12)));
    }

    #[test]
    fn a_first_release_at_the_start_leaves_nothing_before_it() {
        let s = suggest(&grounds(0, None, Some(1)));
        assert_eq!((s.from, s.to), (None, None));
        assert!(s.basis.contains("앞에 받을 회차가 없어"), "{}", s.basis);
    }

    #[test]
    fn without_grounds_the_person_writes_the_range() {
        let s = suggest(&grounds(0, None, None));
        assert_eq!((s.from, s.to), (None, None));
        assert!(s.basis.contains("직접 적어"), "{}", s.basis);
        // A count of zero is no count.
        assert_eq!(suggest(&grounds(0, Some(0), None)).from, None);
    }
}
