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
    area::ReceiveArea, mapping::MappingKind, store::JobDetail, Created, Follow, JobState, JobStore,
    NewItem, NewJob, Runner, Wait, AUTO,
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
    season: u32,
    /// The season's AniList episode count.
    count: Option<u32>,
    /// The season's AniList airing schedule: episode and Unix seconds.
    airing: Vec<(u32, i64)>,
    /// The episodes of the season before it, as one AniList entry linked to
    /// season 1 (for a season 2).
    earlier: Option<u32>,
    /// The offset of a mapping the user set for the creators `에루샤` and
    /// `다른` before anything is looked at. Most tests are about what is
    /// received once the episodes are mapped, so they start from a mapping the
    /// app never touches; the tests of the app's own decision turn it off.
    user_offset: Option<i64>,
}

impl Default for Sub {
    fn default() -> Self {
        Sub {
            subtitles: SubtitleMode::Follow,
            creator: Some("에루샤"),
            season: 1,
            count: Some(12),
            airing: Vec::new(),
            earlier: None,
            user_offset: Some(0),
        }
    }
}

impl Sub {
    /// The app decides the mapping from the schedule: a weekly `episodes`-episode
    /// season whose first episode airs at `first` (as Anissia writes it).
    fn aired(first: &str, episodes: u32) -> Sub {
        Sub {
            count: Some(episodes),
            airing: weekly(first, episodes),
            user_offset: None,
            ..Sub::default()
        }
    }
}

/// Unix ms of a time as Anissia writes it (Seoul).
fn ms(text: &str) -> i64 {
    trss_anissia::observe::updated_at(text).unwrap()
}

/// A weekly schedule of `episodes` episodes, the first airing at `first`.
fn weekly(first: &str, episodes: u32) -> Vec<(u32, i64)> {
    (1..=episodes)
        .map(|k| (k, ms(first) / 1000 + i64::from(k - 1) * 7 * 86_400))
        .collect()
}

/// The time (Unix ms) `plus_seconds` after the air time of season episode `k`
/// of [`Sub::aired`]'s schedule.
fn at(first: &str, k: u32, plus_seconds: i64) -> i64 {
    ms(first) + i64::from(k - 1) * 7 * 86_400_000 + plus_seconds * 1000
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
        let airing = serde_json::to_string(
            &sub.airing
                .iter()
                .map(|(episode, at)| serde_json::json!({ "episode": episode, "at": at }))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let earlier = sub.earlier;
        db.run::<_, DbError, _>(move |c| {
            c.execute_batch(
                "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
                 INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
                 INSERT INTO seasons (work_id, number) VALUES ('w1', 1), ('w1', 2);
                 INSERT OR IGNORE INTO season_info (work_id, season) VALUES ('w1', 1), ('w1', 2);",
            )?;
            if count.is_some() || airing != "[]" {
                c.execute(
                    "INSERT INTO anilist_entries (id, format, episodes, airing, fetched_at)
                     VALUES (1, 'TV', ?1, ?2, 1)",
                    rusqlite::params![count, airing],
                )?;
                c.execute(
                    "INSERT INTO season_entries (work_id, season, position, anilist_id)
                     VALUES ('w1', ?1, 0, 1)",
                    [season],
                )?;
            }
            if let Some(earlier) = earlier {
                c.execute(
                    "INSERT INTO anilist_entries (id, format, episodes, fetched_at)
                     VALUES (9, 'TV', ?1, 1)",
                    [earlier],
                )?;
                c.execute(
                    "INSERT INTO season_entries (work_id, season, position, anilist_id)
                     VALUES ('w1', 1, 0, 9)",
                    [],
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
        // The mapping the user set, now that the anime is known.
        if let Some(offset) = sub.user_offset {
            let season = sub.season;
            db.run::<_, DbError, _>(move |c| {
                for creator in ["에루샤", "다른"] {
                    c.execute(
                        "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                         VALUES ('src-' || ?1, ?2, ?1, 1)",
                        rusqlite::params![creator, ANIME],
                    )?;
                    c.execute(
                        "INSERT INTO subtitle_episode_mappings
                             (work_id, season, source_id, kind, episode_offset, evidence,
                              decided_at)
                         VALUES ('w1', ?1, 'src-' || ?2, 'user', ?3, '사용자가 정했어요', 1)",
                        rusqlite::params![season, creator, offset],
                    )?;
                }
                Ok(())
            })
            .await
            .unwrap();
        }

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
        self.observe_seen(creator, episode, path, updated, 2).await
    }

    /// [`World::observe`] with the time the app first saw the line.
    async fn observe_seen(
        &self,
        creator: &str,
        episode: &str,
        path: &str,
        updated: &str,
        seen: i64,
    ) -> i64 {
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
                         (source_id, post_url, episode, updated, updated_at, first_seen_at)
                     VALUES ('src-' || ?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        creator,
                        url,
                        episode,
                        updated,
                        trss_anissia::observe::updated_at(&updated),
                        seen
                    ],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    /// [`World::observe`] for a line Anissia wrote at `at` (Unix ms; `None`:
    /// a time that did not read).
    async fn observe_at(&self, creator: &str, episode: &str, path: &str, at: Option<i64>) -> i64 {
        let (creator, episode, url) = (creator.to_owned(), episode.to_owned(), post(path));
        self.db
            .run::<_, DbError, _>(move |c| {
                c.execute(
                    "INSERT OR IGNORE INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('src-' || ?1, ?2, ?1, 1)",
                    rusqlite::params![creator, ANIME],
                )?;
                c.execute(
                    "INSERT INTO caption_observations
                         (source_id, post_url, episode, updated, updated_at, first_seen_at)
                     VALUES ('src-' || ?1, ?2, ?3, 'written', ?4, 2)",
                    rusqlite::params![creator, url, episode, at],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    /// The conflicts recorded for the creator's source in season `season`:
    /// episode and reason.
    async fn conflicts(&self, season: u32, creator: &str) -> Vec<(String, String)> {
        let source = format!("src-{creator}");
        self.db
            .run::<_, DbError, _>(move |c| {
                let mut stmt = c.prepare(
                    "SELECT episode, reason FROM subtitle_mapping_conflicts
                      WHERE work_id = 'w1' AND season = ?1 AND source_id = ?2
                      ORDER BY episode",
                )?;
                let rows = stmt.query_map(rusqlite::params![season, source], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
            .unwrap()
    }

    /// The episodes of the jobs `made`, as written, in order.
    async fn episodes_of(&self, made: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for id in made {
            out.push(self.detail(id).await.items[0].episode.clone());
        }
        out
    }

    /// The mapping of the creator's source in season `season`.
    async fn mapping(&self, season: u32, creator: &str) -> trss_jobs::mapping::Mapping {
        self.follow.mappings(WORK, season).await.unwrap()[&format!("src-{creator}")].clone()
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

    /// The user names `creator` the creator of the season's subtitle of the
    /// episode, at `at` (the file is in the library already).
    async fn attribute(&self, season: u32, episode: u32, creator: &'static str, at: i64) {
        self.db
            .run::<_, DbError, _>(move |c| {
                c.execute(
                    "INSERT OR IGNORE INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('src-' || ?1, ?2, ?1, 1)",
                    rusqlite::params![creator, ANIME],
                )?;
                let changed = c.execute(
                    "UPDATE media_files
                        SET creator_source_id = 'src-' || ?1, creator_set_at = ?2,
                            creator_version = creator_version + 1
                      WHERE work_id = 'w1' AND season = ?3 AND episode = ?4 AND kind = 'subtitle'",
                    rusqlite::params![creator, at, season, format!("{episode:02}")],
                )?;
                assert_eq!(changed, 1);
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

/// The first episode of the season airs here (Seoul time).
const FIRST: &str = "2026-07-04T22:30:00";

#[tokio::test]
async fn two_episodes_posted_after_they_aired_decide_the_mapping_and_the_episodes_are_received() {
    let w = World::new(Sub::aired(FIRST, 12)).await;
    // 1 an hour and a half after it aired, 2 two days after it aired.
    let one = w
        .observe_at("에루샤", "1", "/ok/ep1", Some(at(FIRST, 1, 5_400)))
        .await;
    let two = w
        .observe_at("에루샤", "2", "/ok/ep2", Some(at(FIRST, 2, 2 * 86_400)))
        .await;

    let made = w.evaluate().await;
    let mapping = w.mapping(1, "에루샤").await;
    assert_eq!((mapping.kind, mapping.offset), (MappingKind::Auto, Some(0)));
    assert_eq!(mapping.evidence, "1화·2화가 방영 뒤에 올라왔어요");
    assert_eq!(made.len(), 2);
    let d = w.detail(&made[0]).await;
    assert_eq!(d.items[0].observation_id, Some(one));
    assert_eq!(w.detail(&made[1]).await.items[0].observation_id, Some(two));
    assert!(w.conflicts(1, "에루샤").await.is_empty());
    w.run().await;
    assert_eq!(w.detail(&made[0]).await.row.state, JobState::Done);
    assert!(w.evaluate().await.is_empty());
}

#[tokio::test]
async fn a_cumulative_number_after_the_seasons_first_episode_maps_back_by_the_earlier_season() {
    // Season 2 of 12, after a season of 12: `13` is posted after the season's
    // episode 1 aired, which is one episode and the structure of the numbers.
    let w = World::new(Sub {
        season: 2,
        earlier: Some(12),
        ..Sub::aired(FIRST, 12)
    })
    .await;
    w.file(2, 1, "mkv").await;
    let thirteen = w
        .observe_at("에루샤", "13", "/ok/ep13", Some(at(FIRST, 1, 3_600)))
        .await;
    let made = w.evaluate().await;
    let mapping = w.mapping(2, "에루샤").await;
    assert_eq!(
        (mapping.kind, mapping.offset),
        (MappingKind::Auto, Some(-12))
    );
    assert_eq!(
        mapping.evidence,
        "13화가 1화 방영 뒤에 올라왔고 앞 시즌 회차 수(12)만큼 이어 셌어요"
    );
    assert_eq!(made.len(), 1);
    let d = w.detail(&made[0]).await;
    assert_eq!(d.items[0].observation_id, Some(thirteen));
    assert_eq!(d.row.season, Some(2));
    // Without the earlier season's episodes the offset has nothing to be the
    // sum of, and one episode is no more than a guess.
    let w = World::new(Sub {
        season: 2,
        ..Sub::aired(FIRST, 12)
    })
    .await;
    w.observe_at("에루샤", "13", "/ok/ep13", Some(at(FIRST, 1, 3_600)))
        .await;
    assert!(w.evaluate().await.is_empty());
    let mapping = w.mapping(2, "에루샤").await;
    assert_eq!(mapping.kind, MappingKind::Undecided);
    assert!(mapping.evidence.contains("13화 하나만 차이(−12)"));
}

#[tokio::test]
async fn another_creator_who_counts_from_one_in_the_same_season_maps_as_it_is() {
    let w = World::new(Sub {
        season: 2,
        earlier: Some(12),
        creator: Some("다른"),
        ..Sub::aired(FIRST, 12)
    })
    .await;
    w.observe_at("다른", "1", "/ok/o1", Some(at(FIRST, 1, 3_600)))
        .await;
    let made = w.evaluate().await;
    let mapping = w.mapping(2, "다른").await;
    assert_eq!((mapping.kind, mapping.offset), (MappingKind::Auto, Some(0)));
    assert_eq!(
        mapping.evidence,
        "1화가 방영 뒤에 올라왔고 시즌 회차 수(12) 안이에요"
    );
    assert_eq!(made.len(), 1);
}

#[tokio::test]
async fn grounds_that_do_not_agree_decide_nothing_and_receive_nothing() {
    let late = |first: u32| at(FIRST, first, 3_600);
    // (the case, what is posted, the subscription, what the reason says)
    type Case = (&'static str, Vec<(&'static str, i64)>, Sub, &'static str);
    let cases: Vec<Case> = vec![
        (
            "one episode posted after the next one aired",
            vec![("1", late(2))],
            Sub::aired(FIRST, 12),
            "1화 하나만 차이(+1)를 가리키고 시즌 구조와 맞지 않아요",
        ),
        (
            "singles that disagree",
            vec![("1", late(1)), ("2", late(3))],
            Sub::aired(FIRST, 12),
            "회차마다 가리키는 차이가 달라요(0, +1)",
        ),
        (
            "two offsets of two episodes each",
            vec![
                ("1", late(1)),
                ("2", late(2)),
                ("3", late(4)),
                ("4", late(5)),
            ],
            Sub::aired(FIRST, 12),
            "회차마다 가리키는 차이가 달라요(0, +1)",
        ),
        (
            "posted before the first episode aired",
            vec![("1", at(FIRST, 1, -3_600))],
            Sub::aired(FIRST, 12),
            "방영 시각으로 가리키는 회차가 아직 없어요",
        ),
        (
            "posted a week after the last episode aired",
            vec![("12", at(FIRST, 12, 7 * 86_400))],
            Sub::aired(FIRST, 12),
            "방영 시각으로 가리키는 회차가 아직 없어요",
        ),
        (
            "no schedule",
            vec![("1", late(1)), ("2", late(2))],
            Sub {
                airing: Vec::new(),
                ..Sub::aired(FIRST, 12)
            },
            "방영 일정이 없어요",
        ),
        (
            "an all-at-once release",
            vec![("1", late(1)), ("2", late(1) + 3_600_000)],
            Sub {
                airing: (1..=6).map(|k| (k, ms(FIRST) / 1000)).collect(),
                ..Sub::aired(FIRST, 6)
            },
            "방영 시각으로 가리키는 회차가 아직 없어요",
        ),
    ];
    for (name, posted, sub, why) in cases {
        let w = World::new(sub).await;
        for (episode, when) in &posted {
            w.observe_at("에루샤", episode, &format!("/ok/ep{episode}"), Some(*when))
                .await;
        }
        assert!(w.evaluate().await.is_empty(), "{name}");
        let mapping = w.mapping(1, "에루샤").await;
        assert_eq!(mapping.kind, MappingKind::Undecided, "{name}");
        assert_eq!(mapping.offset, None, "{name}");
        assert!(
            mapping.evidence.contains(why),
            "{name}: {}",
            mapping.evidence
        );
        assert_eq!(w.job_count().await, 0, "{name}");
    }

    // A time that did not read is no evidence, and the first sight of the line
    // is not used in its place.
    let w = World::new(Sub::aired(FIRST, 12)).await;
    w.observe_at("에루샤", "1", "/ok/ep1", None).await;
    w.observe_at("에루샤", "2", "/ok/ep2", None).await;
    assert!(w.evaluate().await.is_empty());
    assert!(w
        .mapping(1, "에루샤")
        .await
        .evidence
        .contains("방영 시각으로 가리키는 회차가 아직 없어요"));
}

#[tokio::test]
async fn the_earliest_observation_of_an_episode_is_the_evidence_not_a_later_fix() {
    let w = World::new(Sub::aired(FIRST, 12)).await;
    // 1 and 2 on time, then 2 posted again a month later (a fix): the first
    // post of each decides.
    w.observe_at("에루샤", "1", "/ok/ep1", Some(at(FIRST, 1, 3_600)))
        .await;
    w.observe_at("에루샤", "02", "/ok/ep2", Some(at(FIRST, 2, 3_600)))
        .await;
    w.observe_at("에루샤", "2", "/ok/ep2", Some(at(FIRST, 6, 3_600)))
        .await;
    w.evaluate().await;
    let mapping = w.mapping(1, "에루샤").await;
    assert_eq!((mapping.kind, mapping.offset), (MappingKind::Auto, Some(0)));
    assert_eq!(mapping.evidence, "1화·2화가 방영 뒤에 올라왔어요");
}

#[tokio::test]
async fn an_episode_that_does_not_fit_is_not_received_and_is_recorded_while_the_others_are() {
    let w = World::new(Sub::aired(FIRST, 12)).await;
    // 1, 2 and 3 on time; `13.5` and `SP` are no whole number; 14 is past the
    // 12 episodes; 5 was posted after episode 8 aired, which is another offset.
    for (episode, k) in [("1", 1), ("2", 2), ("3", 3)] {
        w.observe_at(
            "에루샤",
            episode,
            &format!("/ok/ep{episode}"),
            Some(at(FIRST, k, 3_600)),
        )
        .await;
    }
    w.observe_at("에루샤", "13.5", "/ok/ep13_5", Some(at(FIRST, 4, 3_600)))
        .await;
    w.observe_at("에루샤", "SP", "/ok/sp", Some(at(FIRST, 4, 3_600)))
        .await;
    w.observe_at("에루샤", "14", "/ok/ep14", Some(at(FIRST, 4, 3_600)))
        .await;
    w.observe_at("에루샤", "5", "/ok/ep5", Some(at(FIRST, 8, 3_600)))
        .await;
    w.observe_at("에루샤", "0", "/ok/ep0", Some(at(FIRST, 1, -86_400)))
        .await;

    let made = w.evaluate().await;
    assert_eq!(w.episodes_of(&made).await, ["1", "2", "3"]);
    let mapping = w.mapping(1, "에루샤").await;
    assert_eq!((mapping.kind, mapping.offset), (MappingKind::Auto, Some(0)));
    let conflicts = w.conflicts(1, "에루샤").await;
    assert_eq!(
        conflicts
            .iter()
            .map(|(e, _)| e.as_str())
            .collect::<Vec<_>>(),
        ["13.5", "14", "5", "SP"]
    );
    let reason = |episode: &str| {
        conflicts
            .iter()
            .find(|(e, _)| e == episode)
            .unwrap()
            .1
            .clone()
    };
    assert!(
        reason("13.5").contains("소수 회차(13.5)"),
        "{}",
        reason("13.5")
    );
    assert!(reason("SP").contains("숫자가 아닌 회차(SP)"));
    assert!(
        reason("14").contains("시즌 회차 수(12) 밖"),
        "{}",
        reason("14")
    );
    assert!(reason("5").contains("차이(+3)"), "{}", reason("5"));

    // Looked at again: the same, no job for them, and the set is the same rows.
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.conflicts(1, "에루샤").await, conflicts);
    // A revision of an episode that conflicts is not received either.
    w.observe_at("에루샤", "13.5", "/ok/ep13_5", Some(at(FIRST, 5, 3_600)))
        .await;
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.job_count().await, 3);
}

#[tokio::test]
async fn a_revision_of_a_received_episode_that_conflicts_is_received() {
    // The mapping is the user's 0 for 12 episodes, so the creator's `13.5` is a
    // conflict; the user picked it all the same and received it.
    let w = World::new(Sub::default()).await;
    let first = w
        .observe("에루샤", "13.5", "/ok/ep13_5", "2026-10-02T11:00:00")
        .await;
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.conflicts(1, "에루샤").await.len(), 1);
    let Created::Created(picked) = w
        .jobs
        .create(
            NewJob {
                command_id: "pick-13.5".to_owned(),
                request: "{}".to_owned(),
                origin: "pick".to_owned(),
                work_id: Some(WORK.to_owned()),
                season: Some(1),
                anime_no: Some(ANIME),
                source_id: Some("src-에루샤".to_owned()),
                creator: Some("에루샤".to_owned()),
                revision_of: None,
                revises_attributed: false,
                items: vec![NewItem {
                    observation_id: Some(first),
                    episode: "13.5".to_owned(),
                    post_url: post("/ok/ep13_5"),
                    found_at: 3,
                }],
            },
            NOW,
        )
        .await
        .unwrap()
    else {
        panic!("created");
    };
    w.run().await;
    assert_eq!(w.detail(&picked).await.row.state, JobState::Done);

    // The creator fixes it: the revision is of what was received, whatever
    // the mapping says of the episode.
    w.observe("에루샤", "13.5", "/ok/ep13_5", "2026-10-02T11:50:00")
        .await;
    let revised = w.evaluate().await;
    assert_eq!(revised.len(), 1);
    let d = w.detail(&revised[0]).await;
    assert_eq!(d.row.revision_of, Some(first));
    assert_eq!(d.row.revises_job.as_deref(), Some(picked.as_str()));
}

#[tokio::test]
async fn a_conflict_that_is_gone_is_no_longer_recorded() {
    let w = World::new(Sub::aired(FIRST, 12)).await;
    for (episode, k) in [("1", 1), ("2", 2)] {
        w.observe_at(
            "에루샤",
            episode,
            &format!("/ok/ep{episode}"),
            Some(at(FIRST, k, 3_600)),
        )
        .await;
    }
    w.observe_at("에루샤", "9", "/ok/ep9", Some(at(FIRST, 4, 3_600)))
        .await;
    assert_eq!(w.evaluate().await.len(), 2);
    assert_eq!(w.conflicts(1, "에루샤").await.len(), 1);

    // The season's count is learned to be 24 later: 9 (posted after episode 4)
    // is still a different offset, so it stays; the stored set is the current one.
    w.db.run::<_, DbError, _>(|c| {
        c.execute("UPDATE anilist_entries SET episodes = 24", [])?;
        Ok(())
    })
    .await
    .unwrap();
    w.evaluate().await;
    assert_eq!(w.conflicts(1, "에루샤").await.len(), 1);
    // The user maps the source: 9 is only judged by the season's episodes
    // now, so it is no conflict and is received.
    w.db.run::<_, DbError, _>(|c| {
        c.execute(
            "UPDATE subtitle_episode_mappings
                SET kind = 'user', episode_offset = 0, evidence = '사용자가 정했어요'",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let made = w.evaluate().await;
    assert_eq!(w.episodes_of(&made).await, ["9"]);
    assert!(w.conflicts(1, "에루샤").await.is_empty());
}

#[tokio::test]
async fn a_mapping_the_user_set_is_not_changed_by_the_air_time_and_only_its_structure_conflicts() {
    // The user mapped the source to +1; the creator's posts point at 0.
    let w = World::new(Sub {
        user_offset: Some(1),
        ..Sub::aired(FIRST, 12)
    })
    .await;
    for (episode, k) in [("1", 1), ("2", 2)] {
        w.observe_at(
            "에루샤",
            episode,
            &format!("/ok/ep{episode}"),
            Some(at(FIRST, k, 3_600)),
        )
        .await;
    }
    w.observe_at("에루샤", "12", "/ok/ep12", Some(at(FIRST, 12, 3_600)))
        .await;
    w.observe_at("에루샤", "13.5", "/ok/ep13_5", Some(at(FIRST, 12, 3_600)))
        .await;

    let made = w.evaluate().await;
    let mapping = w.mapping(1, "에루샤").await;
    assert_eq!(
        (mapping.kind, mapping.offset, mapping.decided_at),
        (MappingKind::User, Some(1), 1)
    );
    // 12 maps to 13, past the season; `13.5` has no episode. 1 and 2 are
    // received though their own times say 0: the user's word is not argued with.
    assert_eq!(w.episodes_of(&made).await, ["1", "2"]);
    let conflicts = w.conflicts(1, "에루샤").await;
    assert_eq!(
        conflicts
            .iter()
            .map(|(e, _)| e.as_str())
            .collect::<Vec<_>>(),
        ["12", "13.5"]
    );
}

#[tokio::test]
async fn an_auto_mapping_is_taken_back_when_two_episodes_point_at_another_offset() {
    let w = World::new(Sub::aired(FIRST, 12)).await;
    for (episode, k) in [("1", 1), ("2", 2)] {
        w.observe_at(
            "에루샤",
            episode,
            &format!("/ok/ep{episode}"),
            Some(at(FIRST, k, 3_600)),
        )
        .await;
    }
    assert_eq!(w.evaluate().await.len(), 2);
    assert_eq!(w.mapping(1, "에루샤").await.offset, Some(0));

    // 3 and 4 are posted after episodes 4 and 5 aired: two episodes of +1.
    w.observe_at("에루샤", "3", "/ok/ep3", Some(at(FIRST, 4, 3_600)))
        .await;
    w.observe_at("에루샤", "4", "/ok/ep4", Some(at(FIRST, 5, 3_600)))
        .await;
    assert!(w.evaluate().await.is_empty(), "nothing more is received");
    let mapping = w.mapping(1, "에루샤").await;
    assert_eq!(
        (mapping.kind, mapping.offset),
        (MappingKind::Undecided, None)
    );
    assert!(
        mapping
            .evidence
            .contains("회차마다 가리키는 차이가 달라요(0, +1)"),
        "{}",
        mapping.evidence
    );
    // New episodes keep waiting for the user, and the mapping stays as it was.
    w.observe_at("에루샤", "5", "/ok/ep5", Some(at(FIRST, 6, 3_600)))
        .await;
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.mapping(1, "에루샤").await.kind, MappingKind::Undecided);
    assert_eq!(w.job_count().await, 2);
}

#[tokio::test]
async fn an_auto_mapping_that_the_grounds_would_move_to_another_offset_goes_undecided_and_stays() {
    // Auto 0 from two on-time episodes. The schedule is then corrected so that
    // the same posts say +1: the one decision the grounds make is another offset.
    let w = World::new(Sub::aired(FIRST, 12)).await;
    for (episode, k) in [("1", 1), ("2", 2)] {
        w.observe_at(
            "에루샤",
            episode,
            &format!("/ok/ep{episode}"),
            Some(at(FIRST, k, 3_600)),
        )
        .await;
    }
    w.evaluate().await;
    assert_eq!(w.mapping(1, "에루샤").await.offset, Some(0));
    // The schedule is corrected to a week earlier: the same posts now come
    // after the next episode aired.
    let earlier = weekly(FIRST, 12)
        .into_iter()
        .map(|(k, secs)| serde_json::json!({ "episode": k, "at": secs - 7 * 86_400 }))
        .collect::<Vec<_>>();
    let earlier = serde_json::to_string(&earlier).unwrap();
    w.db.run::<_, DbError, _>(move |c| {
        c.execute("UPDATE anilist_entries SET airing = ?1", [earlier])?;
        Ok(())
    })
    .await
    .unwrap();
    w.evaluate().await;
    let mapping = w.mapping(1, "에루샤").await;
    assert_eq!(mapping.kind, MappingKind::Undecided);
    assert_eq!(
        mapping.evidence,
        "자동으로 정한 차이(0)와 다른 차이(+1)를 가리키는 회차가 생겼어요"
    );
    // The same grounds on the next look do not take the other offset up.
    w.evaluate().await;
    assert_eq!(w.mapping(1, "에루샤").await.kind, MappingKind::Undecided);
    assert_eq!(w.job_count().await, 2);
}

#[tokio::test]
async fn a_schedule_of_more_than_25_episodes_decides_by_the_episodes_past_the_first_page() {
    let w = World::new(Sub::aired(FIRST, 40)).await;
    for k in [26, 27, 38] {
        w.observe_at(
            "에루샤",
            &k.to_string(),
            &format!("/ok/ep{k}"),
            Some(at(FIRST, k, 7_200)),
        )
        .await;
    }
    let made = w.evaluate().await;
    let mapping = w.mapping(1, "에루샤").await;
    assert_eq!((mapping.kind, mapping.offset), (MappingKind::Auto, Some(0)));
    assert_eq!(mapping.evidence, "26화·27화·38화가 방영 뒤에 올라왔어요");
    assert_eq!(w.episodes_of(&made).await, ["26", "27", "38"]);
}

#[tokio::test]
async fn a_second_part_continues_the_season_after_the_first_parts_episodes() {
    // One season of two AniList entries of 12: the second part's schedule.
    let w = World::new(Sub {
        airing: weekly(FIRST, 12),
        count: Some(12),
        user_offset: None,
        ..Sub::default()
    })
    .await;
    let part2 = weekly("2026-10-03T22:30:00", 12);
    let part2 = serde_json::to_string(
        &part2
            .iter()
            .map(|(k, at)| serde_json::json!({ "episode": k, "at": at }))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    w.db.run::<_, DbError, _>(move |c| {
        c.execute(
            "INSERT INTO anilist_entries (id, format, episodes, airing, fetched_at)
             VALUES (2, 'TV', 12, ?1, 1)",
            [part2],
        )?;
        c.execute(
            "INSERT INTO season_entries (work_id, season, position, anilist_id)
             VALUES ('w1', 1, 1, 2)",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    // 13 and 14 posted after the second part's episodes 1 and 2 aired.
    w.observe_at(
        "에루샤",
        "13",
        "/ok/ep13",
        Some(at("2026-10-03T22:30:00", 1, 3_600)),
    )
    .await;
    w.observe_at(
        "에루샤",
        "14",
        "/ok/ep14",
        Some(at("2026-10-03T22:30:00", 2, 3_600)),
    )
    .await;
    let made = w.evaluate().await;
    let mapping = w.mapping(1, "에루샤").await;
    assert_eq!(
        (mapping.kind, mapping.offset),
        (MappingKind::Auto, Some(0)),
        "{}",
        mapping.evidence
    );
    assert_eq!(w.episodes_of(&made).await, ["13", "14"]);
}

#[tokio::test]
async fn a_mapping_the_user_set_is_kept() {
    // The user mapped the source to +2, though the creator's posts say 0.
    let w = World::new(Sub {
        user_offset: Some(2),
        ..Sub::aired(FIRST, 12)
    })
    .await;
    for (episode, k) in [("1", 1), ("2", 2)] {
        w.observe_at(
            "에루샤",
            episode,
            &format!("/ok/ep{episode}"),
            Some(at(FIRST, k, 3_600)),
        )
        .await;
    }
    let made = w.evaluate().await;
    let mapping = w.mapping(1, "에루샤").await;
    assert_eq!(
        (mapping.kind, mapping.offset, mapping.decided_at),
        (MappingKind::User, Some(2), 1)
    );
    assert_eq!(mapping.evidence, "사용자가 정했어요");
    // Its episodes 1 and 2 are the season's 3 and 4, which have no subtitle.
    assert_eq!(made.len(), 2);
    // Looked at again, with more posts: still the user's.
    w.observe_at("에루샤", "3", "/ok/ep3", Some(at(FIRST, 7, 3_600)))
        .await;
    w.evaluate().await;
    assert_eq!(w.mapping(1, "에루샤").await, mapping);
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
    // The mapping of the first work is the app's to write, and fails.
    let w = World::new(Sub::aired(FIRST, 12)).await;
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
    w.observe_at("에루샤", "1", "/ok/ep1", Some(at(FIRST, 1, 3_600)))
        .await;
    w.observe_at("에루샤", "2", "/ok/ep2", Some(at(FIRST, 2, 3_600)))
        .await;
    w.db.run::<_, DbError, _>(|c| {
        c.execute_batch(
            "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                 VALUES ('src-b', 3442, '에루샤', 1);
             INSERT INTO subtitle_episode_mappings
                 (work_id, season, source_id, kind, episode_offset, evidence, decided_at)
                 VALUES ('w2', 1, 'src-b', 'user', 0, '사용자가 정했어요', 1);
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

#[tokio::test]
async fn a_line_first_seen_after_the_creator_was_named_for_the_file_is_received_as_its_revision() {
    let w = World::new(Sub::default()).await;
    w.file(1, 5, "mkv").await;
    w.file(1, 5, "ass").await;
    // The user's own subtitle of episode 5, named as 에루샤's at 100. The line
    // the app saw before that stays a candidate on screen: no job.
    w.observe_seen("에루샤", "5", "/ok/ep5", "2026-10-02T11:00:00", 50)
        .await;
    w.attribute(1, 5, "에루샤", 100).await;
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.job_count().await, 0);
    let library = w.files().await;

    // The creator fixes it after that: one automatic job that receives.
    let fixed = w
        .observe_seen("에루샤", "05", "/ok/ep5", "2026-10-02T11:50:00", 200)
        .await;
    let made = w.evaluate().await;
    assert_eq!(made.len(), 1);
    let d = w.detail(&made[0]).await;
    assert_eq!(d.row.origin, AUTO);
    assert!(d.row.revises_attributed);
    assert_eq!(d.row.revision_of, None);
    assert_eq!(d.row.revises_job, None);
    assert_eq!(d.row.creator.as_deref(), Some("에루샤"));
    assert_eq!(d.items[0].observation_id, Some(fixed));
    assert_eq!(
        d.events.last().unwrap().message,
        "구독 제작자의 수정본이 제작자를 붙인 자막에 맞아 자동으로 작업을 만들었어요"
    );

    // Looked at again, by this process or a restarted one: no second job.
    assert!(w.evaluate().await.is_empty());
    assert!(Follow::new(w.db.clone())
        .evaluate(NOW + 1)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(w.job_count().await, 1);

    // It receives beside the subtitle in place, which stays.
    w.run().await;
    let d = w.detail(&made[0]).await;
    assert_eq!(d.row.state, JobState::Done);
    assert!(w
        .area
        .at(d.items[0].files[0].path.as_deref().unwrap())
        .exists());
    assert_eq!(w.files().await, library);
    assert!(w.evaluate().await.is_empty());

    // A later fix is a revision of that receipt, as for any received episode.
    let again = w
        .observe_seen("에루샤", "5", "/ok/ep5", "2026-10-02T12:30:00", 300)
        .await;
    let made_again = w.evaluate().await;
    assert_eq!(made_again.len(), 1);
    let d = w.detail(&made_again[0]).await;
    assert!(!d.row.revises_attributed);
    assert_eq!(d.row.revision_of, Some(fixed));
    assert_eq!(d.items[0].observation_id, Some(again));
}

#[tokio::test]
async fn a_line_anissia_shows_was_there_before_the_creator_was_named_is_no_such_revision() {
    // 2026-10-02 11:00 and 12:00 in Seoul, as Unix ms.
    const ELEVEN: i64 = 1_790_906_400_000;
    const NOON: i64 = ELEVEN + 3_600_000;
    let w = World::new(Sub::default()).await;
    w.file(1, 5, "mkv").await;
    w.file(1, 5, "ass").await;
    // Named at noon before any line of the creator was observed; the app then
    // first sees the line Anissia dates 11:00, likely the very post of the file.
    w.attribute(1, 5, "에루샤", NOON).await;
    w.observe_seen(
        "에루샤",
        "5",
        "/ok/ep5",
        "2026-10-02T11:00:00",
        NOON + 60_000,
    )
    .await;
    assert!(w.evaluate().await.is_empty());
    // A line first seen at the very moment it was named is not after it.
    w.observe_seen("에루샤", "5", "/ok/ep5", "2026-10-02T12:30:00", NOON)
        .await;
    assert!(w.evaluate().await.is_empty());

    // Named again at 13:00: a line written at 12:30 and seen since is older.
    w.attribute(1, 5, "에루샤", NOON + 3_600_000).await;
    w.observe_seen(
        "에루샤",
        "5",
        "/ok/ep5",
        "2026-10-02T12:45:00",
        NOON + 3_700_000,
    )
    .await;
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.job_count().await, 0);

    // Written and seen after the last naming: the revision is received.
    let fixed = w
        .observe_seen(
            "에루샤",
            "5",
            "/ok/ep5",
            "2026-10-02T13:10:00",
            NOON + 4_300_000,
        )
        .await;
    let made = w.evaluate().await;
    assert_eq!(made.len(), 1);
    let d = w.detail(&made[0]).await;
    assert!(d.row.revises_attributed);
    assert_eq!(d.items[0].observation_id, Some(fixed));
}

#[tokio::test]
async fn a_named_file_of_another_creator_or_a_line_of_another_creator_is_no_such_revision() {
    let w = World::new(Sub::default()).await;
    w.file(1, 5, "mkv").await;
    w.file(1, 5, "ass").await;
    w.file(1, 6, "mkv").await;
    w.file(1, 6, "ass").await;
    // Episode 5's file is 다른's and episode 6's is 에루샤's.
    w.attribute(1, 5, "다른", 100).await;
    w.attribute(1, 6, "에루샤", 100).await;
    w.observe_seen("에루샤", "5", "/ok/ep5", "2026-10-02T11:00:00", 200)
        .await;
    // 다른's line for episode 6 is not the followed creator's.
    w.observe_seen("다른", "6", "/ok/other6", "2026-10-02T11:00:00", 200)
        .await;
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.job_count().await, 0);

    // A subtitle nobody named stays as it is, too.
    w.file(1, 7, "mkv").await;
    w.file(1, 7, "ass").await;
    w.observe_seen("에루샤", "7", "/ok/ep7", "2026-10-02T11:00:00", 200)
        .await;
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.job_count().await, 0);
}

#[tokio::test]
async fn a_named_file_makes_no_receipt_without_a_subscribed_creator_or_with_subtitles_off() {
    for sub in [
        Sub {
            subtitles: SubtitleMode::None,
            creator: None,
            ..Sub::default()
        },
        Sub {
            subtitles: SubtitleMode::Undecided,
            creator: None,
            ..Sub::default()
        },
    ] {
        let w = World::new(sub).await;
        w.file(1, 5, "mkv").await;
        w.file(1, 5, "ass").await;
        w.attribute(1, 5, "에루샤", 100).await;
        w.observe_seen("에루샤", "5", "/ok/ep5", "2026-10-02T11:00:00", 200)
            .await;
        assert!(w.evaluate().await.is_empty());
        assert_eq!(w.job_count().await, 0);
    }
}

#[tokio::test]
async fn the_attributed_revision_follows_the_sources_mapping_and_waits_for_a_decision() {
    // Anissia's 13 is the second season's episode 1: the creator posted it
    // after that episode aired, and counts on from a season of 12.
    let w = World::new(Sub {
        season: 2,
        earlier: Some(12),
        ..Sub::aired(FIRST, 12)
    })
    .await;
    w.file(2, 1, "mkv").await;
    w.file(2, 1, "ass").await;
    w.attribute(2, 1, "에루샤", 100).await;
    w.observe_at("에루샤", "13", "/ok/ep13", Some(at(FIRST, 1, 3_600)))
        .await;
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.mapping(2, "에루샤").await.offset, Some(-12));
    let fixed = w
        .observe_seen("에루샤", "13", "/ok/ep13", "2026-10-02T11:50:00", 200)
        .await;
    let made = w.evaluate().await;
    assert_eq!(made.len(), 1);
    let d = w.detail(&made[0]).await;
    assert!(d.row.revises_attributed);
    assert_eq!(d.items[0].observation_id, Some(fixed));

    // A season whose mapping the app cannot decide (no schedule) receives
    // nothing: which episode the line is about is not known.
    let w = World::new(Sub {
        season: 2,
        count: Some(12),
        user_offset: None,
        ..Sub::default()
    })
    .await;
    w.file(2, 1, "mkv").await;
    w.file(2, 1, "ass").await;
    w.attribute(2, 1, "에루샤", 100).await;
    w.observe_seen("에루샤", "13", "/ok/ep13", "2026-10-02T11:50:00", 200)
        .await;
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.job_count().await, 0);
    assert_eq!(w.mapping(2, "에루샤").await.evidence, "방영 일정이 없어요");
}

#[tokio::test]
async fn an_attributed_revision_of_an_episode_that_conflicts_is_not_received() {
    let w = World::new(Sub::default()).await;
    // The user's mapping is 0 for a season of 12; the library has a 13th file
    // (a special numbered on), and the creator's line says 13.
    w.file(1, 13, "mkv").await;
    w.file(1, 13, "ass").await;
    w.attribute(1, 13, "에루샤", 100).await;
    w.observe_seen("에루샤", "13", "/ok/ep13", "2026-10-02T11:50:00", 200)
        .await;
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.conflicts(1, "에루샤").await.len(), 1);
}

#[tokio::test]
async fn another_creators_job_that_has_not_failed_still_holds_an_attributed_episode() {
    let w = World::new(Sub {
        creator: Some("다른"),
        ..Sub::default()
    })
    .await;
    w.file(1, 4, "mkv").await;
    w.observe("다른", "4", "/ok/other4", "2026-10-02T11:00:00")
        .await;
    assert_eq!(w.evaluate().await.len(), 1);
    let rule = w.rule_now().await;
    w.channels
        .set_creator(&rule.id, rule.version, Some("에루샤".into()))
        .await
        .unwrap();

    // The episode's subtitle is 에루샤's by the user's word, and her line is
    // newer, but 다른's job for the episode is still in line.
    w.file(1, 4, "ass").await;
    w.attribute(1, 4, "에루샤", 100).await;
    w.observe_seen("에루샤", "4", "/ok/ep4", "2026-10-02T12:00:00", 200)
        .await;
    assert!(w.evaluate().await.is_empty());
    assert_eq!(w.job_count().await, 1);
}
