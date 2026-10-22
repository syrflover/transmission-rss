//! The automatic decision (`docs/requirements/library.md`, 자동 판정): a work's
//! folder name against the titles of AniList's search results.
//!
//! Titles are compared after Unicode NFC, case folding, trimming and collapsing
//! runs of white space into one space, and after nothing else: punctuation,
//! brackets, season or part words and numbers stay, and nothing is translated
//! or spelled differently. A candidate matches when its romaji, English or
//! native title or one of its synonyms is then equal to the folder name.
//!
//! The decision selects only when the search was read to its last page and
//! exactly one candidate matches. Another candidate with the same title (a
//! second season or a remake listed under the same name) leaves it empty, and
//! so does an unfinished search. The first result or the most similar one is
//! never taken.
//!
//! How many local seasons the work has does not matter: the cover is the
//! work's, so a work of several seasons takes the one entry named exactly like
//! its folder (usually the first season).
//!
//! A subscription's cover is decided by the Anissia anime it follows
//! ([`decide_original`]): its original title against the native titles only,
//! and its start date against the candidate's, under the same rules.

use trss_core::calendar::days_from_civil;
use unicode_normalization::UnicodeNormalization;

use crate::FuzzyDate;

/// One AniList entry as a search answered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: i64,
    pub romaji: Option<String>,
    pub english: Option<String>,
    pub native: Option<String>,
    pub synonyms: Vec<String>,
    /// AniList's format (`TV`, `MOVIE`, ...), shown to tell entries apart.
    pub format: Option<String>,
    pub season_year: Option<i32>,
    /// When the entry started airing, as far as AniList knows.
    pub start: FuzzyDate,
    /// The large cover, from an allowed image origin; `None` when the entry
    /// has none or its URL is not allowed.
    pub cover_url: Option<String>,
    /// A small cover for a list of results, same rule.
    pub thumb_url: Option<String>,
}

impl Candidate {
    fn titles(&self) -> impl Iterator<Item = &str> {
        [&self.romaji, &self.english, &self.native]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .chain(self.synonyms.iter().map(String::as_str))
    }

    /// The title to show: English, romaji or native, whichever there is.
    pub fn display_title(&self) -> &str {
        self.english
            .as_deref()
            .or(self.romaji.as_deref())
            .or(self.native.as_deref())
            .unwrap_or("")
    }
}

/// NFC, case folding, trimming, and one space for every run of white space.
pub fn normalize(title: &str) -> String {
    let folded = caseless::default_case_fold_str(&title.nfc().collect::<String>());
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What the search came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision<'a> {
    Select(&'a Candidate),
    NoMatch,
    /// More than one candidate matches.
    Ambiguous,
    /// The search was not read to its end, so a match is not known to be the
    /// only one.
    Incomplete,
}

/// Decides for the folder name `observed` among `candidates`, which are the
/// whole answer only when `complete`.
pub fn decide<'a>(observed: &str, candidates: &'a [Candidate], complete: bool) -> Decision<'a> {
    if !complete {
        return Decision::Incomplete;
    }
    let wanted = normalize(observed);
    if wanted.is_empty() {
        return Decision::NoMatch;
    }
    let mut found: Option<&Candidate> = None;
    for candidate in candidates {
        if !candidate.titles().any(|t| normalize(t) == wanted) {
            continue;
        }
        match found {
            // The same entry twice (pages that shifted) is one candidate.
            Some(first) if first.id == candidate.id => {}
            Some(_) => return Decision::Ambiguous,
            None => found = Some(candidate),
        }
    }
    found.map_or(Decision::NoMatch, Decision::Select)
}

/// How many days apart two start days may be and still be one airing.
/// Anissia gives the Korean start, which can come a few days after Japan's,
/// and a late-night slot is dated a day apart in the two listings.
pub const START_SLACK_DAYS: i64 = 7;

/// Whether two start dates are one airing's: their years, and their months
/// where both know them, are the same, and where both know the day, the days
/// are at most [`START_SLACK_DAYS`] apart. A date without a year agrees with
/// nothing.
pub fn starts_together(a: FuzzyDate, b: FuzzyDate) -> bool {
    let (Some(year_a), Some(year_b)) = (a.year, b.year) else {
        return false;
    };
    match (a.month.zip(a.day), b.month.zip(b.day)) {
        (Some((month_a, day_a)), Some((month_b, day_b))) => {
            let apart =
                days_from_civil(year_a, month_a, day_a) - days_from_civil(year_b, month_b, day_b);
            apart.abs() <= START_SLACK_DAYS
        }
        _ => year_a == year_b && (a.month.is_none() || b.month.is_none() || a.month == b.month),
    }
}

/// Decides for an Anissia anime among `candidates`, which are the whole
/// answer only when `complete`: a candidate matches when its native title is
/// `original` after [`normalize`] and it starts together with `start`
/// ([`starts_together`]). As [`decide`], it selects only the one match of a
/// search read to its end; an empty `original` matches nothing.
pub fn decide_original<'a>(
    original: &str,
    start: FuzzyDate,
    candidates: &'a [Candidate],
    complete: bool,
) -> Decision<'a> {
    if !complete {
        return Decision::Incomplete;
    }
    let wanted = normalize(original);
    if wanted.is_empty() {
        return Decision::NoMatch;
    }
    let mut found: Option<&Candidate> = None;
    for candidate in candidates {
        let named = candidate
            .native
            .as_deref()
            .is_some_and(|native| normalize(native) == wanted);
        if !named || !starts_together(candidate.start, start) {
            continue;
        }
        match found {
            Some(first) if first.id == candidate.id => {}
            Some(_) => return Decision::Ambiguous,
            None => found = Some(candidate),
        }
    }
    found.map_or(Decision::NoMatch, Decision::Select)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i64, romaji: &str, english: Option<&str>, synonyms: &[&str]) -> Candidate {
        Candidate {
            id,
            romaji: Some(romaji.to_owned()),
            english: english.map(str::to_owned),
            native: None,
            synonyms: synonyms.iter().map(|s| (*s).to_owned()).collect(),
            format: Some("TV".to_owned()),
            season_year: None,
            start: FuzzyDate::default(),
            cover_url: None,
            thumb_url: None,
        }
    }

    /// An entry with only a native title and a start date (`YYYY-MM-DD`, or less).
    fn native(id: i64, title: &str, start: &str) -> Candidate {
        Candidate {
            native: Some(title.to_owned()),
            start: FuzzyDate::from_text(start),
            ..entry(id, "", None, &[])
        }
    }

    #[test]
    fn normalizing_changes_only_form_case_and_spaces() {
        assert_eq!(normalize("  Lycoris   RECOIL\t"), "lycoris recoil");
        // Decomposed (macOS) and composed Korean are one title.
        assert_eq!(normalize("\u{1100}\u{1161}"), normalize("가"));
        // Case folding, not just lowercasing.
        assert_eq!(normalize("STRASSE"), normalize("straße"));
        // Punctuation, brackets, numbers and season words stay.
        assert_ne!(normalize("Lycoris Recoil!"), normalize("Lycoris Recoil"));
        assert_ne!(
            normalize("Lycoris Recoil (2022)"),
            normalize("Lycoris Recoil")
        );
        assert_ne!(normalize("Lycoris Recoil 2"), normalize("Lycoris Recoil"));
        assert_ne!(normalize("Lycoris-Recoil"), normalize("Lycoris Recoil"));
    }

    #[test]
    fn one_exact_title_among_others_is_selected() {
        let candidates = [
            entry(2, "Lycoris Recoil: Friends are thieves of time.", None, &[]),
            entry(1, "Lycoris Recoil", Some("Lycoris Recoil"), &["LycoReco"]),
            entry(3, "Lycoris Recoil Season 2", None, &[]),
        ];
        assert_eq!(
            decide("lycoris  recoil", &candidates, true),
            Decision::Select(&candidates[1])
        );
        // A synonym counts as a title.
        assert_eq!(
            decide("LycoReco", &candidates, true),
            Decision::Select(&candidates[1])
        );
    }

    #[test]
    fn two_entries_with_the_title_or_an_unfinished_search_leave_it_empty() {
        let candidates = [
            entry(1, "Lycoris Recoil", None, &[]),
            entry(3, "Lycoris Recoil Season 2", None, &["Lycoris Recoil"]),
        ];
        assert_eq!(
            decide("Lycoris Recoil", &candidates, true),
            Decision::Ambiguous
        );
        let one = [entry(1, "Lycoris Recoil", None, &[])];
        assert_eq!(decide("Lycoris Recoil", &one, false), Decision::Incomplete);
        // Only similar: nothing.
        assert_eq!(decide("Lycoris", &one, true), Decision::NoMatch);
        assert_eq!(decide("Lycoris Recoil 2", &one, true), Decision::NoMatch);
        // The same entry on two pages is one.
        let twice = [one[0].clone(), one[0].clone()];
        assert_eq!(
            decide("Lycoris Recoil", &twice, true),
            Decision::Select(&twice[0])
        );
    }

    #[test]
    fn an_anissia_anime_takes_the_one_entry_of_its_native_title_that_starts_with_it() {
        let start = FuzzyDate::from_text("2023-07-03");
        let candidates = [
            native(1, "無職転生", "2021-01-11"),
            native(2, "無職転生Ⅱ 〜異世界行ったら本気だす〜", "2023-07-03"),
            // Only the romaji title is the original title: not a match.
            Candidate {
                romaji: Some("無職転生Ⅱ 〜異世界行ったら本気だす〜".to_owned()),
                start,
                ..entry(3, "", None, &[])
            },
        ];
        assert_eq!(
            decide_original(
                "  無職転生Ⅱ   〜異世界行ったら本気だす〜",
                start,
                &candidates,
                true
            ),
            Decision::Select(&candidates[1])
        );
        // A Korean start a few days after Japan's is the same airing; a week more is not.
        let korean = FuzzyDate::from_text("2023-07-10");
        assert_eq!(
            decide_original(
                "無職転生Ⅱ 〜異世界行ったら本気だす〜",
                korean,
                &candidates,
                true
            ),
            Decision::Select(&candidates[1])
        );
        let later = FuzzyDate::from_text("2023-07-11");
        assert_eq!(
            decide_original(
                "無職転生Ⅱ 〜異世界行ったら本気だす〜",
                later,
                &candidates,
                true
            ),
            Decision::NoMatch
        );
        // Another wave dash is another title.
        assert_eq!(
            decide_original(
                "無職転生Ⅱ ～異世界行ったら本気だす～",
                start,
                &candidates,
                true
            ),
            Decision::NoMatch
        );
    }

    #[test]
    fn a_remake_of_the_same_title_is_told_by_its_start_and_a_doubt_leaves_no_cover() {
        let candidates = [
            native(10, "うる星やつら", "1981-10-14"),
            native(11, "うる星やつら", "2022-10-14"),
            native(12, "うる星やつら", "2022"),
        ];
        // The 2022 TV entry and an entry known only by its year both start in 2022.
        assert_eq!(
            decide_original(
                "うる星やつら",
                FuzzyDate::from_text("2022-10-14"),
                &candidates,
                true
            ),
            Decision::Ambiguous
        );
        assert_eq!(
            decide_original(
                "うる星やつら",
                FuzzyDate::from_text("1981-10"),
                &candidates,
                true
            ),
            Decision::Select(&candidates[0])
        );
        // A month both know must be the same.
        assert_eq!(
            decide_original(
                "うる星やつら",
                FuzzyDate::from_text("1981-11"),
                &candidates,
                true
            ),
            Decision::NoMatch
        );
        // No start date, no title, or a search not read to its end: no cover.
        assert_eq!(
            decide_original("うる星やつら", FuzzyDate::default(), &candidates, true),
            Decision::NoMatch
        );
        assert_eq!(
            decide_original(" ", FuzzyDate::from_text("1981-10"), &candidates, true),
            Decision::NoMatch
        );
        assert_eq!(
            decide_original(
                "うる星やつら",
                FuzzyDate::from_text("1981-10"),
                &candidates,
                false
            ),
            Decision::Incomplete
        );
    }

    #[test]
    fn a_text_date_keeps_the_parts_it_can_read() {
        let date = |y, m, d| FuzzyDate {
            year: y,
            month: m,
            day: d,
        };
        assert_eq!(
            FuzzyDate::from_text("2027-04"),
            date(Some(2027), Some(4), None)
        );
        assert_eq!(
            FuzzyDate::from_text("2026-10-07"),
            date(Some(2026), Some(10), Some(7))
        );
        assert_eq!(
            FuzzyDate::from_text("2026-13-07"),
            date(Some(2026), None, None)
        );
        assert_eq!(FuzzyDate::from_text(""), FuzzyDate::default());
    }
}
