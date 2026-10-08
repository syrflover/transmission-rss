//! The preview of a past episode search (`docs/specs/collection.md`, 지난 회차
//! 검색): what each search result is, given the rule, the range the person
//! confirmed and what the work already has.
//!
//! Every result goes through the rule's own judgement (`picks`), as the
//! worker's cycle does; a result the rule does not pick is only counted. An
//! episode outside the range is folded into a count. What stays is one of
//! the [`State`]s, and only an individual episode the work does not have yet is
//! selected for the person:
//!
//! | state            | the result is                                              | selected |
//! | ---------------- | ---------------------------------------------------------- | -------- |
//! | `Missing`        | an episode the folder and Transmission do not have          | the highest revision of an episode only |
//! | `Replace`        | a higher revision of a video the folder has                 | no       |
//! | `VersionUnknown` | a revision whose video in the folder cannot be told apart   | no       |
//! | `Have`           | an episode the work has (or an item whose torrent is still there) | no |
//! | `Superseded`     | a lower revision of an episode with a higher one in the list | no       |
//! | `Alternate`      | another release of an episode a different result is selected for | no  |
//! | `Batch`          | several episodes in one torrent                             | no       |
//! | `Unnumbered`     | no episode number                                           | no       |
//!
//! # What the work has
//!
//! The episode is judged by its **number in the work folder** (the release
//! number moved by the rule's episode conversion, as `trname` does when it
//! names the file), never by a torrent hash: an episode received from another
//! release is a different torrent with the same number. The work has an
//! episode when the folder holds a video of it (or a download in progress,
//! `.part`) or when history says a torrent of the rule got it into
//! Transmission and that torrent is still there. The web cannot ask
//! Transmission, so it reads history, and the worker's last list of the
//! torrents in Transmission ([`crate::store::status::TorrentListing`]) tells
//! which of them were removed since. An episode whose video was deleted and
//! whose torrent was removed is missing again, and so is a result that history
//! says was received for it ([`World::held`]); a torrent that is still in
//! Transmission keeps both, even before its video is placed in the folder.
//! Without a list, every torrent history knows is taken to be there.
//!
//! # Revisions
//!
//! A result that is a revision (`14v2`) of an episode the work has follows the
//! replacement of `docs/specs/collection.md`, 영상 수정본의 대체, the way
//! `crate::revisions::plan` decides it for the worker:
//!
//! - the video's release is known from history (a torrent of the rule got the
//!   episode in): a lower revision of the same release is replaceable, the same
//!   or a higher one is had, another release makes the result a duplicate;
//! - otherwise the video's CRC32 is read and compared with the name's: equal
//!   means the folder has this revision, equal to a lower revision's name means
//!   replaceable, anything else means the version cannot be told. A name
//!   without a CRC32 cannot be told either.
//!
//! Nothing here receives anything: replacing is the worker's, when the person
//! selects such a result.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    path::{Path, PathBuf},
};

use crate::{
    episode_offset::folder_episode,
    release_name::{Episode, Kind, ReleaseName},
};

/// The release numbers the person confirmed, both included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub from: u32,
    pub to: u32,
}

impl Range {
    pub fn contains(self, number: u32) -> bool {
        (self.from..=self.to).contains(&number)
    }

    fn overlaps(self, from: u32, to: u32) -> bool {
        from <= self.to && to >= self.from
    }
}

/// A release already in the work, as its name tells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    /// See [`ReleaseName::stem`].
    pub stem: String,
    /// What releases are compared by: [`ReleaseName::release_key`].
    pub key: String,
    pub version: u32,
    pub crc: Option<u32>,
}

impl Known {
    pub fn of(title: &str) -> Known {
        let release = ReleaseName::read(title);
        Known {
            key: release.release_key().into_owned(),
            stem: release.stem,
            version: release.version,
            crc: release.crc,
        }
    }
}

/// What the work has of one folder episode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Present {
    /// A video (or download in progress) of the episode in the work folder.
    pub file: Option<PathBuf>,
    /// The releases history says torrents of the rule got in for the episode,
    /// removed since or not. They tell which release a video in the folder is.
    pub records: Vec<Known>,
    /// Whether one of those torrents is (taken to be) in Transmission still.
    pub in_transmission: bool,
}

impl Present {
    /// Whether the work has the episode now: a video of it in the folder, or a
    /// torrent of it in Transmission. History alone is not that: the video may
    /// have been deleted and the torrent removed.
    pub fn there(&self) -> bool {
        self.file.is_some() || self.in_transmission
    }
}

/// What the work has, as far as the web can tell.
#[derive(Debug, Clone, Default)]
pub struct World {
    /// The rule's episode conversion.
    pub offset: i64,
    /// The season of the work folder, for naming an episode (`S02E01`).
    pub season: Option<u32>,
    pub present: BTreeMap<Episode, Present>,
    /// Identity keys of the channel's items that history says Transmission
    /// took (`received`, `duplicate`) and that are not gone from the work
    /// ([`super::world::departed`]); such a result is not chosen again.
    pub held: HashSet<String>,
    /// Identity keys of the channel's items that history says Transmission
    /// took and that are gone from the work ([`super::world::departed`]):
    /// offered again, though history has them as received.
    pub departed: HashSet<String>,
    /// Other releases of the channel's history, to find which revision a video
    /// of unknown version is by its CRC32.
    pub releases: Vec<Known>,
}

impl World {
    /// The folder episode of a release number.
    pub fn folder_of(&self, release: Episode) -> Episode {
        folder_episode(release, self.offset)
    }

    /// What the work has of `folder`, if it has the episode.
    fn has(&self, folder: &Episode) -> Option<&Present> {
        self.present.get(folder).filter(|present| present.there())
    }

    /// `S02E05`, or `5화` when the work folder has no season.
    pub fn label(&self, folder: Episode) -> String {
        match self.season {
            Some(season) => format!(
                "S{season:02}E{:02}{}",
                folder.number,
                if folder.half { ".5" } else { "" }
            ),
            None => format!("{}화", folder.text()),
        }
    }

    /// Whether the work has the episode of release number `number`.
    pub fn has_release(&self, number: u32) -> bool {
        self.has(&self.folder_of(Episode::whole(number))).is_some()
    }
}

/// One search result to judge.
#[derive(Debug, Clone)]
pub struct Result {
    /// The identity key of the item (`crate::store::history::identity_key`).
    pub key: String,
    /// The title as the feed gave it, which is judged.
    pub title: String,
    /// The title as history stores and the screen shows it.
    pub shown: String,
}

/// Why a result's revision cannot be told from the folder's video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// The name carries no CRC32.
    NoCrc,
    /// The video's CRC32 is no revision of this release's.
    Mismatch,
    /// The video could not be read.
    Unreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Missing,
    Replace,
    VersionUnknown,
    Have,
    Superseded,
    Alternate,
    Batch,
    Unnumbered,
}

impl State {
    pub fn code(self) -> &'static str {
        match self {
            State::Missing => "missing",
            State::Replace => "replace",
            State::VersionUnknown => "version_unknown",
            State::Have => "have",
            State::Superseded => "superseded",
            State::Alternate => "alternate",
            State::Batch => "batch",
            State::Unnumbered => "unnumbered",
        }
    }
}

/// A result as the preview lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub key: String,
    pub title: String,
    pub state: State,
    /// A sentence that says why, for the states that need one.
    pub note: Option<String>,
    /// The release number; `None` for a batch or an unnumbered result.
    pub release: Option<u32>,
    pub half: bool,
    pub version: u32,
    /// The folder's name for the episode (`S02E01`).
    pub folder: Option<String>,
    /// Whether the person starts with it selected.
    pub selected: bool,
    /// Whether the person may select it at all.
    pub selectable: bool,
}

/// What the results come to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preview {
    pub items: Vec<Item>,
    /// Individual episodes and batches outside the range.
    pub out_of_range: usize,
    /// Results the rule does not pick (another work's, a title the channel excludes).
    pub not_picked: usize,
    /// The release numbers of the range the work does not have.
    pub missing: Vec<u32>,
    /// The missing ones no result is an episode of.
    pub not_found: Vec<u32>,
}

/// The most files whose CRC32 one preview reads; a video past that is judged as
/// unreadable. A video is read to its end, so this bounds the work.
pub const MAX_CRC_READS: usize = 20;

/// How a video's CRC32 is read.
pub type CrcOf<'a> = &'a mut dyn FnMut(&Path) -> io::Result<u32>;

struct Entry<'a> {
    index: usize,
    result: &'a Result,
    read: ReleaseName,
}

/// Judges `results` for a rule that picks the titles `picks` says it does.
pub fn judge(
    results: &[Result],
    range: Range,
    world: &World,
    picks: &dyn Fn(&str) -> bool,
    crc_of: CrcOf<'_>,
) -> Preview {
    let mut preview = Preview::default();
    let mut entries = Vec::new();
    let mut batches = Vec::new();
    for (index, result) in results.iter().enumerate() {
        if !picks(&result.title) {
            preview.not_picked += 1;
            continue;
        }
        let read = ReleaseName::read(&result.title);
        let entry = Entry {
            index,
            result,
            read,
        };
        match entry.read.kind {
            Kind::Episode(episode) => {
                if range.contains(episode.number) {
                    entries.push(entry);
                } else {
                    preview.out_of_range += 1;
                }
            }
            Kind::Batch {
                range: Some((from, to)),
            } if !range.overlaps(from, to) => preview.out_of_range += 1,
            Kind::Batch { .. } | Kind::Unnumbered => batches.push(entry),
        }
    }

    let mut reads_left = MAX_CRC_READS;
    let mut judged: Vec<(Entry<'_>, Episode, State, Option<String>)> = Vec::new();
    for entry in entries {
        let Kind::Episode(episode) = entry.read.kind else {
            continue;
        };
        let version = entry.read.version;
        let folder = world.folder_of(episode);
        let (state, note) = state_of(
            &entry,
            version,
            folder,
            world,
            results,
            crc_of,
            &mut reads_left,
        );
        judged.push((entry, folder, state, note));
    }

    // One episode, one selection: among the results still to be received,
    // the highest revision, then the newest in the list.
    let mut winner: HashMap<Episode, usize> = HashMap::new();
    for (at, (_, folder, state, _)) in judged.iter().enumerate() {
        if !matches!(state, State::Missing | State::Replace) {
            continue;
        }
        let rank = |at: usize| {
            (
                judged[at].0.read.version,
                std::cmp::Reverse(judged[at].0.index),
            )
        };
        let better = match winner.get(folder) {
            Some(&current) => rank(at) > rank(current),
            None => true,
        };
        if better {
            winner.insert(*folder, at);
        }
    }

    let mut items: Vec<(Option<Episode>, Item)> = Vec::new();
    for (at, (entry, folder, state, note)) in judged.iter().enumerate() {
        let Kind::Episode(episode) = entry.read.kind else {
            continue;
        };
        let version = entry.read.version;
        let (mut state, mut note) = (*state, note.clone());
        let wins = winner.get(folder) == Some(&at);
        if matches!(state, State::Missing | State::Replace) && !wins {
            let top = &judged[winner[folder]].0;
            if top.read.release_key() == entry.read.release_key() {
                state = State::Superseded;
                note = Some(format!(
                    "같은 회차의 더 높은 수정본(v{})이 있어요.",
                    top.read.version
                ));
            } else {
                state = State::Alternate;
                note = Some("같은 회차의 다른 릴리스를 골라 두었어요.".to_owned());
            }
        }
        let selectable = !(state == State::Have && world.held.contains(&entry.result.key));
        items.push((
            Some(*folder),
            Item {
                key: entry.result.key.clone(),
                title: entry.result.shown.clone(),
                state,
                note,
                release: Some(episode.number),
                half: episode.half,
                version,
                folder: Some(world.label(*folder)),
                selected: state == State::Missing,
                selectable,
            },
        ));
    }
    // Episodes in the folder's order, the highest revision first.
    items.sort_by(|(a, x), (b, y)| a.cmp(b).then(y.version.cmp(&x.version)));

    let mut listed: Vec<Item> = items.into_iter().map(|(_, item)| item).collect();
    for entry in batches {
        let (state, note) = match entry.read.kind {
            Kind::Batch {
                range: Some((from, to)),
            } => (
                State::Batch,
                format!("{from}–{to}화를 묶은 배치라 기본으로 고르지 않아요."),
            ),
            Kind::Batch { range: None } => (
                State::Batch,
                "여러 회차를 묶은 배치라 기본으로 고르지 않아요.".to_owned(),
            ),
            _ => (
                State::Unnumbered,
                "회차를 알 수 없는 항목이라 기본으로 고르지 않아요.".to_owned(),
            ),
        };
        listed.push(Item {
            key: entry.result.key.clone(),
            title: entry.result.shown.clone(),
            state,
            note: Some(note),
            release: None,
            half: false,
            version: 1,
            folder: None,
            selected: false,
            selectable: !world.held.contains(&entry.result.key),
        });
    }
    preview.items = listed;

    let found: HashSet<u32> = preview
        .items
        .iter()
        .filter_map(|i| i.release.filter(|_| !i.half))
        .collect();
    preview.missing = (range.from..=range.to)
        .filter(|n| !world.has_release(*n))
        .collect();
    preview.not_found = preview
        .missing
        .iter()
        .copied()
        .filter(|n| !found.contains(n))
        .collect();
    preview
}

/// The release a video of the episode could be by its CRC32: a lower revision
/// of `release` in the results or in the channel's history.
fn lower_revision_with(
    crc: u32,
    key: &str,
    version: u32,
    results: &[Result],
    world: &World,
) -> bool {
    let from_results = results.iter().map(|r| Known::of(&r.title));
    from_results
        .chain(world.releases.iter().cloned())
        .any(|known| known.key == key && known.version < version && known.crc == Some(crc))
}

fn state_of(
    entry: &Entry<'_>,
    version: u32,
    folder: Episode,
    world: &World,
    results: &[Result],
    crc_of: CrcOf<'_>,
    reads_left: &mut usize,
) -> (State, Option<String>) {
    let label = world.label(folder);
    if world.held.contains(&entry.result.key) {
        return (State::Have, Some("이미 받은 항목이에요.".to_owned()));
    }
    let Some(present) = world.has(&folder) else {
        return (State::Missing, None);
    };
    let release = &entry.read;
    let same: Vec<&Known> = present
        .records
        .iter()
        .filter(|k| k.key == release.release_key())
        .collect();
    let in_folder = present.file.is_some();
    let have = |note: String| (State::Have, Some(note));

    if version < 2 {
        return have(
            match (same.is_empty(), present.records.is_empty(), in_folder) {
                (false, _, true) => {
                    format!("같은 릴리스로 받은 {label} 영상이 폴더에 이미 있어요.")
                }
                (false, _, false) => "같은 릴리스를 이미 Transmission에 추가했어요.".to_owned(),
                (true, false, true) => {
                    format!("다른 릴리스로 받은 {label} 영상이 폴더에 이미 있어요.")
                }
                (true, false, false) => "다른 릴리스를 이미 Transmission에 추가했어요.".to_owned(),
                (true, true, _) => format!("작품 폴더에 {label} 영상이 이미 있어요."),
            },
        );
    }

    // A revision of an episode the work has.
    if let Some(newest) = same.iter().map(|k| k.version).max() {
        if newest >= version {
            return have(format!("같거나 더 높은 수정본(v{newest})이 이미 있어요."));
        }
        if release.crc.is_none() {
            return unknown(Why::NoCrc);
        }
        return replace(newest, &label);
    }
    if !present.records.is_empty() {
        return have(if in_folder {
            format!("다른 릴리스로 받은 {label} 영상이 폴더에 이미 있어요.")
        } else {
            "다른 릴리스를 이미 Transmission에 추가했어요.".to_owned()
        });
    }
    // A video of unknown version.
    let Some(crc) = release.crc else {
        return unknown(Why::NoCrc);
    };
    let Some(file) = present.file.as_deref() else {
        return unknown(Why::Unreadable);
    };
    if *reads_left == 0 {
        return unknown(Why::Unreadable);
    }
    *reads_left -= 1;
    let Ok(file_crc) = crc_of(file) else {
        return unknown(Why::Unreadable);
    };
    if file_crc == crc {
        return have("폴더의 영상이 이미 이 수정본이에요.".to_owned());
    }
    if lower_revision_with(
        file_crc,
        &release.release_key(),
        release.version,
        results,
        world,
    ) {
        return replace(1, &label);
    }
    unknown(Why::Mismatch)
}

fn replace(old_version: u32, label: &str) -> (State, Option<String>) {
    (
        State::Replace,
        Some(format!(
            "{label}의 v{old_version} 영상을 이 수정본으로 교체해요. 받으면 이전 영상은 지워져서 기본으로 고르지 않아요."
        )),
    )
}

fn unknown(why: Why) -> (State, Option<String>) {
    let note = match why {
        Why::NoCrc => "이름에 CRC32 값이 없어서 받은 영상을 확인할 수 없어요. 폴더의 영상을 교체하는 일이라 직접 골라야 해요.",
        Why::Mismatch => "폴더에 있는 영상의 CRC32가 이 릴리스의 어느 수정본과도 달라서 버전을 알 수 없어요. 고르면 이전 영상을 교체해요.",
        Why::Unreadable => "폴더에 있는 영상을 읽지 못해 버전을 확인하지 못했어요. 고르면 이전 영상을 교체해요.",
    };
    (State::VersionUnknown, Some(note.to_owned()))
}

#[cfg(test)]
mod tests;
