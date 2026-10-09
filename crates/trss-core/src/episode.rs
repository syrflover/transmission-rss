//! How the app reads an episode text (`13`, `013`, `13.5`, `SP1`) as a key,
//! orders episodes, and writes one as `13화` (`docs/specs/library.md`, 자막의
//! 회차 대응). It also writes an offset ([`signed`]) and a run of episodes
//! ([`ranges`], and [`EpisodeSet`] for episodes given as texts) as the screens
//! do.
//!
//! An episode text that is a decimal number (ASCII digits, then optionally a
//! `.` and ASCII digits) is an [`EpisodeNumber`]. Its value is what is left
//! without the leading zeros of the whole part and the trailing zeros of the
//! fraction: `013`, `13` and `13.0` are the number `13`, and `13.50` is
//! `13.5`. No float is made, so a number keeps every digit it was written
//! with. Anything else (`SP1`, `-1`, `.5`, `1.`, `1e3`, `inf`, `+5`, and the
//! empty text) is no number; it is an episode by its text, and sorts after
//! every number ([`EpisodeKey`]).

use std::{cmp::Ordering, collections::BTreeMap, fmt};

use serde::Serialize;

/// An episode text that is a decimal number, by value.
///
/// The parts are kept as digits, in the form that has no leading zeros in the
/// whole part (`0` for none) and no trailing zeros in the fraction (empty for
/// none).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EpisodeNumber {
    whole: String,
    fraction: String,
}

impl EpisodeNumber {
    /// The number `text` is, or `None` when it is no decimal number. The text
    /// is read as it is: spaces around it make it no number.
    pub fn parse(text: &str) -> Option<EpisodeNumber> {
        let (whole, fraction) = match text.split_once('.') {
            Some((whole, fraction)) => (whole, fraction),
            None => (text, "0"),
        };
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        if !digits(whole) || !digits(fraction) {
            return None;
        }
        let whole = match whole.trim_start_matches('0') {
            "" => "0",
            whole => whole,
        };
        Some(EpisodeNumber {
            whole: whole.to_owned(),
            fraction: fraction.trim_end_matches('0').to_owned(),
        })
    }

    /// Whether the number has no fraction (`13.0` is whole, `13.5` is not).
    pub fn is_whole(&self) -> bool {
        self.fraction.is_empty()
    }

    /// The value of a whole number; `None` for a number with a fraction and
    /// for one that does not fit a `u128`.
    pub fn whole(&self) -> Option<u128> {
        self.is_whole().then(|| self.whole.parse().ok()).flatten()
    }

    /// The nearest float, for a reader that needs a number to sort by. The
    /// key and the order of the app never go through it.
    pub fn to_f64(&self) -> f64 {
        self.to_string().parse().unwrap_or(f64::NAN)
    }

    /// Whether `key` is the number in its key form (`13` for `013`), as a
    /// reader that holds the key as text asks.
    pub fn is_key(&self, key: &str) -> bool {
        self.to_string() == key
    }

    /// `13화`, the number as the app writes an episode.
    pub fn label(&self) -> String {
        format!("{self}화")
    }
}

/// The number in its key form: `13`, `13.5`.
impl fmt::Display for EpisodeNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.is_whole() {
            true => f.write_str(&self.whole),
            false => write!(f, "{}.{}", self.whole, self.fraction),
        }
    }
}

impl PartialOrd for EpisodeNumber {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EpisodeNumber {
    fn cmp(&self, other: &Self) -> Ordering {
        // No leading zeros: the longer whole part is the larger one. The
        // fractions have no trailing zeros, so their text order is their
        // order (`25` is below `5`, and `5` is below `51`).
        self.whole
            .len()
            .cmp(&other.whole.len())
            .then_with(|| self.whole.cmp(&other.whole))
            .then_with(|| self.fraction.cmp(&other.fraction))
    }
}

/// An episode text as a key: two texts of one episode have one key. Numbers
/// order by value and come first; any other text is by its text, after every
/// number.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EpisodeKey {
    Number(EpisodeNumber),
    Text(String),
}

impl EpisodeKey {
    /// The key of `text`, as it is written (not trimmed).
    pub fn of(text: &str) -> EpisodeKey {
        match EpisodeNumber::parse(text) {
            Some(number) => EpisodeKey::Number(number),
            None => EpisodeKey::Text(text.to_owned()),
        }
    }

    /// The number, for a key that is one.
    pub fn number(&self) -> Option<&EpisodeNumber> {
        match self {
            EpisodeKey::Number(number) => Some(number),
            EpisodeKey::Text(_) => None,
        }
    }

    /// The key as the database keeps it, readable in a column: `n:` and the
    /// number (`n:13`, `n:13.5`), or `t:` and the text as written (`t:SP`).
    /// Rows written under this form are read under it, so it does not change.
    pub fn stored(&self) -> String {
        match self {
            EpisodeKey::Number(number) => format!("n:{number}"),
            EpisodeKey::Text(text) => format!("t:{text}"),
        }
    }

    /// `13화` for a number, in its key form (`013` is `13화`), and the text as
    /// it is for any other.
    pub fn label(&self) -> String {
        match self {
            EpisodeKey::Number(number) => number.label(),
            EpisodeKey::Text(text) => text.clone(),
        }
    }
}

/// The key two texts of one episode share, in the form the database keeps it
/// ([`EpisodeKey::stored`]).
pub fn stored_key(text: &str) -> String {
    EpisodeKey::of(text).stored()
}

/// `text` as it is written, with `화` after it when it is a number (`013` is
/// `013화`, `13.50` is `13.50화`), else the text alone. [`EpisodeKey::label`]
/// writes the number in its key form instead.
pub fn episode_label(text: &str) -> String {
    match EpisodeNumber::parse(text) {
        Some(_) => format!("{text}화"),
        None => text.to_owned(),
    }
}

/// A number with a real minus sign, as the screen writes offsets: `0`, `12`,
/// `−12`.
pub fn signed(value: i64) -> String {
    if value < 0 {
        format!("−{}", -value)
    } else {
        value.to_string()
    }
}

/// `1–12, 14` for the episodes given in ascending order.
pub fn ranges(episodes: &[u32]) -> String {
    range_texts(episodes).join(", ")
}

/// The runs of [`ranges`] one by one: `["1–12", "14"]`.
pub fn range_texts(episodes: &[u32]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut at = 0;
    while at < episodes.len() {
        let mut end = at;
        while end + 1 < episodes.len() && episodes[end + 1] == episodes[end] + 1 {
            end += 1;
        }
        out.push(if end == at {
            episodes[at].to_string()
        } else {
            format!("{}–{}", episodes[at], episodes[end])
        });
        at = end + 1;
    }
    out
}

/// An episode text as the screens show it: the leading zeros of a text of
/// digits only go (`01` is `1`, `00` is `0`); any other text, `13.0` and `SP`
/// included, stays as written.
pub fn shown(text: &str) -> &str {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return text;
    }
    match text.trim_start_matches('0') {
        "" => &text[text.len() - 1..],
        rest => rest,
    }
}

/// An episode as a set keys it: a whole number by value (`013`, `13` and
/// `13.0` are one), anything else, a number with a fraction included, by its
/// text. The whole numbers come first, in order, then the others by their
/// text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum SetKey {
    Whole(u128),
    Other(String),
}

fn set_key(text: &str) -> SetKey {
    match EpisodeNumber::parse(text).and_then(|n| n.whole()) {
        Some(whole) => SetKey::Whole(whole),
        None => SetKey::Other(text.to_owned()),
    }
}

/// Episodes given as texts, each distinct episode once, with the smallest of
/// the texts it was written as (`013` is below `13`), in the order the screens
/// list them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpisodeSet {
    written: BTreeMap<SetKey, String>,
}

impl EpisodeSet {
    pub fn new() -> EpisodeSet {
        EpisodeSet::default()
    }

    pub fn insert(&mut self, text: &str) {
        let written = self
            .written
            .entry(set_key(text))
            .or_insert_with(|| text.to_owned());
        if text < written.as_str() {
            *written = text.to_owned();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.written.is_empty()
    }

    /// Whether every episode of `other` is in this set.
    pub fn covers(&self, other: &EpisodeSet) -> bool {
        other
            .written
            .keys()
            .all(|key| self.written.contains_key(key))
    }

    /// The episodes as runs: consecutive whole numbers make one run, a gap
    /// splits it, and each episode that is not a whole number (`13.5`, `SP`)
    /// is a run of its own after them.
    pub fn runs(&self) -> Vec<EpisodeRun> {
        let mut out: Vec<EpisodeRun> = Vec::new();
        let mut last_whole: Option<u128> = None;
        for (key, written) in &self.written {
            match key {
                SetKey::Whole(n) => match out.last_mut() {
                    Some(run) if last_whole.is_some_and(|last| last.checked_add(1) == Some(*n)) => {
                        run.last = written.clone();
                        run.count += 1;
                    }
                    _ => out.push(EpisodeRun::of(written, true)),
                },
                SetKey::Other(_) => out.push(EpisodeRun::of(written, false)),
            }
            last_whole = match key {
                SetKey::Whole(n) => Some(*n),
                SetKey::Other(_) => None,
            };
        }
        out
    }
}

impl<'a> FromIterator<&'a str> for EpisodeSet {
    fn from_iter<I: IntoIterator<Item = &'a str>>(texts: I) -> EpisodeSet {
        let mut set = EpisodeSet::new();
        for text in texts {
            set.insert(text);
        }
        set
    }
}

/// A run of consecutive episodes, as written: `first` and `last` are the
/// written forms of its ends, equal for a single episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeRun {
    pub first: String,
    pub last: String,
    /// How many episodes the run holds.
    pub count: usize,
    /// Whether the run is of whole numbers (`0` and `13.0` included) rather
    /// than an episode of any other text.
    pub whole: bool,
}

impl EpisodeRun {
    fn of(text: &str, whole: bool) -> EpisodeRun {
        EpisodeRun {
            first: text.to_owned(),
            last: text.to_owned(),
            count: 1,
            whole,
        }
    }

    /// The run as the screens show it, its ends through [`shown`]: `1–4`, or
    /// `7` for a single episode.
    pub fn text(&self) -> String {
        range_text(&self.first, &self.last)
    }
}

/// A run from `first` to `last` as the screens show it: the ends through
/// [`shown`], and the end alone for a run of one episode (`1–4`, `7`).
pub fn range_text(first: &str, last: &str) -> String {
    match shown(first) == shown(last) {
        true => shown(first).to_owned(),
        false => format!("{}–{}", shown(first), shown(last)),
    }
}

/// The runs of the episodes `texts` name ([`EpisodeSet::runs`]).
pub fn runs<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vec<EpisodeRun> {
    texts.into_iter().collect::<EpisodeSet>().runs()
}

/// A run as an answer carries it, for a screen to name and shorten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EpisodeSegment {
    /// `1–4`, `7`, `SP` ([`EpisodeRun::text`]).
    pub text: String,
    /// How many episodes it holds.
    pub count: usize,
    /// Whether it is a run of whole numbers.
    pub whole: bool,
}

/// The runs of the episodes `texts` name, as answer segments.
pub fn segments<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vec<EpisodeSegment> {
    runs(texts)
        .into_iter()
        .map(|run| EpisodeSegment {
            text: run.text(),
            count: run.count,
            whole: run.whole,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(text: &str) -> EpisodeNumber {
        EpisodeNumber::parse(text).unwrap_or_else(|| panic!("{text} is a number"))
    }

    /// The key of a number, as `trss-subtitles` compared and showed it before
    /// this module held it (its `numeric_key` test).
    #[test]
    fn the_key_is_the_apps() {
        for (text, key) in [
            ("013", Some("13")),
            ("13", Some("13")),
            ("13.0", Some("13")),
            ("13.50", Some("13.5")),
            ("0", Some("0")),
            ("00", Some("0")),
            ("0.5", Some("0.5")),
            ("SP", None),
            ("", None),
            ("1.", None),
            (".5", None),
            ("1e3", None),
            ("-1", None),
        ] {
            assert_eq!(
                EpisodeNumber::parse(text).map(|n| n.to_string()).as_deref(),
                key,
                "{text}"
            );
        }
        assert_eq!(number("9").cmp(&number("10")), Ordering::Less);
        assert_eq!(number("12.5").cmp(&number("12.25")), Ordering::Greater);
        assert_eq!(number("12").cmp(&number("12.5")), Ordering::Less);
    }

    #[test]
    fn a_text_that_is_not_a_decimal_number_is_no_number() {
        for text in [
            "SP1", "SP", "-1", "+5", ".5", "1.", "1.2.3", "1e3", "1E3", "inf", "NaN", "", " ",
            " 13", "13 ", "1,5", "１３", "0x10", "1_0",
        ] {
            assert_eq!(EpisodeNumber::parse(text), None, "{text:?}");
            assert_eq!(EpisodeKey::of(text), EpisodeKey::Text(text.to_owned()));
        }
    }

    #[test]
    fn spellings_of_one_number_are_one_key() {
        assert_eq!(EpisodeKey::of("013"), EpisodeKey::of("13"));
        assert_eq!(EpisodeKey::of("13.0"), EpisodeKey::of("13"));
        assert_eq!(EpisodeKey::of("13.00"), EpisodeKey::of("013"));
        assert_eq!(EpisodeKey::of("1.50"), EpisodeKey::of("1.5"));
        assert_eq!(EpisodeKey::of("0.0"), EpisodeKey::of("0"));
        assert_ne!(EpisodeKey::of("1.5"), EpisodeKey::of("1"));
        assert_ne!(EpisodeKey::of("1.05"), EpisodeKey::of("1.5"));
        // A text is by its text: case and spaces make another.
        assert_ne!(EpisodeKey::of("SP"), EpisodeKey::of("sp"));
        assert_ne!(EpisodeKey::of("SP"), EpisodeKey::of("SP "));
    }

    #[test]
    fn episodes_order_as_numbers_and_other_text_comes_last() {
        let order = |texts: &[&'static str]| {
            let mut keys: Vec<(&str, EpisodeKey)> =
                texts.iter().map(|&e| (e, EpisodeKey::of(e))).collect();
            keys.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(b.0)));
            keys.into_iter().map(|(e, _)| e).collect::<Vec<&str>>()
        };
        assert_eq!(
            order(&["10", "9", "17.5", "17", "18", "SP", "2.25", "2.5", "02"]),
            ["02", "2.25", "2.5", "9", "10", "17", "17.5", "18", "SP"]
        );
        // What is no number is after every number, by its text.
        assert_eq!(
            order(&["SP", "10", "-1", "+5", "1e3", ".5", "9", "1."]),
            ["9", "10", "+5", "-1", ".5", "1.", "1e3", "SP"]
        );
        // Fractions order as the numbers they are, not as text.
        assert!(number("1.25") < number("1.5"));
        assert!(number("1.5") < number("1.51"));
        assert!(number("1.09") < number("1.1"));
        assert!(number("0.5") < number("1"));
        // Larger than any float keeps its digits.
        assert!(EpisodeKey::of("9007199254740993") < EpisodeKey::of("9007199254740994"));
        let huge = "9".repeat(60);
        assert!(EpisodeKey::of("9007199254740994") < EpisodeKey::of(&huge));
        assert!(EpisodeKey::of(&huge) < EpisodeKey::of("A"));
    }

    #[test]
    fn a_whole_number_has_a_value_when_it_fits() {
        assert!(number("013").is_key("13") && !number("013").is_key("013"));
        assert_eq!(number("013").whole(), Some(13));
        assert_eq!(number("13.0").whole(), Some(13));
        assert_eq!(number("0.0").whole(), Some(0));
        assert_eq!(number("13.5").whole(), None);
        assert!(number("13.0").is_whole() && !number("13.5").is_whole());
        assert_eq!(number(&u128::MAX.to_string()).whole(), Some(u128::MAX));
        assert_eq!(number(&"9".repeat(60)).whole(), None);
        assert!(number(&"9".repeat(60)).is_whole());
    }

    #[test]
    fn a_number_is_a_float_only_when_a_reader_asks() {
        assert_eq!(number("013").to_f64(), 13.0);
        assert_eq!(number("17.50").to_f64(), 17.5);
        assert_eq!(number("0").to_f64(), 0.0);
    }

    #[test]
    fn the_database_keeps_the_key_as_n_and_the_number_or_t_and_the_text() {
        assert_eq!(
            ["013", "13", "13.0", "13.50", "SP"].map(stored_key),
            ["n:13", "n:13", "n:13", "n:13.5", "t:SP"]
        );
        // A text that is no number is kept as it was written.
        assert_eq!(stored_key("sp 1"), "t:sp 1");
        assert_eq!(stored_key("1e3"), "t:1e3");
        assert_eq!(stored_key(""), "t:");
        assert_eq!(EpisodeKey::of("13").stored(), "n:13");
    }

    #[test]
    fn a_number_is_written_with_hwa_and_any_other_text_as_it_is() {
        // In its key form.
        assert_eq!(EpisodeKey::of("013").label(), "13화");
        assert_eq!(EpisodeKey::of("13.50").label(), "13.5화");
        assert_eq!(EpisodeKey::of("SP").label(), "SP");
        assert_eq!(number("12").label(), "12화");
        // As written.
        assert_eq!(episode_label("11"), "11화");
        assert_eq!(episode_label("013"), "013화");
        assert_eq!(episode_label("13.50"), "13.50화");
        for text in ["SP", "1e3", "inf", "+5", "-1", ".5", "1.", ""] {
            assert_eq!(episode_label(text), text);
        }
    }

    #[test]
    fn episodes_are_written_as_ranges() {
        assert_eq!(ranges(&[1, 2, 3, 5, 7, 8]), "1–3, 5, 7–8");
        assert_eq!(ranges(&[4]), "4");
        assert_eq!(range_texts(&[1, 2, 3, 5, 7, 8]), ["1–3", "5", "7–8"]);
        assert_eq!(range_texts(&[]), Vec::<String>::new());
    }

    #[test]
    fn leading_zeros_of_a_text_of_digits_go_and_nothing_else_changes() {
        for (text, shown_text) in [
            ("01", "1"),
            ("02", "2"),
            ("2", "2"),
            ("013", "13"),
            ("1", "1"),
            ("10", "10"),
            ("00", "0"),
            ("000", "0"),
            ("0", "0"),
            ("13.0", "13.0"),
            ("013.0", "013.0"),
            ("0.5", "0.5"),
            ("SP", "SP"),
            ("SP01", "SP01"),
            ("", ""),
            ("-01", "-01"),
            (" 01", " 01"),
            ("０１", "０１"),
        ] {
            assert_eq!(shown(text), shown_text, "{text:?}");
        }
    }

    /// The first and last text of each run `texts` make.
    fn pairs(texts: &[&str]) -> Vec<(String, String)> {
        runs(texts.iter().copied())
            .into_iter()
            .map(|r| (r.first, r.last))
            .collect()
    }

    fn written(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn consecutive_episodes_make_one_run_and_a_gap_splits_it() {
        assert_eq!(pairs(&["01", "02", "03"]), written(&[("01", "03")]));
        assert_eq!(
            pairs(&["01", "02", "03", "05", "06", "12"]),
            written(&[("01", "03"), ("05", "06"), ("12", "12")])
        );
        assert_eq!(pairs(&[]), written(&[]));
    }

    #[test]
    fn episodes_run_by_number_not_by_text_or_by_the_order_given() {
        // Written order is not numeric order, and `2` is not after `10` here.
        assert_eq!(pairs(&["10", "9", "11"]), written(&[("9", "11")]));
        assert_eq!(pairs(&["3", "2", "1"]), written(&[("1", "3")]));
        // `013` and `13` are one episode, shown as the smaller written form.
        assert_eq!(pairs(&["13", "013", "14"]), written(&[("013", "14")]));
        // Zero is an episode, and a leading zero does not change its value.
        assert_eq!(pairs(&["00", "1"]), written(&[("00", "1")]));
        // An episode given twice is one.
        let run = &runs(["2", "2"])[0];
        assert_eq!((run.text().as_str(), run.count), ("2", 1));
        assert_eq!(runs(["2", "2"]).len(), 1);
    }

    #[test]
    fn zero_is_inside_the_runs_of_whole_numbers() {
        assert_eq!(pairs(&["0", "1", "2"]), written(&[("0", "2")]));
        assert_eq!(pairs(&["2", "0", "1"]), written(&[("0", "2")]));
        assert_eq!(pairs(&["0", "2"]), written(&[("0", "0"), ("2", "2")]));
        assert!(runs(["0"])[0].whole);
    }

    #[test]
    fn a_number_larger_than_any_float_keeps_its_digits() {
        let big = "9007199254740993"; // 2^53 + 1: not representable as an f64
        let next = "9007199254740994";
        let apart = "9007199254740996";
        assert_eq!(
            pairs(&[big, next, apart]),
            written(&[(big, next), (apart, apart)])
        );
        // Beyond u128 the text is all there is: it stays a run of its own.
        let huge = "9".repeat(60);
        assert_eq!(pairs(&[&huge]), written(&[(&huge, &huge)]));
        assert!(!runs([huge.as_str()])[0].whole);
    }

    #[test]
    fn episodes_that_are_not_whole_numbers_stay_apart_and_come_last() {
        assert_eq!(
            pairs(&["17.5", "01", "02", "SP", "17"]),
            written(&[("01", "02"), ("17", "17"), ("17.5", "17.5"), ("SP", "SP")])
        );
        // `1.5` does not join `1` and `2`, and `1` and `2` still make a run.
        assert_eq!(
            pairs(&["1", "1.5", "2"]),
            written(&[("1", "2"), ("1.5", "1.5")])
        );
        assert_eq!(
            pairs(&["SP", "1", "2"]),
            written(&[("1", "2"), ("SP", "SP")])
        );
        assert_eq!(
            pairs(&["13.5", "14"]),
            written(&[("14", "14"), ("13.5", "13.5")])
        );
        let segments = segments(["SP", "13.5", "2", "1"]);
        let flags: Vec<(&str, usize, bool)> = segments
            .iter()
            .map(|s| (s.text.as_str(), s.count, s.whole))
            .collect();
        assert_eq!(
            flags,
            [("1–2", 2, true), ("13.5", 1, false), ("SP", 1, false)]
        );
    }

    #[test]
    fn a_whole_number_written_with_a_zero_fraction_is_that_number() {
        // `13.0` joins `12` and `14` in one run; `13.5` stays apart, after it.
        assert_eq!(
            pairs(&["12", "13.0", "14", "13.5"]),
            written(&[("12", "14"), ("13.5", "13.5")])
        );
        // `13.0` and `13` are one episode, shown as the smaller written form;
        // `13.0` alone is shown as written.
        assert_eq!(pairs(&["13.0", "13", "14"]), written(&[("13", "14")]));
        assert_eq!(pairs(&["13.00"]), written(&[("13.00", "13.00")]));
        assert_eq!(runs(["13", "13.0"]).len(), 1);
        // A fraction that is not zeros, or is empty, is no whole number.
        assert_eq!(
            pairs(&["13.01", "13."]),
            written(&[("13.", "13."), ("13.01", "13.01")])
        );
    }

    #[test]
    fn a_run_is_shown_with_its_ends_through_shown() {
        let text = |texts: &[&str]| -> Vec<String> {
            runs(texts.iter().copied())
                .iter()
                .map(|r| r.text())
                .collect()
        };
        assert_eq!(text(&["01", "02", "03", "04", "07"]), ["1–4", "7"]);
        assert_eq!(text(&["00", "01"]), ["0–1"]);
        assert_eq!(text(&["13.0", "14"]), ["13.0–14"]);
        assert_eq!(text(&["013"]), ["13"]);
        assert_eq!(text(&["SP01"]), ["SP01"]);
        assert_eq!(range_text("01", "03"), "1–3");
        assert_eq!(range_text("013", "013"), "13");
        assert_eq!(range_text("0", "00"), "0");
        assert_eq!(text(&[]), Vec::<String>::new());
    }

    #[test]
    fn a_run_counts_the_episodes_it_holds() {
        let counts: Vec<(String, usize)> = runs(["1", "01", "2", "3", "5", "SP", "6"])
            .iter()
            .map(|r| (r.text(), r.count))
            .collect();
        assert_eq!(
            counts,
            [
                ("1–3".to_owned(), 3),
                ("5–6".to_owned(), 2),
                ("SP".to_owned(), 1)
            ]
        );
    }

    #[test]
    fn a_set_covers_the_episodes_it_holds_by_value() {
        let set = |texts: &[&str]| texts.iter().copied().collect::<EpisodeSet>();
        assert!(set(&["13", "14"]).covers(&set(&["13.0"])));
        assert!(set(&["1", "2"]).covers(&set(&["01"])));
        assert!(!set(&["1", "2"]).covers(&set(&["1", "3"])));
        assert!(set(&["1"]).covers(&set(&[])));
        assert!(set(&[]).is_empty() && !set(&["1"]).is_empty());
    }

    #[test]
    fn an_offset_is_written_with_a_real_minus_sign() {
        assert_eq!(signed(0), "0");
        assert_eq!(signed(12), "12");
        assert_eq!(signed(-12), "−12");
        assert_eq!(signed(i64::MIN + 1), format!("−{}", i64::MAX));
    }
}
