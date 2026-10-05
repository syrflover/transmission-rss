//! What each file of a package is and where it goes
//! (`docs/specs/subtitles.md`, 자막 묶음 분석과 안전한 배치 판단, 파일의 회차,
//! 고르지 않은 회차, 폰트): the plan of one post's receipts, made without the
//! database so its rules can be read and tested here.
//!
//! # Kinds
//!
//! | file | kind | action |
//! | --- | --- | --- |
//! | ASS, SRT, SMI | subtitle | on an episode, applied or stored (below) |
//! | SSA, WebVTT, IDX and the other subtitle formats | subtitle (`other`) | stored only |
//! | the SUB of an IDX of the same name | companion | kept in the app data folder |
//! | TTF, OTF, TTC, WOFF, WOFF2 | font | kept beside the subtitles |
//! | text, PDF, documents, pictures | attachment | kept in the app data folder |
//! | anything else (an `.exe`) | — | dropped, with the reason |
//!
//! # Episodes
//!
//! A subtitle is the file of a candidate when the candidate's episode, taken
//! through the source's mapping, is the one its name says
//! ([`super::episode::of_candidate`]); a name without a number is the candidate's
//! when the post has one candidate and no subtitle of it names a number.
//! When no file is any candidate's and the package's subtitles name one
//! episode at most, they are asked about (`회차 확인 필요`), as a single file
//! of another number is. Otherwise a file that is no candidate's is another
//! episode of the package (고르지 않은 회차), placed by its own name:
//!
//! - The package's numbering is read from the candidates' files: a name that
//!   says the candidate's episode as Anissia wrote it numbers as Anissia does,
//!   one that says the season's episode numbers as the season does. Anissia's
//!   numbers go through the source's mapping (`mapped`, from the name); the
//!   season's are the episode (`explicit`). When nothing tells and the
//!   mapping would move the number, the file is on no episode.
//! - A season mark (`S03E01`, `S3`) other than the candidates' files' (else
//!   the job's season) puts the file on no episode, as does a number outside
//!   the season, a decimal or text episode, a range or no number.
//!
//! A file on no episode is stored with the reason. A candidate none of whose
//! files is in the package is reported ([`Planned::missing`]).
//!
//! # What is applied
//!
//! A candidate's episode is applied, and so is another episode of the package
//! when the job is the subscribed creator's (자동 수신); the others are stored
//! only. Of an episode's files, the first format of the format order that is
//! there is applied and the others are stored. Files of that format with
//! different bytes are alternatives no rule tells apart: each is asked about
//! (보류된 대안), and none is taken for being first or largest.

use std::collections::BTreeMap;

use trss_subtitles::{episode::numeric_key, verify::Format};

use crate::{
    mapping::{whole, Mapped, Mapping},
    model::{AssetKind, Outcome, PlanAction, SubtitleFormat},
    place::{
        episode::{named, of_candidate, Assignment, Basis, Named, Target},
        records::Placed,
    },
};

const OTHER_SUBTITLES: [&str; 8] = ["ssa", "vtt", "sup", "sub", "idx", "ttml", "dfxp", "lrc"];
const FONT_EXTENSIONS: [&str; 5] = ["ttf", "otf", "ttc", "woff", "woff2"];
pub(crate) const ARCHIVE_EXTENSIONS: [&str; 8] =
    ["zip", "rar", "7z", "gz", "bz2", "xz", "tar", "tgz"];
/// The extensions of a package's description, licence, notes and pictures.
const ATTACHMENT_EXTENSIONS: [&str; 14] = [
    "txt", "md", "nfo", "pdf", "rtf", "doc", "docx", "hwp", "jpg", "jpeg", "png", "gif", "webp",
    "bmp",
];

/// Why a file that is none of the kept kinds is not kept.
pub const NOT_KEPT: &str = "자막·폰트·첨부·구성 파일이 아니라 보관하지 않았어요";

/// What a member of a package is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Member {
    Subtitle(SubtitleFormat),
    Font,
    Archive,
    Attachment,
    /// A file a subtitle of the package needs beside it (the SUB of an IDX).
    Companion,
    Other,
}

/// `name`'s extension in lower case.
pub fn extension(name: &str) -> Option<String> {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, ext)| ext.to_lowercase())
}

/// What a received file is, from its checked `format` and its name, before
/// the package around it is looked at (a SUB is a subtitle here).
pub fn member(name: &str, format: Format) -> Member {
    let ext = extension(name);
    let ext = ext.as_deref().unwrap_or("");
    match format {
        // SSA opens as ASS does, but is not applied by itself.
        Format::Ass if ext == "ssa" => Member::Subtitle(SubtitleFormat::Other),
        Format::Ass => Member::Subtitle(SubtitleFormat::Ass),
        Format::Srt => Member::Subtitle(SubtitleFormat::Srt),
        Format::Smi => Member::Subtitle(SubtitleFormat::Smi),
        // A document can be a ZIP (`.docx`).
        Format::Zip if ATTACHMENT_EXTENSIONS.contains(&ext) => Member::Attachment,
        Format::Zip => Member::Archive,
        Format::Other if OTHER_SUBTITLES.contains(&ext) => Member::Subtitle(SubtitleFormat::Other),
        Format::Other if FONT_EXTENSIONS.contains(&ext) => Member::Font,
        Format::Other if ARCHIVE_EXTENSIONS.contains(&ext) => Member::Archive,
        Format::Other if ATTACHMENT_EXTENSIONS.contains(&ext) => Member::Attachment,
        Format::Other => Member::Other,
    }
}

/// `name` without its folders and its last extension, in lower case.
fn stem_of(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(base)
        .to_lowercase()
}

/// The season a name marks, `S03E01` or `S3` standing on its own.
pub fn season_mark(name: &str) -> Option<u32> {
    let lower = name.to_lowercase();
    let bytes = lower.as_bytes();
    let boundary = |b: u8| !b.is_ascii_alphanumeric();
    let mut found = None;
    for (at, _) in lower.match_indices('s') {
        if at > 0 && !boundary(bytes[at - 1]) {
            continue;
        }
        let digits: String = lower[at + 1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if digits.is_empty() || digits.len() > 2 {
            continue;
        }
        let rest = &bytes[at + 1 + digits.len()..];
        let marked = match rest {
            [] => true,
            [b'e', d, ..] if d.is_ascii_digit() => true,
            [b, ..] => boundary(*b),
        };
        if marked {
            found = digits.parse().ok();
        }
    }
    found
}

/// One received file of the package.
#[derive(Debug, Clone)]
pub struct File<'a> {
    /// Its name with the folders it was received in.
    pub name: &'a str,
    pub format: Format,
}

/// One candidate the post was received for.
#[derive(Debug, Clone)]
pub struct Candidate<'a> {
    pub item_id: i64,
    /// The episode as Anissia wrote it.
    pub episode: &'a str,
}

/// What the plan is made against.
#[derive(Debug, Clone)]
pub struct Context<'a> {
    /// The source's mapping (`None`: it has none).
    pub mapping: Option<&'a Mapping>,
    /// The season's episodes, when known.
    pub total: Option<u32>,
    /// The job's season.
    pub season: u32,
    /// The formats in the order the first one there is applied.
    pub order: &'a [SubtitleFormat],
    /// The job is the subscribed creator's: another episode of the package
    /// is applied too.
    pub follow: bool,
    /// The bytes' SHA-256 of each file, by index, to tell the same file from
    /// an alternative.
    pub sha256: &'a [&'a str],
}

/// The plan of one file, by its index in the package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub file: usize,
    pub kind: AssetKind,
    pub format: Option<SubtitleFormat>,
    /// The candidate it was picked for; the post's first candidate for any
    /// other file, which is the post it came from.
    pub item_id: i64,
    /// The candidate's episode, for the candidate's own file only.
    pub anissia_episode: Option<String>,
    pub attachment_episode: Option<String>,
    pub placed: Option<Placed>,
    pub question: Option<String>,
    pub action: PlanAction,
    pub outcome: Option<Outcome>,
    pub note: Option<String>,
}

/// The plan of a package.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Planned {
    pub rows: Vec<Row>,
    /// The candidates none of whose files is in the package, with why.
    pub missing: Vec<(i64, String)>,
}

/// How the names of a package number its episodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Numbering {
    Unknown,
    Anissia,
    Season,
    Both,
    Mixed,
}

impl Numbering {
    fn with(self, other: Numbering) -> Numbering {
        match (self, other) {
            (Numbering::Unknown, n) | (n, Numbering::Unknown) => n,
            (a, b) if a == b => a,
            (Numbering::Both, n) | (n, Numbering::Both) => n,
            _ => Numbering::Mixed,
        }
    }
}

fn label(episode: &str) -> String {
    match numeric_key(episode) {
        Some(_) => format!("{episode}화"),
        None => episode.to_owned(),
    }
}

fn format_name(format: SubtitleFormat) -> &'static str {
    match format {
        SubtitleFormat::Ass => "ASS",
        SubtitleFormat::Srt => "SRT",
        SubtitleFormat::Smi => "SMI",
        SubtitleFormat::Other => "그 밖의 형식",
    }
}

/// The plan of a package (see the module docs).
pub fn plan(candidates: &[Candidate<'_>], files: &[File<'_>], ctx: &Context<'_>) -> Planned {
    let Some(first) = candidates.first() else {
        return Planned::default();
    };
    let mut members: Vec<Member> = files.iter().map(|f| member(f.name, f.format)).collect();
    // A SUB beside an IDX of the same name is that IDX's.
    for i in 0..files.len() {
        if extension(files[i].name).as_deref() == Some("sub") {
            let stem = stem_of(files[i].name);
            let paired = files
                .iter()
                .any(|f| extension(f.name).as_deref() == Some("idx") && stem_of(f.name) == stem);
            if paired {
                members[i] = Member::Companion;
            }
        }
    }
    let base = |i: usize| files[i].name.rsplit('/').next().unwrap_or(files[i].name);
    let subtitles: Vec<usize> = (0..files.len())
        .filter(|&i| matches!(members[i], Member::Subtitle(_)))
        .collect();
    let numbers: Vec<Option<String>> = (0..files.len())
        .map(|i| match named(base(i)) {
            Named::One(key) => Some(key),
            _ => None,
        })
        .collect();
    let numbered = subtitles.iter().any(|&i| named(base(i)) != Named::Nothing);
    let alone = candidates.len() == 1 && !numbered;

    // A candidate whose own episode cannot be told (its creator's mapping
    // undecided, or past the season): why, whatever the files are named.
    let unresolved: Vec<(&Candidate<'_>, String)> = candidates
        .iter()
        .filter_map(
            |c| match of_candidate(c.episode, ctx.mapping, "", ctx.total, true) {
                Target::Ask(reason) => Some((c, reason)),
                Target::Episode { .. } => None,
            },
        )
        .collect();

    // Each subtitle against each candidate.
    let mut chosen: BTreeMap<usize, (&Candidate<'_>, Target)> = BTreeMap::new();
    let mut asked: BTreeMap<usize, String> = BTreeMap::new();
    // The files of a candidate whose episode cannot be told: asked with why.
    let mut held: BTreeMap<usize, (&Candidate<'_>, String)> = BTreeMap::new();
    for &i in &subtitles {
        let decided: Vec<(&Candidate<'_>, Target)> = candidates
            .iter()
            .map(|c| {
                (
                    c,
                    of_candidate(c.episode, ctx.mapping, base(i), ctx.total, alone),
                )
            })
            .collect();
        let found: Vec<&(&Candidate<'_>, Target)> = decided
            .iter()
            .filter(|(_, t)| matches!(t, Target::Episode { .. }))
            .collect();
        let theirs = unresolved.iter().find(|(c, _)| match &numbers[i] {
            Some(key) => numeric_key(c.episode.trim()).as_ref() == Some(key),
            None => alone && named(base(i)) == Named::Nothing,
        });
        match found.as_slice() {
            [one] => {
                chosen.insert(i, (one.0, one.1.clone()));
            }
            [] if theirs.is_some() => {
                let (candidate, reason) = theirs.expect("checked");
                held.insert(i, (candidate, reason.clone()));
            }
            [] => {
                let reason = match decided.first() {
                    Some((_, Target::Ask(reason))) if candidates.len() == 1 => reason.clone(),
                    _ => "이 파일 하나를 여러 회차가 함께 받았어요".to_owned(),
                };
                asked.insert(i, reason);
            }
            _ => {
                asked.insert(i, "이 파일 하나를 여러 회차가 함께 받았어요".to_owned());
            }
        }
    }
    // Files of the candidates' episodes under different season marks: the
    // ones of the job's season (or of none) are the candidates'; the others
    // are another season's, placed as other files are.
    let marks: std::collections::BTreeSet<Option<u32>> =
        chosen.keys().map(|&i| season_mark(base(i))).collect();
    // When none is of the job's season, an episode whose files carry several
    // marks asks which is its.
    let mut split: BTreeMap<usize, String> = BTreeMap::new();
    if marks.len() > 1 {
        let ours: Vec<usize> = chosen
            .keys()
            .copied()
            .filter(|&i| season_mark(base(i)).is_none_or(|m| m == ctx.season))
            .collect();
        if ours.is_empty() {
            let mut by_episode: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
            for (&i, (_, target)) in &chosen {
                if let Target::Episode { episode, .. } = target {
                    by_episode.entry(*episode).or_default().push(i);
                }
            }
            for on in by_episode.into_values() {
                let marks: std::collections::BTreeSet<u32> =
                    on.iter().filter_map(|&i| season_mark(base(i))).collect();
                if marks.len() < 2 {
                    continue;
                }
                let seasons: Vec<String> = marks.iter().map(|m| format!("시즌 {m}")).collect();
                let question = format!(
                    "이 회차의 파일 이름에 시즌 표시가 여럿({}) 있어 적용할 것을 골라야 해요",
                    seasons.join(", ")
                );
                for i in on {
                    split.insert(i, question.clone());
                }
            }
        } else {
            chosen.retain(|i, _| ours.contains(i));
        }
    }
    let distinct: std::collections::BTreeSet<&String> = subtitles
        .iter()
        .filter_map(|&i| numbers[i].as_ref())
        .collect();
    // No file is any candidate's and the package is one episode's worth:
    // its files are asked about, as a single file of another number is.
    let ask_all = chosen.is_empty() && held.is_empty() && distinct.len() <= 1;

    // The package's numbering and season mark, from the candidates' files.
    let mut numbering = Numbering::Unknown;
    let mut mark = None;
    for (&i, (candidate, target)) in &chosen {
        if let (Some(key), Target::Episode { episode, .. }) = (&numbers[i], target) {
            let anissia = numeric_key(candidate.episode.trim()).as_ref() == Some(key);
            let season = *key == episode.to_string();
            numbering = numbering.with(match (anissia, season) {
                (true, true) => Numbering::Both,
                (true, false) => Numbering::Anissia,
                (false, true) => Numbering::Season,
                (false, false) => Numbering::Unknown,
            });
        }
        mark = mark.or(season_mark(base(i)));
    }
    let expected_mark = mark.unwrap_or(ctx.season);
    let range = || match ctx.total {
        Some(n) => format!("1–{n}화"),
        None => "1화부터".to_owned(),
    };

    // Where another episode's file goes: its episode, or why none.
    let other = |i: usize| -> Result<Target, String> {
        let key = match named(base(i)) {
            Named::One(key) => key,
            Named::Nothing => return Err("파일 이름에 회차 번호가 없어요".to_owned()),
            Named::Several => return Err("파일 이름이 회차 여럿을 가리켜요".to_owned()),
        };
        if let Some(m) = season_mark(base(i)).filter(|&m| m != expected_mark) {
            return Err(format!(
                "파일 이름의 시즌 {m}이 이 작업의 시즌 {}과 달라 보여요",
                ctx.season
            ));
        }
        let Some(number) = whole(&key) else {
            return Err(format!(
                "{}는 정수 회차가 아니라 시즌의 회차로 정하지 못했어요",
                label(&key)
            ));
        };
        let through = ctx.mapping.map(|m| (m, m.season_episode(&key)));
        let target = match (numbering, through) {
            (Numbering::Season, _) | (_, None) => Target::Episode {
                episode: number,
                assignment: Assignment::Explicit,
                basis: None,
            },
            (_, Some((_, Mapped::NotReceived))) => {
                return Err(format!(
                    "회차 대응이 {}를 받지 않는 회차로 정해 두었어요",
                    label(&key)
                ))
            }
            (_, Some((m, Mapped::Unmapped))) => {
                return Err(match m.decided_offset() {
                    None => "이 제작자의 회차 대응이 아직 미정이에요".to_owned(),
                    Some(_) => format!("{}는 회차 대응으로 옮길 수 없는 회차예요", label(&key)),
                })
            }
            (Numbering::Unknown | Numbering::Mixed, Some((_, Mapped::Episode(n))))
                if n != number =>
            {
                return Err(format!(
                    "파일 이름의 {}가 Anissia의 회차인지 시즌의 회차인지 정하지 못했어요",
                    label(&key)
                ))
            }
            (_, Some((_, Mapped::Episode(n)))) => Target::Episode {
                episode: n,
                assignment: Assignment::Mapped,
                basis: Some(Basis::Attachment),
            },
        };
        match target {
            Target::Episode { episode, .. }
                if episode < 1 || ctx.total.is_some_and(|n| episode > i64::from(n)) =>
            {
                Err(format!(
                    "{}가 시즌의 {} 밖이라 다른 시즌의 파일로 보여요",
                    label(&key),
                    range()
                ))
            }
            target => Ok(target),
        }
    };

    let mut rows: Vec<Row> = Vec::with_capacity(files.len());
    // The rows on an episode, and whether each is its candidate's.
    let mut on_episode: BTreeMap<i64, Vec<(usize, bool)>> = BTreeMap::new();
    for (i, file) in files.iter().enumerate() {
        let mut row = Row {
            file: i,
            kind: AssetKind::Subtitle,
            format: None,
            item_id: first.item_id,
            anissia_episode: None,
            attachment_episode: numbers[i].clone(),
            placed: None,
            question: None,
            action: PlanAction::Store,
            outcome: None,
            note: None,
        };
        match members[i] {
            Member::Subtitle(format) => {
                row.format = Some(format);
                let target = match (chosen.get(&i), asked.get(&i)) {
                    (Some((candidate, target)), _) => {
                        row.item_id = candidate.item_id;
                        row.anissia_episode = Some(candidate.episode.to_owned());
                        Ok((target.clone(), true))
                    }
                    (None, _) if held.contains_key(&i) => {
                        let (candidate, reason) = &held[&i];
                        row.item_id = candidate.item_id;
                        row.anissia_episode = Some(candidate.episode.to_owned());
                        row.question = Some(reason.clone());
                        Err(None)
                    }
                    (None, Some(reason)) if ask_all => {
                        row.question = Some(reason.clone());
                        row.anissia_episode = Some(first.episode.to_owned());
                        Err(None)
                    }
                    _ => other(i).map(|t| (t, false)).map_err(Some),
                };
                match target {
                    Ok((
                        Target::Episode {
                            episode,
                            assignment,
                            basis,
                        },
                        own,
                    )) => {
                        row.placed = Some(Placed {
                            episode,
                            assignment,
                            basis,
                        });
                        on_episode.entry(episode).or_default().push((i, own));
                    }
                    Ok((Target::Ask(reason), _)) | Err(Some(reason)) => {
                        row.note = Some(format!("회차에 붙이지 않고 보관만 해요: {reason}"));
                    }
                    Err(None) => {}
                }
                if extension(file.name).as_deref() == Some("idx") {
                    let stem = stem_of(file.name);
                    let has_sub = files
                        .iter()
                        .enumerate()
                        .any(|(j, f)| members[j] == Member::Companion && stem_of(f.name) == stem);
                    if !has_sub {
                        row.note = Some("짝인 SUB 파일이 묶음에 없어요".to_owned());
                    }
                }
            }
            Member::Font => row.kind = AssetKind::Font,
            Member::Attachment => row.kind = AssetKind::Attachment,
            Member::Companion => row.kind = AssetKind::Companion,
            Member::Other | Member::Archive => {
                row.kind = AssetKind::Other;
                row.action = PlanAction::Drop;
                row.outcome = Some(Outcome::Dropped);
                row.note = Some(NOT_KEPT.to_owned());
            }
        }
        if row.kind != AssetKind::Subtitle {
            row.attachment_episode = None;
        }
        rows.push(row);
    }

    // Which file of each episode is applied.
    for (_, on) in on_episode {
        let applies = |own: bool| own || ctx.follow;
        let wanted: Vec<usize> = on
            .iter()
            .filter(|(_, own)| applies(*own))
            .map(|(i, _)| *i)
            .collect();
        for &(i, own) in &on {
            if !applies(own) {
                rows[i].note = Some("고르지 않은 회차라 보관만 해요".to_owned());
            }
        }
        let formats: Vec<SubtitleFormat> = rows
            .iter()
            .map(|r| r.format.unwrap_or(SubtitleFormat::Other))
            .collect();
        let format_of = |i: usize| formats[i];
        let Some(picked) = ctx
            .order
            .iter()
            .copied()
            .find(|f| wanted.iter().any(|&i| format_of(i) == *f))
        else {
            for &i in &wanted {
                rows[i].note = Some("자동으로 적용하지 않는 형식이라 보관만 해요".to_owned());
            }
            continue;
        };
        let of_picked: Vec<usize> = wanted
            .iter()
            .copied()
            .filter(|&i| format_of(i) == picked)
            .collect();
        for &i in &wanted {
            let format = format_of(i);
            if format == picked {
                continue;
            }
            rows[i].note = Some(match format {
                SubtitleFormat::Other => "자동으로 적용하지 않는 형식이라 보관만 해요".to_owned(),
                _ => format!(
                    "형식 순서에 따라 {}를 적용하고 이 형식은 보관만 해요",
                    format_name(picked)
                ),
            });
        }
        let mut bytes: Vec<&str> = of_picked.iter().map(|&i| ctx.sha256[i]).collect();
        bytes.sort_unstable();
        bytes.dedup();
        match bytes.len() {
            1 => {
                let (apply, rest) = of_picked.split_first().expect("one at least");
                rows[*apply].action = PlanAction::Apply;
                for &i in rest {
                    rows[i].note =
                        Some("같은 내용의 파일을 적용하므로 이 파일은 보관만 해요".to_owned());
                }
            }
            n => {
                for &i in &of_picked {
                    rows[i].action = PlanAction::Apply;
                    rows[i].question = Some(format!(
                        "이 회차에 {} 자막이 {n}개 있어 적용할 것을 골라야 해요",
                        format_name(picked)
                    ));
                }
            }
        }
    }
    for (i, question) in split {
        if rows[i].format.and_then(SubtitleFormat::extension).is_some() {
            rows[i].action = PlanAction::Apply;
            rows[i].question = Some(question);
            rows[i].note = None;
        }
    }
    // A question of the episode applies once answered.
    for row in &mut rows {
        if row.question.is_some() && row.placed.is_none() {
            row.action = match row.format.and_then(SubtitleFormat::extension) {
                Some(_) => PlanAction::Apply,
                None => PlanAction::Store,
            };
            if row.action == PlanAction::Store {
                row.question = None;
                row.note = Some("자동으로 적용하지 않는 형식이라 보관만 해요".to_owned());
            }
        }
    }

    let missing = match ask_all {
        true => Vec::new(),
        false => candidates
            .iter()
            .filter(|c| {
                !chosen.values().any(|(of, _)| of.item_id == c.item_id)
                    && !held.values().any(|(of, _)| of.item_id == c.item_id)
            })
            .map(|c| {
                (
                    c.item_id,
                    format!("받은 묶음에 후보의 {} 파일이 없어요", label(c.episode)),
                )
            })
            .collect(),
    };
    Planned { rows, missing }
}

#[cfg(test)]
mod tests;
