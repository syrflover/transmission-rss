//! The episode mapping of a subtitle source to a season (`docs/specs/library.md`,
//! 자막의 회차 대응): which episode of the season a creator's Anissia episode is.
//!
//! A mapping is one default per (work, season, source): Anissia's whole
//! episode `n` is the season's episode `n + offset`. The app decides it
//! only for the subscribed creator's source (`trss_jobs::follow`), without
//! asking (the subscription settled the anime and the season already), and
//! from when the creator posted, not from what the numbers look like: creators
//! of one anime number their posts differently (a second season counted on
//! from the first by one, from 1 by another), so no rule of the work can say.
//!
//! # The evidence of an episode
//!
//! For each whole episode `n` (`013` and `13` are one) the source's *earliest*
//! observation of it is taken, and `t` is the time Anissia wrote on it
//! (`updDt`, [`Posted::at`]); a line whose time did not read is no evidence
//! (the time the app first saw it is not used). The season's schedule gives
//! the air time of each season episode `k` (AniList, the linked entries in
//! order, a later part's episodes after the earlier parts' counts, from
//! entries of any status). The largest `k` that aired by `t` (`air(k) ≤ t`)
//! is where the post fell, provided `t` is inside that episode's window: before
//! `air(k+1)` when `k+1` is scheduled, within 7 days of `air(k)` when `k` is
//! the season's last episode `N`, and no window when the schedule stops short
//! of `N` (cut, or not out yet). A post before the first airing, outside the
//! window, or at a time several episodes share (an ONA released at once) says
//! nothing. Otherwise the
//! episode's evidence is the offset `d = k − n`.
//!
//! # The decision
//!
//! Evidence is grouped by `d`; the supporters of `d` are the distinct episodes
//! in its group. `N` is the season's AniList episode count, else the highest
//! scheduled episode; `prev` is the sum of the counts of all earlier seasons of
//! the work, known only when every one of them is (and meaningful above 0).
//!
//! | evidence | mapping |
//! | --- | --- |
//! | exactly one `d` has 2 or more supporters | `auto`, `d` (`1화·2화가 방영 뒤에 올라왔어요`); single episodes of other offsets are left out of it and show as conflicts |
//! | two or more `d`s have 2 or more supporters each | `undecided` (`회차마다 가리키는 차이가 달라요`) |
//! | no `d` has 2 supporters, one episode `n` has evidence, and `d = 0` with `1 ≤ n ≤ N`, or `prev > 0` and `d = −prev` | `auto`, `d` (`1화가 방영 뒤에 올라왔고 시즌 회차 수(12) 안이에요`, `13화가 1화 방영 뒤에 올라왔고 앞 시즌 회차 수(12)만큼 이어 셌어요`) |
//! | no `d` has 2 supporters, one episode with evidence that fits neither | `undecided`, with the reason |
//! | no `d` has 2 supporters, several episodes that disagree | `undecided` (`회차마다 가리키는 차이가 달라요`) |
//! | season 0 (specials), no schedule, no whole episode seen, or none with evidence | `undecided`, with the reason |
//!
//! A new decision never changes an `auto` offset to another by itself
//! (`trss_jobs::mapping::store_in`, [`reconcile`]): the mapping goes `undecided` and says both offsets, and it
//! is decided again only to the offset it was taken back from, until the user
//! maps it. A mapping the user set (`user`) is never touched by the app.
//!
//! # The episodes that do not fit
//!
//! For a decided mapping (`auto` or `user`) with offset `d`, an observed
//! episode is a *conflict* ([`conflicts`]) when it is a decimal or other
//! text that is not `0` (`13.5`, `SP`: the offset does not say what they are),
//! when a whole `n` with `n + d` falls below 1 or past `N` (`N` known), or, for an
//! `auto` mapping only, when a whole `n`'s own air-time evidence is another
//! offset. `0` (the line a creator registers before the first episode) is
//! never received and never a conflict, and neither is an episode the user's
//! exception covers (it is received as the exception says, or not at all). A
//! conflict is not received by itself; the source's other episodes are. The set
//! is stored with the mapping (`trss_jobs::mapping::store_conflicts`) for the `회차 확인 필요`
//! to-do and the work detail.
//!
//! The mapping is decided again whenever the subscribed creator's episodes are
//! looked at, so a new episode that points to another offset takes an `auto`
//! mapping back to `undecided` until the user maps it.
//!
//! # What the user sets
//!
//! The user maps any source of the season from the work detail
//! (`trss_jobs::mapping::set_user_in`): a default offset and the per-episode [`Exception`]s, one
//! unit saved with the `user` kind and `retired_offset` cleared. An exception
//! names one episode text of Anissia (compared by [`stored_key`]: `013`, `13`
//! and `13.0` are one) and says which season episode it is, or that it is not
//! received. Exceptions are applied before the offset wherever a mapping is read
//! ([`Mapping::season_episode`]): the subscribed creator's receipt, the conflicts
//! (an episode an exception covers is none), the candidates' revision marks. The
//! app never changes a `user` row (`store_in`); `revert_in` (`자동으로
//! 되돌리기`) deletes it with its exceptions and the offset it had retired, so the
//! app decides again at its next look.
//!
//! The rows these are kept in (the stored mappings, their exceptions and
//! conflicts) are `trss-jobs`' `mapping`, which calls this module for every
//! decision.

use std::collections::BTreeMap;

use trss_core::{
    episode::{ranges, signed, stored_key, EpisodeNumber},
    Millis,
};

/// How long after the last scheduled episode aired a post still belongs to it.
const LAST_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Who decided a mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingKind {
    /// The app, from grounds that agree.
    Auto,
    /// The app found no such grounds; nothing is mapped.
    Undecided,
    /// The user; the app never changes it.
    User,
}

impl MappingKind {
    pub fn code(self) -> &'static str {
        match self {
            MappingKind::Auto => "auto",
            MappingKind::Undecided => "undecided",
            MappingKind::User => "user",
        }
    }

    pub fn parse(code: &str) -> Option<MappingKind> {
        match code {
            "auto" => Some(MappingKind::Auto),
            "undecided" => Some(MappingKind::Undecided),
            "user" => Some(MappingKind::User),
            _ => None,
        }
    }
}

/// The evidence a mapping the user set carries.
pub const USER_EVIDENCE: &str = "사용자가 정했어요";

/// The most exceptions a mapping has, and the longest episode text of one.
pub const MAX_EXCEPTIONS: usize = 200;
const MAX_EPISODE_CHARS: usize = 32;
/// The largest offset and season episode the user can set.
const MAX_NUMBER: i64 = 9999;

/// One exception of a user's mapping: one episode text of Anissia is the
/// season's episode `target`, or is not received (`None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exception {
    /// [`stored_key`] of `episode`: what two texts of one episode share.
    pub key: String,
    /// The text as the user wrote it.
    pub episode: String,
    pub target: Option<u32>,
}

/// A source's mapping to a season, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    pub kind: MappingKind,
    /// What to add to Anissia's whole episode; `None` while undecided.
    pub offset: Option<i64>,
    /// Why: the grounds that agree, or the reason none do.
    pub evidence: String,
    pub decided_at: Millis,
    /// What a save carries (see the module docs); a source with no row is 0.
    pub version: i64,
    /// The user's exceptions, by key; empty for any mapping but a `user` one.
    pub exceptions: Vec<Exception>,
}

/// Where a mapping puts an episode text of Anissia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mapped {
    /// The season's episode, which may be outside the season (below 1 or past
    /// its last): a whole episode `n` is `n + offset`, an exception's is its
    /// target.
    Episode(i64),
    /// The user's exception says the episode is not received.
    NotReceived,
    /// The mapping is undecided, or the text is no whole episode and has no
    /// exception.
    Unmapped,
}

/// A positive whole episode number (not `0`, `13.5` or text).
pub fn whole(text: &str) -> Option<i64> {
    let n = i64::try_from(EpisodeNumber::parse(text)?.whole()?).ok()?;
    (n > 0).then_some(n)
}

impl Mapping {
    /// The exception that covers the episode text, if the user set one.
    pub fn exception_of(&self, text: &str) -> Option<&Exception> {
        if self.exceptions.is_empty() {
            return None;
        }
        let key = stored_key(text);
        self.exceptions.iter().find(|e| e.key == key)
    }

    /// Where the mapping puts the episode text: the exception first, else the
    /// offset for a whole episode (see the module docs).
    pub fn season_episode(&self, text: &str) -> Mapped {
        if let Some(exception) = self.exception_of(text) {
            return match exception.target {
                Some(target) => Mapped::Episode(i64::from(target)),
                None => Mapped::NotReceived,
            };
        }
        match (self.decided_offset(), whole(text)) {
            (Some(offset), Some(n)) => Mapped::Episode(n + offset),
            _ => Mapped::Unmapped,
        }
    }

    /// What to add to Anissia's whole episode to get the season's, `None` while
    /// the mapping is undecided: the one reading of the mapping the receipt of
    /// the subscribed creator's episodes and the work detail's candidates share.
    pub fn decided_offset(&self) -> Option<i64> {
        match self.kind {
            MappingKind::Undecided => None,
            MappingKind::Auto | MappingKind::User => self.offset,
        }
    }
}

/// What the app decides from the grounds ([`decide`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decided {
    /// `None`: undecided.
    pub offset: Option<i64>,
    pub evidence: String,
}

/// A whole episode `n > 0` of the source and when Anissia wrote its earliest
/// observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Posted {
    pub episode: u32,
    /// `updDt` of the source's earliest observation of the episode; `None`
    /// when it did not read as a time.
    pub at: Option<Millis>,
}

/// What is known of the season the source is mapped to.
#[derive(Debug, Clone, Copy)]
pub struct Season<'a> {
    pub number: u32,
    /// The air time (Unix ms) of each season episode.
    pub schedule: &'a BTreeMap<u32, i64>,
    /// The AniList episode count of the season's linked entries together.
    pub count: Option<u32>,
    /// The episodes of all earlier seasons together, when each is known.
    pub previous: Option<u32>,
}

impl Season<'_> {
    /// `N`: the count, else the highest scheduled episode.
    pub fn total(&self) -> Option<u32> {
        self.count
            .or_else(|| self.schedule.keys().next_back().copied())
    }
}

/// The offset as the screen writes it: `0`, `+1`, `−12`.
fn diff(offset: i64) -> String {
    match offset {
        0 => "0".to_owned(),
        o if o > 0 => format!("+{o}"),
        o => signed(o),
    }
}

/// The season episode whose window `at` falls in; `None` when it falls in
/// none or its air time is shared (see the module docs).
///
/// The 7 days stand in for the next airing only for the season's last
/// episode (`N`). An episode the schedule skips, or a schedule that stops
/// short of the count (one stored before the schedule was read in full),
/// leaves the window's end unknown: no evidence rather than a window that may
/// take in the next episode's posts.
fn aired(season: &Season<'_>, at: Millis) -> Option<u32> {
    let schedule = season.schedule;
    let (&k, &air) = schedule.iter().rev().find(|(_, air)| **air <= at)?;
    if schedule.iter().any(|(j, a)| *j != k && *a == air) {
        return None;
    }
    let end = match schedule.get(&(k + 1)) {
        Some(next) => *next,
        None if season.total() == Some(k) => air.saturating_add(LAST_WINDOW_MS),
        None => return None,
    };
    (at < end).then_some(k)
}

/// The offset each posted episode's own time points to, by episode.
pub fn offsets(posted: &[Posted], season: &Season<'_>) -> BTreeMap<u32, i64> {
    posted
        .iter()
        .filter_map(|p| {
            let k = aired(season, p.at?)?;
            Some((p.episode, i64::from(k) - i64::from(p.episode)))
        })
        .collect()
}

/// `1화·2화`, or `1–8화` when there are many.
fn written(episodes: &[u32]) -> String {
    if episodes.len() <= 4 {
        episodes
            .iter()
            .map(|e| format!("{e}화"))
            .collect::<Vec<_>>()
            .join("·")
    } else {
        format!("{}화", ranges(episodes))
    }
}

/// The mapping of the source whose whole episodes are `posted`, to `season`
/// (see the module docs).
pub fn decide(posted: &[Posted], season: &Season<'_>) -> Decided {
    let undecided = |evidence: String| Decided {
        offset: None,
        evidence,
    };
    if season.number == 0 {
        return undecided("시즌 0(특별편)은 방영 시각으로 정하지 않아요".to_owned());
    }
    if season.schedule.is_empty() {
        return undecided("방영 일정이 없어요".to_owned());
    }
    if posted.is_empty() {
        return undecided("구독 제작자의 숫자 회차를 아직 보지 못했어요".to_owned());
    }
    let own = offsets(posted, season);
    if own.is_empty() {
        return undecided("방영 시각으로 가리키는 회차가 아직 없어요".to_owned());
    }
    let mut groups: BTreeMap<i64, Vec<u32>> = BTreeMap::new();
    for (&n, &d) in &own {
        groups.entry(d).or_default().push(n);
    }
    let strong: Vec<(i64, &Vec<u32>)> = groups
        .iter()
        .filter(|(_, episodes)| episodes.len() >= 2)
        .map(|(d, episodes)| (*d, episodes))
        .collect();
    match strong.as_slice() {
        [(d, episodes)] => {
            return Decided {
                offset: Some(*d),
                evidence: format!("{}가 방영 뒤에 올라왔어요", written(episodes)),
            }
        }
        [] => {}
        many => {
            let offsets: Vec<String> = many.iter().map(|(d, _)| diff(*d)).collect();
            return undecided(format!(
                "회차마다 가리키는 차이가 달라요({})",
                offsets.join(", ")
            ));
        }
    }
    // Every offset has a single episode.
    let [(&n, &d)] = own.iter().collect::<Vec<_>>()[..] else {
        let offsets: Vec<String> = groups.keys().map(|d| diff(*d)).collect();
        return undecided(format!(
            "회차마다 가리키는 차이가 달라요({})",
            offsets.join(", ")
        ));
    };
    if d == 0 {
        if let Some(total) = season.total().filter(|t| (1..=*t).contains(&n)) {
            return Decided {
                offset: Some(0),
                evidence: format!("{n}화가 방영 뒤에 올라왔고 시즌 회차 수({total}) 안이에요"),
            };
        }
    }
    if let Some(previous) = season.previous.filter(|p| *p > 0) {
        if d == -i64::from(previous) {
            let k = i64::from(n) + d;
            return Decided {
                offset: Some(d),
                evidence: format!(
                    "{n}화가 {k}화 방영 뒤에 올라왔고 앞 시즌 회차 수({previous})만큼 이어 셌어요"
                ),
            };
        }
    }
    undecided(format!(
        "{n}화 하나만 차이({})를 가리키고 시즌 구조와 맞지 않아요",
        diff(d)
    ))
}

/// What the app writes for a source once [`decide`] has spoken, given the
/// offset the stored mapping has taken back (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciled {
    pub kind: MappingKind,
    /// `None`: undecided.
    pub offset: Option<i64>,
    pub evidence: String,
    /// The offset the mapping was taken back from, while it is `undecided`.
    pub retired: Option<i64>,
}

/// What the app writes when it has `decided` a mapping for a source whose
/// stored mapping is not the user's. `retired` is the offset the stored mapping
/// has to keep to: an `auto` mapping's own offset, an `undecided` one's retired
/// offset, none when nothing is stored.
///
/// The app never takes up another offset by itself: when `retired` is set and
/// what is decided now is another offset, the mapping goes `undecided` and says
/// both. The offset it had is kept as `retired` while it is `undecided`, and
/// deciding it again clears it.
pub fn reconcile(retired: Option<i64>, decided: &Decided) -> Reconciled {
    let (mut offset, mut evidence) = (decided.offset, decided.evidence.clone());
    let mut retired = retired;
    match (offset, retired) {
        (Some(new), Some(old)) if new != old => {
            offset = None;
            evidence = format!(
                "자동으로 정한 차이({})와 다른 차이({})를 가리키는 회차가 생겼어요",
                diff(old),
                diff(new)
            );
        }
        // The offset it had is decided again: it is no longer taken back.
        (Some(_), _) => retired = None,
        (None, _) => {}
    }
    let kind = match offset {
        Some(_) => MappingKind::Auto,
        None => MappingKind::Undecided,
    };
    Reconciled {
        kind,
        offset,
        evidence,
        retired,
    }
}

/// An episode that does not fit the source's mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The episode text as the newest observation wrote it.
    pub episode: String,
    pub reason: String,
}

/// The episodes of the source that do not fit its `mapping` to the season (see
/// the module docs): `texts` are the episodes as written (one per episode, the
/// newest observation's), `posted` the whole ones with their earliest time. An
/// undecided mapping has none: nothing is received under it anyway.
pub fn conflicts(
    mapping: &Mapping,
    texts: &[String],
    posted: &[Posted],
    season: &Season<'_>,
) -> Vec<Conflict> {
    let Some(offset) = mapping.decided_offset() else {
        return Vec::new();
    };
    let own = offsets(posted, season);
    let total = season.total();
    let mut out = Vec::new();
    for text in texts {
        // The user said what the episode is, or that it is not received.
        if mapping.exception_of(text).is_some() {
            continue;
        }
        let reason = match EpisodeNumber::parse(text) {
            None => Some(format!(
                "숫자가 아닌 회차({text})는 정한 차이로 알 수 없어요"
            )),
            Some(n) if n.whole() == Some(0) => None,
            Some(n) if !n.is_whole() => {
                Some(format!("소수 회차({text})는 정한 차이로 알 수 없어요"))
            }
            Some(n) => match n.whole().and_then(|n| u32::try_from(n).ok()) {
                None => Some(format!("회차 번호({text})가 너무 커요")),
                Some(n) => {
                    let video = i64::from(n) + offset;
                    let outside = video < 1 || total.is_some_and(|t| video > i64::from(t));
                    if outside {
                        let bound = match total {
                            Some(t) => format!("시즌 회차 수({t}) 밖이에요"),
                            None => "1화보다 앞이에요".to_owned(),
                        };
                        Some(format!(
                            "{n}화는 차이({})를 더하면 {video}화라 {bound}",
                            diff(offset)
                        ))
                    } else {
                        match own.get(&n) {
                            Some(&theirs)
                                if mapping.kind == MappingKind::Auto && theirs != offset =>
                            {
                                Some(format!(
                                    "{n}화는 방영 시각으로 차이({})를 가리켜요(정한 차이 {})",
                                    diff(theirs),
                                    diff(offset)
                                ))
                            }
                            _ => None,
                        }
                    }
                }
            },
        };
        if let Some(reason) = reason {
            out.push(Conflict {
                episode: text.clone(),
                reason,
            });
        }
    }
    out
}

/// A mapping the user sets, checked ([`UserMapping::new`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserMapping {
    pub offset: i64,
    pub exceptions: Vec<Exception>,
}

/// Why a mapping the user sent cannot be saved, as a sentence for the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidMapping(pub String);

impl UserMapping {
    /// Checks the offset and the exceptions (`episode`, `target`) the user sent:
    /// the offset within ±9999, each episode text non-empty and short, each
    /// target a season episode from 1 to 9999 or `None` for 받지 않음, and no
    /// two texts of one [`stored_key`].
    pub fn new(
        offset: i64,
        exceptions: Vec<(String, Option<i64>)>,
    ) -> Result<UserMapping, InvalidMapping> {
        let invalid = |message: String| Err(InvalidMapping(message));
        if offset.abs() > MAX_NUMBER {
            return invalid(format!(
                "차이는 −{MAX_NUMBER}에서 {MAX_NUMBER} 사이의 정수로 적어 주세요."
            ));
        }
        if exceptions.len() > MAX_EXCEPTIONS {
            return invalid(format!("예외는 {MAX_EXCEPTIONS}개까지 둘 수 있어요."));
        }
        let mut out: Vec<Exception> = Vec::with_capacity(exceptions.len());
        for (text, target) in exceptions {
            let episode = text.trim().to_owned();
            if episode.is_empty() {
                return invalid("예외의 회차 표시를 적어 주세요.".to_owned());
            }
            if episode.chars().count() > MAX_EPISODE_CHARS {
                return invalid(format!(
                    "회차 표시 ‘{episode}’가 너무 길어요. {MAX_EPISODE_CHARS}자까지 적을 수 있어요."
                ));
            }
            let target = match target {
                None => None,
                Some(t) if (1..=MAX_NUMBER).contains(&t) => Some(t as u32),
                Some(_) => {
                    return invalid(format!(
                    "‘{episode}’를 옮길 시즌 회차는 1에서 {MAX_NUMBER} 사이의 정수로 적어 주세요."
                ))
                }
            };
            let key = stored_key(&episode);
            if let Some(same) = out.iter().find(|e| e.key == key) {
                return invalid(if same.episode == episode {
                    format!("회차 ‘{episode}’의 예외가 둘이에요. 같은 회차에는 예외를 하나만 둘 수 있어요.")
                } else {
                    format!(
                        "회차 ‘{}’와 ‘{episode}’는 같은 회차예요. 예외를 하나만 남겨 주세요.",
                        same.episode
                    )
                });
            }
            out.push(Exception {
                key,
                episode,
                target,
            });
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(UserMapping {
            offset,
            exceptions: out,
        })
    }
}

#[cfg(test)]
mod tests;
