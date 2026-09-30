//! Deciding what an import does to the channels that already exist.
//!
//! A file channel is *the same channel* as an existing one when they have the
//! same address apart from query values: scheme, host, port, path and the set
//! of query names. Query values are ignored because they are secrets (a
//! rotated token must not turn a channel into a new one) and because the app
//! marks every query value secret. When several channels qualify, an existing
//! channel whose whole URL equals the file's is preferred, and one existing
//! channel is claimed by at most one file channel (in file order), so no two
//! file channels can replace the same channel.
//!
//! A file channel without an existing counterpart is added without asking.
//! One with a counterpart needs a [`Choice`] from the user; [`build_actions`]
//! accepts the choices only if they cover exactly the channels that conflict
//! *now*, so a review that went stale is refused instead of applied.

use std::collections::HashSet;

use url::Url;

use crate::store::channels::import::{ImportAction, ImportChannel};
use crate::store::channels::{mask_url, query_names, ChannelWithRules, Version};

/// The user's decision for one file channel that already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Replace,
    Add,
    Skip,
}

/// A decision together with what the user reviewed: the file channel's index
/// and the existing channel (with the version) it was compared to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub index: usize,
    pub existing_id: String,
    pub existing_version: Version,
    pub decision: Decision,
}

/// The choices do not match the channels that exist now: the review is stale
/// (or incomplete). Nothing should be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleReview;

/// The actions to apply, each with the index of the file channel it comes from,
/// and the file channels that are skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub actions: Vec<(usize, ImportAction)>,
    pub skipped: Vec<usize>,
}

/// The comparison key of a URL (see the module docs); `None` if it is not a
/// URL, in which case the channel matches nothing.
fn identity(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let mut names = query_names(url);
    names.sort();
    Some(format!(
        "{}://{}:{}{}?{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or_default(),
        parsed.port_or_known_default().unwrap_or_default(),
        parsed.path(),
        names.join("&"),
    ))
}

fn normalized(url: &str) -> Option<String> {
    Url::parse(url).ok().map(|u| u.to_string())
}

/// For each file channel, the index into `existing` of the channel it is the
/// same as, if any.
pub fn find_existing(file: &[ImportChannel], existing: &[ChannelWithRules]) -> Vec<Option<usize>> {
    let mut found = vec![None; file.len()];
    let mut claimed = HashSet::new();

    // Exact URLs first, then the same address with other query values.
    let exact: Vec<_> = existing
        .iter()
        .map(|e| normalized(&e.channel.url))
        .collect();
    for (i, channel) in file.iter().enumerate() {
        let Some(url) = normalized(&channel.input.url) else {
            continue;
        };
        if let Some(e) = (0..existing.len())
            .find(|e| !claimed.contains(e) && exact[*e].as_deref() == Some(url.as_str()))
        {
            claimed.insert(e);
            found[i] = Some(e);
        }
    }

    let keys: Vec<_> = existing.iter().map(|e| identity(&e.channel.url)).collect();
    for (i, channel) in file.iter().enumerate() {
        if found[i].is_some() {
            continue;
        }
        let Some(key) = identity(&channel.input.url) else {
            continue;
        };
        if let Some(e) = (0..existing.len())
            .find(|e| !claimed.contains(e) && keys[*e].as_deref() == Some(key.as_str()))
        {
            claimed.insert(e);
            found[i] = Some(e);
        }
    }
    found
}

/// Turns the choices into actions. `file` channels are consumed in order.
///
/// A file channel with no existing counterpart is added. One with a
/// counterpart must have exactly one choice, naming that counterpart and, for
/// a replacement, the version that was reviewed; any other choice list is
/// [`StaleReview`].
pub fn build_actions(
    file: Vec<ImportChannel>,
    existing: &[ChannelWithRules],
    choices: &[Choice],
) -> Result<Plan, StaleReview> {
    let found = find_existing(&file, existing);

    let mut decided = HashSet::new();
    for choice in choices {
        let counterpart = found
            .get(choice.index)
            .copied()
            .flatten()
            .map(|e| &existing[e].channel)
            .ok_or(StaleReview)?;
        let up_to_date = counterpart.id == choice.existing_id
            && (choice.decision != Decision::Replace
                || counterpart.version == choice.existing_version);
        if !up_to_date || !decided.insert(choice.index) {
            return Err(StaleReview);
        }
    }
    if found.iter().flatten().count() != decided.len() {
        return Err(StaleReview);
    }

    let mut plan = Plan {
        actions: Vec::new(),
        skipped: Vec::new(),
    };
    for (index, channel) in file.into_iter().enumerate() {
        let Some(e) = found[index] else {
            plan.actions.push((index, ImportAction::Add(channel)));
            continue;
        };
        let choice = choices
            .iter()
            .find(|c| c.index == index)
            .expect("every conflicting channel was decided above");
        match choice.decision {
            Decision::Add => plan.actions.push((index, ImportAction::Add(channel))),
            Decision::Skip => plan.skipped.push(index),
            Decision::Replace => {
                let current = &existing[e].channel;
                let mut channel = channel;
                // The file cannot express the past-episode search, so a
                // replacement keeps what the app already has instead of
                // silently dropping it.
                channel.input.past_search = current.past_search.clone();
                plan.actions.push((
                    index,
                    ImportAction::Replace {
                        id: current.id.clone(),
                        expected_version: current.version,
                        channel,
                    },
                ));
            }
        }
    }
    Ok(plan)
}

/// The URL as shown to the user: every query value and any user info masked.
/// This is the only form of an imported URL that leaves the server.
pub fn display_url(url: &str) -> String {
    let masked = mask_url(url, &query_names(url));
    match Url::parse(&masked) {
        Ok(mut parsed) if !parsed.username().is_empty() || parsed.password().is_some() => {
            let _ = parsed.set_username("***");
            if parsed.password().is_some() {
                let _ = parsed.set_password(Some("***"));
            }
            parsed.to_string()
        }
        _ => masked,
    }
}

#[cfg(test)]
mod tests {
    use crate::store::channels::{Channel, ChannelInput, Rule, RuleInput, RuleState};

    use super::*;

    fn file_channel(url: &str) -> ImportChannel {
        ImportChannel {
            input: ChannelInput::new(url, "/media"),
            rules: vec![RuleInput::default()],
        }
    }

    fn existing_channel(id: &str, version: Version, url: &str) -> ChannelWithRules {
        let input = ChannelInput::new(url, "/old");
        ChannelWithRules {
            channel: Channel {
                id: id.into(),
                position: 0,
                version,
                url: input.url,
                base_dir: input.base_dir,
                excludes: vec![],
                secret_query: input.secret_query,
                past_search: Some("[X] {match}".into()),
            },
            rules: vec![Rule {
                id: format!("{id}-r"),
                channel_id: id.into(),
                position: 0,
                version: 1,
                r#match: Some("a".into()),
                regex: false,
                case_insensitive: false,
                directory: String::new(),
                episode: 1,
                episode_auto: false,
                state: RuleState::Active,
            }],
        }
    }

    fn choice(index: usize, id: &str, version: Version, decision: Decision) -> Choice {
        Choice {
            index,
            existing_id: id.into(),
            existing_version: version,
            decision,
        }
    }

    #[test]
    fn the_same_address_with_other_query_values_is_the_same_channel() {
        let existing = vec![
            existing_channel("a", 1, "https://feeds.example/rss?filter=1080p&token=old"),
            existing_channel("b", 1, "https://feeds.example/other?token=old"),
        ];
        let file = vec![
            file_channel("https://FEEDS.example:443/rss?token=NEW&filter=720p"),
            file_channel("https://feeds.example/rss?token=x"),
            file_channel("https://feeds.example/other"),
            file_channel("http://feeds.example/rss?filter=1&token=2"),
            file_channel("https://feeds.example/rss?filter=1080p&token=t&extra=1"),
        ];
        // 0 claims "a"; 1 (other query names) and 4 (extra name) and 3 (scheme)
        // are different channels; 2 (no query names) differs from "b".
        assert_eq!(
            find_existing(&file, &existing),
            [Some(0), None, None, None, None]
        );
    }

    #[test]
    fn an_exact_url_wins_and_an_existing_channel_is_claimed_once() {
        let existing = vec![
            existing_channel("a", 1, "https://f.example/rss?t=1"),
            existing_channel("b", 1, "https://f.example/rss?t=2"),
        ];
        let file = vec![
            file_channel("https://f.example/rss?t=9"),
            file_channel("https://f.example/rss?t=2"),
            file_channel("https://f.example/rss?t=3"),
        ];
        // The second is exactly "b"; the first then takes "a"; nothing is left
        // for the third, which becomes a new channel.
        assert_eq!(find_existing(&file, &existing), [Some(0), Some(1), None]);
    }

    #[test]
    fn nothing_is_asked_when_no_channel_exists() {
        let file = vec![
            file_channel("https://a.example/rss"),
            file_channel("https://b.example/rss"),
        ];
        let plan = build_actions(file, &[], &[]).unwrap();
        assert_eq!(plan.actions.len(), 2);
        assert!(plan.skipped.is_empty());
        assert!(plan
            .actions
            .iter()
            .all(|(_, a)| matches!(a, ImportAction::Add(_))));
    }

    #[test]
    fn every_conflicting_channel_needs_one_choice_that_matches_now() {
        let existing = vec![
            existing_channel("a", 3, "https://a.example/rss?t=1"),
            existing_channel("b", 5, "https://b.example/rss?t=1"),
        ];
        let file = || {
            vec![
                file_channel("https://a.example/rss?t=2"),
                file_channel("https://new.example/rss"),
                file_channel("https://b.example/rss?t=2"),
            ]
        };

        // Only one of the two conflicts chosen.
        let partial = [choice(0, "a", 3, Decision::Replace)];
        assert_eq!(build_actions(file(), &existing, &partial), Err(StaleReview));
        // No choices at all.
        assert_eq!(build_actions(file(), &existing, &[]), Err(StaleReview));
        // A choice for a channel that does not conflict.
        let extra = [
            choice(0, "a", 3, Decision::Skip),
            choice(1, "a", 3, Decision::Skip),
            choice(2, "b", 5, Decision::Skip),
        ];
        assert_eq!(build_actions(file(), &existing, &extra), Err(StaleReview));
        // A stale version, a different channel, a duplicate choice.
        let stale = [
            choice(0, "a", 2, Decision::Replace),
            choice(2, "b", 5, Decision::Skip),
        ];
        assert_eq!(build_actions(file(), &existing, &stale), Err(StaleReview));
        let wrong = [
            choice(0, "b", 5, Decision::Skip),
            choice(2, "b", 5, Decision::Skip),
        ];
        assert_eq!(build_actions(file(), &existing, &wrong), Err(StaleReview));
        let twice = [
            choice(0, "a", 3, Decision::Skip),
            choice(0, "a", 3, Decision::Skip),
            choice(2, "b", 5, Decision::Skip),
        ];
        assert_eq!(build_actions(file(), &existing, &twice), Err(StaleReview));

        // Complete: replace a, skip b, and the new channel is added unasked.
        let all = [
            choice(0, "a", 3, Decision::Replace),
            choice(2, "b", 5, Decision::Skip),
        ];
        let plan = build_actions(file(), &existing, &all).unwrap();
        assert_eq!(plan.skipped, [2]);
        assert_eq!(plan.actions.len(), 2);
        assert!(matches!(
            &plan.actions[0],
            (0, ImportAction::Replace { id, expected_version: 3, .. }) if id == "a"
        ));
        assert!(matches!(&plan.actions[1], (1, ImportAction::Add(_))));
        // The replacement keeps the past-episode search the file cannot hold.
        let ImportAction::Replace { channel, .. } = &plan.actions[0].1 else {
            unreachable!()
        };
        assert_eq!(channel.input.past_search.as_deref(), Some("[X] {match}"));

        // Add ignores the version: the existing channel is not touched.
        let add = [
            choice(0, "a", 99, Decision::Add),
            choice(2, "b", 5, Decision::Add),
        ];
        let plan = build_actions(file(), &existing, &add).unwrap();
        assert!(plan.skipped.is_empty());
        assert_eq!(plan.actions.len(), 3);
    }

    #[test]
    fn displayed_urls_never_carry_query_values_or_credentials() {
        assert_eq!(
            display_url("https://f.example/rss?filter=1080p&token=s3cr3t#frag"),
            "https://f.example/rss?filter=***&token=***#frag"
        );
        let shown = display_url("https://user:hunter2@f.example/rss?token=s3cr3t");
        assert!(!shown.contains("hunter2") && !shown.contains("user:"));
        assert!(!shown.contains("s3cr3t"));
        assert!(shown.contains("f.example/rss"));
    }
}
