//! The pure parts of subscribing to an Anissia anime (`docs/specs/collection.md`,
//! 방영작 구독): the titles a channel's history offers (grouped by the work
//! [`crate::release_name`] reads from them), the folder suggested for a work,
//! and the quarter a subscription belongs to.

pub mod candidates;

use std::collections::HashMap;

use trss_core::{
    calendar::{civil_from_days, DAY_MS, KST_OFFSET_MS},
    Millis,
};

use crate::{release_name::ReleaseName, store::history::HistoryItem};
use trss_anissia::Anime;

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
        let Some(work) = ReleaseName::read(&item.title).work else {
            continue;
        };
        let key = work_key(&work);
        match by_key.get_mut(&key) {
            Some(group) => {
                group.items += 1;
                if item.first_seen_at > group.latest_seen_at {
                    group.work = work;
                    group.latest_title = item.title.clone();
                    group.latest_seen_at = item.first_seen_at;
                }
            }
            None => {
                by_key.insert(
                    key,
                    TitleGroup {
                        work,
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

    /// The quarter after this one.
    pub fn next(self) -> Quarter {
        if self.number >= 4 {
            Quarter {
                year: self.year + 1,
                number: 1,
            }
        } else {
            Quarter {
                year: self.year,
                number: self.number + 1,
            }
        }
    }

    /// The quarter an anime belongs to: the one it started in, or, without a
    /// start date Anissia gave (or a readable one), the one the subscription
    /// began in.
    pub fn of_anime(anime: Option<&Anime>, subscribed_at: Millis) -> Quarter {
        anime
            .and_then(|a| a.start_date.as_deref())
            .and_then(Quarter::of_date)
            .unwrap_or_else(|| Quarter::at(subscribed_at))
    }
}

#[cfg(test)]
mod tests;
