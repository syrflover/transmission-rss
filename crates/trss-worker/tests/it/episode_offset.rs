//! The episode offset of a new season's subscription (ticket 0024), end to end:
//! the real worker cycle, a real library scan of temporary folders, AniList
//! entries linked to the earlier seasons, the rules HTTP API, a fake RSS feed
//! and a fake Transmission that renames the files.
//!
//! The offset is decided from the first release a subscription picks, before
//! that item is named; what the app does not decide is left to the user.

use crate::common;

use std::fs;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use trss_anilist::{Entry, FuzzyDate};
use trss_anissia::Anime;
use trss_collect::store::channels::{ChannelInput, NewSubscription, Rule, RuleInput, SubtitleMode};
use trss_library::store::{library::LibraryStore, seasons::SeasonStore};
use trss_worker::{CommandsOutcome, TickOutcome};

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
        korean_titles: Vec::new(),
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

/// Sets the collect folder (a real folder, which the library scans).
async fn set_collect_folder(h: &Harness, shows: &std::path::Path) {
    trss_core::settings::SettingsStore::new(h.db.clone())
        .put_collection(0, shows.to_str().unwrap().to_owned(), None)
        .await
        .unwrap();
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

    // Before it is received, the rule detail offers what receiving it sets.
    let offer = &s.view(&rule).await["episode_suggestion"];
    assert_eq!(offer["value"], -24);
    let basis = offer["basis"].as_str().unwrap();
    assert!(
        basis.contains("가장 앞선 릴리스가 25화") && basis.contains("시즌 1화"),
        "{basis}"
    );

    s.receive("Show - 25", &rule).await;

    assert_eq!(s.names(), ["Show S03E01.mkv"]);
    let stored = s.rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (-24, true));
    assert_eq!(s.view(&rule).await["episode_suggestion"], Value::Null);
}

// --- `되돌리기` of an automatic offset (user decision, 2026-10-02) ---------------

impl Scene {
    /// The season 3 folder of `Show`.
    fn season3(&self) -> std::path::PathBuf {
        self.shows.join("Show/Season 03")
    }

    /// The names of the files in the season 3 folder, sorted.
    fn on_disk(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.season3())
            .map(|dir| {
                dir.map(|e| e.unwrap().file_name().into_string().unwrap())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// Seasons 1 and 2 of 24 episodes, a third season's rule carried over
    /// with `−24`, and its first releases `- 49` and `- 50` received under the
    /// app's `−48`. The fake Transmission writes and renames the files.
    async fn third_season_received() -> (Scene, Rule) {
        let s = Scene::new().await;
        s.h.tr.on_disk(&s.shows);
        for n in [49, 50, 51] {
            s.h.tr
                .content_on_add(&hash(n), format!("video {n}").as_bytes());
        }
        s.link_earlier_seasons([Some(24), Some(24)]).await;
        s.h.advance(1_000);
        let rule = s.subscribe("Show", "Show/Season 03", 7, -24).await;
        s.feed(&[]);
        s.cycle().await;
        s.feed(&[&show(49)]);
        s.cycle().await;
        s.feed(&[&show(49), &show(50)]);
        s.cycle().await;
        assert_eq!(s.names(), ["Show S03E01.mkv", "Show S03E02.mkv"]);
        assert_eq!(s.on_disk(), ["Show S03E01.mkv", "Show S03E02.mkv"]);
        s.seeding();
        (s, rule)
    }

    /// Asks for `되돌리기` of `episode`.
    async fn ask_undo(&self, rule: &Rule, id: &str, episode: i64) -> (StatusCode, Value) {
        let (status, _, body) = self
            .api
            .call(
                "POST",
                "/api/commands",
                Some(json!({
                    "id": id,
                    "kind": "episode_undo",
                    "payload": { "rule_id": rule.id, "episode": episode },
                })),
            )
            .await;
        (status, body)
    }

    /// Every torrent has been received and is seeding.
    fn seeding(&self) {
        for t in self.h.tr.torrents() {
            self.h.tr.set_status(&t.hash, 6);
        }
    }

    /// `되돌리기` of `episode`, run by the worker: the command as it ended.
    async fn undo(&self, rule: &Rule, id: &str, episode: i64) -> Value {
        let (status, body) = self.ask_undo(rule, id, episode).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        assert_eq!(
            self.h
                .worker()
                .run_commands(&CancellationToken::new())
                .await
                .unwrap(),
            CommandsOutcome::Ran(1)
        );
        let (status, _, command) = self
            .api
            .call("GET", &format!("/api/commands/{id}"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{command}");
        command
    }
}

/// The files of the rule's last undo: `(from, to, state)`.
fn undo_files(view: &Value) -> Vec<(String, String, String)> {
    view["episode_undo"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["from_name"].as_str().unwrap().to_owned(),
                f["to_name"].as_str().unwrap().to_owned(),
                f["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn file(from: &str, to: &str, state: &str) -> (String, String, String) {
    (from.to_owned(), to.to_owned(), state.to_owned())
}

// --- numbers run on from a later season (user decision, 2026-10-02) ---------------

// --- a split cour that restarts at `- 01` (user decision, 2026-10-02) ------------

impl Scene {
    /// Links these AniList entries (id, episodes), in order, to `season` of `Show`.
    async fn link_season(&self, season: u32, entries: &[(i64, Option<u32>)]) {
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
        for (id, count) in entries {
            seasons.put_entry(entry(*id, *count)).await.unwrap();
        }
        let link = seasons.link(&work, season).await.unwrap();
        seasons
            .set_links(
                &work,
                season,
                link.version,
                entries.iter().map(|(id, _)| *id).collect(),
            )
            .await
            .unwrap();
    }

    /// Season 2 of `Show` holds these episodes and is linked to cours of
    /// these counts; a subscription to its next cour, saving into it, picks
    /// `- 01` first. The rule as it is after that.
    async fn restarted_cour(held: &[u32], cours: &[u32]) -> (Scene, Rule) {
        let s = Scene::new().await;
        let dir = s.shows.join("Show/Season 02");
        for e in held {
            fs::write(dir.join(format!("Show S02E{e:02}.mkv")), "x").unwrap();
        }
        for e in [1, 2] {
            if !held.contains(&e) {
                fs::remove_file(dir.join(format!("Show S02E{e:02}.mkv"))).unwrap();
            }
        }
        s.cycle().await; // reads the folder into the library
        s.link_season(1, &[(101, Some(12))]).await;
        let entries: Vec<(i64, Option<u32>)> = cours
            .iter()
            .enumerate()
            .map(|(i, c)| (201 + i as i64, Some(*c)))
            .collect();
        s.link_season(2, &entries).await;
        s.h.advance(1_000);
        let rule = s.subscribe("Show", "Show/Season 02", 8, 0).await;
        s.feed(&[]);
        s.cycle().await;
        s.feed(&[&show(1)]);
        s.cycle().await;
        (s, rule)
    }
}

#[tokio::test]
async fn a_second_cour_that_restarts_at_one_is_offered_a_start_and_never_given_one() {
    let (s, rule) = Scene::restarted_cour(&(1..=12).collect::<Vec<_>>(), &[12, 12]).await;

    // Never set by the app: received as it is. `S02E01` is the first cour's,
    // so the video keeps its release name.
    let first = "[SubsPlease] Show - 01 (1080p) [ABCD0001].mkv";
    assert_eq!(s.names(), [first]);
    let stored = s.rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (0, false));
    let view = s.view(&rule).await;
    assert_eq!(view["episode_basis"], Value::Null);
    assert_eq!(
        view["episode_suggestion"],
        json!({
            "value": 13,
            "basis": "2쿨을 1화부터 센 번호로 보여요. 1화를 13화로 받도록 회차 변환을 13으로 할까요?",
        })
    );

    // `적용`: the cour's releases are named on from the first cour's twelve.
    let (status, _, applied) = s
        .api
        .call(
            "PUT",
            &format!("/api/rules/{}/episode", rule.id),
            Some(json!({ "version": view["version"], "episode": 13 })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    assert_eq!(applied["episode"], 13);
    assert_eq!(applied["episode_auto"], false);
    s.feed(&[&show(1), &show(2)]);
    s.cycle().await;
    // `- 01`, which the feed still shows and which kept its release name, is
    // named now too: a rename an earlier cycle could not make is finished.
    assert_eq!(s.names(), ["Show S02E13.mkv", "Show S02E14.mkv"]);
}

#[tokio::test]
async fn a_restart_without_every_episode_of_the_first_cour_is_offered_nothing() {
    let missing_7: Vec<u32> = (1..=12).filter(|e| *e != 7).collect();
    let (s, rule) = Scene::restarted_cour(&missing_7, &[12, 12]).await;
    assert_eq!(s.view(&rule).await["episode_suggestion"], Value::Null);
    assert_eq!(s.rule(&rule).await.episode, 0);

    // One entry linked to the season: no cours to tell apart.
    let (s, rule) = Scene::restarted_cour(&(1..=12).collect::<Vec<_>>(), &[24]).await;
    assert_eq!(s.view(&rule).await["episode_suggestion"], Value::Null);
    assert_eq!(s.rule(&rule).await.episode, 0);
}

/// A rule that picked items before the grounds were known keeps its value
/// (the app sets nothing after the first item), but the suggestion is shown
/// whatever the field holds once it differs (user direction, 2026-10-02).
#[tokio::test]
async fn a_rule_that_started_with_a_carried_over_value_is_offered_the_sum() {
    let s = Scene::new().await;
    s.h.advance(1_000);
    // Season 3's rule copied from season 2's, which held −12.
    let rule = s.subscribe("Show", "Show/Season 03", 7, -12).await;
    s.feed(&[]);
    s.cycle().await;
    // The earlier seasons are not linked yet: nothing is decided.
    s.feed(&[&show(25)]);
    s.cycle().await;
    assert_eq!(s.names(), ["Show S03E13.mkv"]);
    let stored = s.rule(&rule).await;
    assert_eq!((stored.episode, stored.episode_auto), (-12, false));
    // A note without a value says nothing to a field the user filled.
    assert_eq!(s.view(&rule).await["episode_suggestion"], Value::Null);

    // Linked later: the sum says −24, which the field does not hold.
    s.link_earlier_seasons([Some(12), Some(12)]).await;
    let view = s.view(&rule).await;
    assert_eq!(view["episode_suggestion"]["value"], -24, "{view}");
    assert_eq!(s.rule(&rule).await.episode, -12);

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
    assert_eq!(applied["episode_suggestion"], Value::Null);
    s.feed(&[&show(25), &show(26)]);
    s.cycle().await;
    assert!(
        s.names().contains(&"Show S03E02.mkv".to_owned()),
        "{:?}",
        s.names()
    );
}

/// A field that already holds what the grounds say is offered nothing.
#[tokio::test]
async fn a_rule_whose_field_already_holds_the_suggestion_is_offered_nothing() {
    let s = Scene::new().await;
    s.h.advance(1_000);
    let rule = s.subscribe("Show", "Show/Season 03", 7, -24).await;
    s.feed(&[]);
    s.cycle().await;
    s.feed(&[&show(27)]);
    s.cycle().await;
    s.link_earlier_seasons([Some(12), Some(12)]).await;
    assert_eq!(s.view(&rule).await["episode_suggestion"], Value::Null);
}

// --- revision rows checked file by file -----------------------------------------

impl Scene {
    /// A video revision row of `episode_name` in the season 3 folder, for the
    /// item whose title has `part`.
    async fn revision_row(
        &self,
        rule: &Rule,
        part: &str,
        episode_name: &str,
        receiving: bool,
    ) -> i64 {
        use trss_collect::store::revisions::{NewRevision, RevisionState, RevisionStore};
        let item = self.h.item(part).await;
        RevisionStore::new(self.h.db.clone())
            .create(
                self.h.now(),
                NewRevision {
                    item_id: item.id,
                    old_item_id: None,
                    rule_id: rule.id.clone(),
                    folder: self.season3().to_str().unwrap().to_owned(),
                    episode_name: episode_name.into(),
                    old_version: Some(1),
                    new_version: 2,
                    old_crc: None,
                    expected_crc: None,
                    torrent_hash: None,
                    state: if receiving {
                        RevisionState::Receiving
                    } else {
                        RevisionState::Skipped
                    },
                    reason: None,
                },
            )
            .await
            .unwrap()
            .id
    }
}

/// A start cut short after Transmission renamed the first file carries on;
/// a replacement that began between the starts keeps its episode's file.
#[tokio::test]
async fn a_start_cut_short_carries_on_and_checks_each_file_again() {
    let (s, rule) = Scene::third_season_received().await;
    let (status, body) = s.ask_undo(&rule, "undo-0201-a", -48).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let gate = s.h.tr.hold_answer("torrent-rename-path");
    let worker = s.h.worker();
    let task = tokio::spawn(async move { worker.run_commands(&CancellationToken::new()).await });
    gate.wait_arrived().await;
    // The worker stops while Transmission renames the first file.
    task.abort();
    let _ = task.await;
    assert_eq!(s.names(), ["Show S03E02.mkv", "Show S03E25.mkv"]);
    assert_eq!(s.rule(&rule).await.episode, -24);

    // A cycle between the starts began replacing `S03E02` with a revision.
    s.revision_row(&rule, "Show - 50", "Show S03E02.mkv", true)
        .await;
    gate.release_all();
    s.h.wait_lock_free().await;
    assert_eq!(
        s.h.worker()
            .run_commands(&CancellationToken::new())
            .await
            .unwrap(),
        CommandsOutcome::Ran(1)
    );

    let view = s.view(&rule).await;
    assert_eq!(view["episode_undo"]["command"]["state"], "done");
    assert_eq!(
        view["episode_undo"]["command"]["outcome"]["result"],
        "paused"
    );
    // The replacement's file waits for it to end.
    assert_eq!(
        undo_files(&view),
        [
            file("Show S03E01.mkv", "Show S03E25.mkv", "renamed"),
            file("Show S03E02.mkv", "Show S03E26.mkv", "pending"),
        ]
    );
    let reason = view["episode_undo"]["files"][1]["reason"].as_str().unwrap();
    assert!(reason.contains("수정본"), "{reason}");
    assert_eq!(s.names(), ["Show S03E02.mkv", "Show S03E25.mkv"]);
    assert_eq!(s.on_disk(), ["Show S03E02.mkv", "Show S03E25.mkv"]);
}

// --- an undo that began is never left half done ----------------------------------

impl Scene {
    /// Asks for the undo of `−48` as `id` and stops the worker while
    /// Transmission renames the first file: the undo began, the value is back,
    /// `S03E01` is renamed in Transmission and nothing is recorded of it.
    async fn undo_cut_short(&self, rule: &Rule, id: &str) {
        let (status, body) = self.ask_undo(rule, id, -48).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        let gate = self.h.tr.hold_answer("torrent-rename-path");
        let worker = self.h.worker();
        let task =
            tokio::spawn(async move { worker.run_commands(&CancellationToken::new()).await });
        gate.wait_arrived().await;
        task.abort();
        let _ = task.await;
        gate.release_all();
        self.h.wait_lock_free().await;
        assert_eq!(self.rule(rule).await.episode, -24);
    }

    /// A worker whose Transmission cannot be reached.
    fn worker_without_transmission(&self) -> trss_worker::Worker {
        let env = trss_worker::WorkerEnv::from_lookup(|key| {
            (key == "TRANSMISSION_URL").then(|| "http://127.0.0.1:1/transmission/rpc".to_owned())
        })
        .unwrap();
        self.h.worker_with(self.h.db.clone(), &env)
    }

    async fn command(&self, id: &str) -> Value {
        let (status, _, command) = self
            .api
            .call("GET", &format!("/api/commands/{id}"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{command}");
        command
    }
}

/// An undo that began and cannot reach Transmission ends at once with its
/// files still `pending`: it does not hold up the other commands, and
/// `이어서 되돌리기` carries it on later.
#[tokio::test]
async fn an_undo_that_cannot_reach_transmission_stops_and_is_carried_on_later() {
    use trss_core::commands::{CommandStore, NewCommand};
    let (s, rule) = Scene::third_season_received().await;
    s.undo_cut_short(&rule, "undo-0301-a").await;
    // Another command waits behind it (an undo of a rule that is gone).
    CommandStore::new(s.h.db.clone())
        .accept(
            NewCommand {
                id: "undo-0301-other".into(),
                kind: "episode_undo".into(),
                payload: r#"{"rule_id":"gone","episode":-48}"#.into(),
                subject: Some("gone".into()),
            },
            s.h.now(),
        )
        .await
        .unwrap();

    // Transmission cannot be reached: one look ends both.
    s.worker_without_transmission()
        .run_commands(&CancellationToken::new())
        .await
        .unwrap();
    let command = s.command("undo-0301-a").await;
    assert_eq!(command["state"], "done", "{command}");
    assert_eq!(command["outcome"]["result"], "paused", "{command}");
    let reason = command["outcome"]["reason"].as_str().unwrap();
    assert!(reason.contains("이어서 되돌리기"), "{reason}");
    assert_eq!(s.command("undo-0301-other").await["state"], "failed");
    let view = s.view(&rule).await;
    let states: Vec<String> = undo_files(&view).into_iter().map(|f| f.2).collect();
    assert_eq!(states, ["pending", "pending"]);
    assert_eq!(s.rule(&rule).await.episode, -24);

    // Carried on once Transmission answers.
    let command = s.undo(&rule, "undo-0301-b", -48).await;
    assert_eq!(command["outcome"]["result"], "undone", "{command}");
    assert_eq!(s.names(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    let states: Vec<String> = undo_files(&s.view(&rule).await)
        .into_iter()
        .map(|f| f.2)
        .collect();
    assert_eq!(states, ["renamed", "renamed"]);
}

/// A file an earlier start renamed and did not record is recorded renamed,
/// though a replacement began on its name between the starts.
#[tokio::test]
async fn a_file_renamed_before_a_start_was_cut_short_is_recorded_renamed() {
    let (s, rule) = Scene::third_season_received().await;
    s.undo_cut_short(&rule, "undo-0303-a").await;
    assert_eq!(s.names(), ["Show S03E02.mkv", "Show S03E25.mkv"]);
    s.revision_row(&rule, "Show - 49", "Show S03E01.mkv", true)
        .await;

    assert_eq!(
        s.h.worker()
            .run_commands(&CancellationToken::new())
            .await
            .unwrap(),
        CommandsOutcome::Ran(1)
    );

    assert_eq!(
        undo_files(&s.view(&rule).await),
        [
            file("Show S03E01.mkv", "Show S03E25.mkv", "renamed"),
            file("Show S03E02.mkv", "Show S03E26.mkv", "renamed"),
        ]
    );
    assert_eq!(s.names(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
}

impl Scene {
    /// Rewrites the identities the undo kept as a file system mounted again
    /// (after the machine restarted) shows the same files: another device
    /// number, the same inode, size and times.
    async fn mounted_again(&self) {
        use trss_core::DbError;
        self.h
            .db
            .run::<_, DbError, _>(|c| {
                c.execute(
                    "UPDATE episode_undo_files
                        SET identity = '999999' || substr(identity, instr(identity, ':'))
                      WHERE identity IS NOT NULL",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }
}

/// An undo carried on after a restart that mounted the folder again: the
/// file Transmission renamed before the start was cut short is recorded
/// renamed, and the video whose torrent is gone is renamed on disk.
#[tokio::test]
async fn an_undo_cut_short_knows_its_videos_whatever_device_number_they_were_mounted_with() {
    let (s, rule) = Scene::third_season_received().await;
    s.h.tr.remove(&hash(50));
    s.undo_cut_short(&rule, "undo-0305-a").await;
    assert_eq!(s.on_disk(), ["Show S03E02.mkv", "Show S03E25.mkv"]);
    s.mounted_again().await;

    assert_eq!(
        s.h.worker()
            .run_commands(&CancellationToken::new())
            .await
            .unwrap(),
        CommandsOutcome::Ran(1)
    );

    let view = s.view(&rule).await;
    assert_eq!(
        undo_files(&view),
        [
            file("Show S03E01.mkv", "Show S03E25.mkv", "renamed"),
            file("Show S03E02.mkv", "Show S03E26.mkv", "renamed"),
        ],
        "{view}"
    );
    assert_eq!(s.names(), ["Show S03E25.mkv"]);
    assert_eq!(s.on_disk(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    assert_eq!(
        fs::read(s.season3().join("Show S03E26.mkv")).unwrap(),
        b"video 50"
    );
}

/// On a rule that is automatic again (an import marked it so), `되돌리기` is a
/// new undo of the rule as it is: it does not carry on an older one.
#[tokio::test]
async fn an_automatic_rule_starts_a_new_undo_instead_of_carrying_on_an_old_one() {
    use trss_core::commands::{CommandState, CommandStore, Outcome};
    use trss_core::DbError;
    let (s, rule) = Scene::third_season_received().await;
    s.undo_cut_short(&rule, "undo-0304-a").await;
    CommandStore::new(s.h.db.clone())
        .finish(
            "undo-0304-a",
            CommandState::Failed,
            Outcome {
                result: "failed".into(),
                reason: Some("처리하다 내부 오류가 났어요.".into()),
            },
            s.h.now(),
        )
        .await
        .unwrap();
    // The rule is automatic again, at −48 over −24.
    let id = rule.id.clone();
    s.h.db
        .run::<_, DbError, _>(move |c| {
            c.execute(
                "UPDATE rules SET episode = -48, episode_auto = 1, episode_previous = -24,
                        version = version + 1 WHERE id = ?1",
                rusqlite::params![id],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    let command = s.undo(&rule, "undo-0304-b", -48).await;

    assert_eq!(command["outcome"]["result"], "undone", "{command}");
    let now = s.rule(&rule).await;
    assert_eq!((now.episode, now.episode_auto), (-24, false));
    assert_eq!(s.names(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    let view = s.view(&rule).await;
    assert_eq!(view["episode_undo"]["command"]["id"], "undo-0304-b");
    // Nothing is left to carry on.
    let (status, body) = s.ask_undo(&rule, "undo-0304-c", -48).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn an_undo_that_ended_half_done_is_shown_and_carried_on_when_asked_again() {
    use trss_core::commands::{CommandState, CommandStore, Outcome};
    let (s, rule) = Scene::third_season_received().await;
    s.undo_cut_short(&rule, "undo-0302-a").await;
    // It ended there (a panic, say).
    CommandStore::new(s.h.db.clone())
        .finish(
            "undo-0302-a",
            CommandState::Failed,
            Outcome {
                result: "failed".into(),
                reason: Some("처리하다 내부 오류가 났어요.".into()),
            },
            s.h.now(),
        )
        .await
        .unwrap();

    // The rule shows the value back, and the files still to rename.
    let view = s.view(&rule).await;
    assert_eq!(
        (view["episode"].clone(), view["episode_auto"].clone()),
        (json!(-24), json!(false))
    );
    assert_eq!(view["episode_undo"]["command"]["state"], "failed");
    assert_eq!(view["episode_undo"]["from"], -48);
    assert_eq!(view["episode_undo"]["to"], -24);
    let states: Vec<String> = undo_files(&view).into_iter().map(|f| f.2).collect();
    assert_eq!(states, ["pending", "pending"]);

    // Asked again for the same automatic value, though the rule is no longer
    // automatic: it carries on with the files left.
    let command = s.undo(&rule, "undo-0302-b", -48).await;
    assert_eq!(command["state"], "done", "{command}");
    let view = s.view(&rule).await;
    assert_eq!(view["episode_undo"]["command"]["id"], "undo-0302-b");
    assert_eq!(
        undo_files(&view),
        [
            file("Show S03E01.mkv", "Show S03E25.mkv", "renamed"),
            file("Show S03E02.mkv", "Show S03E26.mkv", "renamed"),
        ]
    );
    assert_eq!(s.names(), ["Show S03E25.mkv", "Show S03E26.mkv"]);
    assert_eq!(s.rule(&rule).await.episode, -24);

    // Nothing is left: another request is refused.
    let (status, body) = s.ask_undo(&rule, "undo-0302-c", -48).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
