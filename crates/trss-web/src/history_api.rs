//! `/api/history`: the collection history for the 기록 tab.
//!
//! | call                 | success                                                     |
//! | -------------------- | ----------------------------------------------------------- |
//! | `GET /history`       | `200 { items: [HistoryItemView], next, counts }`            |
//! | `GET /history/{id}`  | `200 HistoryItemView`                                       |
//!
//! Query of the list:
//!
//! - `result`: one or more result codes separated by commas
//!   (`add_failed,duplicate`); none means every result;
//! - `channel`: a channel ID;
//! - `after`: the previous page's `next`, an opaque cursor;
//! - `limit`: page size, 50 by default, at most 200.
//!
//! Items come newest first (by the time they were first seen). A cursor names
//! the last item of a page, so items that arrive later, or results that change
//! while the screen scrolls, do not move what was already loaded. `counts`
//! are the number of records per result for the channel filter (not for the
//! result filter), so the filter chips can show how many each holds.
//!
//! An item carries `command` while a `receive_once` command for it is
//! pending or running, so a screen that is opened or reloaded after `다시
//! 받기` still shows it as in progress. `can_retry` says whether `다시 받기` is
//! offered (see [`receive_once::retry_plan`]); for an item a rule picked and
//! failed to add but cannot retry, `retry_blocked` says why. A `버전 미상` item
//! whose episode is known to hold its revision or a higher one already has no
//! button either, and `retry_blocked` says so ([`super::in_place`]). The
//! item's own link is never sent: it
//! is only a masked copy, and nothing on the screen needs it.
//!
//! An item that matched no rule carries `name_title` when its channel has a
//! subscription waiting for a title: the work part of its title and those
//! subscriptions, so that the row can lead straight to naming one of them (see
//! `/api/rules/{id}/title`). Without such a subscription it has none.

use std::collections::HashMap;

use axum::{
    extract::{rejection::QueryRejection, Path, Query, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    commands_api::CommandView,
    in_place,
    subscriptions_api::{waiting_views, WaitingView},
    ApiError, AppState,
};
use trss_legacy::{
    store::{
        anissia::Anime,
        channels::{Channel, ChannelWithRules, Rule},
        history::{HistoryCursor, HistoryItem, HistoryQuery, HistoryResult, DEFAULT_PAGE_SIZE},
    },
    subscriptions::{candidates, folder_suggestion, parse_release},
    worker::commands::receive_once,
};

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/history", get(list_history))
        .route("/history/{id}", get(get_history_item))
}

/// The largest page a client may ask for.
const MAX_LIMIT: usize = 200;

// ---------------------------------------------------------------------------
// Response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct HistoryItemView {
    pub id: i64,
    pub channel_id: String,
    /// The channel's name, or its host; for a deleted channel, the host it had.
    pub channel_name: String,
    pub channel_deleted: bool,
    pub title: String,
    /// Unix milliseconds.
    pub first_seen_at: i64,
    /// A result code: `received` (`추가함`), `no_match`, `excluded`, `duplicate`,
    /// `add_failed` or `version_unknown` (`버전 미상`).
    pub result: &'static str,
    pub result_label: &'static str,
    /// When the result was decided, Unix milliseconds.
    pub result_at: i64,
    /// The rule that selected the item, described by its match phrase or
    /// folder; `null` for an item no rule selected, or a deleted rule.
    pub rule_label: Option<String>,
    /// Whether the item was received without a rule (an item received by the
    /// `한 번 받기` that `다시 받기` replaced).
    pub by_hand: bool,
    /// Why adding failed, or (for a received item) a note such as that the file kept its original name.
    pub reason: Option<String>,
    /// Whether `다시 받기` can be offered.
    pub can_retry: bool,
    /// Why `다시 받기` is not offered on an item that failed to be added, as a
    /// sentence; `null` when it is offered or there is nothing to say.
    pub retry_blocked: Option<String>,
    /// The `receive_once` command for this item that has not ended yet.
    pub command: Option<CommandView>,
    /// For an item no rule matched, in a channel with a subscription waiting
    /// for a title: what naming one of them after this item needs.
    pub name_title: Option<NameTitleView>,
}

#[derive(Debug, Serialize)]
pub struct NameTitleView {
    /// The work part of the item's title: the match phrase it would give.
    pub work: String,
    /// The save folder made from the work.
    pub folder: Option<String>,
    /// The channel's subscriptions waiting for a title.
    pub waiting: Vec<WaitingView>,
}

#[derive(Debug, Default, Serialize)]
pub struct Counts {
    pub total: i64,
    pub received: i64,
    pub no_match: i64,
    pub excluded: i64,
    pub duplicate: i64,
    pub add_failed: i64,
    pub version_unknown: i64,
}

#[derive(Serialize)]
struct HistoryList {
    items: Vec<HistoryItemView>,
    /// Pass as `after` for the next page; `null` on the last one.
    next: Option<String>,
    counts: Counts,
}

// ---------------------------------------------------------------------------
// Building views
// ---------------------------------------------------------------------------

/// What the views need to know about the channels and rules.
struct Directory {
    channels: HashMap<String, Channel>,
    /// Rule ID to the rule.
    rules: HashMap<String, Rule>,
    /// The channels' waiting subscriptions, by channel ID, once for the page.
    waiting: HashMap<String, Vec<WaitingView>>,
}

impl Directory {
    /// A short description of the rule: its match phrase, or its folder.
    fn rule_label(rule: &Rule) -> String {
        rule.r#match
            .clone()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| rule.directory.clone())
    }

    async fn load(state: &AppState) -> Result<Directory, ApiError> {
        let all = state.channels.list_channels_with_rules().await?;
        let nos: Vec<i64> = all
            .iter()
            .flat_map(|c| &c.rules)
            .filter(|r| candidates::is_waiting(r))
            .filter_map(|r| r.subscription.as_ref().map(|s| s.anissia_anime_no))
            .collect();
        let animes: HashMap<i64, Anime> = if nos.is_empty() {
            HashMap::new()
        } else {
            state
                .anissia
                .store
                .animes(nos)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
        };
        let mut channels = HashMap::new();
        let mut rules = HashMap::new();
        let mut waiting = HashMap::new();
        for cwr in all {
            let views = waiting_views(&cwr, &animes);
            if !views.is_empty() {
                waiting.insert(cwr.channel.id.clone(), views);
            }
            let ChannelWithRules {
                channel,
                rules: channel_rules,
            } = cwr;
            for rule in channel_rules {
                rules.insert(rule.id.clone(), rule);
            }
            channels.insert(channel.id.clone(), channel);
        }
        Ok(Directory {
            channels,
            rules,
            waiting,
        })
    }
}

/// What naming a waiting subscription after the item needs, when the item's
/// channel has one and the item's title has a work.
fn name_title_of(item: &HistoryItem, directory: &Directory) -> Option<NameTitleView> {
    let waiting = directory.waiting.get(&item.channel_id)?;
    let work = parse_release(&item.title)?.work;
    Some(NameTitleView {
        folder: folder_suggestion(&work),
        work,
        waiting: waiting.clone(),
    })
}

fn host_of(url: &str) -> Option<String> {
    Url::parse(url).ok()?.host_str().map(str::to_owned)
}

/// `held` are the `버전 미상` items whose episode already holds their revision
/// ([`in_place::version_unknown_held`]), with the sentence that replaces the
/// button.
fn views(
    items: Vec<HistoryItem>,
    directory: &Directory,
    open: &HashMap<String, CommandView>,
    held: &HashMap<i64, String>,
) -> Vec<HistoryItemView> {
    items
        .into_iter()
        .map(|item| {
            let channel = directory.channels.get(&item.channel_id);
            let channel_name = channel
                .map(|c| {
                    c.name
                        .clone()
                        .or_else(|| host_of(&c.url))
                        .unwrap_or_default()
                })
                .unwrap_or_else(|| host_of(&item.channel_label).unwrap_or_default());
            let rule = item
                .rule_id
                .as_ref()
                .and_then(|rule| directory.rules.get(rule));
            let retry = receive_once::retry_plan(&item, channel, rule);
            let in_place = held.get(&item.id);
            let name_title = (item.result == HistoryResult::NoMatch)
                .then(|| name_title_of(&item, directory))
                .flatten();
            HistoryItemView {
                name_title,
                id: item.id,
                channel_name,
                channel_deleted: channel.is_none(),
                title: item.title,
                first_seen_at: item.first_seen_at,
                result: item.result.code(),
                result_label: item.result.label(),
                result_at: item.result_at,
                rule_label: rule.map(Directory::rule_label),
                by_hand: item.result == HistoryResult::Received && item.rule_id.is_none(),
                reason: item.reason,
                can_retry: retry.is_ok() && in_place.is_none(),
                retry_blocked: in_place.cloned().or_else(|| {
                    retry
                        .err()
                        .filter(|why| why.explains_missing_button())
                        .map(|why| why.message().to_owned())
                }),
                command: open.get(&item.id.to_string()).cloned(),
                channel_id: item.channel_id,
            }
        })
        .collect()
}

async fn open_commands(
    state: &AppState,
    items: &[HistoryItem],
) -> Result<HashMap<String, CommandView>, ApiError> {
    let subjects = items.iter().map(|item| item.id.to_string()).collect();
    let open = state
        .commands
        .open_for_subjects(receive_once::KIND, subjects)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(open
        .into_iter()
        .map(|(subject, command)| (subject, CommandView::from(&command)))
        .collect())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ListParams {
    result: Option<String>,
    channel: Option<String>,
    after: Option<String>,
    limit: Option<usize>,
}

fn history_error(err: trss_legacy::store::history::HistoryError) -> ApiError {
    ApiError::Internal(err.to_string())
}

async fn list_history(
    State(state): State<AppState>,
    params: Result<Query<ListParams>, QueryRejection>,
) -> Result<Json<HistoryList>, ApiError> {
    let Query(params) = params.map_err(|_| {
        ApiError::invalid("기록을 불러오는 조건을 읽지 못했어요. 필터를 다시 골라 주세요.")
    })?;

    let mut results = Vec::new();
    for code in params
        .result
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|code| !code.is_empty())
    {
        let result = HistoryResult::parse(code)
            .ok_or_else(|| ApiError::invalid("알 수 없는 결과로 걸러 볼 수 없어요."))?;
        if !results.contains(&result) {
            results.push(result);
        }
    }
    let after = params
        .after
        .as_deref()
        .map(|cursor| {
            cursor.parse::<HistoryCursor>().map_err(|()| {
                ApiError::invalid("이어서 불러올 위치를 읽지 못했어요. 목록을 새로고침해 주세요.")
            })
        })
        .transpose()?;
    let channel_id = params.channel.filter(|c| !c.is_empty());

    let page = state
        .history
        .list(HistoryQuery {
            results,
            channel_id: channel_id.clone(),
            after,
            limit: params
                .limit
                .unwrap_or(DEFAULT_PAGE_SIZE)
                .clamp(1, MAX_LIMIT),
            ..Default::default()
        })
        .await
        .map_err(history_error)?;

    let mut counts = Counts::default();
    for (result, count) in state
        .history
        .counts(channel_id)
        .await
        .map_err(history_error)?
    {
        counts.total += count;
        match result {
            HistoryResult::Received => counts.received = count,
            HistoryResult::NoMatch => counts.no_match = count,
            HistoryResult::Excluded => counts.excluded = count,
            HistoryResult::Duplicate => counts.duplicate = count,
            HistoryResult::AddFailed => counts.add_failed = count,
            HistoryResult::VersionUnknown => counts.version_unknown = count,
        }
    }

    let directory = Directory::load(&state).await?;
    let open = open_commands(&state, &page.items).await?;
    let held = in_place::version_unknown_held(&state, &page.items).await?;
    Ok(Json(HistoryList {
        items: views(page.items, &directory, &open, &held),
        next: page.next.map(|cursor| cursor.to_string()),
        counts,
    }))
}

async fn get_history_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<HistoryItemView>, ApiError> {
    let not_found = || ApiError::not_found("기록에서 이 항목을 찾지 못했어요.");
    let id: i64 = id.parse().map_err(|_| not_found())?;
    let item = state
        .history
        .get(id)
        .await
        .map_err(history_error)?
        .ok_or_else(not_found)?;
    let items = vec![item];
    let directory = Directory::load(&state).await?;
    let open = open_commands(&state, &items).await?;
    let held = in_place::version_unknown_held(&state, &items).await?;
    let mut views = views(items, &directory, &open, &held);
    Ok(Json(views.remove(0)))
}
