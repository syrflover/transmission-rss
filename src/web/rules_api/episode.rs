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
//! known: `… 정했어요 (전에는 −24).`
//!
//! The suggestion is read from what is known now (the rule's first items in
//! history, the library and the AniList counts), so it appears when the user
//! links the seasons the sum needs, and goes when the user sets an offset. It
//! is offered only to a subscription whose offset still leaves numbers as they
//! are ([`crate::episode_offset::is_plain`]), that the app has never decided
//! and that has picked an item.

use std::collections::HashMap;

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    Json,
};
use serde::{Deserialize, Serialize};

use super::{body, rule_conflict, rule_view, store_error, RuleView};
use crate::{
    episode_offset::{decide, first_release, gather, is_plain, signed},
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
    /// The offsets the app's own replaced, while the app's are in force.
    pub previous: HashMap<String, i64>,
    pub suggestion: HashMap<String, EpisodeSuggestion>,
}

/// The grounds and suggestions of the given rules. What cannot be read leaves
/// the rule without them, with a line in the log: they only explain and offer,
/// so the rule list still answers.
pub(super) async fn analyze(state: &AppState, rules: &[Rule]) -> Episodes {
    let mut out = Episodes::default();
    let ids: Vec<String> = rules.iter().map(|r| r.id.clone()).collect();
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
        .filter(|r| is_plain(r) && marks.get(&r.id).is_some_and(|m| !m.decided))
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
    for rule in open {
        let titles = match state.history.first_titles_of_rule(&rule.id).await {
            Ok(titles) => titles,
            Err(err) => {
                eprintln!("Episode offset: no suggestion for rule {}: {err}", rule.id);
                continue;
            }
        };
        let Some(first) = first_release(&titles) else {
            continue;
        };
        let basis = match gather(&state.library, &state.seasons.store, &collect.folder, rule).await
        {
            Ok(Some(basis)) => basis,
            Ok(None) => continue,
            Err(err) => {
                eprintln!("Episode offset: no suggestion for rule {}: {err}", rule.id);
                continue;
            }
        };
        if let Some((value, basis)) = decide(first, &basis).as_suggestion() {
            out.suggestion
                .insert(rule.id.clone(), EpisodeSuggestion { value, basis });
        }
    }
    out
}

/// The grounds of an automatic offset with the value it replaced:
/// `… 정했어요 (전에는 −24).`, or `(전에는 변환 없음)` for a value that left
/// numbers as they were.
fn with_previous(basis: &str, previous: i64) -> String {
    let before = if matches!(previous, 0 | 1) {
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
