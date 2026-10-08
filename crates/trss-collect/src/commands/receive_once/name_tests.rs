//! The name a `다시 받기` gives its file, or the reason it keeps the name the
//! release came with (`receive_once::run` with the real rename step against
//! the fake Transmission). The cases of the rename itself are in
//! `receive/tests.rs`; here are what `run` joins to it: the rule's folder and
//! episode conversion, the file's own name (not the feed's title), and the
//! note written on the item.

use std::path::Path;

use trss_core::commands::CommandState;

use super::fixtures::*;
use crate::{
    commands::receive_once::{NAME_NOT_DERIVED, SEVERAL_FILES},
    store::{channels::RuleInput, history::HistoryResult},
    test_world::{feed_xml_of, magnet, World, REFUSAL},
};

/// A world whose channel feed holds `items` as `(guid, title, link)`, read by
/// a cycle: Transmission takes the adds, or refuses them when `refuse`, which
/// leaves the items the rules pick `add_failed`.
async fn read_once(rules: Vec<RuleInput>, items: &[(&str, &str, &str)], refuse: bool) -> World {
    let s = World::with_rules(rules).await;
    s.feeds.set_xml("show", &feed_xml_of(items));
    if refuse {
        s.tr.reject_adds(Some(REFUSAL));
    }
    s.cycle().await;
    s.tr.reject_adds(None);
    s.tr.clear_calls();
    s
}

#[tokio::test]
async fn a_name_trname_cannot_give_stays_in_transmission_with_its_data_and_is_noted() {
    // (what, the release's name, the rule, the files it has, the note, the
    // folder the torrent goes into under the collect folder)
    let cases: [(&str, &str, RuleInput, usize, &str, &str); 4] = [
        (
            "a rule without a folder: nothing to name the file after",
            LIAR,
            rule("LIAR GAME", ""),
            1,
            NAME_NOT_DERIVED,
            "",
        ),
        (
            "a release with no episode in it, into a folder without a season",
            "Some Special Collection.mkv",
            rule("Some Special", "Some Show"),
            1,
            NAME_NOT_DERIVED,
            "Some Show",
        ),
        (
            "a movie: the CRC32 would read as an episode",
            "[Group] Show Movie (BD 1080p) [ABCD1234].mkv",
            rule("Show Movie", "Show/Season 01"),
            1,
            NAME_NOT_DERIVED,
            "Show/Season 01",
        ),
        (
            "a torrent with several files",
            LIAR,
            rule("LIAR GAME", LIAR_DIR),
            12,
            SEVERAL_FILES,
            LIAR_DIR,
        ),
    ];
    for (what, name, the_rule, files, note, folder) in cases {
        let link = magnet(&hash(9), name);
        let s = read_once(vec![the_rule], &[("guid-9", name, &link)], true).await;
        let item = s.item_containing(name).await;
        assert_eq!(item.result, HistoryResult::AddFailed, "{what}");
        s.tr.files_on_add(&hash(9), files);

        let finished = s.retry(item.id, CMD).await.unwrap();

        // Added into the rule's folder, under its own name; nothing is
        // removed or renamed.
        let torrents = s.tr.torrents();
        assert_eq!(torrents.len(), 1, "{what}: {torrents:?}");
        assert_eq!(torrents[0].name, name, "{what}");
        let expected = s.media.join(folder);
        assert_eq!(Path::new(&torrents[0].download_dir), expected, "{what}");
        let adds = s.tr.calls_of("torrent-add");
        assert_eq!(
            Path::new(adds[0].args["download-dir"].as_str().unwrap()),
            expected,
            "{what}"
        );
        assert!(s.tr.calls_of("torrent-remove").is_empty(), "{what}");
        assert!(s.tr.calls_of("torrent-rename-path").is_empty(), "{what}");
        // Added, with a note that the name was not changed.
        let received = s.item_containing(name).await;
        assert_eq!(received.result, HistoryResult::Received, "{what}");
        assert_eq!(received.reason.as_deref(), Some(note), "{what}");
        assert_eq!(finished.state, CommandState::Done, "{what}");
        assert_eq!(finished.outcome.result, "received", "{what}");

        // The next cycle leaves it as well.
        s.cycle().await;
        assert!(s.tr.calls_of("torrent-remove").is_empty(), "{what}");
        assert_eq!(s.tr.torrents().len(), 1, "{what}");
        assert_eq!(s.tr.torrents()[0].name, name, "{what}");
    }
}

/// The feed's item title is not its torrent's file name: read alone, the
/// title's `1080p` would give episode 80.
const SHOW_TITLE: &str = "Show - 05 [1080p]";
const SHOW_FILE: &str = "[Group] Show - 05 [1080p].mkv";
const SONO: &str = "[SubsPlease] Sono Bisque Doll - 13 (1080p) [ABCD1236].mkv";

#[tokio::test]
async fn a_retry_names_the_file_and_chooses_the_folder_as_the_cycle_does() {
    // (what, the rule, the feed's item as (guid, title, link), a part of the
    // title, the name the file comes with, the name it gets and its folder
    // under the collect folder)
    #[allow(clippy::type_complexity)]
    let cases: [(
        &str,
        RuleInput,
        (&str, &str, String),
        &str,
        &str,
        &str,
        &str,
    ); 2] = [
        (
            "the rule's folder and episode conversion",
            RuleInput {
                episode: -12,
                ..rule("Sono Bisque Doll", "Sono Bisque Doll/Season 02")
            },
            ("guid-sono-13", SONO, magnet(&hash(13), SONO)),
            "Sono Bisque Doll - 13",
            SONO,
            "Sono Bisque Doll S02E01.mkv",
            "Sono Bisque Doll/Season 02",
        ),
        (
            "the file's own name, when the feed's title is not it",
            RuleInput {
                episode: 12,
                ..rule("Show", "Show/Season 01")
            },
            ("guid-show-05", SHOW_TITLE, magnet(&hash(5), SHOW_FILE)),
            "Show - 05",
            SHOW_FILE,
            "Show S01E16.mkv",
            "Show/Season 01",
        ),
    ];
    for (what, the_rule, (guid, title, link), part, came_as, file, folder) in cases {
        let items = [(guid, title, link.as_str())];
        let rules = || vec![the_rule.clone()];
        let cycled = read_once(rules(), &items, false).await;
        let retried = read_once(rules(), &items, true).await;
        let item = retried.item_containing(part).await;
        assert_eq!(item.result, HistoryResult::AddFailed, "{what}");

        let finished = retried.retry(item.id, CMD).await.unwrap();

        assert_eq!(finished.state, CommandState::Done, "{what}");
        let by_cycle = &cycled.tr.torrents()[0];
        let by_retry = &retried.tr.torrents()[0];
        assert_eq!(by_cycle.name, file, "{what}");
        assert_eq!(by_retry.name, by_cycle.name, "{what}");
        assert_eq!(
            Path::new(&by_cycle.download_dir),
            cycled.media.join(folder),
            "{what}"
        );
        assert_eq!(
            Path::new(&by_retry.download_dir),
            Path::new(&by_cycle.download_dir)
                .strip_prefix(&cycled.media)
                .map(|relative| retried.media.join(relative))
                .unwrap(),
            "{what}"
        );
        // The command recorded the name the file came with.
        assert_eq!(
            retried.command(CMD).await.original_name.as_deref(),
            Some(came_as),
            "{what}"
        );
    }
}
