//! How the app reads an episode text (`13`, `013`, `13.5`, `SP1`) as a key,
//! orders episodes, and writes one as `13화` (`docs/specs/library.md`, 자막의
//! 회차 대응). It also writes an offset ([`signed`]) and a run of episodes
//! ([`ranges`]) as the screens do.
//!
//! An episode text that is a decimal number (ASCII digits, then optionally a
//! `.` and ASCII digits) is an [`EpisodeNumber`]. Its value is what is left
//! without the leading zeros of the whole part and the trailing zeros of the
//! fraction: `013`, `13` and `13.0` are the number `13`, and `13.50` is
//! `13.5`. No float is made, so a number keeps every digit it was written
//! with. Anything else (`SP1`, `-1`, `.5`, `1.`, `1e3`, `inf`, `+5`, and the
//! empty text) is no number; it is an episode by its text, and sorts after
//! every number ([`EpisodeKey`]).

use std::{cmp::Ordering, fmt};

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
    out.join(", ")
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
    }

    #[test]
    fn an_offset_is_written_with_a_real_minus_sign() {
        assert_eq!(signed(0), "0");
        assert_eq!(signed(12), "12");
        assert_eq!(signed(-12), "−12");
        assert_eq!(signed(i64::MIN + 1), format!("−{}", i64::MAX));
    }
}
