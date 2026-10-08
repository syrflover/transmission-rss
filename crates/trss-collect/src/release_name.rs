//! What a release name says (`docs/specs/collection.md`, 영상 수정본의 대체 and
//! 영상 회차 변환): which work and which episode it is, whether it is a batch,
//! its revision (`14v2`), the CRC32 the release group put in it, and how it
//! writes the episode number. One reader, [`ReleaseName::read`], for the names
//! of RSS titles (which may leave the extension out), of search results, of
//! history and of the torrents Transmission holds.
//!
//! - **The work** is the title between the group tags and the episode
//!   (`[SubsPlease] Work - 01 (1080p)` is `Work`), the rule's match phrase.
//!   A name with no episode keeps its work without the trailing `(…)` and
//!   `[…]` details.
//! - An **episode** is a whole number (`14`), optionally a half (`14.5`) and a
//!   revision (`14v2`), as a release title writes it after the work.
//! - A **batch** says so (`[Batch]`) or names a range (`(01-12)`, `01~12`,
//!   `- 01-12`); a name that has no episode at all is **unnumbered** (a movie,
//!   a special).
//! - **The CRC32** is the last bracket before the extension when it holds
//!   exactly eight hexadecimal digits: `… (1080p) [E2675E51].mkv`, and with
//!   Erai-raws' several brackets `… [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv`.
//!   A name whose last bracket is something else (`[MultiSub]`) has none.
//! - **The revision** is the `vN` right after the episode's number, which
//!   follows ` - ` (`Show - 14v2`, `Show 3v3 - 06v3`), whatever follows it
//!   (`… - 03v2 1080p AAC 2.0`, `… - 03v2 - Part 2`). A name with no such
//!   marker takes the last `vN` right after a number (`Show 14v2 (1080p)`),
//!   unless ` - ` and a number follow it outside brackets: then it is part of
//!   the show's name (`Show 3v3 - 06` is the first revision of episode 6).
//!   Without one the release is its first revision.
//! - **The same release** of an episode is the name without its revision,
//!   its CRC32 bracket and its extension ([`ReleaseName::stem`]): `[SubsPlease]
//!   Show - 14 (1080p)` for both `14` and `14v2`. Another group's release of
//!   the same episode has another stem, so it is a duplicate, not a revision.
//! - The **notation** is how the number is attached to the work: `Work - 05`
//!   (a dash, the number zero-padded to a width) or `Work S02E05`. A search of
//!   the episodes `1000` to `1002` is written in the notation of the titles
//!   the first search returned, since releases differ (`- 01` against `- 1`).

use std::sync::LazyLock;

use regex::{Captures, Regex};

/// One episode of a release name.
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

/// What kind of release a name is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One episode; the revision is [`ReleaseName::version`].
    Episode(Episode),
    /// Several episodes; `range` is the numbers the name gives, if it does.
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

/// A release name read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseName {
    /// The leading `[Group]` tags, without the brackets.
    pub groups: Vec<String>,
    /// The title of the work: what a rule looks for. `None` when no work is
    /// left once the group tags and the trailing details are taken away.
    pub work: Option<String>,
    pub kind: Kind,
    /// The name without its revision, CRC32 bracket and extension, trimmed.
    pub stem: String,
    /// 1 for a name without `vN`.
    pub version: u32,
    /// The CRC32 the name carries.
    pub crc: Option<u32>,
    /// How the name writes the episode; only for an episode.
    pub notation: Option<Notation>,
    /// The episode as written (`01`, `12v2`, `01-12`), if the name has one.
    written: Option<String>,
    without_revision: String,
}

// The extension, as the revision reading takes it: a dot and two to four
// letters or digits.
static EXTENSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.[A-Za-z0-9]{2,4}\s*$").unwrap());
static CRC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([0-9A-Fa-f]{8})\]\s*$").unwrap());
static VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(\d{1,4}(?:\.\d)?)v(\d{1,2})\b").unwrap());

/// ` - ` and a number, as a name gives its episode's number after the
/// show's name.
static EPISODE_AFTER_DASH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s-\s*\d{1,4}(?:\.\d)?(?:\D|$)").unwrap());

// The work and the episode, as the subscription flow takes them.
static LEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:\[([^\]]*)\]|【([^】]*)】)\s*").unwrap());
static WORK_EXTENSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.(?:mkv|mp4|avi|torrent)$").unwrap());
/// `Work - 01 (1080p)`: the last ` - <number>` that something else follows.
static DASH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(.*\S)\s+-\s+(\d{1,4}(?:\.\d)?(?:v\d+)?(?:\s*-\s*\d{1,4})?)(?:\s|[(\[]|$)")
        .unwrap()
});
/// `Work S01E03`.
static SXXEYY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(.*?\S)\s+S\d{1,2}E(\d{1,4}(?:\.\d)?)(?:\s|[(\[.]|$)").unwrap()
});
/// `Work 04 [BDRip ...]`: the number after the title with no dash.
static BARE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.*\S)\s+(\d{1,3}(?:v\d+)?)\s*(?:[(\[].*)?$").unwrap());
static TRAILING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*(?:\([^)]*\)|\[[^\]]*\])\s*$").unwrap());

// Batches and written episodes.
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

/// The episode's revision marker in `text`. The last `NvM` right after ` - `
/// is on the episode's number, whatever follows it. Without one, the last
/// `NvM`, unless ` - ` and a number follow it outside brackets: the episode's
/// number follows the show's name, so in `Show 3v3 - 06` the `3v3` is part
/// of the name and the episode `06` carries no revision.
fn last_version(text: &str) -> Option<Captures<'_>> {
    let mut all: Vec<Captures<'_>> = VERSION.captures_iter(text).collect();
    let on_episode = all
        .iter()
        .rposition(|c| text[..c.get(0).unwrap().start()].trim_end().ends_with('-'));
    if let Some(at) = on_episode {
        return Some(all.swap_remove(at));
    }
    let found = all.pop()?;
    let after = &text[found.get(0).unwrap().end()..];
    let episode_after = EPISODE_AFTER_DASH
        .find_iter(after)
        .any(|dash| bracket_depth(&after[..dash.start()]) == 0);
    (!episode_after).then_some(found)
}

/// How deep in brackets or parentheses the end of `text` is, counting from
/// its start (a closing one with none open counts as none).
fn bracket_depth(text: &str) -> usize {
    text.chars().fold(0usize, |depth, c| match c {
        '[' | '(' => depth + 1,
        ']' | ')' => depth.saturating_sub(1),
        _ => depth,
    })
}

/// The revision, the CRC32 and the stem of a name, and the name without its
/// revision.
struct Revision {
    stem: String,
    version: u32,
    crc: Option<u32>,
    without_revision: String,
}

fn read_revision(name: &str) -> Revision {
    let name = name.trim();
    let mut end = name.len();
    if let Some(found) = EXTENSION.find(name) {
        end = found.start();
    }
    let mut crc = None;
    if let Some(found) = CRC.captures(&name[..end]) {
        crc = Some(u32::from_str_radix(&found[1], 16).expect("eight hex digits"));
        end = found.get(0).unwrap().start();
    }
    let mut stem = name[..end].to_owned();
    let mut without_revision = name.to_owned();
    let mut version = 1;
    if let Some(found) = last_version(&name[..end]) {
        version = found[2].parse().unwrap_or(1).max(1);
        let range = found.get(1).unwrap().end()..found.get(0).unwrap().end();
        stem.replace_range(range.clone(), "");
        without_revision.replace_range(range, "");
    }
    Revision {
        stem: stem.trim().to_owned(),
        version,
        crc,
        without_revision,
    }
}

/// The group tags, the work and the episode as written.
struct Work {
    groups: Vec<String>,
    work: String,
    episode: Option<String>,
}

/// Reads the work and the episode out of a release name such as
/// `[SubsPlease] Work - 01 (1080p) [ABCD1234].mkv`. `None` when no work is left
/// once the group tags and the trailing details are taken away.
fn read_work(name: &str) -> Option<Work> {
    let mut rest = name.trim();
    let mut groups = Vec::new();
    while let Some(found) = LEADING.captures(rest) {
        let tag = found.get(1).or(found.get(2)).map_or("", |m| m.as_str());
        if !tag.trim().is_empty() {
            groups.push(tag.trim().to_owned());
        }
        rest = &rest[found.get(0).map_or(0, |m| m.end())..];
    }
    let rest = WORK_EXTENSION.replace(rest, "");
    let rest = rest.trim();

    for pattern in [&DASH, &SXXEYY, &BARE] {
        if let Some(found) = pattern.captures(rest) {
            let work = found[1].trim();
            if !work.is_empty() {
                return Some(Work {
                    groups,
                    work: work.to_owned(),
                    episode: Some(found[2].to_owned()),
                });
            }
        }
    }

    // No episode: a batch or a movie. Take the title without its trailing
    // `(…)` and `[…]` details.
    let mut work = rest;
    while let Some(m) = TRAILING.find(work) {
        work = &work[..m.start()];
    }
    let work = work.trim();
    (!work.is_empty()).then(|| Work {
        groups,
        work: work.to_owned(),
        episode: None,
    })
}

fn range_of(first: &str, second: &str) -> Option<(u32, u32)> {
    let (a, b) = (first.parse::<u32>().ok()?, second.parse::<u32>().ok()?);
    (a <= b).then_some((a, b))
}

/// The range a batch name gives: `(01-12)`, `[01~12]` or `01~12`.
fn named_range(name: &str) -> Option<(u32, u32)> {
    [&BRACKET_RANGE, &TILDE_RANGE]
        .into_iter()
        .find_map(|pattern| {
            let found = pattern.captures(name)?;
            range_of(&found[1], &found[2])
        })
}

fn kind_of(name: &str, written: Option<&str>) -> Kind {
    let range = written
        .and_then(|w| {
            WRITTEN_RANGE
                .captures(w)
                .and_then(|c| range_of(&c[1], &c[2]))
        })
        .or_else(|| named_range(name));
    if BATCH_WORD.is_match(name) || range.is_some() {
        return Kind::Batch { range };
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
    match episode {
        Some(episode) => Kind::Episode(episode),
        None => Kind::Unnumbered,
    }
}

fn notation_of(name: &str) -> Option<Notation> {
    if let Some(found) = SEASON_EPISODE.captures(name) {
        return Some(Notation::SeasonEpisode {
            season: found[1].parse().ok()?,
            season_width: found[1].len(),
            width: found[2].len(),
        });
    }
    // The last ` - 05` that something else follows.
    let found = DASH_NUMBER.captures_iter(name).last()?;
    Some(Notation::Dash {
        width: found[1].len(),
    })
}

/// `name` without its revision ([`ReleaseName::without_revision`]), as the
/// name `trname` reads an episode from.
pub fn without_revision(name: &str) -> String {
    ReleaseName::read(name).without_revision().to_owned()
}

impl ReleaseName {
    /// Reads a release name.
    pub fn read(name: &str) -> ReleaseName {
        let revision = read_revision(name);
        let (groups, work, written) = match read_work(name) {
            Some(Work {
                groups,
                work,
                episode,
            }) => (groups, Some(work), episode),
            None => (Vec::new(), None, None),
        };
        let kind = kind_of(name, written.as_deref());
        let notation = match kind {
            Kind::Episode(_) => notation_of(name),
            _ => None,
        };
        ReleaseName {
            groups,
            work,
            kind,
            stem: revision.stem,
            version: revision.version,
            crc: revision.crc,
            notation,
            written,
            without_revision: revision.without_revision,
        }
    }

    /// The episode as the name writes it (`01`, `12v2`, `01-12`), if it has one.
    pub fn written_episode(&self) -> Option<&str> {
        self.written.as_deref()
    }

    /// The episode the name gives as a whole number (`12`, `12v2`); not a
    /// batch (`01-12`) or a half episode.
    pub fn whole_episode(&self) -> Option<u32> {
        let written = self.written.as_deref()?;
        let digits: String = written.chars().take_while(char::is_ascii_digit).collect();
        let rest = &written[digits.len()..];
        let revision = rest
            .strip_prefix('v')
            .is_some_and(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()));
        if digits.is_empty() || !(rest.is_empty() || revision) {
            return None;
        }
        digits.parse().ok()
    }

    /// The name without its revision (`14v2` becomes `14`), everything else as
    /// it is: what the episode's file name is derived from. `trname` does not
    /// read `06v2` as episode 6 in every name (Erai-raws' `… - 06v2 [1080p CR
    /// WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv` gives episode 34).
    pub fn without_revision(&self) -> &str {
        &self.without_revision
    }
}

#[cfg(test)]
mod tests;
