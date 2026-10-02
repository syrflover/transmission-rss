//! Setting the episode offset of a new season's rule before the rule's first
//! item is named (ticket 0024; the grounds and the table of results are in
//! [`crate::episode_offset`]).
//!
//! A cycle calls [`settle`] after it has judged the feeds and before it adds
//! anything, with the titles each rule is about to receive. A rule is looked at
//! only when it is a subscription whose offset is not automatic, has not
//! picked an item before and has never been decided by the app (whatever its
//! field holds: a value carried over from the previous season is replaced too):
//! the decision is made once, by the first items, and the items a rule already
//! received are never renamed by it. A field that already names the releases
//! as the decided offset would is left as it is. The `다시 받기` of a past item
//! for a rule that has picked nothing does the same with its one item
//! ([`settle_one`]).
//!
//! The offset is stored with the version the cycle read the rule at. When the
//! user saved the rule meanwhile, it is read again and decided once more if
//! the app may still decide it and the save changed neither its offset nor
//! what picked and places the items ([`same_choice`]); otherwise the rule is
//! left as the user made it. Either way the cycle's items are named with the
//! offset the rule has then: when the stored offset is no longer the one the
//! cycle read (the user saved another), [`settle`] hands that one back, so a
//! value carried over in the cycle's snapshot does not name the first items
//! after the user replaced it.
//!
//! Anything that cannot be read (the library, the season info, the history)
//! leaves the rule as it is, with a line in the log: a rule is received with
//! the offset it has rather than with a guess.

use std::collections::HashMap;

use crate::store::{channels::ChannelStore, history::HistoryStore};
use crate::{
    episode_offset::{decide, first_release, gather, may_decide, same_effect, signed, Verdict},
    store::channels::Rule,
};
use trss_library::store::{library::LibraryStore, seasons::SeasonStore};

/// What the episode offsets use (made from
/// [`CollectContext::offsets`](crate::context::CollectContext::offsets)).
/// Cheap to clone.
#[derive(Clone)]
pub struct OffsetsContext {
    pub channels: ChannelStore,
    /// Where the earlier receives of a rule are read.
    pub history: HistoryStore,
    /// With `seasons`, the seasons before a rule's ([`crate::episode_offset`]).
    pub library: LibraryStore,
    pub seasons: SeasonStore,
}

/// The offsets the rules that are about to receive their first items take,
/// by rule ID: the one the app set, or the user's when the user saved another
/// while the cycle ran. A rule not in it keeps the offset the cycle read. `firsts` has the titles each rule is about to receive; `open`
/// the rules of the cycle's snapshot that may be looked at at all
/// ([`may_decide`]).
pub async fn settle(
    ctx: &OffsetsContext,
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
    let candidates: Vec<String> = candidates
        .into_iter()
        .filter(|id| !picked.contains(id))
        .collect();
    let undecided = match undecided(ctx, candidates).await {
        Some(undecided) => undecided,
        None => return set,
    };
    for id in undecided {
        if let Some(offset) = settle_rule(ctx, collect_folder, &open[&id], &firsts[&id]).await {
            set.insert(id, offset);
        }
    }
    set
}

/// [`settle`] for one rule and one item: the rule as it is after, when the app
/// set its offset or the user saved another meanwhile. Used by `다시 받기` of a past item, which is a rule's first
/// when the rule has picked nothing.
pub async fn settle_one(
    ctx: &OffsetsContext,
    collect_folder: &str,
    rule: &Rule,
    title: &str,
) -> Option<Rule> {
    if !may_decide(rule) {
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
    if undecided(ctx, vec![rule.id.clone()]).await?.is_empty() {
        return None;
    }
    settle_rule(ctx, collect_folder, rule, &[title.to_owned()]).await?;
    ctx.channels.get_rule(&rule.id).await.ok().flatten()
}

/// Decides one rule from its first titles and stores the offset if the app
/// sets one. The offset the titles take: the one set, or the stored one when
/// the user saved another since `rule` was read; `None` when `rule`'s holds.
async fn settle_rule(
    ctx: &OffsetsContext,
    collect_folder: &str,
    rule: &Rule,
    titles: &[String],
) -> Option<i64> {
    let first = first_release(titles)?;
    let read = rule.episode;
    // What the items take when the app sets nothing: the offset as last read.
    let kept = |now: &Rule| (now.episode != read).then_some(now.episode);
    let mut rule = rule.clone();
    // The second try is for a rule the user saved while the cycle ran.
    for _ in 0..2 {
        let basis = match gather(&ctx.library, &ctx.seasons, collect_folder, &rule).await {
            Ok(Some(basis)) => basis,
            Ok(None) => return kept(&rule),
            Err(err) => {
                eprintln!(
                    "Episode offset: rule {} is received without one: {err}",
                    rule.id
                );
                return kept(&rule);
            }
        };
        let Verdict::Auto { offset, basis, .. } = decide(first, &basis) else {
            return kept(&rule);
        };
        if same_effect(rule.episode, offset) {
            // Named as the app would name them already: nothing to tell.
            return kept(&rule);
        }
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
                return Some(offset);
            }
            Ok(None) => match ctx.channels.get_rule(&rule.id).await {
                Ok(Some(now)) if may_decide(&now) && same_choice(&now, &rule) => rule = now,
                Ok(now) => {
                    println!(
                        "Episode offset: rule {} was changed meanwhile and is left as it is",
                        rule.id
                    );
                    return now.and_then(|now| kept(&now));
                }
                Err(err) => {
                    eprintln!("Episode offset: cannot read rule {} again: {err}", rule.id);
                    return None;
                }
            },
            Err(err) => {
                eprintln!(
                    "Episode offset: cannot save the offset of rule {}: {err}",
                    rule.id
                );
                return kept(&rule);
            }
        }
    }
    println!(
        "Episode offset: rule {} kept changing and is received without one",
        rule.id
    );
    kept(&rule)
}

/// The rules among `ids` the app has never decided, or `None` (with a line in
/// the log) when that cannot be read.
async fn undecided(ctx: &OffsetsContext, ids: Vec<String>) -> Option<Vec<String>> {
    match ctx.channels.episode_marks(ids.clone()).await {
        Ok(marks) => Some(
            ids.into_iter()
                .filter(|id| marks.get(id).is_some_and(|mark| !mark.decided))
                .collect(),
        ),
        Err(err) => {
            eprintln!("Episode offset: cannot read what was decided: {err}");
            None
        }
    }
}

/// Whether a save left what the cycle decided from as it was: the offset, and
/// what picked the titles and names the folder they go to. The cycle's items
/// were chosen and placed by the rule it read, so another choice is not
/// decided from them.
fn same_choice(now: &Rule, read: &Rule) -> bool {
    now.episode == read.episode
        && now.r#match == read.r#match
        && now.regex == read.regex
        && now.case_insensitive == read.case_insensitive
        && now.directory == read.directory
}
