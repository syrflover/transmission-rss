//! The episode mapping the user sets for a subtitle source of a season
//! (`docs/specs/library.md`, 자막의 회차 대응 and 자막 후보 구역;
//! [`trss_jobs::mapping`]).
//!
//! | method and path                                                                  | body                                                                    |
//! | -------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
//! | `PUT  /api/library/works/{id}/seasons/{n}/anissia/sources/{source}/mapping`        | `{ "version": 3, "offset": -12, "exceptions": [{ "episode": "13.5", "target": null }] }` |
//! | `POST /api/library/works/{id}/seasons/{n}/anissia/sources/{source}/mapping/revert` | `{ "version": 3 }`                                                      |
//! | `POST /api/library/works/{id}/seasons/{n}/anissia/sources/{source}/mapping/preview` | the dialog's input, see below                                          |
//!
//! `{source}` is the `source_id` of a creator of the season's linked anime (the
//! candidates' `source_id`), whether or not it is the subscribed creator. The
//! mapping (also each of the candidates' `mappings`):
//!
//! ```json
//! { "source_id": "6f0c…", "kind": "user", "offset": -12,
//!   "evidence": "사용자가 정했어요", "decided_at": 1790780400000, "version": 7,
//!   "exceptions": [{ "episode": "13.5", "target": null },
//!                  { "episode": "14", "target": 3 }],
//!   "line": "직접 정함 · 13화 → 1화 · 예외 13.5 받지 않음, 14 → 3화",
//!   "choice": "continue" }
//! ```
//!
//! `line` is the one line the creator's group shows for the mapping
//! ([`trss_library::mapping::Mapping::line`]) and `choice` the default
//! mapping the dialog opens on (`same`, `continue` or `custom`; `null` while
//! the mapping is undecided).
//!
//! - `version` is the mapping's version, what a save or a revert carries; a
//!   source the app has no mapping for is version 0 (it has no row). The
//!   answers below that hold a source's mapping are
//!   `{ "source_id": "…", "mapping": <the mapping> | null }`.
//! - `PUT` saves the default `offset` (an integer within ±9999) and the
//!   `exceptions` as one unit: the mapping becomes the user's (`kind` `user`),
//!   the offset the app had retired is cleared, and the exceptions are exactly
//!   those sent. An exception's `episode` is the text of Anissia's episode
//!   (compared by number, so `013`, `13` and `13.0` are one) and `target` the
//!   season episode (1–9999) it is, or `null` for 받지 않음. It answers the
//!   mapping saved. `400` says why a body cannot be saved (a repeated episode,
//!   an offset out of range, an empty episode); `404` that the season or the
//!   source is not there.
//! - `POST …/revert` (`자동으로 되돌리기`) deletes the user's mapping with its
//!   exceptions and the offset the app had retired, and asks the follower to
//!   look again, so the app decides again at once when the source is the
//!   subscribed creator's; the answer is the source's mapping then: `null` for
//!   a source the app does not decide, `undecided` for a subscribed creator it
//!   has no grounds for yet. A source whose stored mapping is not the user's
//!   (the app's, or none at version 0) answers `400`; only a version that is not
//!   the stored one answers `409`.
//! - A save or a revert made from a version that is not the stored one answers
//!   `409` with the stored mapping in `current` (as `{ "source_id", "mapping" }`),
//!   and changes nothing.
//! - After either, the follower looks at the subscribed creators again, so the
//!   episodes the new mapping lets through are received at once and the
//!   `회차 확인 필요` to-do reads the new state.
//! - `POST …/preview` changes nothing: it tells what the dialog shows for what
//!   is typed in it ([`trss_library::mapping::preview`]). The body is the
//!   dialog's input as it is held, and the answer what `저장` would send and
//!   what the dialog shows beside it:
//!
//!   ```json
//!   { "choice": "custom", "custom": "-12",
//!     "exceptions": [{ "episode": "13.5", "target": "", "skip": true }] }
//!   ```
//!
//!   ```json
//!   { "continue_reason": null,
//!     "previews": { "same": "13화 → 시즌 밖 · 14화 → 시즌 밖",
//!                   "continue": "13화 → 1화 · 14화 → 2화",
//!                   "custom": "13화 → 1화 · 14화 → 2화" },
//!     "offset": -12, "offset_problem": null,
//!     "exceptions": [{ "episode": "13.5", "target": null }],
//!     "exceptions_problem": null,
//!     "warnings": [], "to_add": [] }
//!   ```
//!
//!   `choice` is `same`, `continue`, `custom` or `null` (none made yet),
//!   `custom` the text in the offset box and each row the texts of an
//!   exception: the season episode as typed in `target`, and `skip` for
//!   `받지 않음`. In the answer `offset` and `exceptions` are what `저장` sends
//!   (`offset` is `null` while the choice has no usable offset, `exceptions`
//!   empty while a row cannot be saved), `continue_reason` says why `앞 시즌에
//!   이어 셈` cannot be chosen, each of `previews` is the text under that
//!   choice (empty when it has none), `offset_problem` and
//!   `exceptions_problem` are the sentences for input that cannot be saved,
//!   `warnings` the season episodes two or more episodes land on and `to_add`
//!   the episodes to offer as exceptions. The source's episodes, the earlier
//!   seasons' count and the season's own are the server's, not the body's.
//!   `400` says the body cannot be read or the season is not linked; `404`
//!   that the season or the source is not there.
//! - `POST …/preview` changes nothing: it tells what the dialog shows for what
//!   is typed in it ([`trss_library::mapping::preview`]). The body is the
//!   dialog's input as it is held, and the answer what `저장` would send and
//!   what the dialog shows beside it:
//!
//!   ```json
//!   { "choice": "custom", "custom": "-12",
//!     "exceptions": [{ "episode": "13.5", "target": "", "skip": true }] }
//!   ```
//!
//!   ```json
//!   { "continue_reason": null,
//!     "previews": { "same": "13화 → 시즌 밖 · 14화 → 시즌 밖",
//!                   "continue": "13화 → 1화 · 14화 → 2화",
//!                   "custom": "13화 → 1화 · 14화 → 2화" },
//!     "offset": -12, "offset_problem": null,
//!     "exceptions": [{ "episode": "13.5", "target": null }],
//!     "exceptions_problem": null,
//!     "warnings": [], "to_add": [] }
//!   ```
//!
//!   `choice` is `same`, `continue`, `custom` or `null` (none made yet),
//!   `custom` the text in the offset box and each row the texts of an
//!   exception: the season episode as typed in `target`, and `skip` for
//!   `받지 않음`. In the answer `offset` and `exceptions` are what `저장` sends
//!   (`offset` is `null` while the choice has no usable offset, `exceptions`
//!   empty while a row cannot be saved), `continue_reason` says why `앞 시즌에
//!   이어 셈` cannot be chosen, each of `previews` is the text under that
//!   choice (empty when it has none), `offset_problem` and
//!   `exceptions_problem` are the sentences for input that cannot be saved,
//!   `warnings` the season episodes two or more episodes land on and `to_add`
//!   the episodes to offer as exceptions. The source's episodes, the earlier
//!   seasons' count and the season's own are the server's, not the body's.
//!   `400` says the body cannot be read or the season is not linked; `404`
//!   that the season or the source is not there.

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    routing::{post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{jobs_api::follow_now, seasons_anissia_api::refused_store, ApiError, AppState};
use trss_collect::store::anissia::Candidate;
use trss_core::episode::stored_key;
use trss_jobs::{
    follow::SeasonFacts,
    mapping::{Exception, Mapping, Saved, UserMapping},
};
use trss_library::mapping::preview::{self, Choice, Ground, Input, Row};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/library/works/{id}/seasons/{season}/anissia/sources/{source}/mapping",
            put(save),
        )
        .route(
            "/library/works/{id}/seasons/{season}/anissia/sources/{source}/mapping/revert",
            post(revert),
        )
        .route(
            "/library/works/{id}/seasons/{season}/anissia/sources/{source}/mapping/preview",
            post(preview),
        )
}

/// One exception of a user's mapping.
#[derive(Debug, Serialize)]
pub struct ExceptionView {
    episode: String,
    target: Option<u32>,
}

impl From<&Exception> for ExceptionView {
    fn from(e: &Exception) -> Self {
        ExceptionView {
            episode: e.episode.clone(),
            target: e.target,
        }
    }
}

/// What the library knows of a source and the season it is mapped to: what a
/// mapping's line and the dialog's opening choice are written from.
#[derive(Debug, Clone)]
pub struct SourceFacts {
    /// Anissia's episode texts of the source, one for each episode: the
    /// newest observation's.
    pub episodes: Vec<String>,
    /// The episodes of all earlier seasons together, when each is known.
    pub previous: Option<u32>,
    /// The season's episode count, when it is known.
    pub total: Option<u32>,
}

impl SourceFacts {
    /// The facts of the source `source_id`, from the observed `candidates` of
    /// its anime.
    pub fn of(candidates: &[Candidate], source_id: &str, season: SeasonFacts) -> SourceFacts {
        let mut newest: HashMap<String, &Candidate> = HashMap::new();
        for c in candidates.iter().filter(|c| c.source_id == source_id) {
            let seen = newest.entry(stored_key(&c.episode)).or_insert(c);
            if c.id > seen.id {
                *seen = c;
            }
        }
        SourceFacts {
            episodes: newest.into_values().map(|c| c.episode.clone()).collect(),
            previous: season.previous,
            total: season.total,
        }
    }
}

/// A source's episode mapping to the season (see the module docs).
#[derive(Debug, Serialize)]
pub struct MappingView {
    pub source_id: String,
    /// `auto`, `undecided` or `user`.
    pub kind: &'static str,
    pub offset: Option<i64>,
    pub evidence: String,
    pub decided_at: i64,
    pub version: i64,
    pub exceptions: Vec<ExceptionView>,
    /// The one line the creator's group shows for the mapping.
    pub line: String,
    /// The default mapping the dialog opens on: `same`, `continue` or
    /// `custom`; `None` while the mapping is undecided.
    pub choice: Option<&'static str>,
}

impl MappingView {
    pub fn of(source_id: String, m: Mapping, facts: &SourceFacts) -> MappingView {
        MappingView {
            source_id,
            kind: m.kind.code(),
            offset: m.offset,
            evidence: m.evidence.clone(),
            decided_at: m.decided_at,
            version: m.version,
            exceptions: m.exceptions.iter().map(ExceptionView::from).collect(),
            line: m.line(&facts.episodes, facts.total),
            choice: preview::choice_of(m.decided_offset(), facts.previous).map(Choice::code),
        }
    }
}

/// What the answers that hold a source's mapping carry.
#[derive(Debug, Serialize)]
struct SourceMapping {
    source_id: String,
    mapping: Option<MappingView>,
}

impl SourceMapping {
    fn of(source_id: &str, mapping: Option<Mapping>, facts: &SourceFacts) -> SourceMapping {
        SourceMapping {
            source_id: source_id.to_owned(),
            mapping: mapping.map(|m| MappingView::of(source_id.to_owned(), m, facts)),
        }
    }
}

const BAD_BODY: &str = "요청 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";
const NO_SOURCE: &str = "이 제작자를 찾지 못했어요. 자막 후보를 새로고침해 주세요.";
const STALE: &str =
    "다른 곳에서 먼저 이 제작자의 회차 대응을 바꿨어요. 지금 대응을 확인하고 다시 정해 주세요.";

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

/// Checks that the season is linked and the source is a creator of its anime,
/// and tells the anime.
async fn check(
    state: &AppState,
    work_id: &str,
    season: u32,
    source_id: &str,
) -> Result<i64, ApiError> {
    let link = state
        .seasons
        .store
        .anissia_link(work_id, season)
        .await
        .map_err(refused_store)?;
    let Some(anime_no) = link.anime_no else {
        return Err(ApiError::invalid(
            "이 시즌에 Anissia 작품을 먼저 연결해 주세요. 연결한 작품의 제작자에게 회차 대응을 정할 수 있어요.",
        ));
    };
    match state
        .follow
        .source_anime(source_id)
        .await
        .map_err(internal)?
    {
        Some(source_anime) if source_anime == anime_no => Ok(anime_no),
        _ => Err(ApiError::not_found(NO_SOURCE)),
    }
}

/// What the library knows of the source and its season ([`SourceFacts`]).
async fn facts(
    state: &AppState,
    work_id: &str,
    season: u32,
    anime_no: i64,
    source_id: &str,
) -> Result<SourceFacts, ApiError> {
    let observed = state
        .anissia_store
        .candidates(anime_no, Vec::new())
        .await
        .map_err(internal)?;
    let season_facts = state
        .follow
        .season_facts(work_id, season)
        .await
        .map_err(internal)?;
    Ok(SourceFacts::of(&observed, source_id, season_facts))
}

fn stale(source_id: &str, current: Option<Mapping>, facts: &SourceFacts) -> ApiError {
    ApiError::Conflict {
        message: STALE.into(),
        current: serde_json::to_value(SourceMapping::of(source_id, current, facts)).ok(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExceptionBody {
    episode: String,
    target: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveBody {
    version: i64,
    offset: i64,
    exceptions: Vec<ExceptionBody>,
}

async fn save(
    State(state): State<AppState>,
    Path((id, season, source_id)): Path<(String, u32, String)>,
    body: Result<Json<SaveBody>, JsonRejection>,
) -> Result<Json<MappingView>, ApiError> {
    let Json(body) = body.map_err(|_| ApiError::invalid(BAD_BODY))?;
    if body.version < 0 {
        return Err(ApiError::invalid(BAD_BODY));
    }
    let anime_no = check(&state, &id, season, &source_id).await?;
    let user = UserMapping::new(
        body.offset,
        body.exceptions
            .into_iter()
            .map(|e| (e.episode, e.target))
            .collect(),
    )
    .map_err(|e| ApiError::invalid(e.to_string()))?;
    match state
        .follow
        .set_user_mapping(
            &id,
            season,
            &source_id,
            body.version,
            user,
            state.anissia.now(),
        )
        .await
        .map_err(internal)?
    {
        Saved::Done(Some(mapping)) => {
            // The jobs whose rows the change moved are back in line
            // ([`trss_jobs::place::relocate`]).
            if let Some(path) = &state.worker_wake {
                trss_core::wake::wake_worker(path);
            }
            // The follower reads the new mapping: what it lets through is
            // received now, and the conflicts are found against it.
            follow_now(&state).await;
            let facts = facts(&state, &id, season, anime_no, &source_id).await?;
            Ok(Json(MappingView::of(source_id, mapping, &facts)))
        }
        Saved::Done(None) => Err(internal("the mapping saved is not there")),
        Saved::Stale(current) | Saved::NotTheUsers(current) => {
            let facts = facts(&state, &id, season, anime_no, &source_id).await?;
            Err(stale(&source_id, current, &facts))
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevertBody {
    version: i64,
}

async fn revert(
    State(state): State<AppState>,
    Path((id, season, source_id)): Path<(String, u32, String)>,
    body: Result<Json<RevertBody>, JsonRejection>,
) -> Result<Json<SourceMapping>, ApiError> {
    let Json(body) = body.map_err(|_| ApiError::invalid(BAD_BODY))?;
    if body.version < 0 {
        return Err(ApiError::invalid(BAD_BODY));
    }
    let anime_no = check(&state, &id, season, &source_id).await?;
    match state
        .follow
        .revert_mapping(&id, season, &source_id, body.version)
        .await
        .map_err(internal)?
    {
        Saved::Done(_) => {
            // The app decides again at once for the subscribed creator's source.
            follow_now(&state).await;
            let mapping = state
                .follow
                .mappings(&id, season)
                .await
                .map_err(internal)?
                .remove(&source_id);
            let facts = facts(&state, &id, season, anime_no, &source_id).await?;
            Ok(Json(SourceMapping::of(&source_id, mapping, &facts)))
        }
        // The version is the stored one, so this is no stale screen: there is
        // no user's mapping to give back.
        Saved::NotTheUsers(None) => Err(ApiError::invalid(
            "되돌릴 직접 정한 대응이 없어요. 이 제작자는 아직 대응이 정해지지 않았어요.",
        )),
        Saved::NotTheUsers(Some(_)) => Err(ApiError::invalid(
            "직접 정한 대응만 자동으로 되돌릴 수 있어요. 이 대응은 앱이 정한 것이에요.",
        )),
        Saved::Stale(current) => {
            let facts = facts(&state, &id, season, anime_no, &source_id).await?;
            Err(stale(&source_id, current, &facts))
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ChoiceBody {
    Same,
    Continue,
    Custom,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RowBody {
    episode: String,
    target: String,
    skip: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewBody {
    choice: Option<ChoiceBody>,
    custom: String,
    exceptions: Vec<RowBody>,
}

/// The texts under each choice of the dialog.
#[derive(Debug, Serialize)]
struct Previews {
    same: String,
    #[serde(rename = "continue")]
    continued: String,
    custom: String,
}

/// What the dialog shows for what is typed in it (see the module docs).
#[derive(Debug, Serialize)]
struct PreviewView {
    continue_reason: Option<&'static str>,
    previews: Previews,
    offset: Option<i64>,
    offset_problem: Option<String>,
    exceptions: Vec<ExceptionView>,
    exceptions_problem: Option<String>,
    warnings: Vec<String>,
    to_add: Vec<String>,
}

async fn preview(
    State(state): State<AppState>,
    Path((id, season, source_id)): Path<(String, u32, String)>,
    body: Result<Json<PreviewBody>, JsonRejection>,
) -> Result<Json<PreviewView>, ApiError> {
    let Json(body) = body.map_err(|_| ApiError::invalid(BAD_BODY))?;
    let anime_no = check(&state, &id, season, &source_id).await?;
    let facts = facts(&state, &id, season, anime_no, &source_id).await?;
    let input = Input {
        choice: body.choice.map(|choice| match choice {
            ChoiceBody::Same => Choice::Same,
            ChoiceBody::Continue => Choice::Continue,
            ChoiceBody::Custom => Choice::Custom,
        }),
        custom: body.custom,
        rows: body
            .exceptions
            .into_iter()
            .map(|row| Row {
                episode: row.episode,
                target: row.target,
                skip: row.skip,
            })
            .collect(),
    };
    let shown = preview::shown(
        &input,
        &Ground {
            episodes: &facts.episodes,
            previous: facts.previous,
            total: facts.total,
        },
    );
    Ok(Json(PreviewView {
        continue_reason: shown.continue_reason,
        previews: Previews {
            same: shown.same,
            continued: shown.continued,
            custom: shown.custom,
        },
        offset: shown.offset,
        offset_problem: shown.offset_problem,
        exceptions: shown.exceptions.iter().map(ExceptionView::from).collect(),
        exceptions_problem: shown.exceptions_problem,
        warnings: shown.warnings,
        to_add: shown.to_add,
    }))
}

#[cfg(test)]
mod tests;
