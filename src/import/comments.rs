//! Reading the comments above the rules of a legacy channels YAML.
//!
//! A person who followed anime with the old file wrote, above each rule, where
//! the anime is on Anissia and which subtitle creator to follow
//! (`docs/specs/settings.md`, 기존 YAML). The YAML parser drops comments, so
//! this module scans the file's lines itself: [`rule_comments`] finds the
//! comment lines directly above each rule (the last two are read), and [`read`] turns one rule's
//! comment into a [`Reading`].
//!
//! # The comment format
//!
//! The format is the author's own convention, verified on 2026-10-01 against
//! the author's channel configuration (every commented rule there has this
//! shape). Directly above a rule's `- ` line, with no blank line between,
//! stand two comment lines:
//!
//! ```yaml
//! # Mon. 23:30. <creator name>
//! # https://anissia.net/anime?animeNo=<number>
//! ```
//!
//! - **Line 1** is the weekday, the time and the creator, separated by `. `
//!   (a period and a space). The weekday is an English three-letter
//!   abbreviation (`Mon` `Tue` `Wed` `Thu` `Fri` `Sat` `Sun`), the time is
//!   `HH:MM`, and the creator is everything after the second separator, trimmed,
//!   so it may hold spaces and periods. A line 1 that does not have this shape
//!   (another weekday word, a time that is not `HH:MM`), or whose creator is
//!   empty or `미정`, says no creator: the rule is *address only*, not
//!   unreadable. The weekday and time are not authoritative: Anissia's own
//!   values are shown, and [`Airs`] is only what the review falls back on when
//!   Anissia cannot be reached.
//! - **Line 2** is the Anissia address, with the anime number as the query
//!   value `animeNo`. A comment without a readable Anissia address is
//!   [`Reading::Unreadable`] with the reason.
//!
//! Also accepted, for hand-written comments that do not follow the convention
//! but fit its two lines: the anime number as a query value named `animeNo`,
//! `anime_no`, `no` or `id` or as the last all-digit path segment, and a
//! creator written as `자막: <name>`, `자막 제작자: <name>`, `제작자: <name>` or
//! `자막팀: <name>` (`:` may be full-width), alone on a line or as a part of a
//! line split at `|`.
//!
//! **Which comment.** The two comment lines (`# ...`) directly above a rule's
//! `- ` line, the address being the nearer one. Lines above those two (another
//! rule left commented out, a note, a heading) are ignored, so an address in
//! them is never taken; an address that is not on the line directly above the
//! rule (a note stands between) leaves the comment unreadable. A blank line
//! ends a block, so a section heading such as `# Q. 2026/3` followed by a blank
//! line belongs to no rule, and a comment separated from its rule by a blank
//! line is no comment of that rule. Comments elsewhere (above a channel or
//! between keys) are not read.

use std::sync::LazyLock;

use regex::Regex;
use url::Url;

/// What one rule's comment says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// No comment above the rule.
    None,
    /// A comment that does not name an Anissia anime the way this module reads.
    /// `reason` completes `... 주석을 읽을 수 없음: <reason>`, such as
    /// `Anissia 주소 형식이 달라서`.
    Unreadable { reason: String },
    /// An Anissia anime, and the subtitle creator and the airing time when the
    /// comment gives them.
    Address {
        anime_no: i64,
        creator: Option<String>,
        airs: Option<Airs>,
    },
}

/// The weekday and time the comment gives, kept for showing when Anissia
/// cannot be reached.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Airs {
    /// 0 (Sunday) to 6 (Saturday), as Anissia numbers its weekdays.
    pub week: u8,
    /// `HH:MM`.
    pub time: String,
}

/// Line 1 of the convention: `Mon. 23:30. <creator>`.
#[derive(Debug, PartialEq, Eq)]
struct Headline {
    airs: Airs,
    creator: Option<String>,
}

/// Whether a creator field names nobody.
fn names_nobody(name: &str) -> bool {
    name.is_empty() || matches!(name, "미정" | "없음" | "-" | "?")
}

/// Reads `line` as line 1 of the convention, if it has that shape.
fn headline(line: &str) -> Option<Headline> {
    static HEADLINE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?i:(Sun|Mon|Tue|Wed|Thu|Fri|Sat))\. (\d{2}):(\d{2})\.(?:\s+(.*))?$")
            .expect("a valid pattern")
    });
    let captures = HEADLINE.captures(line.trim())?;
    let hour: u8 = captures[2].parse().ok()?;
    let minute: u8 = captures[3].parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    let week = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"]
        .iter()
        .position(|day| captures[1].eq_ignore_ascii_case(day))?;
    let name = captures.get(4).map_or("", |m| m.as_str().trim());
    Some(Headline {
        airs: Airs {
            week: u8::try_from(week).ok()?,
            time: format!("{hour:02}:{minute:02}"),
        },
        creator: (!names_nobody(name)).then(|| name.to_owned()),
    })
}

const NO_ADDRESS: &str = "Anissia 주소가 없어서";
const ODD_ADDRESS: &str = "Anissia 주소 형식이 달라서";
const ADDRESS_NOT_LAST: &str = "Anissia 주소가 규칙 바로 윗줄에 있지 않아서";
const SEVERAL_ANIME: &str = "Anissia 주소가 서로 다른 작품을 가리켜서";
/// The scan could not tell which rule a comment belongs to.
const NO_PLACE: &str = "주석이 어느 규칙 것인지 찾지 못해서";

/// The hosts that are Anissia's.
fn is_anissia(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "anissia.net" || host.ends_with(".anissia.net")
}

/// The `http(s)` addresses written in `text`, as the user typed them.
fn addresses(text: &str) -> Vec<Url> {
    static ADDRESS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"https?://[^\s<>"'`)\]]+"#).expect("a valid pattern"));
    ADDRESS
        .find_iter(text)
        .filter_map(|m| Url::parse(m.as_str().trim_end_matches(['.', ',', ';'])).ok())
        .collect()
}

/// The anime number an Anissia address names, if it names one.
fn anime_no(url: &Url) -> Option<i64> {
    let number = |text: &str| {
        (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
            .then(|| text.parse::<i64>().ok())
            .flatten()
            .filter(|n| *n > 0)
    };
    url.query_pairs()
        .find(|(key, _)| matches!(&**key, "animeNo" | "anime_no" | "no" | "id"))
        .and_then(|(_, value)| number(&value))
        .or_else(|| {
            url.path_segments()?
                .rev()
                .find(|segment| !segment.is_empty())
                .and_then(number)
        })
}

/// The creator a comment line names, if it does.
fn creator_of(line: &str) -> Option<String> {
    static LABEL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:자막\s*제작자|자막\s*제작|제작자|자막팀|자막)\s*[:：]\s*(.*)$")
            .expect("a valid pattern")
    });
    static ADDRESS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"https?://[^\s<>"'`)\]]+"#).expect("a valid pattern"));
    let without_addresses = ADDRESS.replace_all(line, " ");
    without_addresses.split('|').find_map(|part| {
        let name = LABEL.captures(part.trim())?.get(1)?.as_str().trim();
        (!names_nobody(name)).then(|| name.to_owned())
    })
}

/// Reads the comment lines above one rule (each without its `#`). Only the two
/// lines nearest the rule are the rule's comment, and the address must be on
/// the nearest: what stands above them is another rule left commented out, a
/// note or a heading, and an address there belongs to nothing here.
pub fn read(lines: &[String]) -> Reading {
    let lines: Vec<&str> = lines.iter().map(|l| l.trim()).collect();
    if lines.iter().all(|l| l.is_empty()) {
        return Reading::None;
    }
    let lines = &lines[lines.len().saturating_sub(2)..];
    let has_anissia_address = |line: &str| {
        addresses(line)
            .iter()
            .any(|url| url.host_str().is_some_and(is_anissia))
    };
    if lines.last().is_some_and(|last| !has_anissia_address(last)) {
        let above = lines.iter().any(|line| has_anissia_address(line));
        return Reading::Unreadable {
            reason: if above { ADDRESS_NOT_LAST } else { NO_ADDRESS }.to_owned(),
        };
    }

    let mut anissia = Vec::new();
    for line in lines {
        anissia.extend(
            addresses(line)
                .into_iter()
                .filter(|url| url.host_str().is_some_and(is_anissia)),
        );
    }
    if anissia.is_empty() {
        return Reading::Unreadable {
            reason: NO_ADDRESS.to_owned(),
        };
    }
    let mut numbers: Vec<i64> = anissia.iter().filter_map(anime_no).collect();
    numbers.sort_unstable();
    numbers.dedup();
    let reason = |reason: &str| Reading::Unreadable {
        reason: reason.to_owned(),
    };
    let anime_no = match numbers.as_slice() {
        [] => return reason(ODD_ADDRESS),
        [one] => *one,
        _ => return reason(SEVERAL_ANIME),
    };
    // The convention's line 1 decides the creator when it is there, even when
    // it names nobody; without it the labelled forms are tried. A line 1 of
    // another shape names no creator and no time, but the address still makes
    // the rule address only.
    let headline = lines.iter().find_map(|line| headline(line));
    let creator = match &headline {
        Some(h) => h.creator.clone(),
        None => lines.iter().find_map(|line| creator_of(line)),
    };
    Reading::Address {
        anime_no,
        creator,
        airs: headline.map(|h| h.airs),
    }
}

/// Which kind of line a YAML line is, for the scan.
enum Line<'a> {
    Blank,
    Comment(&'a str),
    /// Content with its indentation.
    Content(usize, &'a str),
}

fn classify(line: &str) -> Line<'_> {
    let body = line.trim_start_matches([' ', '\t']);
    let indent = line.len() - body.len();
    let body = body.trim_end();
    if body.is_empty() {
        Line::Blank
    } else if let Some(comment) = body.strip_prefix('#') {
        Line::Comment(comment.trim_start_matches('#'))
    } else {
        Line::Content(indent, body)
    }
}

fn is_item(text: &str) -> bool {
    text == "-" || text.starts_with("- ")
}

/// The comment lines directly above each rule (all of them; [`read`] keeps the
/// last two): for each channel, for each rule
/// in order. `None` when the file's layout is not one this scan follows (flow
/// style, a `rules:` on the same line as a channel's `- `, ...); the caller
/// then compares the shape with what the YAML parser found.
fn scan(content: &str) -> Option<Vec<Vec<Vec<String>>>> {
    let mut channels: Vec<Vec<Vec<String>>> = Vec::new();
    let mut channel_indent: Option<usize> = None;
    let mut pending: Vec<String> = Vec::new();
    // In the current channel: the `rules:` key's indentation and the rules'.
    let mut rules_key: Option<usize> = None;
    let mut in_rules = false;
    let mut item_indent: Option<usize> = None;

    for line in content.lines() {
        let (indent, text) = match classify(line) {
            Line::Blank => {
                pending.clear();
                continue;
            }
            Line::Comment(comment) => {
                pending.push(comment.trim().to_owned());
                continue;
            }
            Line::Content(indent, text) => (indent, text),
        };
        let comment = std::mem::take(&mut pending);
        if text == "---" && channel_indent.is_none() {
            continue;
        }
        let first = *channel_indent.get_or_insert(indent);
        if indent == first && is_item(text) {
            channels.push(Vec::new());
            in_rules = false;
            rules_key = None;
            item_indent = None;
            continue;
        }
        let rules = channels.last_mut()?;

        if in_rules {
            match item_indent {
                Some(at) if indent == at && is_item(text) => {
                    rules.push(comment);
                    continue;
                }
                Some(at) if indent > at => continue,
                None if is_item(text) && indent >= rules_key.unwrap_or(0) => {
                    item_indent = Some(indent);
                    rules.push(comment);
                    continue;
                }
                _ => in_rules = false,
            }
        }

        if let Some(rest) = text.strip_prefix("rules:") {
            let rest = rest.trim();
            if rest.is_empty() || rest.starts_with('#') {
                in_rules = true;
                rules_key = Some(indent);
                item_indent = None;
            } else if rest != "[]" {
                // Rules written inline: not scanned.
                return None;
            }
        }
    }
    Some(channels)
}

/// The reading of the comment above each rule, shaped like the parsed file:
/// `rule_counts[i]` is how many rules channel `i` has, and the answer has that
/// many readings for it.
///
/// When the scan finds a different number of channels or rules than the YAML
/// parser did, it cannot tell which comment belongs to which rule, so the rules
/// of a file that has comments are all [`Reading::Unreadable`] (a file with no
/// comment at all has none to misplace).
pub fn rule_comments(content: &str, rule_counts: &[usize]) -> Vec<Vec<Reading>> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let scanned = scan(content).filter(|channels| {
        channels.len() == rule_counts.len()
            && channels.iter().zip(rule_counts).all(|(c, n)| c.len() == *n)
    });
    match scanned {
        Some(channels) => channels
            .iter()
            .map(|rules| rules.iter().map(|lines| read(lines)).collect())
            .collect(),
        None => {
            let any_comment = content
                .lines()
                .any(|line| matches!(classify(line), Line::Comment(_)));
            rule_counts
                .iter()
                .map(|n| {
                    vec![
                        if any_comment {
                            Reading::Unreadable {
                                reason: NO_PLACE.to_owned(),
                            }
                        } else {
                            Reading::None
                        };
                        *n
                    ]
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests;
