//! `/api/settings/collection`: the collect folder and the archive folder.
//!
//! | call                         | success                                  |
//! | ---------------------------- | ---------------------------------------- |
//! | `GET /settings/collection`   | `200 CollectionView`                     |
//! | `PUT /settings/collection`   | `200 CollectionView` (as saved)          |
//!
//! ```json
//! { "folder": "/downloads/Shows (current)" | null,
//!   "archive_folder": "/downloads/Shows" | null,
//!   "version": 2 }
//! ```
//!
//! `folder` is `null` and `version` is 0 until the collect folder has been
//! chosen; the worker adds no torrents until then. `archive_folder` is `null`
//! when the user keeps none. The `PUT` body is
//! `{ "version": <the version the client saw>, "folder": <text>,
//! "archive_folder": <text or null> }`; a version that is not the stored one
//! answers `409` with the stored settings as `current`, and nothing is saved.
//!
//! # What the web checks
//!
//! The web sees the media folders read-only, so it checks what it can see and
//! leaves writability to the worker, which finds out when it writes:
//!
//! - both folders are absolute paths to existing directories;
//! - they are not the same folder and neither is inside the other, compared
//!   after resolving links (`canonicalize`), so a link cannot hide a nesting;
//! - both are on the same filesystem (`st_dev`), because the archive move is a
//!   rename and a copy across filesystems is not something it does.
//!
//! - neither is inside a watch folder the user registered, and neither contains
//!   one (the same work would be found twice).
//!
//! The folders are stored as typed (trailing slashes dropped), not resolved:
//! Transmission's folder names are compared as written.
//!
//! # The watch folders
//!
//! The two folders are always watch folders ([`trss_legacy::automatic_watch`]). The
//! save registers a newly set folder (reading it once, so that its first
//! reading is the baseline as for a folder added by hand), turns a folder the
//! user registered at the same place into the automatic one with its records,
//! and removes the automatic watch folder of a path that is not used any more,
//! all in the transaction that stores the settings.

use std::path::{Path, PathBuf};

use axum::{
    extract::{rejection::JsonRejection, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::{commands_api::now_millis, watch_folders_api, ApiError, AppState};
use trss_legacy::{
    automatic_watch::{self, Wanted},
    store::{
        db::DbError,
        library::{self, LibraryError},
        settings::{CollectionSettings, SettingsError},
    },
};

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new().route("/settings/collection", get(read).put(write))
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionView {
    /// `null` until the collect folder is chosen.
    pub folder: Option<String>,
    pub archive_folder: Option<String>,
    /// 0 while nothing is stored; send it back as `version` when saving.
    pub version: i64,
}

impl From<Option<CollectionSettings>> for CollectionView {
    fn from(settings: Option<CollectionSettings>) -> Self {
        match settings {
            Some(s) => CollectionView {
                folder: Some(s.folder),
                archive_folder: s.archive_folder,
                version: s.version,
            },
            None => CollectionView {
                folder: None,
                archive_folder: None,
                version: 0,
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteBody {
    /// The version the client saw.
    version: i64,
    folder: String,
    #[serde(default)]
    archive_folder: Option<String>,
}

const BAD_BODY: &str = "요청 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";

fn store_error(e: SettingsError) -> ApiError {
    match e {
        SettingsError::Invalid(_) => ApiError::invalid("입력한 값으로는 저장할 수 없어요."),
        SettingsError::Conflict { .. } => ApiError::Conflict {
            message: "다른 곳에서 먼저 저장했어요. 지금 값을 확인하고 다시 저장해 주세요."
                .to_owned(),
            current: None,
        },
        SettingsError::Db(e) => ApiError::Internal(e.to_string()),
    }
}

async fn current(state: &AppState) -> Result<CollectionView, ApiError> {
    Ok(state
        .settings
        .collection()
        .await
        .map_err(store_error)?
        .into())
}

async fn read(State(state): State<AppState>) -> Result<Json<CollectionView>, ApiError> {
    Ok(Json(current(&state).await?))
}

async fn write(
    State(state): State<AppState>,
    parsed: Result<Json<WriteBody>, JsonRejection>,
) -> Result<Json<CollectionView>, ApiError> {
    let Json(body) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;

    let WriteBody {
        version,
        folder,
        archive_folder,
    } = body;
    let (folder, archive_folder) =
        tokio::task::spawn_blocking(move || check_folders(&folder, archive_folder.as_deref()))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))??;

    // The settings and the watch folders change together. The plan is made
    // from the registered folders as read here; if they change before the
    // transaction, it fails and the plan is made again.
    let mut attempt = 0;
    loop {
        let registered = state.library.folders().await.map_err(watch_error)?;
        let plan = {
            let wanted = wanted_folders(&folder, archive_folder.as_deref());
            tokio::task::spawn_blocking(move || {
                let mut plan =
                    automatic_watch::plan(&wanted, &registered).map_err(ApiError::invalid)?;
                automatic_watch::read_new_folders(&mut plan);
                Ok::<_, ApiError>(plan)
            })
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))??
        };
        let saved = state
            .settings
            .put_collection_with(version, folder.clone(), archive_folder.clone(), move |tx| {
                library::apply_automatic_in(tx, &plan, now_millis()).map_err(SaveError::Library)
            })
            .await;
        match saved {
            Ok((saved, _)) => return Ok(Json(Some(saved).into())),
            Err(SaveError::Library(LibraryError::Changed)) if attempt < SAVE_ATTEMPTS => {
                attempt += 1;
            }
            Err(SaveError::Library(e)) => return Err(watch_error(e)),
            Err(SaveError::Settings(SettingsError::Conflict { .. })) => {
                return Err(ApiError::conflict_with(current(&state).await?))
            }
            Err(SaveError::Settings(e)) => return Err(store_error(e)),
        }
    }
}

/// How often a save plans again after the watch folders changed under it.
const SAVE_ATTEMPTS: usize = 3;

fn wanted_folders(folder: &str, archive_folder: Option<&str>) -> Vec<Wanted> {
    let mut wanted = vec![Wanted {
        what: "수집 폴더",
        path: folder.to_owned(),
    }];
    if let Some(archive) = archive_folder {
        wanted.push(Wanted {
            what: "보관 폴더",
            path: archive.to_owned(),
        });
    }
    wanted
}

fn watch_error(e: LibraryError) -> ApiError {
    watch_folders_api::store_error(e)
}

/// Why saving the settings with their watch folders failed.
enum SaveError {
    Settings(SettingsError),
    Library(LibraryError),
}

impl From<SettingsError> for SaveError {
    fn from(e: SettingsError) -> Self {
        SaveError::Settings(e)
    }
}

impl From<DbError> for SaveError {
    fn from(e: DbError) -> Self {
        SaveError::Settings(SettingsError::Db(e))
    }
}

/// Checks the two folders and returns them as they are to be stored: trimmed
/// and without trailing slashes, the archive folder `None` when left blank.
pub fn check_folders(
    folder: &str,
    archive_folder: Option<&str>,
) -> Result<(String, Option<String>), ApiError> {
    let folder = normalize(folder);
    let archive_folder = archive_folder.map(normalize).filter(|f| !f.is_empty());
    if folder.is_empty() {
        return Err(ApiError::invalid("수집 폴더를 입력해 주세요."));
    }

    let collect = checked_directory("수집 폴더", &folder)?;
    let Some(archive_text) = &archive_folder else {
        return Ok((folder, None));
    };
    let archive = checked_directory("보관 폴더", archive_text)?;

    if collect.real == archive.real {
        return Err(ApiError::invalid(
            "수집 폴더와 보관 폴더가 같은 폴더예요. 서로 다른 폴더를 정해 주세요.",
        ));
    }
    if archive.real.starts_with(&collect.real) {
        return Err(ApiError::invalid(
            "보관 폴더가 수집 폴더 안에 있어요. 작품 폴더를 옮기면 수집 폴더 안에서 겹치므로, 서로 밖에 있는 폴더를 정해 주세요.",
        ));
    }
    if collect.real.starts_with(&archive.real) {
        return Err(ApiError::invalid(
            "수집 폴더가 보관 폴더 안에 있어요. 작품 폴더를 옮기면 보관 폴더 안에서 겹치므로, 서로 밖에 있는 폴더를 정해 주세요.",
        ));
    }
    if collect.device != archive.device {
        return Err(ApiError::invalid(
            "수집 폴더와 보관 폴더가 서로 다른 파일시스템에 있어요. 보관할 때 폴더를 복사하지 않고 이름만 바꿔 옮기므로, 같은 파일시스템 안의 폴더를 정해 주세요.",
        ));
    }
    Ok((folder, archive_folder))
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

struct Checked {
    /// The folder with links resolved, for comparing the two.
    real: PathBuf,
    device: Option<u64>,
}

/// An existing directory the web can open. `what` names the field in messages.
fn checked_directory(what: &str, text: &str) -> Result<Checked, ApiError> {
    let path = Path::new(text);
    if !path.is_absolute() {
        return Err(ApiError::invalid(format!(
            "{what}는 `/`로 시작하는 전체 경로로 입력해 주세요. 예: `/downloads/Shows`"
        )));
    }
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::invalid(format!(
                "{what} `{text}`를 찾지 못했어요. 폴더를 먼저 만들어 두고, 웹이 볼 수 있는 경로인지 확인해 주세요."
            )))
        }
        Err(_) => {
            return Err(ApiError::invalid(format!(
                "{what} `{text}`를 열지 못했어요. 웹이 볼 수 있는 경로인지 확인해 주세요."
            )))
        }
    };
    if !metadata.is_dir() {
        return Err(ApiError::invalid(format!(
            "{what} `{text}`는 폴더가 아니에요."
        )));
    }
    let real = std::fs::canonicalize(path).map_err(|_| {
        ApiError::invalid(format!(
            "{what} `{text}`를 열지 못했어요. 웹이 볼 수 있는 경로인지 확인해 주세요."
        ))
    })?;
    Ok(Checked {
        real,
        device: device_of(&metadata),
    })
}

#[cfg(unix)]
fn device_of(metadata: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.dev())
}

#[cfg(not(unix))]
fn device_of(_: &std::fs::Metadata) -> Option<u64> {
    None
}
