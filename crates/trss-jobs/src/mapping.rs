//! The episode mapping of a subtitle source to a season (`docs/specs/library.md`,
//! 자막의 회차 대응): which episode of the season a creator's Anissia episode is.
//!
//! A mapping is one default per (work, season, source): Anissia's whole
//! episode `n` is the season's episode `n + offset`. The app decides it here
//! only for the subscribed creator's source ([`crate::follow`]), without
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
//! ([`store_in`]): the mapping goes `undecided` and says both offsets, and it
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
//! never received and never a conflict. A conflict is not received by itself;
//! the source's other episodes are. The set is stored with the mapping
//! ([`store_conflicts`]) for the to-do and screen of ticket 0052.
//!
//! The mapping is decided again whenever the subscribed creator's episodes are
//! looked at, so a new episode that points to another offset takes an `auto`
//! mapping back to `undecided` until the user maps it.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{params, Connection, OptionalExtension};
use trss_collect::{
    episode_offset::{ranges, signed},
    store::anissia::numeric_episode,
};
use trss_core::Millis;

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

/// A source's mapping to a season, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    pub kind: MappingKind,
    /// What to add to Anissia's whole episode; `None` while undecided.
    pub offset: Option<i64>,
    /// Why: the grounds that agree, or the reason none do.
    pub evidence: String,
    pub decided_at: Millis,
}

impl Mapping {
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
        let reason = match numeric_episode(text) {
            None => Some(format!(
                "숫자가 아닌 회차({text})는 정한 차이로 알 수 없어요"
            )),
            Some(n) if n == "0" => None,
            Some(n) if n.contains('.') => {
                Some(format!("소수 회차({text})는 정한 차이로 알 수 없어요"))
            }
            Some(n) => match n.parse::<u32>() {
                Err(_) => Some(format!("회차 번호({text})가 너무 커요")),
                Ok(n) => {
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

fn mapping_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<(String, Mapping, Option<i64>)> {
    let code: String = r.get(1)?;
    let kind = MappingKind::parse(&code).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            format!("unknown mapping kind {code:?}").into(),
        )
    })?;
    Ok((
        r.get(0)?,
        Mapping {
            kind,
            offset: r.get(2)?,
            evidence: r.get(3)?,
            decided_at: r.get(4)?,
        },
        r.get(5)?,
    ))
}

/// The mappings of season `season` of the work, by source.
pub fn read_in(
    c: &Connection,
    work_id: &str,
    season: u32,
) -> rusqlite::Result<HashMap<String, Mapping>> {
    let mut stmt = c.prepare(
        "SELECT source_id, kind, episode_offset, evidence, decided_at, retired_offset
           FROM subtitle_episode_mappings WHERE work_id = ?1 AND season = ?2",
    )?;
    let rows = stmt.query_map(params![work_id, season], mapping_of)?;
    rows.map(|row| row.map(|(source, mapping, _)| (source, mapping)))
        .collect()
}

/// Writes what the app decided for the source, unless the user set its
/// mapping or the same is stored already. Returns the mapping now stored.
///
/// The app never takes up another offset by itself: when the stored mapping is
/// `auto` and what is decided now is another offset, the mapping goes
/// `undecided` and says both. When it goes `undecided` from `auto` (that or any
/// other reason), the offset it had is kept as `retired_offset`, and while
/// that is set only the same offset is decided again; any other is refused the
/// same way.
pub fn store_in(
    c: &Connection,
    work_id: &str,
    season: u32,
    source_id: &str,
    decided: &Decided,
    now: Millis,
) -> rusqlite::Result<Mapping> {
    let read = || {
        c.query_row(
            "SELECT source_id, kind, episode_offset, evidence, decided_at, retired_offset
               FROM subtitle_episode_mappings
              WHERE work_id = ?1 AND season = ?2 AND source_id = ?3",
            params![work_id, season, source_id],
            mapping_of,
        )
        .optional()
        .map(|row| row.map(|(_, m, retired)| (m, retired)))
    };
    let stored = read()?;
    let (mut offset, mut evidence) = (decided.offset, decided.evidence.clone());
    let mut retired: Option<i64> = None;
    if let Some((m, kept)) = &stored {
        match m.kind {
            MappingKind::User => return Ok(m.clone()),
            MappingKind::Auto => retired = m.offset,
            MappingKind::Undecided => retired = *kept,
        }
    }
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
    if let Some((m, kept)) = &stored {
        if m.kind == kind && m.offset == offset && m.evidence == evidence && *kept == retired {
            return Ok(m.clone());
        }
    }
    c.execute(
        "INSERT INTO subtitle_episode_mappings
             (work_id, season, source_id, kind, episode_offset, evidence, decided_at,
              retired_offset)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (work_id, season, source_id) DO UPDATE SET
             kind = excluded.kind, episode_offset = excluded.episode_offset,
             evidence = excluded.evidence, decided_at = excluded.decided_at,
             retired_offset = excluded.retired_offset
         WHERE subtitle_episode_mappings.kind <> 'user'",
        params![
            work_id,
            season,
            source_id,
            kind.code(),
            offset,
            evidence,
            now,
            retired
        ],
    )?;
    // The row as it is now: a user's mapping that came in between stays.
    Ok(read()?.map(|(m, _)| m).unwrap_or(Mapping {
        kind,
        offset,
        evidence,
        decided_at: now,
    }))
}

/// Makes the stored conflicts of the source in the season exactly `found`: an
/// episode that was a conflict already keeps its `found_at`.
pub fn store_conflicts(
    c: &Connection,
    work_id: &str,
    season: u32,
    source_id: &str,
    found: &[Conflict],
    now: Millis,
) -> rusqlite::Result<()> {
    let before: HashMap<String, (String, Millis)> = {
        let mut stmt = c.prepare(
            "SELECT episode, reason, found_at FROM subtitle_mapping_conflicts
              WHERE work_id = ?1 AND season = ?2 AND source_id = ?3",
        )?;
        let rows = stmt.query_map(params![work_id, season, source_id], |r| {
            Ok((r.get(0)?, (r.get(1)?, r.get(2)?)))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let same = before.len() == found.len()
        && found.iter().all(|f| {
            before
                .get(&f.episode)
                .is_some_and(|(reason, _)| *reason == f.reason)
        });
    if same {
        return Ok(());
    }
    c.execute(
        "DELETE FROM subtitle_mapping_conflicts
          WHERE work_id = ?1 AND season = ?2 AND source_id = ?3",
        params![work_id, season, source_id],
    )?;
    for f in found {
        c.execute(
            "INSERT INTO subtitle_mapping_conflicts
                 (work_id, season, source_id, episode, reason, found_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                work_id,
                season,
                source_id,
                f.episode,
                f.reason,
                before.get(&f.episode).map_or(now, |(_, at)| *at)
            ],
        )?;
    }
    Ok(())
}

/// The stored conflicts of the source in the season, by episode text.
pub fn conflicts_in(
    c: &Connection,
    work_id: &str,
    season: u32,
    source_id: &str,
) -> rusqlite::Result<Vec<Conflict>> {
    let mut stmt = c.prepare(
        "SELECT episode, reason FROM subtitle_mapping_conflicts
          WHERE work_id = ?1 AND season = ?2 AND source_id = ?3 ORDER BY episode",
    )?;
    let rows = stmt.query_map(params![work_id, season, source_id], |r| {
        Ok(Conflict {
            episode: r.get(0)?,
            reason: r.get(1)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests;
