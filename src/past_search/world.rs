//! What the work has, for a search's preview ([`super::judge::World`]): the
//! videos in the rule's work folder, and what history says the rule's torrents
//! got into Transmission.
//!
//! The folder is read one level deep, by the names `trname` gave the videos
//! (`Show S02E05.mkv`; `.part` while a download is in progress). Its files are
//! only listed here; their contents are read by [`super::judge`], and only for
//! a revision of a video of unknown version.

use std::{
    collections::{BTreeMap, HashSet},
    io,
    path::{Path, PathBuf},
};

use super::{
    judge::{Known, Present, World},
    release::{read, Episode, Kind},
};
use crate::{
    revision::season_episode,
    store::history::{HistoryItem, HistoryResult},
};

/// The most files of one folder that are looked at.
const MAX_FILES: usize = 5000;

/// The episodes the videos of `folder` are of, with their paths. A folder that
/// does not exist (a rule that has received nothing yet) holds none.
pub fn read_folder(folder: &Path) -> io::Result<Vec<(Episode, PathBuf)>> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut found = Vec::new();
    for entry in entries.take(MAX_FILES) {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let name = name.strip_suffix(".part").unwrap_or(&name);
        let Some((_, text)) = season_episode(name) else {
            continue;
        };
        let (whole, half) = match text.split_once('.') {
            Some((whole, _)) => (whole, true),
            None => (text.as_str(), false),
        };
        let Ok(number) = whole.parse::<u32>() else {
            continue;
        };
        found.push((Episode { number, half }, entry.path()));
    }
    found.sort();
    Ok(found)
}

/// The releases of `titles` that a video can be told from by its CRC32: the ones
/// that name one, once each. Nothing else of them is looked at (see
/// [`super::judge::World::releases`]), so a long history leaves a short list.
fn releases(titles: &[String]) -> Vec<Known> {
    let mut seen = HashSet::new();
    titles
        .iter()
        .map(|title| Known::of(title))
        .filter(|known| known.crc.is_some())
        .filter(|known| seen.insert((known.stem.clone(), known.version, known.crc)))
        .collect()
}

/// The world of a rule whose episode conversion is `offset`.
///
/// - `files`: the folder's videos ([`read_folder`]);
/// - `settled`: the channel's items that history says Transmission holds
///   (`received`, `duplicate`); the ones `rule_id` picked also tell which
///   release the folder's episode came from;
/// - `titles`: the titles history holds for the channel; only those that name a
///   CRC32 are kept.
pub fn build(
    offset: i64,
    season: Option<u32>,
    files: Vec<(Episode, PathBuf)>,
    rule_id: &str,
    settled: &[HistoryItem],
    titles: &[String],
) -> World {
    let mut world = World {
        offset,
        season,
        ..World::default()
    };
    for (episode, path) in files {
        let present: &mut Present = world.present.entry(episode).or_default();
        // Two videos of one episode: the first by name is the one looked at.
        present.file.get_or_insert(path);
    }
    let mut held = HashSet::new();
    let mut records: BTreeMap<Episode, Vec<Known>> = BTreeMap::new();
    for item in settled {
        if !matches!(
            item.result,
            HistoryResult::Received | HistoryResult::Duplicate
        ) {
            continue;
        }
        held.insert(item.identity_key.clone());
        if item.rule_id.as_deref() != Some(rule_id) {
            continue;
        }
        if let Kind::Episode { episode, .. } = read(&item.title).kind {
            let folder = super::judge::folder_episode(episode, offset);
            records
                .entry(folder)
                .or_default()
                .push(Known::of(&item.title));
        }
    }
    for (folder, known) in records {
        world.present.entry(folder).or_default().records = known;
    }
    world.held = held;
    world.releases = releases(titles);
    world
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn item(key: &str, title: &str, rule: Option<&str>, result: HistoryResult) -> HistoryItem {
        HistoryItem {
            first_read: false,
            id: 1,
            channel_id: "c".into(),
            channel_label: "c".into(),
            identity_key: key.into(),
            title: title.into(),
            link: String::new(),
            first_seen_at: 0,
            last_seen_at: 0,
            result,
            result_at: 0,
            rule_id: rule.map(str::to_owned),
            reason: None,
            torrent_hash: None,
        }
    }

    #[test]
    fn the_folder_is_read_by_the_names_trname_gives() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "Show S02E01.mkv",
            "Show S02E02.mkv.part",
            "Show S02E03.5.mkv",
            "notes.txt",
            "Show - 04.mkv",
        ] {
            fs::write(dir.path().join(name), b"x").unwrap();
        }
        fs::create_dir(dir.path().join("Show S02E09.mkv")).unwrap();
        let found: Vec<_> = read_folder(dir.path())
            .unwrap()
            .into_iter()
            .map(|(e, _)| e)
            .collect();
        assert_eq!(
            found,
            vec![
                Episode::whole(1),
                Episode::whole(2),
                Episode {
                    number: 3,
                    half: true
                }
            ]
        );
        assert!(read_folder(&dir.path().join("none")).unwrap().is_empty());
    }

    #[test]
    fn history_tells_which_release_the_rules_episode_came_from() {
        let settled = vec![
            item(
                "k1",
                "[SubsPlease] Show - 14 (1080p) [AAAA0014].mkv",
                Some("r"),
                HistoryResult::Received,
            ),
            // Another rule's item holds its torrent, but names no episode of this rule.
            item(
                "k2",
                "[SubsPlease] Show - 15 (1080p) [AAAA0015].mkv",
                Some("other"),
                HistoryResult::Received,
            ),
            item(
                "k3",
                "[SubsPlease] Show - 16 (1080p) [AAAA0016].mkv",
                Some("r"),
                HistoryResult::AddFailed,
            ),
        ];
        let world = build(-12, Some(2), vec![], "r", &settled, &[]);
        assert_eq!(world.held.len(), 2);
        assert!(world.held.contains("k1") && world.held.contains("k2"));
        let present = &world.present[&Episode::whole(2)];
        assert_eq!(present.records.len(), 1);
        assert_eq!(present.records[0].stem, "[SubsPlease] Show - 14 (1080p)");
        assert!(present.file.is_none());
        assert_eq!(world.present.len(), 1);
    }

    #[test]
    fn of_the_channels_titles_only_those_a_crc_can_be_matched_to_are_kept() {
        let titles: Vec<String> = [
            "[SubsPlease] Show - 05 (1080p) [AAAA0005].mkv",
            "[SubsPlease] Show - 05 (1080p) [AAAA0005].mkv",
            "[SubsPlease] Show - 05v2 (1080p) [BBBB0005].mkv",
            "[SubsPlease] Show - 06 (1080p).mkv",
            "[SubsPlease] Show - 07 (1080p).mkv",
        ]
        .map(str::to_owned)
        .to_vec();
        let world = build(0, None, vec![], "r", &[], &titles);
        let versions: Vec<(u32, Option<u32>)> =
            world.releases.iter().map(|k| (k.version, k.crc)).collect();
        assert_eq!(versions, vec![(1, Some(0xAAAA0005)), (2, Some(0xBBBB0005))]);
    }
}
