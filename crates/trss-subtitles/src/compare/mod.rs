//! What differs between two subtitle files (`docs/specs/subtitles.md`, 교체
//! 비교와 승인; ticket 0069): the content a person judges when a new subtitle
//! would replace a current one. Pure: bytes in, a value out, no I/O.
//!
//! [`read`] turns a file's bytes into a [`Script`], or says why it cannot
//! ([`Unreadable`], a Korean reason for the screen). [`compare`] sets two
//! scripts side by side as a [`Diff`] of the four items the screen shows:
//! dialogue (`대사`), timing (`타이밍`), styles (`스타일`) and fonts (`폰트`).
//! A [`Diff`] serializes to compact JSON, so a replacement plan can keep it.
//! A file that cannot be read is never compared: the caller shows
//! `비교 불가` with the reason, not an empty [`Diff`]. What cannot be compared
//! between two readable files (a style of an SRT) is in [`Diff::not_compared`]
//! and its item is `None`, never an empty difference.
//!
//! # Reading
//!
//! - Formats: ASS and SSA, SRT, WebVTT, and SMI/SAMI, by the extension
//!   (`ass`, `ssa`, `srt`, `vtt`, `smi`, `sami`, case and a leading dot
//!   ignored). PGS (`sup`) and VobSub (`idx`, `sub`) are pictures:
//!   [`Unreadable`]; so is any other extension.
//! - Encoding: a BOM wins (UTF-8, UTF-16 LE or BE); else bytes that are valid
//!   UTF-8; else CP949 (`encoding_rs::EUC_KR`, which is Windows-949) when it
//!   reads them without an error; else the encoding is unknown and the file
//!   [`Unreadable`], whatever its format, as a guessed text would make two
//!   unlike files look alike. Bytes with a NUL and no BOM (UTF-16 without a
//!   BOM) are unknown too. The one found is in [`Script::encoding`].
//! - A [`Cue`] has its start and end in milliseconds, [`Cue::shown`] (what a
//!   viewer sees, line breaks kept, blanks as they are) and [`Cue::text`]
//!   (the same with each line's blanks collapsed to one space and empty
//!   lines dropped: what is compared). A cue with no text is no cue.
//!   Cues are sorted by start, end, then text.
//!   - ASS: `{...}` override blocks are removed, `\N` and `\n` are line
//!     breaks, `\h` a blank. Only `Dialogue:` lines are cues; `Comment:` is
//!     not. Text drawn as a vector shape (`\p1`) is not dialogue and is
//!     dropped, so a cue of only a drawing is no cue. Times are
//!     `h:mm:ss.cc`; [`Cue::style`] is the line's style name.
//!   - SRT and WebVTT: a time line and the text lines up to a blank line.
//!     `<i>`-like tags and `{\an8}`-like blocks are removed, `<br>` is a
//!     line break, entities are decoded. WebVTT notes, styles and cue
//!     settings are not cues.
//!   - SMI: tags removed, entities decoded (`&nbsp;` is a blank, also
//!     without its `;`), `<br>` a line break, the source's own line breaks
//!     blanks. A `<SYNC Start=ms>` with `<P Class=...>` paragraphs: a cue
//!     runs from its sync to the next sync that has the same class, and a
//!     sync whose text is empty or only `&nbsp;` ends the cue before it
//!     without being one. A last cue has no end: its end is its start.
//!     The classes (`KRCC`, `ENCC`) are kept as [`Track`]s of the script,
//!     with the `lang:` of the `<STYLE>` block. A tag with no `>` (a `<P
//!     Class=KRCC` that runs into its text) ends after its last
//!     `name=value` attribute and the rest is text; no tag source is left
//!     in a cue's text, and a tag that cannot be read is dropped, not the
//!     text. A few (at most 5) class-less paragraphs in a file whose other
//!     paragraphs all have one class belong to that class.
//! - Styles and fonts are ASS's only ([`Script::styles`], [`Script::fonts`]
//!   are `None` for the others). A style is its name and the fields its
//!   section's `Format:` line names, as written. The fonts used are the
//!   styles' `Fontname` and the `\fn` overrides of the dialogue, a set
//!   without regard to case, a vertical font's `@` prefix removed.
//!
//! # Comparing
//!
//! - Dialogue: each pair of tracks is aligned by the cues' [`Cue::text`] in
//!   time order with a longest common subsequence. In each gap between
//!   aligned cues, the removed old cues and added new cues are first paired
//!   by their time ranges overlapping (in order), then what is left in
//!   order; a pair is a [`Change::Changed`] line, the rest are
//!   [`Change::Removed`] and [`Change::Added`]. Dialogue lines are in time
//!   order. Only the text is compared: a change of override tags alone
//!   (a colour, a position) is not a dialogue change.
//! - Timing: an aligned pair of cues (same text) whose start or end moved
//!   by more than [`TIMING_TOLERANCE_MS`]. A changed line counts as
//!   dialogue only.
//! - Styles: by name without regard to case. Added, removed, and changed,
//!   with each field whose value differs (numbers by value, other values
//!   without regard to case), only among the fields both have.
//! - Fonts: the used set of the new script less the old one, and the old
//!   less the new.
//! - Tracks: when both scripts have one track, those are compared. Else
//!   tracks are matched by class name (without regard to case) and a track
//!   with no match is all removed or all added; a script with a single
//!   track, which cannot say which class it is (an ASS against an SMI of
//!   two languages), is compared with the other's track of its class, or
//!   else the one with the most cues (a Korean class, [`Track::is_korean`],
//!   when they tie), and the others are in [`Diff::not_compared`]. A line
//!   has its [`DialogueLine::class`] only when a script has several tracks.
//! - Different formats (SMI against ASS) compare dialogue and timing. Styles
//!   and fonts are compared only when both are ASS.
//!
//! # Limits
//!
//! A file comes from the internet, so a hostile one must cost bounded time,
//! memory and size of what is kept. Reading is linear in the file's length,
//! and a file past one of the limits below is [`Unreadable`], found while it
//! is read so nothing past a limit is held: [`MAX_CUES`] cues in all tracks
//! together (the alignment takes time in the product of two scripts' cues),
//! [`MAX_SMI_PARAGRAPHS`] SMI paragraphs, [`MAX_STYLES`] styles,
//! `Style:` and `Format:` lines of at most [`MAX_STYLE_BYTES`] each,
//! [`MAX_FONTS`] fonts, [`MAX_CLASSES`] SMI classes, [`MAX_FORMAT_FIELDS`]
//! fields in a `Format:` line, and names of at most [`MAX_NAME_BYTES`]. An
//! SMI `Start` of more than [`MAX_START_DIGITS`] digits is no sync. Real
//! files are far below them: the largest real SMI read had 2,406 cues
//! (2026-10-05).

mod lcs;
mod read;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use read::read;

/// An aligned cue whose start or end moved by no more than this has not
/// moved. ASS keeps hundredths of a second, so one step is exactly this.
pub const TIMING_TOLERANCE_MS: u64 = 10;

/// The most cues a script may have in all its tracks together.
pub const MAX_CUES: usize = 50_000;
/// The most `<P>` paragraphs (or class-less sync contents) an SMI may have.
pub const MAX_SMI_PARAGRAPHS: usize = 4 * MAX_CUES;
/// The most styles an ASS may have.
pub const MAX_STYLES: usize = 1_000;
/// The longest `Style:` or `Format:` line, after its key.
pub const MAX_STYLE_BYTES: usize = 1_024;
/// The most fonts an ASS may use (styles and `\fn` overrides).
pub const MAX_FONTS: usize = 1_000;
/// The most classes (languages) an SMI may have.
pub const MAX_CLASSES: usize = 100;
/// The most fields a `Format:` line may name.
pub const MAX_FORMAT_FIELDS: usize = 100;
/// The longest font or SMI class name.
pub const MAX_NAME_BYTES: usize = 256;
/// The most digits of an SMI `Start` (milliseconds).
pub const MAX_START_DIGITS: usize = 10;

/// A subtitle format [`read`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// ASS and SSA.
    Ass,
    Srt,
    Vtt,
    /// SMI and SAMI.
    Smi,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Ass => "ASS",
            Format::Srt => "SRT",
            Format::Vtt => "WebVTT",
            Format::Smi => "SMI",
        }
    }
}

/// The encoding a file's bytes were read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Encoding {
    #[serde(rename = "UTF-8")]
    Utf8,
    /// With a BOM, little or big endian.
    #[serde(rename = "UTF-16")]
    Utf16,
    /// Windows-949, the Korean legacy encoding.
    #[serde(rename = "CP949")]
    Cp949,
}

/// Why a file's content cannot be compared. The reason is Korean, for the
/// screen.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[error("{reason}")]
pub struct Unreadable {
    pub reason: String,
}

impl Unreadable {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    fn image() -> Self {
        Self::new("이미지 자막이라 내용을 비교할 수 없어요")
    }
}

/// A subtitle file read for comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub format: Format,
    pub encoding: Encoding,
    /// The cues by language: one track, with no class, except an SMI with
    /// several `Class`es. Tracks with no cue are left out.
    pub tracks: Vec<Track>,
    /// ASS only: the styles in the file's order.
    pub styles: Option<Vec<Style>>,
    /// ASS only: the fonts used, by name without regard to case.
    pub fonts: Option<Vec<Font>>,
}

impl Script {
    /// All cues of all tracks.
    pub fn cue_count(&self) -> usize {
        self.tracks.iter().map(|t| t.cues.len()).sum()
    }
}

/// The cues of one language class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// The SMI `Class` as the file first spells it; `None` for the formats
    /// that have none.
    pub class: Option<String>,
    /// The `lang:` its `<STYLE>` block gives the class, in lowercase
    /// (`ko-kr`).
    pub lang: Option<String>,
    pub cues: Vec<Cue>,
}

impl Track {
    /// Whether the class is Korean: named `KRCC`, `KOCC` or `KOKRCC`, or
    /// declared `lang: ko...`.
    pub fn is_korean(&self) -> bool {
        let class = self.class.as_deref().unwrap_or_default().to_lowercase();
        ["krcc", "kocc", "kokrcc"].contains(&class.as_str())
            || self
                .lang
                .as_deref()
                .is_some_and(|l| l == "ko" || l.starts_with("ko-"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    pub start: u64,
    pub end: u64,
    /// What is compared; see the module docs.
    pub text: String,
    /// What a viewer sees, with the blanks as the file has them.
    pub shown: String,
    /// The ASS style name.
    pub style: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Style {
    pub name: String,
    /// `(field, value)` in the `Format:` line's order, the name left out.
    pub fields: Vec<(String, String)>,
}

/// A font name: how it is shown, and how it is matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    pub name: String,
    key: String,
}

impl Font {
    fn new(name: &str) -> Self {
        let name = name.trim().trim_start_matches('@').trim().to_owned();
        Self {
            key: name.to_lowercase(),
            name,
        }
    }
}

// ---------------------------------------------------------------- Diff

/// The difference between an old and a new script. Counts and the lines to
/// show are in each item; the JSON omits what is empty or absent:
///
/// ```json
/// {
///   "old": {"format": "ass", "encoding": "UTF-8", "cues": 40},
///   "new": {"format": "ass", "encoding": "UTF-8", "cues": 42},
///   "dialogue": {"added": 2, "changed": 1, "removed": 0, "lines": [
///     {"kind": "changed",
///      "old": {"text": "안녕", "start": 1000, "end": 2000},
///      "new": {"text": "안녕하세요", "start": 1000, "end": 2000}},
///     {"kind": "added", "new": {"text": "네", "start": 5000, "end": 6000}}]},
///   "timing": {"count": 1, "lines": [
///     {"text": "응", "old": {"start": 3000, "end": 4000},
///      "new": {"start": 3500, "end": 4500}}]},
///   "styles": {"changed": [{"name": "Default", "fields": [
///     {"field": "Fontname", "old": "Arial", "new": "Noto Sans CJK KR"}]}]},
///   "fonts": {"added": ["Noto Sans CJK KR"], "removed": ["Arial"]}
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diff {
    pub old: Side,
    pub new: Side,
    pub dialogue: Dialogue,
    pub timing: Timing,
    /// `None` when the styles cannot be compared (see [`Diff::not_compared`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub styles: Option<Styles>,
    /// `None` when the fonts cannot be compared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fonts: Option<Fonts>,
    /// What was not compared and why, in Korean, for the screen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_compared: Vec<NotCompared>,
}

impl Diff {
    /// Whether any compared item differs. It does not say that nothing
    /// differs when [`Diff::not_compared`] is not empty.
    pub fn differs(&self) -> bool {
        self.dialogue.total() > 0
            || self.timing.count > 0
            || self.styles.as_ref().is_some_and(|s| {
                !(s.added.is_empty() && s.removed.is_empty() && s.changed.is_empty())
            })
            || self
                .fonts
                .as_ref()
                .is_some_and(|f| !(f.added.is_empty() && f.removed.is_empty()))
    }
}

/// What was read of one side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Side {
    pub format: Format,
    pub encoding: Encoding,
    /// The cues of all its tracks.
    pub cues: u64,
}

impl Side {
    fn of(script: &Script) -> Self {
        Self {
            format: script.format,
            encoding: script.encoding,
            cues: script.cue_count() as u64,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dialogue {
    pub added: u64,
    pub changed: u64,
    pub removed: u64,
    pub lines: Vec<DialogueLine>,
}

impl Dialogue {
    pub fn total(&self) -> u64 {
        self.added + self.changed + self.removed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    Added,
    Changed,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogueLine {
    pub kind: Change,
    /// The SMI language class, when a script has several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// Absent for an added line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<Placed>,
    /// Absent for a removed line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<Placed>,
}

/// A line's compared text and where it is, in milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placed {
    pub text: String,
    pub start: u64,
    pub end: u64,
}

impl Placed {
    fn of(cue: &Cue) -> Self {
        Self {
            text: cue.text.clone(),
            start: cue.start,
            end: cue.end,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timing {
    pub count: u64,
    pub lines: Vec<TimingLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingLine {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    pub old: Span,
    pub new: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Styles {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<StyleChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleChange {
    pub name: String,
    pub fields: Vec<FieldChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldChange {
    pub field: String,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fonts {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
}

/// A part of the comparison that could not be made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotCompared {
    pub item: Item,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Item {
    Dialogue,
    Styles,
    Fonts,
}

// ---------------------------------------------------------------- compare

/// Sets `old` against `new`; see the module docs for the rules.
pub fn compare(old: &Script, new: &Script) -> Diff {
    let mut not_compared = Vec::new();
    let mut dialogue = Dialogue::default();
    let mut timing = Timing::default();
    for (old_cues, new_cues, class) in pair_tracks(old, new, &mut not_compared) {
        compare_cues(old_cues, new_cues, class, &mut dialogue, &mut timing);
    }

    let (styles, fonts) = match (&old.styles, &new.styles, &old.fonts, &new.fonts) {
        (Some(old_styles), Some(new_styles), Some(old_fonts), Some(new_fonts)) => (
            Some(compare_styles(old_styles, new_styles)),
            Some(compare_fonts(old_fonts, new_fonts)),
        ),
        _ => {
            let reason = match (old.styles.is_some(), new.styles.is_some()) {
                (false, false) => format!(
                    "{}와 {}에는 스타일과 폰트가 없어요",
                    old.format.label(),
                    new.format.label()
                ),
                (true, false) => format!(
                    "{}에는 스타일과 폰트가 없어 {}와 비교하지 못했어요",
                    new.format.label(),
                    old.format.label()
                ),
                _ => format!(
                    "{}에는 스타일과 폰트가 없어 {}와 비교하지 못했어요",
                    old.format.label(),
                    new.format.label()
                ),
            };
            not_compared.push(NotCompared {
                item: Item::Styles,
                reason: reason.clone(),
            });
            not_compared.push(NotCompared {
                item: Item::Fonts,
                reason,
            });
            (None, None)
        }
    };
    Diff {
        old: Side::of(old),
        new: Side::of(new),
        dialogue,
        timing,
        styles,
        fonts,
        not_compared,
    }
}

type Pair<'a> = (&'a [Cue], &'a [Cue], Option<String>);

/// The tracks of the two scripts that are compared with each other.
fn pair_tracks<'a>(
    old: &'a Script,
    new: &'a Script,
    not_compared: &mut Vec<NotCompared>,
) -> Vec<Pair<'a>> {
    let named = |script: &'a Script| -> Vec<(Option<&'a str>, &'a [Cue], bool)> {
        match script.tracks.is_empty() {
            true => vec![(None, &[][..], false)],
            false => script
                .tracks
                .iter()
                .map(|t| (t.class.as_deref(), t.cues.as_slice(), t.is_korean()))
                .collect(),
        }
    };
    let (old_tracks, new_tracks) = (named(old), named(new));
    let several = old_tracks.len() > 1 || new_tracks.len() > 1;
    let label = |class: Option<&str>| class.filter(|_| several).map(str::to_owned);
    let same = |a: Option<&str>, b: Option<&str>| {
        a.unwrap_or_default()
            .eq_ignore_ascii_case(b.unwrap_or_default())
    };

    // One side has a single track: it meets the other side's track of its
    // class, or else the one with the most cues (a Korean one when they
    // tie, then the first).
    if old_tracks.len() == 1 || new_tracks.len() == 1 {
        let (single, others, single_is_old) = match old_tracks.len() == 1 {
            true => (old_tracks[0], &new_tracks, true),
            false => (new_tracks[0], &old_tracks, false),
        };
        let at = others
            .iter()
            .position(|(class, _, _)| same(*class, single.0))
            .or_else(|| {
                (0..others.len())
                    .max_by_key(|&n| (others[n].1.len(), others[n].2, std::cmp::Reverse(n)))
            })
            .unwrap_or(0);
        for (n, (class, _, _)) in others.iter().enumerate() {
            if n != at && others.len() > 1 {
                not_compared.push(NotCompared {
                    item: Item::Dialogue,
                    reason: format!(
                        "{} 언어 {}는 상대 쪽에 대응하는 언어가 없어 비교하지 못했어요",
                        match single_is_old {
                            true => new.format.label(),
                            false => old.format.label(),
                        },
                        class.unwrap_or("(이름 없음)")
                    ),
                });
            }
        }
        let (other_class, other_cues, _) = others[at];
        let class = label(other_class.or(single.0));
        return vec![match single_is_old {
            true => (single.1, other_cues, class),
            false => (other_cues, single.1, class),
        }];
    }

    let mut pairs = Vec::new();
    let mut matched = vec![false; new_tracks.len()];
    for (class, cues, _) in &old_tracks {
        let at = new_tracks
            .iter()
            .enumerate()
            .position(|(n, (other, _, _))| !matched[n] && same(*class, *other));
        match at {
            Some(n) => {
                matched[n] = true;
                pairs.push((*cues, new_tracks[n].1, label(*class)));
            }
            None => pairs.push((*cues, &[][..], label(*class))),
        }
    }
    for (n, (class, cues, _)) in new_tracks.iter().enumerate() {
        if !matched[n] {
            pairs.push((&[][..], *cues, label(*class)));
        }
    }
    pairs
}

fn compare_cues<'a>(
    old: &'a [Cue],
    new: &'a [Cue],
    class: Option<String>,
    dialogue: &mut Dialogue,
    timing: &mut Timing,
) {
    let mut ids: HashMap<&'a str, u32> = HashMap::new();
    let mut id_of = |cue: &'a Cue| {
        let next = ids.len() as u32;
        *ids.entry(cue.text.as_str()).or_insert(next)
    };
    let old_ids: Vec<u32> = old.iter().map(&mut id_of).collect();
    let new_ids: Vec<u32> = new.iter().map(&mut id_of).collect();

    let (mut old_at, mut new_at) = (0, 0);
    let end = (old.len(), new.len());
    for (i, j) in lcs::align(&old_ids, &new_ids).into_iter().chain([end]) {
        gap(&old[old_at..i], &new[new_at..j], &class, dialogue);
        if i < old.len() {
            let (was, is) = (&old[i], &new[j]);
            if was.start.abs_diff(is.start) > TIMING_TOLERANCE_MS
                || was.end.abs_diff(is.end) > TIMING_TOLERANCE_MS
            {
                timing.count += 1;
                timing.lines.push(TimingLine {
                    text: is.text.clone(),
                    class: class.clone(),
                    old: Span {
                        start: was.start,
                        end: was.end,
                    },
                    new: Span {
                        start: is.start,
                        end: is.end,
                    },
                });
            }
        }
        (old_at, new_at) = (i + 1, j + 1);
    }
}

/// Whether two time ranges share time; an empty range counts as a
/// millisecond.
fn overlaps(a: &Cue, b: &Cue) -> bool {
    let end = |c: &Cue| c.end.max(c.start.saturating_add(1));
    a.start < end(b) && b.start < end(a)
}

/// The cues between two aligned ones: the old ones that went, the new ones
/// that came. Overlapping ones are the same line changed, then the rest in
/// order.
fn gap(old: &[Cue], new: &[Cue], class: &Option<String>, dialogue: &mut Dialogue) {
    if old.is_empty() && new.is_empty() {
        return;
    }
    let mut partner: Vec<Option<usize>> = vec![None; old.len()];
    let mut taken = vec![false; new.len()];
    let (mut i, mut j) = (0, 0);
    while i < old.len() && j < new.len() {
        if overlaps(&old[i], &new[j]) {
            partner[i] = Some(j);
            taken[j] = true;
            i += 1;
            j += 1;
        } else if old[i].end <= new[j].start {
            i += 1;
        } else {
            j += 1;
        }
    }
    let loose_old = (0..old.len()).filter(|&i| partner[i].is_none());
    let loose_new = (0..new.len()).filter(|&j| !taken[j]);
    for (i, j) in loose_old.zip(loose_new).collect::<Vec<_>>() {
        partner[i] = Some(j);
        taken[j] = true;
    }

    let mut lines: Vec<(u64, DialogueLine)> = Vec::new();
    for (i, cue) in old.iter().enumerate() {
        lines.push(match partner[i] {
            Some(j) => (
                new[j].start,
                DialogueLine {
                    kind: Change::Changed,
                    class: class.clone(),
                    old: Some(Placed::of(cue)),
                    new: Some(Placed::of(&new[j])),
                },
            ),
            None => (
                cue.start,
                DialogueLine {
                    kind: Change::Removed,
                    class: class.clone(),
                    old: Some(Placed::of(cue)),
                    new: None,
                },
            ),
        });
    }
    for cue in new.iter().zip(&taken).filter(|(_, t)| !**t).map(|(c, _)| c) {
        lines.push((
            cue.start,
            DialogueLine {
                kind: Change::Added,
                class: class.clone(),
                old: None,
                new: Some(Placed::of(cue)),
            },
        ));
    }
    lines.sort_by_key(|(start, _)| *start);
    for (_, line) in lines {
        match line.kind {
            Change::Added => dialogue.added += 1,
            Change::Changed => dialogue.changed += 1,
            Change::Removed => dialogue.removed += 1,
        }
        dialogue.lines.push(line);
    }
}

fn compare_styles(old: &[Style], new: &[Style]) -> Styles {
    let by_name: HashMap<String, &Style> = old.iter().map(|s| (s.name.to_lowercase(), s)).collect();
    let mut styles = Styles::default();
    for style in new {
        let Some(was) = by_name.get(&style.name.to_lowercase()) else {
            styles.added.push(style.name.clone());
            continue;
        };
        let fields: Vec<FieldChange> = style
            .fields
            .iter()
            .filter_map(|(field, value)| {
                let (_, before) = was
                    .fields
                    .iter()
                    .find(|(f, _)| f.eq_ignore_ascii_case(field))?;
                (!same_value(before, value)).then(|| FieldChange {
                    field: field.clone(),
                    old: before.clone(),
                    new: value.clone(),
                })
            })
            .collect();
        if !fields.is_empty() {
            styles.changed.push(StyleChange {
                name: style.name.clone(),
                fields,
            });
        }
    }
    let kept: HashSet<String> = new.iter().map(|s| s.name.to_lowercase()).collect();
    styles.removed = old
        .iter()
        .filter(|s| !kept.contains(&s.name.to_lowercase()))
        .map(|s| s.name.clone())
        .collect();
    styles
}

/// Numbers by value (`20` and `20.0`), anything else without regard to case
/// (`&H00FFFFFF` and `&H00ffffff`).
fn same_value(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a.eq_ignore_ascii_case(b),
    }
}

fn compare_fonts(old: &[Font], new: &[Font]) -> Fonts {
    let names = |fonts: &[Font]| fonts.iter().map(|f| f.key.clone()).collect::<HashSet<_>>();
    let (had, has) = (names(old), names(new));
    let only = |fonts: &[Font], other: &HashSet<String>| {
        fonts
            .iter()
            .filter(|f| !other.contains(&f.key))
            .map(|f| f.name.clone())
            .collect()
    };
    Fonts {
        added: only(new, &had),
        removed: only(old, &has),
    }
}
