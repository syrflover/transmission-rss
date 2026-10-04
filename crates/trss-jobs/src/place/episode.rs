//! Which episode of the season a received subtitle is (`docs/specs/subtitles.md`,
//! 파일의 회차): for a candidate's job, the candidate's episode taken through
//! its source's mapping, checked against the number the file's name says.

use trss_subtitles::episode::{holds, numeric_key, Holds};

use crate::mapping::{whole, Mapped, Mapping};

/// What puts a stored subtitle on an episode (`assignment` in
/// `docs/specs/settings.md`, 보관 관계 필드).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assignment {
    /// The source's mapping from an observed episode: recomputed when the
    /// mapping changes.
    Mapped,
    /// A person's choice, or the same number where the source had no
    /// mapping.
    Explicit,
}

impl Assignment {
    pub fn code(self) -> &'static str {
        match self {
            Assignment::Mapped => "mapped",
            Assignment::Explicit => "explicit",
        }
    }

    pub fn parse(code: &str) -> Option<Assignment> {
        match code {
            "mapped" => Some(Assignment::Mapped),
            "explicit" => Some(Assignment::Explicit),
            _ => None,
        }
    }
}

/// Which observed episode a mapped assignment starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// The candidate's episode as Anissia wrote it.
    Anissia,
    /// The episode the file's name says.
    Attachment,
}

impl Basis {
    pub fn code(self) -> &'static str {
        match self {
            Basis::Anissia => "anissia",
            Basis::Attachment => "attachment",
        }
    }

    pub fn parse(code: &str) -> Option<Basis> {
        match code {
            "anissia" => Some(Basis::Anissia),
            "attachment" => Some(Basis::Attachment),
            _ => None,
        }
    }
}

/// Where a file goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The season's episode, and what puts the file there.
    Episode {
        episode: i64,
        assignment: Assignment,
        /// `Some` for a mapped assignment.
        basis: Option<Basis>,
    },
    /// A person has to say which episode it is (`회차 확인 필요`), for this
    /// reason.
    Ask(String),
}

/// What the name of a file says of its episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Named {
    /// No number that stands for an episode.
    Nothing,
    /// One episode, as [`numeric_key`] has it (`13`, `13.5`).
    One(String),
    /// Several episodes or a range: not one file of one episode.
    Several,
}

/// What `name` (a file's name, its extension included) says of its episode
/// ([`holds`]'s reading of a link's words).
pub fn named(name: &str) -> Named {
    match holds(name) {
        Holds::Episodes(spans) => match spans.as_slice() {
            [one] if one.from == one.to => Named::One(one.from.clone()),
            _ => Named::Several,
        },
        Holds::Font | Holds::Bundle | Holds::Nothing => Named::Nothing,
    }
}

/// `13화`, or the text as it is when it is no number.
fn label(episode: &str) -> String {
    match numeric_key(episode) {
        Some(_) => format!("{episode}화"),
        None => episode.to_owned(),
    }
}

/// The season episode a candidate's episode text is: through the source's
/// mapping, or the same number where the source has none.
fn candidate_target(candidate: &str, mapping: Option<&Mapping>) -> Result<(i64, Target), String> {
    match mapping {
        None => match whole(candidate) {
            Some(n) => Ok((
                n,
                Target::Episode {
                    episode: n,
                    assignment: Assignment::Explicit,
                    basis: None,
                },
            )),
            None => Err(format!(
                "후보의 회차 {}는 정수 회차가 아니라 시즌의 회차로 옮기지 못했어요",
                label(candidate)
            )),
        },
        Some(mapping) => match mapping.season_episode(candidate) {
            Mapped::Episode(n) => Ok((
                n,
                Target::Episode {
                    episode: n,
                    assignment: Assignment::Mapped,
                    basis: Some(Basis::Anissia),
                },
            )),
            Mapped::NotReceived => Err(format!(
                "회차 대응이 후보의 {}를 받지 않는 회차로 정해 두었어요",
                label(candidate)
            )),
            Mapped::Unmapped if mapping.decided_offset().is_none() => {
                Err("이 제작자의 회차 대응이 아직 미정이에요".to_owned())
            }
            Mapped::Unmapped => Err(format!(
                "후보의 회차 {}는 회차 대응으로 옮길 수 없는 회차예요",
                label(candidate)
            )),
        },
    }
}

/// Where the subtitle `name` of a candidate's job goes: the candidate's
/// episode `candidate` taken through the source's `mapping` (`None`: the
/// source has none, so the same number), within the season's `total`
/// episodes when that is known. The file's name must say that episode, as the
/// candidate wrote it or as the season has it, or say none when the file is
/// the only subtitle the candidate brought (`alone`).
pub fn of_candidate(
    candidate: &str,
    mapping: Option<&Mapping>,
    name: &str,
    total: Option<u32>,
    alone: bool,
) -> Target {
    let (episode, target) = match candidate_target(candidate, mapping) {
        Ok(found) => found,
        Err(reason) => return Target::Ask(reason),
    };
    if episode < 1 || total.is_some_and(|n| episode > i64::from(n)) {
        let range = match total {
            Some(n) => format!("1–{n}화"),
            None => "1화부터".to_owned(),
        };
        return Target::Ask(format!(
            "후보의 {}를 옮긴 시즌 {episode}화가 시즌의 {range} 밖이에요",
            label(candidate)
        ));
    }
    match named(name) {
        Named::Nothing if alone => target,
        Named::Nothing => Target::Ask("파일 이름에 회차 번호가 없어요".to_owned()),
        Named::Several => Target::Ask("파일 이름이 회차 여럿을 가리켜요".to_owned()),
        Named::One(key) => {
            let as_written = numeric_key(candidate.trim()).is_some_and(|c| c == key);
            if as_written || key == episode.to_string() {
                target
            } else {
                Target::Ask(format!(
                    "파일 이름의 {key}화가 후보의 {}(시즌 {episode}화)와 달라요",
                    label(candidate)
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::MappingKind;

    fn offset(offset: i64) -> Mapping {
        Mapping {
            kind: MappingKind::User,
            offset: Some(offset),
            evidence: "시험".to_owned(),
            decided_at: 0,
            version: 1,
            exceptions: Vec::new(),
        }
    }

    fn mapped(episode: i64) -> Target {
        Target::Episode {
            episode,
            assignment: Assignment::Mapped,
            basis: Some(Basis::Anissia),
        }
    }

    // The examples of `docs/specs/subtitles.md`, 파일의 회차.
    #[test]
    fn the_candidates_episode_goes_through_the_mapping() {
        let m = offset(-12);
        assert_eq!(
            of_candidate("14", Some(&m), "Show - 14.ass", Some(12), true),
            mapped(2)
        );
        assert_eq!(
            of_candidate("14", Some(&m), "Show S2 - 02.ass", Some(12), true),
            mapped(2)
        );
        assert_eq!(
            of_candidate("14", Some(&m), "Show.ass", Some(12), true),
            mapped(2)
        );
        let Target::Ask(reason) = of_candidate("14", Some(&m), "Show - 13.ass", Some(12), true)
        else {
            panic!("13 is neither 14 nor 2");
        };
        assert!(reason.contains("13화"), "{reason}");
    }

    #[test]
    fn a_source_without_a_mapping_is_the_same_number() {
        assert_eq!(
            of_candidate("13", None, "키미시누 13화 미완성.ass", Some(13), true),
            Target::Episode {
                episode: 13,
                assignment: Assignment::Explicit,
                basis: None
            }
        );
        // Leading zeros are one episode.
        assert!(matches!(
            of_candidate(
                "8",
                None,
                "[SubsPlease] Show - 08 (1080p) [F3B053C5].ass",
                None,
                true
            ),
            Target::Episode { episode: 8, .. }
        ));
    }

    #[test]
    fn what_the_mapping_cannot_place_is_asked() {
        let undecided = Mapping {
            kind: MappingKind::Undecided,
            offset: None,
            ..offset(0)
        };
        assert!(matches!(
            of_candidate("3", Some(&undecided), "a - 03.ass", None, true),
            Target::Ask(r) if r.contains("미정")
        ));
        // A decimal episode, and one outside the season.
        assert!(matches!(
            of_candidate("13.5", None, "a.ass", None, true),
            Target::Ask(_)
        ));
        assert!(matches!(
            of_candidate("14", None, "a - 14.ass", Some(12), true),
            Target::Ask(r) if r.contains("1–12화")
        ));
        assert!(matches!(
            of_candidate("3", Some(&offset(-12)), "a.ass", Some(12), true),
            Target::Ask(_)
        ));
    }

    #[test]
    fn a_name_without_a_number_is_the_candidates_only_when_alone() {
        assert!(matches!(
            of_candidate("2", None, "Show.ass", None, false),
            Target::Ask(_)
        ));
        assert!(matches!(
            of_candidate("2", None, "Show 01~12.ass", None, true),
            Target::Ask(_)
        ));
        assert_eq!(named("dev-check16.srt"), Named::Nothing);
        assert_eq!(named("Show - 1~12화.ass"), Named::Several);
    }
}
