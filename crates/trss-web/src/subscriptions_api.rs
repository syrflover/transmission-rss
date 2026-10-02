//! `/api/anissia` and `/api/subscriptions`: Anissia's schedule, and the rules
//! that follow an anime of it (`docs/specs/collection.md`, 방영작 구독).
//!
//! | call                                         | success                                        |
//! | -------------------------------------------- | ---------------------------------------------- |
//! | `GET /anissia/schedule/{week}`               | `200 Schedule` (0 Sunday to 6 Saturday, 7 `기타`, 8 `신작`) |
//! | `GET /anissia/anime/{no}/creators`           | `200 Creators`                                 |
//! | `GET /subscriptions`                         | `200 { quarter, subscriptions: [SubscriptionItem] }` |
//! | `GET /subscriptions/titles?channel_id=&q=`   | `200 Titles`                                   |
//! | `POST /subscriptions`                        | `201 { rule: RuleView }`                       |
//! | `GET /subscriptions/candidates`              | `200 { candidates: [CandidateView] }`          |
//! | `POST /subscriptions/candidates/reject`      | `200 { rejected: true }`                       |
//! | `PUT /rules/{id}/creator`                    | `200 RuleView` (`제작자 변경`)                 |
//! | `POST /rules/{id}/subscription`              | `200 RuleView` (`편성표와 연결`)               |
//! | `POST /rules/{id}/title`                     | `200 RuleView` (`제목 정하기`)                 |
//!
//! Anissia is a third party: a call that needs it and cannot get an answer
//! fails with `502 { error: "unavailable", message }` and a sentence that says
//! why, so the screen can show the reason and a retry. The web waits for the
//! shared request pace for a few seconds at most.
//!
//! Subscribing creates the rule and nothing else. It receives nothing: the
//! items history recorded before are past, the cycle leaves them alone, and
//! the user receives the ones they pick with `receive_once` and the new rule's
//! ID (see [`trss_collect::commands::receive_once`]).
//!
//! Everything a subscription is made of is checked here against the stored
//! data and Anissia, not trusted from the request: the release title must be
//! one the channel's history holds, the anime one the schedule lists, and a
//! subtitle creator one of the anime's captions names.
//!
//! A subscription made without a `work` waits for its title (`제목 대기`): the
//! rule has no match phrase and receives nothing. It needs a channel whose
//! history has been recorded already, because a work is new only against what
//! history held when the subscription began to wait.
//!
//! `GET /subscriptions/candidates` lists the title candidates
//! ([`title_candidates`]; the conditions under which one appears and goes are in
//! [`trss_collect::subscriptions::candidates`]). It is the source the 할 일 list reads
//! for its `제목 후보` suggestions, and it is not counted in any menu badge.
//! `POST /subscriptions/candidates/reject` (`{ channel_id, work }`) turns a
//! work down for good: it is not offered again and the subscriptions stay as
//! they are. `POST /rules/{id}/title` (`{ version, work, directory? }`) gives a
//! collecting subscription that waits for its title the phrase of a work the
//! channel's history holds, and, when `directory` is sent, a new save folder.
//! It receives nothing: what history recorded before is past, left to the user
//! to pick with `receive_once` and the rule's ID.
//!
//! `PUT /rules/{id}/creator` (`{ version, creator }`, `creator` a name or
//! `null` for `제작자 미정`) changes whom a subscription follows, for a
//! subscription that receives subtitles; `POST /rules/{id}/subscription`
//! (`{ version, anissia_anime_no, week, subtitles, creator }`) makes an existing
//! rule a subscription and changes nothing else about it. A version that is not
//! the stored one answers `409` with the rule's current view.

use std::{collections::HashMap, time::Duration};

use axum::{
    extract::{rejection::JsonRejection, rejection::QueryRejection, Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    rules_api::{self, RuleView},
    ApiError, AppState,
};
use trss_anissia::{Anime, AnissiaError, Fetched, LAST_WEEK, WEEK_OTHER, WEEK_UPCOMING};
use trss_collect::{
    store::channels::{
        ChannelError, ChannelWithRules, NewSubscription, RuleInput, RuleState, Subscription,
        SubtitleMode,
    },
    subscriptions::{
        candidates::{self, TitleCandidate},
        folder_suggestion, title_groups, work_key, Quarter,
    },
};
use trss_core::Millis;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/anissia/schedule/{week}", get(schedule))
        .route("/anissia/anime/{no}/creators", get(creators))
        .route("/subscriptions", get(list).post(subscribe))
        .route("/subscriptions/titles", get(titles))
        .route("/subscriptions/candidates", get(candidates))
        .route("/subscriptions/candidates/reject", post(reject_candidate))
        .route("/rules/{id}/title", post(name_title))
        .route("/rules/{id}/creator", put(change_creator))
        .route("/rules/{id}/subscription", post(link_rule))
}

/// The longest a request of the screen waits for its turn to ask Anissia.
const USER_MAX_WAIT: Duration = Duration::from_secs(8);

/// The most titles one answer lists.
const TITLE_LIMIT: usize = 200;

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

/// The snapshot of a subscribed anime, as Anissia last listed it.
#[derive(Debug, Clone, Serialize)]
pub struct AnimeView {
    pub anime_no: i64,
    pub subject: String,
    pub original_subject: Option<String>,
    /// 0 (Sunday) to 6 (Saturday), 7 (`기타`) or 8 (`신작`).
    pub week: u8,
    /// `HH:MM`, Asia/Seoul.
    pub air_time: Option<String>,
    /// `YYYY-MM-DD`, or `YYYY-MM` when only the month is known.
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    /// Anissia's `ON` or `OFF`.
    pub status: String,
    /// When Anissia was asked (Unix ms).
    pub fetched_at: Millis,
}

impl From<&Anime> for AnimeView {
    fn from(a: &Anime) -> Self {
        AnimeView {
            anime_no: a.anime_no,
            subject: a.subject.clone(),
            original_subject: a.original_subject.clone(),
            week: a.week,
            air_time: a.air_time.clone(),
            start_date: a.start_date.clone(),
            end_date: a.end_date.clone(),
            status: a.status.clone(),
            fetched_at: a.fetched_at,
        }
    }
}

/// A rule's subscription.
#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionBrief {
    pub anissia_anime_no: i64,
    /// `follow`, `undecided` or `none`.
    pub subtitles: &'static str,
    pub creator: Option<String>,
    pub season_id: Option<String>,
    pub subscribed_at: Millis,
    /// The stored snapshot; `null` only if the row is gone.
    pub anime: Option<AnimeView>,
    /// The quarter the anime started in (or, without a start date, the
    /// subscription began in).
    pub quarter: QuarterView,
}

pub fn subscription_brief(
    subscription: &Subscription,
    animes: &HashMap<i64, Anime>,
) -> SubscriptionBrief {
    let anime = animes.get(&subscription.anissia_anime_no);
    SubscriptionBrief {
        anissia_anime_no: subscription.anissia_anime_no,
        subtitles: subscription.subtitles.as_str(),
        creator: subscription.creator.clone(),
        season_id: subscription.season_id.clone(),
        subscribed_at: subscription.subscribed_at,
        anime: anime.map(AnimeView::from),
        quarter: quarter_of(anime, subscription.subscribed_at).into(),
    }
}

/// The quarter an anime belongs to: the one it started in, or, without a start
/// date, the one the subscription began in.
pub(super) fn quarter_of(anime: Option<&Anime>, subscribed_at: Millis) -> Quarter {
    anime
        .and_then(|a| a.start_date.as_deref())
        .and_then(Quarter::of_date)
        .unwrap_or_else(|| Quarter::at(subscribed_at))
}

#[derive(Debug, Clone, Serialize)]
pub struct QuarterView {
    year: i32,
    /// 1 to 4.
    number: u8,
}

impl From<Quarter> for QuarterView {
    fn from(q: Quarter) -> Self {
        QuarterView {
            year: q.year,
            number: q.number,
        }
    }
}

#[derive(Debug, Serialize)]
struct ScheduleEntryView {
    anime_no: i64,
    week: u8,
    subject: String,
    original_subject: Option<String>,
    air_time: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    status: String,
    genres: Vec<String>,
    /// How many captions Anissia lists for the anime.
    caption_count: u32,
    /// The rules that already follow this anime, in any channel.
    subscribed_rules: Vec<SubscribedRule>,
}

#[derive(Debug, Serialize)]
struct SubscribedRule {
    rule_id: String,
    channel_id: String,
}

#[derive(Debug, Serialize)]
struct Schedule {
    week: u8,
    entries: Vec<ScheduleEntryView>,
    /// When Anissia was asked (Unix ms).
    fetched_at: Millis,
    /// Whether this is an answer kept from an earlier look.
    cached: bool,
}

#[derive(Debug, Serialize)]
struct CreatorView {
    name: String,
    captions: usize,
    last_updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct Creators {
    anime_no: i64,
    creators: Vec<CreatorView>,
    fetched_at: Millis,
    cached: bool,
}

// ---------------------------------------------------------------------------
// Anissia's answers
// ---------------------------------------------------------------------------

/// The sentence for a failed ask of Anissia.
fn unavailable(error: AnissiaError) -> ApiError {
    match error {
        AnissiaError::Busy { retry_after } => ApiError::Unavailable(format!(
            "Anissia에 요청이 몰려 있어요. {}초쯤 뒤에 다시 시도해 주세요.",
            retry_after.as_secs().max(1)
        )),
        AnissiaError::Unreachable(_) => {
            ApiError::Unavailable("Anissia에 연결하지 못했어요. 잠시 뒤 다시 시도해 주세요.".into())
        }
        AnissiaError::Status(code) => ApiError::Unavailable(format!(
            "Anissia가 오류로 답했어요(HTTP {code}). 잠시 뒤 다시 시도해 주세요."
        )),
        AnissiaError::Invalid(_) => ApiError::Unavailable(
            "Anissia의 응답을 읽지 못했어요. 잠시 뒤 다시 시도해 주세요.".into(),
        ),
        AnissiaError::NoSuchWeek(_) => ApiError::invalid("없는 요일이에요."),
        AnissiaError::Store(e) => ApiError::Internal(e.to_string()),
    }
}

/// The rules that follow each anime.
async fn subscribed(state: &AppState) -> Result<HashMap<i64, Vec<SubscribedRule>>, ApiError> {
    let mut out: HashMap<i64, Vec<SubscribedRule>> = HashMap::new();
    for cwr in state
        .channels
        .list_channels_with_rules()
        .await
        .map_err(ApiError::from)?
    {
        for rule in cwr.rules {
            if let Some(subscription) = rule.subscription {
                out.entry(subscription.anissia_anime_no)
                    .or_default()
                    .push(SubscribedRule {
                        rule_id: rule.id,
                        channel_id: rule.channel_id,
                    });
            }
        }
    }
    Ok(out)
}

async fn schedule(
    State(state): State<AppState>,
    Path(week): Path<u8>,
) -> Result<Json<Schedule>, ApiError> {
    if week > LAST_WEEK {
        return Err(ApiError::invalid("없는 요일이에요."));
    }
    let fetched = state
        .anissia
        .schedule(week, Some(USER_MAX_WAIT))
        .await
        .map_err(unavailable)?;
    let mut followed = subscribed(&state).await?;
    let mut entries: Vec<ScheduleEntryView> = fetched
        .value
        .iter()
        .map(|entry| ScheduleEntryView {
            anime_no: entry.anime_no,
            week: entry.week,
            subject: entry.subject.clone(),
            original_subject: entry.original_subject.clone(),
            air_time: entry.air_time.clone(),
            start_date: entry.start_date.clone(),
            end_date: entry.end_date.clone(),
            status: entry.status.clone(),
            genres: entry.genres.clone(),
            caption_count: entry.caption_count,
            subscribed_rules: followed.remove(&entry.anime_no).unwrap_or_default(),
        })
        .collect();
    if week == WEEK_OTHER || week == WEEK_UPCOMING {
        entries.sort_by(|a, b| {
            (a.start_date.is_none(), &a.start_date, &a.subject).cmp(&(
                b.start_date.is_none(),
                &b.start_date,
                &b.subject,
            ))
        });
    } else {
        entries.sort_by(|a, b| {
            (a.air_time.is_none(), &a.air_time, &a.subject).cmp(&(
                b.air_time.is_none(),
                &b.air_time,
                &b.subject,
            ))
        });
    }
    Ok(Json(Schedule {
        week,
        entries,
        fetched_at: fetched.fetched_at,
        cached: fetched.cached,
    }))
}

fn creators_of(fetched: &Fetched<trss_anissia::Caption>) -> Vec<CreatorView> {
    trss_anissia::parse::creators(&fetched.value)
        .into_iter()
        .map(|c| CreatorView {
            name: c.name,
            captions: c.captions,
            last_updated_at: c.last_updated_at,
        })
        .collect()
}

async fn creators(
    State(state): State<AppState>,
    Path(no): Path<i64>,
) -> Result<Json<Creators>, ApiError> {
    let fetched = state
        .anissia
        .captions(no, Some(USER_MAX_WAIT))
        .await
        .map_err(unavailable)?;
    Ok(Json(Creators {
        anime_no: no,
        creators: creators_of(&fetched),
        fetched_at: fetched.fetched_at,
        cached: fetched.cached,
    }))
}

// ---------------------------------------------------------------------------
// The subscriptions
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct SubscriptionItem {
    rule_id: String,
    /// `active` or `paused` (`영상 받기` off).
    state: &'static str,
    rule_version: i64,
    channel_id: String,
    channel_name: Option<String>,
    channel_host: String,
    /// The rule's match phrase.
    title: Option<String>,
    directory: String,
    subscription: SubscriptionBrief,
    /// The quarter the anime started in (or, without a start date, the
    /// subscription began in).
    quarter: QuarterView,
    /// The quarter has not begun yet.
    upcoming: bool,
}

#[derive(Debug, Serialize)]
struct SubscriptionList {
    /// The current quarter in Asia/Seoul.
    quarter: QuarterView,
    /// Subscriptions that are not archived (paused ones too, with their
    /// `state`), this quarter's and any begun in earlier quarters first, then
    /// the ones of a coming quarter.
    subscriptions: Vec<SubscriptionItem>,
}

async fn list(State(state): State<AppState>) -> Result<Json<SubscriptionList>, ApiError> {
    let now = state.anissia.now();
    let current = Quarter::at(now);
    let all = state
        .channels
        .list_channels_with_rules()
        .await
        .map_err(ApiError::from)?;
    let nos: Vec<i64> = all
        .iter()
        .flat_map(|c| &c.rules)
        .filter_map(|r| r.subscription.as_ref().map(|s| s.anissia_anime_no))
        .collect();
    let animes = state
        .anissia_store
        .animes(nos)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut items = Vec::new();
    for cwr in &all {
        let host = Url::parse(&cwr.channel.url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        for rule in &cwr.rules {
            let Some(subscription) = &rule.subscription else {
                continue;
            };
            if rule.state == RuleState::Archived {
                continue;
            }
            let anime = animes.get(&subscription.anissia_anime_no);
            let quarter = quarter_of(anime, subscription.subscribed_at);
            items.push(SubscriptionItem {
                rule_id: rule.id.clone(),
                state: rule.state.as_str(),
                rule_version: rule.version,
                channel_id: cwr.channel.id.clone(),
                channel_name: cwr.channel.name.clone(),
                channel_host: host.clone(),
                title: rule.r#match.clone(),
                directory: rule.directory.clone(),
                subscription: subscription_brief(subscription, &animes),
                quarter: quarter.into(),
                upcoming: quarter > current,
            });
        }
    }
    // Weekdays Sunday to Saturday in the schedule's order, then `기타`, then
    // `신작`; within one, by air time.
    items.sort_by(|a, b| {
        let key = |i: &SubscriptionItem| {
            let anime = i.subscription.anime.as_ref();
            (
                i.upcoming,
                anime.map_or(WEEK_OTHER, |a| a.week),
                anime.and_then(|a| a.air_time.clone()).unwrap_or_default(),
                anime.map(|a| a.subject.clone()).unwrap_or_default(),
            )
        };
        key(a).cmp(&key(b))
    });
    Ok(Json(SubscriptionList {
        quarter: current.into(),
        subscriptions: items,
    }))
}

#[derive(Debug, Serialize)]
struct TitleView {
    /// The work, the way the newest item writes it: the rule's match phrase.
    work: String,
    /// The newest item's full title.
    latest_title: String,
    /// How many recorded items the work has.
    items: usize,
    latest_seen_at: Millis,
    /// The save folder suggested for it.
    folder: Option<String>,
}

#[derive(Debug, Serialize)]
struct Titles {
    /// How many items the channel's history holds (the newest ones are read).
    recorded_items: usize,
    /// How many works match `q`.
    total: usize,
    /// There are more matching works than `titles` lists.
    truncated: bool,
    titles: Vec<TitleView>,
}

#[derive(Deserialize)]
struct TitlesQuery {
    channel_id: String,
    #[serde(default)]
    q: String,
}

fn matches_query(query: &str, work: &str, latest_title: &str) -> bool {
    let needle = work_key(query);
    needle.is_empty()
        || work_key(work).contains(&needle)
        || work_key(latest_title).contains(&needle)
}

async fn titles(
    State(state): State<AppState>,
    parsed: Result<Query<TitlesQuery>, QueryRejection>,
) -> Result<Json<Titles>, ApiError> {
    let Query(q) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;
    state
        .channels
        .get_channel(&q.channel_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::not_found("채널을 찾지 못했어요. 이미 삭제됐을 수 있어요."))?;
    let items = rules_api::channel_items(&state.history, &q.channel_id).await?;
    let groups = title_groups(&items);
    let matching: Vec<_> = groups
        .into_iter()
        .filter(|g| matches_query(&q.q, &g.work, &g.latest_title))
        .collect();
    let total = matching.len();
    Ok(Json(Titles {
        recorded_items: items.len(),
        total,
        truncated: total > TITLE_LIMIT,
        titles: matching
            .into_iter()
            .take(TITLE_LIMIT)
            .map(|g| TitleView {
                folder: folder_suggestion(&g.work),
                work: g.work,
                latest_title: g.latest_title,
                items: g.items,
                latest_seen_at: g.latest_seen_at,
            })
            .collect(),
    }))
}

/// The way the channel's newest record of the work `work` writes it: the match
/// phrase it gives. The work must be one the channel's history holds.
fn recorded_work(
    items: &[trss_collect::store::history::HistoryItem],
    work: &str,
) -> Result<String, ApiError> {
    let wanted = work_key(work);
    title_groups(items)
        .into_iter()
        .find(|g| work_key(&g.work) == wanted)
        .map(|g| g.work)
        .ok_or_else(|| {
            ApiError::invalid(
                "이 채널의 수집 기록에 없는 릴리스 제목이에요. 기록에 있는 제목에서 골라 주세요.",
            )
        })
}

// ---------------------------------------------------------------------------
// Title candidates
// ---------------------------------------------------------------------------

/// The title candidates of every channel, the newest work first, as they are
/// now. A candidate is read off the rules and the history each time (see
/// [`trss_collect::subscriptions::candidates`]), so it goes the moment its conditions
/// fail: the work gets a rule, the last subscription waiting for a title is
/// paused, archived, given a title or deleted, or the user rejects the work.
///
/// This is what the 할 일 list reads for its `제목 후보` suggestions. A
/// suggestion is not something that needs doing, so nothing counts these in a
/// menu badge.
pub async fn title_candidates(state: &AppState) -> Result<Vec<TitleCandidate>, ApiError> {
    let all = state
        .channels
        .list_channels_with_rules()
        .await
        .map_err(ApiError::from)?;
    if !all
        .iter()
        .any(|c| c.rules.iter().any(candidates::is_waiting))
    {
        return Ok(Vec::new());
    }
    let rejected = state
        .channels
        .rejected_titles()
        .await
        .map_err(ApiError::from)?;
    let mut found = Vec::new();
    for cwr in &all {
        if !cwr.rules.iter().any(candidates::is_waiting) {
            continue;
        }
        let (items, truncated) =
            rules_api::channel_items_window(&state.history, &cwr.channel.id).await?;
        let rejected_here = rejected
            .iter()
            .filter(|(channel_id, _)| channel_id == &cwr.channel.id)
            .map(|(_, key)| key.clone())
            .collect();
        found.extend(candidates::title_candidates(
            cwr,
            &items,
            truncated,
            &rejected_here,
        ));
    }
    found.sort_by(|a, b| {
        b.latest_seen_at
            .cmp(&a.latest_seen_at)
            .then_with(|| a.work.cmp(&b.work))
    });
    Ok(found)
}

/// A subscription that waits for a title, as the work is offered to it.
#[derive(Debug, Clone, Serialize)]
pub struct WaitingView {
    pub rule_id: String,
    pub rule_version: i64,
    /// The anime it follows, as the stored schedule snapshot lists it.
    pub anime: Option<AnimeView>,
    /// The save folder the subscription has now.
    pub directory: String,
}

#[derive(Debug, Serialize)]
struct CandidateView {
    channel_id: String,
    channel_name: Option<String>,
    channel_host: String,
    /// What identifies the candidate within its channel.
    key: String,
    /// The work as the newest item writes it: the match phrase it would give.
    work: String,
    latest_title: String,
    items: usize,
    first_seen_at: Millis,
    latest_seen_at: Millis,
    /// The save folder made from the work, to offer in place of the folder the
    /// subscription has.
    folder: Option<String>,
    /// The channel's subscriptions waiting for a title, to pick one from. The
    /// link between an anime and the work is only a suggestion.
    waiting: Vec<WaitingView>,
}

/// The waiting subscriptions of a channel as the screen picks among them.
pub fn waiting_views(cwr: &ChannelWithRules, animes: &HashMap<i64, Anime>) -> Vec<WaitingView> {
    cwr.rules
        .iter()
        .filter(|rule| candidates::is_waiting(rule))
        .map(|rule| WaitingView {
            rule_id: rule.id.clone(),
            rule_version: rule.version,
            anime: rule
                .subscription
                .as_ref()
                .and_then(|s| animes.get(&s.anissia_anime_no))
                .map(AnimeView::from),
            directory: rule.directory.clone(),
        })
        .collect()
}

#[derive(Serialize)]
struct CandidateList {
    candidates: Vec<CandidateView>,
}

async fn candidates(State(state): State<AppState>) -> Result<Json<CandidateList>, ApiError> {
    let found = title_candidates(&state).await?;
    if found.is_empty() {
        return Ok(Json(CandidateList {
            candidates: Vec::new(),
        }));
    }
    let all = state
        .channels
        .list_channels_with_rules()
        .await
        .map_err(ApiError::from)?;
    let nos: Vec<i64> = all
        .iter()
        .flat_map(|c| &c.rules)
        .filter_map(|r| r.subscription.as_ref().map(|s| s.anissia_anime_no))
        .collect();
    let animes = state
        .anissia_store
        .animes(nos)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let views = found
        .into_iter()
        .filter_map(|candidate| {
            let cwr = all.iter().find(|c| c.channel.id == candidate.channel_id)?;
            let host = Url::parse(&cwr.channel.url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_default();
            Some(CandidateView {
                channel_name: cwr.channel.name.clone(),
                channel_host: host,
                folder: folder_suggestion(&candidate.work),
                waiting: waiting_views(cwr, &animes),
                channel_id: candidate.channel_id,
                key: candidate.key,
                work: candidate.work,
                latest_title: candidate.latest_title,
                items: candidate.items,
                first_seen_at: candidate.first_seen_at,
                latest_seen_at: candidate.latest_seen_at,
            })
        })
        .collect();
    Ok(Json(CandidateList { candidates: views }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RejectBody {
    channel_id: String,
    /// The work of the candidate, as the candidate lists it.
    work: String,
}

#[derive(Serialize)]
struct Rejected {
    rejected: bool,
}

async fn reject_candidate(
    State(state): State<AppState>,
    parsed: Result<Json<RejectBody>, JsonRejection>,
) -> Result<Json<Rejected>, ApiError> {
    let Json(b) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;
    state
        .channels
        .get_channel(&b.channel_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::not_found("채널을 찾지 못했어요. 이미 삭제됐을 수 있어요."))?;
    let items = rules_api::channel_items(&state.history, &b.channel_id).await?;
    let work = recorded_work(&items, &b.work)?;
    state
        .channels
        .reject_title(&b.channel_id, &work_key(&work), &work, state.anissia.now())
        .await
        .map_err(ApiError::from)?;
    Ok(Json(Rejected { rejected: true }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TitleBody {
    /// The version the client saw.
    version: i64,
    /// The work of a release title the channel's history holds.
    work: String,
    /// A new save folder; absent keeps the subscription's.
    #[serde(default)]
    directory: Option<String>,
}

async fn name_title(
    State(state): State<AppState>,
    Path(id): Path<String>,
    parsed: Result<Json<TitleBody>, JsonRejection>,
) -> Result<Json<RuleView>, ApiError> {
    let Json(b) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;
    let rule = rule_at(&state, &id, b.version).await?;
    if rule.subscription.is_none() || rule.r#match.is_some() {
        return Err(ApiError::invalid(
            "제목을 기다리는 구독만 제목을 정해요. 화면을 새로고침해 주세요.",
        ));
    }
    if rule.state != RuleState::Active {
        return Err(ApiError::invalid(
            "영상 받기를 켠 구독에만 제목을 정할 수 있어요. 영상 받기를 켠 뒤 다시 시도해 주세요.",
        ));
    }
    let directory = match b.directory.as_deref().map(str::trim) {
        // The folder the subscription has stays, and must still be a work folder.
        None => {
            rules_api::check_stored_work_folder(&rule)?;
            None
        }
        Some(directory) => {
            rules_api::check_work_folder(directory)?;
            if std::path::Path::new(directory).is_absolute() {
                return Err(ApiError::invalid(
                    "저장 폴더는 수집 폴더 아래 경로로 적어 주세요. /로 시작하면 안 돼요.",
                ));
            }
            rules_api::check_directory(&state, Some(&rule), directory).await?;
            Some(directory)
        }
    };
    let items = rules_api::channel_items(&state.history, &rule.channel_id).await?;
    let phrase = recorded_work(&items, &b.work)?;
    let written = state
        .channels
        .give_title(&id, b.version, &phrase, directory, state.anissia.now())
        .await;
    match written {
        Err(ChannelError::Invalid(_)) => Err(ApiError::invalid(
            "입력한 값으로는 제목을 정할 수 없어요. 화면을 새로고침해 주세요.",
        )),
        written => rule_after(&state, &id, written).await,
    }
}

/// The anime of the schedule's week `week` numbered `anime_no`, as the
/// snapshot to keep: the request names an anime the schedule really lists.
async fn scheduled_anime(state: &AppState, week: u8, anime_no: i64) -> Result<Anime, ApiError> {
    let listed = state
        .anissia
        .schedule(week, Some(USER_MAX_WAIT))
        .await
        .map_err(unavailable)?;
    let entry = listed
        .value
        .iter()
        .find(|e| e.anime_no == anime_no)
        .ok_or_else(|| {
            ApiError::invalid(
                "편성표에서 이 작품을 찾지 못했어요. 편성표를 다시 불러와 골라 주세요.",
            )
        })?;
    Ok(entry.snapshot(listed.fetched_at))
}

/// The creator a subscription stores for the subtitle mode asked: to follow
/// one the anime's captions must name it, and only that mode takes one.
async fn chosen_creator(
    state: &AppState,
    subtitles: SubtitleMode,
    creator: Option<&str>,
    anime_no: i64,
) -> Result<Option<String>, ApiError> {
    match (subtitles, creator.map(str::trim)) {
        (SubtitleMode::Follow, Some(name)) if !name.is_empty() => {
            Ok(Some(named_creator(state, anime_no, name).await?))
        }
        (SubtitleMode::Follow, _) => Err(ApiError::invalid("따라 받을 자막 제작자를 골라 주세요.")),
        (_, None) => Ok(None),
        (_, Some(_)) => Err(ApiError::invalid(
            "자막 제작자는 제작자를 따라 받을 때만 정해요.",
        )),
    }
}

/// `name` if the anime's captions name that creator.
async fn named_creator(state: &AppState, anime_no: i64, name: &str) -> Result<String, ApiError> {
    let captions = state
        .anissia
        .captions(anime_no, Some(USER_MAX_WAIT))
        .await
        .map_err(unavailable)?;
    if creators_of(&captions).iter().any(|c| c.name == name) {
        Ok(name.to_owned())
    } else {
        Err(ApiError::invalid(
            "이 작품의 자막 목록에 없는 제작자예요. 목록에서 골라 주세요.",
        ))
    }
}

const BAD_BODY: &str = "요청 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubscribeBody {
    channel_id: String,
    /// Anissia's `animeNo`.
    anissia_anime_no: i64,
    /// The schedule week the anime was picked from, to find its entry.
    week: u8,
    /// The work of a release title the channel's history holds (a
    /// [`TitleView::work`]): the rule's match phrase. Absent, the subscription
    /// waits for its title (`아직 첫 화 전이에요`).
    #[serde(default)]
    work: Option<String>,
    /// `follow`, `undecided` or `none`.
    subtitles: String,
    /// The creator to follow; only with `follow`, and one of the anime's.
    #[serde(default)]
    creator: Option<String>,
    /// The save folder, relative to the collect folder.
    directory: String,
}

#[derive(Serialize)]
struct Subscribed {
    rule: RuleView,
}

async fn subscribe(
    State(state): State<AppState>,
    parsed: Result<Json<SubscribeBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Subscribed>), ApiError> {
    let Json(b) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;

    let subtitles = SubtitleMode::parse(&b.subtitles).ok_or_else(|| ApiError::invalid(BAD_BODY))?;
    let directory = b.directory.trim().to_owned();
    rules_api::check_work_folder(&directory)?;
    if std::path::Path::new(&directory).is_absolute() {
        return Err(ApiError::invalid(
            "저장 폴더는 수집 폴더 아래 경로로 적어 주세요. /로 시작하면 안 돼요.",
        ));
    }
    let channel = state
        .channels
        .get_channel(&b.channel_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::not_found("채널을 찾지 못했어요. 이미 삭제됐을 수 있어요."))?;

    // The release title is one this channel's history holds. A subscription
    // that waits for its title has none, and needs a history to be new against.
    let items = rules_api::channel_items(&state.history, &channel.id).await?;
    let phrase = match &b.work {
        Some(work) => Some(recorded_work(&items, work)?),
        None if items.is_empty() => {
            return Err(ApiError::invalid(
                "이 채널에는 수집 기록이 아직 없어요. worker가 채널을 한 번 읽은 뒤에 첫 화 전 작품을 구독해 주세요. 그 기록보다 나중에 올라온 제목만 제목 후보가 돼요.",
            ))
        }
        None => None,
    };

    let anime = scheduled_anime(&state, b.week, b.anissia_anime_no).await?;
    let creator =
        chosen_creator(&state, subtitles, b.creator.as_deref(), b.anissia_anime_no).await?;

    let input = RuleInput {
        r#match: phrase,
        directory,
        ..RuleInput::default()
    };
    rules_api::check_directory(&state, None, &input.directory).await?;
    let created = state
        .channels
        .create_subscription_rule(
            &channel.id,
            input,
            NewSubscription {
                anime,
                subtitles,
                creator,
                subscribed_at: state.anissia.now(),
            },
        )
        .await
        .map_err(|e| match e {
            ChannelError::Invalid(_) => ApiError::invalid("입력한 값으로는 구독할 수 없어요."),
            e => e.into(),
        })?;
    Ok((
        StatusCode::CREATED,
        Json(Subscribed {
            rule: rules_api::rule_view(&state, &created.id).await?,
        }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreatorBody {
    /// The version the client saw.
    version: i64,
    /// The creator to follow, one of the anime's; `null` is `제작자 미정`.
    creator: Option<String>,
}

/// The stored rule at the version the client saw, or the `404`/`409` to answer.
async fn rule_at(
    state: &AppState,
    id: &str,
    version: i64,
) -> Result<trss_collect::store::channels::Rule, ApiError> {
    let rule = state
        .channels
        .get_rule(id)
        .await
        .map_err(rules_api::store_error)?
        .ok_or_else(|| ApiError::not_found("규칙을 찾지 못했어요. 이미 삭제됐을 수 있어요."))?;
    if rule.version != version {
        return Err(rules_api::rule_conflict(state, id).await);
    }
    Ok(rule)
}

/// The answer of a write on a rule: its view now, or the conflict when the
/// store found the version changed meanwhile.
async fn rule_after(
    state: &AppState,
    id: &str,
    written: Result<trss_collect::store::channels::Rule, ChannelError>,
) -> Result<Json<RuleView>, ApiError> {
    match written {
        Ok(_) => Ok(Json(rules_api::rule_view(state, id).await?)),
        Err(e) if e.is_conflict() => Err(rules_api::rule_conflict(state, id).await),
        Err(e) => Err(rules_api::store_error(e)),
    }
}

async fn change_creator(
    State(state): State<AppState>,
    Path(id): Path<String>,
    parsed: Result<Json<CreatorBody>, JsonRejection>,
) -> Result<Json<RuleView>, ApiError> {
    let Json(b) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;
    let rule = rule_at(&state, &id, b.version).await?;
    let Some(subscription) = &rule.subscription else {
        return Err(ApiError::invalid(
            "편성표와 연결된 규칙만 자막 제작자를 정해요.",
        ));
    };
    if subscription.subtitles == SubtitleMode::None {
        return Err(ApiError::invalid(
            "자막을 받지 않는 구독이에요. 자막 받기를 켠 뒤 제작자를 바꿔 주세요.",
        ));
    }
    let creator = match b.creator.as_deref().map(str::trim) {
        Some("") => return Err(ApiError::invalid("따라 받을 자막 제작자를 골라 주세요.")),
        Some(name) => Some(named_creator(&state, subscription.anissia_anime_no, name).await?),
        None => None,
    };
    // The rule may have changed while Anissia was asked; the store checks again.
    let written = state.channels.set_creator(&id, b.version, creator).await;
    rule_after(&state, &id, written).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkBody {
    /// The version the client saw.
    version: i64,
    /// Anissia's `animeNo`.
    anissia_anime_no: i64,
    /// The schedule week the anime was picked from, to find its entry.
    week: u8,
    /// `follow`, `undecided` or `none`.
    subtitles: String,
    /// The creator to follow; only with `follow`, and one of the anime's.
    #[serde(default)]
    creator: Option<String>,
}

async fn link_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
    parsed: Result<Json<LinkBody>, JsonRejection>,
) -> Result<Json<RuleView>, ApiError> {
    let Json(b) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;
    let subtitles = SubtitleMode::parse(&b.subtitles).ok_or_else(|| ApiError::invalid(BAD_BODY))?;
    let rule = rule_at(&state, &id, b.version).await?;
    if rule.subscription.is_some() {
        return Err(ApiError::invalid(
            "이미 편성표와 연결된 규칙이에요. 화면을 새로고침해 주세요.",
        ));
    }
    // A subscription saves into a work folder below the collect folder, as
    // `subscribe` requires; a plain rule may save into the collect folder itself.
    rules_api::check_stored_work_folder(&rule)?;
    let anime = scheduled_anime(&state, b.week, b.anissia_anime_no).await?;
    let creator =
        chosen_creator(&state, subtitles, b.creator.as_deref(), b.anissia_anime_no).await?;
    let written = state
        .channels
        .subscribe_rule(
            &id,
            b.version,
            NewSubscription {
                anime,
                subtitles,
                creator,
                subscribed_at: state.anissia.now(),
            },
        )
        .await;
    match written {
        Err(ChannelError::AlreadySubscribed { .. }) => Err(ApiError::invalid(
            "이 채널에서 이미 구독 중인 작품이에요. 그 구독 규칙을 열어 주세요.",
        )),
        written => rule_after(&state, &id, written).await,
    }
}

#[cfg(test)]
mod tests;
