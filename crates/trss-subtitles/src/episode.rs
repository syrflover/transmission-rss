//! Which links of a post serve a candidate's episode (`docs/specs/jobs.md`,
//! 공통 수신 결과와 실패 분류): a post on Blogger, or a Tistory post whose
//! subtitle is in Google Drive, links one file per episode, a ZIP of a range
//! of them, a font, or all of these together, and says which is which only in
//! the words of each link (`24화`, `1 ~ 12화`, `폰트`, `… 4 24.zip`). A Naver
//! post attaches them, and says it in the files' names.
//!
//! - A link that names episodes is taken when one of them is the candidate's:
//!   a single one (`24화`, `제24화`, `EP24`, `고양이와 용 08`) when it is the
//!   same, a range (`1 ~ 12화`, `03-04`, `01~13.ass`) when it holds it. The
//!   comparison is the app's (`trss_core::episode`): `08`, `8` and `8.0` are one
//!   episode, and `8.5` is never rounded.
//! - A font (`폰트`, `글꼴`, `font`) that names no episode is always taken: an
//!   ASS needs it. One that names an episode (`24화 (폰트 포함)`) is that
//!   episode's. A font file (`.ttf`, `.otf`, `.ttc`, `.woff`, `.woff2`) is a
//!   font whatever its name's numbers (`H2MPRB.TTF`).
//! - An archive whose name says no episode (`자막 모음.zip`) is taken: the
//!   analysis tells what it holds.
//! - When nothing names the episode and the post has one subtitle link only,
//!   whose words say nothing (`자막 다운로드`, a picture, `BLACK TORCH`), that
//!   one is taken: the common post of one file.
//! - Otherwise nothing serves the episode, and the post has changed from what
//!   the candidate said (`게시물에 24화 파일이 없어요`).
//!
//! Words name an episode only with a number that stands on its own: a number
//! glued to a letter (`S2`, `4th`, `2기`, `1080p`, `그랑블루3`) is no episode,
//! but `제24`, `第24`, `EP24`, `E24` and `S02E24` are, and one glued by a dot
//! is none either (`H.264`). A number marked `화` (`話`, `회`) counts before any other;
//! without one, the last number does (`… 4 24.zip` is 24), leaving out years
//! and dates, numbers over 1000 and decimals but `.5` ([`mentions`] has the
//! ranges).

use trss_core::episode::{EpisodeKey, EpisodeNumber};

/// Episodes from one to another, both included (one episode: the same key
/// twice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub from: String,
    pub to: String,
}

impl Span {
    pub(crate) fn holds(&self, key: &EpisodeNumber) -> bool {
        let number = |text: &str| EpisodeNumber::parse(text);
        match (number(&self.from), number(&self.to)) {
            (Some(from), Some(to)) => from <= *key && *key <= to,
            _ => false,
        }
    }
}

/// What a link's words say it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holds {
    Font,
    Episodes(Vec<Span>),
    /// An archive that names no episode.
    Bundle,
    Nothing,
}

const DASHES: [char; 5] = ['~', '-', '–', '〜', '～'];
const EPISODE_MARKS: [char; 3] = ['화', '話', '회'];
/// The extensions of font files, which Naver posts attach beside the
/// subtitle.
const FONT_FILES: [&str; 5] = [".ttf", ".otf", ".ttc", ".woff", ".woff2"];

/// What the words of a link say it holds (see the module docs).
pub fn holds(text: &str) -> Holds {
    let text = text.trim();
    let lower = text.to_lowercase();
    // An address as its own words says nothing (its ID has digits).
    if lower.contains("://") || lower.starts_with("drive.google.") {
        return Holds::Nothing;
    }
    // A font file is a font whatever numbers its name has (`H2MPRB.TTF`,
    // `Pretendard 700.otf`): no episode's subtitle is one.
    if FONT_FILES.iter().any(|e| lower.ends_with(e)) {
        return Holds::Font;
    }
    let found = mentions(text);
    let marked: Vec<Span> = found
        .iter()
        .filter(|(_, marked)| *marked)
        .map(|(span, _)| span.clone())
        .collect();
    if !marked.is_empty() {
        return Holds::Episodes(marked);
    }
    if let Some((span, _)) = found.last() {
        return Holds::Episodes(vec![span.clone()]);
    }
    // A font only when no episode is named: `24화 (폰트 포함)` is 24화's.
    if ["폰트", "글꼴", "font"].iter().any(|w| lower.contains(w)) {
        return Holds::Font;
    }
    match [".zip", ".7z", ".rar"].iter().any(|e| lower.ends_with(e)) {
        true => Holds::Bundle,
        false => Holds::Nothing,
    }
}

/// The numbers that stand on their own in `text`, single or ranges, each with
/// whether `화` marks it.
///
/// - `~` joins a range, spaced or not (`1 ~ 12화`, `01~13`). `-` joins one
///   when nothing parts it from the numbers (`1-3`, `03-04`), or when `화`
///   marks a side (`1 - 12화`); `Title 3 - 05` is two numbers, 3 and 5.
/// - Ends that are equal or descending (`03-03`, `24-12`) are no range: each
///   is read on its own.
/// - After `E` or `EP`, the mark may come again before the last number
///   (`E01-E12`, `S02E01-E12`).
/// - A date (`2026-10-03`, `2026.10.03`) is no number.
/// - Unmarked, a number over 1000 (a year, `[12345678]`) and a decimal other
///   than `.5` (`AAC 2.0`) are no episode; `13.5` is one, and so is anything
///   `화` marks.
fn mentions(text: &str) -> Vec<(Span, bool)> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() || i.checked_sub(1).is_some_and(|b| chars[b].is_ascii_digit())
        {
            i += 1;
            continue;
        }
        if let Some(end) = date_at(&chars, i) {
            i = end;
            continue;
        }
        let Some((first, end)) = number_at(&chars, i) else {
            i += 1;
            continue;
        };
        if !free_before(&chars, i) {
            i = end;
            continue;
        }
        let Some((marked, after)) = after_number(&chars, end) else {
            i = end;
            continue;
        };
        let dotted = |from: usize, to: usize| chars[from..to].contains(&'.');
        let dash = skip_spaces(&chars, after);
        if let Some(&joint) = chars.get(dash).filter(|c| DASHES.contains(c)) {
            let gap = skip_spaces(&chars, dash + 1);
            // `E01-E12`, `EP01-EP12`, `S02E01-E12`: the mark again before
            // the last.
            let start = match marked_by_word(&chars, i) {
                true => gap + episode_word_at(&chars, gap),
                false => gap,
            };
            let last = number_at(&chars, start).and_then(|(last, end_last)| {
                Some((last, end_last, after_number(&chars, end_last)?))
            });
            if let Some((last, end_last, (marked_last, after_last))) = last {
                let tilde = !matches!(joint, '-' | '–');
                let spaced = dash > end || gap > dash + 1;
                let either = marked || marked_last;
                if (tilde || !spaced || either) && first < last {
                    keep(
                        &mut found,
                        Span {
                            from: first.to_string(),
                            to: last.to_string(),
                        },
                        either,
                        dotted(i, end) || dotted(start, end_last),
                    );
                    i = after_last;
                    continue;
                }
                // Not a range: this number alone, and the next on its own.
                keep(
                    &mut found,
                    Span {
                        from: first.to_string(),
                        to: first.to_string(),
                    },
                    marked,
                    dotted(i, end),
                );
                i = dash + 1;
                continue;
            }
        }
        keep(
            &mut found,
            Span {
                from: first.to_string(),
                to: first.to_string(),
            },
            marked,
            dotted(i, end),
        );
        i = after;
    }
    found
}

/// Whether `E` or `EP` (any case) is right before the number at `i`.
fn marked_by_word(chars: &[char], i: usize) -> bool {
    let at = |back: usize| i.checked_sub(back).and_then(|b| chars.get(b));
    match at(1) {
        Some('e' | 'E') => true,
        Some('p' | 'P') => matches!(at(2), Some('e' | 'E')),
        _ => false,
    }
}

/// How long an `E` or `EP` (any case) at `at` is, when a digit follows it;
/// otherwise 0.
fn episode_word_at(chars: &[char], at: usize) -> usize {
    let is = |i: usize, set: [char; 2]| chars.get(i).is_some_and(|c| set.contains(c));
    let len = match (is(at, ['e', 'E']), is(at + 1, ['p', 'P'])) {
        (true, true) => 2,
        (true, false) => 1,
        _ => return 0,
    };
    match chars.get(at + len).is_some_and(|c| c.is_ascii_digit()) {
        true => len,
        false => 0,
    }
}

/// Keeps a mention, unless it is unmarked and no episode could be it (see
/// [`mentions`]).
/// `dotted` says whether either end was written with a decimal part, which
/// `2.0` loses in its key.
fn keep(found: &mut Vec<(Span, bool)>, span: Span, marked: bool, dotted: bool) {
    let plausible = |key: &str| {
        let (whole, fraction) = key.split_once('.').unwrap_or((key, ""));
        EpisodeNumber::parse(whole)
            .and_then(|n| n.whole())
            .is_some_and(|n| n <= 1000)
            && match dotted {
                true => fraction == "5",
                false => fraction.is_empty(),
            }
    };
    if marked || (plausible(&span.from) && plausible(&span.to)) {
        found.push((span, marked));
    }
}

/// Where a date that starts at `i` ends: a year from 1900 to 2099, then a
/// month and a day, joined by one of `-`, `.` or `/`.
fn date_at(chars: &[char], i: usize) -> Option<usize> {
    let digits = |at: usize, most: usize| {
        chars
            .get(at..)
            .unwrap_or_default()
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count()
            .min(most + 1)
    };
    let year: String = chars.get(i..i + 4)?.iter().collect();
    if digits(i, 4) != 4 || !(year.starts_with("19") || year.starts_with("20")) {
        return None;
    }
    let joint = *chars.get(i + 4).filter(|c| matches!(c, '-' | '.' | '/'))?;
    let month = digits(i + 5, 2);
    if !(1..=2).contains(&month) || chars.get(i + 5 + month) != Some(&joint) {
        return None;
    }
    let day_at = i + 6 + month;
    let day = digits(day_at, 2);
    (1..=2).contains(&day).then_some(day_at + day)
}

/// The number starting at `i` (`08`, `12.5`), and where it ends.
fn number_at(chars: &[char], i: usize) -> Option<(EpisodeNumber, usize)> {
    let digits_from = |at: usize| {
        chars[at..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count()
    };
    let whole = digits_from(i);
    if whole == 0 {
        return None;
    }
    let mut end = i + whole;
    // A decimal part, not the dot of an extension (`12.zip`).
    if chars.get(end) == Some(&'.') && chars.get(end + 1).is_some_and(|c| c.is_ascii_digit()) {
        end += 1 + digits_from(end + 1);
    }
    let text: String = chars[i..end].iter().collect();
    Some((EpisodeNumber::parse(&text)?, end))
}

/// Whether the number at `start` is not glued to a word before it, but for
/// the words that mark an episode (`제`, `第`, `EP`, `E`, `Episode`, and `E`
/// after a season, `S02E01`). A dot between them glues them as well
/// (`H.264`, `1.2.3`).
fn free_before(chars: &[char], start: usize) -> bool {
    let Some(&before) = start.checked_sub(1).and_then(|i| chars.get(i)) else {
        return true;
    };
    if before == '제' || before == '第' {
        return true;
    }
    let word_end = match before {
        '.' if start
            .checked_sub(2)
            .and_then(|i| chars.get(i))
            .is_some_and(|c| c.is_alphanumeric()) =>
        {
            start - 1
        }
        c if c.is_alphabetic() => start,
        _ => return true,
    };
    let word: String = chars[..word_end]
        .iter()
        .rev()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>()
        .to_ascii_lowercase();
    let before_word = &chars[..word_end - word.chars().count()];
    // `S02E01`: the episode after the season.
    let season = word == "e" && {
        let digits = before_word
            .iter()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .count();
        let s_at = before_word.len().checked_sub(digits + 1);
        digits > 0
            && s_at.is_some_and(|at| {
                matches!(before_word[at], 's' | 'S')
                    && before_word[..at]
                        .last()
                        .is_none_or(|c| !c.is_alphanumeric())
            })
    };
    // A word that is not all ASCII letters ends in some other letter or a
    // digit.
    season
        || (before_word.last().is_none_or(|c| !c.is_alphanumeric())
            && ["e", "ep", "episode"].contains(&word.as_str()))
}

/// After the number ending at `end`: whether `화` marks it and where the
/// mention ends, or `None` when a word is glued to it (`4th`, `2기`).
fn after_number(chars: &[char], end: usize) -> Option<(bool, usize)> {
    let Some(&next) = chars.get(end) else {
        return Some((false, end));
    };
    if EPISODE_MARKS.contains(&next) {
        return Some((true, end + 1));
    }
    if next.is_alphabetic() {
        return None;
    }
    let mark = skip_spaces(chars, end);
    match chars.get(mark).is_some_and(|c| EPISODE_MARKS.contains(c)) {
        true => Some((true, mark + 1)),
        false => Some((false, end)),
    }
}

fn skip_spaces(chars: &[char], from: usize) -> usize {
    from + chars
        .get(from..)
        .unwrap_or_default()
        .iter()
        .take_while(|c| c.is_whitespace())
        .count()
}

/// Which of a post's links, by the words of each, serve `episode`: their
/// places in `texts`, in order, or why none does (see the module docs).
pub fn choose(episode: &str, texts: &[&str]) -> Result<Vec<usize>, String> {
    let key = EpisodeKey::of(episode.trim());
    let held: Vec<Holds> = texts.iter().map(|t| holds(t)).collect();
    let serves = |h: &Holds| match h {
        Holds::Episodes(spans) => key
            .number()
            .is_some_and(|k| spans.iter().any(|s| s.holds(k))),
        Holds::Bundle => true,
        Holds::Font | Holds::Nothing => false,
    };
    let font = |i: &usize| held[*i] == Holds::Font;
    let all = 0..held.len();
    if all.clone().any(|i| serves(&held[i])) {
        return Ok(all.filter(|i| font(i) || serves(&held[*i])).collect());
    }
    let subtitles: Vec<usize> = all.clone().filter(|i| !font(i)).collect();
    if let [only] = subtitles[..] {
        if held[only] == Holds::Nothing {
            return Ok(all.filter(|i| font(i) || *i == only).collect());
        }
    }
    let label = key.label();
    let unnamed = subtitles
        .iter()
        .filter(|i| held[**i] == Holds::Nothing)
        .count();
    Err(match () {
        _ if subtitles.is_empty() && held.is_empty() => "게시물에 받을 링크가 없어요".to_owned(),
        _ if subtitles.is_empty() => "게시물에 폰트 말고 받을 자막 링크가 없어요".to_owned(),
        _ if unnamed > 1 => format!(
            "게시물의 링크 {unnamed}개가 몇 화인지 밝히지 않아 {label} 파일을 고를 수 없어요"
        ),
        _ => format!("게시물에 {label} 파일이 없어요"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(from: &str, to: &str) -> Span {
        Span {
            from: from.to_owned(),
            to: to.to_owned(),
        }
    }

    fn one(n: &str) -> Holds {
        Holds::Episodes(vec![span(n, n)])
    }

    /// The words of the links seen on real posts (2026-10-03).
    #[test]
    fn the_words_seen_on_real_posts_say_what_they_hold() {
        for (text, expected) in [
            // C소라: one link per episode, a range, the fonts.
            ("15화", one("15")),
            ("24화", one("24")),
            ("1 ~ 12화", Holds::Episodes(vec![span("1", "12")])),
            ("1 ~ 13화", Holds::Episodes(vec![span("1", "13")])),
            ("폰트", Holds::Font),
            // 이마이: past 100.
            ("133화", one("133")),
            // 별명따위: the ZIP's name, with the season before the episode.
            ("소녀 괴수 캐러멜리제 12.zip", one("12")),
            ("전생했더니 슬라임이었던 건에 대하여 4 24.zip", one("24")),
            // 에텔레로사: a release name, a season glued to `S`.
            ("공각기동대 03 ToonsHub", one("3")),
            (
                "공각기동대 1-3 ToonsHub",
                Holds::Episodes(vec![span("1", "3")]),
            ),
            (
                "클레바테스 S2 03-04 SubsPlease",
                Holds::Episodes(vec![span("3", "4")]),
            ),
            (
                "클레바테스 1-12 SubsPlease",
                Holds::Episodes(vec![span("1", "12")]),
            ),
            // soso_sagak: a bare number, or the title alone.
            ("고양이와 용 08", one("8")),
            ("BLACK TORCH", Holds::Nothing),
            // fanic: the title and the episode.
            ("낯가림 심한 미망인 설녀와 저주의 반지 1화 자막", one("1")),
            // 카이란 (a picture) and felia.
            ("", Holds::Nothing),
            ("자막 다운로드", Holds::Nothing),
            // The names Drive gave the files.
            ("Koori no Jouheki (The Ramparts of Ice) 15.ass", one("15")),
            ("그랑블루3 1-12.zip", Holds::Episodes(vec![span("1", "12")])),
            ("리제로4 19화 미완성(2).ass", one("19")),
            ("얼음폰트.zip", Holds::Font),
            (
                "BanG Dream! Yumemita 01~13.ass",
                Holds::Episodes(vec![span("1", "13")]),
            ),
        ] {
            assert_eq!(holds(text), expected, "{text}");
        }
    }

    #[test]
    fn a_number_glued_to_a_word_is_no_episode_but_the_marks_of_one_are() {
        for (text, expected) in [
            ("제24화", one("24")),
            ("第24話", one("24")),
            ("EP24", one("24")),
            ("[SubsPlease] Mebius Dust - E12 (1080p)", one("12")),
            ("Episode 7", one("7")),
            ("12.5화", one("12.5")),
            ("Re:제로 4th 19화", one("19")),
            ("2기 7 화", one("7")),
            ("3장 자막", Holds::Nothing),
            ("x264 1080p", Holds::Nothing),
            ("1화~12화", Holds::Episodes(vec![span("1", "12")])),
            ("13–24화", Holds::Episodes(vec![span("13", "24")])),
            (
                "24화(96화)",
                Holds::Episodes(vec![span("24", "24"), span("96", "96")]),
            ),
            ("자막 모음.zip", Holds::Bundle),
            ("Hotori Fonts.zip", Holds::Font),
            // A season, then the episode.
            ("Title S02E01 1080p", one("1")),
            ("Title s2e05.ass", one("5")),
            ("Title S02", Holds::Nothing),
            // A range of them, the mark again before the last.
            (
                "Title S02E01-E12 1080p",
                Holds::Episodes(vec![span("1", "12")]),
            ),
            ("E01-E12", Holds::Episodes(vec![span("1", "12")])),
            ("ep01-ep12.zip", Holds::Episodes(vec![span("1", "12")])),
            ("EP01 - EP12", Holds::Episodes(vec![span("12", "12")])),
            ("Title 01-E12", one("12")),
            ("Title XS02E01", Holds::Nothing),
            // Font files, whatever their numbers.
            ("H2MPRB.TTF", Holds::Font),
            ("a옛날목욕탕L.ttf", Holds::Font),
            ("Pretendard 700.otf", Holds::Font),
            (
                "https://drive.google.com/file/d/1W0LRBy-gxCLtmDpQBd8kGYb79oh19YRe/view",
                Holds::Nothing,
            ),
        ] {
            assert_eq!(holds(text), expected, "{text}");
        }
    }

    #[test]
    fn a_spaced_dash_joins_a_range_only_when_an_episode_mark_says_so() {
        for (text, expected) in [
            ("Title 3 - 05", one("5")),
            ("Title 2 - 05", one("5")),
            ("Title 3 – 05.ass", one("5")),
            ("1 - 12화", Holds::Episodes(vec![span("1", "12")])),
            ("1화 - 12", Holds::Episodes(vec![span("1", "12")])),
            ("13 ~ 24화", Holds::Episodes(vec![span("13", "24")])),
            ("13 ~ 24", Holds::Episodes(vec![span("13", "24")])),
            ("1-3", Holds::Episodes(vec![span("1", "3")])),
            ("Title 1-12.zip", Holds::Episodes(vec![span("1", "12")])),
        ] {
            assert_eq!(holds(text), expected, "{text}");
        }
    }

    #[test]
    fn years_dates_codecs_and_large_numbers_are_no_episode_unless_marked() {
        for (text, expected) in [
            ("Title 05 (2026)", one("5")),
            ("Title 05 (1999)", one("5")),
            ("[SubsPlease] Title - 05 (1080p) [12345678].ass", one("5")),
            ("Title 05 H.264", one("5")),
            ("Title 05 x.264", one("5")),
            ("Title 05 AAC2.0", one("5")),
            ("Title 05 AAC 2.0", one("5")),
            ("Title 05 5.1", one("5")),
            ("Title 05 v1.2.3", one("5")),
            ("2026-10-03 Title 05", one("5")),
            ("Title 05 2026.10.03", one("5")),
            ("Title 05 2026/1/3", one("5")),
            ("Title 13.5", one("13.5")),
            ("13.5화", one("13.5")),
            ("5.1화", one("5.1")),
            ("Title 1001", Holds::Nothing),
            ("1001화", one("1001")),
            ("Title 1000", one("1000")),
            ("(2026)", Holds::Nothing),
            ("2025-2026.zip", Holds::Bundle),
        ] {
            assert_eq!(holds(text), expected, "{text}");
        }
    }

    #[test]
    fn equal_or_descending_ends_are_read_one_by_one() {
        assert_eq!(holds("03-03"), one("3"));
        assert_eq!(holds("Title 24-12"), one("12"));
        assert_eq!(holds("24-12화"), one("12"));
        assert_eq!(holds("12 ~ 12화"), one("12"));
    }

    #[test]
    fn a_font_link_that_names_an_episode_is_that_episodes() {
        assert_eq!(holds("24화 (폰트 포함)"), one("24"));
        assert_eq!(holds("1화 폰트"), one("1"));
        assert_eq!(holds("폰트 (2026)"), Holds::Font);
        assert_eq!(choose("1", &["1화 폰트", "2화"]), Ok(vec![0]));
        assert_eq!(choose("24", &["24화 (폰트 포함)", "폰트"]), Ok(vec![0, 1]));
        assert_eq!(
            choose("2", &["1화 폰트"]),
            Err("게시물에 2화 파일이 없어요".to_owned())
        );
    }

    /// The links of the sixteen real posts sampled on 2026-10-03, each with
    /// the episode Anissia gave: the files chosen.
    #[test]
    fn the_real_posts_sampled_resolve_to_their_files() {
        let csora2: Vec<String> = std::iter::once("폰트".to_owned())
            .chain((13..=24).map(|n| format!("{n}화")))
            .collect();
        let csora2: Vec<&str> = csora2.iter().map(String::as_str).collect();
        let beyimai: Vec<String> = (101..=133).map(|n| format!("{n}화")).collect();
        let beyimai: Vec<&str> = beyimai.iter().map(String::as_str).collect();
        let cleveates = [
            "클레바테스 S2 03-04 SubsPlease",
            "클레바테스 S2 1-4 SubsPlease",
            "클레바테스 1-12 SubsPlease",
        ];
        let cases: Vec<(&str, &str, Vec<&str>, Vec<&str>)> = vec![
            (
                "csora556 2026/10/2",
                "15",
                vec!["폰트", "15화"],
                vec!["폰트", "15화"],
            ),
            (
                "csora556 2026/07/2",
                "24",
                csora2.clone(),
                vec!["폰트", "24화"],
            ),
            (
                "csora556 season-3",
                "12",
                vec!["폰트", "1 ~ 12화"],
                vec!["폰트", "1 ~ 12화"],
            ),
            (
                "csora556 bang-dream",
                "13",
                vec!["폰트", "1 ~ 13화"],
                vec!["폰트", "1 ~ 13화"],
            ),
            (
                "csora556 blog-post_07",
                "12",
                vec!["폰트", "1 ~ 12화"],
                vec!["폰트", "1 ~ 12화"],
            ),
            ("kairan03 re-4th-19", "19", vec![""], vec![""]),
            ("kairan03 13", "13", vec![""], vec![""]),
            (
                "bluewater91 4-2496",
                "24",
                vec!["전생했더니 슬라임이었던 건에 대하여 4 24.zip"],
                vec!["전생했더니 슬라임이었던 건에 대하여 4 24.zip"],
            ),
            (
                "bluewater91 12",
                "12",
                vec!["소녀 괴수 캐러멜리제 12.zip"],
                vec!["소녀 괴수 캐러멜리제 12.zip"],
            ),
            (
                "ehtelerosa 2026/07",
                "3",
                vec!["공각기동대 03 ToonsHub", "공각기동대 1-3 ToonsHub"],
                vec!["공각기동대 03 ToonsHub", "공각기동대 1-3 ToonsHub"],
            ),
            (
                "ehtelerosa 2025/07",
                "4",
                cleveates.to_vec(),
                cleveates.to_vec(),
            ),
            (
                "sososagak 08",
                "8",
                vec!["고양이와 용 08"],
                vec!["고양이와 용 08"],
            ),
            (
                "sososagak 0708",
                "8",
                vec!["BLACK TORCH"],
                vec!["BLACK TORCH"],
            ),
            ("beyimai x-3", "133", beyimai.clone(), vec!["133화"]),
            (
                "han1sub",
                "1",
                vec!["낯가림 심한 미망인 설녀와 저주의 반지 1화 자막"],
                vec!["낯가림 심한 미망인 설녀와 저주의 반지 1화 자막"],
            ),
            (
                "felia 1187",
                "1",
                vec!["자막 다운로드"],
                vec!["자막 다운로드"],
            ),
        ];
        assert_eq!(cases.len(), 16);
        for (post, episode, texts, expected) in cases {
            let chosen: Vec<&str> = choose(episode, &texts)
                .unwrap_or_else(|e| panic!("{post}: {e}"))
                .into_iter()
                .map(|i| texts[i])
                .collect();
            assert_eq!(chosen, expected, "{post}");
        }
    }

    #[test]
    fn the_episodes_file_and_the_fonts_are_chosen_from_a_post_of_many() {
        // C소라 2026/07/2: fonts, then 13–24화.
        let mut texts = vec!["폰트".to_owned()];
        texts.extend((13..=24).map(|n| format!("{n}화")));
        let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
        assert_eq!(choose("24", &texts), Ok(vec![0, 12]));
        assert_eq!(choose("013", &texts), Ok(vec![0, 1]));
        assert_eq!(
            choose("25", &texts),
            Err("게시물에 25화 파일이 없어요".to_owned())
        );
        assert_eq!(
            choose("SP", &texts),
            Err("게시물에 SP 파일이 없어요".to_owned())
        );
    }

    #[test]
    fn a_range_holds_its_episodes_and_an_unnamed_archive_is_taken() {
        let texts = ["폰트", "1 ~ 12화"];
        assert_eq!(choose("12", &texts), Ok(vec![0, 1]));
        assert_eq!(choose("1", &texts), Ok(vec![0, 1]));
        assert!(choose("13", &texts).is_err());
        // 에텔레로사: the episode and the range that holds it alike.
        assert_eq!(
            choose("3", &["공각기동대 03 ToonsHub", "공각기동대 1-3 ToonsHub"]),
            Ok(vec![0, 1])
        );
        assert_eq!(choose("7", &["7화", "자막 모음.zip"]), Ok(vec![0, 1]));
        assert_eq!(choose("8", &["7화", "자막 모음.zip"]), Ok(vec![1]));
    }

    #[test]
    fn a_post_of_one_unnamed_file_gives_it_and_more_unnamed_ones_are_no_choice() {
        assert_eq!(choose("19", &[""]), Ok(vec![0]));
        assert_eq!(choose("8", &["BLACK TORCH"]), Ok(vec![0]));
        assert_eq!(choose("1", &["자막 다운로드", "폰트"]), Ok(vec![0, 1]));
        // One link that names another episode is not taken.
        assert_eq!(
            choose("7", &["고양이와 용 08"]),
            Err("게시물에 7화 파일이 없어요".to_owned())
        );
        assert_eq!(
            choose("2", &["자막", "자막 다운로드"]),
            Err("게시물의 링크 2개가 몇 화인지 밝히지 않아 2화 파일을 고를 수 없어요".to_owned())
        );
        assert_eq!(
            choose("2", &["폰트"]),
            Err("게시물에 폰트 말고 받을 자막 링크가 없어요".to_owned())
        );
        assert_eq!(
            choose("2", &[]),
            Err("게시물에 받을 링크가 없어요".to_owned())
        );
    }
}
