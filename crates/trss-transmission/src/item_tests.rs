//! What [`add_item`] does with a torrent Transmission already has, and which
//! torrents [`remove_stale`] takes out, against the fake Transmission. The
//! rules are the crate's (ADR 0015); the cycle that calls them is tested in
//! trss-collect.

use std::path::Path;

use serde_json::json;

use crate::{
    add_item, client,
    fake::{FakeTorrent, FakeTransmission},
    http_client, item_label, remove_stale, AddKind, AddLabels, Redactor, BOT_LABEL,
};

const ITEM: &str = "trss-item:channel:guid:ab12";

fn hash(n: u32) -> String {
    format!("aaaa{n:036}")
}

fn link(hash: &str) -> String {
    format!("magnet:?xt=urn:btih:{hash}&dn=Show.mkv")
}

async fn connect(fake: &FakeTransmission) -> transmission_rpc::TransClient {
    let http = http_client(std::time::Duration::from_secs(5)).unwrap();
    client(fake.url().parse().unwrap(), &http)
}

#[tokio::test]
async fn an_add_of_a_torrent_already_there_puts_the_item_label_only_on_a_bot_torrent() {
    let fake = FakeTransmission::start().await;
    // A bot torrent from before the item labels (the legacy cron's), one a
    // person added, and one a person added that is seeding.
    fake.preload(FakeTorrent::new(&hash(1), "Bot.mkv").bot());
    fake.preload(FakeTorrent::new(&hash(2), "Mine.mkv"));
    fake.preload(FakeTorrent::new(&hash(3), "Mine seeding.mkv").status(6));
    let mut transmission = connect(&fake).await;

    for (n, labelled) in [(1, true), (2, false), (3, false)] {
        let added = add_item(
            &mut transmission,
            &link(&hash(n)),
            Path::new("/media/show"),
            AddLabels {
                item: Some(ITEM),
                command: Some("trss-cmd:c1"),
            },
            &Redactor::none(),
        )
        .await
        .unwrap();

        assert_eq!(added.kind, AddKind::Duplicate, "{n}");
        let labels = fake.torrent(&hash(n)).labels;
        assert_eq!(
            labels.contains(&ITEM.to_owned()),
            labelled,
            "{n}: {labels:?}"
        );
        assert!(
            !labels.contains(&"trss-cmd:c1".to_owned()),
            "{n}: the command label is never put on a torrent that was there: {labels:?}"
        );
    }
    assert_eq!(
        fake.torrent(&hash(1)).labels,
        [BOT_LABEL.to_owned(), ITEM.to_owned()],
        "the bot torrent keeps its labels"
    );
    assert!(
        fake.torrent(&hash(2)).labels.is_empty(),
        "a person's torrent keeps its labels"
    );
    assert_eq!(item_label("channel", "guid:ab12"), ITEM);
}

#[tokio::test]
async fn an_add_of_a_torrent_already_there_stops_it_only_when_it_is_a_finished_bot_torrent() {
    // (what, the torrent as Transmission has it, whether the add stops it)
    let cases = [
        (
            "a bot torrent seeding",
            FakeTorrent::new(&hash(1), "T").bot().status(6),
            true,
        ),
        (
            "a bot torrent queued to seed",
            FakeTorrent::new(&hash(1), "T").bot().status(5),
            true,
        ),
        (
            "a bot torrent still downloading",
            FakeTorrent::new(&hash(1), "T").bot(),
            false,
        ),
        (
            "a bot torrent stopped",
            FakeTorrent::new(&hash(1), "T").bot().status(0),
            false,
        ),
        (
            "a person's torrent seeding",
            FakeTorrent::new(&hash(1), "T").status(6),
            false,
        ),
    ];
    for (what, torrent, stopped) in cases {
        let status_before = torrent.status;
        let fake = FakeTransmission::start().await;
        fake.preload(torrent);
        let mut transmission = connect(&fake).await;

        add_item(
            &mut transmission,
            &link(&hash(1)),
            Path::new("/media/show"),
            AddLabels::default(),
            &Redactor::none(),
        )
        .await
        .unwrap();

        let stops = fake.calls_of("torrent-stop");
        if stopped {
            assert_eq!(stops.len(), 1, "{what}");
            assert_eq!(stops[0].args["ids"], json!([hash(1)]), "{what}");
            assert_eq!(fake.torrent(&hash(1)).status, 0, "{what}");
        } else {
            assert!(stops.is_empty(), "{what}");
            assert_eq!(fake.torrent(&hash(1)).status, status_before, "{what}");
        }
    }
}

#[tokio::test]
async fn a_cleanup_removes_only_finished_bot_torrents_whose_item_is_gone_and_keeps_their_data() {
    let fake = FakeTransmission::start().await;
    fake.preload(FakeTorrent::new(&hash(1), "Gone.mkv").bot());
    fake.preload(FakeTorrent::new(&hash(2), "Kept.mkv").bot());
    fake.preload(FakeTorrent::new(&hash(3), "Mine.mkv"));
    fake.preload(FakeTorrent::new(&hash(4), "Stalled.mkv").bot().unfinished());
    let mut transmission = connect(&fake).await;
    let kept = hash(2);

    let removed = remove_stale(
        &mut transmission,
        |hash, labels| {
            assert!(labels.contains(&BOT_LABEL.to_owned()), "asked about {hash}");
            hash == kept
        },
        &Redactor::none(),
    )
    .await;

    // A person's torrent and one still downloading are not even asked about;
    // the one whose item is in the feeds stays.
    assert_eq!(
        removed.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["Gone.mkv"]
    );
    let removes = fake.calls_of("torrent-remove");
    assert_eq!(removes.len(), 1);
    assert_eq!(removes[0].args["ids"], json!([hash(1)]));
    assert_eq!(removes[0].args["delete-local-data"], false, "data is kept");
    let mut left: Vec<String> = fake.torrents().into_iter().map(|t| t.hash).collect();
    left.sort();
    assert_eq!(left, [hash(2), hash(3), hash(4)]);

    // The stalled one goes once it has finished.
    fake.finish(&hash(4));
    let removed = remove_stale(&mut transmission, |hash, _| hash == kept, &Redactor::none()).await;
    assert_eq!(
        removed.iter().map(|t| t.hash.as_str()).collect::<Vec<_>>(),
        [hash(4)]
    );
}
