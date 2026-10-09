//! `/api/rules`: the rules of every channel in one list, editing them, and the
//! preview of what a rule would do with the items the worker has recorded.
//!
//! | call                                   | success                                   |
//! | -------------------------------------- | ----------------------------------------- |
//! | `GET /rules`                           | `200 { rules: [RuleView], channels: [ChannelBrief] }` |
//! | `GET /rules/{id}`                      | `200 RuleView`                            |
//! | `POST /rules`                          | `201 RuleView`                            |
//! | `PUT /rules/{id}`                      | `200 RuleView`                            |
//! | `DELETE /rules/{id}?version=N`         | `200 { removed: true }`                   |
//! | `PUT /rules/order`                     | `200 { rules: [RuleView] }` (the channel's) |
//! | `PUT /rules/{id}/switch`               | `200 RuleView`                            |
//! | `PUT /rules/{id}/episode`              | `200 RuleView`                            |
//! | `GET /rules/archived-work?directory=`  | `200 { archived: ArchivedWorkView \| null }` |
//! | `POST /rules/preview`                  | `200 Preview`                             |
//!
//! Failures use the shape in [`super::error`]. A version that is not the
//! stored one answers `409` with the rule's current [`RuleView`] as `current`
//! (for `PUT /rules/order`: the channel's current rules). A regular expression
//! that does not compile is refused with `400` and a sentence; nothing is saved.
//!
//! A rule's `state` is not edited by `PUT`, which refuses a `state` other than
//! the stored one. Archiving and restoring go through the `rule_archive`
//! command (`/api/commands`), because the worker has to turn the rule off
//! before its folder moves to the archive folder and on only after it moved
//! back. A rule's view carries the last such command as `archive_move`.
//!
//! `GET /rules/archived-work?directory=...[&from=...]` tells a form, before it
//! makes a rule saving to `directory` (the folder as typed: the server finds
//! its work folder), whether the rule's work folder is a work in the archive
//! folder: `200 { archived: null | { work, archive_folder, collect_folder,
//! merges } }`. It reads the library's records of the archive and collect watch
//! folders by folder name, never the disk ([`rule_archive::archived_work`]).
//! The form of a stored rule sends its folder as `from`, and a `directory` in
//! the same work folder is `null`.
//!
//! **A rule that starts collecting waits for its work folder.** When a rule or
//! a subscription is made as collecting, a collecting rule is saved into
//! another work folder, or `영상 받기` is switched on, and the rule's work
//! folder is in the archive folder ([`rule_archive::plan_start`], read from the
//! disk), the rule is made `paused` (or stays so) and the web stores a `start`
//! (a new rule, an edit of the folder, or the retry of a `start` that failed:
//! the rule counts as collecting all along) or `resume` (`영상 받기`)
//! `rule_archive` command, which the worker runs: it moves the work folder into
//! the collect folder first, merging into the one there, and turns the rule on
//! only then. A paused rule collects nothing, so no RSS check can make the work
//! folder in the collect folder before the move. The answer is the rule as it is
//! then: `paused`, with the open `start` as `archive_move`. While that command
//! is open a switch is refused (`400`, `영상 받기` off too, since the worker
//! turns the rule on whatever is pressed); when it fails the rule stays paused
//! with the reason in `archive_move`, and switching `영상 받기` on again tries
//! again.
//!
//! `PUT /rules/{id}/switch` (`{ version, video?: bool, subtitles?: bool }`) is
//! the rule detail's pair of switches, applied at once: `video` is `영상 받기`
//! (`active` or, off, `paused`: the rule collects nothing and its folder stays
//! where it is) and `subtitles` is `자막 받기` of a subscription (off is the
//! subtitle mode `none`, which keeps the creator). Exactly one is sent. A
//! switch is refused (`400`) for an archived rule, for `subtitles` while
//! `영상 받기` is off, and for `subtitles` of a rule that is no subscription.
//! A version that is not the stored one answers `409` with the current view.
//! Turning `영상 받기` on notes the time on the rule (`resumed_at`), as does a
//! restore: the items first seen before then are left to the user (see the
//! preview below).
//!
//! `PUT /rules/{id}/episode` (`{ version, episode }`) is `적용` of the episode
//! offset the rule detail suggests, and saves it as the user's own value (see
//! [`episode`]). A view carries `episode_basis` (why the app set the offset it
//! did, when it did), `episode_previous` (the offset it replaced) and
//! `episode_suggestion` (`{ value, basis }`, what the app offers when it did
//! not).
//!
//! A subscription's view also tells where it stands in the library:
//! `season` (the season its received videos appeared in, with the work's name,
//! cover, how many episodes have a video and the AniList episode count) and
//! `season_blocked` (the season they appeared in is held by another Anissia
//! anime, so the rule was not connected; `held_by` says whether another
//! `subscription` holds it, or the season is linked to that anime from the work
//! detail, `link`).
//!
//! A save folder is also refused (`400`, with a sentence) when it is new or
//! changed and:
//!
//! - has a `..` component, which could lead anywhere once links are followed.
//!   A folder stored before this rule keeps saving as it is;
//! - belongs to a rule whose `rule_archive` command is still open: the move
//!   decided on the old folder;
//! - is inside a work folder that an open `rule_archive` command is moving.
//!
//! # The preview is the worker's evaluation
//!
//! The preview does not judge titles itself. [`trss_collect::plan::preview`]
//! puts the edited rule into the channel's stored rules (at the requested place
//! in the order), hands that to the very mapping the worker uses
//! ([`ChannelPlan`], which builds the shared [`trss_collect::rss`] evaluation
//! from stored channels and rules) and judges every item the channel has in the
//! collection history with it; this API loads the items and says the result in
//! the wire's shapes. So the channel's
//! excludes, collect folder and rule order apply exactly as they do in a
//! cycle, and the same items and settings give the same selection, applied
//! rule and save path. The history holds the worker's last read of the feed
//! too, so the web never reads RSS.
//!
//! One limit: history keeps titles with the channel's long secret query values
//! replaced by `***` (see the history module). The preview judges what is
//! stored, so a title that contained such a value may judge differently from
//! the worker, which saw the original. Items whose stored title contains the
//! mask are flagged `masked`.
//!
//! A subscription rule leaves alone the items history recorded, without a
//! rule taking them, before the subscription began, before a subscription that
//! waited for its title was given one (a save that fills the empty match phrase
//! of one is previewed as if it were saved after everything recorded so far),
//! and any rule leaves alone those it recorded while the rule was paused or
//! archived (before it was last turned back on). A subscription also leaves
//! alone the items the feed already held when the channel was first read, even
//! when it began before that read (the first read had no history to tell old
//! items from new; the items it recorded are marked
//! ([`HistoryItem::first_read`])). The preview applies the cycle's own test
//! ([`ChannelPlan::is_past`]) to the item's history record and lists such an
//! item as `past` (with `past_cause` and the folder that `받기` would save it
//! to) rather than `mine`; a `receive_once` command naming the rule receives
//! it. A rule that is paused now is previewed as if turned on after the
//! recorded items.
//!
//! # Overlap
//!
//! A rule is `overlap` when some recorded item is taken by an earlier rule
//! although this rule matches it too. It is computed over the recorded items
//! with the channel's active rules ([`preview::overlapping_rules`]); archived
//! rules neither take items nor overlap.

use std::collections::{HashMap, HashSet};
use std::path::Path as FsPath;

use axum::{
    extract::{
        rejection::{JsonRejection, QueryRejection},
        Path, Query, State,
    },
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    artwork_api::image_url,
    commands_api::CommandView,
    subscriptions_api::{subscription_brief, SubscriptionBrief},
    ApiError, AppState,
};
use trss_anissia::Anime;
use trss_collect::{
    commands::rule_archive::{self, RuleArchive},
    plan::{preview, ChannelPlan, PastCause},
    rss::regex_error,
    store::{
        channels::{
            Channel, ChannelError, ChannelWithRules, OrderItem, Rule, RuleInput, RuleState,
            SeasonRef,
        },
        history::{HistoryError, HistoryItem, HistoryQuery, HistoryStore, MAX_PAGE_SIZE},
    },
};
use trss_core::commands::{Accepted, Command, CommandState};

mod episode;
#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/rules", get(list_rules).post(create_rule))
        .route("/rules/preview", post(preview))
        .route("/rules/order", put(reorder_rules))
        .route("/rules/archived-work", get(archived_work))
        .route("/rules/{id}/switch", put(switch_rule))
        .route("/rules/{id}/episode", put(episode::put_episode))
        .route(
            "/rules/{id}",
            get(read_rule).put(update_rule).delete(delete_rule),
        )
}

/// The most recorded items of one channel that are read for the list and the
/// preview. History is kept for good, so this bounds the work of a request;
/// the newest items are read first.
const MAX_ITEMS_PER_CHANNEL: usize = 20_000;

/// How many matching items a preview lists (the counts cover all of them).
const PREVIEW_LIST_LIMIT: usize = 100;

// ---------------------------------------------------------------------------
// Response shapes
// ---------------------------------------------------------------------------

/// A regular expression that does not compile, said for people.
#[derive(Debug, Clone, Serialize)]
pub struct RegexProblem {
    /// A full sentence for the screen.
    pub message: String,
    /// The regex library's own reason, short and in English.
    pub detail: String,
}

fn regex_problem(err: &regex::Error) -> RegexProblem {
    match err {
        regex::Error::CompiledTooBig(_) => RegexProblem {
            message: "정규식이 너무 커요. 더 짧게 줄여 주세요.".into(),
            detail: "compiled program too big".into(),
        },
        other => {
            let text = other.to_string();
            let detail = text
                .lines()
                .rev()
                .find_map(|line| line.trim().strip_prefix("error:"))
                .map(|reason| reason.trim().to_owned())
                .unwrap_or_else(|| text.lines().last().unwrap_or_default().trim().to_owned());
            RegexProblem {
                message: "정규식이 올바르지 않아요. 괄호와 특수 문자를 확인해 주세요.".into(),
                detail,
            }
        }
    }
}

/// A stored rule as the screen sees it.
#[derive(Debug, Serialize)]
pub struct RuleView {
    pub id: String,
    pub channel_id: String,
    /// Send back as `version` when saving, deleting or reordering.
    pub version: i64,
    /// Place among the channel's rules, archived ones included, from 1. This
    /// is the order the worker checks the rules in.
    pub order: usize,
    /// The match phrase; `null` while the rule waits for its title.
    pub r#match: Option<String>,
    pub regex: bool,
    pub case_insensitive: bool,
    /// Relative to the collect folder.
    pub directory: String,
    pub episode: i64,
    pub episode_auto: bool,
    /// Why the app set the offset by itself, when it did and the rule has the
    /// sentence (see [`trss_collect::episode_offset`]).
    pub episode_basis: Option<String>,
    /// The offset the rule had before the app set its own, while the offset
    /// is the app's and the value it replaced is known.
    pub episode_previous: Option<i64>,
    /// What the app offers for the offset of a rule it did not set one for.
    pub episode_suggestion: Option<episode::EpisodeSuggestion>,
    /// The last `되돌리기` of the rule's automatic offset (an `episode_undo`
    /// command), open or ended; `null` when it never had one.
    pub episode_undo: Option<episode::EpisodeUndoView>,
    /// `active`, `paused` or `archived`.
    pub state: &'static str,
    /// An earlier rule takes an item that this rule also matches.
    pub overlap: bool,
    /// Set when the rule's regular expression does not compile.
    pub error: Option<RegexProblem>,
    /// When the rule last got an item into Transmission (Unix ms), if ever.
    pub last_received_at: Option<i64>,
    /// The last archive or restore of the rule (a `rule_archive` command),
    /// open or ended; `null` when it never had one.
    pub archive_move: Option<ArchiveMoveView>,
    /// Set when the rule follows an anime of Anissia's schedule.
    pub subscription: Option<SubscriptionBrief>,
    /// The season a subscription is connected to; `null` before it is, or
    /// when the work left the library.
    pub season: Option<RuleSeasonView>,
    /// Why a subscription is not connected: its videos are in a season that
    /// another anime holds.
    pub season_blocked: Option<SeasonBlockedView>,
}

/// The season a subscription is connected to and how far it has come.
#[derive(Debug, Clone, Serialize)]
pub struct RuleSeasonView {
    pub season_id: String,
    pub work_id: String,
    /// The work's folder name.
    pub work_name: String,
    pub number: u32,
    /// Where the work's cover is served, if it has one.
    pub cover_url: Option<String>,
    /// How many episodes of the season have a video.
    pub videos: u32,
    /// The season's episode count by AniList, `null` when unknown.
    pub episodes: Option<u32>,
}

/// A season that another anime holds, which kept a rule from connecting.
#[derive(Debug, Clone, Serialize)]
pub struct SeasonBlockedView {
    pub work_id: String,
    pub work_name: Option<String>,
    pub number: u32,
    /// Anissia's `animeNo` of the holder, and its title when the app has one.
    pub holder_anime_no: Option<i64>,
    pub holder_subject: Option<String>,
    /// What holds the season: another `subscription`, or the anime the season
    /// was linked to from the work detail (`link`), which the work detail changes.
    pub held_by: &'static str,
}

/// An archive or restore of a rule and where it is.
#[derive(Debug, Serialize)]
pub struct ArchiveMoveView {
    /// `archive` or `restore`.
    pub direction: &'static str,
    /// The command; its outcome's `result` is `moved`, `kept` (the rule
    /// changed state and the folder stayed, for the `reason`) or `failed`.
    pub command: CommandView,
}

fn archive_move_view(command: &Command) -> Option<ArchiveMoveView> {
    let payload: RuleArchive = serde_json::from_str(&command.payload).ok()?;
    Some(ArchiveMoveView {
        direction: payload.direction.code(),
        command: CommandView::from(command),
    })
}

/// The channel a rule belongs to, enough to name and group its rules.
#[derive(Debug, Serialize)]
pub struct ChannelBrief {
    pub id: String,
    pub position: i64,
    pub name: Option<String>,
    pub host: String,
    pub rule_count: usize,
}

fn host_of(channel: &Channel) -> String {
    Url::parse(&channel.url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}

fn brief(cwr: &ChannelWithRules) -> ChannelBrief {
    ChannelBrief {
        id: cwr.channel.id.clone(),
        position: cwr.channel.position,
        name: cwr.channel.name.clone(),
        host: host_of(&cwr.channel),
        rule_count: cwr.rules.len(),
    }
}

#[derive(Debug, Serialize)]
struct RuleList {
    rules: Vec<RuleView>,
    channels: Vec<ChannelBrief>,
    /// The app's collect folder, which every rule's `directory` is relative
    /// to: a rule saves to `collect_folder` + `directory`. `null` while it is
    /// not set.
    collect_folder: Option<String>,
}

#[derive(Serialize)]
struct RuleGroup {
    rules: Vec<RuleView>,
}

#[derive(Serialize)]
struct Removed {
    removed: bool,
}

// ---------------------------------------------------------------------------
// Reading the history
// ---------------------------------------------------------------------------

fn history_error(e: HistoryError) -> ApiError {
    ApiError::Internal(e.to_string())
}

pub(super) fn store_error(e: ChannelError) -> ApiError {
    match e {
        ChannelError::Invalid(_) => ApiError::invalid("입력한 값으로는 저장할 수 없어요."),
        e => e.into(),
    }
}

/// The collect folder, or `None` while it is not set.
async fn collect_folder(state: &AppState) -> Result<Option<String>, ApiError> {
    Ok(state
        .settings
        .collection()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map(|settings| settings.folder))
}

/// The collect folder and the archive folder, or `None` while no collect folder
/// is set.
async fn collection_folders(
    state: &AppState,
) -> Result<Option<(String, Option<String>)>, ApiError> {
    Ok(state
        .settings
        .collection()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map(|settings| (settings.folder, settings.archive_folder)))
}

/// Whether a rule saving to `directory` has to wait for its work folder to come
/// out of the archive folder before it collects (see the module docs).
pub(super) async fn waits_for_work_folder(
    state: &AppState,
    directory: &str,
) -> Result<bool, ApiError> {
    let plan = rule_archive::plan_start(collection_folders(state).await?, directory).await;
    Ok(matches!(plan, rule_archive::StartPlan::MoveFirst { .. }))
}

/// Whether saving to `to` instead of `from` puts a rule into another work
/// folder (the first part of the folder below the collect folder).
pub(super) async fn changes_work_folder(
    state: &AppState,
    from: &str,
    to: &str,
) -> Result<bool, ApiError> {
    let Some(collect) = collect_folder(state).await? else {
        return Ok(false);
    };
    let collect = FsPath::new(&collect);
    Ok(rule_archive::work_folder(collect, from) != rule_archive::work_folder(collect, to))
}

/// Stores the `start` (a new rule) or `resume` (`영상 받기` on) command of the
/// paused rule `rule_id` and wakes the worker. A `start` carries the past items
/// `receive` to receive once the rule is on ([`rule_archive::RuleArchive::receive`]).
/// `false` when the rule has another `rule_archive` command open.
pub(super) async fn ask_start(
    state: &AppState,
    rule_id: &str,
    direction: rule_archive::Direction,
    receive: Vec<i64>,
) -> Result<bool, ApiError> {
    let accepted = rule_archive::ask_start_receiving(
        &state.commands,
        rule_id,
        direction,
        receive,
        super::commands_api::now_millis(),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    match accepted {
        Accepted::Created(_) => {
            state.wake_worker();
            Ok(true)
        }
        Accepted::Existing(_) => Ok(true),
        Accepted::Mismatch(_) | Accepted::Busy(_) => Ok(false),
    }
}

/// [`ask_start`] with `start` for a rule just made, or edited into another work
/// folder: the rule exists paused whatever happens, so a start that could not
/// be stored is logged, and `영상 받기` switched on starts it again. `receive`
/// are the past items a subscription's person ticked.
pub(super) async fn start_new_rule(state: &AppState, rule_id: &str, receive: Vec<i64>) {
    match ask_start(state, rule_id, rule_archive::Direction::Start, receive).await {
        Ok(true) => {}
        Ok(false) => eprintln!("trss-web: rule {rule_id} already has a rule_archive command open"),
        Err(e) => eprintln!("trss-web: cannot store the start of rule {rule_id}: {e:?}"),
    }
}

/// Refuses a save folder typed for a subscription that is no folder below the
/// collect folder: a subscription saves into a work folder, never into the
/// collect folder itself (`.` and `./` name it, as an empty one does).
pub(super) fn check_work_folder(directory: &str) -> Result<(), ApiError> {
    let directory = directory.trim();
    if directory.is_empty() {
        return Err(ApiError::invalid("저장 폴더를 적어 주세요."));
    }
    if trss_core::folders::is_collect_folder_itself(FsPath::new(directory)) {
        return Err(ApiError::invalid(
            "저장 폴더로 `.`만 적을 수는 없어요. 수집 폴더 자체에 받게 되니, 그 아래의 작품 폴더 이름을 적어 주세요.",
        ));
    }
    Ok(())
}

/// [`check_work_folder`] for the folder a rule already has, which the request
/// does not name: the user is told to choose one first.
pub(super) fn check_stored_work_folder(rule: &Rule) -> Result<(), ApiError> {
    let directory = rule.directory.trim();
    if directory.is_empty() || trss_core::folders::is_collect_folder_itself(FsPath::new(directory))
    {
        return Err(ApiError::invalid(
            "이 규칙은 수집 폴더 자체에 받아요. 구독은 작품 폴더에 받으니, 규칙의 저장 폴더를 작품 폴더로 먼저 정해 주세요.",
        ));
    }
    Ok(())
}

/// Refuses a save folder `directory` for a new rule (`stored` is `None`) or
/// a changed one. See the module docs.
pub(super) async fn check_directory(
    state: &AppState,
    stored: Option<&Rule>,
    directory: &str,
) -> Result<(), ApiError> {
    if stored.is_some_and(|rule| rule.directory == directory) {
        return Ok(());
    }
    if trss_core::folders::has_parent_dir(FsPath::new(directory)) {
        return Err(ApiError::invalid(
            "저장 폴더에는 `..`를 쓸 수 없어요. 수집 폴더 아래 경로를 `..` 없이 적어 주세요.",
        ));
    }

    let rules: Vec<Rule> = state
        .channels
        .list_channels_with_rules()
        .await
        .map_err(store_error)?
        .into_iter()
        .flat_map(|cwr| cwr.rules)
        .collect();
    let open = archive_moves(state, &rules).await?;
    let moving: Vec<&Rule> = rules
        .iter()
        .filter(|rule| open.get(&rule.id).is_some_and(|c| c.state.is_open()))
        .collect();
    if moving.is_empty() {
        return Ok(());
    }
    if let Some(stored) = stored {
        if moving.iter().any(|rule| rule.id == stored.id) {
            return Err(ApiError::invalid(
                "이 규칙의 작품 폴더를 옮기는 중이라 저장 폴더를 바꿀 수 없어요. 옮기기가 끝난 뒤 다시 시도해 주세요.",
            ));
        }
    }
    let Some(collect) = collect_folder(state).await? else {
        return Ok(());
    };
    let collect = FsPath::new(&collect);
    let rule_archive::WorkFolder::Named(name) = rule_archive::work_folder(collect, directory)
    else {
        return Ok(());
    };
    if moving.iter().any(|rule| {
        rule_archive::work_folder(collect, &rule.directory)
            == rule_archive::WorkFolder::Named(name.clone())
    }) {
        return Err(ApiError::invalid(format!(
            "지금 옮기는 중인 작품 폴더 `{name}` 안으로는 규칙을 만들거나 옮길 수 없어요. 옮기기가 끝난 뒤 다시 시도해 주세요."
        )));
    }
    Ok(())
}

/// The channel's recorded items, newest first, up to [`MAX_ITEMS_PER_CHANNEL`].
pub(super) async fn channel_items(
    history: &HistoryStore,
    channel_id: &str,
) -> Result<Vec<HistoryItem>, ApiError> {
    Ok(channel_items_window(history, channel_id).await?.0)
}

/// [`channel_items`] and whether the history holds older items than it read.
pub(super) async fn channel_items_window(
    history: &HistoryStore,
    channel_id: &str,
) -> Result<(Vec<HistoryItem>, bool), ApiError> {
    channel_items_up_to(history, channel_id, MAX_ITEMS_PER_CHANNEL).await
}

/// [`channel_items_window`] with the number of items at which it stops reading
/// (a whole page past it at most) given.
pub(super) async fn channel_items_up_to(
    history: &HistoryStore,
    channel_id: &str,
    max_items: usize,
) -> Result<(Vec<HistoryItem>, bool), ApiError> {
    let mut items: Vec<HistoryItem> = Vec::new();
    let mut after = None;
    let mut truncated = false;
    loop {
        let page = history
            .list(HistoryQuery {
                channel_id: Some(channel_id.to_owned()),
                after,
                limit: MAX_PAGE_SIZE,
                ..HistoryQuery::default()
            })
            .await
            .map_err(history_error)?;
        items.extend(page.items);
        match page.next {
            Some(next) if items.len() < max_items => after = Some(next),
            Some(_) => {
                truncated = true;
                break;
            }
            None => break,
        }
    }
    Ok((items, truncated))
}

// ---------------------------------------------------------------------------
// Views of stored rules
// ---------------------------------------------------------------------------

/// What the recorded items say about a channel's stored rules.
#[derive(Default)]
struct Analysis {
    /// Rules that an earlier rule shadows for some recorded item.
    overlap: HashSet<String>,
    /// Latest time each rule got an item into Transmission.
    last_received: HashMap<String, i64>,
    errors: HashMap<String, RegexProblem>,
    /// The stored schedule snapshots of the subscribed anime.
    animes: HashMap<i64, Anime>,
    /// The connected season of each subscription rule.
    seasons: HashMap<String, RuleSeasonView>,
    /// The season that kept a subscription rule from connecting.
    blocked: HashMap<String, SeasonBlockedView>,
    /// The grounds and suggestions for the rules' episode offsets.
    episodes: episode::Episodes,
}

/// The season of `season_id` as a subscription's progress shows it, if the
/// work is still in the library.
async fn season_of(state: &AppState, season_id: &str) -> Result<Option<RuleSeasonView>, ApiError> {
    let Some(parsed) = SeasonRef::parse(season_id) else {
        return Ok(None);
    };
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let Some(holdings) = state
        .library
        .season_holdings(&parsed.work_id, parsed.number)
        .await
        .map_err(|e| internal(&e))?
    else {
        return Ok(None);
    };
    let episodes = state
        .seasons
        .store
        .link(&parsed.work_id, parsed.number)
        .await
        .map_err(|e| internal(&e))?;
    let episodes =
        trss_library::seasons::combine::combine(&episodes.entries).and_then(|c| c.episodes);
    let cover_url = state
        .artwork
        .store
        .selection(&parsed.work_id)
        .await
        .map_err(|e| internal(&e))?
        .image
        .map(|image| image_url(&parsed.work_id, &image.id));
    Ok(Some(RuleSeasonView {
        season_id: season_id.to_owned(),
        work_id: parsed.work_id,
        work_name: holdings.dir_name,
        number: parsed.number,
        cover_url,
        videos: holdings.videos,
        episodes,
    }))
}

/// The reason a subscription is not connected to `season_id`.
async fn blocked_by(state: &AppState, season_id: &str) -> Result<SeasonBlockedView, ApiError> {
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let (work_id, number) = match SeasonRef::parse(season_id) {
        Some(parsed) => (parsed.work_id, parsed.number),
        None => (season_id.to_owned(), 0),
    };
    let work_name = state
        .library
        .season_holdings(&work_id, number)
        .await
        .map_err(|e| internal(&e))?
        .map(|h| h.dir_name);
    let holder_anime_no = state
        .channels
        .season_holder(season_id)
        .await
        .map_err(store_error)?;
    let holder_subject = match holder_anime_no {
        Some(no) => state
            .anissia_store
            .anime(no)
            .await
            .map_err(|e| internal(&e))?
            .map(|a| a.subject),
        None => None,
    };
    let held_by_subscription = state
        .channels
        .subscriptions_of_work(&work_id)
        .await
        .map_err(store_error)?
        .iter()
        .any(|(season, _)| *season == number);
    Ok(SeasonBlockedView {
        work_id,
        work_name,
        number,
        holder_anime_no,
        holder_subject,
        held_by: if held_by_subscription {
            "subscription"
        } else {
            "link"
        },
    })
}

async fn analyze(state: &AppState, cwr: &ChannelWithRules) -> Result<Analysis, ApiError> {
    let mut analysis = Analysis::default();
    if cwr.rules.is_empty() {
        return Ok(analysis);
    }
    let subscribed: Vec<i64> = cwr
        .rules
        .iter()
        .filter_map(|r| r.subscription.as_ref().map(|s| s.anissia_anime_no))
        .collect();
    if !subscribed.is_empty() {
        analysis.animes = state
            .anissia_store
            .animes(subscribed)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    analysis.episodes = episode::analyze(state, cwr).await;
    // Only the judgement is used here, never a save path.
    let plan = ChannelPlan::new(cwr.clone(), FsPath::new(""));
    for rule in &cwr.rules {
        let Some(subscription) = &rule.subscription else {
            continue;
        };
        if let Some(season_id) = &subscription.season_id {
            if let Some(season) = season_of(state, season_id).await? {
                analysis.seasons.insert(rule.id.clone(), season);
            }
        }
        if let Some(season_id) = &subscription.season_blocked {
            // The worker clears the note when nothing holds the season any
            // more; until it does, there is no block to explain.
            let blocked = blocked_by(state, season_id).await?;
            if blocked.holder_anime_no.is_some() {
                analysis.blocked.insert(rule.id.clone(), blocked);
            }
        }
    }
    for problem in plan.rule_errors() {
        analysis
            .errors
            .insert(problem.rule_id, regex_problem(&problem.error));
    }
    // Asked of the store rule by rule, so that an item older than the window
    // of recorded items read below still counts.
    analysis.last_received = state
        .history
        .last_received_of_rules(cwr.rules.iter().map(|r| r.id.clone()).collect())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let items = channel_items(&state.history, &cwr.channel.id).await?;
    analysis.overlap = preview::overlapping_rules(&plan, items.iter().map(|i| i.title.as_str()));
    Ok(analysis)
}

/// The last `rule_archive` command of each of the rules.
async fn archive_moves(
    state: &AppState,
    rules: &[Rule],
) -> Result<HashMap<String, Command>, ApiError> {
    state
        .commands
        .latest_for_subjects(
            rule_archive::KIND,
            rules.iter().map(|r| r.id.clone()).collect(),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

fn views(
    cwr: &ChannelWithRules,
    analysis: &Analysis,
    moves: &HashMap<String, Command>,
) -> Vec<RuleView> {
    cwr.rules
        .iter()
        .enumerate()
        .map(|(index, rule)| RuleView {
            id: rule.id.clone(),
            channel_id: rule.channel_id.clone(),
            version: rule.version,
            order: index + 1,
            r#match: rule.r#match.clone(),
            regex: rule.regex,
            case_insensitive: rule.case_insensitive,
            directory: rule.directory.clone(),
            episode: rule.episode,
            episode_auto: rule.episode_auto,
            episode_basis: analysis.episodes.basis.get(&rule.id).cloned(),
            episode_previous: analysis.episodes.previous.get(&rule.id).copied(),
            episode_suggestion: analysis.episodes.suggestion.get(&rule.id).cloned(),
            episode_undo: analysis.episodes.undo.get(&rule.id).cloned(),
            state: rule.state.as_str(),
            overlap: analysis.overlap.contains(&rule.id),
            error: analysis.errors.get(&rule.id).cloned(),
            last_received_at: analysis.last_received.get(&rule.id).copied(),
            archive_move: moves.get(&rule.id).and_then(archive_move_view),
            subscription: rule
                .subscription
                .as_ref()
                .map(|s| subscription_brief(s, &analysis.animes)),
            season: analysis.seasons.get(&rule.id).cloned(),
            season_blocked: analysis.blocked.get(&rule.id).cloned(),
        })
        .collect()
}

async fn load_channel(state: &AppState, channel_id: &str) -> Result<ChannelWithRules, ApiError> {
    let channel = state
        .channels
        .get_channel(channel_id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ChannelError::NotFound {
            kind: "channel",
            id: channel_id.to_owned(),
        })?;
    let rules = state
        .channels
        .list_rules(channel_id)
        .await
        .map_err(store_error)?;
    Ok(ChannelWithRules { channel, rules })
}

/// The views of a channel's rules as they are now.
async fn channel_views(state: &AppState, channel_id: &str) -> Result<Vec<RuleView>, ApiError> {
    let cwr = load_channel(state, channel_id).await?;
    let analysis = analyze(state, &cwr).await?;
    let moves = archive_moves(state, &cwr.rules).await?;
    Ok(views(&cwr, &analysis, &moves))
}

/// One rule as it is now, or 404.
pub(super) async fn rule_view(state: &AppState, id: &str) -> Result<RuleView, ApiError> {
    let rule = state
        .channels
        .get_rule(id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ChannelError::NotFound {
            kind: "rule",
            id: id.to_owned(),
        })?;
    channel_views(state, &rule.channel_id)
        .await?
        .into_iter()
        .find(|view| view.id == id)
        .ok_or_else(|| {
            ChannelError::NotFound {
                kind: "rule",
                id: id.to_owned(),
            }
            .into()
        })
}

/// 409 with the rule as it is now, or 404 if it is gone.
pub(super) async fn rule_conflict(state: &AppState, id: &str) -> ApiError {
    match rule_view(state, id).await {
        Ok(current) => ApiError::conflict_with(current),
        Err(e) => e,
    }
}

async fn list_rules(State(state): State<AppState>) -> Result<Json<RuleList>, ApiError> {
    let all = state
        .channels
        .list_channels_with_rules()
        .await
        .map_err(store_error)?;
    let mut rules = Vec::new();
    for cwr in &all {
        let analysis = analyze(&state, cwr).await?;
        let moves = archive_moves(&state, &cwr.rules).await?;
        rules.extend(views(cwr, &analysis, &moves));
    }
    Ok(Json(RuleList {
        rules,
        channels: all.iter().map(brief).collect(),
        collect_folder: collect_folder(&state).await?,
    }))
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// The editable fields of a rule, as the screen sends them.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFields {
    /// Blank or `null` means the rule waits for its title.
    #[serde(default)]
    r#match: Option<String>,
    #[serde(default)]
    regex: bool,
    #[serde(default)]
    case_insensitive: bool,
    /// Relative to the collect folder; may be empty.
    #[serde(default)]
    directory: String,
    episode: i64,
    /// `active` (the default), `paused` or `archived`; a save sends the stored
    /// one (see the module docs).
    #[serde(default)]
    state: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    channel_id: String,
    #[serde(flatten)]
    fields: RuleFields,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBody {
    /// The version the client saw.
    version: i64,
    /// The channel the client believes the rule belongs to.
    channel_id: String,
    #[serde(flatten)]
    fields: RuleFields,
}

#[derive(Deserialize)]
struct DeleteQuery {
    version: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OrderBody {
    channel_id: String,
    /// Every rule of the channel once, in the wanted order, each with the
    /// version the client saw.
    order: Vec<OrderEntry>,
}

#[derive(Deserialize)]
struct OrderEntry {
    id: String,
    version: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewBody {
    channel_id: String,
    /// The rule being edited; absent for a rule not saved yet.
    #[serde(default)]
    rule_id: Option<String>,
    /// The edited fields (the `state` is ignored: a preview always judges the
    /// rule as if it were collecting).
    rule: RuleFields,
    /// Where the edited rule stands among the channel's rules, from 0. Absent
    /// keeps its place (a new rule goes last).
    #[serde(default)]
    position: Option<usize>,
    /// The rule not saved yet is a subscription about to be made: the preview
    /// says what the app offers its offset before anything is received
    /// ([`Preview::episode_suggestion`]).
    #[serde(default)]
    subscribing: bool,
}

const BAD_BODY: &str = "요청 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";
const STATE_NOT_EDITED: &str =
    "보관과 복원은 규칙 상세의 `보관`·`복원` 버튼으로 해요. 화면을 새로고침한 뒤 다시 시도해 주세요.";

fn body<T>(parsed: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    parsed
        .map(|Json(v)| v)
        .map_err(|_| ApiError::invalid(BAD_BODY))
}

impl RuleFields {
    /// The store input. The stored rule, when there is one, tells whether the
    /// episode offset was typed or derived.
    fn into_input(self, stored: Option<&Rule>) -> Result<RuleInput, ApiError> {
        let state = match self.state.as_deref() {
            None | Some("active") => RuleState::Active,
            Some("paused") => RuleState::Paused,
            Some("archived") => RuleState::Archived,
            Some(_) => return Err(ApiError::invalid(BAD_BODY)),
        };
        // A blank phrase is a rule waiting for its title. Anything else is
        // kept exactly: a trailing space can be part of the phrase.
        let r#match = self.r#match.filter(|phrase| !phrase.is_empty());
        let directory = self.directory.trim().to_owned();
        if std::path::Path::new(&directory).is_absolute() {
            return Err(ApiError::invalid(
                "저장 폴더는 수집 폴더 아래 경로로 적어 주세요. /로 시작하면 안 돼요.",
            ));
        }
        Ok(RuleInput {
            r#match,
            regex: self.regex,
            case_insensitive: self.case_insensitive,
            directory,
            episode: self.episode,
            // A typed change makes the offset the user's own.
            episode_auto: stored.is_some_and(|s| s.episode == self.episode && s.episode_auto),
            state,
        })
    }
}

/// Refuses a rule whose regular expression does not compile.
fn require_valid_regex(input: RuleInput) -> Result<RuleInput, ApiError> {
    match regex_problem_of(&input) {
        Some(problem) => Err(ApiError::invalid(format!(
            "{} ({})",
            problem.message, problem.detail
        ))),
        None => Ok(input),
    }
}

/// Whether the rule's regular expression fails to compile, decided by the same
/// compilation the worker's evaluation uses ([`trss_collect::rss::regex_error`]).
fn regex_problem_of(input: &RuleInput) -> Option<RegexProblem> {
    if !input.regex {
        return None;
    }
    let pattern = input.r#match.as_deref()?;
    regex_error(pattern, input.case_insensitive).map(|err| regex_problem(&err))
}

// ---------------------------------------------------------------------------
// Handlers: save, delete, reorder
// ---------------------------------------------------------------------------

async fn create_rule(
    State(state): State<AppState>,
    parsed: Result<Json<CreateBody>, JsonRejection>,
) -> Result<(StatusCode, Json<RuleView>), ApiError> {
    let b = body(parsed)?;
    let mut input = require_valid_regex(b.fields.into_input(None)?)?;
    check_directory(&state, None, &input.directory).await?;
    // A rule that would collect into a work folder the archive folder holds is
    // made paused, and turned on by the worker once the folder is moved.
    let waits =
        input.state == RuleState::Active && waits_for_work_folder(&state, &input.directory).await?;
    if waits {
        input.state = RuleState::Paused;
    }
    let created = state
        .channels
        .create_rule(&b.channel_id, input)
        .await
        .map_err(store_error)?;
    if waits {
        start_new_rule(&state, &created.id, Vec::new()).await;
    }
    Ok((
        StatusCode::CREATED,
        Json(rule_view(&state, &created.id).await?),
    ))
}

async fn read_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RuleView>, ApiError> {
    Ok(Json(rule_view(&state, &id).await?))
}

async fn update_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
    parsed: Result<Json<UpdateBody>, JsonRejection>,
) -> Result<Json<RuleView>, ApiError> {
    let b = body(parsed)?;
    let stored = state
        .channels
        .get_rule(&id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ChannelError::NotFound {
            kind: "rule",
            id: id.clone(),
        })?;
    if stored.version != b.version {
        return Err(rule_conflict(&state, &id).await);
    }
    let mut input = require_valid_regex(b.fields.into_input(Some(&stored))?)?;
    if input.state != stored.state {
        return Err(ApiError::invalid(STATE_NOT_EDITED));
    }
    if stored.subscription.is_some() {
        check_work_folder(&input.directory)?;
    }
    check_directory(&state, Some(&stored), &input.directory).await?;
    // A collecting rule moved into a work folder the archive folder holds
    // would make that work folder in the collect folder on its next cycle:
    // it is saved paused, and the worker turns it on once the folder came over.
    let waits = stored.state == RuleState::Active
        && changes_work_folder(&state, &stored.directory, &input.directory).await?
        && waits_for_work_folder(&state, &input.directory).await?;
    if waits {
        input.state = RuleState::Paused;
    }
    match state
        .channels
        .update_rule_at(&id, b.version, &b.channel_id, input, state.anissia.now())
        .await
    {
        Ok(_) => {
            if waits {
                // The rule had been collecting, so it is not noted as resumed.
                start_new_rule(&state, &id, Vec::new()).await;
            }
            Ok(Json(rule_view(&state, &id).await?))
        }
        Err(e) if e.is_conflict() => Err(rule_conflict(&state, &id).await),
        Err(e) => Err(store_error(e)),
    }
}

async fn delete_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
    parsed: Result<Query<DeleteQuery>, QueryRejection>,
) -> Result<Json<Removed>, ApiError> {
    let Query(q) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;
    match state.channels.delete_rule(&id, q.version).await {
        Ok(()) => Ok(Json(Removed { removed: true })),
        Err(e) if e.is_conflict() => Err(rule_conflict(&state, &id).await),
        Err(e) => Err(store_error(e)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SwitchBody {
    /// The version the client saw.
    version: i64,
    /// `영상 받기`.
    #[serde(default)]
    video: Option<bool>,
    /// `자막 받기`.
    #[serde(default)]
    subtitles: Option<bool>,
}

async fn switch_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
    parsed: Result<Json<SwitchBody>, JsonRejection>,
) -> Result<Json<RuleView>, ApiError> {
    let b = body(parsed)?;
    let stored = state
        .channels
        .get_rule(&id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ChannelError::NotFound {
            kind: "rule",
            id: id.clone(),
        })?;
    if stored.version != b.version {
        return Err(rule_conflict(&state, &id).await);
    }
    if stored.state == RuleState::Archived {
        return Err(ApiError::invalid(
            "보관된 규칙은 스위치를 바꿀 수 없어요. 복원한 뒤 바꿔 주세요.",
        ));
    }
    // Exactly one switch is sent, whatever the rule is doing.
    if b.video.is_some() == b.subtitles.is_some() {
        return Err(ApiError::invalid(BAD_BODY));
    }
    if stored.state == RuleState::Paused && b.video.is_some() {
        // The work folder may have to come out of the archive folder before the
        // rule collects: the worker turns it on after the move (see the module
        // docs).
        const MOVING: &str =
            "작품 폴더를 옮기는 중이에요. 옮기기가 끝나면 켜져요. 끝난 뒤에 다시 바꿔 주세요.";
        let last = archive_moves(&state, std::slice::from_ref(&stored))
            .await?
            .remove(&id);
        let last_direction = last
            .as_ref()
            .and_then(|c| serde_json::from_str::<RuleArchive>(&c.payload).ok())
            .map(|p| p.direction);
        let open = last.as_ref().is_some_and(|c| c.state.is_open());
        if open && b.video == Some(false) {
            // Off now would change nothing: the worker turns the rule on after
            // the move, whatever the screen was told.
            if matches!(
                last_direction,
                Some(rule_archive::Direction::Start | rule_archive::Direction::Resume)
            ) {
                return Err(ApiError::invalid(MOVING));
            }
        }
        if open && b.video == Some(true) {
            return Err(ApiError::invalid(MOVING));
        }
        if b.video == Some(true) {
            // A `start` that failed left a rule that never collected: its retry
            // is a start again, so the time it waited does not make what came
            // meanwhile past (`resume` notes the time of the press).
            let never_collected = last
                .as_ref()
                .is_some_and(|c| c.state == CommandState::Failed)
                && last_direction == Some(rule_archive::Direction::Start);
            if never_collected || waits_for_work_folder(&state, &stored.directory).await? {
                let direction = if never_collected {
                    rule_archive::Direction::Start
                } else {
                    rule_archive::Direction::Resume
                };
                if !ask_start(&state, &id, direction, Vec::new()).await? {
                    return Err(ApiError::invalid(MOVING));
                }
                return Ok(Json(rule_view(&state, &id).await?));
            }
        }
    }
    let written = match (b.video, b.subtitles) {
        (Some(on), None) => {
            state
                .channels
                .set_video_receiving(&id, b.version, on, state.anissia.now())
                .await
        }
        (None, Some(on)) => {
            if stored.subscription.is_none() {
                return Err(ApiError::invalid(
                    "편성표와 연결된 규칙만 자막을 받아요. 먼저 편성표와 연결해 주세요.",
                ));
            }
            if stored.state != RuleState::Active {
                return Err(ApiError::invalid("영상 받기를 켜야 자막을 받을 수 있어요."));
            }
            state
                .channels
                .set_subtitle_receiving(&id, b.version, on)
                .await
        }
        // Refused above.
        _ => return Err(ApiError::invalid(BAD_BODY)),
    };
    match written {
        Ok(_) => {
            let view = rule_view(&state, &id).await?;
            // A subscription that takes part again receives its creator's
            // episodes it missed meanwhile (`trss_jobs::follow`).
            if b.video == Some(true) || b.subtitles == Some(true) {
                super::jobs_api::follow_now(&state).await;
            }
            Ok(Json(view))
        }
        Err(e) if e.is_conflict() => Err(rule_conflict(&state, &id).await),
        Err(e) => Err(store_error(e)),
    }
}

#[derive(Deserialize)]
struct ArchivedWorkQuery {
    /// The save folder as typed; the server finds its work folder.
    directory: String,
    /// The folder a stored rule saves to now, for the form that edits it: a
    /// rule that stays in its work folder moves nothing.
    #[serde(default)]
    from: Option<String>,
}

/// A work of the archive folder that a rule saving to the typed folder would
/// bring into the collect folder.
#[derive(Debug, Serialize)]
pub struct ArchivedWorkView {
    /// The work folder's name.
    pub work: String,
    pub archive_folder: String,
    pub collect_folder: String,
    /// The collect folder holds a work of that name too; the archive's merges
    /// into it.
    pub merges: bool,
}

#[derive(Serialize)]
struct ArchivedWorkAnswer {
    archived: Option<ArchivedWorkView>,
}

async fn archived_work(
    State(state): State<AppState>,
    parsed: Result<Query<ArchivedWorkQuery>, QueryRejection>,
) -> Result<Json<ArchivedWorkAnswer>, ApiError> {
    let Query(q) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;
    if let Some(from) = &q.from {
        if !changes_work_folder(&state, from.trim(), q.directory.trim()).await? {
            return Ok(Json(ArchivedWorkAnswer { archived: None }));
        }
    }
    let found = rule_archive::archived_work(
        collection_folders(&state).await?,
        &state.library,
        q.directory.trim(),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(ArchivedWorkAnswer {
        archived: found.map(|work| ArchivedWorkView {
            work: work.work,
            archive_folder: work.archive_folder,
            collect_folder: work.collect_folder,
            merges: work.merges,
        }),
    }))
}

async fn reorder_rules(
    State(state): State<AppState>,
    parsed: Result<Json<OrderBody>, JsonRejection>,
) -> Result<Json<RuleGroup>, ApiError> {
    let b = body(parsed)?;
    let order = b
        .order
        .into_iter()
        .map(|e| OrderItem {
            id: e.id,
            version: e.version,
        })
        .collect();
    match state.channels.reorder_rules(&b.channel_id, order).await {
        Ok(_) => Ok(Json(RuleGroup {
            rules: channel_views(&state, &b.channel_id).await?,
        })),
        Err(e) if e.is_conflict() => match channel_views(&state, &b.channel_id).await {
            Ok(current) => Err(ApiError::conflict_with(RuleGroup { rules: current })),
            Err(e) => Err(e),
        },
        Err(e) => Err(store_error(e)),
    }
}

// ---------------------------------------------------------------------------
// The preview
// ---------------------------------------------------------------------------

/// How an item relates to the edited rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The edited rule is the first to match: it would take the item.
    Mine,
    /// The edited rule matches, but an earlier rule takes the item.
    Earlier,
    /// The edited rule matches, but a channel exclude keeps the item out.
    Excluded,
    /// The edited rule would take the item, but history recorded it before the
    /// rule became a subscription or while it was paused or archived, so the
    /// cycle leaves it alone until the user receives it
    /// ([`ChannelPlan::is_past`]).
    Past,
}

#[derive(Debug, Serialize)]
pub struct TakenBy {
    /// The rule that takes the item; `null` for the edited rule itself.
    pub rule_id: Option<String>,
    pub r#match: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PreviewItem {
    pub id: i64,
    /// The title as history stores it.
    pub title: String,
    pub first_seen_at: i64,
    /// The stored title contains the mask that replaces a long secret value,
    /// so the worker, which saw the original, may judge it differently.
    pub masked: bool,
    pub kind: Kind,
    /// Where the item would be saved by the rule that takes it; `null` when it
    /// is excluded.
    pub save_path: Option<String>,
    /// The rule that takes the item (only for [`Kind::Earlier`]).
    pub taken_by: Option<TakenBy>,
    /// The channel exclude that keeps the item out (only for [`Kind::Excluded`]).
    pub excluded_by: Option<String>,
    /// Why the item is past: `subscribed` (it came before the subscription),
    /// `titled` (it came before the subscription, which waited for its title,
    /// was given one), `resumed` (it came while the rule was paused or
    /// archived) or `first_read` (the feed already held it when the channel
    /// was first read). Only for [`Kind::Past`].
    pub past_cause: Option<&'static str>,
    /// What history recorded for the item so far, as its stable code.
    pub stored_result: &'static str,
    /// The whole episode the release names (`24` of `- 24`), for an item the
    /// edited rule takes ([`Kind::Mine`], [`Kind::Past`]); `null` otherwise or
    /// when the title names none. A screen receiving several sends the lowest
    /// first, so the rule's first episode offset is decided from it.
    pub release: Option<u32>,
    /// The episode the item is received as with the edited rule's offset and
    /// folder (`S02E24`, [`trss_collect::episode_offset::received_as`]); set with `release`.
    pub episode_name: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct PreviewCounts {
    /// Recorded items of the channel that were judged.
    pub total: usize,
    pub mine: usize,
    pub earlier: usize,
    pub excluded: usize,
    /// Items of a subscription rule that were recorded before the subscription
    /// and wait for the user to receive them.
    pub past: usize,
    /// Items the edited rule does not match (taken by other rules or by none).
    pub unmatched: usize,
}

#[derive(Debug, Serialize)]
pub struct Preview {
    /// Set when the edited rule's regular expression does not compile; the rule
    /// then matches nothing and `items` is empty.
    pub error: Option<RegexProblem>,
    pub counts: PreviewCounts,
    /// How many of the judged items have a masked title.
    pub masked_total: usize,
    /// The items the edited rule matches, newest first, up to a limit.
    pub items: Vec<PreviewItem>,
    /// There are more matching items than `items` lists.
    pub truncated: bool,
    /// What the app offers a new subscription's offset before anything is
    /// received (ticket 0126): only for a preview asked for a subscription
    /// (`subscribing`), and only with a value. The rule detail carries its own
    /// in the rule's view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub episode_suggestion: Option<episode::EpisodeSuggestion>,
    /// The earliest release among the items the edited rule may be asked to
    /// receive (taken by it and not received by any rule), listed or not.
    #[serde(skip)]
    pub earliest_receivable: Option<u32>,
}

/// Judges `items` with the edited rule put into the channel's rules, by
/// [`preview::preview`], the evaluation the worker's cycle uses, and says it in
/// the wire's shapes.
pub fn build_preview(
    collect_folder: &FsPath,
    cwr: &ChannelWithRules,
    edited_id: Option<&str>,
    edited: &RuleInput,
    position: Option<usize>,
    items: &[HistoryItem],
) -> Result<Preview, ApiError> {
    preview::preview(
        collect_folder,
        cwr,
        preview::Edit {
            id: edited_id,
            input: edited,
            position,
        },
        items,
        PREVIEW_LIST_LIMIT,
    )
    .map(Preview::from)
    .map_err(|preview::RuleNotFound(id)| {
        ApiError::from(ChannelError::NotFound { kind: "rule", id })
    })
}

impl From<preview::Kind> for Kind {
    fn from(kind: preview::Kind) -> Self {
        match kind {
            preview::Kind::Mine => Kind::Mine,
            preview::Kind::Earlier => Kind::Earlier,
            preview::Kind::Excluded => Kind::Excluded,
            preview::Kind::Past => Kind::Past,
        }
    }
}

impl From<preview::PreviewItem> for PreviewItem {
    fn from(item: preview::PreviewItem) -> Self {
        PreviewItem {
            id: item.id,
            title: item.title,
            first_seen_at: item.first_seen_at,
            masked: item.masked,
            kind: item.kind.into(),
            save_path: item.save_path.map(|path| path.display().to_string()),
            taken_by: item.taken_by.map(|by| TakenBy {
                rule_id: Some(by.rule_id),
                r#match: by.r#match,
            }),
            excluded_by: item.excluded_by,
            past_cause: item.past_cause.map(PastCause::code),
            stored_result: item.stored_result.code(),
            release: item.release,
            episode_name: item.episode_name,
        }
    }
}

impl From<preview::PreviewCounts> for PreviewCounts {
    fn from(counts: preview::PreviewCounts) -> Self {
        PreviewCounts {
            total: counts.total,
            mine: counts.mine,
            earlier: counts.earlier,
            excluded: counts.excluded,
            past: counts.past,
            unmatched: counts.unmatched,
        }
    }
}

impl From<preview::Preview> for Preview {
    fn from(preview: preview::Preview) -> Self {
        Preview {
            error: preview.error.as_ref().map(regex_problem),
            counts: preview.counts.into(),
            masked_total: preview.masked_total,
            items: preview.items.into_iter().map(PreviewItem::from).collect(),
            truncated: preview.truncated,
            episode_suggestion: None,
            earliest_receivable: preview.earliest_receivable,
        }
    }
}

async fn preview(
    State(state): State<AppState>,
    parsed: Result<Json<PreviewBody>, JsonRejection>,
) -> Result<Json<Preview>, ApiError> {
    let b = body(parsed)?;
    // The fields are shaped the way a save shapes them, but a regular
    // expression that does not compile is a result to show, not a refusal.
    let edited = b.rule.into_input(None)?;
    let cwr = load_channel(&state, &b.channel_id).await?;
    let items = channel_items(&state.history, &b.channel_id).await?;
    // Unset, the folder is empty and a save path is the rule's directory alone.
    let collect_folder = collect_folder(&state).await?.unwrap_or_default();
    let mut preview = build_preview(
        FsPath::new(&collect_folder),
        &cwr,
        b.rule_id.as_deref(),
        &edited,
        b.position,
        &items,
    )?;
    if b.subscribing && b.rule_id.is_none() {
        if let Some(first) = preview.earliest_receivable {
            preview.episode_suggestion =
                episode::for_new_subscription(&state, &b.channel_id, &edited, first).await;
        }
    }
    Ok(Json(preview))
}
