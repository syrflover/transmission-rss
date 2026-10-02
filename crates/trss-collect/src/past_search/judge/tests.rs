use std::cell::RefCell;

use crate::past_search::{judge::*, release::Episode};

fn result(n: usize, title: &str) -> Result {
    Result {
        key: format!("guid:{n}"),
        title: title.to_owned(),
        shown: title.to_owned(),
    }
}

fn results(titles: &[&str]) -> Vec<Result> {
    titles
        .iter()
        .enumerate()
        .map(|(n, t)| result(n, t))
        .collect()
}

fn everything(_: &str) -> bool {
    true
}

fn no_crc(_: &Path) -> io::Result<u32> {
    panic!("no video was to be read")
}

fn range(from: u32, to: u32) -> Range {
    Range { from, to }
}

fn world(offset: i64) -> World {
    World {
        offset,
        season: Some(2),
        ..World::default()
    }
}

fn judged(titles: &[&str], range: Range, world: &World) -> Preview {
    judge(&results(titles), range, world, &everything, &mut no_crc)
}

fn state_of<'a>(preview: &'a Preview, title_part: &str) -> &'a Item {
    let mut found = preview
        .items
        .iter()
        .filter(|i| i.title.contains(title_part));
    let item = found.next().unwrap_or_else(|| panic!("no {title_part}"));
    assert!(found.next().is_none(), "several {title_part}");
    item
}

fn selected(preview: &Preview) -> Vec<u32> {
    preview
        .items
        .iter()
        .filter(|i| i.selected)
        .filter_map(|i| i.release)
        .collect()
}

#[test]
fn a_search_that_returns_every_episode_selects_the_missing_ones_of_the_range() {
    let titles: Vec<String> = (1..=12)
        .rev()
        .map(|n| format!("[SubsPlease] Sayonara Lara - {n:02} (1080p) [AAAA{n:04}].mkv"))
        .collect();
    let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
    let preview = judged(&refs, range(1, 12), &world(0));
    assert_eq!(preview.items.len(), 12);
    assert_eq!(selected(&preview), (1..=12).collect::<Vec<u32>>());
    assert_eq!(preview.missing, (1..=12).collect::<Vec<u32>>());
    assert!(preview.not_found.is_empty());
    assert_eq!(preview.out_of_range, 0);
    // In the folder's order.
    let order: Vec<_> = preview.items.iter().filter_map(|i| i.release).collect();
    assert_eq!(order, (1..=12).collect::<Vec<u32>>());
}

#[test]
fn another_season_a_batch_and_a_revision_are_told_apart() {
    // Sono Bisque Doll: season 1 is 01-12, season 2 is 13-24 (folder 1-12 by -12).
    let mut titles: Vec<String> = (1..=24)
        .map(|n| format!("[SubsPlease] Sono Bisque Doll - {n:02} (1080p) [AAAA{n:04}].mkv"))
        .collect();
    titles.push("[SubsPlease] Sono Bisque Doll - 14v2 (1080p) [BBBB0014].mkv".into());
    titles.push("[SubsPlease] Sono Bisque Doll (01-12) (1080p) [Batch]".into());
    titles.push("[Unofficial] Sono Bisque Doll (01-24) Unofficial Batch".into());
    let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
    let preview = judged(&refs, range(13, 24), &world(-12));

    // Season 1 (12 episodes) and the official batch of 1-12 are only counted.
    assert_eq!(preview.out_of_range, 13);
    // 12 individual episodes + 14v2 + the unofficial batch that covers the range.
    assert_eq!(preview.items.len(), 12 + 1 + 1);

    let unofficial = state_of(&preview, "Unofficial Batch");
    assert_eq!(
        (unofficial.state, unofficial.selected),
        (State::Batch, false)
    );

    let v2 = state_of(&preview, "14v2");
    assert_eq!((v2.state, v2.selected), (State::Missing, true));
    let v1 = state_of(&preview, "- 14 (1080p)");
    assert_eq!((v1.state, v1.selected), (State::Superseded, false));
    assert!(v1.note.as_deref().unwrap().contains("v2"));

    assert_eq!(selected(&preview), (13..=24).collect::<Vec<u32>>());
    assert_eq!(
        state_of(&preview, "- 13 ").folder.as_deref(),
        Some("S02E01")
    );
    assert_eq!(v2.folder.as_deref(), Some("S02E02"));
    // The batches come after the episodes.
    assert_eq!(preview.items.last().unwrap().state, State::Batch);
}

#[test]
fn an_episode_the_folder_has_is_shown_and_not_selected() {
    let mut world = world(-12);
    world.present.insert(
        Episode::whole(1),
        Present {
            file: Some("/media/Show/Season 02/Show S02E01.mkv".into()),
            records: vec![],
            in_transmission: false,
        },
    );
    let preview = judged(
        &[
            "[SubsPlease] Show - 14 (1080p) [AAAA0014].mkv",
            "[SubsPlease] Show - 13 (1080p) [AAAA0013].mkv",
        ],
        range(13, 14),
        &world,
    );
    let thirteen = state_of(&preview, "- 13 ");
    assert_eq!(
        (thirteen.state, thirteen.selected, thirteen.selectable),
        (State::Have, false, true)
    );
    assert!(thirteen.note.as_deref().unwrap().contains("S02E01"));
    assert_eq!(selected(&preview), vec![14]);
    assert_eq!(preview.missing, vec![14]);
}

#[test]
fn an_episode_received_from_another_release_is_judged_by_its_number() {
    // Erai-raws' release is in Transmission (and the folder); the search
    // returns SubsPlease's torrent of the same episode: a different hash.
    let mut world = world(0);
    world.present.insert(
        Episode::whole(5),
        Present {
            file: Some("/media/Show/Season 01/Show S01E05.mkv".into()),
            records: vec![Known::of("[Erai-raws] Show - 05 [1080p][ABCD1234].mkv")],
            in_transmission: false,
        },
    );
    let preview = judged(
        &["[SubsPlease] Show - 05 (1080p) [AAAA0005].mkv"],
        range(1, 12),
        &world,
    );
    let item = &preview.items[0];
    assert_eq!((item.state, item.selected), (State::Have, false));
    assert!(item.note.as_deref().unwrap().contains("다른 릴리스"));

    // The same, when the video is still downloading and only history knows it.
    let held = world.present.get_mut(&Episode::whole(5)).unwrap();
    held.file = None;
    held.in_transmission = true;
    let preview = judged(
        &["[SubsPlease] Show - 05 (1080p) [AAAA0005].mkv"],
        range(1, 12),
        &world,
    );
    assert_eq!(preview.items[0].state, State::Have);
    assert!(preview.items[0]
        .note
        .as_deref()
        .unwrap()
        .contains("Transmission"));
}

#[test]
fn an_item_history_says_transmission_holds_cannot_be_selected() {
    let mut world = world(0);
    world.held.insert("guid:0".into());
    let preview = judged(
        &["[SubsPlease] Show - 05 (1080p) [AAAA0005].mkv"],
        range(1, 12),
        &world,
    );
    let item = &preview.items[0];
    assert_eq!(
        (item.state, item.selected, item.selectable),
        (State::Have, false, false)
    );
}

#[test]
fn a_revision_of_a_video_whose_release_history_knows_is_judged_by_that_release() {
    let mut world = world(0);
    world.present.insert(
        Episode::whole(14),
        Present {
            file: Some("/media/Show/Season 01/Show S01E14.mkv".into()),
            records: vec![Known::of("[SubsPlease] Show - 14 (1080p) [AAAA0014].mkv")],
            in_transmission: false,
        },
    );
    let preview = judged(
        &[
            "[SubsPlease] Show - 14v2 (1080p) [BBBB0014].mkv",
            "[SubsPlease] Show - 14v3 (1080p)",
            "[Erai-raws] Show - 14v2 [1080p][CCCC0014].mkv",
        ],
        range(14, 14),
        &world,
    );
    let v2 = state_of(&preview, "SubsPlease] Show - 14v2");
    assert_eq!((v2.state, v2.selected), (State::Replace, false));
    // Replacing deletes a video, so it is the person's to choose.
    assert!(v2.note.as_deref().unwrap().contains("교체"));
    // v3 has no CRC32 to check the received video by.
    let v3 = state_of(&preview, "14v3");
    assert_eq!(v3.state, State::VersionUnknown);
    assert!(v3.note.as_deref().unwrap().contains("CRC32"));
    // Another group's release of the episode is a duplicate, not a revision.
    assert_eq!(state_of(&preview, "Erai-raws").state, State::Have);

    // The same or a higher revision is had.
    world
        .present
        .get_mut(&Episode::whole(14))
        .unwrap()
        .records
        .push(Known::of("[SubsPlease] Show - 14v2 (1080p) [BBBB0014].mkv"));
    let preview = judged(
        &["[SubsPlease] Show - 14v2 (1080p) [BBBB0014].mkv"],
        range(14, 14),
        &world,
    );
    assert_eq!(preview.items[0].state, State::Have);
}

fn file_world(file: &str) -> World {
    let mut world = world(0);
    world.present.insert(
        Episode::whole(14),
        Present {
            file: Some(file.into()),
            records: vec![],
            in_transmission: false,
        },
    );
    world
}

#[test]
fn a_video_of_unknown_version_is_told_by_its_crc() {
    let reads = RefCell::new(Vec::new());
    let mut read_crc = |path: &Path| -> io::Result<u32> {
        reads.borrow_mut().push(path.to_owned());
        Ok(0xE2675E51)
    };
    let world = file_world("/media/Show/Season 01/Show S01E14.mkv");

    // Equal to the result's: the folder has this revision.
    let preview = judge(
        &results(&["[SubsPlease] Show - 14v2 (1080p) [E2675E51].mkv"]),
        range(14, 14),
        &world,
        &everything,
        &mut read_crc,
    );
    assert_eq!(preview.items[0].state, State::Have);

    // Equal to the v1 of the same release: replaced.
    let preview = judge(
        &results(&[
            "[SubsPlease] Show - 14v2 (1080p) [1A2B3C4D].mkv",
            "[SubsPlease] Show - 14 (1080p) [E2675E51].mkv",
        ]),
        range(14, 14),
        &world,
        &everything,
        &mut read_crc,
    );
    assert_eq!(state_of(&preview, "14v2").state, State::Replace);

    // Equal to nothing known: the version is unknown.
    let preview = judge(
        &results(&["[SubsPlease] Show - 14v2 (1080p) [1A2B3C4D].mkv"]),
        range(14, 14),
        &world,
        &everything,
        &mut read_crc,
    );
    let item = &preview.items[0];
    assert_eq!((item.state, item.selected), (State::VersionUnknown, false));
    assert!(item.note.as_deref().unwrap().contains("CRC32"));
    assert!(reads
        .borrow()
        .iter()
        .all(|p| p.ends_with("Show S01E14.mkv")));
}

#[test]
fn the_lower_revision_may_be_in_the_channels_history_instead_of_the_results() {
    let mut world = file_world("/media/Show/Season 01/Show S01E14.mkv");
    world
        .releases
        .push(Known::of("[SubsPlease] Show - 14 (1080p) [E2675E51].mkv"));
    let preview = judge(
        &results(&["[SubsPlease] Show - 14v2 (1080p) [1A2B3C4D].mkv"]),
        range(14, 14),
        &world,
        &everything,
        &mut |_: &Path| Ok(0xE2675E51),
    );
    assert_eq!(preview.items[0].state, State::Replace);
}

#[test]
fn a_revision_whose_name_has_no_crc_or_whose_video_cannot_be_read_is_unknown() {
    let world = file_world("/media/Show/Season 01/Show S01E14.mkv");
    let preview = judged(&["[SubsPlease] Show - 14v2 (1080p)"], range(14, 14), &world);
    assert_eq!(preview.items[0].state, State::VersionUnknown);

    let preview = judge(
        &results(&["[SubsPlease] Show - 14v2 (1080p) [1A2B3C4D].mkv"]),
        range(14, 14),
        &world,
        &everything,
        &mut |_: &Path| Err(io::Error::other("no")),
    );
    assert_eq!(preview.items[0].state, State::VersionUnknown);
}

#[test]
fn a_revision_of_an_episode_the_work_lacks_is_an_ordinary_missing_episode() {
    let preview = judged(
        &["[SubsPlease] Show - 14v2 (1080p) [1A2B3C4D].mkv"],
        range(14, 14),
        &world(0),
    );
    assert_eq!(preview.items[0].state, State::Missing);
    assert!(preview.items[0].selected);
}

#[test]
fn the_files_read_for_one_preview_are_bounded() {
    let mut world = world(0);
    let mut titles = Vec::new();
    for n in 1..=(MAX_CRC_READS as u32 + 5) {
        world.present.insert(
            Episode::whole(n),
            Present {
                file: Some(format!("/media/Show/Season 01/Show S01E{n:02}.mkv").into()),
                records: vec![],
                in_transmission: false,
            },
        );
        titles.push(format!(
            "[SubsPlease] Show - {n:02}v2 (1080p) [1A2B{n:04X}].mkv"
        ));
    }
    let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
    let reads = RefCell::new(0);
    let preview = judge(
        &results(&refs),
        range(1, 40),
        &world,
        &everything,
        &mut |_: &Path| {
            *reads.borrow_mut() += 1;
            Ok(1)
        },
    );
    assert_eq!(*reads.borrow(), MAX_CRC_READS);
    assert_eq!(preview.items.len(), MAX_CRC_READS + 5);
}

#[test]
fn what_the_rule_does_not_pick_is_only_counted() {
    let picks = |title: &str| title.contains("Sayonara Lara");
    let preview = judge(
        &results(&[
            "[SubsPlease] Sayonara Lara - 03 (1080p) [AAAA0003].mkv",
            "[SubsPlease] Sayonara Lara Log - 03 (1080p) [AAAA0013].mkv",
            "[SubsPlease] Another Show - 03 (1080p) [AAAA0023].mkv",
        ]),
        range(1, 12),
        &world(0),
        &|t| picks(t) && !t.contains("Log"),
        &mut no_crc,
    );
    assert_eq!(preview.items.len(), 1);
    assert_eq!(preview.not_picked, 2);
}

#[test]
fn the_missing_episodes_no_result_is_are_listed() {
    let preview = judged(
        &[
            "[SubsPlease] Show - 03 (1080p) [AAAA0003].mkv",
            "[SubsPlease] Show (01-12) [Batch]",
        ],
        range(1, 5),
        &world(0),
    );
    assert_eq!(preview.missing, vec![1, 2, 3, 4, 5]);
    // A batch is not an episode of the range, whatever it covers.
    assert_eq!(preview.not_found, vec![1, 2, 4, 5]);
}

#[test]
fn a_title_without_a_number_is_shown_and_not_selected() {
    let preview = judged(
        &["[SubsPlease] Show Movie (1080p).mkv"],
        range(1, 12),
        &world(0),
    );
    assert_eq!(preview.items[0].state, State::Unnumbered);
    assert!(!preview.items[0].selected);
}

#[test]
fn two_releases_of_one_episode_select_only_one() {
    let preview = judged(
        &[
            "[SubsPlease] Show - 05 (1080p) [AAAA0005].mkv",
            "[SubsPlease] Show - 05 (720p) [BBBB0005].mkv",
        ],
        range(1, 12),
        &world(0),
    );
    assert_eq!(selected(&preview), vec![5]);
    assert_eq!(state_of(&preview, "(720p)").state, State::Alternate);
}

#[test]
fn the_folder_episode_follows_trname() {
    let folder = std::path::Path::new("/media/Show/Season 02");
    for (offset, release) in [
        (-12i64, 13u32),
        (-12, 24),
        (-12, 5),
        (0, 7),
        (1, 7),
        (3, 7),
        (-48, 62),
        (-24, 25),
    ] {
        let title = format!("[SubsPlease] Show - {release:02} (1080p) [ABCD1234].mkv");
        let named = trname::trname(folder, &title, offset as isize).unwrap();
        let got = folder_episode(Episode::whole(release), offset);
        let expected = format!("Show S02E{:02}.mkv", got.number);
        assert_eq!(named, expected, "release {release} by {offset}");
    }
    assert_eq!(
        folder_episode(
            Episode {
                number: 65,
                half: true
            },
            -48
        ),
        Episode {
            number: 17,
            half: true
        }
    );
}
