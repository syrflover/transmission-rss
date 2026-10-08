//! What only a process shows of replacing a video with a higher revision of
//! the same release (ticket 0025, `docs/specs/collection.md` 영상 수정본의
//! 대체). The replacement's own rules (what a cycle decides, how each step
//! advances, what `다시 받기` does, and how the web shows a failure) are tested
//! in trss-collect and trss-web (ADR 0015); this file keeps what the worker's
//! concurrent item tasks do, against the fake Transmission acting on a
//! temporary media folder (see [`FakeTransmission::on_disk`]).
//!
//! Release names, hashes and contents are made up; a name's CRC32 is the CRC32
//! of the bytes its torrent writes.

use crate::common;

use common::*;
use tokio_util::sync::CancellationToken;
use trss_collect::store::channels::{ChannelInput, RuleInput};
use trss_core::settings::SettingsStore;
use trss_worker::{CycleReport, TickOutcome};

/// The CRC32 of `bytes` as a release name writes it.
fn crc(bytes: &[u8]) -> String {
    format!("{:08X}", crc32fast::hash(bytes))
}

fn magnet(hash: &str, name: &str) -> String {
    let dn: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
    format!("magnet:?xt=urn:btih:{hash}&dn={dn}")
}

/// A feed of owned `(hash, title)` items.
fn feed_of(items: &[(String, String)]) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>Show</title><link>https://feeds.example.test/show</link><description>made up</description>"#,
    );
    for (hash, title) in items {
        let link = magnet(hash, title).replace('&', "&amp;");
        xml.push_str(&format!(
            "<item><title>{title}</title><link>{link}</link><guid isPermaLink=\"false\">guid-{hash}</guid></item>"
        ));
    }
    xml.push_str("</channel></rss>");
    xml
}

/// `[SubsPlease] Show - <episode><version> (1080p) [<crc>].mkv`.
fn episode_release(episode: u32, version: &str, bytes: &[u8]) -> String {
    format!(
        "[SubsPlease] Show - {episode}{version} (1080p) [{}].mkv",
        crc(bytes)
    )
}

/// A harness whose collect folder is a temporary `media` folder the fake
/// Transmission acts on, with one channel (`show` feed) and its rule saving to
/// `Show/Season 01`.
struct Setup {
    h: Harness,
    season: std::path::PathBuf,
}

impl Setup {
    async fn new() -> Setup {
        let h = Harness::without_collect_folder().await;
        let media = h.dir.path().join("media");
        let season = media.join("Show").join("Season 01");
        std::fs::create_dir_all(&season).unwrap();
        SettingsStore::new(h.db.clone())
            .put_collection(0, media.to_str().unwrap().to_owned(), None)
            .await
            .unwrap();
        h.tr.on_disk(&media);
        let url = format!("{}?token={SECRET}", h.feeds.url("show"));
        h.channels
            .create_channel_with_rules(
                ChannelInput::new(url),
                vec![RuleInput {
                    r#match: Some("[SubsPlease] Show - ".to_owned()),
                    directory: "Show/Season 01".to_owned(),
                    ..Default::default()
                }],
            )
            .await
            .unwrap();
        Setup { h, season }
    }

    /// The names in the season folder.
    fn names(&self) -> Vec<String> {
        std::fs::read_dir(&self.season)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect()
    }

    async fn cycle(&self) -> CycleReport {
        match self
            .h
            .worker()
            .tick(&CancellationToken::new())
            .await
            .unwrap()
        {
            TickOutcome::Ran(report) => report,
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    /// Transmission has all of the torrent `hash`.
    fn complete(&self, hash: &str) {
        self.h.tr.finish(hash);
        self.h.tr.set_status(hash, 6);
    }

    fn added(&self, hash: &str) -> usize {
        self.h
            .tr
            .calls_of("torrent-add")
            .iter()
            .filter(|c| c.args["filename"].as_str().unwrap().contains(hash))
            .count()
    }
}

/// Transmission's whole file list grows with everything it holds; a cycle
/// asks for it at most once, and not at all for revisions whose own torrent
/// holds the episode name.
#[tokio::test]
async fn a_cycle_reads_transmissions_whole_file_list_at_most_once() {
    let episodes = [11_u32, 12, 13];
    let release_of = |prefix: &str, episode: u32, version: &str| {
        let bytes = format!("episode {episode}, {version}");
        (
            format!("{prefix}{episode:0>36}"),
            episode_release(episode, version, bytes.as_bytes()),
            bytes,
        )
    };
    let firsts: Vec<_> = episodes
        .iter()
        .map(|&e| release_of("1111", e, ""))
        .collect();
    let seconds: Vec<_> = episodes
        .iter()
        .map(|&e| release_of("2222", e, "v2"))
        .collect();
    let items = |of: &[&Vec<(String, String, String)>]| -> Vec<(String, String)> {
        of.iter()
            .flat_map(|v| v.iter().map(|(h, t, _)| (h.clone(), t.clone())))
            .collect()
    };

    let s = Setup::new().await;
    for (hash, _, bytes) in firsts.iter().chain(&seconds) {
        s.h.tr.content_on_add(hash, bytes.as_bytes());
    }
    for (hash, _, _) in &seconds {
        s.h.tr.unfinished_on_add(hash);
    }
    s.h.feeds.set_xml("show", &feed_of(&items(&[&firsts])));
    s.cycle().await;
    for (hash, _, _) in &firsts {
        s.complete(hash);
    }
    assert_eq!(s.names().len(), 3);

    // Three revisions to decide in one cycle.
    s.h.feeds
        .set_xml("show", &feed_of(&items(&[&seconds, &firsts])));
    s.h.tr.clear_calls();
    s.cycle().await;
    for (hash, _, _) in &seconds {
        assert_eq!(s.added(hash), 1);
    }
    let listings = s.h.tr.full_file_listings();
    assert!(listings <= 1, "{listings} full listings in one cycle");

    // Steady state: revisions received while their episode had no video hold
    // the name with their own torrent.
    let s = Setup::new().await;
    for (hash, _, bytes) in &seconds {
        s.h.tr.content_on_add(hash, bytes.as_bytes());
    }
    s.h.feeds.set_xml("show", &feed_of(&items(&[&seconds])));
    s.cycle().await;
    for (hash, _, _) in &seconds {
        s.complete(hash);
    }
    assert_eq!(s.names().len(), 3);
    s.h.tr.clear_calls();
    s.cycle().await;
    assert_eq!(s.h.tr.full_file_listings(), 0);
}
