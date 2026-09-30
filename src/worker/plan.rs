//! Mapping from the stored channels and rules to the shared rule evaluation.

use std::path::PathBuf;

use crate::{
    rss::{ChannelEvaluator, ChannelSpec, Outcome, RuleSpec, SkipReason},
    store::channels::{Channel, ChannelWithRules, Rule, RuleState},
    transmission::Redactor,
};

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

/// A channel ready to judge items: the shared evaluator built from the
/// channel's active rules, with the way back from the evaluation's rule
/// numbers to stored rule IDs.
pub struct ChannelPlan {
    pub channel: Channel,
    evaluator: ChannelEvaluator,
    /// Stored rule ID at each evaluation rule number.
    rule_ids: Vec<String>,
}

impl ChannelPlan {
    /// Archived rules are left out; the remaining rules keep their stored order,
    /// which is their priority. A rule without a match phrase (waiting for its
    /// title) is passed on as `pattern: None`, which matches nothing.
    pub fn new(channel_with_rules: ChannelWithRules) -> ChannelPlan {
        let ChannelWithRules { channel, rules } = channel_with_rules;

        let active: Vec<&Rule> = rules
            .iter()
            .filter(|rule| rule.state == RuleState::Active)
            .collect();

        let spec = ChannelSpec {
            directory: PathBuf::from(&channel.base_dir),
            excludes: channel.excludes.clone(),
            rules: active.iter().map(|rule| rule_spec(rule)).collect(),
        };
        let rule_ids = active.iter().map(|rule| rule.id.clone()).collect();

        ChannelPlan {
            channel,
            evaluator: ChannelEvaluator::new(spec),
            rule_ids,
        }
    }

    pub fn judge(&self, title: &str) -> Judgement {
        match self.evaluator.evaluate(title).outcome {
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
        }
    }

    /// One line per active rule whose regular expression does not compile.
    /// Such a rule matches nothing; the channel's other rules still apply.
    pub fn rule_problems(&self) -> Vec<String> {
        self.evaluator
            .rule_errors()
            .iter()
            .map(|err| {
                format!(
                    "Invalid regex in rule {} of {}: {}",
                    self.rule_ids[err.rule],
                    self.channel.masked_url(),
                    err.source
                )
            })
            .collect()
    }

    /// Knows the channel URL's secret query values, for cleaning error text.
    pub fn redactor(&self) -> Redactor {
        let mut redactor = Redactor::none();
        for value in secret_values(&self.channel.url, &self.channel.secret_query) {
            redactor.add(&value);
        }
        redactor
    }
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
            base_dir: "/media/anime".into(),
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
        }
    }

    fn plan(rules: Vec<Rule>) -> ChannelPlan {
        ChannelPlan::new(ChannelWithRules {
            channel: channel(),
            rules,
        })
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
    fn archived_rules_never_apply() {
        let p = plan(vec![rule("old", 0, Some("Show"), RuleState::Archived)]);
        assert_eq!(p.judge("Show - 01"), Judgement::NoMatch);
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
}
