//! What the worker does around the connecting of a subscription to its season
//! (tickets 0019, 0035, 0045): the cycle that connects a season makes the
//! `anissia_captions` command and the creator's jobs in the same cycle, and a
//! `start` or `resume` that turns a followed subscription on looks at the
//! creator. The real worker, a real library scan of temporary folders, the
//! history of what a rule received, and a fake Transmission that says where
//! each torrent's files are.
//!
//! Which season a rule connects to, what a pass repeats and what it reports are
//! the rules of `trss-collect` (`season_link` tests).

use crate::common;

use std::{fs, path::Path};

use common::*;
use tokio_util::sync::CancellationToken;
use trss_anissia::Anime;
use trss_collect::store::{
    channels::{NewSubscription, Rule, SubtitleMode},
    history::{HistoryResult, Observation},
};
use trss_library::store::library::LibraryStore;
use trss_worker::{TickOutcome, Worker};

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "x").unwrap();
}

fn anime(no: i64) -> Anime {
    Anime {
        anime_no: no,
        subject: format!("작품 {no}"),
        original_subject: None,
        week: 3,
        air_time: Some("22:00".into()),
        start_date: Some("2026-10-07".into()),
        end_date: None,
        status: "ON".into(),
        fetched_at: 1_000_000,
    }
}

struct Scene {
    h: Harness,
    worker: Worker,
    library: LibraryStore,
    /// The watch folder, which holds the work folders the torrents save into.
    shows: std::path::PathBuf,
    channel: String,
    next_hash: u32,
}

impl Scene {
    async fn new() -> Scene {
        let h = Harness::new().await;
        let shows = h.dir.path().join("Shows");
        fs::create_dir_all(&shows).unwrap();
        let channel = h.add_channel("feed-a", "/media", &[], Vec::new()).await;
        Scene {
            worker: h.worker(),
            library: LibraryStore::new(h.db.clone()),
            shows,
            channel: channel.channel.id,
            h,
            next_hash: 0,
        }
    }

    /// A subscription rule for anime `no`.
    async fn subscription(&self, no: i64, phrase: &str) -> Rule {
        self.h
            .channels
            .create_subscription_rule(
                &self.channel,
                rule(phrase, &format!("{phrase}/Season 01")),
                NewSubscription {
                    anime: anime(no),
                    subtitles: SubtitleMode::Undecided,
                    creator: None,
                    subscribed_at: 1_000_000,
                },
            )
            .await
            .unwrap()
    }

    /// The rule received a torrent whose file is `file` (a path below the
    /// watch folder); Transmission has it saved next to the file.
    async fn received(&mut self, rule: &Rule, file: &str) {
        self.next_hash += 1;
        let hash = format!("{:040x}", self.next_hash);
        let path = Path::new(file);
        let dir = self.shows.join(path.parent().unwrap());
        touch(&self.shows.join(file));
        self.h.tr.preload(
            FakeTorrent::new(&hash, "name").in_dir(&dir).files(&[path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()]),
        );
        self.h
            .history
            .record(
                self.h.now(),
                vec![Observation {
                    channel_id: self.channel.clone(),
                    channel_label: "feed".into(),
                    identity_key: format!("key-{hash}"),
                    title: format!("[Group] {file}"),
                    link: format!("magnet:?xt=urn:btih:{hash}"),
                    result: HistoryResult::Received,
                    rule_id: Some(rule.id.clone()),
                    torrent_hash: Some(hash),
                    reason: None,
                }],
            )
            .await
            .unwrap();
    }

    /// Registers the watch folder (which scans it) once the files are there.
    async fn register(&self) {
        let (status, text, _) = self
            .h
            .web_api()
            .call(
                "POST",
                "/api/library/watch-folders",
                Some(serde_json::json!({ "path": self.shows.to_str().unwrap() })),
            )
            .await;
        assert_eq!(status, 201, "{text}");
    }

    async fn tick(&self) {
        match self.worker.tick(&CancellationToken::new()).await.unwrap() {
            TickOutcome::Ran(_) => {}
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    async fn work_id(&self, name: &str) -> String {
        let folder = self.library.folders().await.unwrap().remove(0);
        self.library
            .works(&folder.id)
            .await
            .unwrap()
            .into_iter()
            .find(|w| w.dir_name == name)
            .unwrap_or_else(|| panic!("no work {name}"))
            .id
    }

    async fn rule(&self, rule: &Rule) -> Rule {
        self.h.channels.get_rule(&rule.id).await.unwrap().unwrap()
    }
}

fn season_of(rule: &Rule) -> Option<&str> {
    rule.subscription.as_ref().unwrap().season_id.as_deref()
}

// --- the subscription's anime is read when its season is connected (ticket 0035) ---

#[tokio::test]
async fn connecting_a_subscription_to_a_season_has_the_animes_subtitle_lines_read_once() {
    use std::{sync::atomic::Ordering, sync::Arc, time::Duration};
    use trss_anissia::{fake::Fake, Anissia};
    use trss_collect::{anissia::captions::CaptionObserver, store::anissia::AnissiaStore};
    use trss_worker::CommandsOutcome;

    let mut scene = Scene::new().await;
    let fake = Fake::start().await;
    let clock = {
        let now = scene.h.clock.clone();
        Arc::new(move || now.load(Ordering::SeqCst)) as trss_core::Clock
    };
    let anissia =
        Anissia::new(scene.h.db.clone(), fake.config(), clock).with_spacing(Duration::ZERO);
    let store = AnissiaStore::new(scene.h.db.clone());
    scene.worker = scene
        .h
        .worker()
        .with_captions(CaptionObserver::new(anissia, store.clone()));
    // Anissia's own list for the anime has a line the recent list does not reach.
    fake.set_captions(
        7,
        vec![serde_json::json!({
            "episode": "24", "updDt": "2026-03-01T00:00:00",
            "website": "https://blog.test/old", "name": "에루샤"})],
    );
    let rule = scene.subscription(7, "Work").await;
    scene
        .received(&rule, "Work/Season 01/Work - S01E01.mkv")
        .await;
    scene.register().await;

    // The cycle that connects the rule makes the command, and nothing reads yet.
    scene.tick().await;
    assert!(season_of(&scene.rule(&rule).await).is_some());
    let commands = trss_core::commands::CommandStore::new(scene.h.db.clone());
    let command = commands
        .latest_for_subjects("anissia_captions", vec!["7".into()])
        .await
        .unwrap()
        .remove("7")
        .expect("the connection asks for a read");
    assert_eq!(command.payload, r#"{"anime_no":7}"#);
    assert_eq!(command.state, trss_core::commands::CommandState::Pending);
    assert_eq!(fake.count("/anime/caption/animeNo/"), 0);

    // The worker reads it, and the older line is a candidate of the anime.
    assert_eq!(
        scene
            .worker
            .run_commands(&CancellationToken::new())
            .await
            .unwrap(),
        CommandsOutcome::Ran(1)
    );
    assert_eq!(fake.count("/anime/caption/animeNo/7"), 1);

    // Cycles that change no link ask for nothing more.
    scene.h.advance(5_000);
    scene.tick().await;
    scene.h.advance(5_000);
    scene.tick().await;
    assert!(!commands.has_open().await.unwrap());
    let latest = commands
        .latest_for_subjects("anissia_captions", vec!["7".into()])
        .await
        .unwrap()
        .remove("7")
        .unwrap();
    assert_eq!(latest.id, command.id);
    assert_eq!(fake.count("/anime/caption/animeNo/"), 1);
}

/// A subscription to anime 7 that follows creator 에루샤 and has received the
/// first episode of `Work`, with that creator's episode 2 observed and the
/// season's AniList entry stored: once the cycle connects the rule to its
/// season, the creator's episode 2 is there to receive.
async fn followed_subscription(scene: &mut Scene) -> Rule {
    let rule = scene
        .h
        .channels
        .create_subscription_rule(
            &scene.channel,
            rule("Work", "Work/Season 01"),
            NewSubscription {
                anime: anime(7),
                subtitles: SubtitleMode::Follow,
                creator: Some("에루샤".into()),
                subscribed_at: 1_000_000,
            },
        )
        .await
        .unwrap();
    scene
        .received(&rule, "Work/Season 01/Work - S01E01.mkv")
        .await;
    scene.register().await;
    // The season's episode count, and the creator's episode 2 already observed.
    let work = scene.work_id("Work").await;
    scene
        .h
        .db
        .run::<_, trss_core::DbError, _>(move |c| {
            // Episode 2 aired an hour before the creator's line (`updDt`
            // 2026-10-02T11:00:00); one a week.
            let second =
                trss_anissia::observe::updated_at("2026-10-02T11:00:00").unwrap() / 1000 - 3600;
            let airing = (1..=12)
                .map(|k| {
                    format!(
                        r#"{{"episode":{k},"at":{}}}"#,
                        second + (k - 2) * 7 * 86_400
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            c.execute(
                "INSERT INTO anilist_entries (id, format, episodes, airing, fetched_at)
                 VALUES (1, 'TV', 12, ?1, 1)",
                [format!("[{airing}]")],
            )?;
            c.execute(
                "INSERT OR IGNORE INTO season_info (work_id, season) VALUES (?1, 1)",
                [&work],
            )?;
            c.execute(
                "INSERT INTO season_entries (work_id, season, position, anilist_id)
                 VALUES (?1, 1, 0, 1)",
                [&work],
            )?;
            c.execute_batch(&format!(
                "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('src-a', 7, '에루샤', 1);
                 INSERT INTO caption_observations
                     (source_id, post_url, episode, updated, updated_at, first_seen_at)
                 VALUES ('src-a', 'https://fake.trss.invalid/ok/ep2', '2',
                         '2026-10-02T11:00:00', {}, 2);",
                (second + 3600) * 1000
            ))?;
            Ok(())
        })
        .await
        .unwrap();
    rule
}

// --- a subscribed creator's season, once connected, is looked at (ticket 0045) ---

#[tokio::test]
async fn connecting_a_followed_subscription_to_its_season_makes_the_creators_jobs_in_that_cycle() {
    use std::sync::Arc;
    use trss_jobs::{area::ReceiveArea, JobRun, JobViews, Runner};
    use trss_subtitles::{fake::FakeSource, Sources};

    let mut scene = Scene::new().await;
    let jobs = JobViews::new(scene.h.db.clone());
    scene.worker = scene.h.worker().with_jobs(Runner::new(
        JobRun::new(scene.h.db.clone()),
        Sources::none().with_fake(FakeSource),
        ReceiveArea::in_app_data(scene.h.dir.path()),
        Arc::new(|| 2_000),
    ));
    let rule = followed_subscription(&mut scene).await;
    // Without a season the rule's creator has nothing to receive.
    assert_eq!(scene.worker.follow_once().await.unwrap(), 0);

    // The cycle that connects the rule looks at the creator's posts at once.
    scene.tick().await;
    assert!(season_of(&scene.rule(&rule).await).is_some());
    let open = jobs.open_jobs().await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].origin, trss_jobs::AUTO);
}

#[tokio::test]
async fn a_start_or_resume_that_turns_a_followed_subscription_on_makes_the_creators_jobs() {
    use std::sync::Arc;
    use trss_collect::commands::rule_archive::{ask_start, Direction};
    use trss_collect::store::channels::RuleState;
    use trss_core::commands::CommandStore;
    use trss_jobs::{area::ReceiveArea, JobRun, JobViews, Runner};
    use trss_subtitles::{fake::FakeSource, Sources};
    use trss_worker::CommandsOutcome;

    for direction in [Direction::Resume, Direction::Start] {
        let mut scene = Scene::new().await;
        let rule = followed_subscription(&mut scene).await;
        // The cycle connects the rule to its season; this worker runs no jobs.
        scene.tick().await;
        assert!(season_of(&scene.rule(&rule).await).is_some());
        // The rule is off, as the web leaves it for a move, and the creator's
        // episode 2 is there to receive when it is on again.
        scene
            .h
            .channels
            .set_rule_state(&rule.id, RuleState::Paused, 1)
            .await
            .unwrap();
        let jobs = JobViews::new(scene.h.db.clone());
        scene.worker = scene.h.worker().with_jobs(Runner::new(
            JobRun::new(scene.h.db.clone()),
            Sources::none().with_fake(FakeSource),
            ReceiveArea::in_app_data(scene.h.dir.path()),
            Arc::new(|| 2_000),
        ));
        assert_eq!(scene.worker.follow_once().await.unwrap(), 0);
        assert!(jobs.open_jobs().await.unwrap().is_empty());

        // No archive folder is set, so the command only turns the rule on.
        let commands = CommandStore::new(scene.h.db.clone());
        ask_start(&commands, &rule.id, direction, 3_000)
            .await
            .unwrap();
        assert_eq!(
            scene
                .worker
                .run_commands(&CancellationToken::new())
                .await
                .unwrap(),
            CommandsOutcome::Ran(1)
        );

        assert_eq!(scene.rule(&rule).await.state, RuleState::Active);
        let open = jobs.open_jobs().await.unwrap();
        assert_eq!(open.len(), 1, "{direction:?}");
        assert_eq!(open[0].origin, trss_jobs::AUTO);
    }
}
