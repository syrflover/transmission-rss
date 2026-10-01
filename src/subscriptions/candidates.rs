//! Title candidates: the works that first show up in a channel's history while a
//! subscription there is waiting for its title (`docs/specs/collection.md`,
//! 방영작 구독, and `docs/specs/jobs.md`, 할 일).
//!
//! A candidate is not stored. It is read off the channel's rules and history
//! every time, so it appears and disappears with the facts it rests on. A work
//! of a channel is a candidate when all of these hold:
//!
//! - The channel has a collecting (`active`) subscription with no match phrase
//!   yet. A paused or archived one offers nothing, and a candidate goes when
//!   the last such subscription is paused, archived, given its title or
//!   deleted.
//! - History recorded the work first at or after the moment that subscription
//!   began to wait: the later of when it was subscribed, last turned back on or
//!   given a title ([`crate::worker::plan::PastSince::until`]). A work already
//!   in the history by then is not new, and one that first showed up while
//!   nothing waited, or while the subscription was paused, never becomes a
//!   candidate. The items of the work are compared by their first sighting, of
//!   any result, so an excluded batch seen earlier makes the work old.
//! - At least one item of the work was recorded as matching no rule
//!   (`no_match`), and no item of it was taken by a rule: none was added,
//!   found a duplicate or failed to add, and no rule of the channel, in any
//!   state, matches any of its titles now. A work another rule handles is not
//!   anyone's to name.
//! - The user did not reject it for this channel. A rejection is permanent and
//!   changes no subscription.
//!
//! One candidate stands for the work, however many of its episodes and
//! releases appear: items are grouped by [`work_key`] of the work part of the
//! title ([`parse_release`]). Which subscription the work belongs to is only
//! the user's call; the candidate lists the waiting subscriptions to pick from.

use std::{collections::HashSet, path::Path};

use crate::{
    store::{
        channels::{ChannelWithRules, Rule, RuleState},
        history::{HistoryItem, HistoryResult, Millis},
    },
    worker::plan::{past_since, ChannelPlan, Judgement},
};

use super::{parse_release, work_key};

/// A work that is new in a channel, offered to the subscriptions waiting there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleCandidate {
    pub channel_id: String,
    /// [`work_key`] of the work: what identifies the candidate and its rejection.
    pub key: String,
    /// The work as the newest item writes it: the match phrase it would give.
    pub work: String,
    /// The newest item's full title.
    pub latest_title: String,
    /// How many recorded items the work has.
    pub items: usize,
    /// When history first saw the work.
    pub first_seen_at: Millis,
    /// When history last saw a new item of it.
    pub latest_seen_at: Millis,
    /// The channel's collecting subscriptions that wait for a title, in the
    /// order the channel checks its rules.
    pub waiting: Vec<String>,
}

/// Whether `rule` is a collecting subscription that waits for its title.
pub fn is_waiting(rule: &Rule) -> bool {
    rule.state == RuleState::Active && rule.r#match.is_none() && rule.subscription.is_some()
}

/// The channel's title candidates, the work with the newest item first. `items`
/// are the channel's recorded items, `rejected` the [`work_key`]s the user
/// turned down for this channel.
pub fn title_candidates(
    channel: &ChannelWithRules,
    items: &[HistoryItem],
    rejected: &HashSet<String>,
) -> Vec<TitleCandidate> {
    let waiting: Vec<&Rule> = channel.rules.iter().filter(|r| is_waiting(r)).collect();
    // A work must be new at the moment the earliest waiting subscription began
    // to wait.
    let Some(since) = waiting
        .iter()
        .filter_map(|rule| past_since(rule).map(|p| p.until()))
        .min()
    else {
        return Vec::new();
    };

    // Every rule, whatever its state, is asked whether it matches a title.
    let mut open = channel.clone();
    for rule in &mut open.rules {
        rule.state = RuleState::Active;
    }
    let plan = ChannelPlan::new(open, Path::new(""));

    struct Group {
        key: String,
        work: String,
        latest_title: String,
        items: usize,
        first_seen_at: Millis,
        latest_seen_at: Millis,
        unmatched: bool,
        taken: bool,
    }
    let mut groups: Vec<Group> = Vec::new();
    for item in items {
        let Some(release) = parse_release(&item.title) else {
            continue;
        };
        let key = work_key(&release.work);
        let taken = matches!(
            item.result,
            HistoryResult::Received | HistoryResult::Duplicate | HistoryResult::AddFailed
        ) || matches!(plan.judge(&item.title), Judgement::Selected { .. });
        let unmatched = item.result == HistoryResult::NoMatch;
        match groups.iter_mut().find(|g| g.key == key) {
            Some(group) => {
                group.items += 1;
                group.first_seen_at = group.first_seen_at.min(item.first_seen_at);
                group.taken |= taken;
                group.unmatched |= unmatched;
                if item.first_seen_at > group.latest_seen_at {
                    group.work = release.work;
                    group.latest_title = item.title.clone();
                    group.latest_seen_at = item.first_seen_at;
                }
            }
            None => groups.push(Group {
                key,
                work: release.work,
                latest_title: item.title.clone(),
                items: 1,
                first_seen_at: item.first_seen_at,
                latest_seen_at: item.first_seen_at,
                unmatched,
                taken,
            }),
        }
    }

    let mut found: Vec<TitleCandidate> = groups
        .into_iter()
        .filter(|g| {
            g.unmatched && !g.taken && g.first_seen_at >= since && !rejected.contains(&g.key)
        })
        .map(|g| TitleCandidate {
            channel_id: channel.channel.id.clone(),
            key: g.key,
            work: g.work,
            latest_title: g.latest_title,
            items: g.items,
            first_seen_at: g.first_seen_at,
            latest_seen_at: g.latest_seen_at,
            waiting: waiting.iter().map(|r| r.id.clone()).collect(),
        })
        .collect();
    found.sort_by(|a, b| {
        b.latest_seen_at
            .cmp(&a.latest_seen_at)
            .then_with(|| a.work.cmp(&b.work))
    });
    found
}

#[cfg(test)]
#[path = "candidates_tests.rs"]
mod tests;
