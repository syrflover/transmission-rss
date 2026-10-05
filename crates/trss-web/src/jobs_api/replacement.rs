//! A job's replacements (`docs/specs/subtitles.md`, 교체 비교와 승인): what
//! its detail shows of each row's latest plan, and the person's decision on
//! one (see the module docs of [`super`]).

use axum::{
    extract::{Path, State},
    http::{header::CONTENT_TYPE, HeaderValue},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use trss_jobs::{
    model::{PathAction, PlanState},
    place::replace::{
        records::{
            Compared, Comparison, Decided, Decision, Diff, Encoding, NotCompared, PlanPath,
            PlanView, Side, StoredFacts,
        },
        NEW_REVISION,
    },
};

use super::{internal, now_millis, ApiError, AppState};

/// One subtitle of a decision card's version lines: `현재` or `새 자막`.
#[derive(Debug, Serialize)]
pub(super) struct VersionView {
    /// When the app received it (its post's receipt, else when it was
    /// stored); `null` for a file the app did not manage.
    received_at: Option<i64>,
    /// The file's change time, for a file the app did not manage.
    changed_at: Option<i64>,
    size: u64,
    /// Its dialogue lines, `null` when the app cannot count them.
    lines: Option<u64>,
    creator: Option<String>,
    format: Option<&'static str>,
    /// The post it came from.
    post: Option<String>,
    encoding: Option<String>,
    /// Whether the app manages it: a stored subtitle, or a copy the app
    /// applied with the bytes it applied.
    managed: bool,
    /// Its server path: the file beside the video for `current`, the stored
    /// file for `new`.
    path: String,
    /// The stored file of the current subtitle, which stays whatever the
    /// person decides.
    stored: Option<String>,
}

/// What the plan does to one path beside the video.
#[derive(Debug, Serialize)]
pub(super) struct PathView {
    path: String,
    /// `add`, `replace`, `remove` or `keep`.
    action: &'static str,
    managed: bool,
    /// A change the person may not expect, shown with its exact path:
    /// `overwrite_unmanaged` (a file the app did not manage is replaced; its
    /// bytes are kept as the unknown creator's stored subtitle) or
    /// `remove_applied` (an earlier applied copy at another path goes; its
    /// stored file stays).
    warning: Option<&'static str>,
}

/// Why the version before this one went stale.
#[derive(Debug, Serialize)]
pub(super) struct AgainView {
    reason: String,
    /// A newer revision from the same source came (`새 수정본 발견`).
    new_revision: bool,
}

/// What differs between the current subtitle and the new one, as the plan
/// was made with it. Without the dialogue and timing lines: the detail is
/// polled, and a whole file's change is large (`lines`).
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub(super) enum ComparisonView {
    /// The contents were not compared: why. Never shown as no difference.
    Unreadable {
        reason: String,
    },
    Compared(Box<ComparedView>),
}

/// The counts of a difference.
#[derive(Debug, Serialize)]
pub(super) struct ComparedView {
    current: SideView,
    new: SideView,
    dialogue: DialogueView,
    timing: TimingView,
    /// `null` when the styles cannot be compared (`not_compared`).
    styles: Option<StylesView>,
    /// `null` when the fonts cannot be compared (`not_compared`).
    fonts: Option<FontsView>,
    /// What was not compared between two readable files, and why.
    not_compared: Vec<NotCompared>,
}

/// What was read of one subtitle.
#[derive(Debug, Serialize)]
pub(super) struct SideView {
    /// `ASS`, `SRT`, `WebVTT` or `SMI`.
    format: &'static str,
    /// `UTF-8`, `UTF-16` or `CP949`.
    encoding: Encoding,
    cues: u64,
}

#[derive(Debug, Serialize)]
pub(super) struct DialogueView {
    added: u64,
    changed: u64,
    removed: u64,
}

#[derive(Debug, Serialize)]
pub(super) struct TimingView {
    count: u64,
}

#[derive(Debug, Serialize)]
pub(super) struct StylesView {
    added: Vec<String>,
    removed: Vec<String>,
    changed: Vec<StyleChangeView>,
}

#[derive(Debug, Serialize)]
pub(super) struct StyleChangeView {
    name: String,
    fields: Vec<FieldChangeView>,
}

#[derive(Debug, Serialize)]
pub(super) struct FieldChangeView {
    field: String,
    old: String,
    new: String,
}

#[derive(Debug, Serialize)]
pub(super) struct FontsView {
    added: Vec<String>,
    removed: Vec<String>,
}

impl ComparisonView {
    fn of(comparison: &Comparison) -> Self {
        match &comparison.result {
            Compared::Unreadable(reason) => Self::Unreadable {
                reason: reason.clone(),
            },
            Compared::Diff(diff) => Self::of_diff(diff),
        }
    }

    fn of_diff(diff: &Diff) -> Self {
        let side = |s: &Side| SideView {
            format: s.format.label(),
            encoding: s.encoding,
            cues: s.cues,
        };
        Self::Compared(Box::new(ComparedView {
            current: side(&diff.old),
            new: side(&diff.new),
            dialogue: DialogueView {
                added: diff.dialogue.added,
                changed: diff.dialogue.changed,
                removed: diff.dialogue.removed,
            },
            timing: TimingView {
                count: diff.timing.count,
            },
            styles: diff.styles.as_ref().map(|s| StylesView {
                added: s.added.clone(),
                removed: s.removed.clone(),
                changed: s
                    .changed
                    .iter()
                    .map(|c| StyleChangeView {
                        name: c.name.clone(),
                        fields: c
                            .fields
                            .iter()
                            .map(|f| FieldChangeView {
                                field: f.field.clone(),
                                old: f.old.clone(),
                                new: f.new.clone(),
                            })
                            .collect(),
                    })
                    .collect(),
            }),
            fonts: diff.fonts.as_ref().map(|f| FontsView {
                added: f.added.clone(),
                removed: f.removed.clone(),
            }),
            not_compared: diff.not_compared.clone(),
        }))
    }
}

/// A row's latest replacement plan.
#[derive(Debug, Serialize)]
pub(super) struct ReplacementView {
    /// The plan and its version, which a decision names: never shown.
    plan_id: String,
    version: i64,
    position: i64,
    /// `open` (to decide), `approved`, `kept`, `done`, `stale`, `held` or
    /// `failed`.
    state: &'static str,
    /// Why it is `stale`, `held` or `failed`.
    reason: Option<String>,
    /// For `stale`: whether a newer revision from the same source came.
    new_revision: bool,
    season: u32,
    episode: i64,
    /// Set when the version before went stale (`다시 비교 필요`).
    again: Option<AgainView>,
    /// The subtitle the episode has: the file the new copy replaces, else
    /// the applied copy it removes, else a file it keeps beside it.
    current: Option<VersionView>,
    new: Option<VersionView>,
    /// Whether the two files' facts are shown side by side: the creator,
    /// format or post differs, or the current file's source is not known.
    side_by_side: bool,
    /// What differs between `current` and `new`: `null` for a plan made
    /// before the app compared contents.
    comparison: Option<ComparisonView>,
    paths: Vec<PathView>,
    /// The limits of the comparison that hold: `unknown_source` (the
    /// current file is not the app's), `lines_unknown` (a file's lines
    /// could not be counted).
    limits: Vec<&'static str>,
}

/// The views of the job's latest plans.
pub(super) async fn views(
    state: &AppState,
    job_id: &str,
) -> Result<Vec<ReplacementView>, ApiError> {
    let plans = state
        .jobs
        .replacements(job_id)
        .await
        .map_err(|e| internal(&e))?;
    Ok(plans.iter().map(view).collect())
}

fn view(v: &PlanView) -> ReplacementView {
    let plan = &v.plan;
    let full = |relative: &str| format!("{}/{relative}", plan.folder);
    let facts_of = |path: &PlanPath| {
        v.applied
            .iter()
            .find(|(p, _)| *p == path.path)
            .map(|(_, facts)| facts)
    };
    let current = plan.current().map(|(path, file)| {
        let facts = facts_of(path);
        VersionView {
            received_at: facts.map(|f| f.received_at),
            changed_at: facts.is_none().then_some(file.mtime / 1_000_000),
            size: file.size,
            lines: file.lines,
            creator: facts.and_then(|f| f.creator.clone()),
            format: facts
                .map(|f| f.format.code())
                .or_else(|| format_of(&path.path)),
            post: facts.and_then(|f| f.post.clone()),
            encoding: facts.and_then(|f| f.encoding.clone()),
            managed: path.applied_id.is_some(),
            path: full(&path.path),
            stored: facts.map(|f| full(&f.asset_path)),
        }
    });
    let new = v.new.as_ref().map(|f: &StoredFacts| VersionView {
        received_at: Some(f.received_at),
        changed_at: None,
        size: plan.asset_size,
        lines: plan.asset_lines,
        creator: f.creator.clone(),
        format: Some(f.format.code()),
        post: f.post.clone(),
        encoding: f.encoding.clone(),
        managed: true,
        path: full(&plan.asset_path),
        stored: None,
    });
    let side_by_side = match (&current, &new) {
        (Some(c), Some(n)) => {
            !c.managed || c.creator != n.creator || c.format != n.format || c.post != n.post
        }
        _ => false,
    };
    let mut limits = Vec::new();
    if current.as_ref().is_some_and(|c| !c.managed) {
        limits.push("unknown_source");
    }
    let uncounted = |v: &Option<VersionView>| v.as_ref().is_some_and(|v| v.lines.is_none());
    if uncounted(&current) || uncounted(&new) {
        limits.push("lines_unknown");
    }
    ReplacementView {
        plan_id: plan.id.clone(),
        version: plan.version,
        position: plan.position,
        state: plan.state.code(),
        reason: plan.reason.clone(),
        new_revision: plan.state == PlanState::Stale
            && plan.reason.as_deref() == Some(NEW_REVISION),
        season: plan.season,
        episode: plan.episode,
        again: v.previous.as_ref().map(|reason| AgainView {
            new_revision: reason == NEW_REVISION,
            reason: reason.clone(),
        }),
        current,
        new,
        side_by_side,
        comparison: v.comparison.as_ref().map(ComparisonView::of),
        paths: plan
            .paths
            .iter()
            .map(|p| PathView {
                path: full(&p.path),
                action: p.action.code(),
                managed: p.applied_id.is_some(),
                warning: match (p.action, p.applied_id.is_some()) {
                    (PathAction::Replace, false) => Some("overwrite_unmanaged"),
                    (PathAction::Remove, _) => Some("remove_applied"),
                    _ => None,
                },
            })
            .collect(),
        limits,
    }
}

/// The format a file's extension says.
fn format_of(path: &str) -> Option<&'static str> {
    let (_, ext) = path.rsplit_once('.')?;
    Some(match ext.to_ascii_lowercase().as_str() {
        "ass" | "ssa" => "ass",
        "srt" => "srt",
        "smi" | "sami" => "smi",
        _ => "other",
    })
}

/// `GET /api/subtitle-jobs/{id}/replacements/{plan}/lines`: the dialogue and
/// timing lines of the difference the plan was made with, as the comparison
/// engine writes them (times in milliseconds). `404` when the plan is not
/// the job's or its contents were not compared. The stored text is sent
/// as it is: a rewritten file's lines are large, and the web's memory is
/// small.
pub(super) async fn lines(
    State(state): State<AppState>,
    Path((id, plan)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    match state
        .jobs
        .replacement_lines(&id, &plan)
        .await
        .map_err(|e| internal(&e))?
    {
        Some(json) => Ok((
            [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
            json,
        )
            .into_response()),
        None => Err(ApiError::not_found("비교한 줄을 찾지 못했어요.")),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct DecideRequest {
    version: i64,
    /// `replace` (`새 자막으로 교체`) or `keep` (`현재 유지`).
    decision: String,
}

/// `POST /api/subtitle-jobs/{id}/replacements/{plan}`: the person's decision
/// on the plan of `version`.
pub(super) async fn decide(
    State(state): State<AppState>,
    Path((id, plan)): Path<(String, String)>,
    Json(request): Json<DecideRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let replace = match request.decision.as_str() {
        "replace" => true,
        "keep" => false,
        _ => return Err(ApiError::invalid("결정은 replace나 keep이어야 해요.")),
    };
    let decided = state
        .jobs
        .decide_replacement(&id, &plan, request.version, replace, now_millis())
        .await
        .map_err(|e| internal(&e))?;
    match decided {
        Decided::Done(to) => {
            if let Some(path) = &state.worker_wake {
                trss_core::wake::wake_worker(path);
            }
            Ok(Json(json!({ "state": to.code() })))
        }
        Decided::NotFound => Err(ApiError::not_found("교체 계획을 찾지 못했어요.")),
        Decided::Stale => Err(ApiError::Conflict {
            message: "다시 비교가 필요해요. 화면을 새로고침해 주세요.".to_owned(),
            current: None,
        }),
    }
}

/// The most decisions one request carries: a season's episodes with room.
const DECIDE_MAX: usize = 1000;

#[derive(Debug, Deserialize)]
pub(super) struct DecideManyRequest {
    decisions: Vec<DecideManyItem>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DecideManyItem {
    plan: String,
    version: i64,
    /// `replace` or `keep`, as [`DecideRequest::decision`].
    decision: String,
}

/// `POST /api/subtitle-jobs/{id}/replacements`: the person's decisions on
/// several plans of the job at once (`모두 교체`, `모두 유지`, a list's
/// picks), each as [`decide`] takes one. A plan that is not the plan to
/// decide any more is `stale` in the results and left as it is, the others
/// are written. A plan of another job fails the whole request, with nothing
/// written.
pub(super) async fn decide_many(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<DecideManyRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if request.decisions.is_empty() {
        return Err(ApiError::invalid("결정할 교체 계획이 없어요."));
    }
    if request.decisions.len() > DECIDE_MAX {
        return Err(ApiError::invalid(format!(
            "한 번에 {DECIDE_MAX}개까지만 결정할 수 있어요."
        )));
    }
    let mut seen = std::collections::HashSet::new();
    let mut decisions = Vec::with_capacity(request.decisions.len());
    for item in &request.decisions {
        if !seen.insert(item.plan.as_str()) {
            return Err(ApiError::invalid(
                "같은 교체 계획을 두 번 결정할 수 없어요.",
            ));
        }
        let replace = match item.decision.as_str() {
            "replace" => true,
            "keep" => false,
            _ => return Err(ApiError::invalid("결정은 replace나 keep이어야 해요.")),
        };
        decisions.push(Decision {
            plan_id: item.plan.clone(),
            version: item.version,
            replace,
        });
    }
    let decided = state
        .jobs
        .decide_replacements(&id, decisions, now_millis())
        .await
        .map_err(|e| internal(&e))?;
    if decided.contains(&Decided::NotFound) {
        return Err(ApiError::not_found("교체 계획을 찾지 못했어요."));
    }
    if decided.iter().any(|d| matches!(d, Decided::Done(_))) {
        if let Some(path) = &state.worker_wake {
            trss_core::wake::wake_worker(path);
        }
    }
    let results: Vec<_> = request
        .decisions
        .iter()
        .zip(&decided)
        .map(|(item, decided)| {
            let state = match decided {
                Decided::Done(to) => to.code(),
                _ => "stale",
            };
            json!({ "plan": item.plan, "state": state })
        })
        .collect();
    Ok(Json(json!({ "results": results })))
}

#[cfg(test)]
mod tests;
