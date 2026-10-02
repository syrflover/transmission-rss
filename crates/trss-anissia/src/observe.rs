//! Reading Anissia's caption lines for observation (`docs/specs/subtitles.md`,
//! 자막 후보 조회): the recent list (`/anime/caption/recent/<page>`) and the
//! captions of one anime (`/anime/caption/animeNo/<n>`).
//!
//! Observed on 2026-10-02 (the shapes the fake server of [`crate::fake`]
//! answers with):
//!
//! - `recent/<page>`: `page` counts from 0. `data` is Spring's page:
//!   `content` (up to `size` = 20 lines), `number`, `size`, `numberOfElements`,
//!   `totalElements`, `totalPages`, `first`, `last`, `empty` and `pageable`.
//!   A line is `animeNo`, `subject`, `episode`, `updDt`, `website`, `name`.
//!   The list holds the lines updated in the last 90 days that have an address,
//!   newest `updDt` first: 70 lines on 4 pages, the 4th with 10 and `last`
//!   `true`. A page past the end is `content: []` with `empty` `true`, still
//!   HTTP 200 and `code` `ok` (page 4 and page 5 answered alike).
//! - `animeNo/<n>`: `data` is a plain list of lines without `animeNo` and
//!   `subject`; an anime Anissia does not know is `[]`.
//! - `episode` is a string (`"12"`, `"0"`), `updDt` is
//!   `YYYY-MM-DDTHH:MM:SS` without a zone, `website` is the post's address
//!   and `name` is the creator's display name.
//!
//! The values are tolerated, not trusted: a line that lacks a field, whose
//! field is not a string, whose field is longer than the bound below, or whose
//! address is not an `http(s)` one is left out, and counted so the caller can
//! say so. `episode` is kept exactly as written and never read as a number.
//! `updDt` is kept as written too, together with the moment it names when it
//! names one.

use serde_json::Value;
use url::Url;

use crate::parse::{list_of, Unreadable};
use trss_core::{
    calendar::{days_from_civil, KST_OFFSET_MS},
    Millis,
};

/// The longest creator name read.
pub const MAX_NAME_CHARS: usize = 128;
/// The longest episode text read.
pub const MAX_EPISODE_CHARS: usize = 64;
/// The longest post address read.
pub const MAX_ADDRESS_CHARS: usize = 2048;
/// The longest `updDt` read.
pub const MAX_UPDATED_CHARS: usize = 64;

/// One line of Anissia's captions: what a creator last released for an anime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptionLine {
    /// Anissia's `animeNo`; the lines of a recent page carry it, the lines of
    /// one anime's captions do not (the caller knows it).
    pub anime_no: Option<i64>,
    /// The creator's display name, as written.
    pub creator: String,
    /// The episode as written (`"0"`, `"13.5"`, ...), never read as a number.
    pub episode: String,
    /// The post's `http(s)` address.
    pub website: String,
    /// `updDt` as written.
    pub updated: String,
    /// The moment `updDt` names (Unix ms); `None` when it is not a date and
    /// time. A value without a zone is Asia/Seoul.
    pub updated_at: Option<Millis>,
}

/// One page of the recent list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentPage {
    /// The usable lines.
    pub lines: Vec<CaptionLine>,
    /// How many lines the page held, usable or not. A page that holds none is
    /// the empty page that ends the list; a page whose lines are all unusable
    /// is not.
    pub rows: usize,
}

impl RecentPage {
    /// Lines the page held that could not be used.
    pub fn skipped(&self) -> usize {
        self.rows - self.lines.len()
    }
}

/// A text field, as written, within `max` characters.
fn field(raw: &Value, key: &str, max: usize) -> Option<String> {
    let text = raw.get(key)?.as_str()?;
    (text.chars().count() <= max).then(|| text.to_owned())
}

fn line(raw: &Value) -> Option<CaptionLine> {
    let creator = field(raw, "name", MAX_NAME_CHARS).filter(|n| !n.trim().is_empty())?;
    let website = field(raw, "website", MAX_ADDRESS_CHARS)?;
    let website = website.trim();
    let url = Url::parse(website).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return None;
    }
    let updated = field(raw, "updDt", MAX_UPDATED_CHARS)?;
    Some(CaptionLine {
        anime_no: raw
            .get("animeNo")
            .and_then(Value::as_i64)
            .filter(|n| *n > 0),
        creator,
        episode: field(raw, "episode", MAX_EPISODE_CHARS)?,
        website: website.to_owned(),
        updated_at: updated_at(&updated),
        updated,
    })
}

/// The usable lines of `list`.
fn lines_of(list: &[Value]) -> Vec<CaptionLine> {
    list.iter().filter_map(line).collect()
}

/// The page an answer of `/anime/caption/recent/<page>` holds. An answer that
/// is not a page with a `content` list is refused rather than guessed at.
pub fn recent_page(body: &[u8]) -> Result<RecentPage, Unreadable> {
    let list = list_of(body)?;
    let lines: Vec<CaptionLine> = lines_of(&list)
        .into_iter()
        // A line of the recent list names its anime.
        .filter(|l| l.anime_no.is_some())
        .collect();
    Ok(RecentPage {
        lines,
        rows: list.len(),
    })
}

/// The usable lines of an answer of `/anime/caption/animeNo/<n>`, and how many
/// lines it held.
pub fn anime_lines(body: &[u8]) -> Result<(Vec<CaptionLine>, usize), Unreadable> {
    let list = list_of(body)?;
    Ok((lines_of(&list), list.len()))
}

fn digits(text: &str, len: usize) -> Option<i64> {
    (text.len() == len && text.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse().unwrap())
}

fn days_in(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        _ => 28,
    }
}

/// The offset of a zone suffix in ms: `Z`, `+09:00`, `-0500`, `+09`; `None` for
/// anything else.
fn zone_offset(zone: &str) -> Option<i64> {
    if zone == "Z" || zone == "z" {
        return Some(0);
    }
    let sign = match zone.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let rest = &zone[1..];
    let (hours, minutes) = match rest.len() {
        2 => (digits(rest, 2)?, 0),
        4 => (digits(&rest[..2], 2)?, digits(&rest[2..], 2)?),
        5 if rest.as_bytes()[2] == b':' => (digits(&rest[..2], 2)?, digits(&rest[3..], 2)?),
        _ => return None,
    };
    (hours <= 23 && minutes <= 59).then(|| sign * (hours * 60 + minutes) * 60_000)
}

/// The moment an `updDt` names, in Unix ms.
///
/// `YYYY-MM-DD HH:mm:ss` (the example of Anissia's documentation) and
/// `YYYY-MM-DDTHH:mm:ss` (what the API gives), with fractional seconds
/// tolerated. A value without a zone is Asia/Seoul (UTC+9, all year), whatever
/// the server's own zone is: nothing here asks the system for one. A value
/// with `Z` or an offset keeps it. Anything else, an impossible date included,
/// is `None`.
pub fn updated_at(text: &str) -> Option<Millis> {
    let text = text.trim();
    let (date, rest) = text.split_once(['T', ' '])?;
    let mut d = date.splitn(3, '-');
    let year = digits(d.next()?, 4)? as i32;
    let month = digits(d.next()?, 2)? as u32;
    let day = digits(d.next()?, 2)? as u32;
    if year < 1970 || !(1..=12).contains(&month) || day == 0 || day > days_in(year, month) {
        return None;
    }
    let zone_at = rest.find(['Z', 'z', '+', '-']).unwrap_or(rest.len());
    let (clock, zone) = rest.split_at(zone_at);
    let offset = if zone.is_empty() {
        KST_OFFSET_MS
    } else {
        zone_offset(zone)?
    };
    let (hms, fraction) = match clock.split_once('.') {
        Some((hms, fraction)) => (hms, Some(fraction)),
        None => (clock, None),
    };
    if fraction.is_some_and(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let mut t = hms.splitn(3, ':');
    let hour = digits(t.next()?, 2)?;
    let minute = digits(t.next()?, 2)?;
    let second = digits(t.next()?, 2)?;
    if t.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000 - offset)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// 2026-10-02 12:00:00 UTC.
    const NOON_UTC: Millis = 1_790_942_400_000;

    #[test]
    fn a_value_without_a_zone_is_seoul_time_in_either_spelling() {
        assert_eq!(updated_at("2026-10-02 21:00:00"), Some(NOON_UTC));
        assert_eq!(updated_at("2026-10-02T21:00:00"), Some(NOON_UTC));
        assert_eq!(updated_at(" 2026-10-02T21:00:00 "), Some(NOON_UTC));
        assert_eq!(updated_at("2026-10-02T21:00:00.500"), Some(NOON_UTC));
    }

    #[test]
    fn a_value_with_a_zone_keeps_it() {
        assert_eq!(updated_at("2026-10-02T12:00:00Z"), Some(NOON_UTC));
        assert_eq!(updated_at("2026-10-02T21:00:00+09:00"), Some(NOON_UTC));
        assert_eq!(updated_at("2026-10-02T21:00:00+0900"), Some(NOON_UTC));
        assert_eq!(updated_at("2026-10-02T21:00:00+09"), Some(NOON_UTC));
        assert_eq!(updated_at("2026-10-02T07:00:00-05:00"), Some(NOON_UTC));
    }

    #[test]
    fn what_is_not_a_date_and_time_is_not_an_instant() {
        for bad in [
            "",
            "yesterday",
            "2026-10-02",
            "2026-10-02T21:00",
            "2026-13-02T21:00:00",
            "2026-02-30T21:00:00",
            "2026-10-02T24:00:00",
            "2026-10-02T21:60:00",
            "2026-10-02T21:00:60",
            "0000-00-00T00:00:00",
            "1969-12-31T23:59:59",
            "2026-10-02T21:00:00+9",
            "2026-10-02T21:00:00 KST",
            "2026-10-02T21:00:00.",
            "20261002T210000",
        ] {
            assert_eq!(updated_at(bad), None, "{bad:?}");
        }
        // A leap day is a day.
        assert!(updated_at("2028-02-29T00:00:00").is_some());
        assert_eq!(updated_at("2027-02-29T00:00:00"), None);
    }

    #[test]
    fn a_recent_page_is_read_as_observed() {
        // Shortened from `/anime/caption/recent/0` of 2026-10-02.
        let body = json!({"code": "ok", "data": {
            "content": [
                {"animeNo": 3550, "subject": "얼음 성벽 2기", "episode": "15",
                 "updDt": "2026-10-02T11:17:00",
                 "website": "https://csora556.blogspot.com/2026/10/2.html", "name": "C소라"},
                {"animeNo": 3010, "subject": "마법기사 레이어스", "episode": "0",
                 "updDt": "2026-10-01T17:00:00",
                 "website": "https://www.youtube.com/watch?v=1I44u3JN0H4", "name": "하느"},
            ],
            "empty": false, "first": true, "last": false, "number": 0, "size": 20,
            "numberOfElements": 2, "totalElements": 70, "totalPages": 4,
            "pageable": {"offset": 0, "pageNumber": 0, "pageSize": 20, "paged": true,
                         "unpaged": false, "sort": {"empty": true, "sorted": false, "unsorted": true}},
        }});
        let page = recent_page(body.to_string().as_bytes()).unwrap();
        assert_eq!((page.rows, page.skipped()), (2, 0));
        assert_eq!(
            page.lines[0],
            CaptionLine {
                anime_no: Some(3550),
                creator: "C소라".into(),
                episode: "15".into(),
                website: "https://csora556.blogspot.com/2026/10/2.html".into(),
                updated: "2026-10-02T11:17:00".into(),
                // 11:17 in Seoul is 02:17 UTC.
                updated_at: Some(NOON_UTC - (9 * 60 + 43) * 60_000),
            }
        );
        assert_eq!(page.lines[1].episode, "0");
    }

    #[test]
    fn the_empty_page_holds_no_rows_and_is_not_an_error() {
        let body = json!({"code": "ok", "data": {
            "content": [], "empty": true, "first": false, "last": true, "number": 4,
            "numberOfElements": 0, "size": 20, "totalElements": 70, "totalPages": 4}});
        let page = recent_page(body.to_string().as_bytes()).unwrap();
        assert_eq!((page.rows, page.lines.len()), (0, 0));
    }

    #[test]
    fn a_line_that_cannot_be_used_is_left_out_and_counted_without_ending_the_list() {
        let ok = |no: i64| {
            json!({"animeNo": no, "episode": "1", "updDt": "2026-10-02T11:17:00",
                   "website": "https://a.test/1", "name": "제작자"})
        };
        let mut no_name = ok(2);
        no_name["name"] = json!("  ");
        let mut script = ok(3);
        script["website"] = json!("javascript:alert(1)");
        let mut no_address = ok(4);
        no_address["website"] = json!("");
        let mut number_episode = ok(5);
        number_episode["episode"] = json!(13.5);
        let mut long_episode = ok(6);
        long_episode["episode"] = json!("9".repeat(MAX_EPISODE_CHARS + 1));
        let mut no_anime = ok(7);
        no_anime.as_object_mut().unwrap().remove("animeNo");
        let body = json!({"code": "ok", "data": {"content": [
            ok(1), no_name, script, no_address, number_episode, long_episode, no_anime,
            json!("text"),
        ]}});
        let page = recent_page(body.to_string().as_bytes()).unwrap();
        assert_eq!(page.rows, 8);
        assert_eq!(page.skipped(), 7);
        assert_eq!(page.lines[0].anime_no, Some(1));
    }

    #[test]
    fn episodes_are_kept_as_written_and_an_unreadable_date_is_kept_with_its_text() {
        let body = json!({"code": "ok", "data": [
            {"episode": "13.5", "updDt": "2026-10-02 21:00:00", "website": "https://a.test/1", "name": "가"},
            {"episode": "0", "updDt": "soon", "website": "https://a.test/2", "name": "나"},
            {"episode": " 07 ", "updDt": "2026-10-02T21:00:00", "website": "https://a.test/3", "name": "다"},
        ]});
        let (lines, rows) = anime_lines(body.to_string().as_bytes()).unwrap();
        assert_eq!(rows, 3);
        assert_eq!(lines[0].episode, "13.5");
        assert_eq!(lines[0].updated_at, Some(NOON_UTC));
        assert_eq!(lines[0].anime_no, None);
        assert_eq!(lines[1].episode, "0");
        assert_eq!(
            (lines[1].updated.as_str(), lines[1].updated_at),
            ("soon", None)
        );
        assert_eq!(lines[2].episode, " 07 ");
    }

    #[test]
    fn an_answer_that_is_not_the_api_s_is_refused() {
        assert!(recent_page(b"{}").is_err());
        assert!(recent_page(br#"{"code":"error","message":"x"}"#).is_err());
        assert!(recent_page(br#"{"code":"ok","data":{"content":"x"}}"#).is_err());
        assert!(anime_lines(b"not json").is_err());
    }
}
