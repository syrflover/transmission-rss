//! `/api/commands`: actions the web accepts and the worker carries out.
//!
//! | call                      | success                                                    |
//! | ------------------------- | ---------------------------------------------------------- |
//! | `POST /commands`          | `202 CommandView` when stored now, `200 CommandView` when the same command was accepted before |
//! | `GET /commands/{id}`      | `200 CommandView`, or `404` when no command has that ID    |
//!
//! Request: `{ "id": <command ID>, "kind": <kind>, "payload": <the kind's payload> }`:
//!
//! | kind           | on screen    | payload                                                        |
//! | -------------- | ------------ | -------------------------------------------------------------- |
//! | `receive_once` | `다시 받기`·`받기` | `{ "item_id": <history item>, "rule_id": <rule> }` (`rule_id` only to receive an item no rule has picked) |
//! | `rule_archive` | `보관`·`복원`·`영상 받기` | `{ "rule_id": <rule>, "direction": "archive" \| "restore" }` (`"start"` and `"resume"` are the web's own, refused here) |
//! | `receive_past` | `받기`       | `{ "rule_id": <rule>, "search_id": <past episode search>, "key": <result's key> }` |
//! | `watch_rescan` | `다시 확인`  | `{ "folder_id": <watch folder> }`                              |
//! | `episode_undo` | `되돌리기`   | `{ "rule_id": <rule>, "episode": <the automatic offset seen> }` |
//! | `anissia_captions` | `새로고침` | `{ "anime_no": <Anissia anime a season is linked to> }`    |
//!
//! `receive_once` of a video revision is refused with `400` and the reason
//! when the episode's place is known to hold the same or a higher revision
//! already, the same case the screens show instead of the button
//! ([`super::in_place`]); the worker still looks at the folder when it runs
//! the command.
//!
//! `receive_past` adds one result of a finished past episode search
//! ([`super::past_search_api`]). The web resolves the result from the search it
//! keeps and stores its title and link as history would; the browser supplies
//! neither. A repeat of the request is the same rule and result, whatever search
//! it names, so a lost answer can be asked again after the search is gone.
//! A result history says Transmission took is accepted only when the search
//! found it gone from the work (its torrent removed and its video not in the
//! folder), and then as a result nothing has received; the worker checks that
//! again when it runs the command.
//!
//! `episode_undo` is accepted while the rule's offset is the automatic one the
//! request names and the value it replaced is known; the worker puts that
//! value back and renames the videos ([`episode_undo`]). On a rule that is
//! not automatic it is accepted too for an undo of that value that ended with
//! files still to rename, which it carries on.
//!
//! `anissia_captions` reads one Anissia anime's subtitle lines now and observes
//! them ([`trss_collect::commands::anissia_captions`]). It is accepted only for
//! an anime some season is linked to. The web makes the same command itself,
//! with an ID of its own, when a season is linked
//! ([`super::seasons_anissia_api`]); its state and outcome are the candidates'
//! `refresh`.
//!
//! A rule is archived and restored only through `rule_archive`: the worker
//! turns the rule off before its folder moves and on after it moved back.
//! `start` (a new rule, subscription or edited save folder) and `resume`
//! (`영상 받기` on) turn a paused rule on after its work folder came out of the
//! archive folder into the collect folder. Only the web makes these, itself,
//! when it saves a rule that starts collecting for a work in the archive folder
//! ([`super::rules_api`]); a browser's `POST` of one is refused (`400`).
//! The browser makes one ID per user action and sends it with the content.
//!
//! **Accepted is not done.** The answer to a `POST` says the command is stored
//! (`state: "pending"`); the worker runs it, and the screen reads the
//! outcome from `GET /commands/{id}` (or from the item the command is about).
//! A stored command wakes the worker ([`trss_core::wake`]), which starts it at
//! once rather than at its next look a few seconds later.
//! Nothing here reports a command as succeeded before the worker has written
//! its outcome.
//!
//! **Repeats.** The same ID with the same kind and payload returns the stored
//! command, in whatever state it has reached, and queues nothing new. The same
//! ID with a different kind or payload is refused with `409` and the stored
//! command as `current`. So is a second command for a subject that already has
//! an open one (`current` is the open one).
//!
//! **A lost answer.** The screen asks `GET /commands/{id}` with the same ID.
//! A `404` means the request never reached the store; sending it again with the
//! *same* ID is safe. It never needs a new ID.
//!
//! Validation that depends on the request's kind (which item, which rule)
//! happens before a command is stored, so a refused request stores nothing.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{in_place::Evidence, ApiError, AppState};
use trss_collect::{
    commands::{anissia_captions, episode_undo, receive_once, receive_past, rule_archive},
    past_search::service::Resolve,
    store::channels::RuleState,
};
use trss_core::commands::{Accepted, Command, CommandState, NewCommand};
use trss_library::watch_rescan;

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/commands", post(create_command))
        .route("/commands/{id}", get(read_command))
}

// ---------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------

/// A command as the screen sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommandView {
    pub id: String,
    pub kind: String,
    /// `pending`, `running`, `done` or `failed`.
    pub state: CommandState,
    pub created_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
    /// Set once the command has ended.
    pub outcome: Option<OutcomeView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutcomeView {
    /// A code of the command's kind; for `receive_once`, a history result.
    pub result: String,
    /// A sentence, when there is something to explain.
    pub reason: Option<String>,
}

impl From<&Command> for CommandView {
    fn from(command: &Command) -> Self {
        CommandView {
            id: command.id.clone(),
            kind: command.kind.clone(),
            state: command.state,
            created_at: command.created_at,
            updated_at: command.updated_at,
            finished_at: command.finished_at,
            outcome: command.outcome.as_ref().map(|outcome| OutcomeView {
                result: outcome.result.clone(),
                reason: outcome.reason.clone(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    id: String,
    kind: String,
    payload: Value,
}

const BAD_BODY: &str = "요청 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";
const MISMATCH: &str =
    "이 명령 ID는 다른 내용으로 이미 접수됐어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";
const FOLDER_REFUSED: &str = "`다시 받기`는 그 항목을 고른 규칙의 저장 폴더에 받아서 폴더를 고를 수 없어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";
const BUSY: &str = "이 항목은 이미 추가하는 중이에요. 그 결과가 나올 때까지 기다려 주세요.";
const RULE_BUSY: &str =
    "이 규칙은 이미 보관하거나 복원하거나 작품 폴더를 옮기는 중이에요. 그 결과가 나올 때까지 기다려 주세요.";
const FOLDER_BUSY: &str =
    "이 폴더는 이미 다시 확인하는 중이에요. 그 결과가 나올 때까지 기다려 주세요.";
const CAPTIONS_BUSY: &str =
    "이 작품의 자막 정보는 이미 읽는 중이에요. 그 결과가 나올 때까지 기다려 주세요.";
const UNDO_BUSY: &str =
    "이 규칙의 회차 변환은 이미 되돌리는 중이에요. 그 결과가 나올 때까지 기다려 주세요.";

/// The shortest and longest command ID.
const ID_LEN: std::ops::RangeInclusive<usize> = 8..=64;

fn valid_id(id: &str) -> bool {
    ID_LEN.contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub(super) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// A request read into a kind's own payload, before it is checked against the
/// stored data.
enum Request {
    ReceiveOnce(receive_once::ReceiveOnce),
    ReceivePast(PastRequest),
    RuleArchive(rule_archive::RuleArchive),
    WatchRescan(watch_rescan::WatchRescan),
    EpisodeUndo(episode_undo::EpisodeUndo),
    AnissiaCaptions(anissia_captions::AnissiaCaptions),
}

/// What the browser sends to receive a result of a past episode search.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PastRequest {
    rule_id: String,
    search_id: String,
    key: String,
}

impl Request {
    fn read(kind: &str, payload: Value) -> Result<Request, ApiError> {
        match kind {
            receive_past::KIND => {
                let payload: PastRequest =
                    serde_json::from_value(payload).map_err(|_| ApiError::invalid(BAD_BODY))?;
                Ok(Request::ReceivePast(payload))
            }
            receive_once::KIND => {
                let payload: receive_once::ReceiveOnce =
                    serde_json::from_value(payload).map_err(|_| ApiError::invalid(BAD_BODY))?;
                Ok(Request::ReceiveOnce(payload))
            }
            rule_archive::KIND => {
                let payload: rule_archive::RuleArchive =
                    serde_json::from_value(payload).map_err(|_| ApiError::invalid(BAD_BODY))?;
                Ok(Request::RuleArchive(payload))
            }
            watch_rescan::KIND => {
                let payload: watch_rescan::WatchRescan =
                    serde_json::from_value(payload).map_err(|_| ApiError::invalid(BAD_BODY))?;
                Ok(Request::WatchRescan(payload))
            }
            episode_undo::KIND => {
                let payload: episode_undo::EpisodeUndo =
                    serde_json::from_value(payload).map_err(|_| ApiError::invalid(BAD_BODY))?;
                Ok(Request::EpisodeUndo(payload))
            }
            anissia_captions::KIND => {
                let payload: anissia_captions::AnissiaCaptions =
                    serde_json::from_value(payload).map_err(|_| ApiError::invalid(BAD_BODY))?;
                Ok(Request::AnissiaCaptions(payload))
            }
            _ => Err(ApiError::invalid("모르는 종류의 명령이에요.")),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Request::ReceiveOnce(_) => receive_once::KIND,
            Request::ReceivePast(_) => receive_past::KIND,
            Request::RuleArchive(_) => rule_archive::KIND,
            Request::WatchRescan(_) => watch_rescan::KIND,
            Request::EpisodeUndo(_) => episode_undo::KIND,
            Request::AnissiaCaptions(_) => anissia_captions::KIND,
        }
    }

    /// What a second open command for the same subject is told.
    fn busy(&self) -> &'static str {
        match self {
            Request::ReceiveOnce(_) | Request::ReceivePast(_) => BUSY,
            Request::RuleArchive(_) => RULE_BUSY,
            Request::WatchRescan(_) => FOLDER_BUSY,
            Request::EpisodeUndo(_) => UNDO_BUSY,
            Request::AnissiaCaptions(_) => CAPTIONS_BUSY,
        }
    }

    /// Whether `stored`, accepted earlier, is this same request. Payloads are
    /// compared in canonical form, so a command stored before the payload
    /// dropped its `folder` is still the request an old tab sends again.
    fn is_repeat_of(&self, stored: &Command) -> bool {
        if stored.kind != self.kind() {
            return false;
        }
        match self {
            Request::ReceiveOnce(payload) => {
                serde_json::from_str::<receive_once::ReceiveOnce>(&stored.payload)
                    .is_ok_and(|stored| stored.canonical() == payload.canonical())
            }
            Request::ReceivePast(payload) => serde_json::from_str::<receive_past::ReceivePast>(
                &stored.payload,
            )
            .is_ok_and(|stored| stored.rule_id == payload.rule_id && stored.key == payload.key),
            Request::RuleArchive(payload) => {
                serde_json::from_str::<rule_archive::RuleArchive>(&stored.payload)
                    .is_ok_and(|stored| stored == *payload)
            }
            Request::WatchRescan(payload) => {
                serde_json::from_str::<watch_rescan::WatchRescan>(&stored.payload)
                    .is_ok_and(|stored| stored == *payload)
            }
            Request::EpisodeUndo(payload) => {
                serde_json::from_str::<episode_undo::EpisodeUndo>(&stored.payload)
                    .is_ok_and(|stored| stored == *payload)
            }
            Request::AnissiaCaptions(payload) => {
                serde_json::from_str::<anissia_captions::AnissiaCaptions>(&stored.payload)
                    .is_ok_and(|stored| stored == *payload)
            }
        }
    }

    /// Refuses what a new command may not carry. A request already accepted
    /// is answered before this is asked.
    fn refuse_if_not_new(&self) -> Result<(), ApiError> {
        match self {
            // Only commands accepted before a rule decided the folder have one.
            Request::ReceiveOnce(payload) if payload.names_a_folder() => {
                Err(ApiError::invalid(FOLDER_REFUSED))
            }
            Request::ReceiveOnce(_)
            | Request::ReceivePast(_)
            | Request::RuleArchive(_)
            | Request::WatchRescan(_)
            | Request::EpisodeUndo(_)
            | Request::AnissiaCaptions(_) => Ok(()),
        }
    }

    /// `past` is what [`Request::check`] resolved a `receive_past` request to.
    fn new_command(&self, id: String, past: Option<receive_past::ReceivePast>) -> NewCommand {
        match self {
            Request::ReceiveOnce(payload) => NewCommand {
                id,
                kind: receive_once::KIND.to_owned(),
                payload: payload.canonical(),
                subject: Some(payload.subject()),
            },
            Request::ReceivePast(_) => {
                let payload = past.expect("a checked receive_past request is resolved");
                NewCommand {
                    id,
                    kind: receive_past::KIND.to_owned(),
                    payload: payload.canonical(),
                    subject: Some(payload.subject()),
                }
            }
            Request::RuleArchive(payload) => NewCommand {
                id,
                kind: rule_archive::KIND.to_owned(),
                payload: payload.canonical(),
                subject: Some(payload.subject()),
            },
            Request::WatchRescan(payload) => NewCommand {
                id,
                kind: watch_rescan::KIND.to_owned(),
                payload: payload.canonical(),
                subject: Some(payload.subject()),
            },
            Request::EpisodeUndo(payload) => NewCommand {
                id,
                kind: episode_undo::KIND.to_owned(),
                payload: payload.canonical(),
                subject: Some(payload.subject()),
            },
            Request::AnissiaCaptions(payload) => NewCommand {
                id,
                kind: anissia_captions::KIND.to_owned(),
                payload: payload.canonical(),
                subject: Some(payload.subject()),
            },
        }
    }

    /// Checks the request against the stored data; a refusal stores nothing.
    /// A `receive_past` request comes back as the payload to store.
    async fn check(&self, state: &AppState) -> Result<Option<receive_past::ReceivePast>, ApiError> {
        match self {
            Request::ReceiveOnce(payload) => {
                check_receive_once(payload, state).await.map(|()| None)
            }
            Request::ReceivePast(payload) => check_receive_past(payload, state).await.map(Some),
            Request::RuleArchive(payload) => {
                check_rule_archive(payload, state).await.map(|()| None)
            }
            Request::WatchRescan(payload) => {
                check_watch_rescan(payload, state).await.map(|()| None)
            }
            Request::EpisodeUndo(payload) => {
                check_episode_undo(payload, state).await.map(|()| None)
            }
            Request::AnissiaCaptions(payload) => {
                check_anissia_captions(payload, state).await.map(|()| None)
            }
        }
    }
}

/// Only an anime some season is linked to is read: the command is not a way to
/// ask Anissia about any anime.
async fn check_anissia_captions(
    payload: &anissia_captions::AnissiaCaptions,
    state: &AppState,
) -> Result<(), ApiError> {
    let linked = payload.anime_no > 0
        && state
            .seasons
            .store
            .anime_is_linked(payload.anime_no)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    if linked {
        Ok(())
    } else {
        Err(ApiError::invalid(
            "이 작품에 연결한 시즌이 없어요. 화면을 새로고침해 주세요.",
        ))
    }
}

/// Makes the worker read Anissia anime `anime_no`'s subtitle lines now, as a
/// command the web makes itself ([`anissia_captions::ask`]). A failure to store
/// it is logged and left: the reading of the recent list every 30 minutes and
/// the user's `새로고침` still observe the anime.
pub(super) async fn ask_anissia_captions(state: &AppState, anime_no: i64) {
    match anissia_captions::ask(&state.commands, anime_no, now_millis()).await {
        Ok(true) => {
            state.wake_worker();
        }
        Ok(false) => {}
        Err(e) => eprintln!("trss-web: cannot ask for the subtitle lines of anime {anime_no}: {e}"),
    }
}

/// The result must be one of the rule's finished search, and the rule must be
/// one that receives it.
async fn check_receive_past(
    request: &PastRequest,
    state: &AppState,
) -> Result<receive_past::ReceivePast, ApiError> {
    let stored = state
        .past_search
        .resolve(&request.search_id, &request.rule_id, &request.key)
        .map_err(|why| match why {
            Resolve::Gone => ApiError::not_found(
                "이 검색은 끝났거나 서버가 다시 시작돼서 결과가 없어요. 다시 검색해 주세요.",
            ),
            Resolve::OtherRule => ApiError::invalid("이 검색은 다른 규칙의 검색이에요."),
            Resolve::Running => ApiError::invalid("검색이 아직 끝나지 않았어요."),
            Resolve::NoItem => ApiError::invalid("이 검색 결과에 없는 항목이에요."),
        })?;
    let payload = receive_past::ReceivePast {
        rule_id: request.rule_id.clone(),
        key: request.key.clone(),
        title: stored.title.clone(),
        link: stored.link.clone(),
    };
    let rule = state
        .channels
        .get_rule(&payload.rule_id)
        .await?
        .ok_or_else(|| ApiError::not_found("규칙을 찾지 못했어요. 삭제됐을 수 있어요."))?;
    let channel = state.channels.get_channel(&rule.channel_id).await?;
    let existing = match &channel {
        Some(channel) => state
            .history
            .item_by_key(channel.id.clone(), payload.key.clone())
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?,
        None => None,
    };
    let probe = existing.unwrap_or_else(|| {
        receive_past::past_item(&payload, channel.as_ref().map_or("", |c| c.id.as_str()), 0)
    });
    // An item Transmission took that the search found gone from the work is
    // judged as one nothing has received, as the worker does: the rule must be
    // active and pick it. The worker looks at Transmission and the folder
    // again when it runs the command.
    let probe = if stored.departed && probe.result.is_settled() {
        trss_collect::store::history::HistoryItem {
            result: trss_collect::store::history::HistoryResult::NoMatch,
            ..probe
        }
    } else {
        probe
    };
    receive_once::adoption_plan(&probe, channel.as_ref(), Some(&rule))
        .map_err(|why| ApiError::invalid(why.message()))?;
    Ok(payload)
}

/// The rule's offset must be the automatic one the request names, with the
/// value it replaced known: the worker decides it the same way
/// ([`episode_undo::standing`]).
async fn check_episode_undo(
    payload: &episode_undo::EpisodeUndo,
    state: &AppState,
) -> Result<(), ApiError> {
    match episode_undo::standing(&state.channels, payload).await? {
        episode_undo::Standing::RuleGone => Err(ApiError::not_found(
            "규칙을 찾지 못했어요. 삭제됐을 수 있어요.",
        )),
        episode_undo::Standing::New { .. } | episode_undo::Standing::CarryOn(_) => Ok(()),
        episode_undo::Standing::Changed | episode_undo::Standing::PreviousUnknown => Err(
            ApiError::invalid("되돌릴 자동 회차 변환이 없어요. 화면을 새로고침해 주세요."),
        ),
    }
}

/// The watch folder must be registered.
async fn check_watch_rescan(
    payload: &watch_rescan::WatchRescan,
    state: &AppState,
) -> Result<(), ApiError> {
    state
        .library
        .folder(&payload.folder_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::not_found("감시 폴더를 찾지 못했어요. 등록이 해제됐을 수 있어요.")
        })?;
    Ok(())
}

/// The rule must exist and only an archived rule is restored. Archiving an
/// archived rule is `다시 옮기기`: the folder is moved again if it can be. A
/// `start` or `resume` is refused: the web makes them itself, after it saved
/// the rule paused for the move, so a posted one could turn a rule on before its
/// folder came over.
async fn check_rule_archive(
    payload: &rule_archive::RuleArchive,
    state: &AppState,
) -> Result<(), ApiError> {
    if matches!(
        payload.direction,
        rule_archive::Direction::Start | rule_archive::Direction::Resume
    ) {
        return Err(ApiError::invalid(
            "작품 폴더를 옮기고 규칙을 켜는 일은 규칙을 만들거나 영상 받기를 켤 때 서버가 해요. 화면을 새로고침해 주세요.",
        ));
    }
    let rule = state
        .channels
        .get_rule(&payload.rule_id)
        .await?
        .ok_or_else(|| ApiError::not_found("규칙을 찾지 못했어요. 삭제됐을 수 있어요."))?;
    if payload.direction == rule_archive::Direction::Restore && rule.state != RuleState::Archived {
        return Err(ApiError::invalid(
            "이 규칙은 보관돼 있지 않아요. 화면을 새로고침해 주세요.",
        ));
    }
    Ok(())
}

async fn check_receive_once(
    payload: &receive_once::ReceiveOnce,
    state: &AppState,
) -> Result<(), ApiError> {
    let item = state
        .history
        .get(payload.item_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("기록에서 이 항목을 찾지 못했어요."))?;
    if payload.rule_id.is_some() {
        let channel = state.channels.get_channel(&item.channel_id).await?;
        let rule = match &payload.rule_id {
            Some(id) => state.channels.get_rule(id).await?,
            None => None,
        };
        receive_once::adoption_plan(&item, channel.as_ref(), rule.as_ref())
            .map(|_| ())
            .map_err(|why| ApiError::invalid(why.message()))
    } else {
        // A revision whose episode already holds it or a higher one is
        // refused first, as the screens say it first (`web::in_place`).
        match Evidence::load(state).await?.retry_check(&item).await? {
            Ok(()) => Ok(()),
            Err(blocked) => Err(ApiError::invalid(blocked.message())),
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn conflict(message: &str, stored: &Command) -> ApiError {
    ApiError::Conflict {
        message: message.to_owned(),
        current: serde_json::to_value(CommandView::from(stored)).ok(),
    }
}

async fn create_command(
    State(state): State<AppState>,
    parsed: Result<Json<CreateBody>, JsonRejection>,
) -> Result<(StatusCode, Json<CommandView>), ApiError> {
    // The rejection text can quote the offending value, so it is dropped.
    let Json(body) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;
    if !valid_id(&body.id) {
        return Err(ApiError::invalid(
            "명령 ID가 올바르지 않아요. 화면을 새로고침한 뒤 다시 시도해 주세요.",
        ));
    }
    let request = Request::read(&body.kind, body.payload)?;

    // A command accepted before is answered as it stands, before anything else
    // is asked of the request: the data it was checked against may have
    // changed since, and a tab that predates an upgrade may send it again in
    // its old form.
    let store = |e: trss_core::commands::CommandError| ApiError::Internal(e.to_string());
    match state.commands.get(&body.id).await.map_err(store)? {
        Some(stored) if request.is_repeat_of(&stored) => {
            return Ok((StatusCode::OK, Json(CommandView::from(&stored))));
        }
        Some(stored) => return Err(conflict(MISMATCH, &stored)),
        None => {}
    }

    request.refuse_if_not_new()?;
    let past = request.check(&state).await?;
    let new = request.new_command(body.id, past);

    match state
        .commands
        .accept(new, now_millis())
        .await
        .map_err(store)?
    {
        Accepted::Created(command) => {
            // The worker starts it now rather than at its next look.
            state.wake_worker();
            Ok((StatusCode::ACCEPTED, Json(CommandView::from(&command))))
        }
        Accepted::Existing(command) => Ok((StatusCode::OK, Json(CommandView::from(&command)))),
        Accepted::Mismatch(command) => Err(conflict(MISMATCH, &command)),
        Accepted::Busy(command) => Err(conflict(request.busy(), &command)),
    }
}

async fn read_command(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<CommandView>, ApiError> {
    let command = state
        .commands
        .get(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("이 명령 ID로 접수된 요청을 찾지 못했어요."))?;
    Ok(Json(CommandView::from(&command)))
}
