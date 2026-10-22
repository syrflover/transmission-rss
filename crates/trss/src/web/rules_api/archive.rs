//! `보관` of a rule: `POST /api/rules/{id}/archive` (see the parent module's
//! docs). The rule is deleted with the move of its work folder, in one
//! transaction ([`work_archive::archive_rule`]); the worker moves the folder,
//! or keeps it while another rule still collects into it.

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use super::{body, collection_folders, rule_conflict, store_error};
use crate::web::{
    commands_api::{now_millis, valid_id, CommandView},
    ApiError, AppState,
};
use trss_collect::{
    commands::work_archive::{self, ArchiveError, NotArchived, RuleArchived, WorkArchive},
    store::channels::ChannelError,
};
use trss_core::commands::Command;

const MISMATCH: &str =
    "이 명령 ID는 다른 내용으로 이미 접수됐어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";
const WORK_BUSY: &str =
    "이 규칙의 작품 폴더를 이미 보관 폴더로 옮기는 중이에요. 그 결과가 나온 뒤 다시 보관해 주세요.";

/// What `POST /rules/{id}/archive` takes.
#[derive(Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "RuleArchiveBody"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchiveBody {
    /// The move's ID, one per person's action (see
    /// [`crate::web::commands_api`]).
    id: String,
    /// The rule's version the person saw.
    version: i64,
}

/// What `보관` of a rule did: the rule is deleted.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "RuleArchived"))]
pub(crate) struct ArchivedView {
    /// The move of the rule's work folder, stored with the deletion: the
    /// screen follows it by its ID (`GET /api/commands/{id}`), and its outcome
    /// says whether the folder moved, or stayed and why. `null` when the rule
    /// had no work folder.
    command: Option<CommandView>,
    /// Why no folder moves, when `command` is `null`.
    reason: Option<String>,
    /// The work of the rule's work folder in the library, for a link to its
    /// detail; `null` when the library does not know it.
    work_id: Option<String>,
}

/// The work of the folder `command` moves, in the library.
async fn work_of(state: &AppState, command: &Command) -> Result<Option<String>, ApiError> {
    let Ok(payload) = serde_json::from_str::<WorkArchive>(&command.payload) else {
        return Ok(None);
    };
    let Some((collect, archive)) = collection_folders(state).await? else {
        return Ok(None);
    };
    work_archive::work_in_library(&state.library, &collect, archive.as_deref(), &payload.work)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

async fn moving(state: &AppState, command: &Command) -> Result<ArchivedView, ApiError> {
    Ok(ArchivedView {
        command: Some(CommandView::from(command)),
        reason: None,
        work_id: work_of(state, command).await?,
    })
}

fn conflict(message: &str, command: &Command) -> ApiError {
    ApiError::Conflict {
        message: message.to_owned(),
        current: serde_json::to_value(CommandView::from(command)).ok(),
    }
}

/// `POST /api/rules/{id}/archive`: deletes the rule and stores the move of its
/// work folder.
pub(super) async fn archive(
    State(state): State<AppState>,
    Path(id): Path<String>,
    parsed: Result<Json<ArchiveBody>, JsonRejection>,
) -> Result<(StatusCode, Json<ArchivedView>), ApiError> {
    let b = body(parsed)?;
    if !valid_id(&b.id) {
        return Err(ApiError::invalid(
            "명령 ID가 올바르지 않아요. 화면을 새로고침한 뒤 다시 시도해 주세요.",
        ));
    }
    let store = |e: trss_core::commands::CommandError| ApiError::Internal(e.to_string());

    // A request accepted before is answered as it stands: the rule is gone by
    // then, and the folder may have moved since. A rule that is still there
    // was not archived by it (the rule goes with the move's acceptance), so
    // the ID was used for another rule.
    if let Some(stored) = state.commands.get(&b.id).await.map_err(store)? {
        if stored.kind != work_archive::KIND
            || state
                .channels
                .get_rule(&id)
                .await
                .map_err(store_error)?
                .is_some()
        {
            return Err(conflict(MISMATCH, &stored));
        }
        return Ok((StatusCode::OK, Json(moving(&state, &stored).await?)));
    }
    let rule = state
        .channels
        .get_rule(&id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ChannelError::NotFound {
            kind: "rule",
            id: id.clone(),
        })?;
    if rule.version != b.version {
        return Err(rule_conflict(&state, &id).await);
    }
    let collect = collection_folders(&state)
        .await?
        .map(|(collect, _)| collect);
    let archived = work_archive::archive_rule(
        &state.channels,
        &state.commands,
        collect.as_deref(),
        &rule,
        b.id,
        now_millis(),
    )
    .await
    .map_err(|e| match e {
        ArchiveError::Channels(e) => store_error(e),
        ArchiveError::Commands(e) => store(e),
    })?;
    match archived {
        Ok(RuleArchived::Moving(command)) => {
            // The commands' loop starts it now.
            state.wakes.commands();
            Ok((StatusCode::ACCEPTED, Json(moving(&state, &command).await?)))
        }
        Ok(RuleArchived::Deleted(no_move)) => Ok((
            StatusCode::OK,
            Json(ArchivedView {
                command: None,
                reason: Some(no_move.reason()),
                work_id: None,
            }),
        )),
        Err(NotArchived::Changed(ChannelError::NotFound { kind, id })) => {
            Err(ChannelError::NotFound { kind, id }.into())
        }
        Err(NotArchived::Changed(_)) => Err(rule_conflict(&state, &id).await),
        Err(NotArchived::Busy(open)) => Err(conflict(WORK_BUSY, &open)),
        Err(NotArchived::Mismatch(stored)) => Err(conflict(MISMATCH, &stored)),
    }
}
