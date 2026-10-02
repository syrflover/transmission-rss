//! One page of the library list: the sort, the filter and the search, and the
//! cursor that continues after the last item a client received.
//!
//! The summaries of every work are computed ([`super::overview`]) and the page
//! is cut out of them in memory. A sort or a filter by what a work holds (the
//! latest video or subtitle time, the subtitle coverage) needs every work's
//! summary anyway, and for the library's size (500 works, a few milliseconds)
//! that is cheaper than keeping a second copy of the summaries in SQL.
//!
//! - **Order.** A sort is a key of three parts: the sort's time (latest first,
//!   an unknown time after every known one), the title, and the work's ID.
//!   Title and ID make the key unique, so two works are never in doubt about
//!   which comes first. The sort by title has no time, so it is the title
//!   order; the sort by airing year has the year the latest season's first
//!   linked AniList entry started, latest first and unknown after every known one.
//! - **Title** is compared as NFC text without regard to case: a folder name
//!   made on macOS may be decomposed, and `a` and `A` are the same letter to a
//!   reader. Code point order after that, so digits come before Latin letters
//!   and those before Hangul.
//! - **Cursor.** It is the key of the last item of a page, so the next page is
//!   what comes *after that key*, whatever was added or removed meanwhile: a
//!   work inserted ahead of the cursor is not shown (the client has passed that
//!   place), one inserted after it is, and nothing the client has seen comes
//!   twice. Only a work whose own key changes between two pages (a new file
//!   moved its time) can be seen twice or missed, and the next first page
//!   shows it where it belongs now.
//! - **Search** is a case-insensitive match of the NFC text in the work's
//!   title (the folder name) or in any native, English or romaji title of the
//!   AniList entries linked to its seasons.

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::store::library::overview::{SubtitleCoverage, WorkOverview};

/// How a page is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Title,
    /// The latest season's first linked entry's start year, latest first.
    Year,
    /// Latest added work first.
    Added,
    /// The latest video added first.
    Video,
    /// The latest subtitle added first.
    Subtitle,
}

impl Sort {
    pub const ALL: [Sort; 5] = [
        Sort::Title,
        Sort::Year,
        Sort::Added,
        Sort::Video,
        Sort::Subtitle,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Sort::Title => "title",
            Sort::Year => "year",
            Sort::Added => "added",
            Sort::Video => "video",
            Sort::Subtitle => "subtitle",
        }
    }

    pub fn from_code(code: &str) -> Option<Sort> {
        Sort::ALL.into_iter().find(|s| s.code() == code)
    }

    /// The time this sort looks at, `None` for an unknown one.
    fn time_of(self, work: &WorkOverview) -> Option<i64> {
        match self {
            Sort::Title => None,
            Sort::Year => work.airing_year.map(i64::from),
            Sort::Added => work.first_seen_at,
            Sort::Video => work.video_added_at,
            Sort::Subtitle => work.subtitle_added_at,
        }
    }
}

/// Which works are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    /// An entry linked to the latest season is airing now.
    Airing,
    /// Subtitles for every episode that has a video.
    Complete,
    Partial,
    None,
    /// A subtitle file that could not be placed, or a work whose folder is gone.
    Check,
}

impl Filter {
    pub const ALL: [Filter; 6] = [
        Filter::All,
        Filter::Airing,
        Filter::Complete,
        Filter::Partial,
        Filter::None,
        Filter::Check,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Airing => "airing",
            Filter::Complete => "complete",
            Filter::Partial => "partial",
            Filter::None => "none",
            Filter::Check => "check",
        }
    }

    pub fn from_code(code: &str) -> Option<Filter> {
        Filter::ALL.into_iter().find(|f| f.code() == code)
    }

    fn admits(self, work: &WorkOverview) -> bool {
        match self {
            Filter::All => true,
            Filter::Airing => work.airing,
            Filter::Complete => work.subtitle_coverage == Some(SubtitleCoverage::All),
            Filter::Partial => work.subtitle_coverage == Some(SubtitleCoverage::Some),
            Filter::None => work.subtitle_coverage == Some(SubtitleCoverage::None),
            Filter::Check => work.subtitle_check_needed || work.missing,
        }
    }
}

/// The text a title is compared and searched as: NFC, lower case.
fn fold(text: &str) -> String {
    text.nfc().collect::<String>().to_lowercase()
}

/// A place in an ordered list. Two keys of different works are never equal.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    time: Option<i64>,
    title: String,
    id: String,
}

impl Ord for Key {
    fn cmp(&self, other: &Key) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        let by_time = match (self.time, other.time) {
            // Latest first, so the larger time is the smaller key.
            (Some(a), Some(b)) => b.cmp(&a),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        by_time
            .then_with(|| self.title.cmp(&other.title))
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Key) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Where the next page starts: after the last item of the previous one.
/// Opaque to clients ([`Cursor::encode`]); it belongs to the sort that made it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    sort: Sort,
    key: Key,
}

/// What a cursor is written as before it is turned into text.
#[derive(Serialize, Deserialize)]
struct CursorBody {
    s: String,
    t: Option<i64>,
    k: String,
    i: String,
}

impl Cursor {
    /// Text safe to put in a query string.
    pub fn encode(&self) -> String {
        let body = CursorBody {
            s: self.sort.code().to_owned(),
            t: self.key.time,
            k: self.key.title.clone(),
            i: self.key.id.clone(),
        };
        let json = serde_json::to_vec(&body).expect("a cursor is plain data");
        json.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// The cursor that `text` stands for, `None` when it is none of ours.
    pub fn decode(text: &str) -> Option<Cursor> {
        if text.is_empty() || !text.len().is_multiple_of(2) || !text.is_ascii() {
            return None;
        }
        let bytes: Option<Vec<u8>> = (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok())
            .collect();
        let body: CursorBody = serde_json::from_slice(&bytes?).ok()?;
        Some(Cursor {
            sort: Sort::from_code(&body.s)?,
            key: Key {
                time: body.t,
                title: body.k,
                id: body.i,
            },
        })
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }
}

/// What the library list is asked for.
#[derive(Debug, Clone)]
pub struct ListQuery {
    pub sort: Sort,
    pub filter: Filter,
    /// Text the title must contain; empty for no search.
    pub search: String,
    pub after: Option<Cursor>,
    /// At least 1.
    pub limit: usize,
}

/// One page of the list.
#[derive(Debug, Clone)]
pub struct Page {
    pub items: Vec<WorkOverview>,
    /// Where the page after this one starts; `None` after the last page.
    pub next: Option<Cursor>,
    /// How many works the filter and the search match, over every page.
    pub total: usize,
    /// How many works the library has, whatever the filter and the search.
    pub library_count: usize,
}

/// The page `query` asks for out of `works` (in any order).
pub fn page(works: Vec<WorkOverview>, query: &ListQuery) -> Page {
    let library_count = works.len();
    let needle = fold(query.search.trim());
    let mut matched: Vec<(Key, WorkOverview)> = works
        .into_iter()
        .filter(|work| query.filter.admits(work))
        .filter_map(|work| {
            let title = fold(&work.dir_name);
            if !needle.is_empty()
                && !title.contains(&needle)
                && !work.linked_titles.iter().any(|t| fold(t).contains(&needle))
            {
                return None;
            }
            let key = Key {
                time: query.sort.time_of(&work),
                title,
                id: work.id.clone(),
            };
            Some((key, work))
        })
        .collect();
    matched.sort_by(|a, b| a.0.cmp(&b.0));
    let total = matched.len();

    let start = match &query.after {
        Some(cursor) => matched.partition_point(|(key, _)| *key <= cursor.key),
        None => 0,
    };
    let end = start.saturating_add(query.limit).min(total);
    let next = (end < total && end > start).then(|| Cursor {
        sort: query.sort,
        key: matched[end - 1].0.clone(),
    });
    let items = matched
        .drain(start..end)
        .map(|(_, work)| work)
        .collect::<Vec<_>>();
    Page {
        items,
        next,
        total,
        library_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work(id: &str, name: &str) -> WorkOverview {
        WorkOverview {
            id: id.into(),
            dir_name: name.into(),
            missing: false,
            watch_folder_id: "f".into(),
            watch_folder_path: "/w".into(),
            first_seen_at: None,
            latest_season: Some(1),
            video: Vec::new(),
            subtitle: Vec::new(),
            subtitle_coverage: Some(SubtitleCoverage::None),
            subtitle_check_needed: false,
            video_added_at: None,
            subtitle_added_at: None,
            airing_year: None,
            airing: false,
            linked_titles: Vec::new(),
        }
    }

    fn timed(
        id: &str,
        name: &str,
        added: Option<i64>,
        video: Option<i64>,
        sub: Option<i64>,
    ) -> WorkOverview {
        WorkOverview {
            first_seen_at: added,
            video_added_at: video,
            subtitle_added_at: sub,
            ..work(id, name)
        }
    }

    fn query(sort: Sort) -> ListQuery {
        ListQuery {
            sort,
            filter: Filter::All,
            search: String::new(),
            after: None,
            limit: 60,
        }
    }

    fn ids(page: &Page) -> Vec<&str> {
        page.items.iter().map(|w| w.id.as_str()).collect()
    }

    /// Works with times that repeat and are missing, and names that tie.
    fn mixed() -> Vec<WorkOverview> {
        let mut works = Vec::new();
        for n in 0..23 {
            let t = |m: i64| (n % m != 0).then_some(1000 + (n % 4) * 10);
            // Two works share each name, so the title alone never settles the order.
            works.push(timed(
                &format!("id{n:02}"),
                &format!("Work {}", n / 2),
                t(3),
                t(5),
                t(2),
            ));
        }
        works
    }

    /// Every page, each asked with the cursor of the one before.
    fn walk(works: &[WorkOverview], base: &ListQuery, limit: usize) -> Vec<Page> {
        let mut pages = Vec::new();
        let mut after = None;
        loop {
            let page = page(
                works.to_vec(),
                &ListQuery {
                    after,
                    limit,
                    ..base.clone()
                },
            );
            after = page.next.clone();
            let done = after.is_none();
            pages.push(page);
            if done {
                return pages;
            }
        }
    }

    #[test]
    fn pages_continue_without_gap_or_repeat_for_every_sort_and_page_size() {
        let works = mixed();
        for sort in Sort::ALL {
            let whole = page(works.clone(), &query(sort));
            assert_eq!(whole.items.len(), works.len());
            assert!(whole.next.is_none());
            for limit in [1, 2, 5, 7, 22, 23, 24] {
                let pages = walk(&works, &query(sort), limit);
                let joined: Vec<&str> = pages.iter().flat_map(ids).collect();
                assert_eq!(joined, ids(&whole), "{sort:?}, {limit} per page");
                assert!(pages.iter().all(|p| p.total == works.len()));
                assert!(pages.iter().all(|p| p.items.len() <= limit));
                // Only the last page has no continuation.
                assert!(pages[..pages.len() - 1].iter().all(|p| p.next.is_some()));
            }
        }
    }

    #[test]
    fn a_time_sort_is_latest_first_with_unknown_after_known_and_title_then_id_on_ties() {
        let works = vec![
            timed("a", "Delta", None, None, None),
            timed("b", "Alpha", None, None, Some(10)),
            timed("c", "Bravo", None, None, Some(20)),
            timed("d", "Charlie", None, None, Some(20)),
            timed("e", "Charlie", None, None, Some(20)),
            timed("f", "Echo", None, None, None),
        ];
        let p = page(works, &query(Sort::Subtitle));
        // 20 (Bravo, then the two Charlies by ID), 10, then the unknown by title.
        assert_eq!(ids(&p), ["c", "d", "e", "b", "a", "f"]);
    }

    #[test]
    fn the_three_time_sorts_look_at_their_own_time() {
        let works = vec![
            timed("a", "A", Some(3), Some(1), Some(2)),
            timed("b", "B", Some(1), Some(2), Some(3)),
            timed("c", "C", Some(2), Some(3), Some(1)),
        ];
        assert_eq!(
            ids(&page(works.clone(), &query(Sort::Added))),
            ["a", "c", "b"]
        );
        assert_eq!(
            ids(&page(works.clone(), &query(Sort::Video))),
            ["c", "b", "a"]
        );
        assert_eq!(ids(&page(works, &query(Sort::Subtitle))), ["b", "a", "c"]);
    }

    #[test]
    fn title_and_year_sort_by_title_and_ignore_times() {
        let works = vec![
            timed("a", "banana", Some(1), None, None),
            timed("b", "Apple", None, None, None),
            timed("c", "Cherry", Some(9), None, None),
            timed("d", "가나다", None, None, None),
            timed("e", "10 Things", None, None, None),
        ];
        for sort in [Sort::Title, Sort::Year] {
            let p = page(works.clone(), &query(sort));
            assert_eq!(ids(&p), ["e", "b", "a", "c", "d"], "{sort:?}");
        }
    }

    #[test]
    fn decomposed_and_composed_titles_are_the_same_title() {
        // `각` as one syllable and as three jamo, with a work of a plain name between.
        let composed = "\u{AC01}";
        let decomposed = "\u{1100}\u{1161}\u{11A8}";
        let works = vec![
            work("b", decomposed),
            work("a", composed),
            work("c", "Zeta"),
        ];
        // The tie of equal titles is settled by ID.
        assert_eq!(ids(&page(works, &query(Sort::Title))), ["c", "a", "b"]);
    }

    #[test]
    fn a_work_added_between_pages_is_never_a_repeat_and_one_ahead_of_the_cursor_is_not_shown() {
        for sort in Sort::ALL {
            let mut works = mixed();
            let first = page(
                works.clone(),
                &ListQuery {
                    limit: 6,
                    ..query(sort)
                },
            );
            let seen: Vec<String> = first.items.iter().map(|w| w.id.clone()).collect();
            let cursor = first.next.clone().expect("more pages");

            // One work that sorts before everything seen, one after.
            works.push(timed(
                "new-first",
                "",
                Some(i64::MAX),
                Some(i64::MAX),
                Some(i64::MAX),
            ));
            works.push(timed("new-last", "~", None, None, None));
            let rest = page(
                works.clone(),
                &ListQuery {
                    after: Some(cursor),
                    limit: 1000,
                    ..query(sort)
                },
            );
            let later: Vec<&str> = ids(&rest);
            assert!(
                later.iter().all(|id| !seen.contains(&id.to_string())),
                "{sort:?}"
            );
            assert!(!later.contains(&"new-first"), "{sort:?}");
            assert!(later.contains(&"new-last"), "{sort:?}");
            // What was left is all there: nothing existing is skipped.
            assert_eq!(seen.len() + later.len(), works.len() - 1, "{sort:?}");
        }
    }

    #[test]
    fn a_work_removed_between_pages_does_not_move_the_cursor() {
        let mut works = mixed();
        let sort = Sort::Subtitle;
        let first = page(
            works.clone(),
            &ListQuery {
                limit: 5,
                ..query(sort)
            },
        );
        let cursor = first.next.clone().unwrap();
        let last_seen = first.items.last().unwrap().id.clone();
        // The work the cursor points at is gone.
        works.retain(|w| w.id != last_seen);
        let rest = page(
            works.clone(),
            &ListQuery {
                after: Some(cursor),
                limit: 1000,
                ..query(sort)
            },
        );
        let seen: Vec<&str> = first.items.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(rest.items.len(), works.len() - (seen.len() - 1));
        assert!(rest.items.iter().all(|w| !seen.contains(&w.id.as_str())));
    }

    #[test]
    fn filters_pick_the_works_their_definitions_name() {
        let mut complete = work("c", "Complete");
        complete.subtitle_coverage = Some(SubtitleCoverage::All);
        let mut partial = work("p", "Partial");
        partial.subtitle_coverage = Some(SubtitleCoverage::Some);
        let none = work("n", "None");
        let mut check = work("k", "Check");
        check.subtitle_check_needed = true;
        let mut missing = work("m", "Missing");
        missing.missing = true;
        missing.subtitle_coverage = None;
        let works = vec![complete, partial, none, check, missing];

        let filtered = |filter| {
            let p = page(
                works.clone(),
                &ListQuery {
                    filter,
                    ..query(Sort::Title)
                },
            );
            (
                ids(&p).iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                p.total,
                p.library_count,
            )
        };
        assert_eq!(filtered(Filter::All).1, 5);
        assert_eq!(filtered(Filter::Complete).0, ["c"]);
        assert_eq!(filtered(Filter::Partial).0, ["p"]);
        // `none` is a subtitle coverage of none: the work with a check badge has it too.
        assert_eq!(filtered(Filter::None).0, ["k", "n"]);
        // A folder that is gone needs a check, and is in no coverage filter.
        assert_eq!(filtered(Filter::Check).0, ["k", "m"]);
        // No season knows when it airs.
        let airing = filtered(Filter::Airing);
        assert!(airing.0.is_empty());
        assert_eq!((airing.1, airing.2), (0, 5));
    }

    #[test]
    fn search_matches_the_title_by_nfc_text_without_case_and_total_counts_the_matches() {
        let works = vec![
            work("a", "Lycoris Recoil"),
            work("b", "[SubsPlease] lycoris special"),
            work("c", "Other"),
            work("d", "\u{1100}\u{1161}\u{11A8} 이야기"),
        ];
        let search = |text: &str| {
            let p = page(
                works.clone(),
                &ListQuery {
                    search: text.into(),
                    ..query(Sort::Title)
                },
            );
            (
                ids(&p).iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                p.total,
                p.library_count,
            )
        };
        assert_eq!(
            search("LYCORIS"),
            (vec!["b".to_string(), "a".to_string()], 2, 4)
        );
        assert_eq!(search("  lycoris  ").1, 2);
        assert_eq!(search("[subs").0, ["b"]);
        // Typed composed, stored decomposed.
        assert_eq!(search("\u{AC01} 이야기").0, ["d"]);
        assert_eq!(search("nothing"), (vec![], 0, 4));
        assert_eq!(search("").1, 4);
    }

    #[test]
    fn a_page_is_cut_at_the_limit_and_an_empty_result_has_no_next() {
        let works = mixed();
        let p = page(
            works.clone(),
            &ListQuery {
                limit: 10,
                ..query(Sort::Title)
            },
        );
        assert_eq!(p.items.len(), 10);
        assert!(p.next.is_some());
        assert_eq!((p.total, p.library_count), (23, 23));
        let none = page(
            works,
            &ListQuery {
                search: "zzz".into(),
                ..query(Sort::Title)
            },
        );
        assert!(none.items.is_empty() && none.next.is_none());
        let empty = page(Vec::new(), &query(Sort::Title));
        assert_eq!(
            (empty.items.len(), empty.total, empty.library_count),
            (0, 0, 0)
        );
    }

    #[test]
    fn a_cursor_round_trips_and_text_that_is_not_one_is_refused() {
        let cursor = Cursor {
            sort: Sort::Video,
            key: Key {
                time: Some(-5),
                title: "가나 \"title\" ünï".into(),
                id: "abc".into(),
            },
        };
        assert_eq!(Cursor::decode(&cursor.encode()), Some(cursor.clone()));
        let unknown = Cursor {
            key: Key {
                time: None,
                ..cursor.key
            },
            ..cursor
        };
        assert_eq!(Cursor::decode(&unknown.encode()), Some(unknown));
        for bad in ["", "x", "zz", "abc", "7b7d", "가나", "00ff00ff"] {
            assert_eq!(Cursor::decode(bad), None, "{bad:?}");
        }
    }
}
