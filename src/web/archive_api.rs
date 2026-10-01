//! `/api/archive-suggestions`: the rules the app suggests archiving
//! (`docs/specs/collection.md`, 규칙 보관).
//!
//! | call                                  | success                                     |
//! | ------------------------------------- | ------------------------------------------- |
//! | `GET /archive-suggestions`            | `200 { suggestions: [SuggestionView] }`     |
//! | `POST /archive-suggestions/keep`      | `200 { kept: N }` (`수집 유지`)             |
//!
//! A suggestion is read off the rules, the stored Anissia snapshots and the
//! history each time ([`crate::archive_suggestions`] has the grounds and the
//! rules of when one appears and goes); nothing here asks Anissia or the feed.
//! [`archive_suggestions`] is the source the 할 일 list reads for its
//! `보관 제안` suggestions. A suggestion is not something that needs doing, so
//! nothing counts these in a menu badge.
//!
//! Archiving is not done here. The screen sends the rule's `rule_archive`
//! command (`/api/commands`) for each rule, in order, so the work folder moves
//! as the command's own rules say (the folder shared by several of the rules
//! moves once, with the last of them); every view carries `after`, the
//! sentence for what archiving that rule would do with its folder now.
//!
//! `POST /archive-suggestions/keep` (`{ rule_id, grounds }`, the `key`s of the
//! grounds the user saw) remembers `수집 유지` for those grounds of the rule:
//! it is not suggested again on them, and a ground that is new to it is. It
//! changes nothing else about the rule.

use std::collections::HashMap;

use axum::{
    extract::{rejection::JsonRejection, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{subscriptions_api::AnimeView, ApiError, AppState};
use crate::{
    archive_suggestions::{
        recent_matches, ArchiveSuggestion, Facts, Ground, Recent, WINDOW_TITLES,
    },
    schedule::slot::Over,
    store::{
        channels::{ChannelWithRules, Rule, RuleState},
        history::Millis,
    },
    worker::commands::rule_archive,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/archive-suggestions", get(list))
        .route("/archive-suggestions/keep", post(keep))
}

const BAD_BODY: &str = "요청 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

/// What the suggestions were read from, kept for the views.
struct Gathered {
    all: Vec<ChannelWithRules>,
    animes: HashMap<i64, crate::store::anissia::Anime>,
    suggestions: Vec<ArchiveSuggestion>,
    last_received: HashMap<String, Millis>,
}

/// Gathers the facts and decides. Bounded work: one indexed lookup per rule for
/// its last receive, one per channel for its first read and one for its read
/// days (at most 28 rows), and, only for the channels with a rule that is
/// quiet and not kept, the titles since the start of its window (the last 4
/// weeks, or back to the first of its last 28 read days when the feed was not
/// read on some of them; at most [`WINDOW_TITLES`] of them, read through the
/// channel's index), never the whole history.
async fn gather(state: &AppState) -> Result<Gathered, ApiError> {
    let now = state.anissia.now();
    let all = state
        .channels
        .list_channels_with_rules()
        .await
        .map_err(ApiError::from)?;
    let rule_ids: Vec<String> = all
        .iter()
        .flat_map(|c| &c.rules)
        .filter(|r| r.state != RuleState::Archived && r.r#match.is_some())
        .map(|r| r.id.clone())
        .collect();
    if rule_ids.is_empty() {
        return Ok(Gathered {
            all,
            animes: HashMap::new(),
            suggestions: Vec::new(),
            last_received: HashMap::new(),
        });
    }
    let nos: Vec<i64> = all
        .iter()
        .flat_map(|c| &c.rules)
        .filter(|r| rule_ids.contains(&r.id))
        .filter_map(|r| r.subscription.as_ref().map(|s| s.anissia_anime_no))
        .collect();
    let animes = state
        .anissia
        .store
        .animes(nos.clone())
        .await
        .map_err(internal)?;
    let unlisted = state.anissia.store.unlisted(nos).await.map_err(internal)?;
    let last_received = state
        .history
        .last_received_of_rules(rule_ids)
        .await
        .map_err(internal)?;
    let started = state.channels.rule_starts().await.map_err(ApiError::from)?;
    let first_read = state
        .history
        .first_sightings(all.iter().map(|c| c.channel.id.clone()).collect())
        .await
        .map_err(internal)?;
    let read_floors = state
        .status
        .read_day_floors(all.iter().map(|c| c.channel.id.clone()).collect(), now)
        .await
        .map_err(internal)?;
    let kept = state
        .channels
        .kept_archive_grounds()
        .await
        .map_err(ApiError::from)?;

    let facts = Facts {
        now,
        channels: &all,
        animes: &animes,
        unlisted: &unlisted,
        last_received: &last_received,
        started: &started,
        first_read: &first_read,
        read_floors: &read_floors,
        kept: &kept,
    };

    let mut recent: HashMap<String, Recent> = HashMap::new();
    for channel_id in facts.channels_to_read() {
        let (titles, truncated) = state
            .history
            .titles_since(
                channel_id.clone(),
                facts.window_start(&channel_id),
                WINDOW_TITLES,
            )
            .await
            .map_err(internal)?;
        if truncated {
            eprintln!(
                "Archive suggestions: channel {channel_id} recorded more than {WINDOW_TITLES} items in the last 4 weeks; its rules get no `새 항목 없음` suggestion"
            );
        }
        let Some(cwr) = all.iter().find(|c| c.channel.id == channel_id).cloned() else {
            continue;
        };
        // Matching every title against every rule is the work of a moment, but
        // not one to hold the runtime for.
        let matched = tokio::task::spawn_blocking(move || recent_matches(&cwr, &titles, truncated))
            .await
            .map_err(internal)?;
        recent.insert(channel_id, matched);
    }
    let suggestions = facts.suggestions(&recent);
    Ok(Gathered {
        all,
        animes,
        suggestions,
        last_received,
    })
}

/// The rules to suggest archiving as they are now, with the grounds that are
/// new to the user, in the order of the channels and their rules. This is what
/// the 할 일 list reads for its `보관 제안` suggestions.
pub async fn archive_suggestions(state: &AppState) -> Result<Vec<ArchiveSuggestion>, ApiError> {
    Ok(gather(state).await?.suggestions)
}

#[derive(Debug, Serialize)]
struct GroundView {
    /// What `POST /archive-suggestions/keep` names the ground by.
    key: String,
    /// `ended` (the anime's end date has passed), `unlisted` (it has none and
    /// Anissia no longer lists the anime) or `quiet` (no new item for 4 weeks).
    kind: &'static str,
    /// `ended`: the end date as Anissia gave it.
    end_date: Option<String>,
    /// `quiet`: the moment the 4 weeks count from (Unix ms).
    since: Option<Millis>,
}

impl From<&Ground> for GroundView {
    fn from(ground: &Ground) -> Self {
        let (kind, end_date, since) = match ground {
            Ground::Ended {
                over: Over::EndDate(date),
                ..
            } => ("ended", Some(date.clone()), None),
            Ground::Ended {
                over: Over::Unlisted,
                ..
            } => ("unlisted", None, None),
            Ground::Quiet { since, .. } => ("quiet", None, Some(*since)),
        };
        GroundView {
            key: ground.key(),
            kind,
            end_date,
            since,
        }
    }
}

#[derive(Debug, Serialize)]
struct SuggestionView {
    rule_id: String,
    channel_id: String,
    channel_name: Option<String>,
    channel_host: String,
    /// `active` or `paused`.
    state: &'static str,
    /// The rule's match phrase.
    title: String,
    directory: String,
    /// The anime a subscription follows, as the stored snapshot lists it.
    anime: Option<AnimeView>,
    /// When the rule last got an item into Transmission; `null` if it never did.
    last_received_at: Option<Millis>,
    /// Why, the end of the anime first.
    grounds: Vec<GroundView>,
    /// What archiving the rule would do with its work folder now.
    after: String,
}

#[derive(Debug, Serialize)]
struct SuggestionList {
    suggestions: Vec<SuggestionView>,
}

async fn list(State(state): State<AppState>) -> Result<Json<SuggestionList>, ApiError> {
    let gathered = gather(&state).await?;
    if gathered.suggestions.is_empty() {
        return Ok(Json(SuggestionList {
            suggestions: Vec::new(),
        }));
    }
    let settings = state
        .settings
        .collection()
        .await
        .map_err(internal)?
        .map(|s| (s.folder, s.archive_folder));
    let rules: Vec<Rule> = gathered
        .all
        .iter()
        .flat_map(|c| c.rules.iter().cloned())
        .collect();

    let views = gathered
        .suggestions
        .iter()
        .filter_map(|suggestion| {
            let cwr = gathered
                .all
                .iter()
                .find(|c| c.channel.id == suggestion.channel_id)?;
            let rule = cwr.rules.iter().find(|r| r.id == suggestion.rule_id)?;
            let host = Url::parse(&cwr.channel.url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_default();
            Some(SuggestionView {
                rule_id: rule.id.clone(),
                channel_id: cwr.channel.id.clone(),
                channel_name: cwr.channel.name.clone(),
                channel_host: host,
                state: rule.state.as_str(),
                title: rule.r#match.clone().unwrap_or_default(),
                directory: rule.directory.clone(),
                anime: rule
                    .subscription
                    .as_ref()
                    .and_then(|s| gathered.animes.get(&s.anissia_anime_no))
                    .map(AnimeView::from),
                last_received_at: gathered.last_received.get(&rule.id).copied(),
                grounds: suggestion.grounds.iter().map(GroundView::from).collect(),
                after: rule_archive::forecast_archive(settings.clone(), &rules, rule),
            })
        })
        .collect();
    Ok(Json(SuggestionList { suggestions: views }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeepBody {
    rule_id: String,
    /// The `key`s of the grounds the user saw.
    grounds: Vec<String>,
}

#[derive(Serialize)]
struct Kept {
    kept: usize,
}

/// The most grounds one rule can have at once, with room to spare.
const MAX_GROUNDS: usize = 8;
const MAX_GROUND_LEN: usize = 200;

async fn keep(
    State(state): State<AppState>,
    parsed: Result<Json<KeepBody>, JsonRejection>,
) -> Result<Json<Kept>, ApiError> {
    let Json(body) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;
    let known = |ground: &str| ground.len() <= MAX_GROUND_LEN && is_ground_key(ground);
    if body.grounds.is_empty()
        || body.grounds.len() > MAX_GROUNDS
        || !body.grounds.iter().all(|g| known(g))
    {
        return Err(ApiError::invalid(BAD_BODY));
    }
    state
        .channels
        .keep_archive_grounds(&body.rule_id, body.grounds.clone(), state.anissia.now())
        .await
        .map_err(ApiError::from)?;
    Ok(Json(Kept {
        kept: body.grounds.len(),
    }))
}

/// Whether `key` has the shape of a [`crate::archive_suggestions::Ground::key`]:
/// `ended:<anime>:<date>`, `unlisted:<anime>` or `quiet:<ms>`.
fn is_ground_key(key: &str) -> bool {
    let number = |s: &str| s.parse::<i64>().is_ok();
    match key.split(':').collect::<Vec<_>>().as_slice() {
        ["ended", anime, date] => {
            number(anime)
                && !date.is_empty()
                && date.chars().all(|c| c.is_ascii_digit() || c == '-')
        }
        ["unlisted", anime] => number(anime),
        ["quiet", since] => number(since),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
