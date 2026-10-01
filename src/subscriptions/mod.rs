//! The pure parts of subscribing to an Anissia anime (`docs/specs/collection.md`,
//! 방영작 구독): the work part of a release title (the rule's match phrase),
//! the titles a channel's history offers, the folder suggested for it, and the
//! quarter a subscription belongs to.

use std::{collections::HashMap, sync::OnceLock};

use regex::Regex;

use crate::store::history::{HistoryItem, Millis};

/// A release title read into the parts the subscription flow needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The leading `[Group]` tags, without the brackets.
    pub groups: Vec<String>,
    /// The title of the work: what the rule looks for.
    pub work: String,
    /// The episode as written (`01`, `12v2`, `01-12`), if the title has one.
    pub episode: Option<String>,
}

fn regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("a valid pattern"))
}

/// Reads the work and the episode out of a release title such as
/// `[SubsPlease] Work - 01 (1080p) [ABCD1234].mkv`. `None` when no work is left
/// once the group tags and the trailing details are taken away.
pub fn parse_release(title: &str) -> Option<Release> {
    static LEADING: OnceLock<Regex> = OnceLock::new();
    static EXT: OnceLock<Regex> = OnceLock::new();
    static DASH: OnceLock<Regex> = OnceLock::new();
    static SXXEYY: OnceLock<Regex> = OnceLock::new();
    static BARE: OnceLock<Regex> = OnceLock::new();
    static TRAILING: OnceLock<Regex> = OnceLock::new();

    let leading = regex(&LEADING, r"^\s*(?:\[([^\]]*)\]|【([^】]*)】)\s*");
    let mut rest = title.trim();
    let mut groups = Vec::new();
    while let Some(found) = leading.captures(rest) {
        let tag = found.get(1).or(found.get(2)).map_or("", |m| m.as_str());
        if !tag.trim().is_empty() {
            groups.push(tag.trim().to_owned());
        }
        rest = &rest[found.get(0).map_or(0, |m| m.end())..];
    }
    let rest = regex(&EXT, r"(?i)\.(?:mkv|mp4|avi|torrent)$").replace(rest, "");
    let rest = rest.trim();

    // `Work - 01 (1080p)`: the last ` - <number>` that something else follows.
    let dash = regex(
        &DASH,
        r"^(.*\S)\s+-\s+(\d{1,4}(?:\.\d)?(?:v\d+)?(?:\s*-\s*\d{1,4})?)(?:\s|[(\[]|$)",
    );
    // `Work S01E03`.
    let sxxeyy = regex(
        &SXXEYY,
        r"(?i)^(.*?\S)\s+S\d{1,2}E(\d{1,4}(?:\.\d)?)(?:\s|[(\[.]|$)",
    );
    // `Work 04 [BDRip ...]`: the number after the title with no dash.
    let bare = regex(&BARE, r"^(.*\S)\s+(\d{1,3}(?:v\d+)?)\s*(?:[(\[].*)?$");
    for pattern in [dash, sxxeyy, bare] {
        if let Some(found) = pattern.captures(rest) {
            let work = found[1].trim();
            if !work.is_empty() {
                return Some(Release {
                    groups,
                    work: work.to_owned(),
                    episode: Some(found[2].to_owned()),
                });
            }
        }
    }

    // No episode: a batch or a movie. Take the title without its trailing
    // `(…)` and `[…]` details.
    let trailing = regex(&TRAILING, r"\s*(?:\([^)]*\)|\[[^\]]*\])\s*$");
    let mut work = rest;
    while let Some(m) = trailing.find(work) {
        work = &work[..m.start()];
    }
    let work = work.trim();
    (!work.is_empty()).then(|| Release {
        groups,
        work: work.to_owned(),
        episode: None,
    })
}

/// How two spellings of a work are told to be one: case and runs of spaces do
/// not count.
pub fn work_key(work: &str) -> String {
    work.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The recorded items of one work in a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleGroup {
    /// The work, as the newest item writes it: the rule's match phrase.
    pub work: String,
    /// The newest item's full title, to tell releases of the work apart.
    pub latest_title: String,
    /// How many recorded items the work has.
    pub items: usize,
    /// When the newest item was first seen.
    pub latest_seen_at: Millis,
}

/// Groups recorded items by their work, the work with the newest item first.
/// Items whose title has no work part are left out.
pub fn title_groups<'a>(items: impl IntoIterator<Item = &'a HistoryItem>) -> Vec<TitleGroup> {
    let mut by_key: HashMap<String, TitleGroup> = HashMap::new();
    for item in items {
        let Some(release) = parse_release(&item.title) else {
            continue;
        };
        let key = work_key(&release.work);
        match by_key.get_mut(&key) {
            Some(group) => {
                group.items += 1;
                if item.first_seen_at > group.latest_seen_at {
                    group.work = release.work;
                    group.latest_title = item.title.clone();
                    group.latest_seen_at = item.first_seen_at;
                }
            }
            None => {
                by_key.insert(
                    key,
                    TitleGroup {
                        work: release.work,
                        latest_title: item.title.clone(),
                        items: 1,
                        latest_seen_at: item.first_seen_at,
                    },
                );
            }
        }
    }
    let mut groups: Vec<TitleGroup> = by_key.into_values().collect();
    groups.sort_by(|a, b| {
        b.latest_seen_at
            .cmp(&a.latest_seen_at)
            .then_with(|| a.work.cmp(&b.work))
    });
    groups
}

/// The longest folder name taken, in bytes, so a name stays well inside what
/// file systems allow.
const MAX_FOLDER_BYTES: usize = 120;

/// `<Work>/Season 01` for a work, the layout `trname` reads seasons from. The
/// characters a folder name cannot hold are dropped. `None` when nothing is left.
pub fn folder_suggestion(work: &str) -> Option<String> {
    let cleaned: String = work
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut name = String::new();
    for c in joined.chars() {
        if name.len() + c.len_utf8() > MAX_FOLDER_BYTES {
            break;
        }
        name.push(c);
    }
    let name = name.trim_matches(|c: char| c == '.' || c.is_whitespace());
    (!name.is_empty()).then(|| format!("{name}/Season 01"))
}

/// A calendar quarter in Asia/Seoul, as Anissia numbers them (1분기 is
/// January to March).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Quarter {
    pub year: i32,
    /// 1 to 4.
    pub number: u8,
}

const KST_OFFSET_MS: i64 = 9 * 60 * 60 * 1000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The civil date of a day count since 1970-01-01 (proleptic Gregorian).
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
    (year, month, day)
}

impl Quarter {
    pub fn of_month(year: i32, month: u32) -> Option<Quarter> {
        (1..=12).contains(&month).then(|| Quarter {
            year,
            number: ((month - 1) / 3 + 1) as u8,
        })
    }

    /// The quarter a moment (Unix ms) is in, in Asia/Seoul.
    pub fn at(ms: Millis) -> Quarter {
        let (year, month, _) = civil_from_days((ms + KST_OFFSET_MS).div_euclid(DAY_MS));
        Quarter::of_month(year, month).expect("a civil month")
    }

    /// The quarter of a date Anissia gave, `YYYY-MM-DD` or `YYYY-MM`.
    pub fn of_date(date: &str) -> Option<Quarter> {
        let mut parts = date.splitn(3, '-');
        let year: i32 = parts.next()?.parse().ok()?;
        let month: u32 = parts.next()?.parse().ok()?;
        Quarter::of_month(year, month)
    }
}

#[cfg(test)]
mod tests;
