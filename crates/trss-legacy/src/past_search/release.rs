//! What the title of a search result says: which episode it is, whether it is
//! a batch, and how its releases write episode numbers (the notation the extra
//! searches reuse).
//!
//! - An **episode** is a whole number (`14`), optionally a half (`14.5`) and a
//!   revision (`14v2`), as a release title writes it after the work.
//! - A **batch** says so (`[Batch]`) or names a range (`(01-12)`, `01~12`,
//!   `- 01-12`); a title that names no episode at all is **unnumbered** (a
//!   movie, a special).
//! - The **notation** is how the number is attached to the work: `Work - 05`
//!   (a dash, the number zero-padded to a width) or `Work S02E05`. A search of
//!   the episodes `1000` to `1002` is written in the notation of the titles
//!   the first search returned, since releases differ (`- 01` against `- 1`).

use std::sync::LazyLock;

use regex::Regex;

use crate::{revision::Release, subscriptions::parse_release};

/// One episode of a release title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Episode {
    pub number: u32,
    /// `14.5`: a half episode.
    pub half: bool,
}

impl Episode {
    pub fn whole(number: u32) -> Episode {
        Episode {
            number,
            half: false,
        }
    }

    /// `14`, or `14.5`.
    pub fn text(self) -> String {
        if self.half {
            format!("{}.5", self.number)
        } else {
            self.number.to_string()
        }
    }
}

/// What kind of release a title is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One episode; `version` is 1 for a title without `vN`.
    Episode { episode: Episode, version: u32 },
    /// Several episodes; `range` is the numbers the title names, if it does.
    Batch { range: Option<(u32, u32)> },
    /// No episode number and not a batch.
    Unnumbered,
}

/// How a release writes the number after the work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Notation {
    /// `Work - 05`: the number is zero-padded to `width` digits.
    Dash { width: usize },
    /// `Work S02E05`.
    SeasonEpisode {
        season: u32,
        season_width: usize,
        width: usize,
    },
}

impl Notation {
    /// What follows the work in a search for `numbers`: ` - (1000|1001)` or
    /// ` (S02E05|S02E06)`. The numbers are padded as the release pads them.
    pub fn alternatives(self, numbers: &[u32]) -> String {
        match self {
            Notation::Dash { width } => {
                let list: Vec<String> = numbers.iter().map(|n| format!("{n:0width$}")).collect();
                format!(" - ({})", list.join("|"))
            }
            Notation::SeasonEpisode {
                season,
                season_width,
                width,
            } => {
                let list: Vec<String> = numbers
                    .iter()
                    .map(|n| format!("S{season:0season_width$}E{n:0width$}"))
                    .collect();
                format!(" ({})", list.join("|"))
            }
        }
    }
}

/// A title read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub kind: Kind,
    /// The release's name without revision, CRC32 bracket and extension, its
    /// revision and its CRC32 ([`crate::revision::Release`]).
    pub release: Release,
    /// How the title writes the episode; only for an episode.
    pub notation: Option<Notation>,
}

static BATCH_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bbatch\b").unwrap());
static BRACKET_RANGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[(\[【]\s*(\d{1,4})\s*[-~–]\s*(\d{1,4})\s*[)\]】]").unwrap());
static TILDE_RANGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{1,4})\s*~\s*(\d{1,4})\b").unwrap());
static WRITTEN_RANGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{1,4})\s*[-~–]\s*(\d{1,4})$").unwrap());
static WRITTEN_EPISODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{1,4})(?:\.(\d))?(?:v(\d{1,2}))?$").unwrap());
static DASH_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s-\s(\d{1,4})(?:\.\d)?(?:v\d+)?(?:\s|[(\[]|$)").unwrap());
static SEASON_EPISODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bS(\d{1,3})E(\d{1,4})(?:\.\d)?\b").unwrap());

fn range_of(first: &str, second: &str) -> Option<(u32, u32)> {
    let (a, b) = (first.parse::<u32>().ok()?, second.parse::<u32>().ok()?);
    (a <= b).then_some((a, b))
}

/// The range a batch title names: `(01-12)`, `[01~12]` or `01~12`.
fn named_range(title: &str) -> Option<(u32, u32)> {
    [&BRACKET_RANGE, &TILDE_RANGE]
        .into_iter()
        .find_map(|pattern| {
            let found = pattern.captures(title)?;
            range_of(&found[1], &found[2])
        })
}

/// Reads a release title.
pub fn read(title: &str) -> Read {
    let release = Release::parse(title);
    let parsed = parse_release(title);
    let written = parsed.as_ref().and_then(|p| p.episode.as_deref());

    let range = written
        .and_then(|w| {
            WRITTEN_RANGE
                .captures(w)
                .and_then(|c| range_of(&c[1], &c[2]))
        })
        .or_else(|| named_range(title));
    let batch_word = BATCH_WORD.is_match(title);
    if batch_word || range.is_some() {
        return Read {
            kind: Kind::Batch { range },
            release,
            notation: None,
        };
    }

    let episode = written.and_then(|w| {
        let found = WRITTEN_EPISODE.captures(w)?;
        let number = found[1].parse().ok()?;
        let half = found.get(2).is_some_and(|d| d.as_str() == "5");
        // `.0` and other fractions are not episodes of a folder.
        if found.get(2).is_some_and(|d| d.as_str() != "5") {
            return None;
        }
        Some(Episode { number, half })
    });
    let Some(episode) = episode else {
        return Read {
            kind: Kind::Unnumbered,
            release,
            notation: None,
        };
    };
    Read {
        kind: Kind::Episode {
            episode,
            version: release.version,
        },
        notation: notation_of(title),
        release,
    }
}

fn notation_of(title: &str) -> Option<Notation> {
    if let Some(found) = SEASON_EPISODE.captures(title) {
        return Some(Notation::SeasonEpisode {
            season: found[1].parse().ok()?,
            season_width: found[1].len(),
            width: found[2].len(),
        });
    }
    // The last ` - 05` that something else follows.
    let found = DASH_NUMBER.captures_iter(title).last()?;
    Some(Notation::Dash {
        width: found[1].len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(title: &str) -> Kind {
        read(title).kind
    }

    fn episode(number: u32, version: u32) -> Kind {
        Kind::Episode {
            episode: Episode::whole(number),
            version,
        }
    }

    #[test]
    fn an_episode_title_gives_its_number_and_revision() {
        assert_eq!(
            kind("[SubsPlease] Sono Bisque Doll - 14 (1080p) [E2675E51].mkv"),
            episode(14, 1)
        );
        assert_eq!(
            kind("[SubsPlease] Sono Bisque Doll - 14v2 (1080p) [1A2B3C4D].mkv"),
            episode(14, 2)
        );
        assert_eq!(
            kind("[Erai-raws] Show - 05 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv"),
            episode(5, 1)
        );
        assert_eq!(
            kind("[SubsPlease] Tensei Shitara Slime Datta Ken - 65.5 (1080p) [0214B01E].mkv"),
            Kind::Episode {
                episode: Episode {
                    number: 65,
                    half: true
                },
                version: 1
            }
        );
        assert_eq!(
            kind("[SubsPlease] One Piece - 1000 (1080p) [AAAA1111].mkv"),
            episode(1000, 1)
        );
    }

    #[test]
    fn a_batch_says_so_or_names_a_range() {
        assert_eq!(
            kind("[SubsPlease] Sayonara Lara (01-12) (1080p) [Batch]"),
            Kind::Batch {
                range: Some((1, 12))
            }
        );
        assert_eq!(
            kind("[Unofficial] Sono Bisque Doll (01-24) Unofficial Batch"),
            Kind::Batch {
                range: Some((1, 24))
            }
        );
        assert_eq!(
            kind("[SubsPlease] Sono Bisque Doll - 01~12 [Batch] (1080p)"),
            Kind::Batch {
                range: Some((1, 12))
            }
        );
        assert_eq!(
            kind("[Group] Show - 01-12 (1080p)"),
            Kind::Batch {
                range: Some((1, 12))
            }
        );
        // The word alone is a batch with no range.
        assert_eq!(
            kind("[Group] Show Complete (1080p) [Batch]"),
            Kind::Batch { range: None }
        );
    }

    #[test]
    fn a_title_without_a_number_is_unnumbered() {
        assert_eq!(
            kind("[SubsPlease] Show Movie (1080p).mkv"),
            Kind::Unnumbered
        );
    }

    #[test]
    fn the_resolution_and_the_crc_are_not_a_range() {
        assert_eq!(
            kind("[SubsPlease] Show - 05 (1080p) [ABCD1234].mkv"),
            episode(5, 1)
        );
        assert_eq!(
            kind("[SubsPlease] Show - 05 (1080p) [01234567].mkv"),
            episode(5, 1)
        );
    }

    #[test]
    fn the_release_carries_the_stem_the_version_and_the_crc() {
        let read = read("[SubsPlease] Show - 14v2 (1080p) [1A2B3C4D].mkv");
        assert_eq!(read.release.stem, "[SubsPlease] Show - 14 (1080p)");
        assert_eq!(read.release.version, 2);
        assert_eq!(read.release.crc, Some(0x1A2B3C4D));
    }

    #[test]
    fn the_notation_is_read_from_the_title() {
        let width = |title: &str| read(title).notation;
        assert_eq!(
            width("[SubsPlease] One Piece - 1000 (1080p) [AAAA1111].mkv"),
            Some(Notation::Dash { width: 4 })
        );
        assert_eq!(
            width("[SubsPlease] Show - 05 (1080p) [AAAA1111].mkv"),
            Some(Notation::Dash { width: 2 })
        );
        assert_eq!(
            width("[SubsPlease] Show - 5 (1080p) [AAAA1111].mkv"),
            Some(Notation::Dash { width: 1 })
        );
        assert_eq!(
            width("Show S02E05 1080p WEB.mkv"),
            Some(Notation::SeasonEpisode {
                season: 2,
                season_width: 2,
                width: 2
            })
        );
        assert_eq!(width("[SubsPlease] Show (01-12) (1080p) [Batch]"), None);
    }

    #[test]
    fn a_notation_writes_a_search_for_episodes_as_the_release_does() {
        assert_eq!(
            Notation::Dash { width: 4 }.alternatives(&[1000, 1001, 1002]),
            " - (1000|1001|1002)"
        );
        assert_eq!(
            Notation::Dash { width: 2 }.alternatives(&[1, 12]),
            " - (01|12)"
        );
        assert_eq!(Notation::Dash { width: 1 }.alternatives(&[7]), " - (7)");
        assert_eq!(
            Notation::SeasonEpisode {
                season: 2,
                season_width: 2,
                width: 2
            }
            .alternatives(&[5, 6]),
            " (S02E05|S02E06)"
        );
    }
}
