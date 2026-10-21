//! `/api/archive-batches`: how a `rule_archive_batch` command is going
//! (`docs/specs/collection.md`, 규칙 보관).
//!
//! | call                                | success                                              |
//! | ----------------------------------- | ---------------------------------------------------- |
//! | `GET /archive-batches/{id}`         | `200 ArchiveBatchView`, or `404` when no batch has that ID |
//! | `GET /archive-batches/current`      | `200 ArchiveBatchView` of the open batch, or `404` when none is open |
//!
//! The batch is sent as a command (`POST /api/commands`, kind
//! `rule_archive_batch`, [`super::commands_api`]) and archived by the server
//! rule after rule ([`crate::archive_batch`]); this is the read of where it
//! is. The ID is the batch command's, which the event stream names when the
//! batch changes, and `current` is for a screen that opens while a batch runs
//! and does not know its ID. The view is computed from the batch command and
//! the `rule_archive` commands it stored for its rules each time it is read.

use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};

use super::{ApiError, AppState};
use crate::archive_batch::{self, BatchView};
use trss_core::commands::Command;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/archive-batches/current", get(current))
        .route("/archive-batches/{id}", get(read))
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

async fn view(state: &AppState, batch: &Command) -> Result<Json<BatchView>, ApiError> {
    archive_batch::view(&state.commands, batch)
        .await
        .map(Json)
        .map_err(internal)
}

async fn read(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<BatchView>, ApiError> {
    let batch = state
        .commands
        .get(&id)
        .await
        .map_err(internal)?
        .filter(|command| command.kind == archive_batch::KIND)
        .ok_or_else(|| ApiError::not_found("이 ID의 일괄 보관을 찾지 못했어요."))?;
    view(&state, &batch).await
}

async fn current(State(state): State<AppState>) -> Result<Json<BatchView>, ApiError> {
    let open = state
        .commands
        .open_for_subjects(archive_batch::KIND, vec![archive_batch::SUBJECT.to_owned()])
        .await
        .map_err(internal)?;
    let batch = open
        .get(archive_batch::SUBJECT)
        .ok_or_else(|| ApiError::not_found("진행 중인 일괄 보관이 없어요."))?;
    view(&state, batch).await
}

#[cfg(test)]
mod tests;
