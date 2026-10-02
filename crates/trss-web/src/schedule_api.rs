//! `GET /api/schedule/week`: what the home screen, `이번 주 편성`, shows
//! (`docs/specs/web-app.md`).
//!
//! ```json
//! { "now": 1790780400000,
//!   "first_run": null | { "active": true, "steps": [...] },
//!   "week": null | {
//!     "start": "2026-09-28", "end": "2026-10-04", "today": "2026-10-01",
//!     "quarter": { "year": 2026, "number": 4 },
//!     "days": [{ "date": "2026-09-28", "weekday": 0, "today": false,
//!                "cards": [Card] }, ... 7 days, Monday first],
//!     "next_quarter": { "quarter": { ... }, "subscriptions": 3, "title_waiting": 1 }
//!   } }
//! ```
//!
//! While the first run's checklist is up ([`super::setup_api`]) only
//! `first_run` is sent and `week` is `null`; otherwise `first_run` is `null`.
//! The collect status beside the schedule is the existing
//! `GET /collect/status`, not repeated here.
//!
//! # The cards
//!
//! A card is one subscription that airs in the week. Which subscriptions have
//! one, on which day and at what time, is read from the stored Anissia
//! snapshot in Asia/Seoul ([`trss_anissia::slot`]); the web neither asks
//! Anissia nor Transmission. A subscription has no card when
//!
//! - its rule is archived (a subscription that is no longer followed; the spec
//!   names the active and the paused ones, so the archived ones are left out);
//! - its snapshot puts no airing in the week (before its start, after its end,
//!   `기타`, or a `신작` that starts in another week): an anime with an end
//!   date leaves once the date has passed;
//! - the worker's daily refresh found that Anissia no longer lists the anime
//!   (`unlisted_at`, see [`trss_collect::store::anissia`]): an anime without an end
//!   date leaves this way. While Anissia cannot be reached that is not found
//!   out, so the card stays; nor is it found out from a weekday whose list came
//!   back empty although the anime was last listed in it.
//!
//! The card's `episode` is the season's episode that airs in the slot
//! ([`trss_anissia::slot::episode_on`]); it is `null` when that cannot be
//! told, and for a `결방` card. The status lines come from
//! [`trss_collect::schedule::state`]:
//!
//! - an anime the stored Anissia snapshot marks `OFF` is `video: "off"`
//!   (`결방`) with no subtitle line, whatever the library holds; a paused rule
//!   says `paused` instead. The stand-in snapshot an import keeps while Anissia
//!   could not be asked (status `OFF`, `fetched_at` 0) is not
//!   Anissia's word and is never read as `OFF`. It sits on the weekday and time
//!   the legacy comment gave, so its card shows on that weekday until the
//!   daily refresh replaces it; without a weekday in the comment it is in `기타`
//!   and makes no card.
//! - `video_held` / `subtitle_held`: the season the subscription follows holds a
//!   video / a subtitle for the episode. A subscription whose season is not
//!   connected holds nothing.
//! - `downloading`: a torrent the rule received for the episode (release
//!   episode plus the rule's offset) is among the hashes the worker last saw
//!   Transmission download, provided that look is recent: not older than
//!   [`DOWNLOADING_FRESH_CYCLES`] cycle intervals, so a worker that is down
//!   does not leave an episode downloading for good. History is read only for
//!   the cards' rules, only when something is downloading, and only for the
//!   torrents downloading.
//!
//! `work_id` (the card's link to the work's detail) and `cover_url` exist only
//! for a connected season; without them the card links to the rule and the
//! cover is an empty slot.

use std::collections::{BTreeMap, HashMap, HashSet};

use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;

use super::{
    artwork_api::image_url,
    setup_api::{self, FirstRunView},
    status_api::worker_busy,
    subscriptions_api::{quarter_of, QuarterView},
    ApiError, AppState,
};
use trss_anissia::{
    slot::{episode_on, slot_in_week, Slot},
    Anime,
};
use trss_collect::{
    schedule::state::{self, Facts, SubtitleState, VideoState},
    store::channels::{Rule, RuleState, SeasonRef, SubtitleMode},
    subscriptions::{whole_episode, Quarter},
};
use trss_core::{
    calendar::{date_text, day_of, week_start, weekday},
    Millis,
};
use trss_library::{seasons::combine::air_times, store::library::Held};

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new().route("/schedule/week", get(week))
}

#[derive(Debug, Serialize)]
pub struct Card {
    pub rule_id: String,
    pub anime_no: i64,
    pub title: String,
    /// `HH:MM` (past 24 for late-night programmes), `null` without a time.
    pub time: Option<String>,
    /// When the episode airs (Unix ms).
    pub air_at: Millis,
    pub episode: Option<u32>,
    pub video: VideoState,
    /// `null` when the subscription takes no subtitles or is paused.
    pub subtitle: Option<SubtitleState>,
    /// The subtitle creator being followed.
    pub creator: Option<String>,
    /// The work of the connected season: the card links to its detail.
    pub work_id: Option<String>,
    pub cover_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DayView {
    /// `YYYY-MM-DD`, Asia/Seoul.
    pub date: String,
    /// 0 (Monday) to 6 (Sunday).
    pub weekday: u8,
    pub today: bool,
    pub cards: Vec<Card>,
}

#[derive(Debug, Serialize)]
pub struct NextQuarter {
    pub quarter: QuarterView,
    /// Subscriptions of a coming quarter.
    pub subscriptions: u32,
    /// Of those, the ones that wait for their title.
    pub title_waiting: u32,
}

#[derive(Debug, Serialize)]
pub struct WeekView {
    pub start: String,
    pub end: String,
    pub today: String,
    /// The quarter of `today`.
    pub quarter: QuarterView,
    pub days: Vec<DayView>,
    pub next_quarter: NextQuarter,
}

#[derive(Debug, Serialize)]
pub struct Answer {
    pub now: Millis,
    pub first_run: Option<FirstRunView>,
    pub week: Option<WeekView>,
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

async fn week(State(state): State<AppState>) -> Result<Json<Answer>, ApiError> {
    let now = state.anissia.now();
    if let Some(first_run) = setup_api::checklist(&state).await? {
        return Ok(Json(Answer {
            now,
            first_run: Some(first_run),
            week: None,
        }));
    }
    Ok(Json(Answer {
        now,
        first_run: None,
        week: Some(week_at(&state, now).await?),
    }))
}

/// What a connected season gives the cards of the subscriptions that follow it.
struct SeasonFacts {
    work_id: String,
    held: BTreeMap<u32, Held>,
    air_times: BTreeMap<u32, i64>,
}

/// The seasons the cards follow as the cards need them, by season ID, each read
/// once and all in one query per store. A season that is not one of a work in
/// the library maps to `None`.
async fn season_facts(
    state: &AppState,
    airings: &[Airing<'_>],
) -> Result<HashMap<String, Option<SeasonFacts>>, ApiError> {
    let mut ids: Vec<&str> = Vec::new();
    for airing in airings {
        let id = airing
            .rule
            .subscription
            .as_ref()
            .and_then(|s| s.season_id.as_deref());
        if let Some(id) = id {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    // The ones that name a season, in the order of `ids`.
    let parsed: Vec<(&str, SeasonRef)> = ids
        .into_iter()
        .filter_map(|id| SeasonRef::parse(id).map(|season| (id, season)))
        .collect();
    let wanted: Vec<(String, u32)> = parsed
        .iter()
        .map(|(_, s)| (s.work_id.clone(), s.number))
        .collect();
    let held = state
        .library
        .seasons_episodes(wanted.clone())
        .await
        .map_err(internal)?;
    let links = state
        .seasons
        .store
        .links_of_seasons(wanted)
        .await
        .map_err(internal)?;

    let mut out: HashMap<String, Option<SeasonFacts>> = HashMap::new();
    for (((id, season), held), link) in parsed.into_iter().zip(held).zip(links) {
        let facts = held.map(|held| SeasonFacts {
            work_id: season.work_id,
            held,
            air_times: link.map_or_else(BTreeMap::new, |link| air_times(&link.entries)),
        });
        out.insert(id.to_owned(), facts);
    }
    Ok(out)
}

/// The episode a release title names as a whole number (`12`, `12v2`); not a
/// batch (`01-12`) or a half episode.
fn release_episode(title: &str) -> Option<i64> {
    whole_episode(title).map(i64::from)
}

/// The quarter after `quarter`.
fn next_of(quarter: Quarter) -> Quarter {
    if quarter.number >= 4 {
        Quarter {
            year: quarter.year + 1,
            number: 1,
        }
    } else {
        Quarter {
            year: quarter.year,
            number: quarter.number + 1,
        }
    }
}

/// A subscription that airs in the week, before its card is made.
struct Airing<'a> {
    rule: &'a Rule,
    anime: &'a Anime,
    slot: Slot,
}

/// How many collection cycles old the worker's look at Transmission may be
/// for `영상 받는 중` to be believed. The worker looks every cycle, and a failed
/// look keeps the earlier list, so a worker that is down (or a Transmission it
/// cannot reach) would otherwise leave an episode downloading for good. Three
/// cycles ride out a slow or failed one.
const DOWNLOADING_FRESH_CYCLES: i64 = 3;

/// The torrents Transmission is downloading as of `now`: what the worker saw
/// last, provided that look is not older than [`DOWNLOADING_FRESH_CYCLES`]
/// cycle intervals. Without a recorded interval (no worker of this version has
/// run) nothing is believed.
///
/// While the worker is busy ([`worker_busy`]: it beats while it holds the cycle
/// lock, through the folder reading after the cycle's RSS work too) it is alive,
/// and the look it left is the one before this cycle; a cycle longer than the
/// allowance would otherwise age it out before the cycle can leave a newer
/// one. So the look's age is counted up to the cycle's start. A worker that
/// died in a cycle, or holds the lock past the bound, does not hold the look.
async fn downloading_hashes(state: &AppState, now: Millis) -> Result<HashSet<String>, ApiError> {
    let Some(counts) = state.status.transmission().await.map_err(internal)? else {
        return Ok(HashSet::new());
    };
    let Some(interval) = state.status.cycle_interval().await.map_err(internal)? else {
        return Ok(HashSet::new());
    };
    let last = state.history.last_cycle().await.map_err(internal)?;
    let beat = state.heartbeat.read().await.map_err(internal)?;
    let seen_until = match last {
        Some(cycle) if worker_busy(&cycle, beat.as_ref(), interval, now) => {
            cycle.started_at.min(now)
        }
        _ => now,
    };
    if seen_until.saturating_sub(counts.taken_at)
        > interval.saturating_mul(DOWNLOADING_FRESH_CYCLES)
    {
        return Ok(HashSet::new());
    }
    state.status.downloading_hashes().await.map_err(internal)
}

/// The episodes Transmission is downloading for each card's rule, by rule ID: a
/// torrent the rule received whose release names the episode, shifted by the
/// rule's episode offset.
async fn downloading_by_rule<'a>(
    state: &AppState,
    now: Millis,
    airings: &[Airing<'a>],
) -> Result<HashMap<&'a str, HashSet<i64>>, ApiError> {
    let mut downloading: HashMap<&str, HashSet<i64>> = HashMap::new();
    if airings.is_empty() {
        return Ok(downloading);
    }
    let hashes = downloading_hashes(state, now).await?;
    if hashes.is_empty() {
        return Ok(downloading);
    }
    let ids: Vec<String> = airings.iter().map(|a| a.rule.id.clone()).collect();
    let received = state
        .history
        .received_titles_of_rules(ids, hashes.iter().cloned().collect())
        .await
        .map_err(internal)?;
    for airing in airings {
        for (hash, title) in received.get(&airing.rule.id).into_iter().flatten() {
            if !hashes.contains(hash) {
                continue;
            }
            if let Some(episode) = release_episode(title) {
                downloading
                    .entry(airing.rule.id.as_str())
                    .or_default()
                    .insert(episode + airing.rule.episode);
            }
        }
    }
    Ok(downloading)
}

/// The week `now` (Unix ms) is in, as the home screen shows it.
pub async fn week_at(state: &AppState, now: Millis) -> Result<WeekView, ApiError> {
    let today = day_of(now);
    let start = week_start(today);
    let current = Quarter::at(now);
    let next = next_of(current);

    let all = state
        .channels
        .list_channels_with_rules()
        .await
        .map_err(ApiError::from)?;
    let rules: Vec<&Rule> = all
        .iter()
        .flat_map(|c| &c.rules)
        .filter(|r| r.state != RuleState::Archived && r.subscription.is_some())
        .collect();
    let nos: Vec<i64> = rules
        .iter()
        .filter_map(|r| r.subscription.as_ref().map(|s| s.anissia_anime_no))
        .collect();
    let animes = state
        .anissia_store
        .animes(nos.clone())
        .await
        .map_err(internal)?;
    let unlisted = state.anissia_store.unlisted(nos).await.map_err(internal)?;

    let (mut coming, mut title_waiting) = (0, 0);
    let mut airings = Vec::new();
    for rule in &rules {
        let Some(subscription) = &rule.subscription else {
            continue;
        };
        let anime = animes.get(&subscription.anissia_anime_no);
        if quarter_of(anime, subscription.subscribed_at) > current {
            coming += 1;
            if rule.r#match.is_none() {
                title_waiting += 1;
            }
        }
        let Some(anime) = anime else { continue };
        // An end date says when the anime leaves (see `slot_in_week`); only
        // one without leaves when Anissia no longer lists it.
        if anime.end_date.is_none() && unlisted.contains(&anime.anime_no) {
            continue;
        }
        if let Some(slot) = slot_in_week(anime, start) {
            airings.push(Airing { rule, anime, slot });
        }
    }

    let seasons = season_facts(state, &airings).await?;
    let work_ids: Vec<String> = seasons
        .values()
        .flatten()
        .map(|s| s.work_id.clone())
        .collect();
    let covers = if work_ids.is_empty() {
        HashMap::new()
    } else {
        state
            .artwork
            .store
            .image_ids_of(work_ids)
            .await
            .map_err(internal)?
    };
    let downloading = downloading_by_rule(state, now, &airings).await?;

    let mut by_day: HashMap<i64, Vec<Card>> = HashMap::new();
    for Airing { rule, anime, slot } in airings {
        let Some(subscription) = &rule.subscription else {
            continue;
        };
        let season = subscription
            .season_id
            .as_deref()
            .and_then(|id| seasons.get(id))
            .and_then(Option::as_ref);
        let no_times = BTreeMap::new();
        let off = state::is_off(anime);
        // A `결방` week airs no episode, and a count from the start date would
        // name one that did not.
        let episode = if off {
            None
        } else {
            episode_on(anime, &slot, season.map_or(&no_times, |s| &s.air_times))
        };
        let held = episode
            .and_then(|e| season.and_then(|s| s.held.get(&e)))
            .copied()
            .unwrap_or_default();
        let facts = Facts {
            now,
            instant: slot.instant,
            paused: rule.state == RuleState::Paused,
            off,
            subtitles: subscription.subtitles,
            video_held: held.video,
            subtitle_held: held.subtitle,
            downloading: episode.is_some_and(|e| {
                downloading
                    .get(rule.id.as_str())
                    .is_some_and(|set| set.contains(&i64::from(e)))
            }),
        };
        by_day.entry(slot.day).or_default().push(Card {
            rule_id: rule.id.clone(),
            anime_no: anime.anime_no,
            title: anime.subject.clone(),
            time: slot.time.clone(),
            air_at: slot.instant,
            episode,
            video: state::video(&facts),
            subtitle: state::subtitle(&facts),
            creator: (subscription.subtitles == SubtitleMode::Follow)
                .then(|| subscription.creator.clone())
                .flatten(),
            work_id: season.map(|s| s.work_id.clone()),
            cover_url: season.and_then(|s| {
                covers
                    .get(&s.work_id)
                    .map(|image_id| image_url(&s.work_id, image_id))
            }),
        });
    }

    let days = (0..7)
        .map(|offset| {
            let day = start + offset;
            let mut cards = by_day.remove(&day).unwrap_or_default();
            cards.sort_by(|a, b| a.air_at.cmp(&b.air_at).then_with(|| a.title.cmp(&b.title)));
            DayView {
                date: date_text(day),
                weekday: weekday(day),
                today: day == today,
                cards,
            }
        })
        .collect();

    Ok(WeekView {
        start: date_text(start),
        end: date_text(start + 6),
        today: date_text(today),
        quarter: current.into(),
        days,
        next_quarter: NextQuarter {
            quarter: next.into(),
            subscriptions: coming,
            title_waiting,
        },
    })
}
