use axum::{
    http::{Method, StatusCode},
    Router,
};
use serde_json::Value;

use super::*;
use crate::testing;
use trss_collect::store::{
    channels::ChannelInput,
    history::{HistoryResult, Observation},
    status::{ChannelReadResult, TransmissionCounts},
};
use trss_core::Db;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 3_600_000;
/// 2026-09-30 12:00:00 UTC.
const NOON: i64 = 1_790_769_600_000;

fn app() -> (AppState, Router) {
    let state = AppState::new(Db::open_blocking(":memory:").unwrap());
    let router = testing::api(&state);
    (state, router)
}

async fn get(router: &Router, uri: &str) -> (StatusCode, Value) {
    testing::call(router, Method::GET, uri, None).await
}

fn observation(key: &str, result: HistoryResult) -> Observation {
    Observation {
        channel_id: "c1".into(),
        channel_label: "https://feed.test/".into(),
        identity_key: key.into(),
        title: key.into(),
        link: "https://feed.test/x".into(),
        result,
        rule_id: None,
        torrent_hash: None,
        reason: None,
    }
}

#[tokio::test]
async fn an_empty_installation_has_no_snapshots_and_seven_zero_days() {
    let (state, _) = app();
    let board = board(&state, NOON, 0).await.unwrap();
    assert!(board.channels.is_empty());
    assert!(board.transmission.is_none());
    assert!(board.cycle.is_none());
    assert_eq!(board.received.total, 0);
    let dates: Vec<&str> = board
        .received
        .days
        .iter()
        .map(|d| d.date.as_str())
        .collect();
    assert_eq!(
        dates,
        [
            "2026-09-24",
            "2026-09-25",
            "2026-09-26",
            "2026-09-27",
            "2026-09-28",
            "2026-09-29",
            "2026-09-30"
        ]
    );
    assert!(board.received.days.iter().all(|d| d.count == 0));
    assert_eq!(board.problems, 0);
}

#[tokio::test]
async fn channels_show_the_workers_last_read_and_a_channel_never_read_is_unknown() {
    let (state, router) = app();
    let mut named = ChannelInput::new("https://feed-a.test/rss?token=SECRETVALUE99");
    named.name = Some("주간 애니".into());
    let a = state.channels.create_channel(named).await.unwrap();
    let b = state
        .channels
        .create_channel(ChannelInput::new(
            "https://feed-b.test/rss?token=SECRETVALUE99",
        ))
        .await
        .unwrap();
    let c = state
        .channels
        .create_channel(ChannelInput::new("https://feed-c.test/rss"))
        .await
        .unwrap();
    let all = vec![a.id.clone(), b.id.clone(), c.id.clone()];
    state
        .status
        .record_reads(
            NOON - HOUR,
            vec![
                ChannelReadResult {
                    channel_id: a.id.clone(),
                    ok: true,
                },
                ChannelReadResult {
                    channel_id: b.id.clone(),
                    ok: true,
                },
            ],
            all.clone(),
        )
        .await
        .unwrap();
    state
        .status
        .record_reads(
            NOON,
            vec![ChannelReadResult {
                channel_id: b.id.clone(),
                ok: false,
            }],
            all,
        )
        .await
        .unwrap();

    let (status, json) = get(&router, "/api/collect/status").await;
    assert_eq!(status, StatusCode::OK);
    let channels = json["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 3);
    assert_eq!(channels[0]["name"], "주간 애니");
    assert_eq!(channels[0]["host"], "feed-a.test");
    assert_eq!(channels[1]["ok"], false);
    assert_eq!(channels[1]["read_at"], NOON);
    assert_eq!(channels[1]["ok_at"], NOON - HOUR);
    assert_eq!(channels[2]["ok"], Value::Null);
    assert_eq!(channels[2]["read_at"], Value::Null);
    // No secret value of any channel URL is in the response.
    assert!(!json.to_string().contains("SECRETVALUE99"));
}

#[tokio::test]
async fn received_items_are_counted_per_day_of_the_viewers_time_zone() {
    let (state, _) = app();
    // In UTC+9 (540 minutes) the local day changes at 15:00 UTC.
    let today_9am_local = NOON - 3 * HOUR; // 09:00 UTC = 18:00 local, still 09-30 local
    let after_local_midnight = NOON + 4 * HOUR; // 16:00 UTC = 01:00 local on 10-01
    let long_ago = NOON - 8 * 24 * HOUR;
    state
        .history
        .record(
            long_ago,
            vec![observation("ancient", HistoryResult::Received)],
        )
        .await
        .unwrap();
    state
        .history
        .record(
            today_9am_local,
            vec![
                observation("a", HistoryResult::Received),
                observation("b", HistoryResult::Received),
                observation("dup", HistoryResult::Duplicate),
            ],
        )
        .await
        .unwrap();
    state
        .history
        .record(
            NOON - 2 * 24 * HOUR,
            vec![
                observation("c", HistoryResult::Received),
                observation("fail", HistoryResult::AddFailed),
            ],
        )
        .await
        .unwrap();

    // Viewed at NOON UTC in UTC+9 (21:00 local on 09-30).
    let board = board(&state, NOON, 540).await.unwrap();
    let counts: Vec<u32> = board.received.days.iter().map(|d| d.count).collect();
    assert_eq!(counts, [0, 0, 0, 0, 1, 0, 2]);
    assert_eq!(
        board.received.total, 3,
        "the item 8 days ago is outside the window"
    );
    assert_eq!(board.received.days[6].date, "2026-09-30");
    assert_eq!(
        board.problems, 2,
        "one duplicate and one failed inside the window"
    );

    // A viewer 5 hours later in the same zone sees the next local day start.
    let later = super::board(&state, after_local_midnight, 540)
        .await
        .unwrap();
    assert_eq!(later.received.days[6].date, "2026-10-01");
    assert_eq!(later.received.days[5].count, 2);
}

#[tokio::test]
async fn transmission_counts_and_the_last_cycle_are_reported_with_their_time() {
    let (state, router) = app();
    state
        .status
        .record_transmission(
            TransmissionCounts {
                downloading: 1,
                seeding: 3,
                taken_at: NOON,
            },
            Vec::new(),
        )
        .await
        .unwrap();
    assert!(state.history.try_begin_cycle(NOON - HOUR, 0).await.unwrap());
    state
        .history
        .finish_cycle(NOON - HOUR + 1_000)
        .await
        .unwrap();

    // Before any worker recorded its interval the next check is not known.
    let (_, json) = get(&router, "/api/collect/status?tz_offset=540").await;
    assert_eq!(json["cycle"]["next_at"], serde_json::Value::Null);
    state
        .status
        .record_cycle_interval(5 * 60_000)
        .await
        .unwrap();

    let (status, json) = get(&router, "/api/collect/status?tz_offset=540").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["transmission"]["downloading"], 1);
    assert_eq!(json["transmission"]["seeding"], 3);
    assert_eq!(json["transmission"]["taken_at"], NOON);
    assert_eq!(json["cycle"]["started_at"], NOON - HOUR);
    assert_eq!(json["cycle"]["finished_at"], NOON - HOUR + 1_000);
    assert_eq!(json["cycle"]["next_at"], NOON - HOUR + 5 * 60_000);
    assert_eq!(json["received"]["days"].as_array().unwrap().len(), 7);
}

#[tokio::test]
async fn without_a_recorded_interval_nothing_is_stalled() {
    let (state, _router) = app();
    assert!(state.history.try_begin_cycle(NOON - HOUR, 0).await.unwrap());
    state
        .history
        .finish_cycle(NOON - HOUR + 1_000)
        .await
        .unwrap();
    let stalled = |board: Board| board.cycle.unwrap().stalled;

    // Without a recorded interval nothing is known to be late (with one, the
    // same cycle is late: trss-core `heartbeat` tests it).
    assert!(!stalled(board(&state, NOON, 0).await.unwrap()));
}

/// Records a five-minute interval and a cycle that began `ago` before `NOON`
/// and, if `finished`, ended a minute later.
async fn cycle_of_five_minutes(state: &AppState, ago: i64, finished: bool) {
    state
        .status
        .record_cycle_interval(5 * MINUTE)
        .await
        .unwrap();
    assert!(state.history.try_begin_cycle(NOON - ago, 0).await.unwrap());
    if finished {
        state
            .history
            .finish_cycle(NOON - ago + MINUTE)
            .await
            .unwrap();
    }
}

fn stalled_at(board: Board) -> bool {
    board.cycle.unwrap().stalled
}

#[tokio::test]
async fn a_worker_killed_in_a_cycle_is_stalled_once_its_heartbeat_is_stale() {
    let (state, _router) = app();
    // The cycle began ten minutes ago and never ended: well within the half
    // hour a cycle may take, so only the heartbeat tells it from a slow one.
    cycle_of_five_minutes(&state, 10 * MINUTE, false).await;

    // It was killed: the beat stopped two minutes ago, and the board says so.
    state
        .heartbeat
        .record(NOON - 2 * MINUTE, Some(NOON - 10 * MINUTE))
        .await
        .unwrap();
    assert!(stalled_at(board(&state, NOON, 0).await.unwrap()));
}

#[tokio::test]
async fn a_bad_offset_is_refused_and_a_huge_one_is_clamped() {
    let (state, router) = app();
    let (status, json) = get(&router, "/api/collect/status?tz_offset=abc").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"], "invalid");

    let clamped = board(&state, NOON, 100_000).await.unwrap();
    let edge = board(&state, NOON, 14 * 60).await.unwrap();
    assert_eq!(
        clamped
            .received
            .days
            .iter()
            .map(|d| d.date.clone())
            .collect::<Vec<_>>(),
        edge.received
            .days
            .iter()
            .map(|d| d.date.clone())
            .collect::<Vec<_>>()
    );
}
