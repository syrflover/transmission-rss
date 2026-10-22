//! `보관 폴더로 옮기기` of a work: `POST /api/library/works/{id}/archive`, and
//! the work detail's `archive` (see the parent module's docs). The worker
//! moves the folder ([`work_archive`]); this deletes the rules first, so no new
//! episode arrives at the old place while the folder moves.

use std::{collections::BTreeSet, path::Path as FsPath};

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use super::{rules_of, RuleRef};
use crate::web::{
    commands_api::{now_millis, valid_id, CommandView},
    ApiError, AppState,
};
use trss_collect::{
    commands::{
        rule_archive::{FAILED, KEPT},
        work_archive::{self, WorkArchive},
    },
    store::channels::ChannelError,
};
use trss_core::commands::{Accepted, Command};
use trss_library::store::library::{LibraryError, WorkDetail};

const BAD_BODY: &str = "요청 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";
const NOT_IN_COLLECT: &str = "이 작품은 수집 폴더에 있지 않아서 보관 폴더로 옮길 수 없어요.";
const NO_ARCHIVE_FOLDER: &str =
    "보관 폴더를 정하지 않아서 옮길 수 없어요. 설정에서 보관 폴더를 정해 주세요.";
const MISSING: &str = "작품 폴더가 없어서 옮길 것이 없어요.";
const RULES_CHANGED: &str =
    "이 작품 폴더에 받는 규칙이 바뀌었어요. 지울 규칙을 다시 확인해 주세요.";
const BUSY: &str =
    "이 작품은 이미 보관 폴더로 옮기는 중이에요. 그 결과가 나올 때까지 기다려 주세요.";
const MISMATCH: &str =
    "이 명령 ID는 다른 내용으로 이미 접수됐어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";

/// The work detail's `archive`: whether the work can be moved and the move to
/// show.
#[derive(Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "WorkArchive"))]
pub(super) struct ArchiveView {
    /// The work is in the collect folder, its folder is there and an archive
    /// folder is set: `보관 폴더로 옮기기` is offered.
    available: bool,
    /// The work's move while it is open; and the last one that ended `failed`
    /// or `kept` while the work is still in the collect folder and no rule
    /// saves into its folder (the rules were deleted for it). `null`
    /// otherwise.
    command: Option<CommandView>,
}

/// What `POST /api/library/works/{id}/archive` takes.
#[derive(Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "WorkArchiveBody"))]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchiveBody {
    /// The command's ID, one per person's action (see
    /// [`crate::web::commands_api`]).
    id: String,
    /// The rules the confirmation showed, with the versions it showed: the
    /// ones saving into the work folder now.
    rules: Vec<RuleVersion>,
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "WorkArchiveRule"))]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleVersion {
    id: String,
    version: i64,
}

/// The `current` of a `409` from `POST …/archive`: the rules saving into the
/// work folder now, and the command the conflict is about (the open move, or
/// the one stored under the ID sent) when there is one.
#[derive(Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "WorkArchiveConflict"))]
pub(crate) struct ArchiveConflict {
    rules: Vec<RuleRef>,
    command: Option<CommandView>,
}

/// Where the work stands for a move, from the settings and the library.
struct Standing {
    /// The rules saving into the work folder.
    rules: Vec<RuleRef>,
    /// Why the work cannot be moved, if it cannot.
    refusal: Option<&'static str>,
}

async fn standing(state: &AppState, work: &WorkDetail) -> Result<Standing, ApiError> {
    let settings = state
        .settings
        .collection()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let channels = state.channels.list_channels_with_rules().await?;
    let collect = settings.as_ref().map(|s| s.folder.as_str());
    let archive = settings.as_ref().and_then(|s| s.archive_folder.as_deref());
    let rules = rules_of(work, collect, archive, &channels);
    let in_collect =
        collect.is_some_and(|c| FsPath::new(&work.watch_folder_path) == FsPath::new(c));
    let refusal = if !in_collect {
        Some(NOT_IN_COLLECT)
    } else if archive.is_none() {
        Some(NO_ARCHIVE_FOLDER)
    } else if work.missing {
        Some(MISSING)
    } else {
        None
    };
    Ok(Standing { rules, refusal })
}

/// The work detail's `archive` for `work`, whose rules are `rules`.
pub(super) async fn archive_view(
    state: &AppState,
    work: &WorkDetail,
    rules: &[RuleRef],
    collect: Option<&str>,
    archive: Option<&str>,
) -> Result<ArchiveView, ApiError> {
    let in_collect =
        collect.is_some_and(|c| FsPath::new(&work.watch_folder_path) == FsPath::new(c));
    let available = in_collect && archive.is_some() && !work.missing;
    let latest = state
        .commands
        .latest_for_subjects(work_archive::KIND, vec![work.dir_name.clone()])
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .remove(&work.dir_name);
    let shown = latest.filter(|command| {
        command.state.is_open()
            || (in_collect
                && rules.is_empty()
                && command
                    .outcome
                    .as_ref()
                    .is_some_and(|o| o.result == FAILED || o.result == KEPT))
    });
    Ok(ArchiveView {
        available,
        command: shown.as_ref().map(CommandView::from),
    })
}

fn conflict(message: &str, rules: Vec<RuleRef>, command: Option<&Command>) -> ApiError {
    ApiError::Conflict {
        message: message.to_owned(),
        current: serde_json::to_value(ArchiveConflict {
            rules,
            command: command.map(CommandView::from),
        })
        .ok(),
    }
}

/// `POST /api/library/works/{id}/archive`: deletes the rules saving into the
/// work folder and accepts the move (see the parent module's docs).
pub(super) async fn archive(
    State(state): State<AppState>,
    Path(id): Path<String>,
    parsed: Result<Json<ArchiveBody>, JsonRejection>,
) -> Result<(StatusCode, Json<CommandView>), ApiError> {
    // The rejection text can quote the offending value, so it is dropped.
    let Json(body) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;
    if !valid_id(&body.id) {
        return Err(ApiError::invalid(
            "명령 ID가 올바르지 않아요. 화면을 새로고침한 뒤 다시 시도해 주세요.",
        ));
    }
    let work = state
        .library
        .work_detail(&id)
        .await
        .map_err(|e| match e {
            LibraryError::Db(e) => ApiError::Internal(e.to_string()),
            LibraryError::Duplicate | LibraryError::Automatic | LibraryError::Changed => {
                ApiError::Internal(format!("unexpected: {e}"))
            }
        })?
        .ok_or_else(|| ApiError::not_found("이 작품을 찾지 못했어요."))?;
    let payload = WorkArchive {
        work: work.dir_name.clone(),
    };
    let store = |e: trss_core::commands::CommandError| ApiError::Internal(e.to_string());

    // A request accepted before is answered as it stands: its rules are gone
    // and the work may have moved since.
    match state.commands.get(&body.id).await.map_err(store)? {
        Some(stored)
            if stored.kind == work_archive::KIND && stored.payload == payload.canonical() =>
        {
            return Ok((StatusCode::OK, Json(CommandView::from(&stored))));
        }
        Some(stored) => {
            let rules = standing(&state, &work).await?.rules;
            return Err(conflict(MISMATCH, rules, Some(&stored)));
        }
        None => {}
    }

    let Standing { rules, refusal } = standing(&state, &work).await?;
    if let Some(why) = refusal {
        return Err(ApiError::Conflict {
            message: why.to_owned(),
            current: None,
        });
    }
    let open = state
        .commands
        .open_for_subjects(work_archive::KIND, vec![payload.subject()])
        .await
        .map_err(store)?
        .remove(&payload.subject());
    if let Some(open) = open {
        return Err(conflict(BUSY, rules, Some(&open)));
    }
    let sent: BTreeSet<(&str, i64)> = body
        .rules
        .iter()
        .map(|r| (r.id.as_str(), r.version))
        .collect();
    let now: BTreeSet<(&str, i64)> = rules.iter().map(|r| (r.id.as_str(), r.version)).collect();
    if sent.len() != body.rules.len() || sent != now {
        return Err(conflict(RULES_CHANGED, rules, None));
    }

    // The rules go with the command: none of them adds into the folder while
    // it moves, and a refused command deletes none of them. The checks above
    // are made again in its transaction.
    let doomed = rules.iter().map(|r| (r.id.clone(), r.version)).collect();
    let accepted = work_archive::accept_deleting(
        &state.commands,
        payload.command(body.id),
        doomed,
        now_millis(),
    )
    .await
    .map_err(store)?;
    match accepted {
        Ok(Accepted::Created(command)) => {
            // The commands' loop starts it now.
            state.wakes.commands();
            Ok((StatusCode::ACCEPTED, Json(CommandView::from(&command))))
        }
        // The same request, stored by a delivery that got there first.
        Ok(Accepted::Existing(command)) => Ok((StatusCode::OK, Json(CommandView::from(&command)))),
        Ok(Accepted::Mismatch(command)) => Err(conflict(MISMATCH, rules, Some(&command))),
        Ok(Accepted::Busy(command)) => Err(conflict(BUSY, rules, Some(&command))),
        Err(e) if e.is_conflict() || matches!(e, ChannelError::NotFound { .. }) => {
            let rules = standing(&state, &work).await?.rules;
            Err(conflict(RULES_CHANGED, rules, None))
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests;
