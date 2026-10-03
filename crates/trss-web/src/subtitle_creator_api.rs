//! The creator of the subtitle files of a season (`docs/specs/library.md`,
//! 머리와 시즌의 `제작자 지정`): a subtitle file found in a watch folder is by
//! `제작자 알 수 없음` until the user names one of the creators of the Anissia
//! anime the season is linked to.
//!
//! | method and path                                                       | body                                                  |
//! | --------------------------------------------------------------------- | ----------------------------------------------------- |
//! | `POST /api/library/works/{id}/seasons/{n}/subtitle-creators`          | `{ "creator": "하느" }`                                |
//! | `PUT  /api/library/works/{id}/seasons/{n}/subtitle-creators/file`     | `{ "path": "Season 01/… .ass", "version": 0, "creator": "하느" }` |
//!
//! - `POST` names the creator of every subtitle file of the season that has
//!   none, at once (the head's `제작자 지정`): `{ "season": 1, "creator":
//!   { "source_id": "…", "name": "하느", "anime_no": 3441 }, "attached": 12 }`.
//!   It touches only files that are unknown when it runs, so it never replaces
//!   a creator another screen named in the meantime; `attached` is how many it
//!   named (0 when none was unknown any more).
//! - `PUT` changes one subtitle file's creator (an expanded episode row), or
//!   puts it back to `제작자 알 수 없음` with `"creator": null`. `version` is the
//!   file's `creator_version` the screen read in the work detail; a file whose
//!   creator changed since answers `409` with the file as it is now in
//!   `current`, and changes nothing. A file the work no longer has answers
//!   `404`. The answer, and `current`, are `{ "path": "…", "version": 3,
//!   "creator": { "source_id": "…", "name": "하느", "anime_no": 3441 } | null }`.
//! - The creator is one of the creators of the season's anime, found by the
//!   rule of a subscription's creator ([`super::subscriptions_api`]): a line
//!   the app observed, or one Anissia lists now. A season with no Anissia link
//!   answers `400` (link it first); a creator the anime does not have answers
//!   `400`. The creator's subtitle source is made if no line of it was
//!   observed yet.
//! - No file is moved, renamed or received, and no job is made: a work with no
//!   subscription gets no automatic receipt out of this.

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    routing::{post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::{
    seasons_anissia_api::refused_store, subscriptions_api::named_creator, ApiError, AppState,
};
use trss_library::store::library::{CreatorError, CreatorSet, FileCreator};

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/library/works/{id}/seasons/{season}/subtitle-creators",
            post(name_unknown),
        )
        .route(
            "/library/works/{id}/seasons/{season}/subtitle-creators/file",
            put(set_file),
        )
}

/// A creator the user named for a subtitle file.
#[derive(Debug, Serialize)]
pub struct CreatorView {
    pub source_id: String,
    pub name: String,
    pub anime_no: i64,
}

impl From<FileCreator> for CreatorView {
    fn from(c: FileCreator) -> Self {
        CreatorView {
            source_id: c.source_id,
            name: c.name,
            anime_no: c.anime_no,
        }
    }
}

/// One subtitle file's creator and its version.
#[derive(Debug, Serialize)]
struct FileCreatorView {
    path: String,
    version: i64,
    creator: Option<CreatorView>,
}

impl FileCreatorView {
    fn of(path: String, set: CreatorSet) -> Self {
        FileCreatorView {
            path,
            version: set.version,
            creator: set.creator.map(CreatorView::from),
        }
    }
}

const BAD_BODY: &str = "요청 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

/// The Anissia anime the season is linked to: the creators to name are its.
async fn season_anime(state: &AppState, work_id: &str, season: u32) -> Result<i64, ApiError> {
    let link = state
        .seasons
        .store
        .anissia_link(work_id, season)
        .await
        .map_err(refused_store)?;
    link.anime_no.ok_or_else(|| {
        ApiError::invalid("이 시즌에 Anissia 작품을 먼저 연결해 주세요. 연결한 작품의 제작자 중에서 고를 수 있어요.")
    })
}

/// The subtitle source of the creator `name` of the anime, and the creator's
/// name as the anime has it.
async fn source_of(
    state: &AppState,
    anime_no: i64,
    name: &str,
) -> Result<(String, String), ApiError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::invalid("자막 제작자를 골라 주세요."));
    }
    let name = named_creator(state, anime_no, name).await?;
    let source = state
        .anissia_store
        .source_of_creator(anime_no, name.clone(), state.anissia.now())
        .await
        .map_err(internal)?;
    Ok((source, name))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NameBody {
    creator: String,
}

#[derive(Serialize)]
struct Named {
    season: u32,
    creator: CreatorView,
    attached: usize,
}

async fn name_unknown(
    State(state): State<AppState>,
    Path((id, season)): Path<(String, u32)>,
    body: Result<Json<NameBody>, JsonRejection>,
) -> Result<Json<Named>, ApiError> {
    let Json(body) = body.map_err(|_| ApiError::invalid(BAD_BODY))?;
    let anime_no = season_anime(&state, &id, season).await?;
    let (source_id, name) = source_of(&state, anime_no, &body.creator).await?;
    let attached = state
        .library
        .name_unknown_creators(&id, season, &source_id, state.anissia.now())
        .await
        .map_err(internal)?;
    Ok(Json(Named {
        season,
        creator: CreatorView {
            source_id,
            name,
            anime_no,
        },
        attached,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileBody {
    path: String,
    version: i64,
    /// `null` puts the file back to `제작자 알 수 없음`.
    creator: Option<String>,
}

const NO_FILE: &str =
    "이 자막 파일을 찾지 못했어요. 파일이 폴더에서 없어졌거나 이름이 바뀌었을 수 있어요.";

async fn set_file(
    State(state): State<AppState>,
    Path((id, season)): Path<(String, u32)>,
    body: Result<Json<FileBody>, JsonRejection>,
) -> Result<Json<FileCreatorView>, ApiError> {
    let Json(body) = body.map_err(|_| ApiError::invalid(BAD_BODY))?;
    if body.version < 0 {
        return Err(ApiError::invalid(BAD_BODY));
    }
    let source_id = match &body.creator {
        Some(name) => {
            let anime_no = season_anime(&state, &id, season).await?;
            Some(source_of(&state, anime_no, name).await?.0)
        }
        None => {
            // Back to unknown needs no anime, but the season is the work's.
            state
                .seasons
                .store
                .anissia_link(&id, season)
                .await
                .map_err(refused_store)?;
            None
        }
    };
    match state
        .library
        .set_file_creator(
            &id,
            season,
            &body.path,
            body.version,
            source_id.as_deref(),
            state.anissia.now(),
        )
        .await
    {
        Ok(set) => Ok(Json(FileCreatorView::of(body.path, set))),
        Err(CreatorError::NoFile) => Err(ApiError::not_found(NO_FILE)),
        Err(CreatorError::Conflict) => {
            let current = state
                .library
                .file_creator(&id, season, &body.path)
                .await
                .map_err(internal)?
                .map(|set| FileCreatorView::of(body.path.clone(), set));
            Err(ApiError::Conflict {
                message: "다른 곳에서 먼저 이 자막의 제작자를 바꿨어요. 지금 상태를 확인하고 다시 골라 주세요."
                    .into(),
                current: current.and_then(|c| serde_json::to_value(c).ok()),
            })
        }
        Err(CreatorError::Db(e)) => Err(internal(e)),
    }
}
