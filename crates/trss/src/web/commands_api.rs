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
//! | `receive_past` | `받기`       | `{ "rule_id": <rule>, "search_id": <past episode search>, "key": <result's key> }` |
//! | `watch_rescan` | `다시 확인`  | `{ "folder_id": <watch folder> }`                              |
//! | `episode_undo` | `되돌리기`   | `{ "rule_id": <rule>, "episode": <the automatic offset seen> }` |
//! | `anissia_captions` | `새로고침` | `{ "anime_no": <Anissia anime a season is linked to> }`    |
//! | `rule_archive_batch` | `모두 보관`·`N개 보관`·`보관` of an archive suggestion | `{ "rules": [{ "rule_id": <rule>, "name": <the title the screen showed> }] }`, at least one rule, distinct, in the order to archive them (`rule_id` at most 64 bytes, `name` at most 500 characters) |
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
//! `rule_archive_batch` archives the chosen rules one after another on the
//! server, deleting each with the move of its work folder, and stops at the
//! first that does not end well ([`crate::archive_batch`]). Only one is open
//! at a time. A new batch is refused with `400` when a rule it names is still
//! there and is not suggested now ([`super::archive_api`]): the screen's list
//! may be old, and archiving deletes the rule. The screen reads its progress from
//! `GET /archive-batches/{id}` ([`super::archive_batch_api`]).
//!
//! Two kinds are the web's own, and a browser's `POST` of one is refused
//! (`400`). A `rule_archive` command (`start` of a new rule, subscription or
//! edited save folder, `resume` of `영상 받기` on) turns a paused rule on after
//! its work folder came out of the archive folder into the collect folder; the
//! web makes it when it saves a rule that starts collecting for a work in the
//! archive folder ([`super::rules_api`]). A `work_archive` command moves a work
//! folder into the archive folder; the web stores it with the deletion of the
//! rules that save into the folder (`보관` of a rule, [`super::rules_api`], and
//! `보관 폴더로 옮기기` of a work, [`super::library_work_api`]).
//! The browser makes one ID per user action and sends it with the content.
//!
//! **Accepted is not done.** The answer to a `POST` says the command is stored
//! (`state: "pending"`); the worker runs it, and the screen reads the
//! outcome from `GET /commands/{id}` (or from the item the command is about).
//! A stored command rings the commands' loop of the same process
//! ([`crate::wake`]), which starts it at once.
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
use crate::archive_batch;
use trss_collect::{
    commands::{
        anissia_captions, episode_undo, receive_once, receive_past, rule_archive, work_archive,
    },
    past_search::service::Resolve,
};
use trss_core::commands::{Accepted, Command, CommandState, NewCommand};
use trss_library::watch_rescan;

#[cfg(test)]
mod past_tests;
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
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "Command"))]
pub struct CommandView {
    pub id: String,
    /// One of the kinds in the table above ([`CommandKind`]).
    #[cfg_attr(test, ts(as = "CommandKind"))]
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
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "CommandOutcome"))]
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

/// The kinds of command, as `POST /commands` names them. The screens send
/// them through [`CommandRequest`]; the web stores the kind as text, and the
/// tests of [`super::api_types`] keep this list and [`Request::read`] together.
/// `work_archive` is the one kind the web accepts only through its own route
/// (`POST /library/works/{id}/archive`, [`super::library_work_api`]); a screen
/// reads it here like any other.
#[cfg(test)]
#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub(super) enum CommandKind {
    ReceiveOnce,
    ReceivePast,
    RuleArchive,
    WatchRescan,
    EpisodeUndo,
    AnissiaCaptions,
    RuleArchiveBatch,
    WorkArchive,
}

/// A request to `POST /commands` as a screen writes it: the ID, the kind and
/// the kind's own payload.
#[cfg(test)]
#[derive(Serialize, ts_rs::TS)]
pub(super) struct CommandRequest {
    pub id: String,
    #[serde(flatten)]
    pub command: CommandPayload,
}

/// A command's kind with the payload that kind takes.
#[cfg(test)]
#[derive(Serialize, ts_rs::TS)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub(super) enum CommandPayload {
    ReceiveOnce(receive_once::ReceiveOnce),
    ReceivePast(PastRequest),
    WatchRescan(watch_rescan::WatchRescan),
    EpisodeUndo(episode_undo::EpisodeUndo),
    AnissiaCaptions(anissia_captions::AnissiaCaptions),
    RuleArchiveBatch(archive_batch::ArchiveBatch),
}

const BAD_BODY: &str = "요청 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";
const MISMATCH: &str =
    "이 명령 ID는 다른 내용으로 이미 접수됐어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";
const FOLDER_REFUSED: &str = "`다시 받기`는 그 항목을 고른 규칙의 저장 폴더에 받아서 폴더를 고를 수 없어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";
const BUSY: &str = "이 항목은 이미 추가하는 중이에요. 그 결과가 나올 때까지 기다려 주세요.";
const WEB_OWN: &str = "보관과 폴더 이동은 규칙 상세의 `보관`이나 작품 상세의 `보관 폴더로 옮기기`로 해요. 화면을 새로고침한 뒤 다시 시도해 주세요.";
const BATCH_BUSY: &str = "다른 일괄 보관이 진행 중이에요. 그 결과가 나올 때까지 기다려 주세요.";
const FOLDER_BUSY: &str =
    "이 폴더는 이미 다시 확인하는 중이에요. 그 결과가 나올 때까지 기다려 주세요.";
const CAPTIONS_BUSY: &str =
    "이 작품의 자막 정보는 이미 읽는 중이에요. 그 결과가 나올 때까지 기다려 주세요.";
const UNDO_BUSY: &str =
    "이 규칙의 회차 변환은 이미 되돌리는 중이에요. 그 결과가 나올 때까지 기다려 주세요.";

/// The shortest and longest command ID.
const ID_LEN: std::ops::RangeInclusive<usize> = 8..=64;

pub(super) fn valid_id(id: &str) -> bool {
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
pub(super) enum Request {
    ReceiveOnce(receive_once::ReceiveOnce),
    ReceivePast(PastRequest),
    WatchRescan(watch_rescan::WatchRescan),
    EpisodeUndo(episode_undo::EpisodeUndo),
    AnissiaCaptions(anissia_captions::AnissiaCaptions),
    RuleArchiveBatch(archive_batch::ArchiveBatch),
}

/// What the browser sends to receive a result of a past episode search.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(test, derive(Serialize, ts_rs::TS), ts(rename = "ReceivePastRequest"))]
#[serde(deny_unknown_fields)]
pub(super) struct PastRequest {
    pub(super) rule_id: String,
    pub(super) search_id: String,
    pub(super) key: String,
}

impl Request {
    pub(super) fn read(kind: &str, payload: Value) -> Result<Request, ApiError> {
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
            rule_archive::KIND | work_archive::KIND => Err(ApiError::invalid(WEB_OWN)),
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
            archive_batch::KIND => {
                let payload: archive_batch::ArchiveBatch =
                    serde_json::from_value(payload).map_err(|_| ApiError::invalid(BAD_BODY))?;
                Ok(Request::RuleArchiveBatch(payload))
            }
            _ => Err(ApiError::invalid("모르는 종류의 명령이에요.")),
        }
    }

    pub(super) fn kind(&self) -> &'static str {
        match self {
            Request::ReceiveOnce(_) => receive_once::KIND,
            Request::ReceivePast(_) => receive_past::KIND,
            Request::WatchRescan(_) => watch_rescan::KIND,
            Request::EpisodeUndo(_) => episode_undo::KIND,
            Request::AnissiaCaptions(_) => anissia_captions::KIND,
            Request::RuleArchiveBatch(_) => archive_batch::KIND,
        }
    }

    /// What a second open command for the same subject is told.
    fn busy(&self) -> &'static str {
        match self {
            Request::ReceiveOnce(_) | Request::ReceivePast(_) => BUSY,
            Request::WatchRescan(_) => FOLDER_BUSY,
            Request::EpisodeUndo(_) => UNDO_BUSY,
            Request::AnissiaCaptions(_) => CAPTIONS_BUSY,
            Request::RuleArchiveBatch(_) => BATCH_BUSY,
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
            Request::RuleArchiveBatch(payload) => {
                serde_json::from_str::<archive_batch::ArchiveBatch>(&stored.payload)
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
            Request::RuleArchiveBatch(payload) => match payload.refusal() {
                Some(why) => Err(ApiError::invalid(why)),
                None => Ok(()),
            },
            Request::ReceiveOnce(_)
            | Request::ReceivePast(_)
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
            Request::RuleArchiveBatch(payload) => NewCommand {
                id,
                kind: archive_batch::KIND.to_owned(),
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
            Request::WatchRescan(payload) => {
                check_watch_rescan(payload, state).await.map(|()| None)
            }
            Request::EpisodeUndo(payload) => {
                check_episode_undo(payload, state).await.map(|()| None)
            }
            Request::AnissiaCaptions(payload) => {
                check_anissia_captions(payload, state).await.map(|()| None)
            }
            Request::RuleArchiveBatch(payload) => {
                check_archive_batch(payload, state).await.map(|()| None)
            }
        }
    }
}

/// Every rule of a new batch that is still there must be suggested now: the
/// screen sends the suggestions it showed, which an item received since may
/// have made old, and archiving deletes the rule for good. A rule that is gone
/// is skipped when its turn comes, as one deleted while the batch runs is.
async fn check_archive_batch(
    payload: &archive_batch::ArchiveBatch,
    state: &AppState,
) -> Result<(), ApiError> {
    let ids = payload.rules.iter().map(|rule| rule.rule_id.as_str());
    let Some(id) = super::archive_api::first_unsuggested(state, ids).await? else {
        return Ok(());
    };
    let name = payload
        .rules
        .iter()
        .find(|rule| rule.rule_id == id)
        .map_or("", |rule| rule.name.as_str());
    Err(ApiError::invalid(format!(
        "보관 제안이 바뀌었어요. ‘{name}’ 규칙은 이제 제안되지 않아요. 바뀐 제안을 확인한 뒤 다시 보관해 주세요."
    )))
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
            state.wakes.commands();
        }
        Ok(false) => {}
        Err(e) => eprintln!("trss: cannot ask for the subtitle lines of anime {anime_no}: {e}"),
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
            // The commands' loop starts it now.
            state.wakes.commands();
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
