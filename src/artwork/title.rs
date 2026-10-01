//! The automatic decision (`docs/specs/library.md`, 자동 판정): a work's
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

use unicode_normalization::UnicodeNormalization;

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
            cover_url: None,
            thumb_url: None,
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
}
