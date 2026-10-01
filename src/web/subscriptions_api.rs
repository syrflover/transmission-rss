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
//!
//! Anissia is a third party: a call that needs it and cannot get an answer
//! fails with `502 { error: "unavailable", message }` and a sentence that says
//! why, so the screen can show the reason and a retry. The web waits for the
//! shared request pace for a few seconds at most.
//!
//! Subscribing creates the rule and nothing else. It receives nothing: the
//! items history recorded before are past, the cycle leaves them alone, and
//! the user receives the ones they pick with `receive_once` and the new rule's
//! ID (see [`crate::worker::commands::receive_once`]).
//!
//! Everything a subscription is made of is checked here against the stored
//! data and Anissia, not trusted from the request: the release title must be
//! one the channel's history holds, the anime one the schedule lists, and a
//! subtitle creator one of the anime's captions names.

use std::{collections::HashMap, time::Duration};

use axum::{
    extract::{rejection::JsonRejection, rejection::QueryRejection, Path, Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    rules_api::{self, RuleView},
    ApiError, AppState,
};
use crate::{
    anissia::{AnissiaError, Fetched, LAST_WEEK},
    store::{
        anissia::{Anime, WEEK_OTHER, WEEK_UPCOMING},
        channels::{
            ChannelError, NewSubscription, RuleInput, RuleState, Subscription, SubtitleMode,
        },
        history::Millis,
    },
    subscriptions::{folder_suggestion, title_groups, work_key, Quarter},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/anissia/schedule/{week}", get(schedule))
        .route("/anissia/anime/{no}/creators", get(creators))
        .route("/subscriptions", get(list).post(subscribe))
        .route("/subscriptions/titles", get(titles))
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
}

pub fn subscription_brief(
    subscription: &Subscription,
    animes: &HashMap<i64, Anime>,
) -> SubscriptionBrief {
    SubscriptionBrief {
        anissia_anime_no: subscription.anissia_anime_no,
        subtitles: subscription.subtitles.as_str(),
        creator: subscription.creator.clone(),
        season_id: subscription.season_id.clone(),
        subscribed_at: subscription.subscribed_at,
        anime: animes
            .get(&subscription.anissia_anime_no)
            .map(AnimeView::from),
    }
}

#[derive(Debug, Serialize)]
struct QuarterView {
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

fn creators_of(fetched: &Fetched<crate::anissia::Caption>) -> Vec<CreatorView> {
    crate::anissia::parse::creators(&fetched.value)
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
    /// Active subscriptions, this quarter's and any begun in earlier quarters
    /// that are still collected first, then the ones of a coming quarter.
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
        .anissia
        .store
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
            if rule.state != RuleState::Active {
                continue;
            }
            let anime = animes.get(&subscription.anissia_anime_no);
            let quarter = anime
                .and_then(|a| a.start_date.as_deref())
                .and_then(Quarter::of_date)
                .unwrap_or_else(|| Quarter::at(subscription.subscribed_at));
            items.push(SubscriptionItem {
                rule_id: rule.id.clone(),
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
    /// [`TitleView::work`]): the rule's match phrase.
    work: String,
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
    if directory.is_empty() {
        return Err(ApiError::invalid("저장 폴더를 적어 주세요."));
    }
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

    // The release title is one this channel's history holds.
    let items = rules_api::channel_items(&state.history, &channel.id).await?;
    let wanted = work_key(&b.work);
    let group = title_groups(&items)
        .into_iter()
        .find(|g| work_key(&g.work) == wanted)
        .ok_or_else(|| {
            ApiError::invalid(
                "이 채널의 수집 기록에 없는 릴리스 제목이에요. 기록에 있는 제목에서 골라 주세요.",
            )
        })?;

    // The anime is one the schedule lists, and its snapshot is that entry.
    let listed = state
        .anissia
        .schedule(b.week, Some(USER_MAX_WAIT))
        .await
        .map_err(unavailable)?;
    let entry = listed
        .value
        .iter()
        .find(|e| e.anime_no == b.anissia_anime_no)
        .ok_or_else(|| {
            ApiError::invalid(
                "편성표에서 이 작품을 찾지 못했어요. 편성표를 다시 불러와 골라 주세요.",
            )
        })?;
    let anime = entry.snapshot(listed.fetched_at);

    // A creator to follow is one the anime's captions name.
    let creator = match (subtitles, b.creator.as_deref().map(str::trim)) {
        (SubtitleMode::Follow, Some(name)) if !name.is_empty() => {
            let captions = state
                .anissia
                .captions(b.anissia_anime_no, Some(USER_MAX_WAIT))
                .await
                .map_err(unavailable)?;
            if !creators_of(&captions).iter().any(|c| c.name == name) {
                return Err(ApiError::invalid(
                    "이 작품의 자막 목록에 없는 제작자예요. 목록에서 골라 주세요.",
                ));
            }
            Some(name.to_owned())
        }
        (SubtitleMode::Follow, _) => {
            return Err(ApiError::invalid("따라 받을 자막 제작자를 골라 주세요."));
        }
        (_, None) => None,
        (_, Some(_)) => {
            return Err(ApiError::invalid(
                "자막 제작자는 제작자를 따라 받을 때만 정해요.",
            ));
        }
    };

    let input = RuleInput {
        r#match: Some(group.work),
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

#[cfg(test)]
mod tests;
