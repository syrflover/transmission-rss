//! The episode mapping of a subtitle source to a season (`docs/specs/library.md`,
//! 자막의 회차 대응): which episode of the season a creator's Anissia episode is.
//!
//! A mapping is one default per (work, season, source): Anissia's whole
//! episode `n` is the season's episode `n + offset`. The app decides it here
//! only for the subscribed creator's source ([`crate::follow`]), and only when
//! the grounds agree, without asking (the subscription settled the anime and
//! the season already):
//!
//! | grounds | mapping |
//! | --- | --- |
//! | the creator's whole episodes `E` are all within `1..=N`, `N` the season's AniList episode count | `auto`, offset 0 (`Anissia 1–4화가 시즌 회차 수(12) 안에 있어요`) |
//! | otherwise, the rule's 회차 변환 shifts its numbers by `k ≠ 0` and every `e + k` is within `1..=N` | `auto`, offset `k` (`규칙의 회차 변환(13→S02E01)과 AniList 회차 수(12)가 맞아요`) |
//! | `N` unknown, no whole episode seen yet, or neither fits | `undecided`, with the reason |
//!
//! The rule's 회차 변환 is the video's, not the subtitle's (`docs/specs/
//! collection.md`, 영상 회차 변환): it is read here as evidence only, never
//! copied. A negative value `v` takes `v` from a release's number, a positive
//! one makes `1` episode `v` (`v − 1` is added), and `0` and `1` leave numbers
//! as they are, as `trname` reads it.
//!
//! `0` (the line a creator registers before the first episode), decimals
//! (`13.5`) and other texts are no evidence and are never mapped by the
//! offset: an exception the user writes would map them, which is a later
//! screen's (result goal 4), as is changing a mapping. A mapping the user set
//! (`user`) is never touched by the app.
//!
//! The mapping is recomputed whenever the subscribed creator's episodes are
//! looked at, so a new episode past the count turns an `auto` mapping into
//! `undecided` until the grounds agree again.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use trss_collect::episode_offset::{ranges, signed};
use trss_core::Millis;

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

/// How much the rule's 회차 변환 `value` adds to a release's number, as
/// `trname` applies it.
pub fn shift_of(value: i64) -> i64 {
    match value {
        v if v < 0 => v,
        v if v > 1 => v - 1,
        _ => 0,
    }
}

/// The mapping of the creator's whole episodes `episodes` (ascending, no
/// repeats, none below 1) to season `season`, whose AniList episode count is
/// `count`, given the rule's 회차 변환 `rule_episode` (see the module docs).
pub fn decide(episodes: &[u32], count: Option<u32>, rule_episode: i64, season: u32) -> Decided {
    let undecided = |evidence: String| Decided {
        offset: None,
        evidence,
    };
    let (Some(&first), Some(&last)) = (episodes.first(), episodes.last()) else {
        return undecided("구독 제작자의 숫자 회차를 아직 보지 못했어요".to_owned());
    };
    let Some(n) = count else {
        return undecided(format!(
            "시즌 {season}의 AniList 회차 수를 몰라서 정하지 않았어요"
        ));
    };
    let within = |shift: i64| {
        let (lo, hi) = (i64::from(first) + shift, i64::from(last) + shift);
        lo >= 1 && hi <= i64::from(n)
    };
    let written = ranges(episodes);
    if within(0) {
        return Decided {
            offset: Some(0),
            evidence: format!("Anissia {written}화가 시즌 회차 수({n}) 안에 있어요"),
        };
    }
    let shift = shift_of(rule_episode);
    if shift != 0 && within(shift) {
        let video = i64::from(first) + shift;
        return Decided {
            offset: Some(shift),
            evidence: format!(
                "규칙의 회차 변환({first}→S{season:02}E{video:02})과 AniList 회차 수({n})가 맞아요"
            ),
        };
    }
    undecided(match shift {
        0 => format!("Anissia {written}화가 시즌 회차 수({n})를 넘고, 규칙에 회차 변환이 없어요"),
        _ => format!(
            "Anissia {written}화가 시즌 회차 수({n})를 넘고, 규칙의 회차 변환({})으로도 맞지 않아요",
            signed(shift)
        ),
    })
}

fn mapping_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<(String, Mapping)> {
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
    ))
}

/// The mappings of season `season` of the work, by source.
pub fn read_in(
    c: &Connection,
    work_id: &str,
    season: u32,
) -> rusqlite::Result<HashMap<String, Mapping>> {
    let mut stmt = c.prepare(
        "SELECT source_id, kind, episode_offset, evidence, decided_at
           FROM subtitle_episode_mappings WHERE work_id = ?1 AND season = ?2",
    )?;
    let rows = stmt.query_map(params![work_id, season], mapping_of)?;
    rows.collect()
}

/// Writes what the app decided for the source, unless the user set its
/// mapping or the same is stored already. Returns the mapping now stored.
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
            "SELECT source_id, kind, episode_offset, evidence, decided_at
               FROM subtitle_episode_mappings
              WHERE work_id = ?1 AND season = ?2 AND source_id = ?3",
            params![work_id, season, source_id],
            mapping_of,
        )
        .optional()
        .map(|row| row.map(|(_, m)| m))
    };
    let stored = read()?;
    let kind = match decided.offset {
        Some(_) => MappingKind::Auto,
        None => MappingKind::Undecided,
    };
    match stored {
        Some(m) if m.kind == MappingKind::User => return Ok(m),
        Some(m)
            if m.kind == kind && m.offset == decided.offset && m.evidence == decided.evidence =>
        {
            return Ok(m)
        }
        _ => {}
    }
    c.execute(
        "INSERT INTO subtitle_episode_mappings
             (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (work_id, season, source_id) DO UPDATE SET
             kind = excluded.kind, episode_offset = excluded.episode_offset,
             evidence = excluded.evidence, decided_at = excluded.decided_at
         WHERE subtitle_episode_mappings.kind <> 'user'",
        params![
            work_id,
            season,
            source_id,
            kind.code(),
            decided.offset,
            decided.evidence,
            now
        ],
    )?;
    // The row as it is now: a user's mapping that came in between stays.
    Ok(read()?.unwrap_or(Mapping {
        kind,
        offset: decided.offset,
        evidence: decided.evidence.clone(),
        decided_at: now,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_decided_mapping_has_an_offset_whatever_the_stored_number_is() {
        let mapping = |kind, offset| Mapping {
            kind,
            offset,
            evidence: String::new(),
            decided_at: 0,
        };
        assert_eq!(
            mapping(MappingKind::Auto, Some(-12)).decided_offset(),
            Some(-12)
        );
        assert_eq!(
            mapping(MappingKind::User, Some(0)).decided_offset(),
            Some(0)
        );
        assert_eq!(
            mapping(MappingKind::Undecided, Some(3)).decided_offset(),
            None
        );
        assert_eq!(mapping(MappingKind::Undecided, None).decided_offset(), None);
        assert_eq!(mapping(MappingKind::Auto, None).decided_offset(), None);
    }

    #[test]
    fn the_rule_shifts_its_numbers_as_trname_does() {
        assert_eq!(
            [-24, -12, 0, 1, 13].map(shift_of),
            [-24, -12, 0, 0, 12],
            "0 and 1 leave numbers as they are; 13 makes 1 episode 13"
        );
    }

    #[test]
    fn episodes_within_the_season_map_as_they_are() {
        let d = decide(&[1, 2, 3, 4], Some(12), 0, 1);
        assert_eq!(d.offset, Some(0));
        assert_eq!(d.evidence, "Anissia 1–4화가 시즌 회차 수(12) 안에 있어요");
    }

    #[test]
    fn cumulative_episodes_map_by_the_rules_conversion_when_the_count_agrees() {
        // Season 2 of 12 after a season of 12: Anissia's 13 is S02E01.
        let d = decide(&[13, 14], Some(12), -12, 2);
        assert_eq!(d.offset, Some(-12));
        assert_eq!(
            d.evidence,
            "규칙의 회차 변환(13→S02E01)과 AniList 회차 수(12)가 맞아요"
        );
    }

    #[test]
    fn grounds_that_do_not_agree_leave_the_mapping_undecided() {
        // Past the count with no conversion; a conversion that does not fit
        // either; no count; nothing seen.
        for (episodes, count, rule, why) in [
            (&[13u32, 14][..], Some(12), 1, "규칙에 회차 변환이 없어요"),
            (
                &[13, 14][..],
                Some(12),
                -1,
                "규칙의 회차 변환(−1)으로도 맞지 않아요",
            ),
            (
                &[25][..],
                Some(12),
                -12,
                "규칙의 회차 변환(−12)으로도 맞지 않아요",
            ),
            (&[1][..], None, 0, "AniList 회차 수를 몰라서"),
            (&[][..], Some(12), 0, "숫자 회차를 아직 보지 못했어요"),
        ] {
            let d = decide(episodes, count, rule, 2);
            assert_eq!(d.offset, None, "{episodes:?}");
            assert!(d.evidence.contains(why), "{}", d.evidence);
        }
    }
}
