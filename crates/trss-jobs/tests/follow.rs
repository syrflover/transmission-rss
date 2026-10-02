//! The subscribed creator's subtitles received without a pick, with the fake
//! source and Anissia's lines as the app stores what it observed
//! (`docs/specs/subtitles.md`, 구독 제작자 자동 수신).

use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

use tokio_util::sync::CancellationToken;
use trss_collect::store::channels::{
    ChannelInput, ChannelStore, NewSubscription, Rule, RuleInput, RuleState, SubtitleMode,
};
use trss_core::{Clock, Db, DbError};
use trss_jobs::{
    area::ReceiveArea, mapping::MappingKind, store::JobDetail, Follow, JobState, JobStore, Runner,
    Wait, AUTO,
};
use trss_subtitles::{
    fake::{self, FakeSource},
    Sources,
};

const ANIME: i64 = 3441;
const WORK: &str = "w1";
const NOW: i64 = 1_000_000;

struct World {
    _dir: tempfile::TempDir,
    db: Db,
    follow: Follow,
    jobs: JobStore,
    channels: ChannelStore,
    runner: Runner,
    area: ReceiveArea,
    rule: Rule,
}

/// How the subscription is set up.
struct Sub {
    subtitles: SubtitleMode,
    creator: Option<&'static str>,
    /// The rule's 회차 변환.
    episode: i64,
    season: u32,
    /// The season's AniList episode count.
    count: Option<u32>,
}

impl Default for Sub {
    fn default() -> Self {
        Sub {
            subtitles: SubtitleMode::Follow,
            creator: Some("에루샤"),
            episode: 0,
            season: 1,
            count: Some(12),
        }
    }
}

fn ticking_clock() -> Clock {
    let now = Arc::new(AtomicI64::new(NOW));
    Arc::new(move || now.fetch_add(10, Ordering::SeqCst))
}

fn post(path: &str) -> String {
    format!("https://{}{path}", fake::HOST)
}

impl World {
    async fn new(sub: Sub) -> World {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        let (season, count) = (sub.season, sub.count);
        db.run::<_, DbError, _>(move |c| {
            c.execute_batch(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
                 INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
                 INSERT INTO seasons (work_id, number) VALUES ('w1', 1), ('w1', 2);
                 INSERT OR IGNORE INTO season_info (work_id, season) VALUES ('w1', 1), ('w1', 2);",
            )?;
            if let Some(count) = count {
                c.execute(
                    "INSERT INTO anilist_entries (id, format, episodes, fetched_at)
                     VALUES (1, 'TV', ?1, 1)",
                    [count],
                )?;
                c.execute(
                    "INSERT INTO season_entries (work_id, season, position, anilist_id)
                     VALUES ('w1', ?1, 0, 1)",
                    [season],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();

        let channels = ChannelStore::new(db.clone());
        let channel = channels
            .create_channel(ChannelInput::new("https://feed.test/rss"))
            .await
            .unwrap();
        let rule = channels
            .create_subscription_rule(
                &channel.id,
                RuleInput {
                    r#match: Some("Show".into()),
                    directory: "Show".into(),
                    episode: sub.episode,
                    ..RuleInput::default()
                },
                NewSubscription {
                    anime: trss_anissia::Anime {
                        anime_no: ANIME,
                        subject: "작품".into(),
                        original_subject: None,
                        week: 3,
                        air_time: Some("22:30".into()),
                        start_date: None,
                        end_date: None,
                        status: "ON".into(),
                        fetched_at: 1,
                    },
                    subtitles: sub.subtitles,
                    creator: sub.creator.map(str::to_owned),
                    subscribed_at: 1,
                },
            )
            .await
            .unwrap();
        channels
            .link_season(&rule.id, &format!("{WORK}:{}", sub.season))
            .await
            .unwrap();
        let rule = channels.get_rule(&rule.id).await.unwrap().unwrap();

        let jobs = JobStore::new(db.clone());
        let area = ReceiveArea::in_app_data(dir.path());
        let runner = Runner::new(
            jobs.clone(),
            Sources::none().with_fake(FakeSource),
            area.clone(),
            ticking_clock(),
        );
        World {
            follow: Follow::new(db.clone()),
            _dir: dir,
            db,
            jobs,
            channels,
            runner,
            area,
            rule,
        }
    }

    /// A line of the anime as the reading stores it: a new observation of
    /// `creator`'s source.
    async fn observe(&self, creator: &str, episode: &str, path: &str, updated: &str) -> i64 {
        let (creator, episode, url, updated) = (
            creator.to_owned(),
            episode.to_owned(),
            post(path),
            updated.to_owned(),
        );
        self.db
            .run::<_, DbError, _>(move |c| {
                c.execute(
                    "INSERT OR IGNORE INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('src-' || ?1, ?2, ?1, 1)",
                    rusqlite::params![creator, ANIME],
                )?;
                c.execute(
                    "INSERT INTO caption_observations
                         (source_id, post_url, episode, updated, first_seen_at)
                     VALUES ('src-' || ?1, ?2, ?3, ?4, 2)",
                    rusqlite::params![creator, url, episode, updated],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    /// A file of the season's episode in the library: a video (`mkv`) or a
    /// subtitle (`ass`).
    async fn file(&self, season: u32, episode: u32, ext: &'static str) {
        let kind = if ext == "mkv" { "video" } else { "subtitle" };
        self.db
            .run::<_, DbError, _>(move |c| {
                let episode = format!("{episode:02}");
                c.execute(
                    "INSERT OR IGNORE INTO episodes (work_id, season, episode) VALUES ('w1', ?1, ?2)",
                    rusqlite::params![season, episode],
                )?;
                c.execute(
                    "INSERT INTO media_files (work_id, path, season, episode, kind)
                     VALUES ('w1', ?1, ?2, ?3, ?4)",
                    rusqlite::params![
                        format!("Season {season:02}/Show S{season:02}E{episode}.{ext}"),
                        season,
                        episode,
                        kind
                    ],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// The library's files, as paths.
    async fn files(&self) -> Vec<String> {
        self.db
            .run::<_, DbError, _>(|c| {
                let mut stmt = c.prepare("SELECT path FROM media_files ORDER BY path")?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
            })
            .await
            .unwrap()
    }

    async fn evaluate(&self) -> Vec<String> {
        self.follow.evaluate(NOW).await.unwrap()
    }

    async fn run(&self) {
        self.runner
            .run_ready(&CancellationToken::new())
            .await
            .unwrap();
    }

    async fn detail(&self, id: &str) -> JobDetail {
        self.jobs.detail(id).await.unwrap().unwrap()
    }

    async fn job_count(&self) -> i64 {
        self.db
            .run::<_, DbError, _>(|c| {
                Ok(c.query_row("SELECT count(*) FROM subtitle_jobs", [], |r| r.get(0))?)
            })
            .await
            .unwrap()
    }

    async fn rule_now(&self) -> Rule {
        self.channels
            .get_rule(&self.rule.id)
            .await
            .unwrap()
            .unwrap()
    }
}

#[tokio::test]
async fn a_new_episode_of_the_subscribed_creator_is_received_without_a_pick() {
    let w = World::new(Sub::default()).await;
    w.file(1, 5, "mkv").await;
    let five = w
        .observe("에루샤", "5", "/ok/ep5", "2026-10-02T11:00:00")
        .await;

    let made = w.evaluate().await;
    assert_eq!(made.len(), 1);
    let d = w.detail(&made[0]).await;
    assert_eq!(d.row.origin, AUTO);
    assert_eq!(d.row.revision_of, None);
    assert_eq!(d.row.work_id.as_deref(), Some(WORK));
    assert_eq!(d.row.season, Some(1));
    assert_eq!(d.row.creator.as_deref(), Some("에루샤"));
    assert_eq!(d.items[0].observation_id, Some(five));
    assert_eq!(d.items[0].episode, "5");
    assert_eq!(
        d.events.last().unwrap().message,
        "구독 제작자의 새 회차라 자동으로 작업을 만들었어요"
    );

    w.run().await;
    let d = w.detail(&made[0]).await;
    assert_eq!(d.row.state, JobState::Done);
    let file = &d.items[0].files[0];
    assert_eq!(
        std::fs::read(w.area.at(file.path.as_deref().unwrap())).unwrap(),
        fake::ass("ep5")
    );
}

#[tokio::test]
async fn a_post_that_asks_for_a_check_stops_at_auth() {
    let w = World::new(Sub::default()).await;
    w.observe("에루샤", "6", "/auth/ep6", "2026-10-02T11:00:00")
        .await;
    let made = w.evaluate().await;
    assert_eq!(made.len(), 1);
    w.run().await;
    let d = w.detail(&made[0]).await;
    assert_eq!(d.row.state, JobState::Waiting);
    assert_eq!(d.row.wait, Some(Wait::Auth));
    // While it waits, nothing more is made of the episode.
    assert!(w.evaluate().await.is_empty());
}

#[tokio::test]
async fn a_revision_of_a_received_episode_is_received_and_the_subtitle_in_place_stays() {
    let w = World::new(Sub::default()).await;
    w.file(1, 3, "mkv").await;
    let first = w
        .observe("에루샤", "3", "/ok/ep3", "2026-10-02T11:00:00")
        .await;
    let made = w.evaluate().await;
    w.run().await;
    // The receipt was put in place.
    w.file(1, 3, "ass").await;
    let library = w.files().await;

    // The same post with a new update time is a revision of what was received.
    let fixed = w
        .observe("에루샤", "03", "/ok/ep3", "2026-10-02T11:50:00")
        .await;
    let revised = w.evaluate().await;
    assert_eq!(revised.len(), 1);
    let d = w.detail(&revised[0]).await;
    assert_eq!(d.row.origin, AUTO);
    assert_eq!(d.row.revision_of, Some(first));
    assert_eq!(d.row.revises_job.as_deref(), Some(made[0].as_str()));
    assert_eq!(d.items[0].observation_id, Some(fixed));
    assert_eq!(
        d.events.last().unwrap().message,
        "구독 제작자의 수정본이라 자동으로 작업을 만들었어요"
    );

    w.run().await;
    let d = w.detail(&revised[0]).await;
    assert_eq!(d.row.state, JobState::Done);
    // Received beside the first, and the library's subtitle is as it was.
    assert!(w
        .area
        .at(d.items[0].files[0].path.as_deref().unwrap())
        .exists());
    let earlier = w.detail(&made[0]).await;
    assert!(w
        .area
        .at(earlier.items[0].files[0].path.as_deref().unwrap())
        .exists());
    assert_eq!(w.files().await, library);
}

#[tokio::test]
async fn the_same_line_twice_or_a_restart_makes_one_job_and_a_new_update_time_another() {
    let w = World::new(Sub::default()).await;
    w.file(1, 3, "mkv").await;
    w.observe("에루샤", "3", "/ok/ep3", "2026-10-02T11:00:00")
        .await;
    assert_eq!(w.evaluate().await.len(), 1);
    w.run().await;
    let fixed = w
        .observe("에루샤", "3", "/ok/ep3", "2026-10-02T11:50:00")
        .await;
    let revised = w.evaluate().await;
    assert_eq!(revised.len(), 1);

    // Looked at again, by this process or a new one (a restarted worker).
    assert!(w.evaluate().await.is_empty());
    assert!(Follow::new(w.db.clone())
        .evaluate(NOW + 1)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(w.job_count().await, 2);
    w.run().await;
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.job_count().await, 2);

    // The update time changed again: a new revision, of the latest receipt.
    let again = w
        .observe("에루샤", "3", "/ok/ep3", "2026-10-02T12:30:00")
        .await;
    let made = w.evaluate().await;
    assert_eq!(made.len(), 1);
    let d = w.detail(&made[0]).await;
    assert_eq!(d.row.revision_of, Some(fixed));
    assert_eq!(d.items[0].observation_id, Some(again));
    assert_eq!(w.job_count().await, 3);
}

#[tokio::test]
async fn another_creators_episode_is_never_received_by_itself() {
    let w = World::new(Sub::default()).await;
    w.file(1, 3, "mkv").await;
    w.observe("에루샤", "3", "/ok/ep3", "2026-10-02T11:00:00")
        .await;
    w.evaluate().await;
    w.run().await;
    w.file(1, 3, "ass").await;

    // Another creator's episode 3, where the subscribed creator's subtitle is,
    // and another creator's episode 6 alone.
    w.observe("다른", "3", "/ok/other3", "2026-10-02T12:00:00")
        .await;
    w.observe("다른", "6", "/ok/other6", "2026-10-02T12:00:00")
        .await;
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.job_count().await, 1);
    // The work has a creator: no `자막 구독` suggestion either.
    assert!(w.follow.suggestions().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_new_creators_episode_where_the_earlier_ones_subtitle_was_received_is_no_revision() {
    let w = World::new(Sub::default()).await;
    w.file(1, 3, "mkv").await;
    w.observe("에루샤", "3", "/ok/ep3", "2026-10-02T11:00:00")
        .await;
    w.evaluate().await;
    w.run().await;
    // Not put in place yet: the receipt alone holds the episode.
    w.observe("다른", "3", "/ok/other3", "2026-10-02T12:00:00")
        .await;
    w.observe("다른", "4", "/ok/other4", "2026-10-02T12:00:00")
        .await;
    let rule = w.rule_now().await;
    w.channels
        .set_creator(&rule.id, rule.version, Some("다른".into()))
        .await
        .unwrap();

    // The new creator is followed from the episodes nobody received.
    let made = w.evaluate().await;
    assert_eq!(made.len(), 1);
    let d = w.detail(&made[0]).await;
    assert_eq!(d.items[0].episode, "4");
    assert_eq!(d.row.revision_of, None);
    assert_eq!(d.row.creator.as_deref(), Some("다른"));
}

#[tokio::test]
async fn an_undecided_work_with_candidates_is_one_suggestion_until_a_creator_is_chosen() {
    let w = World::new(Sub {
        subtitles: SubtitleMode::Undecided,
        creator: None,
        ..Sub::default()
    })
    .await;
    for episode in 1..=4 {
        w.file(1, episode, "mkv").await;
    }
    w.file(1, 2, "ass").await;
    for (creator, episode) in [
        ("에루샤", "1"),
        ("에루샤", "2"),
        ("코코렛", "02"),
        ("코코렛", "3"),
        ("에루샤", "4"),
    ] {
        w.observe(
            creator,
            episode,
            &format!("/ok/{creator}{episode}"),
            "2026-10-02T11:00:00",
        )
        .await;
    }

    // Nothing is received while no creator is chosen.
    assert!(w.evaluate().await.is_empty());
    let suggestions = w.follow.suggestions().await.unwrap();
    assert_eq!(suggestions.len(), 1);
    let s = &suggestions[0];
    assert_eq!(s.work_id, WORK);
    assert_eq!(s.work_name, "Show");
    assert_eq!(s.anime_title.as_deref(), Some("작품"));
    assert_eq!(s.season, 1);
    assert_eq!(s.rule_id, w.rule.id);
    assert_eq!(s.episodes, ["1", "2", "3", "4"]);
    assert_eq!(s.creators, 2);

    // Choosing the creator receives the episodes that have no subtitle.
    let rule = w.rule_now().await;
    w.channels
        .set_creator(&rule.id, rule.version, Some("에루샤".into()))
        .await
        .unwrap();
    assert!(w.follow.suggestions().await.unwrap().is_empty());
    let made = w.evaluate().await;
    let mut episodes = Vec::new();
    for id in &made {
        episodes.push(w.detail(id).await.items[0].episode.clone());
    }
    assert_eq!(episodes, ["1", "4"]);
}

#[tokio::test]
async fn a_paused_or_archived_rule_or_no_subtitles_receives_and_suggests_nothing() {
    // `영상 받기` off, then archived.
    for state in [RuleState::Paused, RuleState::Archived] {
        let w = World::new(Sub::default()).await;
        let rule = w.rule_now().await;
        match state {
            RuleState::Paused => {
                w.channels
                    .set_video_receiving(&rule.id, rule.version, false, NOW)
                    .await
                    .unwrap();
            }
            _ => {
                w.channels
                    .set_rule_state(&rule.id, RuleState::Archived, NOW)
                    .await
                    .unwrap();
            }
        }
        w.observe("에루샤", "5", "/ok/ep5", "2026-10-02T11:00:00")
            .await;
        assert!(w.evaluate().await.is_empty(), "{state:?}");
        assert!(w.follow.subscribed().await.unwrap().is_empty(), "{state:?}");
    }

    // Undecided but paused: no suggestion.
    let w = World::new(Sub {
        subtitles: SubtitleMode::Undecided,
        creator: None,
        ..Sub::default()
    })
    .await;
    let rule = w.rule_now().await;
    w.channels
        .set_video_receiving(&rule.id, rule.version, false, NOW)
        .await
        .unwrap();
    w.observe("에루샤", "1", "/ok/ep1", "2026-10-02T11:00:00")
        .await;
    assert!(w.follow.suggestions().await.unwrap().is_empty());

    // `자막 받기` off.
    let w = World::new(Sub {
        subtitles: SubtitleMode::None,
        creator: None,
        ..Sub::default()
    })
    .await;
    w.observe("에루샤", "5", "/ok/ep5", "2026-10-02T11:00:00")
        .await;
    assert!(w.evaluate().await.is_empty());
    assert!(w.follow.suggestions().await.unwrap().is_empty());
    assert_eq!(w.job_count().await, 0);
}

#[tokio::test]
async fn grounds_that_agree_map_episode_13_to_s02e01_and_grounds_that_do_not_decide_nothing() {
    // The rule turns 13 into 1, and the second season has 12 episodes.
    let w = World::new(Sub {
        episode: -12,
        season: 2,
        count: Some(12),
        ..Sub::default()
    })
    .await;
    w.file(2, 1, "mkv").await;
    let thirteen = w
        .observe("에루샤", "13", "/ok/ep13", "2026-10-02T11:00:00")
        .await;
    let made = w.evaluate().await;
    let mapping = w.follow.mappings(WORK, 2).await.unwrap()["src-에루샤"].clone();
    assert_eq!(mapping.kind, MappingKind::Auto);
    assert_eq!(mapping.offset, Some(-12));
    assert!(
        mapping.evidence.contains("13→S02E01"),
        "{}",
        mapping.evidence
    );
    assert_eq!(made.len(), 1);
    let d = w.detail(&made[0]).await;
    assert_eq!(d.items[0].observation_id, Some(thirteen));
    assert_eq!(d.row.season, Some(2));

    // The creator's 26 is past the 12 episodes even with the rule's 회차 변환,
    // and a rule without one leaves 13 past them.
    for (episode, posted) in [(-12, ["13", "26"]), (0, ["13", "14"])] {
        let w = World::new(Sub {
            episode,
            season: 2,
            count: Some(12),
            ..Sub::default()
        })
        .await;
        w.file(2, 1, "mkv").await;
        for n in posted {
            w.observe("에루샤", n, &format!("/ok/ep{n}"), "2026-10-02T11:00:00")
                .await;
        }
        assert!(w.evaluate().await.is_empty(), "{episode}");
        let mapping = w.follow.mappings(WORK, 2).await.unwrap()["src-에루샤"].clone();
        assert_eq!(mapping.kind, MappingKind::Undecided, "{episode}");
        assert_eq!(mapping.offset, None);
        assert!(mapping.evidence.contains("12"), "{}", mapping.evidence);
        assert_eq!(w.job_count().await, 0);
    }

    // No AniList count: nothing to check the episodes against.
    let w = World::new(Sub {
        episode: -12,
        season: 2,
        count: None,
        ..Sub::default()
    })
    .await;
    w.observe("에루샤", "13", "/ok/ep13", "2026-10-02T11:00:00")
        .await;
    assert!(w.evaluate().await.is_empty());
    assert_eq!(
        w.follow.mappings(WORK, 2).await.unwrap()["src-에루샤"].kind,
        MappingKind::Undecided
    );
}

#[tokio::test]
async fn a_mapping_the_user_set_is_kept() {
    let w = World::new(Sub::default()).await;
    w.db.run::<_, DbError, _>(|c| {
        c.execute_batch(
            "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                 VALUES ('src-에루샤', 3441, '에루샤', 1);
             INSERT INTO subtitle_episode_mappings
                 VALUES ('w1', 1, 'src-에루샤', 'user', 2, '사용자가 정했어요', 5);",
        )?;
        Ok(())
    })
    .await
    .unwrap();
    w.observe("에루샤", "1", "/ok/ep1", "2026-10-02T11:00:00")
        .await;
    let made = w.evaluate().await;
    let mapping = w.follow.mappings(WORK, 1).await.unwrap()["src-에루샤"].clone();
    assert_eq!(
        (mapping.kind, mapping.offset, mapping.decided_at),
        (MappingKind::User, Some(2), 5)
    );
    // Its episode 1 is the season's 3, which has no subtitle.
    assert_eq!(made.len(), 1);
}

#[tokio::test]
async fn a_work_whose_folder_is_gone_receives_nothing_until_it_is_back() {
    let w = World::new(Sub::default()).await;
    let missing = |flag: i64| {
        w.db.run::<_, DbError, _>(move |c| {
            c.execute("UPDATE works SET missing = ?1 WHERE id = 'w1'", [flag])?;
            Ok(())
        })
    };
    missing(1).await.unwrap();
    w.observe("에루샤", "5", "/ok/ep5", "2026-10-02T11:00:00")
        .await;

    // The library knows of no file of a missing work, which is not "no subtitle".
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.job_count().await, 0);

    missing(0).await.unwrap();
    assert_eq!(w.evaluate().await.len(), 1);
}

#[tokio::test]
async fn another_creators_job_that_has_not_failed_holds_the_episode() {
    // The work followed 다른 first, whose episode 4 is still in line.
    let w = World::new(Sub {
        creator: Some("다른"),
        ..Sub::default()
    })
    .await;
    w.observe("다른", "4", "/ok/other4", "2026-10-02T11:00:00")
        .await;
    assert_eq!(w.evaluate().await.len(), 1);
    let rule = w.rule_now().await;
    w.channels
        .set_creator(&rule.id, rule.version, Some("에루샤".into()))
        .await
        .unwrap();

    // The new creator's episode 4 waits for that job; its episode 5 is new.
    w.observe("에루샤", "4", "/ok/ep4", "2026-10-02T12:00:00")
        .await;
    w.observe("에루샤", "5", "/ok/ep5", "2026-10-02T12:00:00")
        .await;
    let made = w.evaluate().await;
    assert_eq!(made.len(), 1);
    assert_eq!(w.detail(&made[0]).await.items[0].episode, "5");
}

#[tokio::test]
async fn a_subscription_that_cannot_be_looked_at_does_not_stop_the_others() {
    let w = World::new(Sub::default()).await;
    // A second work, subscribed to another anime of the same creator.
    w.db.run::<_, DbError, _>(|c| {
        c.execute_batch(
            "INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w2', 'f1', 'Other');
             INSERT INTO seasons (work_id, number) VALUES ('w2', 1);
             INSERT OR IGNORE INTO season_info (work_id, season) VALUES ('w2', 1);
             INSERT INTO anilist_entries (id, format, episodes, fetched_at)
                 VALUES (2, 'TV', 12, 1);
             INSERT INTO season_entries (work_id, season, position, anilist_id)
                 VALUES ('w2', 1, 0, 2);",
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let channel = w
        .channels
        .create_channel(ChannelInput::new("https://feed.test/other"))
        .await
        .unwrap();
    let other = w
        .channels
        .create_subscription_rule(
            &channel.id,
            RuleInput {
                r#match: Some("Other".into()),
                directory: "Other".into(),
                ..RuleInput::default()
            },
            NewSubscription {
                anime: trss_anissia::Anime {
                    anime_no: ANIME + 1,
                    subject: "다른 작품".into(),
                    original_subject: None,
                    week: 4,
                    air_time: None,
                    start_date: None,
                    end_date: None,
                    status: "ON".into(),
                    fetched_at: 1,
                },
                subtitles: SubtitleMode::Follow,
                creator: Some("에루샤".into()),
                subscribed_at: 1,
            },
        )
        .await
        .unwrap();
    w.channels.link_season(&other.id, "w2:1").await.unwrap();
    // Both have the creator's new episode; the first work's mapping cannot be written.
    w.observe("에루샤", "5", "/ok/ep5", "2026-10-02T11:00:00")
        .await;
    w.db.run::<_, DbError, _>(|c| {
        c.execute_batch(
            "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                 VALUES ('src-b', 3442, '에루샤', 1);
             INSERT INTO caption_observations (source_id, post_url, episode, updated, first_seen_at)
                 VALUES ('src-b', 'https://fake.trss.invalid/ok/b5', '5', '2026-10-02T11:00:00', 2);
             CREATE TRIGGER broken BEFORE INSERT ON subtitle_episode_mappings
                 WHEN NEW.work_id = 'w1'
             BEGIN SELECT RAISE(ABORT, 'broken'); END;",
        )?;
        Ok(())
    })
    .await
    .unwrap();

    let made = w.evaluate().await;
    assert_eq!(made.len(), 1);
    assert_eq!(w.detail(&made[0]).await.row.work_id.as_deref(), Some("w2"));
}
