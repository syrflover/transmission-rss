//! A rule's episode offset in the rule detail (ticket 0024): the grounds of an
//! offset the app set, the suggestion for a rule whose offset it did not set,
//! and `적용` of that suggestion.
//!
//! `PUT /rules/{id}/episode` (`{ version, episode }`) saves the offset as the
//! user's own, at once, like the switches: the rule loses `자동` and the app's
//! logic never sets it again. A version that is not the stored one answers
//! `409` with the current view.
//!
//! The suggestion is read from what is known now (the rule's first items in
//! history, the library and the AniList counts), so it appears when the user
//! links the seasons the sum needs, and goes when the user sets an offset. It
//! is offered only to a subscription whose offset still leaves numbers as they
//! are ([`crate::episode_offset::is_open`]) and that has picked an item.

use std::collections::HashMap;

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    Json,
};
use serde::{Deserialize, Serialize};

use super::{body, rule_conflict, rule_view, store_error, RuleView};
use crate::{
    episode_offset::{decide, first_release, gather, is_open},
    store::channels::{ChannelError, Rule},
    web::{ApiError, AppState},
};

/// What the app offers for a rule's offset.
#[derive(Debug, Clone, Serialize)]
pub struct EpisodeSuggestion {
    /// The offset `적용` saves; `null` when the grounds do not say one and the
    /// user has to write it.
    pub value: Option<i64>,
    /// Why, as a sentence for the screen.
    pub basis: String,
}

/// What the views of some rules say of their offsets.
#[derive(Default)]
pub(super) struct Episodes {
    /// The grounds of the offsets the app set.
    pub basis: HashMap<String, String>,
    pub suggestion: HashMap<String, EpisodeSuggestion>,
}

/// The grounds and suggestions of the given rules. What cannot be read leaves
/// the rule without a suggestion: the detail then offers nothing, as before.
pub(super) async fn analyze(state: &AppState, rules: &[Rule]) -> Result<Episodes, ApiError> {
    let mut out = Episodes::default();
    let auto: Vec<String> = rules
        .iter()
        .filter(|r| r.episode_auto)
        .map(|r| r.id.clone())
        .collect();
    if !auto.is_empty() {
        out.basis = state
            .channels
            .episode_bases(auto)
            .await
            .map_err(store_error)?;
    }

    let open: Vec<&Rule> = rules.iter().filter(|r| is_open(r)).collect();
    if open.is_empty() {
        return Ok(out);
    }
    let internal = |e: &dyn std::fmt::Display| ApiError::Internal(e.to_string());
    let Some(collect) = state
        .settings
        .collection()
        .await
        .map_err(|e| internal(&e))?
    else {
        return Ok(out);
    };
    for rule in open {
        let titles = state
            .history
            .first_titles_of_rule(&rule.id)
            .await
            .map_err(|e| internal(&e))?;
        let Some(first) = first_release(&titles) else {
            continue;
        };
        let Some(basis) = gather(&state.library, &state.seasons.store, &collect.folder, rule)
            .await
            .map_err(|e| internal(&e))?
        else {
            continue;
        };
        if let Some((value, basis)) = decide(first, &basis).as_suggestion() {
            out.suggestion
                .insert(rule.id.clone(), EpisodeSuggestion { value, basis });
        }
    }
    Ok(out)
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
