//! `/api/library/watch-folders`: the folders the app watches for works
//! (`docs/specs/library.md`, 작품 발견과 감시 폴더; `docs/specs/settings.md`,
//! 설정 화면의 감시 폴더).
//!
//! | call                                   | success                                   |
//! | -------------------------------------- | ----------------------------------------- |
//! | `GET /library/watch-folders`           | `200 { "folders": [FolderView] }`         |
//! | `POST /library/watch-folders`          | `201 { "folder": FolderView, "works_found": 3 }` |
//! | `DELETE /library/watch-folders/{id}`   | `200 { "removed_works": 3 }`              |
//!
//! ```json
//! { "id": "…", "path": "/media/anime", "automatic": false,
//!   "works": 3, "missing_works": 0, "linked_works": 0, "new_works": 1,
//!   "checked_at": 1760000000000, "error": null }
//! ```
//!
//! - `works` counts every work of the folder, `missing_works` of them are works
//!   whose folder is gone (`폴더 없음`), `linked_works` is 0 until works can be
//!   linked to Anissia, and `new_works` are the works found after the folder's
//!   first check, within the last [`NEW_DAYS`] days.
//! - `automatic` is true for the collect folder and the archive folder, which
//!   the app registers itself ([`crate::automatic_watch`]) while the settings
//!   use them; `DELETE` refuses them with a `400` and a sentence.
//! - `checked_at` is the last attempt to read the folder (`null` before any) and
//!   `error` a sentence when that attempt could not read everything; the works
//!   recorded before are kept.
//! - `POST` takes `{ "path": <text> }`, refuses with a `400` and a sentence a
//!   folder that is registered already, inside a registered folder, around a
//!   registered folder, missing, not a folder or unreadable, and otherwise
//!   reads it once, in the web, read-only, and answers how many works it found.
//!   The worker reads it again every cycle and on `다시 확인` (the `watch_rescan`
//!   command, see [`super::commands_api`]).
//! - `DELETE` removes the folder and its works from the library. No file on the
//!   disk is touched.
//!
//! Folders are compared after resolving links (`canonicalize`), so a link cannot
//! hide that two folders overlap; the path is stored as typed (trailing slashes
//! dropped), as it is what the worker opens. The check runs against the folders
//! as read before the transaction that registers the new one, and the
//! transaction fails if they are not the registered folders any more (another
//! folder was added or removed meanwhile); the check is then run again.

use std::path::Path;

use axum::{
    extract::{rejection::JsonRejection, Path as UrlPath, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::{commands_api::now_millis, ApiError, AppState};
use crate::{
    discovery,
    store::library::{FolderSummary, LibraryError, WatchFolder},
};

/// How often adding a folder checks again after the registered folders changed
/// under it.
const ADD_ATTEMPTS: usize = 3;

/// How long after it was found a work counts as newly found.
pub const NEW_DAYS: i64 = 7;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/library/watch-folders", get(list).post(add))
        .route("/library/watch-folders/{id}", axum::routing::delete(remove))
}

#[derive(Debug, Clone, Serialize)]
pub struct FolderView {
    pub id: String,
    pub path: String,
    /// The collect or archive folder: it cannot be unregistered.
    pub automatic: bool,
    pub works: usize,
    pub missing_works: usize,
    /// Works linked to Anissia: none can be yet.
    pub linked_works: usize,
    pub new_works: usize,
    pub checked_at: Option<i64>,
    pub error: Option<String>,
}

impl From<FolderSummary> for FolderView {
    fn from(summary: FolderSummary) -> Self {
        FolderView {
            id: summary.folder.id,
            path: summary.folder.path,
            automatic: summary.folder.automatic,
            works: summary.works,
            missing_works: summary.missing_works,
            linked_works: 0,
            new_works: summary.new_works,
            checked_at: summary.folder.checked_at,
            error: summary.folder.error,
        }
    }
}

#[derive(Serialize)]
struct FolderList {
    folders: Vec<FolderView>,
}

#[derive(Serialize)]
struct Added {
    folder: FolderView,
    works_found: usize,
}

#[derive(Serialize)]
struct Removed {
    removed_works: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddBody {
    path: String,
}

const BAD_BODY: &str = "요청 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";
const NOT_FOUND: &str = "감시 폴더를 찾지 못했어요. 이미 등록이 해제됐을 수 있어요.";

const AUTOMATIC: &str = "수집 폴더나 보관 폴더라서 등록을 해제할 수 없어요. 수집 폴더 설정에서 그 폴더를 바꾸면 감시 폴더도 함께 바뀌어요.";
const CHANGED: &str = "감시 폴더가 그사이에 바뀌었어요. 목록을 확인하고 다시 시도해 주세요.";

pub(super) fn store_error(e: LibraryError) -> ApiError {
    match e {
        LibraryError::Duplicate => ApiError::invalid("이미 등록한 감시 폴더예요."),
        LibraryError::Automatic => ApiError::invalid(AUTOMATIC),
        LibraryError::Changed => ApiError::Conflict {
            message: CHANGED.to_owned(),
            current: None,
        },
        LibraryError::Db(e) => ApiError::Internal(e.to_string()),
    }
}

async fn summaries(state: &AppState) -> Result<Vec<FolderSummary>, ApiError> {
    state
        .library
        .summaries(now_millis() - NEW_DAYS * DAY_MS)
        .await
        .map_err(store_error)
}

async fn list(State(state): State<AppState>) -> Result<Json<FolderList>, ApiError> {
    let folders = summaries(&state)
        .await?
        .into_iter()
        .map(FolderView::from)
        .collect();
    Ok(Json(FolderList { folders }))
}

async fn add(
    State(state): State<AppState>,
    parsed: Result<Json<AddBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Added>), ApiError> {
    let Json(body) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;
    let path = normalize(&body.path);
    if path.is_empty() {
        return Err(ApiError::invalid("감시 폴더의 경로를 입력해 주세요."));
    }

    // Check against the folders as they are, read the folder once, and add it
    // in a transaction that notices if the folders changed in between.
    let mut read: Option<discovery::Scan> = None;
    let mut attempt = 0;
    let (folder, report) = loop {
        let registered = state.library.folders().await.map_err(store_error)?;
        let (registered, scan) = {
            let path = path.clone();
            let earlier = read.take();
            tokio::task::spawn_blocking(move || {
                let scan = check_and_scan(&path, &registered, earlier)?;
                Ok::<_, ApiError>((registered, scan))
            })
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))??
        };
        match state
            .library
            .add_folder(path.clone(), scan.clone(), now_millis(), &registered)
            .await
        {
            Err(LibraryError::Changed) if attempt < ADD_ATTEMPTS => {
                attempt += 1;
                read = Some(scan);
            }
            other => break other.map_err(store_error)?,
        }
    };
    let summary = state
        .library
        .summaries(now_millis() - NEW_DAYS * DAY_MS)
        .await
        .map_err(store_error)?
        .into_iter()
        .find(|s| s.folder.id == folder.id)
        .unwrap_or(FolderSummary {
            works: report.works_found,
            missing_works: 0,
            new_works: 0,
            folder,
        });
    Ok((
        StatusCode::CREATED,
        Json(Added {
            works_found: report.works_found,
            folder: summary.into(),
        }),
    ))
}

async fn remove(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<Removed>, ApiError> {
    match state
        .library
        .remove_folder(&id)
        .await
        .map_err(store_error)?
    {
        Some(removed_works) => Ok(Json(Removed { removed_works })),
        None => Err(ApiError::not_found(NOT_FOUND)),
    }
}

/// The folder text as stored: surrounding spaces and trailing slashes dropped
/// (a lone `/` stays).
fn normalize(text: &str) -> String {
    let text = text.trim();
    match text.trim_end_matches('/') {
        "" if text.starts_with('/') => "/".to_owned(),
        trimmed => trimmed.to_owned(),
    }
}

/// Checks that `text` can be a new watch folder and reads it once (`read` is a
/// reading made for an earlier check of the same folder, which is kept).
fn check_and_scan(
    text: &str,
    registered: &[WatchFolder],
    read: Option<discovery::Scan>,
) -> Result<discovery::Scan, ApiError> {
    let path = Path::new(text);
    if !path.is_absolute() {
        return Err(ApiError::invalid(
            "감시 폴더는 `/`로 시작하는 전체 경로로 입력해 주세요. 예: `/media/anime`",
        ));
    }
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::invalid(format!(
                "`{text}`를 찾지 못했어요. 웹이 볼 수 있는 경로인지, 폴더가 마운트돼 있는지 확인해 주세요."
            )))
        }
        Err(_) => {
            return Err(ApiError::invalid(format!(
                "`{text}`를 열지 못했어요. 웹이 볼 수 있는 경로인지, 읽을 권한이 있는지 확인해 주세요."
            )))
        }
    };
    if !metadata.is_dir() {
        return Err(ApiError::invalid(format!("`{text}`는 폴더가 아니에요.")));
    }

    let real = std::fs::canonicalize(path).map_err(|_| {
        ApiError::invalid(format!(
            "`{text}`를 열지 못했어요. 웹이 볼 수 있는 경로인지 확인해 주세요."
        ))
    })?;
    for other in registered {
        let other_real = crate::automatic_watch::resolved(&other.path);
        if other_real == real {
            return Err(ApiError::invalid("이미 등록한 감시 폴더예요."));
        }
        if real.starts_with(&other_real) {
            return Err(ApiError::invalid(format!(
                "이미 등록한 감시 폴더 `{}` 안에 있는 폴더예요. 그 폴더가 이 안의 작품도 이미 찾고 있어요.",
                other.path
            )));
        }
        if other_real.starts_with(&real) {
            return Err(ApiError::invalid(if other.automatic {
                format!(
                    "이 폴더 안에 수집 폴더나 보관 폴더 `{}`가 있어요. 두 폴더는 늘 감시하므로, 같은 작품을 두 번 찾게 되는 이 폴더는 추가할 수 없어요.",
                    other.path
                )
            } else {
                format!(
                    "이 폴더 안에 이미 등록한 감시 폴더 `{}`가 있어요. 같은 작품을 두 번 찾게 되므로, 그 폴더의 등록을 해제한 뒤 이 폴더를 추가해 주세요.",
                    other.path
                )
            }));
        }
    }

    if let Some(read) = read {
        return Ok(read);
    }
    discovery::scan(path).map_err(|e| {
        eprintln!("trss-web: cannot read the watch folder {text}: {e}");
        ApiError::invalid(format!("`{text}`를 읽지 못했어요. {}", e.message))
    })
}
