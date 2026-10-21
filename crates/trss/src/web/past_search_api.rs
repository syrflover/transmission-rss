//! `/api/rules/{id}/past-search` and `/api/past-searches/{id}`: the rule
//! detail's `지난 회차 검색` (`docs/specs/collection.md`, 지난 회차 검색).
//!
//! | call                               | success                                              |
//! | ---------------------------------- | ---------------------------------------------------- |
//! | `GET /rules/{id}/past-search`      | `200` [`Context`]: the search words and the range to start from |
//! | `POST /rules/{id}/past-search`     | `202` `{ "search_id": … }` once the search runs      |
//! | `GET /past-searches/{search_id}`   | `200` [`Poll`]: running, failed or done with the preview |
//! | `DELETE /past-searches/{search_id}`| `204`; the search ends and its results are forgotten |
//! | `POST /rules/{id}/past-search/range` | `200` [`RangeLabel`]: the folder episodes of a typed range |
//!
//! `POST` takes `{ "query": <search words>, "from": <release>, "to": <release> }`.
//! The search reads the tracker's search RSS (never its HTML), up to a minute
//! when the first page is full ([`trss_collect::past_search`]). A search leaves no
//! channel and no history; the results live in this process's memory, and
//! receiving one of them is the `receive_past` command
//! ([`super::commands_api`]), which names the item by its key.
//!
//! A new search of a rule ends the rule's earlier one. A search is forgotten
//! [`trss_collect::past_search::service::KEEP`] (30 minutes) after it started, however
//! often it is polled, and when the web restarts: the screen then says the
//! search is gone and the person searches again.
//!
//! `POST /rules/{id}/past-search/range` takes `{ "from": 1, "to": 12 }` while the
//! person types the range and answers `{ "from": 1, "to": 12, "label":
//! "S02E01–12" }`: the folder episodes of those releases under the rule's
//! conversion ([`trss_collect::episode_offset::range_label`]), `1–12화` when the
//! rule's folder has no season. It answers `400` for a range the search
//! would refuse (`from` below 1, or above `to`) and `404` for a rule that is
//! not there. A finished search's result carries the same label for its own
//! range as `range_label`.
//!
//! A finished search's `missing` and `not_found` are the release numbers in
//! ascending order; `missing_ranges` and `not_found_ranges` are the same as
//! runs of consecutive ones (`["4–6", "9"]`) for the sentence that names them.

use std::path::Path as FsPath;

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::{rules_api, ApiError, AppState};
use trss_collect::{
    episode_offset::{first_release, place_of, range_label, season_total},
    past_search::{
        judge::{Item, Range},
        query::{prefill, tidy},
        range::{suggest, Grounds, Suggestion},
        service::{Outcome, Spec, Status},
        MAX_SPAN,
    },
    plan::{rule_destination, ChannelPlan},
    store::{
        channels::{Channel, ChannelError, Rule, RuleState},
        history::{HistoryItem, HistoryQuery, HistoryResult, MAX_PAGE_SIZE},
    },
};
use trss_core::episode::range_texts;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/rules/{id}/past-search", get(context).post(start))
        .route("/rules/{id}/past-search/range", post(range))
        .route("/past-searches/{search_id}", delete(cancel).get(poll))
}

/// The most history items read for a search's picture of the work, and the
/// most titles read to tell a video's revision. The newest are read; a longer
/// history is told in the search's notes.
const MAX_SETTLED: usize = 20_000;
const MAX_TITLES: usize = 20_000;
/// The longest search words.
const MAX_QUERY_CHARS: usize = 300;

const GONE: &str = "이 검색은 끝났거나 서버가 다시 시작돼서 결과가 없어요. 다시 검색해 주세요.";

// ---------------------------------------------------------------------------
// The start of a search
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Context {
    pub rule_id: String,
    pub channel_id: String,
    /// The channel's search format, as saved.
    pub format: Option<String>,
    /// The search words to start with.
    pub query: String,
    /// Whether the channel's format made them.
    pub from_format: bool,
    /// Why the box starts empty, when it does.
    pub empty_because: Option<String>,
    /// The release range offered, and why.
    pub suggestion: Suggestion,
    /// The rule's episode conversion, which the folder episodes follow.
    pub offset: i64,
    /// The season number of the work folder.
    pub season: Option<u32>,
    /// A search of this rule that is running now.
    pub running: Option<String>,
    /// Why no search can start, as a sentence.
    pub blocked: Option<String>,
}

fn not_found(id: &str) -> ApiError {
    ChannelError::NotFound {
        kind: "rule",
        id: id.to_owned(),
    }
    .into()
}

/// The rule and its channel.
async fn rule_and_channel(state: &AppState, id: &str) -> Result<(Rule, Channel), ApiError> {
    let rule = state
        .channels
        .get_rule(id)
        .await
        .map_err(rules_api::store_error)?
        .ok_or_else(|| not_found(id))?;
    let channel = state
        .channels
        .get_channel(&rule.channel_id)
        .await
        .map_err(rules_api::store_error)?
        .ok_or_else(|| not_found(id))?;
    Ok((rule, channel))
}

/// Why a rule's past episodes cannot be searched and received now.
fn blocked(rule: &Rule, collect: Option<&str>) -> Option<String> {
    match rule.state {
        RuleState::Active => {}
        RuleState::Paused => {
            return Some(
                "규칙이 멈춰 있어요. 영상 받기를 켠 뒤에 지난 회차를 검색해 주세요.".to_owned(),
            )
        }
        RuleState::Archived => {
            return Some("규칙이 보관돼 있어요. 복원한 뒤에 지난 회차를 검색해 주세요.".to_owned())
        }
    }
    if collect.is_none() {
        return Some(
            "수집 폴더가 정해지지 않았어요. 설정에서 수집 폴더를 정한 뒤 검색해 주세요.".to_owned(),
        );
    }
    if rules_api::check_stored_work_folder(rule).is_err() {
        return Some(
            "이 규칙은 수집 폴더 자체에 받아요. 저장 폴더를 작품 폴더로 정한 뒤 검색해 주세요."
                .to_owned(),
        );
    }
    None
}

async fn collect_folder(state: &AppState) -> Result<Option<String>, ApiError> {
    Ok(state
        .settings
        .collection()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map(|settings| settings.folder))
}

/// The season the rule's videos go to and that season's AniList episode
/// count: the folder's season, or the season a subscription links the rule to.
async fn season_grounds(
    state: &AppState,
    rule: &Rule,
    collect: Option<&str>,
) -> (Option<u32>, Option<u32>) {
    let mut season = place_of(&rule.directory).map(|(_, season)| season);
    let mut total = None;
    if let Some(collect) = collect {
        match season_total(&state.library, &state.seasons.store, collect, rule).await {
            Ok(Some((linked, count))) => {
                season = Some(linked);
                total = count;
            }
            Ok(None) => {}
            Err(err) => eprintln!("Past search: no episode count for rule {}: {err}", rule.id),
        }
    }
    (season, total)
}

async fn context(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Context>, ApiError> {
    let (rule, channel) = rule_and_channel(&state, &id).await?;
    let collect = collect_folder(&state).await?;
    let start = prefill(channel.past_search.as_deref(), &rule);

    let (season, episodes) = season_grounds(&state, &rule, collect.as_deref()).await;
    let mut grounds = Grounds {
        offset: rule.episode,
        season,
        episodes,
        ..Grounds::default()
    };
    match state.history.first_titles_of_rule(&rule.id).await {
        Ok(titles) => grounds.first_release = first_release(&titles),
        Err(err) => eprintln!("Past search: no first release for rule {}: {err}", rule.id),
    }

    Ok(Json(Context {
        blocked: blocked(&rule, collect.as_deref()),
        running: state.past_search.running_of(&rule.id),
        rule_id: rule.id.clone(),
        channel_id: channel.id.clone(),
        format: channel.past_search.clone(),
        query: start.query,
        from_format: start.from_format,
        empty_because: start.empty_because.map(str::to_owned),
        suggestion: suggest(&grounds),
        offset: rule.episode,
        season: grounds.season,
    }))
}

// ---------------------------------------------------------------------------
// Starting a search
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartBody {
    query: String,
    from: u32,
    to: u32,
}

#[derive(Serialize)]
struct Started {
    search_id: String,
}

/// The sentence for a range of releases the search would refuse.
const BAD_RANGE: &str = "릴리스 회차 범위를 1 이상으로, 시작이 끝보다 크지 않게 적어 주세요.";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RangeBody {
    from: u32,
    to: u32,
}

/// The folder episodes of a typed range of releases.
#[derive(Debug, Serialize)]
pub struct RangeLabel {
    pub from: u32,
    pub to: u32,
    /// `S02E01–12`, or `1–12화` when the rule's folder has no season.
    pub label: String,
}

async fn range(
    State(state): State<AppState>,
    Path(id): Path<String>,
    parsed: Result<Json<RangeBody>, JsonRejection>,
) -> Result<Json<RangeLabel>, ApiError> {
    let b = body(parsed)?;
    let rule = state
        .channels
        .get_rule(&id)
        .await
        .map_err(rules_api::store_error)?
        .ok_or_else(|| not_found(&id))?;
    if b.from == 0 || b.to == 0 || b.from > b.to {
        return Err(ApiError::invalid(BAD_RANGE));
    }
    let collect = collect_folder(&state).await?;
    let (season, _) = season_grounds(&state, &rule, collect.as_deref()).await;
    Ok(Json(RangeLabel {
        from: b.from,
        to: b.to,
        label: range_label(b.from, b.to, rule.episode, season),
    }))
}

fn body<T>(parsed: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    parsed.map(|Json(value)| value).map_err(|_| {
        ApiError::invalid("요청 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 시도해 주세요.")
    })
}

/// The channel's items that history says Transmission holds.
/// Returns the items, newest first, and whether older ones were left out.
async fn settled_items(
    state: &AppState,
    channel_id: &str,
) -> Result<(Vec<HistoryItem>, bool), ApiError> {
    let mut items = Vec::new();
    let mut after = None;
    let mut cut = false;
    loop {
        let page = state
            .history
            .list(HistoryQuery {
                results: vec![HistoryResult::Received, HistoryResult::Duplicate],
                channel_id: Some(channel_id.to_owned()),
                after,
                limit: MAX_PAGE_SIZE,
                ..HistoryQuery::default()
            })
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        items.extend(page.items);
        match page.next {
            Some(next) if items.len() < MAX_SETTLED => after = Some(next),
            Some(_) => {
                cut = true;
                break;
            }
            None => break,
        }
    }
    Ok((items, cut))
}

async fn start(
    State(state): State<AppState>,
    Path(id): Path<String>,
    parsed: Result<Json<StartBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Started>), ApiError> {
    let b = body(parsed)?;
    let (rule, channel) = rule_and_channel(&state, &id).await?;
    let collect = collect_folder(&state).await?;
    if let Some(why) = blocked(&rule, collect.as_deref()) {
        return Err(ApiError::invalid(why));
    }
    let collect = collect.expect("blocked() refuses a missing collect folder");

    let query = tidy(&b.query);
    if query.is_empty() {
        return Err(ApiError::invalid("검색어를 적어 주세요."));
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(ApiError::invalid(format!(
            "검색어는 {MAX_QUERY_CHARS}자까지 적을 수 있어요."
        )));
    }
    if b.from == 0 || b.to == 0 || b.from > b.to {
        return Err(ApiError::invalid(BAD_RANGE));
    }
    if b.to - b.from >= MAX_SPAN {
        return Err(ApiError::invalid(format!(
            "한 번에 검색하는 범위는 {MAX_SPAN}화까지예요. 범위를 나눠서 검색해 주세요."
        )));
    }

    let (save_path, episode) = rule_destination(FsPath::new(&collect), &rule);
    let (settled, settled_cut) = settled_items(&state, &channel.id).await?;
    // What the worker last saw in Transmission; the web cannot ask it.
    let listing = state
        .status
        .torrent_listing()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    // One more than the cap is asked for, to tell a history of exactly that
    // length from a longer one.
    let mut titles: Vec<String> = state
        .history
        .recent_titles_of_channel(channel.id.clone(), MAX_TITLES + 1)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .into_iter()
        .map(|(_, title)| title)
        .collect();
    let history_cut = settled_cut || titles.len() > MAX_TITLES;
    titles.truncate(MAX_TITLES);
    let redactor = ChannelPlan::new(
        trss_collect::store::channels::ChannelWithRules {
            channel: channel.clone(),
            rules: Vec::new(),
        },
        FsPath::new(""),
    )
    .redactor();

    let search_id = state.past_search.start(Spec {
        season: place_of(&rule.directory).map(|(_, season)| season),
        offset: episode as i64,
        save_path,
        settled,
        listing,
        titles,
        history_cut,
        redactor,
        range: Range {
            from: b.from,
            to: b.to,
        },
        query,
        channel,
        rule,
    });
    Ok((StatusCode::ACCEPTED, Json(Started { search_id })))
}

// ---------------------------------------------------------------------------
// Following a search
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Poll {
    /// `running`, `failed` or `done`.
    pub state: &'static str,
    pub rule_id: String,
    /// While running: the extra searches sent, and how many are planned.
    pub sent: usize,
    pub needed: usize,
    /// The sentence for a failed search.
    pub error: Option<String>,
    pub result: Option<ResultView>,
}

#[derive(Debug, Serialize)]
pub struct ResultView {
    pub from: u32,
    pub to: u32,
    /// The range as the folder names its episodes (`S02E01–12`), as the range
    /// route tells it; `None` when the rule is gone.
    pub range_label: Option<String>,
    pub query: String,
    pub items: Vec<ItemView>,
    pub out_of_range: usize,
    pub not_picked: usize,
    pub missing: Vec<u32>,
    /// `missing` as runs: `["4–6", "9"]` ([`trss_core::episode::range_texts`]).
    pub missing_ranges: Vec<String>,
    pub not_found: Vec<u32>,
    /// `not_found` as runs, as `missing_ranges`.
    pub not_found_ranges: Vec<String>,
    pub notes: Vec<String>,
    pub first_full: bool,
    pub extra_sent: usize,
    pub extra_needed: usize,
}

#[derive(Debug, Serialize)]
pub struct ItemView {
    pub key: String,
    pub title: String,
    /// See [`trss_collect::past_search::judge::State::code`].
    pub state: &'static str,
    pub note: Option<String>,
    pub release: Option<u32>,
    pub half: bool,
    pub version: u32,
    pub folder: Option<String>,
    pub selected: bool,
    pub selectable: bool,
}

impl From<&Item> for ItemView {
    fn from(item: &Item) -> Self {
        ItemView {
            key: item.key.clone(),
            title: item.title.clone(),
            state: item.state.code(),
            note: item.note.clone(),
            release: item.release,
            half: item.half,
            version: item.version,
            folder: item.folder.clone(),
            selected: item.selected,
            selectable: item.selectable,
        }
    }
}

fn result_view(outcome: &Outcome, range_label: Option<String>) -> ResultView {
    ResultView {
        from: outcome.range.from,
        to: outcome.range.to,
        range_label,
        query: outcome.query.clone(),
        items: outcome.preview.items.iter().map(ItemView::from).collect(),
        out_of_range: outcome.preview.out_of_range,
        not_picked: outcome.preview.not_picked,
        missing: outcome.preview.missing.clone(),
        missing_ranges: range_texts(&outcome.preview.missing),
        not_found: outcome.preview.not_found.clone(),
        not_found_ranges: range_texts(&outcome.preview.not_found),
        notes: outcome.notes.clone(),
        first_full: outcome.first_full,
        extra_sent: outcome.extra_sent,
        extra_needed: outcome.extra_needed,
    }
}

/// The label of a finished search's range, written the way [`range`] writes
/// it for the rule as it is now.
async fn result_label(state: &AppState, outcome: &Outcome) -> Option<String> {
    let rule = state.channels.get_rule(&outcome.rule_id).await.ok()??;
    let collect = collect_folder(state).await.ok()?;
    let (season, _) = season_grounds(state, &rule, collect.as_deref()).await;
    Some(range_label(
        outcome.range.from,
        outcome.range.to,
        rule.episode,
        season,
    ))
}

async fn poll(
    State(state): State<AppState>,
    Path(search_id): Path<String>,
) -> Result<Json<Poll>, ApiError> {
    let (rule_id, status) = state
        .past_search
        .status(&search_id)
        .ok_or_else(|| ApiError::not_found(GONE))?;
    let mut poll = Poll {
        state: "running",
        rule_id,
        sent: 0,
        needed: 0,
        error: None,
        result: None,
    };
    match status {
        Status::Running { sent, needed } => {
            poll.sent = sent;
            poll.needed = needed;
        }
        Status::Failed(sentence) => {
            poll.state = "failed";
            poll.error = Some(sentence);
        }
        Status::Done(outcome) => {
            poll.state = "done";
            let label = result_label(&state, &outcome).await;
            poll.result = Some(result_view(&outcome, label));
        }
    }
    Ok(Json(poll))
}

async fn cancel(State(state): State<AppState>, Path(search_id): Path<String>) -> StatusCode {
    state.past_search.cancel(&search_id);
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests;
