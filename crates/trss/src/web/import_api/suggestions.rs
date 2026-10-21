//! The subscription suggestions of the legacy import, on the wire.
//!
//! The preview lists, for every rule, what the comment above it offers
//! ([`trss_import::suggest`]): nothing about the anime's weekday and time,
//! which the screen asks Anissia for (`GET /api/anissia/schedule/{week}`) so the
//! review does not wait for a third party. The apply takes the suggestions the
//! user checked as `subscriptions: [{ channel, rule }]` and makes those rules
//! subscriptions together with the import. What they come to is decided by
//! [`trss_import::picks`]; this module asks Anissia for the schedule of the
//! picked anime once per anime ([`resolve`]) and says why when it cannot. The
//! subscriptions never block an import.

use std::{collections::HashMap, time::Duration};

use serde::Serialize;

use super::super::AppState;
use trss_anissia::{AnissiaError, LAST_WEEK};
use trss_collect::store::channels::{ChannelWithRules, RuleInput};
use trss_import::{
    comments::{Airs, Reading},
    picks::Resolved,
    suggest::suggest,
};

/// The longest the apply waits for its turn to ask Anissia, per request.
const MAX_WAIT: Duration = Duration::from_secs(8);

/// What the comment above a rule offers, for the review.
#[derive(Serialize)]
pub(super) struct SuggestionView {
    /// `with_creator`, `address_only`, `unreadable` or `none`.
    kind: &'static str,
    /// Anissia's `animeNo` of an address that was read.
    anime_no: Option<i64>,
    /// The creator the comment names; `null` is `제작자 미정`.
    creator: Option<String>,
    /// The weekday and time the comment gives: shown only while Anissia has
    /// not answered, since Anissia is the authority.
    comment_airs: Option<Airs>,
    /// Why an unreadable comment could not be read.
    reason: Option<String>,
    /// Why a suggestion that was read cannot become a subscription.
    blocked: Option<String>,
    /// Whether the review starts with it checked.
    checked: bool,
    /// Replacing the channel keeps an existing rule for this one, and that
    /// rule is a subscription already: it stays as it is. The review leaves
    /// the suggestion unchecked and unavailable while the channel is replaced.
    keeps_subscription: bool,
}

pub(super) fn views(
    readings: &[Reading],
    rules: &[RuleInput],
    existing: Option<&ChannelWithRules>,
    kept: &[Option<usize>],
) -> Vec<SuggestionView> {
    suggest(readings, rules)
        .into_iter()
        .zip(kept)
        .map(|(suggestion, kept)| {
            let (anime_no, creator, airs, reason) = match &suggestion.reading {
                Reading::Address {
                    anime_no,
                    creator,
                    airs,
                } => (Some(*anime_no), creator.clone(), airs.clone(), None),
                Reading::Unreadable { reason } => (None, None, None, Some(reason.clone())),
                Reading::None => (None, None, None, None),
            };
            let keeps_subscription = existing
                .zip(*kept)
                .is_some_and(|(e, index)| e.rules[index].subscription.is_some());
            SuggestionView {
                kind: suggestion.kind().code(),
                anime_no,
                creator,
                comment_airs: airs,
                reason,
                checked: suggestion.checked_at_first(),
                blocked: suggestion.blocked,
                keeps_subscription,
            }
        })
        .collect()
}

fn why(error: &AnissiaError) -> String {
    match error {
        AnissiaError::Busy { retry_after } => format!(
            "Anissia에 요청이 몰려 있어서 {}초쯤 기다려야 해요.",
            retry_after.as_secs().max(1)
        ),
        AnissiaError::Unreachable(_) => "Anissia에 연결하지 못했어요.".to_owned(),
        AnissiaError::Status(code) => format!("Anissia가 오류로 답했어요(HTTP {code})."),
        AnissiaError::Invalid(_) => "Anissia의 응답을 읽지 못했어요.".to_owned(),
        AnissiaError::NoSuchWeek(_) | AnissiaError::Store(_) => {
            "Anissia에 묻는 중에 문제가 생겼어요.".to_owned()
        }
    }
}

/// Looks the anime up in the schedule, week by week, until every one is found
/// or Anissia stops answering. Anissia has no request for one anime.
pub(super) async fn resolve(state: &AppState, anime_nos: &[i64]) -> Resolved {
    let mut missing: Vec<i64> = anime_nos.to_vec();
    missing.sort_unstable();
    missing.dedup();
    let mut resolved = Resolved {
        found: HashMap::new(),
        unavailable: None,
    };
    for week in 0..=LAST_WEEK {
        if missing.is_empty() {
            break;
        }
        match state.anissia.schedule(week, Some(MAX_WAIT)).await {
            Ok(listed) => {
                for entry in listed.value.iter() {
                    if let Some(at) = missing.iter().position(|no| *no == entry.anime_no) {
                        missing.remove(at);
                        resolved
                            .found
                            .insert(entry.anime_no, entry.snapshot(listed.fetched_at));
                    }
                }
            }
            Err(e) => {
                resolved.unavailable = Some(why(&e));
                break;
            }
        }
    }
    resolved
}

/// Which action of the plan imports each file channel.
pub(super) fn actions_by_channel(indexes: &[usize]) -> HashMap<usize, usize> {
    indexes
        .iter()
        .enumerate()
        .map(|(action, &channel)| (channel, action))
        .collect()
}
