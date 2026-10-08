//! What the work has, for a search's preview ([`super::judge::World`]): the
//! videos in the rule's work folder, and what history says the rule's torrents
//! got into Transmission and the worker's last list of Transmission's torrents
//! says are still there.
//!
//! History only says a torrent was added. An episode whose video was deleted
//! and whose torrent was removed from Transmission is not the work's any more,
//! so the history of such an item ([`departed`]) neither makes its episode
//! present nor blocks its result from being chosen. The worker decides the same
//! way when it receives the result (`receive_past`), from Transmission itself.
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

use crate::{
    past_search::{
        judge::{folder_episode, Known, Present, World},
        release::{read, Episode, Kind},
    },
    store::{history::HistoryItem, status::TorrentListing},
};
use trss_core::{files::without_part, trname_names::season_episode};

/// The most files of one folder that are looked at.
const MAX_FILES: usize = 5000;

/// What reading a work folder found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folder {
    /// The folder episodes the videos are of, with their paths.
    pub files: Vec<(Episode, PathBuf)>,
    /// Whether the folder was read to its end. A folder that is not there (a
    /// rule that has received nothing yet, or a media volume that is not
    /// mounted) or that holds more than [`MAX_FILES`] entries was not, and its
    /// videos that are not in `files` cannot be said to be gone.
    pub whole: bool,
}

impl Folder {
    /// A folder read to its end.
    pub fn complete(files: Vec<(Episode, PathBuf)>) -> Folder {
        Folder { files, whole: true }
    }

    /// The episodes of the videos, when the folder was read to its end: what a
    /// video's absence can be told from ([`departed`]).
    pub fn episodes(&self) -> Option<HashSet<Episode>> {
        self.whole
            .then(|| self.files.iter().map(|(episode, _)| *episode).collect())
    }
}

/// The episodes the videos of `folder` are of, with their paths, and whether
/// the folder was read whole ([`Folder`]).
pub fn read_folder(folder: &Path) -> io::Result<Folder> {
    let mut entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Folder::default()),
        Err(err) => return Err(err),
    };
    let mut found = Vec::new();
    for entry in entries.by_ref().take(MAX_FILES) {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let name = without_part(&name).unwrap_or(&name);
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
    // An entry left over, or one that cannot be read, is a folder not read whole.
    let whole = entries.next().is_none();
    Ok(Folder {
        files: found,
        whole,
    })
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

/// Whether the torrent history says Transmission took for `item` is no longer
/// in Transmission, as `listing` lists it.
///
/// Only a list taken after the item got its result can say so (a torrent added
/// later is not on an older list), and only for an item whose torrent hash is
/// known. Without either, the torrent is taken to be there.
pub fn torrent_gone(item: &HistoryItem, listing: Option<&TorrentListing>) -> bool {
    let (Some(listing), Some(hash)) = (listing, item.torrent_hash.as_deref()) else {
        return false;
    };
    item.result_at < listing.taken_at && !listing.holds(hash)
}

/// The folder episode of a single-episode `item`; a batch or an unnumbered
/// item is of none.
pub fn folder_episode_of(item: &HistoryItem, offset: i64) -> Option<Episode> {
    folder_episode_of_title(&item.title, offset)
}

/// [`folder_episode_of`] for a release title.
pub fn folder_episode_of_title(title: &str, offset: i64) -> Option<Episode> {
    match read(title).kind {
        Kind::Episode { episode, .. } => Some(folder_episode(episode, offset)),
        Kind::Batch { .. } | Kind::Unnumbered => None,
    }
}

/// Whether an episode's result history says Transmission took is gone from the
/// work: its torrent is no longer in Transmission ([`torrent_gone`]) and the
/// work folder holds no video (or download in progress) of the episode.
/// Such a result is received again when it is chosen; the one whose video is
/// still in the folder is not, as the folder has the episode.
///
/// Only a single episode can be gone: a batch or an item with no episode
/// number has no video to look for, and its torrent is routinely removed once
/// it is done, so it stays held. So does everything when `folder` is `None`,
/// the folder not having been read to its end ([`Folder::episodes`]): a video
/// that is not seen may be on a volume that is not mounted or past the files
/// that are looked at.
///
/// `folder` is the folder episodes the work folder holds and `offset` the
/// rule's episode conversion.
pub fn departed(
    item: &HistoryItem,
    offset: i64,
    folder: Option<&HashSet<Episode>>,
    listing: Option<&TorrentListing>,
) -> bool {
    let Some(folder) = folder else {
        return false;
    };
    if !item.result.is_settled() || !torrent_gone(item, listing) {
        return false;
    }
    folder_episode_of(item, offset).is_some_and(|episode| !folder.contains(&episode))
}

/// The world of a rule whose episode conversion is `offset`.
///
/// - `folder`: the folder's videos ([`read_folder`]);
/// - `settled`: the channel's items that history says Transmission took
///   (`received`, `duplicate`); the ones `rule_id` picked also tell which
///   release the folder's episode came from;
/// - `listing`: the torrents Transmission held when the worker last looked,
///   which tells the items of `settled` that were removed since ([`departed`]);
/// - `titles`: the titles history holds for the channel; only those that name a
///   CRC32 are kept.
pub fn build(
    offset: i64,
    season: Option<u32>,
    folder: Folder,
    rule_id: &str,
    settled: &[HistoryItem],
    listing: Option<&TorrentListing>,
    titles: &[String],
) -> World {
    let mut world = World {
        offset,
        season,
        ..World::default()
    };
    let in_folder = folder.episodes();
    for (episode, path) in folder.files {
        let present: &mut Present = world.present.entry(episode).or_default();
        // Two videos of one episode: the first by name is the one looked at.
        present.file.get_or_insert(path);
    }
    let mut records: BTreeMap<Episode, (Vec<Known>, bool)> = BTreeMap::new();
    for item in settled.iter().filter(|item| item.result.is_settled()) {
        if item.rule_id.as_deref() != Some(rule_id) {
            continue;
        }
        if let Some(folder) = folder_episode_of(item, offset) {
            let (known, there) = records.entry(folder).or_default();
            // The release stays known for a video that is still in the folder
            // after its torrent was removed.
            known.push(Known::of(&item.title));
            *there |= !torrent_gone(item, listing);
        }
    }
    let mut held = HashSet::new();
    let mut gone = HashSet::new();
    for item in settled.iter().filter(|item| item.result.is_settled()) {
        // Another torrent of the rule for the same episode is still in
        // Transmission: the episode is there, whatever became of this one.
        let elsewhere = item.rule_id.as_deref() == Some(rule_id)
            && folder_episode_of(item, offset)
                .and_then(|folder| records.get(&folder))
                .is_some_and(|(_, there)| *there);
        if !elsewhere && departed(item, offset, in_folder.as_ref(), listing) {
            gone.insert(item.identity_key.clone());
        } else {
            held.insert(item.identity_key.clone());
        }
    }
    for (folder, (known, in_transmission)) in records {
        let present = world.present.entry(folder).or_default();
        present.records = known;
        present.in_transmission = in_transmission;
    }
    world.departed = gone;
    world.held = held;
    world.releases = releases(titles);
    world
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::store::history::HistoryResult;

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
        let folder = read_folder(dir.path()).unwrap();
        assert!(folder.whole);
        let found: Vec<_> = folder.files.into_iter().map(|(e, _)| e).collect();
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
        // A folder that is not there holds none, and was not read.
        let none = read_folder(&dir.path().join("none")).unwrap();
        assert!(none.files.is_empty() && !none.whole);
    }

    #[test]
    fn a_folder_with_more_entries_than_are_looked_at_is_not_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        for n in 0..MAX_FILES {
            fs::write(dir.path().join(format!("{n}.txt")), b"").unwrap();
        }
        assert!(read_folder(dir.path()).unwrap().whole);
        fs::write(dir.path().join("one-more.txt"), b"").unwrap();
        assert!(!read_folder(dir.path()).unwrap().whole);
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
        let world = build(-12, Some(2), Folder::default(), "r", &settled, None, &[]);
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
        let world = build(0, None, Folder::default(), "r", &[], None, &titles);
        let versions: Vec<(u32, Option<u32>)> =
            world.releases.iter().map(|k| (k.version, k.crc)).collect();
        assert_eq!(versions, vec![(1, Some(0xAAAA0005)), (2, Some(0xBBBB0005))]);
    }

    // --- an item whose torrent and video are both gone ----------------------------------

    const KEY: &str = "guid:ep5";
    const TITLE: &str = "[SubsPlease] Show - 05 (1080p) [AAAA0005].mkv";
    const HASH: &str = "aaaa000000000000000000000000000000000005";

    /// Episode 5 of the rule `r`, received at 100 with its torrent hash known.
    fn received_five() -> HistoryItem {
        let mut received = item(KEY, TITLE, Some("r"), HistoryResult::Received);
        received.result_at = 100;
        received.torrent_hash = Some(HASH.into());
        received
    }

    /// What Transmission held when the worker looked at 200.
    fn listing(hashes: &[&str]) -> TorrentListing {
        TorrentListing {
            taken_at: 200,
            hashes: hashes.iter().map(|h| h.to_string()).collect(),
        }
    }

    /// A folder read whole that holds no video.
    fn empty() -> Folder {
        Folder::complete(vec![])
    }

    fn five() -> Vec<(Episode, PathBuf)> {
        vec![(Episode::whole(5), PathBuf::from("Show S01E05.mkv"))]
    }

    fn judged_five(world: &World) -> crate::past_search::judge::Item {
        use crate::past_search::judge::{judge, Range, Result as Judged};
        let results = vec![Judged {
            key: KEY.into(),
            title: TITLE.into(),
            shown: TITLE.into(),
        }];
        let preview = judge(
            &results,
            Range { from: 1, to: 12 },
            world,
            &|_: &str| true,
            &mut |_: &Path| panic!("no video was to be read"),
        );
        preview.items.into_iter().next().unwrap()
    }

    #[test]
    fn an_episode_whose_video_and_torrent_are_gone_is_missing_and_can_be_chosen() {
        use crate::past_search::judge::State;
        let settled = [received_five()];
        // The video was deleted and the torrent removed: Transmission's list,
        // taken after the item was received, does not have it.
        let world = build(0, None, empty(), "r", &settled, Some(&listing(&["x"])), &[]);
        let five = judged_five(&world);
        assert_eq!(
            (five.state, five.selected, five.selectable),
            (State::Missing, true, true)
        );
        assert!(world.held.is_empty());
        assert!(world.departed.contains(KEY));
        assert!(!world.has_release(5));
    }

    #[test]
    fn an_episode_whose_video_is_in_the_folder_is_had_whatever_became_of_its_torrent() {
        use crate::past_search::judge::State;
        let settled = [received_five()];
        let world = build(
            0,
            None,
            Folder::complete(five()),
            "r",
            &settled,
            Some(&listing(&["x"])),
            &[],
        );
        let five = judged_five(&world);
        assert_eq!(
            (five.state, five.selected, five.selectable),
            (State::Have, false, false)
        );
        assert!(world.held.contains(KEY) && world.departed.is_empty());
        // The video's release stays known after its torrent was removed.
        assert_eq!(world.present[&Episode::whole(5)].records.len(), 1);
    }

    #[test]
    fn a_torrent_still_in_transmission_keeps_its_episode_before_the_video_is_placed() {
        use crate::past_search::judge::State;
        let settled = [received_five()];
        let world = build(
            0,
            None,
            empty(),
            "r",
            &settled,
            Some(&listing(&[HASH])),
            &[],
        );
        let five = judged_five(&world);
        assert_eq!(
            (five.state, five.selected, five.selectable),
            (State::Have, false, false)
        );
        assert!(world.has_release(5));
    }

    #[test]
    fn what_the_list_cannot_say_leaves_the_torrent_there() {
        // No list yet, an item received after the list was taken, and an item
        // whose hash history does not know.
        let mut after = received_five();
        after.result_at = 300;
        let mut unhashed = received_five();
        unhashed.torrent_hash = None;
        for (settled, list) in [
            (received_five(), None),
            (after, Some(listing(&["x"]))),
            (unhashed, Some(listing(&["x"]))),
        ] {
            let world = build(0, None, empty(), "r", &[settled], list.as_ref(), &[]);
            assert!(world.held.contains(KEY));
            assert!(world.has_release(5));
        }
    }

    #[test]
    fn a_hash_is_found_whatever_its_case() {
        let mut upper = received_five();
        upper.torrent_hash = Some(HASH.to_ascii_uppercase());
        assert!(!torrent_gone(&upper, Some(&listing(&[HASH]))));
    }

    #[test]
    fn a_batch_or_an_unnumbered_item_is_never_gone() {
        // A batch's torrent is routinely removed once it is done.
        let gone = listing(&["x"]);
        for title in [
            "[SubsPlease] Show (01-12) (1080p) [Batch]",
            "[SubsPlease] Show (1080p) [Special]",
        ] {
            let mut batch = item("guid:batch", title, None, HistoryResult::Received);
            batch.result_at = 100;
            batch.torrent_hash = Some("bbbb".into());
            assert!(torrent_gone(&batch, Some(&gone)));
            assert!(!departed(&batch, 0, Some(&HashSet::new()), Some(&gone)));
            let world = build(0, None, empty(), "r", &[batch], Some(&gone), &[]);
            assert!(world.held.contains("guid:batch") && world.departed.is_empty());
        }
    }

    #[test]
    fn a_folder_that_is_missing_or_not_read_whole_says_no_video_is_gone() {
        use crate::past_search::judge::State;
        // A media volume that is not mounted, or too many entries to look at.
        let settled = [received_five()];
        for folder in [
            Folder::default(),
            Folder {
                files: vec![],
                whole: false,
            },
        ] {
            let world = build(0, None, folder, "r", &settled, Some(&listing(&["x"])), &[]);
            let five = judged_five(&world);
            assert_eq!(
                (five.state, five.selected, five.selectable),
                (State::Have, false, false)
            );
            assert!(world.held.contains(KEY) && world.departed.is_empty());
        }
    }

    #[test]
    fn another_torrent_of_the_rule_for_the_episode_keeps_the_episode_there() {
        // Episode 5 came twice, from two releases; the first one's torrent was
        // removed and the second one's is in Transmission.
        let mut other = item(
            "guid:erai5",
            "[Erai-raws] Show - 05 [1080p][ABCD1234].mkv",
            Some("r"),
            HistoryResult::Received,
        );
        other.result_at = 100;
        other.torrent_hash = Some("eeee".into());
        let settled = [received_five(), other];
        let world = build(
            0,
            None,
            empty(),
            "r",
            &settled,
            Some(&listing(&["eeee"])),
            &[],
        );
        assert!(world.held.contains(KEY) && world.departed.is_empty());
        assert!(world.has_release(5));
    }
}
