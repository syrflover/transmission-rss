//! The preview of an edited rule: what it would do with the items a channel has
//! recorded.
//!
//! The preview does not judge titles itself. [`preview`] puts the edited rule
//! into the channel's stored rules (at the requested place in the order), hands
//! that to the mapping the worker uses ([`ChannelPlan`]) and judges every item
//! it is given with it, so the channel's excludes, collect folder and rule order
//! apply as they do in a cycle, and the same items and settings give the same
//! selection, applied rule and save path. The web's rule API and the episode
//! offset suggestions both ask here.
//!
//! The preview and a cycle still differ where the preview has no choice:
//!
//! - A rule that is not saved yet is no subscription (the preview of a new
//!   subscription is asked with a plain new rule), so [`Kind::Past`] is never
//!   shown for it, although the saved subscription would leave what was
//!   recorded before it past.
//! - A rule that waits for its title and is given one, or that is paused or
//!   archived now, is previewed as if it were saved after everything recorded
//!   so far (`i64::MAX` stands for the save time), so what it has not taken is
//!   past for it.
//! - The items are the channel's history, up to what the caller read, not the
//!   current feed. History keeps titles with the channel's long secret query
//!   values replaced by [`MASK`]; items whose stored title contains it are
//!   flagged `masked`, because the worker, which saw the original, may judge
//!   them differently.
//! - A video revision that a replacement rule withholds, and an unset collect
//!   folder (the cycle adds nothing, the preview shows the save path), are not
//!   looked at.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use super::{ChannelPlan, Judgement, PastCause, PlanEvaluation};
use crate::{
    episode_offset,
    release_name::ReleaseName,
    store::{
        channels::{ChannelWithRules, Rule, RuleInput, RuleState, MASK},
        history::{HistoryItem, HistoryResult, KnownItem},
    },
};

#[cfg(test)]
mod tests;

/// The ID the edited rule has in a preview when it is not saved yet.
pub const NEW_RULE_ID: &str = "new";

/// The rule to edit is not one of the channel's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("rule {0} not found")]
pub struct RuleNotFound(pub String);

/// The edit to preview.
#[derive(Debug, Clone, Copy)]
pub struct Edit<'a> {
    /// The stored rule it replaces, or `None` for a rule not saved yet.
    pub id: Option<&'a str>,
    /// The rule's fields as the edit leaves them.
    pub input: &'a RuleInput,
    /// Its place in the channel's rules; `None` keeps the stored rule's place
    /// (a new rule goes last).
    pub position: Option<usize>,
}

/// How an item relates to the edited rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The edited rule is the first to match: it would take the item.
    Mine,
    /// The edited rule matches, but an earlier rule takes the item.
    Earlier,
    /// The edited rule matches, but a channel exclude keeps the item out.
    Excluded,
    /// The edited rule would take the item, but history recorded it before the
    /// rule became a subscription or while it was paused or archived, so the
    /// cycle leaves it alone until the user receives it
    /// ([`ChannelPlan::is_past`]).
    Past,
}

/// The rule that takes an item instead of the edited rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakenBy {
    pub rule_id: String,
    pub r#match: Option<String>,
}

/// One recorded item the edited rule matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewItem {
    pub id: i64,
    /// The title as history stores it.
    pub title: String,
    pub first_seen_at: i64,
    /// The stored title contains [`MASK`].
    pub masked: bool,
    pub kind: Kind,
    /// Where the item would be saved by the rule that takes it; `None` when it
    /// is excluded.
    pub save_path: Option<PathBuf>,
    /// The rule that takes the item (only for [`Kind::Earlier`]).
    pub taken_by: Option<TakenBy>,
    /// The channel exclude that keeps the item out (only for [`Kind::Excluded`]).
    pub excluded_by: Option<String>,
    /// Why the item is past (only for [`Kind::Past`]).
    pub past_cause: Option<PastCause>,
    /// What history recorded for the item so far.
    pub stored_result: HistoryResult,
    /// The whole episode the release names, for an item the edited rule takes
    /// ([`Kind::Mine`], [`Kind::Past`]); `None` otherwise or when the title
    /// names none.
    pub release: Option<u32>,
    /// The episode the item is received as with the edited rule's offset and
    /// folder ([`episode_offset::received_as`]); set with `release`.
    pub episode_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviewCounts {
    /// Recorded items of the channel that were judged.
    pub total: usize,
    pub mine: usize,
    pub earlier: usize,
    pub excluded: usize,
    /// Items of a rule that holds back its past items, recorded before it
    /// began or resumed, which wait for the user to receive them.
    pub past: usize,
    /// Items the edited rule does not match (taken by other rules or by none).
    pub unmatched: usize,
}

#[derive(Debug, Clone)]
pub struct Preview {
    /// Set when the edited rule's regular expression does not compile; the rule
    /// then matches nothing and `items` is empty.
    pub error: Option<regex::Error>,
    pub counts: PreviewCounts,
    /// How many of the judged items have a masked title.
    pub masked_total: usize,
    /// The items the edited rule matches, newest first as given, up to the
    /// limit.
    pub items: Vec<PreviewItem>,
    /// There are more matching items than `items` lists.
    pub truncated: bool,
    /// The earliest release among the items the edited rule may be asked to
    /// receive (taken by it and not received by any rule), listed or not.
    pub earliest_receivable: Option<u32>,
}

/// The channel's rules with the edited one put in: replacing the stored rule
/// `edit.id` when there is one, or added as a new rule. It always collects.
fn substitute(cwr: &ChannelWithRules, edit: Edit<'_>) -> Result<ChannelWithRules, RuleNotFound> {
    let edited = edit.input;
    let mut rules = cwr.rules.clone();
    let stored_index = match edit.id {
        Some(id) => Some(
            rules
                .iter()
                .position(|r| r.id == id)
                .ok_or_else(|| RuleNotFound(id.to_owned()))?,
        ),
        None => None,
    };
    let mut rule = match stored_index {
        Some(index) => rules.remove(index),
        None => Rule {
            id: NEW_RULE_ID.to_owned(),
            channel_id: cwr.channel.id.clone(),
            position: rules.len() as i64,
            version: 0,
            r#match: None,
            regex: false,
            case_insensitive: false,
            directory: String::new(),
            episode: 0,
            episode_auto: false,
            state: RuleState::Active,
            subscription: None,
            resumed_at: None,
        },
    };
    // A subscription that waits for its title is given one by a save, and what
    // history recorded before is past for it, as the store notes it.
    if rule.r#match.is_none() && edited.r#match.is_some() {
        if let Some(subscription) = rule.subscription.as_mut() {
            subscription.titled_at = Some(i64::MAX);
        }
    }
    rule.r#match = edited.r#match.clone();
    rule.regex = edited.regex;
    rule.case_insensitive = edited.case_insensitive;
    rule.directory = edited.directory.clone();
    rule.episode = edited.episode;
    // The preview shows the rule collecting. One that is off now would be
    // turned back on after everything recorded so far, so what it has not
    // taken is past for it.
    if rule.state != RuleState::Active {
        rule.resumed_at = Some(i64::MAX);
    }
    rule.state = RuleState::Active;

    let at = edit
        .position
        .or(stored_index)
        .unwrap_or(rules.len())
        .min(rules.len());
    rules.insert(at, rule);
    Ok(ChannelWithRules {
        channel: cwr.channel.clone(),
        rules,
    })
}

/// Judges `items` with the edited rule put into the channel's rules, listing
/// at most `list_limit` of the items the rule matches (the counts cover all of
/// them). Each item says whether the channel's first read recorded it, which
/// tells a subscription what the feed already held then. Pure: the same items
/// and settings always give the same [`Preview`].
///
/// Rules save under `collect_folder`; an empty path (no folder set yet) leaves
/// a rule's directory alone as the save path.
pub fn preview(
    collect_folder: &Path,
    cwr: &ChannelWithRules,
    edit: Edit<'_>,
    items: &[HistoryItem],
    list_limit: usize,
) -> Result<Preview, RuleNotFound> {
    let substituted = substitute(cwr, edit)?;
    let edited = edit.input;
    let id = edit.id.unwrap_or(NEW_RULE_ID).to_owned();
    let match_of: HashMap<&str, Option<String>> = substituted
        .rules
        .iter()
        .map(|r| (r.id.as_str(), r.r#match.clone()))
        .collect();

    // The same mapping twice: as the channel is, and with no excludes, which
    // tells whether an excluded item would have been the edited rule's.
    let plan = ChannelPlan::new(substituted.clone(), collect_folder);
    let mut open_channel = substituted.clone();
    open_channel.channel.excludes.clear();
    let open_plan = ChannelPlan::new(open_channel, collect_folder);

    let error = plan
        .rule_errors()
        .into_iter()
        .find(|problem| problem.rule_id == id)
        .map(|problem| problem.error);

    let mut counts = PreviewCounts {
        total: items.len(),
        ..PreviewCounts::default()
    };
    let mut masked_total = 0;
    let mut listed = Vec::new();
    let mut earliest_receivable: Option<u32> = None;

    for item in items {
        let masked = item.title.contains(MASK);
        masked_total += usize::from(masked);
        let PlanEvaluation {
            judgement,
            overlapping,
        } = plan.evaluate(&item.title);

        let matches_edited = |applied: Option<&str>, overlapping: &[String]| {
            applied == Some(id.as_str()) || overlapping.iter().any(|r| r == &id)
        };
        let mut past_cause = None;
        let (kind, save_path, taken_by, excluded_by) = match &judgement {
            Judgement::Selected {
                rule_id, save_path, ..
            } if rule_id == &id => {
                // The cycle's own test, on the same record of the item.
                let known = Some(KnownItem::from(item));
                let kind = if plan.is_past(&id, known) {
                    past_cause = plan.past_cause(&id, item.first_seen_at);
                    Kind::Past
                } else {
                    Kind::Mine
                };
                (kind, Some(save_path.clone()), None, None)
            }
            Judgement::Selected {
                rule_id, save_path, ..
            } if overlapping.iter().any(|r| r == &id) => (
                Kind::Earlier,
                Some(save_path.clone()),
                Some(TakenBy {
                    rule_id: rule_id.clone(),
                    r#match: match_of.get(rule_id.as_str()).cloned().flatten(),
                }),
                None,
            ),
            Judgement::Excluded => {
                let open = open_plan.evaluate(&item.title);
                let applied = match &open.judgement {
                    Judgement::Selected { rule_id, .. } => Some(rule_id.as_str()),
                    _ => None,
                };
                if matches_edited(applied, &open.overlapping) {
                    let by = cwr
                        .channel
                        .excludes
                        .iter()
                        .find(|ex| item.title.contains(ex.as_str()))
                        .cloned();
                    (Kind::Excluded, None, None, by)
                } else {
                    counts.unmatched += 1;
                    continue;
                }
            }
            _ => {
                counts.unmatched += 1;
                continue;
            }
        };

        match kind {
            Kind::Mine => counts.mine += 1,
            Kind::Earlier => counts.earlier += 1,
            Kind::Excluded => counts.excluded += 1,
            Kind::Past => counts.past += 1,
        }
        let release = matches!(kind, Kind::Mine | Kind::Past)
            .then(|| ReleaseName::read(&item.title).whole_episode())
            .flatten();
        if item.result == HistoryResult::NoMatch {
            if let Some(release) = release {
                earliest_receivable = Some(earliest_receivable.map_or(release, |e| e.min(release)));
            }
        }
        if listed.len() < list_limit {
            listed.push(PreviewItem {
                id: item.id,
                title: item.title.clone(),
                first_seen_at: item.first_seen_at,
                masked,
                kind,
                save_path,
                taken_by,
                excluded_by,
                past_cause,
                stored_result: item.result,
                release,
                episode_name: release.and_then(|_| {
                    episode_offset::received_as(&edited.directory, edited.episode, &item.title)
                }),
            });
        }
    }

    let matching = counts.mine + counts.earlier + counts.excluded + counts.past;
    Ok(Preview {
        error,
        counts,
        masked_total,
        truncated: matching > listed.len(),
        items: listed,
        earliest_receivable,
    })
}

/// The IDs of the rules some title is taken by an earlier rule although they
/// match it too, as `plan` judges the titles: a rule is `overlap` when it is
/// among the rules an item's judgement lists as also matching. Archived and
/// paused rules are not in the plan, so they neither take items nor overlap.
pub fn overlapping_rules<'a>(
    plan: &ChannelPlan,
    titles: impl IntoIterator<Item = &'a str>,
) -> HashSet<String> {
    let mut overlap = HashSet::new();
    for title in titles {
        overlap.extend(plan.evaluate(title).overlapping);
    }
    overlap
}
