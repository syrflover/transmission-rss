//! What the subscriptions the user checked in an import come to.
//!
//! The apply takes the suggestions the user checked as `subscriptions:
//! [{ channel, rule }]` (the channel's place in the file and the rule's place in
//! the channel). What a checked suggestion becomes is decided from the file's
//! suggestions ([`super::suggest`]), not from the preview the client holds: a
//! pick that the file no longer backs (a rule with no readable address) makes
//! the review stale. A pick for a channel the user skipped, or that is not
//! imported, is dropped with the channel. A pick that cannot be a subscription
//! (the rule saves into the collect folder itself, or an earlier rule of the
//! channel follows the same anime) is reported with its reason and the rule is
//! imported without one.
//!
//! The creator is the one the comment names, kept as written; a suggestion
//! without a creator is `제작자 미정` ([`SubtitleMode::Undecided`]). When Anissia
//! answered and does not list an anime (a mistyped number, a show that ended),
//! nothing is known of it: the pick is reported as not created and the rule is
//! imported plain ([`settle`]). When Anissia could not answer, the subscription
//! is made with a stand-in snapshot that the worker's daily refresh replaces,
//! on the weekday and time the comment gave (in `기타` when it gave none), and
//! the result says so. Asking Anissia is the caller's: it passes what it found
//! as [`Resolved`].

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{
    comments::{Airs, Reading},
    suggest::Suggestion,
};
use trss_anissia::Anime;
use trss_collect::store::channels::{
    import_subscriptions::{ImportSubscription, SubscriptionOutcome},
    NewSubscription, RuleInput, SubtitleMode,
};

/// A suggestion the user checked, by its place in the file.
#[derive(Debug, Deserialize)]
pub struct Pick {
    /// The channel's place in the file, as the preview numbered it.
    channel: usize,
    /// The rule's place in the channel.
    rule: usize,
}

/// The review is out of step with the file: a pick that names no readable
/// address.
#[derive(Debug)]
pub struct StalePick;

/// A checked suggestion that becomes a subscription, before Anissia is asked.
#[derive(Debug)]
pub struct Wanted {
    channel: usize,
    rule: usize,
    action: usize,
    anime_no: i64,
    creator: Option<String>,
    /// The rule's match phrase, for a stand-in snapshot's title.
    phrase: String,
    /// The weekday and time the comment gives, for a stand-in snapshot.
    airs: Option<Airs>,
}

/// A checked suggestion that was left out.
#[derive(Debug, Serialize)]
pub struct NotCreated {
    channel: usize,
    rule: usize,
    reason: String,
}

/// What the picks come to before Anissia is asked.
#[derive(Debug)]
pub struct Picked {
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
/// which of those channels the plan imports and in which action (`action_of`, by the
/// channel's place in the file). A pick for a
/// channel with no action (skipped) is dropped.
pub fn pick(
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
        let Reading::Address { airs, .. } = &suggestion.reading else {
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
            airs: airs.clone(),
        });
    }
    Ok(out)
}

/// Why a pick for an anime that Anissia answered without is left out.
const UNLISTED: &str = "Anissia 편성표에 없는 작품이라서 규칙만 가져왔어요.";

/// Leaves out the picks for an anime Anissia was asked about and did not list
/// (a mistyped number, a show that ended): nothing can be said of its schedule,
/// so no subscription is made and the rule comes in plain. When Anissia could
/// not be asked at all the picks stay, to be made with a stand-in snapshot.
pub fn settle(picked: Picked, resolved: &Resolved) -> Picked {
    if resolved.unavailable.is_some() {
        return picked;
    }
    let Picked {
        wanted,
        mut not_created,
    } = picked;
    let (listed, unlisted): (Vec<_>, Vec<_>) = wanted
        .into_iter()
        .partition(|w| resolved.found.contains_key(&w.anime_no));
    not_created.extend(unlisted.into_iter().map(|w| NotCreated {
        channel: w.channel,
        rule: w.rule,
        reason: UNLISTED.to_owned(),
    }));
    Picked {
        wanted: listed,
        not_created,
    }
}

/// The schedule snapshots of the anime Anissia lists, and why it could not be
/// asked, if it could not.
#[derive(Debug)]
pub struct Resolved {
    pub found: HashMap<i64, Anime>,
    /// A sentence that says why Anissia did not answer.
    pub unavailable: Option<String>,
}

/// The subscriptions to write with the import, given what Anissia said.
pub fn subscriptions(wanted: &[Wanted], resolved: &Resolved) -> Vec<ImportSubscription> {
    wanted
        .iter()
        .map(|w| {
            let (anime, placeholder) = match resolved.found.get(&w.anime_no) {
                Some(anime) => (anime.clone(), false),
                None => {
                    let airs = w.airs.as_ref().map(|a| (a.week, a.time.as_str()));
                    (
                        ImportSubscription::stand_in(w.anime_no, &w.phrase, airs),
                        true,
                    )
                }
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
                    // The store stamps the import time inside its transaction.
                    subscribed_at: 0,
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
    /// Whether, until then, the subscription sits on the weekday and time the
    /// comment gave (`기타` when the comment gave none).
    schedule_from_comment: bool,
}

/// What became of the checked suggestions.
#[derive(Serialize)]
pub struct SubscriptionsResult {
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
pub fn result(
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
                    schedule_from_comment: found.is_none() && w.airs.is_some(),
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

#[cfg(test)]
mod tests;
