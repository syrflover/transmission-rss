//! What the `회차 대응 정하기` dialog shows before anything is saved
//! (`docs/specs/library.md`, 자막의 회차 대응), and the one line a group shows
//! for its mapping afterwards.
//!
//! The dialog holds what the user typed as it is ([`Input`]: the radio chosen,
//! the text in the offset box, the exception rows with their texts) and asks
//! what that comes to ([`shown`]): the offset each choice means, where the
//! source's episodes go under it, the exceptions as read, the warnings, and the
//! sentence for input that cannot be saved. Every decision is made by the
//! functions the save and the receipt use ([`Mapping::season_episode`],
//! [`whole`], [`stored_key`]), so what the dialog shows is what the mapping
//! will do.
//!
//! Texts the user wrote are trimmed as JavaScript trims them ([`trimmed`]),
//! because the dialog has always read its boxes that way.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use trss_core::episode::{stored_key, EpisodeKey, EpisodeNumber};

use super::{whole, Exception, Mapped, Mapping, MappingKind, MAX_NUMBER};

/// `앞 시즌에 이어 셈` is not offered when the earlier seasons' episode count is unknown.
const UNKNOWN_PREVIOUS: &str = "앞 시즌의 회차 수를 알 수 없어서 고를 수 없어요.";
/// ... or when there is no earlier season.
const NO_PREVIOUS: &str = "앞 시즌이 없어서 고를 수 없어요.";
/// The label of an episode an exception puts past the season's count.
const PAST: &str = "시즌 회차 수 밖";

/// The default mapping the user picks: `같은 번호`, `앞 시즌에 이어 셈` or `직접 차이`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Same,
    Continue,
    Custom,
}

impl Choice {
    pub fn code(self) -> &'static str {
        match self {
            Choice::Same => "same",
            Choice::Continue => "continue",
            Choice::Custom => "custom",
        }
    }

    pub fn parse(code: &str) -> Option<Choice> {
        match code {
            "same" => Some(Choice::Same),
            "continue" => Some(Choice::Continue),
            "custom" => Some(Choice::Custom),
            _ => None,
        }
    }
}

/// One exception row as the dialog holds it: Anissia's episode and the season
/// episode as typed, or `받지 않음` (`skip`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub episode: String,
    pub target: String,
    pub skip: bool,
}

/// What the dialog holds: the radio chosen (`None` before one is), the text in
/// the offset box and the exception rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub choice: Option<Choice>,
    pub custom: String,
    pub rows: Vec<Row>,
}

/// What the library knows of the source and the season it is mapped to.
#[derive(Debug, Clone, Copy)]
pub struct Ground<'a> {
    /// Anissia's episode texts of the source, one for each episode (the
    /// newest observation's).
    pub episodes: &'a [String],
    /// The episodes of all earlier seasons together, when each is known.
    pub previous: Option<u32>,
    /// The season's own episode count, when it is known.
    pub total: Option<u32>,
}

/// Why `앞 시즌에 이어 셈` cannot be chosen, or `None` when it can.
pub fn continue_reason(previous: Option<u32>) -> Option<&'static str> {
    match previous {
        None => Some(UNKNOWN_PREVIOUS),
        Some(0) => Some(NO_PREVIOUS),
        Some(_) => None,
    }
}

/// What JavaScript's `String.prototype.trim` takes off: white space and line
/// terminators, with the byte order mark and without the next-line control.
pub fn trimmed(text: &str) -> &str {
    text.trim_matches(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}')
}

/// An integer written in a box (`-12`, `0`, `+3`; one to five digits), or
/// `None` for anything else.
fn integer_of(text: &str) -> Option<i64> {
    let text = trimmed(text);
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if !(1..=5).contains(&digits.len()) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// The offset a choice means (what is added to Anissia's whole episode), or
/// `None` while it cannot be said: `앞 시즌에 이어 셈` without an earlier
/// season's count, or an offset box that holds no integer within ±9999.
pub fn offset_of(choice: Choice, custom: &str, previous: Option<u32>) -> Option<i64> {
    match choice {
        Choice::Same => Some(0),
        Choice::Continue => match (continue_reason(previous), previous) {
            (None, Some(previous)) => Some(-i64::from(previous)),
            _ => None,
        },
        Choice::Custom => integer_of(custom).filter(|n| n.abs() <= MAX_NUMBER),
    }
}

/// The choice that already means `offset`: the one a dialog opens on.
pub fn choice_of(offset: Option<i64>, previous: Option<u32>) -> Option<Choice> {
    let offset = offset?;
    if offset == 0 {
        return Some(Choice::Same);
    }
    match (continue_reason(previous), previous) {
        (None, Some(previous)) if offset == -i64::from(previous) => Some(Choice::Continue),
        _ => Some(Choice::Custom),
    }
}

/// The exceptions the rows say, or the sentence for the first row that cannot
/// be saved. A row left empty (nothing written, not `받지 않음`) is left out.
/// Two rows for one episode number (`013` and `13`) cannot be saved.
pub fn exceptions_of(rows: &[Row]) -> Result<Vec<Exception>, String> {
    let mut out: Vec<Exception> = Vec::new();
    for row in rows {
        if trimmed(&row.episode).is_empty() && trimmed(&row.target).is_empty() && !row.skip {
            continue;
        }
        let episode = trimmed(&row.episode);
        if episode.is_empty() {
            return Err("예외의 회차를 써 주세요.".to_owned());
        }
        let key = stored_key(episode);
        if out.iter().any(|e| e.key == key) {
            return Err(format!(
                "회차 ‘{episode}’의 예외가 둘이에요. 같은 회차에는 예외를 하나만 둘 수 있어요."
            ));
        }
        let target = if row.skip {
            None
        } else {
            match integer_of(&row.target).filter(|t| (1..=MAX_NUMBER).contains(t)) {
                Some(target) => Some(target as u32),
                None => {
                    return Err(format!(
                        "회차 ‘{episode}’가 시즌의 몇 화인지 1 이상의 숫자로 써 주세요. 받지 않으려면 ‘받지 않음’을 골라 주세요."
                    ))
                }
            }
        };
        out.push(Exception {
            key,
            episode: episode.to_owned(),
            target,
        });
    }
    Ok(out)
}

/// The mapping an offset and exceptions make, for reading it like a stored one.
fn user_mapping(offset: Option<i64>, exceptions: &[Exception]) -> Mapping {
    Mapping {
        kind: MappingKind::User,
        offset,
        evidence: String::new(),
        decided_at: 0,
        version: 0,
        exceptions: exceptions.to_vec(),
    }
}

/// Anissia's episode texts of a source, each once (by number) and in order:
/// numbers ascending, then the other texts. `0` is left out.
pub fn episodes_of(texts: &[String]) -> Vec<String> {
    let mut by_key: BTreeMap<EpisodeKey, &str> = BTreeMap::new();
    for text in texts {
        let key = EpisodeKey::of(text);
        if key.number().and_then(EpisodeNumber::whole) == Some(0) {
            continue;
        }
        by_key.entry(key).or_insert(text);
    }
    by_key.into_values().map(str::to_owned).collect()
}

/// Where a mapping puts one of the source's episodes, with the sentence for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fate {
    /// The user's exception says it is not received.
    Skipped,
    /// The mapping says nothing of it: undecided, or no whole episode and no exception.
    Unplaced,
    /// The default offset puts it outside the season (below 1 or past its last).
    Outside,
    /// It is received as this season episode, inside the season.
    Episode(i64),
    /// An exception puts it outside the season; it is received all the same.
    Past(i64),
}

/// One line of the preview: Anissia's episode, and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub episode: String,
    pub fate: Fate,
}

impl Move {
    /// `1화`, `32화 (시즌 회차 수 밖)`, `받지 않음`, `시즌 밖` or `정해지지 않음`.
    pub fn to(&self) -> String {
        match self.fate {
            Fate::Skipped => "받지 않음".to_owned(),
            Fate::Unplaced => "정해지지 않음".to_owned(),
            Fate::Outside => "시즌 밖".to_owned(),
            Fate::Episode(n) => format!("{n}화"),
            Fate::Past(n) => format!("{n}화 ({PAST})"),
        }
    }

    /// The season episode it is received as, when it is received at all.
    pub fn received_as(&self) -> Option<i64> {
        match self.fate {
            Fate::Episode(n) | Fate::Past(n) => Some(n),
            _ => None,
        }
    }

    /// It is received as an episode inside the season.
    pub fn lands(&self) -> bool {
        matches!(self.fate, Fate::Episode(_))
    }
}

/// Where each of the source's episodes goes under an offset and exceptions;
/// `total` is the season's episode count when known. Only the default offset
/// is held to `1` to `total`: the receipt takes what an exception covers
/// whatever the count says.
pub fn moves(
    episodes: &[String],
    offset: Option<i64>,
    exceptions: &[Exception],
    total: Option<u32>,
) -> Vec<Move> {
    let covered: HashSet<&str> = exceptions.iter().map(|e| e.key.as_str()).collect();
    let mapping = user_mapping(offset, exceptions);
    episodes_of(episodes)
        .into_iter()
        .map(|episode| {
            let fate = match mapping.season_episode(&episode) {
                Mapped::NotReceived => Fate::Skipped,
                Mapped::Unmapped => Fate::Unplaced,
                Mapped::Episode(n) => {
                    let outside = n < 1 || total.is_some_and(|t| n > i64::from(t));
                    match (covered.contains(stored_key(&episode).as_str()), outside) {
                        (true, true) => Fate::Past(n),
                        (false, true) => Fate::Outside,
                        (_, false) => Fate::Episode(n),
                    }
                }
            };
            Move { episode, fate }
        })
        .collect()
}

/// The warnings of a preview: a season episode that two or more of the
/// source's episodes land on (`13.5 → 13` with offset 0 while `13` exists).
/// The save is not refused; the user is told.
pub fn collisions(moves: &[Move]) -> Vec<String> {
    let mut by_episode: BTreeMap<i64, Vec<&str>> = BTreeMap::new();
    for m in moves {
        if let Some(n) = m.received_as() {
            by_episode.entry(n).or_default().push(&m.episode);
        }
    }
    by_episode
        .into_iter()
        .filter(|(_, episodes)| episodes.len() > 1)
        .map(|(n, episodes)| {
            let count = match episodes.len() {
                2 => "두".to_owned(),
                3 => "세".to_owned(),
                many => format!("{many}개"),
            };
            format!("{n}화에 {count} 회차가 들어와요 ({})", episodes.join(", "))
        })
        .collect()
}

/// An episode text as the preview writes it: `13화` for a number's text (`013`
/// too), the text itself for any other (`SP`).
fn label(episode: &str) -> String {
    EpisodeKey::of(episode).label()
}

/// The preview as one short text: `13화 → 1화 · 14화 → 2화 … 24화 → 12화`,
/// with at most `shown` entries (the last one after `…` when there are more).
pub fn preview_text(moves: &[Move], shown: usize) -> String {
    let one = |m: &Move| format!("{} → {}", label(&m.episode), m.to());
    if moves.len() <= shown {
        return moves.iter().map(one).collect::<Vec<_>>().join(" · ");
    }
    let head = moves[..shown - 1]
        .iter()
        .map(one)
        .collect::<Vec<_>>()
        .join(" · ");
    format!("{head} … {}", one(&moves[moves.len() - 1]))
}

/// The episodes the default offset does not place in the season and no
/// exception covers: a decimal or text episode, or one that falls outside `1`
/// to `total`. The dialog offers them as exceptions to write.
pub fn misfits(
    episodes: &[String],
    offset: Option<i64>,
    exceptions: &[Exception],
    total: Option<u32>,
) -> Vec<String> {
    let covered: HashSet<&str> = exceptions.iter().map(|e| e.key.as_str()).collect();
    moves(episodes, offset, exceptions, total)
        .into_iter()
        .filter(|m| {
            !covered.contains(stored_key(&m.episode).as_str())
                && matches!(m.fate, Fate::Unplaced | Fate::Outside)
        })
        .map(|m| m.episode)
        .collect()
}

/// How many entries of the preview text a choice shows under it.
const PREVIEW_SHOWN: usize = 4;

/// What the dialog shows for what it holds ([`shown`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// Why `앞 시즌에 이어 셈` cannot be chosen.
    pub continue_reason: Option<&'static str>,
    /// The text under `같은 번호`.
    pub same: String,
    /// The text under `앞 시즌에 이어 셈`; empty when it cannot be chosen.
    pub continued: String,
    /// The text under `직접 차이`; empty while the box holds no usable offset.
    pub custom: String,
    /// The offset of the choice made: what `저장` sends.
    pub offset: Option<i64>,
    /// The sentence for an offset box that holds no usable offset, once the
    /// choice is `직접 차이` and something is written.
    pub offset_problem: Option<String>,
    /// The exceptions the rows say, as `저장` sends them; empty while a row
    /// cannot be saved.
    pub exceptions: Vec<Exception>,
    /// The sentence for the first row that cannot be saved.
    pub exceptions_problem: Option<String>,
    /// A season episode that two or more episodes land on.
    pub warnings: Vec<String>,
    /// Episodes to offer as exceptions: the ones the choice does not place and
    /// no row names yet.
    pub to_add: Vec<String>,
}

/// What the dialog shows for `input` against what the library knows of the
/// source and the season.
pub fn shown(input: &Input, ground: &Ground<'_>) -> Shown {
    let Ground {
        episodes,
        previous,
        total,
    } = *ground;
    let read = exceptions_of(&input.rows);
    let (exceptions, exceptions_problem) = match read {
        Ok(exceptions) => (exceptions, None),
        Err(problem) => (Vec::new(), Some(problem)),
    };
    let offset = input
        .choice
        .and_then(|choice| offset_of(choice, &input.custom, previous));
    let text_of = |choice: Choice| {
        offset_of(choice, &input.custom, previous)
            .map(|offset| {
                preview_text(
                    &moves(episodes, Some(offset), &exceptions, total),
                    PREVIEW_SHOWN,
                )
            })
            .unwrap_or_default()
    };
    let offset_problem = (input.choice == Some(Choice::Custom)
        && offset.is_none()
        && !trimmed(&input.custom).is_empty())
    .then(|| format!("-{MAX_NUMBER}에서 {MAX_NUMBER} 사이의 정수를 써 주세요."));
    let written: BTreeSet<&str> = input.rows.iter().map(|r| trimmed(&r.episode)).collect();
    Shown {
        continue_reason: continue_reason(previous),
        same: text_of(Choice::Same),
        continued: text_of(Choice::Continue),
        custom: text_of(Choice::Custom),
        offset,
        offset_problem,
        warnings: collisions(&moves(episodes, offset, &exceptions, total)),
        to_add: misfits(episodes, offset, &exceptions, total)
            .into_iter()
            .filter(|episode| !written.contains(episode.as_str()))
            .collect(),
        exceptions,
        exceptions_problem,
    }
}

/// `13.5 받지 않음` or `14 → 3화`.
fn exception_text(e: &Exception) -> String {
    match e.target {
        None => format!("{} 받지 않음", e.episode),
        Some(target) => format!("{} → {target}화", e.episode),
    }
}

impl Mapping {
    /// The group's one line for this mapping: `자동 · <근거>`, `회차 대응 미정 ·
    /// <이유>` or, for the user's, `직접 정함 · <대응>`.
    ///
    /// The user's: `같은 번호` for offset 0, else the first of the source's
    /// episodes that the offset places inside the season and no exception
    /// covers, written by its number (`013` is `13화`) as `13화 → 1화`; with
    /// none, the offset itself (`+5화 차이`). The exceptions follow
    /// (`· 예외 13.5 받지 않음, 14 → 3화`, with `외 N개` past two).
    pub fn line(&self, episodes: &[String], total: Option<u32>) -> String {
        let label = match self.kind {
            MappingKind::Auto => "자동",
            MappingKind::Undecided => "회차 대응 미정",
            MappingKind::User => return self.user_line(episodes, total),
        };
        format!("{label} · {}", self.evidence)
    }

    fn user_line(&self, episodes: &[String], total: Option<u32>) -> String {
        let mut parts = vec!["직접 정함".to_owned()];
        match self.offset {
            None => parts.push("대응 미정".to_owned()),
            Some(0) => parts.push("같은 번호".to_owned()),
            Some(offset) => {
                let uncovered: Vec<String> = episodes
                    .iter()
                    .filter(|episode| self.exception_of(episode).is_none())
                    .cloned()
                    .collect();
                let example = moves(&uncovered, Some(offset), &[], total)
                    .into_iter()
                    .find(|m| m.lands() && whole(&m.episode).is_some());
                parts.push(match example {
                    Some(m) => {
                        format!("{} → {}화", label(&m.episode), m.received_as().unwrap_or(0))
                    }
                    None => format!("{}{offset}화 차이", if offset > 0 { "+" } else { "" }),
                });
            }
        }
        if !self.exceptions.is_empty() {
            let names: Vec<String> = self.exceptions.iter().take(2).map(exception_text).collect();
            let more = match self.exceptions.len() {
                0..=2 => String::new(),
                many => format!(" 외 {}개", many - 2),
            };
            parts.push(format!("예외 {}{more}", names.join(", ")));
        }
        parts.join(" · ")
    }
}

#[cfg(test)]
mod tests;
