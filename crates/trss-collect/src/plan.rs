//! Mapping from the stored channels and rules to the shared rule evaluation.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::{
    rss::{save_path, ChannelEvaluator, ChannelSpec, Outcome, RuleSpec, SkipReason},
    store::{
        channels::{Channel, ChannelWithRules, Rule, RuleState},
        history::{HistoryResult, KnownItem},
    },
};
use trss_core::Millis;
use trss_transmission::Redactor;

pub mod preview;

/// What the evaluation decided about one item title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Judgement {
    Selected {
        /// ID of the applied stored rule.
        rule_id: String,
        save_path: PathBuf,
        episode: isize,
    },
    /// The title contains one of the channel's excludes.
    Excluded,
    /// No active rule matched the title.
    NoMatch,
}

/// A [`Judgement`] together with the later rules that also matched the title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEvaluation {
    pub judgement: Judgement,
    /// IDs of the active rules after the applied one that also match the title,
    /// in order. Empty unless a rule was applied.
    pub overlapping: Vec<String>,
}

/// An active rule whose regular expression does not compile.
#[derive(Debug, Clone)]
pub struct RuleProblem {
    pub rule_id: String,
    pub error: regex::Error,
}

/// A channel ready to judge items: the shared evaluator built from the
/// channel's active rules, with the way back from the evaluation's rule
/// numbers to stored rule IDs.
pub struct ChannelPlan {
    pub channel: Channel,
    evaluator: ChannelEvaluator,
    /// Stored rule ID at each evaluation rule number.
    rule_ids: Vec<String>,
    /// For each active rule that holds back its past items, since when. See
    /// [`ChannelPlan::is_past`].
    past_since: HashMap<String, PastSince>,
}

/// The moments before which a rule leaves unpicked items to the user: when it
/// became a subscription, when it was given the title it had waited for, and
/// when it was last turned back on. At least one is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PastSince {
    pub subscribed: Option<Millis>,
    pub titled: Option<Millis>,
    pub resumed: Option<Millis>,
}

impl PastSince {
    /// The latest moment: what history first saw before it is past.
    pub fn until(self) -> Millis {
        self.subscribed
            .into_iter()
            .chain(self.titled)
            .chain(self.resumed)
            .max()
            .unwrap_or(Millis::MIN)
    }
}

/// What made an item past for a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PastCause {
    /// The item was recorded before the rule became a subscription.
    Subscribed,
    /// The item was recorded before the subscription, which had waited for its
    /// title, was given one.
    Titled,
    /// The item was first seen while the rule was paused or archived.
    Resumed,
    /// The item was already in the feed when the channel was first read, which
    /// is before any history of it existed.
    FirstRead,
}

impl PastCause {
    /// The stable code the web sends.
    pub fn code(self) -> &'static str {
        match self {
            PastCause::Subscribed => "subscribed",
            PastCause::Titled => "titled",
            PastCause::Resumed => "resumed",
            PastCause::FirstRead => "first_read",
        }
    }
}

/// `None` for a rule that is no subscription and was never turned back on.
pub fn past_since(rule: &Rule) -> Option<PastSince> {
    let subscribed = rule.subscription.as_ref().map(|s| s.subscribed_at);
    let titled = rule.subscription.as_ref().and_then(|s| s.titled_at);
    (subscribed.is_some() || rule.resumed_at.is_some()).then_some(PastSince {
        subscribed,
        titled,
        resumed: rule.resumed_at,
    })
}

impl ChannelPlan {
    /// Paused and archived rules are left out (they collect nothing); the
    /// remaining rules keep their stored order,
    /// which is their priority. A rule without a match phrase (waiting for its
    /// title) is passed on as `pattern: None`, which matches nothing.
    ///
    /// Rules save under `collect_folder`, the app-wide collect folder. A caller
    /// with no folder set (the web preview on a fresh database) passes an empty
    /// path, which leaves each rule's own directory as the save path; the worker
    /// never adds with one (see `run_cycle` in the `trss-worker` crate).
    pub fn new(channel_with_rules: ChannelWithRules, collect_folder: &Path) -> ChannelPlan {
        ChannelPlan::build(channel_with_rules, collect_folder, false)
    }

    /// The plan for reading a channel that has no history yet. Its
    /// subscription rules sit out, as paused rules do: whatever the feed
    /// already holds is for the user to pick, not for a subscription to
    /// receive (`docs/specs/collection.md`, 방영작 구독), so the items are
    /// judged by the other rules and the ones nothing takes are recorded as
    /// `no_match`. Later plans call those items past ([`ChannelPlan::is_past`]).
    pub fn for_first_read(
        channel_with_rules: ChannelWithRules,
        collect_folder: &Path,
    ) -> ChannelPlan {
        ChannelPlan::build(channel_with_rules, collect_folder, true)
    }

    fn build(
        channel_with_rules: ChannelWithRules,
        collect_folder: &Path,
        first_read: bool,
    ) -> ChannelPlan {
        let ChannelWithRules { channel, rules } = channel_with_rules;

        let active: Vec<&Rule> = rules
            .iter()
            .filter(|rule| rule.state == RuleState::Active)
            .filter(|rule| !(first_read && rule.subscription.is_some()))
            .collect();

        let spec = ChannelSpec {
            directory: collect_folder.to_path_buf(),
            excludes: channel.excludes.clone(),
            rules: active.iter().map(|rule| rule_spec(rule)).collect(),
        };
        let rule_ids = active.iter().map(|rule| rule.id.clone()).collect();
        let past_since = active
            .iter()
            .filter_map(|rule| Some((rule.id.clone(), past_since(rule)?)))
            .collect();

        ChannelPlan {
            channel,
            evaluator: ChannelEvaluator::new(spec),
            rule_ids,
            past_since,
        }
    }

    /// Whether `rule_id` is an active rule of this plan that holds back past
    /// items (see [`ChannelPlan::is_past`]): a subscription, or a rule that was
    /// turned back on after being paused or archived.
    pub fn holds_past(&self, rule_id: &str) -> bool {
        self.past_since.contains_key(rule_id)
    }

    /// Whether the plan has an active rule that holds back past items.
    pub fn has_past_holders(&self) -> bool {
        !self.past_since.is_empty()
    }

    /// Why an item of `rule_id` first seen at `first_seen_at` is past: it came
    /// before the subscription, else before the subscription got its title,
    /// else while the rule was off, or else the feed already held it when the
    /// channel was first read. `None` when the rule holds nothing back.
    pub fn past_cause(&self, rule_id: &str, first_seen_at: Millis) -> Option<PastCause> {
        let since = self.past_since.get(rule_id)?;
        Some(if since.subscribed.is_some_and(|at| first_seen_at < at) {
            PastCause::Subscribed
        } else if since.titled.is_some_and(|at| first_seen_at < at) {
            PastCause::Titled
        } else if since.resumed.is_some_and(|at| first_seen_at < at) {
            PastCause::Resumed
        } else {
            PastCause::FirstRead
        })
    }

    /// Whether `rule_id` must leave an item alone because the item is past:
    /// history recorded it, without any rule taking it, before the rule became
    /// a subscription, before a subscription that waited for its title was
    /// given one, or while the rule was paused or archived (before it was last
    /// turned back on). Only the user receives those, after looking at them
    /// (`docs/specs/collection.md`, 방영작 구독 and `영상 받기`). So is what the
    /// feed already held when the channel was first read, whenever the
    /// subscription began: the first read has no history to tell the old
    /// items from the new, so a subscription (never a plain rule) leaves all
    /// of them ([`ChannelPlan::for_first_read`]). A rule that is no
    /// subscription and was never turned back on has no past.
    /// `known` is the item's history record: when it was first seen, its result
    /// and whether the channel's first read recorded it. An item a rule picked
    /// and failed to add is not past, nor is one first seen after the rule began
    /// or resumed collecting. What the first read recorded is told by the item
    /// itself, not by comparing times, so no clock moves that line.
    pub fn is_past(&self, rule_id: &str, known: Option<KnownItem>) -> bool {
        let Some(since) = self.past_since.get(rule_id) else {
            return false;
        };
        matches!(
            known,
            Some(KnownItem {
                first_seen_at,
                result: HistoryResult::NoMatch | HistoryResult::Excluded,
                first_read,
            }) if first_seen_at < since.until() || (since.subscribed.is_some() && first_read)
        )
    }

    pub fn judge(&self, title: &str) -> Judgement {
        self.evaluate(title).judgement
    }

    /// The judgement of `judge`, plus which later rules also matched. The web's
    /// rule preview reads this, so that it and the worker cannot differ.
    pub fn evaluate(&self, title: &str) -> PlanEvaluation {
        let evaluation = self.evaluator.evaluate(title);
        let judgement = match evaluation.outcome {
            Outcome::Selected {
                rule,
                save_path,
                episode,
            } => Judgement::Selected {
                rule_id: self.rule_ids[rule].clone(),
                save_path,
                episode,
            },
            Outcome::Skipped(SkipReason::ChannelExcluded) => Judgement::Excluded,
            Outcome::Skipped(SkipReason::NoRuleMatched) => Judgement::NoMatch,
        };
        PlanEvaluation {
            judgement,
            overlapping: evaluation
                .overlapping
                .iter()
                .map(|&rule| self.rule_ids[rule].clone())
                .collect(),
        }
    }

    /// The active rules whose regular expression does not compile, by rule ID.
    /// Such a rule matches nothing; the channel's other rules still apply.
    pub fn rule_errors(&self) -> Vec<RuleProblem> {
        self.evaluator
            .rule_errors()
            .iter()
            .map(|err| RuleProblem {
                rule_id: self.rule_ids[err.rule].clone(),
                error: err.source.clone(),
            })
            .collect()
    }

    /// One line per active rule whose regular expression does not compile.
    pub fn rule_problems(&self) -> Vec<String> {
        self.rule_errors()
            .iter()
            .map(|problem| {
                format!(
                    "Invalid regex in rule {} of {}: {}",
                    problem.rule_id,
                    self.channel.masked_url(),
                    problem.error
                )
            })
            .collect()
    }

    /// Knows the channel URL's secret query values (those long enough to be
    /// replaced in free text, see [`trss_transmission::MIN_QUERY_SECRET_LEN`]), for cleaning error
    /// text and the text that goes into history.
    pub fn redactor(&self) -> Redactor {
        let mut redactor = Redactor::none();
        for value in secret_values(&self.channel.url, &self.channel.secret_query) {
            redactor.add_query_value(&value);
        }
        redactor
    }
}

/// Where a rule saves what it selects and the episode conversion it applies:
/// the same values [`ChannelPlan::judge`] gives for an item the rule selects.
/// A retry of a failed item uses them, so that it lands where the rule's own
/// cycle would have put it. Both put the rule's directory under
/// `collect_folder` with [`save_path`].
pub fn rule_destination(collect_folder: &Path, rule: &Rule) -> (PathBuf, isize) {
    let spec = rule_spec(rule);
    (save_path(collect_folder, &spec.directory), spec.episode)
}

/// The folder whose turn ([`trss_core::folder_locks`]) work saving into
/// `save_path` takes: the work folder, which is the collect folder joined with
/// the first part of the save folder below it (`Clevatess` of
/// `Clevatess/Season 02`); the collect folder itself for a save folder that is
/// the collect folder; the save folder for one outside it. Placed by text, as
/// the turns are.
pub fn work_folder_of(collect_folder: &Path, save_path: &Path) -> PathBuf {
    let collect = trss_core::folders::lexical(collect_folder);
    let save = trss_core::folders::lexical(save_path);
    match save.strip_prefix(&collect) {
        Ok(below) => match below.components().next() {
            Some(first) => collect.join(first),
            None => collect,
        },
        Err(_) => save,
    }
}

/// [`work_folder_of`] the folder `rule` saves into.
pub fn rule_work_folder(collect_folder: &Path, rule: &Rule) -> PathBuf {
    work_folder_of(collect_folder, &rule_destination(collect_folder, rule).0)
}

/// Whether `rule` alone would select an item with this title in `channel`:
/// the channel does not exclude it and the rule matches. Other rules and the
/// rule's state are not looked at.
pub fn picks(channel: &Channel, rule: &Rule, title: &str) -> bool {
    let active = Rule {
        state: RuleState::Active,
        ..rule.clone()
    };
    let plan = ChannelPlan::new(
        ChannelWithRules {
            channel: channel.clone(),
            rules: vec![active],
        },
        Path::new(""),
    );
    matches!(plan.judge(title), Judgement::Selected { .. })
}

fn rule_spec(rule: &Rule) -> RuleSpec {
    RuleSpec {
        // `NULL` (title-waiting) stays `None`; the database never holds `''`.
        pattern: rule.r#match.clone(),
        regex: rule.regex,
        case_insensitive: rule.case_insensitive,
        directory: PathBuf::from(&rule.directory),
        episode: rule.episode as isize,
    }
}

/// The raw values of the query parameters of `url` named in `secret_query`.
fn secret_values(url: &str, secret_query: &[String]) -> Vec<String> {
    let rest = url.split_once('#').map_or(url, |(rest, _)| rest);
    let Some((_, query)) = rest.split_once('?') else {
        return Vec::new();
    };

    query
        .split('&')
        .filter_map(|pair| {
            let (raw_name, raw_value) = pair.split_once('=')?;
            let name = url::form_urlencoded::parse(raw_name.as_bytes())
                .next()
                .map(|(name, _)| name.into_owned())?;
            (!raw_value.is_empty() && secret_query.contains(&name)).then(|| raw_value.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::channels::{Channel, Rule};

    fn channel() -> Channel {
        Channel {
            id: "c1".into(),
            position: 0,
            version: 1,
            url: "https://feed.test/rss?r=1080&token=s3cret%20x&token2=keep".into(),
            excludes: vec!["[Batch]".into()],
            secret_query: vec!["token".into()],
            past_search: None,
            name: None,
        }
    }

    fn rule(id: &str, position: i64, m: Option<&str>, state: RuleState) -> Rule {
        Rule {
            id: id.into(),
            channel_id: "c1".into(),
            position,
            version: 1,
            r#match: m.map(str::to_owned),
            regex: false,
            case_insensitive: false,
            directory: format!("{id}/Season 01"),
            episode: 1,
            episode_auto: false,
            state,
            subscription: None,
            resumed_at: None,
        }
    }

    fn known(first_seen_at: i64, result: HistoryResult, first_read: bool) -> KnownItem {
        KnownItem {
            first_seen_at,
            result,
            first_read,
        }
    }

    fn plan(rules: Vec<Rule>) -> ChannelPlan {
        ChannelPlan::new(
            ChannelWithRules {
                channel: channel(),
                rules,
            },
            Path::new("/media/anime"),
        )
    }

    #[test]
    fn evaluation_numbers_map_back_to_stored_rule_ids() {
        let p = plan(vec![
            rule("archived-first", 0, Some("Show"), RuleState::Archived),
            rule("waiting", 1, None, RuleState::Active),
            rule("second", 2, Some("Show"), RuleState::Active),
            rule("third", 3, Some("Show"), RuleState::Active),
        ]);

        // Archived rules are skipped, so evaluation number 1 is "second".
        let Judgement::Selected {
            rule_id,
            save_path,
            episode,
        } = p.judge("[G] Show - 01")
        else {
            panic!("expected a selection");
        };
        assert_eq!(rule_id, "second");
        assert_eq!(save_path, PathBuf::from("/media/anime/second/Season 01"));
        assert_eq!(episode, 1);
    }

    #[test]
    fn evaluation_lists_the_later_active_rules_that_also_match_by_id() {
        let p = plan(vec![
            rule("first", 0, Some("Show"), RuleState::Active),
            rule("archived", 1, Some("Show"), RuleState::Archived),
            rule("waiting", 2, None, RuleState::Active),
            rule("other", 3, Some("Other"), RuleState::Active),
            rule("last", 4, Some("Sho"), RuleState::Active),
        ]);

        let evaluation = p.evaluate("Show - 01");
        assert_eq!(evaluation.overlapping, ["last"]);
        assert_eq!(evaluation.judgement, p.judge("Show - 01"));
        // Nothing overlaps when a single rule matches, or none does.
        assert!(p.evaluate("Other - 01").overlapping.is_empty());
        assert!(p.evaluate("Nothing").overlapping.is_empty());
        // An excluded item is taken by no rule, so it overlaps nothing either.
        let excluded = p.evaluate("Show [Batch]");
        assert_eq!(excluded.judgement, Judgement::Excluded);
        assert!(excluded.overlapping.is_empty());
    }

    #[test]
    fn a_rule_holds_back_what_came_before_the_later_of_its_subscription_and_its_resume() {
        use crate::store::channels::{Subscription, SubtitleMode};
        let subscribed = |at| Subscription {
            anissia_anime_no: 1,
            subtitles: SubtitleMode::None,
            creator: None,
            season_id: None,
            subscribed_at: at,
            season_blocked: None,
            titled_at: None,
        };
        let held = |subscribed_at: Option<i64>, resumed_at: Option<i64>| {
            let mut r = rule("r", 0, Some("Show"), RuleState::Active);
            r.subscription = subscribed_at.map(subscribed);
            r.resumed_at = resumed_at;
            plan(vec![r])
        };
        let seen = |at| Some(known(at, HistoryResult::NoMatch, false));

        // Never subscribed nor turned back on: nothing is past.
        let plain = held(None, None);
        assert!(!plain.holds_past("r") && !plain.has_past_holders());
        assert!(!plain.is_past("r", seen(1)));

        // Resumed alone: what was first seen before is past, with that cause.
        let resumed = held(None, Some(100));
        assert!(resumed.is_past("r", seen(99)));
        assert!(!resumed.is_past("r", seen(100)));
        assert_eq!(resumed.past_cause("r", 99), Some(PastCause::Resumed));

        // A subscription that resumed later is held back to the resume; an
        // item from before the subscription says so.
        let both = held(Some(50), Some(100));
        assert!(both.is_past("r", seen(99)));
        assert_eq!(both.past_cause("r", 99), Some(PastCause::Resumed));
        assert_eq!(both.past_cause("r", 49), Some(PastCause::Subscribed));
        // One that resumed before it subscribed, to the subscription.
        let subscribed_later = held(Some(100), Some(50));
        assert!(subscribed_later.is_past("r", seen(99)));
        assert!(!subscribed_later.is_past("r", seen(100)));
        assert_eq!(
            subscribed_later.past_cause("r", 99),
            Some(PastCause::Subscribed)
        );

        // Only an item nobody took is past: a failed add or a held torrent is not.
        for result in [
            HistoryResult::AddFailed,
            HistoryResult::Received,
            HistoryResult::Duplicate,
        ] {
            assert!(
                !both.is_past("r", Some(known(1, result, false))),
                "{result:?}"
            );
        }
        assert!(both.is_past("r", Some(known(1, HistoryResult::Excluded, false))));
        // An item history does not know is not past.
        assert!(!both.is_past("r", None));
    }

    #[test]
    fn a_title_given_late_holds_back_what_history_recorded_before_it() {
        use crate::store::channels::{Subscription, SubtitleMode};
        let titled = |subscribed: i64, titled: Option<i64>, resumed: Option<i64>| {
            let mut r = rule("r", 0, Some("Show"), RuleState::Active);
            r.subscription = Some(Subscription {
                anissia_anime_no: 1,
                subtitles: SubtitleMode::None,
                creator: None,
                season_id: None,
                subscribed_at: subscribed,
                season_blocked: None,
                titled_at: titled,
            });
            r.resumed_at = resumed;
            plan(vec![r])
        };
        let seen = |at| Some(known(at, HistoryResult::NoMatch, false));

        // Subscribed at 50, given its title at 200: what came before 200 is
        // past, and says it came before the title, or before the subscription.
        let p = titled(50, Some(200), None);
        assert!(p.is_past("r", seen(199)));
        assert!(!p.is_past("r", seen(200)));
        assert_eq!(p.past_cause("r", 199), Some(PastCause::Titled));
        assert_eq!(p.past_cause("r", 49), Some(PastCause::Subscribed));
        assert_eq!(PastCause::Titled.code(), "titled");

        // A subscription that had its title from the start holds back to the
        // subscription only.
        let p = titled(50, None, None);
        assert!(p.is_past("r", seen(49)) && !p.is_past("r", seen(50)));

        // The latest of the moments rules: a resume after the title.
        let p = titled(50, Some(200), Some(300));
        assert!(p.is_past("r", seen(299)) && !p.is_past("r", seen(300)));
        assert_eq!(p.past_cause("r", 250), Some(PastCause::Resumed));
    }

    #[test]
    fn what_the_feed_held_at_the_first_read_is_past_for_a_subscription_only() {
        use crate::store::channels::{Subscription, SubtitleMode};
        let rules = |subscribed_at: Option<i64>| {
            let mut sub = rule("sub", 0, Some("Show"), RuleState::Active);
            sub.subscription = subscribed_at.map(|at| Subscription {
                anissia_anime_no: 1,
                subtitles: SubtitleMode::None,
                creator: None,
                season_id: None,
                subscribed_at: at,
                season_blocked: None,
                titled_at: None,
            });
            vec![sub, rule("plain", 1, Some("Show"), RuleState::Active)]
        };
        // `first_read`: the item was recorded by the channel's first read.
        let seen = |at, result, first_read| Some(known(at, result, first_read));

        // The subscription began at 50, before the first read: the boundary
        // alone calls nothing past, the first read does.
        let p = plan(rules(Some(50)));
        assert!(p.is_past("sub", seen(100, HistoryResult::NoMatch, true)));
        assert!(p.is_past("sub", seen(100, HistoryResult::Excluded, true)));
        assert_eq!(p.past_cause("sub", 100), Some(PastCause::FirstRead));
        assert_eq!(PastCause::FirstRead.code(), "first_read");
        // An item the first read did not record is the subscription's own,
        // whenever its time says it was seen: so is an item a rule took.
        assert!(!p.is_past("sub", seen(101, HistoryResult::NoMatch, false)));
        assert!(!p.is_past("sub", seen(100, HistoryResult::NoMatch, false)));
        assert!(!p.is_past("sub", seen(60, HistoryResult::NoMatch, false)));
        assert!(!p.is_past("sub", seen(100, HistoryResult::AddFailed, true)));
        assert!(!p.is_past("sub", seen(100, HistoryResult::Received, true)));
        // An item of the first read is past however late its time says it is.
        assert!(p.is_past("sub", seen(10_000, HistoryResult::NoMatch, true)));
        // A plain rule has no past, and none of the first read's.
        assert!(!p.is_past("plain", seen(100, HistoryResult::NoMatch, true)));

        // A subscription that began after the first read: the earlier cause
        // is the one shown.
        let p = plan(rules(Some(150)));
        assert!(p.is_past("sub", seen(100, HistoryResult::NoMatch, true)));
        assert_eq!(p.past_cause("sub", 100), Some(PastCause::Subscribed));

        // A rule turned back on, no subscription, ignores the first read.
        let mut resumed = rule("r", 0, Some("Show"), RuleState::Active);
        resumed.resumed_at = Some(50);
        let p = plan(vec![resumed]);
        assert!(!p.is_past("r", seen(100, HistoryResult::NoMatch, true)));
    }

    #[test]
    fn a_plan_for_a_first_read_lets_the_subscriptions_sit_out() {
        use crate::store::channels::{Subscription, SubtitleMode};
        let mut sub = rule("sub", 0, Some("Show"), RuleState::Active);
        sub.subscription = Some(Subscription {
            anissia_anime_no: 1,
            subtitles: SubtitleMode::None,
            creator: None,
            season_id: None,
            subscribed_at: 50,
            season_blocked: None,
            titled_at: None,
        });
        let rules = vec![
            sub,
            rule("plain", 1, Some("Show"), RuleState::Active),
            rule("other", 2, Some("Other"), RuleState::Active),
        ];
        let cwr = ChannelWithRules {
            channel: channel(),
            rules,
        };

        // As a paused rule: the later rule takes what the subscription would
        // have, and nothing else changes.
        let first = ChannelPlan::for_first_read(cwr.clone(), Path::new("/media/anime"));
        assert!(matches!(
            first.judge("Show - 01"),
            Judgement::Selected { ref rule_id, .. } if rule_id == "plain"
        ));
        assert!(first.evaluate("Show - 01").overlapping.is_empty());
        assert!(!first.holds_past("sub"));

        let ordinary = ChannelPlan::new(cwr, Path::new("/media/anime"));
        assert!(matches!(
            ordinary.judge("Show - 01"),
            Judgement::Selected { ref rule_id, .. } if rule_id == "sub"
        ));
    }

    #[test]
    fn archived_rules_never_apply() {
        let p = plan(vec![rule("old", 0, Some("Show"), RuleState::Archived)]);
        assert_eq!(p.judge("Show - 01"), Judgement::NoMatch);
    }

    #[test]
    fn paused_rules_never_apply_and_never_shadow_a_later_rule() {
        let p = plan(vec![
            rule("paused", 0, Some("Show"), RuleState::Paused),
            rule("later", 1, Some("Show"), RuleState::Active),
        ]);
        assert!(matches!(
            p.judge("Show - 01"),
            Judgement::Selected { ref rule_id, .. } if rule_id == "later"
        ));
        // Nor does the paused rule count as overlapping the one that takes it.
        assert!(p.evaluate("Show - 01").overlapping.is_empty());

        let only = plan(vec![rule("paused", 0, Some("Show"), RuleState::Paused)]);
        assert_eq!(only.judge("Show - 01"), Judgement::NoMatch);
    }

    #[test]
    fn a_rule_without_a_match_phrase_matches_nothing_and_does_not_shadow() {
        let p = plan(vec![
            rule("waiting", 0, None, RuleState::Active),
            rule("real", 1, Some("Show"), RuleState::Active),
        ]);
        assert!(matches!(
            p.judge("Show - 01"),
            Judgement::Selected { ref rule_id, .. } if rule_id == "real"
        ));
        assert_eq!(p.judge("Other - 01"), Judgement::NoMatch);

        let only_waiting = plan(vec![rule("waiting", 0, None, RuleState::Active)]);
        assert_eq!(only_waiting.judge("anything at all"), Judgement::NoMatch);
        assert_eq!(only_waiting.judge(""), Judgement::NoMatch);
    }

    #[test]
    fn excludes_beat_rules() {
        let p = plan(vec![rule("r", 0, Some("Show"), RuleState::Active)]);
        assert_eq!(p.judge("Show - 01-12 [Batch]"), Judgement::Excluded);
    }

    #[test]
    fn a_broken_regex_rule_is_reported_by_id_and_matches_nothing() {
        let mut broken = rule("broken", 0, Some("("), RuleState::Active);
        broken.regex = true;
        let p = plan(vec![broken, rule("ok", 1, Some("Show"), RuleState::Active)]);

        let problems = p.rule_problems();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("broken"));
        assert!(matches!(
            p.judge("Show"),
            Judgement::Selected { ref rule_id, .. } if rule_id == "ok"
        ));
    }

    #[test]
    fn secret_values_come_only_from_the_secret_parameters() {
        let p = plan(vec![]);
        let r = p.redactor();
        let text = "error for url (https://feed.test/rss?r=1080&token=s3cret%20x&token2=keep)";
        let out = r.apply(text);
        assert!(!out.contains("s3cret"), "{out}");
        assert!(out.contains("token2=keep") && out.contains("r=1080"));
    }

    #[test]
    fn short_values_of_secret_parameters_do_not_garble_unrelated_text() {
        // Every query name is secret by default, so `r=1080` and `f=0` are too.
        let mut channel = channel();
        channel.url = "https://feed.test/rss?r=1080&f=0&page=rss&token=Tk3n-0123456789".into();
        channel.secret_query = ["r", "f", "page", "token"].map(str::to_owned).into();
        let p = ChannelPlan::new(
            ChannelWithRules {
                channel,
                rules: vec![],
            },
            Path::new("/media/anime"),
        );

        let r = p.redactor();
        let text = "HTTP status 503 on Show - 1080p, page 10 (0 of 10)";
        assert_eq!(r.apply(text), text);
        // A real token in the same URL stays masked, in every spelling.
        let out = r.apply("failed: Tk3n-0123456789 / Tk3n-0123456789");
        assert!(!out.contains("Tk3n"), "{out}");
        // The URL-shaped masking still hides the short values by name.
        assert_eq!(
            p.channel.masked_url(),
            "https://feed.test/rss?r=***&f=***&page=***&token=***"
        );
    }
}
