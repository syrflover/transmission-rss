//! Reading the comments above the rules of a legacy channels YAML.
//!
//! A person who followed anime with the old file wrote, above each rule, where
//! the anime is on Anissia and which subtitle creator to follow
//! (`docs/specs/settings.md`, 기존 YAML). The YAML parser drops comments, so
//! this module scans the file's lines itself: [`rule_comments`] finds the
//! comment lines directly above each rule, and [`read`] turns one rule's
//! comment into a [`Reading`].
//!
//! # The comment format is an assumption
//!
//! The spec says the format is the convention of the author's own channel
//! configuration, read on 2026-09-29, and that other formats count as unread
//! comments. The repository holds no example of it (the fixtures carry no
//! comments, and the brainstorm only says that each rule is preceded by
//! comments naming the Anissia schedule's weekday and time, the subtitle
//! creator and the Anissia anime address). So the grammar below is a
//! documented guess, kept in this file and nowhere else; to match the real
//! convention only [`read`] and its helpers need to change.
//!
//! - **Which comment.** The comment lines (`# ...`) written directly above a
//!   rule's `- ` line, with no blank line between. A blank line ends a block,
//!   so a section heading such as `# Q. 2026/3` followed by a blank line
//!   belongs to no rule. Comments elsewhere (above a channel or between keys)
//!   are not read.
//! - **The Anissia address.** Any `http(s)` address in the block whose host is
//!   `anissia.net` or a subdomain of it, with the anime number either as a
//!   query value named `animeNo`, `anime_no`, `no` or `id`, or as the last
//!   all-digit path segment. The weekday and time the author wrote are not
//!   read: they come from Anissia, so they stay current.
//! - **The creator.** A line, or a part of a line split at `|`, written as
//!   `자막: <name>`, `자막 제작자: <name>`, `제작자: <name>` or `자막팀: <name>`
//!   (`:` may be full-width). An empty name or `미정`, `없음` or `-` is no
//!   creator.
//!
//! Anything else is not read, and the reason is a sentence fragment that the
//! review shows after `주석을 읽을 수 없음`.

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
    /// An Anissia anime, and the subtitle creator when the comment names one.
    Address {
        anime_no: i64,
        creator: Option<String>,
    },
}

const NO_ADDRESS: &str = "Anissia 주소가 없어서";
const ODD_ADDRESS: &str = "Anissia 주소 형식이 달라서";
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
        let none = name.is_empty() || matches!(name, "미정" | "없음" | "-" | "?");
        (!none).then(|| name.to_owned())
    })
}

/// Reads the comment lines above one rule (each without its `#`).
pub fn read(lines: &[String]) -> Reading {
    let lines: Vec<&str> = lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return Reading::None;
    }

    let mut anissia = Vec::new();
    for line in &lines {
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
    Reading::Address {
        anime_no,
        creator: lines.iter().find_map(|line| creator_of(line)),
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

/// The comment lines directly above each rule: for each channel, for each rule
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
