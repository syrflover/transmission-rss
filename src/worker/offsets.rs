//! Setting the episode offset of a new season's rule before the rule's first
//! item is named (ticket 0024; the grounds and the table of results are in
//! [`crate::episode_offset`]).
//!
//! A cycle calls [`settle`] after it has judged the feeds and before it adds
//! anything, with the titles each rule is about to receive. A rule is looked at
//! only when it is a subscription whose offset is still `0` or `1` and was not
//! set by the app, and has not picked an item before: the decision is made once,
//! by the first items, and the items a rule already received are never renamed.
//! The `다시 받기` of a past item for a rule that has picked nothing does the
//! same with its one item ([`settle_one`]).
//!
//! The offset is stored with the version the cycle read the rule at, so a rule
//! the user edited meanwhile is left as the user made it, and its items are
//! named without an offset this time.
//!
//! Anything that cannot be read (the library, the season info, the history)
//! leaves the rule as it is, with a line in the log: a rule is received without
//! an offset rather than with a guess.

use std::collections::HashMap;

use super::CycleContext;
use crate::{
    episode_offset::{decide, first_release, gather, is_open, signed, Verdict},
    store::channels::Rule,
};

/// The offsets set for the rules that are about to receive their first items,
/// by rule ID. `firsts` has the titles each rule is about to receive; `open`
/// the rules of the cycle's snapshot that may be looked at at all
/// ([`is_open`]).
pub async fn settle(
    ctx: &CycleContext,
    collect_folder: &str,
    open: &HashMap<String, Rule>,
    firsts: &HashMap<String, Vec<String>>,
) -> HashMap<String, i64> {
    let candidates: Vec<String> = firsts
        .keys()
        .filter(|id| open.contains_key(*id))
        .cloned()
        .collect();
    let mut set = HashMap::new();
    if candidates.is_empty() {
        return set;
    }
    let picked = match ctx.history.rules_with_items(candidates.clone()).await {
        Ok(picked) => picked,
        Err(err) => {
            eprintln!("Episode offset: cannot read the history: {err}");
            return set;
        }
    };
    for id in candidates.into_iter().filter(|id| !picked.contains(id)) {
        if let Some(offset) = settle_rule(ctx, collect_folder, &open[&id], &firsts[&id]).await {
            set.insert(id, offset);
        }
    }
    set
}

/// [`settle`] for one rule and one item: the rule as it is after, when the app
/// set its offset. Used by `다시 받기` of a past item, which is a rule's first
/// when the rule has picked nothing.
pub async fn settle_one(
    ctx: &CycleContext,
    collect_folder: &str,
    rule: &Rule,
    title: &str,
) -> Option<Rule> {
    if !is_open(rule) {
        return None;
    }
    match ctx.history.rules_with_items(vec![rule.id.clone()]).await {
        Ok(picked) if picked.is_empty() => {}
        Ok(_) => return None,
        Err(err) => {
            eprintln!("Episode offset: cannot read the history: {err}");
            return None;
        }
    }
    settle_rule(ctx, collect_folder, rule, &[title.to_owned()]).await?;
    ctx.channels.get_rule(&rule.id).await.ok().flatten()
}

/// Decides one rule from its first titles and stores the offset if the app
/// sets one. The offset set, or `None`.
async fn settle_rule(
    ctx: &CycleContext,
    collect_folder: &str,
    rule: &Rule,
    titles: &[String],
) -> Option<i64> {
    let first = first_release(titles)?;
    let basis = match gather(&ctx.library, &ctx.seasons, collect_folder, rule).await {
        Ok(Some(basis)) => basis,
        Ok(None) => return None,
        Err(err) => {
            eprintln!(
                "Episode offset: rule {} is received without one: {err}",
                rule.id
            );
            return None;
        }
    };
    let Verdict::Auto { offset, basis, .. } = decide(first, &basis) else {
        return None;
    };
    match ctx
        .channels
        .set_auto_episode(&rule.id, rule.version, offset, &basis)
        .await
    {
        Ok(Some(_)) => {
            println!(
                "Episode offset: rule {} gets {} from its first release, {first}",
                rule.id,
                signed(offset)
            );
            Some(offset)
        }
        // The user edited the rule since the cycle read it, or it is gone.
        Ok(None) => None,
        Err(err) => {
            eprintln!(
                "Episode offset: cannot save the offset of rule {}: {err}",
                rule.id
            );
            None
        }
    }
}
