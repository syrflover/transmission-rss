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
//!
//! # Subscription suggestions
//!
//! The comment above a rule can name an Anissia anime and a subtitle creator;
//! the preview offers a subscription for it (`suggestion` on every rule) and
//! the apply takes the ones the user checked (`subscriptions`). See
//! [`suggestions`]. Nothing is received by an import, subscriptions included.
//!
//! # Save folders of subscriptions
//!
//! A subscription saves into a work folder below the collect folder, which the
//! rule screen checks when it saves ([`trss_core::folders::is_work_folder`]). A
//! replacement that would give a subscription's rule a folder that fails that
//! check (the channel's folder is the collect folder itself and the file's
//! rule names no folder of its own, or the folder has `..`) keeps the
//! subscription's folder instead; the rule is replaced otherwise. The preview
//! says so per rule (`folder_kept`) and the result lists the subscriptions
//! (`folders_kept`). It never fails the import.
//!
//! # Title-waiting subscriptions
//!
//! A file cannot express a subscription still waiting for its title (a rule
//! with `match: null` and a subscription). Replacing a channel therefore leaves
//! every such subscription of it as it is, after the file's rules, instead of
//! deleting it with the rules the file lacks; the preview says how many
//! (`existing.title_waiting_kept`) and the result counts them
//! (`title_waiting_kept`).
//!
//! # Folders
//!
//! The app has one collect folder and a rule's directory is relative to it, so
//! the file's channel `directory` is placed under it ([`trss_import::fit`]).
//! While no collect folder is set, the import sets it from the file (the
//! channels' shared folder, or their common ancestor) in the same transaction
//! that imports the channels. A channel folder inside the collect folder is
//! imported with the part between the two put in front of its rules'
//! directories; one outside it is reported with a reason and not imported,
//! and needs no choice even if it matches an existing channel. A collect folder
//! the import would set is checked like the settings screen checks one (an
//! absolute path to an existing directory, here also not `/`); when it fails,
//! the whole review is refused with the reason, and nothing is imported. The
//! collect folder the review saw (`reviewed_collect_folder` in the apply
//! request) must still be the one now, like a channel's version.

use axum::{
    extract::{rejection::JsonRejection, State},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};

mod suggestions;

use std::collections::HashMap;

use self::suggestions::SuggestionView;
use super::settings_api::check_folders;
use super::{ApiError, AppState};
use trss_collect::rss::regex_error;
use trss_collect::store::channels::{
    import::{is_title_waiting_subscription, match_rules, ImportChannel, ImportedChannel},
    ChannelError, ChannelWithRules, Rule, Version,
};
use trss_import::picks::{self, Pick, SubscriptionsResult};
use trss_import::{
    fit::{fit, Fit, Fitted},
    legacy::{self, LegacyChannel},
    plan::{
        build_actions, display_url, find_existing, keep_subscription_folders, Choice, Decision,
    },
    suggest::suggest,
};

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
    /// The collect folder the review saw (`collect_folder.current` of the
    /// preview), `null` when none was set. A different one now makes the
    /// review stale.
    #[serde(default)]
    reviewed_collect_folder: Option<String>,
    /// The subscription suggestions the user kept checked.
    #[serde(default)]
    subscriptions: Vec<Pick>,
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
    /// The save folder that stays when the channel is replaced, because the
    /// rule is a subscription and the file's folder for it is no work folder
    /// (empty, only `.`, or with `..`). `null` when the file's folder is used.
    folder_kept: Option<String>,
    /// What the comment above the rule offers.
    suggestion: SuggestionView,
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
    /// Title-waiting subscriptions of the channel: a replacement leaves them as
    /// they are, because the file cannot express them.
    title_waiting_kept: usize,
}

#[derive(Serialize)]
struct ChannelView {
    index: usize,
    /// Masked: every query value is `***`.
    url: String,
    /// The folder the file names for the channel, as written.
    directory: String,
    /// Why the channel is not imported (its folder is outside the collect
    /// folder); `null` when it is. A channel that is not imported has no
    /// `existing` and needs no choice.
    not_imported: Option<String>,
    excludes: Vec<String>,
    rules: Vec<RuleView>,
    existing: Option<ExistingView>,
}

#[derive(Serialize)]
struct PreviewResponse {
    channels: Vec<ChannelView>,
    /// How many file channels need a choice.
    conflict_count: usize,
    collect_folder: CollectFolderView,
}

#[derive(Serialize)]
struct CollectFolderView {
    /// The collect folder now; `null` when none is set. Send it back as
    /// `reviewed_collect_folder` with the apply.
    current: Option<String>,
    /// The folder the import sets because none is set yet; `null` when one is
    /// set or no channel is imported.
    will_set: Option<String>,
}

fn invalid_regex(rule: &trss_collect::store::channels::RuleInput) -> bool {
    rule.regex
        && rule
            .r#match
            .as_deref()
            .is_some_and(|pattern| regex_error(pattern, rule.case_insensitive).is_some())
}

fn channel_view(
    index: usize,
    legacy: &LegacyChannel,
    fit: &Fit,
    existing: Option<&ChannelWithRules>,
) -> ChannelView {
    let (folder, original) = (legacy.folder.as_str(), &legacy.channel);
    // A channel that is not imported is shown as the file has it.
    let (channel, not_imported) = match fit {
        Fit::Import(channel) => (channel, None),
        Fit::Outside(reason) => (original, Some(reason.clone())),
    };
    let kept: Vec<Option<usize>> = existing
        .map(|e| match_rules(&e.rules, &channel.rules))
        .unwrap_or_else(|| vec![None; channel.rules.len()]);
    let kept_folders = existing
        .map(|e| keep_subscription_folders(&mut channel.clone(), e))
        .unwrap_or_default();
    let removed_rules = existing
        .map(|e| {
            e.rules
                .iter()
                .enumerate()
                .filter(|(i, rule)| {
                    !kept.contains(&Some(*i)) && !is_title_waiting_subscription(rule)
                })
                .map(|(_, rule)| RemovedRule::from(rule))
                .collect()
        })
        .unwrap_or_default();

    let suggestions = suggestions::views(&legacy.readings, &channel.rules, existing, &kept);

    ChannelView {
        index,
        url: display_url(&channel.input.url),
        directory: folder.to_owned(),
        not_imported,
        excludes: channel.input.excludes.clone(),
        rules: channel
            .rules
            .iter()
            .zip(&kept)
            .zip(suggestions)
            .enumerate()
            .map(|(index, ((rule, kept), suggestion))| RuleView {
                r#match: rule.r#match.clone(),
                regex: rule.regex,
                case_insensitive: rule.case_insensitive,
                directory: rule.directory.clone(),
                episode: rule.episode,
                invalid_regex: invalid_regex(rule),
                keeps_existing_rule: kept.is_some(),
                folder_kept: kept_folders
                    .iter()
                    .find(|f| f.rule == index)
                    .map(|f| f.directory.clone()),
                suggestion,
            })
            .collect(),
        existing: existing.map(|e| ExistingView {
            id: e.channel.id.clone(),
            version: e.channel.version,
            url: display_url(&e.channel.url),
            rule_count: e.rules.len(),
            removed_rules,
            title_waiting_kept: e
                .rules
                .iter()
                .filter(|rule| is_title_waiting_subscription(rule))
                .count(),
        }),
    }
}

fn json<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    match body {
        Ok(Json(body)) => Ok(body),
        Err(JsonRejection::BytesRejection(_)) => Err(ApiError::invalid(
            "요청이 너무 크거나 읽을 수 없어요. 파일이 2 MB 이하인지 확인해 주세요.",
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

/// The file read, placed under the collect folder, and what exists now.
struct Reviewed {
    file: Vec<LegacyChannel>,
    fitted: Fitted,
    current_folder: Option<String>,
    existing: Vec<ChannelWithRules>,
    /// File index of each channel that is imported, in file order.
    positions: Vec<usize>,
    /// Those channels, placed under the collect folder.
    importable: Vec<ImportChannel>,
}

impl Reviewed {
    /// For each file channel, the existing channel it is the same as, among the
    /// channels that are imported; a channel that is not imported has none.
    fn existing_of_file(&self) -> Vec<Option<usize>> {
        let found = find_existing(&self.importable, &self.existing);
        let mut of_file = vec![None; self.file.len()];
        for (local, e) in found.into_iter().enumerate() {
            of_file[self.positions[local]] = e;
        }
        of_file
    }
}

async fn review(state: &AppState, content: &str) -> Result<Reviewed, ApiError> {
    let file = legacy::parse(content).map_err(|e| ApiError::invalid(e.message()))?;
    let current_folder = state
        .settings
        .collection()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map(|settings| settings.folder);
    let existing = state
        .channels
        .list_channels_with_rules()
        .await
        .map_err(store_error)?;

    let fitted = fit(file.clone(), current_folder.as_deref())
        .map_err(|refused| ApiError::invalid(refused.0))?;
    if let Some(folder) = fitted.collect_folder.clone() {
        check_folder_to_set(folder).await?;
    }
    let mut positions = Vec::new();
    let mut importable = Vec::new();
    for (index, fit) in fitted.channels.iter().enumerate() {
        if let Fit::Import(channel) = fit {
            positions.push(index);
            importable.push(channel.clone());
        }
    }
    Ok(Reviewed {
        file,
        fitted,
        current_folder,
        existing,
        positions,
        importable,
    })
}

/// The import would set the collect folder to `folder`, which skips the
/// settings screen. The web sees the media read-only, so it can check what the
/// screen checks: an existing directory it can open, at an absolute path.
async fn check_folder_to_set(folder: String) -> Result<(), ApiError> {
    let checked = tokio::task::spawn_blocking(move || check_folders(&folder, None))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    match checked {
        Ok(_) => Ok(()),
        Err(ApiError::Invalid(message)) => Err(ApiError::invalid(format!(
            "아직 수집 폴더가 없어서 파일의 채널 폴더로 정하려 했어요. {message} 설정에서 수집 폴더를 먼저 정한 뒤 다시 가져와 주세요."
        ))),
        Err(other) => Err(other),
    }
}

async fn preview(
    State(state): State<AppState>,
    body: Result<Json<PreviewRequest>, JsonRejection>,
) -> Result<Json<PreviewResponse>, ApiError> {
    let request = json(body)?;
    let reviewed = review(&state, &request.content).await?;

    let found = reviewed.existing_of_file();
    let channels: Vec<_> = reviewed
        .file
        .iter()
        .zip(&reviewed.fitted.channels)
        .enumerate()
        .map(|(i, (legacy, fit))| {
            channel_view(i, legacy, fit, found[i].map(|e| &reviewed.existing[e]))
        })
        .collect();
    Ok(Json(PreviewResponse {
        conflict_count: found.iter().flatten().count(),
        channels,
        collect_folder: CollectFolderView {
            will_set: reviewed
                .fitted
                .collect_folder
                .clone()
                .filter(|_| !reviewed.importable.is_empty()),
            current: reviewed.current_folder,
        },
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
    /// Title-waiting subscriptions the replacement left as they were.
    title_waiting_kept: usize,
    /// Subscriptions whose save folder stayed because the file's folder for
    /// their rule is no work folder.
    folders_kept: Vec<FolderKept>,
}

/// A subscription whose save folder a replacement kept.
#[derive(Serialize)]
struct FolderKept {
    /// The rule's place in the file channel.
    rule: usize,
    r#match: Option<String>,
    /// The folder the subscription keeps.
    directory: String,
}

#[derive(Serialize)]
struct SkippedView {
    index: usize,
    url: String,
}

/// A channel the import left out because its folder is outside the collect
/// folder.
#[derive(Serialize)]
struct NotImportedView {
    index: usize,
    url: String,
    reason: String,
}

#[derive(Serialize)]
struct Counts {
    channels_added: usize,
    channels_replaced: usize,
    channels_skipped: usize,
    /// Channels of the file left out because their folder is outside the
    /// collect folder.
    channels_not_imported: usize,
    /// Existing channels this import did not change.
    channels_unchanged: usize,
    /// Rules created, in added channels and as new rules of replaced ones.
    rules_added: usize,
    rules_kept: usize,
    rules_removed: usize,
    /// Title-waiting subscriptions that replaced channels kept as they were.
    title_waiting_kept: usize,
    /// Subscriptions whose save folder stayed (see [`FolderKept`]).
    folders_kept: usize,
    /// Rules that became subscriptions.
    subscriptions_created: usize,
}

#[derive(Serialize)]
struct ApplyResponse {
    added: Vec<AddedView>,
    replaced: Vec<ReplacedView>,
    skipped: Vec<SkippedView>,
    not_imported: Vec<NotImportedView>,
    /// The collect folder this import set; `null` when it changed none.
    collect_folder_set: Option<String>,
    /// What became of the subscription suggestions the user checked.
    subscriptions: SubscriptionsResult,
    counts: Counts,
}

async fn apply(
    State(state): State<AppState>,
    body: Result<Json<ApplyRequest>, JsonRejection>,
) -> Result<Json<ApplyResponse>, ApiError> {
    let request = json(body)?;
    let reviewed = review(&state, &request.content).await?;
    let stale = || ApiError::Conflict {
        message: STALE_MESSAGE.into(),
        current: None,
    };
    if reviewed.current_folder != request.reviewed_collect_folder {
        return Err(stale());
    }

    // The choices name channels by their place in the file; the plan numbers
    // only the channels that are imported.
    let choices: Vec<Choice> = request
        .choices
        .into_iter()
        .map(|c| {
            let index = reviewed
                .positions
                .iter()
                .position(|&file_index| file_index == c.index)
                .ok_or_else(stale)?;
            Ok(Choice {
                index,
                existing_id: c.existing_id,
                existing_version: c.existing_version,
                decision: match c.decision {
                    DecisionRequest::Replace => Decision::Replace,
                    DecisionRequest::Add => Decision::Add,
                    DecisionRequest::Skip => Decision::Skip,
                },
            })
        })
        .collect::<Result<_, ApiError>>()?;
    let urls: Vec<String> = reviewed
        .file
        .iter()
        .map(|c| display_url(&c.channel.input.url))
        .collect();
    // The suggestions of the imported channels, by their place in the file.
    let mut suggested = HashMap::new();
    let mut rules_of = HashMap::new();
    for (local, channel) in reviewed.importable.iter().enumerate() {
        let at = reviewed.positions[local];
        suggested.insert(at, suggest(&reviewed.file[at].readings, &channel.rules));
        rules_of.insert(at, channel.rules.clone());
    }
    let Reviewed {
        fitted,
        existing,
        positions,
        importable,
        ..
    } = reviewed;
    let not_imported: Vec<NotImportedView> = fitted
        .channels
        .iter()
        .enumerate()
        .filter_map(|(index, fit)| match fit {
            Fit::Outside(reason) => Some(NotImportedView {
                index,
                url: urls[index].clone(),
                reason: reason.clone(),
            }),
            Fit::Import(_) => None,
        })
        .collect();
    let plan = build_actions(importable, &existing, &choices).map_err(|_| stale())?;

    // The subscriptions whose folder the replacement keeps, by the channel's
    // place in the file.
    let mut folders_kept: HashMap<usize, Vec<FolderKept>> = HashMap::new();
    for (local, kept) in plan.kept_folders {
        let at = positions[local];
        folders_kept.insert(
            at,
            kept.into_iter()
                .map(|f| FolderKept {
                    rule: f.rule,
                    r#match: rules_of[&at][f.rule].r#match.clone(),
                    directory: f.directory,
                })
                .collect(),
        );
    }
    let (indexes, actions): (Vec<usize>, Vec<_>) = plan.actions.into_iter().unzip();
    let indexes: Vec<usize> = indexes.into_iter().map(|local| positions[local]).collect();
    let skipped: Vec<usize> = plan.skipped.iter().map(|&local| positions[local]).collect();
    let picked = picks::pick(
        &request.subscriptions,
        &suggested,
        &rules_of,
        &suggestions::actions_by_channel(&indexes),
    )
    .map_err(|_| stale())?;
    // The folder is set together with the channels that need it.
    let collect_folder_set = fitted.collect_folder.filter(|_| !actions.is_empty());
    if let Some(folder) = &collect_folder_set {
        // The collect folder is always a watch folder (it is registered with
        // the import), so it must not overlap one registered by hand.
        let registered = state
            .library
            .folders()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let wanted = vec![trss_library::automatic_watch::Wanted {
            what: "수집 폴더",
            path: folder.clone(),
        }];
        tokio::task::spawn_blocking(move || {
            trss_library::automatic_watch::plan(&wanted, &registered)
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(ApiError::invalid)?;
    }
    // Anissia is asked only for the checked suggestions, and never blocks the
    // import: what it cannot say is left for the worker's daily refresh.
    let resolved = suggestions::resolve(&state, &picked.anime_nos()).await;
    let picked = picks::settle(picked, &resolved);
    let to_subscribe = picks::subscriptions(&picked.wanted, &resolved);
    // The import is stamped with the time read inside its transaction.
    let anissia = state.anissia.clone();
    let (results, outcomes) = state
        .channels
        .import_channels_subscribing(
            actions,
            collect_folder_set.clone(),
            to_subscribe,
            move || anissia.now(),
        )
        .await
        .map_err(store_error)?;
    // The first run's import step is done once an import has created or
    // changed a channel (its rules and subscriptions come with one). An apply
    // that skipped every channel or left every channel out did nothing, so the
    // step stays open. The import itself is committed, so a failure here is
    // only logged: the user can still skip the step.
    if !results.is_empty() {
        if let Err(e) = state
            .setup
            .mark_import_applied(super::commands_api::now_millis())
            .await
        {
            eprintln!("import: cannot record that the import was applied: {e}");
        }
    }
    let subscriptions = picks::result(picked, &resolved, &outcomes);

    let mut response = ApplyResponse {
        added: Vec::new(),
        replaced: Vec::new(),
        skipped: skipped
            .iter()
            .map(|&index| SkippedView {
                index,
                url: urls[index].clone(),
            })
            .collect(),
        not_imported: Vec::new(),
        collect_folder_set,
        counts: Counts {
            channels_added: 0,
            channels_replaced: 0,
            channels_skipped: skipped.len(),
            channels_not_imported: not_imported.len(),
            channels_unchanged: 0,
            rules_added: 0,
            rules_kept: 0,
            rules_removed: 0,
            title_waiting_kept: 0,
            folders_kept: 0,
            subscriptions_created: subscriptions.created_count(),
        },
        subscriptions,
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
                waiting_kept,
                ..
            } => {
                let added_rules = rule_count - kept_rules - waiting_kept;
                response.counts.rules_added += added_rules;
                response.counts.rules_kept += kept_rules;
                response.counts.rules_removed += removed_rules.len();
                response.counts.title_waiting_kept += waiting_kept;
                let folders_kept = folders_kept.remove(&index).unwrap_or_default();
                response.counts.folders_kept += folders_kept.len();
                response.replaced.push(ReplacedView {
                    index,
                    id,
                    url,
                    rule_count,
                    kept_rules,
                    added_rules,
                    removed_rules: removed_rules.iter().map(RemovedRule::from).collect(),
                    title_waiting_kept: waiting_kept,
                    folders_kept,
                });
            }
        }
    }
    response.not_imported = not_imported;
    response.counts.channels_added = response.added.len();
    response.counts.channels_replaced = response.replaced.len();
    response.counts.channels_unchanged = existing.len() - response.replaced.len();
    Ok(Json(response))
}

#[cfg(test)]
mod subscription_tests;
#[cfg(test)]
mod tests;
