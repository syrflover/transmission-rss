//! `GET /api/collect/status?tz_offset=<minutes>`: what the status board above
//! the collection tabs shows.
//!
//! ```json
//! {
//!   "now": 1790000000000,
//!   "channels": [{ "id", "name", "host", "ok": true|false|null, "read_at", "ok_at" }],
//!   "received": { "total": 12, "days": [{ "date": "2026-09-24", "count": 3 }, ...7] },
//!   "problems": 2,
//!   "transmission": { "downloading": 1, "seeding": 3, "taken_at": 1790000000000 } | null,
//!   "cycle": { "started_at", "finished_at", "next_at", "stalled" } | null,
//!   "collect_folder_set": true|false
//! }
//! ```
//!
//! The web neither reads RSS feeds nor asks Transmission. Channel read status
//! and Transmission's counts are snapshots the worker leaves each cycle
//! ([`trss_collect::store::status`]); each carries the time it was taken and `null`
//! means the worker has not recorded one (yet). The received counts and the
//! failure count come from the collection history.
//!
//! `tz_offset` is the viewer's offset from UTC in minutes, east positive, and
//! decides where a day begins for the seven bars (the seven days ending with
//! today, oldest first). `received.total` is the sum of the bars and
//! `problems` counts items that ended as failed, of unknown version or
//! duplicate within the same seven days.
//!
//! `collect_folder_set` is false while the app's collect folder has not been
//! chosen. The worker then adds no torrent, and the board says so: a rule's
//! folder is relative to it, so there is nowhere to save. Nothing is recorded
//! as failed meanwhile, so what a rule picks is received once the folder is set.

use axum::{
    extract::{rejection::QueryRejection, Query, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{ApiError, AppState};
use trss_collect::store::{channels::ChannelError, history::HistoryError, status::StatusError};
use trss_core::{heartbeat::cycle_stalled, Millis};

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new().route("/collect/status", get(status))
}

const DAY_MS: i64 = 86_400_000;
const DAYS: i64 = 7;
/// The largest offset any place has (UTC+14), in minutes.
const MAX_OFFSET_MINUTES: i64 = 14 * 60;

#[derive(Deserialize)]
struct StatusQuery {
    /// Minutes east of UTC. Absent means UTC.
    #[serde(default)]
    tz_offset: i64,
}

#[derive(Debug, Serialize)]
pub struct ChannelStatus {
    pub id: String,
    pub name: Option<String>,
    pub host: String,
    /// Whether the worker's last attempt to read the feed worked; `null` if it
    /// has not tried yet.
    pub ok: Option<bool>,
    /// When the worker last tried.
    pub read_at: Option<Millis>,
    /// When a read last worked.
    pub ok_at: Option<Millis>,
}

#[derive(Debug, Serialize)]
pub struct Day {
    /// `YYYY-MM-DD` in the viewer's time zone.
    pub date: String,
    pub count: u32,
}

#[derive(Debug, Serialize)]
pub struct Received {
    pub total: u32,
    pub days: Vec<Day>,
}

#[derive(Debug, Serialize)]
pub struct TransmissionStatus {
    pub downloading: u32,
    pub seeding: u32,
    /// When the worker took these counts.
    pub taken_at: Millis,
}

#[derive(Debug, Serialize)]
pub struct CycleStatus {
    pub started_at: Millis,
    /// `null` while that cycle runs or if the worker died in it.
    pub finished_at: Option<Millis>,
    /// When the next cycle is due: the start plus the interval the worker
    /// recorded when it started. `null` while no worker has recorded one.
    pub next_at: Option<Millis>,
    /// The worker is not checking the feeds, and the board says so instead of a
    /// past time: its heartbeat has stopped while a cycle never ended or the
    /// next cycle is more than one interval overdue, or it has held the cycle
    /// lock past the bound while still beating. A worker that is busy
    /// (beating), however long its cycle and the folder reading after it have
    /// taken, is not stalled. The rule is
    /// [`trss_core::heartbeat::cycle_stalled`]; this API only passes the stored
    /// cycle and heartbeat, and says `false` while no interval is recorded.
    pub stalled: bool,
}

#[derive(Debug, Serialize)]
pub struct Board {
    pub now: Millis,
    pub channels: Vec<ChannelStatus>,
    pub received: Received,
    pub problems: u32,
    pub transmission: Option<TransmissionStatus>,
    pub cycle: Option<CycleStatus>,
    /// False until the collect folder is chosen; the worker adds nothing then.
    pub collect_folder_set: bool,
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

async fn status(
    State(state): State<AppState>,
    query: Result<Query<StatusQuery>, QueryRejection>,
) -> Result<Json<Board>, ApiError> {
    let Query(q) = query.map_err(|_| ApiError::invalid("조회 조건을 읽지 못했어요."))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as Millis);
    Ok(Json(board(&state, now, q.tz_offset).await?))
}

/// The board as of `now` (Unix ms) for a viewer `tz_offset` minutes east of UTC.
pub async fn board(state: &AppState, now: Millis, tz_offset: i64) -> Result<Board, ApiError> {
    let offset_ms = tz_offset.clamp(-MAX_OFFSET_MINUTES, MAX_OFFSET_MINUTES) * 60_000;
    let today = (now + offset_ms).div_euclid(DAY_MS);
    let first_day = today - (DAYS - 1);
    let since = first_day * DAY_MS - offset_ms;

    let channels = state
        .channels
        .list_channels()
        .await
        .map_err(|e: ChannelError| internal(e))?;
    let reads = state
        .status
        .channel_reads()
        .await
        .map_err(|e: StatusError| internal(e))?;
    let channels = channels
        .into_iter()
        .map(|channel| {
            let read = reads.iter().find(|r| r.channel_id == channel.id);
            ChannelStatus {
                host: Url::parse(&channel.url)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned))
                    .unwrap_or_default(),
                name: channel.name,
                ok: read.map(|r| r.ok),
                read_at: read.map(|r| r.read_at),
                ok_at: read.and_then(|r| r.ok_at),
                id: channel.id,
            }
        })
        .collect();

    let mut counts = vec![0u32; DAYS as usize];
    for at in state.status.received_since(since).await.map_err(internal)? {
        let day = (at + offset_ms).div_euclid(DAY_MS);
        if (first_day..=today).contains(&day) {
            counts[(day - first_day) as usize] += 1;
        }
    }
    let received = Received {
        total: counts.iter().sum(),
        days: counts
            .into_iter()
            .enumerate()
            .map(|(i, count)| Day {
                date: civil_date(first_day + i as i64),
                count,
            })
            .collect(),
    };

    let transmission = state
        .status
        .transmission()
        .await
        .map_err(internal)?
        .map(|t| TransmissionStatus {
            downloading: t.downloading,
            seeding: t.seeding,
            taken_at: t.taken_at,
        });
    let interval = state.status.cycle_interval().await.map_err(internal)?;
    let heartbeat = state.heartbeat.read().await.map_err(internal)?;
    let cycle = state
        .history
        .last_cycle()
        .await
        .map_err(|e: HistoryError| internal(e))?
        .map(|c| CycleStatus {
            started_at: c.started_at,
            finished_at: c.finished_at,
            next_at: interval.map(|ms| c.started_at.saturating_add(ms)),
            stalled: interval
                .is_some_and(|ms| cycle_stalled(c.into(), heartbeat.as_ref(), ms, now)),
        });

    let collect_folder_set = state
        .settings
        .collection()
        .await
        .map_err(internal)?
        .is_some();

    Ok(Board {
        now,
        channels,
        received,
        problems: state.status.problems_since(since).await.map_err(internal)?,
        transmission,
        cycle,
        collect_folder_set,
    })
}

/// `YYYY-MM-DD` of a day counted from 1970-01-01 (proleptic Gregorian; the
/// algorithm is Howard Hinnant's `civil_from_days`).
fn civil_date(days_since_epoch: i64) -> String {
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod date_tests {
    use super::civil_date;

    #[test]
    fn days_since_the_epoch_become_calendar_dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(-1), "1969-12-31");
        assert_eq!(civil_date(19_723), "2024-01-01");
        assert_eq!(civil_date(19_782), "2024-02-29");
        assert_eq!(civil_date(19_783), "2024-03-01");
        assert_eq!(civil_date(20_362), "2025-10-01");
    }
}
