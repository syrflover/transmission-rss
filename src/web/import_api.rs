//! Importing the legacy channels YAML, in two steps.
//!
//! Both endpoints take the file text in the request body and keep no state on
//! the server:
//!
//! - `POST /api/import/legacy/preview` `{ content }` parses the file and
//!   compares it to the channels that exist. Nothing is changed.
//! - `POST /api/import/legacy/apply` `{ content, choices }` parses the file
//!   again, checks that `choices` cover exactly the channels that conflict
//!   *now* (with the versions the user reviewed), and applies everything in one
//!   transaction.
//!
//! Sending the file again with the apply, instead of holding a preview on the
//! server, means the apply never trusts data the client got back from the
//! preview: the masked URLs it displays cannot be applied, and the real URLs
//! only ever travel from the browser to the server. No response contains a
//! secret value. A review that has gone stale (a channel changed, appeared or
//! disappeared since the preview) is answered `409` with nothing applied, so
//! the screen can review the file again.

use axum::{
    extract::{rejection::JsonRejection, State},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::{ApiError, AppState};
use crate::import::{
    legacy,
    plan::{build_actions, display_url, find_existing, Choice, Decision},
};
use crate::store::channels::import::{match_rules, ImportChannel, ImportedChannel};
use crate::store::channels::{ChannelError, ChannelWithRules, Rule, Version};

const STALE_MESSAGE: &str = "검토한 뒤에 채널이 바뀌었어요. 파일을 다시 검토한 다음 선택해 주세요.";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/import/legacy/preview", post(preview))
        .route("/import/legacy/apply", post(apply))
}

#[derive(Deserialize)]
struct PreviewRequest {
    content: String,
}

#[derive(Deserialize)]
struct ApplyRequest {
    content: String,
    #[serde(default)]
    choices: Vec<ChoiceRequest>,
}

#[derive(Deserialize)]
struct ChoiceRequest {
    /// Position of the channel in the file, as the preview numbered it.
    index: usize,
    existing_id: String,
    existing_version: Version,
    decision: DecisionRequest,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum DecisionRequest {
    Replace,
    Add,
    Skip,
}

/// A rule as the review lists it.
#[derive(Serialize)]
struct RuleView {
    /// The match phrase; `null` is a rule waiting for its title.
    r#match: Option<String>,
    regex: bool,
    case_insensitive: bool,
    directory: String,
    episode: i64,
    /// True when the phrase is a regex that cannot be compiled: the rule is
    /// imported as written but matches no title.
    invalid_regex: bool,
    /// True when replacing keeps the ID of an existing rule for this one.
    /// Only ever true on a channel that already exists.
    keeps_existing_rule: bool,
}

/// An existing rule that a replacement would delete.
#[derive(Serialize)]
struct RemovedRule {
    r#match: Option<String>,
    directory: String,
}

impl From<&Rule> for RemovedRule {
    fn from(rule: &Rule) -> Self {
        RemovedRule {
            r#match: rule.r#match.clone(),
            directory: rule.directory.clone(),
        }
    }
}

/// The channel that already exists and is the same as a file channel.
#[derive(Serialize)]
struct ExistingView {
    id: String,
    /// Send back with a `replace` choice so a changed channel is refused.
    version: Version,
    /// Masked.
    url: String,
    rule_count: usize,
    /// What a replacement would delete, in the channel's current order.
    removed_rules: Vec<RemovedRule>,
}

#[derive(Serialize)]
struct ChannelView {
    index: usize,
    /// Masked: every query value is `***`.
    url: String,
    directory: String,
    excludes: Vec<String>,
    rules: Vec<RuleView>,
    existing: Option<ExistingView>,
}

#[derive(Serialize)]
struct PreviewResponse {
    channels: Vec<ChannelView>,
    /// How many file channels need a choice.
    conflict_count: usize,
}

fn invalid_regex(rule: &crate::store::channels::RuleInput) -> bool {
    rule.regex
        && rule.r#match.as_deref().is_some_and(|pattern| {
            regex::RegexBuilder::new(pattern)
                .case_insensitive(rule.case_insensitive)
                .build()
                .is_err()
        })
}

fn channel_view(
    index: usize,
    channel: &ImportChannel,
    existing: Option<&ChannelWithRules>,
) -> ChannelView {
    let kept: Vec<Option<usize>> = existing
        .map(|e| match_rules(&e.rules, &channel.rules))
        .unwrap_or_else(|| vec![None; channel.rules.len()]);
    let removed_rules = existing
        .map(|e| {
            e.rules
                .iter()
                .enumerate()
                .filter(|(i, _)| !kept.contains(&Some(*i)))
                .map(|(_, rule)| RemovedRule::from(rule))
                .collect()
        })
        .unwrap_or_default();

    ChannelView {
        index,
        url: display_url(&channel.input.url),
        directory: channel.input.base_dir.clone(),
        excludes: channel.input.excludes.clone(),
        rules: channel
            .rules
            .iter()
            .zip(&kept)
            .map(|(rule, kept)| RuleView {
                r#match: rule.r#match.clone(),
                regex: rule.regex,
                case_insensitive: rule.case_insensitive,
                directory: rule.directory.clone(),
                episode: rule.episode,
                invalid_regex: invalid_regex(rule),
                keeps_existing_rule: kept.is_some(),
            })
            .collect(),
        existing: existing.map(|e| ExistingView {
            id: e.channel.id.clone(),
            version: e.channel.version,
            url: display_url(&e.channel.url),
            rule_count: e.rules.len(),
            removed_rules,
        }),
    }
}

fn json<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    match body {
        Ok(Json(body)) => Ok(body),
        Err(JsonRejection::BytesRejection(_)) => Err(ApiError::invalid(
            "요청이 너무 크거나 읽을 수 없어요. 파일이 2MB 이하인지 확인해 주세요.",
        )),
        Err(_) => Err(ApiError::invalid(
            "요청을 읽을 수 없어요. 화면을 새로 고친 다음 다시 시도해 주세요.",
        )),
    }
}

fn store_error(e: ChannelError) -> ApiError {
    match e {
        ChannelError::Conflict { .. }
        | ChannelError::OrderMismatch { .. }
        | ChannelError::NotFound { .. } => ApiError::Conflict {
            message: STALE_MESSAGE.into(),
            current: None,
        },
        ChannelError::Invalid(_) => {
            ApiError::invalid("저장할 수 없는 값이 있어요. 파일의 채널과 규칙 값을 확인해 주세요.")
        }
        other => other.into(),
    }
}

async fn preview(
    State(state): State<AppState>,
    body: Result<Json<PreviewRequest>, JsonRejection>,
) -> Result<Json<PreviewResponse>, ApiError> {
    let request = json(body)?;
    let file = legacy::parse(&request.content).map_err(|e| ApiError::invalid(e.message()))?;
    let existing = state
        .channels
        .list_channels_with_rules()
        .await
        .map_err(store_error)?;

    let found = find_existing(&file, &existing);
    let channels: Vec<_> = file
        .iter()
        .enumerate()
        .map(|(i, channel)| channel_view(i, channel, found[i].map(|e| &existing[e])))
        .collect();
    Ok(Json(PreviewResponse {
        conflict_count: found.iter().flatten().count(),
        channels,
    }))
}

#[derive(Serialize)]
struct AddedView {
    index: usize,
    id: String,
    url: String,
    rule_count: usize,
}

#[derive(Serialize)]
struct ReplacedView {
    index: usize,
    id: String,
    url: String,
    rule_count: usize,
    /// Rules of the file that took over an existing rule's ID.
    kept_rules: usize,
    /// Rules of the file that got a new ID.
    added_rules: usize,
    removed_rules: Vec<RemovedRule>,
}

#[derive(Serialize)]
struct SkippedView {
    index: usize,
    url: String,
}

#[derive(Serialize)]
struct Counts {
    channels_added: usize,
    channels_replaced: usize,
    channels_skipped: usize,
    /// Existing channels this import did not change.
    channels_unchanged: usize,
    /// Rules created, in added channels and as new rules of replaced ones.
    rules_added: usize,
    rules_kept: usize,
    rules_removed: usize,
}

#[derive(Serialize)]
struct ApplyResponse {
    added: Vec<AddedView>,
    replaced: Vec<ReplacedView>,
    skipped: Vec<SkippedView>,
    counts: Counts,
}

async fn apply(
    State(state): State<AppState>,
    body: Result<Json<ApplyRequest>, JsonRejection>,
) -> Result<Json<ApplyResponse>, ApiError> {
    let request = json(body)?;
    let file = legacy::parse(&request.content).map_err(|e| ApiError::invalid(e.message()))?;
    let existing = state
        .channels
        .list_channels_with_rules()
        .await
        .map_err(store_error)?;

    let choices: Vec<Choice> = request
        .choices
        .into_iter()
        .map(|c| Choice {
            index: c.index,
            existing_id: c.existing_id,
            existing_version: c.existing_version,
            decision: match c.decision {
                DecisionRequest::Replace => Decision::Replace,
                DecisionRequest::Add => Decision::Add,
                DecisionRequest::Skip => Decision::Skip,
            },
        })
        .collect();
    let urls: Vec<String> = file.iter().map(|c| display_url(&c.input.url)).collect();
    let plan = build_actions(file, &existing, &choices).map_err(|_| ApiError::Conflict {
        message: STALE_MESSAGE.into(),
        current: None,
    })?;

    let (indexes, actions): (Vec<usize>, Vec<_>) = plan.actions.into_iter().unzip();
    let results = state
        .channels
        .import_channels(actions)
        .await
        .map_err(store_error)?;

    let mut response = ApplyResponse {
        added: Vec::new(),
        replaced: Vec::new(),
        skipped: plan
            .skipped
            .iter()
            .map(|&index| SkippedView {
                index,
                url: urls[index].clone(),
            })
            .collect(),
        counts: Counts {
            channels_added: 0,
            channels_replaced: 0,
            channels_skipped: plan.skipped.len(),
            channels_unchanged: 0,
            rules_added: 0,
            rules_kept: 0,
            rules_removed: 0,
        },
    };
    for (index, result) in indexes.into_iter().zip(results) {
        let stored = result.channel();
        let (id, url, rule_count) = (
            stored.channel.id.clone(),
            display_url(&stored.channel.url),
            stored.rules.len(),
        );
        match result {
            ImportedChannel::Added(_) => {
                response.counts.rules_added += rule_count;
                response.added.push(AddedView {
                    index,
                    id,
                    url,
                    rule_count,
                });
            }
            ImportedChannel::Replaced {
                kept_rules,
                removed_rules,
                ..
            } => {
                response.counts.rules_added += rule_count - kept_rules;
                response.counts.rules_kept += kept_rules;
                response.counts.rules_removed += removed_rules.len();
                response.replaced.push(ReplacedView {
                    index,
                    id,
                    url,
                    rule_count,
                    kept_rules,
                    added_rules: rule_count - kept_rules,
                    removed_rules: removed_rules.iter().map(RemovedRule::from).collect(),
                });
            }
        }
    }
    response.counts.channels_added = response.added.len();
    response.counts.channels_replaced = response.replaced.len();
    response.counts.channels_unchanged = existing.len() - response.replaced.len();
    Ok(Json(response))
}

#[cfg(test)]
mod tests;
