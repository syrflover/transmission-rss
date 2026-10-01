use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
};

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::{
    anissia::Anissia,
    discovery::{EpisodeFile, FileKind, Scan, ScannedWork, WorkRead},
    store::{
        anissia::Anime,
        channels::{
            Channel, ChannelInput, NewSubscription, Rule, RuleInput, RuleState, SubtitleMode,
        },
        history::{HistoryResult, Observation},
        status::TransmissionCounts,
        Db,
    },
    web::{api, AppState},
    worker::Clock,
};

/// Thursday 2026-10-01 12:00 in Seoul (4분기); the week is 09-28 to 10-04.
const NOW: i64 = 1_790_780_400_000 + 12 * 60 * 60 * 1000;

struct App {
    state: AppState,
    now: Arc<AtomicI64>,
    channel: Channel,
}

fn file(episode: &str, name: &str, kind: FileKind) -> EpisodeFile {
    EpisodeFile {
        path: format!("Season 01/{name}"),
        kind,
        season: 1,
        episode: episode.to_owned(),
    }
}

fn work(name: &str, files: Vec<EpisodeFile>) -> WorkRead {
    WorkRead::Read(ScannedWork {
        dir_name: name.into(),
        seasons: BTreeSet::from([1]),
        files,
        unrecognized: vec![],
    })
}

impl App {
    /// A library with three works in one watch folder, and a channel, so the
    /// first run is over:
    ///
    /// - `Both` holds episode 14 with a video and a subtitle;
    /// - `VideoOnly` holds episode 14's video;
    /// - `Empty` holds episode 13's video only.
    async fn new() -> App {
        let db = Db::open_blocking(":memory:").unwrap();
        let now = Arc::new(AtomicI64::new(NOW));
        let clock: Clock = {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        };
        let anissia = Anissia::new(db.clone(), Default::default(), clock);
        let state = AppState::new(db).with_anissia(anissia);
        state
            .library
            .add_folder(
                "/c".into(),
                Scan {
                    works: vec![
                        work(
                            "Both",
                            vec![
                                file("14", "S01E14.mkv", FileKind::Video),
                                file("14", "S01E14.ko.ass", FileKind::Subtitle),
                            ],
                        ),
                        work("VideoOnly", vec![file("14", "S01E14.mkv", FileKind::Video)]),
                        work("Empty", vec![file("13", "S01E13.mkv", FileKind::Video)]),
                    ],
                },
                100,
                &[],
            )
            .await
            .unwrap();
        let channel = state
            .channels
            .create_channel(ChannelInput::new(
                "https://feed.test/rss?token=SECRETVALUE99",
            ))
            .await
            .unwrap();
        state.setup.mark_import_applied(100).await.unwrap();
        App {
            state,
            now,
            channel,
        }
    }

    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        self.call(Method::GET, uri, None).await
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(json) => {
                request = request.header("content-type", "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let response = api::router()
            .with_state(self.state.clone())
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn week(&self) -> Value {
        let (status, body) = self.get("/schedule/week").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn work_id(&self, name: &str) -> String {
        self.state
            .library
            .overview()
            .await
            .unwrap()
            .into_iter()
            .find(|w| w.dir_name == name)
            .unwrap()
            .id
    }

    /// Subscribes to `anime` with the rule `input`, and connects it to the
    /// first season of `work` when one is named.
    async fn subscribe(
        &self,
        anime: Anime,
        input: RuleInput,
        subtitles: SubtitleMode,
        work: Option<&str>,
    ) -> Rule {
        let creator = (subtitles == SubtitleMode::Follow).then(|| "제작자".to_owned());
        let rule = self
            .state
            .channels
            .create_subscription_rule(
                &self.channel.id,
                input,
                NewSubscription {
                    anime,
                    subtitles,
                    creator,
                    subscribed_at: NOW - 1000,
                },
            )
            .await
            .unwrap();
        match work {
            Some(name) => {
                let season = format!("{}:1", self.work_id(name).await);
                self.state
                    .channels
                    .link_season(&rule.id, &season)
                    .await
                    .unwrap();
                self.state
                    .channels
                    .get_rule(&rule.id)
                    .await
                    .unwrap()
                    .unwrap()
            }
            None => rule,
        }
    }
}

/// A snapshot Anissia listed a minute ago. Weeks are Anissia's: 0 Sunday.
fn anime(no: i64, subject: &str, week: u8, time: Option<&str>, start: Option<&str>) -> Anime {
    Anime {
        anime_no: no,
        subject: subject.into(),
        original_subject: None,
        week,
        air_time: time.map(str::to_owned),
        start_date: start.map(str::to_owned),
        end_date: None,
        status: "ON".into(),
        fetched_at: NOW - 60_000,
    }
}

fn rule(title: &str) -> RuleInput {
    RuleInput {
        r#match: Some(title.into()),
        directory: format!("{title}/Season 01"),
        // The release's number is the season's.
        episode: 0,
        ..Default::default()
    }
}

fn card_titles(day: &Value) -> Vec<&str> {
    day["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["title"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn the_week_has_a_card_on_each_day_something_airs_and_a_bare_day_otherwise() {
    let app = App::new().await;
    // Wednesday (Anissia's 3) 23:30 twice, Friday (5) once.
    app.subscribe(
        anime(1, "수요 둘째", 3, Some("23:30"), Some("2026-07-01")),
        rule("B"),
        SubtitleMode::None,
        Some("Both"),
    )
    .await;
    app.subscribe(
        anime(2, "수요 첫째", 3, Some("23:30"), Some("2026-07-01")),
        rule("A"),
        SubtitleMode::None,
        None,
    )
    .await;
    app.subscribe(
        anime(3, "금요", 5, Some("01:00"), Some("2026-07-03")),
        rule("C"),
        SubtitleMode::None,
        None,
    )
    .await;

    let body = app.week().await;
    assert_eq!(body["first_run"], Value::Null);
    let week = &body["week"];
    assert_eq!(week["start"], "2026-09-28");
    assert_eq!(week["end"], "2026-10-04");
    assert_eq!(week["today"], "2026-10-01");
    assert_eq!(week["quarter"], json!({ "year": 2026, "number": 4 }));
    let days = week["days"].as_array().unwrap();
    assert_eq!(days.len(), 7);
    assert_eq!(
        days.iter()
            .map(|d| d["weekday"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5, 6]
    );
    let counts: Vec<usize> = days
        .iter()
        .map(|d| d["cards"].as_array().unwrap().len())
        .collect();
    assert_eq!(counts, [0, 0, 2, 0, 1, 0, 0], "{week}");
    assert_eq!(days[2]["date"], "2026-09-30");
    assert_eq!(days[4]["date"], "2026-10-02");
    assert_eq!(
        days.iter().filter(|d| d["today"] == true).count(),
        1,
        "exactly one today"
    );
    assert_eq!(days[3]["today"], true);
    // Same time: by title.
    assert_eq!(card_titles(&days[2]), ["수요 둘째", "수요 첫째"]);
    assert_eq!(days[2]["cards"][0]["time"], "23:30");

    // A card of a connected season links to its work; the other to its rule.
    let both = app.work_id("Both").await;
    assert_eq!(days[2]["cards"][0]["work_id"], both.as_str());
    assert_eq!(days[2]["cards"][1]["work_id"], Value::Null);
    assert!(days[2]["cards"][1]["rule_id"].is_string());
}

#[tokio::test]
async fn a_held_video_without_a_subtitle_is_received_and_the_subtitle_waits() {
    let app = App::new().await;
    // Thursday 10:00, aired two hours ago: this week's episode is the 14th.
    app.subscribe(
        anime(1, "영상만", 4, Some("10:00"), Some("2026-07-02")),
        rule("V"),
        SubtitleMode::Follow,
        Some("VideoOnly"),
    )
    .await;
    app.subscribe(
        anime(2, "둘 다", 4, Some("10:00"), Some("2026-07-02")),
        rule("W"),
        SubtitleMode::Follow,
        Some("Both"),
    )
    .await;
    app.subscribe(
        anime(3, "아직", 4, Some("10:00"), Some("2026-07-02")),
        rule("E"),
        SubtitleMode::Follow,
        Some("Empty"),
    )
    .await;

    let body = app.week().await;
    let cards = body["week"]["days"][3]["cards"].as_array().unwrap();
    let by_title = |title: &str| cards.iter().find(|c| c["title"] == title).unwrap();

    let video_only = by_title("영상만");
    assert_eq!(video_only["episode"], 14);
    assert_eq!(video_only["video"], "received");
    assert_eq!(video_only["subtitle"], "waiting");
    assert_eq!(video_only["creator"], "제작자");

    let both = by_title("둘 다");
    assert_eq!(both["video"], "received");
    assert_eq!(both["subtitle"], "received");

    // The work has episode 13 only: the 14th has aired and not come.
    let empty = by_title("아직");
    assert_eq!(empty["episode"], 14);
    assert_eq!(empty["video"], "waiting");
    assert_eq!(empty["subtitle"], "waiting");
}

#[tokio::test]
async fn a_work_before_its_air_time_is_upcoming_and_a_new_subscription_has_an_empty_cover() {
    let app = App::new().await;
    // Thursday 22:00 is ten hours away; the season is not connected yet.
    app.subscribe(
        anime(1, "저녁", 4, Some("22:00"), Some("2026-07-02")),
        rule("N"),
        SubtitleMode::Follow,
        None,
    )
    .await;

    let body = app.week().await;
    let card = &body["week"]["days"][3]["cards"][0];
    assert_eq!(card["video"], "upcoming");
    assert_eq!(card["subtitle"], "waiting");
    assert_eq!(card["episode"], 14);
    assert_eq!(card["work_id"], Value::Null);
    assert_eq!(card["cover_url"], Value::Null);
}

#[tokio::test]
async fn a_paused_subscription_keeps_its_card_and_one_without_subtitles_has_no_subtitle_line() {
    let app = App::new().await;
    app.subscribe(
        anime(1, "멈춤", 4, Some("10:00"), Some("2026-07-02")),
        RuleInput {
            state: RuleState::Paused,
            ..rule("P")
        },
        SubtitleMode::Follow,
        Some("Both"),
    )
    .await;
    app.subscribe(
        anime(2, "자막 없음", 4, Some("10:00"), Some("2026-07-02")),
        rule("S"),
        SubtitleMode::None,
        Some("VideoOnly"),
    )
    .await;

    let body = app.week().await;
    let cards = body["week"]["days"][3]["cards"].as_array().unwrap();
    let by_title = |title: &str| cards.iter().find(|c| c["title"] == title).unwrap();
    let paused = by_title("멈춤");
    assert_eq!(paused["video"], "paused");
    assert_eq!(paused["subtitle"], Value::Null);
    let none = by_title("자막 없음");
    assert_eq!(none["video"], "received");
    assert_eq!(none["subtitle"], Value::Null, "{none}");
    assert_eq!(none["creator"], Value::Null);
}

#[tokio::test]
async fn an_archived_subscription_has_no_card() {
    let app = App::new().await;
    let archived = app
        .subscribe(
            anime(1, "보관", 4, Some("10:00"), Some("2026-07-02")),
            rule("A"),
            SubtitleMode::None,
            None,
        )
        .await;
    app.state
        .channels
        .set_rule_state(&archived.id, RuleState::Archived, NOW)
        .await
        .unwrap();
    app.subscribe(
        anime(3, "방영 중", 4, Some("10:00"), Some("2026-07-02")),
        rule("L"),
        SubtitleMode::None,
        None,
    )
    .await;

    let body = app.week().await;
    assert_eq!(card_titles(&body["week"]["days"][3]), ["방영 중"]);
}

#[tokio::test]
async fn an_anime_anissia_marks_off_has_a_quiet_off_card_in_place_of_the_video_and_subtitle_lines()
{
    let app = App::new().await;
    let mut off = anime(1, "결방 작품", 4, Some("10:00"), Some("2026-07-02"));
    off.status = "OFF".into();
    // The library holds the 14th, which still does not make it a received card.
    app.subscribe(off, rule("O"), SubtitleMode::Follow, Some("Both"))
        .await;
    // A paused rule of an off anime says it is paused.
    let mut paused = anime(2, "멈춘 결방", 4, Some("10:00"), Some("2026-07-02"));
    paused.status = "OFF".into();
    app.subscribe(
        paused,
        RuleInput {
            state: RuleState::Paused,
            ..rule("P")
        },
        SubtitleMode::Follow,
        None,
    )
    .await;
    app.subscribe(
        anime(3, "방영", 4, Some("10:00"), Some("2026-07-02")),
        rule("N"),
        SubtitleMode::Follow,
        Some("VideoOnly"),
    )
    .await;

    let body = app.week().await;
    let cards = body["week"]["days"][3]["cards"].as_array().unwrap();
    let by_title = |title: &str| cards.iter().find(|c| c["title"] == title).unwrap();

    let off = by_title("결방 작품");
    assert_eq!(off["video"], "off");
    assert_eq!(off["subtitle"], Value::Null);
    assert_eq!(off["episode"], Value::Null);
    assert_eq!(off["time"], "10:00");
    assert_eq!(by_title("멈춘 결방")["video"], "paused");
    assert_eq!(by_title("방영")["video"], "received");
    assert_eq!(by_title("방영")["subtitle"], "waiting");
}

#[tokio::test]
async fn the_stand_in_an_import_keeps_is_not_anissias_off() {
    let app = App::new().await;
    // What an import stores while Anissia cannot be asked: `기타`, `OFF`, never
    // received. It has no weekday, so no card; and if it were moved to a weekday
    // without being received, it is still not read as `OFF`.
    let stand_in =
        crate::store::channels::import_subscriptions::ImportSubscription::stand_in(1, "대역", None);
    assert_eq!(stand_in.status, "OFF");
    app.subscribe(stand_in.clone(), rule("S"), SubtitleMode::None, None)
        .await;
    let body = app.week().await;
    assert!(body["week"]["days"]
        .as_array()
        .unwrap()
        .iter()
        .all(|d| d["cards"].as_array().unwrap().is_empty()));

    let weekday = Anime {
        week: 4,
        air_time: Some("10:00".into()),
        start_date: Some("2026-07-02".into()),
        ..stand_in
    };
    app.state.anissia.store.put_anime(weekday).await.unwrap();
    let body = app.week().await;
    let card = &body["week"]["days"][3]["cards"][0];
    assert_eq!(card["video"], "waiting");
}

#[tokio::test]
async fn a_stand_in_with_the_comments_weekday_shows_its_card_on_that_weekday() {
    let app = App::new().await;
    // What an import stores while Anissia cannot be asked, when the comment
    // above the rule gave a weekday and time (Anissia's 4 is Thursday): the
    // card sits on that weekday right away, and the stand-in is not read as
    // `OFF` (it is not Anissia's word).
    let stand_in = crate::store::channels::import_subscriptions::ImportSubscription::stand_in(
        1,
        "주석의 요일",
        Some((4, "10:00")),
    );
    assert_eq!((stand_in.fetched_at, stand_in.status.as_str()), (0, "OFF"));
    app.subscribe(stand_in, rule("T"), SubtitleMode::None, None)
        .await;

    let body = app.week().await;
    let cards = body["week"]["days"][3]["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["title"], "주석의 요일");
    assert_eq!(cards[0]["time"], "10:00");
    assert_eq!(cards[0]["video"], "waiting");
}

#[tokio::test]
async fn a_card_leaves_once_the_end_date_has_passed() {
    let app = App::new().await;
    let mut ended = anime(1, "종영", 4, Some("10:00"), Some("2026-07-02"));
    ended.end_date = Some("2026-09-24".into());
    app.subscribe(ended, rule("E"), SubtitleMode::None, None)
        .await;
    let mut last = anime(2, "마지막 주", 4, Some("10:00"), Some("2026-07-02"));
    last.end_date = Some("2026-10-01".into());
    app.subscribe(last, rule("L"), SubtitleMode::None, None)
        .await;

    let body = app.week().await;
    assert_eq!(card_titles(&body["week"]["days"][3]), ["마지막 주"]);
}

#[tokio::test]
async fn an_anime_without_an_end_date_leaves_once_anissia_is_found_not_to_list_it_and_comes_back_if_listed(
) {
    let app = App::new().await;
    // A snapshot a month old: Anissia could not be reached since, so the card stays.
    let mut old = anime(1, "오래됨", 4, Some("10:00"), Some("2026-07-02"));
    old.fetched_at = NOW - 30 * 24 * 60 * 60 * 1000;
    app.subscribe(old, rule("O"), SubtitleMode::None, None)
        .await;
    app.subscribe(
        anime(2, "빠짐", 4, Some("10:00"), Some("2026-07-02")),
        rule("G"),
        SubtitleMode::None,
        None,
    )
    .await;

    let titles = |body: &Value| -> Vec<String> {
        card_titles(&body["week"]["days"][3])
            .into_iter()
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(titles(&app.week().await), ["빠짐", "오래됨"]);

    // The refresh asked every week and the anime was in none of them.
    app.state
        .anissia
        .store
        .mark_unlisted(vec![2], NOW, NOW + 24 * 60 * 60 * 1000, NOW)
        .await
        .unwrap();
    assert_eq!(titles(&app.week().await), ["오래됨"]);

    // Anissia lists it again.
    app.state
        .anissia
        .store
        .put_anime(anime(2, "빠짐", 4, Some("10:00"), Some("2026-07-02")))
        .await
        .unwrap();
    assert_eq!(titles(&app.week().await), ["빠짐", "오래됨"]);

    // With an end date still ahead, the end date decides, not the listing.
    app.state
        .anissia
        .store
        .put_anime(Anime {
            end_date: Some("2026-12-31".into()),
            ..anime(2, "빠짐", 4, Some("10:00"), Some("2026-07-02"))
        })
        .await
        .unwrap();
    app.state
        .anissia
        .store
        .mark_unlisted(vec![2], NOW, NOW + 24 * 60 * 60 * 1000, NOW)
        .await
        .unwrap();
    assert_eq!(titles(&app.week().await), ["빠짐", "오래됨"]);
}

#[tokio::test]
async fn an_episode_in_transmission_is_downloading_until_the_library_holds_it() {
    let app = App::new().await;
    let rule = app
        .subscribe(
            anime(1, "받는 중", 4, Some("10:00"), Some("2026-07-02")),
            self::rule("Work"),
            SubtitleMode::None,
            Some("Empty"),
        )
        .await;
    let observation = |title: &str, hash: &str| Observation {
        channel_id: app.channel.id.clone(),
        channel_label: app.channel.masked_url(),
        identity_key: format!("title:{title}"),
        title: title.to_owned(),
        link: "https://feed.test/item".into(),
        result: HistoryResult::Received,
        rule_id: Some(rule.id.clone()),
        torrent_hash: Some(hash.to_owned()),
        reason: None,
    };
    app.state
        .history
        .record(
            NOW - 1000,
            vec![
                observation("[G] Work - 13 (1080p) [AAAA1111].mkv", "aa"),
                observation("[G] Work - 14v2 (1080p) [BBBB2222].mkv", "bb"),
                observation("[G] Work - 01-14 (1080p) [CCCC3333].mkv", "cc"),
            ],
        )
        .await
        .unwrap();

    app.state
        .status
        .record_cycle_interval(5 * 60_000)
        .await
        .unwrap();
    // Nothing is downloading: the 14th has aired and not come.
    let video = |body: &Value| body["week"]["days"][3]["cards"][0]["video"].clone();
    assert_eq!(video(&app.week().await), "waiting");

    // Episode 13 and a batch are downloading, not the 14th.
    app.state
        .status
        .record_transmission(
            TransmissionCounts {
                downloading: 2,
                seeding: 0,
                taken_at: NOW,
            },
            vec!["aa".into(), "cc".into()],
        )
        .await
        .unwrap();
    assert_eq!(video(&app.week().await), "waiting");

    // The 14th is (a revision of it counts as the episode).
    let record = |taken_at: i64| {
        app.state.status.record_transmission(
            TransmissionCounts {
                downloading: 1,
                seeding: 0,
                taken_at,
            },
            vec!["bb".into()],
        )
    };
    record(NOW).await.unwrap();
    assert_eq!(video(&app.week().await), "downloading");

    // The look is as old as three cycles: still believed. Older: the worker is
    // not looking any more, so the episode is not shown as downloading.
    let cycle = 5 * 60_000;
    record(NOW - 3 * cycle).await.unwrap();
    assert_eq!(video(&app.week().await), "downloading");
    record(NOW - 3 * cycle - 1).await.unwrap();
    assert_eq!(video(&app.week().await), "waiting");
    // A worker that comes back and looks again shows it once more.
    record(NOW).await.unwrap();
    assert_eq!(video(&app.week().await), "downloading");
}

#[tokio::test]
async fn an_episode_stays_downloading_while_a_long_cycle_is_running() {
    let app = App::new().await;
    let rule = app
        .subscribe(
            anime(1, "받는 중", 4, Some("10:00"), Some("2026-07-02")),
            self::rule("Work"),
            SubtitleMode::None,
            Some("Empty"),
        )
        .await;
    app.state
        .history
        .record(
            NOW - 1000,
            vec![Observation {
                channel_id: app.channel.id.clone(),
                channel_label: app.channel.masked_url(),
                identity_key: "title:x".into(),
                title: "[G] Work - 14 (1080p) [AAAA1111].mkv".into(),
                link: "https://feed.test/item".into(),
                result: HistoryResult::Received,
                rule_id: Some(rule.id.clone()),
                torrent_hash: Some("aa".into()),
                reason: None,
            }],
        )
        .await
        .unwrap();
    let minute = 60_000;
    app.state
        .status
        .record_cycle_interval(minute)
        .await
        .unwrap();
    // The worker looked at Transmission four minutes ago, during the cycle
    // before this one.
    app.state
        .status
        .record_transmission(
            TransmissionCounts {
                downloading: 1,
                seeding: 0,
                taken_at: NOW - 4 * minute,
            },
            vec!["aa".into()],
        )
        .await
        .unwrap();
    let video = |body: &Value| body["week"]["days"][3]["cards"][0]["video"].clone();

    // Between cycles that look is more than three intervals old.
    assert_eq!(video(&app.week().await), "waiting");

    // A cycle began a minute after that look and is still running: the worker is
    // alive, so what it saw is kept until the cycle ends.
    assert!(app
        .state
        .history
        .try_begin_cycle(NOW - 3 * minute, 0)
        .await
        .unwrap());
    assert_eq!(video(&app.week().await), "downloading");

    // Once it ends without a newer look, the look is old again.
    app.state.history.finish_cycle(NOW - 1000).await.unwrap();
    assert_eq!(video(&app.week().await), "waiting");

    // A cycle running past the bound is a hung worker: not believed.
    assert!(app
        .state
        .history
        .try_begin_cycle(NOW - 31 * minute, 0)
        .await
        .unwrap());
    assert_eq!(video(&app.week().await), "waiting");
}

#[tokio::test]
async fn nothing_is_downloading_without_a_recorded_cycle_interval() {
    let app = App::new().await;
    let rule = app
        .subscribe(
            anime(1, "받는 중", 4, Some("10:00"), Some("2026-07-02")),
            self::rule("Work"),
            SubtitleMode::None,
            Some("Empty"),
        )
        .await;
    app.state
        .history
        .record(
            NOW - 1000,
            vec![Observation {
                channel_id: app.channel.id.clone(),
                channel_label: app.channel.masked_url(),
                identity_key: "title:x".into(),
                title: "[G] Work - 14 (1080p) [AAAA1111].mkv".into(),
                link: "https://feed.test/item".into(),
                result: HistoryResult::Received,
                rule_id: Some(rule.id.clone()),
                torrent_hash: Some("aa".into()),
                reason: None,
            }],
        )
        .await
        .unwrap();
    app.state
        .status
        .record_transmission(
            TransmissionCounts {
                downloading: 1,
                seeding: 0,
                taken_at: NOW,
            },
            vec!["aa".into()],
        )
        .await
        .unwrap();
    // Without the interval, how old a look may be cannot be told.
    let body = app.week().await;
    assert_eq!(body["week"]["days"][3]["cards"][0]["video"], "waiting");
}

#[tokio::test]
async fn the_next_quarter_counts_its_subscriptions_and_those_waiting_for_a_title() {
    let app = App::new().await;
    // 1분기 2027 starts after this 4분기; one has no title yet.
    app.subscribe(
        anime(1, "내년", 2, Some("22:00"), Some("2027-01-05")),
        rule("N"),
        SubtitleMode::None,
        None,
    )
    .await;
    app.subscribe(
        anime(2, "내년 둘", 3, Some("22:00"), Some("2027-01")),
        RuleInput {
            r#match: None,
            directory: "Next/Season 01".into(),
            ..Default::default()
        },
        SubtitleMode::None,
        None,
    )
    .await;
    // This quarter's does not count.
    app.subscribe(
        anime(3, "이번", 6, Some("22:00"), Some("2026-10-03")),
        rule("T"),
        SubtitleMode::None,
        None,
    )
    .await;

    let body = app.week().await;
    let next = &body["week"]["next_quarter"];
    assert_eq!(next["quarter"], json!({ "year": 2027, "number": 1 }));
    assert_eq!(next["subscriptions"], 2);
    assert_eq!(next["title_waiting"], 1);
    // Neither next-quarter anime airs in this week; this quarter's does, on its day.
    let airing: Vec<&str> = body["week"]["days"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|d| card_titles(d))
        .collect();
    assert_eq!(airing, ["이번"]);
}

#[tokio::test]
async fn an_anime_that_has_not_started_airs_on_its_start_day() {
    let app = App::new().await;
    // 신작 (8) starting Saturday 10-03.
    app.subscribe(
        anime(1, "신작", 8, None, Some("2026-10-03")),
        rule("N"),
        SubtitleMode::None,
        None,
    )
    .await;
    let body = app.week().await;
    let saturday = &body["week"]["days"][5];
    assert_eq!(saturday["date"], "2026-10-03");
    assert_eq!(card_titles(saturday), ["신작"]);
    assert_eq!(saturday["cards"][0]["time"], Value::Null);
    assert_eq!(saturday["cards"][0]["episode"], 1);
    assert_eq!(saturday["cards"][0]["video"], "upcoming");
}

#[tokio::test]
async fn the_week_changes_with_the_day() {
    let app = App::new().await;
    app.subscribe(
        anime(1, "월요", 1, Some("22:00"), Some("2026-07-06")),
        rule("M"),
        SubtitleMode::None,
        None,
    )
    .await;
    // Sunday evening is still the week of Monday 09-28; Monday 00:30 is the next.
    app.now
        .store(NOW + 3 * 24 * 60 * 60 * 1000, Ordering::SeqCst);
    let body = app.week().await;
    assert_eq!(body["week"]["start"], "2026-09-28");
    assert_eq!(body["week"]["days"][6]["today"], true);
    assert_eq!(card_titles(&body["week"]["days"][0]), ["월요"]);

    app.now.store(
        NOW + 3 * 24 * 60 * 60 * 1000 + 13 * 60 * 60 * 1000,
        Ordering::SeqCst,
    );
    let body = app.week().await;
    assert_eq!(body["week"]["start"], "2026-10-05");
    assert_eq!(body["week"]["days"][0]["today"], true);
    assert_eq!(body["week"]["days"][0]["cards"][0]["episode"], 14);
}
