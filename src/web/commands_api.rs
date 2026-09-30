//! `/api/commands`: actions the web accepts and the worker carries out.
//!
//! | call                      | success                                                    |
//! | ------------------------- | ---------------------------------------------------------- |
//! | `POST /commands`          | `202 CommandView` when stored now, `200 CommandView` when the same command was accepted before |
//! | `GET /commands/{id}`      | `200 CommandView`, or `404` when no command has that ID    |
//!
//! Request: `{ "id": <command ID>, "kind": "receive_once", "payload": {...} }`.
//! The browser makes one ID per user action and sends it with the content.
//!
//! **Accepted is not done.** The answer to a `POST` says the command is stored
//! (`state: "pending"`); the worker runs it later, and the screen reads the
//! outcome from `GET /commands/{id}` (or from the item the command is about).
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
//! Validation that depends on the request's kind (which item, which folder)
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

use super::{ApiError, AppState};
use crate::{
    store::commands::{Accepted, Command, CommandState, NewCommand},
    worker::commands::{folder, receive_once},
};

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
#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
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
const BUSY: &str = "이 항목은 이미 받는 중이에요. 그 결과가 나올 때까지 기다려 주세요.";

/// The shortest and longest command ID.
const ID_LEN: std::ops::RangeInclusive<usize> = 8..=64;

fn valid_id(id: &str) -> bool {
    ID_LEN.contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// A request read into a kind's own payload, before it is checked against the
/// stored data.
enum Request {
    ReceiveOnce(receive_once::ReceiveOnce),
}

impl Request {
    fn read(kind: &str, payload: Value) -> Result<Request, ApiError> {
        match kind {
            receive_once::KIND => serde_json::from_value(payload)
                .map(Request::ReceiveOnce)
                .map_err(|_| ApiError::invalid(BAD_BODY)),
            _ => Err(ApiError::invalid("모르는 종류의 명령이에요.")),
        }
    }

    fn new_command(&self, id: String) -> NewCommand {
        match self {
            Request::ReceiveOnce(payload) => NewCommand {
                id,
                kind: receive_once::KIND.to_owned(),
                payload: payload.canonical(),
                subject: Some(payload.subject()),
            },
        }
    }

    /// Checks the request against the stored data; a refusal stores nothing.
    async fn check(&self, state: &AppState) -> Result<(), ApiError> {
        match self {
            Request::ReceiveOnce(payload) => check_receive_once(payload, state).await,
        }
    }
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
    if !receive_once::can_receive(item.result) {
        return Err(ApiError::invalid(
            "이 항목은 Transmission에 이미 있어서 다시 받을 필요가 없어요.",
        ));
    }
    let channel = state
        .channels
        .get_channel(&item.channel_id)
        .await?
        .ok_or_else(|| {
            ApiError::invalid("이 항목의 채널이 삭제돼서 저장 폴더를 정할 수 없어요.")
        })?;
    folder::resolve(&channel.base_dir, &payload.folder)
        .map_err(|err| ApiError::invalid(err.message()))?;
    Ok(())
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
    let new = request.new_command(body.id);

    // A command accepted before is answered as it stands, before anything is
    // checked again: the data it was checked against may have changed since.
    let store = |e: crate::store::commands::CommandError| ApiError::Internal(e.to_string());
    match state.commands.get(&new.id).await.map_err(store)? {
        Some(stored) if stored.kind == new.kind && stored.payload == new.payload => {
            return Ok((StatusCode::OK, Json(CommandView::from(&stored))));
        }
        Some(stored) => return Err(conflict(MISMATCH, &stored)),
        None => {}
    }

    request.check(&state).await?;

    match state
        .commands
        .accept(new, now_millis())
        .await
        .map_err(store)?
    {
        Accepted::Created(command) => Ok((StatusCode::ACCEPTED, Json(CommandView::from(&command)))),
        Accepted::Existing(command) => Ok((StatusCode::OK, Json(CommandView::from(&command)))),
        Accepted::Mismatch(command) => Err(conflict(MISMATCH, &command)),
        Accepted::Busy(command) => Err(conflict(BUSY, &command)),
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
