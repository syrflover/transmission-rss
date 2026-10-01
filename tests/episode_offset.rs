//! The episode offset of a new season's subscription (ticket 0024), end to end:
//! the real worker cycle, a real library scan of temporary folders, AniList
//! entries linked to the earlier seasons, the rules HTTP API, a fake RSS feed
//! and a fake Transmission that renames the files.
//!
//! The offset is decided from the first release a subscription picks, before
//! that item is named; what the app does not decide is left to the user.

mod common;

use std::fs;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use transmission_rss::{
    store::{
        anissia::Anime,
        channels::{ChannelInput, NewSubscription, Rule, RuleInput, SubtitleMode},
        library::LibraryStore,
        seasons::{Entry, FuzzyDate, SeasonStore},
    },
    worker::{CommandsOutcome, TickOutcome},
};

const FEED: &str = "feed-offset";
const CMD: &str = "7c1f0e0e-0a70-4c1e-8f6b-7d0c2a9b3e11";

fn hash(n: u32) -> String {
    format!("dddd{n:036}")
}

struct Release {
    guid: String,
    title: String,
    link: String,
}

/// Episode `n` of `Show`, released by SubsPlease.
fn show(n: u32) -> Release {
    release(
        n,
        &format!("[SubsPlease] Show - {n:02} (1080p) [ABCD{n:04}].mkv"),
    )
}

/// Episode `n` of `Fresh`, a work the library does not have.
fn fresh(n: u32) -> Release {
    release(
        500 + n,
        &format!("[SubsPlease] Fresh - {n:02} (1080p) [EFGH{n:04}].mkv"),
    )
}

/// A release of another work, which a channel read once holds already.
fn other() -> Release {
    release(900, "[SubsPlease] Other - 01 (1080p) [ZZZZ0001].mkv")
}

fn release(n: u32, title: &str) -> Release {
    let dn: String = url::form_urlencoded::byte_serialize(title.as_bytes()).collect();
    Release {
        guid: format!("guid-{n}"),
        title: title.to_owned(),
        link: format!("magnet:?xt=urn:btih:{}&dn={dn}", hash(n)),
    }
}

fn feed_xml(releases: &[&Release]) -> String {
    let items: String = releases
        .iter()
        .map(|r| {
            format!(
                "<item><title>{}</title><link>{}</link><guid isPermaLink=\"false\">{}</guid></item>",
                r.title.replace('&', "&amp;"),
                r.link.replace('&', "&amp;"),
                r.guid
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>x</title>
        <link>http://x/</link><description>d</description>{items}</channel></rss>"#
    )
}

fn entry(id: i64, episodes: Option<u32>) -> Entry {
    Entry {
        id,
        romaji: Some(format!("Show {id}")),
        english: None,
        native: None,
        format: Some("TV".into()),
        status: Some("FINISHED".into()),
        episodes,
        start: FuzzyDate::default(),
        end: FuzzyDate::default(),
        studios: Vec::new(),
        genres: Vec::new(),
        description: None,
        airing: Vec::new(),
        sequels: Vec::new(),
        fetched_at: 1,
    }
}

struct Scene {
    h: Harness,
    api: WebApi,
    /// The collect folder, which is also the library's watch folder.
    shows: std::path::PathBuf,
    channel: String,
    library: LibraryStore,
}

impl Scene {
    /// A collect folder holding `Show` with seasons 1 and 2 (two videos each),
    /// registered as a watch folder, and an empty feed.
    async fn new() -> Scene {
        let h = Harness::without_collect_folder().await;
        let shows = h.dir.path().join("Shows");
        for (season, episodes) in [(1, [1, 2]), (2, [1, 2])] {
            for episode in episodes {
                let dir = shows.join(format!("Show/Season {season:02}"));
                fs::create_dir_all(&dir).unwrap();
                fs::write(dir.join(format!("Show S{season:02}E{episode:02}.mkv")), "x").unwrap();
            }
        }
        fs::create_dir_all(shows.join("Fresh")).unwrap();
        set_collect_folder(&h, &shows).await;
        h.feeds.set_xml(FEED, &feed_xml(&[]));
        let channel = h
            .channels
            .create_channel_with_rules(
                ChannelInput::new(format!("{}?filter=1080p&token={SECRET}", h.feeds.url(FEED))),
                Vec::new(),
            )
            .await
            .unwrap()
            .channel
            .id;
        let scene = Scene {
            api: h.web_api(),
            library: LibraryStore::new(h.db.clone()),
            shows,
            channel,
            h,
        };
        let (status, text, _) = scene
            .api
            .call(
                "POST",
                "/api/library/watch-folders",
                Some(json!({ "path": scene.shows.to_str().unwrap() })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{text}");
        scene
    }

    /// Links AniList entries to seasons 1 and 2 of `Show` with these counts.
    async fn link_earlier_seasons(&self, counts: [Option<u32>; 2]) {
        let folder = self.library.folders().await.unwrap().remove(0);
        let work = self
            .library
            .works(&folder.id)
            .await
            .unwrap()
            .into_iter()
            .find(|w| w.dir_name == "Show")
            .unwrap()
            .id;
        let seasons = SeasonStore::new(self.h.db.clone());
        for (index, count) in counts.into_iter().enumerate() {
            let season = index as u32 + 1;
            let id = 100 + i64::from(season);
            seasons.put_entry(entry(id, count)).await.unwrap();
            let link = seasons.link(&work, season).await.unwrap();
            seasons
                .set_links(&work, season, link.version, vec![id])
                .await
                .unwrap();
        }
    }

    /// A subscription to the anime `no` saving into `directory`.
    async fn subscribe(&self, phrase: &str, directory: &str, no: i64, episode: i64) -> Rule {
        self.h
            .channels
            .create_subscription_rule(
                &self.channel,
                RuleInput {
                    r#match: Some(phrase.to_owned()),
                    directory: directory.to_owned(),
                    episode,
                    ..Default::default()
                },
                NewSubscription {
                    anime: Anime {
                        anime_no: no,
                        subject: phrase.to_owned(),
                        original_subject: None,
                        week: 4,
                        air_time: Some("23:00".to_owned()),
                        start_date: Some("2026-10-08".to_owned()),
                        end_date: None,
                        status: "ON".to_owned(),
                        fetched_at: self.h.now(),
                    },
                    subtitles: SubtitleMode::Undecided,
                    creator: None,
                    subscribed_at: self.h.now(),
                },
            )
            .await
            .unwrap()
    }

    /// The feed serves `releases` after a release of another work: a channel
    /// read once holds something already, so that what comes later is not what
    /// the feed held at the first read.
    fn feed(&self, releases: &[&Release]) {
        let other = other();
        let all: Vec<&Release> = std::iter::once(&other)
            .chain(releases.iter().copied())
            .collect();
        self.h.feeds.set_xml(FEED, &feed_xml(&all));
    }

    async fn cycle(&self) {
        self.h.advance(300_000);
        match self
            .h
            .worker()
            .tick(&CancellationToken::new())
            .await
            .unwrap()
        {
            TickOutcome::Ran(_) => {}
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    async fn rule(&self, rule: &Rule) -> Rule {
        self.h.channels.get_rule(&rule.id).await.unwrap().unwrap()
    }

    async fn view(&self, rule: &Rule) -> Value {
        let (status, _, view) = self
            .api
            .call("GET", &format!("/api/rules/{}", rule.id), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        view
    }

    /// The names Transmission's torrents have now, sorted.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.h.tr.torrents().into_iter().map(|t| t.name).collect();
        names.sort();
        names
    }

    /// The user picks the item with `part` for the rule, and the worker runs it.
    async fn receive(&self, part: &str, rule: &Rule) {
        let item = self.h.item(part).await;
        let (status, _, body) = self
            .api
            .call(
                "POST",
                "/api/commands",
                Some(json!({
                    "id": CMD,
                    "kind": "receive_once",
                    "payload": { "item_id": item.id, "rule_id": rule.id },
                })),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        assert_eq!(
            self.h
                .worker()
                .run_commands(&CancellationToken::new())
                .await
                .unwrap(),
            CommandsOutcome::Ran(1)
        );
    }
}

/// What the rule detail sends to save the rule with this offset.
async fn save(s: &Scene, rule: &Rule, version: &Value, episode: i64) -> (StatusCode, Value) {
    let body = json!({
        "version": version,
        "channel_id": s.channel,
        "match": "Show",
        "directory": "Show/Season 03",
        "episode": episode,
    });
    let (status, _, saved) = s
        .api
        .call("PUT", &format!("/api/rules/{}", rule.id), Some(body))
        .await;
    (status, saved)
}

/// Sets the collect folder (a real folder, which the library scans).
async fn set_collect_folder(h: &Harness, shows: &std::path::Path) {
    transmission_rss::store::settings::SettingsStore::new(h.db.clone())
        .put_collection(0, shows.to_str().unwrap().to_owned(), None)
        .await
        .unwrap();
}

#[tokio::test]
async fn the_first_item_of_a_third_season_is_named_from_the_sum_of_the_earlier_ones() {
    let s = Scene::new().await;
    s.link_earlier_seasons([Some(12), Some(12)]).await;
    s.h.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;

    // The first read leaves what the feed held; the new season's first
    // release comes after it.
    s.feed(&[&show(24)]);
    s.cycle().await;
    assert!(s.names().is_empty());
    s.feed(&[&show(24), &show(25)]);
    s.cycle().await;

    assert_eq!(s.names(), ["Show S03E01.mkv"]);
    let stored = s.rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (-24, true));
    let view = s.view(&rule).await;
    assert_eq!(view["episode"], -24);
    assert_eq!(view["episode_auto"], true);
    let basis = view["episode_basis"].as_str().unwrap();
    assert!(basis.contains("24화") && basis.contains("25화"), "{basis}");
    assert_eq!(view["episode_suggestion"], Value::Null);

    // The decision is made once: the next release is converted by the same
    // value, and the item already received keeps its name.
    s.feed(&[&show(24), &show(25), &show(26)]);
    s.cycle().await;
    assert_eq!(s.names(), ["Show S03E01.mkv", "Show S03E02.mkv"]);
    assert_eq!(s.rule(&rule).await.episode, -24);
}

#[tokio::test]
async fn a_new_works_first_release_is_not_converted_and_nobody_is_asked() {
    let s = Scene::new().await;
    s.h.advance(1_000);
    let rule = s.subscribe("Fresh", "Fresh/Season 01", 8, 1).await;

    s.feed(&[]);
    s.cycle().await;
    s.feed(&[&fresh(1)]);
    s.cycle().await;

    assert_eq!(s.names(), ["Fresh S01E01.mkv"]);
    let stored = s.rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (0, true));
    let view = s.view(&rule).await;
    assert!(view["episode_basis"].as_str().unwrap().contains("1화"));
    assert_eq!(view["episode_suggestion"], Value::Null);
}

#[tokio::test]
async fn a_first_release_in_the_middle_of_a_season_is_suggested_and_received_unconverted() {
    let s = Scene::new().await;
    s.link_earlier_seasons([Some(12), Some(12)]).await;
    s.h.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;

    s.feed(&[]);
    s.cycle().await;
    s.feed(&[&show(27)]);
    s.cycle().await;

    // Received as it is, with nothing decided.
    assert_eq!(s.names(), ["Show S03E27.mkv"]);
    let stored = s.rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (1, false));
    let view = s.view(&rule).await;
    assert_eq!(view["episode_basis"], Value::Null);
    assert_eq!(view["episode_suggestion"]["value"], -24);
    let basis = view["episode_suggestion"]["basis"].as_str().unwrap();
    assert!(basis.contains("27화") && basis.contains("24화"), "{basis}");

    // The next release is still received as it is until the user applies it.
    s.feed(&[&show(27), &show(28)]);
    s.cycle().await;
    assert_eq!(s.names(), ["Show S03E27.mkv", "Show S03E28.mkv"]);
    assert_eq!(s.rule(&rule).await.episode, 1);

    // `적용` saves the value as the user's own.
    let (status, _, applied) = s
        .api
        .call(
            "PUT",
            &format!("/api/rules/{}/episode", rule.id),
            Some(json!({ "version": view["version"], "episode": -24 })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    assert_eq!(applied["episode"], -24);
    assert_eq!(applied["episode_auto"], false);
    assert_eq!(applied["episode_suggestion"], Value::Null);
    s.feed(&[&show(27), &show(28), &show(29)]);
    s.cycle().await;
    assert!(
        s.names().contains(&"Show S03E05.mkv".to_owned()),
        "{:?}",
        s.names()
    );

    // A stale version is a conflict that carries the current rule.
    let (status, _, conflict) = s
        .api
        .call(
            "PUT",
            &format!("/api/rules/{}/episode", rule.id),
            Some(json!({ "version": view["version"], "episode": -12 })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["current"]["episode"], -24);
}

#[tokio::test]
async fn earlier_seasons_without_a_known_count_are_never_guessed() {
    let s = Scene::new().await;
    // Season 2's entry does not know its episode count.
    s.link_earlier_seasons([Some(12), None]).await;
    s.h.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;

    s.feed(&[]);
    s.cycle().await;
    s.feed(&[&show(25)]);
    s.cycle().await;

    assert_eq!(s.names(), ["Show S03E25.mkv"]);
    let stored = s.rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (1, false));
    let view = s.view(&rule).await;
    assert_eq!(view["episode_suggestion"]["value"], Value::Null);
    let basis = view["episode_suggestion"]["basis"].as_str().unwrap();
    assert!(basis.contains("시즌 2의 AniList 회차 수"), "{basis}");
}

#[tokio::test]
async fn a_season_folder_that_has_videos_already_keeps_the_app_from_choosing() {
    let s = Scene::new().await;
    s.link_earlier_seasons([Some(12), Some(12)]).await;
    // A split cour: the first cour's episodes are in the third season already.
    let dir = s.shows.join("Show/Season 03");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Show S03E01.mkv"), "x").unwrap();
    fs::write(dir.join("Show S03E02.mkv"), "x").unwrap();
    s.cycle().await; // reads the folder into the library
    s.h.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;

    s.feed(&[]);
    s.cycle().await;
    s.feed(&[&show(25)]);
    s.cycle().await;

    assert_eq!(s.names(), ["Show S03E25.mkv"]);
    assert!(!s.rule(&rule).await.episode_auto);
    let view = s.view(&rule).await;
    assert_eq!(view["episode_suggestion"]["value"], Value::Null);
    assert!(view["episode_suggestion"]["basis"]
        .as_str()
        .unwrap()
        .contains("1–2화"));
}

#[tokio::test]
async fn an_offset_the_user_typed_is_never_overwritten() {
    let s = Scene::new().await;
    s.link_earlier_seasons([Some(12), Some(12)]).await;
    s.h.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, -12).await;

    s.feed(&[]);
    s.cycle().await;
    s.feed(&[&show(25)]);
    s.cycle().await;

    // The user's -12 names the release as the user said.
    assert_eq!(s.names(), ["Show S03E13.mkv"]);
    let stored = s.rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (-12, false));
    assert_eq!(s.view(&rule).await["episode_suggestion"], Value::Null);
}

#[tokio::test]
async fn an_automatic_value_the_user_changes_loses_its_mark_and_stays_changed() {
    let s = Scene::new().await;
    s.link_earlier_seasons([Some(12), Some(12)]).await;
    s.h.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;
    s.feed(&[]);
    s.cycle().await;
    s.feed(&[&show(25)]);
    s.cycle().await;
    let auto = s.view(&rule).await;
    assert_eq!(
        (auto["episode"].clone(), auto["episode_auto"].clone()),
        (json!(-24), json!(true))
    );

    // The rule detail saves the whole rule. Saving the value as it is keeps
    // the mark and the grounds ...
    let (status, kept) = save(&s, &rule, &auto["version"], -24).await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["episode_auto"], true);
    assert_eq!(kept["episode_basis"], auto["episode_basis"]);

    // ... and a changed value is the user's.
    let (status, saved) = save(&s, &rule, &kept["version"], -12).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["episode"], -12);
    assert_eq!(saved["episode_auto"], false);
    assert_eq!(saved["episode_basis"], Value::Null);

    s.feed(&[&show(25), &show(26)]);
    s.cycle().await;
    let stored = s.rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (-12, false));
    assert!(
        s.names().contains(&"Show S03E14.mkv".to_owned()),
        "{:?}",
        s.names()
    );
}

#[tokio::test]
async fn a_rule_that_picked_items_before_is_not_decided_by_later_ones() {
    let s = Scene::new().await;
    s.link_earlier_seasons([Some(12), Some(12)]).await;
    s.h.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;
    s.feed(&[]);
    s.cycle().await;
    // The first thing it picks is episode 27: nothing is decided.
    s.feed(&[&show(27)]);
    s.cycle().await;
    // A later release that would have been a season's first changes nothing.
    s.feed(&[&show(27), &show(25)]);
    s.cycle().await;

    assert_eq!(s.names(), ["Show S03E25.mkv", "Show S03E27.mkv"]);
    let stored = s.rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (1, false));
}

#[tokio::test]
async fn a_past_item_the_user_picks_first_is_named_with_the_decided_offset() {
    let s = Scene::new().await;
    s.link_earlier_seasons([Some(12), Some(12)]).await;
    // The feed already holds the new season's first release: it is past.
    s.feed(&[&show(25)]);
    s.cycle().await;
    s.h.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;
    s.cycle().await;
    assert!(s.names().is_empty());

    s.receive("Show - 25", &rule).await;

    assert_eq!(s.names(), ["Show S03E01.mkv"]);
    let stored = s.rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (-24, true));
}

#[tokio::test]
async fn a_suggestion_that_cannot_be_read_leaves_the_rule_list_answering() {
    let s = Scene::new().await;
    s.link_earlier_seasons([Some(12), Some(12)]).await;
    s.h.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;
    s.feed(&[]);
    s.cycle().await;
    s.feed(&[&show(27)]);
    s.cycle().await;
    assert_eq!(s.view(&rule).await["episode_suggestion"]["value"], -24);

    // The AniList counts the suggestion needs cannot be read any more.
    s.h.db
        .run(|c| {
            c.execute_batch("ALTER TABLE season_info RENAME TO season_info_gone")
                .map_err(transmission_rss::store::db::DbError::from)
        })
        .await
        .unwrap();

    let view = s.view(&rule).await;
    assert_eq!(view["episode_suggestion"], Value::Null);
    let (status, text, _) = s
        .api
        .call("GET", &format!("/api/channels/{}", s.channel), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
}

/// Runs a cycle whose feed read waits until `meanwhile` is done, so that the
/// cycle judges the rules as they were before it.
async fn cycle_while<F: std::future::Future<Output = ()>>(s: &Scene, meanwhile: F) {
    s.h.advance(300_000);
    let gate = s.h.feeds.hold(FEED);
    let worker = s.h.worker();
    let tick = tokio::spawn(async move { worker.tick(&CancellationToken::new()).await });
    gate.wait_arrived().await;
    meanwhile.await;
    gate.release_one();
    match tick.await.unwrap().unwrap() {
        TickOutcome::Ran(_) => {}
        other => panic!("expected a cycle, got {other:?}"),
    }
}

#[tokio::test]
async fn a_rule_saved_while_its_first_release_is_read_still_gets_the_offset() {
    let s = Scene::new().await;
    s.link_earlier_seasons([Some(12), Some(12)]).await;
    s.h.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;
    s.feed(&[]);
    s.cycle().await;

    s.feed(&[&show(25)]);
    cycle_while(&s, async {
        // A save that changes nothing the decision rests on (the form saved
        // as it was); it still makes a new version.
        let now = s.rule(&rule).await;
        let saved =
            s.h.channels
                .update_rule(&now.id, now.version, &s.channel, now.to_input())
                .await
                .unwrap();
        assert_ne!(saved.version, now.version);
    })
    .await;

    assert_eq!(s.names(), ["Show S03E01.mkv"]);
    let stored = s.rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (-24, true));
}

#[tokio::test]
async fn an_offset_the_user_saves_while_the_first_release_is_read_is_kept() {
    let s = Scene::new().await;
    s.link_earlier_seasons([Some(12), Some(12)]).await;
    s.h.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;
    s.feed(&[]);
    s.cycle().await;

    s.feed(&[&show(25)]);
    cycle_while(&s, async {
        let now = s.rule(&rule).await;
        let mut input = now.to_input();
        input.episode = 0;
        s.h.channels
            .update_rule(&now.id, now.version, &s.channel, input)
            .await
            .unwrap();
    })
    .await;

    let stored = s.rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (0, false));
    assert_eq!(s.names(), ["Show S03E25.mkv"]);
}

#[tokio::test]
async fn a_rule_whose_match_changes_while_its_first_release_is_read_is_not_decided() {
    let s = Scene::new().await;
    s.link_earlier_seasons([Some(12), Some(12)]).await;
    s.h.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, 1).await;
    s.feed(&[]);
    s.cycle().await;

    s.feed(&[&show(25)]);
    cycle_while(&s, async {
        // The items of this cycle were picked by the phrase the cycle read.
        let now = s.rule(&rule).await;
        let mut input = now.to_input();
        input.r#match = Some("Show -".to_owned());
        s.h.channels
            .update_rule(&now.id, now.version, &s.channel, input)
            .await
            .unwrap();
    })
    .await;

    let stored = s.rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (1, false));
    assert_eq!(s.names(), ["Show S03E25.mkv"]);
}
