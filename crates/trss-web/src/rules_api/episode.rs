//! A rule's episode offset in the rule detail (ticket 0024): the grounds of an
//! offset the app set, the suggestion for a rule whose offset it did not set,
//! and `적용` of that suggestion.
//!
//! `PUT /rules/{id}/episode` (`{ version, episode }`) saves the offset as the
//! user's own, at once, like the switches: the rule loses `자동` and the app's
//! logic never sets it again. A version that is not the stored one answers
//! `409` with the current view.
//!
//! The grounds of an automatic offset say the value it replaced, when that is
//! known: `… 정했어요 (전에는 −24).` The view then offers `되돌리기`, an
//! `episode_undo` command ([`trss_collect::commands::episode_undo`]); the last
//! such command of the rule comes with the view (`episode_undo`), with each
//! video it renames or left as it was, so the screen shows its progress and
//! the files it could not rename.
//!
//! The suggestion is read from what is known now (the rule's first items in
//! history, the library and the AniList counts), so it appears when the user
//! links the seasons the sum needs, and goes when the user sets the offset it
//! offers. It is offered to a subscription that the app has never decided,
//! whatever its field holds, when the value differs from it
//! ([`trss_collect::episode_offset::worth_offering`]). One that has picked
//! nothing yet is offered what the earliest of its past items would give, when
//! that gives a value ([`episode_offset::before_receiving`], ticket 0126), so
//! the person sees it before the first item is received and named; so is a
//! subscription about to be made, in the preview ([`for_new_subscription`]).

use std::collections::HashMap;

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    Json,
};
use serde::{Deserialize, Serialize};

use super::{body, build_preview, channel_items, rule_conflict, rule_view, store_error, RuleView};
use crate::{commands_api::CommandView, ApiError, AppState};
use trss_collect::{
    commands::episode_undo,
    episode_offset::{self, decide, first_release, gather, may_decide, worth_offering},
    store::{
        channels::{ChannelError, ChannelWithRules, Rule, RuleInput, RuleState},
        history::HistoryItem,
    },
};
use trss_core::{episode::signed, settings::CollectionSettings};

/// What the app offers for a rule's offset.
#[derive(Debug, Clone, Serialize)]
pub struct EpisodeSuggestion {
    /// The offset `적용` saves; `null` when the grounds do not say one and the
    /// user has to write it.
    pub value: Option<i64>,
    /// Why, as a sentence for the screen.
    pub basis: String,
}

/// The last `되돌리기` of a rule's automatic offset.
#[derive(Debug, Clone, Serialize)]
pub struct EpisodeUndoView {
    /// The command; it ends `done` (`undone`) once the value is back, or
    /// `failed` with the reason nothing changed.
    pub command: CommandView,
    /// The automatic value undone, once the undo began: a request for it
    /// carries an unfinished undo on.
    pub from: Option<i64>,
    /// The value put back, once the undo began.
    pub to: Option<i64>,
    /// The videos it renames, in order, once it began.
    pub files: Vec<UndoFileView>,
}

/// One video of an undo.
#[derive(Debug, Clone, Serialize)]
pub struct UndoFileView {
    pub from_name: String,
    pub to_name: String,
    /// `pending`, `renamed` or `kept` (left as it is, for the `reason`).
    pub state: &'static str,
    pub reason: Option<String>,
}

/// What the views of some rules say of their offsets.
#[derive(Default)]
pub(super) struct Episodes {
    /// The grounds of the offsets the app set.
    pub basis: HashMap<String, String>,
    /// The offsets the app's own replaced, while the app's are in force.
    pub previous: HashMap<String, i64>,
    pub suggestion: HashMap<String, EpisodeSuggestion>,
    /// The last undo of each rule that had one.
    pub undo: HashMap<String, EpisodeUndoView>,
}

/// The grounds and suggestions of the given rules. What cannot be read leaves
/// the rule without them, with a line in the log: they only explain and offer,
/// so the rule list still answers.
pub(super) async fn analyze(state: &AppState, cwr: &ChannelWithRules) -> Episodes {
    let rules = &cwr.rules;
    let mut out = Episodes::default();
    let ids: Vec<String> = rules.iter().map(|r| r.id.clone()).collect();
    out.undo = undos(state, ids.clone()).await;
    let marks = match state.channels.episode_marks(ids).await {
        Ok(marks) => marks,
        Err(err) => {
            eprintln!("Episode offset: cannot read the grounds: {err}");
            return out;
        }
    };
    for rule in rules.iter().filter(|r| r.episode_auto) {
        let Some(mark) = marks.get(&rule.id) else {
            continue;
        };
        if let Some(previous) = mark.previous {
            out.previous.insert(rule.id.clone(), previous);
        }
        if let Some(basis) = &mark.basis {
            let basis = match mark.previous {
                Some(previous) => with_previous(basis, previous),
                None => basis.clone(),
            };
            out.basis.insert(rule.id.clone(), basis);
        }
    }

    let open: Vec<&Rule> = rules
        .iter()
        .filter(|r| may_decide(r) && marks.get(&r.id).is_some_and(|m| !m.decided))
        .collect();
    if open.is_empty() {
        return out;
    }
    let collect = match state.settings.collection().await {
        Ok(Some(collect)) => collect,
        Ok(None) => return out,
        Err(err) => {
            eprintln!("Episode offset: cannot read the collect folder: {err}");
            return out;
        }
    };
    // The channel's items, read once for the rules that picked nothing.
    let mut items: Option<Vec<HistoryItem>> = None;
    for rule in open {
        let titles = match state.history.first_titles_of_rule(&rule.id).await {
            Ok(titles) => titles,
            Err(err) => {
                eprintln!("Episode offset: no suggestion for rule {}: {err}", rule.id);
                continue;
            }
        };
        let Some(first) = first_release(&titles) else {
            // Nothing picked yet: what its past items would give. A
            // subscription waiting for its title takes none.
            if rule.r#match.is_none() {
                continue;
            }
            if let Some(first) = earliest_past(state, &collect.folder, cwr, &mut items, rule).await
            {
                if let Some(offer) = offer_before_receiving(state, &collect, rule, first).await {
                    out.suggestion.insert(rule.id.clone(), offer);
                }
            }
            continue;
        };
        let basis = match gather(
            &state.library,
            &state.seasons.store,
            &collect.folder,
            collect.archive_folder.as_deref(),
            rule,
        )
        .await
        {
            Ok(Some(basis)) => basis,
            Ok(None) => continue,
            Err(err) => {
                eprintln!("Episode offset: no suggestion for rule {}: {err}", rule.id);
                continue;
            }
        };
        let Some((value, basis)) = decide(first, &basis).as_suggestion() else {
            continue;
        };
        if worth_offering(rule, value) {
            out.suggestion
                .insert(rule.id.clone(), EpisodeSuggestion { value, basis });
        }
    }
    out
}

/// The earliest release among the past items `rule` of `cwr` may be asked to
/// receive, judged as the rule's preview judges them; `None` (with a line in
/// the log when something cannot be read) when there is none. `items` holds
/// the channel's items once read, for the next rule.
async fn earliest_past(
    state: &AppState,
    collect_folder: &str,
    cwr: &ChannelWithRules,
    items: &mut Option<Vec<HistoryItem>>,
    rule: &Rule,
) -> Option<u32> {
    if items.is_none() {
        *items = Some(match channel_items(&state.history, &cwr.channel.id).await {
            Ok(read) => read,
            Err(err) => {
                eprintln!(
                    "Episode offset: no suggestion before receiving in channel {}: {err:?}",
                    cwr.channel.id
                );
                Vec::new()
            }
        });
    }
    let items = items.as_deref().unwrap_or_default();
    match build_preview(
        std::path::Path::new(collect_folder),
        cwr,
        Some(&rule.id),
        &rule.to_input(),
        None,
        items,
    ) {
        Ok(preview) => preview.earliest_receivable,
        Err(err) => {
            eprintln!(
                "Episode offset: no suggestion for rule {}: {err:?}",
                rule.id
            );
            None
        }
    }
}

/// What `rule`, which has picked nothing, is offered before its first receive
/// when `first` is the earliest of its past items.
async fn offer_before_receiving(
    state: &AppState,
    collect: &CollectionSettings,
    rule: &Rule,
    first: u32,
) -> Option<EpisodeSuggestion> {
    let basis = match gather(
        &state.library,
        &state.seasons.store,
        &collect.folder,
        collect.archive_folder.as_deref(),
        rule,
    )
    .await
    {
        Ok(basis) => basis?,
        Err(err) => {
            eprintln!("Episode offset: no suggestion for rule {}: {err}", rule.id);
            return None;
        }
    };
    let (value, basis) = episode_offset::before_receiving(first, &basis)?;
    worth_offering(rule, Some(value)).then_some(EpisodeSuggestion {
        value: Some(value),
        basis,
    })
}

/// What a subscription about to be made on `channel_id` with the fields
/// `edited` is offered before anything is received, when `first` is the
/// earliest of the past items it would list. The work the folder names is
/// looked for in the archive folder too: the subscription brings it over.
pub(super) async fn for_new_subscription(
    state: &AppState,
    channel_id: &str,
    edited: &RuleInput,
    first: u32,
) -> Option<EpisodeSuggestion> {
    let collect = match state.settings.collection().await {
        Ok(collect) => collect?,
        Err(err) => {
            eprintln!("Episode offset: cannot read the collect folder: {err}");
            return None;
        }
    };
    // Only its folder and field are read; it is no subscription the app has
    // decided, nor linked to a season.
    let draft = Rule {
        id: String::new(),
        channel_id: channel_id.to_owned(),
        position: 0,
        version: 0,
        r#match: edited.r#match.clone(),
        regex: edited.regex,
        case_insensitive: edited.case_insensitive,
        directory: edited.directory.clone(),
        episode: edited.episode,
        episode_auto: false,
        state: RuleState::Active,
        subscription: None,
        resumed_at: None,
    };
    offer_before_receiving(state, &collect, &draft, first).await
}

/// The last undo of each of the rules that had one. What cannot be read leaves
/// the rules without it, with a line in the log.
async fn undos(state: &AppState, ids: Vec<String>) -> HashMap<String, EpisodeUndoView> {
    let commands = match state
        .commands
        .latest_for_subjects(episode_undo::KIND, ids)
        .await
    {
        Ok(commands) => commands,
        Err(err) => {
            eprintln!("Episode offset: cannot read the undos: {err}");
            return HashMap::new();
        }
    };
    if commands.is_empty() {
        return HashMap::new();
    }
    let begun = match state
        .channels
        .episode_undos(commands.values().map(|c| c.id.clone()).collect())
        .await
    {
        Ok(begun) => begun,
        Err(err) => {
            eprintln!("Episode offset: cannot read the undos: {err}");
            HashMap::new()
        }
    };
    commands
        .into_iter()
        .map(|(rule_id, command)| {
            let undo = begun.get(&command.id);
            let view = EpisodeUndoView {
                command: CommandView::from(&command),
                from: undo.map(|u| u.from),
                to: undo.map(|u| u.to),
                files: undo
                    .map(|u| {
                        u.files
                            .iter()
                            .map(|f| UndoFileView {
                                from_name: f.file.from_name.clone(),
                                to_name: f.file.to_name.clone(),
                                state: f.state.code(),
                                reason: f.reason.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            (rule_id, view)
        })
        .collect()
}

/// The grounds of an automatic offset with the value it replaced:
/// `… 정했어요 (전에는 −24).`, or `(전에는 변환 없음)` for a value that left
/// numbers as they were.
fn with_previous(basis: &str, previous: i64) -> String {
    let before = if episode_offset::leaves_numbers(previous) {
        "변환 없음".to_owned()
    } else {
        signed(previous)
    };
    format!(
        "{} (전에는 {before}).",
        basis.trim_end().trim_end_matches('.')
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EpisodeBody {
    /// The version the client saw.
    version: i64,
    episode: i64,
}

pub(super) async fn put_episode(
    State(state): State<AppState>,
    Path(id): Path<String>,
    parsed: Result<Json<EpisodeBody>, JsonRejection>,
) -> Result<Json<RuleView>, ApiError> {
    let b = body(parsed)?;
    let stored = state
        .channels
        .get_rule(&id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ChannelError::NotFound {
            kind: "rule",
            id: id.clone(),
        })?;
    if stored.version != b.version {
        return Err(rule_conflict(&state, &id).await);
    }
    match state.channels.set_episode(&id, b.version, b.episode).await {
        Ok(_) => Ok(Json(rule_view(&state, &id).await?)),
        Err(e) if e.is_conflict() => Err(rule_conflict(&state, &id).await),
        Err(e) => Err(store_error(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::with_previous;

    #[test]
    fn the_grounds_say_the_value_they_replaced() {
        let basis = "첫 화가 49화라서 회차 변환을 −48로 정했어요.";
        assert_eq!(
            with_previous(basis, -24),
            "첫 화가 49화라서 회차 변환을 −48로 정했어요 (전에는 −24)."
        );
        assert_eq!(
            with_previous(basis, 1),
            "첫 화가 49화라서 회차 변환을 −48로 정했어요 (전에는 변환 없음)."
        );
    }
}
