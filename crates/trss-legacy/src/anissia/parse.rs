//! Reading Anissia's JSON into the app's types.
//!
//! Anissia's answers are not documented. What is relied on was read from the
//! live API on 2026-10-01: every answer is `{"code": "ok", "data": [...]}`
//! (`{"code": "error", "message": "..."}` otherwise); a schedule entry has
//! `week` (a string), `animeNo`, `status`, `time`, `subject`,
//! `originalSubject`, `genres` (comma separated), `startDate`, `endDate`,
//! `website` and `captionCount`; a caption has `episode`, `updDt`, `website`
//! and `name`. Unknown fields are ignored, and `data` may also be an object
//! with the list in `content` (the shape of the paged `recent` endpoint).
//!
//! The values are tolerated, not trusted: an entry without a number or a
//! title is left out, a date that is not one is unknown, and a website that is
//! not an `http(s)` address is dropped.

use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::store::anissia::{Anime, WEEK_UPCOMING};
use trss_core::Millis;

/// An entry of a week's schedule, as the schedule screen shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleEntry {
    pub anime_no: i64,
    /// 0 (Sunday) to 6 (Saturday), 7 (`기타`) or 8 (`신작`).
    pub week: u8,
    /// Anissia's `ON` or `OFF`.
    pub status: String,
    /// `HH:MM` in Asia/Seoul. `신작` entries carry a date there instead, which
    /// is not a time and is left out.
    pub air_time: Option<String>,
    pub subject: String,
    pub original_subject: Option<String>,
    pub genres: Vec<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub website: Option<String>,
    pub caption_count: u32,
}

impl ScheduleEntry {
    /// The part of the entry the app keeps for a subscribed anime.
    pub fn snapshot(&self, fetched_at: Millis) -> Anime {
        Anime {
            anime_no: self.anime_no,
            subject: self.subject.clone(),
            original_subject: self.original_subject.clone(),
            week: self.week,
            air_time: self.air_time.clone(),
            start_date: self.start_date.clone(),
            end_date: self.end_date.clone(),
            status: self.status.clone(),
            fetched_at,
        }
    }
}

/// One caption (a subtitle release) of an anime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caption {
    /// The episode as Anissia writes it (`"4"`, `"0"`, ...).
    pub episode: String,
    /// When the caption was updated, `YYYY-MM-DDTHH:MM:SS` in Asia/Seoul as
    /// Anissia gives it; `None` when it is not that.
    pub updated_at: Option<String>,
    /// Where the creator published it.
    pub website: Option<String>,
    /// The creator's name.
    pub creator: String,
}

/// A subtitle creator of an anime and what they last released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Creator {
    pub name: String,
    /// How many captions the creator has for the anime.
    pub captions: usize,
    /// When the creator last updated a caption (see [`Caption::updated_at`]).
    pub last_updated_at: Option<String>,
}

/// The creators of `captions`, the most recently active first (then by name).
/// A caption without a creator's name names nobody.
pub fn creators(captions: &[Caption]) -> Vec<Creator> {
    let mut out: Vec<Creator> = Vec::new();
    for caption in captions {
        let name = caption.creator.trim();
        if name.is_empty() {
            continue;
        }
        match out.iter_mut().find(|c| c.name == name) {
            Some(creator) => {
                creator.captions += 1;
                if caption.updated_at > creator.last_updated_at {
                    creator.last_updated_at = caption.updated_at.clone();
                }
            }
            None => out.push(Creator {
                name: name.to_owned(),
                captions: 1,
                last_updated_at: caption.updated_at.clone(),
            }),
        }
    }
    out.sort_by(|a, b| {
        b.last_updated_at
            .cmp(&a.last_updated_at)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// Why an answer cannot be read.
#[derive(Debug, PartialEq, Eq)]
pub struct Unreadable(pub String);

#[derive(Deserialize)]
struct Envelope {
    code: Option<String>,
    message: Option<String>,
    data: Option<Value>,
}

/// The list in an answer's `data`, if the answer says it is `ok`.
pub fn list_of(body: &[u8]) -> Result<Vec<Value>, Unreadable> {
    let envelope: Envelope =
        serde_json::from_slice(body).map_err(|e| Unreadable(format!("unexpected shape: {e}")))?;
    if envelope.code.as_deref() != Some("ok") {
        let said = envelope
            .message
            .map(|m| m.chars().take(120).collect::<String>())
            .unwrap_or_default();
        return Err(Unreadable(format!(
            "the answer is not ok ({:?}): {said}",
            envelope.code.unwrap_or_default()
        )));
    }
    match envelope.data {
        Some(Value::Array(list)) => Ok(list),
        Some(Value::Object(mut object)) => match object.remove("content") {
            Some(Value::Array(list)) => Ok(list),
            _ => Err(Unreadable("the answer's data has no list".to_owned())),
        },
        _ => Err(Unreadable("the answer has no data".to_owned())),
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// A web address, if it is an `http(s)` one.
fn address(value: Option<&Value>) -> Option<String> {
    let text = text(value)?;
    let url = Url::parse(&text).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(text)
}

fn days_in(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        _ => 28,
    }
}

/// `YYYY-MM-DD` for a date Anissia gave; `YYYY-MM` when its day is not a day
/// of the month (Anissia writes `2027-01-99` for a start known to the month);
/// `None` when even the month is not one.
pub fn normalize_date(value: &str) -> Option<String> {
    let value = value.trim();
    let mut parts = value.splitn(3, '-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next());
    let digits = |s: &str, len: usize| s.len() == len && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(year, 4) || !digits(month, 2) {
        return None;
    }
    let (y, m): (i32, u32) = (year.parse().ok()?, month.parse().ok()?);
    if !(1..=12).contains(&m) || y < 1900 {
        return None;
    }
    match day {
        Some(day) if digits(day, 2) => {
            let d: u32 = day.parse().ok()?;
            if (1..=days_in(y, m)).contains(&d) {
                Some(format!("{year}-{month}-{day}"))
            } else {
                Some(format!("{year}-{month}"))
            }
        }
        Some(_) => None,
        None => Some(format!("{year}-{month}")),
    }
}

/// `HH:MM` for a time of day; Anissia's schedule goes past 24 o'clock for
/// late-night programmes, so hours up to 47 are kept. `None` for anything else
/// (an empty time, or the date a `신작` entry carries there).
pub fn normalize_time(value: &str) -> Option<String> {
    let (hour, minute) = value.trim().split_once(':')?;
    if hour.is_empty() || hour.len() > 2 || minute.len() != 2 {
        return None;
    }
    if !hour
        .bytes()
        .chain(minute.bytes())
        .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let (h, m): (u32, u32) = (hour.parse().ok()?, minute.parse().ok()?);
    (h <= 47 && m < 60).then(|| format!("{h:02}:{m:02}"))
}

fn week_of(value: Option<&Value>) -> Option<u8> {
    let week = match value? {
        Value::String(s) => s.trim().parse::<u8>().ok()?,
        Value::Number(n) => u8::try_from(n.as_u64()?).ok()?,
        _ => return None,
    };
    (week <= WEEK_UPCOMING).then_some(week)
}

/// One schedule entry of the week `asked`; `None` for an entry that cannot be
/// used (no anime number or title).
fn entry(raw: &Value, asked: u8) -> Option<ScheduleEntry> {
    let anime_no = raw.get("animeNo")?.as_i64().filter(|n| *n > 0)?;
    let subject = text(raw.get("subject"))?;
    let genres = text(raw.get("genres"))
        .map(|g| {
            g.split(',')
                .map(str::trim)
                .filter(|g| !g.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Some(ScheduleEntry {
        anime_no,
        week: week_of(raw.get("week")).unwrap_or(asked),
        status: text(raw.get("status")).unwrap_or_else(|| "ON".to_owned()),
        air_time: text(raw.get("time")).and_then(|t| normalize_time(&t)),
        subject,
        original_subject: text(raw.get("originalSubject")),
        genres,
        start_date: text(raw.get("startDate")).and_then(|d| normalize_date(&d)),
        end_date: text(raw.get("endDate")).and_then(|d| normalize_date(&d)),
        website: address(raw.get("website")),
        caption_count: raw
            .get("captionCount")
            .and_then(Value::as_u64)
            .map_or(0, |n| n.min(u32::MAX as u64) as u32),
    })
}

/// The usable entries of a week's schedule, each anime once. An answer that
/// lists entries but none usable is unreadable: it is not the schedule.
pub fn schedule(list: &[Value], asked: u8) -> Result<Vec<ScheduleEntry>, Unreadable> {
    let mut out: Vec<ScheduleEntry> = Vec::with_capacity(list.len());
    for raw in list {
        if let Some(entry) = entry(raw, asked) {
            if out.iter().all(|e| e.anime_no != entry.anime_no) {
                out.push(entry);
            }
        }
    }
    if out.is_empty() && !list.is_empty() {
        return Err(Unreadable("no entry of the schedule is usable".to_owned()));
    }
    Ok(out)
}

fn updated_at(value: Option<&Value>) -> Option<String> {
    let text = text(value)?;
    let bytes = text.as_bytes();
    let shape = b"dddd-dd-ddTdd:dd:dd";
    let fits = bytes.len() >= shape.len()
        && bytes.iter().zip(shape).all(|(b, s)| match s {
            b'd' => b.is_ascii_digit(),
            other => b == other,
        });
    fits.then(|| text[..shape.len()].to_owned())
}

/// The usable captions of an anime: those that name a creator.
pub fn captions(list: &[Value]) -> Vec<Caption> {
    list.iter()
        .filter_map(|raw| {
            Some(Caption {
                episode: text(raw.get("episode")).unwrap_or_default(),
                updated_at: updated_at(raw.get("updDt")),
                website: address(raw.get("website")),
                creator: text(raw.get("name"))?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn dates_keep_what_is_known_and_unknown_days_fall_back_to_the_month() {
        assert_eq!(normalize_date("2026-10-07").as_deref(), Some("2026-10-07"));
        assert_eq!(
            normalize_date(" 2026-10-07 ").as_deref(),
            Some("2026-10-07")
        );
        // Anissia's own spelling of a day nobody knows yet.
        assert_eq!(normalize_date("2027-01-99").as_deref(), Some("2027-01"));
        assert_eq!(normalize_date("2027-01-00").as_deref(), Some("2027-01"));
        assert_eq!(normalize_date("2026-02-30").as_deref(), Some("2026-02"));
        assert_eq!(normalize_date("2028-02-29").as_deref(), Some("2028-02-29"));
        assert_eq!(normalize_date("2027-02-29").as_deref(), Some("2027-02"));
        assert_eq!(normalize_date("2027-01").as_deref(), Some("2027-01"));
        for bad in [
            "",
            "2027",
            "2027-13-01",
            "2027-00-10",
            "27-01-10",
            "abcd-ef-gh",
            "2027-1-5",
            "2027-01-1x",
        ] {
            assert_eq!(normalize_date(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn times_are_clock_times_and_a_date_is_not_one() {
        assert_eq!(normalize_time("22:00").as_deref(), Some("22:00"));
        assert_eq!(normalize_time("1:05").as_deref(), Some("01:05"));
        assert_eq!(normalize_time("25:30").as_deref(), Some("25:30"));
        for bad in [
            "",
            "2027-01-09",
            "24",
            "12:5",
            "12:60",
            "48:00",
            "ab:cd",
            "-1:00",
        ] {
            assert_eq!(normalize_time(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_schedule_reads_the_live_shape_and_leaves_out_what_is_not_usable() {
        let list = [
            json!({
                "week": "3", "animeNo": 3527, "status": "ON", "time": "00:00",
                "subject": "추방된 치트 부여 마술사", "originalSubject": "追放された",
                "genres": "판타지, 액션,", "captionCount": 2,
                "startDate": "2026-10-07", "endDate": "",
                "website": "https://sh-anime.shochiku.co.jp/chiifuyo-anime", "x": "https://x.com/a",
            }),
            // A 신작 entry carries its date where the time goes.
            json!({
                "week": "8", "animeNo": 3593, "status": "ON", "time": "2027-01-09",
                "subject": "진 사무라이전", "originalSubject": "", "genres": "",
                "captionCount": 0, "startDate": "2027-01-99", "endDate": "",
                "website": "javascript:alert(1)",
            }),
            json!({ "week": "3", "animeNo": 0, "subject": "번호 없음" }),
            json!({ "week": "3", "animeNo": 5, "subject": "  " }),
            json!({ "week": "3", "animeNo": 3527, "subject": "again" }),
            json!("not an object"),
        ];
        let entries = schedule(&list, 3).unwrap();
        assert_eq!(entries.len(), 2);
        let first = &entries[0];
        assert_eq!(first.anime_no, 3527);
        assert_eq!(first.week, 3);
        assert_eq!(first.air_time.as_deref(), Some("00:00"));
        assert_eq!(first.original_subject.as_deref(), Some("追放された"));
        assert_eq!(first.genres, ["판타지", "액션"]);
        assert_eq!(first.start_date.as_deref(), Some("2026-10-07"));
        assert_eq!(first.end_date, None);
        assert_eq!(first.caption_count, 2);
        assert!(first.website.is_some());
        let upcoming = &entries[1];
        assert_eq!(upcoming.week, 8);
        assert_eq!(upcoming.air_time, None);
        assert_eq!(upcoming.original_subject, None);
        assert_eq!(upcoming.start_date.as_deref(), Some("2027-01"));
        assert_eq!(upcoming.website, None);
    }

    #[test]
    fn an_answer_with_entries_but_none_usable_is_not_a_schedule() {
        assert!(schedule(&[json!({ "foo": 1 }), json!(7)], 2).is_err());
        assert_eq!(schedule(&[], 2).unwrap(), []);
    }

    #[test]
    fn an_entry_without_a_valid_week_takes_the_week_that_was_asked() {
        let list = [json!({ "animeNo": 1, "subject": "a", "week": "x" })];
        assert_eq!(schedule(&list, 5).unwrap()[0].week, 5);
        let list = [json!({ "animeNo": 1, "subject": "a", "week": 4 })];
        assert_eq!(schedule(&list, 5).unwrap()[0].week, 4);
        let list = [json!({ "animeNo": 1, "subject": "a", "week": "9" })];
        assert_eq!(schedule(&list, 5).unwrap()[0].week, 5);
    }

    #[test]
    fn the_envelope_is_ok_with_a_list_or_content_and_anything_else_is_refused() {
        let ok = br#"{"code":"ok","data":[{"a":1}]}"#;
        assert_eq!(list_of(ok).unwrap().len(), 1);
        let paged = br#"{"code":"ok","data":{"content":[{"a":1},{"a":2}],"page":0}}"#;
        assert_eq!(list_of(paged).unwrap().len(), 2);
        for bad in [
            &br#"{"code":"error","message":"No static resource anime/schedule/9"}"#[..],
            br#"{"data":[]}"#,
            br#"{"code":"ok"}"#,
            br#"{"code":"ok","data":"text"}"#,
            br#"{"code":"ok","data":{"rows":[]}}"#,
            b"<html>",
            b"",
        ] {
            assert!(list_of(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
        let message = list_of(br#"{"code":"error","message":"nope"}"#)
            .unwrap_err()
            .0;
        assert!(message.contains("nope"), "{message}");
    }

    #[test]
    fn captions_need_a_creator_and_the_creators_are_ordered_by_recent_work() {
        let list = [
            json!({ "episode": "3", "updDt": "2026-08-07T20:52:00", "website": "https://a.test/1", "name": "에텔레로사" }),
            json!({ "episode": "4", "updDt": "2026-08-14T20:52:00", "website": "", "name": "에텔레로사" }),
            json!({ "episode": "1", "updDt": "2026-09-01T10:00:00", "website": "ftp://x", "name": " C소라 " }),
            json!({ "episode": "0", "updDt": "garbage", "name": "에루샤" }),
            json!({ "episode": "2", "updDt": "2026-09-02T00:00:00", "name": "" }),
            json!({ "episode": "2", "updDt": "2026-09-02T00:00:00" }),
        ];
        let captions = captions(&list);
        assert_eq!(captions.len(), 4);
        assert_eq!(captions[0].website.as_deref(), Some("https://a.test/1"));
        assert_eq!(captions[1].website, None);
        assert_eq!(captions[2].website, None);
        assert_eq!(captions[2].creator, "C소라");
        assert_eq!(captions[3].updated_at, None);

        let creators = creators(&captions);
        let names: Vec<_> = creators.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["C소라", "에텔레로사", "에루샤"]);
        assert_eq!(creators[1].captions, 2);
        assert_eq!(
            creators[1].last_updated_at.as_deref(),
            Some("2026-08-14T20:52:00")
        );
        assert_eq!(creators[2].last_updated_at, None);
    }

    #[test]
    fn an_entry_without_a_status_is_on_and_one_that_gives_off_is_off() {
        let status = |raw: Value| entry(&raw, 2).unwrap().status;
        assert_eq!(status(json!({ "animeNo": 9, "subject": "s" })), "ON");
        assert_eq!(
            status(json!({ "animeNo": 9, "subject": "s", "status": "OFF" })),
            "OFF"
        );
    }

    #[test]
    fn the_snapshot_is_the_part_of_an_entry_the_app_keeps() {
        let entry = entry(
            &json!({ "week": "2", "animeNo": 9, "subject": "s", "time": "21:30",
                     "originalSubject": "o", "status": "OFF", "startDate": "2026-07-01",
                     "endDate": "2026-09-30", "genres": "g", "captionCount": 3 }),
            2,
        )
        .unwrap();
        let snapshot = entry.snapshot(77);
        assert_eq!(snapshot.anime_no, 9);
        assert_eq!(snapshot.week, 2);
        assert_eq!(snapshot.air_time.as_deref(), Some("21:30"));
        assert_eq!(snapshot.status, "OFF");
        assert_eq!(snapshot.end_date.as_deref(), Some("2026-09-30"));
        assert_eq!(snapshot.fetched_at, 77);
    }
}
