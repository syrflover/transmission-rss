//! The subscription suggestions of the legacy import, on the wire.
//!
//! The preview lists, for every rule, what the comment above it offers
//! ([`crate::import::suggest`]): nothing about the anime's weekday and time,
//! which the screen asks Anissia for (`GET /api/anissia/schedule/{week}`) so the
//! review does not wait for a third party. The apply takes the suggestions the
//! user checked as `subscriptions: [{ channel, rule }]` (the channel's place in
//! the file and the rule's place in the channel) and makes those rules
//! subscriptions together with the import.
//!
//! What a checked suggestion becomes is decided here from the file, not from
//! the preview the client holds: the file is read again, so a pick that the file
//! no longer backs (a rule with no readable address) makes the review stale. A
//! pick for a channel the user skipped, or that is not imported, is dropped with
//! the channel. A pick that cannot be a subscription (the rule saves into the
//! collect folder itself, or an earlier rule of the channel follows the same
//! anime) is reported with its reason and the rule is imported without one.
//!
//! The creator is the one the comment names, kept as written: the review says
//! when the anime's caption list does not name it, and the user decides by
//! checking or not. A suggestion without a creator is `제작자 미정`. The
//! anime's schedule values are asked of Anissia once per anime when the import
//! is applied; when Anissia cannot answer (or does not list the anime), the
//! subscription is still made with a stand-in snapshot that the worker's daily
//! refresh replaces, and the result says so. Suggestions never block an import.

use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};

use super::super::AppState;
use crate::{
    anissia::{AnissiaError, LAST_WEEK},
    import::{
        comments::{Airs, Reading},
        suggest::{suggest, Suggestion},
    },
    store::{
        anissia::Anime,
        channels::{
            import_subscriptions::{ImportSubscription, SubscriptionOutcome},
            ChannelWithRules, NewSubscription, RuleInput, SubtitleMode,
        },
        history::Millis,
    },
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

/// A suggestion the user checked, by its place in the file.
#[derive(Deserialize)]
pub(super) struct Pick {
    /// The channel's place in the file, as the preview numbered it.
    channel: usize,
    /// The rule's place in the channel.
    rule: usize,
}

/// The review is out of step with the file: a pick that names no readable
/// address.
pub(super) struct StalePick;

/// A checked suggestion that becomes a subscription, before Anissia is asked.
pub(super) struct Wanted {
    channel: usize,
    rule: usize,
    action: usize,
    anime_no: i64,
    creator: Option<String>,
    /// The rule's match phrase, for a stand-in snapshot's title.
    phrase: String,
}

/// A checked suggestion that was left out.
#[derive(Serialize)]
pub(super) struct NotCreated {
    channel: usize,
    rule: usize,
    reason: String,
}

/// What the picks come to before Anissia is asked.
pub(super) struct Picked {
    pub wanted: Vec<Wanted>,
    not_created: Vec<NotCreated>,
}

impl Picked {
    /// The anime the subscriptions follow.
    pub fn anime_nos(&self) -> Vec<i64> {
        self.wanted.iter().map(|w| w.anime_no).collect()
    }
}

/// Decides the picks against the file's suggestions. `suggestions` are the
/// suggestions of each imported channel by its place in the file; `actions` tell
/// which of those channels the plan imports and in which action. A pick for a
/// channel with no action (skipped) is dropped.
pub(super) fn pick(
    picks: &[Pick],
    suggestions: &HashMap<usize, Vec<Suggestion>>,
    rules: &HashMap<usize, Vec<RuleInput>>,
    action_of: &HashMap<usize, usize>,
) -> Result<Picked, StalePick> {
    let mut out = Picked {
        wanted: Vec::new(),
        not_created: Vec::new(),
    };
    let mut seen = Vec::new();
    for pick in picks {
        if seen.contains(&(pick.channel, pick.rule)) {
            continue;
        }
        seen.push((pick.channel, pick.rule));

        let Some(offered) = suggestions.get(&pick.channel) else {
            // A channel that is not imported has no suggestions to keep.
            continue;
        };
        let Some(&action) = action_of.get(&pick.channel) else {
            // A skipped channel drops its suggestions with it.
            continue;
        };
        let suggestion = offered.get(pick.rule).ok_or(StalePick)?;
        let Reading::Address { .. } = suggestion.reading else {
            return Err(StalePick);
        };
        if let Some(reason) = &suggestion.blocked {
            out.not_created.push(NotCreated {
                channel: pick.channel,
                rule: pick.rule,
                reason: reason.clone(),
            });
            continue;
        }
        let (anime_no, creator) = suggestion.offer().ok_or(StalePick)?;
        out.wanted.push(Wanted {
            channel: pick.channel,
            rule: pick.rule,
            action,
            anime_no,
            creator: creator.map(str::to_owned),
            phrase: rules[&pick.channel][pick.rule]
                .r#match
                .clone()
                .unwrap_or_default(),
        });
    }
    Ok(out)
}

/// The schedule snapshots of the anime Anissia lists, and why it could not be
/// asked, if it could not.
pub(super) struct Resolved {
    found: HashMap<i64, Anime>,
    /// A sentence that says why Anissia did not answer.
    unavailable: Option<String>,
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

/// The subscriptions to write with the import, given what Anissia said.
pub(super) fn subscriptions(
    wanted: &[Wanted],
    resolved: &Resolved,
    at: Millis,
) -> Vec<ImportSubscription> {
    wanted
        .iter()
        .map(|w| {
            let (anime, placeholder) = match resolved.found.get(&w.anime_no) {
                Some(anime) => (anime.clone(), false),
                None => (ImportSubscription::stand_in(w.anime_no, &w.phrase), true),
            };
            ImportSubscription {
                action: w.action,
                rule: w.rule,
                subscription: NewSubscription {
                    anime,
                    subtitles: if w.creator.is_some() {
                        SubtitleMode::Follow
                    } else {
                        SubtitleMode::Undecided
                    },
                    creator: w.creator.clone(),
                    subscribed_at: at,
                },
                placeholder,
            }
        })
        .collect()
}

/// A rule that became a subscription.
#[derive(Serialize)]
struct Created {
    channel: usize,
    rule: usize,
    anime_no: i64,
    /// Anissia's title for the anime; `null` while the schedule is unknown.
    subject: Option<String>,
    creator: Option<String>,
    /// Whether the weekday and time were read from Anissia. `false` leaves
    /// them for the worker's daily refresh.
    schedule_known: bool,
}

/// What became of the checked suggestions.
#[derive(Serialize)]
pub(super) struct SubscriptionsResult {
    created: Vec<Created>,
    /// Checked suggestions that did not become subscriptions, with the reason;
    /// the rules were imported all the same.
    not_created: Vec<NotCreated>,
    /// Why Anissia could not be asked, if it could not.
    unavailable: Option<String>,
}

impl SubscriptionsResult {
    pub fn created_count(&self) -> usize {
        self.created.len()
    }
}

/// The result of the picks, from the outcomes of the store.
pub(super) fn result(
    picked: Picked,
    resolved: &Resolved,
    outcomes: &[SubscriptionOutcome],
) -> SubscriptionsResult {
    let Picked {
        wanted,
        not_created,
    } = picked;
    let mut result = SubscriptionsResult {
        created: Vec::new(),
        not_created,
        unavailable: resolved.unavailable.clone(),
    };
    for (w, outcome) in wanted.iter().zip(outcomes) {
        match outcome {
            SubscriptionOutcome::Created => {
                let found = resolved.found.get(&w.anime_no);
                result.created.push(Created {
                    channel: w.channel,
                    rule: w.rule,
                    anime_no: w.anime_no,
                    subject: found.map(|a| a.subject.clone()),
                    creator: w.creator.clone(),
                    schedule_known: found.is_some(),
                });
            }
            SubscriptionOutcome::RuleAlreadySubscribed => result.not_created.push(NotCreated {
                channel: w.channel,
                rule: w.rule,
                reason: "이 규칙은 이미 구독이라서 그대로 두었어요.".to_owned(),
            }),
            SubscriptionOutcome::AnimeTakenInChannel { .. } => {
                result.not_created.push(NotCreated {
                    channel: w.channel,
                    rule: w.rule,
                    reason: "이 채널에는 같은 작품을 구독하는 규칙이 이미 있어요.".to_owned(),
                })
            }
        }
    }
    result
}

/// Which action of the plan imports each file channel.
pub(super) fn actions_by_channel(indexes: &[usize]) -> HashMap<usize, usize> {
    indexes
        .iter()
        .enumerate()
        .map(|(action, &channel)| (channel, action))
        .collect()
}
