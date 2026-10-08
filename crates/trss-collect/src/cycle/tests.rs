//! What a cycle decides about the items of a feed (ADR 0015: a rule is tested
//! in the crate that owns it): which rule takes an item, where and under which
//! name its torrent goes, what is done with a torrent Transmission holds
//! already, which torrents are removed when their items leave the feeds, and
//! what history records. The cycle is the world's [`World::cycle`], which runs
//! the steps of the worker's `run_cycle` in its order against the fakes; what
//! only a process shows (the lock, the order of cycles and commands, the
//! tallies of a report, a cycle that is dropped or shut down) is the worker's.
//!
//! Release names, hashes and secrets are made up.

use serde_json::json;
use trss_transmission::{fake::FakeTorrent, item_label, BOT_LABEL};

use crate::{
    feed::MAX_FEED_BYTES,
    receive::{NAME_NOT_DERIVED, NAME_TAKEN, SEVERAL_FILES},
    store::{
        channels::{ChannelInput, RuleInput},
        history::{HistoryItem, HistoryResult},
    },
    test_world::{feed_xml_of, magnet, World, SECRET},
};

/// A hash for the n-th made-up release.
fn hash(n: u32) -> String {
    format!("aaaa{n:036}")
}

fn rule(match_text: &str, directory: &str) -> RuleInput {
    RuleInput {
        r#match: Some(match_text.to_owned()),
        directory: directory.to_owned(),
        ..Default::default()
    }
}

const SLIME: &str = "[SubsPlease] Tensei Shitara Slime Datta Ken - 62 (1080p) [AAAA0006].mkv";
const SLIME_RULE: &str = "[SubsPlease] Tensei Shitara Slime Datta Ken";

fn slime_rule() -> RuleInput {
    RuleInput {
        episode: -24,
        ..rule(SLIME_RULE, "Slime/Season 04")
    }
}

impl World {
    /// The `show` feed holds one item per `(number, title)`, each a magnet
    /// link named after its title, with the guid `g<number>`.
    fn feed_numbered(&self, items: &[(u32, &str)]) {
        self.feed_numbered_at("show", items);
    }

    fn feed_numbered_at(&self, path: &str, items: &[(u32, &str)]) {
        let rows: Vec<(String, String, String)> = items
            .iter()
            .map(|(n, title)| {
                (
                    format!("g{n}"),
                    (*title).to_owned(),
                    magnet(&hash(*n), title),
                )
            })
            .collect();
        let rows: Vec<(&str, &str, &str)> = rows
            .iter()
            .map(|(g, t, l)| (g.as_str(), t.as_str(), l.as_str()))
            .collect();
        self.feeds.set_xml(path, &feed_xml_of(&rows));
    }

    fn renames(&self) -> usize {
        self.tr.calls_of("torrent-rename-path").len()
    }

    /// The torrent `hash` as Transmission holds it.
    fn torrent_named(&self, n: u32) -> FakeTorrent {
        self.tr.torrent(&hash(n))
    }

    /// Everything history holds, as text, with the trail of each item.
    async fn history_dump(&self) -> String {
        let mut dump = String::new();
        for item in self.history_items().await {
            dump.push_str(&format!("{item:?}\n"));
            for change in self.ctx.history.changes(item.id).await.unwrap() {
                dump.push_str(&format!("{change:?}\n"));
            }
        }
        dump
    }

    /// A torrent the bot added for the item, labelled as an add labels it.
    fn labelled_for(&self, item: &HistoryItem, hash: &str, name: &str) -> FakeTorrent {
        FakeTorrent {
            labels: vec![
                BOT_LABEL.to_owned(),
                item_label(&item.channel_id, &item.identity_key),
            ],
            ..FakeTorrent::new(hash, name)
        }
    }
}

// --- which rule takes an item, and where its torrent goes ------------------------------

#[tokio::test]
async fn a_cycle_judges_each_item_by_the_first_rule_that_takes_it_and_adds_it_there() {
    let s = World::with_rules(vec![
        rule("[SubsPlease] Sayonara Lara - ", "Sayonara Lara/Season 01"),
        RuleInput {
            case_insensitive: true,
            episode: -12,
            ..rule("sono bisque doll", "Sono Bisque Doll/Season 02")
        },
        rule("Sono Bisque Doll", "Sono Bisque Doll (overlap)"),
        slime_rule(),
    ])
    .await;
    let channel = s
        .ctx
        .channels
        .get_channel(&s.channel_id)
        .await
        .unwrap()
        .unwrap();
    s.ctx
        .channels
        .update_channel(
            &channel.id,
            channel.version,
            ChannelInput {
                excludes: vec!["[Batch]".to_owned(), "(720p)".to_owned()],
                ..channel.to_input()
            },
        )
        .await
        .unwrap();
    let rules = s.ctx.channels.list_rules(&s.channel_id).await.unwrap();
    s.feed_numbered(&[
        (1, "[SubsPlease] Sayonara Lara - 03 (1080p) [AAAA0001].mkv"),
        (2, "[SubsPlease] Sayonara Lara - 03 (720p) [AAAA0002].mkv"),
        (
            3,
            "[SubsPlease] Sono Bisque Doll - 13 (1080p) [AAAA0003].mkv",
        ),
        (4, SLIME),
        (
            5,
            "[SubsPlease] Sono Bisque Doll - 01~12 [Batch] (1080p).mkv",
        ),
        (6, "[SubsPlease] Unrelated Show - 05 (1080p) [AAAA0009].mkv"),
        (7, ""),
    ]);

    s.cycle().await;

    // The selected items went to Transmission into their rule's folder, with
    // the bot's label and the label of their item, and were named by trname
    // with the rule's episode offset.
    let mut adds: Vec<String> =
        s.tr.calls_of("torrent-add")
            .iter()
            .map(|c| c.args["download-dir"].as_str().unwrap().to_owned())
            .collect();
    adds.sort();
    let in_media = |dir: &str| s.media.join(dir).to_str().unwrap().to_owned();
    let mut expected = [
        in_media("Slime/Season 04"),
        in_media("Sayonara Lara/Season 01"),
        in_media("Sono Bisque Doll/Season 02"),
    ];
    expected.sort();
    assert_eq!(adds, expected);
    let items = s.history_items().await;
    assert_eq!(items.len(), 7, "every item is recorded");
    for call in s.tr.calls_of("torrent-add") {
        let filename = call.args["filename"].as_str().unwrap();
        let item = items
            .iter()
            .find(|i| {
                i.torrent_hash
                    .as_deref()
                    .is_some_and(|h| filename.contains(h))
            })
            .unwrap();
        assert_eq!(call.args["labels"], json!([BOT_LABEL, s.item_label(item)]));
    }
    assert_eq!(
        s.torrent_names(),
        [
            "Sayonara Lara S01E03.mkv",
            "Slime S04E38.mkv",
            "Sono Bisque Doll S02E01.mkv",
        ]
    );

    // History has the results, the rule that applied and the hash.
    let sayonara = s.item_containing("Sayonara Lara - 03 (1080p)").await;
    assert_eq!(sayonara.result, HistoryResult::Received);
    assert_eq!(sayonara.rule_id.as_deref(), Some(rules[0].id.as_str()));
    assert_eq!(sayonara.torrent_hash.as_deref(), Some(hash(1).as_str()));
    // Both "sono bisque doll" rules match; the first one wins.
    let sono = s.item_containing("Sono Bisque Doll - 13").await;
    assert_eq!(sono.result, HistoryResult::Received);
    assert_eq!(sono.rule_id.as_deref(), Some(rules[1].id.as_str()));
    let slime = s.item_containing("Slime Datta Ken - 62").await;
    assert_eq!(slime.rule_id.as_deref(), Some(rules[3].id.as_str()));
    for (part, result) in [
        ("(720p)", HistoryResult::Excluded),
        ("[Batch]", HistoryResult::Excluded),
        ("Unrelated Show", HistoryResult::NoMatch),
    ] {
        let item = s.item_containing(part).await;
        assert_eq!(item.result, result, "{part}");
        assert_eq!(item.rule_id, None, "{part}");
        assert_eq!(item.torrent_hash, None, "{part}");
    }
    // The untitled item is judged like any other and recorded.
    let untitled = items.iter().find(|i| i.title.is_empty()).unwrap();
    assert_eq!(untitled.result, HistoryResult::NoMatch);
    for item in &items {
        assert_eq!(item.channel_id, s.channel_id);
        assert_eq!(item.first_seen_at, 1_000_000);
    }
    // The identity key is a hash of the guid, not the guid.
    assert!(items.iter().all(|i| i.identity_key.starts_with("guid:")));
    assert!(items.iter().all(|i| !i.identity_key.contains("g1")));
}

#[tokio::test]
async fn items_that_differ_only_in_a_secret_named_value_are_all_added_and_recorded() {
    let s = World::bare().await;
    s.feed(&[]);
    // Every query name of the channel URL is secret, `id` included, and the
    // items' guids differ only in their `id` value.
    let input = ChannelInput::new(format!("{}?id=0&token={SECRET}", s.feeds.url("ids")));
    assert!(input.secret_query.contains(&"id".to_owned()));
    s.ctx
        .channels
        .create_channel_with_rules(input, vec![rule("Show", "Show/Season 01")])
        .await
        .unwrap();
    let rows: Vec<(String, String, String)> = (1..=3)
        .map(|n| {
            let title = format!("Show - 0{n}");
            (
                format!("https://t.test/details.php?id=10{n}"),
                title.clone(),
                magnet(&hash(n), &title),
            )
        })
        .collect();
    let rows: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|(g, t, l)| (g.as_str(), t.as_str(), l.as_str()))
        .collect();
    s.feeds.set_xml("ids", &feed_xml_of(&rows));

    s.cycle().await;

    assert_eq!(s.tr.torrents().len(), 3);
    let items = s.history_items().await;
    assert_eq!(items.len(), 3);
    assert!(items.iter().all(|i| i.result == HistoryResult::Received));
    let keys: std::collections::HashSet<_> = items.iter().map(|i| &i.identity_key).collect();
    assert_eq!(keys.len(), 3);

    // The same sighting again is still three known items, not three new ones.
    s.cycle_later().await;
    assert_eq!(s.history_items().await.len(), 3);
    assert_eq!(s.tr.torrents().len(), 3);
}

// --- a secret never reaches history, but the add uses the real link ---------------------

#[tokio::test]
async fn secrets_are_masked_in_what_a_cycle_records_under_every_spelling_but_used_for_adding() {
    let s = World::bare().await;
    s.feed(&[]);
    // The channel URL spells the secret percent-encoded; feeds may quote it
    // decoded, in a path, or encoded again inside another URL. `r=1080` is
    // secret like every query name, but too short to be replaced in text.
    const IN_URL: &str = "Tk%2Fen%2BSECRETVALUE99";
    const DECODED: &str = "Tk/en+SECRETVALUE99";
    const ENCODED_TWICE: &str = "Tk%252Fen%252BSECRETVALUE99";
    let title_one = format!("Show - 01 [{DECODED}]");
    let link_one = format!(
        "magnet:?xt=urn:btih:{}&dn=Show%20-%2001&tr=https%3A%2F%2Ftr.test%2Fa%3Ftorrent_pass%3D{ENCODED_TWICE}",
        hash(1)
    );
    let guid_one = format!("https://t.test/details/{IN_URL}/1");
    let link_two = format!("https://t.test/{IN_URL}/dl/2.torrent?torrent_pass={IN_URL}&id=2");
    let guid_two = format!("https://t.test/details/{DECODED}/2");
    let link_three = magnet(&hash(3), "Show - 03 (1080p)");
    s.feeds.set_xml(
        "other-names",
        &feed_xml_of(&[
            (&guid_one, &title_one, &link_one),
            (&guid_two, "Show - 02", &link_two),
            ("guid-3", "Show - 03 (1080p)", &link_three),
        ]),
    );
    s.ctx
        .channels
        .create_channel_with_rules(
            ChannelInput::new(format!(
                "{}?passkey={IN_URL}&r=1080",
                s.feeds.url("other-names")
            )),
            vec![rule("Show", "Show/Season 01")],
        )
        .await
        .unwrap();
    // A refusal that quotes the secret ends up in `reason`.
    s.tr.reject_adds(Some(&format!("cannot use {DECODED} or {IN_URL}")));

    s.cycle().await;

    // Transmission was asked with the real link.
    assert!(s
        .tr
        .calls_of("torrent-add")
        .iter()
        .any(|c| c.args["filename"].as_str().unwrap().contains(ENCODED_TWICE)));
    let dump = s.history_dump().await;
    for form in ["SECRETVALUE99", IN_URL, DECODED, ENCODED_TWICE] {
        assert!(!dump.contains(form), "{form} in history:\n{dump}");
    }
    let first = s.item_containing("Show - 01").await;
    assert_eq!(first.title, "Show - 01 [***]");
    assert!(first.link.contains("torrent_pass%3D***"), "{}", first.link);
    assert_eq!(first.result, HistoryResult::AddFailed);
    assert_eq!(
        first.reason.as_deref(),
        Some("Transmission이 토렌트를 받지 않았어요: cannot use *** or ***")
    );
    let second = s.item_containing("Show - 02").await;
    assert_eq!(
        second.link,
        "https://t.test/***/dl/2.torrent?torrent_pass=***&id=2"
    );
    // A short secret-looking value does not garble the titles.
    assert_eq!(
        s.item_containing("Show - 03").await.title,
        "Show - 03 (1080p)"
    );

    // Added after all, the real link is still what Transmission gets.
    s.tr.reject_adds(None);
    s.tr.clear_calls();
    s.cycle_later().await;
    let third_add =
        s.tr.calls_of("torrent-add")
            .into_iter()
            .find(|c| c.args["filename"].as_str().unwrap().contains(&hash(1)))
            .expect("the first item is added again");
    assert_eq!(third_add.args["filename"], link_one);
    let dump = s.history_dump().await;
    for form in ["SECRETVALUE99", IN_URL, DECODED, ENCODED_TWICE] {
        assert!(!dump.contains(form), "{form} in history:\n{dump}");
    }
}

// --- a name that stays ---------------------------------------------------------------------

/// What the torrent of a row meets.
#[derive(Debug, Clone, Copy)]
enum Meets {
    Nothing,
    /// Several files in the torrent.
    Files(usize),
    /// Another file holds the name `trname` gives, in the rule's folder.
    ATakenName,
}

#[tokio::test]
async fn a_name_trname_cannot_give_keeps_its_name_and_its_torrent_and_the_item_notes_why() {
    // (what, the release, the rule, what the torrent meets, the note)
    let cases: [(&str, &str, RuleInput, Meets, &str); 6] = [
        (
            "a movie: the CRC32 would read as an episode",
            "[Group] Show Movie (BD 1080p) [ABCD1234].mkv",
            rule("[Group] Show", "Show/Season 01"),
            Meets::Nothing,
            NAME_NOT_DERIVED,
        ),
        (
            "a batch",
            "[Group] Show Season 1 [BDRip 1920x1080 HEVC FLAC] (01-12).mkv",
            rule("[Group] Show", "Show/Season 01"),
            Meets::Nothing,
            NAME_NOT_DERIVED,
        ),
        (
            "a numbered release into a folder without a season",
            "[Group] Show - 05 (1080p) [ABCD1234].mkv",
            rule("[Group] Show", ""),
            Meets::Nothing,
            NAME_NOT_DERIVED,
        ),
        (
            "an episode zero: trname reads no episode from 00",
            "[SubsPlease] Show - 00 (1080p) [ABCD1234].mkv",
            rule("[SubsPlease] Show", "Show/Season 01"),
            Meets::Nothing,
            NAME_NOT_DERIVED,
        ),
        (
            "a torrent with several files",
            SLIME,
            slime_rule(),
            Meets::Files(2),
            SEVERAL_FILES,
        ),
        (
            "a name another file has in the folder",
            SLIME,
            slime_rule(),
            Meets::ATakenName,
            NAME_TAKEN,
        ),
    ];
    for (what, release, the_rule, meets, note) in cases {
        let s = World::with_rules(vec![the_rule]).await;
        s.feed_numbered(&[(1, release)]);
        let taken = s.media.join("Slime/Season 04/Slime S04E38.mkv");
        match meets {
            Meets::Nothing => {}
            Meets::Files(count) => s.tr.files_on_add(&hash(1), count),
            Meets::ATakenName => {
                std::fs::create_dir_all(taken.parent().unwrap()).unwrap();
                std::fs::write(&taken, b"someone else's").unwrap();
            }
        }

        s.cycle().await;

        assert_eq!(s.tr.torrents().len(), 1, "{what}");
        assert_eq!(s.torrent_named(1).name, release, "{what}");
        assert!(s.tr.calls_of("torrent-remove").is_empty(), "{what}");
        assert_eq!(s.renames(), 0, "{what}");
        let item = s.item_containing(release).await;
        assert_eq!(item.result, HistoryResult::Received, "{what}");
        assert_eq!(
            item.torrent_hash.as_deref(),
            Some(hash(1).as_str()),
            "{what}"
        );
        assert_eq!(item.reason.as_deref(), Some(note), "{what}");
        if let Meets::ATakenName = meets {
            assert_eq!(std::fs::read(&taken).unwrap(), b"someone else's", "{what}");
        }
    }
}

// --- a torrent Transmission holds already ------------------------------------------------

#[tokio::test]
async fn a_torrent_the_cycle_meets_again_is_renamed_only_while_it_is_an_unnamed_release_in_the_rules_folder(
) {
    // (what, the torrent's name, the folder it is in, the name afterwards)
    let cases = [
        (
            "a name trname cannot derive one from",
            "Some Special Collection.mkv",
            "Slime/Season 04",
            "Some Special Collection.mkv",
        ),
        (
            "a name the rule gave already",
            "Slime S04E38.mkv",
            "Slime/Season 04",
            "Slime S04E38.mkv",
        ),
        (
            "a name given under the folder whose title was written in another case",
            "SLIME S04E38.mkv",
            "SLIME/Season 04",
            "SLIME S04E38.mkv",
        ),
        (
            "a release another rule received into its own folder",
            SLIME,
            "Other/Season 01",
            SLIME,
        ),
        (
            "a release whose own name ends in SxxEyy, left unrenamed",
            "Tensura S04E62.mkv",
            "Slime/Season 04",
            "Slime S04E38.mkv",
        ),
        (
            "a rename cut short in the rules folder",
            SLIME,
            "Slime/Season 04",
            "Slime S04E38.mkv",
        ),
    ];
    for (what, name, folder, afterwards) in cases {
        let s = World::with_rules(vec![slime_rule()]).await;
        s.feed_numbered(&[(4, SLIME)]);
        s.tr.preload(
            FakeTorrent::new(&hash(4), name)
                .in_dir(s.media.join(folder))
                .bot(),
        );

        s.cycle().await;

        assert_eq!(
            s.item_containing("Slime Datta Ken - 62").await.result,
            HistoryResult::Duplicate,
            "{what}"
        );
        assert_eq!(s.tr.torrents().len(), 1, "{what}");
        assert_eq!(s.torrent_named(4).name, afterwards, "{what}");
        assert_eq!(s.renames(), usize::from(name != afterwards), "{what}");
        assert!(s.tr.calls_of("torrent-remove").is_empty(), "{what}");
    }
}

#[tokio::test]
async fn a_torrent_a_rule_received_and_named_is_not_renamed_again_when_it_is_met_again() {
    let s = World::with_rules(vec![slime_rule()]).await;
    s.feed_numbered(&[(4, SLIME)]);
    s.cycle().await;
    assert_eq!(s.torrent_named(4).name, "Slime S04E38.mkv");
    s.tr.clear_calls();

    // Met again as a duplicate. Its rule's episode offset (-24) must not be
    // applied a second time to the name it already has.
    s.cycle_later().await;

    assert_eq!(s.torrent_named(4).name, "Slime S04E38.mkv");
    assert_eq!(s.renames(), 0);
}

#[tokio::test]
async fn a_torrent_a_retry_added_is_kept_and_left_alone_by_the_next_cycles() {
    let (liar, other) = (
        "[SubsPlease] LIAR GAME - 26 (1080p) [ABCD1234].mkv",
        "[SubsPlease] Another Show - 03 (1080p) [ABCD1235].mkv",
    );
    let s = World::with_rules(vec![rule("LIAR GAME", "LIAR GAME/Season 01")]).await;
    s.fail_adds(&[(&hash(26), liar), (&hash(3), other)]).await;
    let item = s.item_containing("LIAR GAME - 26").await;
    s.retry(item.id, "0b7d5a44-6c1e-4c62-9a6a-3f0c1d2e4b55")
        .await
        .unwrap();
    assert_eq!(s.torrent_named(26).name, "LIAR GAME S01E26.mkv");

    // The rule picks the item again each cycle and meets its own torrent: nothing
    // is removed, and the file, named already, is not named again.
    s.tr.clear_calls();
    s.cycle_later().await;
    s.cycle_later().await;
    assert_eq!(s.tr.torrents().len(), 1);
    assert!(s.tr.calls_of("torrent-remove").is_empty());
    assert_eq!(s.renames(), 0);
    let kept = s.item_containing("LIAR GAME - 26").await;
    assert_eq!(kept.result, HistoryResult::Received);
    assert_eq!(kept.rule_id, item.rule_id);

    // Once the item leaves the feed, the ordinary cleanup applies to it again.
    s.feed(&[(&hash(3), other)]);
    s.cycle_later().await;
    assert!(s.removed(&hash(26)));
    assert!(s.tr.torrents().is_empty());
}

// --- an item received by hand ------------------------------------------------------------------

#[tokio::test]
async fn a_rule_that_later_selects_an_item_received_by_hand_neither_removes_nor_renames_it() {
    // (what, the release, the folder it was received into, the name its file
    // has, the note it had, the rule that selects it later)
    let cases = [
        (
            "received into a folder where trname has no name for the file",
            "Some Special Collection.mkv",
            "Some Show",
            "Some Special Collection.mkv",
            Some(NAME_NOT_DERIVED),
            rule("Some Special", "Some Special/Season 01"),
        ),
        (
            "a rule with another folder",
            "[SubsPlease] LIAR GAME - 26 (1080p) [ABCD1234].mkv",
            "LIAR GAME/Season 01",
            "LIAR GAME S01E26.mkv",
            None,
            rule("LIAR GAME", "LIAR GAME/Season 02"),
        ),
        (
            "the release name a person kept, in the folder of the rule",
            SLIME,
            "Slime/Season 04",
            SLIME,
            None,
            slime_rule(),
        ),
    ];
    for (what, release, folder, name, note, the_rule) in cases {
        let s = World::with_rules(vec![rule("Some Other Show", "Some Other Show/Season 01")]).await;
        s.feed_numbered(&[(9, release)]);
        s.cycle().await;
        let item = s.item_containing(release).await;
        assert_eq!(item.result, HistoryResult::NoMatch, "{what}");
        // Received by hand, as the `한 번 받기` that `다시 받기` replaced left it:
        // with no rule, its torrent in the folder and under the name a person chose.
        s.tr.preload(FakeTorrent {
            download_dir: s.media.join(folder).to_str().unwrap().to_owned(),
            ..s.labelled_for(&item, &hash(9), name)
        });
        s.ctx
            .history
            .record_outcome(
                item.id,
                s.now(),
                HistoryResult::Received,
                None,
                None,
                Some(hash(9)),
            )
            .await
            .unwrap();
        if let Some(note) = note {
            s.ctx.history.note_received(item.id, note).await.unwrap();
        }
        // The rule selects it now (a rule made from the item, say).
        s.ctx
            .channels
            .create_rule(&s.channel_id, the_rule)
            .await
            .unwrap();
        s.tr.clear_calls();

        s.cycle_later().await;

        assert_eq!(s.renames(), 0, "{what}");
        assert!(s.tr.calls_of("torrent-remove").is_empty(), "{what}");
        assert_eq!(s.tr.torrents().len(), 1, "{what}");
        assert_eq!(s.torrent_named(9).name, name, "{what}");
        let kept = s.item_containing(release).await;
        assert_eq!(kept.result, HistoryResult::Received, "{what}");
        assert_eq!(kept.rule_id, None, "{what}: still received by hand");
        assert_eq!(kept.reason.as_deref(), note, "{what}");
    }
}

// --- the torrents of items that left the feeds ---------------------------------------------------

#[tokio::test]
async fn a_torrent_labelled_for_an_item_stays_while_the_item_is_in_a_feed_and_goes_after() {
    let s = World::with_rules(vec![slime_rule()]).await;
    s.feed_numbered(&[
        (4, SLIME),
        (6, "[SubsPlease] Unrelated Show - 05 (1080p) [AAAA0009].mkv"),
    ]);
    s.cycle().await;
    // A bot torrent history knows nothing of (its add was never answered,
    // say), labelled for an item of the feed that no rule took, and one for an
    // item not in it.
    let in_feed = s.item_containing("Unrelated Show").await;
    assert_eq!(in_feed.torrent_hash, None);
    s.tr.preload(s.labelled_for(
        &in_feed,
        "feed0000000000000000000000000000000000aa",
        "Kept.mkv",
    ));
    s.tr.preload(FakeTorrent {
        labels: vec![BOT_LABEL.to_owned(), item_label(&s.channel_id, "guid:gone")],
        ..FakeTorrent::new("gone0000000000000000000000000000000000aa", "Gone.mkv")
    });

    s.cycle_later().await;

    assert!(!s.removed("feed0000000000000000000000000000000000aa"));
    assert!(s.removed("gone0000000000000000000000000000000000aa"));
    assert_eq!(s.tr.torrents().len(), 2, "the slime one and the kept one");
}

/// What goes wrong with a channel's feed.
#[derive(Debug, Clone, Copy)]
enum Feed {
    Down,
    /// Past the size cap, which a fetch refuses.
    TooLarge,
}

#[tokio::test]
async fn torrents_of_a_channel_whose_feed_failed_stay_while_the_stale_ones_of_a_read_channel_go() {
    for feed in [Feed::Down, Feed::TooLarge] {
        let s = World::with_rules(vec![rule("Nothing Takes This", "x")]).await;
        // Y is another channel, read; X is the world's, whose feed fails.
        let y = s.second_channel("feed-y").await;
        s.feed_numbered(&[(1, "[G] Show X1 - 01"), (2, "[G] Show X2 - 01")]);
        s.feed_numbered_at(
            "feed-y",
            &[(3, "[G] Show Y1 - 01"), (4, "[G] Show Y2 - 01")],
        );
        s.cycle().await;
        let item_of = |title: &'static str| {
            let s = &s;
            async move { s.item_containing(title).await }
        };
        let (x1, x2) = (item_of("X1").await, item_of("X2").await);
        let (y1, y2) = (item_of("Y1").await, item_of("Y2").await);
        assert_eq!(y1.channel_id, y);
        // Torrents the bot added for these, whose hashes history does not hold.
        for (item, hash) in [(&x1, "1"), (&x2, "2"), (&y1, "3"), (&y2, "4")] {
            s.tr.preload(s.labelled_for(item, &hash.repeat(40), &format!("Torrent {hash}")));
        }
        // Added by something else that labels its torrents (the legacy cron
        // before the switch): history has no record of it. And a person's.
        s.tr.preload(
            FakeTorrent::new("gone0000000000000000000000000000000000aa", "Old Show").bot(),
        );
        s.tr.preload(FakeTorrent::new(
            "mine0000000000000000000000000000000000bb",
            "Manual Download",
        ));
        s.tr.clear_calls();

        // X's feed fails. Y is read, and its second show has left the feed.
        match feed {
            Feed::Down => s.feeds.set_status("show", 503),
            Feed::TooLarge => s.feeds.set_large("show", MAX_FEED_BYTES + 1),
        }
        s.feed_numbered_at("feed-y", &[(3, "[G] Show Y1 - 01")]);
        s.cycle_later().await;

        // X's torrents stay whatever its feed would say; Y's departed one goes,
        // and so does the one of unknown origin.
        let mut left: Vec<String> = s.tr.torrents().into_iter().map(|t| t.name).collect();
        left.sort();
        assert_eq!(
            left,
            ["Manual Download", "Torrent 1", "Torrent 2", "Torrent 3"],
            "{feed:?}"
        );

        // Once X's feed is back and no longer lists its second show, that one goes too.
        s.feeds.set_xml("show", &feed_xml_of(&[]));
        s.feed_numbered(&[(1, "[G] Show X1 - 01")]);
        s.cycle_later().await;
        let mut left: Vec<String> = s.tr.torrents().into_iter().map(|t| t.name).collect();
        left.sort();
        assert_eq!(
            left,
            ["Manual Download", "Torrent 1", "Torrent 3"],
            "{feed:?}"
        );
    }
}

// --- history ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_new_matching_rule_turns_no_match_into_received_and_keeps_the_first_seen_time() {
    let s = World::with_rules(vec![rule("Sayonara Lara", "Sayonara Lara/Season 01")]).await;
    let unrelated = "[SubsPlease] Unrelated Show - 05 (1080p) [AAAA0009].mkv";
    s.feed_numbered(&[
        (1, "[SubsPlease] Sayonara Lara - 03 (1080p) [AAAA0001].mkv"),
        (6, unrelated),
    ]);
    s.cycle().await;
    let before = s.item_containing("Unrelated Show").await;
    assert_eq!(before.result, HistoryResult::NoMatch);

    let new_rule = s
        .ctx
        .channels
        .create_rule(
            &s.channel_id,
            rule("Unrelated Show", "Unrelated Show/Season 01"),
        )
        .await
        .unwrap();
    s.cycle_later().await;

    let after = s.item_containing("Unrelated Show").await;
    assert_eq!(after.id, before.id, "still one record");
    assert_eq!(after.result, HistoryResult::Received);
    assert_eq!(after.first_seen_at, 1_000_000);
    assert_eq!(after.result_at, 1_300_000);
    assert_eq!(after.rule_id.as_deref(), Some(new_rule.id.as_str()));
    let changes = s.ctx.history.changes(after.id).await.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        (changes[0].from, changes[0].to),
        (HistoryResult::NoMatch, HistoryResult::Received)
    );
    assert_eq!(s.history_items().await.len(), 2);
}

/// What goes wrong with the add of a cycle.
#[derive(Debug, Clone, Copy)]
enum Trouble {
    /// Transmission is not running.
    Stopped,
    /// Transmission refuses the torrent with a reason.
    Refuses,
}

#[tokio::test]
async fn an_add_that_fails_is_recorded_with_its_reason_and_rule_and_a_later_cycle_recovers_it() {
    for trouble in [Trouble::Stopped, Trouble::Refuses] {
        let (mut s, says) = (
            World::with_rules(vec![rule("LIAR GAME", "LIAR GAME/Season 01")]).await,
            match trouble {
                Trouble::Stopped => "Transmission에 연결하지 못했어요",
                Trouble::Refuses => "download directory path is not absolute",
            },
        );
        s.feed_numbered(&[(26, "[SubsPlease] LIAR GAME - 26 (1080p) [ABCD1234].mkv")]);
        let rule_id = s.rule_of(0).await.id;
        match trouble {
            Trouble::Stopped => s.tr.stop().await,
            Trouble::Refuses => {
                s.tr.reject_adds(Some("download directory path is not absolute"))
            }
        }

        s.cycle().await;

        // Everything is still recorded, with the reason, the rule and no hash.
        let failed = s.item_containing("LIAR GAME - 26").await;
        assert_eq!(failed.result, HistoryResult::AddFailed, "{trouble:?}");
        assert!(
            failed.reason.as_deref().unwrap().contains(says),
            "{trouble:?}: {failed:?}"
        );
        assert_eq!(
            failed.rule_id.as_deref(),
            Some(rule_id.as_str()),
            "{trouble:?}"
        );
        assert_eq!(failed.torrent_hash, None, "{trouble:?}");

        // The next cycle, with Transmission back, adds it.
        match trouble {
            Trouble::Stopped => s.tr.restart().await,
            Trouble::Refuses => s.tr.reject_adds(None),
        }
        s.cycle_later().await;
        let recovered = s.item_containing("LIAR GAME - 26").await;
        assert_eq!(recovered.result, HistoryResult::Received, "{trouble:?}");
        assert_eq!(recovered.reason, None, "{trouble:?}");
        assert_eq!(recovered.first_seen_at, 1_000_000, "{trouble:?}");
        let changes = s.ctx.history.changes(recovered.id).await.unwrap();
        assert_eq!(changes.len(), 1, "{trouble:?}");
        assert_eq!(changes[0].from, HistoryResult::AddFailed, "{trouble:?}");

        // Transmission failing later does not turn a received item back into a failure.
        match trouble {
            Trouble::Stopped => s.tr.stop().await,
            Trouble::Refuses => {
                s.tr.reject_adds(Some("download directory path is not absolute"))
            }
        }
        s.cycle_later().await;
        assert_eq!(
            s.item_containing("LIAR GAME - 26").await.result,
            HistoryResult::Received,
            "{trouble:?}"
        );
    }
}
